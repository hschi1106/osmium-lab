//! Order checks on arrival and changes to orders
//! (`docs/reference/matching.md` sections 4.3 and 5).

mod common;

use common::{Market, Seen, book, filled, price, stock, symbol};
use osmium_book::{BookId, Method};
use osmium_matching::{AuctionMarginal, InstrumentClass, PriceLimits};
use osmium_types::{
    Board, CancelReason, Date, FillBasis, OrderChange, OrderId, OrderPrice, Phase, RejectReason,
    Request, Shares, Side, Symbol, TimeInForce,
};

fn rejected(reason: RejectReason) -> Vec<Seen> {
    vec![Seen::Rejected(Request::New, reason)]
}

const QUIET_BID: common::Levels = &[("99.5", 5_000)];
const QUIET_ASK: common::Levels = &[("100.5", 5_000)];

#[test]
fn an_instrument_that_was_not_added_is_unknown() {
    let mut market = Market::regular();
    let mut order = market.order(Side::Buy, "100", 1_000);
    order.symbol = Symbol::new("9999").unwrap();
    let id = market.send("08:45:00", order);
    assert_eq!(
        market.seen_for(id),
        rejected(RejectReason::UnknownInstrument)
    );

    // The same instrument on a board without data today.
    let mut order = market.order(Side::Buy, "100", 100);
    order.board = Board::OddLot;
    let id = market.send("09:10:00", order);
    assert_eq!(
        market.seen_for(id),
        rejected(RejectReason::UnknownInstrument)
    );
}

#[test]
fn an_untradable_book_refuses_orders_with_its_reason_and_still_follows_the_market() {
    let mut market = Market::regular();
    let other = BookId {
        symbol: Symbol::new("9999").unwrap(),
        board: Board::Regular,
    };
    market.engine.add_untradable_book(
        other,
        Method::Continuous,
        RejectReason::UnsupportedInstrument,
    );
    let mut order = market.order(Side::Buy, "100", 1_000);
    order.symbol = other.symbol;
    let id = market.send("08:45:00", order);
    assert_eq!(
        market.seen_for(id),
        rejected(RejectReason::UnsupportedInstrument)
    );
    assert!(market.engine.book(other).is_some());
    assert!(market.engine.spec(other).is_none());
    assert!(market.engine.book(market.id()).is_some());
}

#[test]
fn the_regular_board_takes_orders_from_0830_to_1330() {
    let mut market = Market::regular();
    let early = market.buy("08:29:59.999999", "100", 1_000);
    assert_eq!(market.seen_for(early), rejected(RejectReason::MarketClosed));
    let first = market.buy("08:30:00", "100", 1_000);
    assert_eq!(market.seen_for(first), [Seen::Accepted]);

    market.auction(
        "09:00:00",
        Phase::OpenAuction,
        book(QUIET_BID, QUIET_ASK),
        None,
    );
    let last = market.buy("13:29:59.999999", "99", 1_000);
    assert_eq!(market.seen_for(last), [Seen::Accepted]);
    // The closing result has not arrived, but the collection is over.
    let late = market.buy("13:30:00", "99", 1_000);
    assert_eq!(market.seen_for(late), rejected(RejectReason::MarketClosed));
}

#[test]
fn a_delayed_close_takes_orders_again_from_1331_to_1333() {
    let mut market = Market::opened(QUIET_BID, QUIET_ASK);
    for (time, accepted) in [
        ("13:30:30", false),
        ("13:31:00", true),
        ("13:32:59", true),
        ("13:33:00", false),
    ] {
        let id = market.buy(time, "99", 1_000);
        let expected = if accepted {
            vec![Seen::Accepted]
        } else {
            rejected(RejectReason::MarketClosed)
        };
        assert_eq!(market.seen_for(id), expected, "{time}");
    }
}

#[test]
fn a_closed_book_takes_no_orders() {
    let mut market = Market::opened(QUIET_BID, QUIET_ASK);
    market.auction(
        "13:30:00",
        Phase::CloseAuction,
        book(QUIET_BID, QUIET_ASK),
        None,
    );
    // Without the closing result this would be inside the delayed-close window.
    let id = market.buy("13:31:30", "99", 1_000);
    assert_eq!(market.seen_for(id), rejected(RejectReason::MarketClosed));
}

#[test]
fn the_odd_lot_board_opens_for_orders_at_0900_until_the_2026_change() {
    let mut market = Market::odd_lot(AuctionMarginal::Conservative);
    let early = market.buy("08:59:59", "100", 10);
    assert_eq!(market.seen_for(early), rejected(RejectReason::MarketClosed));
    let first = market.buy("09:00:00", "100", 10);
    assert_eq!(market.seen_for(first), [Seen::Accepted]);
    let late = market.buy("13:30:00", "100", 10);
    assert_eq!(market.seen_for(late), rejected(RejectReason::MarketClosed));

    let mut spec = stock();
    spec.method = Method::Call;
    for (date, accepted) in [((2026, 12, 4), false), ((2026, 12, 7), true)] {
        let date = Date::from_ymd(date.0, date.1, date.2).unwrap();
        let mut market = Market::new(date, Board::OddLot, spec, AuctionMarginal::Conservative);
        let id = market.buy("08:30:00", "100", 10);
        let expected = if accepted {
            vec![Seen::Accepted]
        } else {
            rejected(RejectReason::MarketClosed)
        };
        assert_eq!(market.seen_for(id), expected, "{date:?}");
    }
}

#[test]
fn a_regular_order_is_a_whole_number_of_lots_up_to_499() {
    let mut market = Market::regular();
    for (qty, valid) in [
        (0, false),
        (-1_000, false),
        (500, false),
        (1_500, false),
        (1_000, true),
        (499_000, true),
        (500_000, false),
    ] {
        let id = market.buy("08:45:00", "100", qty);
        let expected = if valid {
            vec![Seen::Accepted]
        } else {
            rejected(RejectReason::InvalidQuantity)
        };
        assert_eq!(market.seen_for(id), expected, "{qty}");
    }
}

#[test]
fn an_odd_lot_order_is_less_than_one_lot() {
    let mut market = Market::odd_lot(AuctionMarginal::Conservative);
    for (qty, valid) in [(0, false), (1, true), (999, true), (1_000, false)] {
        let id = market.buy("09:05:00", "100", qty);
        let expected = if valid {
            vec![Seen::Accepted]
        } else {
            rejected(RejectReason::InvalidQuantity)
        };
        assert_eq!(market.seen_for(id), expected, "{qty}");
    }
}

#[test]
fn a_limit_price_is_on_the_tick_grid_and_inside_the_limits() {
    let mut market = Market::regular();
    for (limit, expected) in [
        ("100.2", rejected(RejectReason::InvalidTick)),
        ("99.95", rejected(RejectReason::InvalidTick)),
        ("89.9", rejected(RejectReason::PriceOutOfLimit)),
        ("110.5", rejected(RejectReason::PriceOutOfLimit)),
        ("90", vec![Seen::Accepted]),
        ("110", vec![Seen::Accepted]),
        ("99.9", vec![Seen::Accepted]),
    ] {
        let id = market.buy("08:45:00", limit, 1_000);
        assert_eq!(market.seen_for(id), expected, "{limit}");
    }
    // An off-grid price outside the limits fails the tick check first.
    let id = market.buy("08:45:00", "120.3", 1_000);
    assert_eq!(market.seen_for(id), rejected(RejectReason::InvalidTick));
}

#[test]
fn without_limits_any_price_on_the_grid_is_accepted_and_market_orders_are_not() {
    for (limits, unknown) in [(PriceLimits::Unlimited, 0), (PriceLimits::Unknown, 1)] {
        let mut spec = stock();
        spec.limits = limits;
        let mut market = Market::of(spec);
        market.auction(
            "09:00:00",
            Phase::OpenAuction,
            book(QUIET_BID, QUIET_ASK),
            None,
        );

        let far = market.buy("09:01:00", "50", 1_000);
        assert_eq!(market.seen_for(far), [Seen::Accepted], "{limits:?}");
        assert_eq!(market.engine.diagnostics().limit_unknown, unknown);

        let mut order = market.order(Side::Buy, "100", 1_000);
        order.price = OrderPrice::Market;
        let id = market.send("09:01:00", order);
        assert_eq!(
            market.seen_for(id),
            rejected(RejectReason::OrderTypeNotAllowed),
            "{limits:?}"
        );
        assert_eq!(market.engine.diagnostics().limit_unknown, 2 * unknown);
    }
}

#[test]
fn only_limit_day_orders_are_taken_while_a_book_collects() {
    let mut market = Market::regular();
    let kinds = |market: &Market| {
        let limit = OrderPrice::Limit(price("100"));
        [
            (limit, TimeInForce::Rod),
            (limit, TimeInForce::Ioc),
            (limit, TimeInForce::Fok),
            (OrderPrice::Market, TimeInForce::Rod),
            (OrderPrice::Market, TimeInForce::Ioc),
            (OrderPrice::Market, TimeInForce::Fok),
        ]
        .map(|(price, time_in_force)| {
            let mut order = market.order(Side::Buy, "100", 1_000);
            order.price = price;
            order.time_in_force = time_in_force;
            order
        })
    };

    // Before the open.
    for (position, order) in kinds(&market).into_iter().enumerate() {
        let id = market.send("08:45:00", order);
        let expected = if position == 0 {
            vec![Seen::Accepted]
        } else {
            rejected(RejectReason::OrderTypeNotAllowed)
        };
        assert_eq!(market.seen_for(id), expected, "{order:?}");
    }

    // In continuous trading every kind is taken.
    let mut market = Market::opened(QUIET_BID, &[]);
    for order in kinds(&market) {
        let id = market.send("09:01:00", order);
        assert_eq!(market.seen_for(id)[0], Seen::Accepted, "{order:?}");
    }

    // During a matching delay and before the close.
    market.delay("09:02:00");
    let mut ioc = market.order(Side::Buy, "100", 1_000);
    ioc.time_in_force = TimeInForce::Ioc;
    let id = market.send("09:02:30", ioc);
    assert_eq!(
        market.seen_for(id),
        rejected(RejectReason::OrderTypeNotAllowed)
    );
    let mut market = Market::opened(QUIET_BID, QUIET_ASK);
    let id = market.send("13:25:00", ioc);
    assert_eq!(
        market.seen_for(id),
        rejected(RejectReason::OrderTypeNotAllowed)
    );
}

#[test]
fn the_account_is_asked_last_and_only_about_orders_the_exchange_takes() {
    let mut market = Market::regular();
    let mut asked = false;
    let bad = market.order(Side::Buy, "100.2", 1_000);
    let id = market.send_with("08:45:00", bad, || {
        asked = true;
        Ok(())
    });
    assert!(!asked);
    assert_eq!(market.seen_for(id), rejected(RejectReason::InvalidTick));

    let good = market.order(Side::Buy, "100", 1_000);
    let id = market.send_with("08:45:00", good, || Err(RejectReason::InsufficientCash));
    assert_eq!(
        market.seen_for(id),
        rejected(RejectReason::InsufficientCash)
    );
    assert_eq!(market.engine.resting(id), None);
}

#[test]
#[should_panic(expected = "was used before")]
fn an_order_identifier_is_used_once() {
    let mut market = Market::regular();
    let order = market.order(Side::Buy, "100", 1_000);
    let now = market.at("08:45:00");
    let mut events = Vec::new();
    market
        .engine
        .submit(OrderId(7), &order, now, || Ok(()), &mut events);
    market
        .engine
        .submit(OrderId(7), &order, now, || Ok(()), &mut events);
}

#[test]
fn a_change_to_an_order_that_is_not_resting_is_refused() {
    let mut market = Market::opened(QUIET_BID, QUIET_ASK);
    market.cancel("09:01:00", OrderId(99));
    assert_eq!(
        market.seen_for(OrderId(99)),
        [Seen::Rejected(Request::Cancel, RejectReason::UnknownOrder)]
    );

    // An order that was refused.
    let refused = market.buy("09:01:00", "100.2", 1_000);
    market.cancel("09:01:01", refused);
    // An order that was filled completely on arrival.
    let filled_order = market.buy("09:01:00", "100.5", 1_000);
    market.change(
        "09:01:01",
        filled_order,
        OrderChange::Reduce {
            by: Shares::new(1_000),
        },
    );
    // An order that was cancelled.
    let cancelled = market.buy("09:01:00", "99", 1_000);
    market.cancel("09:01:01", cancelled);
    market.change(
        "09:01:02",
        cancelled,
        OrderChange::Reprice {
            price: price("99.5"),
        },
    );
    assert_eq!(
        market.seen_for(refused).last().unwrap(),
        &Seen::Rejected(Request::Cancel, RejectReason::TooLate)
    );
    market.cancel("09:01:03", refused);
    market.change(
        "09:01:03",
        filled_order,
        OrderChange::Reduce {
            by: Shares::new(1_000),
        },
    );
    market.change(
        "09:01:03",
        cancelled,
        OrderChange::Reprice {
            price: price("99.5"),
        },
    );
    assert_eq!(
        market.seen(),
        [
            (
                refused,
                Seen::Rejected(Request::Cancel, RejectReason::TooLate)
            ),
            (
                filled_order,
                Seen::Rejected(Request::Reduce, RejectReason::TooLate)
            ),
            (
                cancelled,
                Seen::Rejected(Request::Reprice, RejectReason::TooLate)
            ),
        ]
    );
}

#[test]
fn cancelling_removes_what_remains_at_any_time() {
    let mut market = Market::opened(QUIET_BID, QUIET_ASK);
    let id = market.buy("09:01:00", "99", 3_000);
    market.seen();
    // Between the scheduled close and the delayed-close window nothing new is taken,
    // but cancelling still works.
    market.cancel("13:30:30", id);
    assert_eq!(
        market.seen_for(id),
        [Seen::Cancelled(3_000, CancelReason::Requested)]
    );
    assert_eq!(market.engine.resting(id), None);
}

#[test]
fn reducing_keeps_the_queue_position() {
    let mut market = Market::opened(QUIET_BID, QUIET_ASK);
    let id = market.buy("09:01:00", "99.5", 3_000);
    assert_eq!(market.queue(id), 5_000);
    market.seen();

    for by in [0, -1_000, 500] {
        market.change(
            "09:01:01",
            id,
            OrderChange::Reduce {
                by: Shares::new(by),
            },
        );
        assert_eq!(
            market.seen_for(id),
            [Seen::Rejected(
                Request::Reduce,
                RejectReason::InvalidQuantity
            )],
            "{by}"
        );
    }
    market.change(
        "09:01:02",
        id,
        OrderChange::Reduce {
            by: Shares::new(1_000),
        },
    );
    assert_eq!(market.seen_for(id), [Seen::Reduced(1_000, 2_000)]);
    assert_eq!(market.remaining(id), Some(2_000));
    assert_eq!(market.queue(id), 5_000);

    // Reducing by everything that remains, or more, cancels.
    market.change(
        "09:01:03",
        id,
        OrderChange::Reduce {
            by: Shares::new(5_000),
        },
    );
    assert_eq!(
        market.seen_for(id),
        [Seen::Cancelled(2_000, CancelReason::Requested)]
    );
}

#[test]
fn reducing_is_refused_outside_order_hours() {
    let mut market = Market::opened(QUIET_BID, QUIET_ASK);
    let id = market.buy("09:01:00", "99", 3_000);
    market.seen();
    market.change(
        "13:30:30",
        id,
        OrderChange::Reduce {
            by: Shares::new(1_000),
        },
    );
    assert_eq!(
        market.seen_for(id),
        [Seen::Rejected(Request::Reduce, RejectReason::MarketClosed)]
    );
    assert_eq!(market.remaining(id), Some(3_000));
}

#[test]
fn repricing_is_a_new_arrival_at_the_new_price() {
    let mut market = Market::opened(QUIET_BID, &[("100.5", 2_000), ("101", 5_000)]);
    let first = market.buy("09:01:00", "99.5", 3_000);
    let second = market.buy("09:01:01", "99", 1_000);
    market.seen();

    // To a resting price: behind what the level shows, and behind our earlier order.
    market.change(
        "09:02:00",
        second,
        OrderChange::Reprice {
            price: price("99.5"),
        },
    );
    assert_eq!(market.seen_for(second), [Seen::Repriced("99.5".to_owned())]);
    assert_eq!(market.queue(second), 5_000);

    // To a crossing price: it takes what the opposite side shows, then rests.
    market.change(
        "09:03:00",
        first,
        OrderChange::Reprice {
            price: price("100.5"),
        },
    );
    assert_eq!(
        market.seen_for(first),
        [
            Seen::Repriced("100.5".to_owned()),
            filled("100.5", 2_000, FillBasis::Aggressive),
        ]
    );
    assert_eq!(market.remaining(first), Some(1_000));
    assert_eq!(market.queue(first), 0);
}

#[test]
fn a_refused_reprice_leaves_the_order_as_it_was() {
    let mut market = Market::opened(QUIET_BID, QUIET_ASK);
    let id = market.buy("09:01:00", "99.5", 3_000);
    market.seen();
    for (to, reason) in [
        ("99.5", RejectReason::ChangeNotAllowed),
        ("99.52", RejectReason::InvalidTick),
        ("111", RejectReason::PriceOutOfLimit),
    ] {
        market.change("09:02:00", id, OrderChange::Reprice { price: price(to) });
        assert_eq!(
            market.seen_for(id),
            [Seen::Rejected(Request::Reprice, reason)],
            "{to}"
        );
    }
    // The account refuses the new price.
    let now = market.at("09:02:00");
    let mut events = Vec::new();
    market.engine.change(
        id,
        OrderChange::Reprice {
            price: price("100"),
        },
        now,
        |resting| {
            assert_eq!(resting.price, price("99.5"));
            assert_eq!(resting.remaining, Shares::new(3_000));
            assert_eq!(resting.side, Side::Buy);
            Err(RejectReason::InsufficientCash)
        },
        &mut events,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(market.remaining(id), Some(3_000));
    assert_eq!(market.queue(id), 5_000);
    assert_eq!(market.engine.resting(id).unwrap().price, price("99.5"));
}

#[test]
fn an_odd_lot_order_cannot_be_repriced() {
    let mut market = Market::odd_lot(AuctionMarginal::Conservative);
    let id = market.buy("09:05:00", "100", 10);
    market.seen();
    market.change(
        "09:06:00",
        id,
        OrderChange::Reprice {
            price: price("101"),
        },
    );
    assert_eq!(
        market.seen_for(id),
        [Seen::Rejected(
            Request::Reprice,
            RejectReason::ChangeNotAllowed
        )]
    );
    // Reducing an odd-lot order by any number of shares is fine.
    market.change("09:06:00", id, OrderChange::Reduce { by: Shares::new(3) });
    assert_eq!(market.seen_for(id), [Seen::Reduced(3, 7)]);
}

#[test]
fn the_end_of_the_day_expires_what_still_rests() {
    let mut market = Market::opened(QUIET_BID, QUIET_ASK);
    let other = BookId {
        symbol: Symbol::new("1101").unwrap(),
        board: Board::Regular,
    };
    market.engine.add_book(other, stock());
    let first = market.sell("09:01:00", "101", 2_000);
    let mut elsewhere = market.order(Side::Buy, "99", 1_000);
    elsewhere.symbol = other.symbol;
    let second = market.send("09:01:01", elsewhere);
    let third = market.buy("09:01:02", "99", 1_000);
    market.seen();

    market.end_day("14:00:00");
    // Books in symbol order, then arrival order.
    assert_eq!(
        market.seen(),
        [
            (second, Seen::Expired(1_000)),
            (first, Seen::Expired(2_000)),
            (third, Seen::Expired(1_000)),
        ]
    );
    assert_eq!(market.engine.resting(first), None);
    let _ = (symbol(), InstrumentClass::Stock);
}
