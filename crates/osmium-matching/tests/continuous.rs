//! Continuous matching (`docs/reference/matching.md` section 6).

mod common;

use common::{Levels, Market, Seen, book, book_with_market, filled, stock};
use osmium_matching::{InstrumentClass, PriceLimits};
use osmium_types::{CancelReason, FillBasis, OrderEventKind, OrderPrice, Phase, Side, TimeInForce};

const BID: Levels = &[("99.5", 5_000), ("99", 4_000)];
const ASK: Levels = &[("100", 2_000), ("100.5", 3_000), ("101", 4_000)];

fn opened() -> Market {
    Market::opened(BID, ASK)
}

// ---- 6.2 arrival ---------------------------------------------------------------

#[test]
fn an_arriving_order_takes_each_level_at_the_level_price() {
    let mut market = opened();
    let id = market.buy("09:01:00", "100.5", 4_000);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            filled("100", 2_000, FillBasis::Aggressive),
            filled("100.5", 2_000, FillBasis::Aggressive),
        ]
    );
    assert_eq!(market.engine.resting(id), None);

    let id = market.sell("09:01:01", "99", 6_000);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            filled("99.5", 5_000, FillBasis::Aggressive),
            filled("99", 1_000, FillBasis::Aggressive),
        ]
    );
}

#[test]
fn an_aggressive_fill_carries_the_arrival_time_and_no_trigger() {
    let mut market = opened();
    let id = market.buy("09:01:00.25", "100", 1_000);
    let events = market.take();
    assert_eq!(events.len(), 2);
    for event in &events {
        assert_eq!(event.order, id);
        assert_eq!(event.time, market.at("09:01:00.25"));
    }
    let OrderEventKind::Filled(fill) = events[1].kind else {
        panic!("{:?}", events[1]);
    };
    assert_eq!(fill.trigger, None);
}

#[test]
fn the_rest_of_a_limit_day_order_rests_at_its_price() {
    let mut market = opened();
    let id = market.buy("09:01:00", "100", 5_000);
    assert_eq!(
        market.seen_for(id),
        [Seen::Accepted, filled("100", 2_000, FillBasis::Aggressive)]
    );
    assert_eq!(market.remaining(id), Some(3_000));
    // Better than the best bid: nothing real is ahead.
    assert_eq!(market.queue(id), 0);
}

#[test]
fn the_rest_of_an_immediate_order_is_cancelled() {
    let mut market = opened();
    let mut order = market.order(Side::Buy, "100", 5_000);
    order.time_in_force = TimeInForce::Ioc;
    let id = market.send("09:01:00", order);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            filled("100", 2_000, FillBasis::Aggressive),
            Seen::Cancelled(3_000, CancelReason::IocRemainder),
        ]
    );
    assert_eq!(market.engine.resting(id), None);
}

#[test]
fn a_fill_or_kill_order_trades_all_or_nothing() {
    let mut market = opened();
    let mut order = market.order(Side::Buy, "100.5", 6_000);
    order.time_in_force = TimeInForce::Fok;
    let killed = market.send("09:01:00", order);
    assert_eq!(
        market.seen_for(killed),
        [
            Seen::Accepted,
            Seen::Cancelled(6_000, CancelReason::FokUnfilled)
        ]
    );
    // Nothing was used: the same quantity is still there.
    order.qty = osmium_types::Shares::new(5_000);
    let whole = market.send("09:01:01", order);
    assert_eq!(
        market.seen_for(whole),
        [
            Seen::Accepted,
            filled("100", 2_000, FillBasis::Aggressive),
            filled("100.5", 3_000, FillBasis::Aggressive),
        ]
    );
}

#[test]
fn a_market_order_takes_the_visible_levels_and_the_rest_is_cancelled() {
    let mut market = opened();
    for time_in_force in [TimeInForce::Rod, TimeInForce::Ioc] {
        let mut market_order = market.order(Side::Sell, "100", 10_000);
        market_order.price = OrderPrice::Market;
        market_order.time_in_force = time_in_force;
        let id = market.send("09:01:00", market_order);
        let seen = market.seen_for(id);
        assert_eq!(seen[0], Seen::Accepted);
        assert_eq!(
            seen.last().unwrap(),
            &Seen::Cancelled(
                if time_in_force == TimeInForce::Rod {
                    1_000
                } else {
                    10_000
                },
                CancelReason::MarketRemainder
            ),
            "{time_in_force:?}"
        );
        assert_eq!(market.engine.resting(id), None);
    }
    let mut fok = market.order(Side::Buy, "100", 10_000);
    fok.price = OrderPrice::Market;
    fok.time_in_force = TimeInForce::Fok;
    let id = market.send("09:01:00", fok);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            Seen::Cancelled(10_000, CancelReason::FokUnfilled)
        ]
    );
}

// ---- 6.1 consumed ----------------------------------------------------------------

#[test]
fn quantity_taken_by_one_order_is_not_there_for_the_next() {
    let mut market = opened();
    market.buy("09:01:00", "100", 1_000);
    let mut order = market.order(Side::Buy, "100", 2_000);
    order.time_in_force = TimeInForce::Ioc;
    let id = market.send("09:01:01", order);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            filled("100", 1_000, FillBasis::Aggressive),
            Seen::Cancelled(1_000, CancelReason::IocRemainder),
        ]
    );
}

#[test]
fn a_disclosure_does_not_bring_taken_quantity_back() {
    let mut market = opened();
    market.buy("09:01:00", "100", 2_000);
    // The real book still shows the level, because the simulated order never existed.
    market.disclose("09:01:01", BID, ASK);
    let mut order = market.order(Side::Buy, "100", 1_000);
    order.time_in_force = TimeInForce::Ioc;
    let id = market.send("09:01:02", order);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            Seen::Cancelled(1_000, CancelReason::IocRemainder)
        ]
    );

    // New real quantity at the price is available.
    market.disclose(
        "09:01:03",
        BID,
        &[("100", 2_500), ("100.5", 3_000), ("101", 4_000)],
    );
    let id = market.send("09:01:04", order);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            filled("100", 500, FillBasis::Aggressive),
            Seen::Cancelled(500, CancelReason::IocRemainder)
        ]
    );
}

#[test]
fn taken_quantity_is_capped_by_what_the_level_shows_and_cleared_when_it_goes() {
    let mut market = opened();
    market.buy("09:01:00", "100", 2_000);
    let mut order = market.order(Side::Buy, "100", 3_000);
    order.time_in_force = TimeInForce::Ioc;

    // Real trades took the level down to 500 and new orders then raised it to 1,500:
    // the 500 that stayed are the ones the simulated order used.
    market.disclose("09:01:01", BID, &[("100", 500), ("100.5", 3_000)]);
    market.disclose("09:01:02", BID, &[("100", 1_500), ("100.5", 3_000)]);
    let id = market.send("09:01:03", order);
    assert_eq!(
        market.seen_for(id)[1..],
        [
            filled("100", 1_000, FillBasis::Aggressive),
            Seen::Cancelled(2_000, CancelReason::IocRemainder)
        ]
    );

    // The level disappears and comes back: everything shown is new.
    market.disclose("09:01:04", BID, &[("100.5", 3_000)]);
    market.disclose("09:01:05", BID, &[("100", 700), ("100.5", 3_000)]);
    let id = market.send("09:01:06", order);
    assert_eq!(
        market.seen_for(id)[1..],
        [
            filled("100", 700, FillBasis::Aggressive),
            Seen::Cancelled(2_300, CancelReason::IocRemainder)
        ]
    );
}

#[test]
fn taken_quantity_is_kept_while_the_level_is_pushed_out_of_sight() {
    let deep: Levels = &[
        ("100", 1_000),
        ("100.5", 1_000),
        ("101", 1_000),
        ("101.5", 1_000),
        ("102", 1_000),
    ];
    let mut market = Market::opened(BID, deep);
    // Takes all five levels.
    let id = market.buy("09:01:00", "102", 5_000);
    assert_eq!(market.seen_for(id).len(), 6);
    // Four better levels appear. 101.5 is still shown as the fifth level, 102 is beyond
    // it, and 100 to 101 are inside the visible range but absent.
    market.disclose(
        "09:01:01",
        &[("97", 1_000)],
        &[
            ("98", 9_000),
            ("98.5", 9_000),
            ("99", 9_000),
            ("99.5", 9_000),
            ("101.5", 9_000),
        ],
    );
    market.disclose("09:01:02", BID, deep);
    let mut order = market.order(Side::Buy, "102", 5_000);
    order.time_in_force = TimeInForce::Ioc;
    let id = market.send("09:01:03", order);
    // 100 to 101 went away, so what they show now is new. 101.5 never showed less than
    // what was taken and 102 was out of sight: both keep what was taken.
    assert_eq!(
        market.seen_for(id)[1..],
        [
            filled("100", 1_000, FillBasis::Aggressive),
            filled("100.5", 1_000, FillBasis::Aggressive),
            filled("101", 1_000, FillBasis::Aggressive),
            Seen::Cancelled(2_000, CancelReason::IocRemainder),
        ]
    );
}

// ---- 6.3 queue position ------------------------------------------------------------

#[test]
fn an_order_joins_behind_what_its_level_shows() {
    let mut market = opened();
    let at_best = market.buy("09:01:00", "99.5", 1_000);
    let at_second = market.buy("09:01:00", "99", 1_000);
    let between = market.sell("09:01:00", "100.5", 1_000);
    let new_level = market.buy("09:01:00", "98", 1_000);
    assert_eq!(market.queue(at_best), 5_000);
    assert_eq!(market.queue(at_second), 4_000);
    assert_eq!(market.queue(between), 3_000);
    // Fewer than five levels are shown, so the side has nothing at 98.
    assert_eq!(market.queue(new_level), 0);
}

#[test]
fn market_orders_on_the_same_side_are_ahead_of_the_best_level() {
    let mut market = Market::regular();
    market.auction(
        "09:00:00",
        Phase::OpenAuction,
        book_with_market(BID, 700, ASK, 0),
        None,
    );
    let at_best = market.buy("09:01:00", "99.5", 1_000);
    let at_second = market.buy("09:01:00", "99", 1_000);
    assert_eq!(market.queue(at_best), 5_700);
    assert_eq!(market.queue(at_second), 4_000);
}

#[test]
fn our_own_orders_at_one_price_keep_their_arrival_order() {
    let mut market = opened();
    let first = market.buy("09:01:00", "99.5", 1_000);
    let second = market.buy("09:01:01", "99.5", 1_000);
    market.seen();
    // A sell of 5,500 at 99.5: 5,000 real shares are ahead of both.
    market.trade("09:02:00", "99.5", 5_500, &[("99", 4_000)], ASK);
    assert_eq!(
        market.seen(),
        [(first, filled("99.5", 500, FillBasis::PassiveQueue))]
    );
    assert_eq!(market.remaining(second), Some(1_000));
}

#[test]
fn the_queue_of_a_hidden_price_is_set_when_the_price_is_first_shown() {
    let five: Levels = &[
        ("99.5", 1_000),
        ("99", 1_000),
        ("98.5", 1_000),
        ("98", 1_000),
        ("97.5", 1_000),
    ];
    let mut market = Market::opened(five, ASK);
    let id = market.buy("09:01:00", "97", 1_000);
    assert!(market.queue_is_unknown(id));
    market.disclose("09:01:01", five, ASK);
    assert!(market.queue_is_unknown(id));
    // 99.5 goes; 97 becomes the fifth level and shows 2,500.
    market.disclose(
        "09:01:02",
        &[
            ("99", 1_000),
            ("98.5", 1_000),
            ("98", 1_000),
            ("97.5", 1_000),
            ("97", 2_500),
        ],
        ASK,
    );
    assert_eq!(market.queue(id), 2_500);
    // Pushed out of sight again, the estimate is kept.
    market.disclose("09:01:03", five, ASK);
    assert_eq!(market.queue(id), 2_500);
}

#[test]
fn cancellations_are_assumed_to_be_behind_the_order() {
    let mut market = opened();
    let id = market.buy("09:01:00", "99.5", 1_000);
    assert_eq!(market.queue(id), 5_000);
    // More real orders: they are behind.
    market.disclose("09:01:01", &[("99.5", 8_000), ("99", 4_000)], ASK);
    assert_eq!(market.queue(id), 5_000);
    // Fewer, without a trade: the cancelled ones were behind.
    market.disclose("09:01:02", &[("99.5", 6_000), ("99", 4_000)], ASK);
    assert_eq!(market.queue(id), 5_000);
    // Fewer than the queue: no more than what is shown can be ahead.
    market.disclose("09:01:03", &[("99.5", 3_000), ("99", 4_000)], ASK);
    assert_eq!(market.queue(id), 3_000);
    market.disclose("09:01:04", &[("99.5", 9_000), ("99", 4_000)], ASK);
    assert_eq!(market.queue(id), 3_000);
    // The level disappears from inside the visible book: nothing is ahead.
    market.disclose("09:01:05", &[("99", 4_000)], ASK);
    assert_eq!(market.queue(id), 0);
    market.disclose("09:01:06", &[("99.5", 9_000), ("99", 4_000)], ASK);
    assert_eq!(market.queue(id), 0);
}

// ---- 6.4 passive fills ---------------------------------------------------------------

#[test]
fn a_trade_at_the_order_price_fills_what_passes_the_queue() {
    let mut market = opened();
    let id = market.buy("09:01:00", "99.5", 2_000);
    market.seen();

    market.trade(
        "09:02:00",
        "99.5",
        3_000,
        &[("99.5", 2_000), ("99", 4_000)],
        ASK,
    );
    assert_eq!(market.seen(), []);
    assert_eq!(market.queue(id), 2_000);

    // 2,000 are still ahead; this trade of 2,500 reaches the order with 500.
    market.trade("09:02:01", "99.5", 2_500, &[("99", 4_000)], ASK);
    let events = market.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].time, market.at("09:02:01"));
    let OrderEventKind::Filled(fill) = events[0].kind else {
        panic!("{:?}", events[0]);
    };
    assert_eq!(fill.basis, FillBasis::PassiveQueue);
    assert_eq!(fill.qty.get(), 500);
    assert_eq!(
        fill.trigger,
        Some((osmium_types::Feed::TwseF6, market.seq()))
    );
    assert_eq!(market.remaining(id), Some(1_500));
    assert_eq!(market.queue(id), 0);

    // Nothing is ahead any more: the next trade fills up to its own size.
    market.disclose("09:02:02", &[("99.5", 1_000), ("99", 4_000)], ASK);
    market.trade("09:02:03", "99.5", 1_000, &[("99", 4_000)], ASK);
    assert_eq!(
        market.seen(),
        [(id, filled("99.5", 1_000, FillBasis::PassiveQueue))]
    );
    assert_eq!(market.remaining(id), Some(500));
}

#[test]
fn a_trade_through_the_order_price_fills_at_the_order_price() {
    let mut market = opened();
    let id = market.buy("09:01:00", "99.5", 2_000);
    market.seen();
    // A sell sweeps 99.5 and trades at 99: every real order at 99.5 is gone and the
    // seller would have met ours on the way.
    market.intermediate("09:02:00", "99.5", 5_000);
    market.trade("09:02:00", "99", 1_500, &[("99", 2_500)], ASK);
    assert_eq!(
        market.seen(),
        [(id, filled("99.5", 1_500, FillBasis::PassiveThrough))]
    );
    assert_eq!(market.remaining(id), Some(500));
    assert_eq!(market.queue(id), 0);
}

#[test]
fn every_fill_of_a_sweep_is_judged_against_the_book_before_the_sweep() {
    let mut market = opened();
    let id = market.sell("09:01:00", "100.5", 1_000);
    assert_eq!(market.queue(id), 3_000);
    market.seen();
    // A buy sweeps 100 and takes 3,400 at 100.5. Against the book after the first fill
    // 100.5 would look like a price inside the spread.
    market.intermediate("09:02:00", "100", 2_000);
    market.trade("09:02:00", "100.5", 3_400, BID, &[("101", 4_000)]);
    assert_eq!(
        market.seen(),
        [(id, filled("100.5", 400, FillBasis::PassiveQueue))]
    );
    assert_eq!(market.engine.diagnostics().aggressor_unknown, 0);
}

#[test]
fn a_trade_whose_aggressor_is_unknown_fills_nothing() {
    let mut market = opened();
    let id = market.buy("09:01:00", "99.5", 1_000);
    market.seen();
    // 99.75 is inside the spread of the book before the trade.
    market.trade("09:02:00", "99.8", 9_000, BID, ASK);
    assert_eq!(market.seen(), []);
    assert_eq!(market.queue(id), 5_000);
    assert_eq!(market.engine.diagnostics().aggressor_unknown, 1);
}

#[test]
fn a_buyer_initiated_trade_does_not_fill_a_resting_buy_order() {
    let mut market = opened();
    let id = market.buy("09:01:00", "99.5", 1_000);
    market.seen();
    market.trade("09:02:00", "100", 2_000, BID, &[("100.5", 3_000)]);
    assert_eq!(market.seen(), []);
    assert_eq!(market.queue(id), 5_000);
}

#[test]
fn one_real_trade_is_shared_by_the_orders_it_reaches() {
    let mut market = opened();
    let first = market.buy("09:01:00", "99.8", 1_000);
    let second = market.buy("09:01:01", "99.8", 1_000);
    market.seen();
    // A sell of 1,500 trades at the best bid; it would have met both orders first.
    market.trade(
        "09:02:00",
        "99.5",
        1_500,
        &[("99.5", 3_500), ("99", 4_000)],
        ASK,
    );
    assert_eq!(
        market.seen(),
        [
            (first, filled("99.8", 1_000, FillBasis::PassiveThrough)),
            (second, filled("99.8", 500, FillBasis::PassiveThrough)),
        ]
    );
    assert_eq!(market.remaining(second), Some(500));
}

#[test]
fn a_sweep_reaches_our_orders_by_price_then_arrival() {
    let mut market = opened();
    let low = market.buy("09:01:00", "99", 1_000);
    let high = market.buy("09:01:01", "99.5", 1_000);
    let inside = market.buy("09:01:02", "99.8", 6_000);
    market.seen();
    // A sell takes the 5,000 at 99.5 and 2,500 at 99.
    market.intermediate("09:02:00", "99.5", 5_000);
    market.trade("09:02:00", "99", 2_500, &[("99", 1_500)], ASK);
    assert_eq!(
        market.seen(),
        [
            // The fill at 99.5 passes 99.8; the real orders at 99.5 are ahead of ours.
            (inside, filled("99.8", 5_000, FillBasis::PassiveThrough)),
            // The fill at 99 passes both prices.
            (inside, filled("99.8", 1_000, FillBasis::PassiveThrough)),
            (high, filled("99.5", 1_000, FillBasis::PassiveThrough)),
        ]
    );
    // 500 of the fill at 99 are left for the order at 99, behind 4,000 real shares.
    assert_eq!(market.remaining(low), Some(1_000));
    assert_eq!(market.queue(low), 1_500);
}

// ---- 6.5 crossing ----------------------------------------------------------------------

#[test]
fn a_resting_order_trades_at_its_own_price_when_the_other_side_comes_to_it() {
    let mut market = opened();
    let id = market.buy("09:01:00", "99.8", 3_000);
    market.seen();
    // A real sell order rests at 99.6, inside our price.
    market.disclose("09:02:00", BID, &[("99.6", 1_000), ("100", 2_000)]);
    assert_eq!(
        market.seen(),
        [(id, filled("99.8", 1_000, FillBasis::CrossedPassive))]
    );
    // The same disclosure again does not fill twice.
    market.disclose("09:02:01", BID, &[("99.6", 1_000), ("100", 2_000)]);
    assert_eq!(market.seen(), []);
    market.disclose("09:02:02", BID, &[("99.6", 1_800), ("100", 2_000)]);
    assert_eq!(
        market.seen(),
        [(id, filled("99.8", 800, FillBasis::CrossedPassive))]
    );
    assert_eq!(market.remaining(id), Some(1_200));
}

#[test]
fn crossing_serves_our_orders_in_priority_order() {
    let mut market = opened();
    let low = market.sell("09:01:00", "99.6", 1_000);
    let high = market.sell("09:01:01", "99.7", 1_000);
    market.seen();
    // Both rest inside the spread; a real bid of 99.7 arrives.
    market.disclose("09:02:00", &[("99.7", 1_500), ("99.5", 5_000)], ASK);
    assert_eq!(
        market.seen(),
        [
            (low, filled("99.6", 1_000, FillBasis::CrossedPassive)),
            (high, filled("99.7", 500, FillBasis::CrossedPassive)),
        ]
    );
}

// ---- 6.6 price band ---------------------------------------------------------------------

/// Asks from 100 up to 105; the reference is the day's reference price of 100, so a buy
/// may trade up to 103.5.
const WIDE_ASK: Levels = &[
    ("100", 1_000),
    ("103.5", 1_000),
    ("104", 1_000),
    ("105", 1_000),
];

#[test]
fn an_arriving_order_does_not_trade_outside_the_band() {
    let mut market = Market::opened(BID, WIDE_ASK);
    let id = market.buy("09:01:00", "105", 4_000);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            filled("100", 1_000, FillBasis::Aggressive),
            filled("103.5", 1_000, FillBasis::Aggressive),
            Seen::PriceBandLimited,
        ]
    );
    // The rest waits at its price.
    assert_eq!(market.remaining(id), Some(2_000));

    let mut market = Market::opened(BID, WIDE_ASK);
    let mut ioc = market.order(Side::Buy, "105", 4_000);
    ioc.time_in_force = TimeInForce::Ioc;
    let id = market.send("09:01:00", ioc);
    assert_eq!(
        market.seen_for(id)[3..],
        [
            Seen::PriceBandLimited,
            Seen::Cancelled(2_000, CancelReason::IocRemainder)
        ]
    );

    // Fill-or-kill counts only what is inside the band.
    let mut market = Market::opened(BID, WIDE_ASK);
    let mut fok = market.order(Side::Buy, "105", 3_000);
    fok.time_in_force = TimeInForce::Fok;
    let id = market.send("09:01:00", fok);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            Seen::PriceBandLimited,
            Seen::Cancelled(3_000, CancelReason::FokUnfilled)
        ]
    );
}

#[test]
fn the_band_follows_the_real_trades() {
    let mut market = Market::opened(BID, WIDE_ASK);
    // Real trades move the reference to 104 after the opening five minutes.
    market.trade(
        "09:06:00",
        "104",
        1_000,
        BID,
        &[("104", 1_000), ("105", 1_000)],
    );
    let id = market.buy("09:06:01", "105", 2_000);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            filled("104", 1_000, FillBasis::Aggressive),
            filled("105", 1_000, FillBasis::Aggressive),
        ]
    );
}

#[test]
fn a_resting_order_crosses_only_inside_the_band_and_reports_it_once() {
    let mut market = Market::opened(&[("96", 1_000)], ASK);
    let id = market.sell("09:01:00", "95", 2_000);
    // 96 is more than 3.5 % below the reference of 100.
    assert_eq!(
        market.seen_for(id),
        [Seen::Accepted, Seen::PriceBandLimited]
    );
    market.disclose("09:01:01", &[("96", 1_500)], ASK);
    assert_eq!(market.seen(), []);
    market.disclose("09:01:02", &[("96.5", 500), ("96", 1_500)], ASK);
    assert_eq!(
        market.seen(),
        [(id, filled("95", 500, FillBasis::CrossedPassive))]
    );
    assert_eq!(market.remaining(id), Some(1_500));
}

#[test]
fn the_band_does_not_apply_without_limits_or_a_reference_price_or_to_warrants() {
    let mut warrant = stock();
    warrant.class = InstrumentClass::Warrant;
    let mut unlimited = stock();
    unlimited.limits = PriceLimits::Unlimited;
    let mut cheap = stock();
    cheap.ref_price = Some(common::price("0.99"));
    let mut unpriced = stock();
    unpriced.ref_price = None;
    for spec in [warrant, unlimited, cheap, unpriced] {
        let mut market = Market::of(spec);
        market.auction("09:00:00", Phase::OpenAuction, book(BID, WIDE_ASK), None);
        let id = market.buy("09:01:00", "105", 4_000);
        assert_eq!(market.seen_for(id).len(), 5, "{spec:?}");
    }
}

// ---- messages the clock places inside a collection period ------------------------------

#[test]
fn a_continuous_message_after_1325_updates_the_book_without_matching() {
    let mut market = opened();
    let resting = market.buy("09:01:00", "99.5", 1_000);
    let arrived = market.buy("13:25:00", "99.8", 1_000);
    market.seen();
    // Another book's first message of 13:25 moved the clock; this trade is earlier at
    // the exchange but later in the feed.
    market.trade(
        "13:25:00.001",
        "99.5",
        9_000,
        &[("99.5", 500)],
        &[("99.6", 1_000)],
    );
    assert_eq!(market.seen(), []);
    assert_eq!(market.queue(resting), 500);
    assert_eq!(market.remaining(arrived), Some(1_000));
}
