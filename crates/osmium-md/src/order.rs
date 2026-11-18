//! The order of a feed day's messages: which symbol's rows make the message of each
//! `seq`.
//!
//! `books` and `trades` are sorted by `(symbol, seq)`; the messages are wanted by `seq`.
//! Building the order is also where the two tables are joined: a `trades` row and a
//! `books` row with the same seq are one message.

use std::path::{Path, PathBuf};

use osmium_types::{Phase, Symbol};

use crate::{
    MdError,
    decode::{Joined, TradeRow},
    stream::{put_varint, varint},
};

/// The message has a `books` row.
pub(crate) const BOOK: u32 = 1;
/// The message has a `trades` row.
pub(crate) const TRADE: u32 = 1 << 1;
/// The `books` row says a `trades` row shares its seq. Only read while building.
const EXPECTS: u32 = 1 << 2;
const SYMBOL_SHIFT: u32 = 3;

/// The largest number of symbols an entry can name.
pub(crate) const MAX_SYMBOLS: usize = 1 << (32 - SYMBOL_SHIFT);

/// The index of an entry's symbol among the feed day's symbols.
pub(crate) const fn symbol_of(entry: u32) -> usize {
    (entry >> SYMBOL_SHIFT) as usize
}

/// The kept `books` rows of one chunk in file order.
pub(crate) struct BookSeqs {
    /// The symbol index and number of rows of each run.
    pub(crate) runs: Vec<(u32, usize)>,
    /// The seq of every row and whether it has a trade, written by [`pack_seq`].
    pub(crate) packed: Vec<u8>,
}

/// Appends a `books` row to [`BookSeqs::packed`]. `previous` is the seq of the row before
/// it in the same run, which is smaller, or 0 for the first row of a run.
pub(crate) fn pack_seq(packed: &mut Vec<u8>, previous: u64, seq: u64, has_trade: bool) {
    put_varint(packed, (seq - previous) << 1 | u64::from(has_trade));
}

impl BookSeqs {
    /// The symbol index, seq and `has_trade` of every row.
    fn rows(&self) -> BookRows<'_> {
        BookRows {
            chunk: self,
            run: 0,
            left: 0,
            at: 0,
            seq: 0,
        }
    }

    fn len(&self) -> usize {
        self.runs.iter().map(|run| run.1).sum()
    }
}

struct BookRows<'a> {
    chunk: &'a BookSeqs,
    /// The number of runs entered.
    run: usize,
    /// The rows left in the run entered last.
    left: usize,
    at: usize,
    seq: u64,
}

impl Iterator for BookRows<'_> {
    type Item = (u32, u64, bool);

    fn next(&mut self) -> Option<Self::Item> {
        while self.left == 0 {
            self.left = self.chunk.runs.get(self.run)?.1;
            self.run += 1;
            self.seq = 0;
        }
        self.left -= 1;
        let packed = varint(&self.chunk.packed, &mut self.at);
        self.seq += packed >> 1;
        Some((self.chunk.runs[self.run - 1].0, self.seq, packed & 1 == 1))
    }
}

/// What [`Order::build`] reads.
pub(crate) struct Rows<'a> {
    pub(crate) books: Vec<BookSeqs>,
    /// The `books` rows with `has_trade`, in file order.
    pub(crate) joined: &'a [Joined],
    /// The `trades` rows in file order.
    pub(crate) trades: &'a [TradeRow],
    /// The feed day's symbols by index.
    pub(crate) symbols: &'a [Symbol],
    pub(crate) books_path: &'a Path,
    pub(crate) trades_path: &'a Path,
}

impl Rows<'_> {
    fn invalid(&self, path: &Path, symbol: usize, seq: u64, reason: &str) -> MdError {
        MdError::InvalidRow {
            path: PathBuf::from(path),
            symbol: self.symbols[symbol].to_string(),
            seq,
            reason: reason.to_owned(),
        }
    }

    fn book_rows(&self) -> usize {
        self.books.iter().map(BookSeqs::len).sum()
    }

    /// The smallest and largest seq of any row.
    fn span(&self) -> Option<(u64, u64)> {
        let books = self
            .books
            .iter()
            .flat_map(BookSeqs::rows)
            .map(|(_, seq, _)| seq);
        let trades = self.trades.iter().map(|row| row.seq);
        books.chain(trades).fold(None, |span, seq| {
            Some(span.map_or((seq, seq), |(low, high): (u64, u64)| {
                (low.min(seq), high.max(seq))
            }))
        })
    }
}

/// The messages of a feed day in increasing seq.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Order {
    /// The seq of each entry; empty when entry `i` has seq `base + i`.
    seqs: Vec<u64>,
    base: u64,
    /// Per message its symbol and which tables have a row of it. Zero where the seq of
    /// that position has no message.
    entries: Vec<u32>,
    /// The number of messages.
    len: usize,
}

const DISAGREE: &str = "the trades row and the books row of one seq disagree";
const UNEXPECTED_TRADE: &str = "a trades row shares the seq of a row without has_trade";
const MISSING_TRADE: &str = "has_trade without a trades row";
const TRADE_OUTSIDE_CONTINUOUS: &str = "a trade without a books row outside continuous trading";
const SHARED_SEQ: &str = "two rows share one seq";

impl Order {
    /// Joins the rows of the two tables into messages.
    pub(crate) fn build(rows: Rows<'_>) -> Result<Self, MdError> {
        let count = rows.book_rows() + rows.trades.len();
        let Some((low, high)) = rows.span() else {
            return Ok(Self {
                seqs: Vec::new(),
                base: 0,
                entries: Vec::new(),
                len: 0,
            });
        };
        // An exchange numbers a feed's messages 1, 2, 3…, so a whole feed fills a table
        // indexed by seq. A selection of symbols leaves most of it empty: then sorting
        // the rows takes less memory than the table.
        let span = (high - low).saturating_add(1);
        match usize::try_from(span) {
            Ok(span) if span / 4 <= count => Self::dense(&rows, low, span),
            _ => Self::sparse(&rows),
        }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    /// The number of positions; some may hold no message.
    pub(crate) const fn positions(&self) -> usize {
        self.entries.len()
    }

    /// The seq and entry at `position`; `None` when that seq has no message.
    pub(crate) fn get(&self, position: usize) -> Option<(u64, u32)> {
        let entry = self.entries[position];
        (entry != 0).then(|| {
            let seq = match self.seqs.get(position) {
                Some(&seq) => seq,
                None => self.base + position as u64,
            };
            (seq, entry)
        })
    }

    /// The seq of every message, increasing.
    pub(crate) fn message_seqs(&self) -> Vec<u64> {
        (0..self.positions())
            .filter_map(|position| self.get(position).map(|(seq, _)| seq))
            .collect()
    }

    fn entry(symbol: u32, flags: u32) -> u32 {
        (symbol << SYMBOL_SHIFT) | flags
    }

    /// Places every row in a table indexed by `seq - low`.
    fn dense(rows: &Rows<'_>, low: u64, span: usize) -> Result<Self, MdError> {
        let mut entries = vec![0_u32; span];
        #[allow(clippy::cast_possible_truncation)]
        let position = |seq: u64| (seq - low) as usize;
        let mut len = 0;
        for (symbol, seq, has_trade) in rows.books.iter().flat_map(BookSeqs::rows) {
            let slot = &mut entries[position(seq)];
            if *slot != 0 {
                return Err(rows.invalid(rows.books_path, symbol as usize, seq, SHARED_SEQ));
            }
            let expects = if has_trade { EXPECTS } else { 0 };
            *slot = Self::entry(symbol, BOOK | expects);
            len += 1;
        }

        // Both tables are sorted by `(symbol, seq)`, so the `books` rows that expect a
        // trade come up in the order the `trades` rows do.
        let key = |symbol: u32, seq: u64| (rows.symbols[symbol as usize], seq);
        let mut next_joined = 0;
        let mut joined = 0;
        for trade in rows.trades {
            let slot = &mut entries[position(trade.seq)];
            let symbol = trade.symbol as usize;
            if *slot == 0 {
                if trade.phase != Phase::Continuous {
                    return Err(rows.invalid(
                        rows.trades_path,
                        symbol,
                        trade.seq,
                        TRADE_OUTSIDE_CONTINUOUS,
                    ));
                }
                *slot = Self::entry(trade.symbol, TRADE);
                len += 1;
                continue;
            }
            if *slot & TRADE != 0 {
                return Err(rows.invalid(rows.trades_path, symbol, trade.seq, SHARED_SEQ));
            }
            if *slot & EXPECTS == 0 {
                return Err(rows.invalid(
                    rows.books_path,
                    symbol_of(*slot),
                    trade.seq,
                    UNEXPECTED_TRADE,
                ));
            }
            let disagree = || rows.invalid(rows.trades_path, symbol, trade.seq, DISAGREE);
            if symbol_of(*slot) != symbol {
                return Err(disagree());
            }
            let wanted = key(trade.symbol, trade.seq);
            while rows
                .joined
                .get(next_joined)
                .is_some_and(|book| key(book.symbol, book.seq) < wanted)
            {
                next_joined += 1;
            }
            let book = rows
                .joined
                .get(next_joined)
                .filter(|book| key(book.symbol, book.seq) == wanted)
                .expect("the entry was placed by a books row that expects a trade");
            if book.phase != trade.phase || book.exch_ts != trade.exch_ts {
                return Err(disagree());
            }
            *slot |= TRADE;
            joined += 1;
        }
        if joined != rows.joined.len() {
            let (position, entry) = entries
                .iter()
                .enumerate()
                .find(|(_, entry)| **entry & (EXPECTS | TRADE) == EXPECTS)
                .expect("fewer trades joined than books rows expect one");
            return Err(rows.invalid(
                rows.books_path,
                symbol_of(*entry),
                low + position as u64,
                MISSING_TRADE,
            ));
        }
        Ok(Self {
            seqs: Vec::new(),
            base: low,
            entries,
            len,
        })
    }

    /// Sorts the rows of each table by seq and merges them.
    fn sparse(rows: &Rows<'_>) -> Result<Self, MdError> {
        let mut books: Vec<(u64, u32)> = rows
            .books
            .iter()
            .flat_map(BookSeqs::rows)
            .map(|(symbol, seq, has_trade)| {
                let expects = if has_trade { EXPECTS } else { 0 };
                (seq, Self::entry(symbol, BOOK | expects))
            })
            .collect();
        books.sort_unstable();
        let mut trades: Vec<&TradeRow> = rows.trades.iter().collect();
        trades.sort_unstable_by_key(|row| (row.seq, row.symbol));
        let mut joined: Vec<&Joined> = rows.joined.iter().collect();
        joined.sort_unstable_by_key(|book| (book.seq, book.symbol));

        let mut seqs = Vec::with_capacity(books.len() + trades.len());
        let mut entries = Vec::with_capacity(books.len() + trades.len());
        let mut last_seq = None;
        let (mut next_book, mut next_trade, mut next_joined) = (0, 0, 0);
        loop {
            let book = books.get(next_book).copied();
            let trade = trades.get(next_trade).copied();
            let (book, trade) = match (book, trade) {
                (None, None) => break,
                (Some(book), Some(trade)) if book.0 == trade.seq => (Some(book), Some(trade)),
                (Some(book), Some(trade)) if book.0 < trade.seq => (Some(book), None),
                (Some(book), None) => (Some(book), None),
                (_, Some(trade)) => (None, Some(trade)),
            };
            next_book += usize::from(book.is_some());
            next_trade += usize::from(trade.is_some());

            let (seq, entry) = match (book, trade) {
                (Some((seq, entry)), Some(trade)) => {
                    if entry & EXPECTS == 0 {
                        return Err(rows.invalid(
                            rows.books_path,
                            symbol_of(entry),
                            seq,
                            UNEXPECTED_TRADE,
                        ));
                    }
                    while joined.get(next_joined).is_some_and(|book| {
                        (book.seq, book.symbol as usize) < (seq, symbol_of(entry))
                    }) {
                        next_joined += 1;
                    }
                    let agrees = symbol_of(entry) == trade.symbol as usize
                        && joined.get(next_joined).is_some_and(|book| {
                            book.seq == seq
                                && book.symbol == trade.symbol
                                && book.phase == trade.phase
                                && book.exch_ts == trade.exch_ts
                        });
                    if !agrees {
                        return Err(rows.invalid(
                            rows.trades_path,
                            trade.symbol as usize,
                            seq,
                            DISAGREE,
                        ));
                    }
                    (seq, entry | TRADE)
                }
                (Some((seq, entry)), None) => {
                    if entry & EXPECTS != 0 {
                        return Err(rows.invalid(
                            rows.books_path,
                            symbol_of(entry),
                            seq,
                            MISSING_TRADE,
                        ));
                    }
                    (seq, entry)
                }
                (None, Some(trade)) => {
                    if trade.phase != Phase::Continuous {
                        return Err(rows.invalid(
                            rows.trades_path,
                            trade.symbol as usize,
                            trade.seq,
                            TRADE_OUTSIDE_CONTINUOUS,
                        ));
                    }
                    (trade.seq, Self::entry(trade.symbol, TRADE))
                }
                (None, None) => unreachable!("the loop ended above"),
            };
            if last_seq.is_some_and(|last| last >= seq) {
                let (path, symbol) = match trade {
                    Some(trade) if book.is_none() => (rows.trades_path, trade.symbol as usize),
                    _ => (rows.books_path, symbol_of(entry)),
                };
                return Err(rows.invalid(path, symbol, seq, SHARED_SEQ));
            }
            last_seq = Some(seq);
            seqs.push(seq);
            entries.push(entry);
        }
        Ok(Self {
            len: entries.len(),
            seqs,
            base: 0,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use osmium_types::{Phase, Symbol};

    use super::{BOOK, BookSeqs, EXPECTS, Order, Rows, SYMBOL_SHIFT, TRADE, pack_seq};
    use crate::{
        MdError,
        decode::{Joined, TradeRow},
    };

    /// A feed day's rows: per symbol its `books` rows as `(seq, has_trade)` and its
    /// `trades` rows as `(seq, phase)`. A joined pair has `exch_ts == seq`.
    #[derive(Clone, Default)]
    struct Day {
        books: Vec<Vec<(u64, bool)>>,
        trades: Vec<Vec<(u64, Phase)>>,
        /// `(seq, exch_ts, phase)` the `books` row of a seq says, when not the default.
        said: Vec<(u64, i64, Phase)>,
    }

    impl Day {
        fn symbols(&self) -> Vec<Symbol> {
            (0..self.books.len().max(self.trades.len()))
                .map(|index| Symbol::new(&format!("S{index}")).unwrap())
                .collect()
        }

        /// Builds the order both ways; they must agree.
        fn order(&self) -> Result<Order, String> {
            let symbols = self.symbols();
            let mut joined = Vec::new();
            let mut trades = Vec::new();
            // Two chunks in file order, the second starting in the middle of a symbol.
            let file: Vec<(u32, u64, bool)> = self
                .books
                .iter()
                .enumerate()
                .flat_map(|(symbol, rows)| {
                    let symbol = u32::try_from(symbol).unwrap();
                    rows.iter()
                        .map(move |&(seq, has_trade)| (symbol, seq, has_trade))
                })
                .collect();
            let mut chunks = Vec::new();
            for part in [&file[..file.len() / 2], &file[file.len() / 2..]] {
                let mut chunk = BookSeqs {
                    runs: Vec::new(),
                    packed: Vec::new(),
                };
                let mut previous = 0;
                for &(symbol, seq, has_trade) in part {
                    match chunk.runs.last_mut() {
                        Some(run) if run.0 == symbol => run.1 += 1,
                        _ => {
                            chunk.runs.push((symbol, 1));
                            previous = 0;
                        }
                    }
                    pack_seq(&mut chunk.packed, previous, seq, has_trade);
                    previous = seq;
                    if has_trade {
                        let (exch_ts, phase) = self
                            .said
                            .iter()
                            .find(|said| said.0 == seq)
                            .map_or((i64::try_from(seq).unwrap(), Phase::Continuous), |said| {
                                (said.1, said.2)
                            });
                        joined.push(Joined {
                            symbol,
                            seq,
                            exch_ts,
                            phase,
                        });
                    }
                }
                chunks.push(chunk);
            }
            for (symbol, rows) in self.trades.iter().enumerate() {
                for &(seq, phase) in rows {
                    trades.push(TradeRow {
                        seq,
                        exch_ts: i64::try_from(seq).unwrap(),
                        symbol: u32::try_from(symbol).unwrap(),
                        phase,
                        price: 1,
                        qty: 1,
                        cum_qty: None,
                    });
                }
            }
            let rows = |books| Rows {
                books,
                joined: &joined,
                trades: &trades,
                symbols: &symbols,
                books_path: Path::new("books"),
                trades_path: Path::new("trades"),
            };
            let copy = |chunks: &[BookSeqs]| {
                chunks
                    .iter()
                    .map(|chunk| BookSeqs {
                        runs: chunk.runs.clone(),
                        packed: chunk.packed.clone(),
                    })
                    .collect::<Vec<_>>()
            };
            let describe = |result: Result<Order, MdError>| {
                result.map_err(|error| match error {
                    MdError::InvalidRow {
                        path,
                        symbol,
                        seq,
                        reason,
                    } => format!("{} {symbol} {seq}: {reason}", path.display()),
                    other => other.to_string(),
                })
            };
            let all = rows(copy(&chunks));
            let Some((low, high)) = all.span() else {
                return describe(Order::build(all));
            };
            let dense = describe(Order::dense(
                &all,
                low,
                usize::try_from(high - low + 1).unwrap(),
            ));
            let sparse = describe(Order::sparse(&all));
            match (&dense, &sparse) {
                (Ok(dense), Ok(sparse)) => {
                    assert_eq!(dense.len(), sparse.len());
                    assert_eq!(dense.message_seqs(), sparse.message_seqs());
                    let messages = |order: &Order| -> Vec<(u64, u32)> {
                        (0..order.positions())
                            .filter_map(|position| order.get(position))
                            .map(|(seq, entry)| (seq, entry & !EXPECTS))
                            .collect()
                    };
                    assert_eq!(messages(dense), messages(sparse));
                }
                _ => assert_eq!(dense.as_ref().err(), sparse.as_ref().err()),
            }
            let built = describe(Order::build(rows(copy(&chunks))));
            assert!(built == dense || built == sparse);
            built
        }

        fn messages(&self) -> Vec<(u64, u32, u32)> {
            let order = self.order().unwrap();
            (0..order.positions())
                .filter_map(|position| order.get(position))
                .map(|(seq, entry)| (seq, entry >> SYMBOL_SHIFT, entry & (BOOK | TRADE)))
                .collect()
        }
    }

    fn day() -> Day {
        Day {
            books: vec![
                vec![(1, false), (4, true), (7, false)],
                vec![(2, false), (3, false), (9, true)],
            ],
            trades: vec![
                vec![(4, Phase::Continuous), (5, Phase::Continuous)],
                vec![(9, Phase::Continuous)],
            ],
            said: Vec::new(),
        }
    }

    #[test]
    fn rows_become_messages_in_seq_order() {
        assert_eq!(
            day().messages(),
            vec![
                (1, 0, BOOK),
                (2, 1, BOOK),
                (3, 1, BOOK),
                (4, 0, BOOK | TRADE),
                (5, 0, TRADE),
                (7, 0, BOOK),
                (9, 1, BOOK | TRADE),
            ]
        );
    }

    #[test]
    fn a_selection_far_apart_in_seq_is_ordered_too() {
        let mut day = day();
        for rows in &mut day.books {
            for row in rows {
                row.0 *= 1_000_000;
            }
        }
        for rows in &mut day.trades {
            for row in rows {
                row.0 *= 1_000_000;
            }
        }
        let seqs: Vec<u64> = day.messages().iter().map(|message| message.0).collect();
        assert_eq!(
            seqs,
            vec![
                1_000_000, 2_000_000, 3_000_000, 4_000_000, 5_000_000, 7_000_000, 9_000_000
            ]
        );
        assert_eq!(day.order().unwrap().positions(), 7);
    }

    #[test]
    fn a_day_without_rows_has_no_messages() {
        let order = Day::default().order().unwrap();
        assert_eq!((order.len(), order.positions()), (0, 0));
        let trades_only = Day {
            books: Vec::new(),
            trades: vec![vec![(3, Phase::Continuous)]],
            said: Vec::new(),
        };
        assert_eq!(trades_only.messages(), vec![(3, 0, TRADE)]);
    }

    #[test]
    fn rows_that_do_not_join_are_rejected() {
        let error = |change: fn(&mut Day)| {
            let mut day = day();
            change(&mut day);
            day.order().unwrap_err()
        };
        assert_eq!(
            error(|day| day.trades[1].clear()),
            "books S1 9: has_trade without a trades row"
        );
        assert_eq!(
            error(|day| day.trades[1].push((10, Phase::CloseAuction))),
            "trades S1 10: a trade without a books row outside continuous trading"
        );
        assert_eq!(
            error(|day| day.trades[0].push((7, Phase::Continuous))),
            "books S0 7: a trades row shares the seq of a row without has_trade"
        );
        assert_eq!(
            error(|day| day.trades[1][0].1 = Phase::CloseAuction),
            "trades S1 9: the trades row and the books row of one seq disagree"
        );
        assert_eq!(
            error(|day| day.said.push((4, 5, Phase::Continuous))),
            "trades S0 4: the trades row and the books row of one seq disagree"
        );
        // The trade of seq 9 is another symbol's.
        assert_eq!(
            error(|day| {
                day.trades[1].clear();
                day.trades[0].push((9, Phase::Continuous));
            }),
            "trades S0 9: the trades row and the books row of one seq disagree"
        );
        assert_eq!(
            error(|day| day.books[1][1] = (7, false)),
            "books S1 7: two rows share one seq"
        );
        assert_eq!(
            error(|day| day.trades[1].insert(0, (5, Phase::Continuous))),
            "trades S1 5: two rows share one seq"
        );
    }
}
