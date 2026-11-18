use std::collections::HashMap;

use osmium_book::{BookId, BookState, Collecting, Method, Mode};
use osmium_types::{
    Board, Date, Message, NewOrder, OrderChange, OrderEvent, OrderEventKind, OrderId, OrderPrice,
    Price, RejectReason, Request, TimeInForce, Timestamp,
};

use crate::{
    BookSpec, PriceLimits, RestingOrder,
    matcher::{Arrival, Matcher, Sink},
};

/// How an order at the auction price is treated when the opposite side has nothing
/// left there (`docs/reference/matching.md` section 7.2, step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AuctionMarginal {
    /// It is not filled: its place among the orders at that price is unknown.
    #[default]
    Conservative,
    /// It is filled by the auction volume that exceeds its estimated queue.
    Queue,
}

/// Counts of the situations in which the simulator could not use the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Diagnostics {
    /// Continuous trades whose aggressing side the book before them does not show;
    /// they fill no simulated order.
    pub aggressor_unknown: u64,
    /// Orders checked for an instrument without limit prices in marketdata.
    pub limit_unknown: u64,
}

/// The most lots one regular-board order may ask for.
const MAX_LOTS: i64 = 499;

/// A book that is followed but cannot be traded.
#[derive(Debug)]
struct Watched {
    state: BookState,
    reason: RejectReason,
}

/// The matching simulation of one trading day: every book, and every simulated order.
#[derive(Debug)]
pub struct Engine {
    date: Date,
    marginal: AuctionMarginal,
    books: HashMap<BookId, Matcher>,
    watched: HashMap<BookId, Watched>,
    /// Every order ever accepted or refused as new, and the book it was sent to.
    orders: HashMap<OrderId, Option<BookId>>,
    next_ordinal: u64,
    diagnostics: Diagnostics,
}

impl Engine {
    #[must_use]
    pub fn new(date: Date, marginal: AuctionMarginal) -> Self {
        Self {
            date,
            marginal,
            books: HashMap::new(),
            watched: HashMap::new(),
            orders: HashMap::new(),
            next_ordinal: 0,
            diagnostics: Diagnostics::default(),
        }
    }

    /// Adds a book that orders can be sent to. Panics when the book was added before.
    pub fn add_book(&mut self, id: BookId, spec: BookSpec) {
        assert!(!self.watched.contains_key(&id), "{id:?} was added before");
        let state = BookState::new(spec.method, self.date);
        let previous = self.books.insert(id, Matcher::new(id.board, spec, state));
        assert!(previous.is_none(), "{id:?} was added before");
    }

    /// Adds a book whose state is followed but that refuses every order with `reason`:
    /// an instrument marketdata does not describe today, or one the simulator does not
    /// support. Panics when the book was added before.
    pub fn add_untradable_book(&mut self, id: BookId, method: Method, reason: RejectReason) {
        assert!(!self.books.contains_key(&id), "{id:?} was added before");
        let state = BookState::new(method, self.date);
        let previous = self.watched.insert(id, Watched { state, reason });
        assert!(previous.is_none(), "{id:?} was added before");
    }

    /// The disclosed state of a book.
    #[must_use]
    pub fn book(&self, id: BookId) -> Option<&BookState> {
        self.books
            .get(&id)
            .map(Matcher::state)
            .or_else(|| self.watched.get(&id).map(|watched| &watched.state))
    }

    /// The matching facts of a tradable book.
    #[must_use]
    pub fn spec(&self, id: BookId) -> Option<&BookSpec> {
        self.books.get(&id).map(Matcher::spec)
    }

    /// An order that is resting in its book now.
    #[must_use]
    pub fn resting(&self, id: OrderId) -> Option<RestingOrder> {
        let book = (*self.orders.get(&id)?)?;
        self.books.get(&book)?.resting(id)
    }

    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// A market message takes effect at the exchange, at its `observed_time`. Messages
    /// of books that were not added are ignored.
    pub fn on_market(&mut self, message: &Message, out: &mut Vec<OrderEvent>) {
        let id = BookId::of(message);
        if let Some(matcher) = self.books.get_mut(&id) {
            matcher.on_market(message, self.marginal, out, &mut self.diagnostics);
        } else if let Some(watched) = self.watched.get_mut(&id) {
            watched.state.apply(message);
        }
    }

    /// Whether a book takes new orders and changes at `now` (section 4.3).
    fn accepts(&self, board: Board, mode: Mode, now: Timestamp) -> bool {
        if mode == Mode::Closed {
            return false;
        }
        let at = |hour, minute| self.date.at_taipei(hour, minute, 0);
        let close = at(13, 30);
        match board {
            Board::Regular => {
                (at(8, 30) <= now && now < close)
                    // A delayed close takes orders again for two minutes.
                    || (mode == Mode::Collecting(Collecting::Close)
                        && at(13, 31) <= now
                        && now < at(13, 33))
            }
            Board::OddLot => {
                let early = Date::from_ymd(2026, 12, 7).expect("a valid date");
                let open = if self.date >= early {
                    at(8, 30)
                } else {
                    at(9, 0)
                };
                open <= now && now < close
            }
        }
    }

    /// Checks 4 and 5 of section 5.2 for a limit price.
    fn check_price(&mut self, spec: &BookSpec, price: Price) -> Result<(), RejectReason> {
        if !spec.class.is_on_tick(price) {
            return Err(RejectReason::InvalidTick);
        }
        match spec.limits {
            PriceLimits::Range { down, up } if price < down || price > up => {
                Err(RejectReason::PriceOutOfLimit)
            }
            PriceLimits::Range { .. } | PriceLimits::Unlimited => Ok(()),
            PriceLimits::Unknown => {
                self.diagnostics.limit_unknown += 1;
                Ok(())
            }
        }
    }

    /// Checks 1 to 7 of section 5.2.
    fn check_new(&mut self, order: &NewOrder, now: Timestamp) -> Result<BookId, RejectReason> {
        let id = BookId {
            symbol: order.symbol,
            board: order.board,
        };
        let Some(matcher) = self.books.get(&id) else {
            return Err(self
                .watched
                .get(&id)
                .map_or(RejectReason::UnknownInstrument, |watched| watched.reason));
        };
        let spec = *matcher.spec();
        let mode = matcher.state().mode(now);
        if !self.accepts(order.board, mode, now) {
            return Err(RejectReason::MarketClosed);
        }
        let lot = i64::from(spec.shares_per_lot);
        let qty = order.qty.get();
        let valid_qty = match order.board {
            Board::Regular => qty > 0 && qty % lot == 0 && qty <= MAX_LOTS * lot,
            Board::OddLot => qty >= 1 && qty < lot,
        };
        if !valid_qty {
            return Err(RejectReason::InvalidQuantity);
        }
        if let OrderPrice::Limit(price) = order.price {
            self.check_price(&spec, price)?;
        }
        let limit_rod =
            matches!(order.price, OrderPrice::Limit(_)) && order.time_in_force == TimeInForce::Rod;
        if mode != Mode::Continuous && !limit_rod {
            return Err(RejectReason::OrderTypeNotAllowed);
        }
        if order.price == OrderPrice::Market {
            match spec.limits {
                PriceLimits::Range { .. } => {}
                PriceLimits::Unlimited => return Err(RejectReason::OrderTypeNotAllowed),
                PriceLimits::Unknown => {
                    self.diagnostics.limit_unknown += 1;
                    return Err(RejectReason::OrderTypeNotAllowed);
                }
            }
        }
        Ok(id)
    }

    /// A new order arrives at the exchange at `now`.
    ///
    /// The exchange checks run first; `account` is then asked for the account checks
    /// (check 8 of section 5.2) and is not called for an order the exchange refuses.
    /// The order is accepted and matched only when both pass. Panics when `id` was
    /// used before.
    pub fn submit(
        &mut self,
        id: OrderId,
        order: &NewOrder,
        now: Timestamp,
        account: impl FnOnce() -> Result<(), RejectReason>,
        out: &mut Vec<OrderEvent>,
    ) {
        assert!(!self.orders.contains_key(&id), "{id:?} was used before");
        let checked = self
            .check_new(order, now)
            .and_then(|book| account().map(|()| book));
        let mut sink = Sink {
            time: now,
            trigger: None,
            out,
        };
        match checked {
            Err(reason) => {
                self.orders.insert(id, None);
                sink.out.push(OrderEvent {
                    order: id,
                    time: now,
                    kind: OrderEventKind::Rejected {
                        request: Request::New,
                        reason,
                    },
                });
            }
            Ok(book) => {
                self.orders.insert(id, Some(book));
                sink.out.push(OrderEvent {
                    order: id,
                    time: now,
                    kind: OrderEventKind::Accepted,
                });
                let ordinal = self.next_ordinal;
                self.next_ordinal += 1;
                let matcher = self.books.get_mut(&book).expect("checked above");
                matcher.place(
                    &Arrival {
                        id,
                        side: order.side,
                        price: order.price,
                        time_in_force: order.time_in_force,
                        qty: order.qty,
                        ordinal,
                    },
                    &mut sink,
                );
            }
        }
    }

    /// The checks of a change to a resting order (section 5.5).
    fn check_change(
        &mut self,
        book: BookId,
        resting: &RestingOrder,
        change: OrderChange,
        now: Timestamp,
    ) -> Result<(), RejectReason> {
        let matcher = &self.books[&book];
        let spec = *matcher.spec();
        let mode = matcher.state().mode(now);
        match change {
            // Cancelling is possible at any time.
            OrderChange::Cancel => Ok(()),
            OrderChange::Reduce { by } => {
                if !self.accepts(book.board, mode, now) {
                    return Err(RejectReason::MarketClosed);
                }
                let lot = i64::from(spec.shares_per_lot);
                let valid = by.get() > 0 && (book.board == Board::OddLot || by.get() % lot == 0);
                if valid {
                    Ok(())
                } else {
                    Err(RejectReason::InvalidQuantity)
                }
            }
            OrderChange::Reprice { price } => {
                if book.board == Board::OddLot || price == resting.price {
                    return Err(RejectReason::ChangeNotAllowed);
                }
                if !self.accepts(book.board, mode, now) {
                    return Err(RejectReason::MarketClosed);
                }
                self.check_price(&spec, price)
            }
        }
    }

    /// A change to an order arrives at the exchange at `now`.
    ///
    /// `account` is asked about a new price after the exchange checks passed, with the
    /// order as it rests now; it is not called for other changes.
    pub fn change(
        &mut self,
        id: OrderId,
        change: OrderChange,
        now: Timestamp,
        account: impl FnOnce(&RestingOrder) -> Result<(), RejectReason>,
        out: &mut Vec<OrderEvent>,
    ) {
        let request = match change {
            OrderChange::Cancel => Request::Cancel,
            OrderChange::Reduce { .. } => Request::Reduce,
            OrderChange::Reprice { .. } => Request::Reprice,
        };
        let located = match self.orders.get(&id) {
            None => Err(RejectReason::UnknownOrder),
            Some(None) => Err(RejectReason::TooLate),
            Some(&Some(book)) => self.books[&book]
                .resting(id)
                .map(|resting| (book, resting))
                .ok_or(RejectReason::TooLate),
        };
        let checked = located.and_then(|(book, resting)| {
            self.check_change(book, &resting, change, now)?;
            if matches!(change, OrderChange::Reprice { .. }) {
                account(&resting)?;
            }
            Ok(book)
        });
        let mut sink = Sink {
            time: now,
            trigger: None,
            out,
        };
        let book = match checked {
            Ok(book) => book,
            Err(reason) => {
                sink.out.push(OrderEvent {
                    order: id,
                    time: now,
                    kind: OrderEventKind::Rejected { request, reason },
                });
                return;
            }
        };
        let matcher = self.books.get_mut(&book).expect("located above");
        match change {
            OrderChange::Cancel => {
                matcher.cancel(id, &mut sink);
            }
            OrderChange::Reduce { by } => matcher.reduce(id, by, &mut sink),
            OrderChange::Reprice { price } => {
                let ordinal = self.next_ordinal;
                self.next_ordinal += 1;
                matcher.reprice(id, price, ordinal, &mut sink);
            }
        }
    }

    /// The trading day is over at `now`: every order still resting expires, in book
    /// order and then arrival order.
    pub fn end_day(&mut self, now: Timestamp, out: &mut Vec<OrderEvent>) {
        let mut books: Vec<BookId> = self.books.keys().copied().collect();
        books.sort_unstable();
        let mut sink = Sink {
            time: now,
            trigger: None,
            out,
        };
        for book in books {
            self.books
                .get_mut(&book)
                .expect("a key of the map")
                .expire(&mut sink);
        }
    }
}
