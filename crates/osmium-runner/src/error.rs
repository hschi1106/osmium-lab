use std::{error::Error, fmt, path::PathBuf};

use osmium_ledger::LedgerError;
use osmium_md::MdError;
use osmium_types::Date;

use crate::ConfigError;

/// Why a run stopped. A run never continues past something it cannot account for.
#[derive(Debug)]
pub enum RunError {
    Config(ConfigError),
    Md(MdError),
    /// The accounting of `date` cannot be done (`docs/reference/accounting.md`).
    Ledger {
        date: Date,
        source: LedgerError,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The output directory exists and is not empty.
    OutputExists {
        path: PathBuf,
    },
    /// The date range holds no trading day of the calendar.
    NoTradingDay {
        start: Date,
        end: Date,
    },
    /// The data is outside what the specifications define.
    Unsupported {
        date: Date,
        reason: String,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(formatter, "configuration: {error}"),
            Self::Md(error) => write!(formatter, "marketdata: {error}"),
            Self::Ledger { date, source } => write!(formatter, "accounting on {date}: {source}"),
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::OutputExists { path } => write!(
                formatter,
                "the output directory {} is not empty",
                path.display()
            ),
            Self::NoTradingDay { start, end } => {
                write!(formatter, "no trading day from {start} to {end}")
            }
            Self::Unsupported { date, reason } => write!(formatter, "{date}: {reason}"),
        }
    }
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::Md(error) => Some(error),
            Self::Ledger { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::OutputExists { .. } | Self::NoTradingDay { .. } | Self::Unsupported { .. } => {
                None
            }
        }
    }
}

impl From<MdError> for RunError {
    fn from(error: MdError) -> Self {
        Self::Md(error)
    }
}

impl From<ConfigError> for RunError {
    fn from(error: ConfigError) -> Self {
        Self::Config(error)
    }
}
