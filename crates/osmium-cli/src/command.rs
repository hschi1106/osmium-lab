use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use data_sync::{
    CacheBuilder, PartitionedSourceRepository, SourceAdapterRuntime, normalizer_config_for,
};
use execution_sim::{
    AccountingModel, ChargeBasis, ChargeModel, ChargeSides, DayTradeTaxModel, EvidenceMode,
    FillModel, InstrumentEconomics, InstrumentLedgerConfig, MultiLedger, MultiSimulator,
    QuantityPolicy, RoundingPolicy, ScheduledDepthModel, ScheduledDepthSimulator,
    ScheduledInstrumentConfig,
};
use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{InstrumentClass, MarketId};
use osmium_config::{RUN_CONFIG_VERSION, RunConfig, plan};
use replay_engine::{ReplayContextWindow, ReplayCore};
use run_planner::{
    CacheAction, ChargeBasis as PlanChargeBasis, ChargeSides as PlanChargeSides,
    ExecutionPolicyConfig, FillEvidence, NetworkRequirement, QuantityEvidence,
    RoundingPolicy as PlanRounding, SlippageModelConfig, SourceAction, SourceState,
};
use strategy_api::{AcceptanceStrategyFactory, SessionKind, SessionSegment, StrategyRegistry};

use crate::ExitCategory;

/// Adds externally compiled strategies to the registry used for a config file.
///
/// The CLI always installs its built-in strategies first. Implementations may inspect the
/// config path (for example through [`osmium_config::strategy_bootstrap`]) before registering
/// additional factories. Returning an error aborts config loading; it is never treated as an
/// empty registry.
pub trait StrategyRegistryProvider: Send + Sync {
    fn register_strategies(
        &self,
        config_path: &Path,
        registry: &mut StrategyRegistry,
    ) -> Result<(), CommandError>;
}

impl<F> StrategyRegistryProvider for F
where
    F: Fn(&Path, &mut StrategyRegistry) -> Result<(), CommandError> + Send + Sync,
{
    fn register_strategies(
        &self,
        config_path: &Path,
        registry: &mut StrategyRegistry,
    ) -> Result<(), CommandError> {
        self(config_path, registry)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltInStrategyRegistry;

impl StrategyRegistryProvider for BuiltInStrategyRegistry {
    fn register_strategies(
        &self,
        _config_path: &Path,
        _registry: &mut StrategyRegistry,
    ) -> Result<(), CommandError> {
        Ok(())
    }
}

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
    execute_with_registry_provider(command, &BuiltInStrategyRegistry)
}

pub fn execute_with_registry_provider(
    command: &Command,
    provider: &dyn StrategyRegistryProvider,
) -> Result<String, CommandError> {
    match command.kind {
        CommandKind::ConfigCheck => {
            execute_config_check_with_registry_provider(&command.config, provider)
        }
        CommandKind::Plan => execute_plan(&command.config, provider),
        CommandKind::DataSync => execute_sync(&command.config, provider),
        CommandKind::DataVerify => execute_verify(&command.config, provider),
        CommandKind::CachePrepare => prepare_cache(&command.config, provider),
        CommandKind::Replay => execute_replay(&command.config, provider),
        CommandKind::Backtest => execute_backtest(
            &command.config,
            command
                .output
                .as_deref()
                .ok_or(CommandError::OutputRequired)?,
            provider,
        ),
        CommandKind::Run => execute_run(&command.config, command.output.as_deref(), provider),
    }
}

pub fn execute_config_check(path: &Path) -> Result<String, CommandError> {
    execute_config_check_with_registry_provider(path, &BuiltInStrategyRegistry)
}

pub fn execute_config_check_with_registry_provider(
    path: &Path,
    provider: &dyn StrategyRegistryProvider,
) -> Result<String, CommandError> {
    let config = load_config_with_registry_provider(path, provider)?;
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

fn execute_plan(
    path: &Path,
    provider: &dyn StrategyRegistryProvider,
) -> Result<String, CommandError> {
    let config = load_config_with_registry_provider(path, provider)?;
    let bundle = plan(&config)?;
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

pub(crate) fn compiled_strategy_registry() -> Result<StrategyRegistry, CommandError> {
    let mut registry = StrategyRegistry::new();
    let acceptance =
        AcceptanceStrategyFactory::new().map_err(osmium_config::ConfigError::Strategy)?;
    registry
        .register(acceptance)
        .map_err(osmium_config::ConfigError::Strategy)?;
    let example = example_strategy::PriceThresholdBuyOnceFactory::new()
        .map_err(osmium_config::ConfigError::Strategy)?;
    registry
        .register(example)
        .map_err(osmium_config::ConfigError::Strategy)?;
    Ok(registry)
}

pub fn load_config_with_registry_provider(
    path: &Path,
    provider: &dyn StrategyRegistryProvider,
) -> Result<RunConfig, CommandError> {
    let mut registry = compiled_strategy_registry()?;
    provider.register_strategies(path, &mut registry)?;
    Ok(osmium_config::load(path, &registry)?)
}

fn normalizer_config(
    config: &RunConfig,
    key: &run_planner::SourcePartitionKey,
) -> Result<data_sync::PartitionNormalizerConfig, CommandError> {
    let session_plan = config.session_plan_for(key)?;
    let kind = config
        .instrument_class_for(key.instrument())
        .ok_or_else(|| CommandError::Other("partition instrument is not selected".to_owned()))?;
    let contract_shape = config
        .selection_for(key.instrument())
        .and_then(|selection| selection.contract_shape());
    Ok(normalizer_config_for(
        config.effective().source(),
        key,
        kind,
        contract_shape,
        &session_plan,
    )
    .map_err(data_sync::SourceAdapterError::from)?)
}

fn load_dotenv() {
    if Path::new(".env").is_file() {
        let _ = dotenvy::dotenv();
    }
}

fn execute_sync(
    path: &Path,
    provider: &dyn StrategyRegistryProvider,
) -> Result<String, CommandError> {
    let config = load_config_with_registry_provider(path, provider)?;
    let bundle = plan(&config)?;
    for partition in bundle.execution.partitions() {
        match partition.source_action() {
            SourceAction::RejectIncomplete { .. } | SourceAction::RejectCorrupt { .. } => {
                let repository = PartitionedSourceRepository::new(
                    config.effective().data_root(),
                    partition.key().clone(),
                )?;
                let inspection = repository.inspect();
                return Err(source_not_complete_error(
                    partition.key(),
                    inspection.state(),
                    inspection.diagnostic(),
                ));
            }
            SourceAction::CoverageUnavailable => {
                return Err(source_not_complete_error(
                    partition.key(),
                    partition.source_state(),
                    Some("source coverage is unavailable"),
                ));
            }
            SourceAction::ReuseCompleteSource { .. }
            | SourceAction::DownloadMissingSource
            | SourceAction::ResumeOrRestartBuilding => {}
        }
    }
    let needs_network = bundle
        .execution
        .partitions()
        .iter()
        .any(|partition| partition.source_action().requires_network());
    if !needs_network {
        return Ok("source=reused\nhttp_requests=0".to_owned());
    }
    load_dotenv();
    let mut source_runtime = SourceAdapterRuntime::new(config.effective().source())?;
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
        let selection = config.selection_for(key.instrument()).ok_or_else(|| {
            CommandError::Other("partition instrument is not selected".to_owned())
        })?;
        let session_plan = config.session_plan_for(key)?;
        let report = source_runtime.sync_partition(
            config.effective().data_root(),
            key,
            selection.class(),
            selection.contract_shape(),
            &session_plan,
        )?;
        total_pages += u64::from(report.page_count());
        published += 1;
        output.push_str(&format!(
            "partition={:?}/{:?}@{} status=published pages={} revision={}\n",
            key.instrument().market(),
            key.instrument().symbol(),
            key.trading_date(),
            report.page_count(),
            report.revision_identity()
        ));
    }
    output.push_str(&format!("published={} pages={}\n", published, total_pages));
    Ok(output.trim_end().to_owned())
}

fn execute_verify(
    path: &Path,
    provider: &dyn StrategyRegistryProvider,
) -> Result<String, CommandError> {
    let config = load_config_with_registry_provider(path, provider)?;
    let mut output = String::from("source=verified\n");
    for key in config.partition_keys()? {
        let repository =
            PartitionedSourceRepository::new(config.effective().data_root(), key.clone())?;
        let inspection = repository.inspect();
        let report = match (inspection.state(), inspection.report()) {
            (SourceState::Complete { .. }, Some(report)) => report,
            (state, _) => {
                return Err(source_not_complete_error(
                    &key,
                    state,
                    inspection.diagnostic(),
                ));
            }
        };
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

fn prepare_cache(
    path: &Path,
    provider: &dyn StrategyRegistryProvider,
) -> Result<String, CommandError> {
    let config = load_config_with_registry_provider(path, provider)?;
    let bundle = plan(&config)?;
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
                    let repository = PartitionedSourceRepository::new(
                        config.effective().data_root(),
                        partition.key().clone(),
                    )?;
                    let inspection = repository.inspect();
                    return Err(source_not_complete_error(
                        partition.key(),
                        inspection.state(),
                        inspection.diagnostic(),
                    ));
                }
                let normalizer = normalizer_config(&config, partition.key())?;
                let built = builder.build_partition(partition.key(), normalizer)?;
                output.push_str(&format!(
                    "partition={:?}/{:?}@{} status=built cache_identity={}\n",
                    partition.key().instrument().market(),
                    partition.key().instrument().symbol(),
                    partition.key().trading_date(),
                    built.descriptor().cache_identity
                ));
            }
            CacheAction::AwaitCompleteSource => {
                let repository = PartitionedSourceRepository::new(
                    config.effective().data_root(),
                    partition.key().clone(),
                )?;
                let inspection = repository.inspect();
                return Err(source_not_complete_error(
                    partition.key(),
                    inspection.state(),
                    inspection.diagnostic(),
                ));
            }
        }
    }
    Ok(output.trim_end().to_owned())
}

fn source_not_complete_error(
    key: &run_planner::SourcePartitionKey,
    state: SourceState,
    diagnostic: Option<&str>,
) -> CommandError {
    CommandError::SourceNotComplete {
        partition: format!(
            "{:?}/{}@{}",
            key.instrument().market(),
            key.instrument().symbol().as_str(),
            key.trading_date()
        ),
        state,
        diagnostic: diagnostic.map(str::to_owned),
    }
}

fn execute_replay(
    path: &Path,
    provider: &dyn StrategyRegistryProvider,
) -> Result<String, CommandError> {
    let completed = replay(path, provider)?;
    Ok(format!(
        "replay=complete\nevents={}\nevent_checksum={}\nfinal_state_checksum={}",
        completed.summary().event_count(),
        hex(completed.summary().event_checksum().as_bytes()),
        hex(completed.summary().final_state_checksum().as_bytes())
    ))
}

fn replay(
    path: &Path,
    provider: &dyn StrategyRegistryProvider,
) -> Result<replay_engine::CompletedReplay, CommandError> {
    let config = load_config_with_registry_provider(path, provider)?;
    let bundle = plan(&config)?;
    let replay = bundle.replay.as_ref().ok_or(CommandError::CacheMissing)?;
    let mut core = replay_core(&config, &bundle)?;
    let mut factory = data_sync::LocalCacheFactory::new_partitioned(
        config.effective().data_root(),
        config.effective().source(),
    );
    core.replay_frozen_multi(replay, &mut factory)?;
    Ok(core.complete()?)
}

pub(crate) fn replay_core(
    config: &RunConfig,
    bundle: &osmium_config::PlanBundle,
) -> Result<ReplayCore, CommandError> {
    let mut states = BTreeMap::new();
    let mut reducers = BTreeMap::new();
    let mut contexts = BTreeMap::new();
    let mut schedules: BTreeMap<_, Vec<_>> = BTreeMap::new();
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
        let kind = config
            .instrument_class_for(key.instrument())
            .ok_or_else(|| {
                CommandError::Other("partition instrument is not selected".to_owned())
            })?;
        let reducer = match (key.instrument().market(), kind) {
            (MarketId::Twse, InstrumentClass::Warrant) => MarketStateReducer::twse_warrant(),
            (MarketId::Twse, _) => MarketStateReducer::twse_regular(),
            (MarketId::Tpex, InstrumentClass::Warrant) => MarketStateReducer::tpex_warrant(),
            (MarketId::Tpex, _) => MarketStateReducer::tpex_regular(),
            (MarketId::Taifex, InstrumentClass::Option) => MarketStateReducer::taifex_options(),
            (MarketId::Taifex, _) => MarketStateReducer::taifex_futures(),
        };
        states
            .entry(key.instrument().clone())
            .or_insert_with(|| MarketState::new(key.instrument().clone(), key.trading_date()));
        reducers.entry(key.instrument().clone()).or_insert(reducer);
        contexts.entry(key.instrument().clone()).or_insert(context);
        schedules
            .entry(key.instrument().clone())
            .or_default()
            .extend(windows);
    }
    Ok(ReplayCore::new_multi_with_schedules(
        states.into_values().collect(),
        reducers.into_iter().collect(),
        contexts.into_iter().collect(),
        schedules.into_iter().collect(),
    )?)
}

fn schedule(config: &RunConfig) -> Result<osmium_runner::MultiSessionSchedule, CommandError> {
    let mut entries: BTreeMap<_, Vec<_>> = BTreeMap::new();
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
        entries
            .entry(key.instrument().clone())
            .or_default()
            .extend(segments);
    }
    Ok(osmium_runner::MultiSessionSchedule::new(entries)?)
}

fn execute_backtest(
    path: &Path,
    output: &Path,
    provider: &dyn StrategyRegistryProvider,
) -> Result<String, CommandError> {
    let mut config = load_config_with_registry_provider(path, provider)?;
    let strategy_metadata = config.strategy_metadata().clone();
    let bundle = plan(&config)?;
    let replay = bundle.replay.as_ref().ok_or(CommandError::CacheMissing)?;
    let core = replay_core(&config, &bundle)?;
    let schedule = schedule(&config)?;
    let strategy = config.take_strategy()?;
    let simulation = bundle.execution.config().simulation();
    let fill = simulation.fill_model();
    let latency = simulation.latency();
    let slippage = match simulation.slippage_model() {
        SlippageModelConfig::AdverseFixedDelta { delta } => delta,
    };
    let mut ledger_configs = Vec::new();
    for economics in bundle.execution.config().instrument_economics() {
        let kind = config
            .instrument_class_for(economics.instrument())
            .ok_or_else(|| {
                CommandError::Other("economics instrument is not selected".to_owned())
            })?;
        let model = match kind {
            InstrumentClass::Option => AccountingModel::OptionsV1,
            InstrumentClass::Future => AccountingModel::FuturesV1,
            InstrumentClass::Equity | InstrumentClass::Warrant => AccountingModel::EquityV1,
        };
        let charges = simulation.charges_for(economics.instrument());
        let fee = charge(charges.map_or(simulation.fee_model(), |charges| charges.fee()));
        let tax = charge(charges.map_or(simulation.tax_model(), |charges| charges.tax()));
        let mut ledger_config = InstrumentLedgerConfig::new(
            economics.instrument().clone(),
            economics.quantity_unit(),
            model,
            InstrumentEconomics {
                units_per_trading_unit: economics.units_per_trading_unit(),
                multiplier: economics.multiplier(),
                provenance: economics.provenance().into(),
            },
            fee,
            tax,
        );
        let price_policy = bundle
            .execution
            .config()
            .instrument_contracts()
            .iter()
            .find(|contract| contract.instrument() == economics.instrument())
            .map(|contract| contract.price_policy())
            .ok_or_else(|| {
                CommandError::Other("instrument contract profile is missing".to_owned())
            })?;
        ledger_config = ledger_config.with_price_policy(price_policy);
        if let Some(day_trade) = charges.and_then(|charges| charges.day_trade_tax()) {
            ledger_config = ledger_config.with_day_trade_tax(DayTradeTaxModel::new_for_dates(
                tax,
                charge(day_trade.charge()),
                day_trade.timezone_offset_minutes(),
                day_trade.valid_through(),
                day_trade.eligible_dates().iter().copied(),
                day_trade.provenance(),
            )?);
        }
        ledger_configs.push(ledger_config);
    }
    let ledger = MultiLedger::new(simulation.initial_cash().amount(), ledger_configs)?;
    let allow_midpoint_fallback = match simulation.marking_policy() {
        run_planner::MarkingPolicyConfig::LastObservableV1 {
            allow_midpoint_fallback,
        } => allow_midpoint_fallback,
    };
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
    let mut factory = data_sync::LocalCacheFactory::new_partitioned(
        config.effective().data_root(),
        config.effective().source(),
    );
    match simulation.execution_policy() {
        ExecutionPolicyConfig::SubsequentEventV1 => {
            let simulator_configs = bundle
                .execution
                .config()
                .instrument_economics()
                .iter()
                .map(|economics| {
                    let price_policy = bundle
                        .execution
                        .config()
                        .instrument_contracts()
                        .iter()
                        .find(|contract| contract.instrument() == economics.instrument())
                        .map(|contract| contract.price_policy())
                        .ok_or_else(|| {
                            CommandError::Other("instrument contract profile is missing".to_owned())
                        })?;
                    Ok((
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
                        price_policy,
                    ))
                })
                .collect::<Result<Vec<_>, CommandError>>()?;
            let simulator = MultiSimulator::new(simulator_configs)?;
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
            osmium_runner::publish_multi_backtest(
                output,
                &completed,
                bundle.execution.identity().as_bytes(),
                &source_revision,
                &cache_identity,
                &strategy_metadata,
            )?;
            Ok(backtest_summary(
                completed.replay.summary().event_count(),
                completed.simulator.order_count(),
                completed.simulator.fill_count(),
                &completed.performance,
                output,
            ))
        }
        ExecutionPolicyConfig::ScheduledVisibleDepthV1 => {
            let scheduled = simulation.scheduled_execution().ok_or_else(|| {
                CommandError::Other("missing scheduled execution config".to_owned())
            })?;
            let simulator = ScheduledDepthSimulator::new(
                bundle
                    .execution
                    .config()
                    .instrument_economics()
                    .iter()
                    .map(|economics| {
                        let price_policy = bundle
                            .execution
                            .config()
                            .instrument_contracts()
                            .iter()
                            .find(|contract| contract.instrument() == economics.instrument())
                            .map(|contract| contract.price_policy())
                            .ok_or_else(|| {
                                CommandError::Other(
                                    "instrument contract profile is missing".to_owned(),
                                )
                            })?;
                        Ok(ScheduledInstrumentConfig::new(
                            economics.instrument().clone(),
                            economics.quantity_unit(),
                            ScheduledDepthModel::new(
                                usize::from(scheduled.depth_levels()),
                                scheduled.max_stale_ms(),
                                slippage,
                            )?,
                        )
                        .with_price_policy(price_policy))
                    })
                    .collect::<Result<Vec<_>, CommandError>>()?,
            )?;
            let completed = osmium_runner::run_scheduled_multi_backtest(
                core,
                strategy,
                replay,
                &mut factory,
                &schedule,
                latency.market_data_latency_ms(),
                simulator,
                ledger,
                allow_midpoint_fallback,
            )?;
            osmium_runner::publish_scheduled_multi_backtest(
                output,
                &completed,
                bundle.execution.identity().as_bytes(),
                &source_revision,
                &cache_identity,
                &strategy_metadata,
            )?;
            Ok(backtest_summary(
                completed.replay.summary().event_count(),
                completed.simulator.orders().len(),
                completed.simulator.fills().len(),
                &completed.performance,
                output,
            ))
        }
    }
}

fn backtest_summary(
    events: u64,
    orders: usize,
    fills: usize,
    performance: &execution_sim::MultiPerformanceSummary,
    output: &Path,
) -> String {
    format!(
        "backtest=complete\nevents={events}\norders={orders}\nfills={fills}\nfinal_cash_atoms={}\nrealized_pnl_atoms={}\nunrealized_pnl_atoms={}\noutput={}",
        performance.final_cash().atoms(),
        performance.realized_pnl().atoms(),
        performance.unrealized_pnl().atoms(),
        output.display()
    )
}

fn execute_run(
    path: &Path,
    output: Option<&Path>,
    provider: &dyn StrategyRegistryProvider,
) -> Result<String, CommandError> {
    let config = load_config_with_registry_provider(path, provider)?;
    let bundle = plan(&config)?;
    if bundle.execution.network_requirement() == NetworkRequirement::Required {
        execute_sync(path, provider)?;
    }
    prepare_cache(path, provider)?;
    match output {
        Some(output) => execute_backtest(path, output, provider),
        None => execute_replay(path, provider),
    }
}

fn charge(value: &run_planner::ChargeConfig) -> ChargeModel {
    ChargeModel {
        basis: match value.basis() {
            PlanChargeBasis::NotionalRate => ChargeBasis::NotionalRate,
            PlanChargeBasis::FixedPerUnit => ChargeBasis::FixedPerUnit,
        },
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug)]
pub enum CommandError {
    Config(osmium_config::ConfigError),
    Verify(data_sync::VerificationError),
    CacheBuild(data_sync::CacheBuildError),
    CacheRead(data_sync::CacheReadError),
    SourceAdapter(data_sync::SourceAdapterError),
    Replay(replay_engine::ReplayError),
    State(market_state::SessionSegmentIdError),
    Context(strategy_api::ContextError),
    Strategy(strategy_api::DeclarationError),
    Simulation(execution_sim::SimulationError),
    ScheduledSimulation(execution_sim::ScheduledSimulationError),
    Accounting(execution_sim::AccountingError),
    Backtest(osmium_runner::BacktestError),
    MultiBacktest(osmium_runner::MultiBacktestError),
    Artifact(osmium_runner::ArtifactError),
    Io(std::io::Error),
    Partition(data_sync::PartitionRepositoryError),
    ReplayContextWindow(replay_engine::ReplayContextWindowError),
    SourceNotComplete {
        partition: String,
        state: SourceState,
        diagnostic: Option<String>,
    },
    CacheMissing,
    OutputRequired,
    Other(String),
}

impl CommandError {
    #[must_use]
    pub const fn category(&self) -> ExitCategory {
        match self {
            Self::Config(_)
            | Self::State(_)
            | Self::Context(_)
            | Self::Strategy(_)
            | Self::ReplayContextWindow(_)
            | Self::SourceAdapter(data_sync::SourceAdapterError::Configuration(_)) => {
                ExitCategory::Config
            }
            Self::OutputRequired => ExitCategory::Usage,
            Self::SourceAdapter(_) | Self::Partition(_) => ExitCategory::Source,
            Self::CacheBuild(_) | Self::CacheRead(_) | Self::CacheMissing => ExitCategory::Cache,
            Self::Replay(_) => ExitCategory::Replay,
            Self::Backtest(_)
            | Self::MultiBacktest(_)
            | Self::Simulation(_)
            | Self::ScheduledSimulation(_)
            | Self::Accounting(_) => ExitCategory::Simulation,
            Self::Verify(_) | Self::Artifact(_) | Self::SourceNotComplete { .. } => {
                ExitCategory::Integrity
            }
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
convert!(Verify, data_sync::VerificationError);
convert!(CacheBuild, data_sync::CacheBuildError);
convert!(CacheRead, data_sync::CacheReadError);
convert!(SourceAdapter, data_sync::SourceAdapterError);
convert!(Replay, replay_engine::ReplayError);
convert!(State, market_state::SessionSegmentIdError);
convert!(Context, strategy_api::ContextError);
convert!(Strategy, strategy_api::DeclarationError);
convert!(Simulation, execution_sim::SimulationError);
convert!(ScheduledSimulation, execution_sim::ScheduledSimulationError);
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
            Self::SourceNotComplete {
                partition,
                state,
                diagnostic,
            } => {
                write!(
                    formatter,
                    "source partition {partition} is not complete: {state:?}"
                )?;
                if let Some(diagnostic) = diagnostic {
                    write!(formatter, " ({diagnostic})")?;
                }
                Ok(())
            }
            _ => write!(formatter, "{self:?}"),
        }
    }
}
impl Error for CommandError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicBool, Ordering},
    };

    use data_sync::{
        ArchiveKind, ArchiveTimestamp, CursorStateMachine, StagingRevision, TeralionQuery,
    };
    use market_types::{InstrumentId, Symbol, TradingDate};
    use run_planner::{CorruptReason, SessionPlan, SourceId, SourcePartitionKey};
    use strategy_api::SessionKind;

    #[test]
    fn command_exit_codes_preserve_stable_failure_categories() {
        assert_eq!(CommandError::OutputRequired.exit_code(), 2);
        assert_eq!(
            CommandError::Config(osmium_config::ConfigError::Invalid("field")).exit_code(),
            10
        );
        assert_eq!(CommandError::CacheMissing.exit_code(), 21);
        assert_eq!(
            CommandError::SourceAdapter(data_sync::SourceAdapterError::MissingCredential)
                .exit_code(),
            20
        );
        assert_eq!(
            CommandError::Replay(replay_engine::ReplayError::EmptyUniverse).exit_code(),
            30
        );
        assert_eq!(CommandError::Other("internal".to_owned()).exit_code(), 1);
    }

    #[test]
    fn incomplete_source_error_identifies_partition_state_and_diagnostic() {
        let instrument = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let trading_date: TradingDate = "2026-07-27".parse().unwrap();
        let session = SessionPlan::for_instrument_class(
            &instrument,
            InstrumentClass::Equity,
            trading_date,
            [SessionKind::Regular],
        )
        .unwrap();
        let key = SourcePartitionKey::new(
            SourceId::TeralionFeedArchive,
            instrument,
            trading_date,
            [SessionKind::Regular],
            session.identity(),
        )
        .unwrap();
        let error = source_not_complete_error(
            &key,
            SourceState::Corrupt {
                reason: CorruptReason::ReferenceMismatch,
            },
            Some("partition metadata does not match key"),
        );

        assert_eq!(error.category(), ExitCategory::Integrity);
        let message = error.to_string();
        assert!(message.contains("Twse/2330@2026-07-27"));
        assert!(message.contains("ReferenceMismatch"));
        assert!(message.contains("partition metadata does not match key"));
    }

    #[test]
    fn source_commands_reject_a_mismatched_partition_descriptor_with_context() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("config.yaml");
        let example = fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/config.yaml"),
        )
        .unwrap();
        fs::write(
            &config_path,
            example.replace(
                "data_root: data",
                &format!("data_root: {}", root.path().display()),
            ),
        )
        .unwrap();
        let config =
            load_config_with_registry_provider(&config_path, &BuiltInStrategyRegistry).unwrap();
        let key = config.partition_keys().unwrap().into_iter().next().unwrap();
        let ticks = TeralionQuery::ticks(
            key.instrument().clone(),
            ArchiveTimestamp::parse("2026-07-27T08:55:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-27T13:35:00+08:00").unwrap(),
            [ArchiveKind::Quote],
            5_000,
        )
        .unwrap();
        let tick = br#"{"type":"quote","market":"twse","format":"STOCK_SNAPSHOT","symbol":"2330","match_time":"2026-07-27T09:00:00+08:00","received_at":"2026-07-27T09:00:00+08:00","status_flags":16,"limit_flags":0,"cum_volume":1,"intermediate_print":false,"deal":{"price":100,"quantity":1},"bids":[{"price":99,"quantity":2}],"asks":[{"price":101,"quantity":2}]}"#;
        let body = format!(
            r#"{{"items":[{}],"next_cursor":null}}"#,
            std::str::from_utf8(tick).unwrap()
        )
        .into_bytes();
        let mut staging =
            StagingRevision::create_for_partition(root.path(), &key, "cli-diagnostic").unwrap();
        let mut cursor = CursorStateMachine::new(ticks.clone()).unwrap();
        let request = cursor.request_next().unwrap();
        let pending = cursor.accept_response(&request, body).unwrap();
        let staged = staging.stage_page(pending).unwrap();
        cursor.commit_page(staged.commit_receipt()).unwrap();
        staging
            .stage_daily_instrument(
                TeralionQuery::daily_instrument(key.instrument().clone(), key.trading_date())
                    .identity(),
                br#"{}"#,
            )
            .unwrap();
        staging.publish(ticks.identity(), true).unwrap();

        let repository = PartitionedSourceRepository::new(root.path(), key).unwrap();
        let manifest_path = repository.partition_manifest_path();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["layout_version"] = serde_json::json!(1);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        for kind in [
            CommandKind::DataSync,
            CommandKind::DataVerify,
            CommandKind::CachePrepare,
        ] {
            let error = execute(&Command {
                kind,
                config: config_path.clone(),
                output: None,
            })
            .unwrap_err();
            assert_eq!(error.category(), ExitCategory::Integrity);
            let message = error.to_string();
            assert!(message.contains("Twse/2330@2026-07-27"));
            assert!(message.contains("ReferenceMismatch"));
            assert!(message.contains("partition metadata does not match key"));
        }
    }

    #[test]
    fn run_config_plan_lists_the_representative_partition_without_source_access() {
        let summary = execute(&Command {
            kind: CommandKind::Plan,
            config: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/config.yaml"),
            output: None,
        })
        .unwrap();
        assert!(summary.contains("partitions=1"));
        assert!(summary.contains("2330"));
        assert!(summary.contains("Regular"));
    }

    #[test]
    fn external_registry_provider_is_invoked_for_config_loading() {
        let invoked = AtomicBool::new(false);
        let provider = |_: &Path, _: &mut StrategyRegistry| {
            invoked.store(true, Ordering::SeqCst);
            Ok(())
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/config.yaml");

        execute_config_check_with_registry_provider(&path, &provider).unwrap();

        assert!(invoked.load(Ordering::SeqCst));
    }

    #[test]
    fn external_registry_provider_failure_is_a_config_failure() {
        let provider = |_: &Path, _: &mut StrategyRegistry| {
            Err(CommandError::Config(osmium_config::ConfigError::Invalid(
                "external strategy registry",
            )))
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/config.yaml");

        let error = execute_config_check_with_registry_provider(&path, &provider).unwrap_err();

        assert_eq!(error.category(), ExitCategory::Config);
    }
}
