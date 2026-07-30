use std::{error::Error, fmt};

use crate::{Quantity, QuantityError, QuantityUnit};

/// A cumulative market volume, where zero is a valid observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Volume {
    value: u64,
    unit: QuantityUnit,
}

impl Volume {
    #[must_use]
    pub const fn new(value: u64, unit: QuantityUnit) -> Self {
        Self { value, unit }
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
    }

    #[must_use]
    pub const fn unit(self) -> QuantityUnit {
        self.unit
    }

    /// Adds volumes only when their units are identical.
    pub fn checked_add(self, rhs: Self) -> Result<Self, VolumeError> {
        self.require_same_unit(rhs)?;
        let value = self
            .value
            .checked_add(rhs.value)
            .ok_or(VolumeError::Overflow)?;
        Ok(Self {
            value,
            unit: self.unit,
        })
    }

    /// Subtracts volumes only when their units are identical.
    pub fn checked_sub(self, rhs: Self) -> Result<Self, VolumeError> {
        self.require_same_unit(rhs)?;
        let value = self
            .value
            .checked_sub(rhs.value)
            .ok_or(VolumeError::Underflow)?;
        Ok(Self {
            value,
            unit: self.unit,
        })
    }

    /// Returns `unit discriminant || u64 value` for canonical version 1.
    #[must_use]
    pub fn to_canonical_bytes(self) -> [u8; 9] {
        let mut bytes = [0_u8; 9];
        bytes[0] = self.unit.discriminant();
        bytes[1..].copy_from_slice(&self.value.to_be_bytes());
        bytes
    }

    fn require_same_unit(self, rhs: Self) -> Result<(), VolumeError> {
        if self.unit == rhs.unit {
            Ok(())
        } else {
            Err(VolumeError::UnitMismatch {
                left: self.unit,
                right: rhs.unit,
            })
        }
    }
}

impl From<Quantity> for Volume {
    fn from(quantity: Quantity) -> Self {
        Self::new(quantity.value(), quantity.unit())
    }
}

impl TryFrom<Volume> for Quantity {
    type Error = QuantityError;

    fn try_from(volume: Volume) -> Result<Self, Self::Error> {
        Self::new(volume.value, volume.unit)
    }
}

/// Stable error categories for volume arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeError {
    UnitMismatch {
        left: QuantityUnit,
        right: QuantityUnit,
    },
    Overflow,
    Underflow,
}

impl fmt::Display for VolumeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnitMismatch { left, right } => {
                write!(formatter, "volume unit mismatch: {left:?} != {right:?}")
            }
            Self::Overflow => formatter.write_str("volume addition overflowed"),
            Self::Underflow => formatter.write_str("volume subtraction underflowed"),
        }
    }
}

impl Error for VolumeError {}
