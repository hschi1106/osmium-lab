use std::collections::VecDeque;

use osmium_book::Method;
use osmium_types::{Board, Price, Shares, Side, Timestamp};

use crate::instrument::{BookSpec, InstrumentClass, PriceLimits};

/// Five minutes in microseconds.
const WINDOW_MICROS: i64 = 5 * 60 * 1_000_000;

/// A reference price as an exact ratio of price units: `value / qty`.
#[derive(Debug, Clone, Copy)]
pub struct Reference {
    value: i128,
    qty: i128,
}

impl PartialEq for Reference {
    fn eq(&self, other: &Self) -> bool {
        self.value * other.qty == other.value * self.qty
    }
}

impl Eq for Reference {}

impl Reference {
    const fn price(price: Price) -> Self {
        Self {
            value: price.units() as i128,
            qty: 1,
        }
    }

    /// Whether a fill at `price` for an order on `side` stays inside the band: at most
    /// 3.5 % above the reference for a buy, at most 3.5 % below it for a sell. A price
    /// exactly on the bound is inside.
    #[must_use]
    pub fn allows(self, side: Side, price: Price) -> bool {
        let scaled = i128::from(price.units()) * 1_000 * self.qty;
        match side {
            Side::Buy => scaled <= self.value * 1_035,
            Side::Sell => scaled >= self.value * 965,
        }
    }

    /// The reference as `(numerator, denominator)` in price units.
    #[must_use]
    pub const fn ratio(self) -> (i128, i128) {
        (self.value, self.qty)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Anchor {
    since: Timestamp,
    price: Price,
}

/// The reference price of the price-stabilisation band of one continuous book
/// (`docs/reference/matching.md` section 6.6). All times are exchange times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceBand {
    ref_price: Price,
    /// An opening price, or the price of the auction that ended a delay, which is the
    /// reference for the five minutes after it.
    anchor: Option<Anchor>,
    /// Trades not yet older than five minutes at the latest time asked about.
    window: VecDeque<(Timestamp, Price, Shares)>,
    window_value: i128,
    window_qty: i64,
    /// Trades at or before this time have left the window.
    cutoff: Timestamp,
    last: Option<Price>,
}

impl PriceBand {
    /// The band of a book, or `None` when the band does not apply to it: only regular-board
    /// books in continuous trading have one, and not warrants, instruments without known
    /// daily limits, or instruments whose reference price is missing or below 1 yuan.
    #[must_use]
    pub fn for_book(board: Board, spec: &BookSpec) -> Option<Self> {
        let applies = board == Board::Regular
            && spec.method == Method::Continuous
            && spec.class != InstrumentClass::Warrant
            && matches!(spec.limits, PriceLimits::Range { .. });
        spec.ref_price
            .filter(|price| applies && price.units() >= Price::UNITS_PER_YUAN)
            .map(Self::new)
    }

    /// `ref_price` is the day's reference price, used while the book has no trade.
    #[must_use]
    pub fn new(ref_price: Price) -> Self {
        Self {
            ref_price,
            anchor: None,
            window: VecDeque::new(),
            window_value: 0,
            window_qty: 0,
            cutoff: Timestamp::from_micros(i64::MIN),
            last: None,
        }
    }

    /// Records a real trade of continuous matching.
    ///
    /// Auction trades go to [`Self::on_open`] and [`Self::on_delay_end`] instead: real
    /// trades only fit the band when the five-minute average leaves them out
    /// (`docs/reference/matching.md` section 6.6).
    pub fn on_trade(&mut self, time: Timestamp, price: Price, qty: Shares) {
        self.last = Some(price);
        if time > self.cutoff {
            self.window.push_back((time, price, qty));
            self.window_value += i128::from(price.units()) * i128::from(qty.get());
            self.window_qty += qty.get();
        }
    }

    /// Records the opening auction result at `time`; `price` is `None` when the
    /// auction matched nothing.
    pub fn on_open(&mut self, time: Timestamp, price: Option<Price>) {
        self.last = price.or(self.last);
        self.anchor = Some(Anchor {
            since: time,
            price: price.unwrap_or(self.ref_price),
        });
    }

    /// Records the auction result that ended a matching delay at `time`. Without a
    /// trade the rolling average applies at once.
    pub fn on_delay_end(&mut self, time: Timestamp, price: Option<Price>) {
        self.last = price.or(self.last);
        self.anchor = price.map(|price| Anchor { since: time, price });
    }

    /// The reference price for an order entered at `now`.
    pub fn reference(&mut self, now: Timestamp) -> Reference {
        if let Some(anchor) = self.anchor {
            if now < anchor.since.plus_micros(WINDOW_MICROS) {
                return Reference::price(anchor.price);
            }
            self.anchor = None;
        }
        let cutoff = Timestamp::from_micros(now.micros().saturating_sub(WINDOW_MICROS));
        if cutoff > self.cutoff {
            self.cutoff = cutoff;
            while let Some(&(time, price, qty)) = self.window.front() {
                if time > cutoff {
                    break;
                }
                self.window.pop_front();
                self.window_value -= i128::from(price.units()) * i128::from(qty.get());
                self.window_qty -= qty.get();
            }
        }
        if self.window_qty > 0 {
            Reference {
                value: self.window_value,
                qty: i128::from(self.window_qty),
            }
        } else {
            Reference::price(self.last.unwrap_or(self.ref_price))
        }
    }
}

#[cfg(test)]
mod tests {
    use osmium_types::{Price, Shares, Side, Timestamp};

    use super::{PriceBand, Reference};

    fn price(text: &str) -> Price {
        text.parse().unwrap()
    }

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_micros(seconds * 1_000_000)
    }

    fn fixed(text: &str) -> Reference {
        Reference::price(price(text))
    }

    #[test]
    fn the_band_is_three_and_a_half_percent_with_the_bound_inside() {
        let reference = fixed("100");
        assert!(reference.allows(Side::Buy, price("103.5")));
        assert!(!reference.allows(Side::Buy, price("103.51")));
        assert!(reference.allows(Side::Buy, price("50")));
        assert!(reference.allows(Side::Sell, price("96.5")));
        assert!(!reference.allows(Side::Sell, price("96.49")));
        assert!(reference.allows(Side::Sell, price("150")));
    }

    #[test]
    fn the_bound_is_not_rounded_to_a_tick() {
        // 3.5 % above 33.35 is 34.517...; 34.5 is inside and 34.55 is outside.
        let reference = fixed("33.35");
        assert!(reference.allows(Side::Buy, price("34.5")));
        assert!(!reference.allows(Side::Buy, price("34.55")));
    }

    #[test]
    fn before_any_trade_the_reference_is_the_reference_price() {
        let mut band = PriceBand::new(price("80"));
        assert_eq!(band.reference(at(0)), fixed("80"));
    }

    #[test]
    fn the_opening_price_is_the_reference_for_five_minutes() {
        let mut band = PriceBand::new(price("80"));
        band.on_open(at(0), Some(price("82")));
        band.on_trade(at(60), price("84"), Shares::new(1_000));
        assert_eq!(band.reference(at(299)), fixed("82"));
        // From five minutes on the rolling average applies.
        assert_eq!(band.reference(at(300)), fixed("84"));
    }

    #[test]
    fn an_auction_price_is_the_last_trade_but_not_part_of_the_average() {
        let mut band = PriceBand::new(price("80"));
        band.on_open(at(0), Some(price("82")));
        // No trade since the open: the opening price is the last trade.
        assert_eq!(band.reference(at(900)), fixed("82"));

        let mut band = PriceBand::new(price("80"));
        band.on_trade(at(100), price("100"), Shares::new(1_000));
        band.on_delay_end(at(250), Some(price("90")));
        band.on_delay_end(at(380), None);
        // The second delay ended without a trade, so the average applies at once; the
        // 90 of the first auction is inside five minutes and still left out.
        assert_eq!(band.reference(at(390)), fixed("100"));
        // With nothing in the window it is the last trade.
        assert_eq!(band.reference(at(400)), fixed("90"));
    }

    #[test]
    fn an_opening_without_a_trade_anchors_the_reference_price() {
        let mut band = PriceBand::new(price("80"));
        band.on_open(at(0), None);
        band.on_trade(at(10), price("83"), Shares::new(1_000));
        assert_eq!(band.reference(at(200)), fixed("80"));
        assert_eq!(band.reference(at(305)), fixed("83"));
    }

    #[test]
    fn the_rolling_reference_is_the_exact_volume_weighted_average() {
        let mut band = PriceBand::new(price("80"));
        band.on_trade(at(10), price("100"), Shares::new(1_000));
        band.on_trade(at(20), price("101"), Shares::new(2_000));
        let reference = band.reference(at(30));
        // (100 * 1000 + 101 * 2000) / 3000 = 100.666...
        assert_eq!(
            reference.ratio(),
            (
                i128::from(price("100").units()) * 1_000 + i128::from(price("101").units()) * 2_000,
                3_000
            )
        );
        assert!(reference.allows(Side::Buy, price("104")));
        assert!(!reference.allows(Side::Buy, price("104.5")));
    }

    #[test]
    fn trades_leave_the_window_after_five_minutes() {
        let mut band = PriceBand::new(price("80"));
        band.on_trade(at(10), price("100"), Shares::new(1_000));
        band.on_trade(at(200), price("110"), Shares::new(1_000));
        assert_eq!(band.reference(at(309)).ratio().1, 2_000);
        // A trade exactly five minutes old is outside.
        assert_eq!(band.reference(at(310)), fixed("110"));
        // With no trade in the window the last trade price is the reference.
        assert_eq!(band.reference(at(500)), fixed("110"));
        assert_eq!(band.reference(at(9_000)), fixed("110"));
    }

    #[test]
    fn an_auction_after_a_delay_anchors_its_price_only_when_it_traded() {
        let mut band = PriceBand::new(price("80"));
        band.on_trade(at(10), price("100"), Shares::new(1_000));
        band.on_delay_end(at(400), Some(price("90")));
        band.on_trade(at(420), price("91"), Shares::new(1_000));
        assert_eq!(band.reference(at(699)), fixed("90"));
        assert_eq!(band.reference(at(700)), fixed("91"));

        let mut band = PriceBand::new(price("80"));
        band.on_open(at(0), Some(price("100")));
        band.on_trade(at(50), price("102"), Shares::new(1_000));
        // The delay ends without a trade inside the opening five minutes: the rolling
        // average replaces the opening price.
        band.on_delay_end(at(180), None);
        assert_eq!(band.reference(at(181)), fixed("102"));
    }

    #[test]
    fn a_query_with_an_earlier_time_does_not_bring_trades_back() {
        let mut band = PriceBand::new(price("80"));
        band.on_trade(at(10), price("100"), Shares::new(1_000));
        band.on_trade(at(300), price("110"), Shares::new(1_000));
        assert_eq!(band.reference(at(400)), fixed("110"));
        assert_eq!(band.reference(at(305)), fixed("110"));
        // A trade reported late with a time already outside the window is not added.
        band.on_trade(at(50), price("90"), Shares::new(1_000));
        assert_eq!(band.reference(at(400)), fixed("110"));
    }
}
