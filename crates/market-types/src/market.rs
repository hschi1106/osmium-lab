use std::{cmp::Ordering, error::Error, fmt};

/// A supported exchange market with a stable version-1 identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MarketId {
    Twse = 1,
    Tpex = 2,
    Taifex = 3,
}

impl MarketId {
    /// Returns the fixed version-1 market discriminant.
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
    }

    /// Returns the fixed deterministic-ordering rank.
    #[must_use]
    pub const fn ordering_rank(self) -> u8 {
        self.discriminant()
    }

    /// Decodes a version-1 market discriminant.
    pub const fn from_discriminant(discriminant: u8) -> Result<Self, MarketIdError> {
        match discriminant {
            1 => Ok(Self::Twse),
            2 => Ok(Self::Tpex),
            3 => Ok(Self::Taifex),
            value => Err(MarketIdError::UnknownDiscriminant(value)),
        }
    }
}

impl PartialOrd for MarketId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MarketId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ordering_rank().cmp(&other.ordering_rank())
    }
}

impl From<MarketId> for u8 {
    fn from(market: MarketId) -> Self {
        market.discriminant()
    }
}

impl TryFrom<u8> for MarketId {
    type Error = MarketIdError;

    fn try_from(discriminant: u8) -> Result<Self, Self::Error> {
        Self::from_discriminant(discriminant)
    }
}

/// An error decoding a versioned market identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketIdError {
    UnknownDiscriminant(u8),
}

impl fmt::Display for MarketIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDiscriminant(value) => {
                write!(formatter, "unknown market discriminant: {value}")
            }
        }
    }
}

impl Error for MarketIdError {}
