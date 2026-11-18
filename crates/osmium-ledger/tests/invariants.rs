//! Accounting identities on generated trading: whatever the orders and fills are,
//! cash is the settled entries, equity is cash plus positions, and the result of the
//! run is the realised results plus what the open lots have gained.

use std::collections::{HashMap, HashSet};

use osmium_ledger::{
    Calendar, CashCheck, Config, DayFacts, DayTrade, Entry, Item, Ledger, RealizedKind,
    SecurityClass, SecurityFacts, Uncovered,
};
use osmium_types::{
    Board, DailyQuote, Date, Fill, FillBasis, Money, NewOrder, OrderEvent, OrderEventKind, OrderId,
    OrderPrice, Price, Shares, Side, Symbol, TimeInForce, Timestamp, TradeKind,
};

/// A small deterministic generator; the tests must not depend on a seed from outside.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: i64) -> i64 {
        i64::try_from(self.next() % u64::try_from(bound).unwrap()).unwrap()
    }

    fn between(&mut self, low: i64, high: i64) -> i64 {
        low + self.below(high - low + 1)
    }
}

const SYMBOLS: [&str; 3] = ["2330", "0050", "00679B"];
const DAYS: i32 = 40;

fn calendar() -> (Calendar, Vec<Date>) {
    let first: Date = "2024-03-04".parse().unwrap();
    let days: Vec<Date> = (0..DAYS * 7 / 5 + 7)
        .filter(|offset| offset % 7 < 5)
        .map(|offset| Date::from_epoch_days(first.epoch_days() + offset).unwrap())
        .collect();
    (Calendar::new(days.clone(), []), days)
}

/// Prices are whole multiples of five cents.
fn ticks(count: i64) -> Price {
    Price::from_units(count * Price::UNITS_PER_YUAN / 20)
}

fn facts(index: usize, reference: i64) -> SecurityFacts {
    SecurityFacts {
        class: [
            SecurityClass::Stock,
            SecurityClass::Etf,
            SecurityClass::BondEtf,
        ][index],
        ref_price: Some(ticks(reference)),
        limit_up: Some(ticks(reference + reference / 10)),
        limit_down: Some(ticks(reference - reference / 10)),
        day_trade: DayTrade {
            buy_first: true,
            sell_first: index != 2,
            approx: false,
        },
        disposition: false,
        margin_eligible: true,
        short_eligible: true,
        short_suspended: false,
        short_last_cover_day: false,
        short_price_floor: false,
        non_10_par: Some(false),
        shares_per_lot: Some(1_000),
    }
}

struct Run {
    ledger: Ledger,
    random: Random,
    next_order: u64,
    entries: Vec<Entry>,
    realized: Money,
    forced: bool,
}

impl Run {
    fn fill(&mut self, id: OrderId, price: Price, qty: i64) {
        self.ledger.on_event(&OrderEvent {
            order: id,
            time: Timestamp::from_micros(0),
            kind: OrderEventKind::Filled(Fill {
                price,
                qty: Shares::new(qty),
                basis: FillBasis::Aggressive,
                trigger: None,
            }),
        });
    }

    fn expire(&mut self, id: OrderId, qty: i64) {
        if qty > 0 {
            self.ledger.on_event(&OrderEvent {
                order: id,
                time: Timestamp::from_micros(0),
                kind: OrderEventKind::Expired {
                    qty: Shares::new(qty),
                },
            });
        }
    }

    /// Sends one generated order and fills a generated part of it in up to three
    /// fills, each at the order's price or better.
    fn order(&mut self, symbol: Symbol, reference: i64) {
        let position = self.ledger.position(symbol);
        let kind = [
            TradeKind::Cash,
            TradeKind::Cash,
            TradeKind::CashSellFirst,
            TradeKind::MarginBuy,
            TradeKind::MarginSell,
            TradeKind::ShortSell,
            TradeKind::ShortCover,
        ][usize::try_from(self.random.below(7)).unwrap()];
        let side = kind.side().unwrap_or(if self.random.below(2) == 0 {
            Side::Buy
        } else {
            Side::Sell
        });
        let odd = kind == TradeKind::Cash && self.random.below(4) == 0;
        let qty = if odd {
            self.random.between(1, 999)
        } else {
            self.random.between(1, 3) * 1_000
        };
        let qty = match (kind, side) {
            (TradeKind::Cash, Side::Sell) => qty.min(position.cash),
            (TradeKind::MarginSell, _) => qty.min(position.margin),
            (TradeKind::ShortCover, _) => qty.min(position.short),
            _ => qty,
        };
        if qty <= 0 {
            return;
        }
        let band = reference / 10;
        let limit = reference + self.random.between(-band, band);
        let opens = matches!(
            (kind, side),
            (TradeKind::Cash, Side::Buy) | (TradeKind::MarginBuy | TradeKind::ShortSell, _)
        );
        if opens && self.random.below(4) == 0 {
            self.all_in(symbol, kind, side, limit, reference + band);
            return;
        }
        let market = self.random.below(8) == 0;
        let order = NewOrder {
            symbol,
            board: if odd { Board::OddLot } else { Board::Regular },
            side,
            kind,
            price: if market {
                OrderPrice::Market
            } else {
                OrderPrice::Limit(ticks(limit))
            },
            time_in_force: TimeInForce::Rod,
            qty: Shares::new(qty),
        };
        self.next_order += 1;
        let id = OrderId(self.next_order);
        if self.ledger.check_new(id, &order).is_err() {
            return;
        }
        let (low, high) = match (side, market) {
            (_, true) => (reference - band, reference + band),
            (Side::Buy, false) => (reference - band, limit),
            (Side::Sell, false) => (limit, reference + band),
        };
        let mut left = qty;
        for _ in 0..self.random.below(4) {
            if left == 0 {
                break;
            }
            let part = self.random.between(1, left);
            let price = ticks(self.random.between(low, high));
            self.fill(id, price, part);
            left -= part;
        }
        self.expire(id, left);
    }

    /// Sends the largest order the account accepts and fills it at the price that
    /// costs the most, so that the account is left with as little as the checks allow.
    fn all_in(&mut self, symbol: Symbol, kind: TradeKind, side: Side, limit: i64, limit_up: i64) {
        let order = |qty: i64| NewOrder {
            symbol,
            board: Board::Regular,
            side,
            kind,
            price: OrderPrice::Limit(ticks(limit)),
            time_in_force: TimeInForce::Rod,
            qty: Shares::new(qty),
        };
        let accepts = |ledger: &Ledger, qty: i64| {
            ledger
                .clone()
                .check_new(OrderId(u64::MAX), &order(qty))
                .is_ok()
        };
        if !accepts(&self.ledger, 1) {
            return;
        }
        let (mut low, mut high) = (1_i64, 1_000_000_i64);
        while low < high {
            let middle = (low + high + 1) / 2;
            if accepts(&self.ledger, middle) {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        self.next_order += 1;
        let id = OrderId(self.next_order);
        self.ledger.check_new(id, &order(low)).unwrap();
        let worst = ticks(match side {
            Side::Buy => limit,
            Side::Sell => limit_up,
        });
        let mut left = low;
        while left > 0 {
            let part = if self.random.below(2) == 0 {
                left
            } else {
                self.random.between(1, left)
            };
            self.fill(id, worst, part);
            left -= part;
        }
    }

    /// Buys back what was sold first and is still open, as a strategy has to.
    fn cover(&mut self, symbol: Symbol, reference: i64) {
        let open = -self.ledger.position(symbol).cash;
        if open <= 0 {
            return;
        }
        let price = ticks(reference);
        self.next_order += 1;
        let id = OrderId(self.next_order);
        let order = NewOrder {
            symbol,
            board: Board::Regular,
            side: Side::Buy,
            kind: TradeKind::Cash,
            price: OrderPrice::Limit(price),
            time_in_force: TimeInForce::Rod,
            qty: Shares::new(open),
        };
        if self.ledger.check_new(id, &order).is_ok() {
            self.fill(id, price, open);
        }
    }
}

/// What the open lots have gained at the valuation prices.
fn unrealized(ledger: &Ledger) -> Money {
    let mut total = Money::ZERO;
    for (symbol, holding) in ledger.holdings() {
        let price = ledger.valuation(symbol).unwrap();
        let value = |shares: i64| {
            Money::new(
                i64::try_from(
                    i128::from(price.units()) * i128::from(shares)
                        / i128::from(Price::UNITS_PER_YUAN),
                )
                .unwrap(),
            )
        };
        total +=
            value(holding.cash_shares()) - holding.cash.iter().map(|lot| lot.cost).sum::<Money>();
        total += value(holding.margin_shares())
            - holding.margin.iter().map(|lot| lot.cost).sum::<Money>();
        total += holding
            .short
            .iter()
            .map(|lot| lot.collateral)
            .sum::<Money>()
            - value(holding.short_shares());
        total += holding.owed.iter().map(|lot| lot.proceeds).sum::<Money>()
            - value(holding.owed_shares());
    }
    total
}

/// Runs one generated account and returns the kinds of results it realised and
/// whether the broker forced an order.
fn run(seed: u64, cash_check: CashCheck, uncovered: Uncovered) -> (HashSet<RealizedKind>, bool) {
    let initial = Money::new(600_000);
    let mut config = Config::new(initial);
    config.account.cash_check = cash_check;
    config.daytrade.uncovered = uncovered;
    let (calendar, days) = calendar();
    let symbols: Vec<Symbol> = SYMBOLS
        .iter()
        .map(|code| Symbol::new(code).unwrap())
        .collect();
    let mut run = Run {
        ledger: Ledger::new(config, calendar),
        random: Random(seed),
        next_order: 0,
        entries: Vec::new(),
        realized: Money::ZERO,
        forced: false,
    };
    let mut kinds = HashSet::new();
    // Reference prices in ticks of five cents: 100, 50 and 15 yuan.
    let mut references = [2_000_i64, 1_000, 300];
    for &date in &days[..usize::try_from(DAYS).unwrap()] {
        let securities: HashMap<Symbol, SecurityFacts> = symbols
            .iter()
            .enumerate()
            .map(|(index, &symbol)| (symbol, facts(index, references[index])))
            .collect();
        let open = run
            .ledger
            .begin_day(DayFacts {
                date,
                securities,
                actions: Vec::new(),
            })
            .unwrap();
        run.entries.extend(&open.entries);
        // With the check at the order, and as long as the broker has not forced
        // anything, no settlement leaves the account without cash.
        if cash_check == CashCheck::AtOrder && !run.forced {
            assert!(!run.ledger.cash().is_negative(), "seed {seed} on {date}");
        }
        run.forced |= !open.forced.is_empty();
        for (index, forced) in open.forced.iter().enumerate() {
            run.next_order += 1;
            let id = OrderId(run.next_order);
            run.ledger.check_forced(id, index).unwrap();
            // Most forced orders fill at the open; some are left for the next day.
            let qty = forced.order.qty.get();
            if run.random.below(5) == 0 {
                run.expire(id, qty);
            } else {
                let at = symbols
                    .iter()
                    .position(|&symbol| symbol == forced.order.symbol)
                    .unwrap();
                run.fill(id, ticks(references[at]), qty);
            }
        }
        for _ in 0..run.random.below(9) {
            let index = usize::try_from(run.random.below(3)).unwrap();
            run.order(symbols[index], references[index]);
        }
        if uncovered == Uncovered::Error || run.random.below(3) > 0 {
            for (index, &symbol) in symbols.iter().enumerate() {
                run.cover(symbol, references[index]);
            }
        }
        // With the cash check on, a buy-back can be refused for lack of cash; such a
        // run cannot go on under `uncovered = "error"`.
        let covered = symbols
            .iter()
            .all(|&symbol| run.ledger.position(symbol).cash >= 0);
        if uncovered == Uncovered::Error && !covered {
            return (kinds, run.forced);
        }
        let available = run.ledger.available();

        let mut quotes = HashMap::new();
        for (index, &symbol) in symbols.iter().enumerate() {
            let band = references[index] / 10;
            references[index] = (references[index] + run.random.between(-band, band)).max(40);
            quotes.insert(
                symbol,
                DailyQuote {
                    symbol,
                    close: Some(ticks(references[index])),
                    close_bid: None,
                    close_ask: None,
                    ref_price: None,
                    limit_down: None,
                },
            );
        }
        let close = run.ledger.end_day(&quotes).unwrap();
        run.entries.extend(&close.entries);
        run.realized += close
            .realized
            .iter()
            .map(|realized| realized.pnl)
            .sum::<Money>();
        for realized in &close.realized {
            kinds.insert(realized.kind);
            assert_eq!(
                realized.pnl,
                realized.proceeds - realized.cost + realized.interest,
                "seed {seed} on {date}"
            );
        }

        // Cash is the settled entries; pending is the rest.
        let (settled, later): (Vec<&Entry>, Vec<&Entry>) =
            run.entries.iter().partition(|entry| entry.settles <= date);
        let sum = |entries: &[&Entry]| entries.iter().map(|entry| entry.amount).sum::<Money>();
        assert_eq!(close.cash, initial + sum(&settled), "seed {seed} on {date}");
        assert_eq!(close.pending, sum(&later), "seed {seed} on {date}");
        // With every order finished, what is available is what the day leaves, apart
        // from the borrow fee of the close.
        let borrow_fees: Money = close
            .entries
            .iter()
            .filter(|entry| entry.item == Item::BorrowFee)
            .map(|entry| entry.amount)
            .sum();
        assert_eq!(
            available + borrow_fees,
            close.cash + close.pending,
            "seed {seed} on {date}"
        );
        // The result of the run is the realised results, the borrow fees, and what
        // the open lots have gained.
        let fees: Money = run
            .entries
            .iter()
            .filter(|entry| entry.item == Item::BorrowFee)
            .map(|entry| entry.amount)
            .sum();
        assert_eq!(
            close.equity - initial,
            run.realized + fees + unrealized(&run.ledger),
            "seed {seed} on {date}"
        );
        // Every lot has shares, and no cost or loan is left without them.
        for (_, holding) in run.ledger.holdings() {
            assert!(
                holding
                    .cash
                    .iter()
                    .all(|lot| lot.shares > 0 && lot.cost.get() >= 0)
            );
            assert!(
                holding
                    .margin
                    .iter()
                    .all(|lot| lot.shares > 0 && lot.loan.get() >= 0)
            );
            assert!(
                holding
                    .short
                    .iter()
                    .all(|lot| lot.shares > 0 && lot.margin.get() > 0)
            );
            assert!(holding.owed.iter().all(|lot| lot.shares > 0));
        }
    }
    (kinds, run.forced)
}

#[test]
fn the_books_balance_whatever_is_traded() {
    let mut kinds = HashSet::new();
    let mut forced = false;
    for seed in 0..300 {
        let seen = run(seed, CashCheck::None, Uncovered::ForceCover);
        kinds.extend(seen.0);
        forced |= seen.1;
    }
    // The generated trading reaches every way a position is closed.
    for kind in [
        RealizedKind::DayTrade,
        RealizedKind::Cash,
        RealizedKind::CreditOffset,
        RealizedKind::Margin,
        RealizedKind::Short,
        RealizedKind::SellFirstCover,
    ] {
        assert!(kinds.contains(&kind), "{kind:?}");
    }
    assert!(forced);
}

#[test]
fn checked_orders_never_leave_a_settlement_without_cash() {
    let mut kinds = HashSet::new();
    for seed in 0..300 {
        kinds.extend(run(seed, CashCheck::AtOrder, Uncovered::Error).0);
        kinds.extend(run(seed, CashCheck::AtOrder, Uncovered::ForceCover).0);
    }
    assert!(kinds.len() >= 5, "{kinds:?}");
}
