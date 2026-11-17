use std::{
    fs::File,
    path::{Path, PathBuf},
};

use arrow_array::{
    Array, ArrowPrimitiveType, BooleanArray, PrimitiveArray, RecordBatch, cast::AsArray,
};
use parquet::arrow::{ProjectionMask, arrow_reader::ParquetRecordBatchReaderBuilder};

use crate::MdError;

const BATCH_SIZE: usize = 64 * 1024;

/// One opened marketdata parquet file with its footer already parsed.
pub(crate) struct ParquetFile {
    path: PathBuf,
    builder: ParquetRecordBatchReaderBuilder<File>,
}

impl ParquetFile {
    pub(crate) fn open(path: &Path) -> Result<Self, MdError> {
        let file = File::open(path).map_err(|source| MdError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|error| MdError::parquet(path, error))?;
        Ok(Self {
            path: path.to_path_buf(),
            builder,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn num_rows(&self) -> i64 {
        self.builder.metadata().file_metadata().num_rows()
    }

    /// Value of a footer key-value entry such as `md.git_commit`.
    pub(crate) fn metadata_value(&self, key: &str) -> Option<&str> {
        self.builder
            .metadata()
            .file_metadata()
            .key_value_metadata()?
            .iter()
            .find(|entry| entry.key == key)?
            .value
            .as_deref()
    }

    /// Row groups whose `symbol` statistics may contain any of `sorted_symbols`.
    ///
    /// marketdata files are sorted by `(symbol, seq)`, so min/max statistics prune exactly.
    pub(crate) fn row_groups_for_symbols(&self, sorted_symbols: &[&str]) -> Vec<usize> {
        let metadata = self.builder.metadata();
        let column = metadata
            .file_metadata()
            .schema_descr()
            .columns()
            .iter()
            .position(|column| column.name() == "symbol");
        (0..metadata.num_row_groups())
            .filter(|&index| {
                let Some(stats) =
                    column.and_then(|column| metadata.row_group(index).column(column).statistics())
                else {
                    return true;
                };
                let (Some(min), Some(max)) = (stats.min_bytes_opt(), stats.max_bytes_opt()) else {
                    return true;
                };
                let first = sorted_symbols.partition_point(|symbol| symbol.as_bytes() < min);
                sorted_symbols
                    .get(first)
                    .is_some_and(|symbol| symbol.as_bytes() <= max)
            })
            .collect()
    }

    /// Streams record batches of `columns` from `row_groups` (all when `None`).
    pub(crate) fn read(
        self,
        columns: &[&str],
        row_groups: Option<Vec<usize>>,
        mut visit: impl FnMut(&Path, &RecordBatch) -> Result<(), MdError>,
    ) -> Result<(), MdError> {
        let Self { path, builder } = self;
        let mask = ProjectionMask::columns(builder.parquet_schema(), columns.iter().copied());
        let mut builder = builder.with_projection(mask).with_batch_size(BATCH_SIZE);
        if let Some(row_groups) = row_groups {
            if row_groups.is_empty() {
                return Ok(());
            }
            builder = builder.with_row_groups(row_groups);
        }
        let reader = builder
            .build()
            .map_err(|error| MdError::parquet(&path, error))?;
        for batch in reader {
            let batch = batch.map_err(|error| MdError::parquet(&path, error))?;
            visit(&path, &batch)?;
        }
        Ok(())
    }
}

pub(crate) fn primitive<'a, T: ArrowPrimitiveType>(
    path: &Path,
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a PrimitiveArray<T>, MdError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_primitive_opt::<T>())
        .ok_or_else(|| {
            MdError::inconsistent(
                path,
                format!("column {name} missing or not {}", T::DATA_TYPE),
            )
        })
}

pub(crate) fn boolean<'a>(
    path: &Path,
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a BooleanArray, MdError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_boolean_opt())
        .ok_or_else(|| MdError::inconsistent(path, format!("column {name} missing or not bool")))
}

/// A UTF-8 column in any of Arrow's string layouts.
pub(crate) enum Strings<'a> {
    Small(&'a arrow_array::StringArray),
    Large(&'a arrow_array::LargeStringArray),
    View(&'a arrow_array::StringViewArray),
}

impl Strings<'_> {
    pub(crate) fn get(&self, index: usize) -> Option<&str> {
        match self {
            Self::Small(array) => array.is_valid(index).then(|| array.value(index)),
            Self::Large(array) => array.is_valid(index).then(|| array.value(index)),
            Self::View(array) => array.is_valid(index).then(|| array.value(index)),
        }
    }
}

pub(crate) fn strings<'a>(
    path: &Path,
    batch: &'a RecordBatch,
    name: &str,
) -> Result<Strings<'a>, MdError> {
    let column = batch
        .column_by_name(name)
        .ok_or_else(|| MdError::inconsistent(path, format!("column {name} missing")))?;
    if let Some(array) = column.as_string_opt::<i32>() {
        Ok(Strings::Small(array))
    } else if let Some(array) = column.as_string_opt::<i64>() {
        Ok(Strings::Large(array))
    } else if let Some(array) = column.as_string_view_opt() {
        Ok(Strings::View(array))
    } else {
        Err(MdError::inconsistent(
            path,
            format!("column {name} is not a string"),
        ))
    }
}

pub(crate) fn optional<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    index: usize,
) -> Option<T::Native> {
    array.is_valid(index).then(|| array.value(index))
}

pub(crate) fn optional_bool(array: &BooleanArray, index: usize) -> Option<bool> {
    array.is_valid(index).then(|| array.value(index))
}

/// A required (non-null) string cell.
pub(crate) fn required_str<'a>(
    path: &Path,
    column: &'a Strings<'_>,
    name: &str,
    index: usize,
) -> Result<&'a str, MdError> {
    column
        .get(index)
        .ok_or_else(|| MdError::inconsistent(path, format!("column {name} has a null")))
}
