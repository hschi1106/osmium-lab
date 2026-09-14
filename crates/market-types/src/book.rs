use std::{error::Error, fmt};

use crate::{Price, Quantity, QuantityUnit};

pub const BOOK_DEPTH: usize = 5;

/// One displayed price level in a complete book snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BookLevel {
    price: Price,
    displayed_quantity: Quantity,
}

impl BookLevel {
    #[must_use]
    pub const fn new(price: Price, displayed_quantity: Quantity) -> Self {
        Self {
            price,
            displayed_quantity,
        }
    }

    #[must_use]
    pub const fn price(self) -> Price {
        self.price
    }

    #[must_use]
    pub const fn displayed_quantity(self) -> Quantity {
        self.displayed_quantity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookSideKind {
    Bid,
    Ask,
}

/// One complete displayed side of a book.
///
/// Some exchanges publish aggregate market-order quantity as the first of the
/// five displayed entries with a zero wire price. It is kept separately from
/// priced levels so that zero never becomes executable price evidence.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BookSide {
    kind: BookSideKind,
    market_order_quantity: Option<Quantity>,
    slots: [Option<BookLevel>; BOOK_DEPTH],
}

impl BookSide {
    pub fn new(kind: BookSideKind, levels: Vec<BookLevel>) -> Result<Self, BookError> {
        Self::with_market_order_quantity(kind, None, levels)
    }

    pub fn with_market_order_quantity(
        kind: BookSideKind,
        market_order_quantity: Option<Quantity>,
        levels: Vec<BookLevel>,
    ) -> Result<Self, BookError> {
        if levels.len() > BOOK_DEPTH {
            return Err(BookError::TooManyLevels {
                side: kind,
                count: levels.len(),
            });
        }

        let mut slots = [None; BOOK_DEPTH];
        for (index, level) in levels.into_iter().enumerate() {
            slots[index] = Some(level);
        }
        Self::from_parts(kind, market_order_quantity, slots)
    }

    pub fn from_slots(
        kind: BookSideKind,
        slots: [Option<BookLevel>; BOOK_DEPTH],
    ) -> Result<Self, BookError> {
        Self::from_parts(kind, None, slots)
    }

    pub fn from_parts(
        kind: BookSideKind,
        market_order_quantity: Option<Quantity>,
        slots: [Option<BookLevel>; BOOK_DEPTH],
    ) -> Result<Self, BookError> {
        let side = Self {
            kind,
            market_order_quantity,
            slots,
        };
        side.validate()?;
        Ok(side)
    }

    #[must_use]
    pub const fn kind(&self) -> BookSideKind {
        self.kind
    }

    /// Aggregate displayed quantity for unpriced market orders, when present.
    #[must_use]
    pub const fn market_order_quantity(&self) -> Option<Quantity> {
        self.market_order_quantity
    }

    #[must_use]
    pub const fn slots(&self) -> &[Option<BookLevel>; BOOK_DEPTH] {
        &self.slots
    }

    pub fn levels(&self) -> impl Iterator<Item = &BookLevel> {
        self.slots.iter().map_while(Option::as_ref)
    }

    #[must_use]
    pub fn quantity_unit(&self) -> Option<QuantityUnit> {
        self.market_order_quantity.map(Quantity::unit).or_else(|| {
            self.levels()
                .next()
                .map(|level| level.displayed_quantity().unit())
        })
    }

    fn validate(&self) -> Result<(), BookError> {
        let mut empty_seen = false;
        let mut previous: Option<BookLevel> = None;
        let mut unit = self.market_order_quantity.map(Quantity::unit);
        let priced_level_count = self.levels().count();
        let displayed_entry_count =
            priced_level_count + usize::from(self.market_order_quantity.is_some());
        if displayed_entry_count > BOOK_DEPTH {
            return Err(BookError::TooManyLevels {
                side: self.kind,
                count: displayed_entry_count,
            });
        }

        for (index, slot) in self.slots.iter().enumerate() {
            let Some(level) = slot else {
                empty_seen = true;
                continue;
            };
            if empty_seen {
                return Err(BookError::NonContiguous {
                    side: self.kind,
                    index,
                });
            }

            let level_unit = level.displayed_quantity().unit();
            if let Some(expected) = unit {
                if expected != level_unit {
                    return Err(BookError::UnitMismatch {
                        side: self.kind,
                        expected,
                        actual: level_unit,
                        index,
                    });
                }
            } else {
                unit = Some(level_unit);
            }

            if let Some(previous) = previous {
                let correctly_ordered = match self.kind {
                    BookSideKind::Bid => previous.price() > level.price(),
                    BookSideKind::Ask => previous.price() < level.price(),
                };
                if !correctly_ordered {
                    return Err(BookError::PriceOrder {
                        side: self.kind,
                        index,
                    });
                }
            }
            previous = Some(*level);
        }
        Ok(())
    }
}

/// A complete replacement of both five-level book sides.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompleteBookSnapshot {
    bids: BookSide,
    asks: BookSide,
}

impl CompleteBookSnapshot {
    pub fn new(bids: BookSide, asks: BookSide) -> Result<Self, BookError> {
        if bids.kind() != BookSideKind::Bid {
            return Err(BookError::WrongSide {
                expected: BookSideKind::Bid,
                actual: bids.kind(),
            });
        }
        if asks.kind() != BookSideKind::Ask {
            return Err(BookError::WrongSide {
                expected: BookSideKind::Ask,
                actual: asks.kind(),
            });
        }
        if let (Some(bid_unit), Some(ask_unit)) = (bids.quantity_unit(), asks.quantity_unit())
            && bid_unit != ask_unit
        {
            return Err(BookError::CrossSideUnitMismatch {
                bid: bid_unit,
                ask: ask_unit,
            });
        }
        Ok(Self { bids, asks })
    }

    #[must_use]
    pub const fn bids(&self) -> &BookSide {
        &self.bids
    }

    #[must_use]
    pub const fn asks(&self) -> &BookSide {
        &self.asks
    }

    #[must_use]
    pub fn quantity_unit(&self) -> Option<QuantityUnit> {
        self.bids
            .quantity_unit()
            .or_else(|| self.asks.quantity_unit())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookError {
    TooManyLevels {
        side: BookSideKind,
        count: usize,
    },
    NonContiguous {
        side: BookSideKind,
        index: usize,
    },
    PriceOrder {
        side: BookSideKind,
        index: usize,
    },
    UnitMismatch {
        side: BookSideKind,
        expected: QuantityUnit,
        actual: QuantityUnit,
        index: usize,
    },
    WrongSide {
        expected: BookSideKind,
        actual: BookSideKind,
    },
    CrossSideUnitMismatch {
        bid: QuantityUnit,
        ask: QuantityUnit,
    },
}

impl fmt::Display for BookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyLevels { side, count } => {
                write!(
                    formatter,
                    "{side:?} book has {count} levels; maximum is {BOOK_DEPTH}"
                )
            }
            Self::NonContiguous { side, index } => {
                write!(
                    formatter,
                    "{side:?} book has a level after an empty slot at index {index}"
                )
            }
            Self::PriceOrder { side, index } => {
                write!(
                    formatter,
                    "{side:?} book price order is invalid at index {index}"
                )
            }
            Self::UnitMismatch {
                side,
                expected,
                actual,
                index,
            } => write!(
                formatter,
                "{side:?} book quantity unit mismatch at index {index}: {expected:?} != {actual:?}"
            ),
            Self::WrongSide { expected, actual } => {
                write!(
                    formatter,
                    "wrong book side: expected {expected:?}, got {actual:?}"
                )
            }
            Self::CrossSideUnitMismatch { bid, ask } => {
                write!(
                    formatter,
                    "book side quantity unit mismatch: {bid:?} != {ask:?}"
                )
            }
        }
    }
}

impl Error for BookError {}
