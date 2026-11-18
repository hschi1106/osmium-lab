use osmium_types::{
    Board, Book, Date, DelayDirection, Level, Message, MessageBody, Phase, Symbol, Timestamp, Trade,
};

use crate::Method;

/// One instrument on one board. The regular and the odd-lot book of an instrument are
/// separate books.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BookId {
    pub symbol: Symbol,
    pub board: Board,
}

impl BookId {
    /// The book a message belongs to.
    #[must_use]
    pub const fn of(message: &Message) -> Self {
        Self {
            symbol: message.symbol,
            board: message.feed.board(),
        }
    }
}

/// Why a book is collecting orders for a call auction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Collecting {
    /// Before the opening auction.
    Open,
    /// The book matches by call auction all day.
    Periodic,
    /// Before the closing auction.
    Close,
    /// Matching is postponed by the exchange's price-stabilisation measure.
    Volatility,
}

/// What the exchange does with an order that arrives now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Orders wait for the next call auction.
    Collecting(Collecting),
    /// Orders are matched as they arrive.
    Continuous,
    /// The closing auction is over.
    Closed,
}

/// The latest indicative (simulated) auction result of a book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indicative {
    /// The levels disclosed with the indicative result. They are what would remain
    /// after the simulated match, not resting quantities.
    pub book: Book,
    /// The simulated match price and quantity; `None` when nothing would match.
    pub matched: Option<Level>,
    /// The exchange announces a delayed open; `None` when the source has no such flag.
    pub delay_open: Option<bool>,
    /// The exchange announces a delayed close; `None` when the source has no such flag.
    pub delay_close: Option<bool>,
}

/// Whether a message is the result of a call auction: a disclosure outside continuous
/// trading, with or without a trade.
#[must_use]
pub const fn is_auction_result(message: &Message) -> bool {
    matches!(message.body, MessageBody::Disclosure { .. })
        && !matches!(message.phase, Phase::Continuous)
}

/// What the exchange has disclosed about one book so far on one trading day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookState {
    method: Method,
    close_collection: Timestamp,
    book: Book,
    last_trade: Option<Trade>,
    indicative: Option<Indicative>,
    delay: Option<DelayDirection>,
    opened: bool,
    closed: bool,
}

impl BookState {
    /// The state of a book before its first message of `date`.
    #[must_use]
    pub fn new(method: Method, date: Date) -> Self {
        Self {
            method,
            close_collection: Self::close_collection(date),
            book: Book::EMPTY,
            last_trade: None,
            indicative: None,
            delay: None,
            opened: false,
            closed: false,
        }
    }

    /// When order collection for the closing auction starts on `date`: 13:25:00.
    #[must_use]
    pub fn close_collection(date: Date) -> Timestamp {
        date.at_taipei(13, 25, 0)
    }

    /// Applies the next message of this book.
    pub fn apply(&mut self, message: &Message) {
        match message.body {
            MessageBody::Disclosure { book, trade } => {
                self.book = book;
                if trade.is_some() {
                    self.last_trade = trade;
                }
                if is_auction_result(message) {
                    self.indicative = None;
                    self.delay = None;
                    match message.phase {
                        Phase::OpenAuction => self.opened = true,
                        Phase::CloseAuction => self.closed = true,
                        Phase::PeriodicAuction | Phase::Continuous => {}
                    }
                }
            }
            MessageBody::IntermediateTrade(trade) => self.last_trade = Some(trade),
            MessageBody::Indicative {
                book,
                matched,
                delay_open,
                delay_close,
            } => {
                self.indicative = Some(Indicative {
                    book,
                    matched,
                    delay_open,
                    delay_close,
                });
            }
            MessageBody::Delay(direction) => self.delay = Some(direction),
        }
    }

    #[must_use]
    pub const fn method(&self) -> Method {
        self.method
    }

    /// The mode at replay time `now`, given the messages applied so far.
    #[must_use]
    pub fn mode(&self, now: Timestamp) -> Mode {
        if !self.opened {
            Mode::Collecting(Collecting::Open)
        } else if self.closed {
            Mode::Closed
        } else if self.method == Method::Call {
            Mode::Collecting(Collecting::Periodic)
        } else if now >= self.close_collection {
            Mode::Collecting(Collecting::Close)
        } else if self.delay.is_some() {
            Mode::Collecting(Collecting::Volatility)
        } else {
            Mode::Continuous
        }
    }

    /// The five levels of the latest disclosure; empty before the first one. Indicative
    /// and delay messages do not change it.
    #[must_use]
    pub const fn book(&self) -> &Book {
        &self.book
    }

    /// The latest real trade of the day.
    #[must_use]
    pub const fn last_trade(&self) -> Option<Trade> {
        self.last_trade
    }

    /// The latest indicative result since the last auction result.
    #[must_use]
    pub const fn indicative(&self) -> Option<&Indicative> {
        self.indicative.as_ref()
    }

    /// The direction of a matching delay that no auction result has ended yet.
    #[must_use]
    pub const fn delay(&self) -> Option<DelayDirection> {
        self.delay
    }

    /// Whether the opening auction result has been disclosed.
    #[must_use]
    pub const fn opened(&self) -> bool {
        self.opened
    }

    /// Whether the closing auction result has been disclosed.
    #[must_use]
    pub const fn closed(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use osmium_types::{
        Board, Book, BookSide, Date, DelayDirection, Feed, Level, Message, MessageBody, Phase,
        Shares, Symbol, Timestamp, Trade,
    };

    use super::{BookId, BookState, Collecting, Indicative, Mode, is_auction_result};
    use crate::Method;

    pub(crate) fn date() -> Date {
        Date::from_ymd(2026, 10, 8).unwrap()
    }

    pub(crate) fn at(hour: u32, minute: u32, second: u32) -> Timestamp {
        date().at_taipei(hour, minute, second)
    }

    pub(crate) fn message(time: Timestamp, phase: Phase, body: MessageBody) -> Message {
        Message {
            feed: Feed::TwseF6,
            seq: 1,
            symbol: Symbol::new("2330").unwrap(),
            exch_ts: time,
            observed_time: time,
            phase,
            body,
        }
    }

    fn level(price: &str, qty: i64) -> Level {
        Level {
            price: price.parse().unwrap(),
            qty: Shares::new(qty),
        }
    }

    pub(crate) fn book(bid: &str, ask: &str) -> Book {
        Book::new(
            BookSide::new(&[level(bid, 1_000)], Shares::ZERO).unwrap(),
            BookSide::new(&[level(ask, 2_000)], Shares::ZERO).unwrap(),
        )
        .unwrap()
    }

    fn trade(price: &str, qty: i64) -> Trade {
        Trade {
            price: price.parse().unwrap(),
            qty: Shares::new(qty),
            cum_qty: None,
        }
    }

    pub(crate) fn disclosure(book: Book, trade: Option<Trade>) -> MessageBody {
        MessageBody::Disclosure { book, trade }
    }

    fn indicative(book: Book, matched: Option<Level>) -> MessageBody {
        MessageBody::Indicative {
            book,
            matched,
            delay_open: Some(false),
            delay_close: Some(false),
        }
    }

    fn state(method: Method) -> BookState {
        BookState::new(method, date())
    }

    fn opened(method: Method) -> BookState {
        let mut state = state(method);
        state.apply(&message(
            at(9, 0, 0),
            Phase::OpenAuction,
            disclosure(book("100", "101"), Some(trade("100.5", 5_000))),
        ));
        state
    }

    #[test]
    fn a_book_is_one_symbol_on_one_board() {
        let mut odd = message(
            at(9, 0, 0),
            Phase::OpenAuction,
            disclosure(Book::EMPTY, None),
        );
        odd.feed = Feed::TwseF23;
        let regular = message(
            at(9, 0, 0),
            Phase::OpenAuction,
            disclosure(Book::EMPTY, None),
        );
        assert_eq!(BookId::of(&regular).board, Board::Regular);
        assert_eq!(BookId::of(&odd).board, Board::OddLot);
        assert_eq!(BookId::of(&regular).symbol, BookId::of(&odd).symbol);
        assert_ne!(BookId::of(&regular), BookId::of(&odd));
    }

    #[test]
    fn a_book_collects_orders_until_its_opening_auction_result() {
        let mut state = state(Method::Continuous);
        assert_eq!(state.mode(at(8, 30, 0)), Mode::Collecting(Collecting::Open));
        assert_eq!(*state.book(), Book::EMPTY);

        let trial = book("99", "102");
        state.apply(&message(
            at(8, 45, 0),
            Phase::OpenAuction,
            indicative(trial, Some(level("100.5", 3_000))),
        ));
        // A delayed open needs no rule of its own: the book collects until the result.
        assert_eq!(state.mode(at(9, 4, 0)), Mode::Collecting(Collecting::Open));
        assert_eq!(
            *state.book(),
            Book::EMPTY,
            "indicative levels are not the book"
        );
        assert_eq!(
            state.indicative(),
            Some(&Indicative {
                book: trial,
                matched: Some(level("100.5", 3_000)),
                delay_open: Some(false),
                delay_close: Some(false),
            })
        );

        let open = book("100", "101");
        state.apply(&message(
            at(9, 4, 0),
            Phase::OpenAuction,
            disclosure(open, Some(trade("100.5", 5_000))),
        ));
        assert!(state.opened());
        assert_eq!(state.mode(at(9, 4, 0)), Mode::Continuous);
        assert_eq!(*state.book(), open);
        assert_eq!(state.last_trade(), Some(trade("100.5", 5_000)));
        assert_eq!(
            state.indicative(),
            None,
            "the result ends the trial matches"
        );
    }

    #[test]
    fn an_opening_auction_without_a_trade_still_opens_the_book() {
        let mut state = state(Method::Continuous);
        state.apply(&message(
            at(9, 0, 0),
            Phase::OpenAuction,
            disclosure(book("100", "101"), None),
        ));
        assert_eq!(state.mode(at(9, 0, 0)), Mode::Continuous);
        assert_eq!(state.last_trade(), None);
    }

    #[test]
    fn only_disclosures_replace_the_book() {
        let mut state = opened(Method::Continuous);
        let before = *state.book();

        state.apply(&message(
            at(9, 1, 0),
            Phase::Continuous,
            MessageBody::IntermediateTrade(trade("101", 1_000)),
        ));
        assert_eq!(*state.book(), before);
        assert_eq!(state.last_trade(), Some(trade("101", 1_000)));

        state.apply(&message(
            at(9, 1, 0),
            Phase::Continuous,
            MessageBody::Delay(DelayDirection::Up),
        ));
        state.apply(&message(
            at(9, 1, 5),
            Phase::PeriodicAuction,
            indicative(book("90", "110"), None),
        ));
        assert_eq!(*state.book(), before);

        let after = book("100.5", "101.5");
        state.apply(&message(
            at(9, 3, 0),
            Phase::Continuous,
            disclosure(after, None),
        ));
        assert_eq!(*state.book(), after);
        assert_eq!(
            state.last_trade(),
            Some(trade("101", 1_000)),
            "a disclosure without a trade keeps the last trade"
        );
    }

    #[test]
    fn a_delay_collects_orders_until_the_next_auction_result() {
        let mut state = opened(Method::Continuous);
        state.apply(&message(
            at(9, 10, 0),
            Phase::Continuous,
            MessageBody::Delay(DelayDirection::Down),
        ));
        assert_eq!(state.delay(), Some(DelayDirection::Down));
        assert_eq!(
            state.mode(at(9, 10, 0)),
            Mode::Collecting(Collecting::Volatility)
        );

        state.apply(&message(
            at(9, 10, 5),
            Phase::PeriodicAuction,
            indicative(book("95", "96"), Some(level("95.5", 1_000))),
        ));
        assert_eq!(
            state.mode(at(9, 11, 0)),
            Mode::Collecting(Collecting::Volatility)
        );

        // The auction that ends the delay may match nothing.
        state.apply(&message(
            at(9, 12, 0),
            Phase::PeriodicAuction,
            disclosure(book("95", "96"), None),
        ));
        assert_eq!(state.delay(), None);
        assert_eq!(state.indicative(), None);
        assert_eq!(state.mode(at(9, 12, 0)), Mode::Continuous);
    }

    #[test]
    fn a_continuous_disclosure_does_not_end_a_delay() {
        let mut state = opened(Method::Continuous);
        state.apply(&message(
            at(9, 10, 0),
            Phase::Continuous,
            MessageBody::Delay(DelayDirection::Up),
        ));
        state.apply(&message(
            at(9, 10, 0),
            Phase::Continuous,
            disclosure(book("100", "101"), None),
        ));
        assert_eq!(
            state.mode(at(9, 10, 1)),
            Mode::Collecting(Collecting::Volatility)
        );
    }

    #[test]
    fn closing_collection_starts_at_1325_by_the_clock() {
        let state = opened(Method::Continuous);
        assert_eq!(state.mode(at(13, 24, 59)), Mode::Continuous);
        assert_eq!(
            state.mode(at(13, 25, 0)),
            Mode::Collecting(Collecting::Close)
        );
        // A delayed close keeps collecting until the result arrives.
        assert_eq!(
            state.mode(at(13, 32, 0)),
            Mode::Collecting(Collecting::Close)
        );
    }

    #[test]
    fn closing_collection_takes_precedence_over_a_delay() {
        let mut state = opened(Method::Continuous);
        state.apply(&message(
            at(13, 24, 30),
            Phase::Continuous,
            MessageBody::Delay(DelayDirection::Up),
        ));
        assert_eq!(
            state.mode(at(13, 24, 30)),
            Mode::Collecting(Collecting::Volatility)
        );
        assert_eq!(
            state.mode(at(13, 25, 0)),
            Mode::Collecting(Collecting::Close)
        );
    }

    #[test]
    fn the_closing_auction_result_closes_the_book() {
        let mut state = opened(Method::Continuous);
        state.apply(&message(
            at(13, 25, 0),
            Phase::CloseAuction,
            indicative(book("100", "101"), None),
        ));
        assert!(!state.closed());
        let close = book("100", "102");
        state.apply(&message(
            at(13, 30, 0),
            Phase::CloseAuction,
            disclosure(close, Some(trade("101", 9_000))),
        ));
        assert!(state.closed());
        assert_eq!(state.mode(at(13, 30, 0)), Mode::Closed);
        assert_eq!(*state.book(), close);
        assert_eq!(state.last_trade(), Some(trade("101", 9_000)));
        assert_eq!(state.indicative(), None);
    }

    #[test]
    fn a_call_book_collects_between_its_auctions_all_day() {
        let mut state = state(Method::Call);
        assert_eq!(state.method(), Method::Call);
        assert_eq!(state.mode(at(8, 59, 0)), Mode::Collecting(Collecting::Open));
        state.apply(&message(
            at(9, 0, 0),
            Phase::OpenAuction,
            disclosure(book("100", "101"), None),
        ));
        assert_eq!(
            state.mode(at(9, 0, 0)),
            Mode::Collecting(Collecting::Periodic)
        );

        state.apply(&message(
            at(9, 5, 0),
            Phase::PeriodicAuction,
            disclosure(book("100", "101"), Some(trade("100.5", 1_000))),
        ));
        // A delay does not change the mode of a book that is collecting anyway.
        state.apply(&message(
            at(9, 6, 0),
            Phase::PeriodicAuction,
            MessageBody::Delay(DelayDirection::Up),
        ));
        assert_eq!(
            state.mode(at(13, 26, 0)),
            Mode::Collecting(Collecting::Periodic)
        );

        state.apply(&message(
            at(13, 30, 0),
            Phase::CloseAuction,
            disclosure(book("100", "101"), None),
        ));
        assert_eq!(state.mode(at(13, 30, 0)), Mode::Closed);
    }

    #[test]
    fn auction_results_are_disclosures_outside_continuous_trading() {
        let quiet = book("100", "101");
        for (phase, body, expected) in [
            (Phase::OpenAuction, disclosure(quiet, None), true),
            (Phase::PeriodicAuction, disclosure(quiet, None), true),
            (Phase::CloseAuction, disclosure(quiet, None), true),
            (Phase::Continuous, disclosure(quiet, None), false),
            (Phase::OpenAuction, indicative(quiet, None), false),
            (
                Phase::PeriodicAuction,
                MessageBody::Delay(DelayDirection::Up),
                false,
            ),
            (
                Phase::Continuous,
                MessageBody::IntermediateTrade(trade("100", 1_000)),
                false,
            ),
        ] {
            assert_eq!(
                is_auction_result(&message(at(9, 0, 0), phase, body)),
                expected,
                "{phase:?} {body:?}"
            );
        }
    }
}
