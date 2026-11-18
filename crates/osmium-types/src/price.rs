use std::{error::Error, fmt, str::FromStr};

/// An exact price in units of 1e-4 元, the finest resolution marketdata prices use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Price(i64);

impl Price {
    /// Price units per 元.
    pub const UNITS_PER_YUAN: i64 = 10_000;

    #[must_use]
    pub const fn from_units(units: i64) -> Self {
        Self(units)
    }

    /// The price in units of 1e-4 元.
    #[must_use]
    pub const fn units(self) -> i64 {
        self.0
    }
}

/// Formats the exact decimal value without trailing zeros: `198.45`, `1085`, `0.01`.
impl fmt::Display for Price {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let units = self.0.unsigned_abs();
        let scale = Self::UNITS_PER_YUAN.unsigned_abs();
        let (whole, fraction) = (units / scale, units % scale);
        if fraction == 0 {
            return write!(formatter, "{sign}{whole}");
        }
        let digits = format!("{fraction:04}");
        write!(formatter, "{sign}{whole}.{}", digits.trim_end_matches('0'))
    }
}

/// Parses a non-negative decimal with at most four fractional digits, e.g. `198.45`.
impl FromStr for Price {
    type Err = ParsePriceError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        let all_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
        if whole.is_empty()
            || !all_digits(whole)
            || !all_digits(fraction)
            || fraction.len() > 4
            || (fraction.is_empty() && text.contains('.'))
        {
            return Err(ParsePriceError);
        }
        let whole: i64 = whole.parse().map_err(|_| ParsePriceError)?;
        let mut units = whole
            .checked_mul(Self::UNITS_PER_YUAN)
            .ok_or(ParsePriceError)?;
        if !fraction.is_empty() {
            let padded = format!("{fraction:0<4}");
            units = units
                .checked_add(padded.parse().map_err(|_| ParsePriceError)?)
                .ok_or(ParsePriceError)?;
        }
        Ok(Self(units))
    }
}

/// The text is not a non-negative decimal with at most four fractional digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsePriceError;

impl fmt::Display for ParsePriceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("price must be a non-negative decimal with at most 4 decimal places")
    }
}

impl Error for ParsePriceError {}

#[cfg(test)]
mod tests {
    use super::Price;

    #[test]
    fn text_roundtrips_exactly() {
        for (text, units) in [
            ("198.45", 1_984_500),
            ("1085", 10_850_000),
            ("0.01", 100),
            ("9999.95", 99_999_500),
            ("0.0001", 1),
            ("0", 0),
        ] {
            let price: Price = text.parse().unwrap();
            assert_eq!(price.units(), units, "{text}");
            assert_eq!(price.to_string(), text);
        }
        assert_eq!("10.50".parse::<Price>().unwrap().to_string(), "10.5");
        assert_eq!(Price::from_units(-5_000).to_string(), "-0.5");
    }

    #[test]
    fn rejects_text_that_is_not_an_exact_price() {
        for text in [
            "", ".5", "1.", "1.23456", "-1", "+1", "1e3", "1,000", " 1", "abc",
        ] {
            assert!(text.parse::<Price>().is_err(), "{text:?}");
        }
        assert!("99999999999999999999".parse::<Price>().is_err());
    }

    #[test]
    fn orders_by_value() {
        let low: Price = "9.99".parse().unwrap();
        let high: Price = "10".parse().unwrap();
        assert!(low < high);
    }
}
