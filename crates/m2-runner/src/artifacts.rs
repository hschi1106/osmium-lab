use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::CompletedBacktest;

const RUN_MANIFEST_VERSION: u16 = 1;

pub fn publish_backtest(
    output: &Path,
    completed: &CompletedBacktest,
    plan_identity: &[u8; 32],
    source_revision: &str,
    cache_identity: &str,
) -> Result<(), ArtifactError> {
    if output.exists() {
        return Err(ArtifactError::OutputExists(output.to_path_buf()));
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let name = output
        .file_name()
        .ok_or_else(|| ArtifactError::InvalidOutput(output.to_path_buf()))?
        .to_string_lossy();
    let staging = parent.join(format!(".{name}.osmium-staging"));
    if staging.exists() {
        return Err(ArtifactError::OutputExists(staging));
    }
    fs::create_dir(&staging)?;

    let strategy = completed
        .strategy_output
        .to_canonical_bytes()
        .map_err(|error| ArtifactError::Encoding(error.to_string()))?;
    let orders = encode_orders(completed)?;
    let fills = encode_fills(completed);
    let ledger = encode_ledger(completed);
    let event_checksum = hex(completed.replay.summary().event_checksum().as_bytes());
    let state_checksum = hex(completed.replay.summary().final_state_checksum().as_bytes());

    let mut files = BTreeMap::<&str, Vec<u8>>::new();
    files.insert(
        "effective-config.yaml",
        format!("config_checksum: {}\n", hex(plan_identity)).into_bytes(),
    );
    files.insert(
        "execution-plan.yaml",
        format!("plan_identity: {}\n", hex(plan_identity)).into_bytes(),
    );
    files.insert(
        "data-lineage.yaml",
        format!("source_revision: {source_revision}\n").into_bytes(),
    );
    files.insert(
        "cache-lineage.yaml",
        format!("cache_identity: {cache_identity}\n").into_bytes(),
    );
    files.insert(
        "event-stream.blake3",
        format!("{event_checksum}\n").into_bytes(),
    );
    files.insert(
        "final-state.blake3",
        format!("{state_checksum}\n").into_bytes(),
    );
    files.insert("strategy-output.bin", strategy.clone());
    files.insert(
        "strategy-output.blake3",
        format!("{}\n", hash(&strategy)).into_bytes(),
    );
    files.insert("orders.bin", orders.clone());
    files.insert("orders.blake3", format!("{}\n", hash(&orders)).into_bytes());
    files.insert("fills.bin", fills.clone());
    files.insert("fills.blake3", format!("{}\n", hash(&fills)).into_bytes());
    files.insert("ledger.bin", ledger.clone());
    files.insert("ledger.blake3", format!("{}\n", hash(&ledger)).into_bytes());
    files.insert(
        "positions.yaml",
        format!("2330: {}\n", completed.performance.position).into_bytes(),
    );
    files.insert(
        "performance.yaml",
        format!(
            "initial_cash_atoms: {}\nfinal_cash_atoms: {}\nrealized_pnl_atoms: {}\nunrealized_pnl_atoms: {}\ntotal_fee_atoms: {}\ntotal_tax_atoms: {}\n",
            completed.performance.initial_cash.atoms(),
            completed.performance.final_cash.atoms(),
            completed.performance.realized_pnl.atoms(),
            completed.performance.unrealized_pnl.map_or_else(|| "unavailable".to_owned(), |value| value.atoms().to_string()),
            completed.performance.total_fee.atoms(),
            completed.performance.total_tax.atoms(),
        ).into_bytes(),
    );
    files.insert("warnings.yaml", b"warnings: []\n".to_vec());
    files.insert(
        "run-summary.yaml",
        format!(
            "status: successful\nevents: {}\norders: {}\nfills: {}\n",
            completed.replay.summary().event_count(),
            completed.simulator.orders().len(),
            completed.simulator.fills().len()
        )
        .into_bytes(),
    );
    let checksums = files
        .iter()
        .map(|(name, bytes)| ((*name).to_owned(), hash(bytes)))
        .collect::<BTreeMap<_, _>>();
    let manifest = serde_json::json!({
        "run_manifest_version": RUN_MANIFEST_VERSION,
        "status": "successful",
        "completion_quality": "full",
        "plan_identity": hex(plan_identity),
        "source_revision": source_revision,
        "cache_identity": cache_identity,
        "event_count": completed.replay.summary().event_count(),
        "order_count": completed.simulator.orders().len(),
        "fill_count": completed.simulator.fills().len(),
        "artifact_checksums": checksums,
    });
    files.insert(
        "run-manifest.yaml",
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| ArtifactError::Encoding(error.to_string()))?,
    );
    for (name, bytes) in files {
        write_file(&staging.join(name), &bytes)?;
    }
    File::open(&staging)?.sync_all()?;
    fs::rename(&staging, output)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn inspect_run(path: &Path) -> Result<InspectSummary, ArtifactError> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("run-manifest.yaml"))?)
            .map_err(|error| ArtifactError::Encoding(error.to_string()))?;
    if manifest["run_manifest_version"].as_u64() != Some(u64::from(RUN_MANIFEST_VERSION)) {
        return Err(ArtifactError::Manifest);
    }
    let checksums = manifest["artifact_checksums"]
        .as_object()
        .ok_or(ArtifactError::Manifest)?;
    for (name, expected) in checksums {
        let bytes = fs::read(path.join(name))?;
        if expected.as_str() != Some(&hash(&bytes)) {
            return Err(ArtifactError::Checksum(name.clone()));
        }
    }
    Ok(InspectSummary {
        status: manifest["status"].as_str().unwrap_or("unknown").to_owned(),
        event_count: manifest["event_count"].as_u64().unwrap_or(0),
        order_count: manifest["order_count"].as_u64().unwrap_or(0),
        fill_count: manifest["fill_count"].as_u64().unwrap_or(0),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectSummary {
    pub status: String,
    pub event_count: u64,
    pub order_count: u64,
    pub fill_count: u64,
}

fn encode_orders(completed: &CompletedBacktest) -> Result<Vec<u8>, ArtifactError> {
    let mut bytes = b"OSORDERS1".to_vec();
    bytes.extend_from_slice(&(completed.simulator.orders().len() as u64).to_be_bytes());
    for order in completed.simulator.orders() {
        bytes.extend_from_slice(order.id().as_bytes());
        let intent = order
            .intent()
            .to_canonical_bytes()
            .map_err(|error| ArtifactError::Encoding(error.to_string()))?;
        bytes.extend_from_slice(&(intent.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&intent);
        bytes.extend_from_slice(&order.filled().to_be_bytes());
        bytes.extend_from_slice(&order.remaining().to_be_bytes());
        bytes.push(order.status() as u8);
    }
    Ok(bytes)
}

fn encode_fills(completed: &CompletedBacktest) -> Vec<u8> {
    let mut bytes = b"OSFILLS1".to_vec();
    bytes.extend_from_slice(&(completed.simulator.fills().len() as u64).to_be_bytes());
    for fill in completed.simulator.fills() {
        bytes.extend_from_slice(fill.order_id().as_bytes());
        bytes.extend_from_slice(&fill.triggering_ordinal().to_be_bytes());
        bytes.extend_from_slice(&fill.match_time().as_unix_microseconds().to_be_bytes());
        bytes.push(fill.side() as u8);
        bytes.extend_from_slice(&fill.price().to_canonical_bytes());
        bytes.extend_from_slice(&fill.quantity().to_canonical_bytes());
    }
    bytes
}

fn encode_ledger(completed: &CompletedBacktest) -> Vec<u8> {
    let mut bytes = b"OSLEDGER1".to_vec();
    bytes.extend_from_slice(&completed.ledger.cash().to_canonical_bytes());
    bytes.extend_from_slice(&completed.ledger.position().to_be_bytes());
    bytes.extend_from_slice(&completed.ledger.realized_pnl().to_canonical_bytes());
    bytes
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), io::Error> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
fn hash(bytes: &[u8]) -> String {
    hex(blake3::hash(bytes).as_bytes())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug)]
pub enum ArtifactError {
    Io(io::Error),
    OutputExists(PathBuf),
    InvalidOutput(PathBuf),
    Encoding(String),
    Manifest,
    Checksum(String),
}
impl fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl Error for ArtifactError {}
impl From<io::Error> for ArtifactError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
