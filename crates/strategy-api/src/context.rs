use std::{error::Error, fmt};

use market_state::{MarketStateView, SessionSegmentId, StateField};
use market_types::{
    CompleteBookSnapshot, DomainEvent, EventPayload, IndicativeAuction, IndicativeAuctionKind,
    InstantTrend, LimitPosition, MarketAnnotations, MarketId, MatchTime, MatchingMethod, Price,
    Quantity, StabilityDirection, TpexQuoteAnnotations, TradingDate,
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

/// A normalized borrowed view over an equity indicative-auction observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquityIndicativeObservation<'a>(&'a IndicativeAuction);

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
    pub const fn kind(self) -> IndicativeAuctionKind {
        self.0.kind()
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

    #[must_use]
    pub const fn annotations(self) -> &'a MarketAnnotations {
        self.0.annotations()
    }
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
            | StateField::Known {
                value: MarketAnnotations::TpexQuote(_),
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
        } else if let Some(kind) = indicative_kind(event) {
            indicative_matching(kind, status.delayed_open(), status.delayed_close())
        } else {
            match limits.instant_trend() {
                InstantTrend::VolatilityInterruptionDown => {
                    MatchingState::Indicative(IndicativeReason::VolatilityInterruptionDown)
                }
                InstantTrend::VolatilityInterruptionUp => {
                    MatchingState::Indicative(IndicativeReason::VolatilityInterruptionUp)
                }
                InstantTrend::Normal if status.trial() => MatchingState::Unknown,
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
        } else if matches!(matching, MatchingState::Indicative(_)) {
            NewOrderEntry::Restricted(OrderRestrictionReason::IndicativeMarket)
        } else if matching == MatchingState::Unknown {
            NewOrderEntry::Unknown
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
            market_rule_version: 3,
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TpexTradingContextEvaluator;

impl TpexTradingContextEvaluator {
    pub fn evaluate(
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
        let annotations = match state.last_annotations() {
            StateField::Known {
                value: MarketAnnotations::TpexQuote(value),
                ..
            } => *value,
            StateField::Known {
                value: MarketAnnotations::None,
                ..
            }
            | StateField::Known {
                value: MarketAnnotations::TwseQuote(_),
                ..
            }
            | StateField::Unavailable(_)
            | StateField::Unknown { .. } => return Err(ContextError::MissingTpexAnnotations),
        };
        let matching = evaluate_annotated_matching(event, annotations);
        let new_order_entry = if phase == SessionPhase::CoolDown {
            NewOrderEntry::Blocked(OrderBlockReason::CoolDown)
        } else if annotations.status().closing_marker() {
            NewOrderEntry::Blocked(OrderBlockReason::ClosingResult)
        } else if matching == MatchingState::Indicative(IndicativeReason::PreOpenTrial) {
            NewOrderEntry::Restricted(OrderRestrictionReason::PreOpenLimitOrdersOnly)
        } else if matches!(matching, MatchingState::Indicative(_)) {
            NewOrderEntry::Restricted(OrderRestrictionReason::IndicativeMarket)
        } else if matching == MatchingState::Unknown {
            NewOrderEntry::Unknown
        } else {
            NewOrderEntry::Allowed
        };
        Ok(TradingContext {
            event_fingerprint: *occurrence.event_fingerprint().as_bytes(),
            instrument_state_version: occurrence.instrument_state_version(),
            session,
            new_order_entry,
            matching,
            market_rule_name: "tpex.quote-annotations",
            market_rule_version: 4,
        })
    }
}

fn evaluate_annotated_matching(
    event: &DomainEvent,
    annotations: TpexQuoteAnnotations,
) -> MatchingState {
    let status = annotations.status();
    let limits = annotations.limits();
    let reserved = status.reserved_bits() != 0
        || [limits.trade(), limits.best_bid(), limits.best_ask()]
            .contains(&LimitPosition::Reserved)
        || limits.instant_trend() == InstantTrend::Reserved;
    if reserved {
        return MatchingState::Unknown;
    }
    if let Some(kind) = indicative_kind(event) {
        return indicative_matching(kind, status.delayed_open(), status.delayed_close());
    }
    match limits.instant_trend() {
        InstantTrend::VolatilityInterruptionDown => {
            MatchingState::Indicative(IndicativeReason::VolatilityInterruptionDown)
        }
        InstantTrend::VolatilityInterruptionUp => {
            MatchingState::Indicative(IndicativeReason::VolatilityInterruptionUp)
        }
        InstantTrend::Normal if status.trial() => MatchingState::Unknown,
        InstantTrend::Normal => MatchingState::Enabled(status.matching_method()),
        InstantTrend::Reserved => unreachable!("reserved trend handled above"),
    }
}

fn indicative_kind(event: &DomainEvent) -> Option<IndicativeAuctionKind> {
    match event.payload() {
        EventPayload::IndicativeAuction(auction) => Some(auction.kind()),
        EventPayload::QuoteSnapshot(_)
        | EventPayload::BookSnapshot(_)
        | EventPayload::TradeBatch(_)
        | EventPayload::MarketStatus(_) => None,
    }
}

const fn indicative_matching(
    kind: IndicativeAuctionKind,
    delayed_open: bool,
    delayed_close: bool,
) -> MatchingState {
    match kind {
        _ if delayed_open && delayed_close => MatchingState::Unknown,
        IndicativeAuctionKind::Opening if delayed_close => MatchingState::Unknown,
        IndicativeAuctionKind::Opening if delayed_open => {
            MatchingState::Indicative(IndicativeReason::DelayedOpen)
        }
        IndicativeAuctionKind::Opening => MatchingState::Indicative(IndicativeReason::PreOpenTrial),
        IndicativeAuctionKind::Closing if delayed_open => MatchingState::Unknown,
        IndicativeAuctionKind::Closing if delayed_close => {
            MatchingState::Indicative(IndicativeReason::DelayedClose)
        }
        IndicativeAuctionKind::Closing => {
            MatchingState::Indicative(IndicativeReason::PreCloseTrial)
        }
        IndicativeAuctionKind::IntradayStability {
            direction: StabilityDirection::Down,
        } if !delayed_open && !delayed_close => {
            MatchingState::Indicative(IndicativeReason::VolatilityInterruptionDown)
        }
        IndicativeAuctionKind::IntradayStability {
            direction: StabilityDirection::Up,
        } if !delayed_open && !delayed_close => {
            MatchingState::Indicative(IndicativeReason::VolatilityInterruptionUp)
        }
        IndicativeAuctionKind::IntradayUnclassified if !delayed_open && !delayed_close => {
            MatchingState::Indicative(IndicativeReason::UnclassifiedTrial)
        }
        IndicativeAuctionKind::IntradayStability { .. }
        | IndicativeAuctionKind::IntradayUnclassified => MatchingState::Unknown,
    }
}

/// Evaluates the market-specific trading rules used by a multi-market run.
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
            MarketId::Tpex => {
                TpexTradingContextEvaluator.evaluate(event, occurrence, state, segment)
            }
            MarketId::Taifex => self.evaluate_taifex(event, occurrence, state, segment),
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
        let indicative_kind = indicative_kind(event);
        let matching = if let Some(kind) = indicative_kind {
            indicative_matching(kind, false, false)
        } else if phase == SessionPhase::WarmUp {
            MatchingState::Indicative(IndicativeReason::PreOpenTrial)
        } else {
            MatchingState::Enabled(MatchingMethod::Continuous)
        };
        let new_order_entry = if phase == SessionPhase::CoolDown {
            NewOrderEntry::Blocked(OrderBlockReason::CoolDown)
        } else if indicative_kind.is_some() {
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
            market_rule_version: 2,
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
    MissingTpexAnnotations,
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
            Self::MissingTpexAnnotations => "TPEx trading context requires known TPEx annotations",
            Self::UnsupportedMarket(market) => {
                return write!(formatter, "unsupported trading-context market: {market:?}");
            }
        };
        formatter.write_str(message)
    }
}

impl Error for ContextError {}

#[cfg(test)]
mod tests {
    use market_state::{
        MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
    };
    use market_types::{
        BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, InstrumentId, MarketAnnotations,
        Observation, ObservedTrade, Price, Quantity, QuantityUnit, QuoteSnapshot, SourceFormatId,
        Symbol, TpexQuoteAnnotations, TradeObservationKind, TwseQuoteAnnotations,
    };
    use replay_engine::ReplayCore;

    use super::*;

    fn tpex_event(time: &str, status: u8) -> (DomainEvent, TpexQuoteAnnotations) {
        tpex_event_with_limits(time, status, 0)
    }

    fn twse_event_with_limits(
        time: &str,
        status: u8,
        limit_flags: u8,
    ) -> (DomainEvent, TwseQuoteAnnotations) {
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
        let annotations = TwseQuoteAnnotations::new(status, limit_flags);
        let event = DomainEvent::new(
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            TradingDate::parse("2026-06-23").unwrap(),
            SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
            MatchTime::parse(time).unwrap(),
            None,
            EventPayload::QuoteSnapshot(
                QuoteSnapshot::new(
                    book,
                    Observation::NoObservation,
                    Observation::NoObservation,
                    MarketAnnotations::TwseQuote(annotations),
                )
                .unwrap(),
            ),
        );
        (event, annotations)
    }

    fn tpex_event_with_limits(
        time: &str,
        status: u8,
        limit_flags: u8,
    ) -> (DomainEvent, TpexQuoteAnnotations) {
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
        let annotations = TpexQuoteAnnotations::new(status, limit_flags);
        let payload = if annotations.status().trial() {
            let kind = if annotations.status().opening_marker() {
                IndicativeAuctionKind::Opening
            } else if annotations.status().closing_marker() {
                IndicativeAuctionKind::Closing
            } else {
                IndicativeAuctionKind::IntradayUnclassified
            };
            EventPayload::IndicativeAuction(
                IndicativeAuction::new(
                    kind,
                    Observation::Set(Price::parse("100").unwrap()),
                    Observation::Set(quantity),
                    Observation::Set(book),
                    Observation::NoObservation,
                    MarketAnnotations::TpexQuote(annotations),
                )
                .unwrap(),
            )
        } else {
            EventPayload::QuoteSnapshot(
                QuoteSnapshot::new(
                    book,
                    Observation::Set(ObservedTrade::new(
                        Price::parse("100").unwrap(),
                        quantity,
                        TradeObservationKind::Regular,
                    )),
                    Observation::NoObservation,
                    MarketAnnotations::TpexQuote(annotations),
                )
                .unwrap(),
            )
        };
        let event = DomainEvent::new(
            InstrumentId::new(MarketId::Tpex, Symbol::new("3374").unwrap()),
            TradingDate::parse("2026-06-23").unwrap(),
            SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
            MatchTime::parse(time).unwrap(),
            None,
            payload,
        );
        (event, annotations)
    }

    fn segment() -> SessionSegment {
        SessionSegment::new(
            SessionSegmentId::new("regular").unwrap(),
            SessionKind::Regular,
            TradingDate::parse("2026-06-23").unwrap(),
            MatchTime::parse("2026-06-23T09:00:00+08:00").unwrap(),
            MatchTime::parse("2026-06-23T13:30:00+08:00").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn tpex_formal_auction_markers_are_enabled_but_trial_markers_are_indicative() {
        let cases = [
            (
                "2026-06-23T08:59:59+08:00",
                0x88,
                MatchingState::Indicative(IndicativeReason::PreOpenTrial),
            ),
            (
                "2026-06-23T08:59:59+08:00",
                0xc8,
                MatchingState::Indicative(IndicativeReason::DelayedOpen),
            ),
            (
                "2026-06-23T09:00:00.145482+08:00",
                0x08,
                MatchingState::Enabled(MatchingMethod::CallAuction),
            ),
            (
                "2026-06-23T13:29:59+08:00",
                0x84,
                MatchingState::Indicative(IndicativeReason::PreCloseTrial),
            ),
            (
                "2026-06-23T13:29:59+08:00",
                0xa4,
                MatchingState::Indicative(IndicativeReason::DelayedClose),
            ),
            (
                "2026-06-23T13:30:00+08:00",
                0x04,
                MatchingState::Enabled(MatchingMethod::CallAuction),
            ),
        ];

        for (time, status, expected) in cases {
            let (event, annotations) = tpex_event(time, status);
            assert_eq!(evaluate_annotated_matching(&event, annotations), expected);
        }
    }

    #[test]
    fn tpex_stability_pause_restricts_new_orders_to_limit_rod() {
        let (event, _) = tpex_event_with_limits("2026-06-23T09:01:00+08:00", 0x10, 0x01);
        let instrument = event.instrument().clone();
        let date = event.trading_date();
        let segment_id = SessionSegmentId::new("regular").unwrap();
        let mut core = ReplayCore::new(
            vec![MarketState::new(instrument, date)],
            MarketStateReducer::tpex_regular(),
            ReducerContext::new(date, segment_id, SegmentBoundaryPolicy::Carry, 1),
        )
        .unwrap();
        let commit = core.apply_ordered(&event).unwrap();
        let state = core.state(event.instrument()).unwrap().view();
        let context = TpexTradingContextEvaluator
            .evaluate(&event, commit.occurrence(), state, &segment())
            .unwrap();

        assert_eq!(
            context.matching(),
            MatchingState::Indicative(IndicativeReason::VolatilityInterruptionDown)
        );
        assert_eq!(
            context.new_order_entry(),
            NewOrderEntry::Restricted(OrderRestrictionReason::IndicativeMarket)
        );
    }

    #[test]
    fn twse_stability_pause_restricts_new_orders_to_limit_rod() {
        let (event, _) = twse_event_with_limits("2026-06-23T09:01:00+08:00", 0x10, 0x02);
        let instrument = event.instrument().clone();
        let date = event.trading_date();
        let segment_id = SessionSegmentId::new("regular").unwrap();
        let mut core = ReplayCore::new(
            vec![MarketState::new(instrument, date)],
            MarketStateReducer::twse_regular(),
            ReducerContext::new(date, segment_id, SegmentBoundaryPolicy::Carry, 1),
        )
        .unwrap();
        let commit = core.apply_ordered(&event).unwrap();
        let state = core.state(event.instrument()).unwrap().view();
        let context = TwseTradingContextEvaluator
            .evaluate(&event, commit.occurrence(), state, &segment())
            .unwrap();

        assert_eq!(
            context.matching(),
            MatchingState::Indicative(IndicativeReason::VolatilityInterruptionUp)
        );
        assert_eq!(
            context.new_order_entry(),
            NewOrderEntry::Restricted(OrderRestrictionReason::IndicativeMarket)
        );
    }

    #[test]
    fn intraday_trial_is_unclassified_and_exposed_as_an_indicative_observation() {
        let (event, annotations) = tpex_event_with_limits("2026-06-23T09:04:43+08:00", 0x80, 0x02);
        assert_eq!(
            evaluate_annotated_matching(&event, annotations),
            MatchingState::Indicative(IndicativeReason::UnclassifiedTrial)
        );
        assert_eq!(
            annotations.limits().instant_trend(),
            InstantTrend::VolatilityInterruptionUp
        );
        let observation = EquityIndicativeObservation::from_event(&event).unwrap();
        assert_eq!(
            observation.kind(),
            IndicativeAuctionKind::IntradayUnclassified
        );
        assert_eq!(
            observation.price().unwrap().atoms(),
            100_000_000_000_000_000_000
        );
        assert_eq!(observation.quantity().unwrap().value(), 1);
        assert!(observation.book().is_some());
    }

    #[test]
    fn typed_indicative_kind_controls_reason_before_annotation_markers() {
        assert_eq!(
            indicative_matching(IndicativeAuctionKind::Opening, true, false),
            MatchingState::Indicative(IndicativeReason::DelayedOpen)
        );
        assert_eq!(
            indicative_matching(IndicativeAuctionKind::Closing, true, false),
            MatchingState::Unknown
        );
        assert_eq!(
            indicative_matching(
                IndicativeAuctionKind::IntradayStability {
                    direction: StabilityDirection::Down,
                },
                false,
                false,
            ),
            MatchingState::Indicative(IndicativeReason::VolatilityInterruptionDown)
        );
        assert_eq!(
            indicative_matching(IndicativeAuctionKind::IntradayUnclassified, true, true),
            MatchingState::Unknown
        );
    }
}
