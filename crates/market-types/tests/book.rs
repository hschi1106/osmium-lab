#[path = "support/book.rs"]
mod support;

use market_types::{BookError, BookSide, BookSideKind, Quantity, QuantityUnit};
use support::level;

#[test]
fn book_side_enforces_contiguity_price_order_and_units() {
    let bids = BookSide::new(
        BookSideKind::Bid,
        vec![
            level("100", 2, QuantityUnit::TradingUnit),
            level("99.5", 3, QuantityUnit::TradingUnit),
        ],
    )
    .unwrap();
    assert_eq!(bids.levels().count(), 2);

    let invalid_order = BookSide::new(
        BookSideKind::Bid,
        vec![
            level("100", 2, QuantityUnit::TradingUnit),
            level("100", 3, QuantityUnit::TradingUnit),
        ],
    );
    assert!(matches!(
        invalid_order,
        Err(BookError::PriceOrder {
            side: BookSideKind::Bid,
            index: 1
        })
    ));

    let invalid_units = BookSide::new(
        BookSideKind::Ask,
        vec![
            level("100", 2, QuantityUnit::TradingUnit),
            level("101", 3, QuantityUnit::Share),
        ],
    );
    assert!(matches!(
        invalid_units,
        Err(BookError::UnitMismatch { index: 1, .. })
    ));

    let non_contiguous = BookSide::from_slots(
        BookSideKind::Ask,
        [
            Some(level("100", 2, QuantityUnit::TradingUnit)),
            None,
            Some(level("101", 3, QuantityUnit::TradingUnit)),
            None,
            None,
        ],
    );
    assert!(matches!(
        non_contiguous,
        Err(BookError::NonContiguous { index: 2, .. })
    ));
}

#[test]
fn market_order_quantity_is_separate_from_executable_price_levels() {
    let market_quantity = Quantity::new(7, QuantityUnit::TradingUnit).unwrap();
    let bids = BookSide::with_market_order_quantity(
        BookSideKind::Bid,
        Some(market_quantity),
        vec![
            level("100", 2, QuantityUnit::TradingUnit),
            level("99.5", 3, QuantityUnit::TradingUnit),
        ],
    )
    .unwrap();
    assert_eq!(bids.market_order_quantity(), Some(market_quantity));
    assert_eq!(bids.levels().count(), 2);
    assert_eq!(
        bids.levels().next().unwrap().price().atoms(),
        100_000_000_000_000_000_000
    );

    let too_many = BookSide::with_market_order_quantity(
        BookSideKind::Bid,
        Some(market_quantity),
        vec![
            level("105", 1, QuantityUnit::TradingUnit),
            level("104", 1, QuantityUnit::TradingUnit),
            level("103", 1, QuantityUnit::TradingUnit),
            level("102", 1, QuantityUnit::TradingUnit),
            level("101", 1, QuantityUnit::TradingUnit),
        ],
    );
    assert!(matches!(
        too_many,
        Err(BookError::TooManyLevels {
            side: BookSideKind::Bid,
            count: 6
        })
    ));
}
