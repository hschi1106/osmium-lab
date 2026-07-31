use market_types::{
    CANONICAL_EVENT_VERSION, CanonicalEncodingError, DomainEvent, EVENT_SCHEMA_VERSION,
};

use crate::ORDERING_RULE_VERSION;

pub const CANONICAL_REPLAY_EVENT_STREAM_VERSION: u16 = 1;

#[derive(Debug, Clone)]
pub(crate) struct ReplayEventStreamHasher {
    hasher: blake3::Hasher,
    event_count: u64,
}

impl ReplayEventStreamHasher {
    pub(crate) fn new() -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"OSRS");
        hasher.update(&CANONICAL_REPLAY_EVENT_STREAM_VERSION.to_be_bytes());
        hasher.update(&EVENT_SCHEMA_VERSION.to_be_bytes());
        hasher.update(&CANONICAL_EVENT_VERSION.to_be_bytes());
        hasher.update(&ORDERING_RULE_VERSION.to_be_bytes());
        Self {
            hasher,
            event_count: 0,
        }
    }

    pub(crate) fn prepare_event(
        &self,
        event: &DomainEvent,
    ) -> Result<PreparedChecksumRecord, CanonicalEncodingError> {
        let canonical = event.to_canonical_bytes()?;
        let length =
            u32::try_from(canonical.len()).map_err(|_| CanonicalEncodingError::LengthOverflow)?;
        Ok(PreparedChecksumRecord { canonical, length })
    }

    pub(crate) fn append_prepared(&mut self, record: &PreparedChecksumRecord) {
        self.hasher.update(&[1]);
        self.hasher.update(&record.length.to_be_bytes());
        self.hasher.update(&record.canonical);
        self.event_count += 1;
    }

    pub(crate) fn event_count(&self) -> u64 {
        self.event_count
    }

    pub(crate) fn checksum(&self) -> ReplayEventStreamChecksum {
        let mut hasher = self.hasher.clone();
        hasher.update(&[0]);
        hasher.update(&self.event_count.to_be_bytes());
        ReplayEventStreamChecksum(*hasher.finalize().as_bytes())
    }
}

pub(crate) struct PreparedChecksumRecord {
    canonical: Vec<u8>,
    length: u32,
}

impl PreparedChecksumRecord {
    pub(crate) fn canonical(&self) -> &[u8] {
        &self.canonical
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplayEventStreamChecksum([u8; 32]);

impl ReplayEventStreamChecksum {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
