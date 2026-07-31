use std::{error::Error, fmt};

use market_types::{CanonicalEncodingError, InstrumentId, Price, Quantity, append_bytes};

pub const ORDER_INTENT_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OrderSide {
    Buy = 1,
    Sell = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    Market,
    Limit { limit_price: Price },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TimeInForce {
    Day = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderIntent {
    instrument: InstrumentId,
    side: OrderSide,
    quantity: Quantity,
    order_type: OrderType,
}

impl OrderIntent {
    #[must_use]
    pub const fn new(
        instrument: InstrumentId,
        side: OrderSide,
        quantity: Quantity,
        order_type: OrderType,
    ) -> Self {
        Self {
            instrument,
            side,
            quantity,
            order_type,
        }
    }

    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    #[must_use]
    pub const fn side(&self) -> OrderSide {
        self.side
    }
    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }
    #[must_use]
    pub const fn order_type(&self) -> OrderType {
        self.order_type
    }
    #[must_use]
    pub const fn time_in_force(&self) -> TimeInForce {
        TimeInForce::Day
    }

    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, CanonicalEncodingError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"OSOI");
        bytes.extend_from_slice(&ORDER_INTENT_VERSION.to_be_bytes());
        bytes.push(self.instrument.market().discriminant());
        append_bytes(self.instrument.symbol().as_bytes(), &mut bytes)?;
        bytes.push(self.side as u8);
        bytes.extend_from_slice(&self.quantity.to_canonical_bytes());
        match self.order_type {
            OrderType::Market => bytes.push(1),
            OrderType::Limit { limit_price } => {
                bytes.push(2);
                bytes.extend_from_slice(&limit_price.to_canonical_bytes());
            }
        }
        bytes.push(TimeInForce::Day as u8);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderId([u8; 32]);

impl OrderId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    InstrumentOutsideUniverse,
    QuantityUnitMismatch,
    NewOrderEntryBlocked,
    UnsupportedOrderType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationReason {
    EndOfRun,
    SegmentEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderFeedback {
    Accepted {
        order_id: OrderId,
    },
    Rejected {
        reason: RejectionReason,
    },
    PartiallyFilled {
        order_id: OrderId,
        filled: Quantity,
        remaining: Quantity,
    },
    Filled {
        order_id: OrderId,
        filled: Quantity,
    },
    Cancelled {
        order_id: OrderId,
        reason: CancellationReason,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct StrategyFeedbackContext<'a>(&'a [OrderFeedback]);

impl<'a> StrategyFeedbackContext<'a> {
    #[must_use]
    pub const fn new(feedback: &'a [OrderFeedback]) -> Self {
        Self(feedback)
    }
    #[must_use]
    pub const fn feedback(self) -> &'a [OrderFeedback] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderIntentError;

impl fmt::Display for OrderIntentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("order intent capability is unavailable in this execution mode")
    }
}

impl Error for OrderIntentError {}

#[cfg(test)]
mod tests {
    use market_types::{MarketId, QuantityUnit, Symbol};

    use super::*;
    use crate::StrategyOutputSink;

    fn intent() -> OrderIntent {
        OrderIntent::new(
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            OrderSide::Buy,
            Quantity::new(2, QuantityUnit::TradingUnit).unwrap(),
            OrderType::Limit {
                limit_price: Price::parse("100").unwrap(),
            },
        )
    }

    #[test]
    fn intent_encoding_is_deterministic() {
        assert_eq!(
            intent().to_canonical_bytes().unwrap(),
            intent().to_canonical_bytes().unwrap()
        );
    }

    #[test]
    fn execution_mode_controls_order_capability() {
        assert!(
            StrategyOutputSink::new()
                .emit_order_intent(intent())
                .is_err()
        );
        let mut sink = StrategyOutputSink::with_order_intents();
        sink.emit_order_intent(intent()).unwrap();
        assert_eq!(sink.intents(), &[intent()]);
    }
}
