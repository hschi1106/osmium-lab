use std::{error::Error, fmt};

use crate::{
    CompleteBookSnapshot, MarketAnnotations, Observation, Price, Quantity, TradePrint,
    UnknownValue, Volume,
};

/// Error produced when a variable-length canonical value cannot use its fixed u32 frame.
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

/// Canonical payload encoding shared by events and derived state.
pub trait CanonicalValue {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError>;
}

impl CanonicalValue for CompleteBookSnapshot {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        for side in [self.bids(), self.asks()] {
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
        Ok(())
    }
}

impl CanonicalValue for TradePrint {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        bytes.extend_from_slice(&self.price().to_canonical_bytes());
        bytes.extend_from_slice(&self.quantity().to_canonical_bytes());
        bytes.push(self.print_kind().discriminant());
        Ok(())
    }
}

impl CanonicalValue for Volume {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        bytes.extend_from_slice(&self.to_canonical_bytes());
        Ok(())
    }
}

impl CanonicalValue for Price {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        bytes.extend_from_slice(&self.to_canonical_bytes());
        Ok(())
    }
}

impl CanonicalValue for Quantity {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        bytes.extend_from_slice(&self.to_canonical_bytes());
        Ok(())
    }
}

impl CanonicalValue for MarketAnnotations {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        bytes.push(self.discriminant());
        match self {
            Self::TwseQuote(annotation) => {
                bytes.push(annotation.status_flags_raw());
                bytes.push(annotation.limit_flags_raw());
            }
            Self::TpexQuote(annotation) => {
                bytes.push(annotation.status_flags_raw());
                bytes.push(annotation.limit_flags_raw());
            }
            Self::None => {}
        }
        Ok(())
    }
}

impl CanonicalValue for UnknownValue {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        bytes.push(self.discriminant());
        match self {
            Self::Unsigned(value) => bytes.extend_from_slice(&value.to_be_bytes()),
            Self::Signed(value) => bytes.extend_from_slice(&value.to_be_bytes()),
            Self::Decimal(value) => bytes.extend_from_slice(&value.to_canonical_bytes()),
            Self::Text(value) => append_bytes(value.as_bytes(), bytes)?,
            Self::Bytes(value) => append_bytes(value, bytes)?,
        }
        Ok(())
    }
}

impl<T: CanonicalValue> CanonicalValue for Observation<T> {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        bytes.push(self.discriminant());
        match self {
            Self::Set(value) => value.append_canonical(bytes)?,
            Self::Unknown(value) => value.append_canonical(bytes)?,
            Self::NoObservation | Self::Clear => {}
        }
        Ok(())
    }
}

pub fn append_bytes(value: &[u8], bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
    append_length(value.len(), bytes)?;
    bytes.extend_from_slice(value);
    Ok(())
}

pub fn append_length(length: usize, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
    let length = u32::try_from(length).map_err(|_| CanonicalEncodingError::LengthOverflow)?;
    bytes.extend_from_slice(&length.to_be_bytes());
    Ok(())
}

pub fn append_optional_u64(value: Option<u64>, bytes: &mut Vec<u8>) {
    match value {
        None => bytes.push(0),
        Some(value) => {
            bytes.push(1);
            bytes.extend_from_slice(&value.to_be_bytes());
        }
    }
}
