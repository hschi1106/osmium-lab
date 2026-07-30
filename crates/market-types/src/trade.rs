use std::{error::Error, fmt};

use crate::{Price, Quantity, QuantityUnit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TradePrintKind {
    Regular = 0,
    Intermediate = 1,
}

impl TradePrintKind {
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
    }
}

/// One source-observed trade print without inferred aggressor or order identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TradePrint {
    price: Price,
    quantity: Quantity,
    print_kind: TradePrintKind,
}

impl TradePrint {
    #[must_use]
    pub const fn new(price: Price, quantity: Quantity, print_kind: TradePrintKind) -> Self {
        Self {
            price,
            quantity,
            print_kind,
        }
    }

    #[must_use]
    pub const fn price(self) -> Price {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> Quantity {
        self.quantity
    }

    #[must_use]
    pub const fn print_kind(self) -> TradePrintKind {
        self.print_kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TradeOrder {
    Unspecified = 0,
    SourceOrdered = 1,
}

impl TradeOrder {
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
    }
}

pub(crate) fn validate_trade_units(trades: &[TradePrint]) -> Result<QuantityUnit, TradeError> {
    let first = trades.first().ok_or(TradeError::Empty)?.quantity().unit();
    for (index, trade) in trades.iter().enumerate().skip(1) {
        let actual = trade.quantity().unit();
        if actual != first {
            return Err(TradeError::UnitMismatch {
                expected: first,
                actual,
                index,
            });
        }
    }
    Ok(first)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeError {
    Empty,
    UnitMismatch {
        expected: QuantityUnit,
        actual: QuantityUnit,
        index: usize,
    },
}

impl fmt::Display for TradeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("trade batch must contain at least one trade"),
            Self::UnitMismatch {
                expected,
                actual,
                index,
            } => write!(
                formatter,
                "trade quantity unit mismatch at index {index}: {expected:?} != {actual:?}"
            ),
        }
    }
}

impl Error for TradeError {}
