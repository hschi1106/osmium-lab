//! Decoding of `books` and `trades` rows into compact, validated records.
//!
//! Records keep the file's native quantity unit; [`crate::FeedDay`] converts to shares
//! when it builds a message.

use std::{collections::HashMap, path::Path};

use arrow_array::{
    BooleanArray, PrimitiveArray, RecordBatch, StringArray,
    types::{Float64Type, Int8Type, Int16Type, Int64Type, TimestampMicrosecondType},
};
use osmium_types::{BOOK_DEPTH, DelayDirection, Phase, Symbol};

use crate::{
    MdError,
    parquet_io::{
        Column, ColumnType, ParquetFile, boolean, optional, optional_bool, optional_str,
        price_units, primitive, required, strings,
    },
};

/// One book side: prices in units of 1e-4 元, quantities in the feed's native unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactSide {
    pub(crate) price: [u32; BOOK_DEPTH],
    pub(crate) qty: [u32; BOOK_DEPTH],
    /// Market-order quantity; 0 when there is none.
    pub(crate) market_qty: u32,
    pub(crate) len: u8,
}

impl CompactSide {
    const EMPTY: Self = Self {
        price: [0; BOOK_DEPTH],
        qty: [0; BOOK_DEPTH],
        market_qty: 0,
        len: 0,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BookKind {
    Disclosure {
        has_trade: bool,
    },
    /// A fill of a sweep still in progress that the source wrote to `books` with an empty
    /// book instead of leaving it to `trades` alone; see [`mark_intermediate_fills`].
    IntermediateTrade,
    Indicative,
    Delay(DelayDirection),
}

impl BookKind {
    /// Whether a `trades` row shares this row's seq.
    pub(crate) const fn has_trade(self) -> bool {
        matches!(
            self,
            Self::Disclosure { has_trade: true } | Self::IntermediateTrade
        )
    }
}

/// One `books` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BookRow {
    pub(crate) seq: u64,
    pub(crate) exch_ts: i64,
    /// Index into the [`SymbolTable`].
    pub(crate) symbol: u32,
    pub(crate) phase: Phase,
    pub(crate) kind: BookKind,
    pub(crate) bid: CompactSide,
    pub(crate) ask: CompactSide,
    /// Indicative match price and quantity; both 0 when the row has none.
    pub(crate) indicative_price: u32,
    pub(crate) indicative_qty: u32,
    pub(crate) delay_open: Option<bool>,
    pub(crate) delay_close: Option<bool>,
}

/// One `trades` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TradeRow {
    pub(crate) seq: u64,
    pub(crate) exch_ts: i64,
    pub(crate) symbol: u32,
    pub(crate) phase: Phase,
    pub(crate) price: u32,
    pub(crate) qty: u32,
    pub(crate) cum_qty: Option<u64>,
}

/// A symbol of the loaded feed day and how its quantities convert to shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SymbolEntry {
    pub(crate) symbol: Symbol,
    /// Shares per native quantity unit: shares per lot on the regular board, 1 for odd lots.
    pub(crate) shares_per_unit: u32,
    /// `shares_per_unit` is an assumption because `instruments` has no value.
    pub(crate) assumed: bool,
}

/// The symbols seen while decoding, in order of first appearance.
#[derive(Debug, Default)]
pub(crate) struct SymbolTable {
    pub(crate) entries: Vec<SymbolEntry>,
    index: HashMap<Symbol, u32>,
}

impl SymbolTable {
    fn intern(
        &mut self,
        symbol: Symbol,
        resolve: &dyn Fn(Symbol) -> Result<(u32, bool), MdError>,
    ) -> Result<u32, MdError> {
        if let Some(index) = self.index.get(&symbol) {
            return Ok(*index);
        }
        let (shares_per_unit, assumed) = resolve(symbol)?;
        let index = u32::try_from(self.entries.len()).expect("fewer than 2^32 symbols");
        self.entries.push(SymbolEntry {
            symbol,
            shares_per_unit,
            assumed,
        });
        self.index.insert(symbol, index);
        Ok(index)
    }
}

/// What a decode needs besides the file.
pub(crate) struct DecodeContext<'a> {
    /// Sorted, unique symbols to keep; `None` keeps every symbol.
    pub(crate) selection: Option<&'a [Symbol]>,
    /// Shares per native unit of a symbol, and whether that value is an assumption.
    pub(crate) resolve: &'a dyn Fn(Symbol) -> Result<(u32, bool), MdError>,
}

impl DecodeContext<'_> {
    fn row_groups(&self, file: &ParquetFile) -> Option<Vec<usize>> {
        self.selection.map(|symbols| {
            let codes: Vec<&str> = symbols.iter().map(Symbol::as_str).collect();
            file.row_groups_for_symbols(&codes)
        })
    }
}

/// Walks the `symbol` column of a file sorted by `(symbol, seq)`.
struct SymbolRuns {
    code: String,
    symbol: Option<Symbol>,
    /// Table index of the current symbol when it is selected.
    selected: Option<u32>,
    last_seq: Option<u64>,
}

impl SymbolRuns {
    const fn new() -> Self {
        Self {
            code: String::new(),
            symbol: None,
            selected: None,
            last_seq: None,
        }
    }

    /// Advances to a row; returns its symbol's table index when the symbol is selected.
    fn enter(
        &mut self,
        path: &Path,
        code: &str,
        seq: u64,
        context: &DecodeContext<'_>,
        table: &mut SymbolTable,
    ) -> Result<Option<u32>, MdError> {
        let invalid = |reason: String| MdError::InvalidRow {
            path: path.to_path_buf(),
            symbol: code.to_owned(),
            seq,
            reason,
        };
        if self.symbol.is_none() || self.code != code {
            let symbol = Symbol::new(code).map_err(|error| invalid(error.to_string()))?;
            if self.symbol.is_some_and(|previous| previous >= symbol) {
                return Err(invalid("rows are not sorted by symbol".to_owned()));
            }
            code.clone_into(&mut self.code);
            self.symbol = Some(symbol);
            self.last_seq = None;
            let wanted = context
                .selection
                .is_none_or(|symbols| symbols.binary_search(&symbol).is_ok());
            self.selected = if wanted {
                Some(table.intern(symbol, context.resolve)?)
            } else {
                None
            };
        }
        if self.last_seq.is_some_and(|last| last >= seq) {
            return Err(invalid(
                "rows of one symbol are not in strictly increasing seq order".to_owned(),
            ));
        }
        self.last_seq = Some(seq);
        Ok(self.selected)
    }
}

fn sequence(path: &Path, value: Option<i64>) -> Result<u64, MdError> {
    required(path, value, "seq")?
        .try_into()
        .map_err(|_| MdError::inconsistent(path, "seq is negative"))
}

/// A quantity cell: null, or a positive value that fits the compact record.
fn quantity(value: Option<i64>, name: &str) -> Result<Option<u32>, String> {
    match value {
        None => Ok(None),
        Some(value) => u32::try_from(value)
            .ok()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(|| format!("{name} {value} is outside 1..=4294967295")),
    }
}

fn price(value: f64, name: &str) -> Result<u32, String> {
    price_units(value)
        .ok_or_else(|| format!("{name} {value} is not an exact positive 1e-4 decimal"))
}

fn phase(path: &Path, column: &StringArray, row: usize) -> Result<Result<Phase, String>, MdError> {
    let value = required(path, optional_str(column, row), "phase")?;
    Ok(Phase::from_marketdata(value).ok_or_else(|| format!("unknown phase {value}")))
}

const fn level_columns(side: &str) -> [(&'static str, &'static str); BOOK_DEPTH] {
    match side.as_bytes()[0] {
        b'b' => [
            ("bid_px_1", "bid_qty_1"),
            ("bid_px_2", "bid_qty_2"),
            ("bid_px_3", "bid_qty_3"),
            ("bid_px_4", "bid_qty_4"),
            ("bid_px_5", "bid_qty_5"),
        ],
        _ => [
            ("ask_px_1", "ask_qty_1"),
            ("ask_px_2", "ask_qty_2"),
            ("ask_px_3", "ask_qty_3"),
            ("ask_px_4", "ask_qty_4"),
            ("ask_px_5", "ask_qty_5"),
        ],
    }
}

const BOOK_COLUMNS: &[Column] = &[
    ("symbol", ColumnType::String),
    ("exch_ts", ColumnType::TimestampMicrosUtc),
    ("seq", ColumnType::Int64),
    ("phase", ColumnType::String),
    ("has_trade", ColumnType::Bool),
    ("n_bid", ColumnType::Int8),
    ("n_ask", ColumnType::Int8),
    ("bid_px_1", ColumnType::Float64),
    ("bid_qty_1", ColumnType::Int64),
    ("bid_px_2", ColumnType::Float64),
    ("bid_qty_2", ColumnType::Int64),
    ("bid_px_3", ColumnType::Float64),
    ("bid_qty_3", ColumnType::Int64),
    ("bid_px_4", ColumnType::Float64),
    ("bid_qty_4", ColumnType::Int64),
    ("bid_px_5", ColumnType::Float64),
    ("bid_qty_5", ColumnType::Int64),
    ("ask_px_1", ColumnType::Float64),
    ("ask_qty_1", ColumnType::Int64),
    ("ask_px_2", ColumnType::Float64),
    ("ask_qty_2", ColumnType::Int64),
    ("ask_px_3", ColumnType::Float64),
    ("ask_qty_3", ColumnType::Int64),
    ("ask_px_4", ColumnType::Float64),
    ("ask_qty_4", ColumnType::Int64),
    ("ask_px_5", ColumnType::Float64),
    ("ask_qty_5", ColumnType::Int64),
    ("bid_mkt_qty", ColumnType::Int64),
    ("ask_mkt_qty", ColumnType::Int64),
    ("is_indicative", ColumnType::Bool),
    ("indicative_px", ColumnType::Float64),
    ("indicative_qty", ColumnType::Int64),
    ("delay_open", ColumnType::Bool),
    ("delay_close", ColumnType::Bool),
    ("delay_state", ColumnType::Int8),
];

const TRADE_COLUMNS: &[Column] = &[
    ("symbol", ColumnType::String),
    ("exch_ts", ColumnType::TimestampMicrosUtc),
    ("seq", ColumnType::Int64),
    ("sub_seq", ColumnType::Int16),
    ("price", ColumnType::Float64),
    ("qty", ColumnType::Int64),
    ("cum_qty", ColumnType::Int64),
    ("phase", ColumnType::String),
];

struct SideColumns<'a> {
    levels: &'a PrimitiveArray<Int8Type>,
    price: [&'a PrimitiveArray<Float64Type>; BOOK_DEPTH],
    qty: [&'a PrimitiveArray<Int64Type>; BOOK_DEPTH],
    market: &'a PrimitiveArray<Int64Type>,
    /// Bid prices fall away from the best level; ask prices rise.
    rising: bool,
}

impl<'a> SideColumns<'a> {
    fn new(path: &Path, batch: &'a RecordBatch, side: &'static str) -> Result<Self, MdError> {
        let names = level_columns(side);
        let first_price = primitive::<Float64Type>(path, batch, names[0].0)?;
        let first_qty = primitive::<Int64Type>(path, batch, names[0].1)?;
        let mut price = [first_price; BOOK_DEPTH];
        let mut qty = [first_qty; BOOK_DEPTH];
        for (level, (price_name, qty_name)) in names.into_iter().enumerate().skip(1) {
            price[level] = primitive::<Float64Type>(path, batch, price_name)?;
            qty[level] = primitive::<Int64Type>(path, batch, qty_name)?;
        }
        let (count, market) = if side == "bid" {
            ("n_bid", "bid_mkt_qty")
        } else {
            ("n_ask", "ask_mkt_qty")
        };
        Ok(Self {
            levels: primitive::<Int8Type>(path, batch, count)?,
            price,
            qty,
            market: primitive::<Int64Type>(path, batch, market)?,
            rising: side == "ask",
        })
    }

    /// The side of `row`; `None` when the row discloses no book (`n_*` is null).
    fn row(&self, row: usize) -> Result<Option<CompactSide>, String> {
        let Some(levels) = optional(self.levels, row) else {
            let any_cell = (0..BOOK_DEPTH).any(|level| {
                optional(self.price[level], row).is_some()
                    || optional(self.qty[level], row).is_some()
            });
            if any_cell || optional(self.market, row).is_some() {
                return Err("book values present without a level count".to_owned());
            }
            return Ok(None);
        };
        let len = u8::try_from(levels)
            .ok()
            .filter(|len| usize::from(*len) <= BOOK_DEPTH)
            .ok_or_else(|| format!("level count {levels} is outside 0..=5"))?;
        let market = quantity(optional(self.market, row), "market-order qty")?;
        // The exchange sends market orders as a first level without a price. marketdata moves
        // that level to `*_mkt_qty`, except on days converted from a source that records the
        // missing price as null: there it stays the first level, counted in `n_*`.
        let market_level = len > 0 && market.is_none() && optional(self.price[0], row).is_none();
        let skipped = usize::from(market_level);
        let mut side = CompactSide {
            len: len - u8::from(market_level),
            market_qty: market.unwrap_or(0),
            ..CompactSide::EMPTY
        };
        for source in 0..BOOK_DEPTH {
            let level_price = optional(self.price[source], row);
            let level_qty = optional(self.qty[source], row);
            if source >= usize::from(len) {
                if level_price.is_some() || level_qty.is_some() {
                    return Err("level beyond the disclosed count".to_owned());
                }
                continue;
            }
            let level_qty =
                quantity(level_qty, "level qty")?.ok_or("disclosed level has a null qty")?;
            if source < skipped {
                side.market_qty = level_qty;
                continue;
            }
            let level = source - skipped;
            let value = level_price.ok_or("disclosed level has a null price")?;
            side.price[level] = price(value, "level price")?;
            side.qty[level] = level_qty;
            if level > 0 {
                let (near, far) = (side.price[level - 1], side.price[level]);
                if (self.rising && far <= near) || (!self.rising && far >= near) {
                    return Err("level prices are not strictly ordered from the best".to_owned());
                }
            }
        }
        Ok(Some(side))
    }
}

struct BookColumns<'a> {
    symbol: &'a StringArray,
    exch_ts: &'a PrimitiveArray<TimestampMicrosecondType>,
    seq: &'a PrimitiveArray<Int64Type>,
    phase: &'a StringArray,
    has_trade: &'a BooleanArray,
    bid: SideColumns<'a>,
    ask: SideColumns<'a>,
    is_indicative: &'a BooleanArray,
    indicative_px: &'a PrimitiveArray<Float64Type>,
    indicative_qty: &'a PrimitiveArray<Int64Type>,
    delay_open: &'a BooleanArray,
    delay_close: &'a BooleanArray,
    delay_state: &'a PrimitiveArray<Int8Type>,
}

impl<'a> BookColumns<'a> {
    fn new(path: &Path, batch: &'a RecordBatch) -> Result<Self, MdError> {
        Ok(Self {
            symbol: strings(path, batch, "symbol")?,
            exch_ts: primitive(path, batch, "exch_ts")?,
            seq: primitive(path, batch, "seq")?,
            phase: strings(path, batch, "phase")?,
            has_trade: boolean(path, batch, "has_trade")?,
            bid: SideColumns::new(path, batch, "bid")?,
            ask: SideColumns::new(path, batch, "ask")?,
            is_indicative: boolean(path, batch, "is_indicative")?,
            indicative_px: primitive(path, batch, "indicative_px")?,
            indicative_qty: primitive(path, batch, "indicative_qty")?,
            delay_open: boolean(path, batch, "delay_open")?,
            delay_close: boolean(path, batch, "delay_close")?,
            delay_state: primitive(path, batch, "delay_state")?,
        })
    }

    /// Decodes everything of `row` except its identity (`seq`, `exch_ts`, `symbol`).
    fn body(&self, path: &Path, row: usize) -> Result<Result<BookRow, String>, MdError> {
        let phase = match phase(path, self.phase, row)? {
            Ok(phase) => phase,
            Err(reason) => return Ok(Err(reason)),
        };
        let has_trade = required(path, optional_bool(self.has_trade, row), "has_trade")?;
        let is_indicative = required(
            path,
            optional_bool(self.is_indicative, row),
            "is_indicative",
        )?;
        let delay_state = required(path, optional(self.delay_state, row), "delay_state")?;
        Ok(self.classify(row, phase, has_trade, is_indicative, delay_state))
    }

    fn classify(
        &self,
        row: usize,
        phase: Phase,
        has_trade: bool,
        is_indicative: bool,
        delay_state: i8,
    ) -> Result<BookRow, String> {
        let bid = self.bid.row(row)?;
        let ask = self.ask.row(row)?;
        let indicative_price = optional(self.indicative_px, row);
        let indicative_qty = quantity(optional(self.indicative_qty, row), "indicative qty")?;
        let delay_open = optional_bool(self.delay_open, row);
        let delay_close = optional_bool(self.delay_close, row);
        let mut decoded = BookRow {
            seq: 0,
            exch_ts: 0,
            symbol: 0,
            phase,
            kind: BookKind::Indicative,
            bid: CompactSide::EMPTY,
            ask: CompactSide::EMPTY,
            indicative_price: 0,
            indicative_qty: 0,
            delay_open,
            delay_close,
        };

        if delay_state != 0 {
            decoded.kind = BookKind::Delay(match delay_state {
                1 => DelayDirection::Up,
                -1 => DelayDirection::Down,
                other => return Err(format!("unknown delay_state {other}")),
            });
            if has_trade || is_indicative {
                return Err("a delay row is marked as a trade or as indicative".to_owned());
            }
            if bid.is_some() || ask.is_some() {
                return Err("a delay row discloses a book".to_owned());
            }
            if indicative_price.is_some() || indicative_qty.is_some() {
                return Err("a delay row carries an indicative match".to_owned());
            }
        } else {
            let (Some(bid), Some(ask)) = (bid, ask) else {
                return Err("a row without delay_state discloses no book".to_owned());
            };
            if bid.len > 0 && ask.len > 0 && bid.price[0] >= ask.price[0] {
                return Err("best bid is not below best ask".to_owned());
            }
            decoded.bid = bid;
            decoded.ask = ask;
            if is_indicative {
                if has_trade {
                    return Err("an indicative row is marked as a trade".to_owned());
                }
                match (indicative_price, indicative_qty) {
                    (None, None) => {}
                    (Some(value), Some(qty)) => {
                        decoded.indicative_price = price(value, "indicative price")?;
                        decoded.indicative_qty = qty;
                    }
                    _ => return Err("indicative price and qty are not both present".to_owned()),
                }
            } else {
                if indicative_price.is_some() || indicative_qty.is_some() {
                    return Err("a non-indicative row carries an indicative match".to_owned());
                }
                decoded.kind = BookKind::Disclosure { has_trade };
            }
        }
        if !is_indicative && (delay_open == Some(true) || delay_close == Some(true)) {
            return Err("a non-indicative row announces a delayed open or close".to_owned());
        }
        Ok(decoded)
    }
}

/// Reads the selected symbols' rows of a `books` file, in file order.
pub(crate) fn read_books(
    file: ParquetFile,
    context: &DecodeContext<'_>,
    table: &mut SymbolTable,
) -> Result<Vec<BookRow>, MdError> {
    let row_groups = context.row_groups(&file);
    let mut rows = Vec::new();
    let mut runs = SymbolRuns::new();
    file.read(BOOK_COLUMNS, row_groups, |path, batch| {
        let columns = BookColumns::new(path, batch)?;
        for row in 0..batch.num_rows() {
            let code = required(path, optional_str(columns.symbol, row), "symbol")?;
            let seq = sequence(path, optional(columns.seq, row))?;
            let Some(symbol) = runs.enter(path, code, seq, context, table)? else {
                continue;
            };
            let decoded = columns
                .body(path, row)?
                .map_err(|reason| MdError::InvalidRow {
                    path: path.to_path_buf(),
                    symbol: code.to_owned(),
                    seq,
                    reason,
                })?;
            rows.push(BookRow {
                seq,
                exch_ts: required(path, optional(columns.exch_ts, row), "exch_ts")?,
                symbol,
                ..decoded
            });
        }
        Ok(())
    })?;
    mark_intermediate_fills(&mut rows);
    Ok(rows)
}

/// Marks the fills of a sweep still in progress that `books` records with an empty book.
///
/// One order that trades at several prices produces one message per price, all with the
/// same `exch_ts`; only the last discloses the book. marketdata keeps the earlier ones out
/// of `books`, except on days converted from a source without the exchange's continuation
/// flag: there each is a `has_trade` row with zero levels on both sides, which is also how
/// a trade that really empties the book looks.
///
/// The symbol's next message tells them apart. A sweep continues with another fill, or with
/// a delay row when it stops at the price-stabilisation band, at the same `exch_ts`; nothing
/// can trade against a book that is empty.
fn mark_intermediate_fills(rows: &mut [BookRow]) {
    for index in 1..rows.len() {
        let next = rows[index];
        let row = &mut rows[index - 1];
        let continues = next.symbol == row.symbol
            && next.exch_ts == row.exch_ts
            && (next.kind.has_trade() || matches!(next.kind, BookKind::Delay(_)));
        if continues
            && row.kind == (BookKind::Disclosure { has_trade: true })
            && row.phase == Phase::Continuous
            && row.bid == CompactSide::EMPTY
            && row.ask == CompactSide::EMPTY
        {
            row.kind = BookKind::IntermediateTrade;
        }
    }
}

/// Reads the selected symbols' rows of a `trades` file, in file order.
pub(crate) fn read_trades(
    file: ParquetFile,
    context: &DecodeContext<'_>,
    table: &mut SymbolTable,
) -> Result<Vec<TradeRow>, MdError> {
    let row_groups = context.row_groups(&file);
    let mut rows = Vec::new();
    let mut runs = SymbolRuns::new();
    file.read(TRADE_COLUMNS, row_groups, |path, batch| {
        let symbols = strings(path, batch, "symbol")?;
        let exch_ts = primitive::<TimestampMicrosecondType>(path, batch, "exch_ts")?;
        let seqs = primitive::<Int64Type>(path, batch, "seq")?;
        let sub_seq = primitive::<Int16Type>(path, batch, "sub_seq")?;
        let prices = primitive::<Float64Type>(path, batch, "price")?;
        let qty = primitive::<Int64Type>(path, batch, "qty")?;
        let cum_qty = primitive::<Int64Type>(path, batch, "cum_qty")?;
        let phases = strings(path, batch, "phase")?;
        for row in 0..batch.num_rows() {
            let code = required(path, optional_str(symbols, row), "symbol")?;
            let seq = sequence(path, optional(seqs, row))?;
            let Some(symbol) = runs.enter(path, code, seq, context, table)? else {
                continue;
            };
            let sub_seq = required(path, optional(sub_seq, row), "sub_seq")?;
            let trade_price = required(path, optional(prices, row), "price")?;
            let trade_qty = required(path, optional(qty, row), "qty")?;
            let phase = phase(path, phases, row)?;
            let decoded = (|| {
                if sub_seq != 0 {
                    return Err(format!("sub_seq {sub_seq}: several fills under one seq"));
                }
                Ok(TradeRow {
                    seq,
                    exch_ts: 0,
                    symbol,
                    phase: phase?,
                    price: price(trade_price, "trade price")?,
                    qty: quantity(Some(trade_qty), "trade qty")?.expect("value is present"),
                    cum_qty: match optional(cum_qty, row) {
                        None => None,
                        Some(value) => Some(
                            u64::try_from(value)
                                .ok()
                                .filter(|value| *value > 0)
                                .ok_or_else(|| format!("cum_qty {value} is not positive"))?,
                        ),
                    },
                })
            })()
            .map_err(|reason| MdError::InvalidRow {
                path: path.to_path_buf(),
                symbol: code.to_owned(),
                seq,
                reason,
            })?;
            rows.push(TradeRow {
                exch_ts: required(path, optional(exch_ts, row), "exch_ts")?,
                ..decoded
            });
        }
        Ok(())
    })?;
    Ok(rows)
}

/// Observed time of each `selected` seq over the whole feed.
///
/// The observed time of message `s` is the largest `exch_ts` among every message the
/// feed disseminated with `seq <= s`, including symbols outside the selection, so it
/// does not depend on which symbols are read. `selected` must be sorted and unique;
/// the result is aligned with it.
pub(crate) fn observed_times(
    files: impl IntoIterator<Item = ParquetFile>,
    selected: &[u64],
) -> Result<Vec<i64>, MdError> {
    const COLUMNS: &[Column] = &[
        ("seq", ColumnType::Int64),
        ("exch_ts", ColumnType::TimestampMicrosUtc),
    ];
    let mut bucket_max = vec![i64::MIN; selected.len()];
    let Some(&last) = selected.last() else {
        return Ok(bucket_max);
    };
    for file in files {
        file.read(COLUMNS, None, |path, batch| {
            let seq = primitive::<Int64Type>(path, batch, "seq")?;
            let exch_ts = primitive::<TimestampMicrosecondType>(path, batch, "exch_ts")?;
            for row in 0..batch.num_rows() {
                let seq = sequence(path, optional(seq, row))?;
                let time = required(path, optional(exch_ts, row), "exch_ts")?;
                if seq > last {
                    continue;
                }
                // Message `seq` is visible to every selected seq at or after it.
                let bucket = selected.partition_point(|&selected| selected < seq);
                let slot = &mut bucket_max[bucket];
                *slot = (*slot).max(time);
            }
            Ok(())
        })?;
    }
    let mut running = i64::MIN;
    for slot in &mut bucket_max {
        running = running.max(*slot);
        *slot = running;
    }
    Ok(bucket_max)
}
