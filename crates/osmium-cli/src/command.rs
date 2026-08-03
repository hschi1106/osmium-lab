use std::{
    env,
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use data_sync::{
    ArchiveKind, ArchiveTimestamp, CacheBuilder, FeedArchiveTransport, PartitionNormalizerConfig,
    PartitionedSourceRepository, StagingRevision, TeralionCredential, TeralionQuery, TeralionSync,
};
use execution_sim::{
    AccountingModel, ChargeModel, ChargeSides, EvidenceMode, FillModel, InstrumentEconomics,
    InstrumentLedgerConfig, MultiLedger, MultiSimulator, QuantityPolicy, RoundingPolicy,
};
use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{InstrumentKind, MarketId};
use osmium_config::{RUN_CONFIG_VERSION, RunConfig, load, plan};
use replay_engine::{ReplayContextWindow, ReplayCore};
use run_planner::{
    CacheAction, ChargeSides as PlanChargeSides, FillEvidence, NetworkRequirement,
    QuantityEvidence, RoundingPolicy as PlanRounding, SlippageModelConfig, SourceAction,
    SourceState,
};
use strategy_api::{
    ACCEPTANCE_STRATEGY_ID, ACCEPTANCE_STRATEGY_VERSION, AcceptanceStrategy, SessionKind,
    SessionSegment,
};
use taifex_normalizer::NormalizerConfig as TaifexNormalizerConfig;
use tpex_normalizer::NormalizerConfig as TpexNormalizerConfig;
use twse_normalizer::NormalizerConfig as TwseNormalizerConfig;

use crate::ExitCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    ConfigCheck,
    Plan,
    DataSync,
    DataVerify,
    CachePrepare,
    Replay,
    Backtest,
    Run,
}

impl CommandKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ConfigCheck => "config check",
            Self::Plan => "plan",
            Self::DataSync => "data sync",
            Self::DataVerify => "data verify",
            Self::CachePrepare => "cache prepare",
            Self::Replay => "replay",
            Self::Backtest => "backtest",
            Self::Run => "run",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub kind: CommandKind,
    pub config: PathBuf,
    pub output: Option<PathBuf>,
}

pub fn execute(command: &Command) -> Result<String, CommandError> {
    match command.kind {
        CommandKind::ConfigCheck => execute_config_check(&command.config),
        CommandKind::Plan => execute_plan(&command.config),
        CommandKind::DataSync => execute_sync(&command.config),
        CommandKind::DataVerify => execute_verify(&command.config),
        CommandKind::CachePrepare => prepare_cache(&command.config),
        CommandKind::Replay => execute_replay(&command.config),
        CommandKind::Backtest => execute_backtest(
            &command.config,
            command
                .output
                .as_deref()
                .ok_or(CommandError::OutputRequired)?,
        ),
        CommandKind::Run => execute_run(&command.config, command.output.as_deref()),
    }
}

pub fn execute_config_check(path: &Path) -> Result<String, CommandError> {
    let config = load(path)?;
    Ok(format!(
        "config=valid\nconfig_version={}\ntrading_dates={}\ninstruments={}",
        RUN_CONFIG_VERSION,
        config.effective().trading_dates().len(),
        config.effective().universe().len()
    ))
}

pub fn execute_inspect(path: &Path) -> Result<String, CommandError> {
    let summary = osmium_runner::inspect_run(path)?;
    Ok(format!(
        "status={}\nevents={}\norders={}\nfills={}",
        summary.status, summary.event_count, summary.order_count, summary.fill_count
    ))
}

fn execute_plan(path: &Path) -> Result<String, CommandError> {
    let bundle = plan(load(path)?)?;
    let mut output = format!(
        "plan_identity={}\nnetwork_requirement={:?}\npartitions={}",
        hex(bundle.execution.identity().as_bytes()),
        bundle.execution.network_requirement(),
        bundle.execution.partitions().len()
    );
    for (index, partition) in bundle.execution.partitions().iter().enumerate() {
        output.push_str(&format!(
            "\npartition[{index}]={:?}/{:?}@{} sessions={:?} source_action={:?} cache_action={:?}",
            partition.key().instrument().market(),
            partition.key().instrument().symbol(),
            partition.key().trading_date(),
            partition.key().session_kinds(),
            partition.source_action(),
            partition.cache_action(),
        ));
    }
    Ok(output)
}

fn queries(
    config: &RunConfig,
    key: &run_planner::SourcePartitionKey,
) -> Result<(TeralionQuery, TeralionQuery, PartitionNormalizerConfig), CommandError> {
    let session_plan = config.session_plan_for(key)?;
    let replay_start = session_plan
        .windows()
        .iter()
        .map(|window| window.replay_start())
        .min()
        .ok_or_else(|| CommandError::Other("session plan has no windows".to_owned()))?;
    let replay_end_exclusive = session_plan
        .windows()
        .iter()
        .map(|window| window.replay_end_exclusive())
        .max()
        .ok_or_else(|| CommandError::Other("session plan has no windows".to_owned()))?;
    let kinds = match key.instrument().market() {
        MarketId::Twse => [ArchiveKind::Quote].as_slice(),
        MarketId::Tpex => [ArchiveKind::Quote].as_slice(),
        MarketId::Taifex => [
            ArchiveKind::Book,
            ArchiveKind::Close,
            ArchiveKind::Stats,
            ArchiveKind::Trade,
        ]
        .as_slice(),
    };
    let kind = config.instrument_kind_for(key.instrument());
    let source_market = config.archive_market_for(key.instrument());
    let start = ArchiveTimestamp::parse(replay_start.to_iso8601(480))?;
    let end = ArchiveTimestamp::parse(replay_end_exclusive.to_iso8601(480))?;
    let ticks = match kind {
        InstrumentKind::Warrant | InstrumentKind::Option => TeralionQuery::ticks_for_market(
            key.instrument().clone(),
            start,
            end,
            kinds.iter().copied(),
            5_000,
            source_market,
        )?,
        _ => TeralionQuery::ticks(
            key.instrument().clone(),
            start,
            end,
            kinds.iter().copied(),
            5_000,
        )?,
    };
    let daily = TeralionQuery::daily_instrument(key.instrument().clone(), key.trading_date());
    let normalizer = match (key.instrument().market(), kind) {
        (MarketId::Twse, InstrumentKind::Warrant) => {
            PartitionNormalizerConfig::Warrant(TwseNormalizerConfig::new_warrant(
                key.instrument().clone(),
                key.trading_date(),
                replay_start,
                replay_end_exclusive,
            )?)
        }
        (MarketId::Twse, _) => PartitionNormalizerConfig::Twse(TwseNormalizerConfig::new(
            key.instrument().clone(),
            key.trading_date(),
            replay_start,
            replay_end_exclusive,
        )?),
        (MarketId::Tpex, InstrumentKind::Warrant) => {
            PartitionNormalizerConfig::TpexWarrant(TpexNormalizerConfig::new_warrant(
                key.instrument().clone(),
                key.trading_date(),
                replay_start,
                replay_end_exclusive,
            )?)
        }
        (MarketId::Tpex, _) => PartitionNormalizerConfig::Tpex(TpexNormalizerConfig::new(
            key.instrument().clone(),
            key.trading_date(),
            replay_start,
            replay_end_exclusive,
        )?),
        (MarketId::Taifex, InstrumentKind::Option) => {
            let windows = session_plan
                .windows()
                .iter()
                .map(|window| (window.replay_start(), window.replay_end_exclusive()));
            PartitionNormalizerConfig::TaifexOption(TaifexNormalizerConfig::for_profile(
                key.instrument().clone(),
                key.trading_date(),
                taifex_normalizer::InstrumentProfile::IndexOptions,
                windows,
            )?)
        }
        (MarketId::Taifex, _) => PartitionNormalizerConfig::Taifex(TaifexNormalizerConfig::new(
            key.instrument().clone(),
            key.trading_date(),
            replay_start,
            replay_end_exclusive,
        )?),
    };
    Ok((ticks, daily, normalizer))
}

fn attempt_id(key: &run_planner::SourcePartitionKey) -> String {
    let identity = hex(key.identity().as_bytes());
    format!("run-{}", &identity[..24])
}

fn load_dotenv() {
    if Path::new(".env").is_file() {
        let _ = dotenvy::dotenv();
    }
}

fn execute_sync(path: &Path) -> Result<String, CommandError> {
    let config = load(path)?;
    let bundle = plan(config.clone())?;
    let needs_network = bundle.execution.partitions().iter().any(|partition| {
        !matches!(
            partition.source_action(),
            SourceAction::ReuseCompleteSource { .. }
        )
    });
    if !needs_network {
        return Ok("source=reused\nhttp_requests=0".to_owned());
    }
    load_dotenv();
    let credential = TeralionCredential::new(
        env::var("TERALION_API_KEY").map_err(|_| CommandError::MissingCredential)?,
    )?;
    let mut sync = TeralionSync::new(FeedArchiveTransport::new()?);
    let mut output = String::from("source=partitions\n");
    let mut total_pages = 0_u64;
    let mut published = 0_u32;
    for partition in bundle.execution.partitions() {
        if matches!(
            partition.source_action(),
            SourceAction::ReuseCompleteSource { .. }
        ) {
            output.push_str(&format!(
                "partition={:?}/{:?}@{} status=reused\n",
                partition.key().instrument().market(),
                partition.key().instrument().symbol(),
                partition.key().trading_date()
            ));
            continue;
        }
        let key = partition.key();
        let (ticks, daily_query, _) = queries(&config, key)?;
        validate_json(&sync.fetch_single(
            TeralionQuery::coverage(key.trading_date(), key.trading_date())?,
            &credential,
        )?)?;
        validate_json(&sync.fetch_single(
            TeralionQuery::symbol_range(key.instrument().clone()),
            &credential,
        )?)?;
        let daily = sync.fetch_single(daily_query.clone(), &credential)?;
        validate_json(&daily)?;
        let repository =
            PartitionedSourceRepository::new(config.effective().data_root(), key.clone())?;
        let attempt = attempt_id(key);
        let checkpoint = repository
            .root()
            .join("staging")
            .join(&attempt)
            .join("checkpoint.json");
        let mut staging = if checkpoint.exists() {
            StagingRevision::resume_for_partition(config.effective().data_root(), key, &attempt)?
        } else {
            StagingRevision::create_for_partition(config.effective().data_root(), key, &attempt)?
        };
        let report = sync.sync_pages(ticks.clone(), &credential, &mut staging)?;
        staging.stage_daily_instrument(daily_query.identity(), &daily)?;
        let revision = staging.publish(ticks.identity(), report.terminal)?;
        total_pages += u64::from(report.page_count);
        published += 1;
        output.push_str(&format!(
            "partition={:?}/{:?}@{} status=published pages={} revision={}\n",
            key.instrument().market(),
            key.instrument().symbol(),
            key.trading_date(),
            report.page_count,
            revision.manifest().revision_identity
        ));
    }
    output.push_str(&format!("published={} pages={}\n", published, total_pages));
    Ok(output.trim_end().to_owned())
}

fn execute_verify(path: &Path) -> Result<String, CommandError> {
    let config = load(path)?;
    let mut output = String::from("source=verified\n");
    for key in config.partition_keys()? {
        let repository =
            PartitionedSourceRepository::new(config.effective().data_root(), key.clone())?;
        let report = repository.verify_current()?;
        output.push_str(&format!(
            "partition={:?}/{:?}@{} revision={} records={}\n",
            key.instrument().market(),
            key.instrument().symbol(),
            key.trading_date(),
            report.manifest().revision_identity,
            report.manifest().tick_record_count
        ));
    }
    Ok(output.trim_end().to_owned())
}

fn prepare_cache(path: &Path) -> Result<String, CommandError> {
    let config = load(path)?;
    let bundle = plan(config.clone())?;
    let builder = CacheBuilder::new(config.effective().data_root());
    let mut output = String::from("cache=partitions\n");
    for partition in bundle.execution.partitions() {
        match partition.cache_action() {
            CacheAction::ReuseValidCache { identity } => {
                output.push_str(&format!(
                    "partition={:?}/{:?}@{} status=reused cache_identity={}\n",
                    partition.key().instrument().market(),
                    partition.key().instrument().symbol(),
                    partition.key().trading_date(),
                    hex(identity.as_bytes())
                ));
            }
            CacheAction::RebuildCacheFromCompleteSource => {
                if !matches!(partition.source_state(), SourceState::Complete { .. }) {
                    return Err(CommandError::CacheMissing);
                }
                let (_, _, normalizer) = queries(&config, partition.key())?;
                let built = builder.build_partition(partition.key(), normalizer)?;
                output.push_str(&format!(
                    "partition={:?}/{:?}@{} status=built cache_identity={}\n",
                    partition.key().instrument().market(),
                    partition.key().instrument().symbol(),
                    partition.key().trading_date(),
                    built.descriptor().cache_identity
                ));
            }
            CacheAction::AwaitCompleteSource => return Err(CommandError::CacheMissing),
        }
    }
    Ok(output.trim_end().to_owned())
}

fn execute_replay(path: &Path) -> Result<String, CommandError> {
    let completed = replay(path)?;
    Ok(format!(
        "replay=complete\nevents={}\nevent_checksum={}\nfinal_state_checksum={}",
        completed.summary().event_count(),
        hex(completed.summary().event_checksum().as_bytes()),
        hex(completed.summary().final_state_checksum().as_bytes())
    ))
}

fn replay(path: &Path) -> Result<replay_engine::CompletedReplay, CommandError> {
    let config = load(path)?;
    let bundle = plan(config.clone())?;
    let replay = bundle.replay.as_ref().ok_or(CommandError::CacheMissing)?;
    let mut core = replay_core(&config, &bundle)?;
    let mut factory = data_sync::LocalCacheFactory::new_partitioned(config.effective().data_root());
    core.replay_frozen_multi(replay, &mut factory)?;
    Ok(core.complete()?)
}

pub(crate) fn replay_core(
    config: &RunConfig,
    bundle: &osmium_config::PlanBundle,
) -> Result<ReplayCore, CommandError> {
    let mut states = Vec::new();
    let mut reducers = Vec::new();
    let mut contexts = Vec::new();
    let mut schedules = Vec::new();
    for partition in bundle.execution.partitions() {
        let key = partition.key();
        let session_plan = config.session_plan_for(key)?;
        let mut windows = Vec::new();
        let mut default_context = None;
        for window in session_plan.windows() {
            let segment = match window.kind() {
                strategy_api::SessionKind::Regular => "regular",
                strategy_api::SessionKind::AfterHours => "after_hours",
            };
            let context = ReducerContext::new(
                key.trading_date(),
                SessionSegmentId::new(segment)?,
                SegmentBoundaryPolicy::ResetObservableFields,
                1,
            );
            default_context.get_or_insert(context.clone());
            windows.push(ReplayContextWindow::new(
                window.replay_start(),
                window.replay_end_exclusive(),
                context,
            )?);
        }
        let context = default_context
            .ok_or_else(|| CommandError::Other("session plan has no windows".to_owned()))?;
        let kind = config.instrument_kind_for(key.instrument());
        let reducer = match (key.instrument().market(), kind) {
            (MarketId::Twse, InstrumentKind::Warrant) => MarketStateReducer::twse_warrant(),
            (MarketId::Twse, _) => MarketStateReducer::twse_regular(),
            (MarketId::Tpex, InstrumentKind::Warrant) => MarketStateReducer::tpex_warrant(),
            (MarketId::Tpex, _) => MarketStateReducer::tpex_regular(),
            (MarketId::Taifex, InstrumentKind::Option) => MarketStateReducer::taifex_options(),
            (MarketId::Taifex, _) => MarketStateReducer::taifex_futures(),
        };
        states.push(MarketState::new(
            key.instrument().clone(),
            key.trading_date(),
        ));
        reducers.push((key.instrument().clone(), reducer));
        contexts.push((key.instrument().clone(), context));
        schedules.push((key.instrument().clone(), windows));
    }
    Ok(ReplayCore::new_multi_with_schedules(
        states, reducers, contexts, schedules,
    )?)
}

fn schedule(config: &RunConfig) -> Result<osmium_runner::MultiSessionSchedule, CommandError> {
    let mut entries = Vec::new();
    for key in config.partition_keys()? {
        let session_plan = config.session_plan_for(&key)?;
        let mut segments = Vec::new();
        for window in session_plan.windows() {
            let id = match window.kind() {
                SessionKind::Regular => "regular",
                SessionKind::AfterHours => "after_hours",
            };
            segments.push(SessionSegment::new(
                market_state::SessionSegmentId::new(id)?,
                window.kind(),
                key.trading_date(),
                window.open(),
                window.close(),
            )?);
        }
        entries.push((key.instrument().clone(), segments));
    }
    Ok(osmium_runner::MultiSessionSchedule::new(entries)?)
}

fn execute_backtest(path: &Path, output: &Path) -> Result<String, CommandError> {
    let config = load(path)?;
    let bundle = plan(config.clone())?;
    if bundle
        .execution
        .config()
        .strategy()
        .identity()
        .strategy_id()
        != ACCEPTANCE_STRATEGY_ID
        || bundle
            .execution
            .config()
            .strategy()
            .identity()
            .strategy_version()
            != ACCEPTANCE_STRATEGY_VERSION
    {
        return Err(CommandError::UnsupportedStrategy);
    }
    let replay = bundle.replay.as_ref().ok_or(CommandError::CacheMissing)?;
    let core = replay_core(&config, &bundle)?;
    let schedule = schedule(&config)?;
    let strategy = AcceptanceStrategy::new(
        AcceptanceStrategy::source_binary_identity()?,
        bundle.execution.config().universe().iter().cloned(),
        bundle.execution.config().session_kinds().iter().copied(),
    )?;
    let simulation = bundle.execution.config().simulation();
    let fill = simulation.fill_model();
    let latency = simulation.latency();
    let slippage = match simulation.slippage_model() {
        SlippageModelConfig::AdverseFixedDelta { delta } => delta,
    };
    let simulator =
        MultiSimulator::new(bundle.execution.config().instrument_economics().iter().map(
            |economics| {
                (
                    economics.instrument().clone(),
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
                        market_data_latency_ms: latency.market_data_latency_ms(),
                        order_latency_ms: latency.order_latency_ms(),
                    },
                )
            },
        ))?;
    let ledger = MultiLedger::new(
        simulation.initial_cash().amount(),
        bundle
            .execution
            .config()
            .instrument_economics()
            .iter()
            .map(|economics| {
                let model = match config.instrument_kind_for(economics.instrument()) {
                    InstrumentKind::Option => AccountingModel::OptionsV1,
                    InstrumentKind::Future => AccountingModel::FuturesV1,
                    InstrumentKind::Equity | InstrumentKind::Warrant | InstrumentKind::Unknown => {
                        AccountingModel::EquityV1
                    }
                };
                InstrumentLedgerConfig::new(
                    economics.instrument().clone(),
                    economics.quantity_unit(),
                    model,
                    InstrumentEconomics {
                        units_per_trading_unit: economics.units_per_trading_unit(),
                        multiplier: economics.multiplier(),
                        provenance: economics.provenance().into(),
                    },
                    charge(simulation.fee_model()),
                    charge(simulation.tax_model()),
                )
            }),
    )?;
    let allow_midpoint_fallback = match simulation.marking_policy() {
        run_planner::MarkingPolicyConfig::LastObservableV1 {
            allow_midpoint_fallback,
        } => allow_midpoint_fallback,
    };
    let mut factory = data_sync::LocalCacheFactory::new_partitioned(config.effective().data_root());
    let completed = osmium_runner::run_multi_backtest(
        core,
        strategy,
        replay,
        &mut factory,
        &schedule,
        simulator,
        ledger,
        allow_midpoint_fallback,
    )?;
    let mut source_lineage = Vec::new();
    let mut cache_lineage = Vec::new();
    for partition in bundle.execution.partitions() {
        let source = match partition.source_state() {
            SourceState::Complete { revision } => hex(revision.as_bytes()),
            _ => return Err(CommandError::CacheMissing),
        };
        let cache = match partition.cache_action() {
            CacheAction::ReuseValidCache { identity } => hex(identity.as_bytes()),
            _ => return Err(CommandError::CacheMissing),
        };
        let label = format!(
            "{:?}/{}@{}",
            partition.key().instrument().market(),
            partition.key().instrument().symbol(),
            partition.key().trading_date(),
        );
        source_lineage.push(format!("{label}={source}"));
        cache_lineage.push(format!("{label}={cache}"));
    }
    let source_revision = source_lineage.join(",");
    let cache_identity = cache_lineage.join(",");
    osmium_runner::publish_multi_backtest(
        output,
        &completed,
        bundle.execution.identity().as_bytes(),
        &source_revision,
        &cache_identity,
    )?;
    Ok(format!(
        "backtest=complete\nevents={}\norders={}\nfills={}\nfinal_cash_atoms={}\nrealized_pnl_atoms={}\nunrealized_pnl_atoms={}\noutput={}",
        completed.replay.summary().event_count(),
        completed.simulator.order_count(),
        completed.simulator.fill_count(),
        completed.performance.final_cash().atoms(),
        completed.performance.realized_pnl().atoms(),
        completed.performance.unrealized_pnl().atoms(),
        output.display()
    ))
}

fn execute_run(path: &Path, output: Option<&Path>) -> Result<String, CommandError> {
    let config = load(path)?;
    let bundle = plan(config)?;
    if bundle.execution.network_requirement() == NetworkRequirement::Required {
        execute_sync(path)?;
    }
    prepare_cache(path)?;
    match output {
        Some(output) => execute_backtest(path, output),
        None => execute_replay(path),
    }
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

fn validate_json(bytes: &[u8]) -> Result<(), CommandError> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .map(|_| ())
        .map_err(|error| CommandError::Other(error.to_string()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug)]
pub enum CommandError {
    Config(osmium_config::ConfigError),
    Query(data_sync::QueryError),
    Transport(data_sync::TransportError),
    Sync(data_sync::SyncError),
    Staging(data_sync::StagingError),
    Verify(data_sync::VerificationError),
    CacheBuild(data_sync::CacheBuildError),
    CacheRead(data_sync::CacheReadError),
    Normalizer(twse_normalizer::ConfigError),
    TpexNormalizer(tpex_normalizer::ConfigError),
    TaifexNormalizer(taifex_normalizer::ConfigError),
    Replay(replay_engine::ReplayError),
    State(market_state::SessionSegmentIdError),
    Context(strategy_api::ContextError),
    Strategy(strategy_api::DeclarationError),
    Simulation(execution_sim::SimulationError),
    Accounting(execution_sim::AccountingError),
    Backtest(osmium_runner::BacktestError),
    MultiBacktest(osmium_runner::MultiBacktestError),
    Artifact(osmium_runner::ArtifactError),
    Io(std::io::Error),
    Partition(data_sync::PartitionRepositoryError),
    ReplayContextWindow(replay_engine::ReplayContextWindowError),
    MissingCredential,
    CacheMissing,
    OutputRequired,
    UnsupportedStrategy,
    Other(String),
}

impl CommandError {
    #[must_use]
    pub const fn category(&self) -> ExitCategory {
        match self {
            Self::Config(_)
            | Self::Query(_)
            | Self::Normalizer(_)
            | Self::TpexNormalizer(_)
            | Self::TaifexNormalizer(_)
            | Self::State(_)
            | Self::Context(_)
            | Self::Strategy(_)
            | Self::UnsupportedStrategy
            | Self::ReplayContextWindow(_) => ExitCategory::Config,
            Self::OutputRequired => ExitCategory::Usage,
            Self::Transport(_) | Self::Sync(_) | Self::MissingCredential | Self::Partition(_) => {
                ExitCategory::Source
            }
            Self::CacheBuild(_) | Self::CacheRead(_) | Self::Staging(_) | Self::CacheMissing => {
                ExitCategory::Cache
            }
            Self::Replay(_) => ExitCategory::Replay,
            Self::Backtest(_)
            | Self::MultiBacktest(_)
            | Self::Simulation(_)
            | Self::Accounting(_) => ExitCategory::Simulation,
            Self::Verify(_) | Self::Artifact(_) => ExitCategory::Integrity,
            Self::Io(_) | Self::Other(_) => ExitCategory::Internal,
        }
    }

    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        self.category().exit_code()
    }
}

macro_rules! convert {
    ($variant:ident, $source:ty) => {
        impl From<$source> for CommandError {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        }
    };
}
convert!(Config, osmium_config::ConfigError);
convert!(Query, data_sync::QueryError);
convert!(Transport, data_sync::TransportError);
convert!(Sync, data_sync::SyncError);
convert!(Staging, data_sync::StagingError);
convert!(Verify, data_sync::VerificationError);
convert!(CacheBuild, data_sync::CacheBuildError);
convert!(CacheRead, data_sync::CacheReadError);
convert!(Normalizer, twse_normalizer::ConfigError);
convert!(TpexNormalizer, tpex_normalizer::ConfigError);
convert!(TaifexNormalizer, taifex_normalizer::ConfigError);
convert!(Replay, replay_engine::ReplayError);
convert!(State, market_state::SessionSegmentIdError);
convert!(Context, strategy_api::ContextError);
convert!(Strategy, strategy_api::DeclarationError);
convert!(Simulation, execution_sim::SimulationError);
convert!(Accounting, execution_sim::AccountingError);
convert!(Backtest, osmium_runner::BacktestError);
convert!(MultiBacktest, osmium_runner::MultiBacktestError);
convert!(Artifact, osmium_runner::ArtifactError);
convert!(Partition, data_sync::PartitionRepositoryError);
convert!(ReplayContextWindow, replay_engine::ReplayContextWindowError);
convert!(Io, std::io::Error);

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(formatter, "config error: {error}"),
            _ => write!(formatter, "{self:?}"),
        }
    }
}
impl Error for CommandError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_exit_codes_preserve_stable_failure_categories() {
        assert_eq!(CommandError::OutputRequired.exit_code(), 2);
        assert_eq!(
            CommandError::Config(osmium_config::ConfigError::Invalid("field")).exit_code(),
            10
        );
        assert_eq!(CommandError::CacheMissing.exit_code(), 21);
        assert_eq!(CommandError::MissingCredential.exit_code(), 20);
        assert_eq!(
            CommandError::Replay(replay_engine::ReplayError::EmptyUniverse).exit_code(),
            30
        );
        assert_eq!(CommandError::Other("internal".to_owned()).exit_code(), 1);
    }

    #[test]
    fn run_config_plan_lists_each_partition_without_source_access() {
        let summary = execute(&Command {
            kind: CommandKind::Plan,
            config: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/m3-taifex-multi.yaml"),
            output: None,
        })
        .unwrap();
        assert!(summary.contains("partitions=4"));
        assert!(summary.contains("TXFH6"));
        assert!(summary.contains("AfterHours"));
        assert!(summary.contains("CAFH6"));
    }
}
