use std::{
    env,
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use data_sync::{
    ArchiveKind, ArchiveTimestamp, CacheBuilder, CacheReader, FeedArchiveTransport,
    LocalSourceRepository, StagingRevision, TeralionCredential, TeralionQuery, TeralionSync,
};
use execution_sim::{
    ChargeModel, ChargeSides, EvidenceMode, FillModel, InstrumentEconomics, Ledger, QuantityPolicy,
    RoundingPolicy, Simulator,
};
use m2_config::{M2PlanBundle, load, plan};
use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use replay_engine::ReplayCore;
use run_planner::{
    ChargeSides as PlanChargeSides, FillEvidence, NetworkRequirement, QuantityEvidence,
    RoundingPolicy as PlanRounding, SlippageModelConfig, SourceAction,
};
use strategy_api::M2AcceptanceStrategy;
use twse_normalizer::NormalizerConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum M2CommandKind {
    Plan,
    Sync,
    Verify,
    Replay,
    Backtest,
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M2Command {
    pub kind: M2CommandKind,
    pub config: PathBuf,
    pub output: Option<PathBuf>,
}

pub fn execute(command: &M2Command) -> Result<String, M2CommandError> {
    match command.kind {
        M2CommandKind::Plan => execute_plan(&command.config),
        M2CommandKind::Sync => execute_sync(&command.config),
        M2CommandKind::Verify => execute_verify(&command.config),
        M2CommandKind::Replay => execute_replay(&command.config),
        M2CommandKind::Backtest => execute_backtest(
            &command.config,
            command
                .output
                .as_deref()
                .ok_or(M2CommandError::OutputRequired)?,
        ),
        M2CommandKind::Run => {
            let config = load(&command.config)?;
            let bundle = plan(config)?;
            if bundle.execution.network_requirement() == NetworkRequirement::Required {
                execute_sync(&command.config)?;
            }
            prepare_cache(&command.config)?;
            execute_backtest(
                &command.config,
                command
                    .output
                    .as_deref()
                    .ok_or(M2CommandError::OutputRequired)?,
            )
        }
    }
}

pub fn execute_inspect(path: &Path) -> Result<String, M2CommandError> {
    let summary = m2_runner::inspect_run(path)?;
    Ok(format!(
        "status={}\nevents={}\norders={}\nfills={}",
        summary.status, summary.event_count, summary.order_count, summary.fill_count
    ))
}

fn execute_plan(path: &Path) -> Result<String, M2CommandError> {
    let bundle = plan(load(path)?)?;
    let partition = &bundle.execution.partitions()[0];
    Ok(format!(
        "plan_identity={}\nnetwork_requirement={:?}\nsource_action={:?}\ncache_action={:?}",
        hex(bundle.execution.identity().as_bytes()),
        bundle.execution.network_requirement(),
        partition.source_action(),
        partition.cache_action()
    ))
}

fn execute_sync(path: &Path) -> Result<String, M2CommandError> {
    let config = load(path)?;
    let initial = plan(config.clone())?;
    if matches!(
        initial.execution.partitions()[0].source_action(),
        SourceAction::ReuseCompleteSource { .. }
    ) {
        return Ok("source=reused\nhttp_requests=0".to_owned());
    }
    let credential = TeralionCredential::new(
        env::var("TERALION_API_KEY").map_err(|_| M2CommandError::MissingCredential)?,
    )?;
    let instrument = config.universe()[0].clone();
    let date = config.trading_dates()[0];
    let session = m2_config::materialize_session(&config)?;
    let ticks = TeralionQuery::ticks(
        instrument.clone(),
        ArchiveTimestamp::parse(format!("{date}T08:55:00+08:00"))?,
        ArchiveTimestamp::parse(format!("{date}T13:35:00+08:00"))?,
        [ArchiveKind::Quote],
        5_000,
    )?;
    let mut sync = TeralionSync::new(FeedArchiveTransport::new()?);
    validate_json(&sync.fetch_single(TeralionQuery::coverage(date, date)?, &credential)?)?;
    validate_json(
        &sync.fetch_single(TeralionQuery::symbol_range(instrument.clone()), &credential)?,
    )?;
    let daily_query = TeralionQuery::daily_instrument(instrument, date);
    let daily = sync.fetch_single(daily_query.clone(), &credential)?;
    validate_json(&daily)?;

    let attempt = "m2-twse-2330";
    let checkpoint = config
        .data_root()
        .join("staging")
        .join(attempt)
        .join("checkpoint.json");
    let mut staging = if checkpoint.exists() {
        StagingRevision::resume(config.data_root(), attempt)?
    } else {
        StagingRevision::create(config.data_root(), attempt)?
    };
    let report = sync.sync_pages(ticks.clone(), &credential, &mut staging)?;
    staging.stage_daily_instrument(daily_query.identity(), &daily)?;
    let published = staging.publish(ticks.identity(), report.terminal)?;
    Ok(format!(
        "source=published\npages={}\nrevision={}\nsession_identity={}",
        report.page_count,
        published.manifest().revision_identity,
        hex(session.identity.as_bytes())
    ))
}

fn execute_verify(path: &Path) -> Result<String, M2CommandError> {
    let config = load(path)?;
    let report = LocalSourceRepository::new(config.data_root()).verify_current()?;
    Ok(format!(
        "source=complete\nrevision={}\nrecords={}",
        report.manifest().revision_identity,
        report.manifest().tick_record_count
    ))
}

fn prepare_cache(path: &Path) -> Result<String, M2CommandError> {
    let config = load(path)?;
    let planned = plan(config.clone())?;
    if let Some(cache) = planned.cache_path {
        let reader = CacheReader::open(cache)?;
        return Ok(format!(
            "cache=reused\ncache_identity={}",
            reader.descriptor().cache_identity
        ));
    }
    let session = m2_config::materialize_session(&config)?;
    let built = CacheBuilder::new(config.data_root()).build_current(NormalizerConfig::new(
        config.universe()[0].clone(),
        config.trading_dates()[0],
        session.replay_start,
        session.replay_end_exclusive,
    )?)?;
    Ok(format!("cache={}", built.descriptor().cache_identity))
}

fn execute_replay(path: &Path) -> Result<String, M2CommandError> {
    let bundle = ready_bundle(path)?;
    let cache = bundle
        .cache_path
        .as_ref()
        .ok_or(M2CommandError::CacheMissing)?;
    let mut reader = CacheReader::open(cache)?;
    let mut core = core(&bundle)?;
    core.replay_stream(&mut reader)?;
    let completed = core.complete()?;
    Ok(format!(
        "replay=complete\nevents={}\nevent_checksum={}",
        completed.summary().event_count(),
        hex(completed.summary().event_checksum().as_bytes())
    ))
}

fn execute_backtest(path: &Path, output: &Path) -> Result<String, M2CommandError> {
    let bundle = ready_bundle(path)?;
    let config = bundle.execution.config();
    let cache = bundle
        .cache_path
        .as_ref()
        .ok_or(M2CommandError::CacheMissing)?;
    let mut reader = CacheReader::open(cache)?;
    let source_revision = reader.descriptor().source_revision_identity.clone();
    let cache_identity = reader.descriptor().cache_identity.clone();
    let strategy = M2AcceptanceStrategy::new(
        M2AcceptanceStrategy::source_binary_identity()?,
        config.universe()[0].clone(),
    )?;
    let simulation = config.simulation();
    let fill = simulation.fill_model();
    let slippage = match simulation.slippage_model() {
        SlippageModelConfig::AdverseFixedDelta { delta } => delta,
    };
    let economics = &config.instrument_economics()[0];
    let simulator = Simulator::new(
        config.universe().iter().cloned(),
        economics.quantity_unit(),
        FillModel {
            evidence: match fill.evidence() {
                FillEvidence::TopOfBook => EvidenceMode::TopOfBook,
                FillEvidence::TradePrint => EvidenceMode::TradePrint,
            },
            quantity: match fill.quantity() {
                QuantityEvidence::Unlimited => QuantityPolicy::Unlimited,
                QuantityEvidence::Observed => QuantityPolicy::Displayed,
            },
            adverse_price_delta: slippage,
        },
    );
    let ledger = Ledger::new(
        simulation.initial_cash().amount(),
        InstrumentEconomics {
            units_per_trading_unit: economics.units_per_trading_unit(),
            multiplier: economics.multiplier(),
            provenance: economics.provenance().into(),
        },
        charge(simulation.fee_model()),
        charge(simulation.tax_model()),
    );
    let completed = m2_runner::run_backtest_stream(
        core(&bundle)?,
        strategy,
        &bundle.session.segment,
        &mut reader,
        simulator,
        ledger,
        None,
    )?;
    m2_runner::publish_backtest(
        output,
        &completed,
        bundle.execution.identity().as_bytes(),
        &source_revision,
        &cache_identity,
    )?;
    Ok(format!(
        "backtest=complete\nevents={}\norders={}\nfills={}\nfinal_cash_atoms={}\noutput={}",
        completed.replay.summary().event_count(),
        completed.simulator.orders().len(),
        completed.simulator.fills().len(),
        completed.performance.final_cash.atoms(),
        output.display()
    ))
}

fn ready_bundle(path: &Path) -> Result<M2PlanBundle, M2CommandError> {
    let bundle = plan(load(path)?)?;
    if bundle.replay.is_none() {
        return Err(M2CommandError::CacheMissing);
    }
    Ok(bundle)
}

fn core(bundle: &M2PlanBundle) -> Result<ReplayCore, M2CommandError> {
    let config = bundle.execution.config();
    Ok(ReplayCore::new(
        vec![MarketState::new(
            config.universe()[0].clone(),
            config.trading_dates()[0],
        )],
        MarketStateReducer::twse_regular(),
        ReducerContext::new(
            config.trading_dates()[0],
            SessionSegmentId::new("regular")?,
            SegmentBoundaryPolicy::Carry,
            1,
        ),
    )?)
}

fn charge(value: &run_planner::ChargeConfig) -> ChargeModel {
    ChargeModel {
        rate: value.rate(),
        sides: match value.applicable_sides() {
            PlanChargeSides::Buy => ChargeSides::Buy,
            PlanChargeSides::Sell => ChargeSides::Sell,
            PlanChargeSides::BuyAndSell => ChargeSides::Both,
        },
        minimum: value.minimum(),
        precision: value.precision(),
        rounding: match value.rounding() {
            PlanRounding::Down => RoundingPolicy::Down,
            PlanRounding::HalfUp => RoundingPolicy::HalfUp,
            PlanRounding::Up => RoundingPolicy::Up,
        },
    }
}

fn validate_json(bytes: &[u8]) -> Result<(), M2CommandError> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .map(|_| ())
        .map_err(|error| M2CommandError::Other(error.to_string()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug)]
pub enum M2CommandError {
    Config(m2_config::M2ConfigError),
    Query(data_sync::QueryError),
    Transport(data_sync::TransportError),
    Sync(data_sync::SyncError),
    Staging(data_sync::StagingError),
    Verify(data_sync::VerificationError),
    CacheBuild(data_sync::CacheBuildError),
    CacheRead(data_sync::CacheReadError),
    Normalizer(twse_normalizer::ConfigError),
    Replay(replay_engine::ReplayError),
    State(market_state::SessionSegmentIdError),
    Strategy(strategy_api::DeclarationError),
    Backtest(m2_runner::BacktestError),
    Artifact(m2_runner::ArtifactError),
    MissingCredential,
    CacheMissing,
    OutputRequired,
    Other(String),
}

impl M2CommandError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Config(_)
            | Self::Query(_)
            | Self::Normalizer(_)
            | Self::State(_)
            | Self::Strategy(_)
            | Self::OutputRequired => 2,
            Self::Verify(_)
            | Self::CacheBuild(_)
            | Self::CacheRead(_)
            | Self::Staging(_)
            | Self::Artifact(_)
            | Self::CacheMissing => 20,
            Self::Transport(_) | Self::Sync(_) | Self::MissingCredential => 30,
            Self::Replay(_) | Self::Backtest(_) => 50,
            Self::Other(_) => 1,
        }
    }
}

macro_rules! convert {
    ($variant:ident, $source:ty) => {
        impl From<$source> for M2CommandError {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        }
    };
}
convert!(Config, m2_config::M2ConfigError);
convert!(Query, data_sync::QueryError);
convert!(Transport, data_sync::TransportError);
convert!(Sync, data_sync::SyncError);
convert!(Staging, data_sync::StagingError);
convert!(Verify, data_sync::VerificationError);
convert!(CacheBuild, data_sync::CacheBuildError);
convert!(CacheRead, data_sync::CacheReadError);
convert!(Normalizer, twse_normalizer::ConfigError);
convert!(Replay, replay_engine::ReplayError);
convert!(State, market_state::SessionSegmentIdError);
convert!(Strategy, strategy_api::DeclarationError);
convert!(Backtest, m2_runner::BacktestError);
convert!(Artifact, m2_runner::ArtifactError);

impl fmt::Display for M2CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl Error for M2CommandError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m2_exit_codes_preserve_stable_failure_categories() {
        assert_eq!(M2CommandError::OutputRequired.exit_code(), 2);
        assert_eq!(M2CommandError::CacheMissing.exit_code(), 20);
        assert_eq!(M2CommandError::MissingCredential.exit_code(), 30);
        assert_eq!(M2CommandError::Other("internal".to_owned()).exit_code(), 1);
    }
}
