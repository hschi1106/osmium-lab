use std::{
    fs::File,
    path::{Path, PathBuf},
};

use arrow_array::{
    Array, ArrowPrimitiveType, BooleanArray, PrimitiveArray, RecordBatch, StringArray,
    cast::AsArray,
};
use arrow_schema::{DataType, TimeUnit};
use parquet::arrow::{
    ProjectionMask,
    arrow_reader::{ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder},
};

use crate::MdError;

const BATCH_SIZE: usize = 8 * 1024;

/// The Arrow type marketdata schema version 1 gives a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnType {
    String,
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    Float64,
    /// `date32[day]`.
    Date32,
    /// `decimal128(18, 8)`.
    Decimal18x8,
    /// `timestamp[us, UTC]`.
    TimestampMicrosUtc,
}

impl ColumnType {
    fn matches(self, actual: &DataType) -> bool {
        match (self, actual) {
            (Self::String, DataType::Utf8)
            | (Self::Bool, DataType::Boolean)
            | (Self::Int8, DataType::Int8)
            | (Self::Int16, DataType::Int16)
            | (Self::Int32, DataType::Int32)
            | (Self::Int64, DataType::Int64)
            | (Self::Float64, DataType::Float64)
            | (Self::Date32, DataType::Date32)
            | (Self::Decimal18x8, DataType::Decimal128(18, 8)) => true,
            (Self::TimestampMicrosUtc, DataType::Timestamp(TimeUnit::Microsecond, Some(zone))) => {
                zone.as_ref() == "UTC"
            }
            _ => false,
        }
    }
}

/// A column osmium reads and the type it must have.
pub(crate) type Column = (&'static str, ColumnType);

/// One marketdata parquet file with its footer already parsed.
///
/// Every read opens the path again, so several threads can read row groups of one file
/// at once: handles duplicated from one `File` would share a file position.
pub(crate) struct ParquetFile {
    path: PathBuf,
    footer: ArrowReaderMetadata,
}

impl ParquetFile {
    pub(crate) fn open(path: &Path) -> Result<Self, MdError> {
        let file = Self::file(path)?;
        let footer = ArrowReaderMetadata::load(&file, ArrowReaderOptions::new())
            .map_err(|error| MdError::parquet(path, error))?;
        Ok(Self {
            path: path.to_path_buf(),
            footer,
        })
    }

    fn file(path: &Path) -> Result<File, MdError> {
        File::open(path).map_err(|source| MdError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn num_rows(&self) -> i64 {
        self.footer.metadata().file_metadata().num_rows()
    }

    pub(crate) fn num_row_groups(&self) -> usize {
        self.footer.metadata().num_row_groups()
    }

    /// Value of a footer key-value entry such as `md.git_commit`.
    pub(crate) fn metadata_value(&self, key: &str) -> Option<&str> {
        self.footer
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
        let metadata = self.footer.metadata();
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

    /// Row groups whose `trade_date` statistics may contain `epoch_days`.
    ///
    /// The yearly tables are sorted by `(trade_date, symbol)`.
    pub(crate) fn row_groups_for_date(&self, epoch_days: i32) -> Vec<usize> {
        let metadata = self.footer.metadata();
        let column = metadata
            .file_metadata()
            .schema_descr()
            .columns()
            .iter()
            .position(|column| column.name() == "trade_date");
        let days = |bytes: &[u8]| <[u8; 4]>::try_from(bytes).ok().map(i32::from_le_bytes);
        (0..metadata.num_row_groups())
            .filter(|&index| {
                let Some(stats) =
                    column.and_then(|column| metadata.row_group(index).column(column).statistics())
                else {
                    return true;
                };
                let (Some(min), Some(max)) = (
                    stats.min_bytes_opt().and_then(days),
                    stats.max_bytes_opt().and_then(days),
                ) else {
                    return true;
                };
                min <= epoch_days && epoch_days <= max
            })
            .collect()
    }

    /// Fails when one of `columns` is missing or has another type.
    ///
    /// Columns osmium does not read are not checked: schema version 1 may gain columns.
    pub(crate) fn check(&self, columns: &[Column]) -> Result<(), MdError> {
        let path = self.path.as_path();
        for (name, expected) in columns {
            match self.footer.schema().field_with_name(name) {
                Ok(field) if expected.matches(field.data_type()) => {}
                Ok(field) => {
                    return Err(MdError::schema(
                        path,
                        format!(
                            "column {name} is {}, expected {expected:?}",
                            field.data_type()
                        ),
                    ));
                }
                Err(_) => return Err(MdError::schema(path, format!("column {name} is missing"))),
            }
        }
        Ok(())
    }

    /// Streams record batches of `columns` from `row_groups` (all when `None`).
    ///
    /// Fails before reading any row when [`Self::check`] does.
    pub(crate) fn read(
        &self,
        columns: &[Column],
        row_groups: Option<Vec<usize>>,
        mut visit: impl FnMut(&Path, &RecordBatch) -> Result<(), MdError>,
    ) -> Result<(), MdError> {
        self.check(columns)?;
        let path = self.path.as_path();
        if row_groups.as_ref().is_some_and(Vec::is_empty) {
            return Ok(());
        }
        let builder = ParquetRecordBatchReaderBuilder::new_with_metadata(
            Self::file(path)?,
            self.footer.clone(),
        );
        let mask = ProjectionMask::columns(
            builder.parquet_schema(),
            columns.iter().map(|(name, _)| *name),
        );
        let mut builder = builder.with_projection(mask).with_batch_size(BATCH_SIZE);
        if let Some(row_groups) = row_groups {
            builder = builder.with_row_groups(row_groups);
        }
        let reader = builder
            .build()
            .map_err(|error| MdError::parquet(path, error))?;
        for batch in reader {
            let batch = batch.map_err(|error| MdError::parquet(path, error))?;
            visit(path, &batch)?;
        }
        Ok(())
    }
}

// The accessors below are called for columns `ParquetFile::read` has already checked,
// so a failure means the reader returned a batch that disagrees with the file schema.

pub(crate) fn primitive<'a, T: ArrowPrimitiveType>(
    path: &Path,
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a PrimitiveArray<T>, MdError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_primitive_opt::<T>())
        .ok_or_else(|| MdError::schema(path, format!("column {name} is not {}", T::DATA_TYPE)))
}

pub(crate) fn boolean<'a>(
    path: &Path,
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a BooleanArray, MdError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_boolean_opt())
        .ok_or_else(|| MdError::schema(path, format!("column {name} is not bool")))
}

pub(crate) fn strings<'a>(
    path: &Path,
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a StringArray, MdError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_string_opt::<i32>())
        .ok_or_else(|| MdError::schema(path, format!("column {name} is not string")))
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

pub(crate) fn optional_str(array: &StringArray, index: usize) -> Option<&str> {
    array.is_valid(index).then(|| array.value(index))
}

/// A cell of a column the schema declares non-null.
pub(crate) fn required<T>(path: &Path, value: Option<T>, name: &str) -> Result<T, MdError> {
    value.ok_or_else(|| MdError::inconsistent(path, format!("column {name} has a null")))
}

/// Converts a marketdata float64 price to exact units of 1e-4 元.
///
/// marketdata stores the nearest double to each decimal price, so the conversion is
/// accepted only when it round-trips exactly and the price is positive.
pub(crate) fn price_units(value: f64) -> Option<u32> {
    if !value.is_finite() || value <= 0.0 || value > 400_000.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let units = (value * 10_000.0).round() as u32;
    #[allow(clippy::float_cmp)]
    let exact = f64::from(units) / 10_000.0 == value;
    (exact && units > 0).then_some(units)
}

#[cfg(test)]
mod tests {
    use super::price_units;

    #[test]
    fn prices_convert_only_when_exact_and_positive() {
        assert_eq!(price_units(198.45), Some(1_984_500));
        assert_eq!(price_units(0.01), Some(100));
        assert_eq!(price_units(10.12), Some(101_200));
        assert_eq!(price_units(1_085.0), Some(10_850_000));
        assert_eq!(price_units(9_999.95), Some(99_999_500));
        assert_eq!(price_units(22_995.0), Some(229_950_000));
        assert_eq!(price_units(0.000_05), None);
        assert_eq!(price_units(1.234_56), None);
        assert_eq!(price_units(0.0), None);
        assert_eq!(price_units(-1.0), None);
        assert_eq!(price_units(f64::NAN), None);
        assert_eq!(price_units(f64::INFINITY), None);
        assert_eq!(price_units(400_000.01), None);
    }
}
