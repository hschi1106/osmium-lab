use std::{error::Error, fmt, str::FromStr};

/// An exchange-native instrument code such as `2330`, `00631L` or `030001`.
///
/// One to eight ASCII letters or digits, stored inline and padded with NUL so the type
/// is `Copy` and orders exactly like the code's bytes (the order of marketdata files).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol([u8; Symbol::MAX_LEN]);

impl Symbol {
    pub const MAX_LEN: usize = 8;

    /// Accepts the code exactly as written; nothing is trimmed or case-folded.
    pub fn new(code: &str) -> Result<Self, SymbolError> {
        let source = code.as_bytes();
        if source.is_empty() {
            return Err(SymbolError::Empty);
        }
        if source.len() > Self::MAX_LEN {
            return Err(SymbolError::TooLong);
        }
        if !source.iter().all(u8::is_ascii_alphanumeric) {
            return Err(SymbolError::InvalidCharacter);
        }
        let mut bytes = [0; Self::MAX_LEN];
        bytes[..source.len()].copy_from_slice(source);
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        let len = self
            .0
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(Self::MAX_LEN);
        std::str::from_utf8(&self.0[..len]).expect("symbol bytes are ASCII")
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Symbol({})", self.as_str())
    }
}

impl FromStr for Symbol {
    type Err = SymbolError;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        Self::new(code)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolError {
    Empty,
    TooLong,
    InvalidCharacter,
}

impl fmt::Display for SymbolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("symbol must not be empty"),
            Self::TooLong => write!(
                formatter,
                "symbol must be at most {} bytes",
                Symbol::MAX_LEN
            ),
            Self::InvalidCharacter => {
                formatter.write_str("symbol must contain only ASCII letters and digits")
            }
        }
    }
}

impl Error for SymbolError {}

#[cfg(test)]
mod tests {
    use super::{Symbol, SymbolError};

    #[test]
    fn keeps_the_code_exactly() {
        for code in [
            "2330", "00631L", "2887Z1", "030001", "45491", "A", "12345678",
        ] {
            let symbol = Symbol::new(code).unwrap();
            assert_eq!(symbol.as_str(), code);
            assert_eq!(symbol.to_string(), code);
            assert_eq!(code.parse::<Symbol>(), Ok(symbol));
        }
    }

    #[test]
    fn rejects_codes_it_cannot_represent() {
        assert_eq!(Symbol::new(""), Err(SymbolError::Empty));
        assert_eq!(Symbol::new("123456789"), Err(SymbolError::TooLong));
        for code in ["2330 ", "23-30", "台積", "23\u{0}0"] {
            assert_eq!(
                Symbol::new(code),
                Err(SymbolError::InvalidCharacter),
                "{code:?}"
            );
        }
    }

    #[test]
    fn orders_like_the_code_bytes() {
        let mut codes = ["2330", "00631L", "2330A", "233", "030001", "9958", "2887Z1"];
        let mut symbols = codes.map(|code| Symbol::new(code).unwrap());
        codes.sort_unstable();
        symbols.sort_unstable();
        assert_eq!(symbols.map(|symbol| symbol.to_string()), codes);
    }
}
