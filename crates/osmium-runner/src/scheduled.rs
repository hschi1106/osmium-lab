use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
};

use execution_sim::{
    AuctionMatchEvidence, MultiLedger, MultiPerformanceSummary, ScheduledDepthSimulator,
    ScheduledOrderStatus, VisibleBookEvidence,
};
use market_state::MarketState;
use market_types::{DomainEvent, MatchTime};
use replay_engine::{
    CompletedReplay, EventOccurrence, EventStream, OrderingKey, ReplayCore, ReplayPlan,
    ReplayStreamFactory,
};
use strategy_api::{
    ExecutionFillFeedback, MarketTradingContextEvaluator, OrderFeedback, OrderId,
    ScheduledExecutionPolicy, Strategy, StrategyEventContext, StrategyFeedbackContext,
    StrategyFinalizeContext, StrategyInitializationContext, StrategyOutput, StrategyOutputSink,
    StrategyTimerContext, StrategyTimerId, StrategyTimerRequest, TradingContext,
};

use crate::{
    ControlPhase, ControlTimeQueue, MultiBacktestError, MultiSessionSchedule, add_milliseconds,
    final_mark,
};

#[derive(Debug)]
pub struct CompletedScheduledMultiBacktest {
    pub replay: CompletedReplay,
    pub strategy_output: StrategyOutput,
    pub simulator: ScheduledDepthSimulator,
    pub ledger: MultiLedger,
    pub performance: MultiPerformanceSummary,
}

#[derive(Debug, Clone)]
struct BufferedObservation {
    canonical_event: Box<[u8]>,
    run_event_ordinal: u64,
    event_fingerprint: [u8; 32],
    instrument_state_version: u64,
    trading: TradingContext,
    visible_at: MatchTime,
}

#[derive(Debug, Clone)]
enum ScheduledBacktestControl {
    ReleaseObservation(Box<BufferedObservation>),
    Expire(OrderId),
    Activate(OrderId),
    AllocateVisibleDepth(market_types::InstrumentId),
    AllocateAuctionMatch(AuctionMatchEvidence),
    StrategyTimer(StrategyTimerRequest),
    DeliverFeedback {
        feedback: Box<[OrderFeedback]>,
        execution_fills: Box<[ExecutionFillFeedback]>,
        origin_identity: [u8; 32],
    },
}

struct ScheduledCoordinator<'a, S> {
    strategy: &'a mut S,
    schedule: &'a MultiSessionSchedule,
    market_data_latency_ms: u64,
    simulator: &'a mut ScheduledDepthSimulator,
    ledger: &'a mut MultiLedger,
    output: &'a mut StrategyOutput,
    controls: ControlTimeQueue<ScheduledBacktestControl>,
    timer_generations: BTreeMap<StrategyTimerId, u64>,
    visible_core: ReplayCore,
}

#[allow(clippy::too_many_arguments)]
pub fn run_scheduled_multi_backtest<S: Strategy, F: ReplayStreamFactory>(
    mut core: ReplayCore,
    mut strategy: S,
    plan: &ReplayPlan,
    factory: &mut F,
    schedule: &MultiSessionSchedule,
    market_data_latency_ms: u64,
    mut simulator: ScheduledDepthSimulator,
    mut ledger: MultiLedger,
    allow_midpoint_fallback: bool,
) -> Result<CompletedScheduledMultiBacktest, MultiBacktestError> {
    let declaration = strategy.declaration();
    let core_instruments = core
        .states()
        .map(|state| state.instrument().clone())
        .collect::<Vec<_>>();
    for (name, instruments) in [
        ("strategy", declaration.universe().to_vec()),
        ("schedule", schedule.instruments().cloned().collect()),
        ("ledger", ledger.instruments().cloned().collect()),
    ] {
        if instruments != core_instruments {
            return Err(MultiBacktestError::Schedule(format!(
                "{name} instrument universe differs from replay core"
            )));
        }
    }
    let schedule_sessions = schedule
        .segments
        .values()
        .flat_map(|segments| segments.iter().map(strategy_api::SessionSegment::kind))
        .collect::<std::collections::BTreeSet<_>>();
    if declaration.sessions() != schedule_sessions.into_iter().collect::<Vec<_>>().as_slice() {
        return Err(MultiBacktestError::Schedule(
            "strategy sessions differ from session schedule".to_owned(),
        ));
    }
    strategy
        .initialize(&StrategyInitializationContext::new(&declaration))
        .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?;
    let mut output = StrategyOutput::new(
        strategy.identity().clone(),
        strategy.canonical_params_checksum(),
    );
    let mut streams = Vec::with_capacity(plan.bindings().len());
    for binding in plan.bindings() {
        let state = core
            .state(binding.instrument())
            .ok_or(MultiBacktestError::Declaration)?;
        if state.trading_date() != binding.trading_date()
            && !schedule
                .segments(binding.instrument())
                .is_some_and(|segments| {
                    segments
                        .iter()
                        .any(|segment| segment.trading_date() == binding.trading_date())
                })
        {
            return Err(MultiBacktestError::Schedule(
                "replay binding trading date is absent from schedule".to_owned(),
            ));
        }
        streams.push(
            factory
                .open(binding)
                .map_err(|error| MultiBacktestError::Replay(error.to_string()))?,
        );
    }
    let mut heads = (0..streams.len())
        .map(|_| None)
        .collect::<Vec<Option<DomainEvent>>>();
    let visible_core = core
        .fork_unstarted()
        .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;
    let mut coordinator = ScheduledCoordinator {
        strategy: &mut strategy,
        schedule,
        market_data_latency_ms,
        simulator: &mut simulator,
        ledger: &mut ledger,
        output: &mut output,
        controls: ControlTimeQueue::new(),
        timer_generations: BTreeMap::new(),
        visible_core,
    };

    let mut pending = BinaryHeap::with_capacity(streams.len());
    for (index, stream) in streams.iter_mut().enumerate() {
        let event = stream
            .next_event()
            .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;
        if let Some(event) = event {
            let key = OrderingKey::for_event(&event)
                .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;
            heads[index] = Some(event);
            pending.push(Reverse((key, index)));
        }
    }
    while let Some(Reverse((_, selected_index))) = pending.pop() {
        let event = heads[selected_index]
            .take()
            .expect("selected merge head is present");
        coordinator.process_before(event.match_time())?;
        let commit = core
            .apply_ordered(&event)
            .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;
        coordinator.buffer_committed(&core, event, commit.occurrence().clone())?;
        coordinator.process_at(commit.occurrence().ordering_key().match_time())?;
        let next = streams[selected_index]
            .next_event()
            .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;
        if let Some(next) = next {
            let key = OrderingKey::for_event(&next)
                .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;
            heads[selected_index] = Some(next);
            pending.push(Reverse((key, selected_index)));
        }
    }
    coordinator.drain()?;
    drop(coordinator);

    let states = core.states().map(|state| state.view()).collect::<Vec<_>>();
    let marks = states
        .iter()
        .map(|state| {
            (
                state.instrument().clone(),
                final_mark(*state, allow_midpoint_fallback),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut sink = StrategyOutputSink::new();
    strategy
        .finalize(
            &StrategyFinalizeContext::new(core.clock(), states),
            &mut sink,
        )
        .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?;
    output.extend(
        sink.into_finalize_records()
            .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?,
    );
    ledger
        .reconcile()
        .map_err(|error| MultiBacktestError::Accounting(error.to_string()))?;
    let performance = ledger
        .performance(&marks)
        .map_err(|error| MultiBacktestError::Accounting(error.to_string()))?;
    let replay = core
        .complete()
        .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;
    Ok(CompletedScheduledMultiBacktest {
        replay,
        strategy_output: output,
        simulator,
        ledger,
        performance,
    })
}

impl<S: Strategy> ScheduledCoordinator<'_, S> {
    fn buffer_committed(
        &mut self,
        core: &ReplayCore,
        event: DomainEvent,
        occurrence: EventOccurrence,
    ) -> Result<(), MultiBacktestError> {
        let segment = self
            .schedule
            .segment_for(event.instrument(), event.match_time())
            .ok_or_else(|| {
                MultiBacktestError::Schedule("event is outside session schedule".to_owned())
            })?;
        let state = core
            .state(event.instrument())
            .ok_or(MultiBacktestError::Declaration)?;
        let trading = MarketTradingContextEvaluator
            .evaluate(&event, &occurrence, state.view(), segment)
            .map_err(|error| MultiBacktestError::Context(error.to_string()))?;
        let visible_at = add_milliseconds(event.match_time(), self.market_data_latency_ms)
            .map_err(|_| MultiBacktestError::Sequence)?;
        if trading.matching()
            == strategy_api::MatchingState::Enabled(market_types::MatchingMethod::CallAuction)
            && let Some(clearing_price) = auction_clearing_price(&event)?
        {
            let evidence = AuctionMatchEvidence::new(
                event.instrument().clone(),
                clearing_price,
                event.match_time(),
                visible_at,
                occurrence.run_event_ordinal(),
                trading.matching(),
            )
            .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?;
            if self.instrument_needs_auction_match(evidence.instrument(), event.match_time()) {
                self.controls
                    .schedule(
                        event.match_time(),
                        ControlPhase::FillAllocation,
                        ScheduledBacktestControl::AllocateAuctionMatch(evidence),
                    )
                    .map_err(|_| MultiBacktestError::Sequence)?;
            }
        }
        let observation = BufferedObservation {
            canonical_event: event
                .to_canonical_bytes()
                .map_err(|error| MultiBacktestError::Replay(error.to_string()))?
                .into_boxed_slice(),
            run_event_ordinal: occurrence.run_event_ordinal(),
            event_fingerprint: *occurrence.event_fingerprint().as_bytes(),
            instrument_state_version: occurrence.instrument_state_version(),
            trading,
            visible_at,
        };
        self.controls
            .schedule(
                visible_at,
                ControlPhase::ReleaseObservation,
                ScheduledBacktestControl::ReleaseObservation(Box::new(observation)),
            )
            .map_err(|_| MultiBacktestError::Sequence)?;
        Ok(())
    }

    fn process_before(&mut self, boundary: MatchTime) -> Result<(), MultiBacktestError> {
        while self.controls.next_time().is_some_and(|at| at < boundary) {
            for control in self.controls.pop_before(boundary) {
                self.process_control(control)?;
            }
        }
        Ok(())
    }

    fn process_at(&mut self, at: MatchTime) -> Result<(), MultiBacktestError> {
        while self.controls.next_time() == Some(at) {
            for control in self.controls.pop_at(at) {
                self.process_control(control)?;
            }
        }
        Ok(())
    }

    fn drain(&mut self) -> Result<(), MultiBacktestError> {
        while let Some(at) = self.controls.next_time() {
            self.process_at(at)?;
        }
        Ok(())
    }

    fn process_control(
        &mut self,
        control: crate::ScheduledControl<ScheduledBacktestControl>,
    ) -> Result<(), MultiBacktestError> {
        let at = control.at();
        let sequence = control.sequence();
        match control.into_payload() {
            ScheduledBacktestControl::ReleaseObservation(observation) => {
                self.release_observation(*observation, sequence)
            }
            ScheduledBacktestControl::Expire(order_id) => {
                if self.order_can_expire(order_id) {
                    let feedback = self
                        .simulator
                        .expire(order_id, at)
                        .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?;
                    self.schedule_feedback(at, [feedback], [], order_id.as_bytes(), sequence)?;
                }
                Ok(())
            }
            ScheduledBacktestControl::Activate(order_id) => {
                if !self.order_is_scheduled(order_id) {
                    return Ok(());
                }
                let previous_fill_count = self.simulator.fills().len();
                let activation = self
                    .simulator
                    .activate(order_id, sequence, at)
                    .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?;
                let instrument = self
                    .simulator
                    .orders()
                    .iter()
                    .find(|order| order.id() == order_id)
                    .expect("activated order exists")
                    .request()
                    .intent()
                    .instrument()
                    .clone();
                for fill in self.simulator.fills()[previous_fill_count..]
                    .iter()
                    .cloned()
                {
                    self.ledger
                        .apply_fill(&instrument, fill)
                        .map_err(|error| MultiBacktestError::Accounting(error.to_string()))?;
                }
                self.schedule_feedback(
                    at,
                    [activation.feedback().clone()],
                    activation.execution_fills().iter().cloned(),
                    order_id.as_bytes(),
                    sequence,
                )?;
                Ok(())
            }
            ScheduledBacktestControl::AllocateVisibleDepth(instrument) => {
                let previous_fill_count = self.simulator.fills().len();
                let results = self
                    .simulator
                    .evaluate_active(&instrument, sequence, at)
                    .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?;
                for fill in self.simulator.fills()[previous_fill_count..]
                    .iter()
                    .cloned()
                {
                    self.ledger
                        .apply_fill(&instrument, fill)
                        .map_err(|error| MultiBacktestError::Accounting(error.to_string()))?;
                }
                for result in results {
                    self.schedule_feedback(
                        at,
                        [result.feedback().clone()],
                        result.execution_fills().iter().cloned(),
                        result.order_id().as_bytes(),
                        sequence,
                    )?;
                }
                Ok(())
            }
            ScheduledBacktestControl::AllocateAuctionMatch(evidence) => {
                let instrument = evidence.instrument().clone();
                let feedback_at = evidence.visible_at();
                let previous_fill_count = self.simulator.fills().len();
                let results = self
                    .simulator
                    .evaluate_auction_match(&evidence)
                    .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?;
                for fill in self.simulator.fills()[previous_fill_count..]
                    .iter()
                    .cloned()
                {
                    self.ledger
                        .apply_fill(&instrument, fill)
                        .map_err(|error| MultiBacktestError::Accounting(error.to_string()))?;
                }
                for result in results {
                    self.schedule_feedback(
                        feedback_at,
                        [result.feedback().clone()],
                        result.execution_fills().iter().cloned(),
                        result.order_id().as_bytes(),
                        sequence,
                    )?;
                }
                Ok(())
            }
            ScheduledBacktestControl::StrategyTimer(request) => {
                if self.timer_generations.get(request.timer_id()) != Some(&sequence) {
                    return Ok(());
                }
                self.fire_timer(request, sequence)
            }
            ScheduledBacktestControl::DeliverFeedback {
                feedback,
                execution_fills,
                origin_identity,
            } => self.deliver_feedback(at, sequence, origin_identity, &feedback, &execution_fills),
        }
    }

    fn release_observation(
        &mut self,
        observation: BufferedObservation,
        control_sequence: u64,
    ) -> Result<(), MultiBacktestError> {
        let event = DomainEvent::from_canonical_bytes(&observation.canonical_event)
            .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;
        let visible_commit = self
            .visible_core
            .apply_ordered(&event)
            .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;
        let occurrence = visible_commit.occurrence();
        if occurrence.run_event_ordinal() != observation.run_event_ordinal
            || occurrence.event_fingerprint().as_bytes() != &observation.event_fingerprint
            || occurrence.instrument_state_version() != observation.instrument_state_version
        {
            return Err(MultiBacktestError::Replay(
                "visible replay occurrence differs from committed replay".to_owned(),
            ));
        }
        let views = self
            .visible_core
            .states()
            .map(MarketState::view)
            .collect::<Vec<_>>();
        let state = views
            .iter()
            .copied()
            .find(|state| state.instrument() == event.instrument())
            .ok_or(MultiBacktestError::Declaration)?;
        let published_book = if matches!(
            event.payload(),
            market_types::EventPayload::BookSnapshot(_)
                | market_types::EventPayload::QuoteSnapshot(_)
        ) && let Some(book) = state.book().known().cloned()
        {
            self.simulator
                .publish_visible_book(
                    VisibleBookEvidence::new(
                        event.instrument().clone(),
                        book,
                        event.match_time(),
                        observation.visible_at,
                        observation.trading.matching(),
                        observation.trading.new_order_entry(),
                    )
                    .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?,
                )
                .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?;
            true
        } else {
            false
        };
        let mut sink = StrategyOutputSink::with_scheduled_orders();
        self.strategy
            .on_event(
                StrategyEventContext::new_visible_with_states(
                    occurrence,
                    &event,
                    state,
                    &views,
                    &observation.trading,
                    observation.visible_at,
                ),
                &mut sink,
            )
            .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?;
        let requests = sink.take_scheduled_orders();
        let timers = sink.take_timers();
        self.output.extend(
            sink.into_event_records(occurrence)
                .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?,
        );
        self.schedule_timers(observation.visible_at, timers)?;
        self.submit_requests(
            observation.visible_at,
            *occurrence.event_fingerprint().as_bytes(),
            requests,
            control_sequence,
        )?;
        if published_book
            && self.instrument_needs_allocation(event.instrument(), observation.visible_at)
        {
            self.controls
                .schedule(
                    observation.visible_at,
                    ControlPhase::FillAllocation,
                    ScheduledBacktestControl::AllocateVisibleDepth(event.instrument().clone()),
                )
                .map_err(|_| MultiBacktestError::Sequence)?;
        }
        Ok(())
    }

    fn deliver_feedback(
        &mut self,
        at: MatchTime,
        control_sequence: u64,
        origin_identity: [u8; 32],
        feedback: &[OrderFeedback],
        execution_fills: &[ExecutionFillFeedback],
    ) -> Result<(), MultiBacktestError> {
        let mut sink = StrategyOutputSink::with_scheduled_orders();
        self.strategy
            .on_feedback(
                StrategyFeedbackContext::new_with_execution_fills(feedback, execution_fills),
                &mut sink,
            )
            .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?;
        let requests = sink.take_scheduled_orders();
        let timers = sink.take_timers();
        self.output.extend(
            sink.into_control_records(control_sequence, at)
                .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?,
        );
        self.schedule_timers(at, timers)?;
        self.submit_requests(at, origin_identity, requests, control_sequence)
    }

    fn fire_timer(
        &mut self,
        request: StrategyTimerRequest,
        control_sequence: u64,
    ) -> Result<(), MultiBacktestError> {
        let at = request.fire_at();
        let mut sink = StrategyOutputSink::with_scheduled_orders();
        self.strategy
            .on_timer(StrategyTimerContext::new(request.timer_id(), at), &mut sink)
            .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?;
        let requests = sink.take_scheduled_orders();
        let timers = sink.take_timers();
        self.output.extend(
            sink.into_control_records(control_sequence, at)
                .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?,
        );
        let mut identity = Vec::new();
        identity.extend_from_slice(b"OSTM");
        identity.extend_from_slice(request.timer_id().as_str().as_bytes());
        identity.extend_from_slice(&control_sequence.to_be_bytes());
        let origin_identity = *blake3::hash(&identity).as_bytes();
        self.schedule_timers(at, timers)?;
        self.submit_requests(at, origin_identity, requests, control_sequence)
    }

    fn schedule_timers(
        &mut self,
        decision_time: MatchTime,
        timers: Vec<StrategyTimerRequest>,
    ) -> Result<(), MultiBacktestError> {
        for request in timers {
            if request.fire_at() < decision_time {
                return Err(MultiBacktestError::Strategy(
                    strategy_api::StrategyTimerError::FireBeforeDecision.to_string(),
                ));
            }
            let timer_id = request.timer_id().clone();
            let sequence = self
                .controls
                .schedule(
                    request.fire_at(),
                    ControlPhase::StrategyDecision,
                    ScheduledBacktestControl::StrategyTimer(request),
                )
                .map_err(|_| MultiBacktestError::Sequence)?;
            self.timer_generations.insert(timer_id, sequence);
        }
        Ok(())
    }

    fn submit_requests(
        &mut self,
        decision_time: MatchTime,
        origin_identity: [u8; 32],
        requests: Vec<strategy_api::ScheduledOrderRequest>,
        control_sequence: u64,
    ) -> Result<(), MultiBacktestError> {
        for (index, request) in requests.into_iter().enumerate() {
            let activation = request.activate_at();
            let expiry = request.expire_at();
            let submission = self
                .simulator
                .submit(
                    self.strategy.identity().strategy_id(),
                    execution_sim::ScheduledSubmissionContext::new(origin_identity, decision_time),
                    u32::try_from(index + 1).map_err(|_| MultiBacktestError::Sequence)?,
                    request,
                )
                .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?;
            if let Some(expiry) = expiry {
                self.controls
                    .schedule(
                        expiry,
                        ControlPhase::OrderExpiry,
                        ScheduledBacktestControl::Expire(submission.order_id()),
                    )
                    .map_err(|_| MultiBacktestError::Sequence)?;
            }
            self.controls
                .schedule(
                    activation,
                    ControlPhase::OrderActivation,
                    ScheduledBacktestControl::Activate(submission.order_id()),
                )
                .map_err(|_| MultiBacktestError::Sequence)?;
            let mut feedback = Vec::new();
            if let Some(replaced) = submission.replaced() {
                feedback.push(replaced.clone());
            }
            feedback.push(OrderFeedback::Accepted {
                order_id: submission.order_id(),
            });
            self.schedule_feedback(
                decision_time,
                feedback,
                [],
                submission.order_id().as_bytes(),
                control_sequence,
            )?;
        }
        Ok(())
    }

    fn schedule_feedback(
        &mut self,
        at: MatchTime,
        feedback: impl IntoIterator<Item = OrderFeedback>,
        execution_fills: impl IntoIterator<Item = ExecutionFillFeedback>,
        order_identity: &[u8; 32],
        parent_sequence: u64,
    ) -> Result<(), MultiBacktestError> {
        let mut identity = Vec::new();
        identity.extend_from_slice(b"OSCF");
        identity.extend_from_slice(order_identity);
        identity.extend_from_slice(&parent_sequence.to_be_bytes());
        self.controls
            .schedule(
                at,
                ControlPhase::Feedback,
                ScheduledBacktestControl::DeliverFeedback {
                    feedback: feedback.into_iter().collect(),
                    execution_fills: execution_fills.into_iter().collect(),
                    origin_identity: *blake3::hash(&identity).as_bytes(),
                },
            )
            .map_err(|_| MultiBacktestError::Sequence)?;
        Ok(())
    }

    fn order_is_scheduled(&self, order_id: OrderId) -> bool {
        self.simulator.orders().iter().any(|order| {
            order.id() == order_id && order.status() == ScheduledOrderStatus::Scheduled
        })
    }

    fn order_can_expire(&self, order_id: OrderId) -> bool {
        self.simulator.orders().iter().any(|order| {
            order.id() == order_id
                && matches!(
                    order.status(),
                    ScheduledOrderStatus::Scheduled
                        | ScheduledOrderStatus::Active
                        | ScheduledOrderStatus::PartiallyFilled
                        | ScheduledOrderStatus::MatchAttempted
                )
        })
    }

    fn instrument_needs_allocation(
        &self,
        instrument: &market_types::InstrumentId,
        at: MatchTime,
    ) -> bool {
        self.simulator.orders().iter().any(|order| {
            order.request().intent().instrument() == instrument
                && order.request().execution_policy()
                    == ScheduledExecutionPolicy::VisibleDepthUntilExpiryV1
                && (order.status() == ScheduledOrderStatus::Active
                    || (order.status() == ScheduledOrderStatus::Scheduled
                        && order.request().activate_at() <= at))
        })
    }

    fn instrument_needs_auction_match(
        &self,
        instrument: &market_types::InstrumentId,
        match_time: MatchTime,
    ) -> bool {
        self.simulator.orders().iter().any(|order| {
            order.request().intent().instrument() == instrument
                && order.request().execution_policy()
                    == ScheduledExecutionPolicy::AuctionCrossAtFirstMatchV1
                && order.request().activate_at() <= match_time
                && order.status() == ScheduledOrderStatus::Active
        })
    }
}

fn auction_clearing_price(
    event: &DomainEvent,
) -> Result<Option<market_types::Price>, MultiBacktestError> {
    let price = match event.payload() {
        market_types::EventPayload::QuoteSnapshot(snapshot) => {
            snapshot.trade().as_set().map(|trade| trade.price())
        }
        market_types::EventPayload::TradeBatch(batch) => {
            let Some(first) = batch.trades().first().copied() else {
                return Ok(None);
            };
            if batch
                .trades()
                .iter()
                .any(|trade| trade.price() != first.price())
            {
                return Err(MultiBacktestError::Simulation(
                    "call-auction trade batch contains multiple clearing prices".to_owned(),
                ));
            }
            Some(first.price())
        }
        _ => None,
    };
    Ok(price)
}

#[cfg(test)]
mod tests {
    use execution_sim::{
        AccountingModel, ChargeBasis, ChargeModel, ChargeSides, InstrumentEconomics,
        InstrumentLedgerConfig, RoundingPolicy, ScheduledDepthModel, ScheduledInstrumentConfig,
    };
    use market_state::{
        MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
    };
    use market_types::{
        BookLevel, BookSide, BookSideKind, BookSnapshot, CompleteBookSnapshot, Decimal,
        EventPayload, MarketAnnotations, MarketId, Observation, Price, Quantity, QuantityUnit,
        QuoteSnapshot, SourceFormatId, Symbol, TpexQuoteAnnotations, TradePrint, TradePrintKind,
        TradingDate, TwseQuoteAnnotations, Volume,
    };
    use replay_engine::{ReplayStreamBinding, StableStreamDescriptorId};
    use strategy_api::{
        BinaryIdentity, CanonicalParamsChecksum, ClientOrderId, IndicatorValue, OrderIntent,
        OrderSide, OrderType, ScheduledExecutionPolicy, ScheduledOrderRequest, SessionKind,
        SessionSegment, StrategyDeclaration, StrategyExecutionError, StrategyIdentity,
        StrategyOutputRecord, StrategyTimerId, StrategyTimerRequest,
    };

    use super::*;

    struct VecStream(std::vec::IntoIter<DomainEvent>);

    impl EventStream for VecStream {
        type Error = std::io::Error;

        fn next_event(&mut self) -> Result<Option<DomainEvent>, Self::Error> {
            Ok(self.0.next())
        }
    }

    struct Factory(Vec<DomainEvent>);

    impl ReplayStreamFactory for Factory {
        type Stream = VecStream;
        type Error = std::io::Error;

        fn open(
            &mut self,
            _binding: &replay_engine::ReplayStreamBinding,
        ) -> Result<Self::Stream, Self::Error> {
            Ok(VecStream(self.0.clone().into_iter()))
        }
    }

    #[test]
    fn auction_price_requires_an_explicit_trade_in_the_released_event() {
        let instrument =
            market_types::InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let date = TradingDate::parse("2026-07-27").unwrap();
        let quantity = Quantity::new(1, QuantityUnit::TradingUnit).unwrap();
        let book = CompleteBookSnapshot::new(
            BookSide::new(
                BookSideKind::Bid,
                vec![BookLevel::new(Price::parse("99").unwrap(), quantity)],
            )
            .unwrap(),
            BookSide::new(
                BookSideKind::Ask,
                vec![BookLevel::new(Price::parse("101").unwrap(), quantity)],
            )
            .unwrap(),
        )
        .unwrap();
        let event = |trade| {
            DomainEvent::new(
                instrument.clone(),
                date,
                SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
                MatchTime::from_unix_microseconds(1),
                None,
                EventPayload::QuoteSnapshot(
                    QuoteSnapshot::new(
                        book.clone(),
                        trade,
                        Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
                        MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0, 0)),
                    )
                    .unwrap(),
                ),
            )
        };

        assert_eq!(
            auction_clearing_price(&event(Observation::Set(TradePrint::new(
                Price::parse("100").unwrap(),
                quantity,
                TradePrintKind::Regular,
            ))))
            .unwrap(),
            Some(Price::parse("100").unwrap())
        );
        assert_eq!(
            auction_clearing_price(&event(Observation::NoObservation)).unwrap(),
            None
        );
    }

    struct ScheduledBuyer {
        identity: StrategyIdentity,
        declaration: StrategyDeclaration,
        emitted: bool,
        pending_instrument: Option<market_types::InstrumentId>,
        order_type: OrderType,
        quantity_unit: QuantityUnit,
        execution_policy: ScheduledExecutionPolicy,
        expiry_after_activation_micros: Option<i64>,
    }

    impl Strategy for ScheduledBuyer {
        fn identity(&self) -> &StrategyIdentity {
            &self.identity
        }

        fn canonical_params_checksum(&self) -> CanonicalParamsChecksum {
            CanonicalParamsChecksum::for_empty_params()
        }

        fn declaration(&self) -> StrategyDeclaration {
            self.declaration.clone()
        }

        fn on_event(
            &mut self,
            context: StrategyEventContext<'_>,
            output: &mut StrategyOutputSink,
        ) -> Result<(), StrategyExecutionError> {
            output.emit_indicator(
                "decision_time",
                IndicatorValue::Signed(context.decision_time().as_unix_microseconds()),
            )?;
            if self.emitted {
                return Ok(());
            }
            self.emitted = true;
            self.pending_instrument = Some(context.event().instrument().clone());
            output.emit_timer(StrategyTimerRequest::new(
                StrategyTimerId::new("entry-decision").unwrap(),
                MatchTime::from_unix_microseconds(
                    context.decision_time().as_unix_microseconds() + 25_000,
                ),
            ))?;
            output.emit_timer(StrategyTimerRequest::new(
                StrategyTimerId::new("entry-decision").unwrap(),
                MatchTime::from_unix_microseconds(
                    context.decision_time().as_unix_microseconds() + 50_000,
                ),
            ))?;
            Ok(())
        }

        fn on_timer(
            &mut self,
            context: StrategyTimerContext<'_>,
            output: &mut StrategyOutputSink,
        ) -> Result<(), StrategyExecutionError> {
            let instrument = self
                .pending_instrument
                .take()
                .ok_or_else(|| StrategyExecutionError::new("missing pending instrument"))?;
            let activation = MatchTime::from_unix_microseconds(
                context.fire_at().as_unix_microseconds() + 50_000,
            );
            let expiry = self.expiry_after_activation_micros.map(|delay| {
                MatchTime::from_unix_microseconds(activation.as_unix_microseconds() + delay)
            });
            let request = ScheduledOrderRequest::new(
                ClientOrderId::new("buy-1").unwrap(),
                None,
                activation,
                expiry,
                OrderIntent::new(
                    instrument,
                    OrderSide::Buy,
                    Quantity::new(1, self.quantity_unit).unwrap(),
                    self.order_type,
                ),
                self.execution_policy,
            )
            .map_err(|error| StrategyExecutionError::new(error.to_string()))?;
            output.emit_scheduled_order(request)?;
            Ok(())
        }
    }

    #[test]
    fn delayed_observation_can_fill_at_control_time_without_market_event() {
        let instrument =
            market_types::InstrumentId::new(MarketId::Taifex, Symbol::new("TXFH6").unwrap());
        let date = TradingDate::parse("2026-07-27").unwrap();
        let event_time = MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap();
        let segment_id = SessionSegmentId::new("regular").unwrap();
        let segment = SessionSegment::new(
            segment_id.clone(),
            SessionKind::Regular,
            date,
            event_time,
            MatchTime::parse("2026-07-27T13:30:00+08:00").unwrap(),
        )
        .unwrap();
        let quantity = Quantity::new(2, QuantityUnit::Contract).unwrap();
        let book = CompleteBookSnapshot::new(
            BookSide::new(
                BookSideKind::Bid,
                vec![BookLevel::new(Price::parse("99").unwrap(), quantity)],
            )
            .unwrap(),
            BookSide::new(
                BookSideKind::Ask,
                vec![BookLevel::new(Price::parse("101").unwrap(), quantity)],
            )
            .unwrap(),
        )
        .unwrap();
        let event = DomainEvent::new(
            instrument.clone(),
            date,
            SourceFormatId::new("I080").unwrap(),
            event_time,
            None,
            EventPayload::BookSnapshot(BookSnapshot::new(book, MarketAnnotations::None)),
        );
        let core = ReplayCore::new_multi(
            vec![MarketState::new(instrument.clone(), date)],
            vec![(instrument.clone(), MarketStateReducer::taifex_futures())],
            vec![(
                instrument.clone(),
                ReducerContext::new(date, segment_id, SegmentBoundaryPolicy::Carry, 1),
            )],
        )
        .unwrap();
        let plan = ReplayPlan::new_multi(
            [1; 32],
            vec![ReplayStreamBinding::new(
                StableStreamDescriptorId::from_bytes([2; 32]),
                instrument.clone(),
                date,
                [3; 32],
                [4; 32],
            )],
        )
        .unwrap();
        let strategy = ScheduledBuyer {
            identity: StrategyIdentity::new(
                "scheduled-buyer",
                "1",
                BinaryIdentity::new("test", [5; 32]).unwrap(),
            )
            .unwrap(),
            declaration: StrategyDeclaration::new([instrument.clone()], [SessionKind::Regular])
                .unwrap(),
            emitted: false,
            pending_instrument: None,
            order_type: OrderType::Market,
            quantity_unit: QuantityUnit::Contract,
            execution_policy: ScheduledExecutionPolicy::VisibleDepthAtActivationV1,
            expiry_after_activation_micros: None,
        };
        let simulator = ScheduledDepthSimulator::new([ScheduledInstrumentConfig::new(
            instrument.clone(),
            QuantityUnit::Contract,
            ScheduledDepthModel::new(5, 1_000, Decimal::ZERO).unwrap(),
        )])
        .unwrap();
        let zero_charge = ChargeModel {
            basis: ChargeBasis::NotionalRate,
            rate: Decimal::ZERO,
            sides: ChargeSides::Both,
            minimum: Decimal::ZERO,
            precision: 0,
            rounding: RoundingPolicy::Down,
        };
        let ledger = MultiLedger::new(
            Decimal::parse("1000000").unwrap(),
            [InstrumentLedgerConfig::new(
                instrument.clone(),
                QuantityUnit::Contract,
                AccountingModel::FuturesV1,
                InstrumentEconomics {
                    units_per_trading_unit: 1,
                    multiplier: Decimal::parse("200").unwrap(),
                    provenance: "test".into(),
                },
                zero_charge,
                zero_charge,
            )],
        )
        .unwrap();

        let completed = run_scheduled_multi_backtest(
            core,
            strategy,
            &plan,
            &mut Factory(vec![event]),
            &MultiSessionSchedule::new([(instrument.clone(), vec![segment])]).unwrap(),
            200,
            simulator,
            ledger,
            true,
        )
        .unwrap();

        assert_eq!(completed.replay.summary().event_count(), 1);
        assert_eq!(completed.simulator.fills().len(), 1);
        assert_eq!(completed.simulator.fills()[0].control_sequence(), Some(4));
        assert_eq!(completed.performance.fill_count(), 1);
        let expected_decision = event_time.as_unix_microseconds() + 200_000;
        assert!(matches!(
            completed.strategy_output.records(),
            [StrategyOutputRecord::EventIndicator {
                value: IndicatorValue::Signed(value),
                ..
            }] if *value == expected_decision
        ));
        assert_eq!(
            completed.simulator.orders()[0].request().activate_at(),
            MatchTime::from_unix_microseconds(expected_decision + 100_000)
        );
        assert!(matches!(
            completed.simulator.orders()[0].status(),
            ScheduledOrderStatus::Filled
        ));
    }

    #[test]
    fn passive_limit_fills_only_after_a_matching_book_becomes_visible() {
        let instrument =
            market_types::InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let date = TradingDate::parse("2026-07-27").unwrap();
        let first_time = MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap();
        let second_time =
            MatchTime::from_unix_microseconds(first_time.as_unix_microseconds() + 200_000);
        let segment_id = SessionSegmentId::new("regular").unwrap();
        let segment = SessionSegment::new(
            segment_id.clone(),
            SessionKind::Regular,
            date,
            first_time,
            MatchTime::parse("2026-07-27T13:30:00+08:00").unwrap(),
        )
        .unwrap();
        let quantity = Quantity::new(2, QuantityUnit::TradingUnit).unwrap();
        let make_event = |match_time| {
            let book = CompleteBookSnapshot::new(
                BookSide::new(
                    BookSideKind::Bid,
                    vec![BookLevel::new(Price::parse("99").unwrap(), quantity)],
                )
                .unwrap(),
                BookSide::new(
                    BookSideKind::Ask,
                    vec![BookLevel::new(Price::parse("101").unwrap(), quantity)],
                )
                .unwrap(),
            )
            .unwrap();
            DomainEvent::new(
                instrument.clone(),
                date,
                SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
                match_time,
                None,
                EventPayload::QuoteSnapshot(
                    QuoteSnapshot::new(
                        book,
                        Observation::NoObservation,
                        Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
                        MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0, 0)),
                    )
                    .unwrap(),
                ),
            )
        };
        let events = vec![make_event(first_time), make_event(second_time)];
        let core = ReplayCore::new_multi(
            vec![MarketState::new(instrument.clone(), date)],
            vec![(instrument.clone(), MarketStateReducer::twse_regular())],
            vec![(
                instrument.clone(),
                ReducerContext::new(date, segment_id, SegmentBoundaryPolicy::Carry, 1),
            )],
        )
        .unwrap();
        let plan = ReplayPlan::new_multi(
            [1; 32],
            vec![ReplayStreamBinding::new(
                StableStreamDescriptorId::from_bytes([2; 32]),
                instrument.clone(),
                date,
                [3; 32],
                [4; 32],
            )],
        )
        .unwrap();
        let strategy = ScheduledBuyer {
            identity: StrategyIdentity::new(
                "passive-buyer",
                "1",
                BinaryIdentity::new("test", [6; 32]).unwrap(),
            )
            .unwrap(),
            declaration: StrategyDeclaration::new([instrument.clone()], [SessionKind::Regular])
                .unwrap(),
            emitted: false,
            pending_instrument: None,
            order_type: OrderType::Limit {
                limit_price: Price::parse("102").unwrap(),
            },
            quantity_unit: QuantityUnit::TradingUnit,
            execution_policy: ScheduledExecutionPolicy::VisibleDepthUntilExpiryV1,
            expiry_after_activation_micros: Some(500_000),
        };
        let simulator = ScheduledDepthSimulator::new([ScheduledInstrumentConfig::new(
            instrument.clone(),
            QuantityUnit::TradingUnit,
            ScheduledDepthModel::new(5, 1_000, Decimal::ZERO).unwrap(),
        )])
        .unwrap();
        let zero_charge = ChargeModel {
            basis: ChargeBasis::NotionalRate,
            rate: Decimal::ZERO,
            sides: ChargeSides::Both,
            minimum: Decimal::ZERO,
            precision: 0,
            rounding: RoundingPolicy::Down,
        };
        let ledger = MultiLedger::new(
            Decimal::parse("1000000").unwrap(),
            [InstrumentLedgerConfig::new(
                instrument.clone(),
                QuantityUnit::TradingUnit,
                AccountingModel::EquityV1,
                InstrumentEconomics {
                    units_per_trading_unit: 1_000,
                    multiplier: Decimal::parse("1").unwrap(),
                    provenance: "test".into(),
                },
                zero_charge,
                zero_charge,
            )],
        )
        .unwrap();

        let completed = run_scheduled_multi_backtest(
            core,
            strategy,
            &plan,
            &mut Factory(events),
            &MultiSessionSchedule::new([(instrument.clone(), vec![segment])]).unwrap(),
            200,
            simulator,
            ledger,
            true,
        )
        .unwrap();

        assert_eq!(completed.simulator.fills().len(), 1);
        assert_eq!(completed.performance.fill_count(), 1);
        assert_eq!(
            completed.simulator.fills()[0].match_time(),
            MatchTime::from_unix_microseconds(second_time.as_unix_microseconds() + 200_000)
        );
        assert_eq!(
            completed.simulator.orders()[0].status(),
            ScheduledOrderStatus::Filled
        );
    }

    #[test]
    fn auction_fill_at_match_time_wins_over_expiry_before_feedback_visibility() {
        let instrument =
            market_types::InstrumentId::new(MarketId::Tpex, Symbol::new("3374").unwrap());
        let date = TradingDate::parse("2026-06-23").unwrap();
        let trial_time = MatchTime::parse("2026-06-23T08:59:59+08:00").unwrap();
        let match_time = MatchTime::parse("2026-06-23T09:00:00.145482+08:00").unwrap();
        let segment_id = SessionSegmentId::new("regular").unwrap();
        let segment = SessionSegment::new(
            segment_id.clone(),
            SessionKind::Regular,
            date,
            MatchTime::parse("2026-06-23T09:00:00+08:00").unwrap(),
            MatchTime::parse("2026-06-23T13:30:00+08:00").unwrap(),
        )
        .unwrap();
        let quantity = Quantity::new(1, QuantityUnit::TradingUnit).unwrap();
        let make_event = |at, status, trade| {
            let book = CompleteBookSnapshot::new(
                BookSide::new(
                    BookSideKind::Bid,
                    vec![BookLevel::new(Price::parse("99").unwrap(), quantity)],
                )
                .unwrap(),
                BookSide::new(
                    BookSideKind::Ask,
                    vec![BookLevel::new(Price::parse("101").unwrap(), quantity)],
                )
                .unwrap(),
            )
            .unwrap();
            DomainEvent::new(
                instrument.clone(),
                date,
                SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
                at,
                None,
                EventPayload::QuoteSnapshot(
                    QuoteSnapshot::new(
                        book,
                        trade,
                        Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
                        MarketAnnotations::TpexQuote(TpexQuoteAnnotations::new(status, 0)),
                    )
                    .unwrap(),
                ),
            )
        };
        let events = vec![
            make_event(trial_time, 0x88, Observation::NoObservation),
            make_event(
                match_time,
                0x08,
                Observation::Set(TradePrint::new(
                    Price::parse("100").unwrap(),
                    quantity,
                    TradePrintKind::Regular,
                )),
            ),
        ];
        let core = ReplayCore::new_multi(
            vec![MarketState::new(instrument.clone(), date)],
            vec![(instrument.clone(), MarketStateReducer::tpex_regular())],
            vec![(
                instrument.clone(),
                ReducerContext::new(date, segment_id, SegmentBoundaryPolicy::Carry, 1),
            )],
        )
        .unwrap();
        let plan = ReplayPlan::new_multi(
            [1; 32],
            vec![ReplayStreamBinding::new(
                StableStreamDescriptorId::from_bytes([2; 32]),
                instrument.clone(),
                date,
                [3; 32],
                [4; 32],
            )],
        )
        .unwrap();
        let strategy = ScheduledBuyer {
            identity: StrategyIdentity::new(
                "auction-buyer",
                "1",
                BinaryIdentity::new("test", [6; 32]).unwrap(),
            )
            .unwrap(),
            declaration: StrategyDeclaration::new([instrument.clone()], [SessionKind::Regular])
                .unwrap(),
            emitted: false,
            pending_instrument: None,
            order_type: OrderType::Limit {
                limit_price: Price::parse("101").unwrap(),
            },
            quantity_unit: QuantityUnit::TradingUnit,
            execution_policy: ScheduledExecutionPolicy::AuctionCrossAtFirstMatchV1,
            expiry_after_activation_micros: Some(1_000_000),
        };
        let simulator = ScheduledDepthSimulator::new([ScheduledInstrumentConfig::new(
            instrument.clone(),
            QuantityUnit::TradingUnit,
            ScheduledDepthModel::new(5, 1_000, Decimal::ZERO).unwrap(),
        )])
        .unwrap();
        let zero_charge = ChargeModel {
            basis: ChargeBasis::NotionalRate,
            rate: Decimal::ZERO,
            sides: ChargeSides::Both,
            minimum: Decimal::ZERO,
            precision: 0,
            rounding: RoundingPolicy::Down,
        };
        let ledger = MultiLedger::new(
            Decimal::parse("1000000").unwrap(),
            [InstrumentLedgerConfig::new(
                instrument.clone(),
                QuantityUnit::TradingUnit,
                AccountingModel::EquityV1,
                InstrumentEconomics {
                    units_per_trading_unit: 1_000,
                    multiplier: Decimal::parse("1").unwrap(),
                    provenance: "test".into(),
                },
                zero_charge,
                zero_charge,
            )],
        )
        .unwrap();

        let completed = run_scheduled_multi_backtest(
            core,
            strategy,
            &plan,
            &mut Factory(events),
            &MultiSessionSchedule::new([(instrument.clone(), vec![segment])]).unwrap(),
            200,
            simulator,
            ledger,
            true,
        )
        .unwrap();

        assert_eq!(completed.simulator.fills().len(), 1);
        assert_eq!(completed.simulator.fills()[0].match_time(), match_time);
        assert_eq!(completed.simulator.fills()[0].triggering_ordinal(), Some(2));
        assert_eq!(
            completed.simulator.orders()[0].status(),
            ScheduledOrderStatus::Filled
        );
        assert_eq!(completed.performance.fill_count(), 1);
    }
}
