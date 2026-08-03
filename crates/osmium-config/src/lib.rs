use std::{
    collections::BTreeSet,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use data_sync::{ArchiveMarket, PartitionCacheCatalog, PartitionedSourceRepository};
use market_types::{
    Decimal, InstrumentId, InstrumentKind, MarketId, OptionSide, QuantityUnit, Symbol, TradingDate,
};
use replay_engine::{ReplayPlan, ReplayStreamBinding, StableStreamDescriptorId};
use run_planner::{
    CacheIdentity, CachePolicy, CacheState, ChargeConfig, ChargeSides, Currency, CurrencyAmount,
    EffectiveRunConfig, ExecutionPlan, FillEvidence, FillModelConfig, InstrumentEconomicsConfig,
    LatencyConfig, MarkingPolicyConfig, OutputPolicy, PlannedPartition, PositionAccountingConfig,
    QuantityAllocationConfig, QuantityEvidence, ReplayDataPolicy, RoundingPolicy,
    RunConfig as PlannerRunConfig, SessionPlan, SessionPlanError, SlippageModelConfig, SourceId,
    SourcePartitionKey, SourcePolicy, SourceState, StrategyBinding,
};
use serde::Deserialize;
use strategy_api::{
    BinaryIdentity, CanonicalParamsChecksum, SessionKind, StrategyDeclaration, StrategyIdentity,
};

pub const RUN_CONFIG_VERSION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentSelection {
    instrument: InstrumentId,
    session_kinds: Box<[SessionKind]>,
    kind: InstrumentKind,
    reference: Option<InstrumentReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentReference {
    underlying: Box<str>,
    expiry: TradingDate,
    strike: Decimal,
    option_side: OptionSide,
    currency: Currency,
    multiplier: Decimal,
    quantity_unit: QuantityUnit,
    units_per_trading_unit: u64,
    provenance: Box<str>,
}

impl InstrumentReference {
    #[must_use]
    pub fn underlying(&self) -> &str {
        &self.underlying
    }

    #[must_use]
    pub const fn expiry(&self) -> TradingDate {
        self.expiry
    }

    #[must_use]
    pub const fn strike(&self) -> Decimal {
        self.strike
    }

    #[must_use]
    pub const fn option_side(&self) -> OptionSide {
        self.option_side
    }

    #[must_use]
    pub const fn currency(&self) -> Currency {
        self.currency
    }

    #[must_use]
    pub const fn multiplier(&self) -> Decimal {
        self.multiplier
    }

    #[must_use]
    pub const fn quantity_unit(&self) -> QuantityUnit {
        self.quantity_unit
    }

    #[must_use]
    pub const fn units_per_trading_unit(&self) -> u64 {
        self.units_per_trading_unit
    }

    #[must_use]
    pub fn provenance(&self) -> &str {
        &self.provenance
    }
}

impl InstrumentSelection {
    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn session_kinds(&self) -> &[SessionKind] {
        &self.session_kinds
    }

    #[must_use]
    pub const fn kind(&self) -> InstrumentKind {
        self.kind
    }

    #[must_use]
    pub const fn reference(&self) -> Option<&InstrumentReference> {
        self.reference.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct RunConfig {
    effective: EffectiveRunConfig,
    selections: Box<[InstrumentSelection]>,
}

impl RunConfig {
    #[must_use]
    pub const fn effective(&self) -> &EffectiveRunConfig {
        &self.effective
    }

    #[must_use]
    pub const fn selections(&self) -> &[InstrumentSelection] {
        &self.selections
    }

    #[must_use]
    pub fn selection_for(&self, instrument: &InstrumentId) -> Option<&InstrumentSelection> {
        self.selections
            .iter()
            .find(|selection| selection.instrument() == instrument)
    }

    #[must_use]
    pub fn instrument_kind_for(&self, instrument: &InstrumentId) -> InstrumentKind {
        self.selection_for(instrument).map_or_else(
            || default_kind(instrument.market()),
            InstrumentSelection::kind,
        )
    }

    #[must_use]
    pub fn archive_market_for(&self, instrument: &InstrumentId) -> ArchiveMarket {
        match self.instrument_kind_for(instrument) {
            InstrumentKind::Option => ArchiveMarket::TaifexOptions,
            _ => ArchiveMarket::for_instrument(instrument),
        }
    }

    pub fn session_plan_for(&self, key: &SourcePartitionKey) -> Result<SessionPlan, ConfigError> {
        let kind = self.instrument_kind_for(key.instrument());
        SessionPlan::for_instrument_kind(
            key.instrument(),
            kind,
            key.trading_date(),
            key.session_kinds().iter().copied(),
        )
        .map_err(ConfigError::SessionPlan)
    }

    pub fn partition_keys(&self) -> Result<Box<[SourcePartitionKey]>, ConfigError> {
        let mut keys = Vec::new();
        for selection in &self.selections {
            for trading_date in self.effective.trading_dates() {
                let session_plan = SessionPlan::for_instrument_kind(
                    &selection.instrument,
                    selection.kind,
                    *trading_date,
                    selection.session_kinds.iter().copied(),
                )
                .map_err(ConfigError::SessionPlan)?;
                keys.push(
                    SourcePartitionKey::new(
                        SourceId::TeralionFeedArchive,
                        selection.instrument.clone(),
                        *trading_date,
                        selection.session_kinds.iter().copied(),
                        session_plan.identity(),
                    )
                    .map_err(|error| ConfigError::Value(error.to_string()))?,
                );
            }
        }
        Ok(keys.into_boxed_slice())
    }
}

#[derive(Debug)]
pub struct PlanBundle {
    pub execution: ExecutionPlan,
    pub session_plans: Box<[SessionPlan]>,
    pub replay: Option<ReplayPlan>,
}

pub fn config_version(path: impl AsRef<Path>) -> Result<u16, ConfigError> {
    let bytes = fs::read(path)?;
    let raw: serde_yaml::Value =
        serde_yaml::from_slice(&bytes).map_err(|error| ConfigError::Yaml(error.to_string()))?;
    raw.get("config_version")
        .and_then(serde_yaml::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(ConfigError::Invalid("config_version"))
}

pub fn load(path: impl AsRef<Path>) -> Result<RunConfig, ConfigError> {
    let bytes = fs::read(path)?;
    reject_secrets(&bytes)?;
    let value: serde_yaml::Value =
        serde_yaml::from_slice(&bytes).map_err(|error| ConfigError::Yaml(error.to_string()))?;
    let actual = value
        .get("config_version")
        .and_then(serde_yaml::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(ConfigError::Invalid("config_version"))?;
    if actual != RUN_CONFIG_VERSION {
        return Err(ConfigError::UnsupportedVersion {
            expected: RUN_CONFIG_VERSION,
            actual,
        });
    }
    let raw: FileConfig =
        serde_yaml::from_value(value).map_err(|error| ConfigError::Yaml(error.to_string()))?;
    resolve(raw)
}

pub fn plan(config: RunConfig) -> Result<PlanBundle, ConfigError> {
    let mut partitions = Vec::new();
    let mut session_plans = Vec::new();
    let mut replay_bindings = Vec::new();
    let mut replay_ready = true;
    let cache_catalog = PartitionCacheCatalog::new(config.effective.data_root());
    for selection in &config.selections {
        for trading_date in config.effective.trading_dates() {
            let session_plan = SessionPlan::for_instrument_kind(
                &selection.instrument,
                selection.kind,
                *trading_date,
                selection.session_kinds.iter().copied(),
            )
            .map_err(ConfigError::SessionPlan)?;
            let key = SourcePartitionKey::new(
                SourceId::TeralionFeedArchive,
                selection.instrument.clone(),
                *trading_date,
                selection.session_kinds.iter().copied(),
                session_plan.identity(),
            )
            .map_err(|error| ConfigError::Value(error.to_string()))?;
            let repository =
                PartitionedSourceRepository::new(config.effective.data_root(), key.clone())
                    .map_err(|error| ConfigError::Value(error.to_string()))?;
            let inspection = repository.inspect();
            let cache_state = cache_state(&cache_catalog, &key, inspection.report())?;
            if let (SourceState::Complete { revision }, CacheState::Valid { identity: cache }) =
                (inspection.state(), cache_state)
            {
                replay_bindings.push(ReplayStreamBinding::new(
                    StableStreamDescriptorId::from_bytes(
                        *blake3::hash(cache.as_bytes()).as_bytes(),
                    ),
                    key.instrument().clone(),
                    key.trading_date(),
                    *revision.as_bytes(),
                    *cache.as_bytes(),
                ));
            } else {
                replay_ready = false;
            }
            partitions.push(PlannedPartition::classify(
                key,
                inspection.state(),
                cache_state,
            ));
            session_plans.push(session_plan);
        }
    }
    let execution = ExecutionPlan::new(config.effective, partitions, Vec::new())
        .map_err(|error| ConfigError::Value(error.to_string()))?;
    let replay = if replay_ready {
        Some(
            ReplayPlan::new_multi(*execution.identity().as_bytes(), replay_bindings)
                .map_err(|error| ConfigError::Value(error.to_string()))?,
        )
    } else {
        None
    };
    Ok(PlanBundle {
        execution,
        session_plans: session_plans.into_boxed_slice(),
        replay,
    })
}

fn cache_state(
    catalog: &PartitionCacheCatalog,
    key: &SourcePartitionKey,
    inspection: Option<&data_sync::VerificationReport>,
) -> Result<CacheState, ConfigError> {
    let Some(report) = inspection else {
        return Ok(CacheState::Missing);
    };
    let entry = match catalog.find(key, &report.manifest().revision_identity) {
        Ok(entry) => entry,
        Err(_) => return Ok(CacheState::Corrupt),
    };
    let Some(entry) = entry else {
        return Ok(CacheState::Missing);
    };
    let identity = decode_hex(&entry.descriptor().cache_identity)?;
    Ok(CacheState::Valid {
        identity: CacheIdentity::from_bytes(identity),
    })
}

fn resolve(raw: FileConfig) -> Result<RunConfig, ConfigError> {
    if raw.config_version != RUN_CONFIG_VERSION {
        return Err(ConfigError::UnsupportedVersion {
            expected: RUN_CONFIG_VERSION,
            actual: raw.config_version,
        });
    }
    require(raw.data.source == "teralion", "data.source")?;
    require(
        raw.data.cache_policy == "reuse_or_rebuild",
        "data.cache_policy",
    )?;
    if raw.universe.trading_dates.is_empty() {
        return Err(ConfigError::Invalid("universe.trading_dates"));
    }
    if raw.universe.instruments.is_empty() {
        return Err(ConfigError::Invalid("universe.instruments"));
    }

    let trading_dates = raw
        .universe
        .trading_dates
        .iter()
        .map(|value| {
            value
                .parse::<TradingDate>()
                .map_err(|error| ConfigError::Value(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let selections = raw
        .universe
        .instruments
        .iter()
        .map(parse_selection)
        .collect::<Result<Vec<_>, _>>()?;
    let mut instruments = selections
        .iter()
        .map(|selection| selection.instrument.clone())
        .collect::<Vec<_>>();
    instruments.sort();
    instruments.dedup();
    if instruments.len() != selections.len() {
        return Err(ConfigError::Invalid("universe.instruments"));
    }
    let sessions = selections
        .iter()
        .flat_map(|selection| selection.session_kinds.iter().copied())
        .collect::<BTreeSet<_>>();
    let sessions = sessions.into_iter().collect::<Vec<_>>();
    let declaration = StrategyDeclaration::new(instruments.clone(), sessions.clone())
        .map_err(|error| ConfigError::Value(error.to_string()))?;
    let date_values = trading_dates.clone();
    let economics = raw
        .instrument_economics
        .into_iter()
        .map(parse_economics)
        .collect::<Result<Vec<_>, _>>()?;
    validate_reference_economics(&selections, &economics)?;
    let params_checksum = params_checksum_with_references(&raw.strategy.parameters, &selections)?;
    let binary_digest = strategy_digest(&raw.strategy.id, &raw.strategy.version, params_checksum);
    let strategy_identity = StrategyIdentity::new(
        raw.strategy.id,
        raw.strategy.version,
        BinaryIdentity::new("config", binary_digest.to_vec())
            .map_err(|error| ConfigError::Value(error.to_string()))?,
    )
    .map_err(|error| ConfigError::Value(error.to_string()))?;
    let strategy = StrategyBinding::new(strategy_identity, params_checksum, declaration);
    let effective = EffectiveRunConfig::resolve(PlannerRunConfig {
        // The planner's effective schema is independent from the user-facing file version.
        config_version: 1,
        trading_dates: date_values,
        universe: instruments,
        session_kinds: sessions,
        strategy,
        data_root: raw.data.data_root,
        source_policy: Some(parse_source_policy(&raw.data.source_policy)?),
        cache_policy: Some(CachePolicy::ReuseOrRebuild),
        replay_data_policy: Some(parse_replay_policy(&raw.replay.data_policy)?),
        simulation: parse_simulation(raw.simulation)?,
        instrument_economics: economics,
        output_policy: Some(parse_output_policy(&raw.output.publication)?),
    })
    .map_err(|error| ConfigError::Value(error.to_string()))?;
    Ok(RunConfig {
        effective,
        selections: selections.into_boxed_slice(),
    })
}

fn parse_selection(raw: &InstrumentConfig) -> Result<InstrumentSelection, ConfigError> {
    let market = parse_market(&raw.market)?;
    let instrument = InstrumentId::new(
        market,
        Symbol::new(raw.symbol.clone()).map_err(|error| ConfigError::Value(error.to_string()))?,
    );
    let mut session_kinds = raw
        .session_kinds
        .iter()
        .map(|value| parse_session(value))
        .collect::<Result<Vec<_>, _>>()?;
    session_kinds.sort_unstable();
    session_kinds.dedup();
    if session_kinds.is_empty() {
        return Err(ConfigError::Invalid("universe.instruments.session_kinds"));
    }
    let kind = raw
        .instrument_kind
        .as_deref()
        .map(parse_instrument_kind)
        .transpose()?
        .unwrap_or_else(|| default_kind(market));
    let reference = raw.reference.as_ref().map(parse_reference).transpose()?;
    match (market, kind) {
        (MarketId::Twse | MarketId::Tpex, InstrumentKind::Equity)
        | (MarketId::Twse | MarketId::Tpex, InstrumentKind::Warrant)
        | (MarketId::Taifex, InstrumentKind::Future)
        | (MarketId::Taifex, InstrumentKind::Option) => {}
        _ => {
            return Err(ConfigError::Invalid("universe.instruments.instrument_kind"));
        }
    }
    if matches!(kind, InstrumentKind::Warrant | InstrumentKind::Option) && reference.is_none() {
        return Err(ConfigError::Invalid("universe.instruments.reference"));
    }
    Ok(InstrumentSelection {
        instrument,
        session_kinds: session_kinds.into_boxed_slice(),
        kind,
        reference,
    })
}

fn default_kind(market: MarketId) -> InstrumentKind {
    match market {
        MarketId::Twse | MarketId::Tpex => InstrumentKind::Equity,
        MarketId::Taifex => InstrumentKind::Future,
    }
}

fn parse_instrument_kind(value: &str) -> Result<InstrumentKind, ConfigError> {
    match value {
        "equity" => Ok(InstrumentKind::Equity),
        "warrant" => Ok(InstrumentKind::Warrant),
        "future" => Ok(InstrumentKind::Future),
        "option" => Ok(InstrumentKind::Option),
        _ => Err(ConfigError::Invalid("universe.instruments.instrument_kind")),
    }
}

fn parse_reference(raw: &ReferenceConfig) -> Result<InstrumentReference, ConfigError> {
    if raw.underlying.trim().is_empty() || raw.provenance.trim().is_empty() {
        return Err(ConfigError::Invalid("universe.instruments.reference"));
    }
    let expiry = raw
        .expiry
        .parse::<TradingDate>()
        .map_err(|error| ConfigError::Value(error.to_string()))?;
    let strike = decimal(&raw.strike)?;
    let multiplier = decimal(&raw.multiplier)?;
    if strike <= Decimal::ZERO || multiplier <= Decimal::ZERO || raw.units_per_trading_unit == 0 {
        return Err(ConfigError::Invalid("universe.instruments.reference"));
    }
    let option_side = match raw.option_side.as_str() {
        "call" => OptionSide::Call,
        "put" => OptionSide::Put,
        _ => {
            return Err(ConfigError::Invalid(
                "universe.instruments.reference.option_side",
            ));
        }
    };
    let quantity_unit = match raw.quantity_unit.as_str() {
        "share" => QuantityUnit::Share,
        "trading_unit" => QuantityUnit::TradingUnit,
        "contract" => QuantityUnit::Contract,
        _ => {
            return Err(ConfigError::Invalid(
                "universe.instruments.reference.quantity_unit",
            ));
        }
    };
    Ok(InstrumentReference {
        underlying: raw.underlying.clone().into_boxed_str(),
        expiry,
        strike,
        option_side,
        currency: parse_currency(&raw.currency)?,
        multiplier,
        quantity_unit,
        units_per_trading_unit: raw.units_per_trading_unit,
        provenance: raw.provenance.clone().into_boxed_str(),
    })
}

fn validate_reference_economics(
    selections: &[InstrumentSelection],
    economics: &[InstrumentEconomicsConfig],
) -> Result<(), ConfigError> {
    for selection in selections {
        let Some(reference) = selection.reference() else {
            continue;
        };
        let Some(economics) = economics
            .iter()
            .find(|economics| economics.instrument() == selection.instrument())
        else {
            return Err(ConfigError::Invalid("instrument_economics"));
        };
        if economics.quantity_unit() != reference.quantity_unit()
            || economics.units_per_trading_unit() != reference.units_per_trading_unit()
            || economics.currency() != reference.currency()
            || economics.multiplier() != reference.multiplier()
        {
            return Err(ConfigError::Invalid("instrument_economics"));
        }
    }
    Ok(())
}

fn parse_economics(raw: EconomicsConfig) -> Result<InstrumentEconomicsConfig, ConfigError> {
    Ok(InstrumentEconomicsConfig::new(
        InstrumentId::new(
            parse_market(&raw.market)?,
            Symbol::new(raw.symbol).map_err(|error| ConfigError::Value(error.to_string()))?,
        ),
        match raw.quantity_unit.as_str() {
            "share" => QuantityUnit::Share,
            "trading_unit" => QuantityUnit::TradingUnit,
            "contract" => QuantityUnit::Contract,
            _ => return Err(ConfigError::Invalid("instrument_economics.quantity_unit")),
        },
        raw.units_per_trading_unit,
        parse_currency(&raw.currency)?,
        decimal(&raw.multiplier)?,
        raw.provenance,
    ))
}

fn parse_simulation(
    raw: SimulationFileConfig,
) -> Result<run_planner::SimulationConfig, ConfigError> {
    let fill = FillModelConfig::new(
        match raw.fill.evidence.as_str() {
            "top_of_book" => FillEvidence::TopOfBook,
            "trade_print" => FillEvidence::TradePrint,
            _ => return Err(ConfigError::Invalid("simulation.fill.evidence")),
        },
        match raw.fill.quantity.as_str() {
            "unlimited" => QuantityEvidence::Unlimited,
            "observed" => QuantityEvidence::Observed,
            _ => return Err(ConfigError::Invalid("simulation.fill.quantity")),
        },
    );
    require(
        raw.allocation == "acceptance_sequence",
        "simulation.allocation",
    )?;
    require(
        raw.slippage.model == "adverse_fixed_delta",
        "simulation.slippage.model",
    )?;
    let fee = parse_charge(raw.fee)?;
    let tax = parse_charge(raw.tax)?;
    let initial_cash = CurrencyAmount::new(
        parse_currency(&raw.initial_cash.currency)?,
        decimal(&raw.initial_cash.amount)?,
    );
    require(
        raw.position_accounting == "average_cost_v1",
        "simulation.position_accounting",
    )?;
    require(
        raw.marking.model == "last_observable_mark_v1",
        "simulation.marking.model",
    )?;
    Ok(run_planner::SimulationConfig::new(
        fill,
        QuantityAllocationConfig::AcceptanceSequence,
        SlippageModelConfig::AdverseFixedDelta {
            delta: decimal(&raw.slippage.delta)?,
        },
        fee,
        tax,
        initial_cash,
        PositionAccountingConfig::AverageCostV1,
        MarkingPolicyConfig::LastObservableV1 {
            allow_midpoint_fallback: raw.marking.allow_midpoint_fallback,
        },
    )
    .with_latency(LatencyConfig::new(
        raw.market_data_latency_ms,
        raw.order_latency_ms,
    )))
}

fn parse_charge(raw: ChargeFileConfig) -> Result<ChargeConfig, ConfigError> {
    require(raw.model == "configured_rate", "simulation.charge.model")?;
    let buy = raw.applicable_sides.iter().any(|side| side == "buy");
    let sell = raw.applicable_sides.iter().any(|side| side == "sell");
    let sides = match (buy, sell) {
        (true, true) => ChargeSides::BuyAndSell,
        (true, false) => ChargeSides::Buy,
        (false, true) => ChargeSides::Sell,
        _ => return Err(ConfigError::Invalid("simulation.charge.applicable_sides")),
    };
    Ok(ChargeConfig::new(
        decimal(&raw.rate)?,
        sides,
        decimal(&raw.minimum)?,
        raw.precision,
        match raw.rounding.as_str() {
            "down" => RoundingPolicy::Down,
            "half_up" => RoundingPolicy::HalfUp,
            "up" => RoundingPolicy::Up,
            _ => return Err(ConfigError::Invalid("simulation.charge.rounding")),
        },
        raw.provenance,
    ))
}

fn parse_market(value: &str) -> Result<MarketId, ConfigError> {
    match value {
        "twse" => Ok(MarketId::Twse),
        "tpex" => Ok(MarketId::Tpex),
        "taifex" => Ok(MarketId::Taifex),
        _ => Err(ConfigError::Invalid("market")),
    }
}

fn parse_session(value: &str) -> Result<SessionKind, ConfigError> {
    match value {
        "regular" => Ok(SessionKind::Regular),
        "after_hours" => Ok(SessionKind::AfterHours),
        _ => Err(ConfigError::Invalid("session_kinds")),
    }
}

fn parse_currency(value: &str) -> Result<Currency, ConfigError> {
    match value {
        "TWD" | "twd" => Ok(Currency::Twd),
        _ => Err(ConfigError::Invalid("currency")),
    }
}

fn parse_source_policy(value: &str) -> Result<SourcePolicy, ConfigError> {
    match value {
        "strict" => Ok(SourcePolicy::Strict),
        "explicit_degraded" => Ok(SourcePolicy::ExplicitDegraded),
        _ => Err(ConfigError::Invalid("data.source_policy")),
    }
}

fn parse_replay_policy(value: &str) -> Result<ReplayDataPolicy, ConfigError> {
    match value {
        "strict" => Ok(ReplayDataPolicy::Strict),
        "explicit_degraded" => Ok(ReplayDataPolicy::ExplicitDegraded),
        _ => Err(ConfigError::Invalid("replay.data_policy")),
    }
}

fn parse_output_policy(value: &str) -> Result<OutputPolicy, ConfigError> {
    if value == "create_new" {
        Ok(OutputPolicy::CreateNew)
    } else {
        Err(ConfigError::Invalid("output.publication"))
    }
}

fn decimal(value: &str) -> Result<Decimal, ConfigError> {
    Decimal::parse(value).map_err(|error| ConfigError::Value(error.to_string()))
}

fn params_checksum(value: &serde_yaml::Value) -> Result<CanonicalParamsChecksum, ConfigError> {
    let bytes = serde_json::to_vec(value).map_err(|error| ConfigError::Value(error.to_string()))?;
    let mut canonical = Vec::with_capacity(6 + bytes.len());
    canonical.extend_from_slice(b"OSSP");
    canonical.extend_from_slice(&1_u16.to_be_bytes());
    canonical.extend_from_slice(
        &u32::try_from(bytes.len())
            .map_err(|_| ConfigError::Value("strategy parameters too large".to_owned()))?
            .to_be_bytes(),
    );
    canonical.extend_from_slice(&bytes);
    Ok(CanonicalParamsChecksum::from_bytes(
        *blake3::hash(&canonical).as_bytes(),
    ))
}

fn params_checksum_with_references(
    value: &serde_yaml::Value,
    selections: &[InstrumentSelection],
) -> Result<CanonicalParamsChecksum, ConfigError> {
    let base = params_checksum(value)?;
    if selections
        .iter()
        .all(|selection| selection.reference.is_none())
    {
        return Ok(base);
    }
    let mut ordered = selections.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.instrument.cmp(&right.instrument));
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"OSM5PARAM");
    canonical.extend_from_slice(base.as_bytes());
    canonical.extend_from_slice(&(ordered.len() as u32).to_be_bytes());
    for selection in ordered {
        canonical.push(selection.instrument.market().discriminant());
        append_text_for_checksum(selection.instrument.symbol().as_str(), &mut canonical);
        canonical.push(selection.kind as u8);
        if let Some(reference) = &selection.reference {
            append_text_for_checksum(reference.underlying(), &mut canonical);
            canonical.extend_from_slice(&reference.expiry.to_canonical_bytes());
            canonical.extend_from_slice(&reference.strike.to_canonical_bytes());
            canonical.push(reference.option_side as u8);
            canonical.push(reference.currency as u8);
            canonical.extend_from_slice(&reference.multiplier.to_canonical_bytes());
            canonical.push(reference.quantity_unit.discriminant());
            canonical.extend_from_slice(&reference.units_per_trading_unit.to_be_bytes());
            append_text_for_checksum(reference.provenance(), &mut canonical);
        }
    }
    Ok(CanonicalParamsChecksum::from_bytes(
        *blake3::hash(&canonical).as_bytes(),
    ))
}

fn append_text_for_checksum(value: &str, output: &mut Vec<u8>) {
    output.extend_from_slice(&(value.len() as u32).to_be_bytes());
    output.extend_from_slice(value.as_bytes());
}

fn strategy_digest(id: &str, version: &str, params: CanonicalParamsChecksum) -> [u8; 32] {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"OSM3STRATEGY");
    bytes.extend_from_slice(id.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(version.as_bytes());
    bytes.extend_from_slice(params.as_bytes());
    *blake3::hash(&bytes).as_bytes()
}

fn decode_hex(value: &str) -> Result<[u8; 32], ConfigError> {
    if value.len() != 64 {
        return Err(ConfigError::Invalid("checksum"));
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = nibble(pair[0]).ok_or(ConfigError::Invalid("checksum"))?;
        let low = nibble(pair[1]).ok_or(ConfigError::Invalid("checksum"))?;
        output[index] = high << 4 | low;
    }
    Ok(output)
}

fn nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn reject_secrets(bytes: &[u8]) -> Result<(), ConfigError> {
    let lower = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if ["api_key", "authorization", "bearer", "cookie", "credential"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        Err(ConfigError::SecretField)
    } else {
        Ok(())
    }
}

fn require(condition: bool, field: &'static str) -> Result<(), ConfigError> {
    if condition {
        Ok(())
    } else {
        Err(ConfigError::Invalid(field))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    config_version: u16,
    data: DataConfig,
    universe: UniverseConfig,
    strategy: StrategyConfig,
    replay: ReplayConfig,
    simulation: SimulationFileConfig,
    instrument_economics: Vec<EconomicsConfig>,
    output: OutputConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DataConfig {
    source: String,
    data_root: PathBuf,
    source_policy: String,
    cache_policy: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UniverseConfig {
    trading_dates: Vec<String>,
    instruments: Vec<InstrumentConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstrumentConfig {
    market: String,
    symbol: String,
    session_kinds: Vec<String>,
    #[serde(default)]
    instrument_kind: Option<String>,
    #[serde(default)]
    reference: Option<ReferenceConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceConfig {
    underlying: String,
    expiry: String,
    strike: String,
    option_side: String,
    currency: String,
    multiplier: String,
    quantity_unit: String,
    units_per_trading_unit: u64,
    provenance: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrategyConfig {
    id: String,
    version: String,
    parameters: serde_yaml::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayConfig {
    data_policy: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulationFileConfig {
    fill: FillConfig,
    #[serde(default)]
    market_data_latency_ms: u64,
    #[serde(default)]
    order_latency_ms: u64,
    allocation: String,
    slippage: SlippageConfig,
    fee: ChargeFileConfig,
    tax: ChargeFileConfig,
    initial_cash: CashConfig,
    position_accounting: String,
    marking: MarkingConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FillConfig {
    evidence: String,
    quantity: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlippageConfig {
    model: String,
    delta: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChargeFileConfig {
    model: String,
    rate: String,
    applicable_sides: Vec<String>,
    minimum: String,
    precision: u8,
    rounding: String,
    provenance: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CashConfig {
    currency: String,
    amount: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkingConfig {
    model: String,
    allow_midpoint_fallback: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EconomicsConfig {
    market: String,
    symbol: String,
    quantity_unit: String,
    units_per_trading_unit: u64,
    currency: String,
    multiplier: String,
    provenance: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputConfig {
    publication: String,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Yaml(String),
    SecretField,
    UnsupportedVersion { expected: u16, actual: u16 },
    Invalid(&'static str),
    Value(String),
    SessionPlan(SessionPlanError),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { expected, actual } => write!(
                formatter,
                "unsupported config_version {actual}; expected {expected}; legacy config_version 1 is not supported, upgrade the config"
            ),
            _ => write!(formatter, "{self:?}"),
        }
    }
}

impl Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/config.yaml")
    }

    #[test]
    fn run_config_materializes_the_representative_twse_profile() {
        let config = load(fixture()).unwrap();
        assert_eq!(config.selections().len(), 1);
        assert_eq!(config.effective().universe().len(), 1);
        assert_eq!(config.effective().session_kinds(), &[SessionKind::Regular]);
        assert_eq!(
            config.effective().simulation().latency(),
            run_planner::LatencyConfig::new(0, 0)
        );
        let bundle = plan(config).unwrap();
        assert_eq!(bundle.execution.partitions().len(), 1);
        assert_eq!(bundle.session_plans.len(), 1);
        assert!(
            bundle
                .execution
                .partitions()
                .iter()
                .any(|partition| partition.key().session_kinds() == [SessionKind::Regular])
        );
    }

    #[test]
    fn run_config_materializes_nonzero_latency() {
        let source = fs::read_to_string(fixture())
            .unwrap()
            .replace("market_data_latency_ms: 0", "market_data_latency_ms: 12")
            .replace("order_latency_ms: 0", "order_latency_ms: 34");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("latency.yaml");
        fs::write(&path, source).unwrap();

        let config = load(path).unwrap();
        assert_eq!(
            config.effective().simulation().latency(),
            run_planner::LatencyConfig::new(12, 34)
        );
    }

    #[test]
    fn run_config_rejects_embedded_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secret.yaml");
        fs::write(&path, "config_version: 2\napi_key: forbidden\n").unwrap();
        assert!(matches!(load(path), Err(ConfigError::SecretField)));
    }

    #[test]
    fn run_config_rejects_legacy_schema_with_upgrade_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.yaml");
        fs::write(&path, "config_version: 1\n").unwrap();

        let error = load(path).unwrap_err();
        assert!(matches!(
            &error,
            ConfigError::UnsupportedVersion {
                expected: RUN_CONFIG_VERSION,
                actual: 1
            }
        ));
        assert!(error.to_string().contains("upgrade the config"));
    }
}
