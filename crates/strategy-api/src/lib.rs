mod context;
mod example;
mod identity;
mod output;
mod runner;
mod strategy;

pub use context::{
    ContextError, IndicativeReason, MatchingState, NewOrderEntry, OrderBlockReason,
    OrderRestrictionReason, SessionCallbackContext, SessionKind, SessionPhase, SessionSegment,
    TradingContext, TwseTradingContextEvaluator,
};
pub use example::ExampleStrategy;
pub use identity::{
    BinaryIdentity, CanonicalParamsChecksum, DeclarationError, StrategyDeclaration,
    StrategyIdentity,
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
