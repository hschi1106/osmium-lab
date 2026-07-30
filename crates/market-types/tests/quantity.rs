use market_types::{Quantity, QuantityError, QuantityUnit, QuantityUnitError};

#[test]
fn quantity_unit_has_fixed_canonical_discriminants() {
    let cases = [
        (QuantityUnit::SourceUnit, 0),
        (QuantityUnit::Share, 1),
        (QuantityUnit::TradingUnit, 2),
        (QuantityUnit::Contract, 3),
    ];

    for (unit, discriminant) in cases {
        assert_eq!(unit.discriminant(), discriminant);
        assert_eq!(u8::from(unit), discriminant);
        assert_eq!(QuantityUnit::try_from(discriminant), Ok(unit));
    }
    assert_eq!(
        QuantityUnit::try_from(4),
        Err(QuantityUnitError::UnknownDiscriminant(4))
    );
}

#[test]
fn quantity_requires_a_positive_value_and_encodes_its_unit() {
    assert_eq!(
        Quantity::new(0, QuantityUnit::SourceUnit),
        Err(QuantityError::Zero)
    );

    let quantity = Quantity::new(42, QuantityUnit::Share).unwrap();
    assert_eq!(quantity.value(), 42);
    assert_eq!(quantity.unit(), QuantityUnit::Share);

    let mut expected = [0_u8; 9];
    expected[0] = 1;
    expected[1..].copy_from_slice(&42_u64.to_be_bytes());
    assert_eq!(quantity.to_canonical_bytes(), expected);
}

#[test]
fn quantity_arithmetic_is_checked_and_unit_safe() {
    let ten = Quantity::new(10, QuantityUnit::Contract).unwrap();
    let three = Quantity::new(3, QuantityUnit::Contract).unwrap();

    assert_eq!(ten.checked_add(three).unwrap().value(), 13);
    assert_eq!(ten.checked_sub(three).unwrap().value(), 7);
    assert_eq!(three.checked_sub(ten), Err(QuantityError::Underflow));
    assert_eq!(ten.checked_sub(ten), Err(QuantityError::Zero));
    assert_eq!(
        Quantity::new(u64::MAX, QuantityUnit::Contract)
            .unwrap()
            .checked_add(Quantity::new(1, QuantityUnit::Contract).unwrap()),
        Err(QuantityError::Overflow)
    );

    let shares = Quantity::new(10, QuantityUnit::Share).unwrap();
    assert_eq!(
        ten.checked_add(shares),
        Err(QuantityError::UnitMismatch {
            left: QuantityUnit::Contract,
            right: QuantityUnit::Share,
        })
    );
}
