use std::{error::Error, fmt, str::FromStr};

use crate::{Decimal, DecimalError};

/// A strictly positive exact market price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Price(Decimal);

impl Price {
    /// Constructs a price when the underlying decimal is strictly positive.
    pub const fn new(value: Decimal) -> Result<Self, PriceError> {
        if value.atoms() > 0 {
            Ok(Self(value))
        } else {
            Err(PriceError::NonPositive)
        }
    }

    /// Parses exact decimal text and enforces the positive-price invariant.
    pub fn parse(input: &str) -> Result<Self, PriceError> {
        Decimal::parse(input)
            .map_err(PriceError::InvalidDecimal)
            .and_then(Self::new)
    }

    /// Returns the exact decimal value.
    #[must_use]
    pub const fn as_decimal(self) -> Decimal {
        self.0
    }

    /// Returns the signed 10^-18 atoms.
    #[must_use]
    pub const fn atoms(self) -> i128 {
        self.0.atoms()
    }

    /// Returns the version-1 canonical decimal bytes.
    #[must_use]
    pub const fn to_canonical_bytes(self) -> [u8; 16] {
        self.0.to_canonical_bytes()
    }
}

impl TryFrom<Decimal> for Price {
    type Error = PriceError;

    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl FromStr for Price {
    type Err = PriceError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl TryFrom<&str> for Price {
    type Error = PriceError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        Self::parse(input)
    }
}

/// Stable error categories for price construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceError {
    InvalidDecimal(DecimalError),
    NonPositive,
}

impl fmt::Display for PriceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDecimal(error) => write!(formatter, "invalid price decimal: {error}"),
            Self::NonPositive => formatter.write_str("price must be strictly positive"),
        }
    }
}

impl Error for PriceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidDecimal(error) => Some(error),
            Self::NonPositive => None,
        }
    }
}
