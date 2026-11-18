use std::path::{Path, PathBuf};

use osmium_types::Date;

/// Default marketdata root on the vault host.
pub const DEFAULT_MD_ROOT: &str = "/srv/md/v1";

/// A marketdata table stored as `{table}/{YYYY}/{YYYY-MM-DD}/{part}.parquet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Table {
    Books,
    Trades,
    Instruments,
}

impl Table {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Books => "books",
            Self::Trades => "trades",
            Self::Instruments => "instruments",
        }
    }
}

/// Root directory of a marketdata `v1` tree. Read-only: osmium never writes here.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MdRoot(PathBuf);

impl MdRoot {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Path relative to the root, as recorded in the manifest `path` column.
    #[must_use]
    pub fn relative_file(table: Table, date: Date, part: &str) -> String {
        let day = date.to_string();
        format!("{}/{}/{day}/{part}.parquet", table.as_str(), &day[..4])
    }

    #[must_use]
    pub fn file(&self, table: Table, date: Date, part: &str) -> PathBuf {
        self.0.join(Self::relative_file(table, date, part))
    }

    #[must_use]
    pub fn manifest_dir(&self, date: Date) -> PathBuf {
        let day = date.to_string();
        self.0.join("manifest").join(&day[..4]).join(day)
    }
}

impl Default for MdRoot {
    fn default() -> Self {
        Self::new(DEFAULT_MD_ROOT)
    }
}

#[cfg(test)]
mod tests {
    use super::{MdRoot, Table};
    use osmium_types::Date;

    #[test]
    fn paths_follow_the_marketdata_layout() {
        let root = MdRoot::new("/srv/md/v1");
        let date: Date = "2026-10-15".parse().unwrap();
        assert_eq!(
            root.file(Table::Books, date, "TWSE_F6"),
            std::path::Path::new("/srv/md/v1/books/2026/2026-10-15/TWSE_F6.parquet")
        );
        assert_eq!(
            MdRoot::relative_file(Table::Trades, date, "TPEX_F23"),
            "trades/2026/2026-10-15/TPEX_F23.parquet"
        );
        assert_eq!(
            root.manifest_dir(date),
            std::path::Path::new("/srv/md/v1/manifest/2026/2026-10-15")
        );
    }
}
