//! Exact arithmetic on money: decimal rates, the three rounding modes and proportional
//! allocation. Nothing here uses binary floating point.

use std::{error::Error, fmt, str::FromStr};

use osmium_types::{Money, Price, Shares};

/// How a product is rounded to whole yuan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rounding {
    Floor,
    HalfUp,
    Ceil,
}

/// A non-negative decimal number, exact: `numerator / denominator`.
#[derive(Debug, Clone, Copy)]
pub struct Rate {
    numerator: i64,
    denominator: i64,
}

impl PartialEq for Rate {
    fn eq(&self, other: &Self) -> bool {
        i128::from(self.numerator) * i128::from(other.denominator)
            == i128::from(other.numerator) * i128::from(self.denominator)
    }
}

impl Eq for Rate {}

impl Rate {
    pub const ZERO: Self = Self::new(0, 1);
    pub const ONE: Self = Self::new(1, 1);

    /// # Panics
    ///
    /// When `numerator` is negative or `denominator` is not positive.
    #[must_use]
    pub const fn new(numerator: i64, denominator: i64) -> Self {
        assert!(numerator >= 0 && denominator > 0);
        Self {
            numerator,
            denominator,
        }
    }

    #[must_use]
    pub const fn numerator(self) -> i64 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> i64 {
        self.denominator
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.numerator == 0
    }

    /// `self + other`.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self::new(
            self.numerator * other.denominator + other.numerator * self.denominator,
            self.denominator * other.denominator,
        )
    }

    /// The larger of the two.
    #[must_use]
    pub fn at_least(self, other: Self) -> Self {
        if self.is_less_than(other) {
            other
        } else {
            self
        }
    }

    #[must_use]
    pub fn is_less_than(self, other: Self) -> bool {
        i128::from(self.numerator) * i128::from(other.denominator)
            < i128::from(other.numerator) * i128::from(self.denominator)
    }

    /// `amount × self`, rounded to whole yuan. `amount` must not be negative.
    #[must_use]
    pub fn of(self, amount: Money, rounding: Rounding) -> Money {
        Money::new(divide(
            i128::from(amount.get()) * i128::from(self.numerator),
            i128::from(self.denominator),
            rounding,
        ))
    }
}

/// `numerator / denominator` rounded to an integer; both must be non-negative and
/// the denominator positive.
///
/// # Panics
///
/// When the arguments are out of that range or the result does not fit `i64`.
#[must_use]
pub fn divide(numerator: i128, denominator: i128, rounding: Rounding) -> i64 {
    assert!(
        numerator >= 0 && denominator > 0,
        "{numerator} / {denominator}"
    );
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let up = match rounding {
        Rounding::Floor => false,
        Rounding::HalfUp => remainder * 2 >= denominator,
        Rounding::Ceil => remainder > 0,
    };
    i64::try_from(quotient + i128::from(up)).expect("an amount fits i64")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseRateError(String);

impl fmt::Display for ParseRateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "`{}` is not a non-negative decimal number",
            self.0
        )
    }
}

impl Error for ParseRateError {}

impl FromStr for Rate {
    type Err = ParseRateError;

    /// Parses a plain decimal such as `0.001425` or `1.3`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || ParseRateError(text.to_owned());
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
        if whole.is_empty() || !digits(whole) || !digits(fraction) || fraction.len() > 12 {
            return Err(invalid());
        }
        if text.contains('.') && fraction.is_empty() {
            return Err(invalid());
        }
        let denominator = 10_i64.pow(u32::try_from(fraction.len()).expect("at most 12"));
        let whole: i64 = whole.parse().map_err(|_| invalid())?;
        let fraction: i64 = if fraction.is_empty() {
            0
        } else {
            fraction.parse().map_err(|_| invalid())?
        };
        whole
            .checked_mul(denominator)
            .and_then(|scaled| scaled.checked_add(fraction))
            .map(|numerator| Self::new(numerator, denominator))
            .ok_or_else(invalid)
    }
}

/// `price × shares` in price units (1e-4 yuan).
#[must_use]
pub fn notional_units(price: Price, shares: i64) -> i128 {
    i128::from(price.units()) * i128::from(shares)
}

/// `price × shares` in whole yuan, rounded down.
#[must_use]
pub fn notional(price: Price, shares: i64) -> Money {
    Money::new(divide(
        notional_units(price, shares),
        i128::from(Price::UNITS_PER_YUAN),
        Rounding::Floor,
    ))
}

/// The trade value of the fills of one order on one day: fills at the same price are
/// added up and each sum is rounded down to whole yuan
/// (`docs/reference/accounting.md` section 7.4).
#[must_use]
pub fn trade_value(fills: &[(Price, Shares)]) -> Money {
    let mut by_price: Vec<(Price, i64)> = Vec::new();
    for &(price, qty) in fills {
        match by_price.iter_mut().find(|(at, _)| *at == price) {
            Some((_, total)) => *total += qty.get(),
            None => by_price.push((price, qty.get())),
        }
    }
    by_price
        .into_iter()
        .map(|(price, qty)| notional(price, qty))
        .sum()
}

/// Splits `total` in proportion to `weights` so that the parts are whole and add up to
/// `total`: each part is rounded down and the yuan left over go to the largest
/// remainders, earlier parts first on a tie.
///
/// # Panics
///
/// When `total` or a weight is negative, or `total` is positive and every weight is 0.
#[must_use]
pub fn allocate(total: Money, weights: &[i128]) -> Vec<Money> {
    assert!(total.get() >= 0 && weights.iter().all(|&weight| weight >= 0));
    let sum: i128 = weights.iter().sum();
    if total.get() == 0 {
        return vec![Money::ZERO; weights.len()];
    }
    assert!(sum > 0, "nothing to allocate {total} to");
    let total = i128::from(total.get());
    let mut parts: Vec<i128> = weights.iter().map(|&weight| total * weight / sum).collect();
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by_key(|&index| std::cmp::Reverse(total * weights[index] % sum));
    let left = total - parts.iter().sum::<i128>();
    for &index in order
        .iter()
        .take(usize::try_from(left).expect("fewer than the parts"))
    {
        parts[index] += 1;
    }
    parts
        .into_iter()
        .map(|part| Money::new(i64::try_from(part).expect("at most the total")))
        .collect()
}

/// The part of `amount` that belongs to `part` of `whole` shares, rounded down. Taking
/// every remaining share takes everything that remains.
#[must_use]
pub fn share_of(amount: Money, part: i64, whole: i64) -> Money {
    assert!(0 <= part && part <= whole && whole > 0 && amount.get() >= 0);
    Money::new(divide(
        i128::from(amount.get()) * i128::from(part),
        i128::from(whole),
        Rounding::Floor,
    ))
}

#[cfg(test)]
mod tests {
    use osmium_types::{Money, Price, Shares};

    use super::{Rate, Rounding, allocate, divide, share_of, trade_value};

    fn price(text: &str) -> Price {
        text.parse().unwrap()
    }

    #[test]
    fn rates_parse_exactly() {
        let rate: Rate = "0.001425".parse().unwrap();
        assert_eq!((rate.numerator(), rate.denominator()), (1_425, 1_000_000));
        assert_eq!("1.3".parse::<Rate>().unwrap(), Rate::new(13, 10));
        assert_eq!("0".parse::<Rate>().unwrap(), Rate::ZERO);
        assert_eq!("1".parse::<Rate>().unwrap(), Rate::ONE);
        for bad in [
            "",
            "-1",
            "1.",
            ".5",
            "1e-3",
            "0.1.2",
            "abc",
            "1.0000000000001",
        ] {
            assert!(bad.parse::<Rate>().is_err(), "{bad}");
        }
    }

    #[test]
    fn the_three_roundings_differ_only_on_a_remainder() {
        assert_eq!(divide(10, 4, Rounding::Floor), 2);
        assert_eq!(divide(10, 4, Rounding::HalfUp), 3);
        assert_eq!(divide(10, 4, Rounding::Ceil), 3);
        assert_eq!(divide(9, 4, Rounding::HalfUp), 2);
        assert_eq!(divide(9, 4, Rounding::Ceil), 3);
        for rounding in [Rounding::Floor, Rounding::HalfUp, Rounding::Ceil] {
            assert_eq!(divide(8, 4, rounding), 2);
            assert_eq!(divide(0, 4, rounding), 0);
        }
        assert_eq!(Rate::new(9, 10).plus(Rate::new(1, 10)), Rate::ONE);
        assert_eq!(Rate::new(9, 10).at_least(Rate::ONE), Rate::ONE);
        assert_eq!(Rate::new(13, 10).at_least(Rate::ONE), Rate::new(13, 10));
        // 100,500 × 0.1425 % = 143.2125
        let fee: Rate = "0.001425".parse().unwrap();
        assert_eq!(
            fee.of(Money::new(100_500), Rounding::Floor),
            Money::new(143)
        );
        assert_eq!(fee.of(Money::new(100_500), Rounding::Ceil), Money::new(144));
    }

    #[test]
    fn odd_lot_value_is_rounded_down_once_per_price() {
        // 33.35 × 7 = 233.45 and 33.35 × 9 = 300.15; together 533.6, not 233 + 300.
        let fills = [
            (price("33.35"), Shares::new(7)),
            (price("33.4"), Shares::new(1)),
            (price("33.35"), Shares::new(9)),
        ];
        assert_eq!(trade_value(&fills), Money::new(533 + 33));
        assert_eq!(trade_value(&[]), Money::ZERO);
    }

    #[test]
    fn allocation_is_whole_and_adds_up() {
        assert_eq!(
            allocate(Money::new(10), &[1, 1, 1]),
            [Money::new(4), Money::new(3), Money::new(3)]
        );
        assert_eq!(
            allocate(Money::new(143), &[60_000, 40_500]),
            [Money::new(85), Money::new(58)]
        );
        assert_eq!(
            allocate(Money::new(1), &[1, 3]),
            [Money::ZERO, Money::new(1)]
        );
        assert_eq!(allocate(Money::ZERO, &[0, 0]), [Money::ZERO, Money::ZERO]);
        assert_eq!(
            allocate(Money::new(7), &[0, 5]),
            [Money::ZERO, Money::new(7)]
        );
    }

    #[test]
    fn the_last_part_of_a_lot_takes_what_is_left() {
        let mut left = (Money::new(100), 3);
        let mut taken = Vec::new();
        for _ in 0..3 {
            let part = share_of(left.0, 1, left.1);
            taken.push(part.get());
            left = (left.0 - part, left.1 - 1);
        }
        assert_eq!(taken, [33, 33, 34]);
    }
}
