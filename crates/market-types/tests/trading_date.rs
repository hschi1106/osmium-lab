use std::str::FromStr;

use market_types::{TradingDate, TradingDateError};

#[test]
fn trading_date_uses_signed_epoch_days_and_canonical_bytes() {
    let epoch = TradingDate::parse("1970-01-01").unwrap();
    let before_epoch = TradingDate::from_str("1969-12-31").unwrap();
    let fixture_date = TradingDate::try_from("2026-07-23").unwrap();

    assert_eq!(epoch.as_epoch_days(), 0);
    assert_eq!(before_epoch.as_epoch_days(), -1);
    assert_eq!(fixture_date.as_epoch_days(), 20_657);
    assert_eq!(i32::from(fixture_date), 20_657);
    assert_eq!(before_epoch.to_canonical_bytes(), (-1_i32).to_be_bytes());
    assert!(before_epoch < epoch);
}

#[test]
fn trading_date_parses_and_formats_valid_gregorian_dates() {
    let leap_day = TradingDate::parse("2000-02-29").unwrap();
    assert_eq!(leap_day.as_epoch_days(), 11_016);
    assert_eq!(leap_day.to_string(), "2000-02-29");

    let epoch = TradingDate::from_epoch_days(0);
    assert_eq!(epoch.to_string(), "1970-01-01");
    assert_eq!(
        TradingDate::parse(&epoch.to_string()).unwrap(),
        TradingDate::from_epoch_days(0)
    );
}

#[test]
fn trading_date_rejects_invalid_format_and_nonexistent_dates() {
    let cases = [
        ("2026-7-23", TradingDateError::InvalidFormat),
        ("2026/07/23", TradingDateError::InvalidFormat),
        (" 2026-07-23", TradingDateError::InvalidFormat),
        ("2026-07-23 ", TradingDateError::InvalidFormat),
        ("202A-07-23", TradingDateError::InvalidFormat),
        ("2026-00-23", TradingDateError::InvalidDate),
        ("2026-13-23", TradingDateError::InvalidDate),
        ("2026-07-00", TradingDateError::InvalidDate),
        ("2026-04-31", TradingDateError::InvalidDate),
        ("1900-02-29", TradingDateError::InvalidDate),
        ("2026-02-29", TradingDateError::InvalidDate),
    ];

    for (input, expected) in cases {
        assert_eq!(TradingDate::parse(input), Err(expected), "input: {input}");
    }
}
