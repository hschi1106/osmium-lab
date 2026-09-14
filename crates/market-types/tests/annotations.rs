use market_types::{
    InstantTrend, LimitPosition, MatchingMethod, TpexLimits, TpexStatus, TwseLimits, TwseStatus,
};

fn expected_position(bits: u8) -> LimitPosition {
    match bits {
        0 => LimitPosition::Normal,
        1 => LimitPosition::LowerLimit,
        2 => LimitPosition::UpperLimit,
        _ => LimitPosition::Reserved,
    }
}

fn expected_trend(bits: u8) -> InstantTrend {
    match bits {
        0 => InstantTrend::Normal,
        1 => InstantTrend::VolatilityInterruptionDown,
        2 => InstantTrend::VolatilityInterruptionUp,
        _ => InstantTrend::Reserved,
    }
}

#[test]
fn twse_status_decodes_each_raw_byte_without_losing_reserved_bits() {
    for raw in 0..=u8::MAX {
        let status = TwseStatus::from_raw(raw);
        let trial = raw & 0x80 != 0;
        assert_eq!(status.raw(), raw);
        assert_eq!(status.trial(), trial);
        assert_eq!(status.delayed_open(), trial && raw & 0x40 != 0);
        assert_eq!(status.delayed_close(), trial && raw & 0x20 != 0);
        assert_eq!(
            status.matching_method(),
            if raw & 0x10 == 0 {
                MatchingMethod::CallAuction
            } else {
                MatchingMethod::Continuous
            }
        );
        assert_eq!(status.opening_marker(), raw & 0x08 != 0);
        assert_eq!(status.closing_marker(), raw & 0x04 != 0);
        assert_eq!(status.reserved_bits(), raw & 0x03);
    }
}

#[test]
fn tpex_status_decodes_each_raw_byte_without_losing_reserved_bits() {
    for raw in 0..=u8::MAX {
        let status = TpexStatus::from_raw(raw);
        let trial = raw & 0x80 != 0;
        assert_eq!(status.raw(), raw);
        assert_eq!(status.trial(), trial);
        assert_eq!(status.delayed_open(), trial && raw & 0x40 != 0);
        assert_eq!(status.delayed_close(), trial && raw & 0x20 != 0);
        assert_eq!(
            status.matching_method(),
            if raw & 0x10 == 0 {
                MatchingMethod::CallAuction
            } else {
                MatchingMethod::Continuous
            }
        );
        assert_eq!(status.opening_marker(), raw & 0x08 != 0);
        assert_eq!(status.closing_marker(), raw & 0x04 != 0);
        assert_eq!(status.reserved_bits(), raw & 0x03);
    }
}

#[test]
fn twse_limits_decode_each_two_bit_field() {
    for raw in 0..=u8::MAX {
        let limits = TwseLimits::from_raw(raw);
        assert_eq!(limits.raw(), raw);
        assert_eq!(limits.trade(), expected_position((raw >> 6) & 0x03));
        assert_eq!(limits.best_bid(), expected_position((raw >> 4) & 0x03));
        assert_eq!(limits.best_ask(), expected_position((raw >> 2) & 0x03));
        assert_eq!(limits.instant_trend(), expected_trend(raw & 0x03));
    }
}

#[test]
fn tpex_limits_decode_each_two_bit_field() {
    for raw in 0..=u8::MAX {
        let limits = TpexLimits::from_raw(raw);
        assert_eq!(limits.raw(), raw);
        assert_eq!(limits.trade(), expected_position((raw >> 6) & 0x03));
        assert_eq!(limits.best_bid(), expected_position((raw >> 4) & 0x03));
        assert_eq!(limits.best_ask(), expected_position((raw >> 2) & 0x03));
        assert_eq!(limits.instant_trend(), expected_trend(raw & 0x03));
    }
}
