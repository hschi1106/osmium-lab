//! The ledger on hand-computed cases of `docs/reference/accounting.md`.

use std::collections::HashMap;

use osmium_ledger::{
    ActionOverride, Calendar, CashCheck, Config, DayClose, DayFacts, DayOpen, DayTrade, Entry,
    ForcedReason, Item, Ledger, LedgerError, MissingTerms, Position, RealizedKind, SecurityClass,
    SecurityFacts, Uncovered,
};
use osmium_types::{
    Board, CancelReason, CorporateAction, CorporateActionKind, DailyQuote, Date, Decimal8, Fill,
    FillBasis, Money, NewOrder, OrderEvent, OrderEventKind, OrderId, OrderPrice, PerShare, Price,
    RejectReason, Shares, Side, Symbol, TimeInForce, Timestamp, TradeKind,
};

fn day(text: &str) -> Date {
    text.parse().unwrap()
}

fn price(text: &str) -> Price {
    text.parse().unwrap()
}

fn symbol(code: &str) -> Symbol {
    Symbol::new(code).unwrap()
}

fn tsmc() -> Symbol {
    symbol("2330")
}

/// Monday to Friday of four weeks from 2024-03-04.
fn calendar() -> Calendar {
    let first = day("2024-03-04").epoch_days();
    Calendar::new(
        (0..28)
            .filter(|offset| offset % 7 < 5)
            .map(|offset| Date::from_epoch_days(first + offset).unwrap()),
        [],
    )
}

/// An ordinary stock with every kind of trading allowed.
fn stock(reference: &str) -> SecurityFacts {
    let units = price(reference).units();
    SecurityFacts {
        class: SecurityClass::Stock,
        ref_price: Some(price(reference)),
        limit_up: Some(Price::from_units(units / 10 * 11)),
        limit_down: Some(Price::from_units(units / 10 * 9)),
        day_trade: DayTrade {
            buy_first: true,
            sell_first: true,
            approx: false,
        },
        disposition: false,
        repeated_disposition: false,
        margin_eligible: true,
        short_eligible: true,
        short_suspended: false,
        short_last_cover_day: false,
        short_price_floor: false,
        non_10_par: Some(false),
        shares_per_lot: Some(1_000),
    }
}

struct Account {
    ledger: Ledger,
    next_order: u64,
    /// Every entry reported so far.
    entries: Vec<Entry>,
    initial: Money,
}

impl Account {
    fn new(config: Config) -> Self {
        Self {
            initial: config.account.initial_cash,
            ledger: Ledger::new(config, calendar()),
            next_order: 0,
            entries: Vec::new(),
        }
    }

    fn with_cash(cash: i64) -> Self {
        Self::new(Config::new(Money::new(cash)))
    }

    fn begin_with(
        &mut self,
        date: &str,
        securities: &[(Symbol, SecurityFacts)],
        actions: &[CorporateAction],
    ) -> Result<DayOpen, LedgerError> {
        let open = self.ledger.begin_day(DayFacts {
            date: day(date),
            securities: securities.iter().copied().collect(),
            actions: actions.to_vec(),
        })?;
        self.entries.extend(&open.entries);
        Ok(open)
    }

    fn begin(&mut self, date: &str, facts: SecurityFacts) -> DayOpen {
        self.begin_with(date, &[(tsmc(), facts)], &[]).unwrap()
    }

    fn order(&self, side: Side, kind: TradeKind, at: &str, qty: i64) -> NewOrder {
        let _ = self;
        NewOrder {
            symbol: tsmc(),
            board: Board::Regular,
            side,
            kind,
            price: OrderPrice::Limit(price(at)),
            time_in_force: TimeInForce::Rod,
            qty: Shares::new(qty),
        }
    }

    fn submit(&mut self, order: &NewOrder) -> Result<OrderId, RejectReason> {
        self.next_order += 1;
        let id = OrderId(self.next_order);
        self.ledger.check_new(id, order).map(|()| id)
    }

    fn event(&mut self, order: OrderId, kind: OrderEventKind) {
        self.ledger.on_event(&OrderEvent {
            order,
            time: Timestamp::from_micros(0),
            kind,
        });
    }

    fn fill(&mut self, order: OrderId, at: &str, qty: i64) {
        self.event(
            order,
            OrderEventKind::Filled(Fill {
                price: price(at),
                qty: Shares::new(qty),
                basis: FillBasis::Aggressive,
                trigger: None,
            }),
        );
    }

    /// Sends an order and fills all of it at its price.
    fn trade(&mut self, side: Side, kind: TradeKind, at: &str, qty: i64) -> OrderId {
        let order = self.order(side, kind, at, qty);
        let id = self.submit(&order).unwrap();
        self.fill(id, at, qty);
        id
    }

    fn end_result(&mut self, close: &str) -> Result<DayClose, LedgerError> {
        let quotes = HashMap::from([(
            tsmc(),
            DailyQuote {
                symbol: tsmc(),
                close: Some(price(close)),
                close_bid: None,
                close_ask: None,
                ref_price: None,
                limit_down: None,
            },
        )]);
        let close = self.ledger.end_day(&quotes)?;
        self.entries.extend(&close.entries);
        Ok(close)
    }

    fn end(&mut self, close: &str) -> DayClose {
        self.end_result(close).unwrap()
    }

    /// A day without orders.
    fn idle(&mut self, date: &str, close: &str) -> DayClose {
        self.begin(date, stock(close));
        self.end(close)
    }

    /// Cash is the initial cash plus every entry that has settled.
    fn assert_cash_is_settled_entries(&self, on: &str) {
        let settled: Money = self
            .entries
            .iter()
            .filter(|entry| entry.settles <= day(on))
            .map(|entry| entry.amount)
            .sum();
        assert_eq!(self.ledger.cash(), self.initial + settled);
    }
}

fn amounts(close: &DayClose) -> Vec<(Item, i64)> {
    close
        .entries
        .iter()
        .map(|entry| (entry.item, entry.amount.get()))
        .collect()
}

fn net(close: &DayClose) -> i64 {
    close.entries.iter().map(|entry| entry.amount.get()).sum()
}

#[test]
fn a_cash_round_trip_settles_two_days_later() {
    let mut account = Account::with_cash(1_000_000);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::Cash, "100", 2_000);
    // Value 200,000 and fee floor(285.0) are spoken for at once.
    assert_eq!(account.ledger.available(), Money::new(799_715));
    assert_eq!(account.ledger.position(tsmc()).cash, 2_000);
    let close = account.end("101");
    assert_eq!(close.settles, day("2024-03-06"));
    assert_eq!(
        amounts(&close),
        [(Item::TradeValue, -200_000), (Item::Fee, -285)]
    );
    assert_eq!(close.cash, Money::new(1_000_000));
    assert_eq!(close.pending, Money::new(-200_285));
    assert_eq!(close.equity, Money::new(1_001_715));
    assert_eq!(close.maintenance, None);

    account.idle("2024-03-05", "101");
    assert_eq!(account.ledger.cash(), Money::new(1_000_000));

    account.begin("2024-03-06", stock("101"));
    assert_eq!(account.ledger.cash(), Money::new(799_715));
    account.trade(Side::Sell, TradeKind::Cash, "105", 1_000);
    let close = account.end("105");
    // Fee floor(149.625), tax floor(315).
    assert_eq!(
        amounts(&close),
        [
            (Item::TradeValue, 105_000),
            (Item::Fee, -149),
            (Item::Tax, -315)
        ]
    );
    let [realized] = close.realized[..] else {
        panic!("one sale");
    };
    assert_eq!(realized.kind, RealizedKind::Cash);
    assert_eq!(realized.opened, day("2024-03-04"));
    // Half of the lot's 200,285, rounded down; the lot keeps the other yuan.
    assert_eq!(
        (realized.proceeds, realized.cost, realized.pnl),
        (Money::new(104_536), Money::new(100_142), Money::new(4_394))
    );
    let holding = account.ledger.holding(tsmc()).unwrap();
    assert_eq!(holding.cash[0].shares, 1_000);
    assert_eq!(holding.cash[0].cost, Money::new(100_143));
    // Equity change = realised + unrealised: 4,394 + (105,000 - 100,143).
    assert_eq!(close.equity, Money::new(1_009_251));

    account.idle("2024-03-07", "105");
    account.idle("2024-03-08", "105");
    assert_eq!(account.ledger.cash(), Money::new(904_251));
    account.assert_cash_is_settled_entries("2024-03-08");
}

#[test]
fn a_day_trade_pays_half_the_tax_and_leaves_no_lot() {
    for sell_first in [false, true] {
        let mut account = Account::with_cash(100_000);
        account.begin("2024-03-04", stock("50"));
        if sell_first {
            account.trade(Side::Sell, TradeKind::CashSellFirst, "51", 1_000);
            assert_eq!(account.ledger.position(tsmc()).cash, -1_000);
            account.trade(Side::Buy, TradeKind::Cash, "50", 1_000);
        } else {
            account.trade(Side::Buy, TradeKind::Cash, "50", 1_000);
            account.trade(Side::Sell, TradeKind::Cash, "51", 1_000);
        }
        assert_eq!(account.ledger.position(tsmc()), Position::default());
        let close = account.end("50");
        // Buy fee floor(71.25); sell fee floor(72.675) and tax floor(76.5) at 0.15 %.
        assert_eq!(net(&close), 781);
        let [realized] = close.realized[..] else {
            panic!("one pair");
        };
        assert_eq!(realized.kind, RealizedKind::DayTrade);
        assert_eq!(realized.pnl, Money::new(781));
        assert!(!realized.approx);
        assert!(account.ledger.holding(tsmc()).is_none());
        assert_eq!(close.equity, Money::new(100_781));
    }
}

#[test]
fn a_sale_beyond_the_days_buys_takes_the_ordinary_tax_from_inventory() {
    let mut account = Account::with_cash(1_000_000);
    account.begin("2024-03-04", stock("50"));
    account.trade(Side::Buy, TradeKind::Cash, "50", 2_000);
    account.end("50");
    account.begin("2024-03-05", stock("50"));
    account.trade(Side::Buy, TradeKind::Cash, "50", 1_000);
    // 1,000 pair with today's buy; 2,000 come out of yesterday's lot.
    let sale = account.trade(Side::Sell, TradeKind::Cash, "52", 3_000);
    let close = account.end("52");
    let tax = close
        .entries
        .iter()
        .find(|entry| entry.order == Some(sale) && entry.item == Item::Tax)
        .unwrap();
    // floor(52,000 x 0.15 %) + floor(104,000 x 0.3 %).
    assert_eq!(tax.amount, Money::new(-(78 + 312)));
    let kinds: Vec<(RealizedKind, i64)> = close
        .realized
        .iter()
        .map(|realized| (realized.kind, realized.shares))
        .collect();
    assert_eq!(
        kinds,
        [(RealizedKind::DayTrade, 1_000), (RealizedKind::Cash, 2_000)]
    );
    assert!(account.ledger.holding(tsmc()).is_none());
}

#[test]
fn working_orders_set_cash_aside_until_they_are_done() {
    let mut account = Account::with_cash(300_000);
    account.begin("2024-03-04", stock("100"));
    let order = account.order(Side::Buy, TradeKind::Cash, "100", 2_000);
    let id = account.submit(&order).unwrap();
    assert_eq!(account.ledger.available(), Money::new(99_715));
    // A second order of the same size no longer fits.
    assert_eq!(account.submit(&order), Err(RejectReason::InsufficientCash));
    account.fill(id, "99", 1_000);
    // 99,000 + fee 141 are spent; 100,000 + fee 142 are still set aside.
    assert_eq!(
        account.ledger.available(),
        Money::new(300_000 - 99_141 - 100_142)
    );
    account.event(
        id,
        OrderEventKind::Cancelled {
            qty: Shares::new(1_000),
            reason: CancelReason::Requested,
        },
    );
    assert_eq!(account.ledger.available(), Money::new(300_000 - 99_141));
    let close = account.end("99");
    assert_eq!(net(&close), -99_141);
}

#[test]
fn a_reprice_is_checked_against_cash() {
    let mut account = Account::with_cash(100_500);
    account.begin("2024-03-04", stock("100"));
    let order = account.order(Side::Buy, TradeKind::Cash, "95", 1_000);
    let id = account.submit(&order).unwrap();
    assert_eq!(
        account.ledger.check_reprice(id, price("101")),
        Err(RejectReason::InsufficientCash)
    );
    assert_eq!(account.ledger.available(), Money::new(100_500 - 95_135));
    assert_eq!(account.ledger.check_reprice(id, price("100")), Ok(()));
    assert_eq!(account.ledger.available(), Money::new(100_500 - 100_142));
    assert_eq!(
        account.ledger.check_reprice(OrderId(99), price("100")),
        Err(RejectReason::UnknownOrder)
    );
}

#[test]
fn orders_the_account_cannot_carry_are_refused() {
    let mut account = Account::with_cash(1_000_000);
    let plain = SecurityFacts {
        day_trade: DayTrade::default(),
        margin_eligible: false,
        short_eligible: false,
        ..stock("100")
    };
    account.begin("2024-03-04", plain);
    let reject = |account: &mut Account, side, kind, at| {
        let order = account.order(side, kind, at, 1_000);
        account.submit(&order).unwrap_err()
    };
    assert_eq!(
        reject(&mut account, Side::Sell, TradeKind::Cash, "100"),
        RejectReason::InsufficientPosition
    );
    assert_eq!(
        reject(&mut account, Side::Sell, TradeKind::CashSellFirst, "100"),
        RejectReason::DayTradeNotAllowed
    );
    assert_eq!(
        reject(&mut account, Side::Buy, TradeKind::MarginBuy, "100"),
        RejectReason::KindNotAllowed
    );
    assert_eq!(
        reject(&mut account, Side::Sell, TradeKind::ShortSell, "100"),
        RejectReason::KindNotAllowed
    );
    assert_eq!(
        reject(&mut account, Side::Buy, TradeKind::ShortSell, "100"),
        RejectReason::KindNotAllowed
    );
    assert_eq!(
        reject(&mut account, Side::Sell, TradeKind::MarginSell, "100"),
        RejectReason::InsufficientPosition
    );
    let unknown = NewOrder {
        symbol: symbol("9999"),
        ..account.order(Side::Buy, TradeKind::Cash, "100", 1_000)
    };
    assert_eq!(
        account.submit(&unknown),
        Err(RejectReason::UnsupportedInstrument)
    );
    // Without day trading, shares bought today cannot be sold today.
    account.trade(Side::Buy, TradeKind::Cash, "100", 1_000);
    assert_eq!(
        reject(&mut account, Side::Sell, TradeKind::Cash, "100"),
        RejectReason::InsufficientPosition
    );
    account.end("100");

    account.begin("2024-03-05", plain);
    let order = account.order(Side::Sell, TradeKind::Cash, "100", 1_000);
    account.submit(&order).unwrap();
    // The working sale holds the shares.
    assert_eq!(
        account.submit(&order),
        Err(RejectReason::InsufficientPosition)
    );
}

#[test]
fn short_sales_respect_suspensions_and_the_price_floor() {
    let mut account = Account::with_cash(1_000_000);
    account.begin(
        "2024-03-04",
        SecurityFacts {
            short_price_floor: true,
            ..stock("100")
        },
    );
    let below = account.order(Side::Sell, TradeKind::ShortSell, "99.5", 1_000);
    assert_eq!(
        account.submit(&below),
        Err(RejectReason::ShortPriceRestricted)
    );
    let market = NewOrder {
        price: OrderPrice::Market,
        ..below
    };
    assert_eq!(
        account.submit(&market),
        Err(RejectReason::ShortPriceRestricted)
    );
    let at_reference = account.order(Side::Sell, TradeKind::ShortSell, "100", 1_000);
    let id = account.submit(&at_reference).unwrap();
    assert_eq!(
        account.ledger.check_reprice(id, price("99")),
        Err(RejectReason::ShortPriceRestricted)
    );
    account.end("100");

    account.begin(
        "2024-03-05",
        SecurityFacts {
            short_suspended: true,
            ..stock("100")
        },
    );
    assert_eq!(
        account.submit(&at_reference),
        Err(RejectReason::ShortNotAllowed)
    );
}

#[test]
fn a_disposition_needs_settled_cash_and_settled_shares() {
    let mut account = Account::with_cash(350_000);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::Cash, "100", 1_000);
    account.end("100");

    let disposed = SecurityFacts {
        disposition: true,
        repeated_disposition: true,
        day_trade: DayTrade::default(),
        margin_eligible: false,
        ..stock("100")
    };
    account.begin("2024-03-05", disposed);
    // Yesterday's shares are not settled yet.
    let sell = account.order(Side::Sell, TradeKind::Cash, "100", 1_000);
    assert_eq!(account.submit(&sell), Err(RejectReason::PrepaymentRequired));
    // Settled cash is still 350,000: 200,285 can be prepaid once, not twice.
    let buy = account.order(Side::Buy, TradeKind::Cash, "100", 2_000);
    let id = account.submit(&buy).unwrap();
    assert_eq!(account.submit(&buy), Err(RejectReason::PrepaymentRequired));
    account.event(
        id,
        OrderEventKind::Cancelled {
            qty: Shares::new(2_000),
            reason: CancelReason::Requested,
        },
    );
    account.submit(&buy).unwrap();
    account.end("100");

    account.idle("2024-03-06", "100");
    account.begin("2024-03-07", disposed);
    account.submit(&sell).unwrap();
}

#[test]
fn the_daily_limit_counts_fills_and_working_orders() {
    let mut config = Config::new(Money::new(10_000_000));
    config.account.daily_limit = Some(Money::new(250_000));
    let mut account = Account::new(config);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::Cash, "100", 1_000);
    let order = account.order(Side::Buy, TradeKind::Cash, "100", 1_000);
    let id = account.submit(&order).unwrap();
    assert_eq!(
        account.submit(&order),
        Err(RejectReason::DailyLimitExceeded)
    );
    assert_eq!(account.ledger.check_reprice(id, price("105")), Ok(()));
    assert_eq!(account.ledger.check_reprice(id, price("110")), Ok(()));
    let small = account.order(Side::Sell, TradeKind::Cash, "100", 1_000);
    assert_eq!(
        account.submit(&small),
        Err(RejectReason::DailyLimitExceeded)
    );
}

#[test]
fn a_sell_first_left_open_ends_the_run_or_is_bought_back() {
    let mut account = Account::with_cash(1_000_000);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Sell, TradeKind::CashSellFirst, "100", 1_000);
    assert_eq!(
        account.end_result("100"),
        Err(LedgerError::UncoveredSellFirst {
            symbol: tsmc(),
            shares: 1_000
        })
    );

    let mut config = Config::new(Money::new(1_000_000));
    config.daytrade.uncovered = Uncovered::ForceCover;
    let mut account = Account::new(config);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Sell, TradeKind::CashSellFirst, "100", 1_000);
    let close = account.end("102");
    // Sale 100,000 - fee 142 - tax 300, and a borrow fee of 7 % of 102,000.
    assert_eq!(net(&close), 99_558 - 7_140);
    // The shares are owed at the valuation price.
    assert_eq!(
        close.equity,
        Money::new(1_000_000 + 99_558 - 7_140 - 102_000)
    );

    let open = account.begin("2024-03-05", stock("102"));
    let [forced] = open.forced[..] else {
        panic!("one forced order");
    };
    assert_eq!(forced.reason, ForcedReason::UncoveredSellFirst);
    assert_eq!(
        (forced.order.side, forced.order.kind, forced.order.price),
        (
            Side::Buy,
            TradeKind::Cash,
            OrderPrice::Limit(price("112.2"))
        )
    );
    let id = OrderId(900);
    account.ledger.check_forced(id, 0).unwrap();
    assert_eq!(account.ledger.position(tsmc()).cash, -1_000);
    account.fill(id, "103", 1_000);
    // The position shows the cover at once, although what was bought back cannot be
    // sold today.
    assert_eq!(account.ledger.position(tsmc()).cash, 0);
    let close = account.end("103");
    let [realized] = close.realized[..] else {
        panic!("one cover");
    };
    assert_eq!(realized.kind, RealizedKind::SellFirstCover);
    // Bought back for 103,000 + fee 146.
    assert_eq!(realized.pnl, Money::new(99_558 - 103_146));
    assert!(account.ledger.holding(tsmc()).is_none());
    assert_eq!(
        close.equity,
        Money::new(1_000_000 + 99_558 - 7_140 - 103_146)
    );
    assert!(account.begin("2024-03-06", stock("103")).forced.is_empty());
}

#[test]
fn a_margin_position_borrows_and_pays_interest() {
    let mut account = Account::with_cash(100_000);
    account.begin("2024-03-04", stock("100"));
    let order = account.order(Side::Buy, TradeKind::MarginBuy, "100", 1_000);
    let id = account.submit(&order).unwrap();
    // 100,000 + fee 142 less the loan. The loan is rounded down to a thousand on what
    // fills, so a thousand less than the 60,000 at the order price is counted on.
    assert_eq!(account.ledger.available(), Money::new(100_000 - 41_142));
    account.fill(id, "100", 1_000);
    assert_eq!(account.ledger.position(tsmc()).margin, 1_000);
    let close = account.end("100");
    assert_eq!(
        amounts(&close),
        [
            (Item::TradeValue, -100_000),
            (Item::Fee, -142),
            (Item::MarginLoan, 60_000)
        ]
    );
    let maintenance = close.maintenance.unwrap();
    assert_eq!(maintenance.numerator * 60, maintenance.denominator * 100);
    // Shares at 100,000 less the loan.
    assert_eq!(close.equity, Money::new(100_000 - 40_142 + 40_000));
    let lot = account.ledger.holding(tsmc()).unwrap().margin[0];
    assert_eq!(
        (lot.loan, lot.cost),
        (Money::new(60_000), Money::new(100_142))
    );
    // Six months and two extensions.
    assert_eq!(lot.due, day("2025-09-04"));

    for date in ["2024-03-05", "2024-03-06", "2024-03-07", "2024-03-08"] {
        account.idle(date, "100");
    }
    account.begin("2024-03-11", stock("100"));
    // Margin shares are not cash inventory.
    let cash_sale = account.order(Side::Sell, TradeKind::Cash, "110", 1_000);
    assert_eq!(
        account.submit(&cash_sale),
        Err(RejectReason::InsufficientPosition)
    );
    account.trade(Side::Sell, TradeKind::MarginSell, "110", 1_000);
    assert_eq!(account.ledger.position(tsmc()).margin, 0);
    let close = account.end("110");
    // Fee floor(156.75), tax 330; seven days of interest on the loan, from settlement
    // to settlement: round(60,000 x 6.45 % x 7 / 365) = round(74.2).
    assert_eq!(
        amounts(&close),
        [
            (Item::TradeValue, 110_000),
            (Item::Fee, -156),
            (Item::Tax, -330),
            (Item::MarginLoan, -60_000),
            (Item::MarginInterest, -74)
        ]
    );
    let [realized] = close.realized[..] else {
        panic!("one lot closed");
    };
    assert_eq!(realized.kind, RealizedKind::Margin);
    assert_eq!(realized.pnl, Money::new(109_514 - 100_142 - 74));
    assert_eq!(close.equity, Money::new(100_000 + 9_298));
    assert_eq!(close.maintenance, None);
}

#[test]
fn a_short_position_posts_margin_and_earns_interest() {
    let mut account = Account::with_cash(100_000);
    account.begin("2024-03-04", stock("100"));
    let order = account.order(Side::Sell, TradeKind::ShortSell, "100", 1_000);
    let id = account.submit(&order).unwrap();
    // 90 % of the highest price the sale can fill at.
    assert_eq!(account.ledger.available(), Money::new(100_000 - 99_000));
    account.fill(id, "100", 1_000);
    assert_eq!(account.ledger.available(), Money::new(10_000));
    let close = account.end("100");
    assert_eq!(
        amounts(&close),
        [
            (Item::TradeValue, 100_000),
            (Item::Fee, -142),
            (Item::Tax, -300),
            (Item::ShortFee, -80),
            (Item::ShortMargin, -90_000),
            (Item::ShortCollateral, -99_478)
        ]
    );
    // Margin and collateral less the shares at 100,000.
    assert_eq!(close.equity, Money::new(10_000 + 189_478 - 100_000));
    let maintenance = close.maintenance.unwrap();
    assert_eq!(
        maintenance.numerator * 100_000,
        maintenance.denominator * 189_478
    );

    for date in ["2024-03-05", "2024-03-06", "2024-03-07", "2024-03-08"] {
        account.idle(date, "100");
    }
    account.begin("2024-03-11", stock("100"));
    let cover = account.order(Side::Buy, TradeKind::ShortCover, "90", 1_000);
    let id = account.submit(&cover).unwrap();
    // The cover is paid out of what the broker holds.
    assert_eq!(account.ledger.available(), Money::new(10_000));
    account.fill(id, "90", 1_000);
    let close = account.end("90");
    // Fee floor(128.25); interest round(189,478 x 0.2 % x 7 / 365) = round(7.27).
    assert_eq!(
        amounts(&close),
        [
            (Item::TradeValue, -90_000),
            (Item::Fee, -128),
            (Item::ShortMargin, 90_000),
            (Item::ShortCollateral, 99_478),
            (Item::ShortInterest, 7)
        ]
    );
    let [realized] = close.realized[..] else {
        panic!("one lot closed");
    };
    assert_eq!(realized.kind, RealizedKind::Short);
    assert_eq!(realized.pnl, Money::new(99_478 + 7 - 90_128));
    assert_eq!(close.equity, Money::new(100_000 + 9_357));
}

#[test]
fn a_margin_buy_and_a_short_sale_of_one_day_offset() {
    let mut account = Account::with_cash(200_000);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::MarginBuy, "100", 1_000);
    account.trade(Side::Sell, TradeKind::ShortSell, "101", 1_000);
    let close = account.end("100");
    // Sale fee floor(143.925), ordinary tax 303, short fee floor(80.8); buy fee 142.
    assert_eq!(net(&close), 101_000 - 143 - 303 - 80 - 100_142);
    let [realized] = close.realized[..] else {
        panic!("one pair");
    };
    assert_eq!(realized.kind, RealizedKind::CreditOffset);
    assert!(account.ledger.holding(tsmc()).is_none());
    assert_eq!(close.maintenance, None);
}

#[test]
fn an_unanswered_margin_call_sells_the_lots_at_the_limit() {
    let mut account = Account::with_cash(100_000);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::MarginBuy, "100", 2_000);
    // 200,000 against a loan of 120,000: 166.7 %.
    assert!(!account.end("100").call);

    // 155,000 / 120,000 = 129.2 %: the call is made on T.
    account.begin("2024-03-05", stock("100"));
    assert!(account.end("77.5").call);
    for date in ["2024-03-06", "2024-03-07"] {
        let open = account.begin(date, stock("77.5"));
        assert!(open.forced.is_empty(), "{date}");
        account.end("77.5");
    }
    // T+2 closed below the threshold: the lots are sold from T+3.
    let open = account.begin("2024-03-08", stock("77.5"));
    let [forced] = open.forced[..] else {
        panic!("one forced order");
    };
    assert_eq!(forced.reason, ForcedReason::MarginCall);
    assert_eq!(
        (forced.order.side, forced.order.kind, forced.order.qty),
        (Side::Sell, TradeKind::MarginSell, Shares::new(2_000))
    );
    assert_eq!(forced.order.price, OrderPrice::Limit(price("69.75")));
    let id = OrderId(900);
    account.ledger.check_forced(id, 0).unwrap();
    // Half of it fills; the rest is sent again on the next day.
    account.fill(id, "75", 1_000);
    account.event(
        id,
        OrderEventKind::Expired {
            qty: Shares::new(1_000),
        },
    );
    account.end("75");

    let open = account.begin("2024-03-11", stock("75"));
    assert_eq!(open.forced[0].order.qty, Shares::new(1_000));
    account.ledger.check_forced(OrderId(901), 0).unwrap();
    account.fill(OrderId(901), "75", 1_000);
    let close = account.end("75");
    assert_eq!(close.maintenance, None);
    assert!(!close.call);
    assert!(account.begin("2024-03-12", stock("75")).forced.is_empty());
}

#[test]
fn a_call_is_dropped_when_the_account_recovers() {
    let mut account = Account::with_cash(100_000);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::MarginBuy, "100", 2_000);
    account.end("100");
    account.begin("2024-03-05", stock("100"));
    assert!(account.end("77.5").call);
    // Above the call threshold and below the cancel threshold the record stays.
    account.begin("2024-03-06", stock("77.5"));
    assert!(account.end("90").call);
    account.begin("2024-03-07", stock("90"));
    assert!(account.end("90").call);
    // The deadline passed above the threshold: nothing is sold.
    assert!(account.begin("2024-03-08", stock("90")).forced.is_empty());
    // Below it again on any later day, the lots go on the next day.
    assert!(!account.end("77.5").call);
    let open = account.begin("2024-03-11", stock("77.5"));
    assert_eq!(open.forced.len(), 1);
    account.end("100");

    let mut account = Account::with_cash(100_000);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::MarginBuy, "100", 2_000);
    account.end("100");
    account.begin("2024-03-05", stock("100"));
    assert!(account.end("77.5").call);
    account.begin("2024-03-06", stock("77.5"));
    // 199,200 / 120,000 = 166 %.
    assert!(!account.end("99.6").call);
}

#[test]
fn a_forced_order_that_never_fills_ends_the_run() {
    let mut config = Config::new(Money::new(1_000_000));
    config.daytrade.uncovered = Uncovered::ForceCover;
    let mut account = Account::new(config);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Sell, TradeKind::CashSellFirst, "100", 1_000);
    account.end("100");
    let days = [
        "2024-03-05",
        "2024-03-06",
        "2024-03-07",
        "2024-03-08",
        "2024-03-11",
        "2024-03-12",
    ];
    for (index, date) in days.into_iter().enumerate() {
        let open = account.begin(date, stock("100"));
        assert_eq!(open.forced.len(), 1, "{date}");
        let id = OrderId(900 + index as u64);
        account.ledger.check_forced(id, 0).unwrap();
        account.event(
            id,
            OrderEventKind::Expired {
                qty: Shares::new(1_000),
            },
        );
        let result = account.end_result("100");
        if index < 5 {
            result.unwrap();
        } else {
            assert_eq!(
                result,
                Err(LedgerError::ForcedOrderUnfilled {
                    symbol: tsmc(),
                    shares: 1_000
                })
            );
        }
    }
}

#[test]
fn a_short_position_is_covered_after_the_last_day_to_cover() {
    let mut account = Account::with_cash(200_000);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Sell, TradeKind::ShortSell, "100", 1_000);
    account.end("100");
    account.begin(
        "2024-03-05",
        SecurityFacts {
            short_last_cover_day: true,
            short_suspended: true,
            ..stock("100")
        },
    );
    account.end("100");
    let open = account.begin("2024-03-06", stock("100"));
    let [forced] = open.forced[..] else {
        panic!("one forced order");
    };
    assert_eq!(forced.reason, ForcedReason::ShortSuspension);
    assert_eq!(
        (forced.order.side, forced.order.kind, forced.order.price),
        (
            Side::Buy,
            TradeKind::ShortCover,
            OrderPrice::Limit(price("110"))
        )
    );
}

#[test]
fn settlement_without_cash_ends_the_run_when_asked_to() {
    let mut config = Config::new(Money::new(50_000));
    config.account.cash_check = CashCheck::AtSettlement;
    let mut account = Account::new(config);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::Cash, "100", 1_000);
    account.end("100");
    account.idle("2024-03-05", "100");
    assert_eq!(
        account.begin_with("2024-03-06", &[(tsmc(), stock("100"))], &[]),
        Err(LedgerError::SettlementShortfall {
            date: day("2024-03-06"),
            cash: Money::new(-50_142)
        })
    );

    let mut config = Config::new(Money::new(50_000));
    config.account.cash_check = CashCheck::None;
    let mut account = Account::new(config);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::Cash, "100", 1_000);
    account.end("100");
    account.idle("2024-03-05", "100");
    account.idle("2024-03-06", "100");
    assert_eq!(account.ledger.cash(), Money::new(-50_142));
}

#[test]
fn days_are_opened_in_order_and_inside_the_calendar() {
    let mut account = Account::with_cash(1_000);
    assert_eq!(
        account.begin_with("2024-03-09", &[], &[]),
        Err(LedgerError::NotATradingDay {
            date: day("2024-03-09")
        })
    );
    account.idle("2024-03-05", "100");
    assert_eq!(
        account.begin_with("2024-03-04", &[], &[]),
        Err(LedgerError::DayOutOfOrder {
            date: day("2024-03-04")
        })
    );
    // The last trading day has no settlement day after it.
    assert_eq!(
        account.begin_with("2024-03-29", &[], &[]),
        Err(LedgerError::OutsideCalendar {
            needed_after: day("2024-03-29")
        })
    );
}

fn action(kind: CorporateActionKind, before: &str, reference: &str) -> CorporateAction {
    CorporateAction {
        symbol: tsmc(),
        ex_date: day("2024-03-06"),
        kind,
        before_price: price(before),
        ref_price: price(reference),
        after_price: None,
        cash_dividend: None,
        pay_date: None,
        stock_per_1000: None,
        value: None,
        share_ratio: None,
        refund_per_share: None,
    }
}

fn decimal(text: &str) -> Decimal8 {
    // Exact for the few decimals the tests use.
    Decimal8::from_units(price(text).units() * (Decimal8::UNITS_PER_ONE / Price::UNITS_PER_YUAN))
}

fn yuan_per_share(text: &str) -> PerShare {
    PerShare::from_units(price(text).units() * (PerShare::UNITS_PER_YUAN / Price::UNITS_PER_YUAN))
}

/// Buys 1,500 shares at 100 on 2024-03-04 and opens 2024-03-06 with `action`.
fn hold_through(
    config: Config,
    action: CorporateAction,
) -> (Account, Result<DayOpen, LedgerError>) {
    let mut account = Account::new(config);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::Cash, "100", 1_000);
    let odd = NewOrder {
        board: Board::OddLot,
        ..account.order(Side::Buy, TradeKind::Cash, "100", 500)
    };
    let id = account.submit(&odd).unwrap();
    account.fill(id, "100", 500);
    account.end("100");
    account.idle("2024-03-05", "100");
    let reference = action.ref_price.to_string();
    let open = account.begin_with("2024-03-06", &[(tsmc(), stock(&reference))], &[action]);
    (account, open)
}

#[test]
fn a_cash_dividend_is_paid_after_the_lag() {
    let dividend = CorporateAction {
        cash_dividend: Some(yuan_per_share("2.75")),
        ..action(CorporateActionKind::Dividend, "100", "97.25")
    };
    let mut config = Config::new(Money::new(1_000_000));
    config.corporate_actions.cash_pay_lag_days = 5;
    let (mut account, open) = hold_through(config, dividend);
    assert!(open.unwrap().entries.is_empty());
    let close = account.end("97.25");
    // 1,500 x 2.75 = 4,125 is owed to the account from the ex-date.
    assert_eq!(close.receivable, Money::new(4_125));
    let cost = 100_142 + 50_071;
    assert_eq!(close.equity, Money::new(1_000_000 - cost + 145_875 + 4_125));
    account.idle("2024-03-07", "97.25");
    account.idle("2024-03-08", "97.25");
    // 2024-03-11 is the first settlement day five days after the ex-date.
    let open = account.begin("2024-03-11", stock("97.25"));
    let [entry] = open.entries[..] else {
        panic!("one payment");
    };
    assert_eq!(
        (
            entry.item,
            entry.amount,
            entry.date,
            entry.settles,
            entry.approx
        ),
        (
            Item::CashDividend,
            Money::new(4_125),
            day("2024-03-06"),
            day("2024-03-11"),
            true
        )
    );
    assert_eq!(account.ledger.cash(), Money::new(1_000_000 - cost + 4_125));
    account.end("97.25");
    account.assert_cash_is_settled_entries("2024-03-11");
}

#[test]
fn a_cash_dividend_is_paid_on_the_day_marketdata_states() {
    let dividend = CorporateAction {
        cash_dividend: Some(yuan_per_share("2.75")),
        pay_date: Some(day("2024-03-08")),
        ..action(CorporateActionKind::Dividend, "100", "97.25")
    };
    let (mut account, open) = hold_through(Config::new(Money::new(1_000_000)), dividend);
    open.unwrap();
    account.end("97.25");
    account.idle("2024-03-07", "97.25");
    let open = account.begin("2024-03-08", stock("97.25"));
    let [entry] = open.entries[..] else {
        panic!("one payment");
    };
    assert_eq!(
        (entry.item, entry.amount, entry.settles, entry.approx),
        (
            Item::CashDividend,
            Money::new(4_125),
            day("2024-03-08"),
            false
        )
    );
}

#[test]
fn the_ex_date_values_a_position_at_the_price_after_the_event() {
    // A cash offering on the same day takes more off the price than the dividend.
    let dividend = CorporateAction {
        cash_dividend: Some(yuan_per_share("2")),
        after_price: Some(price("96.5")),
        ..action(CorporateActionKind::Dividend, "100", "98")
    };
    let (account, open) = hold_through(Config::new(Money::new(1_000_000)), dividend);
    open.unwrap();
    assert_eq!(account.ledger.valuation(tsmc()), Some(price("96.5")));

    let rights = CorporateAction {
        after_price: Some(price("97")),
        ..action(CorporateActionKind::Rights, "100", "100")
    };
    let (account, open) = hold_through(Config::new(Money::new(1_000_000)), rights);
    assert_eq!(open.unwrap(), DayOpen::default());
    assert_eq!(account.ledger.valuation(tsmc()), Some(price("97")));
}

#[test]
fn a_dividend_without_amounts_needs_them_or_uses_the_value_of_the_ex_date() {
    // marketdata has no amounts for this row, only what the ex-date takes off the price.
    let dividend = CorporateAction {
        value: Some(yuan_per_share("2.75")),
        ..action(CorporateActionKind::Dividend, "100", "97.25")
    };
    let (_, open) = hold_through(Config::new(Money::new(1_000_000)), dividend);
    assert_eq!(
        open.unwrap_err(),
        LedgerError::MissingActionTerms {
            symbol: tsmc(),
            ex_date: day("2024-03-06"),
            action: CorporateActionKind::Dividend,
            missing: "cash_dividend",
        }
    );

    // Approximately: the value is taken as a cash dividend.
    let mut config = Config::new(Money::new(1_000_000));
    config.corporate_actions.missing_terms = MissingTerms::Approximate;
    let (mut account, open) = hold_through(config, dividend);
    open.unwrap();
    assert_eq!(account.end("97.25").receivable, Money::new(4_125));

    // Exactly: the amount is given, and so is the day it is paid.
    let mut config = Config::new(Money::new(1_000_000));
    config.corporate_actions.overrides.push(ActionOverride {
        cash_dividend: Some(yuan_per_share("2.5")),
        cash_date: Some(day("2024-03-08")),
        ..ActionOverride::new(tsmc(), day("2024-03-06"), CorporateActionKind::Dividend)
    });
    let (mut account, open) = hold_through(config, dividend);
    open.unwrap();
    assert_eq!(account.end("97.25").receivable, Money::new(3_750));
    account.idle("2024-03-07", "97.25");
    let open = account.begin("2024-03-08", stock("97.25"));
    let [entry] = open.entries[..] else {
        panic!("one payment");
    };
    assert_eq!(
        (entry.item, entry.amount, entry.approx),
        (Item::CashDividend, Money::new(3_750), false)
    );

    // A row on which the exchange did not lower the price pays nothing.
    let unchanged = CorporateAction {
        value: Some(yuan_per_share("0.39")),
        ..action(CorporateActionKind::Dividend, "100", "100")
    };
    let (mut account, open) = hold_through(Config::new(Money::new(1_000_000)), unchanged);
    open.unwrap();
    assert_eq!(account.end("100").receivable, Money::ZERO);
}

#[test]
fn a_stock_dividend_delivers_free_shares_and_cash_for_the_fraction() {
    let dividend = CorporateAction {
        cash_dividend: Some(yuan_per_share("20")),
        stock_per_1000: Some(decimal("75")),
        ..action(CorporateActionKind::Dividend, "100", "74.4")
    };
    let mut config = Config::new(Money::new(1_000_000));
    config.account.nhi_enabled = true;
    config.corporate_actions.cash_pay_lag_days = 2;
    config.corporate_actions.stock_credit_lag_days = 5;
    let (mut account, open) = hold_through(config, dividend);
    open.unwrap();
    // 1,500 x 0.075 = 112.5 shares: 112 are delivered, half a share is paid at par.
    let close = account.end("74.4");
    // Cash 30,000, 5 yuan for the fraction, less the premium on 30,000 + 1,125:
    // round(31,125 x 2.11 %) = round(656.7).
    assert_eq!(close.receivable, Money::new(30_000 + 5 - 657));
    let cost = 100_142 + 50_071;
    // The shares to come count at the valuation price: floor(112 x 74.4).
    assert_eq!(
        close.equity,
        Money::new(1_000_000 - cost + 111_600 + 8_332 + 30_000 + 5 - 657)
    );
    account.idle("2024-03-07", "74.4");
    let open = account.begin("2024-03-08", stock("74.4"));
    let paid: Vec<(Item, i64)> = open
        .entries
        .iter()
        .map(|entry| (entry.item, entry.amount.get()))
        .collect();
    assert_eq!(
        paid,
        [(Item::CashDividend, 30_000), (Item::NhiPremium, -657)]
    );
    assert_eq!(account.ledger.position(tsmc()).cash, 1_500);
    account.end("74.4");
    let open = account.begin("2024-03-11", stock("74.4"));
    assert_eq!(open.entries[0].item, Item::FractionCash);
    assert_eq!(open.entries[0].amount, Money::new(5));
    let holding = account.ledger.holding(tsmc()).unwrap();
    assert_eq!(holding.cash_shares(), 1_612);
    let free = holding.cash.last().unwrap();
    assert_eq!((free.shares, free.cost), (112, Money::ZERO));
    let close = account.end("74.4");
    assert_eq!(close.receivable, Money::ZERO);
}

#[test]
fn a_share_with_another_par_value_needs_it_only_for_a_fraction_or_an_amount_at_par() {
    let odd_par = SecurityFacts {
        non_10_par: Some(true),
        ..stock("90.9")
    };
    // Holds 1,000 shares and opens the ex-date; returns the shares a day later.
    let receive = |config: Config, dividend: CorporateAction| {
        let mut account = Account::new(config);
        account.begin("2024-03-04", stock("100"));
        account.trade(Side::Buy, TradeKind::Cash, "100", 1_000);
        account.end("100");
        account.idle("2024-03-05", "100");
        account.begin_with("2024-03-06", &[(tsmc(), odd_par)], &[dividend])?;
        account.end("90.9");
        account.begin("2024-03-07", stock("90.9"));
        Ok::<_, LedgerError>(account.ledger.position(tsmc()).cash)
    };
    let mut on_the_7th = Config::new(Money::new(1_000_000));
    on_the_7th.corporate_actions.stock_credit_lag_days = 1;
    let per_1000 = |text: &str| CorporateAction {
        stock_per_1000: Some(decimal(text)),
        ..action(CorporateActionKind::Dividend, "100", "90.9")
    };
    // Whole shares need no par value.
    assert_eq!(receive(on_the_7th.clone(), per_1000("100")), Ok(1_100));
    // Half a share is paid at par.
    let missing = Err(LedgerError::MissingActionTerms {
        symbol: tsmc(),
        ex_date: day("2024-03-06"),
        action: CorporateActionKind::Dividend,
        missing: "par_value",
    });
    assert_eq!(receive(on_the_7th.clone(), per_1000("100.5")), missing);

    // An amount in yuan at par needs the par value: at 5 yuan, 0.5 yuan is a tenth of
    // a share.
    let mut config = on_the_7th.clone();
    config.corporate_actions.overrides.push(ActionOverride {
        stock_dividend: Some(yuan_per_share("0.5")),
        ..ActionOverride::new(tsmc(), day("2024-03-06"), CorporateActionKind::Dividend)
    });
    assert_eq!(receive(config.clone(), per_1000("100")), missing);
    config.corporate_actions.overrides[0].par_value = Some(yuan_per_share("5"));
    assert_eq!(receive(config, per_1000("100")), Ok(1_100));
}

#[test]
fn a_split_multiplies_the_shares_and_keeps_the_cost() {
    let (mut account, open) = hold_through(
        Config::new(Money::new(1_000_000)),
        action(CorporateActionKind::Split, "100", "25"),
    );
    open.unwrap();
    let holding = account.ledger.holding(tsmc()).unwrap();
    let lots: Vec<(i64, i64)> = holding
        .cash
        .iter()
        .map(|lot| (lot.shares, lot.cost.get()))
        .collect();
    assert_eq!(lots, [(4_000, 100_142), (2_000, 50_071)]);
    let close = account.end("25");
    assert_eq!(close.equity, Money::new(1_000_000 - 150_213 + 150_000));
    assert_eq!(close.receivable, Money::ZERO);
}

#[test]
fn a_reverse_split_pays_cash_for_what_is_not_a_whole_share() {
    // Seven old shares become one.
    let (mut account, open) = hold_through(
        Config::new(Money::new(1_000_000)),
        action(CorporateActionKind::ReverseSplit, "100", "700"),
    );
    open.unwrap();
    let holding = account.ledger.holding(tsmc()).unwrap();
    let lots: Vec<(i64, i64)> = holding
        .cash
        .iter()
        .map(|lot| (lot.shares, lot.cost.get()))
        .collect();
    // 1,500 / 7 = 214 and 2/7; the lots share the rounding: 142 and 72.
    assert_eq!(lots, [(142, 100_142), (72, 50_071)]);
    let close = account.end("700");
    // Two sevenths of a share at 700.
    assert_eq!(close.receivable, Money::new(200));
    assert_eq!(
        close.equity,
        Money::new(1_000_000 - 150_213 + 214 * 700 + 200)
    );
}

#[test]
fn marketdata_states_the_ratio_and_the_refund() {
    // 1 share becomes 2.5 at a change of par value.
    let change = CorporateAction {
        share_ratio: Some((decimal("2.5"), decimal("1"))),
        ..action(CorporateActionKind::ParChange, "250", "100")
    };
    let (account, open) = hold_through(Config::new(Money::new(1_000_000)), change);
    open.unwrap();
    assert_eq!(account.ledger.position(tsmc()).cash, 3_750);

    // 1,000 shares become 800 and 2 yuan per share is returned.
    let reduction = CorporateAction {
        share_ratio: Some((decimal("800"), decimal("1000"))),
        refund_per_share: Some(yuan_per_share("2")),
        ..action(CorporateActionKind::ReductionCash, "100", "122.5")
    };
    let mut config = Config::new(Money::new(1_000_000));
    config.corporate_actions.cash_pay_lag_days = 2;
    let (mut account, open) = hold_through(config, reduction);
    open.unwrap();
    assert_eq!(account.ledger.position(tsmc()).cash, 1_200);
    account.end("122.5");
    account.idle("2024-03-07", "122.5");
    let open = account.begin("2024-03-08", stock("122.5"));
    let [entry] = open.entries[..] else {
        panic!("one refund");
    };
    // Only the day of the refund is not known.
    assert_eq!(
        (entry.item, entry.amount, entry.approx),
        (Item::CapitalRefund, Money::new(3_000), true)
    );
}

#[test]
fn a_ratio_that_the_prices_do_not_show_must_be_given() {
    let change = action(CorporateActionKind::ParChange, "250", "100");
    let (_, open) = hold_through(Config::new(Money::new(1_000_000)), change);
    assert_eq!(
        open,
        Err(LedgerError::MissingActionTerms {
            symbol: tsmc(),
            ex_date: day("2024-03-06"),
            action: CorporateActionKind::ParChange,
            missing: "share_ratio"
        })
    );

    let mut config = Config::new(Money::new(1_000_000));
    config.corporate_actions.overrides.push(ActionOverride {
        share_ratio: Some((5, 2)),
        ..ActionOverride::new(tsmc(), day("2024-03-06"), CorporateActionKind::ParChange)
    });
    let (account, open) = hold_through(config, change);
    open.unwrap();
    assert_eq!(account.ledger.position(tsmc()).cash, 3_750);
}

#[test]
fn a_capital_reduction_needs_its_terms_or_approximates_them() {
    // A fifth of the capital is returned: 2 yuan per share, and 5 shares become 4.
    // The reference price is (100 - 2) / 0.8.
    let reduction = action(CorporateActionKind::ReductionCash, "100", "122.5");
    let (_, open) = hold_through(Config::new(Money::new(1_000_000)), reduction);
    assert_eq!(
        open,
        Err(LedgerError::MissingActionTerms {
            symbol: tsmc(),
            ex_date: day("2024-03-06"),
            action: CorporateActionKind::ReductionCash,
            missing: "share_ratio"
        })
    );

    let mut exact = Config::new(Money::new(1_000_000));
    exact.corporate_actions.overrides.push(ActionOverride {
        share_ratio: Some((4, 5)),
        refund_per_share: Some(yuan_per_share("2")),
        cash_date: Some(day("2024-03-08")),
        ..ActionOverride::new(
            tsmc(),
            day("2024-03-06"),
            CorporateActionKind::ReductionCash,
        )
    });
    let mut approximate = Config::new(Money::new(1_000_000));
    approximate.corporate_actions.missing_terms = MissingTerms::Approximate;
    approximate.corporate_actions.cash_pay_lag_days = 2;
    for (config, approx) in [(exact, false), (approximate, true)] {
        let (mut account, open) = hold_through(config, reduction);
        assert!(open.unwrap().realized.is_empty());
        let holding = account.ledger.holding(tsmc()).unwrap();
        let lots: Vec<(i64, i64)> = holding
            .cash
            .iter()
            .map(|lot| (lot.shares, lot.cost.get()))
            .collect();
        // 3,000 is returned and taken off the cost of the lots.
        assert_eq!(lots, [(800, 98_142), (400, 49_071)]);
        let close = account.end("122.5");
        assert_eq!(close.receivable, Money::new(3_000));
        assert_eq!(
            close.equity,
            Money::new(1_000_000 - 150_213 + 147_000 + 3_000)
        );
        account.idle("2024-03-07", "122.5");
        let open = account.begin("2024-03-08", stock("122.5"));
        let [entry] = open.entries[..] else {
            panic!("one refund");
        };
        assert_eq!(
            (entry.item, entry.amount, entry.approx),
            (Item::CapitalRefund, Money::new(3_000), approx)
        );
    }
}

#[test]
fn a_reduction_to_cover_losses_is_approximated_from_the_prices() {
    let reduction = action(CorporateActionKind::ReductionLoss, "8", "20");
    let mut config = Config::new(Money::new(1_000_000));
    config.corporate_actions.missing_terms = MissingTerms::Approximate;
    let (account, open) = hold_through(config, reduction);
    open.unwrap();
    // 8 / 20 of 1,500.
    assert_eq!(account.ledger.position(tsmc()).cash, 600);
}

#[test]
fn borrowed_shares_pay_the_cash_dividend_and_stop_other_actions() {
    let short_through = |action: CorporateAction| {
        let mut account = Account::with_cash(1_000_000);
        account.begin("2024-03-04", stock("100"));
        account.trade(Side::Sell, TradeKind::ShortSell, "100", 1_000);
        account.end("100");
        account.idle("2024-03-05", "100");
        let open = account.begin_with("2024-03-06", &[(tsmc(), stock("100"))], &[action]);
        (account, open)
    };
    let dividend = CorporateAction {
        cash_dividend: Some(yuan_per_share("1.5")),
        ..action(CorporateActionKind::Dividend, "100", "98.5")
    };
    let (mut account, open) = short_through(dividend);
    open.unwrap();
    assert_eq!(account.end("98.5").receivable, Money::new(-1_500));

    let (_, open) = short_through(action(CorporateActionKind::Split, "100", "50"));
    assert_eq!(
        open,
        Err(LedgerError::ShortAcrossExDate {
            symbol: tsmc(),
            ex_date: day("2024-03-06")
        })
    );
}

#[test]
fn actions_without_a_position_and_rights_issues_book_nothing() {
    let mut account = Account::with_cash(1_000);
    let open = account
        .begin_with(
            "2024-03-06",
            &[(tsmc(), stock("100"))],
            &[action(CorporateActionKind::ReductionCash, "100", "122.5")],
        )
        .unwrap();
    assert_eq!(open, DayOpen::default());

    let (account, open) = hold_through(
        Config::new(Money::new(1_000_000)),
        action(CorporateActionKind::Rights, "100", "95"),
    );
    assert_eq!(open.unwrap(), DayOpen::default());
    assert_eq!(account.ledger.position(tsmc()).cash, 1_500);
}

#[test]
fn a_held_instrument_that_disappears_keeps_its_last_price() {
    let mut account = Account::with_cash(1_000_000);
    account.begin("2024-03-04", stock("100"));
    account.trade(Side::Buy, TradeKind::Cash, "100", 1_000);
    account.end("100");
    account.begin_with("2024-03-05", &[], &[]).unwrap();
    let close = account.ledger.end_day(&HashMap::new()).unwrap();
    assert_eq!(
        close.diagnostics,
        [osmium_ledger::Diagnostic::StalePosition { symbol: tsmc() }]
    );
    assert_eq!(close.equity, Money::new(1_000_000 - 100_142 + 100_000));
}

#[test]
fn without_a_close_the_valuation_follows_the_closing_quotes() {
    let mut account = Account::with_cash(1_000_000);
    for (date, bid, ask, expected) in [
        ("2024-03-04", Some("101"), Some("102"), "101"),
        ("2024-03-05", Some("98"), Some("99"), "99"),
        ("2024-03-06", Some("99"), Some("101"), "100"),
        ("2024-03-07", None, None, "100"),
    ] {
        account.begin(date, stock("100"));
        if date == "2024-03-04" {
            account.trade(Side::Buy, TradeKind::Cash, "100", 1_000);
        }
        let quotes = HashMap::from([(
            tsmc(),
            DailyQuote {
                symbol: tsmc(),
                close: None,
                close_bid: bid.map(price),
                close_ask: ask.map(price),
                ref_price: None,
                limit_down: None,
            },
        )]);
        account.ledger.end_day(&quotes).unwrap();
        assert_eq!(
            account.ledger.valuation(tsmc()),
            Some(price(expected)),
            "{date}"
        );
    }
}
