use std::{error::Error, fmt};

use market_types::{
    CanonicalEncodingError, DomainEvent, EventFingerprint, MatchTime, SourceFormatId, Symbol,
};

pub const ORDERING_RULE_VERSION: u16 = 5;

/// Fully materialized deterministic event ordering key (rule v5).
///
/// Events order by the time they became observable, then by market and source
/// stream. Within one sequenced stream the exchange sequence is authoritative, so
/// `observed_time` (a running maximum of exchange time) never contradicts it.
/// Symbol, kind, match time and fingerprint only break ties among unsequenced
/// events.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderingKey {
    observed_time: MatchTime,
    market_rank: u8,
    source_format: SourceFormatId,
    source_sequence: Option<u64>,
    symbol: Symbol,
    event_kind_rank: u8,
    match_time: MatchTime,
    event_fingerprint: EventFingerprint,
}

impl OrderingKey {
    pub fn for_event(event: &DomainEvent) -> Result<Self, OrderingError> {
        Self::for_event_and_canonical(event).map(|(key, _)| key)
    }

    pub(crate) fn for_event_and_canonical(
        event: &DomainEvent,
    ) -> Result<(Self, Vec<u8>), OrderingError> {
        let canonical = event
            .to_canonical_bytes()
            .map_err(OrderingError::CanonicalEncoding)?;
        let fingerprint = EventFingerprint::hash_canonical_bytes(&canonical);
        let key = Self::for_event_with_fingerprint(event, fingerprint);
        Ok((key, canonical))
    }

    fn for_event_with_fingerprint(
        event: &DomainEvent,
        event_fingerprint: EventFingerprint,
    ) -> Self {
        Self {
            observed_time: event.observed_time(),
            market_rank: event.instrument().market().ordering_rank(),
            source_format: *event.source_format(),
            source_sequence: event.source_sequence(),
            symbol: *event.instrument().symbol(),
            event_kind_rank: event.payload().discriminant(),
            match_time: event.match_time(),
            event_fingerprint,
        }
    }

    #[must_use]
    pub const fn observed_time(&self) -> MatchTime {
        self.observed_time
    }

    #[must_use]
    pub const fn match_time(&self) -> MatchTime {
        self.match_time
    }

    #[must_use]
    pub const fn market_rank(&self) -> u8 {
        self.market_rank
    }

    #[must_use]
    pub const fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    #[must_use]
    pub const fn source_format(&self) -> &SourceFormatId {
        &self.source_format
    }

    #[must_use]
    pub const fn event_kind_rank(&self) -> u8 {
        self.event_kind_rank
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

/// Sorts a rebuild-time collection by the full ordering key without dropping duplicates.
pub fn order_events(events: Vec<DomainEvent>) -> Result<Vec<DomainEvent>, OrderingError> {
    let mut events = events
        .into_iter()
        .map(PreparedOrderEvent::new)
        .collect::<Result<Vec<_>, _>>()?;
    events.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    for pair in events.windows(2) {
        if pair[0].key == pair[1].key && pair[0].canonical != pair[1].canonical {
            return Err(OrderingError::EventFingerprintCollision);
        }
    }
    Ok(events.into_iter().map(|prepared| prepared.event).collect())
}

struct PreparedOrderEvent {
    event: DomainEvent,
    key: OrderingKey,
    canonical: Vec<u8>,
}

impl PreparedOrderEvent {
    fn new(event: DomainEvent) -> Result<Self, OrderingError> {
        let (key, canonical) = OrderingKey::for_event_and_canonical(&event)?;
        Ok(Self {
            event,
            key,
            canonical,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderingError {
    CanonicalEncoding(CanonicalEncodingError),
    EventFingerprintCollision,
}

impl fmt::Display for OrderingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalEncoding(error) => {
                write!(formatter, "canonical event encoding failed: {error}")
            }
            Self::EventFingerprintCollision => {
                formatter.write_str("equal ordering keys have different canonical event bytes")
            }
        }
    }
}

impl Error for OrderingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CanonicalEncoding(error) => Some(error),
            Self::EventFingerprintCollision => None,
        }
    }
}
