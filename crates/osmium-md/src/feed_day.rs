use std::collections::HashMap;

use osmium_types::{
    Board, Book, BookSide, Date, Feed, Level, Message, MessageBody, Price, Shares, Symbol,
    Timestamp, Trade,
};

use crate::{
    DayInstruments, DayManifest, ManifestEntry, MdError, MdRoot, Table,
    decode::{
        BookChunk, BookKind, BookRow, CompactSide, DecodeContext, Edge, Joined, SymbolEntry,
        TradeChunk, TradeRow, book_columns, is_intermediate_fill, observed_times, read_book_chunk,
        read_trade_chunk, trade_columns,
    },
    order::{BOOK, BookSeqs, MAX_SYMBOLS, Order, Rows, TRADE, symbol_of},
    parallel,
    parquet_io::ParquetFile,
    stream::{Decoded, Frame, decode, encode},
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

/// A symbol of a feed day and where its rows start.
#[derive(Debug, Clone, Copy)]
struct SymbolRows {
    entry: SymbolEntry,
    /// Where the symbol's stream starts in `FeedDay::books`.
    books: usize,
    /// The index of the symbol's first row in `FeedDay::trades`.
    trades: usize,
}

/// One feed's messages for one trading date, in the order the exchange sent them.
///
/// Every row is validated while loading. The day is kept the way the files are sorted,
/// symbol by symbol, with each `books` row stored as its difference from the symbol's
/// previous one; [`FeedDay::messages`] puts the rows back in the exchange's order and
/// converts quantities to shares.
#[derive(Debug)]
pub struct FeedDay {
    feed: Feed,
    date: Date,
    identity: FeedDayIdentity,
    symbols: Vec<SymbolRows>,
    /// Every symbol's `books` rows, encoded by [`crate::stream`]; symbols follow each
    /// other in file order.
    books: Vec<u8>,
    /// The `trades` rows in file order.
    trades: Vec<TradeRow>,
    order: Order,
    /// The observed time of each message when the day holds a selection of symbols.
    /// For a whole feed it is the running maximum of the messages' own `exch_ts`.
    clock: Option<Vec<i64>>,
}

/// One row group of a feed day's files, decoded.
enum Chunk {
    Books(Box<BookChunk>),
    Trades(TradeChunk),
}

/// The symbols seen while putting chunks together, in order of first appearance.
#[derive(Default)]
struct SymbolTable {
    rows: Vec<SymbolRows>,
    index: HashMap<Symbol, u32>,
}

impl SymbolTable {
    /// The index of a run's symbol, and whether the run is the symbol's first.
    fn intern(&mut self, entry: SymbolEntry) -> (u32, bool) {
        if let Some(&index) = self.index.get(&entry.symbol) {
            return (index, false);
        }
        assert!(
            self.rows.len() < MAX_SYMBOLS,
            "more symbols than osmium can index"
        );
        let index = u32::try_from(self.rows.len()).expect("fewer than 2^32 symbols");
        self.rows.push(SymbolRows {
            entry,
            books: 0,
            trades: 0,
        });
        self.index.insert(entry.symbol, index);
        (index, true)
    }
}

/// Puts the chunks of a feed day's files together as they arrive: the chunks of `books`
/// in file order, then those of `trades`.
struct Join<'a> {
    books_file: &'a ParquetFile,
    trades_file: &'a ParquetFile,
    table: SymbolTable,
    /// The last row, kept or not, of the chunks so far of the file being read.
    previous: Option<(Symbol, u64)>,
    books: Vec<u8>,
    seqs: Vec<BookSeqs>,
    joined: Vec<Joined>,
    /// The last kept `books` row so far and its symbol: the first row of the next chunk
    /// tells whether it is an intermediate fill.
    tail: Option<(BookRow, u32)>,
    /// Whether `books` has ended.
    in_trades: bool,
    trades: Vec<TradeRow>,
}

impl<'a> Join<'a> {
    fn new(books_file: &'a ParquetFile, trades_file: &'a ParquetFile) -> Self {
        Self {
            books_file,
            trades_file,
            table: SymbolTable::default(),
            previous: None,
            books: Vec::new(),
            seqs: Vec::new(),
            joined: Vec::new(),
            tail: None,
            in_trades: false,
            trades: Vec::new(),
        }
    }

    /// The error of a chunk, if any: that its rows do not follow those of the chunk
    /// before it, or its own.
    fn enter(
        &mut self,
        file: &ParquetFile,
        edge: &Edge,
        error: Option<MdError>,
    ) -> Result<(), MdError> {
        if let (Some(previous), Some(first)) = (self.previous, edge.first) {
            Edge::follows(file.path(), previous, first)?;
        }
        if let Some(error) = error {
            return Err(error);
        }
        self.previous = edge.last.or(self.previous);
        Ok(())
    }

    /// Writes the last row of a chunk, now that the row after it is known. Like the
    /// first row of a chunk it does not depend on the rows before it.
    fn write_tail(&mut self, row: &BookRow) {
        let mut frame = Frame::START;
        encode(&mut self.books, &mut frame, true, row);
    }

    fn push(&mut self, chunk: Chunk) -> Result<(), MdError> {
        match chunk {
            Chunk::Books(chunk) => self.push_books(*chunk),
            Chunk::Trades(chunk) => self.push_trades(chunk),
        }
    }

    fn push_books(&mut self, chunk: BookChunk) -> Result<(), MdError> {
        self.enter(self.books_file, &chunk.edge, chunk.error)?;
        let mut runs = Vec::with_capacity(chunk.runs.len());
        // A chunk that keeps no row leaves the tail waiting for the next one.
        if let Some((exch_ts, kind)) = chunk.head
            && let Some((mut row, symbol)) = self.tail.take()
        {
            let first = chunk.runs.first().map(|run| run.entry.symbol);
            let same_symbol = first == Some(self.table.rows[symbol as usize].entry.symbol);
            if is_intermediate_fill(&row, same_symbol, exch_ts, kind) {
                row.kind = BookKind::IntermediateTrade;
            }
            self.write_tail(&row);
        }
        let mut start = self.books.len();
        for (run, bytes) in chunk.runs.iter().zip(&chunk.run_bytes) {
            let (symbol, new) = self.table.intern(run.entry);
            if new {
                self.table.rows[symbol as usize].books = start;
            }
            start += bytes;
            runs.push((symbol, run.rows));
        }
        self.books.extend_from_slice(&chunk.bytes);
        self.joined.extend(chunk.joined.iter().map(|joined| Joined {
            symbol: runs[joined.symbol as usize].0,
            ..*joined
        }));
        if let Some(row) = chunk.tail {
            let symbol = runs.last().expect("the tail has a run").0;
            self.tail = Some((row, symbol));
        }
        self.seqs.push(BookSeqs {
            runs,
            packed: chunk.seqs,
        });
        Ok(())
    }

    /// Ends `books`, whose last row has no row after it, and starts `trades`.
    fn end_books(&mut self) -> Result<(), MdError> {
        if let Some((row, _)) = self.tail.take() {
            self.write_tail(&row);
        }
        self.books.shrink_to_fit();
        self.trades_file.check(trade_columns())?;
        self.previous = None;
        self.in_trades = true;
        Ok(())
    }

    fn push_trades(&mut self, chunk: TradeChunk) -> Result<(), MdError> {
        if !self.in_trades {
            self.end_books()?;
        }
        self.enter(self.trades_file, &chunk.edge, chunk.error)?;
        let mut start = self.trades.len();
        let mut symbols = Vec::with_capacity(chunk.runs.len());
        for (index, run) in chunk.runs.iter().enumerate() {
            let (symbol, _) = self.table.intern(run.entry);
            // A symbol's rows are consecutive: only the first run of a chunk can go on
            // with the symbol the chunk before it ended with.
            let continues =
                index == 0 && self.trades.last().is_some_and(|row| row.symbol == symbol);
            if !continues {
                self.table.rows[symbol as usize].trades = start;
            }
            start += run.rows;
            symbols.push(symbol);
        }
        self.trades.extend(chunk.rows.iter().map(|row| TradeRow {
            symbol: symbols[row.symbol as usize],
            ..*row
        }));
        Ok(())
    }

    fn finish(mut self) -> Result<Self, MdError> {
        if !self.in_trades {
            self.end_books()?;
        }
        self.trades.shrink_to_fit();
        Ok(self)
    }
}

impl FeedDay {
    /// Reads `feed` on the manifest's date.
    ///
    /// `selection` restricts the read to some symbols; `None` reads the whole feed.
    /// Observed times always span the whole feed, so they do not depend on the selection.
    /// `instruments` supplies shares per lot for regular-board feeds.
    ///
    /// The files are read on as many threads as the machine has cores; the result does
    /// not depend on that number.
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
        let threads = parallel::threads();

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
        books_file.check(book_columns())?;
        let book_groups = context.row_groups(&books_file);
        let trade_groups = context.row_groups(&trades_file);
        let mut join = Join::new(&books_file, &trades_file);
        let mut failed = None;
        // Chunks are put together while later ones are still being read, so the rows of
        // a chunk are held twice only for a moment.
        parallel::ordered(
            threads,
            book_groups.len() + trade_groups.len(),
            |piece| match piece.checked_sub(book_groups.len()) {
                None => Chunk::Books(Box::new(read_book_chunk(
                    &books_file,
                    book_groups[piece],
                    &context,
                ))),
                Some(piece) => Chunk::Trades(read_trade_chunk(
                    &trades_file,
                    trade_groups[piece],
                    &context,
                )),
            },
            |chunk| {
                failed = join.push(chunk).err();
                failed.is_none()
            },
        );
        if let Some(error) = failed {
            return Err(error);
        }
        let Join {
            table,
            books,
            seqs,
            joined,
            trades,
            ..
        } = join.finish()?;
        let symbols: Vec<Symbol> = table.rows.iter().map(|rows| rows.entry.symbol).collect();
        let order = Order::build(Rows {
            books: seqs,
            joined: &joined,
            trades: &trades,
            symbols: &symbols,
            books_path: books_file.path(),
            trades_path: trades_file.path(),
        })?;
        drop(joined);
        let clock = if selection.is_some() {
            // The files were opened and checked above; this reads other columns of them.
            Some(observed_times(
                threads,
                [&books_file, &trades_file],
                &order.message_seqs(),
            )?)
        } else {
            None
        };

        manifest.verify_unchanged(root, Table::Books, part, &books_entry)?;
        manifest.verify_unchanged(root, Table::Trades, part, &trades_entry)?;
        Ok(Self {
            feed,
            date,
            identity: FeedDayIdentity {
                books: books_entry,
                trades: trades_entry,
            },
            symbols: table.rows,
            books,
            trades,
            order,
            clock,
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
    pub const fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.order.len() == 0
    }

    /// The symbols that have at least one message.
    pub fn symbols(&self) -> impl Iterator<Item = Symbol> + '_ {
        self.symbols.iter().map(|rows| rows.entry.symbol)
    }

    /// Symbols whose lots were converted with [`MissingLotSize::Assume`].
    pub fn assumed_lot_sizes(&self) -> impl Iterator<Item = Symbol> + '_ {
        self.symbols
            .iter()
            .filter(|rows| rows.entry.assumed)
            .map(|rows| rows.entry.symbol)
    }

    /// The messages in increasing `seq`, the order the exchange sent them.
    #[must_use]
    pub fn messages(&self) -> Messages<'_> {
        Messages {
            day: self,
            position: 0,
            produced: 0,
            running: i64::MIN,
            cursors: self
                .symbols
                .iter()
                .map(|rows| Cursor {
                    books: rows.books,
                    trades: rows.trades,
                    frame: Frame::START,
                })
                .collect(),
        }
    }
}

fn units(price: u32) -> Price {
    Price::from_units(i64::from(price))
}

fn side(compact: &CompactSide, shares_per_unit: i64) -> BookSide {
    let mut levels = [Level {
        price: units(0),
        qty: Shares::ZERO,
    }; osmium_types::BOOK_DEPTH];
    let len = usize::from(compact.len);
    for (level, slot) in levels.iter_mut().enumerate().take(len) {
        *slot = Level {
            price: units(compact.price[level]),
            qty: Shares::new(i64::from(compact.qty[level]) * shares_per_unit),
        };
    }
    BookSide::new(
        &levels[..len],
        Shares::new(i64::from(compact.market_qty) * shares_per_unit),
    )
    .expect("rows were validated while loading")
}

/// Where the next rows of one symbol are.
#[derive(Debug, Clone, Copy)]
struct Cursor {
    /// The offset of the symbol's next row in `FeedDay::books`.
    books: usize,
    /// The index of the symbol's next row in `FeedDay::trades`.
    trades: usize,
    /// The symbol's `books` rows read so far.
    frame: Frame,
}

/// Iterator over a [`FeedDay`]'s messages in increasing `seq`.
#[derive(Debug, Clone)]
pub struct Messages<'a> {
    day: &'a FeedDay,
    /// The next position of the day's order.
    position: usize,
    /// The number of messages returned.
    produced: usize,
    /// The largest `exch_ts` returned.
    running: i64,
    cursors: Vec<Cursor>,
}

impl Iterator for Messages<'_> {
    type Item = Message;

    fn next(&mut self) -> Option<Message> {
        let day = self.day;
        let (seq, entry) = loop {
            if self.position == day.order.positions() {
                return None;
            }
            let found = day.order.get(self.position);
            self.position += 1;
            if let Some(found) = found {
                break found;
            }
        };
        let rows = &day.symbols[symbol_of(entry)];
        let cursor = &mut self.cursors[symbol_of(entry)];
        let shares_per_unit = i64::from(rows.entry.shares_per_unit);
        let shares = |qty: u32| Shares::new(i64::from(qty) * shares_per_unit);

        let trade = (entry & TRADE != 0).then(|| {
            let row = &day.trades[cursor.trades];
            cursor.trades += 1;
            row
        });
        let book =
            (entry & BOOK != 0).then(|| decode(&day.books, &mut cursor.books, &mut cursor.frame));
        let (exch_ts, phase) = match (&book, trade) {
            (Some(row), _) => (cursor.frame.exch_ts, row.phase),
            (None, Some(row)) => (row.exch_ts, row.phase),
            (None, None) => unreachable!("an entry has a book row or a trade row"),
        };
        let trade = trade.map(|row| Trade {
            price: units(row.price),
            qty: shares(row.qty),
            cum_qty: row.cum_qty.map(|cum| {
                Shares::new(
                    i64::try_from(cum)
                        .ok()
                        .and_then(|cum| cum.checked_mul(shares_per_unit))
                        .expect("cumulative quantity fits in i64 shares"),
                )
            }),
        });
        let full_book = |frame: &Frame| {
            Book::new(
                side(&frame.bid, shares_per_unit),
                side(&frame.ask, shares_per_unit),
            )
            .expect("rows were validated while loading")
        };
        let body = match (book, trade) {
            (None, Some(trade)) => MessageBody::IntermediateTrade(trade),
            (
                Some(Decoded {
                    kind: BookKind::IntermediateTrade,
                    ..
                }),
                trade,
            ) => MessageBody::IntermediateTrade(trade.expect("rows were validated while loading")),
            (Some(row), trade) => match row.kind {
                BookKind::Disclosure { .. } => MessageBody::Disclosure {
                    book: full_book(&cursor.frame),
                    trade,
                },
                BookKind::Indicative => MessageBody::Indicative {
                    book: full_book(&cursor.frame),
                    matched: (row.indicative_price != 0).then(|| Level {
                        price: units(row.indicative_price),
                        qty: shares(row.indicative_qty),
                    }),
                    delay_open: row.delay_open,
                    delay_close: row.delay_close,
                },
                BookKind::Delay(direction) => MessageBody::Delay(direction),
                BookKind::IntermediateTrade => unreachable!("matched above"),
            },
            (None, None) => unreachable!("an entry has a book row or a trade row"),
        };
        self.running = self.running.max(exch_ts);
        let observed_time = day
            .clock
            .as_ref()
            .map_or(self.running, |clock| clock[self.produced]);
        self.produced += 1;
        Some(Message {
            feed: day.feed,
            seq,
            symbol: rows.entry.symbol,
            exch_ts: Timestamp::from_micros(exch_ts),
            observed_time: Timestamp::from_micros(observed_time),
            phase,
            body,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.day.order.len() - self.produced;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for Messages<'_> {}
