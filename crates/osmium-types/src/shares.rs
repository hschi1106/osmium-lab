use std::{
    fmt,
    ops::{Add, AddAssign, Sub, SubAssign},
};

/// A quantity in shares (股), the single quantity unit used after decoding.
///
/// Regular-board marketdata quantities are lots; `osmium-md` multiplies them by the
/// instrument's shares per lot. Signed so that differences and short positions are exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Shares(i64);

impl Shares {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(shares: i64) -> Self {
        Self(shares)
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl Add for Shares {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for Shares {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl AddAssign for Shares {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl SubAssign for Shares {
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

impl fmt::Display for Shares {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Shares;

    #[test]
    fn arithmetic_is_exact_and_signed() {
        let mut held = Shares::new(2_000);
        held -= Shares::new(3_000);
        assert_eq!(held, Shares::new(-1_000));
        held += Shares::new(1_500);
        assert_eq!(held + Shares::new(500) - Shares::new(1_000), Shares::ZERO);
        assert!(Shares::new(999) < Shares::new(1_000));
        assert_eq!(Shares::default(), Shares::ZERO);
    }
}
