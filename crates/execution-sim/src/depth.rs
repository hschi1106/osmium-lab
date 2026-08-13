use std::{error::Error, fmt};

use std::collections::BTreeMap;

use market_types::{
    BOOK_DEPTH, CompleteBookSnapshot, Decimal, InstrumentId, Price, Quantity, QuantityUnit,
};
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

/// A visible five-level snapshot plus strategy-consumed displayed quantity.
///
/// Consumption belongs to the snapshot and must be discarded when a newer complete snapshot is
/// published. Keeping it here makes shared-liquidity allocation explicit without claiming exchange
/// queue reconstruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumableDepth {
    book: CompleteBookSnapshot,
    consumed_bids: [u64; BOOK_DEPTH],
    consumed_asks: [u64; BOOK_DEPTH],
}

impl ConsumableDepth {
    #[must_use]
    pub const fn new(book: CompleteBookSnapshot) -> Self {
        Self {
            book,
            consumed_bids: [0; BOOK_DEPTH],
            consumed_asks: [0; BOOK_DEPTH],
        }
    }

    #[must_use]
    pub const fn book(&self) -> &CompleteBookSnapshot {
        &self.book
    }

    pub fn replace(&mut self, book: CompleteBookSnapshot) {
        self.book = book;
        self.consumed_bids = [0; BOOK_DEPTH];
        self.consumed_asks = [0; BOOK_DEPTH];
    }

    pub fn preview(
        &self,
        side: OrderSide,
        requested: Quantity,
        depth_levels: usize,
    ) -> Result<DepthSweepResult, DepthSweepError> {
        let consumed = match side {
            OrderSide::Buy => &self.consumed_asks,
            OrderSide::Sell => &self.consumed_bids,
        };
        sweep_marketable_depth_with_consumed(
            &self.book,
            side,
            requested,
            depth_levels,
            OrderType::Market,
            Decimal::ZERO,
            consumed,
        )
    }

    fn apply(&mut self, side: OrderSide, sweep: &DepthSweepResult) -> Result<(), DepthSweepError> {
        let consumed = match side {
            OrderSide::Buy => &mut self.consumed_asks,
            OrderSide::Sell => &mut self.consumed_bids,
        };
        for fill in sweep.fills() {
            let index = usize::from(fill.level_index() - 1);
            consumed[index] = consumed[index]
                .checked_add(fill.quantity().value())
                .ok_or(DepthSweepError::QuantityOverflow)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicDepthLeg {
    instrument: InstrumentId,
    side: OrderSide,
}

impl AtomicDepthLeg {
    #[must_use]
    pub const fn new(instrument: InstrumentId, side: OrderSide) -> Self {
        Self { instrument, side }
    }

    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn side(&self) -> OrderSide {
        self.side
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicDepthFill {
    leg: AtomicDepthLeg,
    sweep: DepthSweepResult,
}

impl AtomicDepthFill {
    #[must_use]
    pub const fn leg(&self) -> &AtomicDepthLeg {
        &self.leg
    }

    #[must_use]
    pub const fn sweep(&self) -> &DepthSweepResult {
        &self.sweep
    }
}

/// Preflights every leg against cloned consumption and commits only when every leg can fill the
/// exact requested quantity. A `None` result leaves every book unchanged.
pub fn execute_atomic_depth(
    books: &mut BTreeMap<InstrumentId, ConsumableDepth>,
    legs: &[AtomicDepthLeg],
    requested: Quantity,
    depth_levels: usize,
) -> Result<Option<Vec<AtomicDepthFill>>, DepthSweepError> {
    let mut staged = BTreeMap::new();
    for leg in legs {
        let Some(book) = books.get(leg.instrument()) else {
            return Ok(None);
        };
        staged
            .entry(leg.instrument().clone())
            .or_insert_with(|| book.clone());
    }
    let mut fills = Vec::with_capacity(legs.len());
    for leg in legs {
        let Some(book) = staged.get_mut(leg.instrument()) else {
            return Ok(None);
        };
        let sweep = book.preview(leg.side(), requested, depth_levels)?;
        if !sweep.is_complete() {
            return Ok(None);
        }
        book.apply(leg.side(), &sweep)?;
        fills.push(AtomicDepthFill {
            leg: leg.clone(),
            sweep,
        });
    }
    for (instrument, book) in staged {
        books.insert(instrument, book);
    }
    Ok(Some(fills))
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
    QuantityOverflow,
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
            Self::QuantityOverflow => formatter.write_str("consumed depth quantity overflow"),
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

    #[test]
    fn atomic_depth_commits_all_three_legs_or_none() {
        let future = InstrumentId::new(
            market_types::MarketId::Taifex,
            market_types::Symbol::new("MX2G6").unwrap(),
        );
        let call = InstrumentId::new(
            market_types::MarketId::Taifex,
            market_types::Symbol::new("CALL").unwrap(),
        );
        let put = InstrumentId::new(
            market_types::MarketId::Taifex,
            market_types::Symbol::new("PUT").unwrap(),
        );
        let mut books = [future.clone(), call.clone(), put.clone()]
            .into_iter()
            .map(|instrument| (instrument, ConsumableDepth::new(book())))
            .collect::<BTreeMap<_, _>>();
        let legs = [
            AtomicDepthLeg::new(future.clone(), OrderSide::Sell),
            AtomicDepthLeg::new(call, OrderSide::Buy),
            AtomicDepthLeg::new(put.clone(), OrderSide::Sell),
        ];

        let before = books.clone();
        assert!(
            execute_atomic_depth(&mut books, &legs, quantity(6, QuantityUnit::Contract), 2,)
                .unwrap()
                .is_none()
        );
        assert_eq!(books, before);

        let fills = execute_atomic_depth(&mut books, &legs, quantity(2, QuantityUnit::Contract), 5)
            .unwrap()
            .unwrap();
        assert_eq!(fills.len(), 3);
        assert!(fills.iter().all(|fill| fill.sweep().is_complete()));
        let remaining = books
            .get(&future)
            .unwrap()
            .preview(OrderSide::Sell, quantity(4, QuantityUnit::Contract), 5)
            .unwrap();
        assert_eq!(remaining.filled().unwrap().value(), 4);
        assert_eq!(remaining.fills()[0].price(), Price::parse("99").unwrap());
    }
}
