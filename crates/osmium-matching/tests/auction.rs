//! Call auctions (`docs/reference/matching.md` section 7).

mod common;

use common::{Levels, Market, Seen, book, filled, stock};
use osmium_matching::AuctionMarginal;
use osmium_types::{
    Board, CancelReason, Date, Feed, FillBasis, OrderEventKind, OrderId, Phase, Side, TimeInForce,
};

const BID: Levels = &[("99.5", 5_000), ("99", 4_000)];
const ASK: Levels = &[("100", 2_000), ("100.5", 3_000), ("101", 4_000)];

/// A regular-board book, before its opening auction, with the given setting for orders
/// at the auction price.
fn regular(marginal: AuctionMarginal) -> Market {
    Market::new(
        Date::from_ymd(2026, 10, 8).unwrap(),
        Board::Regular,
        stock(),
        marginal,
    )
}

/// The remaining quantity of an immediate order that takes whatever `side` can get at
/// `limit` right now.
fn probe(market: &mut Market, time: &str, side: Side, limit: &str, qty: i64) -> Vec<Seen> {
    let mut order = market.order(side, limit, qty);
    order.time_in_force = TimeInForce::Ioc;
    let id = market.send(time, order);
    market.seen_for(id)[1..].to_vec()
}

// ---- 7.1 collection ------------------------------------------------------------------

#[test]
fn an_order_sent_before_the_open_waits_for_the_opening_auction() {
    let mut market = Market::regular();
    let id = market.buy("08:45:00", "101", 2_000);
    assert_eq!(market.seen_for(id), [Seen::Accepted]);
    // The trial match shows sell orders below the order's price: nothing happens.
    market.indicative(
        "08:50:00",
        Phase::OpenAuction,
        book(&[("100", 9_000)], &[("100.5", 9_000)]),
        Some(("100.5", 4_000)),
    );
    assert_eq!(market.seen(), []);
    assert_eq!(market.remaining(id), Some(2_000));

    market.auction(
        "09:00:00",
        Phase::OpenAuction,
        book(&[("100", 3_000)], &[("100.5", 1_000), ("101", 500)]),
        Some(("100.5", 5_000)),
    );
    let events = market.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].order, id);
    assert_eq!(events[0].time, market.at("09:00:00"));
    let OrderEventKind::Filled(fill) = events[0].kind else {
        panic!("{:?}", events[0]);
    };
    assert_eq!(fill.price, common::price("100.5"));
    assert_eq!(fill.qty.get(), 2_000);
    assert_eq!(fill.basis, FillBasis::AuctionBetter);
    assert_eq!(fill.trigger, Some((Feed::TwseF6, market.seq())));
    assert_eq!(market.engine.resting(id), None);
}

#[test]
fn trial_matches_and_delays_do_not_touch_resting_orders() {
    let mut market = Market::opened(BID, ASK);
    let id = market.buy("09:01:00", "99.5", 1_000);
    market.buy("09:01:01", "100", 2_000);
    market.seen();

    market.delay("10:00:00");
    // The trial book is what would be left after the match, not the resting orders.
    market.indicative(
        "10:00:30",
        Phase::PeriodicAuction,
        book(&[("99.5", 100)], &[("99.6", 9_000)]),
        Some(("99.5", 7_000)),
    );
    assert_eq!(market.seen(), []);
    assert_eq!(market.queue(id), 5_000);

    // The auction ends the delay with the old book: what was taken from 100 is still
    // taken.
    market.auction("10:02:00", Phase::PeriodicAuction, book(BID, ASK), None);
    assert_eq!(market.seen(), []);
    assert_eq!(
        probe(&mut market, "10:02:01", Side::Buy, "100", 1_000),
        [Seen::Cancelled(1_000, CancelReason::IocRemainder)]
    );
}

#[test]
fn an_order_sent_during_a_delay_trades_in_the_auction_that_ends_it() {
    let mut market = Market::opened(BID, ASK);
    market.delay("10:00:00");
    let id = market.buy("10:00:30", "101", 2_000);
    // The ask of 100 is on the book, but the book is collecting orders.
    assert_eq!(market.seen_for(id), [Seen::Accepted]);

    market.auction(
        "10:02:00",
        Phase::PeriodicAuction,
        book(&[("100", 1_000)], &[("100.5", 500), ("101", 4_000)]),
        Some(("100.5", 3_000)),
    );
    assert_eq!(
        market.seen(),
        [(id, filled("100.5", 2_000, FillBasis::AuctionBetter))]
    );

    // Continuous trading is back; the 500 left at 100.5 were taken in the auction.
    assert_eq!(
        probe(&mut market, "10:02:01", Side::Buy, "101", 1_000),
        [filled("101", 1_000, FillBasis::Aggressive)]
    );
}

// ---- 7.2 step 1: orders better than the auction price ----------------------------------

#[test]
fn better_priced_orders_trade_at_the_auction_price_up_to_what_real_orders_traded() {
    let mut market = Market::regular();
    let low = market.buy("08:40:00", "101", 4_000);
    let high = market.buy("08:45:00", "102", 4_000);
    market.seen();
    // 5,000 traded at 100.5 and 1,000 real shares are left for sale at that price.
    market.auction(
        "09:00:00",
        Phase::OpenAuction,
        book(&[("100", 3_000)], &[("100.5", 1_000), ("103", 500)]),
        Some(("100.5", 5_000)),
    );
    assert_eq!(
        market.seen(),
        [
            // The better price goes first, whatever the arrival order.
            (high, filled("100.5", 4_000, FillBasis::AuctionBetter)),
            // 1,000 + 5,000 - 4,000 are left.
            (low, filled("100.5", 2_000, FillBasis::AuctionBetter)),
        ]
    );
    assert_eq!(market.remaining(low), Some(2_000));
    assert_eq!(market.queue(low), 0);
    // The 1,000 left at 100.5 went to the first order.
    assert_eq!(
        probe(&mut market, "09:00:01", Side::Buy, "100.5", 1_000),
        [Seen::Cancelled(1_000, CancelReason::IocRemainder)]
    );
}

#[test]
fn a_better_priced_sell_order_is_treated_the_same_way() {
    let mut market = Market::regular();
    let id = market.sell("08:45:00", "99", 7_000);
    market.seen();
    market.auction(
        "09:00:00",
        Phase::OpenAuction,
        book(&[("100.5", 1_000), ("98", 500)], &[("101", 3_000)]),
        Some(("100.5", 5_000)),
    );
    assert_eq!(
        market.seen(),
        [(id, filled("100.5", 6_000, FillBasis::AuctionBetter))]
    );
    // The bid of 98 is below its price.
    assert_eq!(market.remaining(id), Some(1_000));
}

// ---- 7.2 step 2: orders at the auction price --------------------------------------------

#[test]
fn an_order_at_the_auction_price_takes_what_the_other_side_left() {
    let mut market = Market::regular();
    let first = market.buy("08:45:00", "100.5", 2_000);
    let second = market.buy("08:46:00", "100.5", 3_000);
    market.seen();
    market.auction(
        "09:00:00",
        Phase::OpenAuction,
        book(&[("100", 3_000)], &[("100.5", 3_000), ("101", 500)]),
        Some(("100.5", 5_000)),
    );
    assert_eq!(
        market.seen(),
        [
            (first, filled("100.5", 2_000, FillBasis::AuctionOpposite)),
            (second, filled("100.5", 1_000, FillBasis::AuctionOpposite)),
        ]
    );
    assert_eq!(market.remaining(second), Some(2_000));
}

#[test]
fn an_order_at_the_auction_price_does_not_trade_when_the_other_side_left_nothing() {
    let mut market = Market::regular();
    let id = market.buy("08:45:00", "100.5", 3_000);
    market.seen();
    // Buy orders are left at 100.5: which of them traded is not in the data.
    market.auction(
        "09:00:00",
        Phase::OpenAuction,
        book(&[("100.5", 2_000), ("100", 3_000)], &[("101", 500)]),
        Some(("100.5", 5_000)),
    );
    assert_eq!(market.seen(), []);
    // 7.3: the real orders left at the price are ahead.
    assert_eq!(market.queue(id), 2_000);
}

#[test]
fn the_queue_setting_fills_orders_at_the_auction_price_from_the_real_volume() {
    let mut market = regular(AuctionMarginal::Queue);
    let first = market.buy("08:45:00", "100.5", 3_000);
    let second = market.buy("08:46:00", "100.5", 3_000);
    market.seen();
    market.auction(
        "09:00:00",
        Phase::OpenAuction,
        book(&[("100.5", 2_000), ("100", 3_000)], &[("101", 500)]),
        Some(("100.5", 5_000)),
    );
    // Nothing was shown ahead of them before the open: together they get the 5,000.
    assert_eq!(
        market.seen(),
        [
            (first, filled("100.5", 3_000, FillBasis::AuctionQueue)),
            (second, filled("100.5", 2_000, FillBasis::AuctionQueue)),
        ]
    );
    assert_eq!(market.queue(second), 2_000);
}

/// Two buy orders rest at 99.5 behind 5,000 real shares; the closing auction trades
/// 8,000 at 99.5 and leaves real buy orders at that price.
fn close_at_the_resting_price(marginal: AuctionMarginal) -> (Vec<(OrderId, Seen)>, [OrderId; 2]) {
    let mut market = regular(marginal);
    market.auction("09:00:00", Phase::OpenAuction, book(BID, ASK), None);
    let first = market.buy("09:01:00", "99.5", 2_000);
    let second = market.buy("09:02:00", "99.5", 3_000);
    market.seen();
    market.auction(
        "13:30:00",
        Phase::CloseAuction,
        book(&[("99.5", 2_000), ("99", 4_000)], ASK),
        Some(("99.5", 8_000)),
    );
    (market.seen(), [first, second])
}

#[test]
fn real_orders_ahead_trade_first_at_the_auction_price() {
    let (seen, [first, second]) = close_at_the_resting_price(AuctionMarginal::Queue);
    assert_eq!(
        seen,
        [
            // 8,000 traded and 5,000 were ahead: 3,000 reach the two orders.
            (first, filled("99.5", 2_000, FillBasis::AuctionQueue)),
            (second, filled("99.5", 1_000, FillBasis::AuctionQueue)),
            (second, Seen::Expired(2_000)),
        ]
    );

    let (seen, [first, second]) = close_at_the_resting_price(AuctionMarginal::Conservative);
    assert_eq!(
        seen,
        [
            (first, Seen::Expired(2_000)),
            (second, Seen::Expired(3_000))
        ]
    );
}

#[test]
fn an_order_with_an_unknown_queue_gets_no_queue_fill() {
    let five: Levels = &[
        ("99.5", 1_000),
        ("99", 1_000),
        ("98.5", 1_000),
        ("98", 1_000),
        ("97.5", 1_000),
    ];
    let mut market = regular(AuctionMarginal::Queue);
    market.auction("09:00:00", Phase::OpenAuction, book(five, ASK), None);
    let id = market.buy("09:01:00", "97", 1_000);
    assert!(market.queue_is_unknown(id));
    market.seen();
    market.auction(
        "13:30:00",
        Phase::CloseAuction,
        book(&[("97", 500)], &[("97.5", 100)]),
        Some(("97", 9_000)),
    );
    assert_eq!(market.seen(), [(id, Seen::Expired(1_000))]);
}

// ---- 7.2 step 3: orders that cross the book after the auction ---------------------------

/// An odd-lot sell order of `qty` shares at `limit` meets bids of 300 at 99.5 and 200 at
/// 99 in a round without a real trade. `last` is the price of the day's only real trade.
fn odd_lot_sell(limit: &str, qty: i64, last: Option<&str>) -> (Vec<Seen>, Option<i64>) {
    let mut market = Market::odd_lot(AuctionMarginal::Conservative);
    market.auction(
        "09:10:00",
        Phase::OpenAuction,
        book(&[("98", 100)], &[("101", 100)]),
        last.map(|at| (at, 10)),
    );
    let id = market.sell("09:10:01", limit, qty);
    assert_eq!(market.seen_for(id), [Seen::Accepted]);
    market.auction(
        "09:10:05",
        Phase::PeriodicAuction,
        book(&[("99.5", 300), ("99", 200)], &[("101", 100)]),
        None,
    );
    (market.seen_for(id), market.remaining(id))
}

#[test]
fn a_crossing_order_trades_at_the_last_level_it_takes_from() {
    assert_eq!(
        odd_lot_sell("99", 100, None),
        (vec![filled("99.5", 100, FillBasis::AuctionResidual)], None)
    );
    assert_eq!(
        odd_lot_sell("99", 400, None),
        (vec![filled("99", 400, FillBasis::AuctionResidual)], None)
    );
}

#[test]
fn a_crossing_order_that_is_not_filled_trades_at_its_own_price() {
    assert_eq!(
        odd_lot_sell("99", 700, None),
        (
            vec![filled("99", 500, FillBasis::AuctionResidual)],
            Some(200)
        )
    );
    assert_eq!(
        odd_lot_sell("98.5", 700, None),
        (
            vec![filled("98.5", 500, FillBasis::AuctionResidual)],
            Some(200)
        )
    );
}

#[test]
fn a_crossing_order_that_empties_a_level_trades_at_the_price_nearest_the_last_trade() {
    // 300 shares empty the level of 99.5. Any price from 99 to 99.5 clears the same
    // quantity.
    for (last, expected) in [
        // No trade today: the reference price of 100.
        (None, "99.5"),
        (Some("100"), "99.5"),
        (Some("99.2"), "99.2"),
        (Some("98"), "99"),
    ] {
        assert_eq!(
            odd_lot_sell("99", 300, last),
            (
                vec![filled(expected, 300, FillBasis::AuctionResidual)],
                None
            ),
            "{last:?}"
        );
        // An order priced below the next level cannot trade below that level either:
        // the bids of 99 would have to be filled first.
        assert_eq!(
            odd_lot_sell("98.5", 300, last),
            (
                vec![filled(expected, 300, FillBasis::AuctionResidual)],
                None
            ),
            "{last:?}"
        );
    }
    // Both levels emptied and nothing shown beyond: down to the order's own price.
    assert_eq!(
        odd_lot_sell("98.5", 500, Some("98")),
        (vec![filled("98.5", 500, FillBasis::AuctionResidual)], None)
    );
    assert_eq!(
        odd_lot_sell("98.5", 500, Some("99.2")),
        (vec![filled("99", 500, FillBasis::AuctionResidual)], None)
    );
}

#[test]
fn a_crossing_buy_order_is_priced_the_same_way() {
    // Asks of 100 at 95 and 500 at 95.5.
    for (limit, qty, last, expected) in [
        // The level of 95 is not emptied.
        ("96", 50, "97", "95"),
        // It is emptied: from 95 up to the next level.
        ("96", 100, "97", "95.5"),
        ("96", 100, "95.2", "95.2"),
        ("96", 100, "94", "95"),
        // Or up to the order's own price, when that comes first.
        ("95.2", 100, "97", "95.2"),
        // Both levels emptied.
        ("96", 600, "97", "96"),
        ("96", 600, "94", "95.5"),
        // Not filled: the order's own price.
        ("96", 700, "94", "96"),
    ] {
        let mut market = Market::odd_lot(AuctionMarginal::Conservative);
        market.auction(
            "09:10:00",
            Phase::OpenAuction,
            book(&[("93", 100)], &[("98", 100)]),
            Some((last, 10)),
        );
        let id = market.buy("09:10:01", limit, qty);
        assert_eq!(market.seen_for(id), [Seen::Accepted]);
        market.auction(
            "09:10:05",
            Phase::PeriodicAuction,
            book(&[("93", 100)], &[("95", 100), ("95.5", 500)]),
            None,
        );
        assert_eq!(
            market.seen(),
            [(
                id,
                filled(expected, qty.min(600), FillBasis::AuctionResidual)
            )],
            "{limit} {qty} {last}"
        );
    }
}

#[test]
fn what_an_order_took_before_an_auction_is_not_there_in_the_auction() {
    for (shown, expected) in [
        (2_000, vec![Seen::Expired(1_000)]),
        (
            2_600,
            vec![
                filled("100", 600, FillBasis::AuctionResidual),
                Seen::Expired(400),
            ],
        ),
    ] {
        let mut market = Market::opened(BID, ASK);
        // Takes the 2,000 at 100 and rests with 1,000.
        let id = market.buy("09:01:00", "100", 3_000);
        market.seen();
        market.auction(
            "13:30:00",
            Phase::CloseAuction,
            book(BID, &[("100", shown), ("100.5", 3_000)]),
            None,
        );
        assert_eq!(market.seen_for(id), expected, "{shown}");
    }
}

// ---- 7.3 after the auction --------------------------------------------------------------

#[test]
fn the_real_orders_left_at_the_price_are_ahead_of_an_order_that_joined_the_auction() {
    let mut market = Market::opened(BID, ASK);
    let resting = market.buy("09:01:00", "99.5", 1_000);
    market.delay("10:00:00");
    let joined = market.buy("10:00:30", "99.5", 1_000);
    assert_eq!(market.queue(resting), 5_000);
    assert_eq!(market.queue(joined), 5_000);

    market.auction(
        "10:02:00",
        Phase::PeriodicAuction,
        book(&[("99.5", 7_000), ("99", 4_000)], ASK),
        None,
    );
    // The order that was resting keeps its place; the other one is behind everything
    // the level shows.
    assert_eq!(market.queue(resting), 5_000);
    assert_eq!(market.queue(joined), 7_000);

    // From here on it is an ordinary resting order.
    market.disclose("10:02:01", &[("99.5", 9_000), ("99", 4_000)], ASK);
    assert_eq!(market.queue(joined), 7_000);
    market.disclose("10:02:02", &[("99.5", 3_000), ("99", 4_000)], ASK);
    assert_eq!(market.queue(resting), 3_000);
    assert_eq!(market.queue(joined), 3_000);
}

#[test]
fn everything_left_expires_after_the_closing_auction() {
    let mut market = Market::opened(BID, ASK);
    let sell = market.sell("09:01:00", "100.5", 1_000);
    let buy = market.buy("09:02:00", "99", 2_000);
    let late = market.buy("13:26:00", "99.5", 1_000);
    market.seen();
    market.auction(
        "13:30:00",
        Phase::CloseAuction,
        book(BID, &[("100", 500), ("100.5", 3_000)]),
        Some(("100", 9_000)),
    );
    assert_eq!(
        market.seen(),
        [
            (sell, Seen::Expired(1_000)),
            (buy, Seen::Expired(2_000)),
            (late, Seen::Expired(1_000)),
        ]
    );
    assert_eq!(market.engine.resting(sell), None);
    // Nothing is left for the end of the day.
    market.end_day("14:30:00");
    assert_eq!(market.seen(), []);
}

// ---- 6.6 after an auction ---------------------------------------------------------------

#[test]
fn the_band_starts_from_the_opening_price() {
    let mut market = Market::regular();
    market.auction(
        "09:00:00",
        Phase::OpenAuction,
        book(&[("96.5", 1_000)], &[("100", 1_000), ("100.5", 1_000)]),
        Some(("97", 1_000)),
    );
    // 97 x 1.035 = 100.395.
    let id = market.buy("09:01:00", "100.5", 2_000);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            filled("100", 1_000, FillBasis::Aggressive),
            Seen::PriceBandLimited
        ]
    );
}

#[test]
fn the_band_starts_again_from_the_price_of_the_auction_that_ends_a_delay() {
    let mut market = Market::opened(BID, ASK);
    market.delay("09:10:00");
    market.auction(
        "09:12:00",
        Phase::PeriodicAuction,
        book(
            &[("103.5", 1_000)],
            &[("105", 1_000), ("107.5", 1_000), ("108", 1_000)],
        ),
        Some(("104", 1_000)),
    );
    // 104 x 1.035 = 107.64.
    let id = market.buy("09:12:01", "108", 3_000);
    assert_eq!(
        market.seen_for(id),
        [
            Seen::Accepted,
            filled("105", 1_000, FillBasis::Aggressive),
            filled("107.5", 1_000, FillBasis::Aggressive),
            Seen::PriceBandLimited
        ]
    );
}

// ---- odd lots -----------------------------------------------------------------------------

#[test]
fn an_odd_lot_order_takes_part_in_every_round_until_it_is_filled() {
    let mut market = Market::odd_lot(AuctionMarginal::Conservative);
    market.auction(
        "09:10:00",
        Phase::OpenAuction,
        book(&[("99", 100)], &[("100", 100)]),
        None,
    );
    let id = market.buy("09:10:01", "99.5", 50);
    assert_eq!(market.seen_for(id), [Seen::Accepted]);

    market.indicative(
        "09:10:03",
        Phase::PeriodicAuction,
        book(&[("99", 100)], &[("99.5", 30)]),
        Some(("99.5", 200)),
    );
    assert_eq!(market.seen(), []);

    // 200 trade at 99.5 and 30 are left for sale.
    market.auction(
        "09:10:05",
        Phase::PeriodicAuction,
        book(&[("99", 100)], &[("99.5", 30), ("100", 100)]),
        Some(("99.5", 200)),
    );
    assert_eq!(
        market.seen(),
        [(id, filled("99.5", 30, FillBasis::AuctionOpposite))]
    );
    assert_eq!(market.remaining(id), Some(20));

    // The same 30 are still shown: they are not taken twice.
    market.auction(
        "09:10:10",
        Phase::PeriodicAuction,
        book(&[("99", 100)], &[("99.5", 30), ("100", 100)]),
        None,
    );
    assert_eq!(market.seen(), []);

    // 10 trade at 99.4, below the order's price.
    market.auction(
        "09:10:15",
        Phase::PeriodicAuction,
        book(&[("99.4", 40)], &[("100", 100)]),
        Some(("99.4", 10)),
    );
    assert_eq!(
        market.seen(),
        [(id, filled("99.4", 10, FillBasis::AuctionBetter))]
    );
    assert_eq!(market.remaining(id), Some(10));

    market.auction(
        "13:30:00",
        Phase::CloseAuction,
        book(&[("99.4", 40)], &[("100", 100)]),
        None,
    );
    assert_eq!(market.seen(), [(id, Seen::Expired(10))]);
}
