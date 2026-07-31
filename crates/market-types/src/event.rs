use std::{error::Error, fmt};

use crate::{
    BookLevel, BookSide, BookSideKind, CanonicalEncodingError, CanonicalValue,
    CompleteBookSnapshot, Decimal, InstrumentId, MarketAnnotations, MarketId, MatchTime,
    Observation, Price, Quantity, QuantityUnit, SourceFormatId, Symbol, TpexQuoteAnnotations,
    TradeError, TradeOrder, TradePrint, TradePrintKind, TradingDate, TwseQuoteAnnotations,
    UnknownValue, Volume, append_bytes, append_length, append_optional_u64,
    trade::validate_trade_units,
};

pub const MARKET_TYPES_VERSION: u16 = 3;
pub const EVENT_SCHEMA_VERSION: u16 = 3;
pub const CANONICAL_EVENT_VERSION: u16 = 3;
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

/// An indicative call-auction observation that is not an executed trade.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndicativeAuction {
    price: Observation<Price>,
    quantity: Observation<Quantity>,
    book: Observation<CompleteBookSnapshot>,
    cumulative_volume: Observation<Volume>,
    annotations: MarketAnnotations,
}

impl IndicativeAuction {
    pub fn new(
        price: Observation<Price>,
        quantity: Observation<Quantity>,
        book: Observation<CompleteBookSnapshot>,
        cumulative_volume: Observation<Volume>,
        annotations: MarketAnnotations,
    ) -> Result<Self, EventError> {
        validate_auction_units(&quantity, &book, &cumulative_volume)?;
        Ok(Self {
            price,
            quantity,
            book,
            cumulative_volume,
            annotations,
        })
    }

    #[must_use]
    pub const fn price(&self) -> &Observation<Price> {
        &self.price
    }

    #[must_use]
    pub const fn quantity(&self) -> &Observation<Quantity> {
        &self.quantity
    }

    #[must_use]
    pub const fn book(&self) -> &Observation<CompleteBookSnapshot> {
        &self.book
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
    IndicativeOpeningAuction(IndicativeAuction),
    IndicativeClosingAuction(IndicativeAuction),
}

impl EventPayload {
    #[must_use]
    pub const fn discriminant(&self) -> u8 {
        self.kind().discriminant()
    }

    #[must_use]
    pub const fn kind(&self) -> EventKind {
        match self {
            Self::QuoteSnapshot(_) => EventKind::QuoteSnapshot,
            Self::BookSnapshot(_) => EventKind::BookSnapshot,
            Self::TradeBatch(_) => EventKind::TradeBatch,
            Self::IndicativeOpeningAuction(_) => EventKind::IndicativeOpeningAuction,
            Self::IndicativeClosingAuction(_) => EventKind::IndicativeClosingAuction,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum EventKind {
    QuoteSnapshot = 10,
    BookSnapshot = 20,
    TradeBatch = 30,
    IndicativeOpeningAuction = 40,
    IndicativeClosingAuction = 50,
}

impl EventKind {
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
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

    /// Encodes the version-3 canonical event frame.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, CanonicalEncodingError> {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(CANONICAL_MAGIC);
        bytes.extend_from_slice(&CANONICAL_EVENT_VERSION.to_be_bytes());
        bytes.extend_from_slice(&EVENT_SCHEMA_VERSION.to_be_bytes());
        bytes.push(self.instrument.market().discriminant());
        append_bytes(self.instrument.symbol().as_bytes(), &mut bytes)?;
        bytes.extend_from_slice(&self.trading_date.to_canonical_bytes());
        append_bytes(self.source_format.as_bytes(), &mut bytes)?;
        bytes.extend_from_slice(&self.match_time.as_unix_microseconds().to_be_bytes());
        append_optional_u64(self.source_sequence, &mut bytes);
        bytes.push(self.payload.discriminant());
        encode_payload(&self.payload, &mut bytes)?;
        Ok(bytes)
    }

    /// Decodes and validates one complete version-3 canonical event frame.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, CanonicalDecodingError> {
        let mut parser = CanonicalParser::new(bytes);
        if parser.take(4)? != CANONICAL_MAGIC
            || parser.u16()? != CANONICAL_EVENT_VERSION
            || parser.u16()? != EVENT_SCHEMA_VERSION
        {
            return Err(CanonicalDecodingError::UnsupportedHeader);
        }
        let market = MarketId::from_discriminant(parser.u8()?)
            .map_err(|_| CanonicalDecodingError::InvalidValue)?;
        let symbol =
            Symbol::new(parser.string()?).map_err(|_| CanonicalDecodingError::InvalidValue)?;
        let trading_date = TradingDate::from_epoch_days(parser.i32()?);
        let source_format = SourceFormatId::new(parser.string()?)
            .map_err(|_| CanonicalDecodingError::InvalidValue)?;
        let match_time = MatchTime::from_unix_microseconds(parser.i64()?);
        let source_sequence = match parser.u8()? {
            0 => None,
            1 => Some(parser.u64()?),
            _ => return Err(CanonicalDecodingError::InvalidValue),
        };
        let payload = match parser.u8()? {
            10 => EventPayload::QuoteSnapshot(
                QuoteSnapshot::new(
                    parser.book()?,
                    parser.observation(CanonicalParser::trade)?,
                    parser.observation(CanonicalParser::volume)?,
                    parser.annotations()?,
                )
                .map_err(|_| CanonicalDecodingError::InvalidValue)?,
            ),
            20 => {
                EventPayload::BookSnapshot(BookSnapshot::new(parser.book()?, parser.annotations()?))
            }
            30 => {
                let count = parser.u32()? as usize;
                let mut trades = Vec::with_capacity(count);
                for _ in 0..count {
                    trades.push(parser.trade()?);
                }
                let order = match parser.u8()? {
                    0 => TradeOrder::Unspecified,
                    1 => TradeOrder::SourceOrdered,
                    _ => return Err(CanonicalDecodingError::InvalidValue),
                };
                EventPayload::TradeBatch(
                    TradeBatch::new(
                        trades,
                        order,
                        parser.observation(CanonicalParser::volume)?,
                        parser.annotations()?,
                    )
                    .map_err(|_| CanonicalDecodingError::InvalidValue)?,
                )
            }
            40 => EventPayload::IndicativeOpeningAuction(
                IndicativeAuction::new(
                    parser.observation(CanonicalParser::price)?,
                    parser.observation(CanonicalParser::quantity)?,
                    parser.observation(CanonicalParser::book)?,
                    parser.observation(CanonicalParser::volume)?,
                    parser.annotations()?,
                )
                .map_err(|_| CanonicalDecodingError::InvalidValue)?,
            ),
            50 => EventPayload::IndicativeClosingAuction(
                IndicativeAuction::new(
                    parser.observation(CanonicalParser::price)?,
                    parser.observation(CanonicalParser::quantity)?,
                    parser.observation(CanonicalParser::book)?,
                    parser.observation(CanonicalParser::volume)?,
                    parser.annotations()?,
                )
                .map_err(|_| CanonicalDecodingError::InvalidValue)?,
            ),
            _ => return Err(CanonicalDecodingError::InvalidValue),
        };
        if !parser.is_finished() {
            return Err(CanonicalDecodingError::TrailingBytes);
        }
        Ok(Self::new(
            InstrumentId::new(market, symbol),
            trading_date,
            source_format,
            match_time,
            source_sequence,
            payload,
        ))
    }

    /// Computes BLAKE3-256 over the canonical event frame.
    pub fn fingerprint(&self) -> Result<EventFingerprint, CanonicalEncodingError> {
        let bytes = self.to_canonical_bytes()?;
        Ok(EventFingerprint(*blake3::hash(&bytes).as_bytes()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalDecodingError {
    Truncated,
    UnsupportedHeader,
    InvalidUtf8,
    InvalidValue,
    TrailingBytes,
}

impl fmt::Display for CanonicalDecodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "canonical event decoding failed: {self:?}")
    }
}

impl Error for CanonicalDecodingError {}

struct CanonicalParser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CanonicalParser<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], CanonicalDecodingError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(CanonicalDecodingError::Truncated)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(CanonicalDecodingError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], CanonicalDecodingError> {
        self.take(N)?
            .try_into()
            .map_err(|_| CanonicalDecodingError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, CanonicalDecodingError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, CanonicalDecodingError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, CanonicalDecodingError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, CanonicalDecodingError> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, CanonicalDecodingError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn i64(&mut self) -> Result<i64, CanonicalDecodingError> {
        Ok(i64::from_be_bytes(self.array()?))
    }

    fn i128(&mut self) -> Result<i128, CanonicalDecodingError> {
        Ok(i128::from_be_bytes(self.array()?))
    }

    fn bytes(&mut self) -> Result<&'a [u8], CanonicalDecodingError> {
        let length = self.u32()? as usize;
        self.take(length)
    }

    fn string(&mut self) -> Result<&'a str, CanonicalDecodingError> {
        std::str::from_utf8(self.bytes()?).map_err(|_| CanonicalDecodingError::InvalidUtf8)
    }

    fn price(&mut self) -> Result<Price, CanonicalDecodingError> {
        Price::new(Decimal::from_atoms(self.i128()?))
            .map_err(|_| CanonicalDecodingError::InvalidValue)
    }

    fn quantity(&mut self) -> Result<Quantity, CanonicalDecodingError> {
        let unit = QuantityUnit::from_discriminant(self.u8()?)
            .map_err(|_| CanonicalDecodingError::InvalidValue)?;
        Quantity::new(self.u64()?, unit).map_err(|_| CanonicalDecodingError::InvalidValue)
    }

    fn volume(&mut self) -> Result<Volume, CanonicalDecodingError> {
        let unit = QuantityUnit::from_discriminant(self.u8()?)
            .map_err(|_| CanonicalDecodingError::InvalidValue)?;
        Ok(Volume::new(self.u64()?, unit))
    }

    fn trade(&mut self) -> Result<TradePrint, CanonicalDecodingError> {
        let price = self.price()?;
        let quantity = self.quantity()?;
        let kind = match self.u8()? {
            0 => TradePrintKind::Regular,
            1 => TradePrintKind::Intermediate,
            _ => return Err(CanonicalDecodingError::InvalidValue),
        };
        Ok(TradePrint::new(price, quantity, kind))
    }

    fn book(&mut self) -> Result<CompleteBookSnapshot, CanonicalDecodingError> {
        let bids = self.book_side(BookSideKind::Bid)?;
        let asks = self.book_side(BookSideKind::Ask)?;
        CompleteBookSnapshot::new(bids, asks).map_err(|_| CanonicalDecodingError::InvalidValue)
    }

    fn book_side(&mut self, kind: BookSideKind) -> Result<BookSide, CanonicalDecodingError> {
        let mut slots = [None; 5];
        for slot in &mut slots {
            *slot = match self.u8()? {
                0 => None,
                1 => Some(BookLevel::new(self.price()?, self.quantity()?)),
                _ => return Err(CanonicalDecodingError::InvalidValue),
            };
        }
        BookSide::from_slots(kind, slots).map_err(|_| CanonicalDecodingError::InvalidValue)
    }

    fn observation<T>(
        &mut self,
        decode: fn(&mut Self) -> Result<T, CanonicalDecodingError>,
    ) -> Result<Observation<T>, CanonicalDecodingError> {
        match self.u8()? {
            0 => Ok(Observation::NoObservation),
            1 => Ok(Observation::Set(decode(self)?)),
            2 => Ok(Observation::Clear),
            3 => Ok(Observation::Unknown(self.unknown()?)),
            _ => Err(CanonicalDecodingError::InvalidValue),
        }
    }

    fn unknown(&mut self) -> Result<UnknownValue, CanonicalDecodingError> {
        match self.u8()? {
            1 => Ok(UnknownValue::Unsigned(self.u64()?)),
            2 => Ok(UnknownValue::Signed(self.i64()?)),
            3 => Ok(UnknownValue::Decimal(Decimal::from_atoms(self.i128()?))),
            4 => Ok(UnknownValue::Text(self.string()?.into())),
            5 => Ok(UnknownValue::Bytes(self.bytes()?.into())),
            _ => Err(CanonicalDecodingError::InvalidValue),
        }
    }

    fn annotations(&mut self) -> Result<MarketAnnotations, CanonicalDecodingError> {
        match self.u8()? {
            0 => Ok(MarketAnnotations::None),
            1 => Ok(MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(
                self.u8()?,
                self.u8()?,
            ))),
            2 => Ok(MarketAnnotations::TpexQuote(TpexQuoteAnnotations::new(
                self.u8()?,
                self.u8()?,
            ))),
            _ => Err(CanonicalDecodingError::InvalidValue),
        }
    }

    const fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
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

fn validate_auction_units(
    quantity: &Observation<Quantity>,
    book: &Observation<CompleteBookSnapshot>,
    cumulative_volume: &Observation<Volume>,
) -> Result<(), EventError> {
    let mut expected = None;
    if let Observation::Set(quantity) = quantity {
        validate_unit(&mut expected, quantity.unit())?;
    }
    if let Observation::Set(book) = book
        && let Some(unit) = book.quantity_unit()
    {
        validate_unit(&mut expected, unit)?;
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
            snapshot.book().append_canonical(bytes)?;
            snapshot.trade().append_canonical(bytes)?;
            snapshot.cumulative_volume().append_canonical(bytes)?;
            snapshot.annotations().append_canonical(bytes)?;
        }
        EventPayload::BookSnapshot(snapshot) => {
            snapshot.book().append_canonical(bytes)?;
            snapshot.annotations().append_canonical(bytes)?;
        }
        EventPayload::TradeBatch(batch) => {
            append_length(batch.trades().len(), bytes)?;
            for trade in batch.trades() {
                trade.append_canonical(bytes)?;
            }
            bytes.push(batch.trade_order().discriminant());
            batch.cumulative_volume().append_canonical(bytes)?;
            batch.annotations().append_canonical(bytes)?;
        }
        EventPayload::IndicativeOpeningAuction(auction)
        | EventPayload::IndicativeClosingAuction(auction) => {
            auction.price().append_canonical(bytes)?;
            auction.quantity().append_canonical(bytes)?;
            auction.book().append_canonical(bytes)?;
            auction.cumulative_volume().append_canonical(bytes)?;
            auction.annotations().append_canonical(bytes)?;
        }
    }
    Ok(())
}
