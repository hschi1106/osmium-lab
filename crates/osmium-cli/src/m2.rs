use std::{
    env,
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use data_sync::{
    ArchiveKind, ArchiveTimestamp, CacheBuilder, CacheReader, FeedArchiveTransport,
    LocalSourceRepository, PartitionNormalizerConfig, PartitionedSourceRepository, StagingRevision,
    TeralionCredential, TeralionQuery, TeralionSync,
};
use execution_sim::{
    AccountingModel, ChargeModel, ChargeSides, EvidenceMode, FillModel, InstrumentEconomics,
    InstrumentLedgerConfig, Ledger, MultiLedger, MultiSimulator, QuantityPolicy, RoundingPolicy,
    Simulator,
};
use m2_config::{M2PlanBundle, load, plan};
use m3_config::{
    M3_CONFIG_VERSION, M3Config, config_version as m3_config_version, load as load_m3,
    plan as plan_m3,
};
use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{InstrumentKind, MarketId};
use replay_engine::{ReplayContextWindow, ReplayCore};
use run_planner::{
    CacheAction, ChargeSides as PlanChargeSides, FillEvidence, NetworkRequirement,
    QuantityEvidence, RoundingPolicy as PlanRounding, SlippageModelConfig, SourceAction,
    SourceState,
};
use strategy_api::M2AcceptanceStrategy;
use strategy_api::{
    M3_ACCEPTANCE_STRATEGY_ID, M3_ACCEPTANCE_STRATEGY_VERSION, M3AcceptanceStrategy, SessionKind,
    SessionSegment,
};
use taifex_normalizer::NormalizerConfig as TaifexNormalizerConfig;
use tpex_normalizer::NormalizerConfig as TpexNormalizerConfig;
use twse_normalizer::NormalizerConfig as TwseNormalizerConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum M2CommandKind {
    Plan,
    Sync,
    Verify,
    CachePrepare,
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
        M2CommandKind::Sync => {
            if is_m3_config(&command.config)? {
                execute_m3_sync(&command.config)
            } else {
                execute_sync(&command.config)
            }
        }
        M2CommandKind::Verify => {
            if is_m3_config(&command.config)? {
                execute_m3_verify(&command.config)
            } else {
                execute_verify(&command.config)
            }
        }
        M2CommandKind::CachePrepare => prepare_cache(&command.config),
        M2CommandKind::Replay => {
            if is_m3_config(&command.config)? {
                execute_m3_replay(&command.config)
            } else {
                execute_replay(&command.config)
            }
        }
        M2CommandKind::Backtest => {
            if is_m3_config(&command.config)? {
                execute_m3_backtest(
                    &command.config,
                    command
                        .output
                        .as_deref()
                        .ok_or(M2CommandError::OutputRequired)?,
                )
            } else {
                execute_backtest(
                    &command.config,
                    command
                        .output
                        .as_deref()
                        .ok_or(M2CommandError::OutputRequired)?,
                )
            }
        }
        M2CommandKind::Run => {
            if is_m3_config(&command.config)? {
                return execute_m3_run(&command.config, command.output.as_deref());
            }
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
    if m3_config_version(path)? == M3_CONFIG_VERSION {
        let bundle = plan_m3(load_m3(path)?)?;
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
        return Ok(output);
    }
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

fn is_m3_config(path: &Path) -> Result<bool, M2CommandError> {
    Ok(m3_config_version(path)? == M3_CONFIG_VERSION)
}

fn m3_queries(
    config: &M3Config,
    key: &run_planner::SourcePartitionKey,
) -> Result<(TeralionQuery, TeralionQuery, PartitionNormalizerConfig), M2CommandError> {
    let session_plan = config.session_plan_for(key)?;
    let replay_start = session_plan
        .windows()
        .iter()
        .map(|window| window.replay_start())
        .min()
        .ok_or_else(|| M2CommandError::Other("M3 session plan has no windows".to_owned()))?;
    let replay_end_exclusive = session_plan
        .windows()
        .iter()
        .map(|window| window.replay_end_exclusive())
        .max()
        .ok_or_else(|| M2CommandError::Other("M3 session plan has no windows".to_owned()))?;
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

fn m3_attempt_id(key: &run_planner::SourcePartitionKey) -> String {
    let identity = hex(key.identity().as_bytes());
    format!("m3-{}", &identity[..24])
}

fn load_dotenv() {
    if Path::new(".env").is_file() {
        let _ = dotenvy::dotenv();
    }
}

fn execute_m3_sync(path: &Path) -> Result<String, M2CommandError> {
    let config = load_m3(path)?;
    let bundle = plan_m3(config.clone())?;
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
        env::var("TERALION_API_KEY").map_err(|_| M2CommandError::MissingCredential)?,
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
        let (ticks, daily_query, _) = m3_queries(&config, key)?;
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
        let attempt = m3_attempt_id(key);
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

fn execute_m3_verify(path: &Path) -> Result<String, M2CommandError> {
    let config = load_m3(path)?;
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

fn execute_sync(path: &Path) -> Result<String, M2CommandError> {
    let config = load(path)?;
    let initial = plan(config.clone())?;
    if matches!(
        initial.execution.partitions()[0].source_action(),
        SourceAction::ReuseCompleteSource { .. }
    ) {
        return Ok("source=reused\nhttp_requests=0".to_owned());
    }
    load_dotenv();
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
    if is_m3_config(path)? {
        return prepare_m3_cache(path);
    }
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
    let built = CacheBuilder::new(config.data_root()).build_current(TwseNormalizerConfig::new(
        config.universe()[0].clone(),
        config.trading_dates()[0],
        session.replay_start,
        session.replay_end_exclusive,
    )?)?;
    Ok(format!("cache={}", built.descriptor().cache_identity))
}

fn prepare_m3_cache(path: &Path) -> Result<String, M2CommandError> {
    let config = load_m3(path)?;
    let bundle = plan_m3(config.clone())?;
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
                    return Err(M2CommandError::CacheMissing);
                }
                let (_, _, normalizer) = m3_queries(&config, partition.key())?;
                let built = builder.build_partition(partition.key(), normalizer)?;
                output.push_str(&format!(
                    "partition={:?}/{:?}@{} status=built cache_identity={}\n",
                    partition.key().instrument().market(),
                    partition.key().instrument().symbol(),
                    partition.key().trading_date(),
                    built.descriptor().cache_identity
                ));
            }
            CacheAction::AwaitCompleteSource => return Err(M2CommandError::CacheMissing),
        }
    }
    Ok(output.trim_end().to_owned())
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

fn execute_m3_replay(path: &Path) -> Result<String, M2CommandError> {
    let completed = replay_m3(path)?;
    Ok(format!(
        "replay=complete\nevents={}\nevent_checksum={}\nfinal_state_checksum={}",
        completed.summary().event_count(),
        hex(completed.summary().event_checksum().as_bytes()),
        hex(completed.summary().final_state_checksum().as_bytes())
    ))
}

fn replay_m3(path: &Path) -> Result<replay_engine::CompletedReplay, M2CommandError> {
    let config = load_m3(path)?;
    let bundle = plan_m3(config.clone())?;
    let replay = bundle.replay.as_ref().ok_or(M2CommandError::CacheMissing)?;
    let mut core = m3_core(&config, &bundle)?;
    let mut factory = data_sync::LocalCacheFactory::new_partitioned(config.effective().data_root());
    core.replay_frozen_multi(replay, &mut factory)?;
    Ok(core.complete()?)
}

pub(crate) fn m3_core(
    config: &M3Config,
    bundle: &m3_config::M3PlanBundle,
) -> Result<ReplayCore, M2CommandError> {
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
            .ok_or_else(|| M2CommandError::Other("M3 session plan has no windows".to_owned()))?;
        let kind = config.instrument_kind_for(key.instrument());
        let reducer = match (key.instrument().market(), kind) {
            (MarketId::Twse, InstrumentKind::Warrant) => MarketStateReducer::twse_warrant(),
            (MarketId::Twse, _) => MarketStateReducer::twse_regular(),
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

fn m3_schedule(config: &M3Config) -> Result<m2_runner::MultiSessionSchedule, M2CommandError> {
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
    Ok(m2_runner::MultiSessionSchedule::new(entries)?)
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
    let latency = simulation.latency();
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
            market_data_latency_ms: latency.market_data_latency_ms(),
            order_latency_ms: latency.order_latency_ms(),
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

fn execute_m3_backtest(path: &Path, output: &Path) -> Result<String, M2CommandError> {
    let config = load_m3(path)?;
    let bundle = plan_m3(config.clone())?;
    if bundle
        .execution
        .config()
        .strategy()
        .identity()
        .strategy_id()
        != M3_ACCEPTANCE_STRATEGY_ID
        || bundle
            .execution
            .config()
            .strategy()
            .identity()
            .strategy_version()
            != M3_ACCEPTANCE_STRATEGY_VERSION
    {
        return Err(M2CommandError::M3Unsupported);
    }
    let replay = bundle.replay.as_ref().ok_or(M2CommandError::CacheMissing)?;
    let core = m3_core(&config, &bundle)?;
    let schedule = m3_schedule(&config)?;
    let strategy = M3AcceptanceStrategy::new(
        M3AcceptanceStrategy::source_binary_identity()?,
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
    let completed = m2_runner::run_multi_backtest(
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
            _ => return Err(M2CommandError::CacheMissing),
        };
        let cache = match partition.cache_action() {
            CacheAction::ReuseValidCache { identity } => hex(identity.as_bytes()),
            _ => return Err(M2CommandError::CacheMissing),
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
    m2_runner::publish_multi_backtest(
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

fn execute_m3_run(path: &Path, output: Option<&Path>) -> Result<String, M2CommandError> {
    let config = load_m3(path)?;
    let bundle = plan_m3(config)?;
    if bundle.execution.network_requirement() == NetworkRequirement::Required {
        execute_m3_sync(path)?;
    }
    prepare_m3_cache(path)?;
    match output {
        Some(output) => execute_m3_backtest(path, output),
        None => execute_m3_replay(path),
    }
}

fn ready_bundle(path: &Path) -> Result<M2PlanBundle, M2CommandError> {
    let bundle = plan(load(path)?)?;
    if bundle.replay.is_none() {
        return Err(M2CommandError::CacheMissing);
    }
    Ok(bundle)
}

pub(crate) fn core(bundle: &M2PlanBundle) -> Result<ReplayCore, M2CommandError> {
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
    M3Config(m3_config::M3ConfigError),
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
    Backtest(m2_runner::BacktestError),
    MultiBacktest(m2_runner::MultiBacktestError),
    Artifact(m2_runner::ArtifactError),
    Io(std::io::Error),
    Partition(data_sync::PartitionRepositoryError),
    ReplayContextWindow(replay_engine::ReplayContextWindowError),
    MissingCredential,
    CacheMissing,
    OutputRequired,
    M3Unsupported,
    Other(String),
}

impl M2CommandError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Config(_)
            | Self::Query(_)
            | Self::Normalizer(_)
            | Self::TpexNormalizer(_)
            | Self::TaifexNormalizer(_)
            | Self::State(_)
            | Self::Context(_)
            | Self::Strategy(_)
            | Self::OutputRequired
            | Self::M3Config(_)
            | Self::M3Unsupported => 2,
            Self::Verify(_)
            | Self::CacheBuild(_)
            | Self::CacheRead(_)
            | Self::Staging(_)
            | Self::Artifact(_)
            | Self::CacheMissing => 20,
            Self::Transport(_) | Self::Sync(_) | Self::MissingCredential => 30,
            Self::Replay(_)
            | Self::Backtest(_)
            | Self::MultiBacktest(_)
            | Self::Simulation(_)
            | Self::Accounting(_) => 50,
            Self::Partition(_) | Self::Io(_) | Self::ReplayContextWindow(_) | Self::Other(_) => 1,
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
convert!(M3Config, m3_config::M3ConfigError);
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
convert!(Backtest, m2_runner::BacktestError);
convert!(MultiBacktest, m2_runner::MultiBacktestError);
convert!(Artifact, m2_runner::ArtifactError);
convert!(Partition, data_sync::PartitionRepositoryError);
convert!(ReplayContextWindow, replay_engine::ReplayContextWindowError);
convert!(Io, std::io::Error);

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

    #[test]
    fn m3_plan_command_lists_each_partition_without_source_access() {
        let summary = execute(&M2Command {
            kind: M2CommandKind::Plan,
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
