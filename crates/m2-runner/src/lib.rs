use std::{error::Error, fmt};

use execution_sim::{Ledger, PerformanceSummary, Simulator};
use market_types::{Decimal, DomainEvent};
use replay_engine::{CompletedReplay, EventStream, ReplayCore};
use strategy_api::{
    OrderFeedback, SessionSegment, Strategy, StrategyEventContext, StrategyFeedbackContext,
    StrategyFinalizeContext, StrategyInitializationContext, StrategyOutput, StrategyOutputSink,
    TwseTradingContextEvaluator,
};

mod artifacts;
pub use artifacts::{ArtifactError, InspectSummary, inspect_run, publish_backtest};

pub const BACKTEST_COORDINATOR_VERSION: u16 = 1;

#[derive(Debug)]
pub struct CompletedBacktest {
    pub replay: CompletedReplay,
    pub strategy_output: StrategyOutput,
    pub simulator: Simulator,
    pub ledger: Ledger,
    pub performance: PerformanceSummary,
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

impl fmt::Display for BacktestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for BacktestError {}

#[cfg(test)]
mod tests {
    use execution_sim::{
        ChargeModel, ChargeSides, EvidenceMode, FillModel, InstrumentEconomics, QuantityPolicy,
        RoundingPolicy,
    };
    use market_state::{
        MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
    };
    use market_types::{
        Decimal, InstrumentId, MarketId, MatchTime, QuantityUnit, Symbol, TradingDate,
    };
    use strategy_api::{ExampleStrategy, SessionKind};

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
}
