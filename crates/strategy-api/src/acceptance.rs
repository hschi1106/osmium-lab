use std::collections::BTreeSet;

use market_types::{InstrumentId, MarketId, Quantity, QuantityUnit};

use crate::{
    BinaryIdentity, CanonicalParamsChecksum, IndicatorValue, MatchingState, NewOrderEntry,
    OrderIntent, OrderSide, OrderType, SessionKind, SessionPhase, Strategy, StrategyDeclaration,
    StrategyEventContext, StrategyExecutionError, StrategyFinalizeContext, StrategyIdentity,
    StrategyOutputSink,
};

pub const ACCEPTANCE_STRATEGY_ID: &str = "acceptance.multi-market";
pub const ACCEPTANCE_STRATEGY_VERSION: &str = "1";

/// Deterministic acceptance strategy for the multi-market universe.
///
/// Each instrument opens one displayed-unit limit order at the best ask and
/// emits a matching close at the best bid on a later event. The strategy does
/// not inspect future events and never emits during indicative or cooldown
/// contexts.
#[derive(Debug)]
pub struct AcceptanceStrategy {
    identity: StrategyIdentity,
    declaration: StrategyDeclaration,
    opened: BTreeSet<InstrumentId>,
    closed: BTreeSet<InstrumentId>,
}

impl AcceptanceStrategy {
    pub fn source_binary_identity() -> Result<BinaryIdentity, crate::DeclarationError> {
        let digest = blake3::hash(include_bytes!("acceptance.rs"));
        BinaryIdentity::new("strategy-source-blake3", *digest.as_bytes())
    }

    pub fn new(
        binary_identity: BinaryIdentity,
        universe: impl IntoIterator<Item = InstrumentId>,
        sessions: impl IntoIterator<Item = SessionKind>,
    ) -> Result<Self, crate::DeclarationError> {
        let declaration = StrategyDeclaration::new(universe, sessions)?;
        let identity = StrategyIdentity::new(
            ACCEPTANCE_STRATEGY_ID,
            ACCEPTANCE_STRATEGY_VERSION,
            binary_identity,
        )?;
        Ok(Self {
            identity,
            declaration,
            opened: BTreeSet::new(),
            closed: BTreeSet::new(),
        })
    }

    fn quantity(instrument: &InstrumentId) -> Result<Quantity, StrategyExecutionError> {
        let unit = match instrument.market() {
            MarketId::Twse | MarketId::Tpex => QuantityUnit::TradingUnit,
            MarketId::Taifex => QuantityUnit::Contract,
        };
        Quantity::new(1, unit)
            .map_err(|_| StrategyExecutionError::new("invalid acceptance order quantity"))
    }
}

impl Strategy for AcceptanceStrategy {
    fn identity(&self) -> &StrategyIdentity {
        &self.identity
    }

    fn canonical_params_checksum(&self) -> CanonicalParamsChecksum {
        CanonicalParamsChecksum::for_empty_params()
    }

    fn declaration(&self) -> StrategyDeclaration {
        self.declaration.clone()
    }

    fn on_event(
        &mut self,
        context: StrategyEventContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        let state = context.market_state();
        output.emit_indicator(
            "state_version",
            IndicatorValue::Unsigned(state.state_version()),
        )?;
        output.emit_indicator(
            "processed_instrument_states",
            IndicatorValue::Unsigned(
                context
                    .market_states()
                    .iter()
                    .filter(|state| state.state_version() > 0)
                    .count() as u64,
            ),
        )?;
        output.emit_indicator(
            "session_kind",
            IndicatorValue::Unsigned(context.session().session_kind() as u64),
        )?;

        if context.session().phase() != SessionPhase::Active
            || !matches!(context.trading().new_order_entry(), NewOrderEntry::Allowed)
            || !matches!(context.trading().matching(), MatchingState::Enabled(_))
        {
            return Ok(());
        }

        let instrument = state.instrument().clone();
        if !self.opened.contains(&instrument)
            && let Some(ask) = state.best_ask()
        {
            output.emit_order_intent(OrderIntent::new(
                instrument.clone(),
                OrderSide::Buy,
                Self::quantity(&instrument)?,
                OrderType::Limit {
                    limit_price: ask.price(),
                },
            ))?;
            self.opened.insert(instrument);
        } else if self.opened.contains(&instrument)
            && !self.closed.contains(&instrument)
            && state.state_version() > 1
            && let Some(bid) = state.best_bid()
        {
            output.emit_order_intent(OrderIntent::new(
                instrument.clone(),
                OrderSide::Sell,
                Self::quantity(&instrument)?,
                OrderType::Limit {
                    limit_price: bid.price(),
                },
            ))?;
            self.closed.insert(instrument);
        }
        Ok(())
    }

    fn finalize(
        &mut self,
        _context: &StrategyFinalizeContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        output.emit_indicator(
            "opened_instruments",
            IndicatorValue::Unsigned(self.opened.len() as u64),
        )?;
        output.emit_indicator(
            "closed_instruments",
            IndicatorValue::Unsigned(self.closed.len() as u64),
        )?;
        Ok(())
    }
}
