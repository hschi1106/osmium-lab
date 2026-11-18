use std::path::Path;

use arrow_array::types::{Float64Type, Int32Type};
use osmium_types::{Currency, Date, Instrument, InstrumentType, Price, Symbol, Venue};

use crate::{
    DayManifest, ManifestEntry, MdError, MdRoot, Table,
    parquet_io::{
        Column, ColumnType, boolean, optional, optional_bool, optional_str, price_units, primitive,
        required, strings,
    },
};

const COLUMNS: &[Column] = &[
    ("symbol", ColumnType::String),
    ("instrument_type", ColumnType::String),
    ("shares_per_lot", ColumnType::Int32),
    ("ref_price", ColumnType::Float64),
    ("limit_up", ColumnType::Float64),
    ("limit_down", ColumnType::Float64),
    ("match_interval_s", ColumnType::Int32),
    ("currency", ColumnType::String),
    ("derived", ColumnType::Bool),
];

/// One venue's `instruments` table for one trading date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayInstruments {
    date: Date,
    venue: Venue,
    entry: ManifestEntry,
    /// Sorted by symbol, as the file is.
    records: Vec<Instrument>,
}

impl DayInstruments {
    /// Reads `instruments/{YYYY}/{date}/{venue}.parquet` of the manifest's date.
    pub fn load(root: &MdRoot, manifest: &DayManifest, venue: Venue) -> Result<Self, MdError> {
        let (file, entry) = manifest.open(root, Table::Instruments, venue.as_str())?;
        let mut records: Vec<Instrument> = Vec::new();
        file.read(COLUMNS, None, |path, batch| {
            let symbol = strings(path, batch, "symbol")?;
            let kind = strings(path, batch, "instrument_type")?;
            let lot = primitive::<Int32Type>(path, batch, "shares_per_lot")?;
            let ref_price = primitive::<Float64Type>(path, batch, "ref_price")?;
            let limit_up = primitive::<Float64Type>(path, batch, "limit_up")?;
            let limit_down = primitive::<Float64Type>(path, batch, "limit_down")?;
            let interval = primitive::<Int32Type>(path, batch, "match_interval_s")?;
            let currency = strings(path, batch, "currency")?;
            let derived = boolean(path, batch, "derived")?;
            for row in 0..batch.num_rows() {
                let code = required(path, optional_str(symbol, row), "symbol")?;
                let invalid = |reason: String| {
                    MdError::inconsistent(path, format!("symbol {code}: {reason}"))
                };
                let symbol = Symbol::new(code).map_err(|error| invalid(error.to_string()))?;
                if records.last().is_some_and(|last| last.symbol >= symbol) {
                    return Err(invalid(
                        "rows are not in increasing symbol order".to_owned(),
                    ));
                }
                let kind = required(path, optional_str(kind, row), "instrument_type")?;
                let positive = |value: Option<i32>, name: &str| match value {
                    None => Ok(None),
                    Some(value) => u32::try_from(value)
                        .map(Some)
                        .map_err(|_| invalid(format!("{name} {value} is negative"))),
                };
                let shares_per_lot = positive(optional(lot, row), "shares_per_lot")?;
                if shares_per_lot == Some(0) {
                    return Err(invalid("shares_per_lot is 0".to_owned()));
                }
                records.push(Instrument {
                    venue,
                    symbol,
                    instrument_type: InstrumentType::from_marketdata(kind)
                        .ok_or_else(|| invalid(format!("unknown instrument_type {kind}")))?,
                    shares_per_lot,
                    ref_price: price(path, code, "ref_price", optional(ref_price, row))?,
                    limit_up: price(path, code, "limit_up", optional(limit_up, row))?,
                    limit_down: price(path, code, "limit_down", optional(limit_down, row))?,
                    match_interval_s: positive(optional(interval, row), "match_interval_s")?,
                    currency: match required(path, optional_str(currency, row), "currency")? {
                        "TWD" => Currency::Twd,
                        _ => Currency::Foreign,
                    },
                    derived: required(path, optional_bool(derived, row), "derived")?,
                });
            }
            Ok(())
        })?;
        manifest.verify_unchanged(root, Table::Instruments, venue.as_str(), &entry)?;
        Ok(Self {
            date: manifest.date(),
            venue,
            entry,
            records,
        })
    }

    #[must_use]
    pub const fn date(&self) -> Date {
        self.date
    }

    #[must_use]
    pub const fn venue(&self) -> Venue {
        self.venue
    }

    /// The manifest row of the file that was read.
    #[must_use]
    pub const fn manifest_entry(&self) -> &ManifestEntry {
        &self.entry
    }

    #[must_use]
    pub fn get(&self, symbol: Symbol) -> Option<&Instrument> {
        self.records
            .binary_search_by_key(&symbol, |record| record.symbol)
            .ok()
            .map(|index| &self.records[index])
    }

    /// Every instrument, in symbol order.
    #[must_use]
    pub fn records(&self) -> &[Instrument] {
        &self.records
    }
}

/// An instrument price cell. Null and 0 both mean the source has no price.
fn price(
    path: &Path,
    symbol: &str,
    name: &str,
    value: Option<f64>,
) -> Result<Option<Price>, MdError> {
    match value {
        None | Some(0.0) => Ok(None),
        Some(value) => price_units(value)
            .map(|units| Some(Price::from_units(i64::from(units))))
            .ok_or_else(|| {
                MdError::inconsistent(
                    path,
                    format!(
                        "symbol {symbol}: {name} {value} is not an exact positive 1e-4 decimal"
                    ),
                )
            }),
    }
}
