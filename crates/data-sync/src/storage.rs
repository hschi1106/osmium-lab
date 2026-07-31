use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{PageCommitReceipt, PendingPage, SanitizedQueryIdentity};

pub const ZSTD_COMPRESSION_LEVEL: i32 = 3;
const SOURCE_MANIFEST_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionPolicy {
    ZstdPerPageV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectKind {
    TickPage,
    DailyInstrument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageMetadata {
    pub kind: ObjectKind,
    pub ordinal: u32,
    pub relative_path: PathBuf,
    pub record_count: u64,
    pub uncompressed_bytes: u64,
    pub compressed_bytes: u64,
    pub uncompressed_sha256: String,
    pub compressed_sha256: String,
    pub compression: CompressionPolicy,
    pub zstd_implementation: String,
    pub zstd_version: String,
    pub query_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedObject {
    metadata: PageMetadata,
}

impl StagedObject {
    #[must_use]
    pub const fn metadata(&self) -> &PageMetadata {
        &self.metadata
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPage {
    object: StagedObject,
    receipt: PageCommitReceipt,
}

impl StagedPage {
    #[must_use]
    pub const fn metadata(&self) -> &PageMetadata {
        self.object.metadata()
    }

    #[must_use]
    pub const fn commit_receipt(&self) -> PageCommitReceipt {
        self.receipt
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceManifest {
    pub manifest_version: u16,
    pub revision_identity: String,
    pub query_identity: String,
    pub terminal_cursor_reached: bool,
    pub tick_record_count: u64,
    pub pages: Vec<PageMetadata>,
    pub daily_instrument: PageMetadata,
}

impl SourceManifest {
    pub(crate) fn build(
        query_identity: SanitizedQueryIdentity,
        terminal_cursor_reached: bool,
        mut pages: Vec<PageMetadata>,
        daily_instrument: PageMetadata,
    ) -> Result<Self, StagingError> {
        if !terminal_cursor_reached {
            return Err(StagingError::CursorNotTerminal);
        }
        pages.sort_by_key(|page| page.ordinal);
        for (expected, page) in pages.iter().enumerate() {
            if page.kind != ObjectKind::TickPage || page.ordinal != expected as u32 {
                return Err(StagingError::NonContiguousPages);
            }
        }
        let tick_record_count = pages
            .iter()
            .try_fold(0_u64, |total, page| total.checked_add(page.record_count))
            .ok_or(StagingError::CountOverflow)?;
        let query_identity = hex(query_identity.as_bytes());
        let mut semantic = Vec::new();
        semantic.extend_from_slice(b"OSSR");
        semantic.extend_from_slice(&SOURCE_MANIFEST_VERSION.to_be_bytes());
        append_semantic_object(&daily_instrument, &mut semantic)?;
        for page in &pages {
            append_semantic_object(page, &mut semantic)?;
        }
        let revision_identity = hex(&Sha256::digest(semantic));
        Ok(Self {
            manifest_version: SOURCE_MANIFEST_VERSION,
            revision_identity,
            query_identity,
            terminal_cursor_reached,
            tick_record_count,
            pages,
            daily_instrument,
        })
    }
}

fn append_semantic_object(
    metadata: &PageMetadata,
    target: &mut Vec<u8>,
) -> Result<(), StagingError> {
    target.push(match metadata.kind {
        ObjectKind::TickPage => 1,
        ObjectKind::DailyInstrument => 2,
    });
    target.extend_from_slice(&metadata.ordinal.to_be_bytes());
    target.extend_from_slice(&metadata.uncompressed_bytes.to_be_bytes());
    let checksum =
        decode_hex_32(&metadata.uncompressed_sha256).ok_or(StagingError::InvalidChecksum)?;
    target.extend_from_slice(&checksum);
    Ok(())
}

#[derive(Debug)]
pub struct StagingRevision {
    publish_root: PathBuf,
    attempt_path: PathBuf,
    pages: Vec<PageMetadata>,
    daily_instrument: Option<PageMetadata>,
}

impl StagingRevision {
    pub fn create(data_root: impl AsRef<Path>, attempt_id: &str) -> Result<Self, StagingError> {
        validate_attempt_id(attempt_id)?;
        let data_root = data_root.as_ref().to_path_buf();
        let attempt_path = data_root.join("staging").join(attempt_id);
        fs::create_dir_all(attempt_path.join("ticks/pages"))?;
        fs::create_dir_all(attempt_path.join("instrument"))?;
        Ok(Self {
            publish_root: data_root.join("source"),
            attempt_path,
            pages: Vec::new(),
            daily_instrument: None,
        })
    }

    pub fn create_for_partition(
        data_root: impl AsRef<Path>,
        key: &run_planner::SourcePartitionKey,
        attempt_id: &str,
    ) -> Result<Self, StagingError> {
        validate_attempt_id(attempt_id)?;
        let repository = crate::PartitionedSourceRepository::new(data_root, key.clone())
            .map_err(|error| StagingError::Partition(error.to_string()))?;
        repository
            .ensure_layout()
            .map_err(|error| StagingError::Partition(error.to_string()))?;
        let publish_root = repository.root().to_path_buf();
        let attempt_path = publish_root.join("staging").join(attempt_id);
        fs::create_dir_all(attempt_path.join("ticks/pages"))?;
        fs::create_dir_all(attempt_path.join("instrument"))?;
        Ok(Self {
            publish_root,
            attempt_path,
            pages: Vec::new(),
            daily_instrument: None,
        })
    }

    pub fn resume(data_root: impl AsRef<Path>, attempt_id: &str) -> Result<Self, StagingError> {
        let data_root = data_root.as_ref().to_path_buf();
        let attempt_path = data_root.join("staging").join(attempt_id);
        let checkpoint = crate::CursorCheckpoint::load(&attempt_path.join("checkpoint.json"))?;
        let mut metadata_paths = fs::read_dir(attempt_path.join("ticks/pages"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "yaml")
            })
            .collect::<Vec<_>>();
        metadata_paths.sort();
        let mut pages = metadata_paths
            .iter()
            .map(|path| {
                serde_json::from_slice::<PageMetadata>(&fs::read(path)?)
                    .map_err(|error| StagingError::Manifest(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if pages.len() < checkpoint.committed_pages() as usize {
            return Err(StagingError::CheckpointPageMismatch);
        }
        pages.truncate(checkpoint.committed_pages() as usize);
        let daily_path = attempt_path.join("instrument/daily.yaml");
        let daily_instrument = if daily_path.exists() {
            Some(
                serde_json::from_slice(&fs::read(daily_path)?)
                    .map_err(|error| StagingError::Manifest(error.to_string()))?,
            )
        } else {
            None
        };
        Ok(Self {
            publish_root: data_root.join("source"),
            attempt_path,
            pages,
            daily_instrument,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.attempt_path
    }

    pub fn stage_page(&mut self, page: &PendingPage) -> Result<StagedPage, StagingError> {
        if page.ordinal() != self.pages.len() as u32 {
            return Err(StagingError::NonContiguousPages);
        }
        let relative_path = PathBuf::from(format!("ticks/pages/{:08}.json.zst", page.ordinal()));
        let metadata_path = self
            .attempt_path
            .join(format!("ticks/pages/{:08}.yaml", page.ordinal()));
        if metadata_path.exists() {
            let metadata: PageMetadata = serde_json::from_slice(&fs::read(&metadata_path)?)
                .map_err(|error| StagingError::Manifest(error.to_string()))?;
            if metadata.record_count != page.record_count()
                || metadata.uncompressed_bytes != page.body().len() as u64
                || metadata.uncompressed_sha256 != hex(&Sha256::digest(page.body()))
            {
                return Err(StagingError::InterruptedPageChanged);
            }
            self.pages.push(metadata.clone());
            return Ok(StagedPage {
                object: StagedObject { metadata },
                receipt: page.commit_receipt(),
            });
        }
        let metadata = write_compressed_object(
            &self.attempt_path,
            &relative_path,
            ObjectKind::TickPage,
            page.ordinal(),
            page.record_count(),
            page.query_identity(),
            page.body(),
        )?;
        let metadata_bytes = serde_json::to_vec_pretty(&metadata)
            .map_err(|error| StagingError::Manifest(error.to_string()))?;
        write_atomic_file(&metadata_path, &metadata_bytes)?;
        self.pages.push(metadata.clone());
        Ok(StagedPage {
            object: StagedObject { metadata },
            receipt: page.commit_receipt(),
        })
    }

    pub fn stage_daily_instrument(
        &mut self,
        query_identity: SanitizedQueryIdentity,
        body: &[u8],
    ) -> Result<StagedObject, StagingError> {
        serde_json::from_slice::<serde_json::Value>(body)
            .map_err(|error| StagingError::InvalidJson(error.to_string()))?;
        let metadata = write_compressed_object(
            &self.attempt_path,
            Path::new("instrument/daily.json.zst"),
            ObjectKind::DailyInstrument,
            0,
            1,
            query_identity,
            body,
        )?;
        let metadata_bytes = serde_json::to_vec_pretty(&metadata)
            .map_err(|error| StagingError::Manifest(error.to_string()))?;
        write_atomic_file(
            &self.attempt_path.join("instrument/daily.yaml"),
            &metadata_bytes,
        )?;
        self.daily_instrument = Some(metadata.clone());
        Ok(StagedObject { metadata })
    }

    pub fn publish(
        self,
        query_identity: SanitizedQueryIdentity,
        terminal_cursor_reached: bool,
    ) -> Result<PublishedRevision, StagingError> {
        let daily_instrument = self
            .daily_instrument
            .ok_or(StagingError::DailyInstrumentMissing)?;
        let manifest = SourceManifest::build(
            query_identity,
            terminal_cursor_reached,
            self.pages,
            daily_instrument,
        )?;
        for staging_only in [
            self.attempt_path.join("checkpoint.json"),
            self.attempt_path.join("instrument/daily.yaml"),
        ] {
            if staging_only.exists() {
                fs::remove_file(staging_only)?;
            }
        }
        for page in &manifest.pages {
            let metadata = self
                .attempt_path
                .join(format!("ticks/pages/{:08}.yaml", page.ordinal));
            if metadata.exists() {
                fs::remove_file(metadata)?;
            }
        }
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| StagingError::Manifest(error.to_string()))?;
        write_atomic_file(&self.attempt_path.join("manifest.yaml"), &manifest_bytes)?;
        sync_directory(&self.attempt_path)?;

        let revisions = self.publish_root.join("revisions");
        fs::create_dir_all(&revisions)?;
        let published_path = revisions.join(&manifest.revision_identity);
        if published_path.exists() {
            return Err(StagingError::RevisionAlreadyExists(
                manifest.revision_identity,
            ));
        }
        fs::rename(&self.attempt_path, &published_path)?;
        sync_directory(&revisions)?;

        let current = self.publish_root.join("current.yaml");
        write_atomic_file(&current, manifest.revision_identity.as_bytes())?;
        Ok(PublishedRevision {
            path: published_path,
            manifest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedRevision {
    path: PathBuf,
    manifest: SourceManifest,
}

impl PublishedRevision {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn manifest(&self) -> &SourceManifest {
        &self.manifest
    }
}

fn write_compressed_object(
    root: &Path,
    relative_path: &Path,
    kind: ObjectKind,
    ordinal: u32,
    record_count: u64,
    query_identity: SanitizedQueryIdentity,
    body: &[u8],
) -> Result<PageMetadata, StagingError> {
    let final_path = root.join(relative_path);
    let tmp_path = final_path.with_extension("zst.tmp");
    let tmp_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp_path)?;
    let writer = BufWriter::new(tmp_file);
    let mut encoder = zstd::stream::write::Encoder::new(writer, ZSTD_COMPRESSION_LEVEL)?;
    encoder.include_checksum(true)?;
    encoder.write_all(body)?;
    let mut writer = encoder.finish()?;
    writer.flush()?;
    writer.get_ref().sync_all()?;

    let compressed_sha256 = hash_file(&tmp_path)?;
    let compressed_bytes = fs::metadata(&tmp_path)?.len();
    let mut decoder = zstd::stream::read::Decoder::new(BufReader::new(File::open(&tmp_path)?))?;
    let mut decoded = Vec::new();
    decoder.read_to_end(&mut decoded)?;
    if decoded != body {
        return Err(StagingError::RoundTripMismatch);
    }
    serde_json::from_slice::<serde_json::Value>(&decoded)
        .map_err(|error| StagingError::InvalidJson(error.to_string()))?;
    fs::rename(&tmp_path, &final_path)?;
    sync_directory(final_path.parent().expect("object has parent"))?;

    Ok(PageMetadata {
        kind,
        ordinal,
        relative_path: relative_path.to_path_buf(),
        record_count,
        uncompressed_bytes: body.len() as u64,
        compressed_bytes,
        uncompressed_sha256: hex(&Sha256::digest(body)),
        compressed_sha256: hex(&compressed_sha256),
        compression: CompressionPolicy::ZstdPerPageV1,
        zstd_implementation: "zstd".to_owned(),
        zstd_version: zstd::zstd_safe::version_string().to_owned(),
        query_identity: hex(query_identity.as_bytes()),
    })
}

fn hash_file(path: &Path) -> Result<[u8; 32], io::Error> {
    let mut file = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().into())
}

fn write_atomic_file(path: &Path, bytes: &[u8]) -> Result<(), StagingError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    sync_directory(path.parent().expect("file has parent"))?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), io::Error> {
    File::open(path)?.sync_all()
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

fn decode_hex_32(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = decode_nibble(pair[0])?;
        let low = decode_nibble(pair[1])?;
        decoded[index] = high << 4 | low;
    }
    Some(decoded)
}

fn decode_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn validate_attempt_id(attempt_id: &str) -> Result<(), StagingError> {
    if attempt_id.is_empty()
        || !attempt_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(StagingError::InvalidAttemptId);
    }
    Ok(())
}

#[derive(Debug)]
pub enum StagingError {
    Io(io::Error),
    InvalidAttemptId,
    InvalidJson(String),
    RoundTripMismatch,
    NonContiguousPages,
    CountOverflow,
    InvalidChecksum,
    CursorNotTerminal,
    DailyInstrumentMissing,
    RevisionAlreadyExists(String),
    CheckpointPageMismatch,
    InterruptedPageChanged,
    Manifest(String),
    Partition(String),
}

impl fmt::Display for StagingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for StagingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for StagingError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use market_types::{InstrumentId, MarketId, Symbol};
    use run_planner::{SessionPlan, SourceId, SourcePartitionKey};
    use strategy_api::SessionKind;

    use super::*;
    use crate::{ArchiveKind, ArchiveTimestamp, CursorStateMachine, TeralionQuery};

    fn query() -> TeralionQuery {
        TeralionQuery::ticks(
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            ArchiveTimestamp::parse("2026-07-27T08:55:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-27T13:35:00+08:00").unwrap(),
            [ArchiveKind::Quote],
            5_000,
        )
        .unwrap()
    }

    fn partition_key() -> SourcePartitionKey {
        let instrument = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let date = "2026-07-27".parse().unwrap();
        let session_plan =
            SessionPlan::for_instrument(&instrument, date, [SessionKind::Regular]).unwrap();
        SourcePartitionKey::new(
            SourceId::TeralionFeedArchive,
            instrument,
            date,
            [SessionKind::Regular],
            session_plan.identity(),
        )
        .unwrap()
    }

    #[test]
    fn page_is_zstd_only_and_round_trips_before_commit() {
        let root = tempfile::tempdir().unwrap();
        let mut staging = StagingRevision::create(root.path(), "attempt-1").unwrap();
        let query = query();
        let mut machine = CursorStateMachine::new(query.clone()).unwrap();
        let request = machine.request_next().unwrap();
        let body = br#"{"items":[],"next_cursor":null}"#.to_vec();
        let pending = machine.accept_response(&request, body.clone()).unwrap();
        let staged = staging.stage_page(pending).unwrap();
        assert!(
            staging
                .path()
                .join("ticks/pages/00000000.json.zst")
                .is_file()
        );
        assert!(!staging.path().join("ticks/pages/00000000.json").exists());
        assert_eq!(
            staged.metadata().uncompressed_sha256,
            hex(&Sha256::digest(&body))
        );
        machine.commit_page(staged.commit_receipt()).unwrap();
    }

    #[test]
    fn publish_requires_terminal_cursor_and_daily_instrument() {
        let root = tempfile::tempdir().unwrap();
        let staging = StagingRevision::create(root.path(), "attempt-2").unwrap();
        let error = staging.publish(query().identity(), false).unwrap_err();
        assert!(matches!(error, StagingError::DailyInstrumentMissing));
    }

    #[test]
    fn publish_atomically_moves_an_immutable_revision() {
        let root = tempfile::tempdir().unwrap();
        let query = query();
        let mut staging = StagingRevision::create(root.path(), "attempt-3").unwrap();
        let mut machine = CursorStateMachine::new(query.clone()).unwrap();
        let request = machine.request_next().unwrap();
        let pending = machine
            .accept_response(&request, br#"{"items":[],"next_cursor":null}"#.to_vec())
            .unwrap();
        let staged = staging.stage_page(pending).unwrap();
        machine.commit_page(staged.commit_receipt()).unwrap();
        staging
            .stage_daily_instrument(
                TeralionQuery::daily_instrument(
                    query.instrument().unwrap().clone(),
                    "2026-07-27".parse().unwrap(),
                )
                .identity(),
                br#"{"symbol":"2330","market":"twse","date":"2026-07-27"}"#,
            )
            .unwrap();

        let published = staging.publish(query.identity(), true).unwrap();
        assert!(published.path().join("manifest.yaml").is_file());
        assert!(published.path().join("instrument/daily.json.zst").is_file());
        assert_eq!(
            fs::read_to_string(root.path().join("source/current.yaml")).unwrap(),
            published.manifest().revision_identity
        );
    }

    #[test]
    fn partition_publish_has_an_independent_current_pointer() {
        let root = tempfile::tempdir().unwrap();
        let query = query();
        let key = partition_key();
        let mut staging =
            StagingRevision::create_for_partition(root.path(), &key, "attempt-4").unwrap();
        let mut machine = CursorStateMachine::new(query.clone()).unwrap();
        let request = machine.request_next().unwrap();
        let pending = machine
            .accept_response(&request, br#"{"items":[],"next_cursor":null}"#.to_vec())
            .unwrap();
        let staged = staging.stage_page(pending).unwrap();
        machine.commit_page(staged.commit_receipt()).unwrap();
        staging
            .stage_daily_instrument(
                TeralionQuery::daily_instrument(
                    query.instrument().unwrap().clone(),
                    "2026-07-27".parse().unwrap(),
                )
                .identity(),
                br#"{"symbol":"2330","market":"twse","date":"2026-07-27"}"#,
            )
            .unwrap();
        let published = staging.publish(query.identity(), true).unwrap();
        let repository = crate::PartitionedSourceRepository::new(root.path(), key).unwrap();
        assert_eq!(
            fs::read_to_string(repository.current_path()).unwrap(),
            published.manifest().revision_identity
        );
        assert!(repository.inspect().report().is_some());
        assert!(!root.path().join("source/current.yaml").exists());
    }
}
