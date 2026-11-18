use std::{
    fmt,
    iter::Sum,
    ops::{Add, AddAssign, Neg, Sub, SubAssign},
};

/// An amount of money in whole New Taiwan dollars (元). Positive is a receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Money(i64);

impl Money {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(yuan: i64) -> Self {
        Self(yuan)
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }
}

impl Add for Money {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for Money {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl Neg for Money {
    type Output = Self;

    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl AddAssign for Money {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl SubAssign for Money {
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

impl Sum for Money {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Self(iter.map(|money| money.0).sum())
    }
}

impl fmt::Display for Money {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Money;

    #[test]
    fn arithmetic_is_exact_and_signed() {
        let mut cash = Money::new(1_000);
        cash -= Money::new(1_500);
        assert_eq!(cash, Money::new(-500));
        assert!(cash.is_negative());
        cash += Money::new(700);
        assert_eq!(cash + Money::new(300) - Money::new(500), Money::ZERO);
        assert_eq!(-Money::new(5), Money::new(-5));
        let total: Money = [Money::new(1), Money::new(2), Money::new(-4)]
            .into_iter()
            .sum();
        assert_eq!(total, Money::new(-1));
        assert_eq!(Money::new(-42).to_string(), "-42");
    }
}
