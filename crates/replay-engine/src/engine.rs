use std::{collections::BTreeMap, error::Error, fmt};

use market_state::{
    FinalStateChecksum, FinalStateEncodingError, MarketState, MarketStateReducer, ReducerContext,
    StateTransitionError, TransitionReceipt, final_state_checksum,
};
use market_types::{
    CanonicalEncodingError, DomainEvent, EventFingerprint, InstrumentId, MatchTime,
};

use crate::{
    EventStream, OrderingError, OrderingKey, ReplayEventStreamChecksum, ReplayPlan,
    ReplayStreamFactory, checksum::ReplayEventStreamHasher, order_events,
};

pub const REPLAY_ENGINE_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayContextWindow {
    start: MatchTime,
    end_exclusive: MatchTime,
    context: ReducerContext,
}

impl ReplayContextWindow {
    pub fn new(
        start: MatchTime,
        end_exclusive: MatchTime,
        context: ReducerContext,
    ) -> Result<Self, ReplayContextWindowError> {
        if start >= end_exclusive {
            return Err(ReplayContextWindowError::InvalidRange);
        }
        Ok(Self {
            start,
            end_exclusive,
            context,
        })
    }

    #[must_use]
    pub const fn start(&self) -> MatchTime {
        self.start
    }

    #[must_use]
    pub const fn end_exclusive(&self) -> MatchTime {
        self.end_exclusive
    }

    #[must_use]
    pub const fn context(&self) -> &ReducerContext {
        &self.context
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayContextWindowError {
    InvalidRange,
}

impl fmt::Display for ReplayContextWindowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("replay context window must have a positive half-open range")
    }
}

impl Error for ReplayContextWindowError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayClock {
    Unstarted,
    At {
        match_time: MatchTime,
        event_ordinal: u64,
    },
}

impl ReplayClock {
    #[must_use]
    pub const fn event_ordinal(self) -> u64 {
        match self {
            Self::Unstarted => 0,
            Self::At { event_ordinal, .. } => event_ordinal,
        }
    }

    #[must_use]
    pub const fn match_time(self) -> Option<MatchTime> {
        match self {
            Self::Unstarted => None,
            Self::At { match_time, .. } => Some(match_time),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOccurrence {
    run_event_ordinal: u64,
    ordering_key: OrderingKey,
    event_fingerprint: EventFingerprint,
    instrument_state_version: u64,
}

impl EventOccurrence {
    #[must_use]
    pub const fn run_event_ordinal(&self) -> u64 {
        self.run_event_ordinal
    }

    #[must_use]
    pub const fn ordering_key(&self) -> &OrderingKey {
        &self.ordering_key
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreCommit {
    occurrence: EventOccurrence,
    transition: TransitionReceipt,
}

impl CoreCommit {
    #[must_use]
    pub const fn occurrence(&self) -> &EventOccurrence {
        &self.occurrence
    }

    #[must_use]
    pub const fn transition(&self) -> &TransitionReceipt {
        &self.transition
    }
}

/// Replay core: ordered events, atomic state transitions, logical clock, and checksums.
#[derive(Debug)]
pub struct ReplayCore {
    states: BTreeMap<InstrumentId, MarketState>,
    reducers: BTreeMap<InstrumentId, MarketStateReducer>,
    reducer_contexts: BTreeMap<InstrumentId, ReducerContext>,
    reducer_context_windows: BTreeMap<InstrumentId, Box<[ReplayContextWindow]>>,
    clock: ReplayClock,
    last_ordering_key: Option<OrderingKey>,
    last_canonical_event: Option<Vec<u8>>,
    event_stream: ReplayEventStreamHasher,
    first_match_time: Option<MatchTime>,
}

impl ReplayCore {
    pub fn new(
        states: Vec<MarketState>,
        reducer: MarketStateReducer,
        reducer_context: ReducerContext,
    ) -> Result<Self, ReplayError> {
        let reducer_contexts = states
            .iter()
            .map(|state| (state.instrument().clone(), reducer_context.clone()))
            .collect::<Vec<_>>();
        let reducers = states
            .iter()
            .map(|state| (state.instrument().clone(), reducer.clone()))
            .collect::<Vec<_>>();
        Self::new_multi(states, reducers, reducer_contexts)
    }

    pub fn new_multi(
        states: Vec<MarketState>,
        reducers: Vec<(InstrumentId, MarketStateReducer)>,
        reducer_contexts: Vec<(InstrumentId, ReducerContext)>,
    ) -> Result<Self, ReplayError> {
        Self::new_multi_with_schedules(states, reducers, reducer_contexts, Vec::new())
    }

    pub fn new_multi_with_schedules(
        states: Vec<MarketState>,
        reducers: Vec<(InstrumentId, MarketStateReducer)>,
        reducer_contexts: Vec<(InstrumentId, ReducerContext)>,
        schedules: Vec<(InstrumentId, Vec<ReplayContextWindow>)>,
    ) -> Result<Self, ReplayError> {
        if states.is_empty() {
            return Err(ReplayError::EmptyUniverse);
        }
        let mut by_instrument = BTreeMap::new();
        for state in states {
            if by_instrument
                .insert(state.instrument().clone(), state)
                .is_some()
            {
                return Err(ReplayError::DuplicateInstrument);
            }
        }
        let reducer_count = reducers.len();
        let context_count = reducer_contexts.len();
        let reducers = reducers.into_iter().collect::<BTreeMap<_, _>>();
        let reducer_contexts = reducer_contexts.into_iter().collect::<BTreeMap<_, _>>();
        if reducers.len() != by_instrument.len()
            || reducer_count != reducers.len()
            || reducer_contexts.len() != by_instrument.len()
            || context_count != reducer_contexts.len()
            || by_instrument.keys().any(|instrument| {
                !reducers.contains_key(instrument) || !reducer_contexts.contains_key(instrument)
            })
        {
            return Err(ReplayError::ReducerConfigurationMismatch);
        }
        if by_instrument.iter().any(|(instrument, state)| {
            reducer_contexts
                .get(instrument)
                .is_none_or(|context| context.trading_date() != state.trading_date())
        }) {
            return Err(ReplayError::StateTradingDateMismatch);
        }
        let mut reducer_context_windows = BTreeMap::new();
        for (instrument, mut windows) in schedules {
            if !by_instrument.contains_key(&instrument)
                || !reducers.contains_key(&instrument)
                || !reducer_contexts.contains_key(&instrument)
            {
                return Err(ReplayError::ReducerConfigurationMismatch);
            }
            windows.sort_by_key(ReplayContextWindow::start);
            if windows.windows(2).any(|pair| {
                pair[0].end_exclusive() > pair[1].start()
                    || pair[0].context().trading_date() != pair[1].context().trading_date()
            }) || windows.iter().any(|window| {
                window.context().trading_date()
                    != by_instrument
                        .get(&instrument)
                        .expect("instrument checked above")
                        .trading_date()
            }) {
                return Err(ReplayError::InvalidContextSchedule);
            }
            if reducer_context_windows
                .insert(instrument, windows.into_boxed_slice())
                .is_some()
            {
                return Err(ReplayError::InvalidContextSchedule);
            }
        }
        Ok(Self {
            states: by_instrument,
            reducers,
            reducer_contexts,
            reducer_context_windows,
            clock: ReplayClock::Unstarted,
            last_ordering_key: None,
            last_canonical_event: None,
            event_stream: ReplayEventStreamHasher::new(),
            first_match_time: None,
        })
    }

    #[must_use]
    pub const fn clock(&self) -> ReplayClock {
        self.clock
    }

    #[must_use]
    pub fn state(&self, instrument: &InstrumentId) -> Option<&MarketState> {
        self.states.get(instrument)
    }

    pub fn states(&self) -> impl Iterator<Item = &MarketState> {
        self.states.values()
    }

    #[must_use]
    pub fn processed_prefix_checksum(&self) -> ReplayEventStreamChecksum {
        self.event_stream.checksum()
    }

    /// Establishes deterministic order for an in-memory rebuild input, then replays it.
    pub fn replay(&mut self, events: Vec<DomainEvent>) -> Result<(), ReplayError> {
        let ordered = order_events(events).map_err(ReplayError::Ordering)?;
        for event in &ordered {
            self.apply_ordered(event)?;
        }
        Ok(())
    }

    /// Replays a bounded event stream without materializing the full period.
    pub fn replay_stream<S: EventStream>(&mut self, stream: &mut S) -> Result<(), ReplayError> {
        while let Some(event) = stream
            .next_event()
            .map_err(|error| ReplayError::Stream(error.to_string().into_boxed_str()))?
        {
            self.apply_ordered(&event)?;
        }
        Ok(())
    }

    /// Opens only the stream frozen into a single-stream replay plan and consumes it offline.
    pub fn replay_frozen<F: ReplayStreamFactory>(
        &mut self,
        plan: &ReplayPlan,
        factory: &mut F,
    ) -> Result<(), ReplayError> {
        let binding = plan.binding();
        let state = self
            .states
            .get(binding.instrument())
            .ok_or(ReplayError::PlanOutsideUniverse)?;
        if state.trading_date() != binding.trading_date() {
            return Err(ReplayError::PlanTradingDateMismatch);
        }
        let mut stream = factory
            .open(binding)
            .map_err(|error| ReplayError::StreamOpen(error.to_string().into_boxed_str()))?;
        self.replay_stream(&mut stream)
    }

    /// Opens every selected stream once and merges one head event per stream.
    ///
    /// The merge keeps only the current head for each opened stream, so memory is
    /// bounded by the number of selected streams plus the stream implementations'
    /// own bounded buffers.
    pub fn replay_frozen_multi<F: ReplayStreamFactory>(
        &mut self,
        plan: &ReplayPlan,
        factory: &mut F,
    ) -> Result<(), ReplayError> {
        self.replay_frozen_multi_with(plan, factory, |_, _, _| Ok(()))
    }

    /// Opens and merges every selected stream, invoking a callback after each
    /// event has been committed to all instrument state owned by this core.
    pub fn replay_frozen_multi_with<F, C>(
        &mut self,
        plan: &ReplayPlan,
        factory: &mut F,
        mut callback: C,
    ) -> Result<(), ReplayError>
    where
        F: ReplayStreamFactory,
        C: FnMut(&mut Self, &DomainEvent, &CoreCommit) -> Result<(), Box<str>>,
    {
        let mut streams = Vec::with_capacity(plan.bindings().len());
        for binding in plan.bindings() {
            let state = self
                .states
                .get(binding.instrument())
                .ok_or(ReplayError::PlanOutsideUniverse)?;
            if state.trading_date() != binding.trading_date() {
                return Err(ReplayError::PlanTradingDateMismatch);
            }
            streams.push(
                factory
                    .open(binding)
                    .map_err(|error| ReplayError::StreamOpen(error.to_string().into_boxed_str()))?,
            );
        }

        let mut heads = (0..streams.len())
            .map(|_| None)
            .collect::<Vec<Option<DomainEvent>>>();
        loop {
            for (index, stream) in streams.iter_mut().enumerate() {
                if heads[index].is_none() {
                    heads[index] = stream
                        .next_event()
                        .map_err(|error| ReplayError::Stream(error.to_string().into_boxed_str()))?;
                }
            }
            let mut selected: Option<(usize, OrderingKey)> = None;
            for (index, event) in heads.iter().enumerate() {
                let Some(event) = event else { continue };
                let key = OrderingKey::for_event(event).map_err(ReplayError::Ordering)?;
                if selected
                    .as_ref()
                    .is_none_or(|(_, previous)| key < *previous)
                {
                    selected = Some((index, key));
                }
            }
            let Some((selected_index, _)) = selected else {
                break;
            };
            let event = heads[selected_index]
                .take()
                .expect("selected merge head is present");
            let commit = self.apply_ordered(&event)?;
            callback(self, &event, &commit).map_err(ReplayError::Callback)?;
        }
        Ok(())
    }

    /// Applies one already-ordered event. Any failure leaves this event uncommitted.
    pub fn apply_ordered(&mut self, event: &DomainEvent) -> Result<CoreCommit, ReplayError> {
        let ordering_key = OrderingKey::for_event(event).map_err(ReplayError::Ordering)?;
        let prepared_checksum = self
            .event_stream
            .prepare_event(event)
            .map_err(ReplayError::CanonicalEncoding)?;
        self.validate_global_order(&ordering_key, prepared_checksum.canonical())?;

        if !self.states.contains_key(event.instrument()) {
            return Err(ReplayError::InstrumentOutsideUniverse);
        }

        let next_ordinal = self
            .clock
            .event_ordinal()
            .checked_add(1)
            .ok_or(ReplayError::EventOrdinalOverflow)?;
        let reducer_context = self
            .context_for_event(event.instrument(), event.match_time())
            .ok_or(ReplayError::EventOutsideContextSchedule)?
            .clone();
        let state = self
            .states
            .get_mut(event.instrument())
            .ok_or(ReplayError::InstrumentOutsideUniverse)?;
        let reducer = self
            .reducers
            .get(event.instrument())
            .ok_or(ReplayError::ReducerConfigurationMismatch)?;
        let proposal = reducer
            .propose(state, event, &reducer_context)
            .map_err(ReplayError::StateTransition)?;

        let transition = reducer
            .commit(state, proposal)
            .map_err(ReplayError::StateTransition)?;
        self.event_stream.append_prepared(&prepared_checksum);
        self.clock = ReplayClock::At {
            match_time: event.match_time(),
            event_ordinal: next_ordinal,
        };
        self.first_match_time.get_or_insert(event.match_time());
        self.last_ordering_key = Some(ordering_key.clone());
        self.last_canonical_event = Some(prepared_checksum.canonical().to_vec());

        Ok(CoreCommit {
            occurrence: EventOccurrence {
                run_event_ordinal: next_ordinal,
                event_fingerprint: ordering_key.event_fingerprint(),
                ordering_key,
                instrument_state_version: transition.new_version(),
            },
            transition,
        })
    }

    pub fn complete(self) -> Result<CompletedReplay, ReplayError> {
        let event_checksum = self.event_stream.checksum();
        let final_state_checksum =
            final_state_checksum(self.states.values()).map_err(ReplayError::FinalStateEncoding)?;
        let summary = ReplaySummary {
            event_count: self.event_stream.event_count(),
            first_match_time: self.first_match_time,
            last_match_time: self.clock.match_time(),
            event_checksum,
            final_state_checksum,
        };
        Ok(CompletedReplay {
            states: self.states,
            summary,
        })
    }

    fn validate_global_order(
        &self,
        current: &OrderingKey,
        canonical: &[u8],
    ) -> Result<(), ReplayError> {
        let Some(previous) = &self.last_ordering_key else {
            return Ok(());
        };
        match current.cmp(previous) {
            std::cmp::Ordering::Less => Err(ReplayError::GlobalOrderingRegression),
            std::cmp::Ordering::Equal
                if self.last_canonical_event.as_deref() != Some(canonical) =>
            {
                Err(ReplayError::EventFingerprintCollision)
            }
            std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => Ok(()),
        }
    }

    fn context_for_event(
        &self,
        instrument: &InstrumentId,
        match_time: MatchTime,
    ) -> Option<&ReducerContext> {
        if let Some(windows) = self.reducer_context_windows.get(instrument)
            && let Some(window) = windows
                .iter()
                .find(|window| window.start() <= match_time && match_time < window.end_exclusive())
        {
            return Some(window.context());
        }
        if self.reducer_context_windows.contains_key(instrument) {
            return None;
        }
        self.reducer_contexts.get(instrument)
    }
}

#[derive(Debug)]
pub struct CompletedReplay {
    states: BTreeMap<InstrumentId, MarketState>,
    summary: ReplaySummary,
}

impl CompletedReplay {
    #[must_use]
    pub fn state(&self, instrument: &InstrumentId) -> Option<&MarketState> {
        self.states.get(instrument)
    }

    pub fn states(&self) -> impl Iterator<Item = &MarketState> {
        self.states.values()
    }

    #[must_use]
    pub const fn summary(&self) -> &ReplaySummary {
        &self.summary
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplaySummary {
    event_count: u64,
    first_match_time: Option<MatchTime>,
    last_match_time: Option<MatchTime>,
    event_checksum: ReplayEventStreamChecksum,
    final_state_checksum: FinalStateChecksum,
}

impl ReplaySummary {
    #[must_use]
    pub const fn event_count(self) -> u64 {
        self.event_count
    }

    #[must_use]
    pub const fn first_match_time(self) -> Option<MatchTime> {
        self.first_match_time
    }

    #[must_use]
    pub const fn last_match_time(self) -> Option<MatchTime> {
        self.last_match_time
    }

    #[must_use]
    pub const fn event_checksum(self) -> ReplayEventStreamChecksum {
        self.event_checksum
    }

    #[must_use]
    pub const fn final_state_checksum(self) -> FinalStateChecksum {
        self.final_state_checksum
    }
}

#[derive(Debug)]
pub enum ReplayError {
    EmptyUniverse,
    DuplicateInstrument,
    ReducerConfigurationMismatch,
    InvalidContextSchedule,
    EventOutsideContextSchedule,
    StateTradingDateMismatch,
    InstrumentOutsideUniverse,
    Ordering(OrderingError),
    CanonicalEncoding(CanonicalEncodingError),
    GlobalOrderingRegression,
    EventFingerprintCollision,
    EventOrdinalOverflow,
    StateTransition(StateTransitionError),
    FinalStateEncoding(FinalStateEncodingError),
    PlanOutsideUniverse,
    PlanTradingDateMismatch,
    StreamOpen(Box<str>),
    Stream(Box<str>),
    Callback(Box<str>),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUniverse => formatter.write_str("replay universe must not be empty"),
            Self::DuplicateInstrument => {
                formatter.write_str("replay universe contains a duplicate instrument")
            }
            Self::ReducerConfigurationMismatch => {
                formatter.write_str("replay reducer configuration does not cover the universe")
            }
            Self::InvalidContextSchedule => {
                formatter.write_str("replay context schedule is invalid or overlaps")
            }
            Self::EventOutsideContextSchedule => {
                formatter.write_str("event falls outside the instrument replay context schedule")
            }
            Self::StateTradingDateMismatch => {
                formatter.write_str("initial state trading date does not match reducer context")
            }
            Self::InstrumentOutsideUniverse => {
                formatter.write_str("event instrument is outside replay universe")
            }
            Self::Ordering(error) => write!(formatter, "event ordering failed: {error}"),
            Self::CanonicalEncoding(error) => {
                write!(formatter, "canonical event encoding failed: {error}")
            }
            Self::GlobalOrderingRegression => formatter.write_str("global ordering key regressed"),
            Self::EventFingerprintCollision => {
                formatter.write_str("equal ordering keys have different canonical event bytes")
            }
            Self::EventOrdinalOverflow => formatter.write_str("replay event ordinal overflowed"),
            Self::StateTransition(error) => {
                write!(formatter, "market-state transition failed: {error}")
            }
            Self::FinalStateEncoding(error) => {
                write!(formatter, "final state encoding failed: {error}")
            }
            Self::PlanOutsideUniverse => {
                formatter.write_str("replay plan binding is outside initialized universe")
            }
            Self::PlanTradingDateMismatch => {
                formatter.write_str("replay plan binding trading date does not match state")
            }
            Self::StreamOpen(error) => write!(formatter, "event stream open failed: {error}"),
            Self::Stream(error) => write!(formatter, "event stream failed: {error}"),
            Self::Callback(error) => write!(formatter, "replay callback failed: {error}"),
        }
    }
}

impl Error for ReplayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Ordering(error) => Some(error),
            Self::CanonicalEncoding(error) => Some(error),
            Self::StateTransition(error) => Some(error),
            Self::FinalStateEncoding(error) => Some(error),
            _ => None,
        }
    }
}
