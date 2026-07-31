use std::{
    error::Error,
    fmt,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use market_state::{
    CANONICAL_FINAL_STATE_SET_VERSION, MARKET_STATE_VERSION, MarketState, MarketStateReducer,
    ReducerContext, STATE_REDUCER_VERSION, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{
    CANONICAL_EVENT_VERSION, CanonicalEncodingError, DomainEvent, EVENT_SCHEMA_VERSION,
    EventPayload, InstrumentId, MARKET_TYPES_VERSION, MarketId, MatchTime, Symbol, TradingDate,
    append_length,
};
use replay_engine::{
    CANONICAL_REPLAY_EVENT_STREAM_VERSION, ORDERING_RULE_VERSION, REPLAY_ENGINE_VERSION,
    ReplayCore, ReplayEventStreamChecksum, order_events,
};
use strategy_api::{
    CANONICAL_STRATEGY_OUTPUT_VERSION, ExampleStrategy, STRATEGY_API_VERSION, SessionKind,
    SessionSegment, StrategyOutputChecksum, StrategyRunError, run_strategy,
};
use twse_normalizer::{
    MAPPING_VERSION, NormalizationError, NormalizationWarning, NormalizerConfig, TwseNormalizer,
    WarningKind,
};

pub const M1_RUN_SUMMARY_VERSION: u16 = 1;
pub const CANONICAL_NORMALIZED_EVENT_SET_VERSION: u16 = 1;
const NORMALIZED_EVENT_SET_MAGIC: &[u8; 4] = b"OSNE";
static ARTIFACT_EXPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct M1FixtureInput {
    events: Vec<DomainEvent>,
    input_record_count: u64,
    outside_replay_window_count: usize,
    known_skipped_count: usize,
    warnings: Vec<M1Warning>,
}

impl M1FixtureInput {
    pub fn load(fixture_directory: &Path) -> Result<Self, M1RunError> {
        let mut shards = fs::read_dir(fixture_directory)
            .map_err(M1RunError::Io)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(M1RunError::Io)?;
        shards.retain(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        });
        shards.sort();
        if shards.is_empty() {
            return Err(M1RunError::EmptyFixture);
        }

        let mut materialized = Vec::new();
        for path in shards {
            let reader = BufReader::new(File::open(path).map_err(M1RunError::Io)?);
            for line in reader.lines() {
                materialized.push(line.map_err(M1RunError::Io)?);
            }
        }
        let report = TwseNormalizer::new(normalizer_config()?)
            .normalize_json_lines(materialized)
            .map_err(M1RunError::Normalization)?;
        let warnings = report.warnings().iter().map(M1Warning::from).collect();
        Ok(Self {
            input_record_count: report.input_records(),
            outside_replay_window_count: report.outside_replay_window().len(),
            known_skipped_count: report.known_skipped().len(),
            warnings,
            events: report.into_events(),
        })
    }

    #[must_use]
    pub fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    pub fn run(&self) -> Result<M1RunArtifacts, M1RunError> {
        self.run_with_events(self.events.clone())
    }

    pub fn run_with_events(&self, events: Vec<DomainEvent>) -> Result<M1RunArtifacts, M1RunError> {
        if events.len() != self.events.len() {
            return Err(M1RunError::EventSetMismatch);
        }
        let normalized_events = canonical_normalized_events(events.clone())?;
        let normalized_events_checksum =
            NormalizedEventsChecksum(*blake3::hash(&normalized_events).as_bytes());
        let instrument = m1_instrument()?;
        let strategy = ExampleStrategy::new(
            ExampleStrategy::source_binary_identity().map_err(M1RunError::Declaration)?,
            instrument.clone(),
        )
        .map_err(M1RunError::Declaration)?;
        let completed = run_strategy(
            replay_core(instrument.clone())?,
            strategy,
            &regular_segment()?,
            events,
        )
        .map_err(M1RunError::Strategy)?;
        let strategy_output = completed
            .strategy_output()
            .to_canonical_bytes()
            .map_err(M1RunError::StrategyOutput)?;
        let strategy_output_checksum = completed
            .strategy_output()
            .checksum()
            .map_err(M1RunError::StrategyOutput)?;
        let summary = completed.replay().summary();
        let quote_snapshot_count = self
            .events
            .iter()
            .filter(|event| matches!(event.payload(), EventPayload::QuoteSnapshot(_)))
            .count();
        let trade_batch_count = self
            .events
            .iter()
            .filter(|event| matches!(event.payload(), EventPayload::TradeBatch(_)))
            .count();

        Ok(M1RunArtifacts {
            normalized_events,
            strategy_output,
            warnings: self.warnings.clone(),
            summary: M1RunSummary {
                run_summary_version: M1_RUN_SUMMARY_VERSION,
                mapping_version: MAPPING_VERSION,
                market_types_version: MARKET_TYPES_VERSION,
                event_schema_version: EVENT_SCHEMA_VERSION,
                canonical_event_version: CANONICAL_EVENT_VERSION,
                normalized_event_set_version: CANONICAL_NORMALIZED_EVENT_SET_VERSION,
                ordering_rule_version: ORDERING_RULE_VERSION,
                replay_engine_version: REPLAY_ENGINE_VERSION,
                replay_event_stream_version: CANONICAL_REPLAY_EVENT_STREAM_VERSION,
                market_state_version: MARKET_STATE_VERSION,
                state_reducer_version: STATE_REDUCER_VERSION,
                final_state_set_version: CANONICAL_FINAL_STATE_SET_VERSION,
                strategy_api_version: STRATEGY_API_VERSION,
                strategy_output_version: CANONICAL_STRATEGY_OUTPUT_VERSION,
                input_record_count: self.input_record_count,
                outside_replay_window_count: self.outside_replay_window_count,
                known_skipped_count: self.known_skipped_count,
                normalized_event_count: self.events.len(),
                quote_snapshot_count,
                trade_batch_count,
                callback_count: completed.callback_count(),
                strategy_output_record_count: completed.strategy_output().records().len(),
                warning_count: self.warnings.len(),
                event_stream_checksum: summary.event_checksum(),
                final_state_checksum: *summary.final_state_checksum().as_bytes(),
                normalized_events_checksum,
                strategy_output_checksum,
            },
        })
    }
}

#[derive(Debug)]
pub struct M1RunArtifacts {
    normalized_events: Vec<u8>,
    strategy_output: Vec<u8>,
    warnings: Vec<M1Warning>,
    summary: M1RunSummary,
}

impl M1RunArtifacts {
    #[must_use]
    pub fn normalized_events(&self) -> &[u8] {
        &self.normalized_events
    }

    #[must_use]
    pub fn strategy_output(&self) -> &[u8] {
        &self.strategy_output
    }

    #[must_use]
    pub fn warnings(&self) -> &[M1Warning] {
        &self.warnings
    }

    #[must_use]
    pub const fn summary(&self) -> &M1RunSummary {
        &self.summary
    }

    /// Writes the complete replay artifact set through a staging directory.
    ///
    /// The destination must not already exist. A successful rename publishes the
    /// complete set atomically; failed staging output is removed best-effort.
    pub fn export(
        &self,
        output_directory: &Path,
        fixture_metadata: &Path,
        fixture_set_checksum: &Path,
    ) -> Result<(), ArtifactExportError> {
        if output_directory.exists() {
            return Err(ArtifactExportError::OutputExists(
                output_directory.to_path_buf(),
            ));
        }
        let name = output_directory
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or(ArtifactExportError::InvalidOutputDirectory)?;
        let parent = output_directory
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| ArtifactExportError::io(parent, source))?;
        let sequence = ARTIFACT_EXPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let staging = parent.join(format!(
            ".{name}.osmium-staging-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&staging).map_err(|source| ArtifactExportError::io(&staging, source))?;

        let result = self
            .write_staging_artifacts(&staging, fixture_metadata, fixture_set_checksum)
            .and_then(|()| {
                fs::rename(&staging, output_directory)
                    .map_err(|source| ArtifactExportError::io(output_directory, source))
            });
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn write_staging_artifacts(
        &self,
        staging: &Path,
        fixture_metadata: &Path,
        fixture_set_checksum: &Path,
    ) -> Result<(), ArtifactExportError> {
        let fixture_set_checksum_text = fs::read_to_string(fixture_set_checksum)
            .map_err(|source| ArtifactExportError::io(fixture_set_checksum, source))?;
        let fixture_set_checksum_value = fixture_set_checksum_text.trim();
        if fixture_set_checksum_value.len() != 64
            || !fixture_set_checksum_value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ArtifactExportError::InvalidFixtureSetChecksum);
        }
        copy_artifact(fixture_metadata, &staging.join("fixture-metadata.yaml"))?;
        copy_artifact(fixture_set_checksum, &staging.join("fixture-set.sha256"))?;
        write_artifact(
            &staging.join("normalized-events.bin"),
            &self.normalized_events,
        )?;
        write_artifact(
            &staging.join("event-stream.blake3"),
            format!(
                "{}\n",
                encode_hex(self.summary.event_stream_checksum.as_bytes())
            )
            .as_bytes(),
        )?;
        write_artifact(
            &staging.join("final-state.blake3"),
            format!("{}\n", encode_hex(&self.summary.final_state_checksum)).as_bytes(),
        )?;
        write_artifact(&staging.join("strategy-output.bin"), &self.strategy_output)?;
        write_artifact(
            &staging.join("strategy-output.blake3"),
            format!(
                "{}\n",
                encode_hex(self.summary.strategy_output_checksum.as_bytes())
            )
            .as_bytes(),
        )?;
        write_artifact(
            &staging.join("warnings.yaml"),
            warnings_yaml(&self.warnings).as_bytes(),
        )?;
        write_artifact(
            &staging.join("run-summary.yaml"),
            self.summary.to_yaml(fixture_set_checksum_value).as_bytes(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M1Warning {
    kind: WarningKind,
    match_time: Option<MatchTime>,
}

impl M1Warning {
    #[must_use]
    pub const fn kind(&self) -> WarningKind {
        self.kind
    }

    #[must_use]
    pub const fn match_time(&self) -> Option<MatchTime> {
        self.match_time
    }
}

impl From<&NormalizationWarning> for M1Warning {
    fn from(warning: &NormalizationWarning) -> Self {
        Self {
            kind: warning.kind(),
            match_time: warning.context().match_time(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedEventsChecksum([u8; 32]);

impl NormalizedEventsChecksum {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct M1RunSummary {
    pub run_summary_version: u16,
    pub mapping_version: u16,
    pub market_types_version: u16,
    pub event_schema_version: u16,
    pub canonical_event_version: u16,
    pub normalized_event_set_version: u16,
    pub ordering_rule_version: u16,
    pub replay_engine_version: u16,
    pub replay_event_stream_version: u16,
    pub market_state_version: u16,
    pub state_reducer_version: u16,
    pub final_state_set_version: u16,
    pub strategy_api_version: u16,
    pub strategy_output_version: u16,
    pub input_record_count: u64,
    pub outside_replay_window_count: usize,
    pub known_skipped_count: usize,
    pub normalized_event_count: usize,
    pub quote_snapshot_count: usize,
    pub trade_batch_count: usize,
    pub callback_count: u64,
    pub strategy_output_record_count: usize,
    pub warning_count: usize,
    pub event_stream_checksum: ReplayEventStreamChecksum,
    pub final_state_checksum: [u8; 32],
    pub normalized_events_checksum: NormalizedEventsChecksum,
    pub strategy_output_checksum: StrategyOutputChecksum,
}

impl M1RunSummary {
    #[must_use]
    pub fn to_yaml(self, fixture_set_sha256: &str) -> String {
        format!(
            concat!(
                "run_summary_version: {run_summary_version}\n",
                "outcome: passed\n",
                "\n",
                "versions:\n",
                "  mapping: {mapping_version}\n",
                "  market_types: {market_types_version}\n",
                "  event_schema: {event_schema_version}\n",
                "  canonical_event: {canonical_event_version}\n",
                "  normalized_event_set: {normalized_event_set_version}\n",
                "  ordering_rule: {ordering_rule_version}\n",
                "  replay_engine: {replay_engine_version}\n",
                "  replay_event_stream: {replay_event_stream_version}\n",
                "  market_state: {market_state_version}\n",
                "  state_reducer: {state_reducer_version}\n",
                "  final_state_set: {final_state_set_version}\n",
                "  strategy_api: {strategy_api_version}\n",
                "  strategy_output: {strategy_output_version}\n",
                "\n",
                "counts:\n",
                "  input_records: {input_record_count}\n",
                "  outside_replay_window: {outside_replay_window_count}\n",
                "  known_skipped: {known_skipped_count}\n",
                "  normalized_events: {normalized_event_count}\n",
                "  quote_snapshots: {quote_snapshot_count}\n",
                "  trade_batches: {trade_batch_count}\n",
                "  strategy_callbacks: {callback_count}\n",
                "  strategy_output_records: {strategy_output_record_count}\n",
                "  warnings: {warning_count}\n",
                "\n",
                "checksums:\n",
                "  fixture_set_sha256: {fixture_set_sha256}\n",
                "  normalized_events_blake3: {normalized_events_checksum}\n",
                "  event_stream_blake3: {event_stream_checksum}\n",
                "  final_state_blake3: {final_state_checksum}\n",
                "  strategy_output_blake3: {strategy_output_checksum}\n",
            ),
            run_summary_version = self.run_summary_version,
            mapping_version = self.mapping_version,
            market_types_version = self.market_types_version,
            event_schema_version = self.event_schema_version,
            canonical_event_version = self.canonical_event_version,
            normalized_event_set_version = self.normalized_event_set_version,
            ordering_rule_version = self.ordering_rule_version,
            replay_engine_version = self.replay_engine_version,
            replay_event_stream_version = self.replay_event_stream_version,
            market_state_version = self.market_state_version,
            state_reducer_version = self.state_reducer_version,
            final_state_set_version = self.final_state_set_version,
            strategy_api_version = self.strategy_api_version,
            strategy_output_version = self.strategy_output_version,
            input_record_count = self.input_record_count,
            outside_replay_window_count = self.outside_replay_window_count,
            known_skipped_count = self.known_skipped_count,
            normalized_event_count = self.normalized_event_count,
            quote_snapshot_count = self.quote_snapshot_count,
            trade_batch_count = self.trade_batch_count,
            callback_count = self.callback_count,
            strategy_output_record_count = self.strategy_output_record_count,
            warning_count = self.warning_count,
            fixture_set_sha256 = fixture_set_sha256,
            normalized_events_checksum = encode_hex(self.normalized_events_checksum.as_bytes()),
            event_stream_checksum = encode_hex(self.event_stream_checksum.as_bytes()),
            final_state_checksum = encode_hex(&self.final_state_checksum),
            strategy_output_checksum = encode_hex(self.strategy_output_checksum.as_bytes()),
        )
    }
}

#[derive(Debug)]
pub enum ArtifactExportError {
    InvalidOutputDirectory,
    InvalidFixtureSetChecksum,
    OutputExists(PathBuf),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl ArtifactExportError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for ArtifactExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOutputDirectory => {
                formatter.write_str("artifact output directory must have a final path component")
            }
            Self::InvalidFixtureSetChecksum => {
                formatter.write_str("fixture-set checksum must be exactly 64 hexadecimal digits")
            }
            Self::OutputExists(path) => {
                write!(
                    formatter,
                    "artifact output directory already exists: {}",
                    path.display()
                )
            }
            Self::Io { path, source } => {
                write!(
                    formatter,
                    "artifact I/O failed at {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for ArtifactExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidOutputDirectory
            | Self::InvalidFixtureSetChecksum
            | Self::OutputExists(_) => None,
        }
    }
}

fn write_artifact(path: &Path, bytes: &[u8]) -> Result<(), ArtifactExportError> {
    fs::write(path, bytes).map_err(|source| ArtifactExportError::io(path, source))
}

fn copy_artifact(source: &Path, destination: &Path) -> Result<(), ArtifactExportError> {
    fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| ArtifactExportError::io(source, error))
}

fn warnings_yaml(warnings: &[M1Warning]) -> String {
    let mut yaml = String::from("warnings_version: 1\n");
    if warnings.is_empty() {
        yaml.push_str("warnings: []\n");
        return yaml;
    }
    yaml.push_str("warnings:\n");
    for warning in warnings {
        let (kind, raw) = match warning.kind() {
            WarningKind::ReservedStatusBits(raw) => ("reserved_status_bits", Some(raw)),
            WarningKind::ReservedTradeLimit => ("reserved_trade_limit", None),
            WarningKind::ReservedBestBidLimit => ("reserved_best_bid_limit", None),
            WarningKind::ReservedBestAskLimit => ("reserved_best_ask_limit", None),
            WarningKind::ReservedInstantTrend => ("reserved_instant_trend", None),
        };
        yaml.push_str("  - kind: ");
        yaml.push_str(kind);
        yaml.push('\n');
        if let Some(raw) = raw {
            yaml.push_str(&format!("    raw: {raw}\n"));
        }
        match warning.match_time() {
            Some(match_time) => yaml.push_str(&format!(
                "    match_time_unix_microseconds: {}\n",
                match_time.as_unix_microseconds()
            )),
            None => yaml.push_str("    match_time_unix_microseconds: null\n"),
        }
    }
    yaml
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

pub fn canonical_normalized_events(events: Vec<DomainEvent>) -> Result<Vec<u8>, M1RunError> {
    let ordered = order_events(events).map_err(M1RunError::Ordering)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(NORMALIZED_EVENT_SET_MAGIC);
    bytes.extend_from_slice(&CANONICAL_NORMALIZED_EVENT_SET_VERSION.to_be_bytes());
    bytes.extend_from_slice(&EVENT_SCHEMA_VERSION.to_be_bytes());
    bytes.extend_from_slice(&CANONICAL_EVENT_VERSION.to_be_bytes());
    let event_count = u64::try_from(ordered.len()).map_err(|_| M1RunError::EventCountOverflow)?;
    bytes.extend_from_slice(&event_count.to_be_bytes());
    for event in ordered {
        let event = event
            .to_canonical_bytes()
            .map_err(M1RunError::CanonicalEncoding)?;
        append_length(event.len(), &mut bytes).map_err(M1RunError::CanonicalEncoding)?;
        bytes.extend_from_slice(&event);
    }
    Ok(bytes)
}

fn m1_instrument() -> Result<InstrumentId, M1RunError> {
    let symbol = Symbol::new("2330").map_err(M1RunError::Symbol)?;
    Ok(InstrumentId::new(MarketId::Twse, symbol))
}

fn m1_date() -> TradingDate {
    TradingDate::parse("2026-07-27").expect("M1 date is a valid constant")
}

fn normalizer_config() -> Result<NormalizerConfig, M1RunError> {
    NormalizerConfig::new(
        m1_instrument()?,
        m1_date(),
        MatchTime::parse("2026-07-27T08:55:00+08:00").expect("M1 replay start is a valid constant"),
        MatchTime::parse("2026-07-27T13:35:00+08:00").expect("M1 replay end is a valid constant"),
    )
    .map_err(M1RunError::NormalizerConfig)
}

fn regular_segment() -> Result<SessionSegment, M1RunError> {
    SessionSegment::new(
        SessionSegmentId::new("regular").expect("M1 segment id is non-empty"),
        SessionKind::Regular,
        m1_date(),
        MatchTime::parse("2026-07-27T09:00:00+08:00").expect("M1 session open is a valid constant"),
        MatchTime::parse("2026-07-27T13:30:00+08:00")
            .expect("M1 session close is a valid constant"),
    )
    .map_err(M1RunError::Context)
}

fn replay_core(instrument: InstrumentId) -> Result<ReplayCore, M1RunError> {
    ReplayCore::new(
        vec![MarketState::new(instrument, m1_date())],
        MarketStateReducer::twse_regular(),
        ReducerContext::new(
            m1_date(),
            SessionSegmentId::new("regular").expect("M1 segment id is non-empty"),
            SegmentBoundaryPolicy::Carry,
            1,
        ),
    )
    .map_err(M1RunError::Replay)
}

#[derive(Debug)]
pub enum M1RunError {
    Io(std::io::Error),
    EmptyFixture,
    EventSetMismatch,
    EventCountOverflow,
    Symbol(market_types::SymbolError),
    NormalizerConfig(twse_normalizer::ConfigError),
    Normalization(NormalizationError),
    Ordering(replay_engine::OrderingError),
    CanonicalEncoding(CanonicalEncodingError),
    Context(strategy_api::ContextError),
    Declaration(strategy_api::DeclarationError),
    Replay(replay_engine::ReplayError),
    Strategy(StrategyRunError),
    StrategyOutput(strategy_api::StrategyOutputEncodingError),
}

impl fmt::Display for M1RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "fixture I/O failed: {error}"),
            Self::EmptyFixture => formatter.write_str("fixture directory has no JSONL shards"),
            Self::EventSetMismatch => {
                formatter.write_str("perturbed event set has a different event count")
            }
            Self::EventCountOverflow => formatter.write_str("normalized event count exceeds u64"),
            Self::Symbol(error) => error.fmt(formatter),
            Self::NormalizerConfig(error) => error.fmt(formatter),
            Self::Normalization(error) => error.fmt(formatter),
            Self::Ordering(error) => error.fmt(formatter),
            Self::CanonicalEncoding(error) => error.fmt(formatter),
            Self::Context(error) => error.fmt(formatter),
            Self::Declaration(error) => error.fmt(formatter),
            Self::Replay(error) => error.fmt(formatter),
            Self::Strategy(error) => error.fmt(formatter),
            Self::StrategyOutput(error) => error.fmt(formatter),
        }
    }
}

impl Error for M1RunError {}
