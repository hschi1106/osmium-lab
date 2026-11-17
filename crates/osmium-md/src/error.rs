use std::{error::Error, fmt, path::PathBuf};

use market_types::{Symbol, TradingDate};

/// Errors from reading or mapping marketdata. Every variant names the file or row involved.
#[derive(Debug)]
pub enum MdError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parquet {
        path: PathBuf,
        message: String,
    },
    /// No manifest exists for the day: marketdata has no data for it.
    MissingManifest {
        date: TradingDate,
    },
    /// The day's manifest has no row for this table part: the data does not exist.
    MissingManifestEntry {
        date: TradingDate,
        table: &'static str,
        part: String,
    },
    /// A file's contents disagree with its manifest row or with the expected schema.
    Inconsistent {
        path: PathBuf,
        reason: String,
    },
    /// The manifest changed while files were being read (a marketdata update raced the read).
    ManifestChanged {
        date: TradingDate,
        table: &'static str,
        part: String,
    },
    /// A price is not the nearest double to a 1e-4 decimal, or is not positive.
    NonCanonicalPrice {
        path: PathBuf,
        seq: u64,
        value: f64,
    },
    /// A row violates a mapping invariant.
    InvalidRow {
        symbol: Symbol,
        seq: u64,
        reason: &'static str,
    },
}

impl MdError {
    pub(crate) fn parquet(path: &std::path::Path, error: impl fmt::Display) -> Self {
        Self::Parquet {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }

    pub(crate) fn inconsistent(path: &std::path::Path, reason: impl Into<String>) -> Self {
        Self::Inconsistent {
            path: path.to_path_buf(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for MdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Parquet { path, message } => {
                write!(formatter, "{}: parquet: {message}", path.display())
            }
            Self::MissingManifest { date } => {
                write!(formatter, "marketdata has no manifest for {date}")
            }
            Self::MissingManifestEntry { date, table, part } => {
                write!(formatter, "marketdata has no {table}/{part} for {date}")
            }
            Self::Inconsistent { path, reason } => {
                write!(formatter, "{}: {reason}", path.display())
            }
            Self::ManifestChanged { date, table, part } => write!(
                formatter,
                "marketdata manifest for {table}/{part} on {date} changed during the read"
            ),
            Self::NonCanonicalPrice { path, seq, value } => write!(
                formatter,
                "{}: seq {seq}: price {value} is not an exact positive 1e-4 decimal",
                path.display()
            ),
            Self::InvalidRow {
                symbol,
                seq,
                reason,
            } => write!(formatter, "{symbol} seq {seq}: {reason}"),
        }
    }
}

impl Error for MdError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
