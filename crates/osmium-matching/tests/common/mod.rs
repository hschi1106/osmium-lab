//! A small market for the rule tests: one engine, hand-written messages and orders.
#![allow(dead_code)]

use osmium_book::{BookId, Method};
use osmium_matching::{
    AuctionMarginal, BookSpec, Engine, InstrumentClass, PriceLimits, QueueAhead,
};
use osmium_types::{
    Board, Book, BookSide, CancelReason, Date, DelayDirection, Feed, FillBasis, Level, Message,
    MessageBody, NewOrder, OrderChange, OrderEvent, OrderEventKind, OrderId, OrderPrice, Phase,
    Price, RejectReason, Request, Shares, Side, Symbol, TimeInForce, Timestamp, Trade, TradeKind,
};

pub const CODE: &str = "2330";

pub fn price(text: &str) -> Price {
    text.parse().unwrap()
}

pub fn symbol() -> Symbol {
    Symbol::new(CODE).unwrap()
}

/// Levels written as `(price, shares)`, best first.
pub type Levels<'a> = &'a [(&'a str, i64)];

fn side(levels: Levels, market_qty: i64) -> BookSide {
    let levels: Vec<Level> = levels
        .iter()
        .map(|&(at, qty)| Level {
            price: price(at),
            qty: Shares::new(qty),
        })
        .collect();
    BookSide::new(&levels, Shares::new(market_qty)).unwrap()
}

pub fn book(bid: Levels, ask: Levels) -> Book {
    Book::new(side(bid, 0), side(ask, 0)).unwrap()
}

pub fn book_with_market(bid: Levels, bid_market: i64, ask: Levels, ask_market: i64) -> Book {
    Book::new(side(bid, bid_market), side(ask, ask_market)).unwrap()
}

/// A stock on the regular board in continuous trading: 1,000-share lots, reference
/// price 100, limits 90 to 110.
pub fn stock() -> BookSpec {
    BookSpec {
        class: InstrumentClass::Stock,
        method: Method::Continuous,
        shares_per_lot: 1_000,
        limits: PriceLimits::Range {
            down: price("90"),
            up: price("110"),
        },
        ref_price: Some(price("100")),
    }
}

/// What a test reads back from an order event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    Accepted,
    Rejected(Request, RejectReason),
    /// Price, shares, basis.
    Filled(String, i64, FillBasis),
    Cancelled(i64, CancelReason),
    Reduced(i64, i64),
    Repriced(String),
    Expired(i64),
    PriceBandLimited,
}

pub fn filled(at: &str, qty: i64, basis: FillBasis) -> Seen {
    Seen::Filled(at.to_owned(), qty, basis)
}

pub struct Market {
    pub engine: Engine,
    pub date: Date,
    pub board: Board,
    feed: Feed,
    seq: u64,
    next_order: u64,
    events: Vec<OrderEvent>,
}

impl Market {
    pub fn new(date: Date, board: Board, spec: BookSpec, marginal: AuctionMarginal) -> Self {
        let mut engine = Engine::new(date, marginal);
        engine.add_book(
            BookId {
                symbol: symbol(),
                board,
            },
            spec,
        );
        Self {
            engine,
            date,
            board,
            feed: match board {
                Board::Regular => Feed::TwseF6,
                Board::OddLot => Feed::TwseF23,
            },
            seq: 0,
            next_order: 0,
            events: Vec::new(),
        }
    }

    /// The regular-board book of [`stock`], before its opening auction.
    pub fn regular() -> Self {
        Self::of(stock())
    }

    pub fn of(spec: BookSpec) -> Self {
        Self::new(
            Date::from_ymd(2026, 10, 8).unwrap(),
            Board::Regular,
            spec,
            AuctionMarginal::Conservative,
        )
    }

    /// The odd-lot book of [`stock`], before its opening auction.
    pub fn odd_lot(marginal: AuctionMarginal) -> Self {
        let mut spec = stock();
        spec.method = Method::Call;
        Self::new(
            Date::from_ymd(2026, 10, 8).unwrap(),
            Board::OddLot,
            spec,
            marginal,
        )
    }

    /// A regular-board book that has opened at 09:00:00 without a trade, showing
    /// `bid` and `ask`.
    pub fn opened(bid: Levels, ask: Levels) -> Self {
        let mut market = Self::regular();
        market.auction("09:00:00", Phase::OpenAuction, book(bid, ask), None);
        market
    }

    pub fn id(&self) -> BookId {
        BookId {
            symbol: symbol(),
            board: self.board,
        }
    }

    /// `HH:MM:SS` or `HH:MM:SS.ffffff` on the market's date, Taipei time.
    pub fn at(&self, text: &str) -> Timestamp {
        let (clock, fraction) = text.split_once('.').unwrap_or((text, ""));
        let parts: Vec<u32> = clock.split(':').map(|part| part.parse().unwrap()).collect();
        let micros: i64 = format!("{fraction:0<6}").parse().unwrap();
        self.date
            .at_taipei(parts[0], parts[1], parts[2])
            .plus_micros(micros)
    }

    fn message(&mut self, time: &str, phase: Phase, body: MessageBody) {
        self.seq += 1;
        let time = self.at(time);
        let message = Message {
            feed: self.feed,
            seq: self.seq,
            symbol: symbol(),
            exch_ts: time,
            observed_time: time,
            phase,
            body,
        };
        self.engine.on_market(&message, &mut self.events);
    }

    /// The `seq` of the last message sent.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// A continuous disclosure without a trade.
    pub fn disclose(&mut self, time: &str, bid: Levels, ask: Levels) {
        self.disclose_book(time, book(bid, ask));
    }

    pub fn disclose_book(&mut self, time: &str, book: Book) {
        self.message(
            time,
            Phase::Continuous,
            MessageBody::Disclosure { book, trade: None },
        );
    }

    /// A continuous trade with the book after it.
    pub fn trade(&mut self, time: &str, at: &str, qty: i64, bid: Levels, ask: Levels) {
        self.message(
            time,
            Phase::Continuous,
            MessageBody::Disclosure {
                book: book(bid, ask),
                trade: Some(trade(at, qty)),
            },
        );
    }

    /// A trade without a book: an earlier level of a sweep.
    pub fn intermediate(&mut self, time: &str, at: &str, qty: i64) {
        self.message(
            time,
            Phase::Continuous,
            MessageBody::IntermediateTrade(trade(at, qty)),
        );
    }

    /// An auction result: a disclosure outside continuous trading.
    pub fn auction(&mut self, time: &str, phase: Phase, book: Book, traded: Option<(&str, i64)>) {
        self.message(
            time,
            phase,
            MessageBody::Disclosure {
                book,
                trade: traded.map(|(at, qty)| trade(at, qty)),
            },
        );
    }

    pub fn indicative(
        &mut self,
        time: &str,
        phase: Phase,
        book: Book,
        matched: Option<(&str, i64)>,
    ) {
        self.message(
            time,
            phase,
            MessageBody::Indicative {
                book,
                matched: matched.map(|(at, qty)| Level {
                    price: price(at),
                    qty: Shares::new(qty),
                }),
                delay_open: Some(false),
                delay_close: Some(false),
            },
        );
    }

    pub fn delay(&mut self, time: &str) {
        let phase = match self.engine.spec(self.id()).unwrap().method {
            Method::Continuous => Phase::Continuous,
            Method::Call => Phase::PeriodicAuction,
        };
        self.message(time, phase, MessageBody::Delay(DelayDirection::Up));
    }

    pub fn order(&self, side: Side, limit: &str, qty: i64) -> NewOrder {
        NewOrder {
            symbol: symbol(),
            board: self.board,
            side,
            kind: TradeKind::Cash,
            price: OrderPrice::Limit(price(limit)),
            time_in_force: TimeInForce::Rod,
            qty: Shares::new(qty),
        }
    }

    /// Sends an order that the account accepts.
    pub fn send(&mut self, time: &str, order: NewOrder) -> OrderId {
        self.send_with(time, order, || Ok(()))
    }

    pub fn send_with(
        &mut self,
        time: &str,
        order: NewOrder,
        account: impl FnOnce() -> Result<(), RejectReason>,
    ) -> OrderId {
        self.next_order += 1;
        let id = OrderId(self.next_order);
        let now = self.at(time);
        self.engine
            .submit(id, &order, now, account, &mut self.events);
        id
    }

    /// A limit day order to buy.
    pub fn buy(&mut self, time: &str, limit: &str, qty: i64) -> OrderId {
        let order = self.order(Side::Buy, limit, qty);
        self.send(time, order)
    }

    /// A limit day order to sell.
    pub fn sell(&mut self, time: &str, limit: &str, qty: i64) -> OrderId {
        let order = self.order(Side::Sell, limit, qty);
        self.send(time, order)
    }

    pub fn change(&mut self, time: &str, id: OrderId, change: OrderChange) {
        let now = self.at(time);
        self.engine
            .change(id, change, now, |_| Ok(()), &mut self.events);
    }

    pub fn cancel(&mut self, time: &str, id: OrderId) {
        self.change(time, id, OrderChange::Cancel);
    }

    pub fn end_day(&mut self, time: &str) {
        let now = self.at(time);
        self.engine.end_day(now, &mut self.events);
    }

    /// The raw events since the last call.
    pub fn take(&mut self) -> Vec<OrderEvent> {
        std::mem::take(&mut self.events)
    }

    /// The events since the last call, by order.
    pub fn seen(&mut self) -> Vec<(OrderId, Seen)> {
        self.take()
            .into_iter()
            .map(|event| {
                let seen = match event.kind {
                    OrderEventKind::Accepted => Seen::Accepted,
                    OrderEventKind::Rejected { request, reason } => Seen::Rejected(request, reason),
                    OrderEventKind::Filled(fill) => {
                        Seen::Filled(fill.price.to_string(), fill.qty.get(), fill.basis)
                    }
                    OrderEventKind::Cancelled { qty, reason } => Seen::Cancelled(qty.get(), reason),
                    OrderEventKind::Reduced { by, remaining } => {
                        Seen::Reduced(by.get(), remaining.get())
                    }
                    OrderEventKind::Repriced { price } => Seen::Repriced(price.to_string()),
                    OrderEventKind::Expired { qty } => Seen::Expired(qty.get()),
                    OrderEventKind::PriceBandLimited => Seen::PriceBandLimited,
                };
                (event.order, seen)
            })
            .collect()
    }

    /// The events of one order since the last call; events of other orders are dropped.
    pub fn seen_for(&mut self, id: OrderId) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|&(order, _)| order == id)
            .map(|(_, seen)| seen)
            .collect()
    }

    /// The remaining quantity of a resting order.
    pub fn remaining(&self, id: OrderId) -> Option<i64> {
        self.engine.resting(id).map(|order| order.remaining.get())
    }

    /// The known queue ahead of a resting order; panics when the order is not resting
    /// or its queue is unknown.
    pub fn queue(&self, id: OrderId) -> i64 {
        match self
            .engine
            .resting(id)
            .expect("a resting order")
            .queue_ahead
        {
            QueueAhead::Known(queue) => queue.get(),
            QueueAhead::Unknown => panic!("the queue of {id:?} is unknown"),
        }
    }

    pub fn queue_is_unknown(&self, id: OrderId) -> bool {
        self.engine
            .resting(id)
            .expect("a resting order")
            .queue_ahead
            == QueueAhead::Unknown
    }
}

pub fn trade(at: &str, qty: i64) -> Trade {
    Trade {
        price: price(at),
        qty: Shares::new(qty),
        cum_qty: None,
    }
}
