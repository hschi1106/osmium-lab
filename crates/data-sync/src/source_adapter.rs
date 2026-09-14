use std::{env, error::Error, fmt, path::Path};

use market_types::{ContractShape, InstrumentClass, MarketId, UtcOffsetMinutes};
use run_planner::{SessionPlan, SourceId, SourcePartitionKey};
use taifex_normalizer::{
    InstrumentProfile as TaifexInstrumentProfile, MAPPING_NAME as TAIFEX_MAPPING_NAME,
    MAPPING_VERSION as TAIFEX_MAPPING_VERSION, NormalizerConfig as TaifexNormalizerConfig,
    OPTION_MAPPING_NAME as TAIFEX_OPTION_MAPPING_NAME,
    OPTION_MAPPING_VERSION as TAIFEX_OPTION_MAPPING_VERSION,
    SPREAD_MAPPING_NAME as TAIFEX_SPREAD_MAPPING_NAME,
    SPREAD_MAPPING_VERSION as TAIFEX_SPREAD_MAPPING_VERSION,
};
use tpex_normalizer::{
    MAPPING_NAME as TPEX_MAPPING_NAME, MAPPING_VERSION as TPEX_MAPPING_VERSION,
    NormalizerConfig as TpexNormalizerConfig, WARRANT_MAPPING_NAME as TPEX_WARRANT_MAPPING_NAME,
    WARRANT_MAPPING_VERSION as TPEX_WARRANT_MAPPING_VERSION,
};
use twse_normalizer::{
    MAPPING_NAME as TWSE_MAPPING_NAME, MAPPING_VERSION as TWSE_MAPPING_VERSION,
    NormalizerConfig as TwseNormalizerConfig, WARRANT_MAPPING_NAME as TWSE_WARRANT_MAPPING_NAME,
    WARRANT_MAPPING_VERSION as TWSE_WARRANT_MAPPING_VERSION,
};

use crate::{
    ArchiveKind, ArchiveMarket, ArchiveTimestamp, FeedArchiveTransport, PartitionNormalizerConfig,
    PartitionedSourceRepository, QueryError, StagingError, StagingRevision, SyncError,
    TeralionCredential, TeralionQuery, TeralionSync, TransportError,
};

#[derive(Debug, Clone)]
struct TeralionPartitionPlan {
    ticks: TeralionQuery,
    daily_instrument: TeralionQuery,
}

/// Built-in online source runtime selected only by stable [`SourceId`].
pub struct SourceAdapterRuntime {
    adapter: SourceAdapterRuntimeKind,
}

enum SourceAdapterRuntimeKind {
    Teralion {
        sync: TeralionSync<FeedArchiveTransport>,
        credential: TeralionCredential,
    },
}

impl SourceAdapterRuntime {
    pub fn new(source: SourceId) -> Result<Self, SourceAdapterError> {
        let adapter = match source {
            SourceId::TeralionFeedArchive => SourceAdapterRuntimeKind::Teralion {
                sync: TeralionSync::new(FeedArchiveTransport::new()?),
                credential: TeralionCredential::new(
                    env::var("TERALION_API_KEY")
                        .map_err(|_| SourceAdapterError::MissingCredential)?,
                )?,
            },
        };
        Ok(Self { adapter })
    }

    pub fn sync_partition(
        &mut self,
        data_root: &Path,
        key: &SourcePartitionKey,
        class: InstrumentClass,
        shape: Option<ContractShape>,
        session_plan: &SessionPlan,
    ) -> Result<SourceSyncResult, SourceAdapterError> {
        match &mut self.adapter {
            SourceAdapterRuntimeKind::Teralion { sync, credential } => sync_teralion_partition(
                sync,
                credential,
                data_root,
                key,
                class,
                shape,
                session_plan,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSyncResult {
    page_count: u32,
    revision_identity: Box<str>,
}

impl SourceSyncResult {
    #[must_use]
    pub const fn page_count(&self) -> u32 {
        self.page_count
    }

    #[must_use]
    pub fn revision_identity(&self) -> &str {
        &self.revision_identity
    }
}

/// Stable identity of the wire-to-domain mapping used to derive a replay cache.
///
/// Cache codecs only need canonical schema versions to read an artifact. Source
/// adapters own this identity; planners use it to decide whether an existing
/// derived artifact may be reused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizerMappingIdentity {
    name: Box<str>,
    version: u16,
}

impl NormalizerMappingIdentity {
    pub fn new(name: impl Into<Box<str>>, version: u16) -> Result<Self, MappingIdentityError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(MappingIdentityError::EmptyName);
        }
        if version == 0 {
            return Err(MappingIdentityError::ZeroVersion);
        }
        Ok(Self { name, version })
    }

    pub(crate) fn from_static(name: &'static str, version: u16) -> Self {
        Self::new(name, version).expect("built-in mapping identities are valid")
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
}

/// Resolve the mapping selected by a source adapter for one instrument contract.
///
/// Provider dispatch deliberately stays outside planner, cache-codec, replay and
/// domain crates. Adding another provider extends this boundary without teaching
/// those layers about provider wire formats.
pub fn normalizer_mapping_for(
    source: SourceId,
    market: MarketId,
    class: InstrumentClass,
    shape: Option<ContractShape>,
) -> Result<NormalizerMappingIdentity, MappingSelectionError> {
    mapping_profile_for(source, market, class, shape).map(MappingProfile::identity)
}

/// Build the normalizer selected by the source adapter for a planned partition.
///
/// This shares the same profile resolver as [`normalizer_mapping_for`], so the
/// catalog cannot approve one mapping while cache preparation silently chooses
/// another.
pub fn normalizer_config_for(
    source: SourceId,
    key: &SourcePartitionKey,
    class: InstrumentClass,
    shape: Option<ContractShape>,
    session_plan: &SessionPlan,
) -> Result<PartitionNormalizerConfig, NormalizerSelectionError> {
    if source != key.source() {
        return Err(NormalizerSelectionError::PartitionSourceMismatch);
    }
    if session_plan.instrument() != key.instrument()
        || session_plan.trading_date() != key.trading_date()
        || session_plan.identity() != key.session_plan_identity()
    {
        return Err(NormalizerSelectionError::SessionPlanMismatch);
    }
    let replay_start = session_plan
        .windows()
        .iter()
        .map(|window| window.replay_start())
        .min()
        .ok_or(NormalizerSelectionError::EmptySessionPlan)?;
    let replay_end_exclusive = session_plan
        .windows()
        .iter()
        .map(|window| window.replay_end_exclusive())
        .max()
        .ok_or(NormalizerSelectionError::EmptySessionPlan)?;
    let profile = mapping_profile_for(source, key.instrument().market(), class, shape)
        .map_err(|_| NormalizerSelectionError::UnsupportedContract)?;
    let instrument = key.instrument().clone();
    let trading_date = key.trading_date();
    let config = match profile {
        MappingProfile::TwseEquity => PartitionNormalizerConfig::Twse(
            TwseNormalizerConfig::new(instrument, trading_date, replay_start, replay_end_exclusive)
                .map_err(invalid_normalizer_config)?,
        ),
        MappingProfile::TwseWarrant => PartitionNormalizerConfig::Warrant(
            TwseNormalizerConfig::new_warrant(
                instrument,
                trading_date,
                replay_start,
                replay_end_exclusive,
            )
            .map_err(invalid_normalizer_config)?,
        ),
        MappingProfile::TpexEquity => PartitionNormalizerConfig::Tpex(
            TpexNormalizerConfig::new(instrument, trading_date, replay_start, replay_end_exclusive)
                .map_err(invalid_normalizer_config)?,
        ),
        MappingProfile::TpexWarrant => PartitionNormalizerConfig::TpexWarrant(
            TpexNormalizerConfig::new_warrant(
                instrument,
                trading_date,
                replay_start,
                replay_end_exclusive,
            )
            .map_err(invalid_normalizer_config)?,
        ),
        MappingProfile::TaifexFuture => PartitionNormalizerConfig::Taifex(
            TaifexNormalizerConfig::for_profile(
                instrument,
                trading_date,
                TaifexInstrumentProfile::Futures,
                session_plan
                    .windows()
                    .iter()
                    .map(|window| (window.replay_start(), window.replay_end_exclusive())),
            )
            .map_err(invalid_normalizer_config)?,
        ),
        MappingProfile::TaifexCalendarSpread => PartitionNormalizerConfig::TaifexCalendarSpread(
            TaifexNormalizerConfig::for_profile(
                instrument,
                trading_date,
                TaifexInstrumentProfile::FuturesCalendarSpread,
                session_plan
                    .windows()
                    .iter()
                    .map(|window| (window.replay_start(), window.replay_end_exclusive())),
            )
            .map_err(invalid_normalizer_config)?,
        ),
        MappingProfile::TaifexOption => {
            let windows = session_plan
                .windows()
                .iter()
                .map(|window| (window.replay_start(), window.replay_end_exclusive()));
            PartitionNormalizerConfig::TaifexOption(
                TaifexNormalizerConfig::for_profile(
                    instrument,
                    trading_date,
                    TaifexInstrumentProfile::IndexOptions,
                    windows,
                )
                .map_err(invalid_normalizer_config)?,
            )
        }
    };
    debug_assert_eq!(config.mapping_identity(), profile.identity());
    Ok(config)
}

fn teralion_partition_plan_for(
    source: SourceId,
    key: &SourcePartitionKey,
    class: InstrumentClass,
    shape: Option<ContractShape>,
    session_plan: &SessionPlan,
) -> Result<TeralionPartitionPlan, SourceAdapterError> {
    normalizer_config_for(source, key, class, shape, session_plan)?;
    let replay_start = session_plan
        .windows()
        .iter()
        .map(|window| window.replay_start())
        .min()
        .ok_or(NormalizerSelectionError::EmptySessionPlan)?;
    let replay_end_exclusive = session_plan
        .windows()
        .iter()
        .map(|window| window.replay_end_exclusive())
        .max()
        .ok_or(NormalizerSelectionError::EmptySessionPlan)?;
    let taipei_offset =
        UtcOffsetMinutes::new(480).expect("480 minutes is a valid ISO-8601 UTC offset");
    let start = ArchiveTimestamp::parse(
        replay_start
            .to_iso8601(taipei_offset)
            .map_err(|error| SourceAdapterError::Configuration(error.to_string()))?,
    )?;
    let end = ArchiveTimestamp::parse(
        replay_end_exclusive
            .to_iso8601(taipei_offset)
            .map_err(|error| SourceAdapterError::Configuration(error.to_string()))?,
    )?;
    let kinds = match key.instrument().market() {
        MarketId::Twse | MarketId::Tpex => vec![ArchiveKind::Quote],
        MarketId::Taifex => vec![
            ArchiveKind::Book,
            ArchiveKind::Close,
            ArchiveKind::Stats,
            ArchiveKind::Trade,
        ],
    };
    let archive_market = match class {
        InstrumentClass::Option => ArchiveMarket::TaifexOptions,
        _ => ArchiveMarket::for_instrument(key.instrument()),
    };
    let ticks = match class {
        InstrumentClass::Warrant | InstrumentClass::Option => TeralionQuery::ticks_for_market(
            key.instrument().clone(),
            start,
            end,
            kinds,
            5_000,
            archive_market,
        )?,
        InstrumentClass::Equity | InstrumentClass::Future => {
            TeralionQuery::ticks(key.instrument().clone(), start, end, kinds, 5_000)?
        }
    };
    Ok(TeralionPartitionPlan {
        ticks,
        daily_instrument: TeralionQuery::daily_instrument(
            key.instrument().clone(),
            key.trading_date(),
        ),
    })
}

fn sync_teralion_partition<T: crate::TeralionTransport>(
    sync: &mut TeralionSync<T>,
    credential: &TeralionCredential,
    data_root: &Path,
    key: &SourcePartitionKey,
    class: InstrumentClass,
    shape: Option<ContractShape>,
    session_plan: &SessionPlan,
) -> Result<SourceSyncResult, SourceAdapterError> {
    let plan = teralion_partition_plan_for(key.source(), key, class, shape, session_plan)?;
    validate_json(&sync.fetch_single(
        TeralionQuery::coverage(key.trading_date(), key.trading_date())?,
        credential,
    )?)?;
    validate_json(&sync.fetch_single(
        TeralionQuery::symbol_range(key.instrument().clone()),
        credential,
    )?)?;
    let daily = sync.fetch_single(plan.daily_instrument.clone(), credential)?;
    validate_json(&daily)?;
    let repository = PartitionedSourceRepository::new(data_root, key.clone())?;
    let attempt = attempt_id(key);
    let checkpoint = repository
        .root()
        .join("staging")
        .join(&attempt)
        .join("checkpoint.json");
    let mut staging = if checkpoint.exists() {
        StagingRevision::resume_for_partition(data_root, key, &attempt)?
    } else {
        StagingRevision::create_for_partition(data_root, key, &attempt)?
    };
    let report = sync.sync_pages(plan.ticks.clone(), credential, &mut staging)?;
    staging.stage_daily_instrument(plan.daily_instrument.identity(), &daily)?;
    let revision = staging.publish(plan.ticks.identity(), report.terminal)?;
    Ok(SourceSyncResult {
        page_count: report.page_count,
        revision_identity: revision
            .manifest()
            .revision_identity
            .clone()
            .into_boxed_str(),
    })
}

fn validate_json(bytes: &[u8]) -> Result<(), SourceAdapterError> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .map(|_| ())
        .map_err(|error| SourceAdapterError::InvalidResponse(error.to_string()))
}

fn attempt_id(key: &SourcePartitionKey) -> String {
    let partition_identity = key.identity();
    let identity = partition_identity.as_bytes();
    let mut encoded = String::with_capacity(24);
    for byte in &identity[..12] {
        use fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("run-{encoded}")
}

fn invalid_normalizer_config(error: impl fmt::Display) -> NormalizerSelectionError {
    NormalizerSelectionError::InvalidConfig(error.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MappingProfile {
    TwseEquity,
    TwseWarrant,
    TpexEquity,
    TpexWarrant,
    TaifexFuture,
    TaifexCalendarSpread,
    TaifexOption,
}

impl MappingProfile {
    fn identity(self) -> NormalizerMappingIdentity {
        let (name, version) = match self {
            Self::TwseEquity => (TWSE_MAPPING_NAME, TWSE_MAPPING_VERSION),
            Self::TwseWarrant => (TWSE_WARRANT_MAPPING_NAME, TWSE_WARRANT_MAPPING_VERSION),
            Self::TpexEquity => (TPEX_MAPPING_NAME, TPEX_MAPPING_VERSION),
            Self::TpexWarrant => (TPEX_WARRANT_MAPPING_NAME, TPEX_WARRANT_MAPPING_VERSION),
            Self::TaifexFuture => (TAIFEX_MAPPING_NAME, TAIFEX_MAPPING_VERSION),
            Self::TaifexCalendarSpread => {
                (TAIFEX_SPREAD_MAPPING_NAME, TAIFEX_SPREAD_MAPPING_VERSION)
            }
            Self::TaifexOption => (TAIFEX_OPTION_MAPPING_NAME, TAIFEX_OPTION_MAPPING_VERSION),
        };
        NormalizerMappingIdentity::from_static(name, version)
    }
}

fn mapping_profile_for(
    source: SourceId,
    market: MarketId,
    class: InstrumentClass,
    shape: Option<ContractShape>,
) -> Result<MappingProfile, MappingSelectionError> {
    let profile = match (source, market, class, shape) {
        (SourceId::TeralionFeedArchive, MarketId::Twse, InstrumentClass::Equity, None) => {
            MappingProfile::TwseEquity
        }
        (SourceId::TeralionFeedArchive, MarketId::Twse, InstrumentClass::Warrant, None) => {
            MappingProfile::TwseWarrant
        }
        (SourceId::TeralionFeedArchive, MarketId::Tpex, InstrumentClass::Equity, None) => {
            MappingProfile::TpexEquity
        }
        (SourceId::TeralionFeedArchive, MarketId::Tpex, InstrumentClass::Warrant, None) => {
            MappingProfile::TpexWarrant
        }
        (
            SourceId::TeralionFeedArchive,
            MarketId::Taifex,
            InstrumentClass::Future,
            Some(ContractShape::Outright),
        ) => MappingProfile::TaifexFuture,
        (
            SourceId::TeralionFeedArchive,
            MarketId::Taifex,
            InstrumentClass::Future,
            Some(ContractShape::CalendarSpread),
        ) => MappingProfile::TaifexCalendarSpread,
        (SourceId::TeralionFeedArchive, MarketId::Taifex, InstrumentClass::Option, None) => {
            MappingProfile::TaifexOption
        }
        _ => return Err(MappingSelectionError),
    };
    Ok(profile)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappingSelectionError;

impl fmt::Display for MappingSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("source adapter does not support this instrument contract")
    }
}

impl Error for MappingSelectionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizerSelectionError {
    EmptySessionPlan,
    PartitionSourceMismatch,
    SessionPlanMismatch,
    UnsupportedContract,
    InvalidConfig(String),
}

impl fmt::Display for NormalizerSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySessionPlan => formatter.write_str("session plan has no windows"),
            Self::PartitionSourceMismatch => {
                formatter.write_str("source adapter does not match the partition source")
            }
            Self::SessionPlanMismatch => {
                formatter.write_str("session plan does not match the partition identity")
            }
            Self::UnsupportedContract => {
                formatter.write_str("source adapter does not support this instrument contract")
            }
            Self::InvalidConfig(message) => formatter.write_str(message),
        }
    }
}

impl Error for NormalizerSelectionError {}

#[derive(Debug)]
pub enum SourceAdapterError {
    MissingCredential,
    Configuration(String),
    Transfer(String),
    Storage(String),
    InvalidResponse(String),
}

impl fmt::Display for SourceAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCredential => formatter.write_str("source credential is missing"),
            Self::Configuration(message) | Self::Transfer(message) | Self::Storage(message) => {
                formatter.write_str(message)
            }
            Self::InvalidResponse(message) => {
                write!(formatter, "source returned invalid JSON: {message}")
            }
        }
    }
}

impl Error for SourceAdapterError {}

impl From<NormalizerSelectionError> for SourceAdapterError {
    fn from(error: NormalizerSelectionError) -> Self {
        Self::Configuration(error.to_string())
    }
}

impl From<QueryError> for SourceAdapterError {
    fn from(error: QueryError) -> Self {
        Self::Configuration(error.to_string())
    }
}

impl From<TransportError> for SourceAdapterError {
    fn from(error: TransportError) -> Self {
        Self::Transfer(error.to_string())
    }
}

impl From<SyncError> for SourceAdapterError {
    fn from(error: SyncError) -> Self {
        Self::Transfer(error.to_string())
    }
}

impl From<StagingError> for SourceAdapterError {
    fn from(error: StagingError) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<crate::PartitionRepositoryError> for SourceAdapterError {
    fn from(error: crate::PartitionRepositoryError) -> Self {
        Self::Storage(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingIdentityError {
    EmptyName,
    ZeroVersion,
}

impl fmt::Display for MappingIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("normalizer mapping name cannot be empty"),
            Self::ZeroVersion => formatter.write_str("normalizer mapping version must be positive"),
        }
    }
}

impl Error for MappingIdentityError {}

#[cfg(test)]
mod tests {
    use market_types::{InstrumentId, Symbol, TradingDate};
    use strategy_api::SessionKind;

    use super::*;
    use crate::{TeralionRequest, TeralionTransport};

    #[derive(Default)]
    struct CompletePartitionTransport;

    impl TeralionTransport for CompletePartitionTransport {
        fn execute(
            &mut self,
            request: &TeralionRequest,
            _credential: &TeralionCredential,
        ) -> Result<Vec<u8>, TransportError> {
            let body = match request.query() {
                TeralionQuery::Ticks { .. } => {
                    br#"{"items":[{"type":"quote","market":"twse","format":"STOCK_SNAPSHOT","symbol":"2330","match_time":"2026-07-27T09:00:00+08:00","received_at":"2026-07-27T09:00:00.001000+08:00"}],"next_cursor":null}"#.as_slice()
                }
                TeralionQuery::Coverage { .. }
                | TeralionQuery::SymbolRange { .. }
                | TeralionQuery::DailyInstrument { .. } => br#"{}"#.as_slice(),
            };
            Ok(body.to_vec())
        }
    }

    #[test]
    fn provider_dispatch_returns_the_contract_specific_mapping() {
        let cases = [
            (
                MarketId::Twse,
                InstrumentClass::Equity,
                None,
                TWSE_MAPPING_NAME,
                TWSE_MAPPING_VERSION,
            ),
            (
                MarketId::Twse,
                InstrumentClass::Warrant,
                None,
                TWSE_WARRANT_MAPPING_NAME,
                TWSE_WARRANT_MAPPING_VERSION,
            ),
            (
                MarketId::Tpex,
                InstrumentClass::Equity,
                None,
                TPEX_MAPPING_NAME,
                TPEX_MAPPING_VERSION,
            ),
            (
                MarketId::Tpex,
                InstrumentClass::Warrant,
                None,
                TPEX_WARRANT_MAPPING_NAME,
                TPEX_WARRANT_MAPPING_VERSION,
            ),
            (
                MarketId::Taifex,
                InstrumentClass::Future,
                Some(ContractShape::Outright),
                TAIFEX_MAPPING_NAME,
                TAIFEX_MAPPING_VERSION,
            ),
            (
                MarketId::Taifex,
                InstrumentClass::Future,
                Some(ContractShape::CalendarSpread),
                TAIFEX_SPREAD_MAPPING_NAME,
                TAIFEX_SPREAD_MAPPING_VERSION,
            ),
            (
                MarketId::Taifex,
                InstrumentClass::Option,
                None,
                TAIFEX_OPTION_MAPPING_NAME,
                TAIFEX_OPTION_MAPPING_VERSION,
            ),
        ];

        for (market, class, shape, expected_name, expected_version) in cases {
            let identity =
                normalizer_mapping_for(SourceId::TeralionFeedArchive, market, class, shape)
                    .unwrap();
            assert_eq!(identity.name(), expected_name);
            assert_eq!(identity.version(), expected_version);
        }
    }

    #[test]
    fn unsupported_contract_is_rejected_at_the_adapter_boundary() {
        assert_eq!(
            normalizer_mapping_for(
                SourceId::TeralionFeedArchive,
                MarketId::Twse,
                InstrumentClass::Option,
                None,
            ),
            Err(MappingSelectionError)
        );
    }

    #[test]
    fn planner_identity_and_built_normalizer_share_one_profile_resolver() {
        let source = SourceId::TeralionFeedArchive;
        let instrument = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let trading_date: TradingDate = "2026-07-27".parse().unwrap();
        let session_plan = SessionPlan::for_instrument_class(
            &instrument,
            InstrumentClass::Equity,
            trading_date,
            [SessionKind::Regular],
        )
        .unwrap();
        let key = SourcePartitionKey::new(
            source,
            instrument,
            trading_date,
            [SessionKind::Regular],
            session_plan.identity(),
        )
        .unwrap();
        let expected =
            normalizer_mapping_for(source, MarketId::Twse, InstrumentClass::Equity, None).unwrap();
        let config =
            normalizer_config_for(source, &key, InstrumentClass::Equity, None, &session_plan)
                .unwrap();

        assert_eq!(config.mapping_identity(), expected);
    }

    #[test]
    fn normalizer_builder_rejects_a_session_plan_from_another_partition() {
        let source = SourceId::TeralionFeedArchive;
        let instrument = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let other_instrument = InstrumentId::new(MarketId::Twse, Symbol::new("2317").unwrap());
        let trading_date: TradingDate = "2026-07-27".parse().unwrap();
        let session_plan = SessionPlan::for_instrument_class(
            &instrument,
            InstrumentClass::Equity,
            trading_date,
            [SessionKind::Regular],
        )
        .unwrap();
        let key = SourcePartitionKey::new(
            source,
            other_instrument,
            trading_date,
            [SessionKind::Regular],
            session_plan.identity(),
        )
        .unwrap();

        assert!(matches!(
            normalizer_config_for(source, &key, InstrumentClass::Equity, None, &session_plan,),
            Err(NormalizerSelectionError::SessionPlanMismatch)
        ));
    }

    #[test]
    fn source_partition_plan_owns_the_provider_query_profile() {
        let source = SourceId::TeralionFeedArchive;
        let instrument =
            InstrumentId::new(MarketId::Taifex, Symbol::new("TXO202609C20000").unwrap());
        let trading_date: TradingDate = "2026-07-27".parse().unwrap();
        let session_plan = SessionPlan::for_instrument_class(
            &instrument,
            InstrumentClass::Option,
            trading_date,
            [SessionKind::Regular],
        )
        .unwrap();
        let key = SourcePartitionKey::new(
            source,
            instrument,
            trading_date,
            [SessionKind::Regular],
            session_plan.identity(),
        )
        .unwrap();

        let plan =
            teralion_partition_plan_for(source, &key, InstrumentClass::Option, None, &session_plan)
                .unwrap();

        assert_eq!(
            plan.ticks.archive_market(),
            Some(ArchiveMarket::TaifexOptions)
        );
        assert_eq!(plan.daily_instrument.instrument(), Some(key.instrument()));
    }

    #[test]
    fn source_runtime_owns_query_validation_staging_and_publish() {
        let root = tempfile::tempdir().unwrap();
        let source = SourceId::TeralionFeedArchive;
        let instrument = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let trading_date: TradingDate = "2026-07-27".parse().unwrap();
        let session_plan = SessionPlan::for_instrument_class(
            &instrument,
            InstrumentClass::Equity,
            trading_date,
            [SessionKind::Regular],
        )
        .unwrap();
        let key = SourcePartitionKey::new(
            source,
            instrument,
            trading_date,
            [SessionKind::Regular],
            session_plan.identity(),
        )
        .unwrap();
        let mut sync = TeralionSync::new(CompletePartitionTransport);

        let result = sync_teralion_partition(
            &mut sync,
            &TeralionCredential::new("test-only").unwrap(),
            root.path(),
            &key,
            InstrumentClass::Equity,
            None,
            &session_plan,
        )
        .unwrap();

        assert_eq!(result.page_count(), 1);
        assert_eq!(result.revision_identity().len(), 64);
        let repository = PartitionedSourceRepository::new(root.path(), key).unwrap();
        assert!(matches!(
            repository.inspect().state(),
            run_planner::SourceState::Complete { .. }
        ));
        assert!(repository.partition_manifest_path().is_file());
    }

    #[test]
    fn taifex_future_normalizer_preserves_disjoint_session_windows() {
        let source = SourceId::TeralionFeedArchive;
        let instrument = InstrumentId::new(MarketId::Taifex, Symbol::new("TXFH6").unwrap());
        let trading_date: TradingDate = "2026-07-27".parse().unwrap();
        let session_plan = SessionPlan::with_profile(
            &instrument,
            trading_date,
            run_planner::SessionProfileId::TaifexIndexFutures,
            [SessionKind::Regular, SessionKind::AfterHours],
        )
        .unwrap();
        let key = SourcePartitionKey::new(
            source,
            instrument,
            trading_date,
            [SessionKind::Regular, SessionKind::AfterHours],
            session_plan.identity(),
        )
        .unwrap();

        let config = normalizer_config_for(
            source,
            &key,
            InstrumentClass::Future,
            Some(ContractShape::Outright),
            &session_plan,
        )
        .unwrap();
        let PartitionNormalizerConfig::Taifex(config) = config else {
            panic!("future contract must select the TAIFEX futures normalizer")
        };

        let mut expected = session_plan
            .windows()
            .iter()
            .map(|window| (window.replay_start(), window.replay_end_exclusive()))
            .collect::<Vec<_>>();
        expected.sort_by_key(|(start, _)| *start);
        assert_eq!(config.replay_windows(), expected);
    }
}
