use std::{error::Error, fmt, str::FromStr};

/// Number of fractional decimal places represented by [`Decimal`].
pub const DECIMAL_SCALE: u32 = 18;

/// Number of atoms in one whole decimal unit.
pub const DECIMAL_SCALE_FACTOR: i128 = 1_000_000_000_000_000_000;

/// An exact fixed-scale decimal represented by signed 10^-18 atoms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Decimal(i128);

impl Decimal {
    pub const ZERO: Self = Self(0);
    pub const SCALE: u32 = DECIMAL_SCALE;
    pub const SCALE_FACTOR: i128 = DECIMAL_SCALE_FACTOR;

    /// Constructs an exact decimal from signed 10^-18 atoms.
    #[must_use]
    pub const fn from_atoms(atoms: i128) -> Self {
        Self(atoms)
    }

    /// Returns the signed 10^-18 atoms.
    #[must_use]
    pub const fn atoms(self) -> i128 {
        self.0
    }

    /// Parses decimal text directly, including an optional base-10 exponent.
    pub fn parse(input: &str) -> Result<Self, DecimalError> {
        parse_decimal(input)
    }

    /// Returns the version-1 canonical two's-complement big-endian bytes.
    #[must_use]
    pub const fn to_canonical_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }

    /// Adds two decimals, returning an error instead of wrapping or saturating.
    pub fn checked_add(self, rhs: Self) -> Result<Self, DecimalError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(DecimalError::OutOfRange)
    }

    /// Subtracts two decimals, returning an error instead of wrapping or saturating.
    pub fn checked_sub(self, rhs: Self) -> Result<Self, DecimalError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(DecimalError::OutOfRange)
    }

    /// Negates a decimal, returning an error for `i128::MIN`.
    pub fn checked_neg(self) -> Result<Self, DecimalError> {
        self.0
            .checked_neg()
            .map(Self)
            .ok_or(DecimalError::OutOfRange)
    }
}

impl FromStr for Decimal {
    type Err = DecimalError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl TryFrom<&str> for Decimal {
    type Error = DecimalError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        Self::parse(input)
    }
}

/// Stable error categories for exact decimal parsing and arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalError {
    InvalidFormat,
    PrecisionLoss,
    OutOfRange,
}

impl fmt::Display for DecimalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidFormat => "value is not valid decimal text",
            Self::PrecisionLoss => "value cannot be represented without losing decimal precision",
            Self::OutOfRange => "value is outside the signed 128-bit decimal range",
        };

        formatter.write_str(message)
    }
}

impl Error for DecimalError {}

fn parse_decimal(input: &str) -> Result<Decimal, DecimalError> {
    let bytes = input.as_bytes();
    if bytes.is_empty() {
        return Err(DecimalError::InvalidFormat);
    }

    let mut cursor = 0;
    let negative = match bytes[cursor] {
        b'-' => {
            cursor += 1;
            true
        }
        b'+' => {
            cursor += 1;
            false
        }
        _ => false,
    };

    let integer_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    let integer_digits = &bytes[integer_start..cursor];

    let fraction_digits = if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let fraction_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        &bytes[fraction_start..cursor]
    } else {
        &bytes[0..0]
    };

    if integer_digits.is_empty() && fraction_digits.is_empty() {
        return Err(DecimalError::InvalidFormat);
    }

    let exponent = if matches!(bytes.get(cursor), Some(b'e' | b'E')) {
        cursor += 1;
        let (exponent, next_cursor) = parse_exponent(bytes, cursor)?;
        cursor = next_cursor;
        exponent
    } else {
        0
    };

    if cursor != bytes.len() {
        return Err(DecimalError::InvalidFormat);
    }

    let fraction_length =
        i64::try_from(fraction_digits.len()).map_err(|_| DecimalError::OutOfRange)?;
    let atom_shift = i64::from(DECIMAL_SCALE)
        .checked_add(exponent)
        .and_then(|value| value.checked_sub(fraction_length))
        .ok_or(DecimalError::OutOfRange)?;

    let digits = integer_digits.iter().chain(fraction_digits.iter());
    if digits.clone().all(|digit| *digit == b'0') {
        return Ok(Decimal::ZERO);
    }

    let total_digits = integer_digits
        .len()
        .checked_add(fraction_digits.len())
        .ok_or(DecimalError::OutOfRange)?;
    let retained_digits = if atom_shift < 0 {
        let removed_digits =
            usize::try_from(atom_shift.unsigned_abs()).map_err(|_| DecimalError::PrecisionLoss)?;
        if removed_digits > total_digits {
            return Err(DecimalError::PrecisionLoss);
        }

        let retained_digits = total_digits - removed_digits;
        if digits
            .clone()
            .skip(retained_digits)
            .any(|digit| *digit != b'0')
        {
            return Err(DecimalError::PrecisionLoss);
        }
        retained_digits
    } else {
        total_digits
    };

    let mut magnitude = 0_u128;
    for digit in digits.take(retained_digits) {
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(u128::from(digit - b'0')))
            .ok_or(DecimalError::OutOfRange)?;
    }

    if atom_shift > 38 {
        return Err(DecimalError::OutOfRange);
    }
    if atom_shift > 0 {
        for _ in 0..atom_shift {
            magnitude = magnitude.checked_mul(10).ok_or(DecimalError::OutOfRange)?;
        }
    }

    signed_atoms(magnitude, negative).map(Decimal::from_atoms)
}

fn parse_exponent(bytes: &[u8], mut cursor: usize) -> Result<(i64, usize), DecimalError> {
    let negative = match bytes.get(cursor) {
        Some(b'-') => {
            cursor += 1;
            true
        }
        Some(b'+') => {
            cursor += 1;
            false
        }
        _ => false,
    };

    let start = cursor;
    let mut magnitude = 0_u64;
    while let Some(digit) = bytes.get(cursor).filter(|digit| digit.is_ascii_digit()) {
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(*digit - b'0')))
            .ok_or(DecimalError::OutOfRange)?;
        cursor += 1;
    }
    if cursor == start {
        return Err(DecimalError::InvalidFormat);
    }

    let exponent = if negative {
        if magnitude == (i64::MAX as u64) + 1 {
            i64::MIN
        } else {
            let magnitude = i64::try_from(magnitude).map_err(|_| DecimalError::OutOfRange)?;
            -magnitude
        }
    } else {
        i64::try_from(magnitude).map_err(|_| DecimalError::OutOfRange)?
    };

    Ok((exponent, cursor))
}

fn signed_atoms(magnitude: u128, negative: bool) -> Result<i128, DecimalError> {
    if negative {
        const I128_MIN_MAGNITUDE: u128 = 1_u128 << 127;
        if magnitude == I128_MIN_MAGNITUDE {
            Ok(i128::MIN)
        } else if magnitude <= i128::MAX as u128 {
            Ok(-(magnitude as i128))
        } else {
            Err(DecimalError::OutOfRange)
        }
    } else {
        i128::try_from(magnitude).map_err(|_| DecimalError::OutOfRange)
    }
}
