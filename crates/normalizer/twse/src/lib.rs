use std::{collections::BTreeMap, error::Error, fmt};

use market_types::{
    BookError, BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, DomainEvent, EventError,
    EventPayload, IndicativeAuction, InstantTrend, InstrumentId, LimitPosition, MarketAnnotations,
    MarketId, MatchTime, MatchTimeError, Observation, Price, PriceError, Quantity, QuantityError,
    QuantityUnit, QuoteSnapshot, SourceFormatId, TradeBatch, TradeOrder, TradePrint,
    TradePrintKind, TradingDate, TwseQuoteAnnotations, Volume,
};
use serde::Deserialize;
use serde_json::value::RawValue;

pub const MAPPING_NAME: &str = "TeralionTwseQuote";
pub const MAPPING_VERSION: u16 = 4;
pub const WARRANT_MAPPING_NAME: &str = "TeralionTwseWarrant";
pub const WARRANT_MAPPING_VERSION: u16 = 1;

const STOCK_SNAPSHOT: &str = "STOCK_SNAPSHOT";
const STOCK_REALTIME: &str = "STOCK_REALTIME";
const WARRANT_SNAPSHOT: &str = "WARRANT_SNAPSHOT";
const WARRANT_REALTIME: &str = "WARRANT_REALTIME";
const INTRADAY_ODDLOT_REALTIME: &str = "INTRADAY_ODDLOT_REALTIME";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentProfile {
    Equity,
    Warrant,
}

/// Validated source-partition identity and the half-open replay window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizerConfig {
    instrument: InstrumentId,
    trading_date: TradingDate,
    replay_start: MatchTime,
    replay_end_exclusive: MatchTime,
    profile: InstrumentProfile,
}

impl NormalizerConfig {
    pub fn new(
        instrument: InstrumentId,
        trading_date: TradingDate,
        replay_start: MatchTime,
        replay_end_exclusive: MatchTime,
    ) -> Result<Self, ConfigError> {
        Self::for_profile(
            instrument,
            trading_date,
            replay_start,
            replay_end_exclusive,
            InstrumentProfile::Equity,
        )
    }

    pub fn new_warrant(
        instrument: InstrumentId,
        trading_date: TradingDate,
        replay_start: MatchTime,
        replay_end_exclusive: MatchTime,
    ) -> Result<Self, ConfigError> {
        Self::for_profile(
            instrument,
            trading_date,
            replay_start,
            replay_end_exclusive,
            InstrumentProfile::Warrant,
        )
    }

    pub fn for_profile(
        instrument: InstrumentId,
        trading_date: TradingDate,
        replay_start: MatchTime,
        replay_end_exclusive: MatchTime,
        profile: InstrumentProfile,
    ) -> Result<Self, ConfigError> {
        if instrument.market() != MarketId::Twse {
            return Err(ConfigError::WrongMarket(instrument.market()));
        }
        if replay_start >= replay_end_exclusive {
            return Err(ConfigError::InvalidReplayWindow);
        }
        Ok(Self {
            instrument,
            trading_date,
            replay_start,
            replay_end_exclusive,
            profile,
        })
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
    pub const fn replay_start(&self) -> MatchTime {
        self.replay_start
    }

    #[must_use]
    pub const fn replay_end_exclusive(&self) -> MatchTime {
        self.replay_end_exclusive
    }

    #[must_use]
    pub const fn profile(&self) -> InstrumentProfile {
        self.profile
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    WrongMarket(MarketId),
    InvalidReplayWindow,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongMarket(market) => {
                write!(formatter, "TWSE normalizer cannot use market {market:?}")
            }
            Self::InvalidReplayWindow => {
                formatter.write_str("replay window start must be before its exclusive end")
            }
        }
    }
}

impl Error for ConfigError {}

/// Batch normalizer that groups realtime intermediate/final records by source identity and time.
#[derive(Debug, Clone)]
pub struct TwseNormalizer {
    config: NormalizerConfig,
}

impl TwseNormalizer {
    #[must_use]
    pub const fn new(config: NormalizerConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(&self) -> &NormalizerConfig {
        &self.config
    }

    /// Normalizes JSON objects without using input order to pair realtime match groups.
    pub fn normalize_json_lines<I, S>(
        &self,
        lines: I,
    ) -> Result<NormalizationReport, NormalizationError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut records = Vec::new();
        let mut outside_replay_window = Vec::new();
        let mut known_skipped = Vec::new();
        let mut warnings = Vec::new();
        let mut input_records = 0_u64;

        for (index, line) in lines.into_iter().enumerate() {
            input_records = input_records
                .checked_add(1)
                .expect("input record count fits in u64");
            let classified = self.parse_record(index + 1, line.as_ref())?;
            match classified {
                ClassifiedRecord::Accepted {
                    record,
                    warnings: record_warnings,
                } => {
                    records.push(*record);
                    warnings.extend(record_warnings);
                }
                ClassifiedRecord::OutsideReplayWindow(context) => {
                    outside_replay_window.push(context);
                }
                ClassifiedRecord::KnownSkipped(skipped) => known_skipped.push(skipped),
            }
        }

        let events = self.normalize_groups(records)?;
        Ok(NormalizationReport {
            input_records,
            events,
            outside_replay_window,
            known_skipped,
            warnings,
        })
    }

    fn parse_record(
        &self,
        record_number: usize,
        json: &str,
    ) -> Result<ClassifiedRecord, NormalizationError> {
        let envelope: WireEnvelope<'_> = serde_json::from_str(json).map_err(|error| {
            NormalizationError::new(
                record_number,
                RecordContext::partition(&self.config),
                NormalizationErrorKind::InvalidJson(error.to_string().into_boxed_str()),
            )
        })?;

        let mut context = RecordContext::partition(&self.config);
        context.source_format = Some(envelope.format.into());
        context.match_time_text = Some(envelope.match_time.into());

        validate_identity(
            record_number,
            &context,
            "type",
            "quote",
            envelope.record_type,
        )?;
        validate_identity(record_number, &context, "market", "twse", envelope.market)?;
        validate_identity(
            record_number,
            &context,
            "symbol",
            self.config.instrument.symbol().as_str(),
            envelope.symbol,
        )?;

        let match_time = MatchTime::parse(envelope.match_time).map_err(|error| {
            NormalizationError::new(
                record_number,
                context.clone(),
                NormalizationErrorKind::InvalidMatchTime(error),
            )
        })?;
        context.match_time = Some(match_time);
        MatchTime::parse(envelope.received_at).map_err(|error| {
            NormalizationError::new(
                record_number,
                context.clone(),
                NormalizationErrorKind::InvalidReceivedAt(error),
            )
        })?;

        let format = match envelope.format {
            STOCK_SNAPSHOT if self.config.profile == InstrumentProfile::Equity => {
                WireFormat::StockSnapshot
            }
            STOCK_REALTIME if self.config.profile == InstrumentProfile::Equity => {
                WireFormat::StockRealtime
            }
            WARRANT_SNAPSHOT if self.config.profile == InstrumentProfile::Warrant => {
                WireFormat::StockSnapshot
            }
            WARRANT_REALTIME if self.config.profile == InstrumentProfile::Warrant => {
                WireFormat::StockRealtime
            }
            INTRADAY_ODDLOT_REALTIME => {
                return Ok(ClassifiedRecord::KnownSkipped(KnownSkipped {
                    context,
                    reason: KnownSkipReason::IntradayOddLot,
                }));
            }
            unknown => {
                return Err(NormalizationError::new(
                    record_number,
                    context,
                    NormalizationErrorKind::UnsupportedFormat(unknown.into()),
                ));
            }
        };

        if match_time < self.config.replay_start || match_time >= self.config.replay_end_exclusive {
            return Ok(ClassifiedRecord::OutsideReplayWindow(context));
        }

        let wire: WireQuote<'_> = serde_json::from_str(json).map_err(|error| {
            NormalizationError::new(
                record_number,
                context.clone(),
                NormalizationErrorKind::InvalidJson(error.to_string().into_boxed_str()),
            )
        })?;

        if format == WireFormat::StockSnapshot && wire.intermediate_print {
            return Err(NormalizationError::new(
                record_number,
                context,
                NormalizationErrorKind::InvalidPayload(
                    "STOCK_SNAPSHOT cannot be an intermediate print",
                ),
            ));
        }

        let annotations = TwseQuoteAnnotations::new(wire.status_flags, wire.limit_flags);
        let record_warnings = annotation_warnings(&context, annotations);
        let cumulative_volume = Volume::new(wire.cum_volume, QuantityUnit::TradingUnit);
        let deal = parse_deal(record_number, &context, wire.deal)?;

        let book = if wire.intermediate_print {
            if format != WireFormat::StockRealtime {
                return Err(NormalizationError::new(
                    record_number,
                    context,
                    NormalizationErrorKind::InvalidPayload(
                        "only STOCK_REALTIME may be an intermediate print",
                    ),
                ));
            }
            if !wire.bids.is_empty() || !wire.asks.is_empty() {
                return Err(NormalizationError::new(
                    record_number,
                    context,
                    NormalizationErrorKind::InvalidPayload(
                        "intermediate print must have empty bid and ask arrays",
                    ),
                ));
            }
            if deal.is_none() {
                return Err(NormalizationError::new(
                    record_number,
                    context,
                    NormalizationErrorKind::InvalidPayload(
                        "intermediate print must contain a deal",
                    ),
                ));
            }
            None
        } else {
            Some(parse_book(record_number, &context, wire.bids, wire.asks)?)
        };

        let source_format = SourceFormatId::new(envelope.format)
            .expect("supported source format constants are non-empty");
        Ok(ClassifiedRecord::Accepted {
            record: Box::new(ValidatedRecord {
                record_number,
                context,
                format,
                source_format,
                match_time,
                intermediate: wire.intermediate_print,
                book,
                deal,
                cumulative_volume,
                annotations,
            }),
            warnings: record_warnings,
        })
    }

    fn normalize_groups(
        &self,
        records: Vec<ValidatedRecord>,
    ) -> Result<Vec<DomainEvent>, NormalizationError> {
        let mut realtime_groups: BTreeMap<MatchTime, Vec<usize>> = BTreeMap::new();
        for (index, record) in records.iter().enumerate() {
            if record.format == WireFormat::StockRealtime {
                realtime_groups
                    .entry(record.match_time)
                    .or_default()
                    .push(index);
            }
        }

        let mut grouped_events: BTreeMap<usize, Vec<DomainEvent>> = BTreeMap::new();
        let mut consumed = vec![false; records.len()];
        for indices in realtime_groups.values() {
            if !indices.iter().any(|index| records[*index].intermediate) {
                continue;
            }
            let anchor = *indices.iter().min().expect("group is non-empty");
            let events = self.normalize_intermediate_group(&records, indices)?;
            grouped_events.insert(anchor, events);
            for index in indices {
                consumed[*index] = true;
            }
        }

        let mut events = Vec::with_capacity(records.len());
        for (index, record) in records.iter().enumerate() {
            if let Some(group) = grouped_events.remove(&index) {
                events.extend(group);
            }
            if !consumed[index] {
                events.push(self.final_event(record)?);
            }
        }
        Ok(events)
    }

    fn normalize_intermediate_group(
        &self,
        records: &[ValidatedRecord],
        indices: &[usize],
    ) -> Result<Vec<DomainEvent>, NormalizationError> {
        let first = &records[indices[0]];
        let mut intermediates = indices
            .iter()
            .map(|index| &records[*index])
            .filter(|record| record.intermediate);
        let intermediate = intermediates.next();
        let extra_intermediate = intermediates.next().is_some();
        let mut finals = indices
            .iter()
            .map(|index| &records[*index])
            .filter(|record| !record.intermediate);
        let final_record = finals.next();
        let extra_final = finals.next().is_some();

        let (Some(intermediate), Some(final_record)) = (intermediate, final_record) else {
            return Err(group_error(
                first,
                RealtimeGroupError::ExpectedOneIntermediateAndOneFinal {
                    records: indices.len(),
                },
            ));
        };
        if indices.len() != 2 || extra_intermediate || extra_final {
            return Err(group_error(
                first,
                RealtimeGroupError::ExpectedOneIntermediateAndOneFinal {
                    records: indices.len(),
                },
            ));
        }

        let intermediate_phase = self.auction_phase(intermediate)?;
        let final_phase = self.auction_phase(final_record)?;
        if intermediate_phase != final_phase {
            return Err(group_error(
                first,
                RealtimeGroupError::MixedAuctionTrialState,
            ));
        }

        if intermediate_phase.is_some() {
            return Ok(vec![
                self.intermediate_event(intermediate)?,
                self.final_event(final_record)?,
            ]);
        }

        let final_trade = final_record
            .deal
            .ok_or_else(|| group_error(first, RealtimeGroupError::FinalDealMissing))?;
        let expected_final = intermediate
            .cumulative_volume
            .checked_add(Volume::from(final_trade.quantity()))
            .map_err(|_| group_error(first, RealtimeGroupError::CumulativeVolumeMismatch))?;
        if final_record.cumulative_volume != expected_final {
            return Err(group_error(
                first,
                RealtimeGroupError::CumulativeVolumeMismatch,
            ));
        }

        Ok(vec![
            self.intermediate_event(intermediate)?,
            self.final_event(final_record)?,
        ])
    }

    fn final_event(&self, record: &ValidatedRecord) -> Result<DomainEvent, NormalizationError> {
        if record.intermediate {
            return Err(group_error(
                record,
                RealtimeGroupError::ExpectedOneIntermediateAndOneFinal { records: 1 },
            ));
        }
        let book = record.book.clone().ok_or_else(|| {
            NormalizationError::new(
                record.record_number,
                record.context.clone(),
                NormalizationErrorKind::InvalidPayload("final quote is missing a complete book"),
            )
        })?;
        if let Some(phase) = self.auction_phase(record)? {
            return self.auction_event(record, phase, Observation::Set(book));
        }
        let trade = record
            .deal
            .map(Observation::Set)
            .unwrap_or(Observation::NoObservation);
        let snapshot = QuoteSnapshot::new(
            book,
            trade,
            Observation::Set(record.cumulative_volume),
            MarketAnnotations::TwseQuote(record.annotations),
        )
        .map_err(|error| event_error(record, error))?;
        Ok(self.domain_event(record, EventPayload::QuoteSnapshot(snapshot)))
    }

    fn intermediate_event(
        &self,
        record: &ValidatedRecord,
    ) -> Result<DomainEvent, NormalizationError> {
        let trade = record
            .deal
            .ok_or_else(|| group_error(record, RealtimeGroupError::IntermediateDealMissing))?;
        if let Some(phase) = self.auction_phase(record)? {
            return self.auction_event(record, phase, Observation::NoObservation);
        }
        let trade = TradePrint::new(
            trade.price(),
            trade.quantity(),
            TradePrintKind::Intermediate,
        );
        let batch = TradeBatch::new(
            vec![trade],
            TradeOrder::SourceOrdered,
            Observation::Set(record.cumulative_volume),
            MarketAnnotations::TwseQuote(record.annotations),
        )
        .map_err(|error| event_error(record, error))?;
        Ok(self.domain_event(record, EventPayload::TradeBatch(batch)))
    }

    fn auction_event(
        &self,
        record: &ValidatedRecord,
        phase: AuctionPhase,
        book: Observation<CompleteBookSnapshot>,
    ) -> Result<DomainEvent, NormalizationError> {
        let (price, quantity) = match record.deal {
            Some(trade) => (
                Observation::Set(trade.price()),
                Observation::Set(trade.quantity()),
            ),
            None => (Observation::NoObservation, Observation::NoObservation),
        };
        let auction = IndicativeAuction::new(
            price,
            quantity,
            book,
            Observation::Set(record.cumulative_volume),
            MarketAnnotations::TwseQuote(record.annotations),
        )
        .map_err(|error| event_error(record, error))?;
        let payload = match phase {
            AuctionPhase::Opening => EventPayload::IndicativeOpeningAuction(auction),
            AuctionPhase::Closing => EventPayload::IndicativeClosingAuction(auction),
        };
        Ok(self.domain_event(record, payload))
    }

    fn auction_phase(
        &self,
        record: &ValidatedRecord,
    ) -> Result<Option<AuctionPhase>, NormalizationError> {
        let status = record.annotations.status();
        if !status.trial() {
            return Ok(None);
        }

        let opening_window_start = self.session_time("08:30:00");
        let opening_window_end = self.session_time("09:00:00");
        let closing_window_start = self.session_time("13:25:00");
        let closing_window_end = self.session_time("13:30:00");
        let in_opening_window =
            record.match_time >= opening_window_start && record.match_time < opening_window_end;
        let in_closing_window =
            record.match_time >= closing_window_start && record.match_time < closing_window_end;
        let opening = status.delayed_open() || status.opening_marker() || in_opening_window;
        let closing = status.delayed_close() || status.closing_marker() || in_closing_window;
        match (opening, closing) {
            (true, false) => Ok(Some(AuctionPhase::Opening)),
            (false, true) => Ok(Some(AuctionPhase::Closing)),
            (false, false) | (true, true) => Err(NormalizationError::new(
                record.record_number,
                record.context.clone(),
                NormalizationErrorKind::InvalidPayload(
                    "trial quote cannot be classified as exactly one auction phase",
                ),
            )),
        }
    }

    fn session_time(&self, time: &str) -> MatchTime {
        let value = format!("{}T{time}+08:00", self.config.trading_date());
        MatchTime::parse(&value).expect("TWSE session constants are valid timestamps")
    }

    fn domain_event(&self, record: &ValidatedRecord, payload: EventPayload) -> DomainEvent {
        DomainEvent::new(
            self.config.instrument.clone(),
            self.config.trading_date,
            record.source_format.clone(),
            record.match_time,
            None,
            payload,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationReport {
    input_records: u64,
    events: Vec<DomainEvent>,
    outside_replay_window: Vec<RecordContext>,
    known_skipped: Vec<KnownSkipped>,
    warnings: Vec<NormalizationWarning>,
}

impl NormalizationReport {
    #[must_use]
    pub const fn input_records(&self) -> u64 {
        self.input_records
    }

    #[must_use]
    pub fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    #[must_use]
    pub fn outside_replay_window(&self) -> &[RecordContext] {
        &self.outside_replay_window
    }

    #[must_use]
    pub fn known_skipped(&self) -> &[KnownSkipped] {
        &self.known_skipped
    }

    #[must_use]
    pub fn warnings(&self) -> &[NormalizationWarning] {
        &self.warnings
    }

    #[must_use]
    pub fn into_events(self) -> Vec<DomainEvent> {
        self.events
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordContext {
    instrument: InstrumentId,
    trading_date: TradingDate,
    source_format: Option<Box<str>>,
    match_time_text: Option<Box<str>>,
    match_time: Option<MatchTime>,
}

impl RecordContext {
    fn partition(config: &NormalizerConfig) -> Self {
        Self {
            instrument: config.instrument.clone(),
            trading_date: config.trading_date,
            source_format: None,
            match_time_text: None,
            match_time: None,
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
    pub fn source_format(&self) -> Option<&str> {
        self.source_format.as_deref()
    }

    #[must_use]
    pub fn match_time_text(&self) -> Option<&str> {
        self.match_time_text.as_deref()
    }

    #[must_use]
    pub const fn match_time(&self) -> Option<MatchTime> {
        self.match_time
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownSkipped {
    context: RecordContext,
    reason: KnownSkipReason,
}

impl KnownSkipped {
    #[must_use]
    pub const fn context(&self) -> &RecordContext {
        &self.context
    }

    #[must_use]
    pub const fn reason(&self) -> KnownSkipReason {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownSkipReason {
    IntradayOddLot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationWarning {
    context: RecordContext,
    kind: WarningKind,
}

impl NormalizationWarning {
    #[must_use]
    pub const fn context(&self) -> &RecordContext {
        &self.context
    }

    #[must_use]
    pub const fn kind(&self) -> WarningKind {
        self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    ReservedStatusBits(u8),
    ReservedTradeLimit,
    ReservedBestBidLimit,
    ReservedBestAskLimit,
    ReservedInstantTrend,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationError {
    record_number: usize,
    context: RecordContext,
    kind: Box<NormalizationErrorKind>,
}

impl NormalizationError {
    fn new(record_number: usize, context: RecordContext, kind: NormalizationErrorKind) -> Self {
        Self {
            record_number,
            context,
            kind: Box::new(kind),
        }
    }

    #[must_use]
    pub const fn record_number(&self) -> usize {
        self.record_number
    }

    #[must_use]
    pub const fn context(&self) -> &RecordContext {
        &self.context
    }

    #[must_use]
    pub fn kind(&self) -> &NormalizationErrorKind {
        self.kind.as_ref()
    }
}

impl fmt::Display for NormalizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "TWSE normalization failed at input record {}",
            self.record_number
        )?;
        if let Some(format) = self.context.source_format() {
            write!(formatter, ", format {format}")?;
        }
        if let Some(match_time) = self.context.match_time_text() {
            write!(formatter, ", match_time {match_time}")?;
        }
        write!(formatter, ": {}", self.kind)
    }
}

impl Error for NormalizationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self.kind.as_ref() {
            NormalizationErrorKind::InvalidMatchTime(error)
            | NormalizationErrorKind::InvalidReceivedAt(error) => Some(error),
            NormalizationErrorKind::InvalidPrice { error, .. } => Some(error),
            NormalizationErrorKind::InvalidQuantity { error, .. } => Some(error),
            NormalizationErrorKind::InvalidBook(error) => Some(error),
            NormalizationErrorKind::InvalidEvent(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizationErrorKind {
    InvalidJson(Box<str>),
    InvalidIdentity {
        field: &'static str,
        expected: Box<str>,
        actual: Box<str>,
    },
    UnsupportedFormat(Box<str>),
    InvalidMatchTime(MatchTimeError),
    InvalidReceivedAt(MatchTimeError),
    InvalidPrice {
        field: &'static str,
        error: PriceError,
    },
    InvalidQuantity {
        field: &'static str,
        error: QuantityError,
    },
    InvalidBook(BookError),
    InvalidEvent(EventError),
    InvalidPayload(&'static str),
    UnsupportedRealtimeMatchGroup(RealtimeGroupError),
}

impl fmt::Display for NormalizationErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid JSON: {error}"),
            Self::InvalidIdentity {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid identity field {field}: expected {expected}, got {actual}"
            ),
            Self::UnsupportedFormat(format) => write!(formatter, "unsupported format {format}"),
            Self::InvalidMatchTime(error) => write!(formatter, "invalid match_time: {error}"),
            Self::InvalidReceivedAt(error) => write!(formatter, "invalid received_at: {error}"),
            Self::InvalidPrice { field, error } => {
                write!(formatter, "invalid {field}: {error}")
            }
            Self::InvalidQuantity { field, error } => {
                write!(formatter, "invalid {field}: {error}")
            }
            Self::InvalidBook(error) => write!(formatter, "invalid complete book: {error}"),
            Self::InvalidEvent(error) => write!(formatter, "invalid domain event: {error}"),
            Self::InvalidPayload(message) => formatter.write_str(message),
            Self::UnsupportedRealtimeMatchGroup(error) => {
                write!(formatter, "unsupported realtime match group: {error}")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeGroupError {
    ExpectedOneIntermediateAndOneFinal { records: usize },
    IntermediateDealMissing,
    FinalDealMissing,
    CumulativeVolumeMismatch,
    MixedAuctionTrialState,
}

impl fmt::Display for RealtimeGroupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedOneIntermediateAndOneFinal { records } => write!(
                formatter,
                "expected exactly one intermediate and one final record, got {records} records"
            ),
            Self::IntermediateDealMissing => {
                formatter.write_str("intermediate record has no deal")
            }
            Self::FinalDealMissing => formatter.write_str("final record has no deal"),
            Self::CumulativeVolumeMismatch => formatter.write_str(
                "final cumulative volume does not equal intermediate cumulative volume plus final deal quantity",
            ),
            Self::MixedAuctionTrialState => {
                formatter.write_str("realtime group mixes trial and non-trial auction records")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireFormat {
    StockSnapshot,
    StockRealtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuctionPhase {
    Opening,
    Closing,
}

#[derive(Debug)]
enum ClassifiedRecord {
    Accepted {
        record: Box<ValidatedRecord>,
        warnings: Vec<NormalizationWarning>,
    },
    OutsideReplayWindow(RecordContext),
    KnownSkipped(KnownSkipped),
}

#[derive(Debug)]
struct ValidatedRecord {
    record_number: usize,
    context: RecordContext,
    format: WireFormat,
    source_format: SourceFormatId,
    match_time: MatchTime,
    intermediate: bool,
    book: Option<CompleteBookSnapshot>,
    deal: Option<TradePrint>,
    cumulative_volume: Volume,
    annotations: TwseQuoteAnnotations,
}

#[derive(Debug, Deserialize)]
struct WireEnvelope<'a> {
    #[serde(rename = "type")]
    record_type: &'a str,
    market: &'a str,
    format: &'a str,
    symbol: &'a str,
    match_time: &'a str,
    received_at: &'a str,
}

#[derive(Debug, Deserialize)]
struct WireQuote<'a> {
    #[serde(borrow)]
    bids: Vec<WireLevel<'a>>,
    #[serde(borrow)]
    asks: Vec<WireLevel<'a>>,
    #[serde(borrow)]
    deal: &'a RawValue,
    cum_volume: u64,
    limit_flags: u8,
    status_flags: u8,
    intermediate_print: bool,
}

#[derive(Debug, Deserialize)]
struct WireLevel<'a> {
    #[serde(borrow)]
    price: &'a RawValue,
    quantity: u64,
}

#[derive(Debug, Deserialize)]
struct WireDeal<'a> {
    #[serde(borrow)]
    price: &'a RawValue,
    quantity: u64,
}

fn validate_identity(
    record_number: usize,
    context: &RecordContext,
    field: &'static str,
    expected: &str,
    actual: &str,
) -> Result<(), NormalizationError> {
    if expected == actual {
        Ok(())
    } else {
        Err(NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidIdentity {
                field,
                expected: expected.into(),
                actual: actual.into(),
            },
        ))
    }
}

fn parse_book(
    record_number: usize,
    context: &RecordContext,
    bids: Vec<WireLevel<'_>>,
    asks: Vec<WireLevel<'_>>,
) -> Result<CompleteBookSnapshot, NormalizationError> {
    let bids = parse_side(record_number, context, BookSideKind::Bid, bids)?;
    let asks = parse_side(record_number, context, BookSideKind::Ask, asks)?;
    CompleteBookSnapshot::new(bids, asks).map_err(|error| {
        NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidBook(error),
        )
    })
}

fn parse_side(
    record_number: usize,
    context: &RecordContext,
    kind: BookSideKind,
    wire_levels: Vec<WireLevel<'_>>,
) -> Result<BookSide, NormalizationError> {
    let mut levels = Vec::with_capacity(wire_levels.len());
    for wire in wire_levels {
        let price = parse_price(record_number, context, "book level price", wire.price)?;
        let quantity =
            Quantity::new(wire.quantity, QuantityUnit::TradingUnit).map_err(|error| {
                NormalizationError::new(
                    record_number,
                    context.clone(),
                    NormalizationErrorKind::InvalidQuantity {
                        field: "book level quantity",
                        error,
                    },
                )
            })?;
        levels.push(BookLevel::new(price, quantity));
    }
    BookSide::new(kind, levels).map_err(|error| {
        NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidBook(error),
        )
    })
}

fn parse_deal(
    record_number: usize,
    context: &RecordContext,
    raw: &RawValue,
) -> Result<Option<TradePrint>, NormalizationError> {
    if raw.get() == "null" {
        return Ok(None);
    }
    let wire: WireDeal<'_> = serde_json::from_str(raw.get()).map_err(|error| {
        NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidJson(error.to_string().into_boxed_str()),
        )
    })?;
    let price = parse_price(record_number, context, "deal price", wire.price)?;
    let quantity = Quantity::new(wire.quantity, QuantityUnit::TradingUnit).map_err(|error| {
        NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidQuantity {
                field: "deal quantity",
                error,
            },
        )
    })?;
    Ok(Some(TradePrint::new(
        price,
        quantity,
        TradePrintKind::Regular,
    )))
}

fn parse_price(
    record_number: usize,
    context: &RecordContext,
    field: &'static str,
    raw: &RawValue,
) -> Result<Price, NormalizationError> {
    Price::parse(raw.get()).map_err(|error| {
        NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidPrice { field, error },
        )
    })
}

fn annotation_warnings(
    context: &RecordContext,
    annotations: TwseQuoteAnnotations,
) -> Vec<NormalizationWarning> {
    let mut warnings = Vec::new();
    let status = annotations.status();
    if status.reserved_bits() != 0 {
        warnings.push(NormalizationWarning {
            context: context.clone(),
            kind: WarningKind::ReservedStatusBits(status.reserved_bits()),
        });
    }

    let limits = annotations.limits();
    for (reserved, kind) in [
        (
            limits.trade() == LimitPosition::Reserved,
            WarningKind::ReservedTradeLimit,
        ),
        (
            limits.best_bid() == LimitPosition::Reserved,
            WarningKind::ReservedBestBidLimit,
        ),
        (
            limits.best_ask() == LimitPosition::Reserved,
            WarningKind::ReservedBestAskLimit,
        ),
        (
            limits.instant_trend() == InstantTrend::Reserved,
            WarningKind::ReservedInstantTrend,
        ),
    ] {
        if reserved {
            warnings.push(NormalizationWarning {
                context: context.clone(),
                kind,
            });
        }
    }
    warnings
}

fn group_error(record: &ValidatedRecord, error: RealtimeGroupError) -> NormalizationError {
    NormalizationError::new(
        record.record_number,
        record.context.clone(),
        NormalizationErrorKind::UnsupportedRealtimeMatchGroup(error),
    )
}

fn event_error(record: &ValidatedRecord, error: EventError) -> NormalizationError {
    NormalizationError::new(
        record.record_number,
        record.context.clone(),
        NormalizationErrorKind::InvalidEvent(error),
    )
}
