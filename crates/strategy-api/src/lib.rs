mod acceptance;
mod context;
mod example;
mod identity;
mod orders;
mod output;
mod registry;
mod runner;
mod strategy;

pub use acceptance::{
    ACCEPTANCE_STRATEGY_ID, ACCEPTANCE_STRATEGY_VERSION, AcceptanceStrategy,
    AcceptanceStrategyFactory,
};
pub use context::{
    ContextError, IndicativeReason, MarketTradingContextEvaluator, MatchingState, NewOrderEntry,
    OrderBlockReason, OrderRestrictionReason, SessionCallbackContext, SessionKind, SessionPhase,
    SessionSegment, TpexTradingContextEvaluator, TradingContext, TwseTradingContextEvaluator,
};
pub use example::{EXAMPLE_STRATEGY_ID, EXAMPLE_STRATEGY_VERSION, ExampleStrategy};
pub use identity::{
    BinaryIdentity, CanonicalParamsChecksum, DeclarationError, StrategyDeclaration,
    StrategyIdentity,
};
pub use market_state::SessionSegmentId;
pub use orders::{
    CancellationReason, ORDER_INTENT_VERSION, OrderFeedback, OrderId, OrderIntent,
    OrderIntentError, OrderSide, OrderType, RejectionReason, StrategyFeedbackContext, TimeInForce,
};
pub use output::{
    CANONICAL_STRATEGY_OUTPUT_VERSION, IndicatorValue, StrategyOutput, StrategyOutputChecksum,
    StrategyOutputEncodingError, StrategyOutputRecord, StrategyOutputSink,
};
pub use registry::{
    FactoryContractField, ParameterRange, RangeBound, RawStrategyParameter, RawStrategyParameters,
    ResolvedStrategy, ResolvedStrategyMetadata, STRATEGY_PARAMETER_CANONICAL_VERSION,
    StrategyDefinition, StrategyFactory, StrategyFactoryError, StrategyParameterField,
    StrategyParameterSchema, StrategyParameterType, StrategyParameterValue, StrategyRegistry,
    StrategyRegistryError, ValidatedStrategyParameters,
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
