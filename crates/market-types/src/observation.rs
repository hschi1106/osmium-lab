use crate::Decimal;

/// An explicit source observation and its reducer semantics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Observation<T> {
    /// This event does not update the corresponding state field.
    NoObservation,
    /// Replace the corresponding state field with the supplied value.
    Set(T),
    /// Explicitly make the corresponding state field unavailable.
    Clear,
    /// Preserve a bounded scalar whose domain meaning is not yet known.
    Unknown(UnknownValue),
}

impl<T> Observation<T> {
    #[must_use]
    pub const fn discriminant(&self) -> u8 {
        match self {
            Self::NoObservation => 0,
            Self::Set(_) => 1,
            Self::Clear => 2,
            Self::Unknown(_) => 3,
        }
    }

    #[must_use]
    pub const fn as_set(&self) -> Option<&T> {
        match self {
            Self::Set(value) => Some(value),
            Self::NoObservation | Self::Clear | Self::Unknown(_) => None,
        }
    }
}

/// A losslessly encodable bounded scalar with unconfirmed domain meaning.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnknownValue {
    Unsigned(u64),
    Signed(i64),
    Decimal(Decimal),
    Text(Box<str>),
    Bytes(Box<[u8]>),
}

impl UnknownValue {
    #[must_use]
    pub const fn discriminant(&self) -> u8 {
        match self {
            Self::Unsigned(_) => 1,
            Self::Signed(_) => 2,
            Self::Decimal(_) => 3,
            Self::Text(_) => 4,
            Self::Bytes(_) => 5,
        }
    }
}
