//! The matching rules against a real marketdata tree (`docs/reference/matching.md`
//! section 11). Ignored by default because they need the data:
//!
//! ```text
//! OSMIUM_MD_ROOT=/srv/md/v1 cargo test --release --locked -p osmium-matching --test marketdata -- --ignored --nocapture
//! ```

use std::collections::{BTreeMap, HashMap};

use osmium_book::{BookId, BookState, Method, Mode, is_auction_result};
use osmium_matching::{AuctionMarginal, BookSpec, Engine, PriceBand, Reference};
use osmium_md::{DEFAULT_MD_ROOT, DayInstruments, DayManifest, FeedDay, MdRoot, MissingLotSize};
use osmium_types::{
    Book, Date, Feed, FillBasis, Message, MessageBody, NewOrder, OrderEvent, OrderEventKind,
    OrderId, OrderPrice, Phase, Price, Shares, Side, Symbol, TimeInForce, Trade, TradeKind,
};

fn root() -> MdRoot {
    MdRoot::new(std::env::var("OSMIUM_MD_ROOT").unwrap_or_else(|_| DEFAULT_MD_ROOT.to_owned()))
}

/// One feed day and the spec of every book the simulator supports, by symbol. The days
/// used here all carry `match_interval_s` and `shares_per_lot`.
fn load(date: &str, feed: Feed) -> (FeedDay, BTreeMap<Symbol, BookSpec>) {
    let root = root();
    let date: Date = date.parse().unwrap();
    let manifest = DayManifest::load(&root, date).unwrap();
    let instruments = DayInstruments::load(&root, &manifest, feed.venue()).unwrap();
    let day = FeedDay::load(
        &root,
        &manifest,
        feed,
        &instruments,
        None,
        MissingLotSize::Assume(1_000),
    )
    .unwrap();
    let specs = day
        .symbols()
        .filter_map(|symbol| {
            let instrument = instruments.get(symbol)?;
            let method = Method::known(feed.board(), instrument.match_interval_s)?;
            let spec = BookSpec::new(instrument, method, instrument.shares_per_lot?)?;
            Some((symbol, spec))
        })
        .collect();
    (day, specs)
}

fn trade_of(message: &Message) -> Option<Trade> {
    match message.body {
        MessageBody::Disclosure { trade, .. } => trade,
        MessageBody::IntermediateTrade(trade) => Some(trade),
        MessageBody::Indicative { .. } | MessageBody::Delay(_) => None,
    }
}

/// The side of the order that caused a trade at `price`, judged against the book before
/// the trade (section 6.4).
fn aggressor(before: &Book, price: Price) -> Option<Side> {
    if before.ask().best().is_some_and(|ask| price >= ask.price) {
        Some(Side::Buy)
    } else if before.bid().best().is_some_and(|bid| price <= bid.price) {
        Some(Side::Sell)
    } else {
        None
    }
}

// ---- 6.6: real trades stay inside the price band -------------------------------------------

struct Banded {
    state: BookState,
    spec: BookSpec,
    band: PriceBand,
    /// The reference and the book before the sweep whose trades are being read.
    sweep: Option<(Reference, Book)>,
}

#[derive(Debug, Default)]
struct BandFacts {
    books: usize,
    trades: u64,
    unknown_aggressor: u64,
    /// Trades outside the exact band that a bound rounded outwards to the tick grid
    /// would let through.
    within_a_tick: u64,
    outside: u64,
}

fn check_band(date: &str, feed: Feed) -> BandFacts {
    let (day, specs) = load(date, feed);
    let mut books: HashMap<Symbol, Banded> = specs
        .iter()
        .filter_map(|(&symbol, &spec)| {
            let band = PriceBand::for_book(feed.board(), &spec)?;
            let state = BookState::new(spec.method, day.date());
            Some((
                symbol,
                Banded {
                    state,
                    spec,
                    band,
                    sweep: None,
                },
            ))
        })
        .collect();
    let mut facts = BandFacts {
        books: books.len(),
        ..BandFacts::default()
    };

    for message in day.messages() {
        let Some(book) = books.get_mut(&message.symbol) else {
            continue;
        };
        let trade = trade_of(&message);
        if is_auction_result(&message) {
            let ended_delay = book.state.delay().is_some();
            let price = trade.map(|trade| trade.price);
            match message.phase {
                Phase::OpenAuction => book.band.on_open(message.exch_ts, price),
                Phase::PeriodicAuction if ended_delay => {
                    book.band.on_delay_end(message.exch_ts, price);
                }
                Phase::PeriodicAuction | Phase::CloseAuction | Phase::Continuous => {}
            }
        } else if let Some(trade) = trade {
            if book.state.mode(message.observed_time) == Mode::Continuous {
                // The exchange checks an order against the band before it trades:
                // every fill of a sweep is judged against the reference before it.
                let (reference, before) = *book.sweep.get_or_insert_with(|| {
                    (book.band.reference(message.exch_ts), *book.state.book())
                });
                facts.trades += 1;
                match aggressor(&before, trade.price) {
                    None => facts.unknown_aggressor += 1,
                    Some(side) if reference.allows(side, trade.price) => {}
                    Some(side) => {
                        let tick = book.spec.class.tick(trade.price);
                        let inwards = Price::from_units(match side {
                            Side::Buy => trade.price.units() - tick,
                            Side::Sell => trade.price.units() + tick,
                        });
                        let (value, qty) = reference.ratio();
                        eprintln!(
                            "outside the band: {date} {feed:?} {} seq {} {side:?} at {} with reference {value}/{qty}",
                            message.symbol, message.seq, trade.price,
                        );
                        if reference.allows(side, inwards) {
                            facts.within_a_tick += 1;
                        } else {
                            facts.outside += 1;
                        }
                    }
                }
            }
            book.band.on_trade(message.exch_ts, trade.price, trade.qty);
        }
        if matches!(message.body, MessageBody::Disclosure { .. }) {
            book.sweep = None;
        }
        book.state.apply(&message);
    }
    facts
}

/// Days with many matching delays: the falls of 2024-08-05 and 2025-04-07 and the days
/// after them.
#[test]
#[ignore = "needs a marketdata tree"]
fn real_continuous_trades_stay_inside_the_price_band() {
    let mut trades = 0;
    let mut within_a_tick = 0;
    for date in [
        "2026-10-08",
        "2025-04-07",
        "2025-04-08",
        "2025-04-09",
        "2025-04-10",
        "2025-04-11",
        "2024-08-05",
        "2024-08-06",
        "2024-08-07",
    ] {
        for feed in [Feed::TwseF6, Feed::TpexF6] {
            let facts = check_band(date, feed);
            eprintln!("{date} {feed:?}: {facts:?}");
            assert!(facts.books > 500, "{date} {feed:?}");
            assert!(facts.trades > 10_000, "{date} {feed:?}");
            assert_eq!(facts.outside, 0, "{date} {feed:?}");
            trades += facts.trades;
            within_a_tick += facts.within_a_tick;
        }
    }
    eprintln!("{trades} trades, {within_a_tick} outside the exact band by less than a tick");
    assert!(within_a_tick <= 3);
}

// ---- 6.2: a real aggressive order sent again gets the same fills ---------------------------

/// Which sweeps of each book are sent again, counted from the book's first; one engine
/// per entry, so no engine holds two probes of one book.
const PROBED: [u32; 5] = [2, 10, 50, 250, 1_000];

struct Sweep {
    engine: usize,
    trades: Vec<Trade>,
}

/// The chosen sweeps of the day by the `seq` of their first message. A sweep is one
/// aggressive order: trades without a book followed by the trade with the book after it.
fn probed_sweeps(day: &FeedDay, specs: &BTreeMap<Symbol, BookSpec>) -> HashMap<u64, Sweep> {
    struct Pending {
        first: u64,
        trades: Vec<Trade>,
        seen: u32,
    }
    let mut pending: HashMap<Symbol, Pending> = HashMap::new();
    let mut chosen = HashMap::new();
    for message in day.messages() {
        if specs
            .get(&message.symbol)
            .is_none_or(|spec| spec.method != Method::Continuous)
        {
            continue;
        }
        let book = pending.entry(message.symbol).or_insert(Pending {
            first: 0,
            trades: Vec::new(),
            seen: 0,
        });
        let trade = trade_of(&message).filter(|_| message.phase == Phase::Continuous);
        let Some(trade) = trade else {
            book.trades.clear();
            continue;
        };
        if book.trades.is_empty() {
            book.first = message.seq;
        }
        book.trades.push(trade);
        if matches!(message.body, MessageBody::Disclosure { .. }) {
            book.seen += 1;
            let trades = std::mem::take(&mut book.trades);
            if let Some(engine) = PROBED.iter().position(|&nth| nth == book.seen) {
                chosen.insert(book.first, Sweep { engine, trades });
            }
        }
    }
    chosen
}

fn engine(day: &FeedDay, specs: &BTreeMap<Symbol, BookSpec>, marginal: AuctionMarginal) -> Engine {
    let mut engine = Engine::new(day.date(), marginal);
    for (&symbol, &spec) in specs {
        engine.add_book(
            BookId {
                symbol,
                board: day.feed().board(),
            },
            spec,
        );
    }
    engine
}

#[derive(Debug, Default)]
struct ResendFacts {
    sweeps: u64,
    not_continuous: u64,
    unknown_aggressor: u64,
    exact: u64,
    rejected: u64,
    band_limited: u64,
    short: u64,
    different: u64,
}

fn check_resend(date: &str, feed: Feed) -> ResendFacts {
    let (day, specs) = load(date, feed);
    let chosen = probed_sweeps(&day, &specs);
    let mut engines: Vec<Engine> = PROBED
        .iter()
        .map(|_| engine(&day, &specs, AuctionMarginal::Conservative))
        .collect();
    let mut facts = ResendFacts::default();
    let mut clock = day.date().at_taipei(0, 0, 0);
    let mut events = Vec::new();
    let mut next_order = 0;

    for message in day.messages() {
        if let Some(sweep) = chosen.get(&message.seq) {
            facts.sweeps += 1;
            let id = BookId::of(&message);
            let engine = &mut engines[sweep.engine];
            let state = engine.book(id).unwrap();
            let side = aggressor(state.book(), sweep.trades[0].price);
            if state.mode(clock) != Mode::Continuous {
                facts.not_continuous += 1;
            } else if let Some(side) = side {
                // The same order, a moment before the real one.
                let order = NewOrder {
                    symbol: message.symbol,
                    board: id.board,
                    side,
                    kind: TradeKind::Cash,
                    price: OrderPrice::Limit(sweep.trades.last().unwrap().price),
                    time_in_force: TimeInForce::Ioc,
                    qty: sweep
                        .trades
                        .iter()
                        .fold(Shares::ZERO, |sum, trade| sum + trade.qty),
                };
                next_order += 1;
                engine.submit(OrderId(next_order), &order, clock, || Ok(()), &mut events);
                let fills: Vec<(Price, Shares)> = events
                    .iter()
                    .filter_map(|event| match event.kind {
                        OrderEventKind::Filled(fill) => {
                            assert_eq!(fill.basis, FillBasis::Aggressive);
                            Some((fill.price, fill.qty))
                        }
                        _ => None,
                    })
                    .collect();
                let real: Vec<(Price, Shares)> = sweep
                    .trades
                    .iter()
                    .map(|trade| (trade.price, trade.qty))
                    .collect();
                let has = |wanted: fn(&OrderEventKind) -> bool| {
                    events.iter().any(|event| wanted(&event.kind))
                };
                let outcome = if fills == real {
                    &mut facts.exact
                } else if has(|kind| matches!(kind, OrderEventKind::Rejected { .. })) {
                    &mut facts.rejected
                } else if has(|kind| matches!(kind, OrderEventKind::PriceBandLimited)) {
                    &mut facts.band_limited
                } else if fills.iter().map(|fill| fill.1.get()).sum::<i64>() < order.qty.get() {
                    &mut facts.short
                } else {
                    &mut facts.different
                };
                *outcome += 1;
                if fills != real && facts.sweeps - facts.exact < 40 {
                    eprintln!(
                        "{date} {feed:?} {} seq {}: real {real:?}, simulated {:?}",
                        message.symbol,
                        message.seq,
                        events.iter().map(|event| event.kind).collect::<Vec<_>>()
                    );
                }
                events.clear();
            } else {
                facts.unknown_aggressor += 1;
            }
        }
        for engine in &mut engines {
            engine.on_market(&message, &mut events);
        }
        assert!(events.is_empty(), "an immediate order left nothing behind");
        clock = message.observed_time;
    }
    facts
}

#[test]
#[ignore = "needs a marketdata tree"]
fn a_real_aggressive_order_sent_again_gets_the_same_fills() {
    for (date, feed) in [
        ("2026-10-08", Feed::TwseF6),
        ("2026-10-08", Feed::TpexF6),
        ("2025-04-07", Feed::TwseF6),
        ("2024-08-05", Feed::TpexF6),
    ] {
        let facts = check_resend(date, feed);
        eprintln!("{date} {feed:?}: {facts:?}");
        assert!(facts.sweeps > 2_000, "{date} {feed:?}");
        let sent = facts.sweeps - facts.not_continuous - facts.unknown_aggressor;
        // Never more than the real order got, and never at another price.
        assert_eq!(facts.different, 0, "{date} {feed:?}");
        assert_eq!(facts.rejected, 0, "{date} {feed:?}");
        assert_eq!(facts.band_limited, 0, "{date} {feed:?}");
        // Less only when the real order went deeper than the five levels shown, or met
        // quantity that no disclosure showed.
        assert!(facts.exact * 1_000 >= sent * 995, "{date} {feed:?}");
    }
}

// ---- 6.3 to 7: resting orders are filled only by what the data shows ------------------------

#[derive(Debug, Clone, Copy)]
struct Placed {
    side: Side,
    price: Price,
    qty: Shares,
    done: Shares,
}

#[derive(Debug, Default)]
struct RestingFacts {
    orders: u64,
    rejected: BTreeMap<String, u64>,
    fills: BTreeMap<String, u64>,
    filled_orders: u64,
    expired_orders: u64,
}

/// Whether `a` is a better price than `b` for an order on `side`.
fn better(side: Side, a: Price, b: Price) -> bool {
    match side {
        Side::Buy => a > b,
        Side::Sell => a < b,
    }
}

/// Checks the events one market message caused against the message itself.
fn audit(
    message: &Message,
    events: &[OrderEvent],
    orders: &mut HashMap<OrderId, Placed>,
    facts: &mut RestingFacts,
) {
    let trade = trade_of(message);
    let auction = is_auction_result(message);
    // Simulated shares one real trade filled, per side.
    let mut passive = [Shares::ZERO; 2];
    for event in events {
        let order = orders
            .get_mut(&event.order)
            .expect("an event of a placed order");
        assert_eq!(event.time, message.observed_time, "{event:?}");
        match event.kind {
            OrderEventKind::Filled(fill) => {
                assert_eq!(fill.trigger, Some((message.feed, message.seq)), "{event:?}");
                assert!(fill.qty > Shares::ZERO, "{event:?}");
                order.done += fill.qty;
                assert!(order.done <= order.qty, "{event:?} {order:?}");
                *facts.fills.entry(format!("{:?}", fill.basis)).or_default() += 1;
                let context = || format!("{event:?} {order:?} {message:?}");
                match fill.basis {
                    FillBasis::Aggressive => panic!("{}", context()),
                    FillBasis::PassiveThrough | FillBasis::PassiveQueue => {
                        let trade = trade.unwrap_or_else(|| panic!("{}", context()));
                        assert!(
                            !auction && message.phase == Phase::Continuous,
                            "{}",
                            context()
                        );
                        assert_eq!(fill.price, order.price, "{}", context());
                        if fill.basis == FillBasis::PassiveThrough {
                            assert!(
                                better(order.side, order.price, trade.price),
                                "{}",
                                context()
                            );
                        } else {
                            assert_eq!(trade.price, order.price, "{}", context());
                        }
                        let side = &mut passive[usize::from(order.side == Side::Sell)];
                        *side += fill.qty;
                        assert!(*side <= trade.qty, "{}", context());
                    }
                    FillBasis::CrossedPassive => {
                        let MessageBody::Disclosure { book, .. } = message.body else {
                            panic!("{}", context());
                        };
                        let opposite = match order.side {
                            Side::Buy => book.ask().best(),
                            Side::Sell => book.bid().best(),
                        };
                        let opposite = opposite.unwrap_or_else(|| panic!("{}", context()));
                        assert!(
                            !better(order.side, opposite.price, order.price),
                            "{}",
                            context()
                        );
                        assert_eq!(fill.price, order.price, "{}", context());
                    }
                    FillBasis::AuctionBetter => {
                        let trade = trade
                            .filter(|_| auction)
                            .unwrap_or_else(|| panic!("{}", context()));
                        assert!(
                            better(order.side, order.price, trade.price),
                            "{}",
                            context()
                        );
                        assert_eq!(fill.price, trade.price, "{}", context());
                    }
                    FillBasis::AuctionOpposite | FillBasis::AuctionQueue => {
                        let trade = trade
                            .filter(|_| auction)
                            .unwrap_or_else(|| panic!("{}", context()));
                        assert_eq!(trade.price, order.price, "{}", context());
                        assert_eq!(fill.price, order.price, "{}", context());
                    }
                    FillBasis::AuctionResidual => {
                        assert!(auction, "{}", context());
                        assert!(
                            !better(order.side, fill.price, order.price),
                            "{}",
                            context()
                        );
                    }
                }
                if order.done == order.qty {
                    facts.filled_orders += 1;
                }
            }
            OrderEventKind::Expired { qty } => {
                assert!(auction && message.phase == Phase::CloseAuction, "{event:?}");
                assert_eq!(order.done + qty, order.qty, "{event:?} {order:?}");
                order.done = order.qty;
                facts.expired_orders += 1;
            }
            // A crossing the band did not allow (section 6.6).
            OrderEventKind::PriceBandLimited => {
                assert!(
                    matches!(message.body, MessageBody::Disclosure { .. }),
                    "{event:?}"
                );
            }
            _ => panic!("{event:?} from {message:?}"),
        }
    }
}

/// Places passive and marketable orders in every book three times during the day and
/// audits every event that follows. Returns all events.
fn check_resting(
    date: &str,
    feed: Feed,
    marginal: AuctionMarginal,
) -> (RestingFacts, Vec<OrderEvent>) {
    let (day, specs) = load(date, feed);
    let board = feed.board();
    let mut engine = engine(&day, &specs, marginal);
    let mut facts = RestingFacts::default();
    let mut orders: HashMap<OrderId, Placed> = HashMap::new();
    let mut all = Vec::new();
    let mut events = Vec::new();
    let mut next_order = 0;
    // (time, marketable): join the best prices, or take them.
    let mut rounds = [
        (day.date().at_taipei(9, 20, 0), false),
        (day.date().at_taipei(11, 0, 0), true),
        (day.date().at_taipei(13, 26, 0), false),
    ]
    .into_iter()
    .peekable();

    for message in day.messages() {
        while let Some((now, marketable)) = rounds.next_if(|&(at, _)| message.observed_time > at) {
            for (&symbol, spec) in &specs {
                let id = BookId { symbol, board };
                let book = *engine.book(id).unwrap().book();
                let (Some(bid), Some(ask)) = (book.bid().best(), book.ask().best()) else {
                    continue;
                };
                let qty = Shares::new(match board {
                    osmium_types::Board::Regular => i64::from(spec.shares_per_lot),
                    osmium_types::Board::OddLot => 1,
                });
                for (side, price) in [
                    (Side::Buy, if marketable { ask.price } else { bid.price }),
                    (Side::Sell, if marketable { bid.price } else { ask.price }),
                ] {
                    let order = NewOrder {
                        symbol,
                        board,
                        side,
                        kind: TradeKind::Cash,
                        price: OrderPrice::Limit(price),
                        time_in_force: TimeInForce::Rod,
                        qty,
                    };
                    next_order += 1;
                    let id = OrderId(next_order);
                    engine.submit(id, &order, now, || Ok(()), &mut events);
                    let mut placed = Placed {
                        side,
                        price,
                        qty,
                        done: Shares::ZERO,
                    };
                    for event in &events {
                        assert_eq!((event.order, event.time), (id, now));
                        match event.kind {
                            OrderEventKind::Accepted => facts.orders += 1,
                            OrderEventKind::Rejected { reason, .. } => {
                                *facts.rejected.entry(format!("{reason:?}")).or_default() += 1;
                            }
                            OrderEventKind::Filled(fill) => {
                                assert_eq!(fill.basis, FillBasis::Aggressive, "{event:?}");
                                assert_eq!(fill.trigger, None, "{event:?}");
                                assert!(!better(side, fill.price, price), "{event:?}");
                                placed.done += fill.qty;
                                *facts.fills.entry("Aggressive".to_owned()).or_default() += 1;
                                if placed.done == qty {
                                    facts.filled_orders += 1;
                                }
                            }
                            OrderEventKind::PriceBandLimited => {}
                            _ => panic!("{event:?}"),
                        }
                    }
                    orders.insert(id, placed);
                    all.append(&mut events);
                }
            }
        }
        engine.on_market(&message, &mut events);
        audit(&message, &events, &mut orders, &mut facts);
        all.append(&mut events);
    }
    engine.end_day(day.date().at_taipei(14, 30, 0), &mut events);
    assert!(events.is_empty(), "every book closed: {events:?}");
    (facts, all)
}

#[test]
#[ignore = "needs a marketdata tree"]
fn resting_orders_are_filled_only_by_what_the_data_shows() {
    for (date, feed, marginal) in [
        ("2026-10-08", Feed::TwseF6, AuctionMarginal::Conservative),
        ("2026-10-08", Feed::TpexF6, AuctionMarginal::Queue),
        ("2026-10-08", Feed::TwseF23, AuctionMarginal::Conservative),
        ("2026-10-08", Feed::TpexF23, AuctionMarginal::Queue),
        ("2025-04-07", Feed::TwseF6, AuctionMarginal::Queue),
    ] {
        let (facts, _) = check_resting(date, feed, marginal);
        eprintln!("{date} {feed:?} {marginal:?}: {facts:?}");
        assert!(facts.orders > 1_000, "{date} {feed:?}");
        // Every accepted order ended filled or expired, once.
        assert_eq!(
            facts.filled_orders + facts.expired_orders,
            facts.orders,
            "{date} {feed:?}"
        );
    }
}

// ---- determinism ---------------------------------------------------------------------------

#[test]
#[ignore = "needs a marketdata tree"]
fn the_same_inputs_give_the_same_events() {
    let (_, first) = check_resting("2026-10-08", Feed::TpexF6, AuctionMarginal::Queue);
    let (_, second) = check_resting("2026-10-08", Feed::TpexF6, AuctionMarginal::Queue);
    assert!(first.len() > 5_000);
    assert_eq!(first, second);
}
