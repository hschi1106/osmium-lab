use std::{collections::BTreeMap, path::Path};

use arrow_array::{
    RecordBatch,
    types::{Float64Type, Int8Type, Int16Type, Int64Type, TimestampMicrosecondType},
};
use market_types::{BOOK_DEPTH, Symbol};

use crate::{
    MdError,
    parquet_io::{ParquetFile, boolean, optional, optional_bool, primitive, required_str, strings},
};

/// Price ticks per unit: marketdata prices are decimals with at most four places.
pub(crate) const TICKS_PER_UNIT: i64 = 10_000;

/// Converts a marketdata float64 price to exact 1e-4 ticks.
///
/// marketdata stores the nearest double to each decimal price, so the conversion is
/// accepted only when it round-trips exactly; anything else is a data error.
pub(crate) fn price_ticks(value: f64) -> Option<i64> {
    if !value.is_finite() || value <= 0.0 || value >= 1e14 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let ticks = (value * 10_000.0).round() as i64;
    #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
    let exact = ticks as f64 / 10_000.0 == value;
    (exact && ticks > 0).then_some(ticks)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    OpenAuction,
    Continuous,
    PeriodicAuction,
    CloseAuction,
}

impl Phase {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "OPEN_AUCTION" => Some(Self::OpenAuction),
            "CONTINUOUS" => Some(Self::Continuous),
            "PERIODIC_AUCTION" => Some(Self::PeriodicAuction),
            "CLOSE_AUCTION" => Some(Self::CloseAuction),
            _ => None,
        }
    }
}

/// One displayed book side: `levels` priced levels in ticks and positive quantities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SideRow {
    pub(crate) levels: u8,
    pub(crate) price: [i64; BOOK_DEPTH],
    pub(crate) quantity: [u64; BOOK_DEPTH],
    pub(crate) market_quantity: Option<u64>,
}

/// One `books` row in compact form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BookRow {
    pub(crate) seq: u64,
    pub(crate) exch_ts: i64,
    pub(crate) phase: Phase,
    pub(crate) has_trade: bool,
    pub(crate) is_indicative: bool,
    /// `None` when the row disclosed no book (e.g. a delay row).
    pub(crate) bid: Option<SideRow>,
    pub(crate) ask: Option<SideRow>,
    pub(crate) indicative_price: Option<i64>,
    pub(crate) indicative_quantity: Option<u64>,
    pub(crate) delay_open: Option<bool>,
    pub(crate) delay_close: Option<bool>,
    pub(crate) delay_state: i8,
}

/// One `trades` row in compact form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TradeRow {
    pub(crate) seq: u64,
    pub(crate) exch_ts: i64,
    pub(crate) price: i64,
    pub(crate) quantity: u64,
    pub(crate) cumulative_quantity: Option<u64>,
    pub(crate) phase: Phase,
}

pub(crate) const BOOK_COLUMNS: &[&str] = &[
    "symbol",
    "exch_ts",
    "seq",
    "phase",
    "has_trade",
    "n_bid",
    "n_ask",
    "bid_px_1",
    "bid_qty_1",
    "bid_px_2",
    "bid_qty_2",
    "bid_px_3",
    "bid_qty_3",
    "bid_px_4",
    "bid_qty_4",
    "bid_px_5",
    "bid_qty_5",
    "ask_px_1",
    "ask_qty_1",
    "ask_px_2",
    "ask_qty_2",
    "ask_px_3",
    "ask_qty_3",
    "ask_px_4",
    "ask_qty_4",
    "ask_px_5",
    "ask_qty_5",
    "bid_mkt_qty",
    "ask_mkt_qty",
    "is_indicative",
    "indicative_px",
    "indicative_qty",
    "delay_open",
    "delay_close",
    "delay_state",
];

pub(crate) const TRADE_COLUMNS: &[&str] = &[
    "symbol", "exch_ts", "seq", "sub_seq", "price", "qty", "cum_qty", "phase",
];

/// Rows of the selected symbols, each symbol's rows in strictly increasing `seq`.
pub(crate) type SymbolRows<R> = BTreeMap<Symbol, Vec<R>>;

/// Reads the rows of `sorted_symbols` from a `books` file.
pub(crate) fn read_books(
    file: ParquetFile,
    sorted_symbols: &[&str],
) -> Result<SymbolRows<BookRow>, MdError> {
    let row_groups = file.row_groups_for_symbols(sorted_symbols);
    let mut rows = SymbolRows::new();
    file.read(BOOK_COLUMNS, Some(row_groups), |path, batch| {
        let columns = BookColumns::new(path, batch)?;
        for_selected_rows(path, batch, sorted_symbols, |symbol, row| {
            let book = columns.row(path, &symbol, row)?;
            push_in_seq_order(&mut rows, symbol, book, book.seq)
        })
    })?;
    Ok(rows)
}

/// Reads the rows of `sorted_symbols` from a `trades` file.
pub(crate) fn read_trades(
    file: ParquetFile,
    sorted_symbols: &[&str],
) -> Result<SymbolRows<TradeRow>, MdError> {
    let row_groups = file.row_groups_for_symbols(sorted_symbols);
    let mut rows = SymbolRows::new();
    file.read(TRADE_COLUMNS, Some(row_groups), |path, batch| {
        let exch_ts = primitive::<TimestampMicrosecondType>(path, batch, "exch_ts")?;
        let seq = primitive::<Int64Type>(path, batch, "seq")?;
        let sub_seq = primitive::<Int16Type>(path, batch, "sub_seq")?;
        let price = primitive::<Float64Type>(path, batch, "price")?;
        let qty = primitive::<Int64Type>(path, batch, "qty")?;
        let cum_qty = primitive::<Int64Type>(path, batch, "cum_qty")?;
        let phase = strings(path, batch, "phase")?;
        for_selected_rows(path, batch, sorted_symbols, |symbol, row| {
            let seq = required_seq(path, optional(seq, row))?;
            let invalid = |reason| MdError::InvalidRow {
                symbol,
                seq,
                reason,
            };
            if optional(sub_seq, row) != Some(0) {
                return Err(invalid(
                    "multi-trade messages (sub_seq != 0) are not supported",
                ));
            }
            let value = optional(price, row).ok_or_else(|| invalid("trade price is null"))?;
            let trade = TradeRow {
                seq,
                exch_ts: optional(exch_ts, row).ok_or_else(|| invalid("exch_ts is null"))?,
                price: price_ticks(value).ok_or(MdError::NonCanonicalPrice {
                    path: path.to_path_buf(),
                    seq,
                    value,
                })?,
                quantity: positive(optional(qty, row)).ok_or_else(|| invalid("trade qty <= 0"))?,
                cumulative_quantity: match optional(cum_qty, row) {
                    None => None,
                    Some(value) => Some(u64::try_from(value).map_err(|_| invalid("cum_qty < 0"))?),
                },
                phase: parse_phase(path, &phase, row, symbol, seq)?,
            };
            push_in_seq_order(&mut rows, symbol, trade, seq)
        })
    })?;
    Ok(rows)
}

fn for_selected_rows(
    path: &Path,
    batch: &RecordBatch,
    sorted_symbols: &[&str],
    mut visit: impl FnMut(Symbol, usize) -> Result<(), MdError>,
) -> Result<(), MdError> {
    let symbols = strings(path, batch, "symbol")?;
    let mut current: Option<(usize, Option<Symbol>)> = None;
    for row in 0..batch.num_rows() {
        let text = required_str(path, &symbols, "symbol", row)?;
        // Rows are sorted by symbol: reuse the previous lookup while the symbol repeats.
        let selected = match &current {
            Some((previous, selected))
                if required_str(path, &symbols, "symbol", *previous)? == text =>
            {
                *selected
            }
            _ => {
                let selected = sorted_symbols
                    .binary_search(&text)
                    .ok()
                    .map(|_| Symbol::new(text))
                    .transpose()
                    .map_err(|error| {
                        MdError::inconsistent(path, format!("symbol {text:?}: {error}"))
                    })?;
                current = Some((row, selected));
                selected
            }
        };
        if let Some(symbol) = selected {
            visit(symbol, row)?;
        }
    }
    Ok(())
}

fn push_in_seq_order<R>(
    rows: &mut SymbolRows<R>,
    symbol: Symbol,
    row: R,
    seq: u64,
) -> Result<(), MdError>
where
    R: HasSeq,
{
    let list = rows.entry(symbol).or_default();
    if list.last().is_some_and(|last| last.seq() >= seq) {
        return Err(MdError::InvalidRow {
            symbol,
            seq,
            reason: "rows are not in strictly increasing seq order",
        });
    }
    list.push(row);
    Ok(())
}

pub(crate) trait HasSeq {
    fn seq(&self) -> u64;
}

impl HasSeq for BookRow {
    fn seq(&self) -> u64 {
        self.seq
    }
}

impl HasSeq for TradeRow {
    fn seq(&self) -> u64 {
        self.seq
    }
}

fn required_seq(path: &Path, value: Option<i64>) -> Result<u64, MdError> {
    value
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| MdError::inconsistent(path, "seq is null or negative"))
}

fn positive(value: Option<i64>) -> Option<u64> {
    value
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn parse_phase(
    path: &Path,
    column: &crate::parquet_io::Strings<'_>,
    row: usize,
    symbol: Symbol,
    seq: u64,
) -> Result<Phase, MdError> {
    Phase::parse(required_str(path, column, "phase", row)?).ok_or(MdError::InvalidRow {
        symbol,
        seq,
        reason: "unknown phase",
    })
}

struct SideColumns<'a> {
    levels: &'a arrow_array::PrimitiveArray<Int8Type>,
    price: [&'a arrow_array::PrimitiveArray<Float64Type>; BOOK_DEPTH],
    quantity: [&'a arrow_array::PrimitiveArray<Int64Type>; BOOK_DEPTH],
    market: &'a arrow_array::PrimitiveArray<Int64Type>,
}

impl<'a> SideColumns<'a> {
    fn new(path: &Path, batch: &'a RecordBatch, side: &str) -> Result<Self, MdError> {
        let mut price = Vec::with_capacity(BOOK_DEPTH);
        let mut quantity = Vec::with_capacity(BOOK_DEPTH);
        for level in 1..=BOOK_DEPTH {
            price.push(primitive::<Float64Type>(
                path,
                batch,
                &format!("{side}_px_{level}"),
            )?);
            quantity.push(primitive::<Int64Type>(
                path,
                batch,
                &format!("{side}_qty_{level}"),
            )?);
        }
        Ok(Self {
            levels: primitive::<Int8Type>(path, batch, &format!("n_{side}"))?,
            price: price.try_into().unwrap_or_else(|_| unreachable!()),
            quantity: quantity.try_into().unwrap_or_else(|_| unreachable!()),
            market: primitive::<Int64Type>(path, batch, &format!("{side}_mkt_qty"))?,
        })
    }

    fn row(
        &self,
        path: &Path,
        symbol: Symbol,
        seq: u64,
        row: usize,
    ) -> Result<Option<SideRow>, MdError> {
        let invalid = |reason| MdError::InvalidRow {
            symbol,
            seq,
            reason,
        };
        let Some(levels) = optional(self.levels, row) else {
            for level in 0..BOOK_DEPTH {
                if optional(self.price[level], row).is_some()
                    || optional(self.quantity[level], row).is_some()
                {
                    return Err(invalid("levels present without a level count"));
                }
            }
            return Ok(None);
        };
        let levels = u8::try_from(levels)
            .ok()
            .filter(|levels| usize::from(*levels) <= BOOK_DEPTH)
            .ok_or_else(|| invalid("level count outside 0..=5"))?;
        let mut side = SideRow {
            levels,
            price: [0; BOOK_DEPTH],
            quantity: [0; BOOK_DEPTH],
            market_quantity: match optional(self.market, row) {
                None => None,
                Some(value) => {
                    Some(positive(Some(value)).ok_or_else(|| invalid("market-order qty <= 0"))?)
                }
            },
        };
        for level in 0..BOOK_DEPTH {
            let price = optional(self.price[level], row);
            let quantity = optional(self.quantity[level], row);
            if level < usize::from(levels) {
                let value = price.ok_or_else(|| invalid("disclosed level has a null price"))?;
                side.price[level] = price_ticks(value).ok_or(MdError::NonCanonicalPrice {
                    path: path.to_path_buf(),
                    seq,
                    value,
                })?;
                side.quantity[level] =
                    positive(quantity).ok_or_else(|| invalid("disclosed level qty <= 0"))?;
            } else if price.is_some() || quantity.is_some() {
                return Err(invalid("level beyond the disclosed count"));
            }
        }
        Ok(Some(side))
    }
}

struct BookColumns<'a> {
    exch_ts: &'a arrow_array::PrimitiveArray<TimestampMicrosecondType>,
    seq: &'a arrow_array::PrimitiveArray<Int64Type>,
    phase: crate::parquet_io::Strings<'a>,
    has_trade: &'a arrow_array::BooleanArray,
    bid: SideColumns<'a>,
    ask: SideColumns<'a>,
    is_indicative: &'a arrow_array::BooleanArray,
    indicative_px: &'a arrow_array::PrimitiveArray<Float64Type>,
    indicative_qty: &'a arrow_array::PrimitiveArray<Int64Type>,
    delay_open: &'a arrow_array::BooleanArray,
    delay_close: &'a arrow_array::BooleanArray,
    delay_state: &'a arrow_array::PrimitiveArray<Int8Type>,
}

impl<'a> BookColumns<'a> {
    fn new(path: &Path, batch: &'a RecordBatch) -> Result<Self, MdError> {
        Ok(Self {
            exch_ts: primitive::<TimestampMicrosecondType>(path, batch, "exch_ts")?,
            seq: primitive::<Int64Type>(path, batch, "seq")?,
            phase: strings(path, batch, "phase")?,
            has_trade: boolean(path, batch, "has_trade")?,
            bid: SideColumns::new(path, batch, "bid")?,
            ask: SideColumns::new(path, batch, "ask")?,
            is_indicative: boolean(path, batch, "is_indicative")?,
            indicative_px: primitive::<Float64Type>(path, batch, "indicative_px")?,
            indicative_qty: primitive::<Int64Type>(path, batch, "indicative_qty")?,
            delay_open: boolean(path, batch, "delay_open")?,
            delay_close: boolean(path, batch, "delay_close")?,
            delay_state: primitive::<Int8Type>(path, batch, "delay_state")?,
        })
    }

    fn row(&self, path: &Path, symbol: &Symbol, row: usize) -> Result<BookRow, MdError> {
        let symbol = *symbol;
        let seq = required_seq(path, optional(self.seq, row))?;
        let invalid = |reason| MdError::InvalidRow {
            symbol,
            seq,
            reason,
        };
        let required = |value: Option<bool>, name| value.ok_or_else(|| invalid(name));
        Ok(BookRow {
            seq,
            exch_ts: optional(self.exch_ts, row).ok_or_else(|| invalid("exch_ts is null"))?,
            phase: parse_phase(path, &self.phase, row, symbol, seq)?,
            has_trade: required(optional_bool(self.has_trade, row), "has_trade is null")?,
            is_indicative: required(
                optional_bool(self.is_indicative, row),
                "is_indicative is null",
            )?,
            bid: self.bid.row(path, symbol, seq, row)?,
            ask: self.ask.row(path, symbol, seq, row)?,
            indicative_price: match optional(self.indicative_px, row) {
                None => None,
                Some(value) => Some(price_ticks(value).ok_or(MdError::NonCanonicalPrice {
                    path: path.to_path_buf(),
                    seq,
                    value,
                })?),
            },
            indicative_quantity: match optional(self.indicative_qty, row) {
                None => None,
                Some(value) => {
                    Some(positive(Some(value)).ok_or_else(|| invalid("indicative qty <= 0"))?)
                }
            },
            delay_open: optional_bool(self.delay_open, row),
            delay_close: optional_bool(self.delay_close, row),
            delay_state: optional(self.delay_state, row)
                .ok_or_else(|| invalid("delay_state is null"))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::price_ticks;

    #[test]
    fn prices_convert_only_when_exact() {
        assert_eq!(price_ticks(198.45), Some(1_984_500));
        assert_eq!(price_ticks(0.01), Some(100));
        assert_eq!(price_ticks(10.12), Some(101_200));
        assert_eq!(price_ticks(1_085.0), Some(10_850_000));
        assert_eq!(price_ticks(0.000_05), None);
        assert_eq!(price_ticks(1.234_56), None);
        assert_eq!(price_ticks(0.0), None);
        assert_eq!(price_ticks(-1.0), None);
        assert_eq!(price_ticks(f64::NAN), None);
    }
}
