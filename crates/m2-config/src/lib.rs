use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use data_sync::{CACHE_FORMAT_VERSION, CacheDescriptor, LocalSourceRepository};
use market_state::SessionSegmentId;
use market_types::{Decimal, InstrumentId, MarketId, MatchTime, QuantityUnit, Symbol, TradingDate};
use replay_engine::{ReplayPlan, ReplayStreamBinding, StableStreamDescriptorId};
use run_planner::{
    CacheIdentity, CacheState, ChargeConfig, ChargeSides, Currency, CurrencyAmount,
    EffectiveRunConfig, ExecutionPlan, FillEvidence, FillModelConfig, InstrumentEconomicsConfig,
    MarkingPolicyConfig, OutputPolicy, PlannedPartition, PositionAccountingConfig,
    QuantityAllocationConfig, QuantityEvidence, ReplayDataPolicy, RoundingPolicy, RunConfig,
    SessionPlan, SessionPlanIdentity, SimulationConfig, SlippageModelConfig, SourceId,
    SourcePartitionKey, SourcePolicy, StrategyBinding,
};
use serde::Deserialize;
use strategy_api::{
    CanonicalParamsChecksum, M2_ACCEPTANCE_STRATEGY_ID, M2_ACCEPTANCE_STRATEGY_VERSION,
    M2AcceptanceStrategy, SessionKind, SessionSegment, Strategy,
};

pub const M2_CONFIG_VERSION: u16 = 1;

#[derive(Debug, Clone)]
pub struct MaterializedSession {
    pub segment: SessionSegment,
    pub replay_start: MatchTime,
    pub replay_end_exclusive: MatchTime,
    pub identity: SessionPlanIdentity,
}

#[derive(Debug)]
pub struct M2PlanBundle {
    pub execution: ExecutionPlan,
    pub replay: Option<ReplayPlan>,
    pub session: MaterializedSession,
    pub cache_path: Option<PathBuf>,
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
    market: String,
    trading_dates: Vec<String>,
    symbols: Vec<String>,
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

pub fn load(path: impl AsRef<Path>) -> Result<EffectiveRunConfig, M2ConfigError> {
    let bytes = fs::read(path)?;
    let lower = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
    if ["api_key", "authorization", "bearer", "cookie", "credential"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return Err(M2ConfigError::SecretField);
    }
    let raw: FileConfig =
        serde_yaml::from_slice(&bytes).map_err(|error| M2ConfigError::Yaml(error.to_string()))?;
    resolve(raw)
}

fn resolve(raw: FileConfig) -> Result<EffectiveRunConfig, M2ConfigError> {
    require(raw.config_version == M2_CONFIG_VERSION, "config_version")?;
    require(raw.data.source == "teralion", "data.source")?;
    require(
        raw.data.cache_policy == "reuse_or_rebuild",
        "data.cache_policy",
    )?;
    require(raw.universe.market == "twse", "universe.market")?;
    require(raw.universe.symbols == ["2330"], "universe.symbols")?;
    require(
        raw.universe.trading_dates == ["2026-07-27"],
        "universe.trading_dates",
    )?;
    require(
        raw.universe.session_kinds == ["regular"],
        "universe.session_kinds",
    )?;
    require(raw.strategy.id == M2_ACCEPTANCE_STRATEGY_ID, "strategy.id")?;
    require(
        raw.strategy.version == M2_ACCEPTANCE_STRATEGY_VERSION,
        "strategy.version",
    )?;
    require(
        raw.strategy
            .parameters
            .as_mapping()
            .is_some_and(|map| map.is_empty()),
        "strategy.parameters",
    )?;
    let instrument = instrument()?;
    let date = TradingDate::parse(&raw.universe.trading_dates[0])
        .map_err(|error| M2ConfigError::Value(error.to_string()))?;
    let strategy_instance = M2AcceptanceStrategy::new(
        M2AcceptanceStrategy::source_binary_identity()
            .map_err(|error| M2ConfigError::Value(error.to_string()))?,
        instrument.clone(),
    )
    .map_err(|error| M2ConfigError::Value(error.to_string()))?;
    let strategy = StrategyBinding::new(
        strategy_instance.identity().clone(),
        CanonicalParamsChecksum::for_empty_params(),
        strategy_instance.declaration(),
    );
    let fee = charge(raw.simulation.fee)?;
    let tax = charge(raw.simulation.tax)?;
    require(
        raw.simulation.slippage.model == "adverse_fixed_delta",
        "slippage.model",
    )?;
    require(
        raw.simulation.allocation == "acceptance_sequence",
        "allocation",
    )?;
    require(
        raw.simulation.position_accounting == "average_cost_v1",
        "position_accounting",
    )?;
    require(
        raw.simulation.marking.model == "last_observable_mark_v1",
        "marking.model",
    )?;
    require(
        raw.simulation.initial_cash.currency == "TWD",
        "initial_cash.currency",
    )?;
    let simulation = SimulationConfig::new(
        FillModelConfig::new(
            match raw.simulation.fill.evidence.as_str() {
                "top_of_book" => FillEvidence::TopOfBook,
                "trade_print" => FillEvidence::TradePrint,
                _ => return Err(M2ConfigError::Invalid("fill.evidence")),
            },
            match raw.simulation.fill.quantity.as_str() {
                "observed" => QuantityEvidence::Observed,
                "unlimited" => QuantityEvidence::Unlimited,
                _ => return Err(M2ConfigError::Invalid("fill.quantity")),
            },
        ),
        QuantityAllocationConfig::AcceptanceSequence,
        SlippageModelConfig::AdverseFixedDelta {
            delta: nonnegative_decimal(&raw.simulation.slippage.delta)?,
        },
        fee,
        tax,
        CurrencyAmount::new(Currency::Twd, decimal(&raw.simulation.initial_cash.amount)?),
        PositionAccountingConfig::AverageCostV1,
        MarkingPolicyConfig::LastObservableV1 {
            allow_midpoint_fallback: raw.simulation.marking.allow_midpoint_fallback,
        },
    );
    let economics = raw
        .instrument_economics
        .into_iter()
        .map(|value| {
            require(value.market == "twse", "economics.market")?;
            require(value.symbol == "2330", "economics.symbol")?;
            require(
                value.quantity_unit == "trading_unit",
                "economics.quantity_unit",
            )?;
            require(value.currency == "TWD", "economics.currency")?;
            require(value.units_per_trading_unit > 0, "economics.units")?;
            require(!value.provenance.is_empty(), "economics.provenance")?;
            Ok(InstrumentEconomicsConfig::new(
                instrument.clone(),
                QuantityUnit::TradingUnit,
                value.units_per_trading_unit,
                Currency::Twd,
                positive_decimal(&value.multiplier)?,
                value.provenance,
            ))
        })
        .collect::<Result<Vec<_>, M2ConfigError>>()?;
    EffectiveRunConfig::resolve(RunConfig {
        config_version: raw.config_version,
        trading_dates: vec![date],
        universe: vec![instrument],
        session_kinds: vec![SessionKind::Regular],
        strategy,
        data_root: raw.data.data_root,
        source_policy: Some(match raw.data.source_policy.as_str() {
            "strict" => SourcePolicy::Strict,
            "explicit_degraded" => SourcePolicy::ExplicitDegraded,
            _ => return Err(M2ConfigError::Invalid("source_policy")),
        }),
        cache_policy: None,
        replay_data_policy: Some(match raw.replay.data_policy.as_str() {
            "strict" => ReplayDataPolicy::Strict,
            "explicit_degraded" => ReplayDataPolicy::ExplicitDegraded,
            _ => return Err(M2ConfigError::Invalid("replay.data_policy")),
        }),
        simulation,
        instrument_economics: economics,
        output_policy: Some(match raw.output.publication.as_str() {
            "create_new" => OutputPolicy::CreateNew,
            _ => return Err(M2ConfigError::Invalid("output.publication")),
        }),
    })
    .map_err(|error| M2ConfigError::Value(error.to_string()))
}

pub fn materialize_session(
    config: &EffectiveRunConfig,
) -> Result<MaterializedSession, M2ConfigError> {
    let date = config.trading_dates()[0];
    let session_plan = SessionPlan::for_instrument(
        config
            .universe()
            .first()
            .ok_or(M2ConfigError::Invalid("universe"))?,
        date,
        config.session_kinds().iter().copied(),
    )
    .map_err(|error| M2ConfigError::Value(error.to_string()))?;
    let window = session_plan
        .window(SessionKind::Regular)
        .ok_or(M2ConfigError::Invalid("regular session"))?;
    let open = window.open();
    let close = window.close();
    Ok(MaterializedSession {
        segment: SessionSegment::new(
            SessionSegmentId::new("regular")
                .map_err(|error| M2ConfigError::Value(error.to_string()))?,
            SessionKind::Regular,
            date,
            open,
            close,
        )
        .map_err(|error| M2ConfigError::Value(error.to_string()))?,
        replay_start: window.replay_start(),
        replay_end_exclusive: window.replay_end_exclusive(),
        identity: session_plan.identity(),
    })
}

pub fn plan(config: EffectiveRunConfig) -> Result<M2PlanBundle, M2ConfigError> {
    let session = materialize_session(&config)?;
    let key = SourcePartitionKey::new(
        SourceId::TeralionFeedArchive,
        config.universe()[0].clone(),
        config.trading_dates()[0],
        [SessionKind::Regular],
        session.identity,
    )
    .map_err(|error| M2ConfigError::Value(error.to_string()))?;
    let repository = LocalSourceRepository::new(config.data_root());
    let inspection = repository.inspect();
    let (cache_state, cache_path, descriptor) = find_cache(
        config.data_root(),
        inspection
            .report()
            .map(|report| &report.manifest().revision_identity),
    )?;
    let partition = PlannedPartition::classify(key, inspection.state(), cache_state);
    let execution = ExecutionPlan::new(config, vec![partition], Vec::new())
        .map_err(|error| M2ConfigError::Value(error.to_string()))?;
    let replay = match descriptor {
        Some(descriptor) => Some(
            ReplayPlan::new(
                *execution.identity().as_bytes(),
                vec![ReplayStreamBinding::new(
                    StableStreamDescriptorId::from_bytes(
                        *blake3::hash(descriptor.cache_identity.as_bytes()).as_bytes(),
                    ),
                    execution.config().universe()[0].clone(),
                    execution.config().trading_dates()[0],
                    decode_hex(&descriptor.source_revision_identity)?,
                    decode_hex(&descriptor.cache_identity)?,
                )],
            )
            .map_err(|error| M2ConfigError::Value(error.to_string()))?,
        ),
        None => None,
    };
    Ok(M2PlanBundle {
        execution,
        replay,
        session,
        cache_path,
    })
}

fn find_cache(
    data_root: &Path,
    revision: Option<&String>,
) -> Result<(CacheState, Option<PathBuf>, Option<CacheDescriptor>), M2ConfigError> {
    let Some(revision) = revision else {
        return Ok((CacheState::Missing, None, None));
    };
    let root = data_root.join("derived/cache");
    if !root.is_dir() {
        return Ok((CacheState::Missing, None, None));
    }
    let mut paths = fs::read_dir(&root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let descriptor_path = path.join("descriptor.yaml");
        if !descriptor_path.is_file() {
            continue;
        }
        let descriptor: CacheDescriptor = serde_json::from_slice(&fs::read(descriptor_path)?)
            .map_err(|error| M2ConfigError::Value(error.to_string()))?;
        if descriptor.source_revision_identity == *revision
            && descriptor.cache_format_version == CACHE_FORMAT_VERSION
        {
            let identity = CacheIdentity::from_bytes(decode_hex(&descriptor.cache_identity)?);
            return Ok((CacheState::Valid { identity }, Some(path), Some(descriptor)));
        }
    }
    Ok((CacheState::Missing, None, None))
}

fn charge(raw: ChargeFileConfig) -> Result<ChargeConfig, M2ConfigError> {
    require(raw.model == "configured_rate", "charge.model")?;
    require(!raw.provenance.is_empty(), "charge.provenance")?;
    let buy = raw.applicable_sides.iter().any(|side| side == "buy");
    let sell = raw.applicable_sides.iter().any(|side| side == "sell");
    Ok(ChargeConfig::new(
        nonnegative_decimal(&raw.rate)?,
        match (buy, sell) {
            (true, true) => ChargeSides::BuyAndSell,
            (true, false) => ChargeSides::Buy,
            (false, true) => ChargeSides::Sell,
            _ => return Err(M2ConfigError::Invalid("charge.applicable_sides")),
        },
        nonnegative_decimal(&raw.minimum)?,
        raw.precision,
        match raw.rounding.as_str() {
            "down" => RoundingPolicy::Down,
            "half_up" => RoundingPolicy::HalfUp,
            "up" => RoundingPolicy::Up,
            _ => return Err(M2ConfigError::Invalid("charge.rounding")),
        },
        raw.provenance,
    ))
}

fn instrument() -> Result<InstrumentId, M2ConfigError> {
    Ok(InstrumentId::new(
        MarketId::Twse,
        Symbol::new("2330").map_err(|error| M2ConfigError::Value(error.to_string()))?,
    ))
}
fn decimal(value: &str) -> Result<Decimal, M2ConfigError> {
    Decimal::parse(value).map_err(|error| M2ConfigError::Value(error.to_string()))
}
fn nonnegative_decimal(value: &str) -> Result<Decimal, M2ConfigError> {
    let value = decimal(value)?;
    require(value.atoms() >= 0, "nonnegative decimal")?;
    Ok(value)
}
fn positive_decimal(value: &str) -> Result<Decimal, M2ConfigError> {
    let value = decimal(value)?;
    require(value.atoms() > 0, "positive decimal")?;
    Ok(value)
}
fn require(condition: bool, field: &'static str) -> Result<(), M2ConfigError> {
    if condition {
        Ok(())
    } else {
        Err(M2ConfigError::Invalid(field))
    }
}
fn decode_hex(value: &str) -> Result<[u8; 32], M2ConfigError> {
    if value.len() != 64 {
        return Err(M2ConfigError::Invalid("checksum"));
    }
    let mut bytes = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (pair[0] as char)
            .to_digit(16)
            .ok_or(M2ConfigError::Invalid("checksum"))? as u8
            * 16
            + (pair[1] as char)
                .to_digit(16)
                .ok_or(M2ConfigError::Invalid("checksum"))? as u8;
    }
    Ok(bytes)
}

#[derive(Debug)]
pub enum M2ConfigError {
    Io(std::io::Error),
    Yaml(String),
    SecretField,
    Invalid(&'static str),
    Value(String),
}
impl fmt::Display for M2ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl Error for M2ConfigError {}
impl From<std::io::Error> for M2ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_config_materializes_reference_session() {
        let config =
            load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/m2-twse-2330.yaml"))
                .unwrap();
        let session = materialize_session(&config).unwrap();
        assert_eq!(
            session.segment.open(),
            MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap()
        );
        assert_eq!(
            session.replay_start,
            MatchTime::parse("2026-07-27T08:55:00+08:00").unwrap()
        );
        assert_eq!(
            session.replay_end_exclusive,
            MatchTime::parse("2026-07-27T13:35:00+08:00").unwrap()
        );
    }

    #[test]
    fn secret_like_config_content_is_rejected_before_yaml_materialization() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.yaml");
        fs::write(&path, "config_version: 1\napi_key: forbidden\n").unwrap();
        assert!(matches!(load(path), Err(M2ConfigError::SecretField)));
    }
}
