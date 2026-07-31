use std::{collections::BTreeMap, error::Error, fmt};

use market_state::{
    FinalStateChecksum, FinalStateEncodingError, MarketState, MarketStateReducer, ReducerContext,
    StateTransitionError, TransitionReceipt, final_state_checksum,
};
use market_types::{
    CanonicalEncodingError, DomainEvent, EventFingerprint, InstrumentId, MatchTime,
};

use crate::{
    OrderingError, OrderingKey, ReplayEventStreamChecksum, checksum::ReplayEventStreamHasher,
    order_events,
};

pub const REPLAY_ENGINE_VERSION: u16 = 1;

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

/// M1 replay core: ordered events, atomic state transitions, logical clock, and checksums.
#[derive(Debug)]
pub struct ReplayCore {
    states: BTreeMap<InstrumentId, MarketState>,
    reducer: MarketStateReducer,
    reducer_context: ReducerContext,
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
        if states.is_empty() {
            return Err(ReplayError::EmptyUniverse);
        }
        let mut by_instrument = BTreeMap::new();
        for state in states {
            if state.trading_date() != reducer_context.trading_date() {
                return Err(ReplayError::StateTradingDateMismatch);
            }
            if by_instrument
                .insert(state.instrument().clone(), state)
                .is_some()
            {
                return Err(ReplayError::DuplicateInstrument);
            }
        }
        Ok(Self {
            states: by_instrument,
            reducer,
            reducer_context,
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

    /// Applies one already-ordered event. Any failure leaves this event uncommitted.
    pub fn apply_ordered(&mut self, event: &DomainEvent) -> Result<CoreCommit, ReplayError> {
        let ordering_key = OrderingKey::for_event(event).map_err(ReplayError::Ordering)?;
        let prepared_checksum = self
            .event_stream
            .prepare_event(event)
            .map_err(ReplayError::CanonicalEncoding)?;
        self.validate_global_order(&ordering_key, prepared_checksum.canonical())?;

        let next_ordinal = self
            .clock
            .event_ordinal()
            .checked_add(1)
            .ok_or(ReplayError::EventOrdinalOverflow)?;
        let state = self
            .states
            .get_mut(event.instrument())
            .ok_or(ReplayError::InstrumentOutsideUniverse)?;
        let proposal = self
            .reducer
            .propose(state, event, &self.reducer_context)
            .map_err(ReplayError::StateTransition)?;

        let transition = self
            .reducer
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
    StateTradingDateMismatch,
    InstrumentOutsideUniverse,
    Ordering(OrderingError),
    CanonicalEncoding(CanonicalEncodingError),
    GlobalOrderingRegression,
    EventFingerprintCollision,
    EventOrdinalOverflow,
    StateTransition(StateTransitionError),
    FinalStateEncoding(FinalStateEncodingError),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUniverse => formatter.write_str("replay universe must not be empty"),
            Self::DuplicateInstrument => {
                formatter.write_str("replay universe contains a duplicate instrument")
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
