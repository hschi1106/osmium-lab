use std::str::FromStr;

use market_types::{MatchTime, MatchTimeError};

#[test]
fn m1_t010_match_time_is_exact_utc_microseconds() {
    let unix_epoch = MatchTime::parse("1970-01-01T00:00:00Z").unwrap();
    assert_eq!(unix_epoch.as_unix_microseconds(), 0);

    let offset_instant = MatchTime::from_str("1970-01-01T08:00:00.123456+08:00").unwrap();
    assert_eq!(offset_instant.as_unix_microseconds(), 123_456);

    let same_instant = MatchTime::try_from("1970-01-01T00:00:00.123456Z").unwrap();
    assert_eq!(offset_instant, same_instant);

    let trailing_zero_precision = MatchTime::parse("1970-01-01T00:00:00.123456000Z").unwrap();
    assert_eq!(trailing_zero_precision, same_instant);

    let before_epoch = MatchTime::parse("1969-12-31T23:59:59.999999Z").unwrap();
    assert_eq!(before_epoch.as_unix_microseconds(), -1);
    assert!(before_epoch < unix_epoch);

    let teralion_observation = MatchTime::parse("2026-07-27T09:00:07.360140+08:00").unwrap();
    let normalized_observation = MatchTime::parse("2026-07-27T01:00:07.360140Z").unwrap();
    assert_eq!(
        teralion_observation.as_unix_microseconds(),
        1_785_114_007_360_140
    );
    assert_eq!(teralion_observation, normalized_observation);
}

#[test]
fn m1_t011_invalid_match_time_is_rejected() {
    let cases = [
        ("2026-07-27T09:00:07.360140", MatchTimeError::InvalidFormat),
        (
            "2026-07-27 09:00:07.360140+08:00",
            MatchTimeError::InvalidFormat,
        ),
        (
            "2026-02-29T09:00:07.360140+08:00",
            MatchTimeError::InvalidDate,
        ),
        ("2026-07-27T24:00:00+08:00", MatchTimeError::InvalidTime),
        ("2026-07-27T09:00:60+08:00", MatchTimeError::InvalidTime),
        ("2026-07-27T09:00:07+24:00", MatchTimeError::InvalidOffset),
        ("2026-07-27T09:00:07+08:60", MatchTimeError::InvalidOffset),
        (
            "2026-07-27T09:00:07.123456001+08:00",
            MatchTimeError::PrecisionLoss,
        ),
        ("2026-07-27T09:00:07.+08:00", MatchTimeError::InvalidFormat),
    ];

    for (input, expected) in cases {
        assert_eq!(MatchTime::parse(input), Err(expected), "input: {input}");
    }
}
