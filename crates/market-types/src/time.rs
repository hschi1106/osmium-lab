use std::{error::Error, fmt, str::FromStr};

const MICROS_PER_SECOND: i64 = 1_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

/// The sole replay-time value, represented as Unix microseconds in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct MatchTime(i64);

impl MatchTime {
    /// Constructs a replay time from Unix microseconds in UTC.
    #[must_use]
    pub const fn from_unix_microseconds(unix_microseconds: i64) -> Self {
        Self(unix_microseconds)
    }

    /// Returns the UTC Unix-microsecond representation.
    #[must_use]
    pub const fn as_unix_microseconds(self) -> i64 {
        self.0
    }

    /// Parses `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)` without rounding.
    pub fn parse(input: &str) -> Result<Self, MatchTimeError> {
        parse_match_time(input)
    }
}

impl FromStr for MatchTime {
    type Err = MatchTimeError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl TryFrom<&str> for MatchTime {
    type Error = MatchTimeError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        Self::parse(input)
    }
}

/// The stable error category returned when a source `match_time` is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchTimeError {
    InvalidFormat,
    InvalidDate,
    InvalidTime,
    InvalidOffset,
    PrecisionLoss,
    OutOfRange,
}

impl fmt::Display for MatchTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidFormat => "match_time is not an offset-aware ISO-8601 timestamp",
            Self::InvalidDate => "match_time contains an invalid Gregorian date",
            Self::InvalidTime => "match_time contains an invalid time of day",
            Self::InvalidOffset => "match_time contains an invalid UTC offset",
            Self::PrecisionLoss => "match_time cannot be represented without losing precision",
            Self::OutOfRange => "match_time is outside the Unix-microsecond range",
        };

        formatter.write_str(message)
    }
}

impl Error for MatchTimeError {}

fn parse_match_time(input: &str) -> Result<MatchTime, MatchTimeError> {
    let bytes = input.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return Err(MatchTimeError::InvalidFormat);
    }

    let year = parse_digits(bytes, 0, 4).ok_or(MatchTimeError::InvalidFormat)?;
    let month = parse_digits(bytes, 5, 2).ok_or(MatchTimeError::InvalidFormat)?;
    let day = parse_digits(bytes, 8, 2).ok_or(MatchTimeError::InvalidFormat)?;
    validate_date(year, month, day)?;

    let hour = parse_digits(bytes, 11, 2).ok_or(MatchTimeError::InvalidFormat)?;
    let minute = parse_digits(bytes, 14, 2).ok_or(MatchTimeError::InvalidFormat)?;
    let second = parse_digits(bytes, 17, 2).ok_or(MatchTimeError::InvalidFormat)?;
    if hour > 23 || minute > 59 || second > 59 {
        return Err(MatchTimeError::InvalidTime);
    }

    let (fraction_end, offset_seconds) = parse_offset(bytes)?;
    let microseconds = parse_fraction(bytes, fraction_end)?;

    let epoch_days = days_from_civil(year, month, day);
    let local_seconds = epoch_days
        .checked_mul(SECONDS_PER_DAY)
        .and_then(|value| value.checked_add(i64::from(hour) * 3_600))
        .and_then(|value| value.checked_add(i64::from(minute) * 60))
        .and_then(|value| value.checked_add(i64::from(second)))
        .ok_or(MatchTimeError::OutOfRange)?;
    let utc_seconds = local_seconds
        .checked_sub(offset_seconds)
        .ok_or(MatchTimeError::OutOfRange)?;
    let unix_microseconds = utc_seconds
        .checked_mul(MICROS_PER_SECOND)
        .and_then(|value| value.checked_add(microseconds))
        .ok_or(MatchTimeError::OutOfRange)?;

    Ok(MatchTime::from_unix_microseconds(unix_microseconds))
}

fn parse_offset(bytes: &[u8]) -> Result<(usize, i64), MatchTimeError> {
    let offset_start = bytes[19..]
        .iter()
        .position(|byte| matches!(byte, b'Z' | b'+' | b'-'))
        .map(|index| index + 19)
        .ok_or(MatchTimeError::InvalidFormat)?;

    match bytes[offset_start] {
        b'Z' if offset_start + 1 == bytes.len() => Ok((offset_start, 0)),
        b'Z' => Err(MatchTimeError::InvalidFormat),
        sign @ (b'+' | b'-') => {
            if offset_start + 6 != bytes.len() || bytes.get(offset_start + 3) != Some(&b':') {
                return Err(MatchTimeError::InvalidOffset);
            }

            let hours =
                parse_digits(bytes, offset_start + 1, 2).ok_or(MatchTimeError::InvalidOffset)?;
            let minutes =
                parse_digits(bytes, offset_start + 4, 2).ok_or(MatchTimeError::InvalidOffset)?;
            if hours > 23 || minutes > 59 {
                return Err(MatchTimeError::InvalidOffset);
            }

            let magnitude = i64::from(hours) * 3_600 + i64::from(minutes) * 60;
            let offset_seconds = if sign == b'-' { -magnitude } else { magnitude };
            Ok((offset_start, offset_seconds))
        }
        _ => unreachable!("offset start is selected from known offset markers"),
    }
}

fn parse_fraction(bytes: &[u8], fraction_end: usize) -> Result<i64, MatchTimeError> {
    if fraction_end == 19 {
        return Ok(0);
    }
    if bytes.get(19) != Some(&b'.') || fraction_end == 20 {
        return Err(MatchTimeError::InvalidFormat);
    }

    let digits = &bytes[20..fraction_end];
    if !digits.iter().all(u8::is_ascii_digit) {
        return Err(MatchTimeError::InvalidFormat);
    }
    if digits
        .get(6..)
        .is_some_and(|truncated| truncated.iter().any(|digit| *digit != b'0'))
    {
        return Err(MatchTimeError::PrecisionLoss);
    }

    let mut microseconds = 0_i64;
    for digit in digits.iter().take(6) {
        microseconds = microseconds * 10 + i64::from(digit - b'0');
    }
    for _ in digits.len()..6 {
        microseconds *= 10;
    }

    Ok(microseconds)
}

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

fn validate_date(year: u32, month: u32, day: u32) -> Result<(), MatchTimeError> {
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return Err(MatchTimeError::InvalidDate),
    };

    if day == 0 || day > days_in_month {
        return Err(MatchTimeError::InvalidDate);
    }

    Ok(())
}

const fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

// Howard Hinnant's proleptic-Gregorian civil-date conversion, shifted to Unix epoch days.
fn days_from_civil(year: u32, month: u32, day: u32) -> i64 {
    let mut year = i64::from(year);
    let month = i64::from(month);
    let day = i64::from(day);
    year -= i64::from(month <= 2);

    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}
