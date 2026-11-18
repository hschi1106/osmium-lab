//! Brokerage fee and securities transaction tax
//! (`docs/reference/accounting.md` sections 5 and 6).

use osmium_types::{Board, Date, Money};

use crate::{
    amount::{Rate, Rounding},
    config::FeeConfig,
    facts::SecurityClass,
};

/// The fee of one order for one day's trade value.
#[must_use]
pub fn fee(config: &FeeConfig, board: Board, value: Money) -> Money {
    if value.get() <= 0 {
        return Money::ZERO;
    }
    let rate = Rate::new(
        config
            .rate
            .numerator()
            .checked_mul(config.discount.numerator())
            .expect("a fee rate"),
        config
            .rate
            .denominator()
            .checked_mul(config.discount.denominator())
            .expect("a fee rate"),
    );
    let minimum = match board {
        Board::Regular => config.min_regular,
        Board::OddLot => config.min_odd,
    };
    rate.of(value, config.rounding).max(minimum)
}

fn date(year: u32, month: u32, day: u32) -> Date {
    Date::from_ymd(year, month, day).expect("a valid date")
}

/// The transaction tax rate on a sale on `on`.
#[must_use]
pub fn tax_rate(class: SecurityClass, on: Date) -> Rate {
    let per_mille = match class {
        SecurityClass::Stock | SecurityClass::Preferred => 3,
        SecurityClass::Tdr | SecurityClass::Etf | SecurityClass::Etn | SecurityClass::Warrant => 1,
        SecurityClass::Reit => 0,
        SecurityClass::BondEtf => i64::from(on < date(2017, 1, 1) || on > date(2026, 12, 31)),
        SecurityClass::ConvertibleBond => {
            i64::from(on < date(2010, 1, 1) || on > date(2026, 12, 31))
        }
    };
    Rate::new(per_mille, 1_000)
}

/// The rate on the part of a sale that is paired as a cash day trade: half the stock
/// rate from 2017-04-28 to 2027-12-31, the ordinary rate otherwise.
#[must_use]
pub fn day_trade_tax_rate(class: SecurityClass, on: Date) -> Rate {
    let halved = matches!(class, SecurityClass::Stock | SecurityClass::Preferred)
        && date(2017, 4, 28) <= on
        && on <= date(2027, 12, 31);
    if halved {
        Rate::new(15, 10_000)
    } else {
        tax_rate(class, on)
    }
}

/// The tax of one sell order for one day: each part is rounded down on its own.
#[must_use]
pub fn tax(
    class: SecurityClass,
    on: Date,
    day_trade_value: Money,
    other_value: Money,
) -> (Money, Money) {
    (
        day_trade_tax_rate(class, on).of(day_trade_value, Rounding::Floor),
        tax_rate(class, on).of(other_value, Rounding::Floor),
    )
}

#[cfg(test)]
mod tests {
    use osmium_types::{Board, Date, Money};

    use super::{day_trade_tax_rate, fee, tax, tax_rate};
    use crate::{
        amount::{Rate, Rounding},
        config::Config,
        facts::SecurityClass,
    };

    fn day(text: &str) -> Date {
        text.parse().unwrap()
    }

    #[test]
    fn the_fee_has_a_minimum_per_order_and_none_without_a_trade() {
        let config = Config::new(Money::ZERO).fee;
        // 100,500 × 0.1425 % = 143.21
        assert_eq!(
            fee(&config, Board::Regular, Money::new(100_500)),
            Money::new(143)
        );
        assert_eq!(
            fee(&config, Board::Regular, Money::new(10_000)),
            Money::new(20)
        );
        assert_eq!(fee(&config, Board::OddLot, Money::new(500)), Money::new(1));
        assert_eq!(
            fee(&config, Board::OddLot, Money::new(10_000)),
            Money::new(14)
        );
        assert_eq!(fee(&config, Board::Regular, Money::ZERO), Money::ZERO);
    }

    #[test]
    fn the_discount_and_rounding_are_applied_before_the_minimum() {
        let mut config = Config::new(Money::ZERO).fee;
        config.discount = Rate::new(28, 100);
        // 1,000,000 × 0.1425 % × 0.28 = 399
        assert_eq!(
            fee(&config, Board::Regular, Money::new(1_000_000)),
            Money::new(399)
        );
        // 100,500 × 0.1425 % × 0.28 = 40.0995
        assert_eq!(
            fee(&config, Board::Regular, Money::new(100_500)),
            Money::new(40)
        );
        config.rounding = Rounding::Ceil;
        assert_eq!(
            fee(&config, Board::Regular, Money::new(100_500)),
            Money::new(41)
        );
        config.rounding = Rounding::HalfUp;
        assert_eq!(
            fee(&config, Board::Regular, Money::new(100_500)),
            Money::new(40)
        );
    }

    #[test]
    fn tax_rates_follow_the_class_and_the_date() {
        let on = day("2026-10-08");
        for (class, per_mille) in [
            (SecurityClass::Stock, 3),
            (SecurityClass::Preferred, 3),
            (SecurityClass::Tdr, 1),
            (SecurityClass::Etf, 1),
            (SecurityClass::Etn, 1),
            (SecurityClass::Warrant, 1),
            (SecurityClass::Reit, 0),
            (SecurityClass::BondEtf, 0),
            (SecurityClass::ConvertibleBond, 0),
        ] {
            assert_eq!(
                tax_rate(class, on),
                Rate::new(per_mille, 1_000),
                "{class:?}"
            );
        }
        // The exemptions end with 2026.
        let later = day("2027-01-04");
        assert_eq!(tax_rate(SecurityClass::BondEtf, later), Rate::new(1, 1_000));
        assert_eq!(
            tax_rate(SecurityClass::ConvertibleBond, later),
            Rate::new(1, 1_000)
        );
    }

    #[test]
    fn only_stocks_pay_half_on_day_trades_and_only_until_2027() {
        let on = day("2026-10-08");
        assert_eq!(
            day_trade_tax_rate(SecurityClass::Stock, on),
            Rate::new(15, 10_000)
        );
        assert_eq!(
            day_trade_tax_rate(SecurityClass::Etf, on),
            Rate::new(1, 1_000)
        );
        assert_eq!(
            day_trade_tax_rate(SecurityClass::Tdr, on),
            Rate::new(1, 1_000)
        );
        assert_eq!(
            day_trade_tax_rate(SecurityClass::Stock, day("2028-01-03")),
            Rate::new(3, 1_000)
        );
    }

    #[test]
    fn each_part_of_the_tax_is_rounded_down_on_its_own() {
        // 100,500 × 0.15 % = 150.75 and 50,300 × 0.3 % = 150.9
        assert_eq!(
            tax(
                SecurityClass::Stock,
                day("2026-10-08"),
                Money::new(100_500),
                Money::new(50_300)
            ),
            (Money::new(150), Money::new(150))
        );
    }
}
