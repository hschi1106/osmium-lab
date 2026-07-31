use std::{
    collections::BTreeSet,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use data_sync::{PartitionCacheCatalog, PartitionedSourceRepository};
use market_types::{Decimal, InstrumentId, MarketId, QuantityUnit, Symbol, TradingDate};
use replay_engine::{ReplayPlan, ReplayStreamBinding, StableStreamDescriptorId};
use run_planner::{
    CacheIdentity, CachePolicy, CacheState, ChargeConfig, ChargeSides, Currency, CurrencyAmount,
    EffectiveRunConfig, ExecutionPlan, FillEvidence, FillModelConfig, InstrumentEconomicsConfig,
    MarkingPolicyConfig, OutputPolicy, PlannedPartition, PositionAccountingConfig,
    QuantityAllocationConfig, QuantityEvidence, ReplayDataPolicy, RoundingPolicy, RunConfig,
    SessionPlan, SessionPlanError, SlippageModelConfig, SourceId, SourcePartitionKey, SourcePolicy,
    SourceState, StrategyBinding,
};
use serde::Deserialize;
use strategy_api::{
    BinaryIdentity, CanonicalParamsChecksum, SessionKind, StrategyDeclaration, StrategyIdentity,
};

pub const M3_CONFIG_VERSION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M3InstrumentSelection {
    instrument: InstrumentId,
    session_kinds: Box<[SessionKind]>,
}

impl M3InstrumentSelection {
    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn session_kinds(&self) -> &[SessionKind] {
        &self.session_kinds
    }
}

#[derive(Debug, Clone)]
pub struct M3Config {
    effective: EffectiveRunConfig,
    selections: Box<[M3InstrumentSelection]>,
}

impl M3Config {
    #[must_use]
    pub const fn effective(&self) -> &EffectiveRunConfig {
        &self.effective
    }

    #[must_use]
    pub const fn selections(&self) -> &[M3InstrumentSelection] {
        &self.selections
    }

    pub fn session_plan_for(&self, key: &SourcePartitionKey) -> Result<SessionPlan, M3ConfigError> {
        SessionPlan::for_instrument(
            key.instrument(),
            key.trading_date(),
            key.session_kinds().iter().copied(),
        )
        .map_err(M3ConfigError::SessionPlan)
    }

    pub fn partition_keys(&self) -> Result<Box<[SourcePartitionKey]>, M3ConfigError> {
        let mut keys = Vec::new();
        for selection in &self.selections {
            for trading_date in self.effective.trading_dates() {
                let session_plan = SessionPlan::for_instrument(
                    &selection.instrument,
                    *trading_date,
                    selection.session_kinds.iter().copied(),
                )
                .map_err(M3ConfigError::SessionPlan)?;
                keys.push(
                    SourcePartitionKey::new(
                        SourceId::TeralionFeedArchive,
                        selection.instrument.clone(),
                        *trading_date,
                        selection.session_kinds.iter().copied(),
                        session_plan.identity(),
                    )
                    .map_err(|error| M3ConfigError::Value(error.to_string()))?,
                );
            }
        }
        Ok(keys.into_boxed_slice())
    }
}

#[derive(Debug)]
pub struct M3PlanBundle {
    pub execution: ExecutionPlan,
    pub session_plans: Box<[SessionPlan]>,
    pub replay: Option<ReplayPlan>,
}

pub fn config_version(path: impl AsRef<Path>) -> Result<u16, M3ConfigError> {
    let bytes = fs::read(path)?;
    let raw: serde_yaml::Value =
        serde_yaml::from_slice(&bytes).map_err(|error| M3ConfigError::Yaml(error.to_string()))?;
    raw.get("config_version")
        .and_then(serde_yaml::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(M3ConfigError::Invalid("config_version"))
}

pub fn load(path: impl AsRef<Path>) -> Result<M3Config, M3ConfigError> {
    let bytes = fs::read(path)?;
    reject_secrets(&bytes)?;
    let raw: FileConfig =
        serde_yaml::from_slice(&bytes).map_err(|error| M3ConfigError::Yaml(error.to_string()))?;
    resolve(raw)
}

pub fn plan(config: M3Config) -> Result<M3PlanBundle, M3ConfigError> {
    let mut partitions = Vec::new();
    let mut session_plans = Vec::new();
    let mut replay_bindings = Vec::new();
    let mut replay_ready = true;
    let cache_catalog = PartitionCacheCatalog::new(config.effective.data_root());
    for selection in &config.selections {
        for trading_date in config.effective.trading_dates() {
            let session_plan = SessionPlan::for_instrument(
                &selection.instrument,
                *trading_date,
                selection.session_kinds.iter().copied(),
            )
            .map_err(M3ConfigError::SessionPlan)?;
            let key = SourcePartitionKey::new(
                SourceId::TeralionFeedArchive,
                selection.instrument.clone(),
                *trading_date,
                selection.session_kinds.iter().copied(),
                session_plan.identity(),
            )
            .map_err(|error| M3ConfigError::Value(error.to_string()))?;
            let repository =
                PartitionedSourceRepository::new(config.effective.data_root(), key.clone())
                    .map_err(|error| M3ConfigError::Value(error.to_string()))?;
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
        .map_err(|error| M3ConfigError::Value(error.to_string()))?;
    let replay = if replay_ready {
        Some(
            ReplayPlan::new_multi(*execution.identity().as_bytes(), replay_bindings)
                .map_err(|error| M3ConfigError::Value(error.to_string()))?,
        )
    } else {
        None
    };
    Ok(M3PlanBundle {
        execution,
        session_plans: session_plans.into_boxed_slice(),
        replay,
    })
}

fn cache_state(
    catalog: &PartitionCacheCatalog,
    key: &SourcePartitionKey,
    inspection: Option<&data_sync::VerificationReport>,
) -> Result<CacheState, M3ConfigError> {
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

fn resolve(raw: FileConfig) -> Result<M3Config, M3ConfigError> {
    require(raw.config_version == M3_CONFIG_VERSION, "config_version")?;
    require(raw.data.source == "teralion", "data.source")?;
    require(
        raw.data.cache_policy == "reuse_or_rebuild",
        "data.cache_policy",
    )?;
    if raw.universe.trading_dates.is_empty() {
        return Err(M3ConfigError::Invalid("universe.trading_dates"));
    }
    if raw.universe.instruments.is_empty() {
        return Err(M3ConfigError::Invalid("universe.instruments"));
    }

    let trading_dates = raw
        .universe
        .trading_dates
        .iter()
        .map(|value| {
            value
                .parse::<TradingDate>()
                .map_err(|error| M3ConfigError::Value(error.to_string()))
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
        return Err(M3ConfigError::Invalid("universe.instruments"));
    }
    let sessions = selections
        .iter()
        .flat_map(|selection| selection.session_kinds.iter().copied())
        .collect::<BTreeSet<_>>();
    let sessions = sessions.into_iter().collect::<Vec<_>>();
    let declaration = StrategyDeclaration::new(instruments.clone(), sessions.clone())
        .map_err(|error| M3ConfigError::Value(error.to_string()))?;
    let params_checksum = params_checksum(&raw.strategy.parameters)?;
    let binary_digest = strategy_digest(&raw.strategy.id, &raw.strategy.version, params_checksum);
    let strategy_identity = StrategyIdentity::new(
        raw.strategy.id,
        raw.strategy.version,
        BinaryIdentity::new("config", binary_digest.to_vec())
            .map_err(|error| M3ConfigError::Value(error.to_string()))?,
    )
    .map_err(|error| M3ConfigError::Value(error.to_string()))?;
    let strategy = StrategyBinding::new(strategy_identity, params_checksum, declaration);
    let date_values = trading_dates.clone();
    let economics = raw
        .instrument_economics
        .into_iter()
        .map(parse_economics)
        .collect::<Result<Vec<_>, _>>()?;
    let effective = EffectiveRunConfig::resolve(RunConfig {
        // The effective schema is unchanged; M3 is the file-format version.
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
    .map_err(|error| M3ConfigError::Value(error.to_string()))?;
    Ok(M3Config {
        effective,
        selections: selections.into_boxed_slice(),
    })
}

fn parse_selection(raw: &InstrumentConfig) -> Result<M3InstrumentSelection, M3ConfigError> {
    let instrument = InstrumentId::new(
        parse_market(&raw.market)?,
        Symbol::new(raw.symbol.clone()).map_err(|error| M3ConfigError::Value(error.to_string()))?,
    );
    let mut session_kinds = raw
        .session_kinds
        .iter()
        .map(|value| parse_session(value))
        .collect::<Result<Vec<_>, _>>()?;
    session_kinds.sort_unstable();
    session_kinds.dedup();
    if session_kinds.is_empty() {
        return Err(M3ConfigError::Invalid("universe.instruments.session_kinds"));
    }
    Ok(M3InstrumentSelection {
        instrument,
        session_kinds: session_kinds.into_boxed_slice(),
    })
}

fn parse_economics(raw: EconomicsConfig) -> Result<InstrumentEconomicsConfig, M3ConfigError> {
    Ok(InstrumentEconomicsConfig::new(
        InstrumentId::new(
            parse_market(&raw.market)?,
            Symbol::new(raw.symbol).map_err(|error| M3ConfigError::Value(error.to_string()))?,
        ),
        match raw.quantity_unit.as_str() {
            "share" => QuantityUnit::Share,
            "trading_unit" => QuantityUnit::TradingUnit,
            "contract" => QuantityUnit::Contract,
            _ => return Err(M3ConfigError::Invalid("instrument_economics.quantity_unit")),
        },
        raw.units_per_trading_unit,
        parse_currency(&raw.currency)?,
        decimal(&raw.multiplier)?,
        raw.provenance,
    ))
}

fn parse_simulation(
    raw: SimulationFileConfig,
) -> Result<run_planner::SimulationConfig, M3ConfigError> {
    let fill = FillModelConfig::new(
        match raw.fill.evidence.as_str() {
            "top_of_book" => FillEvidence::TopOfBook,
            "trade_print" => FillEvidence::TradePrint,
            _ => return Err(M3ConfigError::Invalid("simulation.fill.evidence")),
        },
        match raw.fill.quantity.as_str() {
            "unlimited" => QuantityEvidence::Unlimited,
            "observed" => QuantityEvidence::Observed,
            _ => return Err(M3ConfigError::Invalid("simulation.fill.quantity")),
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
    ))
}

fn parse_charge(raw: ChargeFileConfig) -> Result<ChargeConfig, M3ConfigError> {
    require(raw.model == "configured_rate", "simulation.charge.model")?;
    let buy = raw.applicable_sides.iter().any(|side| side == "buy");
    let sell = raw.applicable_sides.iter().any(|side| side == "sell");
    let sides = match (buy, sell) {
        (true, true) => ChargeSides::BuyAndSell,
        (true, false) => ChargeSides::Buy,
        (false, true) => ChargeSides::Sell,
        _ => return Err(M3ConfigError::Invalid("simulation.charge.applicable_sides")),
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
            _ => return Err(M3ConfigError::Invalid("simulation.charge.rounding")),
        },
        raw.provenance,
    ))
}

fn parse_market(value: &str) -> Result<MarketId, M3ConfigError> {
    match value {
        "twse" => Ok(MarketId::Twse),
        "tpex" => Ok(MarketId::Tpex),
        "taifex" => Ok(MarketId::Taifex),
        _ => Err(M3ConfigError::Invalid("market")),
    }
}

fn parse_session(value: &str) -> Result<SessionKind, M3ConfigError> {
    match value {
        "regular" => Ok(SessionKind::Regular),
        "after_hours" => Ok(SessionKind::AfterHours),
        _ => Err(M3ConfigError::Invalid("session_kinds")),
    }
}

fn parse_currency(value: &str) -> Result<Currency, M3ConfigError> {
    match value {
        "TWD" | "twd" => Ok(Currency::Twd),
        _ => Err(M3ConfigError::Invalid("currency")),
    }
}

fn parse_source_policy(value: &str) -> Result<SourcePolicy, M3ConfigError> {
    match value {
        "strict" => Ok(SourcePolicy::Strict),
        "explicit_degraded" => Ok(SourcePolicy::ExplicitDegraded),
        _ => Err(M3ConfigError::Invalid("data.source_policy")),
    }
}

fn parse_replay_policy(value: &str) -> Result<ReplayDataPolicy, M3ConfigError> {
    match value {
        "strict" => Ok(ReplayDataPolicy::Strict),
        "explicit_degraded" => Ok(ReplayDataPolicy::ExplicitDegraded),
        _ => Err(M3ConfigError::Invalid("replay.data_policy")),
    }
}

fn parse_output_policy(value: &str) -> Result<OutputPolicy, M3ConfigError> {
    if value == "create_new" {
        Ok(OutputPolicy::CreateNew)
    } else {
        Err(M3ConfigError::Invalid("output.publication"))
    }
}

fn decimal(value: &str) -> Result<Decimal, M3ConfigError> {
    Decimal::parse(value).map_err(|error| M3ConfigError::Value(error.to_string()))
}

fn params_checksum(value: &serde_yaml::Value) -> Result<CanonicalParamsChecksum, M3ConfigError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| M3ConfigError::Value(error.to_string()))?;
    let mut canonical = Vec::with_capacity(6 + bytes.len());
    canonical.extend_from_slice(b"OSSP");
    canonical.extend_from_slice(&1_u16.to_be_bytes());
    canonical.extend_from_slice(
        &u32::try_from(bytes.len())
            .map_err(|_| M3ConfigError::Value("strategy parameters too large".to_owned()))?
            .to_be_bytes(),
    );
    canonical.extend_from_slice(&bytes);
    Ok(CanonicalParamsChecksum::from_bytes(
        *blake3::hash(&canonical).as_bytes(),
    ))
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

fn decode_hex(value: &str) -> Result<[u8; 32], M3ConfigError> {
    if value.len() != 64 {
        return Err(M3ConfigError::Invalid("checksum"));
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = nibble(pair[0]).ok_or(M3ConfigError::Invalid("checksum"))?;
        let low = nibble(pair[1]).ok_or(M3ConfigError::Invalid("checksum"))?;
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

fn reject_secrets(bytes: &[u8]) -> Result<(), M3ConfigError> {
    let lower = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if ["api_key", "authorization", "bearer", "cookie", "credential"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        Err(M3ConfigError::SecretField)
    } else {
        Ok(())
    }
}

fn require(condition: bool, field: &'static str) -> Result<(), M3ConfigError> {
    if condition {
        Ok(())
    } else {
        Err(M3ConfigError::Invalid(field))
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
pub enum M3ConfigError {
    Io(std::io::Error),
    Yaml(String),
    SecretField,
    Invalid(&'static str),
    Value(String),
    SessionPlan(SessionPlanError),
}

impl fmt::Display for M3ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for M3ConfigError {}

impl From<std::io::Error> for M3ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/m3-taifex-multi.yaml")
    }

    #[test]
    fn m3_config_materializes_three_taifex_profiles_and_twse() {
        let config = load(fixture()).unwrap();
        assert_eq!(config.selections().len(), 4);
        assert_eq!(config.effective().universe().len(), 4);
        assert_eq!(
            config.effective().session_kinds(),
            &[SessionKind::Regular, SessionKind::AfterHours]
        );
        let bundle = plan(config).unwrap();
        assert_eq!(bundle.execution.partitions().len(), 4);
        assert_eq!(bundle.session_plans.len(), 4);
        assert!(
            bundle
                .execution
                .partitions()
                .iter()
                .any(|partition| partition.key().session_kinds() == [SessionKind::Regular])
        );
    }

    #[test]
    fn m3_config_rejects_embedded_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secret.yaml");
        fs::write(&path, "config_version: 2\napi_key: forbidden\n").unwrap();
        assert!(matches!(load(path), Err(M3ConfigError::SecretField)));
    }
}
