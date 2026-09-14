use market_types::{BookLevel, Price, Quantity, QuantityUnit};

pub(crate) fn level(price: &str, quantity: u64, unit: QuantityUnit) -> BookLevel {
    BookLevel::new(
        Price::parse(price).unwrap(),
        Quantity::new(quantity, unit).unwrap(),
    )
}
