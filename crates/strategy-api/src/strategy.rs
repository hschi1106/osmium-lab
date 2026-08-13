use std::{error::Error, fmt};

use market_state::MarketStateView;
use market_types::{DomainEvent, MatchTime};
use replay_engine::{EventOccurrence, ReplayClock};

use crate::{
    CanonicalParamsChecksum, SessionCallbackContext, StrategyDeclaration, StrategyFeedbackContext,
    StrategyIdentity, StrategyOutputEncodingError, StrategyOutputSink, StrategyTimerContext,
    TradingContext,
};

pub struct StrategyInitializationContext<'a> {
    declaration: &'a StrategyDeclaration,
}

impl<'a> StrategyInitializationContext<'a> {
    pub const fn new(declaration: &'a StrategyDeclaration) -> Self {
        Self { declaration }
    }

    #[must_use]
    pub const fn declaration(&self) -> &'a StrategyDeclaration {
        self.declaration
    }
}

/// The callback deliberately exposes neither mutable market state nor a next-event API.
///
/// ```compile_fail
/// use market_state::MarketState;
/// use strategy_api::StrategyEventContext;
///
/// fn mutate(context: StrategyEventContext<'_>) {
///     let _: &mut MarketState = context.market_state();
/// }
/// ```
///
/// ```compile_fail
/// use strategy_api::StrategyEventContext;
///
/// fn look_ahead(context: StrategyEventContext<'_>) {
///     let _ = context.next_event();
/// }
/// ```
#[derive(Clone, Copy)]
pub struct StrategyEventContext<'event> {
    occurrence: &'event EventOccurrence,
    event: &'event DomainEvent,
    market_state: MarketStateView<'event>,
    market_states: &'event [MarketStateView<'event>],
    trading: &'event TradingContext,
    session: &'event SessionCallbackContext,
    decision_time: MatchTime,
}

impl<'event> StrategyEventContext<'event> {
    pub const fn new(
        occurrence: &'event EventOccurrence,
        event: &'event DomainEvent,
        market_state: MarketStateView<'event>,
        trading: &'event TradingContext,
    ) -> Self {
        Self::new_with_states(occurrence, event, market_state, &[], trading)
    }

    pub const fn new_with_states(
        occurrence: &'event EventOccurrence,
        event: &'event DomainEvent,
        market_state: MarketStateView<'event>,
        market_states: &'event [MarketStateView<'event>],
        trading: &'event TradingContext,
    ) -> Self {
        Self::new_visible_with_states(
            occurrence,
            event,
            market_state,
            market_states,
            trading,
            event.match_time(),
        )
    }

    pub const fn new_visible_with_states(
        occurrence: &'event EventOccurrence,
        event: &'event DomainEvent,
        market_state: MarketStateView<'event>,
        market_states: &'event [MarketStateView<'event>],
        trading: &'event TradingContext,
        decision_time: MatchTime,
    ) -> Self {
        Self {
            occurrence,
            event,
            market_state,
            market_states,
            trading,
            session: trading.session(),
            decision_time,
        }
    }

    #[must_use]
    pub const fn occurrence(self) -> &'event EventOccurrence {
        self.occurrence
    }

    #[must_use]
    pub const fn event(self) -> &'event DomainEvent {
        self.event
    }

    #[must_use]
    pub const fn market_state(self) -> MarketStateView<'event> {
        self.market_state
    }

    /// Returns the post-event views for every instrument in deterministic order.
    #[must_use]
    pub const fn market_states(self) -> &'event [MarketStateView<'event>] {
        self.market_states
    }

    #[must_use]
    pub const fn trading(self) -> &'event TradingContext {
        self.trading
    }

    #[must_use]
    pub const fn session(self) -> &'event SessionCallbackContext {
        self.session
    }

    #[must_use]
    pub const fn decision_time(self) -> MatchTime {
        self.decision_time
    }
}

pub struct StrategyFinalizeContext<'state> {
    clock: ReplayClock,
    states: Box<[MarketStateView<'state>]>,
}

impl<'state> StrategyFinalizeContext<'state> {
    pub fn new(
        clock: ReplayClock,
        states: impl IntoIterator<Item = MarketStateView<'state>>,
    ) -> Self {
        Self {
            clock,
            states: states.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn clock(&self) -> ReplayClock {
        self.clock
    }

    pub fn states(&self) -> impl Iterator<Item = MarketStateView<'state>> + '_ {
        self.states.iter().copied()
    }
}

pub trait Strategy {
    fn identity(&self) -> &StrategyIdentity;

    fn canonical_params_checksum(&self) -> CanonicalParamsChecksum;

    fn declaration(&self) -> StrategyDeclaration;

    fn initialize(
        &mut self,
        _context: &StrategyInitializationContext<'_>,
    ) -> Result<(), StrategyExecutionError> {
        Ok(())
    }

    fn on_event(
        &mut self,
        context: StrategyEventContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError>;

    fn on_feedback(
        &mut self,
        _context: StrategyFeedbackContext<'_>,
        _output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        Ok(())
    }

    fn on_timer(
        &mut self,
        _context: StrategyTimerContext<'_>,
        _output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        Ok(())
    }

    fn finalize(
        &mut self,
        _context: &StrategyFinalizeContext<'_>,
        _output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        Ok(())
    }
}

impl<S: Strategy + ?Sized> Strategy for Box<S> {
    fn identity(&self) -> &StrategyIdentity {
        (**self).identity()
    }

    fn canonical_params_checksum(&self) -> CanonicalParamsChecksum {
        (**self).canonical_params_checksum()
    }

    fn declaration(&self) -> StrategyDeclaration {
        (**self).declaration()
    }

    fn initialize(
        &mut self,
        context: &StrategyInitializationContext<'_>,
    ) -> Result<(), StrategyExecutionError> {
        (**self).initialize(context)
    }

    fn on_event(
        &mut self,
        context: StrategyEventContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        (**self).on_event(context, output)
    }

    fn on_feedback(
        &mut self,
        context: StrategyFeedbackContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        (**self).on_feedback(context, output)
    }

    fn on_timer(
        &mut self,
        context: StrategyTimerContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        (**self).on_timer(context, output)
    }

    fn finalize(
        &mut self,
        context: &StrategyFinalizeContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        (**self).finalize(context, output)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyExecutionError {
    message: Box<str>,
    capability_unavailable: bool,
}

impl StrategyExecutionError {
    #[must_use]
    pub fn new(message: impl Into<Box<str>>) -> Self {
        Self {
            message: message.into(),
            capability_unavailable: false,
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) const fn is_capability_unavailable(&self) -> bool {
        self.capability_unavailable
    }
}

impl fmt::Display for StrategyExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for StrategyExecutionError {}

impl From<StrategyOutputEncodingError> for StrategyExecutionError {
    fn from(error: StrategyOutputEncodingError) -> Self {
        Self::new(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityError {
    OrderIntentUnavailable,
    ScheduledOrderUnavailable,
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrderIntentUnavailable => {
                formatter.write_str("order intent capability is unavailable")
            }
            Self::ScheduledOrderUnavailable => {
                formatter.write_str("scheduled order capability is unavailable")
            }
        }
    }
}

impl Error for CapabilityError {}

impl From<CapabilityError> for StrategyExecutionError {
    fn from(error: CapabilityError) -> Self {
        Self {
            message: error.to_string().into_boxed_str(),
            capability_unavailable: true,
        }
    }
}

impl From<crate::OrderIntentError> for StrategyExecutionError {
    fn from(error: crate::OrderIntentError) -> Self {
        Self {
            message: error.to_string().into_boxed_str(),
            capability_unavailable: true,
        }
    }
}

impl From<crate::ScheduledOrderCapabilityError> for StrategyExecutionError {
    fn from(error: crate::ScheduledOrderCapabilityError) -> Self {
        Self {
            message: error.to_string().into_boxed_str(),
            capability_unavailable: true,
        }
    }
}

impl From<crate::StrategyTimerError> for StrategyExecutionError {
    fn from(error: crate::StrategyTimerError) -> Self {
        Self {
            message: error.to_string().into_boxed_str(),
            capability_unavailable: matches!(
                error,
                crate::StrategyTimerError::CapabilityUnavailable
            ),
        }
    }
}
