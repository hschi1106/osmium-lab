use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use market_types::{
    CANONICAL_EVENT_VERSION, DomainEvent, EVENT_SCHEMA_VERSION, MARKET_TYPES_VERSION, MatchTime,
};
use replay_engine::{ORDERING_RULE_VERSION, OrderingKey, order_events};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use twse_normalizer::{MAPPING_NAME, MAPPING_VERSION, NormalizerConfig, TwseNormalizer};

use crate::{LocalSourceRepository, ObjectKind, VerificationReport};

const CACHE_MAGIC: &[u8; 9] = b"OSMCACHE1";
pub const CACHE_FORMAT_VERSION: u16 = 1;

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

    pub fn build_current(
        &self,
        config: NormalizerConfig,
    ) -> Result<PublishedCache, CacheBuildError> {
        let report = LocalSourceRepository::new(&self.data_root).verify_current()?;
        self.build(&report, config)
    }

    fn build(
        &self,
        source: &VerificationReport,
        config: NormalizerConfig,
    ) -> Result<PublishedCache, CacheBuildError> {
        if source
            .manifest()
            .pages
            .iter()
            .any(|page| page.kind != ObjectKind::TickPage)
        {
            return Err(CacheBuildError::SourceManifest);
        }
        let source_path = self
            .data_root
            .join("source/revisions")
            .join(&source.manifest().revision_identity);
        let mut lines = Vec::new();
        for page in &source.manifest().pages {
            let file = File::open(source_path.join(&page.relative_path))?;
            let mut decoder = zstd::stream::read::Decoder::new(BufReader::new(file))?;
            let value: serde_json::Value = serde_json::from_reader(&mut decoder)
                .map_err(|error| CacheBuildError::SourceJson(error.to_string()))?;
            let items = value
                .get("items")
                .and_then(serde_json::Value::as_array)
                .ok_or(CacheBuildError::SourceManifest)?;
            for item in items {
                lines.push(
                    serde_json::to_string(item)
                        .map_err(|error| CacheBuildError::SourceJson(error.to_string()))?,
                );
            }
        }
        let events = TwseNormalizer::new(config.clone())
            .normalize_json_lines(&lines)
            .map_err(|error| CacheBuildError::Normalization(error.to_string()))?
            .into_events();
        let events =
            order_events(events).map_err(|error| CacheBuildError::Ordering(error.to_string()))?;

        let attempt = self.data_root.join("derived/staging/cache-build");
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
        let cache_identity = cache_identity(source, &config, &payload_sha256, events.len() as u64);
        let descriptor = CacheDescriptor {
            cache_format_version: CACHE_FORMAT_VERSION,
            cache_identity: cache_identity.clone(),
            source_revision_identity: source.manifest().revision_identity.clone(),
            instrument_market: config.instrument().market().discriminant(),
            instrument_symbol: config.instrument().symbol().as_str().to_owned(),
            trading_date_epoch_days: config.trading_date().as_epoch_days(),
            event_count: events.len() as u64,
            first_match_time_micros: first_match_time.map(MatchTime::as_unix_microseconds),
            last_match_time_micros: last_match_time.map(MatchTime::as_unix_microseconds),
            payload_sha256,
            market_types_version: MARKET_TYPES_VERSION,
            event_schema_version: EVENT_SCHEMA_VERSION,
            canonical_event_version: CANONICAL_EVENT_VERSION,
            ordering_rule_version: ORDERING_RULE_VERSION,
            normalizer_mapping_name: MAPPING_NAME.to_owned(),
            normalizer_mapping_version: MAPPING_VERSION,
        };
        let descriptor_bytes = serde_json::to_vec_pretty(&descriptor)
            .map_err(|error| CacheBuildError::Descriptor(error.to_string()))?;
        write_atomic(&attempt.join("descriptor.yaml"), &descriptor_bytes)?;
        File::open(&attempt)?.sync_all()?;

        let caches = self.data_root.join("derived/cache");
        fs::create_dir_all(&caches)?;
        let published_path = caches.join(&cache_identity);
        if published_path.exists() {
            return Err(CacheBuildError::CacheAlreadyExists(cache_identity));
        }
        fs::rename(&attempt, &published_path)?;
        File::open(&caches)?.sync_all()?;
        Ok(PublishedCache {
            path: published_path,
            descriptor,
        })
    }
}

fn cache_identity(
    source: &VerificationReport,
    config: &NormalizerConfig,
    payload_sha256: &str,
    event_count: u64,
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"OSCI");
    bytes.extend_from_slice(&CACHE_FORMAT_VERSION.to_be_bytes());
    bytes.extend_from_slice(source.revision().as_bytes());
    bytes.push(config.instrument().market().discriminant());
    bytes.extend_from_slice(config.instrument().symbol().as_bytes());
    bytes.extend_from_slice(&config.trading_date().to_canonical_bytes());
    bytes.extend_from_slice(&MAPPING_VERSION.to_be_bytes());
    bytes.extend_from_slice(&MARKET_TYPES_VERSION.to_be_bytes());
    bytes.extend_from_slice(&EVENT_SCHEMA_VERSION.to_be_bytes());
    bytes.extend_from_slice(&CANONICAL_EVENT_VERSION.to_be_bytes());
    bytes.extend_from_slice(&ORDERING_RULE_VERSION.to_be_bytes());
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
        let length = read_u32(&mut self.reader)?;
        let mut bytes = vec![0_u8; length as usize];
        self.reader.read_exact(&mut bytes)?;
        self.hasher.update(length.to_be_bytes());
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
        || descriptor.normalizer_mapping_name != MAPPING_NAME
        || descriptor.normalizer_mapping_version != MAPPING_VERSION
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
    PayloadChecksum,
    BoundsMismatch,
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
    use market_types::{InstrumentId, MarketId, Symbol, TradingDate};

    use super::*;
    use crate::{
        ArchiveKind, ArchiveTimestamp, CursorStateMachine, StagingRevision, TeralionQuery,
    };

    fn source(root: &Path) -> NormalizerConfig {
        let instrument = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let date: TradingDate = "2026-07-27".parse().unwrap();
        let ticks = TeralionQuery::ticks(
            instrument.clone(),
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
        let mut staging = StagingRevision::create(root, "cache-source").unwrap();
        let mut cursor = CursorStateMachine::new(ticks.clone()).unwrap();
        let request = cursor.request_next().unwrap();
        let pending = cursor.accept_response(&request, body).unwrap();
        let staged = staging.stage_page(pending).unwrap();
        cursor.commit_page(staged.commit_receipt()).unwrap();
        staging
            .stage_daily_instrument(
                TeralionQuery::daily_instrument(instrument.clone(), date).identity(),
                br#"{"symbol":"2330"}"#,
            )
            .unwrap();
        staging.publish(ticks.identity(), true).unwrap();
        NormalizerConfig::new(
            instrument,
            date,
            MatchTime::parse("2026-07-27T08:55:00+08:00").unwrap(),
            MatchTime::parse("2026-07-27T13:35:00+08:00").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn cache_round_trips_canonical_events_and_validates_eof() {
        let root = tempfile::tempdir().unwrap();
        let config = source(root.path());
        let published = CacheBuilder::new(root.path())
            .build_current(config)
            .unwrap();
        let mut reader = CacheReader::open(published.path()).unwrap();
        let event = reader.next_record().unwrap().unwrap().into_event();
        assert_eq!(event.instrument().symbol().as_str(), "2330");
        assert!(reader.next_record().unwrap().is_none());
        assert!(reader.next_record().unwrap().is_none());
    }

    #[test]
    fn mapping_version_is_part_of_descriptor() {
        let root = tempfile::tempdir().unwrap();
        let config = source(root.path());
        let published = CacheBuilder::new(root.path())
            .build_current(config)
            .unwrap();
        assert_eq!(
            published.descriptor().normalizer_mapping_version,
            MAPPING_VERSION
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
}
