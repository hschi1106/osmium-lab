use std::{error::Error, fmt, panic::AssertUnwindSafe};

use market_state::MarketStateView;
use market_types::{DomainEvent, EventFingerprint};
use replay_engine::{
    CompletedReplay, EventOccurrence, ReplayCore, ReplayError, ReplayEventStreamChecksum,
    order_events,
};

use crate::{
    ContextError, SessionSegment, Strategy, StrategyDeclaration, StrategyEventContext,
    StrategyFinalizeContext, StrategyInitializationContext, StrategyOutput,
    StrategyOutputEncodingError, StrategyOutputSink, TwseTradingContextEvaluator,
};

#[derive(Debug)]
pub struct CompletedStrategyRun {
    replay: CompletedReplay,
    strategy_output: StrategyOutput,
    callback_count: u64,
}

impl CompletedStrategyRun {
    #[must_use]
    pub const fn replay(&self) -> &CompletedReplay {
        &self.replay
    }

    #[must_use]
    pub const fn strategy_output(&self) -> &StrategyOutput {
        &self.strategy_output
    }

    #[must_use]
    pub const fn callback_count(&self) -> u64 {
        self.callback_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyRunErrorCategory {
    InvalidParameters,
    Declaration,
    Initialization,
    Callback,
    Finalize,
    StrategyPanic,
    CapabilityUnavailable,
    Replay,
    Context,
    OutputEncoding,
}

#[derive(Debug)]
pub struct FailedStrategyRun {
    category: StrategyRunErrorCategory,
    message: Box<str>,
    occurrence: Option<FailedOccurrence>,
    processed_event_count: u64,
    processed_prefix_checksum: ReplayEventStreamChecksum,
    committed_output_count: usize,
}

impl FailedStrategyRun {
    #[must_use]
    pub const fn category(&self) -> StrategyRunErrorCategory {
        self.category
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn occurrence(&self) -> Option<&FailedOccurrence> {
        self.occurrence.as_ref()
    }

    #[must_use]
    pub const fn processed_event_count(&self) -> u64 {
        self.processed_event_count
    }

    #[must_use]
    pub const fn processed_prefix_checksum(&self) -> ReplayEventStreamChecksum {
        self.processed_prefix_checksum
    }

    #[must_use]
    pub const fn committed_output_count(&self) -> usize {
        self.committed_output_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailedOccurrence {
    run_event_ordinal: u64,
    event_fingerprint: EventFingerprint,
    instrument_state_version: u64,
}

impl FailedOccurrence {
    #[must_use]
    pub const fn run_event_ordinal(&self) -> u64 {
        self.run_event_ordinal
    }

    #[must_use]
    pub const fn event_fingerprint(&self) -> EventFingerprint {
        self.event_fingerprint
    }

    #[must_use]
    pub const fn instrument_state_version(&self) -> u64 {
        self.instrument_state_version
    }
}

impl From<&EventOccurrence> for FailedOccurrence {
    fn from(value: &EventOccurrence) -> Self {
        Self {
            run_event_ordinal: value.run_event_ordinal(),
            event_fingerprint: value.event_fingerprint(),
            instrument_state_version: value.instrument_state_version(),
        }
    }
}

#[derive(Debug)]
pub struct StrategyRunError(Box<FailedStrategyRun>);

impl StrategyRunError {
    #[must_use]
    pub const fn failure(&self) -> &FailedStrategyRun {
        &self.0
    }
}

impl fmt::Display for StrategyRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.0.category, self.0.message)
    }
}

impl Error for StrategyRunError {}

pub fn run_strategy<S: Strategy>(
    mut core: ReplayCore,
    mut strategy: S,
    segment: &SessionSegment,
    events: Vec<DomainEvent>,
) -> Result<CompletedStrategyRun, StrategyRunError> {
    let mut output = StrategyOutput::new(
        strategy.identity().clone(),
        strategy.canonical_params_checksum(),
    );
    let declaration = strategy.declaration();
    validate_declaration(&core, segment, &declaration).map_err(|message| {
        failure(
            &core,
            &output,
            StrategyRunErrorCategory::Declaration,
            message,
            None,
        )
    })?;
    let ordered = order_events(events).map_err(|error| {
        failure(
            &core,
            &output,
            StrategyRunErrorCategory::Replay,
            error.to_string(),
            None,
        )
    })?;

    let initialization = StrategyInitializationContext::new(&declaration);
    invoke_strategy(StrategyRunErrorCategory::Initialization, || {
        strategy.initialize(&initialization)
    })
    .map_err(|(category, message)| failure(&core, &output, category, message, None))?;

    let mut callback_count = 0_u64;
    for event in &ordered {
        let commit = core.apply_ordered(event).map_err(|error| {
            failure(
                &core,
                &output,
                StrategyRunErrorCategory::Replay,
                error.to_string(),
                None,
            )
        })?;
        let state = core
            .state(event.instrument())
            .expect("replay commit guarantees instrument state")
            .view();
        let trading = TwseTradingContextEvaluator
            .evaluate(event, commit.occurrence(), state, segment)
            .map_err(|error| {
                failure(
                    &core,
                    &output,
                    StrategyRunErrorCategory::Context,
                    error.to_string(),
                    Some(commit.occurrence()),
                )
            })?;
        let context = StrategyEventContext::new(commit.occurrence(), event, state, &trading);
        let mut sink = StrategyOutputSink::new();
        invoke_strategy(StrategyRunErrorCategory::Callback, || {
            strategy.on_event(context, &mut sink)
        })
        .map_err(|(category, message)| {
            failure(&core, &output, category, message, Some(commit.occurrence()))
        })?;
        let records = sink
            .into_event_records(commit.occurrence())
            .map_err(|error| {
                failure(
                    &core,
                    &output,
                    StrategyRunErrorCategory::OutputEncoding,
                    error.to_string(),
                    Some(commit.occurrence()),
                )
            })?;
        output.extend(records);
        callback_count = callback_count
            .checked_add(1)
            .expect("event count already fits in u64");
    }

    let states = core
        .states()
        .map(|state| state.view())
        .collect::<Vec<MarketStateView<'_>>>();
    let finalization = StrategyFinalizeContext::new(core.clock(), states);
    let mut sink = StrategyOutputSink::new();
    invoke_strategy(StrategyRunErrorCategory::Finalize, || {
        strategy.finalize(&finalization, &mut sink)
    })
    .map_err(|(category, message)| failure(&core, &output, category, message, None))?;
    let records = sink.into_finalize_records().map_err(|error| {
        failure(
            &core,
            &output,
            StrategyRunErrorCategory::OutputEncoding,
            error.to_string(),
            None,
        )
    })?;
    output.extend(records);
    let processed_prefix_checksum = core.processed_prefix_checksum();
    let replay = core.complete().map_err(|error| {
        failure_after_complete_error(&output, callback_count, processed_prefix_checksum, error)
    })?;

    Ok(CompletedStrategyRun {
        replay,
        strategy_output: output,
        callback_count,
    })
}

fn validate_declaration(
    core: &ReplayCore,
    segment: &SessionSegment,
    declaration: &StrategyDeclaration,
) -> Result<(), &'static str> {
    let core_universe = core
        .states()
        .map(|state| state.instrument())
        .collect::<Vec<_>>();
    if declaration.universe().iter().collect::<Vec<_>>() != core_universe {
        return Err("strategy declaration and replay universe differ");
    }
    if !declaration.sessions().contains(&segment.kind()) {
        return Err("strategy declaration does not include the replay session kind");
    }
    Ok(())
}

fn invoke_strategy<T>(
    category: StrategyRunErrorCategory,
    callback: impl FnOnce() -> Result<T, crate::StrategyExecutionError>,
) -> Result<T, (StrategyRunErrorCategory, String)> {
    match std::panic::catch_unwind(AssertUnwindSafe(callback)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err((
            if error.is_capability_unavailable() {
                StrategyRunErrorCategory::CapabilityUnavailable
            } else {
                category
            },
            error.to_string(),
        )),
        Err(_) => Err((
            StrategyRunErrorCategory::StrategyPanic,
            "strategy panicked".to_owned(),
        )),
    }
}

fn failure(
    core: &ReplayCore,
    output: &StrategyOutput,
    category: StrategyRunErrorCategory,
    message: impl Into<Box<str>>,
    occurrence: Option<&EventOccurrence>,
) -> StrategyRunError {
    StrategyRunError(Box::new(FailedStrategyRun {
        category,
        message: message.into(),
        occurrence: occurrence.map(FailedOccurrence::from),
        processed_event_count: core.clock().event_ordinal(),
        processed_prefix_checksum: core.processed_prefix_checksum(),
        committed_output_count: output.records().len(),
    }))
}

fn failure_after_complete_error(
    output: &StrategyOutput,
    processed_event_count: u64,
    processed_prefix_checksum: ReplayEventStreamChecksum,
    error: ReplayError,
) -> StrategyRunError {
    StrategyRunError(Box::new(FailedStrategyRun {
        category: StrategyRunErrorCategory::Replay,
        message: error.to_string().into_boxed_str(),
        occurrence: None,
        processed_event_count,
        processed_prefix_checksum,
        committed_output_count: output.records().len(),
    }))
}

#[allow(dead_code)]
fn output_error_is_stable(error: StrategyOutputEncodingError) -> String {
    error.to_string()
}

#[allow(dead_code)]
fn context_error_is_stable(error: ContextError) -> String {
    error.to_string()
}
