use std::collections::{BTreeMap, HashMap};

use osmium_book::{BookId, BookState};
use osmium_types::{Board, Instrument, Message, Money, Position, Symbol};

/// The market as the strategy has seen it: the books built from the messages that have
/// reached the strategy, and today's instruments.
#[derive(Debug, Default)]
pub struct Market {
    books: HashMap<BookId, BookState>,
    /// The keys of `books`, sorted.
    order: Vec<BookId>,
    instruments: HashMap<Symbol, Instrument>,
}

impl Market {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The state of one book; `None` when the run does not replay it today.
    #[must_use]
    pub fn book(&self, symbol: Symbol, board: Board) -> Option<&BookState> {
        self.books.get(&BookId { symbol, board })
    }

    /// Today's reference data of an instrument; `None` when marketdata has no row.
    #[must_use]
    pub fn instrument(&self, symbol: Symbol) -> Option<&Instrument> {
        self.instruments.get(&symbol)
    }

    /// Every book the run replays today, by symbol and then board.
    pub fn books(&self) -> impl Iterator<Item = (BookId, &BookState)> {
        self.order.iter().map(|&id| (id, &self.books[&id]))
    }

    /// Starts a trading day with its instruments and books. For the code that runs a
    /// strategy.
    pub fn start_day(
        &mut self,
        instruments: impl IntoIterator<Item = Instrument>,
        books: impl IntoIterator<Item = (BookId, BookState)>,
    ) {
        self.instruments = instruments
            .into_iter()
            .map(|instrument| (instrument.symbol, instrument))
            .collect();
        self.books = books.into_iter().collect();
        self.order = self.books.keys().copied().collect();
        self.order.sort_unstable();
    }

    /// A message reaches the strategy. For the code that runs a strategy.
    pub fn apply(&mut self, message: &Message) {
        if let Some(state) = self.books.get_mut(&BookId::of(message)) {
            state.apply(message);
        }
    }
}

/// The account as the strategy knows it: the state at the start of the day and the
/// reports that have reached the strategy since.
#[derive(Debug, Default)]
pub struct Account {
    cash: Money,
    available: Money,
    equity: Money,
    positions: BTreeMap<Symbol, Position>,
}

impl Account {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Settled cash. It changes only before the open, when settlements are paid.
    #[must_use]
    pub const fn cash(&self) -> Money {
        self.cash
    }

    /// What new orders may still commit (`docs/reference/accounting.md` section 7.3),
    /// as of the last report that reached the strategy.
    #[must_use]
    pub const fn available(&self) -> Money {
        self.available
    }

    /// The equity at the last close; the initial cash before the first close of the
    /// run.
    #[must_use]
    pub const fn equity(&self) -> Money {
        self.equity
    }

    /// The shares of one instrument, including today's fills that have been reported.
    #[must_use]
    pub fn position(&self, symbol: Symbol) -> Position {
        self.positions.get(&symbol).copied().unwrap_or_default()
    }

    /// Every instrument with a position, in symbol order.
    pub fn positions(&self) -> impl Iterator<Item = (Symbol, Position)> {
        self.positions
            .iter()
            .map(|(&symbol, &position)| (symbol, position))
    }

    /// Replaces the whole view. For the code that runs a strategy.
    pub fn reset(
        &mut self,
        cash: Money,
        available: Money,
        equity: Money,
        positions: impl IntoIterator<Item = (Symbol, Position)>,
    ) {
        self.cash = cash;
        self.available = available;
        self.equity = equity;
        self.positions = positions
            .into_iter()
            .filter(|(_, position)| !position.is_flat())
            .collect();
    }

    /// A report reaches the strategy with the account as it stood when the report was
    /// produced. For the code that runs a strategy.
    pub fn update(&mut self, available: Money, symbol: Symbol, position: Position) {
        self.available = available;
        if position.is_flat() {
            self.positions.remove(&symbol);
        } else {
            self.positions.insert(symbol, position);
        }
    }
}
