use std::{
    error::Error,
    fmt,
    fs::{self, File},
    io::{self, BufReader, Read},
    path::{Component, Path, PathBuf},
};

use run_planner::{CorruptReason, IncompleteReason, SourceRevisionIdentity, SourceState};
use sha2::{Digest, Sha256};

use crate::{ObjectKind, PageMetadata, SourceManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    revision: SourceRevisionIdentity,
    manifest: SourceManifest,
}

impl VerificationReport {
    #[must_use]
    pub const fn revision(&self) -> SourceRevisionIdentity {
        self.revision
    }

    #[must_use]
    pub const fn manifest(&self) -> &SourceManifest {
        &self.manifest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInspection {
    state: SourceState,
    report: Option<VerificationReport>,
    diagnostic: Option<Box<str>>,
}

impl SourceInspection {
    #[must_use]
    pub const fn state(&self) -> SourceState {
        self.state
    }

    #[must_use]
    pub const fn report(&self) -> Option<&VerificationReport> {
        self.report.as_ref()
    }

    #[must_use]
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDisposition {
    ReuseCompleteSource {
        revision: SourceRevisionIdentity,
        http_requests: u64,
    },
    DownloadMissingSource,
    ResumeOrRestartBuilding,
    RepairIncomplete {
        reason: IncompleteReason,
    },
    RepairCorrupt {
        reason: CorruptReason,
    },
}

impl SyncDisposition {
    #[must_use]
    pub const fn requires_http(self) -> bool {
        !matches!(self, Self::ReuseCompleteSource { .. })
    }
}

#[derive(Debug, Clone)]
pub struct LocalSourceRepository {
    data_root: PathBuf,
}

impl LocalSourceRepository {
    #[must_use]
    pub fn new(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    #[must_use]
    pub fn inspect(&self) -> SourceInspection {
        let current_path = self.data_root.join("source/current.yaml");
        if !current_path.exists() {
            let building = self
                .data_root
                .join("staging")
                .read_dir()
                .is_ok_and(|mut entries| entries.next().is_some());
            return SourceInspection {
                state: if building {
                    SourceState::Building
                } else {
                    SourceState::Missing
                },
                report: None,
                diagnostic: None,
            };
        }
        match self.verify_current() {
            Ok(report) => SourceInspection {
                state: SourceState::Complete {
                    revision: report.revision(),
                },
                report: Some(report),
                diagnostic: None,
            },
            Err(error) => {
                let state = error.source_state();
                SourceInspection {
                    state,
                    report: None,
                    diagnostic: Some(error.to_string().into_boxed_str()),
                }
            }
        }
    }

    #[must_use]
    pub fn plan_sync(&self) -> SyncDisposition {
        match self.inspect().state() {
            SourceState::Missing => SyncDisposition::DownloadMissingSource,
            SourceState::Building => SyncDisposition::ResumeOrRestartBuilding,
            SourceState::Complete { revision } => SyncDisposition::ReuseCompleteSource {
                revision,
                http_requests: 0,
            },
            SourceState::Incomplete { reason } => SyncDisposition::RepairIncomplete { reason },
            SourceState::Corrupt { reason } => SyncDisposition::RepairCorrupt { reason },
        }
    }

    pub fn verify_current(&self) -> Result<VerificationReport, VerificationError> {
        let revision_text = fs::read_to_string(self.data_root.join("source/current.yaml"))?;
        let revision_text = revision_text.trim();
        let revision_bytes =
            decode_hex_32(revision_text).ok_or(VerificationError::InvalidCurrentPointer)?;
        let revision_path = self.data_root.join("source/revisions").join(revision_text);
        let manifest_path = revision_path.join("manifest.yaml");
        let manifest_bytes = fs::read(&manifest_path)?;
        let manifest: SourceManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| VerificationError::Manifest(error.to_string()))?;
        if manifest.manifest_version != 1
            || manifest.revision_identity != revision_text
            || !manifest.terminal_cursor_reached
        {
            return Err(VerificationError::ManifestInvariant);
        }
        if manifest.pages.is_empty() {
            return Err(VerificationError::NoTickPages);
        }
        let mut tick_records = 0_u64;
        for (expected, page) in manifest.pages.iter().enumerate() {
            if page.kind != ObjectKind::TickPage || page.ordinal != expected as u32 {
                return Err(VerificationError::ManifestInvariant);
            }
            verify_object(&revision_path, page)?;
            tick_records = tick_records
                .checked_add(page.record_count)
                .ok_or(VerificationError::CountOverflow)?;
        }
        if manifest.daily_instrument.kind != ObjectKind::DailyInstrument {
            return Err(VerificationError::DailyInstrumentMissing);
        }
        verify_object(&revision_path, &manifest.daily_instrument)?;
        if tick_records != manifest.tick_record_count {
            return Err(VerificationError::RecordCountMismatch);
        }
        let rebuilt = SourceManifest::build(
            parse_query_identity(&manifest.query_identity)?,
            true,
            manifest.pages.clone(),
            manifest.daily_instrument.clone(),
        )
        .map_err(|error| VerificationError::Manifest(error.to_string()))?;
        if rebuilt.revision_identity != manifest.revision_identity {
            return Err(VerificationError::RevisionMismatch);
        }
        Ok(VerificationReport {
            revision: SourceRevisionIdentity::from_bytes(revision_bytes),
            manifest,
        })
    }
}

fn verify_object(root: &Path, metadata: &PageMetadata) -> Result<(), VerificationError> {
    if metadata
        .relative_path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(VerificationError::UnsafeObjectPath);
    }
    let path = root.join(&metadata.relative_path);
    let compressed = hash_file(&path)?;
    if hex(&compressed) != metadata.compressed_sha256
        || fs::metadata(&path)?.len() != metadata.compressed_bytes
    {
        return Err(VerificationError::CompressedChecksumMismatch);
    }
    let mut decoder = zstd::stream::read::Decoder::new(BufReader::new(File::open(path)?))
        .map_err(VerificationError::Io)?;
    let mut body = Vec::new();
    decoder
        .read_to_end(&mut body)
        .map_err(|_| VerificationError::CompressionFrameInvalid)?;
    if body.len() as u64 != metadata.uncompressed_bytes
        || hex(&Sha256::digest(&body)) != metadata.uncompressed_sha256
    {
        return Err(VerificationError::UncompressedChecksumMismatch);
    }
    let value: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|error| VerificationError::PayloadJson(error.to_string()))?;
    let actual_records = match metadata.kind {
        ObjectKind::TickPage => value
            .get("items")
            .and_then(serde_json::Value::as_array)
            .ok_or(VerificationError::PayloadEnvelope)?
            .len() as u64,
        ObjectKind::DailyInstrument => 1,
    };
    if actual_records != metadata.record_count {
        return Err(VerificationError::RecordCountMismatch);
    }
    Ok(())
}

fn parse_query_identity(value: &str) -> Result<crate::SanitizedQueryIdentity, VerificationError> {
    let bytes = decode_hex_32(value).ok_or(VerificationError::ManifestInvariant)?;
    Ok(crate::SanitizedQueryIdentity::from_bytes(bytes))
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
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = decode_nibble(pair[0])? << 4 | decode_nibble(pair[1])?;
    }
    Some(output)
}

fn decode_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[derive(Debug)]
pub enum VerificationError {
    Io(io::Error),
    InvalidCurrentPointer,
    Manifest(String),
    ManifestInvariant,
    NoTickPages,
    DailyInstrumentMissing,
    UnsafeObjectPath,
    CompressedChecksumMismatch,
    CompressionFrameInvalid,
    UncompressedChecksumMismatch,
    PayloadJson(String),
    PayloadEnvelope,
    RecordCountMismatch,
    CountOverflow,
    RevisionMismatch,
}

impl VerificationError {
    fn source_state(&self) -> SourceState {
        match self {
            Self::NoTickPages => SourceState::Incomplete {
                reason: IncompleteReason::CursorNotTerminal,
            },
            Self::DailyInstrumentMissing => SourceState::Incomplete {
                reason: IncompleteReason::DailyInstrumentMissing,
            },
            Self::Io(error) if error.kind() == io::ErrorKind::NotFound => SourceState::Corrupt {
                reason: CorruptReason::PayloadMissing,
            },
            Self::CompressedChecksumMismatch | Self::UncompressedChecksumMismatch => {
                SourceState::Corrupt {
                    reason: CorruptReason::PayloadChecksumMismatch,
                }
            }
            Self::CompressionFrameInvalid => SourceState::Corrupt {
                reason: CorruptReason::CompressionFrameInvalid,
            },
            Self::Manifest(_)
            | Self::ManifestInvariant
            | Self::InvalidCurrentPointer
            | Self::UnsafeObjectPath
            | Self::PayloadJson(_)
            | Self::PayloadEnvelope
            | Self::RecordCountMismatch
            | Self::CountOverflow
            | Self::RevisionMismatch
            | Self::Io(_) => SourceState::Corrupt {
                reason: CorruptReason::ManifestInvalid,
            },
        }
    }
}

impl fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for VerificationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for VerificationError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom, Write};

    use market_types::{InstrumentId, MarketId, Symbol};

    use super::*;
    use crate::{
        ArchiveKind, ArchiveTimestamp, CursorStateMachine, StagingRevision, TeralionQuery,
    };

    fn publish(root: &Path) -> crate::PublishedRevision {
        let ticks = TeralionQuery::ticks(
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            ArchiveTimestamp::parse("2026-07-27T08:55:00+08:00").unwrap(),
            ArchiveTimestamp::parse("2026-07-27T13:35:00+08:00").unwrap(),
            [ArchiveKind::Quote],
            5_000,
        )
        .unwrap();
        let mut staging = StagingRevision::create(root, "verify-fixture").unwrap();
        let mut machine = CursorStateMachine::new(ticks.clone()).unwrap();
        let request = machine.request_next().unwrap();
        let pending = machine
            .accept_response(&request, br#"{"items":[],"next_cursor":null}"#.to_vec())
            .unwrap();
        let staged = staging.stage_page(pending).unwrap();
        machine.commit_page(staged.commit_receipt()).unwrap();
        staging
            .stage_daily_instrument(
                TeralionQuery::daily_instrument(
                    ticks.instrument().unwrap().clone(),
                    "2026-07-27".parse().unwrap(),
                )
                .identity(),
                br#"{"symbol":"2330"}"#,
            )
            .unwrap();
        staging.publish(ticks.identity(), true).unwrap()
    }

    #[test]
    fn complete_revision_is_reused_without_http() {
        let root = tempfile::tempdir().unwrap();
        let published = publish(root.path());
        let repository = LocalSourceRepository::new(root.path());
        let inspection = repository.inspect();
        assert_eq!(
            inspection.state(),
            SourceState::Complete {
                revision: inspection.report().unwrap().revision()
            }
        );
        assert_eq!(
            repository.plan_sync(),
            SyncDisposition::ReuseCompleteSource {
                revision: inspection.report().unwrap().revision(),
                http_requests: 0
            }
        );
        assert!(published.path().is_dir());
    }

    #[test]
    fn damaged_payload_is_corrupt_and_requires_visible_repair() {
        let root = tempfile::tempdir().unwrap();
        let published = publish(root.path());
        let page = published.path().join("ticks/pages/00000000.json.zst");
        let mut file = fs::OpenOptions::new().write(true).open(page).unwrap();
        file.seek(SeekFrom::Start(2)).unwrap();
        file.write_all(b"damage").unwrap();
        file.sync_all().unwrap();

        let repository = LocalSourceRepository::new(root.path());
        assert!(matches!(
            repository.inspect().state(),
            SourceState::Corrupt {
                reason: CorruptReason::PayloadChecksumMismatch
            }
        ));
        assert!(repository.plan_sync().requires_http());
    }

    #[test]
    fn staging_without_publish_is_building() {
        let root = tempfile::tempdir().unwrap();
        StagingRevision::create(root.path(), "interrupted").unwrap();
        assert_eq!(
            LocalSourceRepository::new(root.path()).inspect().state(),
            SourceState::Building
        );
    }
}
