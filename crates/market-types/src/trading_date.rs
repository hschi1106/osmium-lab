use std::{error::Error, fmt, str::FromStr};

/// An exchange business date represented as signed days from 1970-01-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct TradingDate(i32);

impl TradingDate {
    /// Constructs a trading date from its canonical signed epoch-day value.
    #[must_use]
    pub const fn from_epoch_days(epoch_days: i32) -> Self {
        Self(epoch_days)
    }

    /// Returns signed days from 1970-01-01.
    #[must_use]
    pub const fn as_epoch_days(self) -> i32 {
        self.0
    }

    /// Returns the version-1 canonical two's-complement big-endian bytes.
    #[must_use]
    pub const fn to_canonical_bytes(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    /// Parses an exact Gregorian `YYYY-MM-DD` date.
    pub fn parse(input: &str) -> Result<Self, TradingDateError> {
        let bytes = input.as_bytes();
        if bytes.len() != 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
            return Err(TradingDateError::InvalidFormat);
        }

        let year = parse_digits(bytes, 0, 4).ok_or(TradingDateError::InvalidFormat)?;
        let month = parse_digits(bytes, 5, 2).ok_or(TradingDateError::InvalidFormat)?;
        let day = parse_digits(bytes, 8, 2).ok_or(TradingDateError::InvalidFormat)?;
        validate_date(year, month, day)?;

        Ok(Self::from_epoch_days(days_from_civil(year, month, day)))
    }
}

impl From<TradingDate> for i32 {
    fn from(date: TradingDate) -> Self {
        date.as_epoch_days()
    }
}

impl FromStr for TradingDate {
    type Err = TradingDateError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl TryFrom<&str> for TradingDate {
    type Error = TradingDateError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        Self::parse(input)
    }
}

impl fmt::Display for TradingDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = civil_from_days(self.0);
        if (0..=9999).contains(&year) {
            write!(formatter, "{year:04}-{month:02}-{day:02}")
        } else if year < 0 {
            write!(formatter, "-{:04}-{month:02}-{day:02}", year.unsigned_abs())
        } else {
            write!(formatter, "+{year:04}-{month:02}-{day:02}")
        }
    }
}

/// Stable error categories for textual trading-date construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingDateError {
    InvalidFormat,
    InvalidDate,
}

impl fmt::Display for TradingDateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidFormat => "trading date must use the exact YYYY-MM-DD format",
            Self::InvalidDate => "trading date is not a valid Gregorian date",
        };

        formatter.write_str(message)
    }
}

impl Error for TradingDateError {}

fn parse_digits(bytes: &[u8], start: usize, length: usize) -> Option<u32> {
    let digits = bytes.get(start..start.checked_add(length)?)?;
    if !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }

    Some(
        digits
            .iter()
            .fold(0_u32, |value, digit| value * 10 + u32::from(digit - b'0')),
    )
}

fn validate_date(year: u32, month: u32, day: u32) -> Result<(), TradingDateError> {
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return Err(TradingDateError::InvalidDate),
    };

    if day == 0 || day > days_in_month {
        return Err(TradingDateError::InvalidDate);
    }

    Ok(())
}

const fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_from_civil(year: u32, month: u32, day: u32) -> i32 {
    let mut year = i64::from(year);
    let month = i64::from(month);
    let day = i64::from(day);
    year -= i64::from(month <= 2);

    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let epoch_days = era * 146_097 + day_of_era - 719_468;

    i32::try_from(epoch_days).expect("four-digit Gregorian dates fit in i32 epoch days")
}

fn civil_from_days(epoch_days: i32) -> (i64, i64, i64) {
    let shifted_days = i64::from(epoch_days) + 719_468;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = shifted_month + if shifted_month < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);

    (year, month, day)
}
