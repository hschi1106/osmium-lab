use std::collections::BTreeMap;

use market_types::{DomainEvent, Symbol, TradingDate};
use replay_engine::EventStream;

use crate::{
    Feed, Instruments, MdError, MdRoot, Table,
    clock::observed_times,
    manifest::{DayManifest, ManifestEntry, open_verified},
    mapper::{Message, SymbolMapper, merge_messages},
    rows::{read_books, read_trades},
};

/// The marketdata identity of everything one feed-day read consumed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedDayIdentity {
    pub books: ManifestEntry,
    pub trades: ManifestEntry,
    pub instruments: ManifestEntry,
}

/// One feed's messages for one trading date, restricted to the selected symbols.
///
/// Rows are held in compact form and mapped to `DomainEvent`s lazily by each
/// symbol's stream, so memory scales with the selected symbols' row count.
#[derive(Debug)]
pub struct FeedDay {
    feed: Feed,
    trading_date: TradingDate,
    identity: FeedDayIdentity,
    streams: BTreeMap<Symbol, SymbolEventStream>,
}

impl FeedDay {
    /// Reads `symbols` of `feed` on `trading_date`.
    ///
    /// The manifest is read before and after the data; any change between the two
    /// reads (a marketdata update racing this read) is an error. Selected symbols
    /// without rows get an empty stream.
    pub fn load(
        root: &MdRoot,
        trading_date: TradingDate,
        feed: Feed,
        symbols: &[Symbol],
    ) -> Result<Self, MdError> {
        let manifest = DayManifest::load(root, trading_date)?;
        let books_entry = manifest.entry(Table::Books, feed.part())?.clone();
        let trades_entry = manifest.entry(Table::Trades, feed.part())?.clone();
        let instruments = Instruments::load(root, trading_date, feed, &manifest)?;

        let mut selected: Vec<&str> = symbols.iter().map(Symbol::as_str).collect();
        selected.sort_unstable();
        selected.dedup();

        let open = |table, entry: &ManifestEntry| {
            open_verified(root, trading_date, table, feed.part(), entry)
        };
        let mut books = read_books(open(Table::Books, &books_entry)?, &selected)?;
        let mut trades = read_trades(open(Table::Trades, &trades_entry)?, &selected)?;

        let mut messages = BTreeMap::new();
        for text in &selected {
            let symbol = Symbol::new(text).expect("selected symbols came from valid Symbols");
            let merged = merge_messages(
                symbol,
                books.remove(&symbol).unwrap_or_default(),
                trades.remove(&symbol).unwrap_or_default(),
            )?;
            messages.insert(symbol, merged);
        }

        let mut seqs: Vec<u64> = messages.values().flatten().map(Message::seq).collect();
        seqs.sort_unstable();
        if seqs.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(MdError::inconsistent(
                &root.file(Table::Books, trading_date, feed.part()),
                "two selected messages share one seq",
            ));
        }
        let clock = if seqs.is_empty() {
            Vec::new()
        } else {
            observed_times(
                [
                    open(Table::Books, &books_entry)?,
                    open(Table::Trades, &trades_entry)?,
                ],
                &seqs,
            )?
        };

        let identity = FeedDayIdentity {
            books: books_entry,
            trades: trades_entry,
            instruments: instruments.manifest_entry().clone(),
        };
        let after = DayManifest::load(root, trading_date)?;
        for (table, part, entry) in [
            (Table::Books, feed.part(), &identity.books),
            (Table::Trades, feed.part(), &identity.trades),
            (Table::Instruments, feed.venue(), &identity.instruments),
        ] {
            if after.entry(table, part)? != entry {
                return Err(MdError::ManifestChanged {
                    date: trading_date,
                    table: table.as_str(),
                    part: part.to_owned(),
                });
            }
        }

        let streams = messages
            .into_iter()
            .map(|(symbol, messages)| {
                let observed = messages
                    .iter()
                    .map(|message| clock[seqs.partition_point(|&seq| seq < message.seq())])
                    .collect();
                let interval = instruments
                    .get(&symbol)
                    .and_then(|record| record.match_interval_s);
                let stream = SymbolEventStream {
                    mapper: SymbolMapper::new(feed, trading_date, symbol, interval),
                    messages,
                    observed,
                    next: 0,
                };
                (symbol, stream)
            })
            .collect();
        Ok(Self {
            feed,
            trading_date,
            identity,
            streams,
        })
    }

    #[must_use]
    pub const fn feed(&self) -> Feed {
        self.feed
    }

    #[must_use]
    pub const fn trading_date(&self) -> TradingDate {
        self.trading_date
    }

    #[must_use]
    pub const fn identity(&self) -> &FeedDayIdentity {
        &self.identity
    }

    /// Symbols with a stream (every selected symbol, including those without rows).
    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.streams.keys()
    }

    /// Removes and returns the stream of `symbol`.
    pub fn take_stream(&mut self, symbol: &Symbol) -> Option<SymbolEventStream> {
        self.streams.remove(symbol)
    }
}

/// One symbol's events in exchange sequence order.
#[derive(Debug)]
pub struct SymbolEventStream {
    mapper: SymbolMapper,
    messages: Vec<Message>,
    observed: Vec<i64>,
    next: usize,
}

impl SymbolEventStream {
    /// Number of messages not yet returned.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.messages.len() - self.next
    }
}

impl EventStream for SymbolEventStream {
    type Error = MdError;

    fn next_event(&mut self) -> Result<Option<DomainEvent>, Self::Error> {
        let Some(message) = self.messages.get(self.next) else {
            return Ok(None);
        };
        let event = self.mapper.map(message, self.observed[self.next])?;
        self.next += 1;
        Ok(Some(event))
    }
}
