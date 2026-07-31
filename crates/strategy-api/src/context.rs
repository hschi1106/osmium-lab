use std::{error::Error, fmt};

use market_state::{MarketStateView, SessionSegmentId, StateField};
use market_types::{
    DomainEvent, EventPayload, InstantTrend, LimitPosition, MarketAnnotations, MarketId, MatchTime,
    MatchingMethod, TradingDate,
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
    IndicativeMarket,
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
pub enum IndicativeReason {
    PreOpenTrial,
    PreCloseTrial,
    DelayedOpen,
    DelayedClose,
    VolatilityInterruptionDown,
    VolatilityInterruptionUp,
    UnclassifiedTrial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchingState {
    Enabled(MatchingMethod),
    Indicative(IndicativeReason),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradingContext {
    event_fingerprint: [u8; 32],
    instrument_state_version: u64,
    session: SessionCallbackContext,
    new_order_entry: NewOrderEntry,
    matching: MatchingState,
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
    pub const fn market_rule_name(&self) -> &'static str {
        self.market_rule_name
    }

    #[must_use]
    pub const fn market_rule_version(&self) -> u16 {
        self.market_rule_version
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TwseTradingContextEvaluator;

impl TwseTradingContextEvaluator {
    pub fn evaluate(
        self,
        event: &DomainEvent,
        occurrence: &EventOccurrence,
        state: MarketStateView<'_>,
        segment: &SessionSegment,
    ) -> Result<TradingContext, ContextError> {
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

        let phase = segment.phase(event.match_time())?;
        let session = SessionCallbackContext {
            segment_id: segment.id().clone(),
            session_kind: segment.kind(),
            phase,
        };
        let annotations = match state.last_annotations() {
            StateField::Known {
                value: MarketAnnotations::TwseQuote(value),
                ..
            } => *value,
            StateField::Known {
                value: MarketAnnotations::None,
                ..
            }
            | StateField::Unavailable(_)
            | StateField::Unknown { .. } => return Err(ContextError::MissingTwseAnnotations),
        };
        let status = annotations.status();
        let limits = annotations.limits();
        let reserved = status.reserved_bits() != 0
            || [limits.trade(), limits.best_bid(), limits.best_ask()]
                .contains(&LimitPosition::Reserved)
            || limits.instant_trend() == InstantTrend::Reserved;

        let matching = if reserved {
            MatchingState::Unknown
        } else {
            match limits.instant_trend() {
                InstantTrend::VolatilityInterruptionDown => {
                    MatchingState::Indicative(IndicativeReason::VolatilityInterruptionDown)
                }
                InstantTrend::VolatilityInterruptionUp => {
                    MatchingState::Indicative(IndicativeReason::VolatilityInterruptionUp)
                }
                InstantTrend::Normal if status.trial() => {
                    let reason = if status.delayed_open() {
                        IndicativeReason::DelayedOpen
                    } else if status.delayed_close() {
                        IndicativeReason::DelayedClose
                    } else if phase == SessionPhase::WarmUp {
                        IndicativeReason::PreOpenTrial
                    } else if phase == SessionPhase::Active && event.match_time() < segment.close()
                    {
                        IndicativeReason::PreCloseTrial
                    } else {
                        IndicativeReason::UnclassifiedTrial
                    };
                    MatchingState::Indicative(reason)
                }
                InstantTrend::Normal => MatchingState::Enabled(status.matching_method()),
                InstantTrend::Reserved => unreachable!("reserved trend handled above"),
            }
        };

        let new_order_entry = if phase == SessionPhase::CoolDown {
            NewOrderEntry::Blocked(OrderBlockReason::CoolDown)
        } else if status.closing_marker() {
            NewOrderEntry::Blocked(OrderBlockReason::ClosingResult)
        } else if reserved {
            NewOrderEntry::Unknown
        } else if matching == MatchingState::Indicative(IndicativeReason::PreOpenTrial) {
            NewOrderEntry::Restricted(OrderRestrictionReason::PreOpenLimitOrdersOnly)
        } else if matches!(
            matching,
            MatchingState::Indicative(IndicativeReason::DelayedOpen)
                | MatchingState::Indicative(IndicativeReason::DelayedClose)
        ) {
            NewOrderEntry::Restricted(OrderRestrictionReason::IndicativeMarket)
        } else {
            NewOrderEntry::Allowed
        };

        Ok(TradingContext {
            event_fingerprint: *occurrence.event_fingerprint().as_bytes(),
            instrument_state_version: occurrence.instrument_state_version(),
            session,
            new_order_entry,
            matching,
            market_rule_name: "twse.quote-annotations",
            market_rule_version: 1,
        })
    }
}

/// Evaluates the market-specific trading rules used by the M3 multi-market run.
///
/// TWSE keeps its annotation-driven evaluator. TAIFEX events currently carry no
/// market annotation flags, so its context is derived only from the session
/// phase and explicit indicative-auction domain events.
#[derive(Debug, Default, Clone, Copy)]
pub struct MarketTradingContextEvaluator;

impl MarketTradingContextEvaluator {
    pub fn evaluate(
        self,
        event: &DomainEvent,
        occurrence: &EventOccurrence,
        state: MarketStateView<'_>,
        segment: &SessionSegment,
    ) -> Result<TradingContext, ContextError> {
        match event.instrument().market() {
            MarketId::Twse => {
                TwseTradingContextEvaluator.evaluate(event, occurrence, state, segment)
            }
            MarketId::Taifex => self.evaluate_taifex(event, occurrence, state, segment),
            market => Err(ContextError::UnsupportedMarket(market)),
        }
    }

    fn evaluate_taifex(
        self,
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
        let indicative = matches!(
            event.payload(),
            EventPayload::IndicativeOpeningAuction(_) | EventPayload::IndicativeClosingAuction(_)
        );
        let matching = if indicative {
            MatchingState::Indicative(IndicativeReason::UnclassifiedTrial)
        } else if phase == SessionPhase::WarmUp {
            MatchingState::Indicative(IndicativeReason::PreOpenTrial)
        } else {
            MatchingState::Enabled(MatchingMethod::Continuous)
        };
        let new_order_entry = if phase == SessionPhase::CoolDown {
            NewOrderEntry::Blocked(OrderBlockReason::CoolDown)
        } else if indicative {
            NewOrderEntry::Restricted(OrderRestrictionReason::IndicativeMarket)
        } else if phase == SessionPhase::WarmUp {
            NewOrderEntry::Restricted(OrderRestrictionReason::PreOpenLimitOrdersOnly)
        } else {
            NewOrderEntry::Allowed
        };

        Ok(TradingContext {
            event_fingerprint: *occurrence.event_fingerprint().as_bytes(),
            instrument_state_version: occurrence.instrument_state_version(),
            session,
            new_order_entry,
            matching,
            market_rule_name: "taifex.futures-session",
            market_rule_version: 1,
        })
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
    MissingTwseAnnotations,
    UnsupportedMarket(MarketId),
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
            Self::MissingTwseAnnotations => "TWSE trading context requires known TWSE annotations",
            Self::UnsupportedMarket(market) => {
                return write!(formatter, "unsupported trading-context market: {market:?}");
            }
        };
        formatter.write_str(message)
    }
}

impl Error for ContextError {}
