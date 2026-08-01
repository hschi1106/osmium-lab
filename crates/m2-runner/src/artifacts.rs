use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use execution_sim::ACCOUNTING_VERSION;

use crate::CompletedBacktest;
use crate::CompletedMultiBacktest;

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
    files.insert(
        "replay-summary.json",
        serde_json::to_vec_pretty(&serde_json::json!({
            "format": "osmium-m3-replay-summary-v1",
            "status": "strategy_simulated",
            "event_count": completed.replay.summary().event_count(),
            "event_checksum": event_checksum,
            "final_state_checksum": state_checksum,
        }))
        .map_err(|error| ArtifactError::Encoding(error.to_string()))?,
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

pub fn publish_multi_backtest(
    output: &Path,
    completed: &CompletedMultiBacktest,
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
    let orders = encode_multi_orders(completed)?;
    let fills = encode_multi_fills(completed);
    let ledger = encode_multi_ledger(completed);
    let ledger_checksum = hash(&ledger);
    let positions = encode_multi_positions(completed, &ledger_checksum);
    let performance = encode_multi_performance(completed, &ledger_checksum);
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
    files.insert("ledger.blake3", format!("{ledger_checksum}\n").into_bytes());
    files.insert("positions.yaml", positions);
    files.insert("performance.yaml", performance);
    files.insert(
        "replay-summary.json",
        serde_json::to_vec_pretty(&serde_json::json!({
            "format": "osmium-m3-replay-summary-v1",
            "status": "successful",
            "event_count": completed.replay.summary().event_count(),
            "event_checksum": event_checksum,
            "final_state_checksum": state_checksum,
            "accounting_version": ACCOUNTING_VERSION,
        }))
        .map_err(|error| ArtifactError::Encoding(error.to_string()))?,
    );
    files.insert(
        "run-summary.yaml",
        format!(
            "status: successful\ncompletion_quality: full\nevents: {}\norders: {}\nfills: {}\naccounting_version: {}\nfinal_cash_atoms: {}\nrealized_pnl_atoms: {}\nunrealized_pnl_atoms: {}\n",
            completed.replay.summary().event_count(),
            completed.simulator.order_count(),
            completed.simulator.fill_count(),
            ACCOUNTING_VERSION,
            completed.performance.final_cash().atoms(),
            completed.performance.realized_pnl().atoms(),
            completed.performance.unrealized_pnl().atoms(),
        )
        .into_bytes(),
    );
    files.insert("warnings.yaml", b"warnings: []\n".to_vec());
    let checksums = files
        .iter()
        .map(|(name, bytes)| ((*name).to_owned(), hash(bytes)))
        .collect::<BTreeMap<_, _>>();
    let manifest = serde_json::json!({
        "run_manifest_version": RUN_MANIFEST_VERSION,
        "status": "successful",
        "completion_quality": "full",
        "accounting_version": ACCOUNTING_VERSION,
        "plan_identity": hex(plan_identity),
        "source_revision": source_revision,
        "cache_identity": cache_identity,
        "event_count": completed.replay.summary().event_count(),
        "order_count": completed.simulator.order_count(),
        "fill_count": completed.simulator.fill_count(),
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

fn encode_multi_orders(completed: &CompletedMultiBacktest) -> Result<Vec<u8>, ArtifactError> {
    let orders = completed.simulator.orders();
    let mut bytes = b"OSORDERS1".to_vec();
    bytes.extend_from_slice(&(orders.len() as u64).to_be_bytes());
    for order in orders {
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

fn encode_multi_fills(completed: &CompletedMultiBacktest) -> Vec<u8> {
    let fill_count = completed
        .performance
        .instruments()
        .iter()
        .map(|instrument| {
            completed
                .simulator
                .fills_for(instrument.instrument())
                .map_or(0, <[execution_sim::FillRecord]>::len)
        })
        .sum::<usize>();
    let mut bytes = b"OSFILLS1".to_vec();
    bytes.extend_from_slice(&(fill_count as u64).to_be_bytes());
    for instrument in completed.performance.instruments() {
        let Some(fills) = completed.simulator.fills_for(instrument.instrument()) else {
            continue;
        };
        for fill in fills {
            append_instrument_identity(&mut bytes, instrument.instrument());
            bytes.extend_from_slice(fill.order_id().as_bytes());
            bytes.extend_from_slice(&fill.triggering_ordinal().to_be_bytes());
            bytes.extend_from_slice(&fill.match_time().as_unix_microseconds().to_be_bytes());
            bytes.push(fill.side() as u8);
            bytes.extend_from_slice(&fill.price().to_canonical_bytes());
            bytes.extend_from_slice(&fill.quantity().to_canonical_bytes());
        }
    }
    bytes
}

fn encode_multi_ledger(completed: &CompletedMultiBacktest) -> Vec<u8> {
    let mut bytes = b"OSLEDGR1".to_vec();
    bytes.extend_from_slice(&ACCOUNTING_VERSION.to_be_bytes());
    bytes.extend_from_slice(&(completed.performance.instruments().len() as u64).to_be_bytes());
    for instrument in completed.performance.instruments() {
        let mut record = Vec::new();
        append_instrument_identity(&mut record, instrument.instrument());
        record.push(instrument.quantity_unit() as u8);
        record.push(instrument.accounting_model() as u8);
        let economics = instrument.economics();
        record.extend_from_slice(&economics.units_per_trading_unit.to_be_bytes());
        record.extend_from_slice(&economics.multiplier.to_canonical_bytes());
        append_text(&mut record, &economics.provenance);
        append_performance_summary(&mut record, instrument.summary(), instrument.average_cost());
        let fills = completed
            .ledger
            .ledger(instrument.instrument())
            .map_or(&[][..], execution_sim::Ledger::fills);
        record.extend_from_slice(&(fills.len() as u64).to_be_bytes());
        for fill in fills {
            record.extend_from_slice(fill.order_id().as_bytes());
            record.extend_from_slice(&fill.triggering_ordinal().to_be_bytes());
            record.extend_from_slice(&fill.match_time().as_unix_microseconds().to_be_bytes());
            record.push(fill.side() as u8);
            record.extend_from_slice(&fill.price().to_canonical_bytes());
            record.extend_from_slice(&fill.quantity().to_canonical_bytes());
        }
        bytes.extend_from_slice(&(record.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&record);
    }
    bytes
}

fn encode_multi_positions(completed: &CompletedMultiBacktest, ledger_checksum: &str) -> Vec<u8> {
    let mut output = format!(
        "schema_version: 1\naccounting_version: {ACCOUNTING_VERSION}\nledger_checksum: {ledger_checksum}\ninstruments:\n"
    );
    for instrument in completed.performance.instruments() {
        let summary = instrument.summary();
        output.push_str(&format!(
            "  - instrument: {}\n    model: {}\n    quantity_unit: {}\n    units_per_trading_unit: {}\n    multiplier_atoms: {}\n    multiplier_provenance: {}\n    position: {}\n    average_cost_atoms: {}\n    final_cash_atoms: {}\n",
            yaml_scalar(&instrument_label(instrument.instrument())),
            accounting_model_name(instrument.accounting_model()),
            quantity_unit_name(instrument.quantity_unit()),
            instrument.economics().units_per_trading_unit,
            instrument.economics().multiplier.atoms(),
            yaml_scalar(&instrument.economics().provenance),
            summary.position,
            optional_atoms(instrument.average_cost()),
            summary.final_cash.atoms(),
        ));
    }
    output.into_bytes()
}

fn encode_multi_performance(completed: &CompletedMultiBacktest, ledger_checksum: &str) -> Vec<u8> {
    let mut output = format!(
        "schema_version: 1\naccounting_version: {ACCOUNTING_VERSION}\nledger_checksum: {ledger_checksum}\ninitial_cash_atoms: {}\nfinal_cash_atoms: {}\nrealized_pnl_atoms: {}\nunrealized_pnl_atoms: {}\ntotal_fee_atoms: {}\ntotal_tax_atoms: {}\nfill_count: {}\ninstruments:\n",
        completed.performance.initial_cash().atoms(),
        completed.performance.final_cash().atoms(),
        completed.performance.realized_pnl().atoms(),
        completed.performance.unrealized_pnl().atoms(),
        completed.performance.total_fee().atoms(),
        completed.performance.total_tax().atoms(),
        completed.performance.fill_count(),
    );
    for instrument in completed.performance.instruments() {
        let summary = instrument.summary();
        output.push_str(&format!(
            "  - instrument: {}\n    model: {}\n    realized_pnl_atoms: {}\n    unrealized_pnl_atoms: {}\n    total_fee_atoms: {}\n    total_tax_atoms: {}\n    fill_count: {}\n",
            yaml_scalar(&instrument_label(instrument.instrument())),
            accounting_model_name(instrument.accounting_model()),
            summary.realized_pnl.atoms(),
            optional_atoms(summary.unrealized_pnl),
            summary.total_fee.atoms(),
            summary.total_tax.atoms(),
            summary.fill_count,
        ));
    }
    output.into_bytes()
}

fn append_instrument_identity(bytes: &mut Vec<u8>, instrument: &market_types::InstrumentId) {
    bytes.push(instrument.market().discriminant());
    append_text(bytes, instrument.symbol().as_str());
}

fn append_text(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn append_performance_summary(
    bytes: &mut Vec<u8>,
    summary: execution_sim::PerformanceSummary,
    average_cost: Option<market_types::Decimal>,
) {
    bytes.extend_from_slice(&summary.initial_cash.to_canonical_bytes());
    bytes.extend_from_slice(&summary.final_cash.to_canonical_bytes());
    bytes.extend_from_slice(&summary.position.to_be_bytes());
    append_optional_decimal(bytes, average_cost);
    bytes.extend_from_slice(&summary.realized_pnl.to_canonical_bytes());
    append_optional_decimal(bytes, summary.unrealized_pnl);
    bytes.extend_from_slice(&summary.total_fee.to_canonical_bytes());
    bytes.extend_from_slice(&summary.total_tax.to_canonical_bytes());
    bytes.extend_from_slice(&summary.fill_count.to_be_bytes());
}

fn append_optional_decimal(bytes: &mut Vec<u8>, value: Option<market_types::Decimal>) {
    match value {
        Some(value) => {
            bytes.push(1);
            bytes.extend_from_slice(&value.to_canonical_bytes());
        }
        None => bytes.push(0),
    }
}

fn optional_atoms(value: Option<market_types::Decimal>) -> String {
    value.map_or_else(
        || "unavailable".to_owned(),
        |value| value.atoms().to_string(),
    )
}

fn yaml_scalar(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn instrument_label(instrument: &market_types::InstrumentId) -> String {
    format!("{:?}:{}", instrument.market(), instrument.symbol())
}

fn accounting_model_name(model: execution_sim::AccountingModel) -> &'static str {
    match model {
        execution_sim::AccountingModel::EquityV1 => "equity_v1",
        execution_sim::AccountingModel::FuturesV1 => "futures_v1",
        execution_sim::AccountingModel::OptionsV1 => "options_v1",
    }
}

fn quantity_unit_name(unit: market_types::QuantityUnit) -> &'static str {
    match unit {
        market_types::QuantityUnit::SourceUnit => "source_unit",
        market_types::QuantityUnit::Share => "share",
        market_types::QuantityUnit::TradingUnit => "trading_unit",
        market_types::QuantityUnit::Contract => "contract",
    }
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
