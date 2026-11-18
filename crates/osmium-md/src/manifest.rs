use arrow_array::types::Int64Type;
use osmium_types::Date;

use crate::{
    MdError, MdRoot, Table,
    parquet_io::{
        Column, ColumnType, ParquetFile, boolean, optional, optional_bool, optional_str, primitive,
        required, strings,
    },
};

/// The marketdata schema version osmium reads (`md.schema_version`, path `/srv/md/v1`).
pub const SCHEMA_VERSION: &str = "1";

const COLUMNS: &[Column] = &[
    ("job", ColumnType::String),
    ("table", ColumnType::String),
    ("part", ColumnType::String),
    ("path", ColumnType::String),
    ("rows", ColumnType::Int64),
    ("partial", ColumnType::Bool),
    ("seq_gaps", ColumnType::Int64),
    ("seq_dup", ColumnType::Int64),
    ("schema_version", ColumnType::String),
    ("git_commit", ColumnType::String),
    ("code_hash", ColumnType::String),
    ("inputs_hash", ColumnType::String),
];

/// One manifest row: the identity and quality summary of one published file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManifestEntry {
    pub job: String,
    pub table: String,
    pub part: String,
    /// Path relative to the marketdata root.
    pub path: String,
    pub rows: i64,
    /// The source was incomplete: some fields are null or some input was skipped.
    pub partial: bool,
    /// Number of missing sequence numbers in the feed that day.
    pub seq_gaps: Option<i64>,
    pub seq_dup: Option<i64>,
    pub schema_version: String,
    pub git_commit: String,
    pub code_hash: String,
    pub inputs_hash: String,
}

/// Every manifest row for one trading date, across all jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayManifest {
    date: Date,
    entries: Vec<ManifestEntry>,
}

impl DayManifest {
    /// Loads `manifest/{YYYY}/{date}/*.parquet`. A missing directory means no data that day.
    pub fn load(root: &MdRoot, date: Date) -> Result<Self, MdError> {
        let dir = root.manifest_dir(date);
        let listing = match std::fs::read_dir(&dir) {
            Ok(listing) => listing,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(MdError::MissingManifest { date });
            }
            Err(source) => return Err(MdError::Io { path: dir, source }),
        };
        let mut files = Vec::new();
        for item in listing {
            let item = item.map_err(|source| MdError::Io {
                path: dir.clone(),
                source,
            })?;
            let path = item.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "parquet")
            {
                files.push(path);
            }
        }
        if files.is_empty() {
            return Err(MdError::MissingManifest { date });
        }
        files.sort();

        let mut entries = Vec::new();
        for path in files {
            ParquetFile::open(&path)?.read(COLUMNS, None, |path, batch| {
                let job = strings(path, batch, "job")?;
                let table = strings(path, batch, "table")?;
                let part = strings(path, batch, "part")?;
                let file = strings(path, batch, "path")?;
                let rows = primitive::<Int64Type>(path, batch, "rows")?;
                let partial = boolean(path, batch, "partial")?;
                let seq_gaps = primitive::<Int64Type>(path, batch, "seq_gaps")?;
                let seq_dup = primitive::<Int64Type>(path, batch, "seq_dup")?;
                let schema_version = strings(path, batch, "schema_version")?;
                let git_commit = strings(path, batch, "git_commit")?;
                let code_hash = strings(path, batch, "code_hash")?;
                let inputs_hash = strings(path, batch, "inputs_hash")?;
                for row in 0..batch.num_rows() {
                    let text = |column, name| {
                        required(path, optional_str(column, row), name).map(str::to_owned)
                    };
                    entries.push(ManifestEntry {
                        job: text(job, "job")?,
                        table: text(table, "table")?,
                        part: text(part, "part")?,
                        path: text(file, "path")?,
                        rows: required(path, optional(rows, row), "rows")?,
                        partial: required(path, optional_bool(partial, row), "partial")?,
                        seq_gaps: optional(seq_gaps, row),
                        seq_dup: optional(seq_dup, row),
                        schema_version: text(schema_version, "schema_version")?,
                        git_commit: text(git_commit, "git_commit")?,
                        code_hash: text(code_hash, "code_hash")?,
                        inputs_hash: text(inputs_hash, "inputs_hash")?,
                    });
                }
                Ok(())
            })?;
        }
        Ok(Self { date, entries })
    }

    #[must_use]
    pub const fn date(&self) -> Date {
        self.date
    }

    #[must_use]
    pub fn entries(&self) -> &[ManifestEntry] {
        &self.entries
    }

    /// The manifest row for `table`/`part`; its absence means the data does not exist.
    pub fn entry(&self, table: Table, part: &str) -> Result<&ManifestEntry, MdError> {
        self.entries
            .iter()
            .find(|entry| entry.table == table.as_str() && entry.part == part)
            .ok_or_else(|| MdError::MissingManifestEntry {
                date: self.date,
                table: table.as_str(),
                part: part.to_owned(),
            })
    }

    /// Opens the file of `table`/`part` after checking it against its manifest row.
    pub(crate) fn open(
        &self,
        root: &MdRoot,
        table: Table,
        part: &str,
    ) -> Result<(ParquetFile, ManifestEntry), MdError> {
        let entry = self.entry(table, part)?;
        let expected = MdRoot::relative_file(table, self.date, part);
        let path = root.file(table, self.date, part);
        if entry.path != expected {
            return Err(MdError::inconsistent(
                &path,
                format!("manifest path {} != {expected}", entry.path),
            ));
        }
        if entry.schema_version != SCHEMA_VERSION {
            return Err(MdError::schema(
                &path,
                format!(
                    "md.schema_version {} is not the supported version {SCHEMA_VERSION}",
                    entry.schema_version
                ),
            ));
        }
        let file = ParquetFile::open(&path)?;
        if file.num_rows() != entry.rows {
            return Err(MdError::inconsistent(
                file.path(),
                format!("{} rows, manifest says {}", file.num_rows(), entry.rows),
            ));
        }
        for (key, expected) in [
            ("md.schema_version", entry.schema_version.as_str()),
            ("md.git_commit", entry.git_commit.as_str()),
        ] {
            let actual = file.metadata_value(key);
            if actual != Some(expected) {
                return Err(MdError::inconsistent(
                    file.path(),
                    format!("{key} {actual:?} != manifest {expected}"),
                ));
            }
        }
        Ok((file, entry.clone()))
    }

    /// Re-reads the manifest and fails when the row a read relied on has changed.
    pub(crate) fn verify_unchanged(
        &self,
        root: &MdRoot,
        table: Table,
        part: &str,
        entry: &ManifestEntry,
    ) -> Result<(), MdError> {
        let changed = || MdError::ManifestChanged {
            date: self.date,
            table: table.as_str(),
            part: part.to_owned(),
        };
        let current = match Self::load(root, self.date) {
            Ok(current) => current,
            Err(MdError::MissingManifest { .. }) => return Err(changed()),
            Err(error) => return Err(error),
        };
        match current.entry(table, part) {
            Ok(now) if now == entry => Ok(()),
            _ => Err(changed()),
        }
    }
}
