use std::collections::BTreeMap;

use arrow_array::types::Int32Type;
use market_types::{Symbol, TradingDate};

use crate::{
    Feed, MdError, MdRoot, Table,
    manifest::{DayManifest, ManifestEntry, open_verified},
    parquet_io::{boolean, optional, optional_bool, primitive, required_str, strings},
};

/// The subset of the daily `instruments` table the replay mapper needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentRecord {
    pub symbol: Symbol,
    pub instrument_type: String,
    pub shares_per_lot: Option<i32>,
    /// Matching cycle in seconds: `0` continuous, `> 0` periodic call auction, `None` unknown.
    pub match_interval_s: Option<i32>,
    pub derived: bool,
}

/// One venue's instrument master for one trading date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruments {
    entry: ManifestEntry,
    records: BTreeMap<Symbol, InstrumentRecord>,
}

impl Instruments {
    pub fn load(
        root: &MdRoot,
        date: TradingDate,
        feed: Feed,
        manifest: &DayManifest,
    ) -> Result<Self, MdError> {
        let entry = manifest.entry(Table::Instruments, feed.venue())?.clone();
        let file = open_verified(root, date, Table::Instruments, feed.venue(), &entry)?;
        let mut records = BTreeMap::new();
        file.read(
            &[
                "symbol",
                "instrument_type",
                "shares_per_lot",
                "match_interval_s",
                "derived",
            ],
            None,
            |path, batch| {
                let symbol = strings(path, batch, "symbol")?;
                let kind = strings(path, batch, "instrument_type")?;
                let lot = primitive::<Int32Type>(path, batch, "shares_per_lot")?;
                let interval = primitive::<Int32Type>(path, batch, "match_interval_s")?;
                let derived = boolean(path, batch, "derived")?;
                for row in 0..batch.num_rows() {
                    let text = required_str(path, &symbol, "symbol", row)?;
                    let symbol = Symbol::new(text).map_err(|error| {
                        MdError::inconsistent(path, format!("symbol {text:?}: {error}"))
                    })?;
                    let record = InstrumentRecord {
                        symbol,
                        instrument_type: required_str(path, &kind, "instrument_type", row)?
                            .to_owned(),
                        shares_per_lot: optional(lot, row),
                        match_interval_s: optional(interval, row),
                        derived: optional_bool(derived, row).ok_or_else(|| {
                            MdError::inconsistent(path, "column derived has a null")
                        })?,
                    };
                    if records.insert(symbol, record).is_some() {
                        return Err(MdError::inconsistent(
                            path,
                            format!("duplicate instrument {symbol}"),
                        ));
                    }
                }
                Ok(())
            },
        )?;
        Ok(Self { entry, records })
    }

    #[must_use]
    pub const fn manifest_entry(&self) -> &ManifestEntry {
        &self.entry
    }

    #[must_use]
    pub fn get(&self, symbol: &Symbol) -> Option<&InstrumentRecord> {
        self.records.get(symbol)
    }

    pub fn iter(&self) -> impl Iterator<Item = &InstrumentRecord> {
        self.records.values()
    }
}
