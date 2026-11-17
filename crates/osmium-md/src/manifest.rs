use arrow_array::types::Int64Type;
use market_types::TradingDate;

use crate::{
    MdError, MdRoot, Table,
    parquet_io::{ParquetFile, boolean, optional, optional_bool, primitive, required_str, strings},
};

/// One manifest row: the identity and quality summary of one published file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManifestEntry {
    pub job: String,
    pub table: String,
    pub part: String,
    /// Path relative to the marketdata root.
    pub path: String,
    pub rows: i64,
    pub partial: bool,
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
    date: TradingDate,
    entries: Vec<ManifestEntry>,
}

impl DayManifest {
    /// Loads `manifest/{YYYY}/{date}/*.parquet`. A missing directory means no data that day.
    pub fn load(root: &MdRoot, date: TradingDate) -> Result<Self, MdError> {
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
            ParquetFile::open(&path)?.read(
                &[
                    "job",
                    "table",
                    "part",
                    "path",
                    "rows",
                    "partial",
                    "seq_gaps",
                    "seq_dup",
                    "schema_version",
                    "git_commit",
                    "code_hash",
                    "inputs_hash",
                ],
                None,
                |path, batch| {
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
                        let text =
                            |column, name| required_str(path, column, name, row).map(str::to_owned);
                        entries.push(ManifestEntry {
                            job: text(&job, "job")?,
                            table: text(&table, "table")?,
                            part: text(&part, "part")?,
                            path: text(&file, "path")?,
                            rows: optional(rows, row).ok_or_else(|| {
                                MdError::inconsistent(path, "column rows has a null")
                            })?,
                            partial: optional_bool(partial, row).ok_or_else(|| {
                                MdError::inconsistent(path, "column partial has a null")
                            })?,
                            seq_gaps: optional(seq_gaps, row),
                            seq_dup: optional(seq_dup, row),
                            schema_version: text(&schema_version, "schema_version")?,
                            git_commit: text(&git_commit, "git_commit")?,
                            code_hash: text(&code_hash, "code_hash")?,
                            inputs_hash: text(&inputs_hash, "inputs_hash")?,
                        });
                    }
                    Ok(())
                },
            )?;
        }
        Ok(Self { date, entries })
    }

    #[must_use]
    pub const fn date(&self) -> TradingDate {
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
}

/// Opens the file a manifest row describes and checks the file matches the row.
pub(crate) fn open_verified(
    root: &MdRoot,
    date: TradingDate,
    table: Table,
    part: &str,
    entry: &ManifestEntry,
) -> Result<ParquetFile, MdError> {
    let expected = MdRoot::relative_file(table, date, part);
    let path = root.file(table, date, part);
    if entry.path != expected {
        return Err(MdError::inconsistent(
            &path,
            format!("manifest path {} != {expected}", entry.path),
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
    Ok(file)
}
