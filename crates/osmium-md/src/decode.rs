//! Decoding of `books` and `trades` rows into compact, validated records.
//!
//! Records keep the file's native quantity unit; [`crate::FeedDay`] converts to shares
//! when it builds a message.
//!
//! A file is decoded one row group at a time, each on its own thread. What a row group
//! cannot decide alone — the order of its rows relative to its neighbours', and whether
//! its last row is an intermediate fill — is left to [`crate::FeedDay`], which puts the
//! row groups together in file order.

use std::path::Path;

use arrow_array::{
    BooleanArray, PrimitiveArray, RecordBatch, StringArray,
    types::{Float64Type, Int8Type, Int16Type, Int64Type, TimestampMicrosecondType},
};
use osmium_types::{BOOK_DEPTH, DelayDirection, Phase, Symbol};

use crate::{
    MdError,
    order::pack_seq,
    parallel,
    parquet_io::{
        Column, ColumnType, ParquetFile, boolean, optional, optional_bool, optional_str,
        price_units, primitive, required, strings,
    },
    stream::{Frame, encode},
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
    pub(crate) const EMPTY: Self = Self {
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

/// One `books` row, without its `symbol` and `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BookRow {
    pub(crate) exch_ts: i64,
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
    /// Index of the symbol: among the chunk's runs when a chunk is read, among the feed
    /// day's symbols once [`crate::FeedDay`] has put the chunks together.
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

/// What a decode needs besides the file.
pub(crate) struct DecodeContext<'a> {
    /// Sorted, unique symbols to keep; `None` keeps every symbol.
    pub(crate) selection: Option<&'a [Symbol]>,
    /// Shares per native unit of a symbol, and whether that value is an assumption.
    pub(crate) resolve: &'a (dyn Fn(Symbol) -> Result<(u32, bool), MdError> + Sync),
}

impl DecodeContext<'_> {
    /// The row groups of `file` that may hold selected symbols, in file order.
    pub(crate) fn row_groups(&self, file: &ParquetFile) -> Vec<usize> {
        match self.selection {
            None => (0..file.num_row_groups()).collect(),
            Some(symbols) => {
                let codes: Vec<&str> = symbols.iter().map(Symbol::as_str).collect();
                file.row_groups_for_symbols(&codes)
            }
        }
    }
}

/// The kept rows of one symbol in a chunk; they are consecutive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) entry: SymbolEntry,
    pub(crate) rows: usize,
}

/// The `(symbol, seq)` of the first and of the last row of a chunk, kept or not. The
/// rows between them are in order; whether the chunk follows the one before it is for
/// the caller to check with [`Edge::follows`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Edge {
    pub(crate) first: Option<(Symbol, u64)>,
    pub(crate) last: Option<(Symbol, u64)>,
}

impl Edge {
    /// Checks that a chunk starting at `first` may follow a row `previous`.
    pub(crate) fn follows(
        path: &Path,
        previous: (Symbol, u64),
        first: (Symbol, u64),
    ) -> Result<(), MdError> {
        let reason = if previous.0 > first.0 {
            "rows are not sorted by symbol"
        } else if previous.0 == first.0 && previous.1 >= first.1 {
            "rows of one symbol are not in strictly increasing seq order"
        } else {
            return Ok(());
        };
        Err(MdError::InvalidRow {
            path: path.to_path_buf(),
            symbol: first.0.to_string(),
            seq: first.1,
            reason: reason.to_owned(),
        })
    }
}

/// What entering a row found.
enum Entered {
    /// The row's symbol is not selected.
    Skip,
    /// The row continues the current run.
    Same,
    /// The row starts the run of a selected symbol.
    New(SymbolEntry),
}

/// Walks the `symbol` column of a chunk of a file sorted by `(symbol, seq)`.
struct SymbolRuns {
    code: String,
    symbol: Option<Symbol>,
    selected: bool,
    last_seq: Option<u64>,
    first: Option<(Symbol, u64)>,
}

impl SymbolRuns {
    const fn new() -> Self {
        Self {
            code: String::new(),
            symbol: None,
            selected: false,
            last_seq: None,
            first: None,
        }
    }

    fn edge(&self) -> Edge {
        Edge {
            first: self.first,
            last: self.symbol.zip(self.last_seq),
        }
    }

    /// Advances to a row.
    fn enter(
        &mut self,
        path: &Path,
        code: &str,
        seq: u64,
        context: &DecodeContext<'_>,
    ) -> Result<Entered, MdError> {
        let invalid = |reason: String| MdError::InvalidRow {
            path: path.to_path_buf(),
            symbol: code.to_owned(),
            seq,
            reason,
        };
        let mut entered = Entered::Same;
        if self.symbol.is_none() || self.code != code {
            let symbol = Symbol::new(code).map_err(|error| invalid(error.to_string()))?;
            if self.first.is_none() {
                self.first = Some((symbol, seq));
            }
            if self.symbol.is_some_and(|previous| previous >= symbol) {
                return Err(invalid("rows are not sorted by symbol".to_owned()));
            }
            code.clone_into(&mut self.code);
            self.symbol = Some(symbol);
            self.last_seq = None;
            self.selected = context
                .selection
                .is_none_or(|symbols| symbols.binary_search(&symbol).is_ok());
            if self.selected {
                let (shares_per_unit, assumed) = (context.resolve)(symbol)?;
                entered = Entered::New(SymbolEntry {
                    symbol,
                    shares_per_unit,
                    assumed,
                });
            }
        }
        if self.last_seq.is_some_and(|last| last >= seq) {
            return Err(invalid(
                "rows of one symbol are not in strictly increasing seq order".to_owned(),
            ));
        }
        self.last_seq = Some(seq);
        Ok(if self.selected {
            entered
        } else {
            Entered::Skip
        })
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
            exch_ts: 0,
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

pub(crate) const fn book_columns() -> &'static [Column] {
    BOOK_COLUMNS
}

pub(crate) const fn trade_columns() -> &'static [Column] {
    TRADE_COLUMNS
}

/// A kept `books` row that a `trades` row must share the seq of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Joined {
    /// Index of the symbol, as in [`TradeRow::symbol`].
    pub(crate) symbol: u32,
    pub(crate) seq: u64,
    pub(crate) exch_ts: i64,
    pub(crate) phase: Phase,
}

/// The kept rows of one row group of a `books` file.
#[derive(Debug, Default)]
pub(crate) struct BookChunk {
    /// The symbols with kept rows, in file order.
    pub(crate) runs: Vec<Run>,
    /// The bytes each run has in `bytes`.
    pub(crate) run_bytes: Vec<usize>,
    /// Every kept row but the last, encoded by [`crate::stream`], run after run. The
    /// first row of every run is written from [`Frame::START`].
    pub(crate) bytes: Vec<u8>,
    /// The seq of every kept row and whether it has a trade, in file order, as
    /// [`crate::order::pack_seq`] writes them.
    pub(crate) seqs: Vec<u8>,
    /// The kept rows with `has_trade`, in file order.
    pub(crate) joined: Vec<Joined>,
    /// `exch_ts` and kind of the first kept row.
    pub(crate) head: Option<(i64, BookKind)>,
    /// The last kept row. Whether it is an intermediate fill depends on the row after
    /// it, which is in another chunk.
    pub(crate) tail: Option<BookRow>,
    pub(crate) edge: Edge,
    /// Why decoding stopped early. The fields above describe the rows before it.
    pub(crate) error: Option<MdError>,
}

/// Whether `row` is a fill of a sweep still in progress that `books` records with an
/// empty book, given the row after it: a row of the same symbol when `same_symbol`.
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
pub(crate) fn is_intermediate_fill(
    row: &BookRow,
    same_symbol: bool,
    next_exch_ts: i64,
    next_kind: BookKind,
) -> bool {
    same_symbol
        && next_exch_ts == row.exch_ts
        && (next_kind.has_trade() || matches!(next_kind, BookKind::Delay(_)))
        && row.kind == (BookKind::Disclosure { has_trade: true })
        && row.phase == Phase::Continuous
        && row.bid == CompactSide::EMPTY
        && row.ask == CompactSide::EMPTY
}

/// Reads the selected symbols' rows of one row group of a `books` file.
pub(crate) fn read_book_chunk(
    file: &ParquetFile,
    row_group: usize,
    context: &DecodeContext<'_>,
) -> BookChunk {
    let mut chunk = BookChunk::default();
    let mut runs = SymbolRuns::new();
    let mut frame = Frame::START;
    // The row before the current one, waiting for it to tell whether it is intermediate.
    let mut pending: Option<BookRow> = None;
    let mut starts_run = true;
    // The seq of the kept row before the current one in its run.
    let mut previous_seq = 0;
    let result = file.read(BOOK_COLUMNS, Some(vec![row_group]), |path, batch| {
        let columns = BookColumns::new(path, batch)?;
        for row in 0..batch.num_rows() {
            let code = required(path, optional_str(columns.symbol, row), "symbol")?;
            let seq = sequence(path, optional(columns.seq, row))?;
            let new_run = match runs.enter(path, code, seq, context)? {
                Entered::Skip => continue,
                Entered::Same => None,
                Entered::New(entry) => Some(entry),
            };
            let decoded = columns
                .body(path, row)?
                .map_err(|reason| MdError::InvalidRow {
                    path: path.to_path_buf(),
                    symbol: code.to_owned(),
                    seq,
                    reason,
                })?;
            let decoded = BookRow {
                exch_ts: required(path, optional(columns.exch_ts, row), "exch_ts")?,
                ..decoded
            };
            if let Some(mut previous) = pending.take() {
                if is_intermediate_fill(&previous, new_run.is_none(), decoded.exch_ts, decoded.kind)
                {
                    previous.kind = BookKind::IntermediateTrade;
                }
                let before = chunk.bytes.len();
                encode(&mut chunk.bytes, &mut frame, starts_run, &previous);
                starts_run = false;
                *chunk.run_bytes.last_mut().expect("the row has a run") +=
                    chunk.bytes.len() - before;
            }
            if let Some(entry) = new_run {
                chunk.runs.push(Run { entry, rows: 0 });
                chunk.run_bytes.push(0);
                starts_run = true;
                previous_seq = 0;
            }
            let has_trade = decoded.kind.has_trade();
            if has_trade {
                chunk.joined.push(Joined {
                    symbol: u32::try_from(chunk.runs.len() - 1).expect("fewer than 2^32 runs"),
                    seq,
                    exch_ts: decoded.exch_ts,
                    phase: decoded.phase,
                });
            }
            if chunk.head.is_none() {
                chunk.head = Some((decoded.exch_ts, decoded.kind));
            }
            pack_seq(&mut chunk.seqs, previous_seq, seq, has_trade);
            previous_seq = seq;
            chunk.runs.last_mut().expect("the row has a run").rows += 1;
            pending = Some(decoded);
        }
        Ok(())
    });
    chunk.tail = pending;
    chunk.edge = runs.edge();
    chunk.error = result.err();
    chunk
}

/// The kept rows of one row group of a `trades` file.
#[derive(Debug, Default)]
pub(crate) struct TradeChunk {
    /// The symbols with kept rows, in file order.
    pub(crate) runs: Vec<Run>,
    /// The kept rows in file order; `symbol` is an index into `runs`.
    pub(crate) rows: Vec<TradeRow>,
    pub(crate) edge: Edge,
    /// Why decoding stopped early.
    pub(crate) error: Option<MdError>,
}

/// Reads the selected symbols' rows of one row group of a `trades` file.
pub(crate) fn read_trade_chunk(
    file: &ParquetFile,
    row_group: usize,
    context: &DecodeContext<'_>,
) -> TradeChunk {
    let mut chunk = TradeChunk::default();
    let mut runs = SymbolRuns::new();
    let result = file.read(TRADE_COLUMNS, Some(vec![row_group]), |path, batch| {
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
            match runs.enter(path, code, seq, context)? {
                Entered::Skip => continue,
                Entered::Same => {}
                Entered::New(entry) => chunk.runs.push(Run { entry, rows: 0 }),
            }
            let symbol = u32::try_from(chunk.runs.len() - 1).expect("fewer than 2^32 runs");
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
            chunk.rows.push(TradeRow {
                exch_ts: required(path, optional(exch_ts, row), "exch_ts")?,
                ..decoded
            });
            chunk.runs.last_mut().expect("the row has a run").rows += 1;
        }
        Ok(())
    });
    chunk.edge = runs.edge();
    chunk.error = result.err();
    chunk
}

/// Observed time of each `selected` seq over the whole feed.
///
/// The observed time of message `s` is the largest `exch_ts` among every message the
/// feed disseminated with `seq <= s`, including symbols outside the selection, so it
/// does not depend on which symbols are read. `selected` must be sorted and unique;
/// the result is aligned with it.
pub(crate) fn observed_times(
    threads: usize,
    files: [&ParquetFile; 2],
    selected: &[u64],
) -> Result<Vec<i64>, MdError> {
    const COLUMNS: &[Column] = &[
        ("seq", ColumnType::Int64),
        ("exch_ts", ColumnType::TimestampMicrosUtc),
    ];
    /// What one thread has seen: the largest `exch_ts` that becomes visible at each
    /// selected seq, and the first piece that failed.
    struct Seen {
        bucket_max: Vec<i64>,
        error: Option<(usize, MdError)>,
    }
    let Some(&last) = selected.last() else {
        return Ok(Vec::new());
    };
    let pieces: Vec<(&ParquetFile, usize)> = files
        .into_iter()
        .flat_map(|file| (0..file.num_row_groups()).map(move |row_group| (file, row_group)))
        .collect();
    let seen = parallel::fold(
        threads,
        pieces.len(),
        || Seen {
            bucket_max: vec![i64::MIN; selected.len()],
            error: None,
        },
        |seen, piece| {
            let (file, row_group) = pieces[piece];
            let bucket_max = &mut seen.bucket_max;
            let result = file.read(COLUMNS, Some(vec![row_group]), |path, batch| {
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
            });
            if let Err(error) = result
                && seen.error.as_ref().is_none_or(|(first, _)| piece < *first)
            {
                seen.error = Some((piece, error));
            }
        },
    );
    let mut bucket_max = vec![i64::MIN; selected.len()];
    let mut first_error: Option<(usize, MdError)> = None;
    for seen in seen {
        if let Some((piece, error)) = seen.error
            && first_error.as_ref().is_none_or(|(first, _)| piece < *first)
        {
            first_error = Some((piece, error));
        }
        for (slot, value) in bucket_max.iter_mut().zip(seen.bucket_max) {
            *slot = (*slot).max(value);
        }
    }
    if let Some((_, error)) = first_error {
        return Err(error);
    }
    let mut running = i64::MIN;
    for slot in &mut bucket_max {
        running = running.max(*slot);
        *slot = running;
    }
    Ok(bucket_max)
}
