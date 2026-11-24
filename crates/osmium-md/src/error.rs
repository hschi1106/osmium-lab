use std::{error::Error, fmt, path::PathBuf};

use osmium_types::{Date, Feed, Symbol};

/// Errors from reading marketdata. Every variant names the file, day or row involved.
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
        date: Date,
    },
    /// The day's manifest has no row for this table part: the data does not exist.
    MissingManifestEntry {
        date: Date,
        table: &'static str,
        part: String,
    },
    /// The file is not marketdata schema version 1, or a column osmium reads is missing
    /// or has a different type.
    Schema {
        path: PathBuf,
        reason: String,
    },
    /// A file's contents disagree with its manifest row or with the documented layout.
    Inconsistent {
        path: PathBuf,
        reason: String,
    },
    /// The manifest changed while files were being read (a marketdata update raced the read).
    ManifestChanged {
        date: Date,
        table: &'static str,
        part: String,
    },
    /// A reference file or its `fm/_build` row changed while it was being read, or the
    /// file is not the one `fm/_build` records (a marketdata rebuild is in progress).
    ReferenceChanged {
        path: String,
    },
    /// A row holds a value or a combination of fields the specification does not define.
    InvalidRow {
        path: PathBuf,
        symbol: String,
        seq: u64,
        reason: String,
    },
    /// A regular-board symbol has no `shares_per_lot`, so its lots cannot become shares.
    MissingLotSize {
        date: Date,
        feed: Feed,
        symbol: Symbol,
    },
}

impl MdError {
    pub(crate) fn parquet(path: &std::path::Path, error: impl fmt::Display) -> Self {
        Self::Parquet {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }

    pub(crate) fn schema(path: &std::path::Path, reason: impl Into<String>) -> Self {
        Self::Schema {
            path: path.to_path_buf(),
            reason: reason.into(),
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
            Self::Schema { path, reason } => {
                write!(formatter, "{}: schema: {reason}", path.display())
            }
            Self::Inconsistent { path, reason } => {
                write!(formatter, "{}: {reason}", path.display())
            }
            Self::ManifestChanged { date, table, part } => write!(
                formatter,
                "marketdata manifest for {table}/{part} on {date} changed during the read"
            ),
            Self::ReferenceChanged { path } => write!(
                formatter,
                "marketdata {path} does not match fm/_build or changed during the read; \
                 marketdata may be rebuilding it, read again later"
            ),
            Self::InvalidRow {
                path,
                symbol,
                seq,
                reason,
            } => write!(
                formatter,
                "{}: symbol {symbol} seq {seq}: {reason}",
                path.display()
            ),
            Self::MissingLotSize { date, feed, symbol } => write!(
                formatter,
                "{feed} {symbol} on {date}: instruments has no shares_per_lot"
            ),
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
