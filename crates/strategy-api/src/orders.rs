use std::{error::Error, fmt};

use market_types::{
    CanonicalEncodingError, InstrumentId, MatchTime, Price, Quantity, append_bytes,
};

pub const ORDER_INTENT_VERSION: u16 = 1;
pub const SCHEDULED_ORDER_REQUEST_VERSION: u16 = 1;
pub const EXECUTION_FILL_FEEDBACK_VERSION: u16 = 1;

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientOrderId(Box<str>);

impl ClientOrderId {
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, OrderCorrelationIdError> {
        let value = value.into();
        validate_correlation_id(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderBatchId(Box<str>);

impl OrderBatchId {
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, OrderCorrelationIdError> {
        let value = value.into();
        validate_correlation_id(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const MAX_CORRELATION_ID_BYTES: usize = 128;

fn validate_correlation_id(value: &str) -> Result<(), OrderCorrelationIdError> {
    if value.is_empty() {
        return Err(OrderCorrelationIdError::Empty);
    }
    if value.len() > MAX_CORRELATION_ID_BYTES {
        return Err(OrderCorrelationIdError::TooLong);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderCorrelationIdError {
    Empty,
    TooLong,
}

impl fmt::Display for OrderCorrelationIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "order correlation identifier must not be empty",
            Self::TooLong => "order correlation identifier exceeds 128 UTF-8 bytes",
        })
    }
}

impl Error for OrderCorrelationIdError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScheduledExecutionPolicy {
    VisibleDepthAtActivationV1 = 1,
    VisibleDepthUntilExpiryV1 = 2,
    AuctionCrossAtFirstMatchV1 = 3,
    /// Cash-settles a derivative at the exact limit price carried by the order intent.
    ///
    /// The price must come from immutable reference data known to the backtest plan. This policy
    /// deliberately does not consult replay market depth.
    SettlementAtActivationV1 = 4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledOrderRequest {
    client_order_id: ClientOrderId,
    batch_id: Option<OrderBatchId>,
    activate_at: MatchTime,
    expire_at: Option<MatchTime>,
    intent: OrderIntent,
    execution_policy: ScheduledExecutionPolicy,
}

impl ScheduledOrderRequest {
    pub fn new(
        client_order_id: ClientOrderId,
        batch_id: Option<OrderBatchId>,
        activate_at: MatchTime,
        expire_at: Option<MatchTime>,
        intent: OrderIntent,
        execution_policy: ScheduledExecutionPolicy,
    ) -> Result<Self, ScheduledOrderRequestError> {
        if expire_at.is_some_and(|expire_at| expire_at <= activate_at) {
            return Err(ScheduledOrderRequestError::InvalidActiveWindow);
        }
        Ok(Self {
            client_order_id,
            batch_id,
            activate_at,
            expire_at,
            intent,
            execution_policy,
        })
    }

    #[must_use]
    pub const fn client_order_id(&self) -> &ClientOrderId {
        &self.client_order_id
    }

    #[must_use]
    pub const fn batch_id(&self) -> Option<&OrderBatchId> {
        self.batch_id.as_ref()
    }

    #[must_use]
    pub const fn activate_at(&self) -> MatchTime {
        self.activate_at
    }

    #[must_use]
    pub const fn expire_at(&self) -> Option<MatchTime> {
        self.expire_at
    }

    #[must_use]
    pub const fn intent(&self) -> &OrderIntent {
        &self.intent
    }

    #[must_use]
    pub const fn execution_policy(&self) -> ScheduledExecutionPolicy {
        self.execution_policy
    }

    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, CanonicalEncodingError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"OSSR");
        bytes.extend_from_slice(&SCHEDULED_ORDER_REQUEST_VERSION.to_be_bytes());
        append_bytes(self.client_order_id.as_str().as_bytes(), &mut bytes)?;
        match &self.batch_id {
            None => bytes.push(0),
            Some(batch_id) => {
                bytes.push(1);
                append_bytes(batch_id.as_str().as_bytes(), &mut bytes)?;
            }
        }
        bytes.extend_from_slice(&self.activate_at.as_unix_microseconds().to_be_bytes());
        match self.expire_at {
            None => bytes.push(0),
            Some(expire_at) => {
                bytes.push(1);
                bytes.extend_from_slice(&expire_at.as_unix_microseconds().to_be_bytes());
            }
        }
        bytes.push(self.execution_policy as u8);
        append_bytes(&self.intent.to_canonical_bytes()?, &mut bytes)?;
        Ok(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledOrderRequestError {
    InvalidActiveWindow,
}

impl fmt::Display for ScheduledOrderRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("scheduled order expiry must be later than activation")
    }
}

impl Error for ScheduledOrderRequestError {}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FillId([u8; 32]);

impl FillId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionFillFeedback {
    fill_id: FillId,
    order_id: OrderId,
    client_order_id: ClientOrderId,
    batch_id: Option<OrderBatchId>,
    instrument: InstrumentId,
    activation_time: MatchTime,
    fill_time: MatchTime,
    side: OrderSide,
    level_index: u8,
    price: Price,
    quantity: Quantity,
    cumulative_filled: Quantity,
    remaining: Option<Quantity>,
}

impl ExecutionFillFeedback {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fill_id: FillId,
        order_id: OrderId,
        client_order_id: ClientOrderId,
        batch_id: Option<OrderBatchId>,
        instrument: InstrumentId,
        activation_time: MatchTime,
        fill_time: MatchTime,
        side: OrderSide,
        level_index: u8,
        price: Price,
        quantity: Quantity,
        cumulative_filled: Quantity,
        remaining: Option<Quantity>,
    ) -> Result<Self, ExecutionFillFeedbackError> {
        if !(1..=5).contains(&level_index) {
            return Err(ExecutionFillFeedbackError::InvalidLevelIndex);
        }
        if fill_time < activation_time {
            return Err(ExecutionFillFeedbackError::FillBeforeActivation);
        }
        if quantity.unit() != cumulative_filled.unit()
            || remaining.is_some_and(|remaining| remaining.unit() != quantity.unit())
        {
            return Err(ExecutionFillFeedbackError::QuantityUnitMismatch);
        }
        if cumulative_filled.value() < quantity.value() {
            return Err(ExecutionFillFeedbackError::InvalidCumulativeQuantity);
        }
        Ok(Self {
            fill_id,
            order_id,
            client_order_id,
            batch_id,
            instrument,
            activation_time,
            fill_time,
            side,
            level_index,
            price,
            quantity,
            cumulative_filled,
            remaining,
        })
    }

    #[must_use]
    pub const fn fill_id(&self) -> FillId {
        self.fill_id
    }

    #[must_use]
    pub const fn order_id(&self) -> OrderId {
        self.order_id
    }

    #[must_use]
    pub const fn client_order_id(&self) -> &ClientOrderId {
        &self.client_order_id
    }

    #[must_use]
    pub const fn batch_id(&self) -> Option<&OrderBatchId> {
        self.batch_id.as_ref()
    }

    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn activation_time(&self) -> MatchTime {
        self.activation_time
    }

    #[must_use]
    pub const fn fill_time(&self) -> MatchTime {
        self.fill_time
    }

    #[must_use]
    pub const fn side(&self) -> OrderSide {
        self.side
    }

    #[must_use]
    pub const fn level_index(&self) -> u8 {
        self.level_index
    }

    #[must_use]
    pub const fn price(&self) -> Price {
        self.price
    }

    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }

    #[must_use]
    pub const fn cumulative_filled(&self) -> Quantity {
        self.cumulative_filled
    }

    #[must_use]
    pub const fn remaining(&self) -> Option<Quantity> {
        self.remaining
    }

    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, CanonicalEncodingError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"OSEF");
        bytes.extend_from_slice(&EXECUTION_FILL_FEEDBACK_VERSION.to_be_bytes());
        bytes.extend_from_slice(self.fill_id.as_bytes());
        bytes.extend_from_slice(self.order_id.as_bytes());
        append_bytes(self.client_order_id.as_str().as_bytes(), &mut bytes)?;
        match &self.batch_id {
            None => bytes.push(0),
            Some(batch_id) => {
                bytes.push(1);
                append_bytes(batch_id.as_str().as_bytes(), &mut bytes)?;
            }
        }
        bytes.push(self.instrument.market().discriminant());
        append_bytes(self.instrument.symbol().as_bytes(), &mut bytes)?;
        bytes.extend_from_slice(&self.activation_time.as_unix_microseconds().to_be_bytes());
        bytes.extend_from_slice(&self.fill_time.as_unix_microseconds().to_be_bytes());
        bytes.push(self.side as u8);
        bytes.push(self.level_index);
        bytes.extend_from_slice(&self.price.to_canonical_bytes());
        bytes.extend_from_slice(&self.quantity.to_canonical_bytes());
        bytes.extend_from_slice(&self.cumulative_filled.to_canonical_bytes());
        match self.remaining {
            None => bytes.push(0),
            Some(remaining) => {
                bytes.push(1);
                bytes.extend_from_slice(&remaining.to_canonical_bytes());
            }
        }
        Ok(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionFillFeedbackError {
    InvalidLevelIndex,
    FillBeforeActivation,
    QuantityUnitMismatch,
    InvalidCumulativeQuantity,
}

impl fmt::Display for ExecutionFillFeedbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLevelIndex => "execution fill level index must be between 1 and 5",
            Self::FillBeforeActivation => "execution fill time is earlier than order activation",
            Self::QuantityUnitMismatch => "execution fill quantities use different units",
            Self::InvalidCumulativeQuantity => {
                "execution cumulative fill is smaller than the current level fill"
            }
        })
    }
}

impl Error for ExecutionFillFeedbackError {}

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
    Expired,
    Replaced,
    StrategyCancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionFailureReason {
    MissingVisibleDepth,
    StaleVisibleDepth,
    MatchingDisabled,
    NewOrderEntryBlocked,
    InsufficientVisibleDepth,
    PriceNotMarketable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderFeedback {
    Accepted {
        order_id: OrderId,
    },
    Activated {
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
    ExecutionFailed {
        order_id: OrderId,
        reason: ExecutionFailureReason,
        filled: Option<Quantity>,
        remaining: Quantity,
    },
    MatchAttempted {
        order_id: OrderId,
        filled: Option<Quantity>,
        remaining: Quantity,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct StrategyFeedbackContext<'a> {
    feedback: &'a [OrderFeedback],
    execution_fills: &'a [ExecutionFillFeedback],
}

impl<'a> StrategyFeedbackContext<'a> {
    #[must_use]
    pub const fn new(feedback: &'a [OrderFeedback]) -> Self {
        Self {
            feedback,
            execution_fills: &[],
        }
    }

    #[must_use]
    pub const fn new_with_execution_fills(
        feedback: &'a [OrderFeedback],
        execution_fills: &'a [ExecutionFillFeedback],
    ) -> Self {
        Self {
            feedback,
            execution_fills,
        }
    }

    #[must_use]
    pub const fn feedback(self) -> &'a [OrderFeedback] {
        self.feedback
    }

    #[must_use]
    pub const fn execution_fills(self) -> &'a [ExecutionFillFeedback] {
        self.execution_fills
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledOrderCapabilityError;

impl fmt::Display for ScheduledOrderCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("scheduled order capability is unavailable in this execution mode")
    }
}

impl Error for ScheduledOrderCapabilityError {}

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

    fn scheduled(expire_at: Option<MatchTime>) -> ScheduledOrderRequest {
        ScheduledOrderRequest::new(
            ClientOrderId::new("entry-2330-1").unwrap(),
            Some(OrderBatchId::new("entry-1").unwrap()),
            MatchTime::parse("2026-07-27T09:00:00.300+08:00").unwrap(),
            expire_at,
            intent(),
            ScheduledExecutionPolicy::VisibleDepthAtActivationV1,
        )
        .unwrap()
    }

    #[test]
    fn scheduled_request_encoding_is_deterministic_and_time_sensitive() {
        let expiry = MatchTime::parse("2026-07-27T09:00:01+08:00").unwrap();
        let request = scheduled(Some(expiry));
        assert_eq!(
            request.to_canonical_bytes().unwrap(),
            request.to_canonical_bytes().unwrap()
        );
        assert_ne!(
            request.to_canonical_bytes().unwrap(),
            scheduled(None).to_canonical_bytes().unwrap()
        );
    }

    #[test]
    fn scheduled_request_rejects_empty_ids_and_non_positive_window() {
        assert_eq!(
            ClientOrderId::new("").unwrap_err(),
            OrderCorrelationIdError::Empty
        );
        assert_eq!(
            OrderBatchId::new("x".repeat(129)).unwrap_err(),
            OrderCorrelationIdError::TooLong
        );
        let activation = MatchTime::parse("2026-07-27T09:00:00.300+08:00").unwrap();
        assert_eq!(
            ScheduledOrderRequest::new(
                ClientOrderId::new("entry-2330-1").unwrap(),
                None,
                activation,
                Some(activation),
                intent(),
                ScheduledExecutionPolicy::VisibleDepthAtActivationV1,
            )
            .unwrap_err(),
            ScheduledOrderRequestError::InvalidActiveWindow
        );
    }

    #[test]
    fn execution_fill_feedback_is_validated_and_canonical() {
        let activation = MatchTime::parse("2026-07-27T09:00:00.300+08:00").unwrap();
        let fill_time = MatchTime::parse("2026-07-27T09:00:00.301+08:00").unwrap();
        let feedback = ExecutionFillFeedback::new(
            FillId::from_bytes([1; 32]),
            OrderId::from_bytes([2; 32]),
            ClientOrderId::new("entry-2330-1").unwrap(),
            Some(OrderBatchId::new("entry-1").unwrap()),
            intent().instrument().clone(),
            activation,
            fill_time,
            OrderSide::Buy,
            2,
            Price::parse("100").unwrap(),
            Quantity::new(2, QuantityUnit::TradingUnit).unwrap(),
            Quantity::new(3, QuantityUnit::TradingUnit).unwrap(),
            Some(Quantity::new(1, QuantityUnit::TradingUnit).unwrap()),
        )
        .unwrap();
        assert_eq!(
            feedback.to_canonical_bytes().unwrap(),
            feedback.to_canonical_bytes().unwrap()
        );
        assert_eq!(feedback.level_index(), 2);
        assert_eq!(feedback.remaining().unwrap().value(), 1);

        assert_eq!(
            ExecutionFillFeedback::new(
                FillId::from_bytes([1; 32]),
                OrderId::from_bytes([2; 32]),
                ClientOrderId::new("entry-2330-1").unwrap(),
                None,
                intent().instrument().clone(),
                fill_time,
                activation,
                OrderSide::Buy,
                1,
                Price::parse("100").unwrap(),
                Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
                Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
                None,
            )
            .unwrap_err(),
            ExecutionFillFeedbackError::FillBeforeActivation
        );
    }

    #[test]
    fn feedback_context_preserves_legacy_and_execution_channels() {
        let legacy = [OrderFeedback::Accepted {
            order_id: OrderId::from_bytes([3; 32]),
        }];
        let context = StrategyFeedbackContext::new(&legacy);
        assert_eq!(context.feedback(), &legacy);
        assert!(context.execution_fills().is_empty());
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

        assert!(
            StrategyOutputSink::new()
                .emit_scheduled_order(scheduled(None))
                .is_err()
        );
        let mut scheduled_sink = StrategyOutputSink::with_scheduled_orders();
        scheduled_sink
            .emit_scheduled_order(scheduled(None))
            .unwrap();
        assert_eq!(scheduled_sink.scheduled_orders(), &[scheduled(None)]);
    }
}
