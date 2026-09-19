use std::{error::Error, fmt};

use market_state::{MarketPhase, MarketStateView, SessionSegmentId, StateField};
use market_types::{
    AuctionObservation, AuctionPurpose, CompleteBookSnapshot, DomainEvent, EventPayload, MarketId,
    MarketSignal, MatchTime, MatchingMethod, Observation, Price, Quantity, TradingDate,
};
use replay_engine::EventOccurrence;

const WARM_UP_MICROSECONDS: i64 = 5 * 60 * 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum SessionKind {
    Regular = 1,
    AfterHours = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    WarmUp,
    Active,
    CoolDown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSegment {
    id: SessionSegmentId,
    kind: SessionKind,
    trading_date: TradingDate,
    open: MatchTime,
    close: MatchTime,
}

impl SessionSegment {
    pub fn new(
        id: SessionSegmentId,
        kind: SessionKind,
        trading_date: TradingDate,
        open: MatchTime,
        close: MatchTime,
    ) -> Result<Self, ContextError> {
        if open >= close {
            return Err(ContextError::InvalidSessionWindow);
        }
        open.as_unix_microseconds()
            .checked_sub(WARM_UP_MICROSECONDS)
            .ok_or(ContextError::SessionWindowOverflow)?;
        close
            .as_unix_microseconds()
            .checked_add(WARM_UP_MICROSECONDS)
            .ok_or(ContextError::SessionWindowOverflow)?;
        Ok(Self {
            id,
            kind,
            trading_date,
            open,
            close,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &SessionSegmentId {
        &self.id
    }

    #[must_use]
    pub const fn kind(&self) -> SessionKind {
        self.kind
    }

    #[must_use]
    pub const fn trading_date(&self) -> TradingDate {
        self.trading_date
    }

    #[must_use]
    pub const fn open(&self) -> MatchTime {
        self.open
    }

    #[must_use]
    pub const fn close(&self) -> MatchTime {
        self.close
    }

    pub fn phase(&self, match_time: MatchTime) -> Result<SessionPhase, ContextError> {
        let value = match_time.as_unix_microseconds();
        let replay_start = self
            .open
            .as_unix_microseconds()
            .checked_sub(WARM_UP_MICROSECONDS)
            .ok_or(ContextError::SessionWindowOverflow)?;
        let replay_end = self
            .close
            .as_unix_microseconds()
            .checked_add(WARM_UP_MICROSECONDS)
            .ok_or(ContextError::SessionWindowOverflow)?;
        if value < replay_start || value >= replay_end {
            return Err(ContextError::OutsideReplayWindow);
        }
        if match_time < self.open {
            Ok(SessionPhase::WarmUp)
        } else if match_time <= self.close {
            Ok(SessionPhase::Active)
        } else {
            Ok(SessionPhase::CoolDown)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCallbackContext {
    segment_id: SessionSegmentId,
    session_kind: SessionKind,
    phase: SessionPhase,
}

impl SessionCallbackContext {
    #[must_use]
    pub const fn segment_id(&self) -> &SessionSegmentId {
        &self.segment_id
    }

    #[must_use]
    pub const fn session_kind(&self) -> SessionKind {
        self.session_kind
    }

    #[must_use]
    pub const fn phase(&self) -> SessionPhase {
        self.phase
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderRestrictionReason {
    PreOpenLimitOrdersOnly,
    AuctionCollecting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderBlockReason {
    CoolDown,
    ClosingResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewOrderEntry {
    Allowed,
    Restricted(OrderRestrictionReason),
    Blocked(OrderBlockReason),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchingState {
    Enabled(MatchingMethod),
    Closed,
    Unknown,
}

/// A normalized borrowed view over an indicative call-auction observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquityIndicativeObservation<'a>(&'a market_types::IndicativeAuction);

impl<'a> EquityIndicativeObservation<'a> {
    #[must_use]
    pub fn from_event(event: &'a DomainEvent) -> Option<Self> {
        if !matches!(event.instrument().market(), MarketId::Twse | MarketId::Tpex) {
            return None;
        }
        match event.payload() {
            EventPayload::IndicativeAuction(auction) => Some(Self(auction)),
            _ => None,
        }
    }

    #[must_use]
    pub const fn observation(self) -> AuctionObservation {
        self.0.observation()
    }

    #[must_use]
    pub fn price(self) -> Option<Price> {
        self.0.price().as_set().copied()
    }

    #[must_use]
    pub fn quantity(self) -> Option<Quantity> {
        self.0.quantity().as_set().copied()
    }

    #[must_use]
    pub fn book(self) -> Option<&'a CompleteBookSnapshot> {
        self.0.book().as_set()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradingContext {
    event_fingerprint: [u8; 32],
    instrument_state_version: u64,
    session: SessionCallbackContext,
    new_order_entry: NewOrderEntry,
    matching: MatchingState,
    auction: Option<AuctionObservation>,
    market_signal: Option<MarketSignal>,
    market_rule_name: &'static str,
    market_rule_version: u16,
}

impl TradingContext {
    #[must_use]
    pub const fn event_fingerprint(&self) -> &[u8; 32] {
        &self.event_fingerprint
    }

    #[must_use]
    pub const fn instrument_state_version(&self) -> u64 {
        self.instrument_state_version
    }

    #[must_use]
    pub const fn session(&self) -> &SessionCallbackContext {
        &self.session
    }

    #[must_use]
    pub const fn new_order_entry(&self) -> NewOrderEntry {
        self.new_order_entry
    }

    #[must_use]
    pub const fn matching(&self) -> MatchingState {
        self.matching
    }

    #[must_use]
    pub const fn auction(&self) -> Option<AuctionObservation> {
        self.auction
    }

    #[must_use]
    pub const fn market_signal(&self) -> Option<MarketSignal> {
        self.market_signal
    }

    #[must_use]
    pub const fn market_rule_name(&self) -> &'static str {
        self.market_rule_name
    }

    #[must_use]
    pub const fn market_rule_version(&self) -> u16 {
        self.market_rule_version
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MarketTradingContextEvaluator;

impl MarketTradingContextEvaluator {
    pub fn evaluate(
        event: &DomainEvent,
        occurrence: &EventOccurrence,
        state: MarketStateView<'_>,
        segment: &SessionSegment,
    ) -> Result<TradingContext, ContextError> {
        validate_event_state(event, occurrence, state, segment)?;
        let phase = segment.phase(event.match_time())?;
        let session = SessionCallbackContext {
            segment_id: segment.id().clone(),
            session_kind: segment.kind(),
            phase,
        };
        let signal = event.payload().market_signal();
        let (matching, auction) = matching_for_signal(&signal, state.phase());
        let new_order_entry = order_entry_for_signal(&signal, matching, auction, phase);

        Ok(TradingContext {
            event_fingerprint: *occurrence.event_fingerprint().as_bytes(),
            instrument_state_version: occurrence.instrument_state_version(),
            session,
            new_order_entry,
            matching,
            auction,
            market_signal: signal.as_set().copied(),
            market_rule_name: "market.signal",
            market_rule_version: 2,
        })
    }
}

fn matching_for_signal(
    signal: &Observation<MarketSignal>,
    state_phase: &StateField<MarketPhase>,
) -> (MatchingState, Option<AuctionObservation>) {
    match signal {
        Observation::Set(MarketSignal::Continuous) => {
            (MatchingState::Enabled(MatchingMethod::Continuous), None)
        }
        Observation::Set(MarketSignal::AuctionCollecting(observation))
        | Observation::Set(MarketSignal::AuctionUncross(observation)) => (
            MatchingState::Enabled(MatchingMethod::CallAuction),
            Some(*observation),
        ),
        Observation::Set(MarketSignal::Closed) => (MatchingState::Closed, None),
        Observation::NoObservation => match state_phase.known() {
            Some(MarketPhase::Continuous) => {
                (MatchingState::Enabled(MatchingMethod::Continuous), None)
            }
            Some(MarketPhase::Auction(auction)) => (
                MatchingState::Enabled(MatchingMethod::CallAuction),
                Some(auction.observation()),
            ),
            Some(MarketPhase::Closed) => (MatchingState::Closed, None),
            None => (MatchingState::Unknown, None),
        },
        Observation::Clear | Observation::Unknown(_) => (MatchingState::Unknown, None),
    }
}

fn order_entry_for_signal(
    signal: &Observation<MarketSignal>,
    matching: MatchingState,
    auction: Option<AuctionObservation>,
    session_phase: SessionPhase,
) -> NewOrderEntry {
    if session_phase == SessionPhase::CoolDown {
        return NewOrderEntry::Blocked(OrderBlockReason::CoolDown);
    }
    match signal {
        Observation::Set(MarketSignal::AuctionCollecting(observation)) => {
            if observation.purpose() == AuctionPurpose::Opening {
                NewOrderEntry::Restricted(OrderRestrictionReason::PreOpenLimitOrdersOnly)
            } else {
                NewOrderEntry::Restricted(OrderRestrictionReason::AuctionCollecting)
            }
        }
        Observation::Set(MarketSignal::AuctionUncross(observation)) => {
            match observation.purpose() {
                AuctionPurpose::Closing => NewOrderEntry::Blocked(OrderBlockReason::ClosingResult),
                AuctionPurpose::Periodic => {
                    NewOrderEntry::Restricted(OrderRestrictionReason::AuctionCollecting)
                }
                AuctionPurpose::Opening | AuctionPurpose::VolatilityInterruption => {
                    NewOrderEntry::Allowed
                }
            }
        }
        Observation::Set(MarketSignal::Closed) => {
            NewOrderEntry::Blocked(OrderBlockReason::ClosingResult)
        }
        Observation::Set(MarketSignal::Continuous) => NewOrderEntry::Allowed,
        Observation::NoObservation => match auction {
            Some(observation) if observation.purpose() == AuctionPurpose::Opening => {
                NewOrderEntry::Restricted(OrderRestrictionReason::PreOpenLimitOrdersOnly)
            }
            Some(_) => NewOrderEntry::Restricted(OrderRestrictionReason::AuctionCollecting),
            None if matching == MatchingState::Closed => {
                NewOrderEntry::Blocked(OrderBlockReason::ClosingResult)
            }
            None if matches!(matching, MatchingState::Enabled(_)) => NewOrderEntry::Allowed,
            None => NewOrderEntry::Unknown,
        },
        Observation::Clear | Observation::Unknown(_) => NewOrderEntry::Unknown,
    }
}

fn validate_event_state(
    event: &DomainEvent,
    occurrence: &EventOccurrence,
    state: MarketStateView<'_>,
    segment: &SessionSegment,
) -> Result<(), ContextError> {
    if event.trading_date() != segment.trading_date()
        || state.trading_date() != segment.trading_date()
    {
        return Err(ContextError::TradingDateMismatch);
    }
    if event.instrument() != state.instrument() {
        return Err(ContextError::InstrumentMismatch);
    }
    if state.current_segment_id() != Some(segment.id()) {
        return Err(ContextError::SegmentMismatch);
    }
    if occurrence.instrument_state_version() != state.state_version() {
        return Err(ContextError::StateVersionMismatch);
    }
    let last_event = state
        .last_event()
        .ok_or(ContextError::MissingAppliedEvent)?;
    if last_event.event_fingerprint() != occurrence.event_fingerprint()
        || last_event.match_time() != event.match_time()
    {
        return Err(ContextError::EventIdentityMismatch);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextError {
    InvalidSessionWindow,
    SessionWindowOverflow,
    OutsideReplayWindow,
    TradingDateMismatch,
    InstrumentMismatch,
    SegmentMismatch,
    StateVersionMismatch,
    MissingAppliedEvent,
    EventIdentityMismatch,
}

impl fmt::Display for ContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidSessionWindow => "session open must be earlier than close",
            Self::SessionWindowOverflow => "session warm-up or cool-down window overflowed",
            Self::OutsideReplayWindow => "event is outside the session replay window",
            Self::TradingDateMismatch => "event, state, and session trading dates differ",
            Self::InstrumentMismatch => "event and state instruments differ",
            Self::SegmentMismatch => "state and callback session segments differ",
            Self::StateVersionMismatch => "occurrence and post-event state versions differ",
            Self::MissingAppliedEvent => "post-event state has no applied event identity",
            Self::EventIdentityMismatch => "event, occurrence, and state identities differ",
        };
        formatter.write_str(message)
    }
}

impl Error for ContextError {}
