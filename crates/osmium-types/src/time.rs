use std::{error::Error, fmt, str::FromStr};

const MICROS_PER_SECOND: i64 = 1_000_000;
const MICROS_PER_DAY: i64 = 86_400 * MICROS_PER_SECOND;
/// Taiwan has used UTC+8 without daylight saving time since 1980.
const TAIPEI_OFFSET_MICROS: i64 = 8 * 3_600 * MICROS_PER_SECOND;

/// An instant in microseconds since 1970-01-01T00:00:00Z, the resolution of `exch_ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    #[must_use]
    pub const fn from_micros(micros: i64) -> Self {
        Self(micros)
    }

    /// Microseconds since the Unix epoch.
    #[must_use]
    pub const fn micros(self) -> i64 {
        self.0
    }

    /// The calendar date in Taiwan at this instant.
    ///
    /// # Panics
    ///
    /// Panics when the instant lies outside the years 0000–9999.
    #[must_use]
    pub fn taipei_date(self) -> Date {
        let days = (self.0 + TAIPEI_OFFSET_MICROS).div_euclid(MICROS_PER_DAY);
        i32::try_from(days)
            .ok()
            .and_then(|days| Date::from_epoch_days(days).ok())
            .expect("timestamp is within the supported date range")
    }
}

/// Formats Taiwan local time, e.g. `2026-10-08T09:00:00.123456+08:00`.
impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let local = self.0 + TAIPEI_OFFSET_MICROS;
        let micros_of_day = local.rem_euclid(MICROS_PER_DAY);
        let seconds = micros_of_day / MICROS_PER_SECOND;
        write!(
            formatter,
            "{}T{:02}:{:02}:{:02}.{:06}+08:00",
            self.taipei_date(),
            seconds / 3_600,
            seconds / 60 % 60,
            seconds % 60,
            micros_of_day % MICROS_PER_SECOND
        )
    }
}

/// A Gregorian calendar date, stored as days since 1970-01-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(i32);

impl Date {
    const MIN_EPOCH_DAYS: i32 = -719_528; // 0000-01-01
    const MAX_EPOCH_DAYS: i32 = 2_932_896; // 9999-12-31

    /// Constructs a date from days since 1970-01-01, the value of an Arrow `date32`.
    pub const fn from_epoch_days(days: i32) -> Result<Self, DateError> {
        if days < Self::MIN_EPOCH_DAYS || days > Self::MAX_EPOCH_DAYS {
            return Err(DateError::OutOfRange);
        }
        Ok(Self(days))
    }

    pub fn from_ymd(year: u32, month: u32, day: u32) -> Result<Self, DateError> {
        if year > 9_999 {
            return Err(DateError::OutOfRange);
        }
        let days_in_month = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if is_leap_year(year) => 29,
            2 => 28,
            _ => return Err(DateError::InvalidDate),
        };
        if day == 0 || day > days_in_month {
            return Err(DateError::InvalidDate);
        }
        Ok(Self(days_from_civil(year, month, day)))
    }

    /// Days since 1970-01-01.
    #[must_use]
    pub const fn epoch_days(self) -> i32 {
        self.0
    }

    /// `(year, month, day)`.
    #[must_use]
    pub fn ymd(self) -> (u32, u32, u32) {
        civil_from_days(self.0)
    }

    /// The instant of a Taiwan wall-clock time on this date.
    ///
    /// # Panics
    ///
    /// Panics when the time is not a valid `hour:minute:second`.
    #[must_use]
    pub fn at_taipei(self, hour: u32, minute: u32, second: u32) -> Timestamp {
        assert!(
            hour < 24 && minute < 60 && second < 60,
            "invalid time of day"
        );
        let seconds = i64::from(hour * 3_600 + minute * 60 + second);
        Timestamp(
            i64::from(self.0) * MICROS_PER_DAY + seconds * MICROS_PER_SECOND - TAIPEI_OFFSET_MICROS,
        )
    }
}

/// Formats `YYYY-MM-DD`.
impl fmt::Display for Date {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = self.ymd();
        write!(formatter, "{year:04}-{month:02}-{day:02}")
    }
}

/// Parses exactly `YYYY-MM-DD`.
impl FromStr for Date {
    type Err = DateError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(DateError::InvalidFormat);
        }
        let number = |range: std::ops::Range<usize>| {
            let digits = &bytes[range];
            digits
                .iter()
                .all(u8::is_ascii_digit)
                .then(|| {
                    digits
                        .iter()
                        .fold(0_u32, |value, digit| value * 10 + u32::from(digit - b'0'))
                })
                .ok_or(DateError::InvalidFormat)
        };
        Self::from_ymd(number(0..4)?, number(5..7)?, number(8..10)?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateError {
    InvalidFormat,
    InvalidDate,
    OutOfRange,
}

impl fmt::Display for DateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidFormat => "date must use the exact YYYY-MM-DD format",
            Self::InvalidDate => "date is not a valid Gregorian date",
            Self::OutOfRange => "date is outside 0000-01-01..=9999-12-31",
        })
    }
}

impl Error for DateError {}

const fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

// Howard Hinnant's civil-date algorithms over the proleptic Gregorian calendar.
fn days_from_civil(year: u32, month: u32, day: u32) -> i32 {
    let year = i64::from(year) - i64::from(month <= 2);
    let month = i64::from(month);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    i32::try_from(era * 146_097 + day_of_era - 719_468).expect("four-digit years fit in i32 days")
}

fn civil_from_days(epoch_days: i32) -> (u32, u32, u32) {
    let shifted_days = i64::from(epoch_days) + 719_468;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = shifted_month + if shifted_month < 10 { 3 } else { -9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let field = |value: i64| u32::try_from(value).expect("date fields are non-negative");
    (field(year), field(month), field(day))
}

#[cfg(test)]
mod tests {
    use super::{Date, DateError, Timestamp};

    #[test]
    fn dates_roundtrip_through_text_and_epoch_days() {
        for (text, days) in [
            ("1970-01-01", 0),
            ("2022-01-03", 18_995),
            ("2024-02-29", 19_782),
            ("2026-10-08", 20_734),
            ("0000-01-01", -719_528),
            ("9999-12-31", 2_932_896),
        ] {
            let date: Date = text.parse().unwrap();
            assert_eq!(date.epoch_days(), days, "{text}");
            assert_eq!(date.to_string(), text);
            assert_eq!(Date::from_epoch_days(days), Ok(date));
        }
        assert_eq!("2026-10-08".parse::<Date>().unwrap().ymd(), (2026, 10, 8));
    }

    #[test]
    fn rejects_invalid_dates() {
        for text in [
            "2026-1-08",
            "20261008",
            "2026/10/08",
            "2026-10-8 ",
            "abcd-10-08",
        ] {
            assert_eq!(
                text.parse::<Date>(),
                Err(DateError::InvalidFormat),
                "{text}"
            );
        }
        for text in [
            "2026-02-29",
            "2026-13-01",
            "2026-00-10",
            "2026-04-31",
            "2026-10-00",
        ] {
            assert_eq!(text.parse::<Date>(), Err(DateError::InvalidDate), "{text}");
        }
        assert_eq!(Date::from_epoch_days(2_932_897), Err(DateError::OutOfRange));
        assert_eq!(Date::from_ymd(10_000, 1, 1), Err(DateError::OutOfRange));
    }

    #[test]
    fn taipei_wall_clock_converts_to_instants() {
        let date: Date = "2026-10-08".parse().unwrap();
        // 09:00 in Taipei is 01:00 UTC.
        let open = date.at_taipei(9, 0, 0);
        assert_eq!(open.micros(), (20_734 * 86_400 + 3_600) * 1_000_000);
        assert_eq!(open.taipei_date(), date);
        assert_eq!(open.to_string(), "2026-10-08T09:00:00.000000+08:00");
        assert_eq!(
            Timestamp::from_micros(open.micros() + 16_212).to_string(),
            "2026-10-08T09:00:00.016212+08:00"
        );
    }

    #[test]
    fn the_taipei_date_changes_at_taipei_midnight() {
        let date: Date = "2026-10-08".parse().unwrap();
        let midnight = date.at_taipei(0, 0, 0);
        assert_eq!(midnight.taipei_date(), date);
        let before = Timestamp::from_micros(midnight.micros() - 1);
        assert_eq!(before.taipei_date().to_string(), "2026-10-07");
        assert_eq!(before.to_string(), "2026-10-07T23:59:59.999999+08:00");
        assert_eq!(date.at_taipei(23, 59, 59).taipei_date(), date);
    }
}
