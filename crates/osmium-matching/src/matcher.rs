//! The matching rules of one book (`docs/reference/matching.md` sections 6 and 7).

use osmium_book::{BookState, Mode, is_auction_result};
use osmium_types::{
    BOOK_DEPTH, Board, Book, BookSide, CancelReason, Feed, Fill, FillBasis, Level, Message,
    MessageBody, OrderEvent, OrderEventKind, OrderId, OrderPrice, Phase, Price, Shares, Side,
    TimeInForce, Timestamp, Trade,
};

use crate::{AuctionMarginal, BookSpec, Diagnostics, PriceBand};

/// The real quantity ahead of a resting order at its price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueAhead {
    Known(Shares),
    /// The price is beyond the fifth disclosed level and has not been seen yet.
    Unknown,
}

/// A simulated order resting in a book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestingOrder {
    pub id: OrderId,
    pub side: Side,
    pub price: Price,
    pub remaining: Shares,
    pub queue_ahead: QueueAhead,
}

#[derive(Debug, Clone, Copy)]
struct Resting {
    id: OrderId,
    price: Price,
    remaining: Shares,
    /// Arrival order among all simulated orders.
    ordinal: u64,
    queue: QueueAhead,
    /// The order arrived while the book was collecting and has not been through an
    /// auction yet.
    joined_collecting: bool,
    band_reported: bool,
}

const fn index(side: Side) -> usize {
    match side {
        Side::Buy => 0,
        Side::Sell => 1,
    }
}

/// The side of the book that orders of `side` rest on.
const fn book_side(book: &Book, side: Side) -> &BookSide {
    match side {
        Side::Buy => book.bid(),
        Side::Sell => book.ask(),
    }
}

/// Whether an order of `side` at `price` can trade with an opposite level at `level`.
fn crosses(side: Side, price: Price, level: Price) -> bool {
    match side {
        Side::Buy => level <= price,
        Side::Sell => level >= price,
    }
}

/// Whether `price` lies beyond the last disclosed level of a full side, where resting
/// quantities are not disclosed.
fn is_hidden(side: Side, levels: &[Level], price: Price) -> bool {
    levels.len() == BOOK_DEPTH && side.is_better(levels[BOOK_DEPTH - 1].price, price)
}

/// The real quantity ahead of an order placed now at `price` (section 6.3).
fn queue_at(side: Side, book: &Book, price: Price) -> QueueAhead {
    let own = book_side(book, side);
    let levels = own.levels();
    match levels.iter().position(|level| level.price == price) {
        Some(0) => QueueAhead::Known(levels[0].qty + own.market_qty()),
        Some(position) => QueueAhead::Known(levels[position].qty),
        None if is_hidden(side, levels, price) => QueueAhead::Unknown,
        None => QueueAhead::Known(Shares::ZERO),
    }
}

/// Matching state of one book: the disclosed state, the simulated orders resting in it
/// and how much of each disclosed level they have used.
#[derive(Debug)]
pub(crate) struct Matcher {
    spec: BookSpec,
    state: BookState,
    band: Option<PriceBand>,
    /// Per book side, the quantity simulated orders took from a price that real
    /// disclosures have not reflected yet.
    consumed: [Vec<Level>; 2],
    /// Resting simulated orders per side, best price first, then earliest arrival.
    orders: [Vec<Resting>; 2],
}

/// An order arriving at the book: a new order, or a repriced one coming back.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Arrival {
    pub(crate) id: OrderId,
    pub(crate) side: Side,
    pub(crate) price: OrderPrice,
    pub(crate) time_in_force: TimeInForce,
    pub(crate) qty: Shares,
    /// Arrival order among all simulated orders.
    pub(crate) ordinal: u64,
}

/// Where the events of one step go, with what every event of the step shares.
pub(crate) struct Sink<'a> {
    pub(crate) time: Timestamp,
    pub(crate) trigger: Option<(Feed, u64)>,
    pub(crate) out: &'a mut Vec<OrderEvent>,
}

impl Sink<'_> {
    fn push(&mut self, order: OrderId, kind: OrderEventKind) {
        self.out.push(OrderEvent {
            order,
            time: self.time,
            kind,
        });
    }

    fn fill(&mut self, order: OrderId, price: Price, qty: Shares, basis: FillBasis) {
        debug_assert!(qty > Shares::ZERO);
        self.push(
            order,
            OrderEventKind::Filled(Fill {
                price,
                qty,
                basis,
                trigger: self.trigger,
            }),
        );
    }
}

impl Matcher {
    pub(crate) fn new(board: Board, spec: BookSpec, state: BookState) -> Self {
        let band = PriceBand::for_book(board, &spec);
        Self {
            spec,
            state,
            band,
            consumed: [Vec::new(), Vec::new()],
            orders: [Vec::new(), Vec::new()],
        }
    }

    pub(crate) const fn spec(&self) -> &BookSpec {
        &self.spec
    }

    pub(crate) const fn state(&self) -> &BookState {
        &self.state
    }

    pub(crate) fn resting(&self, id: OrderId) -> Option<RestingOrder> {
        [Side::Buy, Side::Sell].into_iter().find_map(|side| {
            self.orders[index(side)]
                .iter()
                .find(|order| order.id == id)
                .map(|order| RestingOrder {
                    id,
                    side,
                    price: order.price,
                    remaining: order.remaining,
                    queue_ahead: order.queue,
                })
        })
    }

    /// The disclosed quantity at an opposite `level` that simulated orders can still
    /// take. `side` is the side of the book the level is on.
    fn available(&self, side: Side, level: Level) -> Shares {
        let used = self.consumed[index(side)]
            .iter()
            .find(|used| used.price == level.price)
            .map_or(Shares::ZERO, |used| used.qty);
        Shares::new((level.qty.get() - used.get()).max(0))
    }

    fn consume(&mut self, side: Side, price: Price, qty: Shares) {
        let consumed = &mut self.consumed[index(side)];
        match consumed.iter_mut().find(|used| used.price == price) {
            Some(used) => used.qty += qty,
            None => consumed.push(Level { price, qty }),
        }
    }

    /// Section 6.1: after a disclosure, a level cannot have more used than it shows,
    /// and a price that left the visible book has nothing used.
    fn refresh_consumed(&mut self) {
        let book = *self.state.book();
        for side in [Side::Buy, Side::Sell] {
            let levels = book_side(&book, side).levels();
            self.consumed[index(side)].retain_mut(|used| {
                match levels.iter().find(|level| level.price == used.price) {
                    Some(level) => {
                        used.qty = used.qty.min(level.qty);
                        true
                    }
                    None => is_hidden(side, levels, used.price),
                }
            });
        }
    }

    /// Section 6.3: after a disclosure, a queue only shrinks when the level shows less
    /// than the queue; cancellations are assumed to be behind the order.
    fn refresh_queues(&mut self) {
        let book = *self.state.book();
        for side in [Side::Buy, Side::Sell] {
            for order in &mut self.orders[index(side)] {
                let seen = queue_at(side, &book, order.price);
                order.queue = match (order.queue, seen) {
                    (QueueAhead::Known(queue), QueueAhead::Known(shown)) => {
                        QueueAhead::Known(queue.min(shown))
                    }
                    (QueueAhead::Unknown, shown) => shown,
                    (queue, QueueAhead::Unknown) => queue,
                };
            }
        }
    }

    /// Whether the price-stabilisation band lets an order of `side` trade at `price`.
    fn in_band(&mut self, side: Side, price: Price, now: Timestamp) -> bool {
        self.band
            .as_mut()
            .is_none_or(|band| band.reference(now).allows(side, price))
    }

    fn insert(&mut self, side: Side, order: Resting) {
        let orders = &mut self.orders[index(side)];
        let position = orders
            .iter()
            .position(|other| side.is_better(order.price, other.price))
            .unwrap_or(orders.len());
        orders.insert(position, order);
    }

    /// Puts what remains of a limit order in the book, behind the real quantity its
    /// price shows now (section 6.3).
    fn rest(
        &mut self,
        order: &Arrival,
        price: Price,
        remaining: Shares,
        joined_collecting: bool,
        band_reported: bool,
    ) {
        let queue = queue_at(order.side, self.state.book(), price);
        self.insert(
            order.side,
            Resting {
                id: order.id,
                price,
                remaining,
                ordinal: order.ordinal,
                queue,
                joined_collecting,
                band_reported,
            },
        );
    }

    /// An order that passed every check arrives (sections 6.2 and 7.1).
    pub(crate) fn place(&mut self, order: &Arrival, sink: &mut Sink) {
        match self.state.mode(sink.time) {
            Mode::Continuous => self.aggress(order, sink),
            Mode::Collecting(_) => {
                let price = order
                    .price
                    .limit()
                    .expect("only limit orders while collecting");
                self.rest(order, price, order.qty, true, false);
            }
            Mode::Closed => unreachable!("orders are refused once the book is closed"),
        }
    }

    /// Section 6.2: on arrival in continuous trading an order takes what the opposite
    /// levels show, each at the level's price.
    fn aggress(&mut self, order: &Arrival, sink: &mut Sink) {
        let id = order.id;
        let side = order.side;
        let opposite = side.opposite();
        let book = *self.state.book();
        let mut takes: Vec<Level> = Vec::with_capacity(BOOK_DEPTH);
        let mut remaining = order.qty;
        let mut band_limited = false;
        for &level in book_side(&book, opposite).levels() {
            if remaining == Shares::ZERO {
                break;
            }
            if let OrderPrice::Limit(price) = order.price
                && !crosses(side, price, level.price)
            {
                break;
            }
            let available = self.available(opposite, level);
            if available == Shares::ZERO {
                continue;
            }
            if !self.in_band(side, level.price, sink.time) {
                band_limited = true;
                break;
            }
            let qty = remaining.min(available);
            takes.push(Level {
                price: level.price,
                qty,
            });
            remaining -= qty;
        }

        // A fill-or-kill order that cannot be filled completely trades nothing.
        let killed = order.time_in_force == TimeInForce::Fok && remaining > Shares::ZERO;
        if killed {
            remaining = order.qty;
        } else {
            for take in takes {
                self.consume(opposite, take.price, take.qty);
                sink.fill(id, take.price, take.qty, FillBasis::Aggressive);
            }
        }
        if remaining == Shares::ZERO {
            return;
        }
        if band_limited {
            sink.push(id, OrderEventKind::PriceBandLimited);
        }
        let reason = match (order.price, order.time_in_force) {
            (_, TimeInForce::Fok) => CancelReason::FokUnfilled,
            // Section 6.7: what a market order leaves is cancelled, also for the day.
            (OrderPrice::Market, _) => CancelReason::MarketRemainder,
            (OrderPrice::Limit(_), TimeInForce::Ioc) => CancelReason::IocRemainder,
            (OrderPrice::Limit(price), TimeInForce::Rod) => {
                self.rest(order, price, remaining, false, band_limited);
                return;
            }
        };
        sink.push(
            id,
            OrderEventKind::Cancelled {
                qty: remaining,
                reason,
            },
        );
    }

    /// Section 6.5: a resting order trades, at its own price, with opposite quantity
    /// that has become available at or inside its price.
    fn cross(&mut self, sink: &mut Sink) {
        let book = *self.state.book();
        for side in [Side::Buy, Side::Sell] {
            if self.orders[index(side)].is_empty() {
                continue;
            }
            let opposite = side.opposite();
            let levels = book_side(&book, opposite).levels();
            for position in 0..self.orders[index(side)].len() {
                let mut order = self.orders[index(side)][position];
                for &level in levels {
                    if order.remaining == Shares::ZERO || !crosses(side, order.price, level.price) {
                        break;
                    }
                    let available = self.available(opposite, level);
                    if available == Shares::ZERO {
                        continue;
                    }
                    if !self.in_band(side, level.price, sink.time) {
                        if !order.band_reported {
                            order.band_reported = true;
                            sink.push(order.id, OrderEventKind::PriceBandLimited);
                        }
                        break;
                    }
                    let qty = order.remaining.min(available);
                    self.consume(opposite, level.price, qty);
                    order.remaining -= qty;
                    sink.fill(order.id, order.price, qty, FillBasis::CrossedPassive);
                }
                self.orders[index(side)][position] = order;
            }
            self.orders[index(side)].retain(|order| order.remaining > Shares::ZERO);
        }
    }

    /// Section 6.4: a real trade fills resting orders on the passive side that it
    /// reached. `before` is the book before the aggressing order.
    fn passive(
        &mut self,
        before: &Book,
        trade: Trade,
        sink: &mut Sink,
        diagnostics: &mut Diagnostics,
    ) {
        let aggressor = if before
            .ask()
            .best()
            .is_some_and(|ask| trade.price >= ask.price)
        {
            Side::Buy
        } else if before
            .bid()
            .best()
            .is_some_and(|bid| trade.price <= bid.price)
        {
            Side::Sell
        } else {
            diagnostics.aggressor_unknown += 1;
            return;
        };
        let side = aggressor.opposite();
        let orders = &mut self.orders[index(side)];
        if orders.is_empty() {
            return;
        }
        let mut budget = trade.qty;
        for order in orders.iter_mut() {
            let (qty, basis) = if side.is_better(order.price, trade.price) {
                order.queue = QueueAhead::Known(Shares::ZERO);
                (order.remaining.min(budget), FillBasis::PassiveThrough)
            } else if order.price == trade.price {
                let ahead = match order.queue {
                    QueueAhead::Known(queue) => {
                        let ahead = queue.min(trade.qty);
                        order.queue = QueueAhead::Known(queue - ahead);
                        ahead
                    }
                    QueueAhead::Unknown => trade.qty,
                };
                let reached = Shares::new((budget.get() - ahead.get()).max(0));
                (order.remaining.min(reached), FillBasis::PassiveQueue)
            } else {
                break;
            };
            if qty > Shares::ZERO {
                order.remaining -= qty;
                budget -= qty;
                sink.fill(order.id, order.price, qty, basis);
            }
        }
        orders.retain(|order| order.remaining > Shares::ZERO);
    }

    /// Applies one market message of this book.
    pub(crate) fn on_market(
        &mut self,
        message: &Message,
        marginal: AuctionMarginal,
        out: &mut Vec<OrderEvent>,
        diagnostics: &mut Diagnostics,
    ) {
        let mut sink = Sink {
            time: message.observed_time,
            trigger: Some((message.feed, message.seq)),
            out,
        };
        if is_auction_result(message) {
            self.auction(message, marginal, &mut sink);
            return;
        }
        let continuous = self.state.mode(message.observed_time) == Mode::Continuous;
        match message.body {
            MessageBody::Disclosure { trade, .. } => {
                let before = *self.state.book();
                if let Some(trade) = trade {
                    self.record_trade(message.exch_ts, trade);
                    if continuous {
                        self.passive(&before, trade, &mut sink, diagnostics);
                    }
                }
                self.state.apply(message);
                self.refresh_consumed();
                self.refresh_queues();
                // A continuous message that the clock places inside a collection period
                // (matching.md section 4.2) updates the state without matching.
                if continuous {
                    self.cross(&mut sink);
                }
            }
            MessageBody::IntermediateTrade(trade) => {
                let before = *self.state.book();
                self.record_trade(message.exch_ts, trade);
                if continuous {
                    self.passive(&before, trade, &mut sink, diagnostics);
                }
                self.state.apply(message);
            }
            // Section 7.1: trial matches and delays do not touch simulated orders.
            MessageBody::Indicative { .. } | MessageBody::Delay(_) => self.state.apply(message),
        }
    }

    fn record_trade(&mut self, time: Timestamp, trade: Trade) {
        if let Some(band) = &mut self.band {
            band.on_trade(time, trade.price, trade.qty);
        }
    }

    /// Section 7.2: an auction result. Every resting order took part in the auction.
    fn auction(&mut self, message: &Message, marginal: AuctionMarginal, sink: &mut Sink) {
        let MessageBody::Disclosure { trade, .. } = message.body else {
            unreachable!("an auction result is a disclosure");
        };
        let ended_delay = self.state.delay().is_some();
        self.state.apply(message);
        self.refresh_consumed();
        let book = *self.state.book();

        for side in [Side::Buy, Side::Sell] {
            if self.orders[index(side)].is_empty() {
                continue;
            }
            if let Some(trade) = trade {
                self.auction_at_price(side, &book, trade, marginal, sink);
            }
            self.auction_residual(side, &book, sink);
            self.orders[index(side)].retain(|order| order.remaining > Shares::ZERO);
            // Section 7.3: whatever real quantity is left at the price is ahead of an
            // order that joined during this collection.
            for order in &mut self.orders[index(side)] {
                let shown = queue_at(side, &book, order.price);
                order.queue = match (order.queue, shown) {
                    (_, shown) if order.joined_collecting => shown,
                    (QueueAhead::Known(queue), QueueAhead::Known(shown)) => {
                        QueueAhead::Known(queue.min(shown))
                    }
                    (QueueAhead::Unknown, shown) => shown,
                    (queue, QueueAhead::Unknown) => queue,
                };
                order.joined_collecting = false;
            }
        }

        // An auction price anchors the band; it is not part of the rolling average.
        let price = trade.map(|trade| trade.price);
        if let Some(band) = &mut self.band {
            match message.phase {
                Phase::OpenAuction => band.on_open(message.exch_ts, price),
                Phase::PeriodicAuction if ended_delay => {
                    band.on_delay_end(message.exch_ts, price);
                }
                Phase::PeriodicAuction | Phase::CloseAuction | Phase::Continuous => {}
            }
        }

        match self.state.mode(message.observed_time) {
            Mode::Closed => self.expire(sink),
            Mode::Continuous => self.cross(sink),
            Mode::Collecting(_) => {}
        }
    }

    /// Steps 1 and 2 of section 7.2 for the orders of one side, when the auction
    /// traded `trade.qty` at `trade.price`.
    fn auction_at_price(
        &mut self,
        side: Side,
        book: &Book,
        trade: Trade,
        marginal: AuctionMarginal,
        sink: &mut Sink,
    ) {
        let opposite = side.opposite();
        // What real opposite orders left at the auction price.
        let mut opposite_left = book_side(book, opposite)
            .levels()
            .iter()
            .find(|level| level.price == trade.price)
            .map_or(Shares::ZERO, |&level| self.available(opposite, level));
        // Real same-side volume that traded with lower priority than a better order.
        let mut displaceable = trade.qty;
        let mut from_opposite = Shares::ZERO;

        for order in &mut self.orders[index(side)] {
            if side.is_better(order.price, trade.price) {
                let qty = order.remaining.min(opposite_left + displaceable);
                if qty == Shares::ZERO {
                    continue;
                }
                let taken = qty.min(opposite_left);
                opposite_left -= taken;
                from_opposite += taken;
                displaceable -= qty - taken;
                order.remaining -= qty;
                sink.fill(order.id, trade.price, qty, FillBasis::AuctionBetter);
            } else if order.price == trade.price {
                let qty = order.remaining.min(opposite_left);
                if qty > Shares::ZERO {
                    opposite_left -= qty;
                    from_opposite += qty;
                    order.remaining -= qty;
                    sink.fill(order.id, trade.price, qty, FillBasis::AuctionOpposite);
                }
                if marginal == AuctionMarginal::Queue
                    && let QueueAhead::Known(queue) = order.queue
                {
                    let reached = Shares::new((displaceable.get() - queue.get()).max(0));
                    let qty = order.remaining.min(reached);
                    order.queue = QueueAhead::Known(queue - queue.min(trade.qty));
                    if qty > Shares::ZERO {
                        displaceable -= qty;
                        order.remaining -= qty;
                        sink.fill(order.id, trade.price, qty, FillBasis::AuctionQueue);
                    }
                }
            } else {
                break;
            }
        }
        if from_opposite > Shares::ZERO {
            self.consume(opposite, trade.price, from_opposite);
        }
    }

    /// Step 3 of section 7.2: each order that still crosses the book after the auction
    /// is matched against it in a one-order call auction.
    fn auction_residual(&mut self, side: Side, book: &Book, sink: &mut Sink) {
        let opposite = side.opposite();
        let levels = book_side(book, opposite).levels();
        for position in 0..self.orders[index(side)].len() {
            let order = self.orders[index(side)][position];
            if order.remaining == Shares::ZERO {
                continue;
            }
            let mut need = order.remaining;
            // The last level taken from and whether it was used up.
            let mut last: Option<(usize, bool)> = None;
            let mut takes: Vec<Level> = Vec::with_capacity(BOOK_DEPTH);
            for (level_index, &level) in levels.iter().enumerate() {
                if need == Shares::ZERO || !crosses(side, order.price, level.price) {
                    break;
                }
                let available = self.available(opposite, level);
                if available == Shares::ZERO {
                    continue;
                }
                let qty = need.min(available);
                need -= qty;
                takes.push(Level {
                    price: level.price,
                    qty,
                });
                last = Some((level_index, qty == available));
            }
            let Some((last_index, used_up)) = last else {
                continue;
            };
            let filled = order.remaining - need;
            let marginal = levels[last_index].price;
            let price = if need > Shares::ZERO {
                order.price
            } else if !used_up {
                marginal
            } else {
                // Every price from the marginal level to the next level (or the order's
                // own price, whichever comes first) clears the same quantity and leaves
                // nothing unfilled on the order's side; the exchange takes the one
                // closest to the last trade.
                let bound = match levels.get(last_index + 1) {
                    Some(next) if side.is_better(order.price, next.price) => next.price,
                    _ => order.price,
                };
                let reference = self
                    .state
                    .last_trade()
                    .map(|trade| trade.price)
                    .or(self.spec.ref_price)
                    .unwrap_or(bound);
                let (low, high) = match side {
                    Side::Buy => (marginal, bound),
                    Side::Sell => (bound, marginal),
                };
                reference.clamp(low, high)
            };
            for take in takes {
                self.consume(opposite, take.price, take.qty);
            }
            self.orders[index(side)][position].remaining = need;
            sink.fill(order.id, price, filled, FillBasis::AuctionResidual);
        }
    }

    /// Every remaining order expires.
    pub(crate) fn expire(&mut self, sink: &mut Sink) {
        let mut orders: Vec<Resting> = self.orders.iter_mut().flat_map(std::mem::take).collect();
        orders.sort_by_key(|order| order.ordinal);
        for order in orders {
            sink.push(
                order.id,
                OrderEventKind::Expired {
                    qty: order.remaining,
                },
            );
        }
    }

    fn take(&mut self, id: OrderId) -> Option<(Side, Resting)> {
        [Side::Buy, Side::Sell].into_iter().find_map(|side| {
            let orders = &mut self.orders[index(side)];
            let position = orders.iter().position(|order| order.id == id)?;
            Some((side, orders.remove(position)))
        })
    }

    /// Cancels what remains of a resting order; `false` when it is not resting.
    pub(crate) fn cancel(&mut self, id: OrderId, sink: &mut Sink) -> bool {
        let Some((_, order)) = self.take(id) else {
            return false;
        };
        sink.push(
            id,
            OrderEventKind::Cancelled {
                qty: order.remaining,
                reason: CancelReason::Requested,
            },
        );
        true
    }

    /// Reduces a resting order by `by` shares, keeping its queue position.
    pub(crate) fn reduce(&mut self, id: OrderId, by: Shares, sink: &mut Sink) {
        let order = self
            .orders
            .iter_mut()
            .flatten()
            .find(|order| order.id == id)
            .expect("the caller checked that the order rests here");
        if by >= order.remaining {
            self.cancel(id, sink);
            return;
        }
        order.remaining -= by;
        let remaining = order.remaining;
        sink.push(id, OrderEventKind::Reduced { by, remaining });
    }

    /// Moves a resting order to `price`: it leaves the book and arrives again with the
    /// quantity that remains.
    pub(crate) fn reprice(&mut self, id: OrderId, price: Price, ordinal: u64, sink: &mut Sink) {
        let (side, order) = self
            .take(id)
            .expect("the caller checked that the order rests here");
        sink.push(id, OrderEventKind::Repriced { price });
        self.place(
            &Arrival {
                id,
                side,
                price: OrderPrice::Limit(price),
                time_in_force: TimeInForce::Rod,
                qty: order.remaining,
                ordinal,
            },
            sink,
        );
    }
}
