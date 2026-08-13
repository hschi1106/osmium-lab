use std::{error::Error, fmt};

use market_types::{BOOK_DEPTH, CompleteBookSnapshot, Decimal, Price, Quantity, QuantityUnit};
use strategy_api::{OrderSide, OrderType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelFill {
    level_index: u8,
    price: Price,
    quantity: Quantity,
}

impl LevelFill {
    #[must_use]
    pub const fn level_index(self) -> u8 {
        self.level_index
    }

    #[must_use]
    pub const fn price(self) -> Price {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> Quantity {
        self.quantity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepthSweepResult {
    fills: Box<[LevelFill]>,
    requested: Quantity,
    filled_value: u64,
}

impl DepthSweepResult {
    #[must_use]
    pub const fn fills(&self) -> &[LevelFill] {
        &self.fills
    }

    #[must_use]
    pub const fn requested(&self) -> Quantity {
        self.requested
    }

    #[must_use]
    pub fn filled(&self) -> Option<Quantity> {
        Quantity::new(self.filled_value, self.requested.unit()).ok()
    }

    #[must_use]
    pub fn remaining(&self) -> Option<Quantity> {
        Quantity::new(
            self.requested.value() - self.filled_value,
            self.requested.unit(),
        )
        .ok()
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.filled_value == self.requested.value()
    }
}

pub fn sweep_visible_depth(
    book: &CompleteBookSnapshot,
    side: OrderSide,
    requested: Quantity,
    depth_levels: usize,
) -> Result<DepthSweepResult, DepthSweepError> {
    sweep_marketable_depth(
        book,
        side,
        requested,
        depth_levels,
        OrderType::Market,
        Decimal::ZERO,
    )
}

pub fn sweep_marketable_depth(
    book: &CompleteBookSnapshot,
    side: OrderSide,
    requested: Quantity,
    depth_levels: usize,
    order_type: OrderType,
    adverse_price_delta: Decimal,
) -> Result<DepthSweepResult, DepthSweepError> {
    sweep_marketable_depth_with_consumed(
        book,
        side,
        requested,
        depth_levels,
        order_type,
        adverse_price_delta,
        &[0; BOOK_DEPTH],
    )
}

pub(crate) fn sweep_marketable_depth_with_consumed(
    book: &CompleteBookSnapshot,
    side: OrderSide,
    requested: Quantity,
    depth_levels: usize,
    order_type: OrderType,
    adverse_price_delta: Decimal,
    consumed: &[u64; BOOK_DEPTH],
) -> Result<DepthSweepResult, DepthSweepError> {
    if !(1..=BOOK_DEPTH).contains(&depth_levels) {
        return Err(DepthSweepError::InvalidDepthLevels(depth_levels));
    }
    if adverse_price_delta.atoms() < 0 {
        return Err(DepthSweepError::InvalidAdversePriceDelta);
    }
    let levels = match side {
        OrderSide::Buy => book.asks().levels(),
        OrderSide::Sell => book.bids().levels(),
    };
    if let Some(actual) = book.quantity_unit()
        && actual != requested.unit()
    {
        return Err(DepthSweepError::QuantityUnitMismatch {
            expected: requested.unit(),
            actual,
        });
    }

    let mut remaining = requested.value();
    let mut fills = Vec::new();
    for (index, level) in levels.take(depth_levels).enumerate() {
        if remaining == 0 {
            break;
        }
        let price = apply_adverse_price(level.price(), side, adverse_price_delta)?;
        if !is_marketable(price, side, order_type) {
            break;
        }
        let available = level
            .displayed_quantity()
            .value()
            .checked_sub(consumed[index])
            .ok_or(DepthSweepError::ConsumedQuantityExceedsDisplayed {
                level_index: u8::try_from(index + 1).expect("book depth is at most five"),
            })?;
        if available == 0 {
            continue;
        }
        let fill_value = remaining.min(available);
        let quantity = Quantity::new(fill_value, requested.unit())
            .expect("positive remaining and displayed quantity produce a positive fill");
        fills.push(LevelFill {
            level_index: u8::try_from(index + 1).expect("book depth is at most five"),
            price,
            quantity,
        });
        remaining -= fill_value;
    }
    Ok(DepthSweepResult {
        fills: fills.into_boxed_slice(),
        requested,
        filled_value: requested.value() - remaining,
    })
}

fn apply_adverse_price(
    price: Price,
    side: OrderSide,
    adverse_price_delta: Decimal,
) -> Result<Price, DepthSweepError> {
    let adjusted = match side {
        OrderSide::Buy => price.as_decimal().checked_add(adverse_price_delta),
        OrderSide::Sell => price.as_decimal().checked_sub(adverse_price_delta),
    }
    .map_err(|_| DepthSweepError::InvalidAdversePriceDelta)?;
    Price::new(adjusted).map_err(|_| DepthSweepError::InvalidAdversePriceDelta)
}

fn is_marketable(price: Price, side: OrderSide, order_type: OrderType) -> bool {
    match order_type {
        OrderType::Market => true,
        OrderType::Limit { limit_price } => match side {
            OrderSide::Buy => price <= limit_price,
            OrderSide::Sell => price >= limit_price,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthSweepError {
    InvalidDepthLevels(usize),
    InvalidAdversePriceDelta,
    ConsumedQuantityExceedsDisplayed {
        level_index: u8,
    },
    QuantityUnitMismatch {
        expected: QuantityUnit,
        actual: QuantityUnit,
    },
}

impl fmt::Display for DepthSweepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDepthLevels(levels) => {
                write!(
                    formatter,
                    "depth levels must be between 1 and {BOOK_DEPTH}, got {levels}"
                )
            }
            Self::InvalidAdversePriceDelta => formatter
                .write_str("adverse price delta must be non-negative and keep prices positive"),
            Self::ConsumedQuantityExceedsDisplayed { level_index } => write!(
                formatter,
                "consumed quantity exceeds displayed quantity at level {level_index}"
            ),
            Self::QuantityUnitMismatch { expected, actual } => write!(
                formatter,
                "depth quantity unit mismatch: expected {expected:?}, got {actual:?}"
            ),
        }
    }
}

impl Error for DepthSweepError {}

#[cfg(test)]
mod tests {
    use market_types::{BookLevel, BookSide, BookSideKind, Price};

    use super::*;

    fn quantity(value: u64, unit: QuantityUnit) -> Quantity {
        Quantity::new(value, unit).unwrap()
    }

    fn level(price: &str, quantity_value: u64) -> BookLevel {
        BookLevel::new(
            Price::parse(price).unwrap(),
            quantity(quantity_value, QuantityUnit::Contract),
        )
    }

    fn book() -> CompleteBookSnapshot {
        CompleteBookSnapshot::new(
            BookSide::new(
                BookSideKind::Bid,
                vec![level("100", 2), level("99", 3), level("98", 5)],
            )
            .unwrap(),
            BookSide::new(
                BookSideKind::Ask,
                vec![level("101", 1), level("102", 4), level("103", 6)],
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn buy_sweeps_asks_from_best_price() {
        let result = sweep_visible_depth(
            &book(),
            OrderSide::Buy,
            quantity(5, QuantityUnit::Contract),
            5,
        )
        .unwrap();
        assert!(result.is_complete());
        assert_eq!(result.remaining(), None);
        assert_eq!(
            result
                .fills()
                .iter()
                .map(|fill| (fill.level_index(), fill.price(), fill.quantity().value()))
                .collect::<Vec<_>>(),
            [
                (1, Price::parse("101").unwrap(), 1),
                (2, Price::parse("102").unwrap(), 4),
            ]
        );
    }

    #[test]
    fn sell_sweeps_bids_and_reports_partial_depth() {
        let result = sweep_visible_depth(
            &book(),
            OrderSide::Sell,
            quantity(8, QuantityUnit::Contract),
            2,
        )
        .unwrap();
        assert!(!result.is_complete());
        assert_eq!(result.filled().unwrap().value(), 5);
        assert_eq!(result.remaining().unwrap().value(), 3);
        assert_eq!(result.fills()[0].price(), Price::parse("100").unwrap());
        assert_eq!(result.fills()[1].price(), Price::parse("99").unwrap());
    }

    #[test]
    fn invalid_depth_and_quantity_unit_are_rejected() {
        assert_eq!(
            sweep_visible_depth(
                &book(),
                OrderSide::Buy,
                quantity(1, QuantityUnit::Contract),
                0,
            )
            .unwrap_err(),
            DepthSweepError::InvalidDepthLevels(0)
        );
        assert_eq!(
            sweep_visible_depth(
                &book(),
                OrderSide::Buy,
                quantity(1, QuantityUnit::TradingUnit),
                5,
            )
            .unwrap_err(),
            DepthSweepError::QuantityUnitMismatch {
                expected: QuantityUnit::TradingUnit,
                actual: QuantityUnit::Contract,
            }
        );
    }

    #[test]
    fn limit_order_stops_before_non_marketable_level() {
        let result = sweep_marketable_depth(
            &book(),
            OrderSide::Buy,
            quantity(5, QuantityUnit::Contract),
            5,
            OrderType::Limit {
                limit_price: Price::parse("101").unwrap(),
            },
            Decimal::ZERO,
        )
        .unwrap();

        assert_eq!(result.filled().unwrap().value(), 1);
        assert_eq!(result.remaining().unwrap().value(), 4);
        assert_eq!(result.fills()[0].price(), Price::parse("101").unwrap());
    }

    #[test]
    fn slippage_is_applied_before_limit_check() {
        let result = sweep_marketable_depth(
            &book(),
            OrderSide::Sell,
            quantity(5, QuantityUnit::Contract),
            5,
            OrderType::Limit {
                limit_price: Price::parse("99.5").unwrap(),
            },
            Decimal::parse("0.5").unwrap(),
        )
        .unwrap();

        assert_eq!(result.filled().unwrap().value(), 2);
        assert_eq!(result.fills()[0].price(), Price::parse("99.5").unwrap());
    }

    #[test]
    fn negative_or_non_positive_slipped_price_is_rejected() {
        assert_eq!(
            sweep_marketable_depth(
                &book(),
                OrderSide::Buy,
                quantity(1, QuantityUnit::Contract),
                5,
                OrderType::Market,
                Decimal::parse("-0.1").unwrap(),
            )
            .unwrap_err(),
            DepthSweepError::InvalidAdversePriceDelta
        );
        assert_eq!(
            sweep_marketable_depth(
                &book(),
                OrderSide::Sell,
                quantity(1, QuantityUnit::Contract),
                5,
                OrderType::Market,
                Decimal::parse("100").unwrap(),
            )
            .unwrap_err(),
            DepthSweepError::InvalidAdversePriceDelta
        );
    }
}
