use std::{collections::BTreeSet, error::Error, fmt};

use market_types::{
    DomainEvent, EventPayload, InstrumentId, MatchTime, Price, Quantity, QuantityUnit,
};
use replay_engine::EventOccurrence;
use strategy_api::{
    CancellationReason, MatchingState, NewOrderEntry, OrderFeedback, OrderId, OrderIntent,
    OrderRestrictionReason, OrderSide, OrderType, RejectionReason, TradingContext,
};

pub const EXECUTION_SIM_VERSION: u16 = 1;
pub const FILL_MODEL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceMode {
    TopOfBook,
    TradePrint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantityPolicy {
    Unlimited,
    Displayed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillModel {
    pub evidence: EvidenceMode,
    pub quantity: QuantityPolicy,
    pub adverse_price_delta: market_types::Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Pending,
    PartiallyFilled,
    Filled,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimOrder {
    id: OrderId,
    intent: OrderIntent,
    origin_ordinal: u64,
    acceptance_sequence: u64,
    filled: u64,
    status: OrderStatus,
}

impl SimOrder {
    #[must_use]
    pub const fn id(&self) -> OrderId {
        self.id
    }
    #[must_use]
    pub const fn intent(&self) -> &OrderIntent {
        &self.intent
    }
    #[must_use]
    pub const fn filled(&self) -> u64 {
        self.filled
    }
    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.intent.quantity().value() - self.filled
    }
    #[must_use]
    pub const fn status(&self) -> OrderStatus {
        self.status
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillRecord {
    order_id: OrderId,
    triggering_ordinal: u64,
    match_time: MatchTime,
    side: OrderSide,
    price: Price,
    quantity: Quantity,
}

impl FillRecord {
    #[must_use]
    pub const fn order_id(&self) -> OrderId {
        self.order_id
    }
    #[must_use]
    pub const fn triggering_ordinal(&self) -> u64 {
        self.triggering_ordinal
    }
    #[must_use]
    pub const fn match_time(&self) -> MatchTime {
        self.match_time
    }
    #[must_use]
    pub const fn side(&self) -> OrderSide {
        self.side
    }
    #[must_use]
    pub const fn price(&self) -> Price {
        self.price
    }
    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }
}

#[derive(Debug)]
pub struct Simulator {
    universe: BTreeSet<InstrumentId>,
    quantity_unit: QuantityUnit,
    model: FillModel,
    orders: Vec<SimOrder>,
    fills: Vec<FillRecord>,
    next_acceptance_sequence: u64,
}

impl Simulator {
    #[must_use]
    pub fn new(
        universe: impl IntoIterator<Item = InstrumentId>,
        quantity_unit: QuantityUnit,
        model: FillModel,
    ) -> Self {
        Self {
            universe: universe.into_iter().collect(),
            quantity_unit,
            model,
            orders: Vec::new(),
            fills: Vec::new(),
            next_acceptance_sequence: 1,
        }
    }

    pub fn submit(
        &mut self,
        occurrence: &EventOccurrence,
        trading: &TradingContext,
        output_sequence: u32,
        intent: OrderIntent,
    ) -> Result<OrderFeedback, SimulationError> {
        if !self.universe.contains(intent.instrument()) {
            return Ok(OrderFeedback::Rejected {
                reason: RejectionReason::InstrumentOutsideUniverse,
            });
        }
        if intent.quantity().unit() != self.quantity_unit {
            return Ok(OrderFeedback::Rejected {
                reason: RejectionReason::QuantityUnitMismatch,
            });
        }
        let entry_allowed = match trading.new_order_entry() {
            NewOrderEntry::Allowed => true,
            NewOrderEntry::Restricted(OrderRestrictionReason::PreOpenLimitOrdersOnly)
            | NewOrderEntry::Restricted(OrderRestrictionReason::IndicativeMarket) => {
                matches!(intent.order_type(), OrderType::Limit { .. })
            }
            NewOrderEntry::Blocked(_) | NewOrderEntry::Unknown => false,
        };
        if !entry_allowed {
            return Ok(OrderFeedback::Rejected {
                reason: RejectionReason::NewOrderEntryBlocked,
            });
        }
        let canonical = intent
            .to_canonical_bytes()
            .map_err(|error| SimulationError::Canonical(error.to_string()))?;
        let mut identity = Vec::new();
        identity.extend_from_slice(b"OSOR");
        identity.extend_from_slice(occurrence.event_fingerprint().as_bytes());
        identity.extend_from_slice(&occurrence.run_event_ordinal().to_be_bytes());
        identity.extend_from_slice(&output_sequence.to_be_bytes());
        identity.extend_from_slice(&canonical);
        let id = OrderId::from_bytes(*blake3::hash(&identity).as_bytes());
        let sequence = self.next_acceptance_sequence;
        self.next_acceptance_sequence = sequence
            .checked_add(1)
            .ok_or(SimulationError::SequenceOverflow)?;
        self.orders.push(SimOrder {
            id,
            intent,
            origin_ordinal: occurrence.run_event_ordinal(),
            acceptance_sequence: sequence,
            filled: 0,
            status: OrderStatus::Pending,
        });
        Ok(OrderFeedback::Accepted { order_id: id })
    }

    pub fn evaluate(
        &mut self,
        event: &DomainEvent,
        occurrence: &EventOccurrence,
        trading: &TradingContext,
    ) -> Result<Vec<OrderFeedback>, SimulationError> {
        if !matches!(trading.matching(), MatchingState::Enabled(_)) {
            return Ok(Vec::new());
        }
        let model = self.model;
        let mut available_buy = evidence(event, model.evidence, OrderSide::Buy);
        let mut available_sell = evidence(event, model.evidence, OrderSide::Sell);
        let mut feedback = Vec::new();
        self.orders
            .sort_by_key(|order| (order.acceptance_sequence, order.id));
        for order in &mut self.orders {
            if !matches!(
                order.status,
                OrderStatus::Pending | OrderStatus::PartiallyFilled
            ) || occurrence.run_event_ordinal() <= order.origin_ordinal
                || event.instrument() != order.intent.instrument()
            {
                continue;
            }
            let available = match order.intent.side() {
                OrderSide::Buy => &mut available_buy,
                OrderSide::Sell => &mut available_sell,
            };
            let Some((evidence_price, evidence_quantity)) = *available else {
                continue;
            };
            if !limit_touched(&order.intent, evidence_price) {
                continue;
            }
            let fill_price = apply_slippage(evidence_price, order.intent.side(), model)?;
            if !limit_touched(&order.intent, fill_price) {
                continue;
            }
            let remaining = order.remaining();
            let fill_value = match model.quantity {
                QuantityPolicy::Unlimited => remaining,
                QuantityPolicy::Displayed => remaining.min(evidence_quantity.value()),
            };
            if fill_value == 0 {
                continue;
            }
            let quantity = Quantity::new(fill_value, evidence_quantity.unit())
                .map_err(|_| SimulationError::InvalidQuantity)?;
            order.filled = order
                .filled
                .checked_add(fill_value)
                .ok_or(SimulationError::QuantityOverflow)?;
            let completed = order.remaining() == 0;
            order.status = if completed {
                OrderStatus::Filled
            } else {
                OrderStatus::PartiallyFilled
            };
            self.fills.push(FillRecord {
                order_id: order.id,
                triggering_ordinal: occurrence.run_event_ordinal(),
                match_time: event.match_time(),
                side: order.intent.side(),
                price: fill_price,
                quantity,
            });
            feedback.push(if completed {
                OrderFeedback::Filled {
                    order_id: order.id,
                    filled: quantity,
                }
            } else {
                OrderFeedback::PartiallyFilled {
                    order_id: order.id,
                    filled: quantity,
                    remaining: Quantity::new(order.remaining(), quantity.unit())
                        .map_err(|_| SimulationError::InvalidQuantity)?,
                }
            });
            if model.quantity == QuantityPolicy::Displayed {
                let left = evidence_quantity.value().saturating_sub(fill_value);
                *available = Quantity::new(left, evidence_quantity.unit())
                    .ok()
                    .map(|quantity| (evidence_price, quantity));
            }
        }
        Ok(feedback)
    }

    pub fn cancel_end_of_run(&mut self) -> Vec<OrderFeedback> {
        self.orders
            .iter_mut()
            .filter(|order| {
                matches!(
                    order.status,
                    OrderStatus::Pending | OrderStatus::PartiallyFilled
                )
            })
            .map(|order| {
                order.status = OrderStatus::Cancelled;
                OrderFeedback::Cancelled {
                    order_id: order.id,
                    reason: CancellationReason::EndOfRun,
                }
            })
            .collect()
    }

    #[must_use]
    pub fn orders(&self) -> &[SimOrder] {
        &self.orders
    }
    #[must_use]
    pub fn fills(&self) -> &[FillRecord] {
        &self.fills
    }
}

fn evidence(event: &DomainEvent, mode: EvidenceMode, side: OrderSide) -> Option<(Price, Quantity)> {
    match (mode, event.payload()) {
        (EvidenceMode::TopOfBook, EventPayload::QuoteSnapshot(snapshot)) => {
            let level = match side {
                OrderSide::Buy => snapshot.book().asks().levels().next(),
                OrderSide::Sell => snapshot.book().bids().levels().next(),
            };
            level.map(|level| (level.price(), level.displayed_quantity()))
        }
        (EvidenceMode::TradePrint, EventPayload::QuoteSnapshot(snapshot)) => snapshot
            .trade()
            .as_set()
            .map(|trade| (trade.price(), trade.quantity())),
        (EvidenceMode::TradePrint, EventPayload::TradeBatch(batch)) => batch
            .trades()
            .first()
            .map(|trade| (trade.price(), trade.quantity())),
        _ => None,
    }
}

fn limit_touched(intent: &OrderIntent, price: Price) -> bool {
    match intent.order_type() {
        OrderType::Market => true,
        OrderType::Limit { limit_price } => match intent.side() {
            OrderSide::Buy => price <= limit_price,
            OrderSide::Sell => price >= limit_price,
        },
    }
}

fn apply_slippage(
    price: Price,
    side: OrderSide,
    model: FillModel,
) -> Result<Price, SimulationError> {
    if model.adverse_price_delta.atoms() < 0 {
        return Err(SimulationError::InvalidSlippage);
    }
    let decimal = match side {
        OrderSide::Buy => price.as_decimal().checked_add(model.adverse_price_delta),
        OrderSide::Sell => price.as_decimal().checked_sub(model.adverse_price_delta),
    }
    .map_err(|_| SimulationError::InvalidSlippage)?;
    Price::new(decimal).map_err(|_| SimulationError::InvalidSlippage)
}

#[derive(Debug)]
pub enum SimulationError {
    Canonical(String),
    SequenceOverflow,
    QuantityOverflow,
    InvalidQuantity,
    InvalidSlippage,
}

impl fmt::Display for SimulationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for SimulationError {}

#[cfg(test)]
mod tests {
    use market_types::{
        BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, MarketAnnotations, MarketId,
        Observation, QuoteSnapshot, SourceFormatId, Symbol, TradePrint, TradePrintKind,
        TradingDate, Volume,
    };

    use super::*;

    fn event() -> DomainEvent {
        let quantity = |value| Quantity::new(value, QuantityUnit::TradingUnit).unwrap();
        let book = CompleteBookSnapshot::new(
            BookSide::new(
                BookSideKind::Bid,
                vec![BookLevel::new(Price::parse("99").unwrap(), quantity(3))],
            )
            .unwrap(),
            BookSide::new(
                BookSideKind::Ask,
                vec![BookLevel::new(Price::parse("101").unwrap(), quantity(2))],
            )
            .unwrap(),
        )
        .unwrap();
        DomainEvent::new(
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            TradingDate::parse("2026-07-27").unwrap(),
            SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
            MatchTime::from_unix_microseconds(1),
            None,
            EventPayload::QuoteSnapshot(
                QuoteSnapshot::new(
                    book,
                    Observation::Set(TradePrint::new(
                        Price::parse("100").unwrap(),
                        quantity(1),
                        TradePrintKind::Regular,
                    )),
                    Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
                    MarketAnnotations::None,
                )
                .unwrap(),
            ),
        )
    }

    #[test]
    fn top_of_book_uses_ask_for_buy_and_bid_for_sell() {
        assert_eq!(
            evidence(&event(), EvidenceMode::TopOfBook, OrderSide::Buy)
                .unwrap()
                .0,
            Price::parse("101").unwrap()
        );
        assert_eq!(
            evidence(&event(), EvidenceMode::TopOfBook, OrderSide::Sell)
                .unwrap()
                .0,
            Price::parse("99").unwrap()
        );
    }

    #[test]
    fn adverse_slippage_never_clamps_to_limit() {
        let model = FillModel {
            evidence: EvidenceMode::TopOfBook,
            quantity: QuantityPolicy::Displayed,
            adverse_price_delta: "1".parse().unwrap(),
        };
        assert_eq!(
            apply_slippage(Price::parse("100").unwrap(), OrderSide::Buy, model).unwrap(),
            Price::parse("101").unwrap()
        );
    }
}
