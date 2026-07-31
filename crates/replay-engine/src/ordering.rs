use std::{error::Error, fmt};

use market_types::{
    CanonicalEncodingError, DomainEvent, EventFingerprint, EventPayload, MatchTime, SourceFormatId,
    Symbol, TradePrintKind,
};

pub const ORDERING_RULE_VERSION: u16 = 3;

/// Fully materialized version-2 deterministic event ordering key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderingKey {
    match_time: MatchTime,
    market_rank: u8,
    symbol: Symbol,
    source_format: SourceFormatId,
    source_phase_rank: u8,
    event_kind_rank: u8,
    source_sequence: Option<u64>,
    event_fingerprint: EventFingerprint,
}

impl OrderingKey {
    pub fn for_event(event: &DomainEvent) -> Result<Self, OrderingError> {
        Ok(Self {
            match_time: event.match_time(),
            market_rank: event.instrument().market().ordering_rank(),
            symbol: event.instrument().symbol().clone(),
            source_format: event.source_format().clone(),
            source_phase_rank: source_phase_rank(event)?,
            event_kind_rank: event.payload().discriminant(),
            source_sequence: event.source_sequence(),
            event_fingerprint: event
                .fingerprint()
                .map_err(OrderingError::CanonicalEncoding)?,
        })
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
    pub const fn source_phase_rank(&self) -> u8 {
        self.source_phase_rank
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
        let key = OrderingKey::for_event(&event)?;
        let canonical = event
            .to_canonical_bytes()
            .map_err(OrderingError::CanonicalEncoding)?;
        Ok(Self {
            event,
            key,
            canonical,
        })
    }
}

fn source_phase_rank(event: &DomainEvent) -> Result<u8, OrderingError> {
    if event.instrument().market() != market_types::MarketId::Twse
        || event.source_format().as_str() != "STOCK_REALTIME"
    {
        return Ok(0);
    }

    match event.payload() {
        EventPayload::QuoteSnapshot(_) => Ok(20),
        EventPayload::TradeBatch(batch)
            if !batch.trades().is_empty()
                && batch
                    .trades()
                    .iter()
                    .all(|trade| trade.print_kind() == TradePrintKind::Intermediate) =>
        {
            Ok(10)
        }
        EventPayload::IndicativeOpeningAuction(auction)
        | EventPayload::IndicativeClosingAuction(auction) => {
            Ok(if auction.book().as_set().is_some() {
                20
            } else {
                10
            })
        }
        EventPayload::BookSnapshot(_) | EventPayload::TradeBatch(_) => {
            Err(OrderingError::InvalidTwseRealtimeShape)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderingError {
    CanonicalEncoding(CanonicalEncodingError),
    InvalidTwseRealtimeShape,
    EventFingerprintCollision,
}

impl fmt::Display for OrderingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalEncoding(error) => {
                write!(formatter, "canonical event encoding failed: {error}")
            }
            Self::InvalidTwseRealtimeShape => {
                formatter.write_str("invalid TWSE STOCK_REALTIME ordering shape")
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
            Self::InvalidTwseRealtimeShape | Self::EventFingerprintCollision => None,
        }
    }
}
