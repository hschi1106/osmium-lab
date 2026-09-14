use std::{error::Error, fmt, str::FromStr};

use crate::{Decimal, DecimalError};

/// An exact signed market price. Instrument rules decide which values are valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Price(Decimal);

impl Price {
    /// Constructs a signed market price, including zero.
    pub const fn new(value: Decimal) -> Self {
        Self(value)
    }

    /// Constructs a price for profiles that require strictly positive values.
    pub const fn positive(value: Decimal) -> Result<Self, PriceError> {
        if value.atoms() > 0 {
            Ok(Self(value))
        } else {
            Err(PriceError::NonPositive)
        }
    }

    /// Parses exact signed decimal text, including zero.
    pub fn parse(input: &str) -> Result<Self, PriceError> {
        Decimal::parse(input)
            .map(Self::new)
            .map_err(PriceError::InvalidDecimal)
    }

    /// Parses exact decimal text and enforces a positive-only profile.
    pub fn parse_positive(input: &str) -> Result<Self, PriceError> {
        Decimal::parse(input)
            .map_err(PriceError::InvalidDecimal)
            .and_then(Self::positive)
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

impl From<Decimal> for Price {
    fn from(value: Decimal) -> Self {
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
