use std::str::FromStr;

use market_types::{Decimal, DecimalError};

#[test]
fn m1_t012_decimal_never_rounds_or_uses_float() {
    let integer = Decimal::parse("2350").unwrap();
    let fractional = Decimal::from_str("2350.0").unwrap();
    let exponent = Decimal::try_from("2.350e3").unwrap();

    assert_eq!(integer, fractional);
    assert_eq!(integer, exponent);
    assert_eq!(integer.atoms(), 2_350_000_000_000_000_000_000_i128);

    assert_eq!(Decimal::parse("0.000000000000000001").unwrap().atoms(), 1);
    assert_eq!(Decimal::parse("1e-18").unwrap().atoms(), 1);
    assert_eq!(
        Decimal::parse("1.230000000000000000000").unwrap(),
        Decimal::parse("1.23").unwrap()
    );
    assert_eq!(Decimal::parse("-0").unwrap(), Decimal::ZERO);

    let maximum = Decimal::parse("170141183460469231731.687303715884105727").unwrap();
    let minimum = Decimal::parse("-170141183460469231731.687303715884105728").unwrap();
    assert_eq!(maximum.atoms(), i128::MAX);
    assert_eq!(minimum.atoms(), i128::MIN);

    assert_eq!(
        Decimal::parse("0.0000000000000000001"),
        Err(DecimalError::PrecisionLoss)
    );
    assert_eq!(Decimal::parse("1e-19"), Err(DecimalError::PrecisionLoss));
    assert_eq!(
        Decimal::parse("170141183460469231731.687303715884105728"),
        Err(DecimalError::OutOfRange)
    );
    assert_eq!(
        Decimal::parse("-170141183460469231731.687303715884105729"),
        Err(DecimalError::OutOfRange)
    );

    for input in ["", "+", ".", "NaN", "infinity", " 1", "1 ", "1e"] {
        assert_eq!(
            Decimal::parse(input),
            Err(DecimalError::InvalidFormat),
            "input: {input}"
        );
    }
}

#[test]
fn decimal_canonical_encoding_and_arithmetic_are_exact() {
    let positive = Decimal::from_atoms(1);
    let negative = Decimal::from_atoms(-1);
    assert_eq!(positive.to_canonical_bytes(), 1_i128.to_be_bytes());
    assert_eq!(negative.to_canonical_bytes(), (-1_i128).to_be_bytes());

    assert_eq!(
        Decimal::parse("1.5")
            .unwrap()
            .checked_add(Decimal::parse("2.25").unwrap())
            .unwrap(),
        Decimal::parse("3.75").unwrap()
    );
    assert_eq!(
        Decimal::parse("2.25")
            .unwrap()
            .checked_sub(Decimal::parse("1.5").unwrap())
            .unwrap(),
        Decimal::parse("0.75").unwrap()
    );
    assert_eq!(
        Decimal::parse("1.5").unwrap().checked_neg().unwrap(),
        Decimal::parse("-1.5").unwrap()
    );
    assert_eq!(
        Decimal::from_atoms(i128::MAX).checked_add(positive),
        Err(DecimalError::OutOfRange)
    );
    assert_eq!(
        Decimal::from_atoms(i128::MIN).checked_neg(),
        Err(DecimalError::OutOfRange)
    );
}
