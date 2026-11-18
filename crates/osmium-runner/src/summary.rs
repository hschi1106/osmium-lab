//! `run.json`: what a run was made of and what it produced
//! (`docs/reference/artifacts.md` section 2).

use std::path::PathBuf;

use osmium_md::ReferenceFile;
use osmium_types::{Date, Money};
use serde::Serialize;

use crate::{
    OUTPUT_VERSION, RunConfig, RunError,
    output::{FileSummary, Output},
};

/// One trading day of a run, as it is reported while the run goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaySummary {
    pub date: Date,
    /// Market messages replayed.
    pub messages: u64,
    /// Orders sent that day, forced ones included.
    pub orders: u64,
    pub fills: u64,
    /// Settled cash after the close.
    pub cash: Money,
    pub equity: Money,
}

/// What a finished run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub output: PathBuf,
    pub trading_days: u64,
    pub messages: u64,
    pub orders: u64,
    pub fills: u64,
    pub initial_cash: Money,
    pub final_cash: Money,
    pub final_equity: Money,
    /// Every file of the output except `run.json`, with its digest.
    pub files: Vec<FileSummary>,
}

/// A marketdata file a day was read from, as its manifest row describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FileIdentity {
    pub(crate) table: String,
    pub(crate) part: String,
    pub(crate) path: String,
    pub(crate) rows: i64,
    pub(crate) git_commit: String,
    pub(crate) code_hash: String,
    pub(crate) inputs_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DayFiles {
    pub(crate) date: Date,
    pub(crate) files: Vec<FileIdentity>,
}

#[derive(Serialize)]
struct Versions {
    osmium: &'static str,
    md_schema: &'static str,
    decoder: u32,
    ordering: u32,
    matching_rules: u32,
    ledger_rules: u32,
}

#[derive(Serialize)]
struct StrategyRecord<'a> {
    name: &'a str,
    version: &'a str,
}

#[derive(Serialize)]
struct ReferenceRecord {
    path: String,
    rows: u64,
    git_commit: String,
}

#[derive(Serialize)]
struct DayRecord {
    date: String,
    messages: u64,
    orders: u64,
    fills: u64,
    cash: i64,
    equity: i64,
    files: Vec<FileIdentity>,
}

#[derive(Serialize)]
struct Totals {
    trading_days: u64,
    messages: u64,
    orders: u64,
    fills: u64,
    initial_cash: i64,
    final_cash: i64,
    final_equity: i64,
}

#[derive(Serialize)]
struct Record<'a> {
    output_version: u32,
    versions: Versions,
    strategy: StrategyRecord<'a>,
    config_blake3: String,
    md_root: String,
    start: String,
    end: String,
    reference: Vec<ReferenceRecord>,
    days: Vec<DayRecord>,
    summary: Totals,
    files: &'a [FileSummary],
}

/// Closes the tables, copies the configuration and writes `run.json` last, so that a
/// directory with a `run.json` is a complete result.
pub(crate) fn write_summary(
    output: Output,
    config: &RunConfig,
    config_text: &str,
    strategy_version: &str,
    reference: Vec<ReferenceFile>,
    files: Vec<DayFiles>,
    days: &[DaySummary],
) -> Result<RunSummary, RunError> {
    let config_file = output.file("config.toml", config_text.as_bytes())?;
    let config_blake3 = config_file.blake3.clone();
    let (dir, mut written) = output.finish()?;
    written.push(config_file);

    let last = days.last().expect("a run has at least one trading day");
    let total = |of: fn(&DaySummary) -> u64| days.iter().map(of).sum::<u64>();
    let summary = RunSummary {
        output: dir.clone(),
        trading_days: days.len() as u64,
        messages: total(|day| day.messages),
        orders: total(|day| day.orders),
        fills: total(|day| day.fills),
        initial_cash: config.ledger.account.initial_cash,
        final_cash: last.cash,
        final_equity: last.equity,
        files: written,
    };
    let record = Record {
        output_version: OUTPUT_VERSION,
        versions: Versions {
            osmium: env!("CARGO_PKG_VERSION"),
            md_schema: osmium_md::SCHEMA_VERSION,
            decoder: osmium_md::DECODER_VERSION,
            ordering: osmium_replay::ORDERING_VERSION,
            matching_rules: osmium_matching::RULES_VERSION,
            ledger_rules: osmium_ledger::RULES_VERSION,
        },
        strategy: StrategyRecord {
            name: &config.strategy,
            version: strategy_version,
        },
        config_blake3,
        md_root: config.md_root.display().to_string(),
        start: config.start.to_string(),
        end: config.end.to_string(),
        reference: reference
            .into_iter()
            .map(|file| ReferenceRecord {
                path: file.path,
                rows: file.rows,
                git_commit: file.git_commit,
            })
            .collect(),
        days: days
            .iter()
            .zip(files)
            .map(|(day, files)| {
                debug_assert_eq!(day.date, files.date);
                DayRecord {
                    date: day.date.to_string(),
                    messages: day.messages,
                    orders: day.orders,
                    fills: day.fills,
                    cash: day.cash.get(),
                    equity: day.equity.get(),
                    files: files.files,
                }
            })
            .collect(),
        summary: Totals {
            trading_days: summary.trading_days,
            messages: summary.messages,
            orders: summary.orders,
            fills: summary.fills,
            initial_cash: summary.initial_cash.get(),
            final_cash: summary.final_cash.get(),
            final_equity: summary.final_equity.get(),
        },
        files: &summary.files,
    };
    let mut text = serde_json::to_string_pretty(&record).expect("the record is plain data");
    text.push('\n');
    let path = dir.join("run.json");
    std::fs::write(&path, text).map_err(|source| RunError::Io { path, source })?;
    Ok(summary)
}
