use std::{cmp::Ordering, error::Error, fmt, str::FromStr};

use market_types::{
    CompleteBookSnapshot, DomainEvent, EventFingerprint, EventKind, InstrumentId,
    MarketAnnotations, MatchTime, SourceFormatId, TradeOrder, TradePrint, TradingDate,
    UnknownValue, Volume,
};

/// A non-empty, byte-exact session segment identifier from a market profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionSegmentId(Box<str>);

impl SessionSegmentId {
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, SessionSegmentIdError> {
        let value = value.into();
        if value.is_empty() {
            Err(SessionSegmentIdError::Empty)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl PartialOrd for SessionSegmentId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SessionSegmentId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl fmt::Display for SessionSegmentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SessionSegmentId {
    type Err = SessionSegmentIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSegmentIdError {
    Empty,
}

impl fmt::Display for SessionSegmentIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("session segment identifier must not be empty"),
        }
    }
}

impl Error for SessionSegmentIdError {}

/// Stable identity for one event applied to state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppliedEventRef {
    match_time: MatchTime,
    source_format: SourceFormatId,
    source_phase: u8,
    event_kind: EventKind,
    source_sequence: Option<u64>,
    event_fingerprint: EventFingerprint,
}

impl AppliedEventRef {
    pub fn from_event(event: &DomainEvent) -> Result<Self, market_types::CanonicalEncodingError> {
        Ok(Self {
            match_time: event.match_time(),
            source_format: event.source_format().clone(),
            source_phase: source_phase_for_event(event),
            event_kind: event.payload().kind(),
            source_sequence: event.source_sequence(),
            event_fingerprint: event.fingerprint()?,
        })
    }

    #[must_use]
    pub const fn match_time(&self) -> MatchTime {
        self.match_time
    }

    #[must_use]
    pub const fn source_format(&self) -> &SourceFormatId {
        &self.source_format
    }

    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.event_kind
    }

    #[must_use]
    pub const fn source_phase(&self) -> u8 {
        self.source_phase
    }

    #[must_use]
    pub const fn source_sequence(&self) -> Option<u64> {
        self.source_sequence
    }

    #[must_use]
    pub const fn event_fingerprint(&self) -> EventFingerprint {
        self.event_fingerprint
    }
}

fn source_phase_for_event(event: &DomainEvent) -> u8 {
    if event.instrument().market() != market_types::MarketId::Twse
        || event.source_format().as_str() != "STOCK_REALTIME"
    {
        return 0;
    }
    match event.payload() {
        market_types::EventPayload::TradeBatch(_) => 10,
        market_types::EventPayload::QuoteSnapshot(_)
        | market_types::EventPayload::BookSnapshot(_) => 20,
        market_types::EventPayload::IndicativeOpeningAuction(auction)
        | market_types::EventPayload::IndicativeClosingAuction(auction) => {
            if auction.book().as_set().is_some() {
                20
            } else {
                10
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnavailableReason {
    Initial,
    Cleared { cleared_at: AppliedEventRef },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StateField<T> {
    Unavailable(UnavailableReason),
    Known {
        value: T,
        observed_at: AppliedEventRef,
    },
    Unknown {
        raw: UnknownValue,
        observed_at: AppliedEventRef,
    },
}

impl<T> StateField<T> {
    #[must_use]
    pub const fn initial() -> Self {
        Self::Unavailable(UnavailableReason::Initial)
    }

    #[must_use]
    pub const fn known(&self) -> Option<&T> {
        match self {
            Self::Known { value, .. } => Some(value),
            Self::Unavailable(_) | Self::Unknown { .. } => None,
        }
    }

    #[must_use]
    pub const fn observed_at(&self) -> Option<&AppliedEventRef> {
        match self {
            Self::Known { observed_at, .. } | Self::Unknown { observed_at, .. } => {
                Some(observed_at)
            }
            Self::Unavailable(UnavailableReason::Cleared { cleared_at }) => Some(cleared_at),
            Self::Unavailable(UnavailableReason::Initial) => None,
        }
    }
}

/// The latest source-observed trade value, never an unbounded history.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TradeObservation {
    Single(TradePrint),
    Batch {
        trades: Box<[TradePrint]>,
        trade_order: TradeOrder,
    },
}

impl TradeObservation {
    pub fn batch(trades: Vec<TradePrint>, trade_order: TradeOrder) -> Result<Self, ModelError> {
        if trades.is_empty() {
            Err(ModelError::EmptyTradeBatch)
        } else {
            Ok(Self::Batch {
                trades: trades.into_boxed_slice(),
                trade_order,
            })
        }
    }

    #[must_use]
    pub const fn trades(&self) -> &[TradePrint] {
        match self {
            Self::Single(trade) => std::slice::from_ref(trade),
            Self::Batch { trades, .. } => trades,
        }
    }

    #[must_use]
    pub const fn trade_order(&self) -> Option<TradeOrder> {
        match self {
            Self::Single(_) => None,
            Self::Batch { trade_order, .. } => Some(*trade_order),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelError {
    EmptyTradeBatch,
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTradeBatch => {
                formatter.write_str("trade observation batch must not be empty")
            }
        }
    }
}

impl Error for ModelError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketState {
    instrument: InstrumentId,
    trading_date: TradingDate,
    current_segment_id: Option<SessionSegmentId>,
    book: StateField<CompleteBookSnapshot>,
    recent_trade: StateField<TradeObservation>,
    cumulative_volume: StateField<Volume>,
    last_annotations: StateField<MarketAnnotations>,
    last_event: Option<AppliedEventRef>,
    state_version: u64,
}

impl MarketState {
    #[must_use]
    pub const fn new(instrument: InstrumentId, trading_date: TradingDate) -> Self {
        Self {
            instrument,
            trading_date,
            current_segment_id: None,
            book: StateField::initial(),
            recent_trade: StateField::initial(),
            cumulative_volume: StateField::initial(),
            last_annotations: StateField::initial(),
            last_event: None,
            state_version: 0,
        }
    }

    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn trading_date(&self) -> TradingDate {
        self.trading_date
    }

    #[must_use]
    pub const fn current_segment_id(&self) -> Option<&SessionSegmentId> {
        self.current_segment_id.as_ref()
    }

    #[must_use]
    pub const fn state_version(&self) -> u64 {
        self.state_version
    }

    #[must_use]
    pub const fn last_event(&self) -> Option<&AppliedEventRef> {
        self.last_event.as_ref()
    }

    #[must_use]
    pub fn last_match_time(&self) -> Option<MatchTime> {
        self.last_event.as_ref().map(AppliedEventRef::match_time)
    }

    #[must_use]
    pub const fn book(&self) -> &StateField<CompleteBookSnapshot> {
        &self.book
    }

    #[must_use]
    pub const fn recent_trade(&self) -> &StateField<TradeObservation> {
        &self.recent_trade
    }

    #[must_use]
    pub const fn cumulative_volume(&self) -> &StateField<Volume> {
        &self.cumulative_volume
    }

    #[must_use]
    pub const fn last_annotations(&self) -> &StateField<MarketAnnotations> {
        &self.last_annotations
    }

    #[must_use]
    pub const fn view(&self) -> MarketStateView<'_> {
        MarketStateView { state: self }
    }

    pub(crate) fn reset_observable_fields(&mut self) {
        self.book = StateField::initial();
        self.recent_trade = StateField::initial();
        self.cumulative_volume = StateField::initial();
        self.last_annotations = StateField::initial();
    }

    pub(crate) fn set_segment(&mut self, segment: SessionSegmentId) {
        self.current_segment_id = Some(segment);
    }

    pub(crate) fn set_book(&mut self, book: StateField<CompleteBookSnapshot>) {
        self.book = book;
    }

    pub(crate) fn set_recent_trade(&mut self, trade: StateField<TradeObservation>) {
        self.recent_trade = trade;
    }

    pub(crate) fn set_cumulative_volume(&mut self, volume: StateField<Volume>) {
        self.cumulative_volume = volume;
    }

    pub(crate) fn set_last_annotations(&mut self, annotations: StateField<MarketAnnotations>) {
        self.last_annotations = annotations;
    }

    pub(crate) fn finish_event(&mut self, event: AppliedEventRef, version: u64) {
        self.last_event = Some(event);
        self.state_version = version;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MarketStateView<'a> {
    state: &'a MarketState,
}

impl<'a> MarketStateView<'a> {
    #[must_use]
    pub const fn instrument(self) -> &'a InstrumentId {
        self.state.instrument()
    }

    #[must_use]
    pub const fn trading_date(self) -> TradingDate {
        self.state.trading_date()
    }

    #[must_use]
    pub const fn current_segment_id(self) -> Option<&'a SessionSegmentId> {
        self.state.current_segment_id()
    }

    #[must_use]
    pub const fn state_version(self) -> u64 {
        self.state.state_version()
    }

    #[must_use]
    pub const fn last_event(self) -> Option<&'a AppliedEventRef> {
        self.state.last_event()
    }

    #[must_use]
    pub fn last_match_time(self) -> Option<MatchTime> {
        self.state.last_match_time()
    }

    #[must_use]
    pub const fn book(self) -> &'a StateField<CompleteBookSnapshot> {
        self.state.book()
    }

    #[must_use]
    pub fn best_bid(self) -> Option<&'a market_types::BookLevel> {
        self.book()
            .known()
            .and_then(|book| book.bids().levels().next())
    }

    #[must_use]
    pub fn best_ask(self) -> Option<&'a market_types::BookLevel> {
        self.book()
            .known()
            .and_then(|book| book.asks().levels().next())
    }

    #[must_use]
    pub const fn recent_trade(self) -> &'a StateField<TradeObservation> {
        self.state.recent_trade()
    }

    #[must_use]
    pub fn last_trade(self) -> LastTrade<'a> {
        match self.recent_trade() {
            StateField::Unavailable(reason) => {
                LastTrade::Unavailable(LastTradeUnavailable::State(reason))
            }
            StateField::Unknown { raw, .. } => LastTrade::Unknown(raw),
            StateField::Known {
                value: TradeObservation::Single(trade),
                ..
            } => LastTrade::Known(trade),
            StateField::Known {
                value:
                    TradeObservation::Batch {
                        trades,
                        trade_order: TradeOrder::SourceOrdered,
                    },
                ..
            } => LastTrade::Known(
                trades
                    .last()
                    .expect("validated TradeObservation batch is non-empty"),
            ),
            StateField::Known {
                value:
                    TradeObservation::Batch {
                        trade_order: TradeOrder::Unspecified,
                        ..
                    },
                ..
            } => LastTrade::Unavailable(LastTradeUnavailable::AmbiguousBatchOrder),
        }
    }

    #[must_use]
    pub const fn cumulative_volume(self) -> &'a StateField<Volume> {
        self.state.cumulative_volume()
    }

    #[must_use]
    pub const fn last_annotations(self) -> &'a StateField<MarketAnnotations> {
        self.state.last_annotations()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastTrade<'a> {
    Known(&'a TradePrint),
    Unavailable(LastTradeUnavailable<'a>),
    Unknown(&'a UnknownValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastTradeUnavailable<'a> {
    State(&'a UnavailableReason),
    AmbiguousBatchOrder,
}
