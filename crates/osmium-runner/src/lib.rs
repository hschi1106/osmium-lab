//! Runs a strategy over the trading days of a date range: reads marketdata, replays
//! it through the simulated exchange and the account, and writes the results
//! (`docs/reference/replay.md`, `config.md`, `artifacts.md`).

mod config;
mod day;
mod error;
mod output;
mod run;
mod summary;

pub use config::{ConfigError, Latency, MissingDays, RunConfig};
pub use error::RunError;
pub use output::FileSummary;
pub use run::{Plan, RunRequest, plan, run};
pub use summary::{DaySummary, RunSummary};

/// Changes whenever the files of a run change their names, columns or meaning.
pub const OUTPUT_VERSION: u32 = 1;
