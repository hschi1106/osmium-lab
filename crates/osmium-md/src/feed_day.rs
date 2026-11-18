use std::path::{Path, PathBuf};

use osmium_types::{
    Board, Book, BookSide, Date, Feed, Level, Message, MessageBody, Phase, Price, Shares, Symbol,
    Timestamp, Trade,
};

use crate::{
    DayInstruments, DayManifest, ManifestEntry, MdError, MdRoot, Table,
    decode::{
        BookKind, BookRow, CompactSide, DecodeContext, SymbolEntry, SymbolTable, TradeRow,
        observed_times, read_books, read_trades,
    },
};

/// What to do when a regular-board symbol has no `shares_per_lot` in `instruments`
/// (the value is null, or the symbol has no row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingLotSize {
    /// Fail the load.
    Reject,
    /// Use this many shares per lot and report the symbol in
    /// [`FeedDay::assumed_lot_sizes`].
    Assume(u32),
}

/// The marketdata identity of the files one feed-day read consumed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedDayIdentity {
    pub books: ManifestEntry,
    pub trades: ManifestEntry,
}

/// One message position: its rows and its observed time.
#[derive(Debug, Clone, Copy)]
struct Slot {
    observed_time: i64,
    /// Index into `books`, or `NONE`.
    book: u32,
    /// Index into `trades`, or `NONE`.
    trade: u32,
}

const NONE: u32 = u32::MAX;

/// One feed's messages for one trading date, in the order the exchange sent them.
///
/// Every row is validated while loading; quantities are converted to shares when a
/// message is produced.
#[derive(Debug)]
pub struct FeedDay {
    feed: Feed,
    date: Date,
    identity: FeedDayIdentity,
    symbols: Vec<SymbolEntry>,
    books: Vec<BookRow>,
    trades: Vec<TradeRow>,
    slots: Vec<Slot>,
}

impl FeedDay {
    /// Reads `feed` on the manifest's date.
    ///
    /// `selection` restricts the read to some symbols; `None` reads the whole feed.
    /// Observed times always span the whole feed, so they do not depend on the selection.
    /// `instruments` supplies shares per lot for regular-board feeds.
    ///
    /// The manifest is read again after the data; a change to the feed's rows (a
    /// marketdata update racing this read) is an error.
    ///
    /// # Panics
    ///
    /// Panics when `instruments` is for another venue or date than the feed and manifest.
    pub fn load(
        root: &MdRoot,
        manifest: &DayManifest,
        feed: Feed,
        instruments: &DayInstruments,
        selection: Option<&[Symbol]>,
        missing_lot_size: MissingLotSize,
    ) -> Result<Self, MdError> {
        assert_eq!(instruments.venue(), feed.venue(), "instruments venue");
        assert_eq!(instruments.date(), manifest.date(), "instruments date");
        let date = manifest.date();
        let part = feed.part();

        let selection = selection.map(|symbols| {
            let mut symbols = symbols.to_vec();
            symbols.sort_unstable();
            symbols.dedup();
            symbols
        });
        let resolve = |symbol: Symbol| match feed.board() {
            Board::OddLot => Ok((1, false)),
            Board::Regular => {
                let known = instruments
                    .get(symbol)
                    .and_then(|instrument| instrument.shares_per_lot);
                match (known, missing_lot_size) {
                    (Some(lot), _) => Ok((lot, false)),
                    (None, MissingLotSize::Assume(lot)) => Ok((lot, true)),
                    (None, MissingLotSize::Reject) => {
                        Err(MdError::MissingLotSize { date, feed, symbol })
                    }
                }
            }
        };
        let context = DecodeContext {
            selection: selection.as_deref(),
            resolve: &resolve,
        };

        let (books_file, books_entry) = manifest.open(root, Table::Books, part)?;
        let (trades_file, trades_entry) = manifest.open(root, Table::Trades, part)?;
        let books_path = books_file.path().to_path_buf();
        let trades_path = trades_file.path().to_path_buf();
        let mut table = SymbolTable::default();
        let books = read_books(books_file, &context, &mut table)?;
        let trades = read_trades(trades_file, &context, &mut table)?;
        let symbols = table.entries;

        let mut slots = merge(&books, &trades, &symbols, &books_path, &trades_path)?;
        if selection.is_some() {
            let seqs: Vec<u64> = slots
                .iter()
                .map(|slot| slot_seq(&books, &trades, slot))
                .collect();
            let clock = observed_times(
                [
                    manifest.open(root, Table::Books, part)?.0,
                    manifest.open(root, Table::Trades, part)?.0,
                ],
                &seqs,
            )?;
            for (slot, observed_time) in slots.iter_mut().zip(clock) {
                slot.observed_time = observed_time;
            }
        } else {
            // The whole feed is loaded: the running maximum is over every message.
            let mut running = i64::MIN;
            for slot in &mut slots {
                running = running.max(slot.observed_time);
                slot.observed_time = running;
            }
        }

        manifest.verify_unchanged(root, Table::Books, part, &books_entry)?;
        manifest.verify_unchanged(root, Table::Trades, part, &trades_entry)?;
        Ok(Self {
            feed,
            date,
            identity: FeedDayIdentity {
                books: books_entry,
                trades: trades_entry,
            },
            symbols,
            books,
            trades,
            slots,
        })
    }

    #[must_use]
    pub const fn feed(&self) -> Feed {
        self.feed
    }

    #[must_use]
    pub const fn date(&self) -> Date {
        self.date
    }

    #[must_use]
    pub const fn identity(&self) -> &FeedDayIdentity {
        &self.identity
    }

    /// Number of messages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The symbols that have at least one message.
    pub fn symbols(&self) -> impl Iterator<Item = Symbol> + '_ {
        self.symbols.iter().map(|entry| entry.symbol)
    }

    /// Symbols whose lots were converted with [`MissingLotSize::Assume`].
    pub fn assumed_lot_sizes(&self) -> impl Iterator<Item = Symbol> + '_ {
        self.symbols
            .iter()
            .filter(|entry| entry.assumed)
            .map(|entry| entry.symbol)
    }

    /// The messages in increasing `seq`, the order the exchange sent them.
    #[must_use]
    pub fn messages(&self) -> Messages<'_> {
        Messages { day: self, next: 0 }
    }

    fn message(&self, slot: &Slot) -> Message {
        let book = (slot.book != NONE).then(|| &self.books[slot.book as usize]);
        let trade = (slot.trade != NONE).then(|| &self.trades[slot.trade as usize]);
        let (seq, exch_ts, symbol, phase) = match (book, trade) {
            (Some(row), _) => (row.seq, row.exch_ts, row.symbol, row.phase),
            (None, Some(row)) => (row.seq, row.exch_ts, row.symbol, row.phase),
            (None, None) => unreachable!("a slot has a book row or a trade row"),
        };
        let entry = &self.symbols[symbol as usize];
        let shares = |qty: u32| Shares::new(i64::from(qty) * i64::from(entry.shares_per_unit));
        let trade = trade.map(|row| Trade {
            price: units(row.price),
            qty: shares(row.qty),
            cum_qty: row.cum_qty.map(|cum| {
                Shares::new(
                    i64::try_from(cum)
                        .ok()
                        .and_then(|cum| cum.checked_mul(i64::from(entry.shares_per_unit)))
                        .expect("cumulative quantity fits in i64 shares"),
                )
            }),
        });
        let full_book = |row: &BookRow| {
            Book::new(side(&row.bid, &shares), side(&row.ask, &shares))
                .expect("rows were validated while loading")
        };
        let body = match (book, trade) {
            (None, Some(trade)) => MessageBody::IntermediateTrade(trade),
            (Some(row), trade) => match row.kind {
                BookKind::IntermediateTrade => MessageBody::IntermediateTrade(
                    trade.expect("rows were validated while loading"),
                ),
                BookKind::Disclosure { .. } => MessageBody::Disclosure {
                    book: full_book(row),
                    trade,
                },
                BookKind::Indicative => MessageBody::Indicative {
                    book: full_book(row),
                    matched: (row.indicative_price != 0).then(|| Level {
                        price: units(row.indicative_price),
                        qty: shares(row.indicative_qty),
                    }),
                    delay_open: row.delay_open,
                    delay_close: row.delay_close,
                },
                BookKind::Delay(direction) => MessageBody::Delay(direction),
            },
            (None, None) => unreachable!("a slot has a book row or a trade row"),
        };
        Message {
            feed: self.feed,
            seq,
            symbol: entry.symbol,
            exch_ts: Timestamp::from_micros(exch_ts),
            observed_time: Timestamp::from_micros(slot.observed_time),
            phase,
            body,
        }
    }
}

fn units(price: u32) -> Price {
    Price::from_units(i64::from(price))
}

fn side(compact: &CompactSide, shares: &dyn Fn(u32) -> Shares) -> BookSide {
    let mut levels = [Level {
        price: units(0),
        qty: Shares::ZERO,
    }; osmium_types::BOOK_DEPTH];
    let len = usize::from(compact.len);
    for (level, slot) in levels.iter_mut().enumerate().take(len) {
        *slot = Level {
            price: units(compact.price[level]),
            qty: shares(compact.qty[level]),
        };
    }
    BookSide::new(&levels[..len], shares(compact.market_qty))
        .expect("rows were validated while loading")
}

fn slot_seq(books: &[BookRow], trades: &[TradeRow], slot: &Slot) -> u64 {
    if slot.book == NONE {
        trades[slot.trade as usize].seq
    } else {
        books[slot.book as usize].seq
    }
}

/// Joins book and trade rows by `seq` into messages in increasing `seq`.
///
/// Each slot's `observed_time` is left as the message's own `exch_ts`.
fn merge(
    books: &[BookRow],
    trades: &[TradeRow],
    symbols: &[SymbolEntry],
    books_path: &Path,
    trades_path: &Path,
) -> Result<Vec<Slot>, MdError> {
    let order = |len: usize, seq: &dyn Fn(u32) -> u64, path: &Path| {
        let count = u32::try_from(len)
            .ok()
            .filter(|count| *count < NONE)
            .ok_or_else(|| MdError::inconsistent(path, "more rows than osmium can index"))?;
        let mut order: Vec<u32> = (0..count).collect();
        order.sort_unstable_by_key(|&index| seq(index));
        Ok::<_, MdError>(order)
    };
    let book_order = order(books.len(), &|index| books[index as usize].seq, books_path)?;
    let trade_order = order(
        trades.len(),
        &|index| trades[index as usize].seq,
        trades_path,
    )?;
    let invalid = |path: &Path, symbol: u32, seq: u64, reason: &str| MdError::InvalidRow {
        path: PathBuf::from(path),
        symbol: symbols[symbol as usize].symbol.to_string(),
        seq,
        reason: reason.to_owned(),
    };

    let mut slots = Vec::with_capacity(books.len() + trades.len());
    let mut last_seq = None;
    let (mut next_book, mut next_trade) = (0, 0);
    loop {
        let book = book_order
            .get(next_book)
            .map(|&index| (index, &books[index as usize]));
        let trade = trade_order
            .get(next_trade)
            .map(|&index| (index, &trades[index as usize]));
        let (book, trade) = match (book, trade) {
            (None, None) => break,
            (Some(book), Some(trade)) if book.1.seq == trade.1.seq => (Some(book), Some(trade)),
            (Some(book), Some(trade)) if book.1.seq < trade.1.seq => (Some(book), None),
            (Some(book), None) => (Some(book), None),
            (_, Some(trade)) => (None, Some(trade)),
        };
        next_book += usize::from(book.is_some());
        next_trade += usize::from(trade.is_some());

        let (seq, exch_ts) = match (book, trade) {
            (Some((_, book)), Some((_, trade))) => {
                if !book.kind.has_trade() {
                    return Err(invalid(
                        books_path,
                        book.symbol,
                        book.seq,
                        "a trades row shares the seq of a row without has_trade",
                    ));
                }
                if book.symbol != trade.symbol
                    || book.phase != trade.phase
                    || book.exch_ts != trade.exch_ts
                {
                    return Err(invalid(
                        trades_path,
                        trade.symbol,
                        trade.seq,
                        "the trades row and the books row of one seq disagree",
                    ));
                }
                (book.seq, book.exch_ts)
            }
            (Some((_, book)), None) => {
                if book.kind.has_trade() {
                    return Err(invalid(
                        books_path,
                        book.symbol,
                        book.seq,
                        "has_trade without a trades row",
                    ));
                }
                (book.seq, book.exch_ts)
            }
            (None, Some((_, trade))) => {
                if trade.phase != Phase::Continuous {
                    return Err(invalid(
                        trades_path,
                        trade.symbol,
                        trade.seq,
                        "a trade without a books row outside continuous trading",
                    ));
                }
                (trade.seq, trade.exch_ts)
            }
            (None, None) => unreachable!("the loop ended above"),
        };
        if last_seq.is_some_and(|last| last >= seq) {
            let (path, symbol) = match (book, trade) {
                (Some((_, book)), _) => (books_path, book.symbol),
                (None, Some((_, trade))) => (trades_path, trade.symbol),
                (None, None) => unreachable!("the loop ended above"),
            };
            return Err(invalid(path, symbol, seq, "two rows share one seq"));
        }
        last_seq = Some(seq);
        slots.push(Slot {
            observed_time: exch_ts,
            book: book.map_or(NONE, |(index, _)| index),
            trade: trade.map_or(NONE, |(index, _)| index),
        });
    }
    Ok(slots)
}

/// Iterator over a [`FeedDay`]'s messages in increasing `seq`.
#[derive(Debug, Clone)]
pub struct Messages<'a> {
    day: &'a FeedDay,
    next: usize,
}

impl Iterator for Messages<'_> {
    type Item = Message;

    fn next(&mut self) -> Option<Message> {
        let slot = self.day.slots.get(self.next)?;
        self.next += 1;
        Some(self.day.message(slot))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.day.slots.len() - self.next;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for Messages<'_> {}
