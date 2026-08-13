mod support;

use std::path::Path;

use market_types::Decimal;
use run_planner::{
    CachePolicy, ConfigError, DayTradeMatchingConfig, DayTradeTaxConfig, EffectiveRunConfig,
    InstrumentChargeConfig, LEGACY_EFFECTIVE_CONFIG_VERSION, LatencyConfig, ReplayDataPolicy,
    ScheduledExecutionConfig, SlippageModelConfig, SourcePolicy,
};
use strategy_api::SessionKind;

use support::{date, economics, instrument, run_config, simulation, strategy_binding};

#[test]
fn effective_config_applies_defaults_and_canonicalizes_sets() {
    let instrument = instrument("2330");
    let mut config = run_config(
        vec![date("2026-07-27"), date("2026-07-27")],
        vec![instrument.clone(), instrument.clone()],
        "target/test-data",
    );
    config.session_kinds = vec![SessionKind::Regular, SessionKind::Regular];
    config.strategy = strategy_binding(
        vec![instrument.clone(), instrument.clone()],
        vec![SessionKind::Regular, SessionKind::Regular],
    );
    config.instrument_economics = vec![economics(instrument)];

    let effective = EffectiveRunConfig::resolve(config).unwrap();

    assert_eq!(effective.trading_dates(), [date("2026-07-27")]);
    assert_eq!(effective.universe().len(), 1);
    assert_eq!(effective.session_kinds(), [SessionKind::Regular]);
    assert_eq!(effective.source_policy(), SourcePolicy::Strict);
    assert_eq!(effective.cache_policy(), CachePolicy::ReuseOrRebuild);
    assert_eq!(effective.replay_data_policy(), ReplayDataPolicy::Strict);
    assert_eq!(effective.data_root(), Path::new("target/test-data"));
    assert!(effective.canonical_semantics().starts_with(b"OSECFG01"));
    assert_eq!(
        effective.canonical_version(),
        LEGACY_EFFECTIVE_CONFIG_VERSION
    );
}

#[test]
fn semantic_checksum_excludes_data_root() {
    let instrument = instrument("2330");
    let left = EffectiveRunConfig::resolve(run_config(
        vec![date("2026-07-27")],
        vec![instrument.clone()],
        "target/left",
    ))
    .unwrap();
    let right = EffectiveRunConfig::resolve(run_config(
        vec![date("2026-07-27")],
        vec![instrument],
        "/different-machine/right",
    ))
    .unwrap();

    assert_eq!(left.canonical_semantics(), right.canonical_semantics());
    assert_eq!(left.checksum(), right.checksum());
}

#[test]
fn unsupported_config_version_is_rejected() {
    let instrument = instrument("2330");
    let mut config = run_config(
        vec![date("2026-07-27")],
        vec![instrument],
        "target/test-data",
    );
    config.config_version = 99;

    assert_eq!(
        EffectiveRunConfig::resolve(config),
        Err(ConfigError::UnsupportedConfigVersion { actual: 99 })
    );
}

#[test]
fn strategy_declaration_must_match_effective_universe() {
    let selected = instrument("2330");
    let mut config = run_config(vec![date("2026-07-27")], vec![selected], "target/test-data");
    config.strategy = strategy_binding(vec![instrument("2317")], vec![SessionKind::Regular]);

    assert_eq!(
        EffectiveRunConfig::resolve(config),
        Err(ConfigError::StrategyUniverseMismatch)
    );
}

#[test]
fn every_universe_instrument_requires_valid_economics() {
    let instrument = instrument("2330");
    let mut config = run_config(
        vec![date("2026-07-27")],
        vec![instrument.clone()],
        "target/test-data",
    );
    config.instrument_economics.clear();

    assert_eq!(
        EffectiveRunConfig::resolve(config),
        Err(ConfigError::MissingInstrumentEconomics(instrument))
    );
}

#[test]
fn negative_slippage_is_rejected_before_planning() {
    let instrument = instrument("2330");
    let mut config = run_config(
        vec![date("2026-07-27")],
        vec![instrument.clone()],
        "target/test-data",
    );
    let mut simulation = simulation();
    simulation = run_planner::SimulationConfig::new(
        simulation.fill_model(),
        simulation.quantity_allocation(),
        SlippageModelConfig::AdverseFixedDelta {
            delta: Decimal::parse("-0.01").unwrap(),
        },
        simulation.fee_model().clone(),
        simulation.tax_model().clone(),
        simulation.initial_cash(),
        simulation.position_accounting(),
        simulation.marking_policy(),
    );
    config.simulation = simulation;

    assert_eq!(
        EffectiveRunConfig::resolve(config),
        Err(ConfigError::NegativeSlippage)
    );
}

#[test]
fn latency_is_part_of_effective_identity() {
    let instrument = instrument("2330");
    let zero = EffectiveRunConfig::resolve(run_config(
        vec![date("2026-07-27")],
        vec![instrument.clone()],
        "target/test-data",
    ))
    .unwrap();
    let mut delayed_config = run_config(
        vec![date("2026-07-27")],
        vec![instrument],
        "target/test-data",
    );
    delayed_config.simulation = delayed_config
        .simulation
        .with_latency(LatencyConfig::new(12, 34));
    let delayed = EffectiveRunConfig::resolve(delayed_config).unwrap();

    assert_eq!(delayed.simulation().latency(), LatencyConfig::new(12, 34));
    assert_ne!(zero.checksum(), delayed.checksum());
    assert_ne!(zero.canonical_semantics(), delayed.canonical_semantics());
}

#[test]
fn latency_that_cannot_fit_replay_time_is_rejected_before_planning() {
    let instrument = instrument("2330");
    let mut config = run_config(
        vec![date("2026-07-27")],
        vec![instrument],
        "target/test-data",
    );
    config.simulation = config
        .simulation
        .with_latency(LatencyConfig::new(i64::MAX as u64, 0));

    assert_eq!(
        EffectiveRunConfig::resolve(config),
        Err(ConfigError::InvalidLatency)
    );
}

#[test]
fn scheduled_execution_is_explicit_validated_and_identity_bound() {
    let instrument = instrument("2330");
    let baseline = EffectiveRunConfig::resolve(run_config(
        vec![date("2026-07-27")],
        vec![instrument.clone()],
        "target/test-data",
    ))
    .unwrap();
    let mut scheduled_config = run_config(
        vec![date("2026-07-27")],
        vec![instrument.clone()],
        "target/test-data",
    );
    scheduled_config.simulation = scheduled_config
        .simulation
        .with_scheduled_execution(ScheduledExecutionConfig::new(5, 1_000));
    let scheduled = EffectiveRunConfig::resolve(scheduled_config).unwrap();
    assert_eq!(scheduled.canonical_version(), 4);
    assert_eq!(
        scheduled.simulation().scheduled_execution(),
        Some(ScheduledExecutionConfig::new(5, 1_000))
    );
    assert_ne!(scheduled.checksum(), baseline.checksum());

    let mut invalid = run_config(
        vec![date("2026-07-27")],
        vec![instrument],
        "target/test-data",
    );
    invalid.simulation = invalid
        .simulation
        .with_scheduled_execution(ScheduledExecutionConfig::new(0, 0));
    assert_eq!(
        EffectiveRunConfig::resolve(invalid),
        Err(ConfigError::InvalidScheduledExecution)
    );
}

#[test]
fn economics_outside_universe_is_rejected() {
    let selected = instrument("2330");
    let outside = instrument("2317");
    let mut config = run_config(vec![date("2026-07-27")], vec![selected], "target/test-data");
    config.instrument_economics.push(economics(outside.clone()));

    assert_eq!(
        EffectiveRunConfig::resolve(config),
        Err(ConfigError::EconomicsOutsideUniverse(outside))
    );
}

#[test]
fn instrument_day_trade_tax_is_canonical_and_requires_every_configured_date() {
    let instrument = instrument("2330");
    let dates = vec![date("2026-06-23"), date("2026-06-24")];
    let ordinary = simulation().tax_model().clone();
    let reduced = run_planner::ChargeConfig::new(
        Decimal::parse("0.0015").unwrap(),
        run_planner::ChargeSides::Sell,
        Decimal::ZERO,
        0,
        run_planner::RoundingPolicy::Down,
        "MOF:SecuritiesTransactionTaxAct-2025",
    );
    let charges = |eligible_dates: Vec<market_types::TradingDate>| {
        InstrumentChargeConfig::new(
            instrument.clone(),
            simulation().fee_model().clone(),
            ordinary.clone(),
            Some(DayTradeTaxConfig::new(
                reduced.clone(),
                DayTradeMatchingConfig::SameAccountInstrumentTradingDateFifo,
                480,
                eligible_dates,
                true,
                date("2027-12-31"),
                "TWSE:day-trading-eligibility",
            )),
        )
    };
    let mut missing = run_config(dates.clone(), vec![instrument.clone()], "target/test-data");
    missing.simulation = missing
        .simulation
        .with_instrument_charges([charges(vec![dates[0]])]);
    assert_eq!(
        EffectiveRunConfig::resolve(missing),
        Err(ConfigError::MissingDayTradeEligibility(instrument.clone()))
    );

    let baseline = EffectiveRunConfig::resolve(run_config(
        dates.clone(),
        vec![instrument.clone()],
        "target/test-data",
    ))
    .unwrap();
    let mut configured = run_config(dates.clone(), vec![instrument.clone()], "target/test-data");
    configured.simulation = configured
        .simulation
        .with_instrument_charges([charges(dates)]);
    let configured = EffectiveRunConfig::resolve(configured).unwrap();
    assert_eq!(configured.canonical_version(), 4);
    assert_ne!(configured.checksum(), baseline.checksum());
    assert!(
        configured
            .simulation()
            .charges_for(&instrument)
            .unwrap()
            .day_trade_tax()
            .unwrap()
            .is_eligible(date("2026-06-24"))
    );
}
