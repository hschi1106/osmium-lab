use market_state::StateField;

use crate::{
    BinaryIdentity, CanonicalParamsChecksum, IndicatorValue, SessionKind, Strategy,
    StrategyDeclaration, StrategyEventContext, StrategyExecutionError, StrategyIdentity,
    StrategyOutputSink,
};

pub const EXAMPLE_STRATEGY_ID: &str = "example.twse-post-state-observer";
pub const EXAMPLE_STRATEGY_VERSION: &str = "1";

#[derive(Debug)]
pub struct ExampleStrategy {
    identity: StrategyIdentity,
    declaration: StrategyDeclaration,
}

impl ExampleStrategy {
    pub fn source_binary_identity() -> Result<BinaryIdentity, crate::DeclarationError> {
        let digest = blake3::hash(include_bytes!("example.rs"));
        BinaryIdentity::new("strategy-source-blake3", *digest.as_bytes())
    }

    pub fn new(
        binary_identity: BinaryIdentity,
        instrument: market_types::InstrumentId,
    ) -> Result<Self, crate::DeclarationError> {
        Ok(Self {
            identity: StrategyIdentity::new(
                EXAMPLE_STRATEGY_ID,
                EXAMPLE_STRATEGY_VERSION,
                binary_identity,
            )?,
            declaration: StrategyDeclaration::new([instrument], [SessionKind::Regular])?,
        })
    }
}

impl Strategy for ExampleStrategy {
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
        output.emit_indicator(
            "state_version",
            IndicatorValue::Unsigned(context.market_state().state_version()),
        )?;
        if let StateField::Known { value, .. } = context.market_state().cumulative_volume() {
            output.emit_indicator("cum_volume", IndicatorValue::Unsigned(value.value()))?;
        }
        Ok(())
    }
}
