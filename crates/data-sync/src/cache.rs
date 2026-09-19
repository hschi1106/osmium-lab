use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use crate::{
    LocalSourceRepository, NormalizerMappingIdentity, PartitionRepositoryError, VerificationReport,
    cache_instrument_root, cache_partition_root,
};
use market_types::{
    CANONICAL_EVENT_VERSION, DomainEvent, EVENT_SCHEMA_VERSION, MARKET_TYPES_VERSION, MatchTime,
};
use replay_engine::{
    EventStream, ORDERING_RULE_VERSION, OrderingKey, ReplayStreamBinding, ReplayStreamFactory,
    order_events,
};
use run_planner::SourcePartitionKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CACHE_MAGIC: &[u8; 9] = b"OSMCACHE1";
pub const CACHE_FORMAT_VERSION: u16 = 3;
/// Bounds allocations made while reading any one derived event record.
const MAX_CACHE_EVENT_RECORD_BYTES: u32 = 16 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheDescriptor {
    pub cache_format_version: u16,
    pub cache_identity: String,
    pub source_revision_identity: String,
    pub instrument_market: u8,
    pub instrument_symbol: String,
    pub trading_date_epoch_days: i32,
    pub event_count: u64,
    pub first_match_time_micros: Option<i64>,
    pub last_match_time_micros: Option<i64>,
    pub payload_sha256: String,
    pub market_types_version: u16,
    pub event_schema_version: u16,
    pub canonical_event_version: u16,
    pub ordering_rule_version: u16,
    #[serde(default)]
    pub partition_identity: Option<String>,
    pub normalizer_mapping_name: String,
    pub normalizer_mapping_version: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedCache {
    path: PathBuf,
    descriptor: CacheDescriptor,
}

impl PublishedCache {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn descriptor(&self) -> &CacheDescriptor {
        &self.descriptor
    }
}

#[derive(Debug, Clone)]
pub struct CacheBuilder {
    data_root: PathBuf,
}

impl CacheBuilder {
    #[must_use]
    pub fn new(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    /// Publishes already-normalized events from the generic non-partitioned source layout.
    pub fn build_external_current(
        &self,
        instrument: &market_types::InstrumentId,
        trading_date: market_types::TradingDate,
        mapping: &NormalizerMappingIdentity,
        events: Vec<DomainEvent>,
    ) -> Result<PublishedCache, CacheBuildError> {
        let source = LocalSourceRepository::new(&self.data_root).verify_current()?;
        let events =
            order_events(events).map_err(|error| CacheBuildError::Ordering(error.to_string()))?;
        if events
            .iter()
            .any(|event| event.instrument() != instrument || event.trading_date() != trading_date)
        {
            return Err(CacheBuildError::SourceManifest);
        }
        self.publish_events(
            &source,
            &self.data_root.join("derived/staging"),
            &self.data_root.join("derived/cache"),
            instrument,
            trading_date,
            None,
            mapping,
            events,
        )
    }

    /// Publishes already-normalized domain events while retaining a verified source revision.
    ///
    /// This path is intended for explicit offline adapters. Callers remain responsible for
    /// preserving source lineage and must not use it to bypass source verification.
    pub fn build_external_partition(
        &self,
        key: &SourcePartitionKey,
        mapping: &NormalizerMappingIdentity,
        events: Vec<DomainEvent>,
    ) -> Result<PublishedCache, CacheBuildError> {
        let repository = crate::PartitionedSourceRepository::new(&self.data_root, key.clone())
            .map_err(|error| CacheBuildError::Partition(error.to_string()))?;
        let source = repository.verify_current()?;
        let cache_root = crate::cache_partition_root(&self.data_root, key)
            .map_err(|error| CacheBuildError::Partition(error.to_string()))?;
        let events =
            order_events(events).map_err(|error| CacheBuildError::Ordering(error.to_string()))?;
        if events.iter().any(|event| {
            event.instrument() != key.instrument() || event.trading_date() != key.trading_date()
        }) {
            return Err(CacheBuildError::SourceManifest);
        }
        self.publish_events(
            &source,
            &cache_root,
            &cache_root,
            key.instrument(),
            key.trading_date(),
            Some(hex(key.identity().as_bytes())),
            mapping,
            events,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_events(
        &self,
        source: &VerificationReport,
        staging_root: &Path,
        cache_root: &Path,
        instrument: &market_types::InstrumentId,
        trading_date: market_types::TradingDate,
        partition_identity: Option<String>,
        mapping: &NormalizerMappingIdentity,
        events: Vec<DomainEvent>,
    ) -> Result<PublishedCache, CacheBuildError> {
        let attempt = staging_root.join("cache-build");
        if attempt.exists() {
            return Err(CacheBuildError::BuildAlreadyExists);
        }
        fs::create_dir_all(&attempt)?;
        let events_tmp = attempt.join("events.bin.tmp");
        let mut writer = BufWriter::new(
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&events_tmp)?,
        );
        writer.write_all(CACHE_MAGIC)?;
        writer.write_all(&CACHE_FORMAT_VERSION.to_be_bytes())?;
        writer.write_all(&(events.len() as u64).to_be_bytes())?;
        let mut payload_hasher = Sha256::new();
        let mut first_match_time = None;
        let mut last_match_time = None;
        for event in &events {
            let bytes = event
                .to_canonical_bytes()
                .map_err(|error| CacheBuildError::Canonical(error.to_string()))?;
            let length = u32::try_from(bytes.len()).map_err(|_| CacheBuildError::EventTooLarge)?;
            if length > MAX_CACHE_EVENT_RECORD_BYTES {
                return Err(CacheBuildError::EventTooLarge);
            }
            writer.write_all(&length.to_be_bytes())?;
            writer.write_all(&bytes)?;
            payload_hasher.update(length.to_be_bytes());
            payload_hasher.update(&bytes);
            first_match_time.get_or_insert(event.match_time());
            last_match_time = Some(event.match_time());
        }
        writer.flush()?;
        writer.get_ref().sync_all()?;
        fs::rename(&events_tmp, attempt.join("events.bin"))?;

        let payload_sha256 = hex(&payload_hasher.finalize());
        let cache_identity = cache_identity(
            source,
            instrument,
            trading_date,
            mapping,
            partition_identity.as_deref(),
            &payload_sha256,
            events.len() as u64,
        );
        let descriptor = CacheDescriptor {
            cache_format_version: CACHE_FORMAT_VERSION,
            cache_identity: cache_identity.clone(),
            source_revision_identity: source.manifest().revision_identity.clone(),
            instrument_market: instrument.market().discriminant(),
            instrument_symbol: instrument.symbol().as_str().to_owned(),
            trading_date_epoch_days: trading_date.as_epoch_days(),
            event_count: events.len() as u64,
            first_match_time_micros: first_match_time.map(MatchTime::as_unix_microseconds),
            last_match_time_micros: last_match_time.map(MatchTime::as_unix_microseconds),
            payload_sha256,
            market_types_version: MARKET_TYPES_VERSION,
            event_schema_version: EVENT_SCHEMA_VERSION,
            canonical_event_version: CANONICAL_EVENT_VERSION,
            ordering_rule_version: ORDERING_RULE_VERSION,
            partition_identity,
            normalizer_mapping_name: mapping.name().to_owned(),
            normalizer_mapping_version: mapping.version(),
        };
        let descriptor_bytes = serde_json::to_vec_pretty(&descriptor)
            .map_err(|error| CacheBuildError::Descriptor(error.to_string()))?;
        write_atomic(&attempt.join("descriptor.yaml"), &descriptor_bytes)?;
        File::open(&attempt)?.sync_all()?;

        fs::create_dir_all(cache_root)?;
        let published_path = cache_root.join(&cache_identity);
        if published_path.exists() {
            return Err(CacheBuildError::CacheAlreadyExists(cache_identity));
        }
        fs::rename(&attempt, &published_path)?;
        File::open(cache_root)?.sync_all()?;
        Ok(PublishedCache {
            path: published_path,
            descriptor,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PartitionCacheCatalog {
    data_root: PathBuf,
}

impl PartitionCacheCatalog {
    #[must_use]
    pub fn new(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    pub fn root_for(&self, key: &SourcePartitionKey) -> Result<PathBuf, CacheCatalogError> {
        cache_partition_root(&self.data_root, key).map_err(CacheCatalogError::Layout)
    }

    pub fn inspect(
        &self,
        key: &SourcePartitionKey,
        source_revision_identity: &str,
        expected_mapping: &NormalizerMappingIdentity,
    ) -> Result<PartitionCacheInspection, CacheCatalogError> {
        let root = self.root_for(key)?;
        if !root.is_dir() {
            return Ok(PartitionCacheInspection::Missing);
        }
        let mut paths = fs::read_dir(&root)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        paths.sort();
        let partition_identity = hex(key.identity().as_bytes());
        let mut saw_stale = false;
        let mut current = None;
        for path in paths {
            let descriptor_path = path.join("descriptor.yaml");
            if !descriptor_path.is_file() {
                continue;
            }
            let descriptor: CacheDescriptor =
                serde_json::from_slice(&fs::read(descriptor_path)?)
                    .map_err(|error| CacheCatalogError::Descriptor(error.to_string()))?;
            if descriptor.source_revision_identity == source_revision_identity
                && descriptor.partition_identity.as_deref() == Some(partition_identity.as_str())
            {
                if validate_descriptor(&descriptor).is_err()
                    || descriptor.normalizer_mapping_name != expected_mapping.name()
                    || descriptor.normalizer_mapping_version != expected_mapping.version()
                {
                    saw_stale = true;
                    continue;
                }
                if descriptor.instrument_market != key.instrument().market().discriminant()
                    || descriptor.instrument_symbol != key.instrument().symbol().as_str()
                    || descriptor.trading_date_epoch_days != key.trading_date().as_epoch_days()
                    || path.file_name().and_then(|name| name.to_str())
                        != Some(descriptor.cache_identity.as_str())
                {
                    return Err(CacheCatalogError::Descriptor(
                        "cache descriptor does not match its partition binding".to_owned(),
                    ));
                }
                let entry = Box::new(PartitionCacheEntry { path, descriptor });
                if current.replace(entry).is_some() {
                    return Err(CacheCatalogError::Descriptor(
                        "multiple current cache artifacts exist for one source revision".to_owned(),
                    ));
                }
            }
        }
        if let Some(entry) = current {
            Ok(PartitionCacheInspection::Current(entry))
        } else if saw_stale {
            Ok(PartitionCacheInspection::Stale)
        } else {
            Ok(PartitionCacheInspection::Missing)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionCacheInspection {
    Missing,
    Current(Box<PartitionCacheEntry>),
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionCacheEntry {
    path: PathBuf,
    descriptor: CacheDescriptor,
}

impl PartitionCacheEntry {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn descriptor(&self) -> &CacheDescriptor {
        &self.descriptor
    }
}

fn cache_identity(
    source: &VerificationReport,
    instrument: &market_types::InstrumentId,
    trading_date: market_types::TradingDate,
    mapping: &NormalizerMappingIdentity,
    partition_identity: Option<&str>,
    payload_sha256: &str,
    event_count: u64,
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"OSCI");
    bytes.extend_from_slice(&CACHE_FORMAT_VERSION.to_be_bytes());
    bytes.extend_from_slice(source.revision().as_bytes());
    bytes.push(instrument.market().discriminant());
    bytes.extend_from_slice(instrument.symbol().as_bytes());
    bytes.extend_from_slice(&trading_date.to_canonical_bytes());
    bytes.extend_from_slice(&(mapping.name().len() as u32).to_be_bytes());
    bytes.extend_from_slice(mapping.name().as_bytes());
    bytes.extend_from_slice(&mapping.version().to_be_bytes());
    bytes.extend_from_slice(&MARKET_TYPES_VERSION.to_be_bytes());
    bytes.extend_from_slice(&EVENT_SCHEMA_VERSION.to_be_bytes());
    bytes.extend_from_slice(&CANONICAL_EVENT_VERSION.to_be_bytes());
    bytes.extend_from_slice(&ORDERING_RULE_VERSION.to_be_bytes());
    if let Some(partition_identity) = partition_identity {
        bytes.extend_from_slice(partition_identity.as_bytes());
    }
    bytes.extend_from_slice(&event_count.to_be_bytes());
    bytes.extend_from_slice(payload_sha256.as_bytes());
    hex(blake3::hash(&bytes).as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), io::Error> {
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    Ok(())
}

#[derive(Debug)]
pub struct CacheReader {
    reader: BufReader<File>,
    descriptor: CacheDescriptor,
    remaining: u64,
    ordinal: u64,
    hasher: Sha256,
    previous_key: Option<OrderingKey>,
    first_match_time: Option<i64>,
    last_match_time: Option<i64>,
    finished: bool,
}

impl CacheReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CacheReadError> {
        Self::open_inner(path.as_ref(), None)
    }

    pub fn open_bound(
        path: impl AsRef<Path>,
        expected_source_revision: &str,
    ) -> Result<Self, CacheReadError> {
        Self::open_inner(path.as_ref(), Some(expected_source_revision))
    }

    fn open_inner(
        path: &Path,
        expected_source_revision: Option<&str>,
    ) -> Result<Self, CacheReadError> {
        let descriptor: CacheDescriptor =
            serde_json::from_slice(&fs::read(path.join("descriptor.yaml"))?)
                .map_err(|error| CacheReadError::Descriptor(error.to_string()))?;
        validate_descriptor(&descriptor)?;
        if expected_source_revision
            .is_some_and(|expected| descriptor.source_revision_identity != expected)
        {
            return Err(CacheReadError::StaleSourceLineage);
        }
        let mut reader = BufReader::new(File::open(path.join("events.bin"))?);
        let mut magic = [0_u8; 9];
        reader.read_exact(&mut magic)?;
        if &magic != CACHE_MAGIC {
            return Err(CacheReadError::Header);
        }
        let version = read_u16(&mut reader)?;
        let count = read_u64(&mut reader)?;
        if version != CACHE_FORMAT_VERSION || count != descriptor.event_count {
            return Err(CacheReadError::Header);
        }
        Ok(Self {
            reader,
            remaining: count,
            descriptor,
            ordinal: 0,
            hasher: Sha256::new(),
            previous_key: None,
            first_match_time: None,
            last_match_time: None,
            finished: false,
        })
    }

    #[must_use]
    pub const fn descriptor(&self) -> &CacheDescriptor {
        &self.descriptor
    }

    pub fn next_record(&mut self) -> Result<Option<CacheRecord>, CacheReadError> {
        if self.finished {
            return Ok(None);
        }
        if self.remaining == 0 {
            let mut trailing = [0_u8; 1];
            if self.reader.read(&mut trailing)? != 0 {
                return Err(CacheReadError::TrailingBytes);
            }
            let checksum = hex(&self.hasher.clone().finalize());
            if checksum != self.descriptor.payload_sha256 {
                return Err(CacheReadError::PayloadChecksum);
            }
            if self.first_match_time != self.descriptor.first_match_time_micros
                || self.last_match_time != self.descriptor.last_match_time_micros
            {
                return Err(CacheReadError::BoundsMismatch);
            }
            self.finished = true;
            return Ok(None);
        }
        let record_length = read_u32(&mut self.reader)?;
        let buffer_length = validate_event_record_length(record_length)?;
        let mut bytes = vec![0_u8; buffer_length];
        self.reader.read_exact(&mut bytes)?;
        self.hasher.update(record_length.to_be_bytes());
        self.hasher.update(&bytes);
        let event = DomainEvent::from_canonical_bytes(&bytes)
            .map_err(|error| CacheReadError::Canonical(error.to_string()))?;
        validate_event(&self.descriptor, &event)?;
        let key = OrderingKey::for_event(&event)
            .map_err(|error| CacheReadError::Ordering(error.to_string()))?;
        if self
            .previous_key
            .as_ref()
            .is_some_and(|previous| previous > &key)
        {
            return Err(CacheReadError::OrderingRegression);
        }
        self.previous_key = Some(key);
        self.first_match_time
            .get_or_insert(event.match_time().as_unix_microseconds());
        self.last_match_time = Some(event.match_time().as_unix_microseconds());
        let record = CacheRecord {
            ordinal: self.ordinal,
            event,
        };
        self.ordinal += 1;
        self.remaining -= 1;
        Ok(Some(record))
    }
}

fn validate_event_record_length(length: u32) -> Result<usize, CacheReadError> {
    if length > MAX_CACHE_EVENT_RECORD_BYTES {
        return Err(CacheReadError::EventTooLarge);
    }
    Ok(length as usize)
}

impl EventStream for CacheReader {
    type Error = CacheReadError;

    fn next_event(&mut self) -> Result<Option<DomainEvent>, Self::Error> {
        self.next_record()
            .map(|record| record.map(CacheRecord::into_event))
    }
}

#[derive(Debug, Clone)]
pub struct LocalCacheFactory {
    data_root: PathBuf,
    partitioned_source: Option<run_planner::SourceId>,
    opened: Vec<ReplayStreamBinding>,
}

impl LocalCacheFactory {
    #[must_use]
    pub fn new(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
            partitioned_source: None,
            opened: Vec::new(),
        }
    }

    #[must_use]
    pub fn new_partitioned(data_root: impl Into<PathBuf>, source: run_planner::SourceId) -> Self {
        Self {
            data_root: data_root.into(),
            partitioned_source: Some(source),
            opened: Vec::new(),
        }
    }

    #[must_use]
    pub fn opened_bindings(&self) -> &[ReplayStreamBinding] {
        &self.opened
    }
}

impl ReplayStreamFactory for LocalCacheFactory {
    type Stream = CacheReader;
    type Error = CacheReadError;

    fn open(&mut self, binding: &ReplayStreamBinding) -> Result<Self::Stream, Self::Error> {
        let cache_identity = hex(binding.cache_identity());
        let source_revision = hex(binding.source_revision_identity());
        let cache_root = if let Some(source) = self.partitioned_source {
            cache_instrument_root(
                &self.data_root,
                source,
                binding.instrument(),
                binding.trading_date(),
            )
            .map_err(|_| CacheReadError::BindingMismatch)?
        } else {
            self.data_root.join("derived/cache")
        };
        let reader = CacheReader::open_bound(cache_root.join(&cache_identity), &source_revision)?;
        let descriptor = reader.descriptor();
        if descriptor.cache_identity != cache_identity
            || descriptor.instrument_market != binding.instrument().market().discriminant()
            || descriptor.instrument_symbol != binding.instrument().symbol().as_str()
            || descriptor.trading_date_epoch_days != binding.trading_date().as_epoch_days()
        {
            return Err(CacheReadError::BindingMismatch);
        }
        self.opened.push(binding.clone());
        if let Ok(path) = std::env::var("OSMIUM_STREAM_OPEN_AUDIT") {
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            writeln!(
                file,
                "market={:?} symbol={} trading_date={} descriptor={}",
                binding.instrument().market(),
                binding.instrument().symbol(),
                binding.trading_date(),
                hex(binding.descriptor_id().as_bytes()),
            )?;
        }
        Ok(reader)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRecord {
    ordinal: u64,
    event: DomainEvent,
}

impl CacheRecord {
    #[must_use]
    pub const fn ordinal(&self) -> u64 {
        self.ordinal
    }

    #[must_use]
    pub const fn event(&self) -> &DomainEvent {
        &self.event
    }

    #[must_use]
    pub fn into_event(self) -> DomainEvent {
        self.event
    }
}

fn validate_descriptor(descriptor: &CacheDescriptor) -> Result<(), CacheReadError> {
    if descriptor.cache_format_version != CACHE_FORMAT_VERSION
        || descriptor.market_types_version != MARKET_TYPES_VERSION
        || descriptor.event_schema_version != EVENT_SCHEMA_VERSION
        || descriptor.canonical_event_version != CANONICAL_EVENT_VERSION
        || descriptor.ordering_rule_version != ORDERING_RULE_VERSION
    {
        return Err(CacheReadError::IncompatibleDescriptor);
    }
    Ok(())
}

fn validate_event(descriptor: &CacheDescriptor, event: &DomainEvent) -> Result<(), CacheReadError> {
    if event.instrument().market().discriminant() != descriptor.instrument_market
        || event.instrument().symbol().as_str() != descriptor.instrument_symbol
        || event.trading_date().as_epoch_days() != descriptor.trading_date_epoch_days
    {
        return Err(CacheReadError::CoverageMismatch);
    }
    Ok(())
}

fn read_u16(reader: &mut impl Read) -> Result<u16, io::Error> {
    let mut bytes = [0; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32(reader: &mut impl Read) -> Result<u32, io::Error> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Result<u64, io::Error> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_be_bytes(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[derive(Debug)]
pub enum CacheBuildError {
    Io(io::Error),
    Verification(crate::VerificationError),
    SourceManifest,
    SourceJson(String),
    Normalization(String),
    Ordering(String),
    Canonical(String),
    Descriptor(String),
    EventTooLarge,
    BuildAlreadyExists,
    CacheAlreadyExists(String),
    Partition(String),
}

#[derive(Debug)]
pub enum CacheCatalogError {
    Io(io::Error),
    Layout(PartitionRepositoryError),
    Descriptor(String),
}

impl fmt::Display for CacheCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for CacheCatalogError {}

impl From<io::Error> for CacheCatalogError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<PartitionRepositoryError> for CacheCatalogError {
    fn from(error: PartitionRepositoryError) -> Self {
        Self::Layout(error)
    }
}

impl fmt::Display for CacheBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for CacheBuildError {}

impl From<io::Error> for CacheBuildError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<crate::VerificationError> for CacheBuildError {
    fn from(error: crate::VerificationError) -> Self {
        Self::Verification(error)
    }
}

#[derive(Debug)]
pub enum CacheReadError {
    Io(io::Error),
    Descriptor(String),
    Header,
    IncompatibleDescriptor,
    StaleSourceLineage,
    Canonical(String),
    Ordering(String),
    OrderingRegression,
    CoverageMismatch,
    BindingMismatch,
    PayloadChecksum,
    BoundsMismatch,
    EventTooLarge,
    TrailingBytes,
}

impl fmt::Display for CacheReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for CacheReadError {}

impl From<io::Error> for CacheReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom};

    use market_types::{
        BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, DomainEvent, EventPayload,
        InstrumentId, MarketAnnotations, MarketId, MatchTime, Observation, Price, Quantity,
        QuantityUnit, QuoteSnapshot, SourceFormatId, Symbol, TradingDate, TwseQuoteAnnotations,
        Volume,
    };

    use super::*;
    use crate::{PublishedRevision, SourcePage, SourceRequestIdentity, StagingRevision};
    use run_planner::{SessionPlan, SourceId, SourcePartitionKey};
    use strategy_api::SessionKind;

    #[test]
    fn cache_event_record_length_is_bounded_before_allocation() {
        assert!(matches!(
            validate_event_record_length(MAX_CACHE_EVENT_RECORD_BYTES),
            Ok(length) if length == MAX_CACHE_EVENT_RECORD_BYTES as usize
        ));
        assert!(matches!(
            validate_event_record_length(MAX_CACHE_EVENT_RECORD_BYTES + 1),
            Err(CacheReadError::EventTooLarge)
        ));
    }

    fn instrument() -> InstrumentId {
        InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap())
    }

    fn date() -> TradingDate {
        TradingDate::parse("2026-07-27").unwrap()
    }

    fn event() -> DomainEvent {
        let unit = QuantityUnit::TradingUnit;
        let level = |price: &str| {
            BookLevel::new(
                Price::parse(price).unwrap(),
                Quantity::new(2, unit).unwrap(),
            )
        };
        let book = CompleteBookSnapshot::new(
            BookSide::new(BookSideKind::Bid, vec![level("99")]).unwrap(),
            BookSide::new(BookSideKind::Ask, vec![level("101")]).unwrap(),
        )
        .unwrap();
        DomainEvent::new(
            instrument(),
            date(),
            SourceFormatId::new("SYNTHETIC").unwrap(),
            MatchTime::from_unix_microseconds(1),
            Some(1),
            EventPayload::QuoteSnapshot(
                QuoteSnapshot::new(
                    book,
                    Observation::NoObservation,
                    Observation::Set(Volume::new(1, unit)),
                    MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0)),
                )
                .unwrap(),
            ),
        )
    }

    fn source(root: &Path, key: Option<&SourcePartitionKey>) -> PublishedRevision {
        let mut staging = match key {
            Some(key) => StagingRevision::create_for_partition(root, key, "cache-source").unwrap(),
            None => StagingRevision::create(root, "cache-source").unwrap(),
        };
        let query_identity = SourceRequestIdentity::from_bytes([1; 32]);
        let body = br#"{"items":[{"synthetic":true}],"next_cursor":null}"#.to_vec();
        let page = SourcePage::new(0, query_identity, body, 1);
        staging.stage_page(&page).unwrap();
        staging
            .stage_daily_instrument(
                SourceRequestIdentity::from_bytes([2; 32]),
                br#"{"synthetic":true}"#,
            )
            .unwrap();
        staging.publish(query_identity, true).unwrap()
    }

    fn mapping() -> NormalizerMappingIdentity {
        NormalizerMappingIdentity::from_static("synthetic-provider-mapping", 1)
    }

    fn partition_key() -> SourcePartitionKey {
        let instrument = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let date: TradingDate = "2026-07-27".parse().unwrap();
        let session = SessionPlan::for_instrument(&instrument, date, [SessionKind::Regular])
            .expect("session");
        SourcePartitionKey::new(
            SourceId::new("synthetic-source").unwrap(),
            instrument,
            date,
            [SessionKind::Regular],
            session.identity(),
        )
        .expect("partition key")
    }

    #[test]
    fn cache_round_trips_canonical_events_and_validates_eof() {
        let root = tempfile::tempdir().unwrap();
        source(root.path(), None);
        let published = CacheBuilder::new(root.path())
            .build_external_current(&instrument(), date(), &mapping(), vec![event()])
            .unwrap();
        let mut reader = CacheReader::open(published.path()).unwrap();
        let event = reader.next_record().unwrap().unwrap().into_event();
        assert_eq!(event.instrument().symbol().as_str(), "2330");
        assert!(reader.next_record().unwrap().is_none());
        assert!(reader.next_record().unwrap().is_none());
    }

    #[test]
    fn cache_reader_rejects_oversized_record_before_reading_its_body() {
        let root = tempfile::tempdir().unwrap();
        let published = CacheBuilder::new(root.path())
            .build_external_current(&instrument(), date(), &mapping(), {
                source(root.path(), None);
                vec![event()]
            })
            .unwrap();
        let mut events = OpenOptions::new()
            .write(true)
            .open(published.path().join("events.bin"))
            .unwrap();
        events.seek(SeekFrom::Start(19)).unwrap(); // magic + version + record count
        events
            .write_all(&(MAX_CACHE_EVENT_RECORD_BYTES + 1).to_be_bytes())
            .unwrap();
        drop(events);

        let mut reader = CacheReader::open(published.path()).unwrap();
        assert!(matches!(
            reader.next_record(),
            Err(CacheReadError::EventTooLarge)
        ));
    }

    #[test]
    fn mapping_version_is_part_of_descriptor() {
        let root = tempfile::tempdir().unwrap();
        source(root.path(), None);
        let published = CacheBuilder::new(root.path())
            .build_external_current(&instrument(), date(), &mapping(), vec![event()])
            .unwrap();
        assert_eq!(
            published.descriptor().normalizer_mapping_version,
            mapping().version()
        );
        assert_eq!(
            published.descriptor().ordering_rule_version,
            ORDERING_RULE_VERSION
        );
        assert!(matches!(
            CacheReader::open_bound(published.path(), &"0".repeat(64)).unwrap_err(),
            CacheReadError::StaleSourceLineage
        ));
    }

    #[test]
    fn cache_identity_binds_the_mapping_name_as_well_as_its_version() {
        let root = tempfile::tempdir().unwrap();
        source(root.path(), None);
        let report = LocalSourceRepository::new(root.path())
            .verify_current()
            .unwrap();
        let common = |mapping_name| {
            let mapping = NormalizerMappingIdentity::new(mapping_name, 1).unwrap();
            cache_identity(
                &report,
                &instrument(),
                date(),
                &mapping,
                None,
                "same-payload",
                1,
            )
        };

        assert_ne!(common("provider-a"), common("provider-b"));
    }

    #[test]
    fn cache_reader_rejects_every_stale_canonical_schema_version() {
        let root = tempfile::tempdir().unwrap();
        source(root.path(), None);
        let published = CacheBuilder::new(root.path())
            .build_external_current(&instrument(), date(), &mapping(), vec![event()])
            .unwrap();
        let descriptor_path = published.path().join("descriptor.yaml");

        for version_field in 0..5 {
            let mut descriptor = published.descriptor().clone();
            match version_field {
                0 => descriptor.cache_format_version -= 1,
                1 => descriptor.market_types_version -= 1,
                2 => descriptor.event_schema_version -= 1,
                3 => descriptor.canonical_event_version -= 1,
                4 => descriptor.ordering_rule_version -= 1,
                _ => unreachable!(),
            }
            fs::write(&descriptor_path, serde_json::to_vec(&descriptor).unwrap()).unwrap();

            assert!(matches!(
                CacheReader::open(published.path()),
                Err(CacheReadError::IncompatibleDescriptor)
            ));
        }
    }

    #[test]
    fn cache_reader_does_not_require_a_provider_mapping_registry() {
        let root = tempfile::tempdir().unwrap();
        source(root.path(), None);
        let published = CacheBuilder::new(root.path())
            .build_external_current(&instrument(), date(), &mapping(), vec![event()])
            .unwrap();
        let descriptor_path = published.path().join("descriptor.yaml");
        let mut descriptor = published.descriptor().clone();
        descriptor.normalizer_mapping_name = "another-provider-mapping".to_owned();
        descriptor.normalizer_mapping_version = 42;
        fs::write(&descriptor_path, serde_json::to_vec(&descriptor).unwrap()).unwrap();

        assert!(CacheReader::open(published.path()).is_ok());
    }

    #[test]
    fn partition_cache_preserves_partition_lineage_and_keyed_layout() {
        let root = tempfile::tempdir().unwrap();
        let key = partition_key();
        let source = source(root.path(), Some(&key));
        let expected_mapping = mapping();
        let cache = CacheBuilder::new(root.path())
            .build_external_partition(&key, &expected_mapping, vec![event()])
            .unwrap();
        assert!(
            cache.path().starts_with(
                root.path()
                    .join("cache/replay/synthetic-source/twse/2026-07-27/2330")
            )
        );
        let partition_identity = hex(key.identity().as_bytes());
        assert_eq!(
            cache.descriptor().partition_identity.as_deref(),
            Some(partition_identity.as_str())
        );
        let catalog = PartitionCacheCatalog::new(root.path());
        let found = catalog
            .inspect(
                &key,
                &source.manifest().revision_identity,
                &expected_mapping,
            )
            .unwrap();
        assert!(matches!(
            found,
            PartitionCacheInspection::Current(ref entry)
                if entry.descriptor() == cache.descriptor()
        ));

        let mut reader = CacheReader::open(cache.path()).unwrap();
        let mut events = Vec::new();
        while let Some(record) = reader.next_record().unwrap() {
            events.push(record.into_event());
        }
        let same_version_different_name = NormalizerMappingIdentity::new(
            "another-provider-twse-mapping",
            expected_mapping.version(),
        )
        .unwrap();
        let other_cache = CacheBuilder::new(root.path())
            .build_external_partition(&key, &same_version_different_name, events)
            .unwrap();
        assert_ne!(
            cache.descriptor().cache_identity,
            other_cache.descriptor().cache_identity
        );
        assert_ne!(cache.path(), other_cache.path());

        let descriptor_path = cache.path().join("descriptor.yaml");
        let mut stale_descriptor = cache.descriptor().clone();
        stale_descriptor.normalizer_mapping_version -= 1;
        fs::write(
            descriptor_path,
            serde_json::to_vec(&stale_descriptor).unwrap(),
        )
        .unwrap();
        assert_eq!(
            catalog
                .inspect(
                    &key,
                    &source.manifest().revision_identity,
                    &expected_mapping,
                )
                .unwrap(),
            PartitionCacheInspection::Stale
        );
    }
}
