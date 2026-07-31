use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use market_types::{InstrumentId, MarketId};
use run_planner::{SourceId, SourcePartitionKey};
use serde::{Deserialize, Serialize};

use crate::{LocalSourceRepository, SourceInspection};

pub const PARTITION_LAYOUT_VERSION: u16 = 1;
pub const PARTITION_MANIFEST_FILE: &str = "partition.yaml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePartitionManifest {
    pub layout_version: u16,
    pub source: String,
    pub instrument_market: u8,
    pub instrument_symbol: String,
    pub trading_date_epoch_days: i32,
    pub session_kinds: Vec<u8>,
    pub session_plan_identity: String,
    pub partition_identity: String,
}

impl SourcePartitionManifest {
    #[must_use]
    pub fn from_key(key: &SourcePartitionKey) -> Self {
        Self {
            layout_version: PARTITION_LAYOUT_VERSION,
            source: source_name(key.source()).to_owned(),
            instrument_market: key.instrument().market().discriminant(),
            instrument_symbol: key.instrument().symbol().as_str().to_owned(),
            trading_date_epoch_days: key.trading_date().as_epoch_days(),
            session_kinds: key.session_kinds().iter().map(|kind| *kind as u8).collect(),
            session_plan_identity: hex(key.session_plan_identity().as_bytes()),
            partition_identity: hex(key.identity().as_bytes()),
        }
    }

    #[must_use]
    pub fn matches(&self, key: &SourcePartitionKey) -> bool {
        self == &Self::from_key(key)
    }
}

#[derive(Debug, Clone)]
pub struct PartitionedSourceRepository {
    key: SourcePartitionKey,
    root: PathBuf,
    inner: LocalSourceRepository,
}

impl PartitionedSourceRepository {
    pub fn new(
        data_root: impl AsRef<Path>,
        key: SourcePartitionKey,
    ) -> Result<Self, PartitionRepositoryError> {
        let root = partition_root(data_root.as_ref(), &key)?;
        Ok(Self {
            key,
            inner: LocalSourceRepository::at_source_root(&root),
            root,
        })
    }

    #[must_use]
    pub const fn key(&self) -> &SourcePartitionKey {
        &self.key
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn current_path(&self) -> PathBuf {
        self.root.join("current.yaml")
    }

    #[must_use]
    pub fn partition_manifest_path(&self) -> PathBuf {
        self.root.join(PARTITION_MANIFEST_FILE)
    }

    pub fn ensure_layout(&self) -> Result<(), PartitionRepositoryError> {
        fs::create_dir_all(&self.root)?;
        let manifest_path = self.partition_manifest_path();
        let manifest = SourcePartitionManifest::from_key(&self.key);
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| PartitionRepositoryError::Manifest(error.to_string()))?;
        if manifest_path.exists() {
            let existing: SourcePartitionManifest =
                serde_json::from_slice(&fs::read(&manifest_path)?)
                    .map_err(|error| PartitionRepositoryError::Manifest(error.to_string()))?;
            if !existing.matches(&self.key) {
                return Err(PartitionRepositoryError::ManifestMismatch);
            }
        } else {
            write_atomic(&manifest_path, &bytes)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn inspect(&self) -> SourceInspection {
        match self.read_manifest() {
            Ok(Some(manifest)) if manifest.matches(&self.key) => self.inner.inspect(),
            Ok(Some(_)) => SourceInspection::corrupt("partition metadata does not match key"),
            Ok(None) => self.inner.inspect(),
            Err(error) => SourceInspection::corrupt(error.to_string()),
        }
    }

    #[must_use]
    pub fn plan_sync(&self) -> crate::SyncDisposition {
        self.inner.plan_sync()
    }

    pub fn verify_current(&self) -> Result<crate::VerificationReport, crate::VerificationError> {
        self.inner.verify_current()
    }

    fn read_manifest(&self) -> Result<Option<SourcePartitionManifest>, PartitionRepositoryError> {
        let path = self.partition_manifest_path();
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&fs::read(path)?).map_err(
            |error| PartitionRepositoryError::Manifest(error.to_string()),
        )?))
    }
}

pub fn partition_root(
    data_root: &Path,
    key: &SourcePartitionKey,
) -> Result<PathBuf, PartitionRepositoryError> {
    keyed_root(data_root, "source", key)
}

pub fn cache_partition_root(
    data_root: &Path,
    key: &SourcePartitionKey,
) -> Result<PathBuf, PartitionRepositoryError> {
    keyed_root(data_root, "cache/replay", key)
}

pub fn cache_instrument_root(
    data_root: &Path,
    instrument: &InstrumentId,
    trading_date: market_types::TradingDate,
) -> Result<PathBuf, PartitionRepositoryError> {
    let market = market_name(instrument);
    let symbol = safe_component(instrument.symbol().as_str())?;
    Ok(data_root
        .join("cache/replay")
        .join("teralion")
        .join(market)
        .join(trading_date.to_string())
        .join(symbol))
}

fn keyed_root(
    data_root: &Path,
    prefix: &str,
    key: &SourcePartitionKey,
) -> Result<PathBuf, PartitionRepositoryError> {
    let instrument = key.instrument();
    let market = market_name(instrument);
    let date = key.trading_date().to_string();
    let symbol = safe_component(instrument.symbol().as_str())?;
    Ok(data_root
        .join(prefix)
        .join("teralion")
        .join(market)
        .join(date)
        .join(symbol))
}

fn market_name(instrument: &InstrumentId) -> &'static str {
    match instrument.market() {
        MarketId::Twse => "twse",
        MarketId::Tpex => "tpex",
        MarketId::Taifex => "taifex",
    }
}

fn safe_component(value: &str) -> Result<&str, PartitionRepositoryError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
    {
        return Err(PartitionRepositoryError::UnsafePathComponent);
    }
    Ok(value)
}

fn source_name(source: SourceId) -> &'static str {
    match source {
        SourceId::TeralionFeedArchive => "teralion",
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PartitionRepositoryError> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
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
pub enum PartitionRepositoryError {
    Io(std::io::Error),
    Manifest(String),
    ManifestMismatch,
    UnsafePathComponent,
}

impl fmt::Display for PartitionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for PartitionRepositoryError {}

impl From<std::io::Error> for PartitionRepositoryError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use market_types::{InstrumentId, MarketId, Symbol, TradingDate};
    use run_planner::{SessionPlan, SourceId, SourcePartitionKey};
    use strategy_api::SessionKind;

    use super::*;

    fn key(symbol: &str) -> SourcePartitionKey {
        let instrument = InstrumentId::new(MarketId::Twse, Symbol::new(symbol).unwrap());
        let date: TradingDate = "2026-07-27".parse().unwrap();
        let plan = SessionPlan::for_instrument(&instrument, date, [SessionKind::Regular]).unwrap();
        SourcePartitionKey::new(
            SourceId::TeralionFeedArchive,
            instrument,
            date,
            [SessionKind::Regular],
            plan.identity(),
        )
        .unwrap()
    }

    #[test]
    fn partition_roots_are_independent_and_have_keyed_metadata() {
        let root = tempfile::tempdir().unwrap();
        let first = PartitionedSourceRepository::new(root.path(), key("2330")).unwrap();
        let second = PartitionedSourceRepository::new(root.path(), key("2317")).unwrap();
        first.ensure_layout().unwrap();
        second.ensure_layout().unwrap();
        assert_ne!(first.root(), second.root());
        assert!(first.partition_manifest_path().is_file());
        assert!(second.partition_manifest_path().is_file());
        assert_eq!(first.inspect().state(), run_planner::SourceState::Missing);
        assert_eq!(second.inspect().state(), run_planner::SourceState::Missing);

        let cache_catalog = crate::PartitionCacheCatalog::new(root.path());
        assert_ne!(
            cache_catalog.root_for(first.key()).unwrap(),
            cache_catalog.root_for(second.key()).unwrap()
        );
    }
}
