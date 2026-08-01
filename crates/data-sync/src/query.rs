use std::{collections::BTreeSet, error::Error, fmt};

use market_types::{InstrumentId, MatchTime, MatchTimeError, TradingDate};

pub const TERALION_INTERFACE_VERSION: u16 = 1;
pub const MAX_PAGE_LIMIT: u16 = 5_000;

/// Teralion source market, kept separate from the domain `MarketId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ArchiveMarket {
    Twse = 1,
    Tpex = 2,
    TaifexFutures = 3,
    TaifexOptions = 4,
}

impl ArchiveMarket {
    #[must_use]
    pub const fn for_instrument(instrument: &InstrumentId) -> Self {
        match instrument.market() {
            market_types::MarketId::Twse => Self::Twse,
            market_types::MarketId::Tpex => Self::Tpex,
            market_types::MarketId::Taifex => Self::TaifexFutures,
        }
    }

    #[must_use]
    pub const fn wire_market(self) -> &'static str {
        match self {
            Self::Twse => "twse",
            Self::Tpex => "tpex",
            Self::TaifexFutures => "taifex_fut",
            Self::TaifexOptions => "taifex_opt",
        }
    }

    #[must_use]
    pub const fn domain_market(self) -> market_types::MarketId {
        match self {
            Self::Twse => market_types::MarketId::Twse,
            Self::Tpex => market_types::MarketId::Tpex,
            Self::TaifexFutures | Self::TaifexOptions => market_types::MarketId::Taifex,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TeralionCredential(Box<str>);

impl TeralionCredential {
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, QueryError> {
        let value = value.into();
        if value.is_empty() {
            return Err(QueryError::EmptyCredential);
        }
        Ok(Self(value))
    }

    /// Exposes the secret only to the online transport implementation.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TeralionCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TeralionCredential([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveTimestamp {
    exact: Box<str>,
    utc: MatchTime,
}

impl ArchiveTimestamp {
    pub fn parse(value: impl Into<Box<str>>) -> Result<Self, QueryError> {
        let exact = value.into();
        let utc = MatchTime::parse(&exact).map_err(QueryError::InvalidTimestamp)?;
        Ok(Self { exact, utc })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.exact
    }

    #[must_use]
    pub const fn utc(&self) -> MatchTime {
        self.utc
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum ArchiveKind {
    Quote = 1,
    Book = 2,
    Close = 3,
    Stats = 4,
    Trade = 5,
}

impl ArchiveKind {
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Quote => "quote",
            Self::Book => "book",
            Self::Close => "close",
            Self::Stats => "stats",
            Self::Trade => "trade",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SanitizedQueryIdentity([u8; 32]);

impl SanitizedQueryIdentity {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeralionQuery {
    Coverage {
        start: TradingDate,
        end: TradingDate,
    },
    SymbolRange {
        instrument: InstrumentId,
    },
    Ticks {
        instrument: InstrumentId,
        start: ArchiveTimestamp,
        end: ArchiveTimestamp,
        kinds: Box<[ArchiveKind]>,
        limit: u16,
        /// `None` preserves the original query identity. M5 sets this for
        /// `taifex_opt` so options cannot be read as futures.
        archive_market: Option<ArchiveMarket>,
    },
    DailyInstrument {
        instrument: InstrumentId,
        trading_date: TradingDate,
    },
}

impl TeralionQuery {
    pub fn coverage(start: TradingDate, end: TradingDate) -> Result<Self, QueryError> {
        if start > end {
            return Err(QueryError::InvalidDateWindow);
        }
        Ok(Self::Coverage { start, end })
    }

    #[must_use]
    pub const fn symbol_range(instrument: InstrumentId) -> Self {
        Self::SymbolRange { instrument }
    }

    pub fn ticks(
        instrument: InstrumentId,
        start: ArchiveTimestamp,
        end: ArchiveTimestamp,
        kinds: impl IntoIterator<Item = ArchiveKind>,
        limit: u16,
    ) -> Result<Self, QueryError> {
        Self::ticks_with_archive_market(instrument, start, end, kinds, limit, None)
    }

    pub fn ticks_for_market(
        instrument: InstrumentId,
        start: ArchiveTimestamp,
        end: ArchiveTimestamp,
        kinds: impl IntoIterator<Item = ArchiveKind>,
        limit: u16,
        archive_market: ArchiveMarket,
    ) -> Result<Self, QueryError> {
        if archive_market.domain_market() != instrument.market() {
            return Err(QueryError::ArchiveMarketMismatch);
        }
        Self::ticks_with_archive_market(instrument, start, end, kinds, limit, Some(archive_market))
    }

    fn ticks_with_archive_market(
        instrument: InstrumentId,
        start: ArchiveTimestamp,
        end: ArchiveTimestamp,
        kinds: impl IntoIterator<Item = ArchiveKind>,
        limit: u16,
        archive_market: Option<ArchiveMarket>,
    ) -> Result<Self, QueryError> {
        if start.utc() >= end.utc() {
            return Err(QueryError::InvalidTimestampWindow);
        }
        if limit == 0 || limit > MAX_PAGE_LIMIT {
            return Err(QueryError::InvalidPageLimit(limit));
        }
        let kinds = kinds.into_iter().collect::<BTreeSet<_>>();
        if kinds.is_empty() {
            return Err(QueryError::EmptyKinds);
        }
        Ok(Self::Ticks {
            instrument,
            start,
            end,
            kinds: kinds.into_iter().collect(),
            limit,
            archive_market,
        })
    }

    #[must_use]
    pub const fn daily_instrument(instrument: InstrumentId, trading_date: TradingDate) -> Self {
        Self::DailyInstrument {
            instrument,
            trading_date,
        }
    }

    #[must_use]
    pub fn identity(&self) -> SanitizedQueryIdentity {
        SanitizedQueryIdentity(*blake3::hash(&self.canonical_bytes()).as_bytes())
    }

    #[must_use]
    pub fn instrument(&self) -> Option<&InstrumentId> {
        match self {
            Self::Coverage { .. } => None,
            Self::SymbolRange { instrument }
            | Self::Ticks { instrument, .. }
            | Self::DailyInstrument { instrument, .. } => Some(instrument),
        }
    }

    #[must_use]
    pub fn archive_market(&self) -> Option<ArchiveMarket> {
        match self {
            Self::Ticks {
                instrument,
                archive_market,
                ..
            } => Some(archive_market.unwrap_or_else(|| ArchiveMarket::for_instrument(instrument))),
            _ => None,
        }
    }

    #[must_use]
    pub const fn is_paged(&self) -> bool {
        matches!(self, Self::Ticks { .. })
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"OSTQ");
        bytes.extend_from_slice(&TERALION_INTERFACE_VERSION.to_be_bytes());
        match self {
            Self::Coverage { start, end } => {
                bytes.push(1);
                bytes.extend_from_slice(&start.to_canonical_bytes());
                bytes.extend_from_slice(&end.to_canonical_bytes());
            }
            Self::SymbolRange { instrument } => {
                bytes.push(2);
                append_instrument(instrument, &mut bytes);
            }
            Self::Ticks {
                instrument,
                start,
                end,
                kinds,
                limit,
                archive_market,
            } => {
                bytes.push(3);
                append_instrument(instrument, &mut bytes);
                append_str(start.as_str(), &mut bytes);
                append_str(end.as_str(), &mut bytes);
                bytes.extend_from_slice(&(kinds.len() as u32).to_be_bytes());
                bytes.extend(kinds.iter().map(|kind| *kind as u8));
                bytes.extend_from_slice(&limit.to_be_bytes());
                if let Some(archive_market) = archive_market {
                    bytes.push(1);
                    bytes.push(*archive_market as u8);
                }
            }
            Self::DailyInstrument {
                instrument,
                trading_date,
            } => {
                bytes.push(4);
                append_instrument(instrument, &mut bytes);
                bytes.extend_from_slice(&trading_date.to_canonical_bytes());
            }
        }
        bytes
    }
}

fn append_instrument(instrument: &InstrumentId, bytes: &mut Vec<u8>) {
    bytes.push(instrument.market().discriminant());
    append_str(instrument.symbol().as_str(), bytes);
}

fn append_str(value: &str, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    EmptyCredential,
    InvalidTimestamp(MatchTimeError),
    InvalidTimestampWindow,
    InvalidDateWindow,
    InvalidPageLimit(u16),
    EmptyKinds,
    ArchiveMarketMismatch,
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCredential => formatter.write_str("Teralion credential cannot be empty"),
            Self::InvalidTimestamp(error) => error.fmt(formatter),
            Self::InvalidTimestampWindow => {
                formatter.write_str("archive timestamp start must be before end")
            }
            Self::InvalidDateWindow => {
                formatter.write_str("coverage start date must not be after end date")
            }
            Self::InvalidPageLimit(limit) => {
                write!(
                    formatter,
                    "Teralion page limit must be in 1..=5000, got {limit}"
                )
            }
            Self::EmptyKinds => formatter.write_str("ticks query requires a source kind"),
            Self::ArchiveMarketMismatch => {
                formatter.write_str("archive market does not match the domain instrument market")
            }
        }
    }
}

impl Error for QueryError {}

#[cfg(test)]
mod tests {
    use market_types::{MarketId, Symbol};

    use super::*;

    fn instrument() -> InstrumentId {
        InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap())
    }

    #[test]
    fn ticks_identity_is_canonical_and_secret_free() {
        let first = TeralionQuery::ticks(
            instrument(),
            ArchiveTimestamp::parse("2026-07-27T08:55:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-27T13:35:00+08:00").unwrap(),
            [ArchiveKind::Quote, ArchiveKind::Quote],
            5_000,
        )
        .unwrap();
        let second = TeralionQuery::ticks(
            instrument(),
            ArchiveTimestamp::parse("2026-07-27T08:55:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-27T13:35:00+08:00").unwrap(),
            [ArchiveKind::Quote],
            5_000,
        )
        .unwrap();
        assert_eq!(first.identity(), second.identity());
        assert!(!format!("{first:?}").contains("secret"));
    }

    #[test]
    fn credential_debug_is_redacted() {
        let credential = TeralionCredential::new("not-a-real-secret").unwrap();
        assert_eq!(format!("{credential:?}"), "TeralionCredential([REDACTED])");
        assert_eq!(credential.expose_secret(), "not-a-real-secret");
    }

    #[test]
    fn option_archive_market_is_explicit_and_identity_bound() {
        let instrument = InstrumentId::new(MarketId::Taifex, Symbol::new("TXO24000U6").unwrap());
        let start = ArchiveTimestamp::parse("2026-07-27T14:55:00+08:00").unwrap();
        let end = ArchiveTimestamp::parse("2026-07-28T13:50:00+08:00").unwrap();
        let option = TeralionQuery::ticks_for_market(
            instrument.clone(),
            start.clone(),
            end.clone(),
            [
                ArchiveKind::Book,
                ArchiveKind::Close,
                ArchiveKind::Stats,
                ArchiveKind::Trade,
            ],
            5_000,
            ArchiveMarket::TaifexOptions,
        )
        .unwrap();
        let futures =
            TeralionQuery::ticks(instrument, start, end, [ArchiveKind::Book], 5_000).unwrap();
        assert_eq!(option.archive_market(), Some(ArchiveMarket::TaifexOptions));
        assert_eq!(option.archive_market().unwrap().wire_market(), "taifex_opt");
        assert_ne!(option.identity(), futures.identity());
        assert!(matches!(
            TeralionQuery::ticks_for_market(
                InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
                ArchiveTimestamp::parse("2026-07-27T08:55:00+08:00").unwrap(),
                ArchiveTimestamp::parse("2026-07-27T13:35:00+08:00").unwrap(),
                [ArchiveKind::Quote],
                5_000,
                ArchiveMarket::TaifexOptions,
            ),
            Err(QueryError::ArchiveMarketMismatch)
        ));
    }
}
