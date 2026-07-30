use market_types::{Decimal, DecimalError, Price, PriceError};

#[test]
fn price_requires_a_strictly_positive_exact_decimal() {
    let price = Price::parse("2350.0").unwrap();
    assert_eq!(price.as_decimal(), Decimal::parse("2.350e3").unwrap());
    assert_eq!(price.atoms(), 2_350_000_000_000_000_000_000_i128);
    assert_eq!(price.to_canonical_bytes(), price.atoms().to_be_bytes());
    assert_eq!(Price::try_from(Decimal::ZERO), Err(PriceError::NonPositive));
    assert_eq!(Price::parse("-1"), Err(PriceError::NonPositive));
    assert_eq!(
        Price::parse("0.0000000000000000001"),
        Err(PriceError::InvalidDecimal(DecimalError::PrecisionLoss))
    );
}
