use std::{error::Error, fmt};

use market_types::{
    CanonicalEncodingError, CanonicalValue, InstrumentId, append_bytes, append_length,
    append_optional_u64,
};

use crate::{
    AppliedEventRef, MARKET_STATE_VERSION, MarketState, StateField, TradeObservation,
    UnavailableReason,
};

pub const CANONICAL_MARKET_STATE_VERSION: u16 = 2;
pub const CANONICAL_FINAL_STATE_SET_VERSION: u16 = 2;

impl MarketState {
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, CanonicalEncodingError> {
        let mut bytes = Vec::with_capacity(512);
        bytes.extend_from_slice(b"OSMS");
        bytes.extend_from_slice(&CANONICAL_MARKET_STATE_VERSION.to_be_bytes());
        bytes.extend_from_slice(&MARKET_STATE_VERSION.to_be_bytes());
        append_instrument(self.instrument(), &mut bytes)?;
        bytes.extend_from_slice(&self.trading_date().to_canonical_bytes());
        match self.current_segment_id() {
            None => bytes.push(0),
            Some(segment) => {
                bytes.push(1);
                append_bytes(segment.as_bytes(), &mut bytes)?;
            }
        }
        bytes.extend_from_slice(&self.state_version().to_be_bytes());
        append_state_field(self.book(), &mut bytes)?;
        append_state_field(self.recent_trade(), &mut bytes)?;
        append_state_field(self.cumulative_volume(), &mut bytes)?;
        append_state_field(self.last_annotations(), &mut bytes)?;
        match self.last_event() {
            None => bytes.push(0),
            Some(event) => {
                bytes.push(1);
                append_event_ref(event, &mut bytes)?;
            }
        }
        Ok(bytes)
    }

    pub fn fingerprint(&self) -> Result<StateFingerprint, CanonicalEncodingError> {
        Ok(StateFingerprint(
            *blake3::hash(&self.to_canonical_bytes()?).as_bytes(),
        ))
    }
}

impl CanonicalValue for TradeObservation {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        match self {
            Self::Single(trade) => {
                bytes.push(1);
                trade.append_canonical(bytes)?;
            }
            Self::Batch {
                trades,
                trade_order,
            } => {
                bytes.push(2);
                bytes.push(trade_order.discriminant());
                append_length(trades.len(), bytes)?;
                for trade in trades {
                    trade.append_canonical(bytes)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateFingerprint([u8; 32]);

impl StateFingerprint {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FinalStateChecksum([u8; 32]);

impl FinalStateChecksum {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

pub fn canonical_final_state_set<'a>(
    states: impl IntoIterator<Item = &'a MarketState>,
) -> Result<Vec<u8>, FinalStateEncodingError> {
    let mut states = states.into_iter().collect::<Vec<_>>();
    states.sort_by(|left, right| left.instrument().cmp(right.instrument()));
    if states
        .windows(2)
        .any(|pair| pair[0].instrument() == pair[1].instrument())
    {
        return Err(FinalStateEncodingError::DuplicateInstrument);
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"OSMF");
    bytes.extend_from_slice(&CANONICAL_FINAL_STATE_SET_VERSION.to_be_bytes());
    append_length(states.len(), &mut bytes).map_err(FinalStateEncodingError::Canonical)?;
    for state in states {
        let state_bytes = state
            .to_canonical_bytes()
            .map_err(FinalStateEncodingError::Canonical)?;
        append_length(state_bytes.len(), &mut bytes).map_err(FinalStateEncodingError::Canonical)?;
        bytes.extend_from_slice(&state_bytes);
    }
    Ok(bytes)
}

pub fn final_state_checksum<'a>(
    states: impl IntoIterator<Item = &'a MarketState>,
) -> Result<FinalStateChecksum, FinalStateEncodingError> {
    let bytes = canonical_final_state_set(states)?;
    Ok(FinalStateChecksum(*blake3::hash(&bytes).as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalStateEncodingError {
    Canonical(CanonicalEncodingError),
    DuplicateInstrument,
}

impl fmt::Display for FinalStateEncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Canonical(error) => error.fmt(formatter),
            Self::DuplicateInstrument => {
                formatter.write_str("final state set contains a duplicate instrument")
            }
        }
    }
}

impl Error for FinalStateEncodingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Canonical(error) => Some(error),
            Self::DuplicateInstrument => None,
        }
    }
}

fn append_instrument(
    instrument: &InstrumentId,
    bytes: &mut Vec<u8>,
) -> Result<(), CanonicalEncodingError> {
    bytes.push(instrument.market().discriminant());
    append_bytes(instrument.symbol().as_bytes(), bytes)
}

fn append_state_field<T: CanonicalValue>(
    field: &StateField<T>,
    bytes: &mut Vec<u8>,
) -> Result<(), CanonicalEncodingError> {
    match field {
        StateField::Unavailable(UnavailableReason::Initial) => bytes.push(0),
        StateField::Unavailable(UnavailableReason::Cleared { cleared_at }) => {
            bytes.push(1);
            append_event_ref(cleared_at, bytes)?;
        }
        StateField::Known { value, observed_at } => {
            bytes.push(2);
            append_event_ref(observed_at, bytes)?;
            value.append_canonical(bytes)?;
        }
        StateField::Unknown { raw, observed_at } => {
            bytes.push(3);
            append_event_ref(observed_at, bytes)?;
            raw.append_canonical(bytes)?;
        }
    }
    Ok(())
}

fn append_event_ref(
    event: &AppliedEventRef,
    bytes: &mut Vec<u8>,
) -> Result<(), CanonicalEncodingError> {
    bytes.extend_from_slice(&event.match_time().as_unix_microseconds().to_be_bytes());
    append_bytes(event.source_format().as_bytes(), bytes)?;
    bytes.push(event.source_phase());
    bytes.push(event.event_kind().discriminant());
    append_optional_u64(event.source_sequence(), bytes);
    bytes.extend_from_slice(event.event_fingerprint().as_bytes());
    Ok(())
}
