use market_types::{Quantity, QuantityError, QuantityUnit, Volume, VolumeError};

#[test]
fn volume_allows_zero_and_encodes_its_unit() {
    let volume = Volume::new(0, QuantityUnit::SourceUnit);
    assert_eq!(volume.value(), 0);
    assert_eq!(volume.unit(), QuantityUnit::SourceUnit);
    assert_eq!(volume.to_canonical_bytes(), [0_u8; 9]);
}

#[test]
fn volume_arithmetic_is_checked_and_unit_safe() {
    let ten = Volume::new(10, QuantityUnit::Share);
    let three = Volume::new(3, QuantityUnit::Share);

    assert_eq!(ten.checked_add(three).unwrap().value(), 13);
    assert_eq!(ten.checked_sub(three).unwrap().value(), 7);
    assert_eq!(ten.checked_sub(ten).unwrap().value(), 0);
    assert_eq!(three.checked_sub(ten), Err(VolumeError::Underflow));
    assert_eq!(
        Volume::new(u64::MAX, QuantityUnit::Share).checked_add(Volume::new(1, QuantityUnit::Share)),
        Err(VolumeError::Overflow)
    );
    assert_eq!(
        ten.checked_add(Volume::new(10, QuantityUnit::TradingUnit)),
        Err(VolumeError::UnitMismatch {
            left: QuantityUnit::Share,
            right: QuantityUnit::TradingUnit,
        })
    );
}

#[test]
fn quantity_and_volume_conversion_preserves_the_explicit_unit() {
    let quantity = Quantity::new(5, QuantityUnit::Contract).unwrap();
    let volume = Volume::from(quantity);
    assert_eq!(volume.value(), 5);
    assert_eq!(volume.unit(), QuantityUnit::Contract);
    assert_eq!(Quantity::try_from(volume), Ok(quantity));
    assert_eq!(
        Quantity::try_from(Volume::new(0, QuantityUnit::Contract)),
        Err(QuantityError::Zero)
    );
}
