use osmium_types::Date;

use crate::LedgerError;

/// Trading days and settlement days. Nothing is extrapolated: asking for a day beyond
/// the last known one is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    trading: Vec<Date>,
    settlement: Vec<Date>,
}

impl Calendar {
    /// `trading_days` are the days the market is open. `settlement_days` are the days
    /// on which settlement is done; every trading day is one whether or not it is
    /// listed there (`docs/reference/accounting.md` section 7.1).
    #[must_use]
    pub fn new(
        trading_days: impl IntoIterator<Item = Date>,
        settlement_days: impl IntoIterator<Item = Date>,
    ) -> Self {
        let mut trading: Vec<Date> = trading_days.into_iter().collect();
        trading.sort_unstable();
        trading.dedup();
        let mut settlement = trading.clone();
        settlement.extend(settlement_days);
        settlement.sort_unstable();
        settlement.dedup();
        Self {
            trading,
            settlement,
        }
    }

    #[must_use]
    pub fn is_trading_day(&self, date: Date) -> bool {
        self.trading.binary_search(&date).is_ok()
    }

    /// The settlement date of a trade on `trade_date`: the second settlement day
    /// after it.
    pub fn settlement_date(&self, trade_date: Date) -> Result<Date, LedgerError> {
        let after = self.settlement.partition_point(|&day| day <= trade_date);
        self.settlement
            .get(after + 1)
            .copied()
            .ok_or(LedgerError::OutsideCalendar {
                needed_after: trade_date,
            })
    }

    /// The settlement days after `after` up to and including `until`, in order.
    #[must_use]
    pub fn settlement_days(&self, after: Option<Date>, until: Date) -> &[Date] {
        let from = after.map_or(0, |after| {
            self.settlement.partition_point(|&day| day <= after)
        });
        let to = self.settlement.partition_point(|&day| day <= until);
        &self.settlement[from.min(to)..to]
    }

    /// The `n`-th trading day after `date` (`n >= 1`).
    pub fn trading_day_after(&self, date: Date, n: usize) -> Result<Date, LedgerError> {
        assert!(n >= 1);
        let after = self.trading.partition_point(|&day| day <= date);
        self.trading
            .get(after + n - 1)
            .copied()
            .ok_or(LedgerError::OutsideCalendar { needed_after: date })
    }
}

/// `date` plus `months` calendar months; a day past the end of the month becomes the
/// last day of that month.
#[must_use]
pub fn add_months(date: Date, months: u32) -> Date {
    let (year, month, day) = date.ymd();
    let index = month - 1 + months;
    let (year, month) = (year + index / 12, index % 12 + 1);
    (1..=day)
        .rev()
        .find_map(|day| Date::from_ymd(year, month, day).ok())
        .expect("every month has a first day")
}

/// `date` plus `days` calendar days.
#[must_use]
pub fn add_days(date: Date, days: u32) -> Date {
    Date::from_epoch_days(date.epoch_days() + i32::try_from(days).expect("a small number"))
        .expect("a date in range")
}

#[cfg(test)]
mod tests {
    use osmium_types::Date;

    use super::{Calendar, add_days, add_months};
    use crate::LedgerError;

    fn day(text: &str) -> Date {
        text.parse().unwrap()
    }

    fn days(texts: &[&str]) -> Vec<Date> {
        texts.iter().map(|text| day(text)).collect()
    }

    #[test]
    fn settlement_is_two_settlement_days_later() {
        let calendar = Calendar::new(
            days(&["2026-10-07", "2026-10-08", "2026-10-12", "2026-10-13"]),
            [],
        );
        // 2026-10-09 is a holiday and the weekend follows.
        assert_eq!(
            calendar.settlement_date(day("2026-10-07")),
            Ok(day("2026-10-12"))
        );
        assert_eq!(
            calendar.settlement_date(day("2026-10-08")),
            Ok(day("2026-10-13"))
        );
        assert_eq!(
            calendar.settlement_date(day("2026-10-12")),
            Err(LedgerError::OutsideCalendar {
                needed_after: day("2026-10-12")
            })
        );
    }

    #[test]
    fn settlement_only_days_count_for_settlement_but_not_for_trading() {
        // Before the 2024 Lunar New Year: 02-05 was the last trading day, 02-06 and
        // 02-07 settled only, trading resumed on 02-15.
        let calendar = Calendar::new(
            days(&["2024-02-02", "2024-02-05", "2024-02-15", "2024-02-16"]),
            days(&["2024-02-06", "2024-02-07"]),
        );
        assert_eq!(
            calendar.settlement_date(day("2024-02-02")),
            Ok(day("2024-02-06"))
        );
        assert_eq!(
            calendar.settlement_date(day("2024-02-05")),
            Ok(day("2024-02-07"))
        );
        assert!(!calendar.is_trading_day(day("2024-02-06")));
        assert_eq!(
            calendar.trading_day_after(day("2024-02-05"), 1),
            Ok(day("2024-02-15"))
        );
        assert_eq!(
            calendar.settlement_days(Some(day("2024-02-02")), day("2024-02-15")),
            days(&["2024-02-05", "2024-02-06", "2024-02-07", "2024-02-15"])
        );
        assert_eq!(
            calendar.settlement_days(None, day("2024-02-02")),
            days(&["2024-02-02"])
        );
    }

    #[test]
    fn settlement_days_may_go_beyond_the_trading_days() {
        let calendar = Calendar::new(
            days(&["2026-10-15", "2026-10-16"]),
            days(&["2026-10-19", "2026-10-20"]),
        );
        assert_eq!(
            calendar.settlement_date(day("2026-10-16")),
            Ok(day("2026-10-20"))
        );
        assert!(!calendar.is_trading_day(day("2026-10-19")));
        assert!(calendar.trading_day_after(day("2026-10-16"), 1).is_err());
    }

    #[test]
    fn trading_days_are_counted_from_the_next_one() {
        let calendar = Calendar::new(days(&["2026-10-07", "2026-10-08", "2026-10-12"]), []);
        assert_eq!(
            calendar.trading_day_after(day("2026-10-07"), 1),
            Ok(day("2026-10-08"))
        );
        assert_eq!(
            calendar.trading_day_after(day("2026-10-07"), 2),
            Ok(day("2026-10-12"))
        );
        assert_eq!(
            calendar.trading_day_after(day("2026-10-09"), 1),
            Ok(day("2026-10-12"))
        );
        assert!(calendar.trading_day_after(day("2026-10-07"), 3).is_err());
    }

    #[test]
    fn months_clamp_to_the_end_of_the_month() {
        assert_eq!(add_months(day("2025-08-31"), 6), day("2026-02-28"));
        assert_eq!(add_months(day("2023-08-31"), 6), day("2024-02-29"));
        assert_eq!(add_months(day("2026-01-15"), 18), day("2027-07-15"));
        assert_eq!(add_months(day("2026-12-01"), 1), day("2027-01-01"));
        assert_eq!(add_days(day("2026-02-27"), 3), day("2026-03-02"));
    }
}
