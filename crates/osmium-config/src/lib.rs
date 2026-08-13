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
    DayTradeMatchingConfig, DayTradeTaxConfig, EffectiveRunConfig, ExecutionPlan, FillEvidence,
    FillModelConfig, InstrumentChargeConfig, InstrumentEconomicsConfig, LatencyConfig,
    MarkingPolicyConfig, OutputPolicy, PlannedPartition, PositionAccountingConfig,
    QuantityAllocationConfig, QuantityEvidence, ReplayDataPolicy, RoundingPolicy,
    RunConfig as PlannerRunConfig, ScheduledExecutionConfig, SessionPlan, SessionPlanError,
    SessionProfileId, SlippageModelConfig, SourceId, SourcePartitionKey, SourcePolicy, SourceState,
    StrategyBinding,
};
use serde::Deserialize;
use strategy_api::{
    RawStrategyParameter, RawStrategyParameters, ResolvedStrategyMetadata, SessionKind, Strategy,
    StrategyRegistry, StrategyRegistryError,
};

pub const RUN_CONFIG_VERSION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyReferenceInput {
    schema_version: u16,
    path: PathBuf,
    checksum: Box<str>,
}

impl StrategyReferenceInput {
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn checksum(&self) -> &str {
        &self.checksum
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyBootstrapConfig {
    reference: Option<StrategyReferenceInput>,
    simulation: run_planner::SimulationConfig,
}

impl StrategyBootstrapConfig {
    #[must_use]
    pub const fn reference(&self) -> Option<&StrategyReferenceInput> {
        self.reference.as_ref()
    }

    #[must_use]
    pub const fn order_latency_ms(&self) -> u64 {
        self.simulation.latency().order_latency_ms()
    }

    #[must_use]
    pub const fn market_data_latency_ms(&self) -> u64 {
        self.simulation.latency().market_data_latency_ms()
    }

    #[must_use]
    pub const fn simulation(&self) -> &run_planner::SimulationConfig {
        &self.simulation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentSelection {
    instrument: InstrumentId,
    session_kinds: Box<[SessionKind]>,
    kind: InstrumentKind,
    session_profile: Option<SessionProfileId>,
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
    pub const fn session_profile(&self) -> Option<SessionProfileId> {
        self.session_profile
    }

    #[must_use]
    pub const fn reference(&self) -> Option<&InstrumentReference> {
        self.reference.as_ref()
    }
}

pub struct RunConfig {
    effective: EffectiveRunConfig,
    selections: Box<[InstrumentSelection]>,
    strategy: Option<Box<dyn Strategy>>,
    strategy_metadata: ResolvedStrategyMetadata,
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
    pub const fn strategy_metadata(&self) -> &ResolvedStrategyMetadata {
        &self.strategy_metadata
    }

    pub fn take_strategy(&mut self) -> Result<Box<dyn Strategy>, ConfigError> {
        self.strategy
            .take()
            .ok_or(ConfigError::StrategyAlreadyTaken)
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
        if let Some(selection) = self.selection_for(key.instrument()) {
            session_plan(
                selection,
                key.trading_date(),
                key.session_kinds().iter().copied(),
            )
        } else {
            SessionPlan::for_instrument_kind(
                key.instrument(),
                self.instrument_kind_for(key.instrument()),
                key.trading_date(),
                key.session_kinds().iter().copied(),
            )
            .map_err(ConfigError::SessionPlan)
        }
    }

    pub fn partition_keys(&self) -> Result<Box<[SourcePartitionKey]>, ConfigError> {
        let mut keys = Vec::new();
        for selection in &self.selections {
            for trading_date in self.effective.trading_dates() {
                let session_plan = session_plan(
                    selection,
                    *trading_date,
                    selection.session_kinds.iter().copied(),
                )?;
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

impl fmt::Debug for RunConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunConfig")
            .field("effective", &self.effective)
            .field("selections", &self.selections)
            .field("strategy_metadata", &self.strategy_metadata)
            .finish_non_exhaustive()
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

pub fn strategy_bootstrap(path: impl AsRef<Path>) -> Result<StrategyBootstrapConfig, ConfigError> {
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    reject_secrets(&bytes)?;
    let raw: BootstrapFileConfig =
        serde_yaml::from_slice(&bytes).map_err(|error| ConfigError::Yaml(error.to_string()))?;
    let reference = raw
        .strategy_reference
        .map(|reference| validate_strategy_reference(path, reference))
        .transpose()?;
    Ok(StrategyBootstrapConfig {
        reference,
        simulation: parse_simulation(raw.simulation)?,
    })
}

pub fn load(path: impl AsRef<Path>, registry: &StrategyRegistry) -> Result<RunConfig, ConfigError> {
    let path = path.as_ref();
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
    if let Some(reference) = raw.strategy_reference.clone() {
        validate_strategy_reference(path, reference)?;
    }
    resolve(raw, registry)
}

pub fn plan(config: &RunConfig) -> Result<PlanBundle, ConfigError> {
    let mut partitions = Vec::new();
    let mut session_plans = Vec::new();
    let mut replay_bindings = Vec::new();
    let mut replay_ready = true;
    let cache_catalog = PartitionCacheCatalog::new(config.effective.data_root());
    for selection in &config.selections {
        for trading_date in config.effective.trading_dates() {
            let session_plan = session_plan(
                selection,
                *trading_date,
                selection.session_kinds.iter().copied(),
            )?;
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
    let execution = ExecutionPlan::new(config.effective.clone(), partitions, Vec::new())
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

fn resolve(raw: FileConfig, registry: &StrategyRegistry) -> Result<RunConfig, ConfigError> {
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
    let date_values = trading_dates.clone();
    let economics = raw
        .instrument_economics
        .into_iter()
        .map(parse_economics)
        .collect::<Result<Vec<_>, _>>()?;
    validate_reference_economics(&selections, &economics)?;
    let raw_parameters = parse_strategy_parameters(&raw.strategy.parameters)?;
    let resolved_strategy = registry.resolve(
        &raw.strategy.id,
        &raw.strategy.version,
        &raw_parameters,
        &instruments,
        &sessions,
    )?;
    let (strategy_instance, strategy_metadata) = resolved_strategy.into_parts();
    let strategy = StrategyBinding::new(
        strategy_metadata.definition().identity()?,
        strategy_metadata.parameters().checksum(),
        strategy_metadata.declaration().clone(),
    );
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
        strategy: Some(strategy_instance),
        strategy_metadata,
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
    let session_profile = raw
        .session_profile
        .as_deref()
        .map(parse_session_profile)
        .transpose()?;
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
    if session_profile.is_some_and(|profile| !profile_matches(profile, market, kind)) {
        return Err(ConfigError::Invalid("universe.instruments.session_profile"));
    }
    if matches!(kind, InstrumentKind::Warrant | InstrumentKind::Option) && reference.is_none() {
        return Err(ConfigError::Invalid("universe.instruments.reference"));
    }
    Ok(InstrumentSelection {
        instrument,
        session_kinds: session_kinds.into_boxed_slice(),
        kind,
        session_profile,
        reference,
    })
}

fn session_plan(
    selection: &InstrumentSelection,
    trading_date: TradingDate,
    session_kinds: impl IntoIterator<Item = SessionKind>,
) -> Result<SessionPlan, ConfigError> {
    match selection.session_profile {
        Some(profile) => {
            SessionPlan::with_profile(&selection.instrument, trading_date, profile, session_kinds)
        }
        None => SessionPlan::for_instrument_kind(
            &selection.instrument,
            selection.kind,
            trading_date,
            session_kinds,
        ),
    }
    .map_err(ConfigError::SessionPlan)
}

const fn profile_matches(
    profile: SessionProfileId,
    market: MarketId,
    kind: InstrumentKind,
) -> bool {
    matches!(
        (profile, market, kind),
        (
            SessionProfileId::TwseRegular,
            MarketId::Twse,
            InstrumentKind::Equity | InstrumentKind::Warrant
        ) | (
            SessionProfileId::TpexRegular,
            MarketId::Tpex,
            InstrumentKind::Equity | InstrumentKind::Warrant
        ) | (
            SessionProfileId::TaifexIndexFutures
                | SessionProfileId::TaifexStockFutures
                | SessionProfileId::TaifexStockFuturesRegularOnly,
            MarketId::Taifex,
            InstrumentKind::Future
        ) | (
            SessionProfileId::TaifexIndexOptions,
            MarketId::Taifex,
            InstrumentKind::Option
        )
    )
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

fn parse_session_profile(value: &str) -> Result<SessionProfileId, ConfigError> {
    match value {
        "twse_regular" => Ok(SessionProfileId::TwseRegular),
        "tpex_regular" => Ok(SessionProfileId::TpexRegular),
        "taifex_index_futures" => Ok(SessionProfileId::TaifexIndexFutures),
        "taifex_stock_futures" => Ok(SessionProfileId::TaifexStockFutures),
        "taifex_stock_futures_regular_only" => Ok(SessionProfileId::TaifexStockFuturesRegularOnly),
        "taifex_index_options" => Ok(SessionProfileId::TaifexIndexOptions),
        _ => Err(ConfigError::Invalid("universe.instruments.session_profile")),
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
    let mut simulation = run_planner::SimulationConfig::new(
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
    ));
    match (raw.execution_policy.as_str(), raw.scheduled_execution) {
        ("subsequent_event_v1", None) => {}
        ("scheduled_visible_depth_v1", Some(scheduled)) => {
            simulation = simulation.with_scheduled_execution(ScheduledExecutionConfig::new(
                scheduled.depth_levels,
                scheduled.max_stale_ms,
            ));
        }
        ("subsequent_event_v1", Some(_)) | ("scheduled_visible_depth_v1", None) => {
            return Err(ConfigError::Invalid("simulation.scheduled_execution"));
        }
        _ => return Err(ConfigError::Invalid("simulation.execution_policy")),
    }
    let instrument_charges = raw
        .instrument_charges
        .into_iter()
        .map(parse_instrument_charges)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(simulation.with_instrument_charges(instrument_charges))
}

fn parse_instrument_charges(
    raw: InstrumentChargeFileConfig,
) -> Result<InstrumentChargeConfig, ConfigError> {
    let instrument = InstrumentId::new(
        parse_market(&raw.market)?,
        Symbol::new(raw.symbol).map_err(|error| ConfigError::Value(error.to_string()))?,
    );
    let fee = parse_charge(raw.fee)?;
    let tax = parse_charge(raw.tax)?;
    let day_trade_tax = raw
        .day_trade_tax
        .map(|day_trade| -> Result<DayTradeTaxConfig, ConfigError> {
            require(
                day_trade.matching == "same_account_instrument_trading_date_fifo",
                "simulation.instrument_charges.day_trade_tax.matching",
            )?;
            let eligible_dates = day_trade
                .eligible_dates
                .iter()
                .map(|value| {
                    TradingDate::parse(value).map_err(|error| ConfigError::Value(error.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DayTradeTaxConfig::new(
                parse_charge(day_trade.charge)?,
                DayTradeMatchingConfig::SameAccountInstrumentTradingDateFifo,
                day_trade.timezone_offset_minutes,
                eligible_dates,
                day_trade.eligibility_required,
                TradingDate::parse(&day_trade.valid_through)
                    .map_err(|error| ConfigError::Value(error.to_string()))?,
                day_trade.provenance,
            ))
        })
        .transpose()?;
    Ok(InstrumentChargeConfig::new(
        instrument,
        fee,
        tax,
        day_trade_tax,
    ))
}

fn parse_charge(raw: ChargeFileConfig) -> Result<ChargeConfig, ConfigError> {
    let buy = raw.applicable_sides.iter().any(|side| side == "buy");
    let sell = raw.applicable_sides.iter().any(|side| side == "sell");
    let sides = match (buy, sell) {
        (true, true) => ChargeSides::BuyAndSell,
        (true, false) => ChargeSides::Buy,
        (false, true) => ChargeSides::Sell,
        _ => return Err(ConfigError::Invalid("simulation.charge.applicable_sides")),
    };
    let rounding = match raw.rounding.as_str() {
        "down" => RoundingPolicy::Down,
        "half_up" => RoundingPolicy::HalfUp,
        "up" => RoundingPolicy::Up,
        _ => return Err(ConfigError::Invalid("simulation.charge.rounding")),
    };
    match raw.model.as_str() {
        "configured_rate" if raw.amount_per_unit.is_none() => Ok(ChargeConfig::new(
            decimal(
                raw.rate
                    .as_deref()
                    .ok_or(ConfigError::Invalid("simulation.charge.rate"))?,
            )?,
            sides,
            decimal(raw.minimum.as_deref().unwrap_or("0"))?,
            raw.precision,
            rounding,
            raw.provenance,
        )),
        "fixed_per_unit" if raw.rate.is_none() => Ok(ChargeConfig::fixed_per_unit(
            decimal(
                raw.amount_per_unit
                    .as_deref()
                    .ok_or(ConfigError::Invalid("simulation.charge.amount_per_unit"))?,
            )?,
            sides,
            raw.precision,
            rounding,
            raw.provenance,
        )),
        _ => Err(ConfigError::Invalid("simulation.charge.model")),
    }
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

fn parse_strategy_parameters(
    value: &serde_yaml::Value,
) -> Result<RawStrategyParameters, ConfigError> {
    let mapping = value
        .as_mapping()
        .ok_or(ConfigError::Invalid("strategy.parameters"))?;
    let mut parameters = RawStrategyParameters::new();
    for (key, value) in mapping {
        let key = key
            .as_str()
            .ok_or(ConfigError::Invalid("strategy.parameters field name"))?
            .to_owned();
        let value = match value {
            serde_yaml::Value::Bool(value) => RawStrategyParameter::Bool(*value),
            serde_yaml::Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    RawStrategyParameter::SignedInteger(value)
                } else if let Some(value) = value.as_u64() {
                    RawStrategyParameter::UnsignedInteger(value)
                } else {
                    return Err(ConfigError::Invalid("strategy.parameters numeric value"));
                }
            }
            serde_yaml::Value::String(value) => RawStrategyParameter::String(value.clone()),
            _ => return Err(ConfigError::Invalid("strategy.parameters scalar value")),
        };
        if parameters.insert(key, value).is_some() {
            return Err(ConfigError::Invalid("strategy.parameters duplicate field"));
        }
    }
    Ok(parameters)
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

fn validate_strategy_reference(
    config_path: &Path,
    raw: StrategyReferenceFileConfig,
) -> Result<StrategyReferenceInput, ConfigError> {
    if raw.schema_version != 1 || raw.path.as_os_str().is_empty() {
        return Err(ConfigError::Invalid("strategy_reference"));
    }
    let checksum = decode_hex(&raw.checksum)?;
    let path = if raw.path.is_absolute() {
        raw.path
    } else {
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(raw.path)
    };
    let bytes = fs::read(&path)?;
    if blake3::hash(&bytes).as_bytes() != &checksum {
        return Err(ConfigError::Invalid("strategy_reference.checksum"));
    }
    Ok(StrategyReferenceInput {
        schema_version: raw.schema_version,
        path,
        checksum: raw.checksum.into_boxed_str(),
    })
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
    #[serde(default)]
    strategy_reference: Option<StrategyReferenceFileConfig>,
    replay: ReplayConfig,
    simulation: SimulationFileConfig,
    instrument_economics: Vec<EconomicsConfig>,
    output: OutputConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrategyReferenceFileConfig {
    schema_version: u16,
    path: PathBuf,
    checksum: String,
}

#[derive(Debug, Deserialize)]
struct BootstrapFileConfig {
    #[serde(default)]
    strategy_reference: Option<StrategyReferenceFileConfig>,
    simulation: SimulationFileConfig,
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
    session_profile: Option<String>,
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
    #[serde(default = "default_execution_policy")]
    execution_policy: String,
    #[serde(default)]
    scheduled_execution: Option<ScheduledExecutionFileConfig>,
    #[serde(default)]
    market_data_latency_ms: u64,
    #[serde(default)]
    order_latency_ms: u64,
    allocation: String,
    slippage: SlippageConfig,
    fee: ChargeFileConfig,
    tax: ChargeFileConfig,
    #[serde(default)]
    instrument_charges: Vec<InstrumentChargeFileConfig>,
    initial_cash: CashConfig,
    position_accounting: String,
    marking: MarkingConfig,
}

fn default_execution_policy() -> String {
    "subsequent_event_v1".to_owned()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduledExecutionFileConfig {
    depth_levels: u8,
    max_stale_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstrumentChargeFileConfig {
    market: String,
    symbol: String,
    fee: ChargeFileConfig,
    tax: ChargeFileConfig,
    #[serde(default)]
    day_trade_tax: Option<DayTradeTaxFileConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DayTradeTaxFileConfig {
    charge: ChargeFileConfig,
    matching: String,
    timezone_offset_minutes: i32,
    eligible_dates: Vec<String>,
    eligibility_required: bool,
    valid_through: String,
    provenance: String,
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
    #[serde(default)]
    rate: Option<String>,
    #[serde(default)]
    amount_per_unit: Option<String>,
    applicable_sides: Vec<String>,
    #[serde(default)]
    minimum: Option<String>,
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
    Strategy(StrategyRegistryError),
    StrategyAlreadyTaken,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { expected, actual } => write!(
                formatter,
                "unsupported config_version {actual}; expected {expected}; legacy config_version 1 is not supported, upgrade the config"
            ),
            Self::Strategy(error) => write!(formatter, "{error}"),
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

impl From<StrategyRegistryError> for ConfigError {
    fn from(error: StrategyRegistryError) -> Self {
        Self::Strategy(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> StrategyRegistry {
        let mut registry = StrategyRegistry::new();
        registry
            .register(strategy_api::AcceptanceStrategyFactory::new().unwrap())
            .unwrap();
        registry
            .register(example_strategy::PriceThresholdBuyOnceFactory::new().unwrap())
            .unwrap();
        registry
    }

    fn example_source(parameters: &str) -> String {
        fs::read_to_string(fixture())
            .unwrap()
            .replace(
                "id: acceptance.multi-market",
                "id: example.price-threshold-buy-once",
            )
            .replace("parameters: {}", parameters)
    }

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/config.yaml")
    }

    #[test]
    fn run_config_materializes_the_representative_twse_profile() {
        let config = load(fixture(), &registry()).unwrap();
        assert_eq!(config.selections().len(), 1);
        assert_eq!(config.effective().universe().len(), 1);
        assert_eq!(config.effective().session_kinds(), &[SessionKind::Regular]);
        assert_eq!(
            config.effective().simulation().latency(),
            run_planner::LatencyConfig::new(0, 0)
        );
        let bundle = plan(&config).unwrap();
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

        let config = load(path, &registry()).unwrap();
        assert_eq!(
            config.effective().simulation().latency(),
            run_planner::LatencyConfig::new(12, 34)
        );
    }

    #[test]
    fn run_config_accepts_an_explicit_compatible_session_profile() {
        let source = fs::read_to_string(fixture())
            .unwrap()
            .replace("market: twse", "market: taifex")
            .replace("symbol: \"2330\"", "symbol: \"CDFG6\"")
            .replace(
                "      session_kinds: [regular]",
                "      instrument_kind: future\n      session_profile: taifex_stock_futures\n      session_kinds: [regular]",
            )
            .replace("quantity_unit: trading_unit", "quantity_unit: contract")
            .replace("units_per_trading_unit: 1000", "units_per_trading_unit: 1");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("explicit-profile.yaml");
        fs::write(&path, source).unwrap();

        let config = load(path, &registry()).unwrap();
        assert_eq!(
            config.selections()[0].session_profile(),
            Some(SessionProfileId::TaifexStockFutures)
        );
        assert!(config.partition_keys().is_ok());
    }

    #[test]
    fn run_config_requires_scheduled_parameters_with_scheduled_policy() {
        let scheduled_source = fs::read_to_string(fixture()).unwrap().replace(
            "simulation:\n",
            "simulation:\n  execution_policy: scheduled_visible_depth_v1\n  scheduled_execution: { depth_levels: 5, max_stale_ms: 1000 }\n",
        );
        let directory = tempfile::tempdir().unwrap();
        let scheduled_path = directory.path().join("scheduled.yaml");
        fs::write(&scheduled_path, scheduled_source).unwrap();
        let config = load(&scheduled_path, &registry()).unwrap();
        assert_eq!(
            config.effective().simulation().scheduled_execution(),
            Some(ScheduledExecutionConfig::new(5, 1_000))
        );

        let missing_path = directory.path().join("missing-scheduled.yaml");
        fs::write(
            &missing_path,
            fs::read_to_string(fixture()).unwrap().replace(
                "simulation:\n",
                "simulation:\n  execution_policy: scheduled_visible_depth_v1\n",
            ),
        )
        .unwrap();
        assert!(matches!(
            load(&missing_path, &registry()),
            Err(ConfigError::Invalid("simulation.scheduled_execution"))
        ));
    }

    #[test]
    fn run_config_materializes_per_instrument_day_trade_tax() {
        let block = r#"  instrument_charges:
    - market: twse
      symbol: "2330"
      fee:
        model: configured_rate
        rate: "0.001425"
        applicable_sides: [buy, sell]
        minimum: "0"
        precision: 0
        rounding: down
        provenance: "broker schedule"
      tax:
        model: configured_rate
        rate: "0.003"
        applicable_sides: [sell]
        minimum: "0"
        precision: 0
        rounding: down
        provenance: "MOF ordinary stock tax"
      day_trade_tax:
        charge:
          model: configured_rate
          rate: "0.0015"
          applicable_sides: [sell]
          minimum: "0"
          precision: 0
          rounding: down
          provenance: "MOF reduced day-trade tax"
        matching: same_account_instrument_trading_date_fifo
        timezone_offset_minutes: 480
        eligible_dates: ["2026-07-27"]
        eligibility_required: true
        valid_through: "2027-12-31"
        provenance: "TWSE day-trading eligibility"
"#;
        let source = fs::read_to_string(fixture())
            .unwrap()
            .replace("  initial_cash:", &format!("{block}  initial_cash:"));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("day-trade-tax.yaml");
        fs::write(&path, source).unwrap();

        let config = load(path, &registry()).unwrap();
        let charges = config
            .effective()
            .simulation()
            .charges_for(config.effective().universe().first().unwrap())
            .unwrap();
        assert_eq!(charges.tax().rate(), Decimal::parse("0.003").unwrap());
        let day_trade = charges.day_trade_tax().unwrap();
        assert_eq!(day_trade.charge().rate(), Decimal::parse("0.0015").unwrap());
        assert!(day_trade.is_eligible(TradingDate::parse("2026-07-27").unwrap()));
        assert_eq!(config.effective().canonical_version(), 4);
    }

    #[test]
    fn run_config_rejects_embedded_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secret.yaml");
        fs::write(&path, "config_version: 2\napi_key: forbidden\n").unwrap();
        assert!(matches!(
            load(path, &registry()),
            Err(ConfigError::SecretField)
        ));
    }

    #[test]
    fn run_config_rejects_legacy_schema_with_upgrade_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.yaml");
        fs::write(&path, "config_version: 1\n").unwrap();

        let error = load(path, &registry()).unwrap_err();
        assert!(matches!(
            &error,
            ConfigError::UnsupportedVersion {
                expected: RUN_CONFIG_VERSION,
                actual: 1
            }
        ));
        assert!(error.to_string().contains("upgrade the config"));
    }

    #[test]
    fn strategy_resolution_uses_factory_identity_before_planning() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("example.yaml");
        fs::write(
            &path,
            example_source("parameters: { entry_price: \"101\" }"),
        )
        .unwrap();
        let config = load(&path, &registry()).unwrap();
        let identity = config.effective().strategy().identity();
        assert_eq!(identity.strategy_id(), "example.price-threshold-buy-once");
        assert_eq!(
            identity.binary_identity().algorithm(),
            "strategy-source-blake3"
        );
    }

    #[test]
    fn default_materialization_stabilizes_plan_identity_and_values_change_it() {
        let directory = tempfile::tempdir().unwrap();
        let omitted_path = directory.path().join("omitted.yaml");
        let explicit_path = directory.path().join("explicit.yaml");
        let changed_path = directory.path().join("changed.yaml");
        fs::write(
            &omitted_path,
            example_source("parameters: { entry_price: \"101\" }"),
        )
        .unwrap();
        fs::write(
            &explicit_path,
            example_source("parameters: { quantity: 1, entry_price: \"101.0\" }"),
        )
        .unwrap();
        fs::write(
            &changed_path,
            example_source("parameters: { entry_price: \"101\", quantity: 2 }"),
        )
        .unwrap();
        let omitted = load(&omitted_path, &registry()).unwrap();
        let explicit = load(&explicit_path, &registry()).unwrap();
        let changed = load(&changed_path, &registry()).unwrap();
        assert_eq!(
            plan(&omitted).unwrap().execution.identity(),
            plan(&explicit).unwrap().execution.identity()
        );
        assert_ne!(
            plan(&omitted).unwrap().execution.identity(),
            plan(&changed).unwrap().execution.identity()
        );
    }

    #[test]
    fn acceptance_rejects_unknown_parameters_during_load() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid.yaml");
        let source = fs::read_to_string(fixture())
            .unwrap()
            .replace("parameters: {}", "parameters: { unexpected: true }");
        fs::write(&path, source).unwrap();
        assert!(matches!(
            load(&path, &registry()),
            Err(ConfigError::Strategy(
                StrategyRegistryError::UnknownParameter(field)
            )) if field == "unexpected"
        ));
    }

    #[test]
    fn strategy_bootstrap_resolves_and_verifies_reference_relative_to_config() {
        let directory = tempfile::tempdir().unwrap();
        let reference_path = directory.path().join("reference.yaml");
        let reference = b"schema_version: 1\ndays: []\n";
        fs::write(&reference_path, reference).unwrap();
        let checksum = blake3::hash(reference)
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let config_path = directory.path().join("run.yaml");
        let source = fs::read_to_string(fixture())
            .unwrap()
            .replace("market_data_latency_ms: 0", "market_data_latency_ms: 200")
            .replace("order_latency_ms: 0", "order_latency_ms: 300");
        fs::write(
            &config_path,
            format!(
                "strategy_reference:\n  schema_version: 1\n  path: reference.yaml\n  checksum: \"{checksum}\"\n{source}"
            ),
        )
        .unwrap();
        let bootstrap = strategy_bootstrap(&config_path).unwrap();
        assert_eq!(bootstrap.order_latency_ms(), 300);
        assert_eq!(bootstrap.market_data_latency_ms(), 200);
        assert_eq!(bootstrap.reference().unwrap().path(), reference_path);

        fs::write(&reference_path, b"changed").unwrap();
        assert!(matches!(
            strategy_bootstrap(&config_path),
            Err(ConfigError::Invalid("strategy_reference.checksum"))
        ));
    }
}
