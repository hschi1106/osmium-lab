use std::{error::Error, fmt};

/// The explicit unit carried by every market quantity and volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum QuantityUnit {
    /// The source provides a count, but its economic unit is not yet verified.
    SourceUnit = 0,
    /// One equity share.
    Share = 1,
    /// One exchange-defined trading unit; its security-unit size belongs to metadata.
    TradingUnit = 2,
    /// One derivatives contract; the contract multiplier belongs to metadata.
    Contract = 3,
}

impl QuantityUnit {
    /// Returns the version-1 canonical unit discriminant.
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
    }

    /// Decodes a version-1 canonical unit discriminant.
    pub const fn from_discriminant(discriminant: u8) -> Result<Self, QuantityUnitError> {
        match discriminant {
            0 => Ok(Self::SourceUnit),
            1 => Ok(Self::Share),
            2 => Ok(Self::TradingUnit),
            3 => Ok(Self::Contract),
            value => Err(QuantityUnitError::UnknownDiscriminant(value)),
        }
    }
}

impl From<QuantityUnit> for u8 {
    fn from(unit: QuantityUnit) -> Self {
        unit.discriminant()
    }
}

impl TryFrom<u8> for QuantityUnit {
    type Error = QuantityUnitError;

    fn try_from(discriminant: u8) -> Result<Self, Self::Error> {
        Self::from_discriminant(discriminant)
    }
}

/// An error decoding a versioned quantity-unit identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantityUnitError {
    UnknownDiscriminant(u8),
}

impl fmt::Display for QuantityUnitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDiscriminant(value) => {
                write!(formatter, "unknown quantity-unit discriminant: {value}")
            }
        }
    }
}

impl Error for QuantityUnitError {}

/// A strictly positive market quantity with an explicit unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Quantity {
    value: u64,
    unit: QuantityUnit,
}

impl Quantity {
    /// Constructs a quantity while enforcing the non-zero invariant.
    pub const fn new(value: u64, unit: QuantityUnit) -> Result<Self, QuantityError> {
        if value == 0 {
            Err(QuantityError::Zero)
        } else {
            Ok(Self { value, unit })
        }
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
    }

    #[must_use]
    pub const fn unit(self) -> QuantityUnit {
        self.unit
    }

    /// Adds quantities only when their units are identical.
    pub fn checked_add(self, rhs: Self) -> Result<Self, QuantityError> {
        self.require_same_unit(rhs)?;
        let value = self
            .value
            .checked_add(rhs.value)
            .ok_or(QuantityError::Overflow)?;
        Ok(Self {
            value,
            unit: self.unit,
        })
    }

    /// Subtracts quantities without allowing zero or a negative result.
    pub fn checked_sub(self, rhs: Self) -> Result<Self, QuantityError> {
        self.require_same_unit(rhs)?;
        let value = self
            .value
            .checked_sub(rhs.value)
            .ok_or(QuantityError::Underflow)?;
        Self::new(value, self.unit)
    }

    /// Returns `unit discriminant || u64 value` for canonical version 1.
    #[must_use]
    pub fn to_canonical_bytes(self) -> [u8; 9] {
        let mut bytes = [0_u8; 9];
        bytes[0] = self.unit.discriminant();
        bytes[1..].copy_from_slice(&self.value.to_be_bytes());
        bytes
    }

    fn require_same_unit(self, rhs: Self) -> Result<(), QuantityError> {
        if self.unit == rhs.unit {
            Ok(())
        } else {
            Err(QuantityError::UnitMismatch {
                left: self.unit,
                right: rhs.unit,
            })
        }
    }
}

/// Stable error categories for quantity construction and arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantityError {
    Zero,
    UnitMismatch {
        left: QuantityUnit,
        right: QuantityUnit,
    },
    Overflow,
    Underflow,
}

impl fmt::Display for QuantityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => formatter.write_str("quantity must be greater than zero"),
            Self::UnitMismatch { left, right } => {
                write!(formatter, "quantity unit mismatch: {left:?} != {right:?}")
            }
            Self::Overflow => formatter.write_str("quantity addition overflowed"),
            Self::Underflow => formatter.write_str("quantity subtraction underflowed"),
        }
    }
}

impl Error for QuantityError {}
