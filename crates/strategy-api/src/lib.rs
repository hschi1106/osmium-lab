mod context;
mod example;
mod identity;
mod m2_acceptance;
mod orders;
mod output;
mod runner;
mod strategy;

pub use context::{
    ContextError, IndicativeReason, MatchingState, NewOrderEntry, OrderBlockReason,
    OrderRestrictionReason, SessionCallbackContext, SessionKind, SessionPhase, SessionSegment,
    TradingContext, TwseTradingContextEvaluator,
};
pub use example::{EXAMPLE_STRATEGY_ID, EXAMPLE_STRATEGY_VERSION, ExampleStrategy};
pub use identity::{
    BinaryIdentity, CanonicalParamsChecksum, DeclarationError, StrategyDeclaration,
    StrategyIdentity,
};
pub use m2_acceptance::{
    M2_ACCEPTANCE_STRATEGY_ID, M2_ACCEPTANCE_STRATEGY_VERSION, M2AcceptanceStrategy,
};
pub use orders::{
    CancellationReason, ORDER_INTENT_VERSION, OrderFeedback, OrderId, OrderIntent,
    OrderIntentError, OrderSide, OrderType, RejectionReason, StrategyFeedbackContext, TimeInForce,
};
pub use output::{
    CANONICAL_STRATEGY_OUTPUT_VERSION, IndicatorValue, StrategyOutput, StrategyOutputChecksum,
    StrategyOutputEncodingError, StrategyOutputRecord, StrategyOutputSink,
};
pub use runner::{
    CompletedStrategyRun, FailedStrategyRun, StrategyRunError, StrategyRunErrorCategory,
    run_strategy,
};
pub use strategy::{
    CapabilityError, Strategy, StrategyEventContext, StrategyExecutionError,
    StrategyFinalizeContext, StrategyInitializationContext,
};

pub const STRATEGY_API_VERSION: u16 = 1;
