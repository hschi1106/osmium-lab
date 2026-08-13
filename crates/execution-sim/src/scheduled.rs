use std::{collections::BTreeMap, error::Error, fmt};

use market_types::{
    BOOK_DEPTH, CompleteBookSnapshot, Decimal, InstrumentId, MatchTime, Price, Quantity,
    QuantityUnit,
};
use strategy_api::{
    CancellationReason, ClientOrderId, ExecutionFailureReason, ExecutionFillFeedback, FillId,
    MatchingState, NewOrderEntry, OrderFeedback, OrderId, OrderRestrictionReason, OrderSide,
    OrderType, ScheduledExecutionPolicy, ScheduledOrderRequest,
};

use crate::{DepthSweepError, FillRecord, depth::sweep_marketable_depth_with_consumed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledDepthModel {
    depth_levels: usize,
    max_stale_ms: u64,
    adverse_price_delta: Decimal,
}

impl ScheduledDepthModel {
    pub fn new(
        depth_levels: usize,
        max_stale_ms: u64,
        adverse_price_delta: Decimal,
    ) -> Result<Self, ScheduledSimulationError> {
        if !(1..=BOOK_DEPTH).contains(&depth_levels) {
            return Err(ScheduledSimulationError::InvalidDepthLevels(depth_levels));
        }
        if max_stale_ms == 0 {
            return Err(ScheduledSimulationError::ZeroStalenessWindow);
        }
        if adverse_price_delta.atoms() < 0 {
            return Err(ScheduledSimulationError::InvalidAdversePriceDelta);
        }
        Ok(Self {
            depth_levels,
            max_stale_ms,
            adverse_price_delta,
        })
    }

    #[must_use]
    pub const fn depth_levels(self) -> usize {
        self.depth_levels
    }

    #[must_use]
    pub const fn max_stale_ms(self) -> u64 {
        self.max_stale_ms
    }

    #[must_use]
    pub const fn adverse_price_delta(self) -> Decimal {
        self.adverse_price_delta
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledInstrumentConfig {
    instrument: InstrumentId,
    quantity_unit: QuantityUnit,
    model: ScheduledDepthModel,
}

impl ScheduledInstrumentConfig {
    #[must_use]
    pub const fn new(
        instrument: InstrumentId,
        quantity_unit: QuantityUnit,
        model: ScheduledDepthModel,
    ) -> Self {
        Self {
            instrument,
            quantity_unit,
            model,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleBookEvidence {
    instrument: InstrumentId,
    book: CompleteBookSnapshot,
    match_time: MatchTime,
    visible_at: MatchTime,
    matching: MatchingState,
    new_order_entry: NewOrderEntry,
}

impl VisibleBookEvidence {
    pub fn new(
        instrument: InstrumentId,
        book: CompleteBookSnapshot,
        match_time: MatchTime,
        visible_at: MatchTime,
        matching: MatchingState,
        new_order_entry: NewOrderEntry,
    ) -> Result<Self, ScheduledSimulationError> {
        if visible_at < match_time {
            return Err(ScheduledSimulationError::VisibilityBeforeMatchTime);
        }
        Ok(Self {
            instrument,
            book,
            match_time,
            visible_at,
            matching,
            new_order_entry,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuctionMatchEvidence {
    instrument: InstrumentId,
    clearing_price: Price,
    match_time: MatchTime,
    visible_at: MatchTime,
    run_event_ordinal: u64,
}

impl AuctionMatchEvidence {
    pub fn new(
        instrument: InstrumentId,
        clearing_price: Price,
        match_time: MatchTime,
        visible_at: MatchTime,
        run_event_ordinal: u64,
        matching: MatchingState,
    ) -> Result<Self, ScheduledSimulationError> {
        if visible_at < match_time {
            return Err(ScheduledSimulationError::VisibilityBeforeMatchTime);
        }
        if matching != MatchingState::Enabled(market_types::MatchingMethod::CallAuction) {
            return Err(ScheduledSimulationError::InvalidAuctionMatchEvidence);
        }
        Ok(Self {
            instrument,
            clearing_price,
            match_time,
            visible_at,
            run_event_ordinal,
        })
    }

    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn visible_at(&self) -> MatchTime {
        self.visible_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledSubmissionContext {
    origin_identity: [u8; 32],
    decision_time: MatchTime,
}

impl ScheduledSubmissionContext {
    #[must_use]
    pub const fn new(origin_identity: [u8; 32], decision_time: MatchTime) -> Self {
        Self {
            origin_identity,
            decision_time,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledOrderStatus {
    Scheduled,
    Active,
    PartiallyFilled,
    MatchAttempted,
    Filled,
    Failed(ExecutionFailureReason),
    Expired,
    Replaced,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledOrder {
    id: OrderId,
    request: ScheduledOrderRequest,
    acceptance_sequence: u64,
    filled_value: u64,
    status: ScheduledOrderStatus,
}

impl ScheduledOrder {
    #[must_use]
    pub const fn id(&self) -> OrderId {
        self.id
    }

    #[must_use]
    pub const fn request(&self) -> &ScheduledOrderRequest {
        &self.request
    }

    #[must_use]
    pub const fn acceptance_sequence(&self) -> u64 {
        self.acceptance_sequence
    }

    #[must_use]
    pub const fn filled_value(&self) -> u64 {
        self.filled_value
    }

    #[must_use]
    pub const fn status(&self) -> ScheduledOrderStatus {
        self.status
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledSubmission {
    order_id: OrderId,
    replaced: Option<OrderFeedback>,
}

impl ScheduledSubmission {
    #[must_use]
    pub const fn order_id(&self) -> OrderId {
        self.order_id
    }

    #[must_use]
    pub const fn replaced(&self) -> Option<&OrderFeedback> {
        self.replaced.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledActivation {
    order_id: OrderId,
    status: ScheduledOrderStatus,
    feedback: OrderFeedback,
    execution_fills: Box<[ExecutionFillFeedback]>,
}

impl ScheduledActivation {
    #[must_use]
    pub const fn order_id(&self) -> OrderId {
        self.order_id
    }

    #[must_use]
    pub const fn status(&self) -> ScheduledOrderStatus {
        self.status
    }

    #[must_use]
    pub const fn feedback(&self) -> &OrderFeedback {
        &self.feedback
    }

    #[must_use]
    pub const fn execution_fills(&self) -> &[ExecutionFillFeedback] {
        &self.execution_fills
    }
}

#[derive(Debug, Clone)]
struct VisibleBookState {
    evidence: VisibleBookEvidence,
    consumed_bids: [u64; BOOK_DEPTH],
    consumed_asks: [u64; BOOK_DEPTH],
}

#[derive(Debug)]
pub struct ScheduledDepthSimulator {
    configs: BTreeMap<InstrumentId, (QuantityUnit, ScheduledDepthModel)>,
    books: BTreeMap<InstrumentId, VisibleBookState>,
    orders: Vec<ScheduledOrder>,
    client_orders: BTreeMap<ClientOrderId, OrderId>,
    fills: Vec<FillRecord>,
    execution_fills: Vec<ExecutionFillFeedback>,
    next_acceptance_sequence: u64,
}

impl ScheduledDepthSimulator {
    pub fn new(
        configs: impl IntoIterator<Item = ScheduledInstrumentConfig>,
    ) -> Result<Self, ScheduledSimulationError> {
        let mut indexed = BTreeMap::new();
        for config in configs {
            let instrument = config.instrument;
            if indexed
                .insert(instrument.clone(), (config.quantity_unit, config.model))
                .is_some()
            {
                return Err(ScheduledSimulationError::DuplicateInstrument(instrument));
            }
        }
        if indexed.is_empty() {
            return Err(ScheduledSimulationError::EmptyUniverse);
        }
        Ok(Self {
            configs: indexed,
            books: BTreeMap::new(),
            orders: Vec::new(),
            client_orders: BTreeMap::new(),
            fills: Vec::new(),
            execution_fills: Vec::new(),
            next_acceptance_sequence: 1,
        })
    }

    pub fn publish_visible_book(
        &mut self,
        evidence: VisibleBookEvidence,
    ) -> Result<(), ScheduledSimulationError> {
        let Some((quantity_unit, _)) = self.configs.get(&evidence.instrument) else {
            return Err(ScheduledSimulationError::InstrumentOutsideUniverse(
                evidence.instrument,
            ));
        };
        if let Some(actual) = evidence.book.quantity_unit()
            && actual != *quantity_unit
        {
            return Err(ScheduledSimulationError::QuantityUnitMismatch {
                expected: *quantity_unit,
                actual,
            });
        }
        if self
            .books
            .get(&evidence.instrument)
            .is_some_and(|previous| {
                evidence.visible_at < previous.evidence.visible_at
                    || evidence.match_time < previous.evidence.match_time
            })
        {
            return Err(ScheduledSimulationError::RegressingBookTime);
        }
        self.books.insert(
            evidence.instrument.clone(),
            VisibleBookState {
                evidence,
                consumed_bids: [0; BOOK_DEPTH],
                consumed_asks: [0; BOOK_DEPTH],
            },
        );
        Ok(())
    }

    pub fn submit(
        &mut self,
        strategy_id: &str,
        context: ScheduledSubmissionContext,
        output_sequence: u32,
        request: ScheduledOrderRequest,
    ) -> Result<ScheduledSubmission, ScheduledSimulationError> {
        if request.activate_at() < context.decision_time {
            return Err(ScheduledSimulationError::ActivationBeforeDecision);
        }
        let Some((quantity_unit, _)) = self.configs.get(request.intent().instrument()) else {
            return Err(ScheduledSimulationError::InstrumentOutsideUniverse(
                request.intent().instrument().clone(),
            ));
        };
        if request.intent().quantity().unit() != *quantity_unit {
            return Err(ScheduledSimulationError::QuantityUnitMismatch {
                expected: *quantity_unit,
                actual: request.intent().quantity().unit(),
            });
        }

        let replaced = if let Some(previous_id) = self.client_orders.get(request.client_order_id())
        {
            let previous = self
                .orders
                .iter_mut()
                .find(|order| order.id == *previous_id)
                .expect("client order index references an existing order");
            if previous.status != ScheduledOrderStatus::Scheduled {
                return Err(ScheduledSimulationError::ClientOrderAlreadyTerminal);
            }
            previous.status = ScheduledOrderStatus::Replaced;
            Some(OrderFeedback::Cancelled {
                order_id: previous.id,
                reason: CancellationReason::Replaced,
            })
        } else {
            None
        };

        let canonical = request
            .to_canonical_bytes()
            .map_err(|error| ScheduledSimulationError::Canonical(error.to_string()))?;
        let mut identity = Vec::new();
        identity.extend_from_slice(b"OSSOR");
        identity.extend_from_slice(strategy_id.as_bytes());
        identity.extend_from_slice(&context.origin_identity);
        identity.extend_from_slice(&context.decision_time.as_unix_microseconds().to_be_bytes());
        identity.extend_from_slice(&output_sequence.to_be_bytes());
        identity.extend_from_slice(&canonical);
        let order_id = OrderId::from_bytes(*blake3::hash(&identity).as_bytes());
        let acceptance_sequence = self.next_acceptance_sequence;
        self.next_acceptance_sequence = acceptance_sequence
            .checked_add(1)
            .ok_or(ScheduledSimulationError::SequenceOverflow)?;
        self.client_orders
            .insert(request.client_order_id().clone(), order_id);
        self.orders.push(ScheduledOrder {
            id: order_id,
            request,
            acceptance_sequence,
            filled_value: 0,
            status: ScheduledOrderStatus::Scheduled,
        });
        Ok(ScheduledSubmission { order_id, replaced })
    }

    pub fn cancel_before_activation(
        &mut self,
        client_order_id: &ClientOrderId,
    ) -> Result<OrderFeedback, ScheduledSimulationError> {
        let order_id = self
            .client_orders
            .get(client_order_id)
            .copied()
            .ok_or(ScheduledSimulationError::UnknownClientOrder)?;
        let order = self.order_mut(order_id)?;
        if order.status != ScheduledOrderStatus::Scheduled {
            return Err(ScheduledSimulationError::OrderAlreadyTerminal);
        }
        order.status = ScheduledOrderStatus::Cancelled;
        Ok(OrderFeedback::Cancelled {
            order_id,
            reason: CancellationReason::StrategyCancelled,
        })
    }

    pub fn expire(
        &mut self,
        order_id: OrderId,
        at: MatchTime,
    ) -> Result<OrderFeedback, ScheduledSimulationError> {
        let order = self.order_mut(order_id)?;
        if !matches!(
            order.status,
            ScheduledOrderStatus::Scheduled
                | ScheduledOrderStatus::Active
                | ScheduledOrderStatus::PartiallyFilled
                | ScheduledOrderStatus::MatchAttempted
        ) {
            return Err(ScheduledSimulationError::OrderAlreadyTerminal);
        }
        if order.request.expire_at() != Some(at) {
            return Err(ScheduledSimulationError::ExpiryTimeMismatch);
        }
        order.status = ScheduledOrderStatus::Expired;
        Ok(OrderFeedback::Cancelled {
            order_id,
            reason: CancellationReason::Expired,
        })
    }

    pub fn activate(
        &mut self,
        order_id: OrderId,
        control_sequence: u64,
        at: MatchTime,
    ) -> Result<ScheduledActivation, ScheduledSimulationError> {
        let order_index = self.order_index(order_id)?;
        if self.orders[order_index].status != ScheduledOrderStatus::Scheduled {
            return Err(ScheduledSimulationError::OrderAlreadyTerminal);
        }
        if self.orders[order_index].request.activate_at() != at {
            return Err(ScheduledSimulationError::ActivationTimeMismatch);
        }
        let acceptance_sequence = self.orders[order_index].acceptance_sequence;
        if self.orders.iter().any(|order| {
            order.status == ScheduledOrderStatus::Scheduled
                && order.request.activate_at() == at
                && order.acceptance_sequence < acceptance_sequence
        }) {
            return Err(ScheduledSimulationError::ActivationOutOfOrder);
        }

        if matches!(
            self.orders[order_index].request.execution_policy(),
            ScheduledExecutionPolicy::VisibleDepthUntilExpiryV1
                | ScheduledExecutionPolicy::AuctionCrossAtFirstMatchV1
        ) {
            self.orders[order_index].status = ScheduledOrderStatus::Active;
            return Ok(ScheduledActivation {
                order_id,
                status: ScheduledOrderStatus::Active,
                feedback: OrderFeedback::Activated { order_id },
                execution_fills: Box::new([]),
            });
        }

        let request = self.orders[order_index].request.clone();
        let instrument = request.intent().instrument().clone();
        let (_, model) = self
            .configs
            .get(&instrument)
            .copied()
            .expect("accepted order instrument remains configured");
        let book_state = self
            .books
            .get_mut(&instrument)
            .filter(|state| state.evidence.visible_at <= at);
        let Some(book_state) = book_state else {
            return Ok(self.fail_activation(
                order_index,
                ExecutionFailureReason::MissingVisibleDepth,
                None,
            ));
        };

        let max_stale_micros = model
            .max_stale_ms
            .checked_mul(1_000)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(ScheduledSimulationError::StalenessOverflow)?;
        let age = at
            .as_unix_microseconds()
            .checked_sub(book_state.evidence.match_time.as_unix_microseconds())
            .ok_or(ScheduledSimulationError::StalenessOverflow)?;
        if age < 0 || age > max_stale_micros {
            return Ok(self.fail_activation(
                order_index,
                ExecutionFailureReason::StaleVisibleDepth,
                None,
            ));
        }
        if !matches!(book_state.evidence.matching, MatchingState::Enabled(_)) {
            return Ok(self.fail_activation(
                order_index,
                ExecutionFailureReason::MatchingDisabled,
                None,
            ));
        }
        if !entry_allowed(
            book_state.evidence.new_order_entry,
            request.intent().order_type(),
        ) {
            return Ok(self.fail_activation(
                order_index,
                ExecutionFailureReason::NewOrderEntryBlocked,
                None,
            ));
        }

        let consumed = match request.intent().side() {
            strategy_api::OrderSide::Buy => &mut book_state.consumed_asks,
            strategy_api::OrderSide::Sell => &mut book_state.consumed_bids,
        };
        let sweep = sweep_marketable_depth_with_consumed(
            &book_state.evidence.book,
            request.intent().side(),
            request.intent().quantity(),
            model.depth_levels,
            request.intent().order_type(),
            model.adverse_price_delta,
            consumed,
        )?;
        let mut cumulative = 0_u64;
        let mut execution_fills = Vec::with_capacity(sweep.fills().len());
        for level_fill in sweep.fills() {
            let index = usize::from(level_fill.level_index() - 1);
            consumed[index] = consumed[index]
                .checked_add(level_fill.quantity().value())
                .ok_or(ScheduledSimulationError::QuantityOverflow)?;
            cumulative = cumulative
                .checked_add(level_fill.quantity().value())
                .ok_or(ScheduledSimulationError::QuantityOverflow)?;
            let cumulative_quantity = Quantity::new(cumulative, request.intent().quantity().unit())
                .map_err(|_| ScheduledSimulationError::InvalidQuantity)?;
            let remaining_value = request.intent().quantity().value() - cumulative;
            let remaining = Quantity::new(remaining_value, request.intent().quantity().unit()).ok();
            let fill_id = fill_id(order_id, control_sequence, level_fill.level_index());
            self.fills.push(FillRecord::from_control(
                order_id,
                control_sequence,
                at,
                request.intent().side(),
                level_fill.price(),
                level_fill.quantity(),
            ));
            let feedback = ExecutionFillFeedback::new(
                fill_id,
                order_id,
                request.client_order_id().clone(),
                request.batch_id().cloned(),
                instrument.clone(),
                request.activate_at(),
                at,
                request.intent().side(),
                level_fill.level_index(),
                level_fill.price(),
                level_fill.quantity(),
                cumulative_quantity,
                remaining,
            )
            .map_err(|error| ScheduledSimulationError::FillFeedback(error.to_string()))?;
            self.execution_fills.push(feedback.clone());
            execution_fills.push(feedback);
        }

        self.orders[order_index].filled_value = cumulative;
        if sweep.is_complete() {
            self.orders[order_index].status = ScheduledOrderStatus::Filled;
            Ok(ScheduledActivation {
                order_id,
                status: ScheduledOrderStatus::Filled,
                feedback: OrderFeedback::Filled {
                    order_id,
                    filled: request.intent().quantity(),
                },
                execution_fills: execution_fills.into_boxed_slice(),
            })
        } else {
            let reason = if cumulative == 0
                && matches!(
                    request.intent().order_type(),
                    strategy_api::OrderType::Limit { .. }
                ) {
                ExecutionFailureReason::PriceNotMarketable
            } else {
                ExecutionFailureReason::InsufficientVisibleDepth
            };
            Ok(self.fail_activation(
                order_index,
                reason,
                Some(execution_fills.into_boxed_slice()),
            ))
        }
    }

    pub fn evaluate_active(
        &mut self,
        instrument: &InstrumentId,
        control_sequence: u64,
        at: MatchTime,
    ) -> Result<Vec<ScheduledActivation>, ScheduledSimulationError> {
        let mut indices = self
            .orders
            .iter()
            .enumerate()
            .filter(|(_, order)| {
                order.status == ScheduledOrderStatus::Active
                    && order.request.intent().instrument() == instrument
                    && order.request.execution_policy()
                        == ScheduledExecutionPolicy::VisibleDepthUntilExpiryV1
            })
            .map(|(index, order)| (order.acceptance_sequence, order.id, index))
            .collect::<Vec<_>>();
        indices.sort_by_key(|(sequence, order_id, _)| (*sequence, *order_id));
        let mut results = Vec::new();
        for (_, _, index) in indices {
            if let Some(result) = self.execute_passive_order(index, control_sequence, at)? {
                results.push(result);
            }
        }
        Ok(results)
    }

    pub fn evaluate_auction_match(
        &mut self,
        evidence: &AuctionMatchEvidence,
    ) -> Result<Vec<ScheduledActivation>, ScheduledSimulationError> {
        if !self.configs.contains_key(&evidence.instrument) {
            return Err(ScheduledSimulationError::InstrumentOutsideUniverse(
                evidence.instrument.clone(),
            ));
        }
        let mut indices = self
            .orders
            .iter()
            .enumerate()
            .filter(|(_, order)| {
                order.status == ScheduledOrderStatus::Active
                    && order.request.intent().instrument() == &evidence.instrument
                    && order.request.execution_policy()
                        == ScheduledExecutionPolicy::AuctionCrossAtFirstMatchV1
                    && order.request.activate_at() <= evidence.match_time
            })
            .map(|(index, order)| (order.acceptance_sequence, order.id, index))
            .collect::<Vec<_>>();
        indices.sort_by_key(|(sequence, order_id, _)| (*sequence, *order_id));

        let mut results = Vec::with_capacity(indices.len());
        for (_, _, index) in indices {
            let request = self.orders[index].request.clone();
            let order_id = self.orders[index].id;
            let crosses = match request.intent().order_type() {
                OrderType::Limit { limit_price } => match request.intent().side() {
                    OrderSide::Buy => evidence.clearing_price < limit_price,
                    OrderSide::Sell => evidence.clearing_price > limit_price,
                },
                OrderType::Market => false,
            };
            if !crosses {
                self.orders[index].status = ScheduledOrderStatus::MatchAttempted;
                results.push(ScheduledActivation {
                    order_id,
                    status: ScheduledOrderStatus::MatchAttempted,
                    feedback: OrderFeedback::MatchAttempted {
                        order_id,
                        filled: None,
                        remaining: request.intent().quantity(),
                    },
                    execution_fills: Box::new([]),
                });
                continue;
            }

            let quantity = request.intent().quantity();
            let fill_id = market_fill_id(order_id, evidence.run_event_ordinal, 1);
            self.fills.push(FillRecord::from_market_event(
                order_id,
                evidence.run_event_ordinal,
                evidence.match_time,
                request.intent().side(),
                evidence.clearing_price,
                quantity,
            ));
            let fill = ExecutionFillFeedback::new(
                fill_id,
                order_id,
                request.client_order_id().clone(),
                request.batch_id().cloned(),
                evidence.instrument.clone(),
                request.activate_at(),
                evidence.match_time,
                request.intent().side(),
                1,
                evidence.clearing_price,
                quantity,
                quantity,
                None,
            )
            .map_err(|error| ScheduledSimulationError::FillFeedback(error.to_string()))?;
            self.execution_fills.push(fill.clone());
            self.orders[index].filled_value = quantity.value();
            self.orders[index].status = ScheduledOrderStatus::Filled;
            results.push(ScheduledActivation {
                order_id,
                status: ScheduledOrderStatus::Filled,
                feedback: OrderFeedback::Filled {
                    order_id,
                    filled: quantity,
                },
                execution_fills: Box::new([fill]),
            });
        }
        Ok(results)
    }

    fn execute_passive_order(
        &mut self,
        order_index: usize,
        control_sequence: u64,
        at: MatchTime,
    ) -> Result<Option<ScheduledActivation>, ScheduledSimulationError> {
        let request = self.orders[order_index].request.clone();
        let order_id = self.orders[order_index].id;
        let instrument = request.intent().instrument().clone();
        let (_, model) = self
            .configs
            .get(&instrument)
            .copied()
            .expect("accepted order instrument remains configured");
        let Some(book_state) = self
            .books
            .get_mut(&instrument)
            .filter(|state| state.evidence.visible_at <= at)
        else {
            return Ok(None);
        };
        let max_stale_micros = model
            .max_stale_ms
            .checked_mul(1_000)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(ScheduledSimulationError::StalenessOverflow)?;
        let age = at
            .as_unix_microseconds()
            .checked_sub(book_state.evidence.match_time.as_unix_microseconds())
            .ok_or(ScheduledSimulationError::StalenessOverflow)?;
        if age < 0
            || age > max_stale_micros
            || !matches!(book_state.evidence.matching, MatchingState::Enabled(_))
        {
            return Ok(None);
        }
        let remaining_value =
            request.intent().quantity().value() - self.orders[order_index].filled_value;
        let requested = Quantity::new(remaining_value, request.intent().quantity().unit())
            .map_err(|_| ScheduledSimulationError::InvalidQuantity)?;
        let consumed = match request.intent().side() {
            strategy_api::OrderSide::Buy => &mut book_state.consumed_asks,
            strategy_api::OrderSide::Sell => &mut book_state.consumed_bids,
        };
        let sweep = sweep_marketable_depth_with_consumed(
            &book_state.evidence.book,
            request.intent().side(),
            requested,
            model.depth_levels,
            request.intent().order_type(),
            model.adverse_price_delta,
            consumed,
        )?;
        let starting_filled = self.orders[order_index].filled_value;
        let mut cumulative = starting_filled;
        let mut execution_fills = Vec::with_capacity(sweep.fills().len());
        for level_fill in sweep.fills() {
            let index = usize::from(level_fill.level_index() - 1);
            consumed[index] = consumed[index]
                .checked_add(level_fill.quantity().value())
                .ok_or(ScheduledSimulationError::QuantityOverflow)?;
            cumulative = cumulative
                .checked_add(level_fill.quantity().value())
                .ok_or(ScheduledSimulationError::QuantityOverflow)?;
            let cumulative_quantity = Quantity::new(cumulative, request.intent().quantity().unit())
                .map_err(|_| ScheduledSimulationError::InvalidQuantity)?;
            let remaining = Quantity::new(
                request.intent().quantity().value() - cumulative,
                request.intent().quantity().unit(),
            )
            .ok();
            let fill_id = fill_id(order_id, control_sequence, level_fill.level_index());
            self.fills.push(FillRecord::from_control(
                order_id,
                control_sequence,
                at,
                request.intent().side(),
                level_fill.price(),
                level_fill.quantity(),
            ));
            let feedback = ExecutionFillFeedback::new(
                fill_id,
                order_id,
                request.client_order_id().clone(),
                request.batch_id().cloned(),
                instrument.clone(),
                request.activate_at(),
                at,
                request.intent().side(),
                level_fill.level_index(),
                level_fill.price(),
                level_fill.quantity(),
                cumulative_quantity,
                remaining,
            )
            .map_err(|error| ScheduledSimulationError::FillFeedback(error.to_string()))?;
            self.execution_fills.push(feedback.clone());
            execution_fills.push(feedback);
        }
        self.orders[order_index].filled_value = cumulative;
        let filled_now = cumulative - starting_filled;
        let remaining = Quantity::new(
            request.intent().quantity().value() - cumulative,
            request.intent().quantity().unit(),
        )
        .ok();
        let (status, feedback) = match remaining {
            None => (
                ScheduledOrderStatus::Filled,
                OrderFeedback::Filled {
                    order_id,
                    filled: request.intent().quantity(),
                },
            ),
            Some(remaining) if filled_now > 0 => (
                ScheduledOrderStatus::PartiallyFilled,
                OrderFeedback::PartiallyFilled {
                    order_id,
                    filled: Quantity::new(filled_now, request.intent().quantity().unit())
                        .expect("positive passive fill is a valid quantity"),
                    remaining,
                },
            ),
            Some(remaining) => (
                ScheduledOrderStatus::MatchAttempted,
                OrderFeedback::MatchAttempted {
                    order_id,
                    filled: None,
                    remaining,
                },
            ),
        };
        self.orders[order_index].status = status;
        Ok(Some(ScheduledActivation {
            order_id,
            status,
            feedback,
            execution_fills: execution_fills.into_boxed_slice(),
        }))
    }

    #[must_use]
    pub fn orders(&self) -> &[ScheduledOrder] {
        &self.orders
    }

    #[must_use]
    pub fn fills(&self) -> &[FillRecord] {
        &self.fills
    }

    #[must_use]
    pub fn execution_fills(&self) -> &[ExecutionFillFeedback] {
        &self.execution_fills
    }

    fn fail_activation(
        &mut self,
        order_index: usize,
        reason: ExecutionFailureReason,
        execution_fills: Option<Box<[ExecutionFillFeedback]>>,
    ) -> ScheduledActivation {
        let order = &mut self.orders[order_index];
        order.status = ScheduledOrderStatus::Failed(reason);
        let filled =
            Quantity::new(order.filled_value, order.request.intent().quantity().unit()).ok();
        let remaining = Quantity::new(
            order.request.intent().quantity().value() - order.filled_value,
            order.request.intent().quantity().unit(),
        )
        .expect("failed order always has positive remaining quantity");
        ScheduledActivation {
            order_id: order.id,
            status: order.status,
            feedback: OrderFeedback::ExecutionFailed {
                order_id: order.id,
                reason,
                filled,
                remaining,
            },
            execution_fills: execution_fills.unwrap_or_default(),
        }
    }

    fn order_index(&self, order_id: OrderId) -> Result<usize, ScheduledSimulationError> {
        self.orders
            .iter()
            .position(|order| order.id == order_id)
            .ok_or(ScheduledSimulationError::UnknownOrder)
    }

    fn order_mut(
        &mut self,
        order_id: OrderId,
    ) -> Result<&mut ScheduledOrder, ScheduledSimulationError> {
        let index = self.order_index(order_id)?;
        Ok(&mut self.orders[index])
    }
}

fn entry_allowed(entry: NewOrderEntry, order_type: strategy_api::OrderType) -> bool {
    match entry {
        NewOrderEntry::Allowed => true,
        NewOrderEntry::Restricted(OrderRestrictionReason::PreOpenLimitOrdersOnly)
        | NewOrderEntry::Restricted(OrderRestrictionReason::IndicativeMarket) => {
            matches!(order_type, strategy_api::OrderType::Limit { .. })
        }
        NewOrderEntry::Blocked(_) | NewOrderEntry::Unknown => false,
    }
}

fn fill_id(order_id: OrderId, control_sequence: u64, level_index: u8) -> FillId {
    let mut identity = Vec::new();
    identity.extend_from_slice(b"OSSF");
    identity.extend_from_slice(order_id.as_bytes());
    identity.extend_from_slice(&control_sequence.to_be_bytes());
    identity.push(level_index);
    FillId::from_bytes(*blake3::hash(&identity).as_bytes())
}

fn market_fill_id(order_id: OrderId, run_event_ordinal: u64, level_index: u8) -> FillId {
    let mut identity = Vec::new();
    identity.extend_from_slice(b"OSSFM");
    identity.extend_from_slice(order_id.as_bytes());
    identity.extend_from_slice(&run_event_ordinal.to_be_bytes());
    identity.push(level_index);
    FillId::from_bytes(*blake3::hash(&identity).as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduledSimulationError {
    EmptyUniverse,
    DuplicateInstrument(InstrumentId),
    InstrumentOutsideUniverse(InstrumentId),
    InvalidDepthLevels(usize),
    ZeroStalenessWindow,
    InvalidAdversePriceDelta,
    VisibilityBeforeMatchTime,
    InvalidAuctionMatchEvidence,
    RegressingBookTime,
    ActivationBeforeDecision,
    ActivationTimeMismatch,
    ActivationOutOfOrder,
    ExpiryTimeMismatch,
    UnsupportedExecutionPolicy,
    QuantityUnitMismatch {
        expected: QuantityUnit,
        actual: QuantityUnit,
    },
    UnknownClientOrder,
    UnknownOrder,
    ClientOrderAlreadyTerminal,
    OrderAlreadyTerminal,
    SequenceOverflow,
    StalenessOverflow,
    QuantityOverflow,
    InvalidQuantity,
    Canonical(String),
    FillFeedback(String),
    DepthSweep(DepthSweepError),
}

impl fmt::Display for ScheduledSimulationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUniverse => formatter.write_str("scheduled execution universe is empty"),
            Self::DuplicateInstrument(instrument) => {
                write!(formatter, "duplicate scheduled instrument {instrument:?}")
            }
            Self::InstrumentOutsideUniverse(instrument) => {
                write!(
                    formatter,
                    "instrument is outside scheduled universe: {instrument:?}"
                )
            }
            Self::InvalidDepthLevels(levels) => {
                write!(
                    formatter,
                    "depth levels must be between 1 and {BOOK_DEPTH}, got {levels}"
                )
            }
            Self::ZeroStalenessWindow => {
                formatter.write_str("scheduled execution staleness window must be positive")
            }
            Self::InvalidAdversePriceDelta => {
                formatter.write_str("scheduled adverse price delta must be non-negative")
            }
            Self::VisibilityBeforeMatchTime => {
                formatter.write_str("book visibility time is earlier than match time")
            }
            Self::InvalidAuctionMatchEvidence => {
                formatter.write_str("auction match evidence requires enabled call-auction matching")
            }
            Self::RegressingBookTime => formatter.write_str("visible book time regressed"),
            Self::ActivationBeforeDecision => {
                formatter.write_str("order activation is earlier than strategy decision")
            }
            Self::ActivationTimeMismatch => formatter.write_str("order activation time mismatch"),
            Self::ActivationOutOfOrder => formatter.write_str(
                "scheduled orders at the same activation time must follow acceptance sequence",
            ),
            Self::ExpiryTimeMismatch => formatter.write_str("order expiry time mismatch"),
            Self::UnsupportedExecutionPolicy => {
                formatter.write_str("scheduled execution policy is unsupported")
            }
            Self::QuantityUnitMismatch { expected, actual } => write!(
                formatter,
                "scheduled quantity unit mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::UnknownClientOrder => formatter.write_str("unknown client order identifier"),
            Self::UnknownOrder => formatter.write_str("unknown scheduled order"),
            Self::ClientOrderAlreadyTerminal => {
                formatter.write_str("client order identifier belongs to a terminal order")
            }
            Self::OrderAlreadyTerminal => formatter.write_str("scheduled order is terminal"),
            Self::SequenceOverflow => formatter.write_str("scheduled acceptance sequence overflow"),
            Self::StalenessOverflow => {
                formatter.write_str("scheduled staleness arithmetic overflow")
            }
            Self::QuantityOverflow => formatter.write_str("scheduled fill quantity overflow"),
            Self::InvalidQuantity => formatter.write_str("scheduled fill quantity is invalid"),
            Self::Canonical(message) => {
                write!(formatter, "scheduled canonical encoding failed: {message}")
            }
            Self::FillFeedback(message) => {
                write!(formatter, "scheduled fill feedback failed: {message}")
            }
            Self::DepthSweep(error) => write!(formatter, "scheduled depth sweep failed: {error}"),
        }
    }
}

impl Error for ScheduledSimulationError {}

impl From<DepthSweepError> for ScheduledSimulationError {
    fn from(error: DepthSweepError) -> Self {
        Self::DepthSweep(error)
    }
}

#[cfg(test)]
mod tests {
    use market_types::{
        BookLevel, BookSide, BookSideKind, MarketId, MatchingMethod, Price, Symbol,
    };
    use strategy_api::{
        ClientOrderId, OrderIntent, OrderSide, OrderType, ScheduledExecutionPolicy,
    };

    use super::*;

    fn instrument() -> InstrumentId {
        InstrumentId::new(MarketId::Taifex, Symbol::new("TXF").unwrap())
    }

    fn quantity(value: u64) -> Quantity {
        Quantity::new(value, QuantityUnit::Contract).unwrap()
    }

    fn level(price: &str, value: u64) -> BookLevel {
        BookLevel::new(Price::parse(price).unwrap(), quantity(value))
    }

    fn book() -> CompleteBookSnapshot {
        CompleteBookSnapshot::new(
            BookSide::new(
                BookSideKind::Bid,
                vec![level("100", 2), level("99", 3), level("98", 5)],
            )
            .unwrap(),
            BookSide::new(
                BookSideKind::Ask,
                vec![level("101", 1), level("102", 4), level("103", 6)],
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn simulator(depth_levels: usize, max_stale_ms: u64) -> ScheduledDepthSimulator {
        let model = ScheduledDepthModel::new(depth_levels, max_stale_ms, Decimal::ZERO).unwrap();
        ScheduledDepthSimulator::new([ScheduledInstrumentConfig::new(
            instrument(),
            QuantityUnit::Contract,
            model,
        )])
        .unwrap()
    }

    fn request(
        client_id: &str,
        quantity_value: u64,
        activation: MatchTime,
    ) -> ScheduledOrderRequest {
        ScheduledOrderRequest::new(
            ClientOrderId::new(client_id).unwrap(),
            None,
            activation,
            None,
            OrderIntent::new(
                instrument(),
                OrderSide::Buy,
                quantity(quantity_value),
                OrderType::Market,
            ),
            ScheduledExecutionPolicy::VisibleDepthAtActivationV1,
        )
        .unwrap()
    }

    fn passive_request(
        client_id: &str,
        quantity_value: u64,
        activation: MatchTime,
        expiry: MatchTime,
    ) -> ScheduledOrderRequest {
        ScheduledOrderRequest::new(
            ClientOrderId::new(client_id).unwrap(),
            None,
            activation,
            Some(expiry),
            OrderIntent::new(
                instrument(),
                OrderSide::Buy,
                quantity(quantity_value),
                OrderType::Limit {
                    limit_price: Price::parse("102").unwrap(),
                },
            ),
            ScheduledExecutionPolicy::VisibleDepthUntilExpiryV1,
        )
        .unwrap()
    }

    fn auction_request(
        client_id: &str,
        side: OrderSide,
        limit_price: &str,
        activation: MatchTime,
        expiry: MatchTime,
    ) -> ScheduledOrderRequest {
        ScheduledOrderRequest::new(
            ClientOrderId::new(client_id).unwrap(),
            None,
            activation,
            Some(expiry),
            OrderIntent::new(
                instrument(),
                side,
                quantity(5),
                OrderType::Limit {
                    limit_price: Price::parse(limit_price).unwrap(),
                },
            ),
            ScheduledExecutionPolicy::AuctionCrossAtFirstMatchV1,
        )
        .unwrap()
    }

    fn auction_evidence(price: &str, match_micros: i64, ordinal: u64) -> AuctionMatchEvidence {
        AuctionMatchEvidence::new(
            instrument(),
            Price::parse(price).unwrap(),
            MatchTime::from_unix_microseconds(match_micros),
            MatchTime::from_unix_microseconds(match_micros + 200),
            ordinal,
            MatchingState::Enabled(MatchingMethod::CallAuction),
        )
        .unwrap()
    }

    fn publish(simulator: &mut ScheduledDepthSimulator, match_micros: i64, visible_micros: i64) {
        simulator
            .publish_visible_book(
                VisibleBookEvidence::new(
                    instrument(),
                    book(),
                    MatchTime::from_unix_microseconds(match_micros),
                    MatchTime::from_unix_microseconds(visible_micros),
                    MatchingState::Enabled(MatchingMethod::Continuous),
                    NewOrderEntry::Allowed,
                )
                .unwrap(),
            )
            .unwrap();
    }

    fn submit(
        simulator: &mut ScheduledDepthSimulator,
        request: ScheduledOrderRequest,
        output_sequence: u32,
    ) -> OrderId {
        simulator
            .submit(
                "test-strategy",
                ScheduledSubmissionContext::new([7; 32], MatchTime::from_unix_microseconds(1_000)),
                output_sequence,
                request,
            )
            .unwrap()
            .order_id()
    }

    #[test]
    fn publishing_a_new_book_replaces_expired_depth_history() {
        let mut simulator = simulator(5, 1_000);

        publish(&mut simulator, 1_000, 1_100);
        publish(&mut simulator, 1_200, 1_300);

        assert_eq!(simulator.books.len(), 1);
        assert_eq!(
            simulator.books[&instrument()].evidence.visible_at,
            MatchTime::from_unix_microseconds(1_300)
        );
    }

    #[test]
    fn activation_sweeps_visible_levels_and_traces_control_fills() {
        let activation = MatchTime::from_unix_microseconds(1_100);
        let mut simulator = simulator(5, 1);
        publish(&mut simulator, 900, 1_000);
        let order_id = submit(&mut simulator, request("entry-1", 5, activation), 1);

        let result = simulator.activate(order_id, 42, activation).unwrap();

        assert_eq!(result.status(), ScheduledOrderStatus::Filled);
        assert_eq!(result.execution_fills().len(), 2);
        assert_eq!(result.execution_fills()[0].level_index(), 1);
        assert_eq!(result.execution_fills()[1].level_index(), 2);
        assert_eq!(simulator.fills().len(), 2);
        assert_eq!(simulator.execution_fills(), result.execution_fills());
        assert_eq!(simulator.fills()[0].control_sequence(), Some(42));
    }

    #[test]
    fn orders_share_each_snapshot_displayed_quantity() {
        let activation = MatchTime::from_unix_microseconds(1_100);
        let mut simulator = simulator(2, 1);
        publish(&mut simulator, 900, 1_000);
        let first = submit(&mut simulator, request("entry-1", 4, activation), 1);
        let second = submit(&mut simulator, request("entry-2", 4, activation), 2);

        assert_eq!(
            simulator.activate(first, 1, activation).unwrap().status(),
            ScheduledOrderStatus::Filled
        );
        let second_result = simulator.activate(second, 2, activation).unwrap();
        assert_eq!(
            second_result.status(),
            ScheduledOrderStatus::Failed(ExecutionFailureReason::InsufficientVisibleDepth)
        );
        assert_eq!(second_result.execution_fills().len(), 1);
        assert_eq!(second_result.execution_fills()[0].quantity().value(), 1);
        assert_eq!(simulator.fills().len(), 3);
    }

    #[test]
    fn same_time_activation_must_follow_acceptance_sequence() {
        let activation = MatchTime::from_unix_microseconds(1_100);
        let mut simulator = simulator(2, 1);
        publish(&mut simulator, 900, 1_000);
        let first = submit(&mut simulator, request("entry-1", 1, activation), 1);
        let second = submit(&mut simulator, request("entry-2", 1, activation), 2);

        assert_eq!(
            simulator.activate(second, 1, activation).unwrap_err(),
            ScheduledSimulationError::ActivationOutOfOrder
        );
        simulator.activate(first, 2, activation).unwrap();
        simulator.activate(second, 3, activation).unwrap();
    }

    #[test]
    fn stale_evidence_fails_without_creating_fill() {
        let activation = MatchTime::from_unix_microseconds(5_000);
        let mut simulator = simulator(5, 1);
        publish(&mut simulator, 0, 1_000);
        let order_id = submit(&mut simulator, request("entry-1", 1, activation), 1);

        let result = simulator.activate(order_id, 1, activation).unwrap();

        assert_eq!(
            result.status(),
            ScheduledOrderStatus::Failed(ExecutionFailureReason::StaleVisibleDepth)
        );
        assert!(result.execution_fills().is_empty());
        assert!(simulator.fills().is_empty());
    }

    #[test]
    fn client_id_can_replace_only_a_pending_order_and_expiry_is_exact() {
        let activation = MatchTime::from_unix_microseconds(2_000);
        let expiry = MatchTime::from_unix_microseconds(3_000);
        let mut simulator = simulator(5, 1);
        let first = ScheduledOrderRequest::new(
            ClientOrderId::new("entry-1").unwrap(),
            None,
            activation,
            Some(expiry),
            OrderIntent::new(instrument(), OrderSide::Buy, quantity(1), OrderType::Market),
            ScheduledExecutionPolicy::VisibleDepthAtActivationV1,
        )
        .unwrap();
        let first_id = submit(&mut simulator, first.clone(), 1);
        let replacement = simulator
            .submit(
                "test-strategy",
                ScheduledSubmissionContext::new([8; 32], MatchTime::from_unix_microseconds(1_000)),
                2,
                first,
            )
            .unwrap();
        assert!(matches!(
            replacement.replaced(),
            Some(OrderFeedback::Cancelled {
                order_id,
                reason: CancellationReason::Replaced
            }) if *order_id == first_id
        ));
        assert_eq!(
            simulator.orders()[0].status(),
            ScheduledOrderStatus::Replaced
        );

        let replacement_id = replacement.order_id();
        assert_eq!(
            simulator.expire(replacement_id, expiry).unwrap(),
            OrderFeedback::Cancelled {
                order_id: replacement_id,
                reason: CancellationReason::Expired,
            }
        );
        assert_eq!(
            simulator.orders()[1].status(),
            ScheduledOrderStatus::Expired
        );
    }

    #[test]
    fn passive_limit_waits_for_matching_snapshot_after_activation() {
        let activation = MatchTime::from_unix_microseconds(1_100);
        let expiry = MatchTime::from_unix_microseconds(2_000);
        let mut simulator = simulator(5, 2);
        let order_id = submit(
            &mut simulator,
            passive_request("stock-entry", 5, activation, expiry),
            1,
        );
        let activated = simulator.activate(order_id, 1, activation).unwrap();
        assert_eq!(activated.status(), ScheduledOrderStatus::Active);
        assert!(simulator.fills().is_empty());

        simulator
            .publish_visible_book(
                VisibleBookEvidence::new(
                    instrument(),
                    book(),
                    MatchTime::from_unix_microseconds(1_500),
                    MatchTime::from_unix_microseconds(1_700),
                    MatchingState::Enabled(MatchingMethod::CallAuction),
                    NewOrderEntry::Allowed,
                )
                .unwrap(),
            )
            .unwrap();
        let results = simulator
            .evaluate_active(&instrument(), 2, MatchTime::from_unix_microseconds(1_700))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status(), ScheduledOrderStatus::Filled);
        assert_eq!(simulator.fills().len(), 2);
    }

    #[test]
    fn passive_partial_fill_waits_for_expiry_cancellation() {
        let activation = MatchTime::from_unix_microseconds(1_100);
        let match_time = MatchTime::from_unix_microseconds(1_500);
        let expiry = MatchTime::from_unix_microseconds(2_000);
        let mut simulator = simulator(2, 2);
        let order_id = submit(
            &mut simulator,
            passive_request("stock-entry", 8, activation, expiry),
            1,
        );
        simulator.activate(order_id, 1, activation).unwrap();
        simulator
            .publish_visible_book(
                VisibleBookEvidence::new(
                    instrument(),
                    book(),
                    match_time,
                    match_time,
                    MatchingState::Enabled(MatchingMethod::CallAuction),
                    NewOrderEntry::Allowed,
                )
                .unwrap(),
            )
            .unwrap();
        let result = simulator
            .evaluate_active(&instrument(), 2, match_time)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(result.status(), ScheduledOrderStatus::PartiallyFilled);
        assert_eq!(simulator.orders()[0].filled_value(), 5);
        assert_eq!(
            simulator.expire(order_id, expiry).unwrap(),
            OrderFeedback::Cancelled {
                order_id,
                reason: CancellationReason::Expired,
            }
        );
        assert_eq!(
            simulator.orders()[0].status(),
            ScheduledOrderStatus::Expired
        );
    }

    #[test]
    fn auction_strict_cross_fills_the_entire_order_at_the_clearing_price() {
        let activation = MatchTime::from_unix_microseconds(1_100);
        let expiry = MatchTime::from_unix_microseconds(2_000);
        let mut simulator = simulator(5, 2);
        let buy = submit(
            &mut simulator,
            auction_request("auction-buy", OrderSide::Buy, "102", activation, expiry),
            1,
        );
        let sell = submit(
            &mut simulator,
            auction_request("auction-sell", OrderSide::Sell, "100", activation, expiry),
            2,
        );
        simulator.activate(buy, 1, activation).unwrap();
        simulator.activate(sell, 2, activation).unwrap();

        let results = simulator
            .evaluate_auction_match(&auction_evidence("101", 1_500, 17))
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|result| result.status() == ScheduledOrderStatus::Filled)
        );
        assert_eq!(simulator.fills().len(), 2);
        assert!(
            simulator
                .fills()
                .iter()
                .all(|fill| fill.quantity().value() == 5
                    && fill.price() == Price::parse("101").unwrap()
                    && fill.match_time() == MatchTime::from_unix_microseconds(1_500)
                    && fill.triggering_ordinal() == Some(17))
        );
    }

    #[test]
    fn auction_equal_price_is_conservatively_unfilled_and_only_attempted_once() {
        let activation = MatchTime::from_unix_microseconds(1_100);
        let expiry = MatchTime::from_unix_microseconds(2_000);
        let mut simulator = simulator(5, 2);
        let order_id = submit(
            &mut simulator,
            auction_request("auction-buy", OrderSide::Buy, "101", activation, expiry),
            1,
        );
        simulator.activate(order_id, 1, activation).unwrap();

        let equal = simulator
            .evaluate_auction_match(&auction_evidence("101", 1_500, 17))
            .unwrap();
        let later_cross = simulator
            .evaluate_auction_match(&auction_evidence("100", 1_600, 18))
            .unwrap();

        assert_eq!(equal.len(), 1);
        assert_eq!(equal[0].status(), ScheduledOrderStatus::MatchAttempted);
        assert!(later_cross.is_empty());
        assert!(simulator.fills().is_empty());
    }

    #[test]
    fn auction_policy_does_not_fill_from_visible_book_depth() {
        let activation = MatchTime::from_unix_microseconds(1_100);
        let expiry = MatchTime::from_unix_microseconds(2_000);
        let mut simulator = simulator(5, 2);
        let order_id = submit(
            &mut simulator,
            auction_request("auction-buy", OrderSide::Buy, "102", activation, expiry),
            1,
        );
        simulator.activate(order_id, 1, activation).unwrap();
        publish(&mut simulator, 1_200, 1_300);

        assert!(
            simulator
                .evaluate_active(&instrument(), 2, MatchTime::from_unix_microseconds(1_300))
                .unwrap()
                .is_empty()
        );
        assert!(simulator.fills().is_empty());
    }
}
