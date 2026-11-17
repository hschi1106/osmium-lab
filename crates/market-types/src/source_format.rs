use std::{cmp::Ordering, error::Error, fmt, str::FromStr};

use crate::inline_str::{InlineStr, InlineStrError};

/// A non-empty, byte-exact identifier from the versioned normalizer registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceFormatId(InlineStr<{ SourceFormatId::MAX_LEN }>);

impl SourceFormatId {
    /// Maximum UTF-8 byte length; values are stored inline so the type is `Copy`.
    pub const MAX_LEN: usize = 23;

    /// Constructs an identifier without trimming or normalizing its bytes.
    pub fn new(value: impl AsRef<str>) -> Result<Self, SourceFormatIdError> {
        InlineStr::new(value.as_ref())
            .map(Self)
            .map_err(|error| match error {
                InlineStrError::Empty => SourceFormatIdError::Empty,
                InlineStrError::TooLong => SourceFormatIdError::TooLong,
            })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl PartialOrd for SourceFormatId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SourceFormatId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl fmt::Display for SourceFormatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for SourceFormatId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for SourceFormatId {
    type Err = SourceFormatIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<&str> for SourceFormatId {
    type Error = SourceFormatIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Stable error categories for source-format identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormatIdError {
    Empty,
    TooLong,
}

impl fmt::Display for SourceFormatIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("source format must not be empty"),
            Self::TooLong => write!(
                formatter,
                "source format must be at most {} bytes",
                SourceFormatId::MAX_LEN
            ),
        }
    }
}

impl Error for SourceFormatIdError {}
