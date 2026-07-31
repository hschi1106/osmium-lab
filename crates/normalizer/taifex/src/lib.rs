use std::{error::Error, fmt};

use market_types::{
    BookError, BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, DomainEvent, EventError,
    EventPayload, IndicativeAuction, InstrumentId, MarketAnnotations, MarketId, MatchTime,
    MatchTimeError, Observation, Price, PriceError, Quantity, QuantityError, QuantityUnit,
    SourceFormatId, TradeBatch, TradeOrder, TradePrint, TradePrintKind, TradingDate,
};
use serde::Deserialize;
use serde_json::value::RawValue;

pub const MAPPING_NAME: &str = "TeralionTaifexFutures";
pub const MAPPING_VERSION: u16 = 2;

const MARKET: &str = "taifex_fut";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizerConfig {
    instrument: InstrumentId,
    trading_date: TradingDate,
    replay_start: MatchTime,
    replay_end_exclusive: MatchTime,
}

impl NormalizerConfig {
    pub fn new(
        instrument: InstrumentId,
        trading_date: TradingDate,
        replay_start: MatchTime,
        replay_end_exclusive: MatchTime,
    ) -> Result<Self, ConfigError> {
        if instrument.market() != MarketId::Taifex {
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
                write!(formatter, "TAIFEX normalizer cannot use market {market:?}")
            }
            Self::InvalidReplayWindow => {
                formatter.write_str("replay window start must be before its exclusive end")
            }
        }
    }
}

impl Error for ConfigError {}

#[derive(Debug, Clone)]
pub struct TaifexNormalizer {
    config: NormalizerConfig,
}

impl TaifexNormalizer {
    #[must_use]
    pub const fn new(config: NormalizerConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(&self) -> &NormalizerConfig {
        &self.config
    }

    pub fn normalize_json_lines<I, S>(
        &self,
        lines: I,
    ) -> Result<NormalizationReport, NormalizationError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut input_records = 0_u64;
        let mut events = Vec::new();
        let mut outside_replay_window = Vec::new();
        let mut known_skipped = Vec::new();

        for (index, line) in lines.into_iter().enumerate() {
            input_records = input_records
                .checked_add(1)
                .expect("input record count fits in u64");
            match self.parse_record(index + 1, line.as_ref())? {
                ClassifiedRecord::Accepted(event) => events.push(*event),
                ClassifiedRecord::OutsideReplayWindow(context) => {
                    outside_replay_window.push(context)
                }
                ClassifiedRecord::KnownSkipped(skipped) => known_skipped.push(skipped),
            }
        }

        sort_events(&mut events);
        Ok(NormalizationReport {
            input_records,
            events,
            outside_replay_window,
            known_skipped,
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
            expected_type(envelope.format).ok_or_else(|| {
                NormalizationError::new(
                    record_number,
                    context.clone(),
                    NormalizationErrorKind::UnsupportedFormat(envelope.format.into()),
                )
            })?,
            envelope.record_type,
        )?;
        validate_identity(record_number, &context, "market", MARKET, envelope.market)?;
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

        if match_time < self.config.replay_start || match_time >= self.config.replay_end_exclusive {
            return Ok(ClassifiedRecord::OutsideReplayWindow(context));
        }

        let source_format =
            SourceFormatId::new(envelope.format).expect("TAIFEX format is non-empty");
        match envelope.format {
            "I020" => self.parse_i020(record_number, context, source_format, match_time, json),
            "I022" => self.parse_i022(record_number, context, source_format, match_time, json),
            "I080" | "I082" => {
                self.parse_book(record_number, context, source_format, match_time, json)
            }
            "I021" => Ok(ClassifiedRecord::KnownSkipped(KnownSkipped {
                context,
                reason: KnownSkipReason::IntradayHighLow,
            })),
            "I023" => Ok(ClassifiedRecord::KnownSkipped(KnownSkipped {
                context,
                reason: KnownSkipReason::OpeningReference,
            })),
            "I030" => Ok(ClassifiedRecord::KnownSkipped(KnownSkipped {
                context,
                reason: KnownSkipReason::OrderStatistics,
            })),
            "I070" | "I072" => Ok(ClassifiedRecord::KnownSkipped(KnownSkipped {
                context,
                reason: KnownSkipReason::ClosingStatistics,
            })),
            unknown => Err(NormalizationError::new(
                record_number,
                context,
                NormalizationErrorKind::UnsupportedFormat(unknown.into()),
            )),
        }
    }

    fn parse_i020(
        &self,
        record_number: usize,
        context: RecordContext,
        source_format: SourceFormatId,
        match_time: MatchTime,
        json: &str,
    ) -> Result<ClassifiedRecord, NormalizationError> {
        let wire: WireTradeRecord<'_> = parse_json(record_number, &context, json)?;
        if wire.first_packet != Some(true) {
            return Err(payload_error(
                record_number,
                context,
                "unsupported I020 continuation",
            ));
        }
        if wire.trades.is_empty() {
            return Err(payload_error(
                record_number,
                context,
                "I020 trades must not be empty",
            ));
        }
        let mut trades = Vec::with_capacity(wire.trades.len());
        for trade in wire.trades {
            let price = parse_price(record_number, &context, "price", trade.price)?;
            let quantity = parse_quantity(record_number, &context, "quantity", trade.quantity)?;
            trades.push(TradePrint::new(price, quantity, TradePrintKind::Regular));
        }
        let batch = TradeBatch::new(
            trades,
            TradeOrder::SourceOrdered,
            Observation::NoObservation,
            MarketAnnotations::None,
        )
        .map_err(|error| event_error(record_number, &context, error))?;
        Ok(ClassifiedRecord::Accepted(Box::new(DomainEvent::new(
            self.config.instrument.clone(),
            self.config.trading_date,
            source_format,
            match_time,
            None,
            EventPayload::TradeBatch(batch),
        ))))
    }

    fn parse_i022(
        &self,
        record_number: usize,
        context: RecordContext,
        source_format: SourceFormatId,
        match_time: MatchTime,
        json: &str,
    ) -> Result<ClassifiedRecord, NormalizationError> {
        let wire: WireTradeRecord<'_> = parse_json(record_number, &context, json)?;
        if wire.trades.len() != 1 {
            return Err(payload_error(
                record_number,
                context,
                "I022 must contain exactly one calculated observation",
            ));
        }
        let trade = &wire.trades[0];
        let decimal = market_types::Decimal::parse(trade.price.get()).map_err(|error| {
            NormalizationError::new(
                record_number,
                context.clone(),
                NormalizationErrorKind::InvalidPrice {
                    field: "price",
                    error: PriceError::InvalidDecimal(error),
                },
            )
        })?;
        let price_zero = decimal.atoms() == 0;
        let quantity_zero = trade.quantity == 0;
        if price_zero != quantity_zero {
            return Err(payload_error(
                record_number,
                context,
                "I022 price and quantity must be zero together or positive together",
            ));
        }
        let (price, quantity) = if price_zero {
            (Observation::NoObservation, Observation::NoObservation)
        } else {
            (
                Observation::Set(Price::new(decimal).map_err(|error| {
                    NormalizationError::new(
                        record_number,
                        context.clone(),
                        NormalizationErrorKind::InvalidPrice {
                            field: "price",
                            error,
                        },
                    )
                })?),
                Observation::Set(parse_quantity(
                    record_number,
                    &context,
                    "quantity",
                    trade.quantity,
                )?),
            )
        };
        let auction = IndicativeAuction::new(
            price,
            quantity,
            Observation::NoObservation,
            Observation::NoObservation,
            MarketAnnotations::None,
        )
        .map_err(|error| event_error(record_number, &context, error))?;
        Ok(ClassifiedRecord::Accepted(Box::new(DomainEvent::new(
            self.config.instrument.clone(),
            self.config.trading_date,
            source_format,
            match_time,
            None,
            EventPayload::IndicativeOpeningAuction(auction),
        ))))
    }

    fn parse_book(
        &self,
        record_number: usize,
        context: RecordContext,
        source_format: SourceFormatId,
        match_time: MatchTime,
        json: &str,
    ) -> Result<ClassifiedRecord, NormalizationError> {
        let wire: WireBookRecord<'_> = parse_json(record_number, &context, json)?;
        let bids = parse_side(record_number, &context, BookSideKind::Bid, wire.bids)?;
        let asks = parse_side(record_number, &context, BookSideKind::Ask, wire.asks)?;
        let book = CompleteBookSnapshot::new(bids, asks).map_err(|error| {
            NormalizationError::new(
                record_number,
                context.clone(),
                NormalizationErrorKind::InvalidBook(error),
            )
        })?;
        Ok(ClassifiedRecord::Accepted(Box::new(DomainEvent::new(
            self.config.instrument.clone(),
            self.config.trading_date,
            source_format,
            match_time,
            None,
            EventPayload::BookSnapshot(market_types::BookSnapshot::new(
                book,
                MarketAnnotations::None,
            )),
        ))))
    }
}

fn expected_type(format: &str) -> Option<&'static str> {
    match format {
        "I020" | "I021" | "I022" | "I023" => Some("trade"),
        "I030" => Some("stats"),
        "I070" | "I072" => Some("close"),
        "I080" | "I082" => Some("book"),
        _ => None,
    }
}

fn sort_events(events: &mut [DomainEvent]) {
    events.sort_by(|left, right| {
        left.match_time()
            .cmp(&right.match_time())
            .then_with(|| left.source_format().cmp(right.source_format()))
            .then_with(|| left.payload().kind().cmp(&right.payload().kind()))
            .then_with(|| {
                left.fingerprint()
                    .expect("validated event fingerprint")
                    .cmp(&right.fingerprint().expect("validated event fingerprint"))
            })
    });
}

fn parse_side(
    record_number: usize,
    context: &RecordContext,
    kind: BookSideKind,
    levels: Vec<WireLevel<'_>>,
) -> Result<BookSide, NormalizationError> {
    if levels.len() > 5 {
        return Err(NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidBook(BookError::TooManyLevels {
                side: kind,
                count: levels.len(),
            }),
        ));
    }
    let mut parsed = Vec::with_capacity(levels.len());
    for level in levels {
        let price = parse_price(record_number, context, "book price", level.price)?;
        let quantity = parse_quantity(record_number, context, "book quantity", level.quantity)?;
        parsed.push(BookLevel::new(price, quantity));
    }
    BookSide::new(kind, parsed).map_err(|error| {
        NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidBook(error),
        )
    })
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

fn parse_quantity(
    record_number: usize,
    context: &RecordContext,
    field: &'static str,
    value: u64,
) -> Result<Quantity, NormalizationError> {
    Quantity::new(value, QuantityUnit::Contract).map_err(|error| {
        NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidQuantity { field, error },
        )
    })
}

fn parse_json<'a, T: serde::Deserialize<'a>>(
    record_number: usize,
    context: &RecordContext,
    json: &'a str,
) -> Result<T, NormalizationError> {
    serde_json::from_str(json).map_err(|error| {
        NormalizationError::new(
            record_number,
            context.clone(),
            NormalizationErrorKind::InvalidJson(error.to_string().into_boxed_str()),
        )
    })
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

fn payload_error(
    record_number: usize,
    context: RecordContext,
    message: &'static str,
) -> NormalizationError {
    NormalizationError::new(
        record_number,
        context,
        NormalizationErrorKind::InvalidPayload(message),
    )
}

fn event_error(
    record_number: usize,
    context: &RecordContext,
    error: EventError,
) -> NormalizationError {
    NormalizationError::new(
        record_number,
        context.clone(),
        NormalizationErrorKind::InvalidEvent(error),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationReport {
    input_records: u64,
    events: Vec<DomainEvent>,
    outside_replay_window: Vec<RecordContext>,
    known_skipped: Vec<KnownSkipped>,
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
    pub fn into_events(self) -> Vec<DomainEvent> {
        self.events
    }

    #[must_use]
    pub fn outside_replay_window(&self) -> &[RecordContext] {
        &self.outside_replay_window
    }

    #[must_use]
    pub fn known_skipped(&self) -> &[KnownSkipped] {
        &self.known_skipped
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KnownSkipReason {
    IntradayHighLow,
    OpeningReference,
    OrderStatistics,
    ClosingStatistics,
}

#[derive(Debug)]
enum ClassifiedRecord {
    Accepted(Box<DomainEvent>),
    OutsideReplayWindow(RecordContext),
    KnownSkipped(KnownSkipped),
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
            "TAIFEX normalization failed at record {}",
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
            Self::InvalidPrice { field, error } => write!(formatter, "invalid {field}: {error}"),
            Self::InvalidQuantity { field, error } => {
                write!(formatter, "invalid {field}: {error}")
            }
            Self::InvalidBook(error) => write!(formatter, "invalid book: {error}"),
            Self::InvalidEvent(error) => write!(formatter, "invalid domain event: {error}"),
            Self::InvalidPayload(message) => formatter.write_str(message),
        }
    }
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
struct WireTradeRecord<'a> {
    #[serde(borrow)]
    trades: Vec<WireTrade<'a>>,
    first_packet: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct WireTrade<'a> {
    #[serde(borrow)]
    price: &'a RawValue,
    quantity: u64,
}

#[derive(Debug, Deserialize)]
struct WireBookRecord<'a> {
    #[serde(borrow)]
    bids: Vec<WireLevel<'a>>,
    #[serde(borrow)]
    asks: Vec<WireLevel<'a>>,
}

#[derive(Debug, Deserialize)]
struct WireLevel<'a> {
    #[serde(borrow)]
    price: &'a RawValue,
    quantity: u64,
}
