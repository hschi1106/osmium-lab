use market_types::{Quantity, QuantityUnit};

use crate::{
    BinaryIdentity, CanonicalParamsChecksum, MatchingState, NewOrderEntry, OrderIntent, OrderSide,
    OrderType, SessionKind, SessionPhase, Strategy, StrategyDeclaration, StrategyEventContext,
    StrategyExecutionError, StrategyIdentity, StrategyOutputSink,
};

pub const M2_ACCEPTANCE_STRATEGY_ID: &str = "acceptance.twse-basic-orders";
pub const M2_ACCEPTANCE_STRATEGY_VERSION: &str = "1";

#[derive(Debug)]
pub struct M2AcceptanceStrategy {
    identity: StrategyIdentity,
    declaration: StrategyDeclaration,
    warm_up_emitted: bool,
    active_emissions: u8,
}

impl M2AcceptanceStrategy {
    pub fn source_binary_identity() -> Result<BinaryIdentity, crate::DeclarationError> {
        let digest = blake3::hash(include_bytes!("m2_acceptance.rs"));
        BinaryIdentity::new("strategy-source-blake3", *digest.as_bytes())
    }

    pub fn new(
        binary_identity: BinaryIdentity,
        instrument: market_types::InstrumentId,
    ) -> Result<Self, crate::DeclarationError> {
        Ok(Self {
            identity: StrategyIdentity::new(
                M2_ACCEPTANCE_STRATEGY_ID,
                M2_ACCEPTANCE_STRATEGY_VERSION,
                binary_identity,
            )?,
            declaration: StrategyDeclaration::new([instrument], [SessionKind::Regular])?,
            warm_up_emitted: false,
            active_emissions: 0,
        })
    }

    fn quantity(value: u64) -> Result<Quantity, StrategyExecutionError> {
        Quantity::new(value, QuantityUnit::TradingUnit)
            .map_err(|_| StrategyExecutionError::new("invalid acceptance order quantity"))
    }
}

impl Strategy for M2AcceptanceStrategy {
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
        let instrument = state.instrument().clone();

        if context.session().phase() == SessionPhase::WarmUp
            && !self.warm_up_emitted
            && let Some(bid) = state.best_bid()
        {
            self.warm_up_emitted = true;
            output.emit_order_intent(OrderIntent::new(
                instrument.clone(),
                OrderSide::Buy,
                Self::quantity(1)?,
                OrderType::Market,
            ))?;
            output.emit_order_intent(OrderIntent::new(
                instrument,
                OrderSide::Buy,
                Self::quantity(1)?,
                OrderType::Limit {
                    limit_price: bid.price(),
                },
            ))?;
            return Ok(());
        }

        if context.session().phase() != SessionPhase::Active
            || !matches!(context.trading().new_order_entry(), NewOrderEntry::Allowed)
            || !matches!(context.trading().matching(), MatchingState::Enabled(_))
        {
            return Ok(());
        }

        match self.active_emissions {
            0 => {
                output.emit_order_intent(OrderIntent::new(
                    instrument,
                    OrderSide::Buy,
                    Self::quantity(1)?,
                    OrderType::Market,
                ))?;
            }
            1 => {
                output.emit_order_intent(OrderIntent::new(
                    instrument,
                    OrderSide::Sell,
                    Self::quantity(1)?,
                    OrderType::Market,
                ))?;
            }
            2 => {
                if let Some(bid) = state.best_bid() {
                    output.emit_order_intent(OrderIntent::new(
                        instrument,
                        OrderSide::Buy,
                        Self::quantity(2)?,
                        OrderType::Limit {
                            limit_price: bid.price(),
                        },
                    ))?;
                }
            }
            _ => return Ok(()),
        }
        self.active_emissions += 1;
        Ok(())
    }
}
