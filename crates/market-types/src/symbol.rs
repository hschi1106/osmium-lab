use std::{cmp::Ordering, error::Error, fmt, str::FromStr};

/// A non-empty, byte-exact canonical UTF-8 instrument symbol.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol(Box<str>);

impl Symbol {
    /// Constructs a symbol without trimming or otherwise normalizing its bytes.
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, SymbolError> {
        let value = value.into();
        if value.is_empty() {
            Err(SymbolError::Empty)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the canonical symbol text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the canonical UTF-8 bytes used for identity and ordering.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Symbol {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for Symbol {
    type Err = SymbolError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<&str> for Symbol {
    type Error = SymbolError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for Symbol {
    type Error = SymbolError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A stable error category for generic symbol construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolError {
    Empty,
}

impl fmt::Display for SymbolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("symbol must not be empty"),
        }
    }
}

impl Error for SymbolError {}
