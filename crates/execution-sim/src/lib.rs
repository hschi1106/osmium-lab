mod accounting;

use std::{collections::BTreeMap, collections::BTreeSet, error::Error, fmt};

use market_types::{
    DomainEvent, EventPayload, InstrumentId, MatchTime, Price, Quantity, QuantityUnit,
};
use replay_engine::EventOccurrence;
use strategy_api::{
    CancellationReason, MatchingState, NewOrderEntry, OrderFeedback, OrderId, OrderIntent,
    OrderRestrictionReason, OrderSide, OrderType, RejectionReason, SessionSegmentId,
    TradingContext,
};

pub const EXECUTION_SIM_VERSION: u16 = 2;
pub const FILL_MODEL_VERSION: u16 = 2;

pub use accounting::{
    ACCOUNTING_VERSION, AccountingError, AccountingModel, ChargeModel, ChargeSides,
    InstrumentEconomics, InstrumentLedgerConfig, InstrumentPerformance, Ledger, MultiLedger,
    MultiPerformanceSummary, PerformanceSummary, RoundingPolicy,
};

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
    pub market_data_latency_ms: u64,
    pub order_latency_ms: u64,
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
    eligible_match_time: MatchTime,
    origin_segment_id: SessionSegmentId,
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
    pub const fn eligible_match_time(&self) -> MatchTime {
        self.eligible_match_time
    }
    #[must_use]
    pub const fn status(&self) -> OrderStatus {
        self.status
    }

    #[must_use]
    pub const fn origin_segment_id(&self) -> &SessionSegmentId {
        &self.origin_segment_id
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
        strategy_id: &str,
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
        identity.extend_from_slice(strategy_id.as_bytes());
        identity.extend_from_slice(occurrence.event_fingerprint().as_bytes());
        identity.extend_from_slice(&occurrence.run_event_ordinal().to_be_bytes());
        identity.extend_from_slice(&output_sequence.to_be_bytes());
        identity.extend_from_slice(&canonical);
        let id = OrderId::from_bytes(*blake3::hash(&identity).as_bytes());
        let eligible_match_time = add_latency(
            occurrence.ordering_key().match_time(),
            self.model.market_data_latency_ms,
            self.model.order_latency_ms,
        )?;
        let sequence = self.next_acceptance_sequence;
        self.next_acceptance_sequence = sequence
            .checked_add(1)
            .ok_or(SimulationError::SequenceOverflow)?;
        self.orders.push(SimOrder {
            id,
            intent,
            origin_ordinal: occurrence.run_event_ordinal(),
            eligible_match_time,
            origin_segment_id: trading.session().segment_id().clone(),
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
        if matches!(
            trading.new_order_entry(),
            NewOrderEntry::Blocked(_) | NewOrderEntry::Unknown
        ) {
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
            ) || !order_is_eligible(
                order.origin_ordinal,
                order.eligible_match_time,
                occurrence.run_event_ordinal(),
                event.match_time(),
            ) || event.instrument() != order.intent.instrument()
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
            if evidence_quantity.unit() != order.intent.quantity().unit() {
                continue;
            }
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

    pub fn cancel_segment_end(&mut self, segment_id: &SessionSegmentId) -> Vec<OrderFeedback> {
        self.orders
            .iter_mut()
            .filter(|order| {
                order.origin_segment_id == *segment_id
                    && matches!(
                        order.status,
                        OrderStatus::Pending | OrderStatus::PartiallyFilled
                    )
            })
            .map(|order| {
                order.status = OrderStatus::Cancelled;
                OrderFeedback::Cancelled {
                    order_id: order.id,
                    reason: CancellationReason::SegmentEnd,
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
        (EvidenceMode::TopOfBook, EventPayload::BookSnapshot(snapshot)) => {
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

fn add_latency(
    match_time: MatchTime,
    market_data_latency_ms: u64,
    order_latency_ms: u64,
) -> Result<MatchTime, SimulationError> {
    let total_latency_millis = market_data_latency_ms
        .checked_add(order_latency_ms)
        .ok_or(SimulationError::LatencyOverflow)?;
    let latency_micros = total_latency_millis
        .checked_mul(1_000)
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(SimulationError::LatencyOverflow)?;
    let value = match_time
        .as_unix_microseconds()
        .checked_add(latency_micros)
        .ok_or(SimulationError::LatencyOverflow)?;
    Ok(MatchTime::from_unix_microseconds(value))
}

fn order_is_eligible(
    origin_ordinal: u64,
    eligible_match_time: MatchTime,
    current_ordinal: u64,
    current_match_time: MatchTime,
) -> bool {
    current_ordinal > origin_ordinal && current_match_time >= eligible_match_time
}

#[derive(Debug)]
pub enum SimulationError {
    Canonical(String),
    SequenceOverflow,
    QuantityOverflow,
    InvalidQuantity,
    InvalidSlippage,
    LatencyOverflow,
    EmptyUniverse,
    DuplicateInstrument,
}

impl fmt::Display for SimulationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for SimulationError {}

/// Instrument-isolated simulation facade for an M3 universe.
#[derive(Debug)]
pub struct MultiSimulator {
    simulators: BTreeMap<InstrumentId, Simulator>,
}

impl MultiSimulator {
    pub fn new(
        configs: impl IntoIterator<Item = (InstrumentId, QuantityUnit, FillModel)>,
    ) -> Result<Self, SimulationError> {
        let mut simulators = BTreeMap::new();
        for (instrument, quantity_unit, model) in configs {
            if simulators
                .insert(
                    instrument.clone(),
                    Simulator::new([instrument], quantity_unit, model),
                )
                .is_some()
            {
                return Err(SimulationError::DuplicateInstrument);
            }
        }
        if simulators.is_empty() {
            return Err(SimulationError::EmptyUniverse);
        }
        Ok(Self { simulators })
    }

    pub fn submit(
        &mut self,
        strategy_id: &str,
        occurrence: &EventOccurrence,
        trading: &TradingContext,
        output_sequence: u32,
        intent: OrderIntent,
    ) -> Result<OrderFeedback, SimulationError> {
        let Some(simulator) = self.simulators.get_mut(intent.instrument()) else {
            return Ok(OrderFeedback::Rejected {
                reason: RejectionReason::InstrumentOutsideUniverse,
            });
        };
        simulator.submit(strategy_id, occurrence, trading, output_sequence, intent)
    }

    pub fn evaluate(
        &mut self,
        event: &DomainEvent,
        occurrence: &EventOccurrence,
        trading: &TradingContext,
    ) -> Result<Vec<OrderFeedback>, SimulationError> {
        self.simulators.get_mut(event.instrument()).map_or_else(
            || Ok(Vec::new()),
            |simulator| simulator.evaluate(event, occurrence, trading),
        )
    }

    pub fn cancel_segment_end(&mut self, segment_id: &SessionSegmentId) -> Vec<OrderFeedback> {
        self.simulators
            .values_mut()
            .flat_map(|simulator| simulator.cancel_segment_end(segment_id))
            .collect()
    }

    pub fn cancel_segment_end_for(
        &mut self,
        instrument: &InstrumentId,
        segment_id: &SessionSegmentId,
    ) -> Vec<OrderFeedback> {
        self.simulators
            .get_mut(instrument)
            .map_or_else(Vec::new, |simulator| {
                simulator.cancel_segment_end(segment_id)
            })
    }

    pub fn cancel_end_of_run(&mut self) -> Vec<OrderFeedback> {
        self.simulators
            .values_mut()
            .flat_map(Simulator::cancel_end_of_run)
            .collect()
    }

    pub fn cancel_end_of_run_for(&mut self, instrument: &InstrumentId) -> Vec<OrderFeedback> {
        self.simulators
            .get_mut(instrument)
            .map_or_else(Vec::new, Simulator::cancel_end_of_run)
    }

    #[must_use]
    pub fn simulator(&self, instrument: &InstrumentId) -> Option<&Simulator> {
        self.simulators.get(instrument)
    }

    #[must_use]
    pub fn fills_for(&self, instrument: &InstrumentId) -> Option<&[FillRecord]> {
        self.simulators.get(instrument).map(Simulator::fills)
    }

    #[must_use]
    pub fn orders_for(&self, instrument: &InstrumentId) -> Option<&[SimOrder]> {
        self.simulators.get(instrument).map(Simulator::orders)
    }

    pub fn instruments(&self) -> impl Iterator<Item = &InstrumentId> {
        self.simulators.keys()
    }

    #[must_use]
    pub fn orders(&self) -> Vec<&SimOrder> {
        self.simulators
            .values()
            .flat_map(Simulator::orders)
            .collect()
    }

    #[must_use]
    pub fn fills(&self) -> Vec<&FillRecord> {
        self.simulators
            .values()
            .flat_map(Simulator::fills)
            .collect()
    }

    #[must_use]
    pub fn order_count(&self) -> usize {
        self.simulators
            .values()
            .map(|simulator| simulator.orders().len())
            .sum()
    }

    #[must_use]
    pub fn fill_count(&self) -> usize {
        self.simulators
            .values()
            .map(|simulator| simulator.fills().len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use market_types::{
        BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, EventPayload, IndicativeAuction,
        MarketAnnotations, MarketId, Observation, QuoteSnapshot, SourceFormatId, Symbol,
        TradePrint, TradePrintKind, TradingDate, Volume,
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
            market_data_latency_ms: 0,
            order_latency_ms: 0,
        };
        assert_eq!(
            apply_slippage(Price::parse("100").unwrap(), OrderSide::Buy, model).unwrap(),
            Price::parse("101").unwrap()
        );
    }

    #[test]
    fn indicative_auction_is_never_fill_evidence() {
        let actual = event();
        let EventPayload::QuoteSnapshot(snapshot) = actual.payload() else {
            panic!("fixture helper must create a quote snapshot")
        };
        let auction = DomainEvent::new(
            actual.instrument().clone(),
            actual.trading_date(),
            actual.source_format().clone(),
            actual.match_time(),
            None,
            EventPayload::IndicativeOpeningAuction(
                IndicativeAuction::new(
                    Observation::Set(Price::parse("100").unwrap()),
                    Observation::Set(Quantity::new(1, QuantityUnit::TradingUnit).unwrap()),
                    Observation::Set(snapshot.book().clone()),
                    Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
                    MarketAnnotations::None,
                )
                .unwrap(),
            ),
        );
        assert!(evidence(&auction, EvidenceMode::TopOfBook, OrderSide::Buy).is_none());
        assert!(evidence(&auction, EvidenceMode::TradePrint, OrderSide::Buy).is_none());
    }

    #[test]
    fn latency_is_added_in_milliseconds_and_requires_a_later_match_time() {
        let origin = MatchTime::from_unix_microseconds(10_000);
        let eligible = add_latency(origin, 3, 7).unwrap();

        assert_eq!(eligible, MatchTime::from_unix_microseconds(20_000));
        assert!(!order_is_eligible(
            4,
            eligible,
            5,
            MatchTime::from_unix_microseconds(19_999)
        ));
        assert!(order_is_eligible(
            4,
            eligible,
            5,
            MatchTime::from_unix_microseconds(20_000)
        ));
        assert!(!order_is_eligible(4, origin, 4, origin));
    }

    #[test]
    fn latency_overflow_is_rejected() {
        assert!(matches!(
            add_latency(MatchTime::from_unix_microseconds(0), u64::MAX, 0),
            Err(SimulationError::LatencyOverflow)
        ));
    }
}
