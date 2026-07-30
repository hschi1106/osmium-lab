use std::{error::Error, fmt};

use crate::{
    CompleteBookSnapshot, InstrumentId, MarketAnnotations, MatchTime, Observation, QuantityUnit,
    SourceFormatId, TradeError, TradeOrder, TradePrint, TradingDate, UnknownValue, Volume,
    trade::validate_trade_units,
};

pub const MARKET_TYPES_VERSION: u16 = 1;
pub const EVENT_SCHEMA_VERSION: u16 = 1;
pub const CANONICAL_EVENT_VERSION: u16 = 1;
const CANONICAL_MAGIC: &[u8; 4] = b"OSME";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QuoteSnapshot {
    book: CompleteBookSnapshot,
    trade: Observation<TradePrint>,
    cumulative_volume: Observation<Volume>,
    annotations: MarketAnnotations,
}

impl QuoteSnapshot {
    pub fn new(
        book: CompleteBookSnapshot,
        trade: Observation<TradePrint>,
        cumulative_volume: Observation<Volume>,
        annotations: MarketAnnotations,
    ) -> Result<Self, EventError> {
        validate_snapshot_units(&book, &trade, &cumulative_volume)?;
        Ok(Self {
            book,
            trade,
            cumulative_volume,
            annotations,
        })
    }

    #[must_use]
    pub const fn book(&self) -> &CompleteBookSnapshot {
        &self.book
    }

    #[must_use]
    pub const fn trade(&self) -> &Observation<TradePrint> {
        &self.trade
    }

    #[must_use]
    pub const fn cumulative_volume(&self) -> &Observation<Volume> {
        &self.cumulative_volume
    }

    #[must_use]
    pub const fn annotations(&self) -> &MarketAnnotations {
        &self.annotations
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BookSnapshot {
    book: CompleteBookSnapshot,
    annotations: MarketAnnotations,
}

impl BookSnapshot {
    #[must_use]
    pub const fn new(book: CompleteBookSnapshot, annotations: MarketAnnotations) -> Self {
        Self { book, annotations }
    }

    #[must_use]
    pub const fn book(&self) -> &CompleteBookSnapshot {
        &self.book
    }

    #[must_use]
    pub const fn annotations(&self) -> &MarketAnnotations {
        &self.annotations
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TradeBatch {
    trades: Box<[TradePrint]>,
    trade_order: TradeOrder,
    cumulative_volume: Observation<Volume>,
    annotations: MarketAnnotations,
}

impl TradeBatch {
    pub fn new(
        trades: Vec<TradePrint>,
        trade_order: TradeOrder,
        cumulative_volume: Observation<Volume>,
        annotations: MarketAnnotations,
    ) -> Result<Self, EventError> {
        let unit = validate_trade_units(&trades).map_err(EventError::InvalidTrades)?;
        if let Observation::Set(volume) = &cumulative_volume
            && volume.unit() != unit
        {
            return Err(EventError::QuantityUnitMismatch {
                expected: unit,
                actual: volume.unit(),
            });
        }
        Ok(Self {
            trades: trades.into_boxed_slice(),
            trade_order,
            cumulative_volume,
            annotations,
        })
    }

    #[must_use]
    pub const fn trades(&self) -> &[TradePrint] {
        &self.trades
    }

    #[must_use]
    pub const fn trade_order(&self) -> TradeOrder {
        self.trade_order
    }

    #[must_use]
    pub const fn cumulative_volume(&self) -> &Observation<Volume> {
        &self.cumulative_volume
    }

    #[must_use]
    pub const fn annotations(&self) -> &MarketAnnotations {
        &self.annotations
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventPayload {
    QuoteSnapshot(QuoteSnapshot),
    BookSnapshot(BookSnapshot),
    TradeBatch(TradeBatch),
}

impl EventPayload {
    #[must_use]
    pub const fn discriminant(&self) -> u8 {
        match self {
            Self::QuoteSnapshot(_) => 10,
            Self::BookSnapshot(_) => 20,
            Self::TradeBatch(_) => 30,
        }
    }
}

/// A source-independent, validated market event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DomainEvent {
    instrument: InstrumentId,
    trading_date: TradingDate,
    source_format: SourceFormatId,
    match_time: MatchTime,
    source_sequence: Option<u64>,
    payload: EventPayload,
}

impl DomainEvent {
    #[must_use]
    pub const fn new(
        instrument: InstrumentId,
        trading_date: TradingDate,
        source_format: SourceFormatId,
        match_time: MatchTime,
        source_sequence: Option<u64>,
        payload: EventPayload,
    ) -> Self {
        Self {
            instrument,
            trading_date,
            source_format,
            match_time,
            source_sequence,
            payload,
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
    pub const fn source_format(&self) -> &SourceFormatId {
        &self.source_format
    }

    #[must_use]
    pub const fn match_time(&self) -> MatchTime {
        self.match_time
    }

    #[must_use]
    pub const fn source_sequence(&self) -> Option<u64> {
        self.source_sequence
    }

    #[must_use]
    pub const fn payload(&self) -> &EventPayload {
        &self.payload
    }

    /// Encodes the version-1 canonical event frame.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, CanonicalEncodingError> {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(CANONICAL_MAGIC);
        bytes.extend_from_slice(&CANONICAL_EVENT_VERSION.to_be_bytes());
        bytes.extend_from_slice(&EVENT_SCHEMA_VERSION.to_be_bytes());
        bytes.push(self.instrument.market().discriminant());
        encode_bytes(self.instrument.symbol().as_bytes(), &mut bytes)?;
        bytes.extend_from_slice(&self.trading_date.to_canonical_bytes());
        encode_bytes(self.source_format.as_bytes(), &mut bytes)?;
        bytes.extend_from_slice(&self.match_time.as_unix_microseconds().to_be_bytes());
        match self.source_sequence {
            None => bytes.push(0),
            Some(sequence) => {
                bytes.push(1);
                bytes.extend_from_slice(&sequence.to_be_bytes());
            }
        }
        bytes.push(self.payload.discriminant());
        encode_payload(&self.payload, &mut bytes)?;
        Ok(bytes)
    }

    /// Computes BLAKE3-256 over the canonical event frame.
    pub fn fingerprint(&self) -> Result<EventFingerprint, CanonicalEncodingError> {
        let bytes = self.to_canonical_bytes()?;
        Ok(EventFingerprint(*blake3::hash(&bytes).as_bytes()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventFingerprint([u8; 32]);

impl EventFingerprint {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventError {
    InvalidTrades(TradeError),
    QuantityUnitMismatch {
        expected: QuantityUnit,
        actual: QuantityUnit,
    },
}

impl fmt::Display for EventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTrades(error) => error.fmt(formatter),
            Self::QuantityUnitMismatch { expected, actual } => write!(
                formatter,
                "event quantity unit mismatch: {expected:?} != {actual:?}"
            ),
        }
    }
}

impl Error for EventError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidTrades(error) => Some(error),
            Self::QuantityUnitMismatch { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalEncodingError {
    LengthOverflow,
}

impl fmt::Display for CanonicalEncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthOverflow => {
                formatter.write_str("canonical string, byte value, or vector exceeds u32 length")
            }
        }
    }
}

impl Error for CanonicalEncodingError {}

fn validate_snapshot_units(
    book: &CompleteBookSnapshot,
    trade: &Observation<TradePrint>,
    cumulative_volume: &Observation<Volume>,
) -> Result<(), EventError> {
    let mut expected = book.quantity_unit();
    if let Observation::Set(trade) = trade {
        validate_unit(&mut expected, trade.quantity().unit())?;
    }
    if let Observation::Set(volume) = cumulative_volume {
        validate_unit(&mut expected, volume.unit())?;
    }
    Ok(())
}

fn validate_unit(
    expected: &mut Option<QuantityUnit>,
    actual: QuantityUnit,
) -> Result<(), EventError> {
    match expected {
        Some(expected) if *expected != actual => Err(EventError::QuantityUnitMismatch {
            expected: *expected,
            actual,
        }),
        Some(_) => Ok(()),
        None => {
            *expected = Some(actual);
            Ok(())
        }
    }
}

fn encode_payload(
    payload: &EventPayload,
    bytes: &mut Vec<u8>,
) -> Result<(), CanonicalEncodingError> {
    match payload {
        EventPayload::QuoteSnapshot(snapshot) => {
            encode_book(snapshot.book(), bytes);
            encode_observation(snapshot.trade(), bytes, encode_trade)?;
            encode_observation(snapshot.cumulative_volume(), bytes, |volume, bytes| {
                encode_volume(*volume, bytes);
                Ok(())
            })?;
            encode_annotations(snapshot.annotations(), bytes);
        }
        EventPayload::BookSnapshot(snapshot) => {
            encode_book(snapshot.book(), bytes);
            encode_annotations(snapshot.annotations(), bytes);
        }
        EventPayload::TradeBatch(batch) => {
            encode_len(batch.trades().len(), bytes)?;
            for trade in batch.trades() {
                encode_trade(trade, bytes)?;
            }
            bytes.push(batch.trade_order().discriminant());
            encode_observation(batch.cumulative_volume(), bytes, |volume, bytes| {
                encode_volume(*volume, bytes);
                Ok(())
            })?;
            encode_annotations(batch.annotations(), bytes);
        }
    }
    Ok(())
}

fn encode_book(book: &CompleteBookSnapshot, bytes: &mut Vec<u8>) {
    for side in [book.bids(), book.asks()] {
        for slot in side.slots() {
            match slot {
                None => bytes.push(0),
                Some(level) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&level.price().to_canonical_bytes());
                    bytes.extend_from_slice(&level.displayed_quantity().to_canonical_bytes());
                }
            }
        }
    }
}

fn encode_trade(trade: &TradePrint, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
    bytes.extend_from_slice(&trade.price().to_canonical_bytes());
    bytes.extend_from_slice(&trade.quantity().to_canonical_bytes());
    bytes.push(trade.print_kind().discriminant());
    Ok(())
}

fn encode_volume(volume: Volume, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&volume.to_canonical_bytes());
}

fn encode_annotations(annotations: &MarketAnnotations, bytes: &mut Vec<u8>) {
    bytes.push(annotations.discriminant());
    if let MarketAnnotations::TwseQuote(annotation) = annotations {
        bytes.push(annotation.status_flags_raw());
        bytes.push(annotation.limit_flags_raw());
    }
}

fn encode_observation<T>(
    observation: &Observation<T>,
    bytes: &mut Vec<u8>,
    encode_value: impl FnOnce(&T, &mut Vec<u8>) -> Result<(), CanonicalEncodingError>,
) -> Result<(), CanonicalEncodingError> {
    bytes.push(observation.discriminant());
    match observation {
        Observation::Set(value) => encode_value(value, bytes)?,
        Observation::Unknown(value) => encode_unknown(value, bytes)?,
        Observation::NoObservation | Observation::Clear => {}
    }
    Ok(())
}

fn encode_unknown(value: &UnknownValue, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
    bytes.push(value.discriminant());
    match value {
        UnknownValue::Unsigned(value) => bytes.extend_from_slice(&value.to_be_bytes()),
        UnknownValue::Signed(value) => bytes.extend_from_slice(&value.to_be_bytes()),
        UnknownValue::Decimal(value) => bytes.extend_from_slice(&value.to_canonical_bytes()),
        UnknownValue::Text(value) => encode_bytes(value.as_bytes(), bytes)?,
        UnknownValue::Bytes(value) => encode_bytes(value, bytes)?,
    }
    Ok(())
}

fn encode_bytes(value: &[u8], bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
    encode_len(value.len(), bytes)?;
    bytes.extend_from_slice(value);
    Ok(())
}

fn encode_len(length: usize, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
    let length = u32::try_from(length).map_err(|_| CanonicalEncodingError::LengthOverflow)?;
    bytes.extend_from_slice(&length.to_be_bytes());
    Ok(())
}
