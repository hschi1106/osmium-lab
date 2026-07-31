use std::path::PathBuf;

use market_types::{Decimal, InstrumentId, MarketId, QuantityUnit, Symbol, TradingDate};
use run_planner::{
    ChargeConfig, ChargeSides, Currency, CurrencyAmount, FillEvidence, FillModelConfig,
    InstrumentEconomicsConfig, MarkingPolicyConfig, OutputPolicy, PositionAccountingConfig,
    QuantityAllocationConfig, QuantityEvidence, ReplayDataPolicy, RoundingPolicy, RunConfig,
    SimulationConfig, SlippageModelConfig, SourcePolicy, StrategyBinding,
};
use strategy_api::{
    BinaryIdentity, CanonicalParamsChecksum, SessionKind, StrategyDeclaration, StrategyIdentity,
};

pub fn instrument(symbol: &str) -> InstrumentId {
    InstrumentId::new(MarketId::Twse, Symbol::new(symbol).unwrap())
}

pub fn date(value: &str) -> TradingDate {
    TradingDate::parse(value).unwrap()
}

pub fn strategy_binding(
    universe: Vec<InstrumentId>,
    sessions: Vec<SessionKind>,
) -> StrategyBinding {
    let binary = BinaryIdentity::new("blake3", [7_u8; 32]).unwrap();
    let identity = StrategyIdentity::new("m2.reference", "1", binary).unwrap();
    let declaration = StrategyDeclaration::new(universe, sessions).unwrap();
    StrategyBinding::new(
        identity,
        CanonicalParamsChecksum::for_empty_params(),
        declaration,
    )
}

pub fn simulation() -> SimulationConfig {
    SimulationConfig::new(
        FillModelConfig::new(FillEvidence::TopOfBook, QuantityEvidence::Observed),
        QuantityAllocationConfig::AcceptanceSequence,
        SlippageModelConfig::AdverseFixedDelta {
            delta: Decimal::parse("0.01").unwrap(),
        },
        ChargeConfig::new(
            Decimal::parse("0.001425").unwrap(),
            ChargeSides::BuyAndSell,
            Decimal::ZERO,
            0,
            RoundingPolicy::Down,
            "acceptance-config",
        ),
        ChargeConfig::new(
            Decimal::parse("0.003").unwrap(),
            ChargeSides::Sell,
            Decimal::ZERO,
            0,
            RoundingPolicy::Down,
            "acceptance-config",
        ),
        CurrencyAmount::new(Currency::Twd, Decimal::parse("10000000").unwrap()),
        PositionAccountingConfig::AverageCostV1,
        MarkingPolicyConfig::LastObservableV1 {
            allow_midpoint_fallback: false,
        },
    )
}

pub fn economics(instrument: InstrumentId) -> InstrumentEconomicsConfig {
    InstrumentEconomicsConfig::new(
        instrument,
        QuantityUnit::TradingUnit,
        1_000,
        Currency::Twd,
        Decimal::parse("1").unwrap(),
        "teralion-daily-instrument",
    )
}

pub fn run_config(
    dates: Vec<TradingDate>,
    universe: Vec<InstrumentId>,
    data_root: &str,
) -> RunConfig {
    RunConfig {
        config_version: 1,
        trading_dates: dates,
        universe: universe.clone(),
        session_kinds: vec![SessionKind::Regular],
        strategy: strategy_binding(universe.clone(), vec![SessionKind::Regular]),
        data_root: PathBuf::from(data_root),
        source_policy: None,
        cache_policy: None,
        replay_data_policy: None,
        simulation: simulation(),
        instrument_economics: universe.into_iter().map(economics).collect(),
        output_policy: Some(OutputPolicy::CreateNew),
    }
}

#[allow(dead_code)]
pub fn degraded(mut config: RunConfig) -> RunConfig {
    config.source_policy = Some(SourcePolicy::ExplicitDegraded);
    config.replay_data_policy = Some(ReplayDataPolicy::ExplicitDegraded);
    config
}
