use std::{collections::BTreeSet, error::Error, fmt};

use market_types::{InstrumentId, TradingDate};
use strategy_api::SessionKind;

use crate::canonical::{append_instrument, append_len, append_session};

pub const SOURCE_PARTITION_KEY_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum SourceId {
    TeralionFeedArchive = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionPlanIdentity([u8; 32]);

impl SessionPlanIdentity {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourcePartitionIdentity([u8; 32]);

impl SourcePartitionIdentity {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRevisionIdentity([u8; 32]);

impl SourceRevisionIdentity {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheIdentity([u8; 32]);

impl CacheIdentity {
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
pub struct SourcePartitionKey {
    source: SourceId,
    instrument: InstrumentId,
    trading_date: TradingDate,
    session_kinds: Box<[SessionKind]>,
    session_plan_identity: SessionPlanIdentity,
    canonical: Box<[u8]>,
    identity: SourcePartitionIdentity,
}

impl SourcePartitionKey {
    pub fn new(
        source: SourceId,
        instrument: InstrumentId,
        trading_date: TradingDate,
        session_kinds: impl IntoIterator<Item = SessionKind>,
        session_plan_identity: SessionPlanIdentity,
    ) -> Result<Self, SourcePartitionKeyError> {
        let session_kinds = session_kinds.into_iter().collect::<BTreeSet<_>>();
        if session_kinds.is_empty() {
            return Err(SourcePartitionKeyError::EmptySessions);
        }
        let session_kinds = session_kinds.into_iter().collect::<Box<[_]>>();

        let mut canonical = Vec::new();
        canonical.extend_from_slice(b"OSPK");
        canonical.extend_from_slice(&SOURCE_PARTITION_KEY_VERSION.to_be_bytes());
        canonical.push(source as u8);
        append_instrument(&instrument, &mut canonical)
            .map_err(|_| SourcePartitionKeyError::CanonicalLengthOverflow)?;
        canonical.extend_from_slice(&trading_date.to_canonical_bytes());
        append_len(session_kinds.len(), &mut canonical)
            .map_err(|_| SourcePartitionKeyError::CanonicalLengthOverflow)?;
        for session in &session_kinds {
            append_session(*session, &mut canonical);
        }
        canonical.extend_from_slice(session_plan_identity.as_bytes());
        let identity = SourcePartitionIdentity(*blake3::hash(&canonical).as_bytes());

        Ok(Self {
            source,
            instrument,
            trading_date,
            session_kinds,
            session_plan_identity,
            canonical: canonical.into_boxed_slice(),
            identity,
        })
    }

    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.source
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
    pub const fn session_kinds(&self) -> &[SessionKind] {
        &self.session_kinds
    }

    #[must_use]
    pub const fn session_plan_identity(&self) -> SessionPlanIdentity {
        self.session_plan_identity
    }

    #[must_use]
    pub const fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    #[must_use]
    pub const fn identity(&self) -> SourcePartitionIdentity {
        self.identity
    }
}

impl PartialOrd for SourcePartitionKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SourcePartitionKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.canonical.cmp(&other.canonical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePartitionKeyError {
    EmptySessions,
    CanonicalLengthOverflow,
}

impl fmt::Display for SourcePartitionKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptySessions => "source partition requires a session kind",
            Self::CanonicalLengthOverflow => "source partition canonical field exceeds u32 length",
        };
        formatter.write_str(message)
    }
}

impl Error for SourcePartitionKeyError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SourceStateKind {
    Missing = 1,
    Building = 2,
    Complete = 3,
    Incomplete = 4,
    Corrupt = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IncompleteReason {
    CursorNotTerminal = 1,
    CoverageUnconfirmed = 2,
    DailyInstrumentMissing = 3,
    UnsupportedFormat = 4,
    SessionOwnershipUnconfirmed = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CorruptReason {
    ManifestInvalid = 1,
    ReferenceMismatch = 2,
    PayloadMissing = 3,
    PayloadChecksumMismatch = 4,
    CompressionFrameInvalid = 5,
    VersionIncompatible = 6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    Missing,
    Building,
    Complete { revision: SourceRevisionIdentity },
    Incomplete { reason: IncompleteReason },
    Corrupt { reason: CorruptReason },
}

impl SourceState {
    #[must_use]
    pub const fn kind(self) -> SourceStateKind {
        match self {
            Self::Missing => SourceStateKind::Missing,
            Self::Building => SourceStateKind::Building,
            Self::Complete { .. } => SourceStateKind::Complete,
            Self::Incomplete { .. } => SourceStateKind::Incomplete,
            Self::Corrupt { .. } => SourceStateKind::Corrupt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheState {
    Missing,
    Building,
    Valid { identity: CacheIdentity },
    Stale,
    Corrupt,
}
