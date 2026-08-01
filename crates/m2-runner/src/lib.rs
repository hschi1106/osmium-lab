use std::{collections::BTreeMap, error::Error, fmt};

use execution_sim::{
    Ledger, MultiLedger, MultiPerformanceSummary, MultiSimulator, PerformanceSummary, Simulator,
};
use market_state::LastTrade;
use market_types::{Decimal, DomainEvent, InstrumentId};
use replay_engine::{
    CompletedReplay, CoreCommit, EventStream, ReplayCore, ReplayPlan, ReplayStreamFactory,
};
use strategy_api::{
    MarketTradingContextEvaluator, OrderFeedback, SessionSegment, SessionSegmentId, Strategy,
    StrategyEventContext, StrategyFeedbackContext, StrategyFinalizeContext,
    StrategyInitializationContext, StrategyOutput, StrategyOutputSink, TwseTradingContextEvaluator,
};

mod artifacts;
pub use artifacts::{
    ArtifactError, InspectSummary, inspect_run, publish_backtest, publish_multi_backtest,
};

pub const BACKTEST_COORDINATOR_VERSION: u16 = 1;

#[derive(Debug)]
pub struct CompletedBacktest {
    pub replay: CompletedReplay,
    pub strategy_output: StrategyOutput,
    pub simulator: Simulator,
    pub ledger: Ledger,
    pub performance: PerformanceSummary,
}

#[derive(Debug, Clone)]
pub struct MultiSessionSchedule {
    segments: BTreeMap<InstrumentId, Box<[SessionSegment]>>,
}

impl MultiSessionSchedule {
    pub fn new(
        entries: impl IntoIterator<Item = (InstrumentId, Vec<SessionSegment>)>,
    ) -> Result<Self, MultiBacktestError> {
        let mut segments = BTreeMap::new();
        for (instrument, mut windows) in entries {
            if windows.is_empty() {
                return Err(MultiBacktestError::Schedule(
                    "instrument session schedule is empty".to_owned(),
                ));
            }
            windows.sort_by_key(SessionSegment::open);
            if windows
                .windows(2)
                .any(|pair| pair[0].close() >= pair[1].open())
            {
                return Err(MultiBacktestError::Schedule(
                    "instrument session schedule overlaps".to_owned(),
                ));
            }
            if segments
                .insert(instrument, windows.into_boxed_slice())
                .is_some()
            {
                return Err(MultiBacktestError::Schedule(
                    "instrument session schedule is duplicated".to_owned(),
                ));
            }
        }
        if segments.is_empty() {
            return Err(MultiBacktestError::Schedule(
                "multi-market session schedule is empty".to_owned(),
            ));
        }
        Ok(Self { segments })
    }

    #[must_use]
    pub fn segments(&self, instrument: &InstrumentId) -> Option<&[SessionSegment]> {
        self.segments.get(instrument).map(Box::as_ref)
    }

    #[must_use]
    pub fn segment_for(
        &self,
        instrument: &InstrumentId,
        match_time: market_types::MatchTime,
    ) -> Option<&SessionSegment> {
        self.segments(instrument)?
            .iter()
            .find(|segment| segment.phase(match_time).is_ok())
    }

    pub fn instruments(&self) -> impl Iterator<Item = &InstrumentId> {
        self.segments.keys()
    }
}

#[derive(Debug)]
pub struct CompletedMultiBacktest {
    pub replay: CompletedReplay,
    pub strategy_output: StrategyOutput,
    pub simulator: MultiSimulator,
    pub ledger: MultiLedger,
    pub performance: MultiPerformanceSummary,
}

#[allow(clippy::too_many_arguments)]
pub fn run_multi_backtest<S: Strategy, F: ReplayStreamFactory>(
    mut core: ReplayCore,
    mut strategy: S,
    plan: &ReplayPlan,
    factory: &mut F,
    schedule: &MultiSessionSchedule,
    mut simulator: MultiSimulator,
    mut ledger: MultiLedger,
    allow_midpoint_fallback: bool,
) -> Result<CompletedMultiBacktest, MultiBacktestError> {
    let declaration = strategy.declaration();
    let core_instruments = core
        .states()
        .map(|state| state.instrument().clone())
        .collect::<Vec<_>>();
    if declaration.universe() != core_instruments.as_slice()
        || schedule.instruments().cloned().collect::<Vec<_>>() != core_instruments
        || simulator.instruments().cloned().collect::<Vec<_>>() != core_instruments
        || ledger.instruments().cloned().collect::<Vec<_>>() != core_instruments
    {
        return Err(MultiBacktestError::Declaration);
    }
    let schedule_sessions = schedule
        .segments
        .values()
        .flat_map(|segments| segments.iter().map(SessionSegment::kind))
        .collect::<std::collections::BTreeSet<_>>();
    if declaration.sessions() != schedule_sessions.into_iter().collect::<Vec<_>>().as_slice() {
        return Err(MultiBacktestError::Declaration);
    }
    strategy
        .initialize(&StrategyInitializationContext::new(&declaration))
        .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?;
    let mut output = StrategyOutput::new(
        strategy.identity().clone(),
        strategy.canonical_params_checksum(),
    );
    let mut current_segments = BTreeMap::<InstrumentId, SessionSegmentId>::new();
    let mut last_contexts = BTreeMap::<
        InstrumentId,
        (replay_engine::EventOccurrence, strategy_api::TradingContext),
    >::new();

    core.replay_frozen_multi_with(plan, factory, |core, event, commit| {
        process_multi_event(
            core,
            event,
            commit,
            &mut strategy,
            schedule,
            &mut current_segments,
            &mut last_contexts,
            &mut simulator,
            &mut ledger,
            &mut output,
        )
        .map_err(|error| error.to_string().into_boxed_str())
    })
    .map_err(|error| MultiBacktestError::Replay(error.to_string()))?;

    for (instrument, segment_id) in current_segments {
        let cancellations = simulator.cancel_segment_end_for(&instrument, &segment_id);
        if cancellations.is_empty() {
            continue;
        }
        let (occurrence, trading) = last_contexts
            .get(&instrument)
            .ok_or(MultiBacktestError::Sequence)?;
        process_multi_feedback(
            &mut strategy,
            occurrence,
            trading,
            &mut simulator,
            &mut output,
            cancellations,
            true,
        )?;
    }
    for instrument in core_instruments {
        let final_cancellations = simulator.cancel_end_of_run_for(&instrument);
        if final_cancellations.is_empty() {
            continue;
        }
        let (occurrence, trading) = last_contexts
            .get(&instrument)
            .ok_or(MultiBacktestError::Sequence)?;
        process_multi_feedback(
            &mut strategy,
            occurrence,
            trading,
            &mut simulator,
            &mut output,
            final_cancellations,
            true,
        )?;
    }
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
    Ok(CompletedMultiBacktest {
        replay,
        strategy_output: output,
        simulator,
        ledger,
        performance,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_multi_event<S: Strategy>(
    core: &mut ReplayCore,
    event: &DomainEvent,
    commit: &CoreCommit,
    strategy: &mut S,
    schedule: &MultiSessionSchedule,
    current_segments: &mut BTreeMap<InstrumentId, SessionSegmentId>,
    last_contexts: &mut BTreeMap<
        InstrumentId,
        (replay_engine::EventOccurrence, strategy_api::TradingContext),
    >,
    simulator: &mut MultiSimulator,
    ledger: &mut MultiLedger,
    output: &mut StrategyOutput,
) -> Result<(), MultiBacktestError> {
    let segment = schedule
        .segment_for(event.instrument(), event.match_time())
        .ok_or_else(|| {
            MultiBacktestError::Schedule("event is outside session schedule".to_owned())
        })?;
    let views = core.states().map(|state| state.view()).collect::<Vec<_>>();
    let state = views
        .iter()
        .copied()
        .find(|state| state.instrument() == event.instrument())
        .ok_or(MultiBacktestError::Declaration)?;
    let trading = MarketTradingContextEvaluator
        .evaluate(event, commit.occurrence(), state, segment)
        .map_err(|error| MultiBacktestError::Context(error.to_string()))?;
    if let Some(previous) =
        current_segments.insert(event.instrument().clone(), segment.id().clone())
        && previous != *segment.id()
    {
        let cancellations = simulator.cancel_segment_end_for(event.instrument(), &previous);
        process_multi_feedback(
            strategy,
            commit.occurrence(),
            &trading,
            simulator,
            output,
            cancellations,
            false,
        )?;
    }

    let mut sink = StrategyOutputSink::with_order_intents();
    strategy
        .on_event(
            StrategyEventContext::new_with_states(
                commit.occurrence(),
                event,
                state,
                &views,
                &trading,
            ),
            &mut sink,
        )
        .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?;
    let intents = sink.take_intents();
    output.extend(
        sink.into_event_records(commit.occurrence())
            .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?,
    );
    let mut feedback = Vec::new();
    for (index, intent) in intents.into_iter().enumerate() {
        feedback.push(
            simulator
                .submit(
                    strategy.identity().strategy_id(),
                    commit.occurrence(),
                    &trading,
                    u32::try_from(index + 1).map_err(|_| MultiBacktestError::Sequence)?,
                    intent,
                )
                .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?,
        );
    }
    let previous_fill_count = simulator
        .fills_for(event.instrument())
        .map_or(0, <[execution_sim::FillRecord]>::len);
    feedback.extend(
        simulator
            .evaluate(event, commit.occurrence(), &trading)
            .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?,
    );
    let fills = simulator
        .fills_for(event.instrument())
        .ok_or(MultiBacktestError::Declaration)?;
    for fill in fills[previous_fill_count..].iter().cloned() {
        ledger
            .apply_fill(event.instrument(), fill)
            .map_err(|error| MultiBacktestError::Accounting(error.to_string()))?;
    }
    process_multi_feedback(
        strategy,
        commit.occurrence(),
        &trading,
        simulator,
        output,
        feedback,
        false,
    )?;
    last_contexts.insert(
        event.instrument().clone(),
        (commit.occurrence().clone(), trading),
    );
    Ok(())
}

fn final_mark(
    state: market_state::MarketStateView<'_>,
    allow_midpoint_fallback: bool,
) -> Option<Decimal> {
    if let LastTrade::Known(trade) = state.last_trade() {
        return Some(trade.price().as_decimal());
    }
    if !allow_midpoint_fallback {
        return None;
    }
    let (Some(bid), Some(ask)) = (state.best_bid(), state.best_ask()) else {
        return None;
    };
    let sum = bid
        .price()
        .as_decimal()
        .atoms()
        .checked_add(ask.price().as_decimal().atoms())?;
    (sum % 2 == 0).then(|| Decimal::from_atoms(sum / 2))
}

fn process_multi_feedback<S: Strategy>(
    strategy: &mut S,
    occurrence: &replay_engine::EventOccurrence,
    trading: &strategy_api::TradingContext,
    simulator: &mut MultiSimulator,
    output: &mut StrategyOutput,
    feedback: Vec<OrderFeedback>,
    reject_intents: bool,
) -> Result<(), MultiBacktestError> {
    if feedback.is_empty() {
        return Ok(());
    }
    let mut sink = StrategyOutputSink::with_order_intents();
    strategy
        .on_feedback(StrategyFeedbackContext::new(&feedback), &mut sink)
        .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?;
    let intents = sink.take_intents();
    output.extend(
        sink.into_event_records(occurrence)
            .map_err(|error| MultiBacktestError::Strategy(error.to_string()))?,
    );
    if reject_intents && !intents.is_empty() {
        return Err(MultiBacktestError::IntentAfterEnd);
    }
    for (index, intent) in intents.into_iter().enumerate() {
        simulator
            .submit(
                strategy.identity().strategy_id(),
                occurrence,
                trading,
                u32::try_from(index + 1).map_err(|_| MultiBacktestError::Sequence)?,
                intent,
            )
            .map_err(|error| MultiBacktestError::Simulation(error.to_string()))?;
    }
    Ok(())
}

pub fn run_backtest<S: Strategy>(
    core: ReplayCore,
    strategy: S,
    segment: &SessionSegment,
    events: impl IntoIterator<Item = DomainEvent>,
    simulator: Simulator,
    ledger: Ledger,
    final_mark: Option<Decimal>,
) -> Result<CompletedBacktest, BacktestError> {
    let mut events = events.into_iter();
    run_backtest_with_next(
        core,
        strategy,
        segment,
        || Ok(events.next()),
        simulator,
        ledger,
        final_mark,
    )
}

pub fn run_backtest_stream<S: Strategy, E: EventStream>(
    core: ReplayCore,
    strategy: S,
    segment: &SessionSegment,
    stream: &mut E,
    simulator: Simulator,
    ledger: Ledger,
    final_mark: Option<Decimal>,
) -> Result<CompletedBacktest, BacktestError> {
    run_backtest_with_next(
        core,
        strategy,
        segment,
        || {
            stream
                .next_event()
                .map_err(|error| BacktestError::InputStream(error.to_string()))
        },
        simulator,
        ledger,
        final_mark,
    )
}

fn run_backtest_with_next<S: Strategy>(
    mut core: ReplayCore,
    mut strategy: S,
    segment: &SessionSegment,
    mut next_event: impl FnMut() -> Result<Option<DomainEvent>, BacktestError>,
    mut simulator: Simulator,
    mut ledger: Ledger,
    final_mark: Option<Decimal>,
) -> Result<CompletedBacktest, BacktestError> {
    let declaration = strategy.declaration();
    if declaration.universe()
        != core
            .states()
            .map(|state| state.instrument().clone())
            .collect::<Vec<_>>()
    {
        return Err(BacktestError::Declaration);
    }
    strategy
        .initialize(&StrategyInitializationContext::new(&declaration))
        .map_err(|error| BacktestError::Strategy(error.to_string()))?;
    let mut output = StrategyOutput::new(
        strategy.identity().clone(),
        strategy.canonical_params_checksum(),
    );

    while let Some(event) = next_event()? {
        let commit = core
            .apply_ordered(&event)
            .map_err(|error| BacktestError::Replay(error.to_string()))?;
        let state = core
            .state(event.instrument())
            .ok_or(BacktestError::Declaration)?
            .view();
        let trading = TwseTradingContextEvaluator
            .evaluate(&event, commit.occurrence(), state, segment)
            .map_err(|error| BacktestError::Context(error.to_string()))?;
        let mut sink = StrategyOutputSink::with_order_intents();
        strategy
            .on_event(
                StrategyEventContext::new(commit.occurrence(), &event, state, &trading),
                &mut sink,
            )
            .map_err(|error| BacktestError::Strategy(error.to_string()))?;
        let intents = sink.take_intents();
        output.extend(
            sink.into_event_records(commit.occurrence())
                .map_err(|error| BacktestError::Strategy(error.to_string()))?,
        );

        let mut feedback = Vec::new();
        for (index, intent) in intents.into_iter().enumerate() {
            feedback.push(
                simulator
                    .submit(
                        strategy.identity().strategy_id(),
                        commit.occurrence(),
                        &trading,
                        u32::try_from(index + 1).map_err(|_| BacktestError::Sequence)?,
                        intent,
                    )
                    .map_err(|error| BacktestError::Simulation(error.to_string()))?,
            );
        }
        let previous_fill_count = simulator.fills().len();
        feedback.extend(
            simulator
                .evaluate(&event, commit.occurrence(), &trading)
                .map_err(|error| BacktestError::Simulation(error.to_string()))?,
        );
        for fill in simulator.fills()[previous_fill_count..].iter().cloned() {
            ledger
                .apply_fill(fill)
                .map_err(|error| BacktestError::Accounting(error.to_string()))?;
        }
        process_feedback(
            &mut strategy,
            commit.occurrence(),
            &trading,
            &mut simulator,
            &mut output,
            feedback,
        )?;
    }

    let cancellations = simulator.cancel_end_of_run();
    if !cancellations.is_empty() {
        let mut sink = StrategyOutputSink::with_order_intents();
        strategy
            .on_feedback(StrategyFeedbackContext::new(&cancellations), &mut sink)
            .map_err(|error| BacktestError::Strategy(error.to_string()))?;
        if !sink.intents().is_empty() {
            return Err(BacktestError::IntentAfterEnd);
        }
    }
    let states = core.states().map(|state| state.view()).collect::<Vec<_>>();
    let mut sink = StrategyOutputSink::new();
    strategy
        .finalize(
            &StrategyFinalizeContext::new(core.clock(), states),
            &mut sink,
        )
        .map_err(|error| BacktestError::Strategy(error.to_string()))?;
    output.extend(
        sink.into_finalize_records()
            .map_err(|error| BacktestError::Strategy(error.to_string()))?,
    );
    ledger
        .reconcile()
        .map_err(|error| BacktestError::Accounting(error.to_string()))?;
    let performance = ledger
        .performance(final_mark)
        .map_err(|error| BacktestError::Accounting(error.to_string()))?;
    let replay = core
        .complete()
        .map_err(|error| BacktestError::Replay(error.to_string()))?;
    Ok(CompletedBacktest {
        replay,
        strategy_output: output,
        simulator,
        ledger,
        performance,
    })
}

fn process_feedback<S: Strategy>(
    strategy: &mut S,
    occurrence: &replay_engine::EventOccurrence,
    trading: &strategy_api::TradingContext,
    simulator: &mut Simulator,
    output: &mut StrategyOutput,
    feedback: Vec<OrderFeedback>,
) -> Result<(), BacktestError> {
    if feedback.is_empty() {
        return Ok(());
    }
    let mut sink = StrategyOutputSink::with_order_intents();
    strategy
        .on_feedback(StrategyFeedbackContext::new(&feedback), &mut sink)
        .map_err(|error| BacktestError::Strategy(error.to_string()))?;
    let intents = sink.take_intents();
    output.extend(
        sink.into_event_records(occurrence)
            .map_err(|error| BacktestError::Strategy(error.to_string()))?,
    );
    for (index, intent) in intents.into_iter().enumerate() {
        simulator
            .submit(
                strategy.identity().strategy_id(),
                occurrence,
                trading,
                u32::try_from(index + 1).map_err(|_| BacktestError::Sequence)?,
                intent,
            )
            .map_err(|error| BacktestError::Simulation(error.to_string()))?;
    }
    Ok(())
}

#[derive(Debug)]
pub enum BacktestError {
    Declaration,
    InputStream(String),
    Replay(String),
    Context(String),
    Strategy(String),
    Simulation(String),
    Accounting(String),
    Sequence,
    IntentAfterEnd,
}

#[derive(Debug)]
pub enum MultiBacktestError {
    Declaration,
    Schedule(String),
    Replay(String),
    Context(String),
    Strategy(String),
    Simulation(String),
    Accounting(String),
    Sequence,
    IntentAfterEnd,
}

impl fmt::Display for MultiBacktestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for MultiBacktestError {}

impl fmt::Display for BacktestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for BacktestError {}

#[cfg(test)]
mod tests {
    use execution_sim::{
        AccountingModel, ChargeModel, ChargeSides, EvidenceMode, FillModel, InstrumentEconomics,
        InstrumentLedgerConfig, MultiLedger, QuantityPolicy, RoundingPolicy,
    };
    use market_state::{
        MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
    };
    use market_types::{
        BookLevel, BookSide, BookSideKind, BookSnapshot, CompleteBookSnapshot, Decimal,
        DomainEvent, EventPayload, InstrumentId, MarketAnnotations, MarketId, MatchTime, Price,
        Quantity, QuantityUnit, SourceFormatId, Symbol, TradingDate,
    };
    use replay_engine::{
        EventStream, ReplayPlan, ReplayStreamBinding, ReplayStreamFactory, StableStreamDescriptorId,
    };
    use strategy_api::{ExampleStrategy, M3AcceptanceStrategy, SessionKind};

    use super::*;

    #[test]
    fn empty_stream_still_finalizes_and_reconciles() {
        let instrument = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let date = TradingDate::parse("2026-07-27").unwrap();
        let segment_id = SessionSegmentId::new("regular").unwrap();
        let segment = SessionSegment::new(
            segment_id.clone(),
            SessionKind::Regular,
            date,
            MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap(),
            MatchTime::parse("2026-07-27T13:30:00+08:00").unwrap(),
        )
        .unwrap();
        let core = ReplayCore::new(
            vec![MarketState::new(instrument.clone(), date)],
            MarketStateReducer::twse_regular(),
            ReducerContext::new(date, segment_id, SegmentBoundaryPolicy::Carry, 1),
        )
        .unwrap();
        let strategy = ExampleStrategy::new(
            ExampleStrategy::source_binary_identity().unwrap(),
            instrument.clone(),
        )
        .unwrap();
        let zero_charge = ChargeModel {
            rate: Decimal::ZERO,
            sides: ChargeSides::Both,
            minimum: Decimal::ZERO,
            precision: 0,
            rounding: RoundingPolicy::Down,
        };
        let ledger = Ledger::new(
            Decimal::parse("1000000").unwrap(),
            InstrumentEconomics {
                units_per_trading_unit: 1000,
                multiplier: Decimal::parse("1").unwrap(),
                provenance: "test".into(),
            },
            zero_charge,
            zero_charge,
        );
        let simulator = Simulator::new(
            [instrument],
            QuantityUnit::TradingUnit,
            FillModel {
                evidence: EvidenceMode::TopOfBook,
                quantity: QuantityPolicy::Displayed,
                adverse_price_delta: Decimal::ZERO,
            },
        );
        let completed =
            run_backtest(core, strategy, &segment, [], simulator, ledger, None).unwrap();
        assert_eq!(completed.performance.fill_count, 0);
        assert_eq!(completed.replay.summary().event_count(), 0);

        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("run");
        publish_backtest(&output, &completed, &[7; 32], "source-1", "cache-1").unwrap();
        let inspected = inspect_run(&output).unwrap();
        assert_eq!(inspected.status, "successful");
        assert_eq!(inspected.event_count, 0);

        std::fs::write(output.join("ledger.bin"), b"corrupt").unwrap();
        assert!(matches!(
            inspect_run(&output),
            Err(ArtifactError::Checksum(name)) if name == "ledger.bin"
        ));
    }

    struct MultiVecStream {
        events: std::vec::IntoIter<DomainEvent>,
    }

    impl EventStream for MultiVecStream {
        type Error = std::io::Error;

        fn next_event(&mut self) -> Result<Option<DomainEvent>, Self::Error> {
            Ok(self.events.next())
        }
    }

    struct MultiFactory {
        events: Vec<DomainEvent>,
    }

    impl ReplayStreamFactory for MultiFactory {
        type Stream = MultiVecStream;
        type Error = std::io::Error;

        fn open(&mut self, _binding: &ReplayStreamBinding) -> Result<Self::Stream, Self::Error> {
            Ok(MultiVecStream {
                events: self.events.clone().into_iter(),
            })
        }
    }

    fn taifex_event(instrument: &InstrumentId, match_time: MatchTime) -> DomainEvent {
        let quantity = Quantity::new(1, QuantityUnit::Contract).unwrap();
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
            TradingDate::parse("2026-07-27").unwrap(),
            SourceFormatId::new("I080").unwrap(),
            match_time,
            None,
            EventPayload::BookSnapshot(BookSnapshot::new(book, MarketAnnotations::None)),
        )
    }

    #[test]
    fn multi_backtest_runs_isolated_subsequent_fills() {
        let instrument = InstrumentId::new(MarketId::Taifex, Symbol::new("TXFH6").unwrap());
        let date = TradingDate::parse("2026-07-27").unwrap();
        let segment_id = SessionSegmentId::new("regular").unwrap();
        let segment = SessionSegment::new(
            segment_id.clone(),
            SessionKind::Regular,
            date,
            MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap(),
            MatchTime::parse("2026-07-27T13:30:00+08:00").unwrap(),
        )
        .unwrap();
        let events = vec![
            taifex_event(
                &instrument,
                MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap(),
            ),
            taifex_event(
                &instrument,
                MatchTime::parse("2026-07-27T09:00:01+08:00").unwrap(),
            ),
            taifex_event(
                &instrument,
                MatchTime::parse("2026-07-27T09:00:02+08:00").unwrap(),
            ),
        ];
        let core = ReplayCore::new_multi(
            vec![MarketState::new(instrument.clone(), date)],
            vec![(instrument.clone(), MarketStateReducer::taifex_futures())],
            vec![(
                instrument.clone(),
                ReducerContext::new(date, segment_id, SegmentBoundaryPolicy::Carry, 1),
            )],
        )
        .unwrap();
        let binding = ReplayStreamBinding::new(
            StableStreamDescriptorId::from_bytes([8; 32]),
            instrument.clone(),
            date,
            [2; 32],
            [3; 32],
        );
        let plan = ReplayPlan::new_multi([4; 32], vec![binding]).unwrap();
        let schedule =
            MultiSessionSchedule::new(vec![(instrument.clone(), vec![segment])]).unwrap();
        let strategy = M3AcceptanceStrategy::new(
            M3AcceptanceStrategy::source_binary_identity().unwrap(),
            [instrument.clone()],
            [SessionKind::Regular],
        )
        .unwrap();
        let simulator = MultiSimulator::new([(
            instrument.clone(),
            QuantityUnit::Contract,
            execution_sim::FillModel {
                evidence: execution_sim::EvidenceMode::TopOfBook,
                quantity: execution_sim::QuantityPolicy::Displayed,
                adverse_price_delta: "0".parse().unwrap(),
            },
        )])
        .unwrap();
        let ledger = MultiLedger::new(
            Decimal::parse("1000000").unwrap(),
            [InstrumentLedgerConfig::new(
                instrument.clone(),
                QuantityUnit::Contract,
                AccountingModel::FuturesV1,
                InstrumentEconomics {
                    units_per_trading_unit: 1,
                    multiplier: Decimal::parse("200").unwrap(),
                    provenance: "test TAIFEX multiplier".into(),
                },
                ChargeModel {
                    rate: Decimal::ZERO,
                    sides: ChargeSides::Both,
                    minimum: Decimal::ZERO,
                    precision: 0,
                    rounding: RoundingPolicy::Down,
                },
                ChargeModel {
                    rate: Decimal::ZERO,
                    sides: ChargeSides::Sell,
                    minimum: Decimal::ZERO,
                    precision: 0,
                    rounding: RoundingPolicy::Down,
                },
            )],
        )
        .unwrap();
        let completed = run_multi_backtest(
            core,
            strategy,
            &plan,
            &mut MultiFactory { events },
            &schedule,
            simulator,
            ledger,
            false,
        )
        .unwrap();
        assert_eq!(completed.replay.summary().event_count(), 3);
        assert_eq!(completed.simulator.order_count(), 2);
        assert_eq!(completed.simulator.fill_count(), 2);
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("m3-run");
        publish_multi_backtest(&output, &completed, &[6; 32], "source", "cache").unwrap();
        let inspected = inspect_run(&output).unwrap();
        assert_eq!(inspected.status, "successful");
        assert_eq!(inspected.order_count, 2);
        assert_eq!(inspected.fill_count, 2);
        assert_eq!(
            std::fs::read(output.join("warnings.yaml")).unwrap(),
            b"warnings: []\n"
        );
        assert!(
            std::fs::read(output.join("ledger.bin"))
                .unwrap()
                .starts_with(b"OSLEDGR1")
        );
        let performance = std::fs::read_to_string(output.join("performance.yaml")).unwrap();
        assert!(performance.contains("accounting_version: 3"));
        assert!(performance.contains("Taifex:TXFH6"));
    }
}
