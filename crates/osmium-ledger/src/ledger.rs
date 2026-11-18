use std::collections::{BTreeMap, HashMap};

use osmium_types::{
    Board, DailyQuote, Date, Money, NewOrder, OrderEvent, OrderEventKind, OrderId, OrderPrice,
    Position, Price, RejectReason, Shares, Side, Symbol, TimeInForce, TradeKind,
};

use crate::{
    Calendar, CashCheck, Config, DayFacts, Entry, Holding, Item, LedgerError, LotId, Prepay,
    Realized, SecurityFacts, Uncovered,
    amount::{Rate, Rounding, divide, notional, notional_units, share_of, trade_value},
    calendar::add_months,
    close::{CloseInput, CloseOutput, DayFill, DayOrder, Purpose, close, short_margin_rate},
    tax,
};

/// Why the ledger sends an order of its own (`docs/reference/accounting.md`
/// section 12.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForcedReason {
    /// A margin call was not answered.
    MarginCall,
    /// The lot reached the end of its term.
    Expired,
    /// A short position was open at the close of the last day to cover.
    ShortSuspension,
    /// Shares sold without inventory were not bought back.
    UncoveredSellFirst,
}

impl ForcedReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MarginCall => "MarginCall",
            Self::Expired => "Expired",
            Self::ShortSuspension => "ShortSuspension",
            Self::UncoveredSellFirst => "UncoveredSellFirst",
        }
    }
}

/// An order the ledger wants sent before the open. The caller sends it to the
/// exchange and answers the account check with [`Ledger::check_forced`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForcedOrder {
    pub order: NewOrder,
    pub reason: ForcedReason,
}

/// Something the caller should report but that does not stop the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diagnostic {
    /// A held instrument is not in today's instruments; it keeps its last price.
    StalePosition { symbol: Symbol },
}

/// What happened before the open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DayOpen {
    /// Corporate-action cash that reached the account since the last trading day.
    pub entries: Vec<Entry>,
    pub realized: Vec<Realized>,
    pub forced: Vec<ForcedOrder>,
    pub diagnostics: Vec<Diagnostic>,
}

/// The account maintenance ratio as an exact fraction of amounts in price units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Maintenance {
    pub numerator: i128,
    pub denominator: i128,
}

impl Maintenance {
    fn is_below(self, threshold: Rate) -> bool {
        self.numerator * i128::from(threshold.denominator())
            < i128::from(threshold.numerator()) * self.denominator
    }
}

/// The day-end figures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayClose {
    pub date: Date,
    /// The settlement date of the day's trades.
    pub settles: Date,
    pub entries: Vec<Entry>,
    pub realized: Vec<Realized>,
    pub cash: Money,
    /// Everything that has not settled yet.
    pub pending: Money,
    /// Dividends and refunds declared and not yet paid.
    pub receivable: Money,
    pub equity: Money,
    /// `None` without margin or short positions.
    pub maintenance: Option<Maintenance>,
    /// A margin call is outstanding.
    pub call: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Receivable {
    pub not_before: Date,
    pub entry: Entry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Delivery {
    pub not_before: Date,
    pub symbol: Symbol,
    pub shares: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Call {
    deadline: Option<Date>,
    lots: Vec<LotId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ForcedLot {
    reason: ForcedReason,
    /// Trading days on which an order for the lot was sent and not filled.
    attempts: u32,
}

/// What a forced order of today closes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ForcedPlan {
    order: ForcedOrder,
    purpose: Purpose,
    /// The order it was sent as.
    id: Option<OrderId>,
}

/// The fills and working orders of one instrument today.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Activity {
    fills: Vec<DayFill>,
    /// What today's fills add to the settlement of the day, so far.
    net: Money,
    /// Regular-board cash buys filled today.
    cash_bought: i64,
    /// Regular-board cash sells filled today, with or without inventory.
    cash_sold: i64,
    /// Cash buys and sells filled on the odd-lot board.
    odd_bought: i64,
    odd_sold: i64,
    /// Cash sells from inventory still at the exchange.
    cash_selling: i64,
    /// Margin sales, filled and working.
    margin_selling: i64,
    /// Short covers, filled and working.
    short_covering: i64,
    margin_bought: i64,
    short_sold: i64,
    /// Sells under prepayment, filled and working.
    prepaid_selling: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Day {
    facts: DayFacts,
    settles: Date,
    due: Date,
    orders: BTreeMap<OrderId, DayOrder>,
    activity: BTreeMap<Symbol, Activity>,
    /// The sum of every activity's `net`.
    net: Money,
    /// Cash set aside for working orders.
    reserved: Money,
    /// Settled cash committed to buys under prepayment.
    prepaid: Money,
    /// Trade value filled today, buys and sells.
    traded: Money,
    /// The value of working orders at their prices.
    working: Money,
    forced: Vec<ForcedPlan>,
}

/// The lowest and highest prices an order can carry when the instrument has no limit.
const LOWEST_PRICE: Price = Price::from_units(Price::UNITS_PER_YUAN / 100);
const HIGHEST_PRICE: Price = Price::from_units(9_995 * Price::UNITS_PER_YUAN);

/// A forced order is retried on this many consecutive trading days.
const FORCED_ATTEMPTS: u32 = 6;

/// The board lot of an instrument whose lot size marketdata does not have.
const DEFAULT_LOT: u32 = 1_000;

/// One account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    pub(crate) config: Config,
    calendar: Calendar,
    pub(crate) cash: Money,
    pending: BTreeMap<Date, Money>,
    pub(crate) holdings: BTreeMap<Symbol, Holding>,
    pub(crate) receivables: Vec<Receivable>,
    pub(crate) deliveries: Vec<Delivery>,
    pub(crate) valuation: BTreeMap<Symbol, Price>,
    next_lot: u64,
    last_day: Option<Date>,
    last_settled: Option<Date>,
    call: Option<Call>,
    forced_lots: BTreeMap<LotId, ForcedLot>,
    forced_owed: BTreeMap<Symbol, u32>,
    day: Option<Day>,
}

impl Ledger {
    #[must_use]
    pub fn new(config: Config, calendar: Calendar) -> Self {
        Self {
            cash: config.account.initial_cash,
            config,
            calendar,
            pending: BTreeMap::new(),
            holdings: BTreeMap::new(),
            receivables: Vec::new(),
            deliveries: Vec::new(),
            valuation: BTreeMap::new(),
            next_lot: 0,
            last_day: None,
            last_settled: None,
            call: None,
            forced_lots: BTreeMap::new(),
            forced_owed: BTreeMap::new(),
            day: None,
        }
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Settled cash.
    #[must_use]
    pub const fn cash(&self) -> Money {
        self.cash
    }

    /// Everything that has not settled yet, without today's fills.
    #[must_use]
    pub fn pending(&self) -> Money {
        self.pending.values().copied().sum()
    }

    /// What a new order may still commit: cash, pending settlements and today's fills
    /// so far, less what working orders have set aside (section 7.3).
    #[must_use]
    pub fn available(&self) -> Money {
        let (net, reserved) = self
            .day
            .as_ref()
            .map_or((Money::ZERO, Money::ZERO), |day| (day.net, day.reserved));
        self.cash + self.pending() + net - reserved
    }

    /// The lots of an instrument as of the start of the day.
    #[must_use]
    pub fn holding(&self, symbol: Symbol) -> Option<&Holding> {
        self.holdings.get(&symbol)
    }

    /// Every instrument with lots, as of the start of the day.
    pub fn holdings(&self) -> impl Iterator<Item = (Symbol, &Holding)> {
        self.holdings
            .iter()
            .map(|(&symbol, holding)| (symbol, holding))
    }

    /// Share counts including today's fills.
    #[must_use]
    pub fn position(&self, symbol: Symbol) -> Position {
        let mut position = self
            .holdings
            .get(&symbol)
            .map_or_else(Position::default, |holding| Position {
                cash: holding.cash_shares() - holding.owed_shares(),
                margin: holding.margin_shares(),
                short: holding.short_shares(),
            });
        if let Some(activity) = self.day.as_ref().and_then(|day| day.activity.get(&symbol)) {
            let orders = &self.day.as_ref().expect("checked above").orders;
            let filled = |of: &dyn Fn(&DayOrder) -> bool| -> i64 {
                activity
                    .fills
                    .iter()
                    .filter(|fill| of(&orders[&fill.order]))
                    .map(|fill| fill.shares)
                    .sum()
            };
            // Shares bought back for a forced cover are not inventory to sell today,
            // so the activity does not count them as bought.
            position.cash +=
                activity.cash_bought + activity.odd_bought - activity.cash_sold - activity.odd_sold
                    + filled(&|order| order.purpose == Purpose::CoverOwed);
            position.margin +=
                activity.margin_bought - filled(&|order| order.order.kind == TradeKind::MarginSell);
            position.short +=
                activity.short_sold - filled(&|order| order.order.kind == TradeKind::ShortCover);
        }
        position
    }

    /// The price positions in `symbol` were last valued at.
    #[must_use]
    pub fn valuation(&self, symbol: Symbol) -> Option<Price> {
        self.valuation.get(&symbol).copied()
    }

    // ---- before the open ---------------------------------------------------------

    /// Opens a trading day: settles what is due, applies the corporate actions of the
    /// day and returns the orders the ledger wants sent before the open (section 3).
    pub fn begin_day(&mut self, facts: DayFacts) -> Result<DayOpen, LedgerError> {
        let date = facts.date;
        if self.day.is_some() || self.last_day.is_some_and(|last| last >= date) {
            return Err(LedgerError::DayOutOfOrder { date });
        }
        if !self.calendar.is_trading_day(date) {
            return Err(LedgerError::NotATradingDay { date });
        }
        let mut open = DayOpen::default();
        self.settle_through(date, &mut open)?;
        for action in &facts.actions {
            self.apply_action(action, facts.securities.get(&action.symbol), &mut open)?;
        }
        self.holdings.retain(|_, holding| !holding.is_empty());
        let settles = self.calendar.settlement_date(date)?;
        let months = self.config.credit.term_months * (1 + self.config.credit.max_extensions);
        let forced = self.plan_forced(&facts);
        open.forced = forced.iter().map(|plan| plan.order).collect();
        self.day = Some(Day {
            facts,
            settles,
            due: add_months(date, months),
            orders: BTreeMap::new(),
            activity: BTreeMap::new(),
            net: Money::ZERO,
            reserved: Money::ZERO,
            prepaid: Money::ZERO,
            traded: Money::ZERO,
            working: Money::ZERO,
            forced,
        });
        Ok(open)
    }

    /// Moves everything due on or before `date` into cash, one settlement day at a
    /// time.
    fn settle_through(&mut self, date: Date, open: &mut DayOpen) -> Result<(), LedgerError> {
        let days: Vec<Date> = self
            .calendar
            .settlement_days(self.last_settled, date)
            .to_vec();
        for day in days {
            if let Some(amount) = self.pending.remove(&day) {
                self.cash += amount;
            }
            let (due, later): (Vec<Receivable>, Vec<Receivable>) =
                std::mem::take(&mut self.receivables)
                    .into_iter()
                    .partition(|receivable| receivable.not_before <= day);
            self.receivables = later;
            for receivable in due {
                self.cash += receivable.entry.amount;
                open.entries.push(Entry {
                    settles: day,
                    ..receivable.entry
                });
            }
            let (due, later): (Vec<Delivery>, Vec<Delivery>) = std::mem::take(&mut self.deliveries)
                .into_iter()
                .partition(|delivery| delivery.not_before <= day);
            self.deliveries = later;
            for delivery in due {
                self.holdings
                    .entry(delivery.symbol)
                    .or_default()
                    .cash
                    .push(crate::CashLot {
                        shares: delivery.shares,
                        cost: Money::ZERO,
                        acquired: day,
                        settles: day,
                    });
            }
            if self.config.account.cash_check == CashCheck::AtSettlement && self.cash.is_negative()
            {
                return Err(LedgerError::SettlementShortfall {
                    date: day,
                    cash: self.cash,
                });
            }
        }
        self.last_settled = Some(date);
        Ok(())
    }

    /// The orders for everything that must be closed today: whole lots on the regular
    /// board and what is left of a lot on the odd-lot board.
    fn plan_forced(&self, facts: &DayFacts) -> Vec<ForcedPlan> {
        let mut plans = Vec::new();
        let mut plan = |symbol: Symbol,
                        side: Side,
                        kind: TradeKind,
                        shares: i64,
                        reason: ForcedReason,
                        purpose: &Purpose| {
            let Some(security) = facts.securities.get(&symbol) else {
                return;
            };
            let lot = i64::from(security.shares_per_lot.unwrap_or(DEFAULT_LOT));
            let price = match side {
                Side::Buy => security.limit_up.unwrap_or(HIGHEST_PRICE),
                Side::Sell => security.limit_down.unwrap_or(LOWEST_PRICE),
            };
            for (board, qty) in [
                (Board::Regular, shares / lot * lot),
                (Board::OddLot, shares % lot),
            ] {
                if qty > 0 {
                    plans.push(ForcedPlan {
                        order: ForcedOrder {
                            order: NewOrder {
                                symbol,
                                board,
                                side,
                                kind,
                                price: OrderPrice::Limit(price),
                                time_in_force: TimeInForce::Rod,
                                qty: Shares::new(qty),
                            },
                            reason,
                        },
                        purpose: purpose.clone(),
                        id: None,
                    });
                }
            }
        };
        for (&symbol, holding) in &self.holdings {
            let margin: Vec<(LotId, i64)> = holding
                .margin
                .iter()
                .filter(|lot| self.forced_lots.contains_key(&lot.id))
                .map(|lot| (lot.id, lot.shares))
                .collect();
            let short: Vec<(LotId, i64)> = holding
                .short
                .iter()
                .filter(|lot| self.forced_lots.contains_key(&lot.id))
                .map(|lot| (lot.id, lot.shares))
                .collect();
            for (lots, side, kind) in [
                (margin, Side::Sell, TradeKind::MarginSell),
                (short, Side::Buy, TradeKind::ShortCover),
            ] {
                if let Some(&(first, _)) = lots.first() {
                    plan(
                        symbol,
                        side,
                        kind,
                        lots.iter().map(|&(_, shares)| shares).sum(),
                        self.forced_lots[&first].reason,
                        &Purpose::CloseLots(lots.iter().map(|&(id, _)| id).collect()),
                    );
                }
            }
            if self.forced_owed.contains_key(&symbol) && holding.owed_shares() > 0 {
                plan(
                    symbol,
                    Side::Buy,
                    TradeKind::Cash,
                    holding.owed_shares(),
                    ForcedReason::UncoveredSellFirst,
                    &Purpose::CoverOwed,
                );
            }
        }
        plans
    }

    // ---- during the day ----------------------------------------------------------

    fn day(&self) -> &Day {
        self.day.as_ref().expect("a trading day is open")
    }

    /// The account checks for a new order of the strategy (section 4). On `Ok` the
    /// order is taken as accepted: what it needs is set aside until its events
    /// release it.
    pub fn check_new(&mut self, id: OrderId, order: &NewOrder) -> Result<(), RejectReason> {
        let day = self.day();
        let facts = *day
            .facts
            .securities
            .get(&order.symbol)
            .ok_or(RejectReason::UnsupportedInstrument)?;
        let empty = Holding::default();
        let holding = self.holdings.get(&order.symbol).unwrap_or(&empty);
        let default = Activity::default();
        let activity = day.activity.get(&order.symbol).unwrap_or(&default);
        let account = &self.config.account;
        let shares = order.qty.get();

        // 1. The kind fits the side, the board and the instrument.
        let fits = order.kind.side().is_none_or(|side| side == order.side)
            && (order.board == Board::Regular || order.kind == TradeKind::Cash)
            && match order.kind {
                TradeKind::MarginBuy => facts.margin_eligible,
                TradeKind::ShortSell => facts.short_eligible,
                _ => true,
            };
        if !fits {
            return Err(RejectReason::KindNotAllowed);
        }

        // 2. There is something to sell or to close.
        let day_trading = account.daytrade_enabled && facts.day_trade.buy_first;
        let enough = match (order.kind, order.side) {
            (TradeKind::Cash, Side::Sell) => {
                let bought_today = if order.board == Board::Regular && day_trading {
                    activity.cash_bought
                } else {
                    0
                };
                holding.cash_shares() + bought_today
                    - activity.cash_sold
                    - activity.odd_sold
                    - activity.cash_selling
                    >= shares
            }
            (TradeKind::MarginSell, _) => {
                holding.margin_shares() - activity.margin_selling >= shares
            }
            (TradeKind::ShortCover, _) => {
                holding.short_shares() - activity.short_covering >= shares
            }
            _ => true,
        };
        if !enough {
            return Err(RejectReason::InsufficientPosition);
        }

        // 3. Selling first.
        if order.kind == TradeKind::CashSellFirst
            && !(account.daytrade_enabled && facts.day_trade.sell_first)
        {
            return Err(RejectReason::DayTradeNotAllowed);
        }

        // 4. Selling short.
        if order.kind == TradeKind::ShortSell {
            if facts.short_suspended {
                return Err(RejectReason::ShortNotAllowed);
            }
            let below = match (order.price, facts.ref_price) {
                (OrderPrice::Market, _) => true,
                (OrderPrice::Limit(price), Some(reference)) => price < reference,
                (OrderPrice::Limit(_), None) => true,
            };
            if facts.short_price_floor && below {
                return Err(RejectReason::ShortPriceRestricted);
            }
        }

        // 5. Prepayment during a disposition.
        let prepaid = facts.disposition
            && self.config.disposition_prepay == Prepay::All
            && matches!(order.kind, TradeKind::Cash | TradeKind::CashSellFirst);
        if prepaid {
            let covered = match order.side {
                Side::Buy => prepay_hold(&self.config, order, &facts, order.price, shares)
                    .is_some_and(|hold| self.cash - day.prepaid >= hold),
                Side::Sell => {
                    holding.settled_cash_shares(day.facts.date) - activity.prepaid_selling >= shares
                }
            };
            if !covered {
                return Err(RejectReason::PrepaymentRequired);
            }
        }

        // 6. Cash.
        let working = Working {
            order,
            price: order.price,
            shares,
            filled: Money::ZERO,
        };
        let reserve = reservation(&self.config, &facts, holding, day, &working);
        if account.cash_check == CashCheck::AtOrder
            && reserve.is_none_or(|reserve| self.available() < reserve)
        {
            return Err(RejectReason::InsufficientCash);
        }

        // 7. The daily limit.
        let value = order_value(order, &facts, order.price, shares);
        if let Some(limit) = account.daily_limit
            && value.is_none_or(|value| day.traded + day.working + value > limit)
        {
            return Err(RejectReason::DailyLimitExceeded);
        }

        self.register(id, order, Purpose::Strategy, prepaid);
        Ok(())
    }

    /// The account's answer to forced order number `index` of today's
    /// [`DayOpen::forced`]: always accepted, and not subject to the cash checks.
    ///
    /// # Panics
    ///
    /// When `index` is not an order of today.
    pub fn check_forced(&mut self, id: OrderId, index: usize) -> Result<(), RejectReason> {
        let plan = self.day().forced[index].clone();
        assert!(plan.id.is_none(), "forced order {index} was sent before");
        self.register(id, &plan.order.order, plan.purpose, false);
        self.day.as_mut().expect("a trading day is open").forced[index].id = Some(id);
        Ok(())
    }

    fn register(&mut self, id: OrderId, order: &NewOrder, purpose: Purpose, prepaid: bool) {
        let empty = Holding::default();
        let holding = self.holdings.get(&order.symbol).unwrap_or(&empty);
        let day = self.day.as_ref().expect("a trading day is open");
        let facts = day.facts.securities[&order.symbol];
        let shares = order.qty.get();
        let reserved = if purpose == Purpose::Strategy {
            let working = Working {
                order,
                price: order.price,
                shares,
                filled: Money::ZERO,
            };
            reservation(&self.config, &facts, holding, day, &working).unwrap_or(Money::ZERO)
        } else {
            Money::ZERO
        };
        let hold = if prepaid && order.side == Side::Buy {
            prepay_hold(&self.config, order, &facts, order.price, shares).unwrap_or(Money::ZERO)
        } else {
            Money::ZERO
        };
        let value = order_value(order, &facts, order.price, shares).unwrap_or(Money::ZERO);
        let day = self.day.as_mut().expect("a trading day is open");
        day.reserved += reserved;
        day.prepaid += hold;
        day.working += value;
        let activity = day.activity.entry(order.symbol).or_default();
        match (order.kind, order.side) {
            (TradeKind::Cash, Side::Sell) => activity.cash_selling += shares,
            (TradeKind::MarginSell, _) => activity.margin_selling += shares,
            (TradeKind::ShortCover, _) => activity.short_covering += shares,
            _ => {}
        }
        if prepaid && order.side == Side::Sell {
            activity.prepaid_selling += shares;
        }
        let previous = day.orders.insert(
            id,
            DayOrder {
                order: *order,
                price: order.price,
                working: shares,
                purpose,
                fills: Vec::new(),
                reserved,
                prepaid,
            },
        );
        assert!(previous.is_none(), "{id:?} was used before");
    }

    /// The account check for moving a working order to `price`. On `Ok` the order is
    /// taken as repriced.
    pub fn check_reprice(&mut self, id: OrderId, price: Price) -> Result<(), RejectReason> {
        let day = self.day();
        let order = day.orders.get(&id).ok_or(RejectReason::UnknownOrder)?;
        let facts = day.facts.securities[&order.order.symbol];
        let empty = Holding::default();
        let holding = self.holdings.get(&order.order.symbol).unwrap_or(&empty);
        let new_price = OrderPrice::Limit(price);
        if order.order.kind == TradeKind::ShortSell
            && facts.short_price_floor
            && facts.ref_price.is_none_or(|reference| price < reference)
        {
            return Err(RejectReason::ShortPriceRestricted);
        }
        let strategy = order.purpose == Purpose::Strategy;
        let working = Working {
            order: &order.order,
            price: new_price,
            shares: order.working,
            filled: trade_value(&order.fills),
        };
        let reserve = reservation(&self.config, &facts, holding, day, &working);
        let old_hold = order_hold(&self.config, order, &facts, order.price);
        let new_hold = order_hold(&self.config, order, &facts, new_price);
        if order.prepaid
            && order.order.side == Side::Buy
            && self.cash - day.prepaid + old_hold < new_hold
        {
            return Err(RejectReason::PrepaymentRequired);
        }
        if strategy
            && self.config.account.cash_check == CashCheck::AtOrder
            && reserve.is_none_or(|reserve| self.available() + order.reserved < reserve)
        {
            return Err(RejectReason::InsufficientCash);
        }
        let old_value = order_value(&order.order, &facts, order.price, order.working);
        let new_value = order_value(&order.order, &facts, new_price, order.working);
        if strategy
            && let Some(limit) = self.config.account.daily_limit
            && day.traded + day.working - old_value.unwrap_or(Money::ZERO)
                + new_value.unwrap_or(Money::ZERO)
                > limit
        {
            return Err(RejectReason::DailyLimitExceeded);
        }
        let reserved = if strategy {
            reserve.unwrap_or(Money::ZERO)
        } else {
            Money::ZERO
        };
        let day = self.day.as_mut().expect("a trading day is open");
        let order = day.orders.get_mut(&id).expect("found above");
        day.reserved += reserved - order.reserved;
        day.prepaid += new_hold - old_hold;
        day.working += new_value.unwrap_or(Money::ZERO) - old_value.unwrap_or(Money::ZERO);
        order.reserved = reserved;
        order.price = new_price;
        Ok(())
    }

    /// Reports what happened to an order. Events of orders the account did not accept
    /// are ignored.
    pub fn on_event(&mut self, event: &OrderEvent) {
        let (filled, released) = match event.kind {
            OrderEventKind::Filled(fill) => (Some(fill), fill.qty.get()),
            OrderEventKind::Cancelled { qty, .. } | OrderEventKind::Expired { qty } => {
                (None, qty.get())
            }
            OrderEventKind::Reduced { by, .. } => (None, by.get()),
            OrderEventKind::Accepted
            | OrderEventKind::Rejected { .. }
            | OrderEventKind::Repriced { .. }
            | OrderEventKind::PriceBandLimited => return,
        };
        let day = self.day.as_mut().expect("a trading day is open");
        let Some(order) = day.orders.get_mut(&event.order) else {
            return;
        };
        let symbol = order.order.symbol;
        let facts = day.facts.securities[&symbol];
        let old_value = order_value(&order.order, &facts, order.price, order.working);
        let old_hold = order_hold(&self.config, order, &facts, order.price);
        assert!(
            released <= order.working,
            "{:?} released too much",
            event.order
        );
        order.working -= released;
        let activity = day.activity.entry(symbol).or_default();
        if let Some(fill) = filled {
            order.fills.push((fill.price, fill.qty));
            activity.fills.push(DayFill {
                order: event.order,
                price: fill.price,
                shares: released,
            });
            day.traded += notional(fill.price, released);
            let regular = order.order.board == Board::Regular;
            match (order.order.kind, order.order.side) {
                (TradeKind::Cash, Side::Buy) if order.purpose == Purpose::CoverOwed => {}
                (TradeKind::Cash, Side::Buy) if regular => activity.cash_bought += released,
                (TradeKind::Cash, Side::Buy) => activity.odd_bought += released,
                (TradeKind::Cash | TradeKind::CashSellFirst, Side::Sell) if regular => {
                    activity.cash_sold += released;
                }
                (TradeKind::Cash | TradeKind::CashSellFirst, Side::Sell) => {
                    activity.odd_sold += released;
                }
                (TradeKind::MarginBuy, _) => activity.margin_bought += released,
                (TradeKind::ShortSell, _) => activity.short_sold += released,
                _ => {}
            }
            if order.order.kind == TradeKind::Cash && order.order.side == Side::Sell {
                activity.cash_selling -= released;
            }
        } else {
            match (order.order.kind, order.order.side) {
                (TradeKind::Cash, Side::Sell) => activity.cash_selling -= released,
                (TradeKind::MarginSell, _) => activity.margin_selling -= released,
                (TradeKind::ShortCover, _) => activity.short_covering -= released,
                _ => {}
            }
            if order.prepaid && order.order.side == Side::Sell {
                activity.prepaid_selling -= released;
            }
        }
        let new_value = order_value(&order.order, &facts, order.price, order.working);
        day.working += new_value.unwrap_or(Money::ZERO) - old_value.unwrap_or(Money::ZERO);
        day.prepaid += order_hold(&self.config, order, &facts, order.price) - old_hold;

        if filled.is_some() {
            let output = provisional(&self.config, &self.holdings, day, symbol, self.next_lot);
            let net = output.net();
            let activity = day.activity.get_mut(&symbol).expect("inserted above");
            day.net += net - activity.net;
            activity.net = net;
        }
        // The reservation follows the working quantity and, for closing orders, the
        // lots that are left.
        let order = &day.orders[&event.order];
        let reserved = if order.purpose == Purpose::Strategy && order.working > 0 {
            let empty = Holding::default();
            let holding = self.holdings.get(&symbol).unwrap_or(&empty);
            let working = Working {
                order: &order.order,
                price: order.price,
                shares: order.working,
                filled: trade_value(&order.fills),
            };
            reservation(&self.config, &facts, holding, day, &working).unwrap_or(Money::ZERO)
        } else {
            Money::ZERO
        };
        let order = day.orders.get_mut(&event.order).expect("found above");
        day.reserved += reserved - order.reserved;
        order.reserved = reserved;
    }

    // ---- the close ---------------------------------------------------------------

    /// Closes the trading day (section 3 step 3). `quotes` are the official closing
    /// quotes of the day.
    pub fn end_day(
        &mut self,
        quotes: &HashMap<Symbol, DailyQuote>,
    ) -> Result<DayClose, LedgerError> {
        let day = self.day.take().expect("a trading day is open");
        let date = day.facts.date;
        let mut entries = Vec::new();
        let mut realized = Vec::new();
        let mut diagnostics = Vec::new();
        let mut uncovered: Vec<(Symbol, i64)> = Vec::new();
        for (&symbol, activity) in &day.activity {
            if activity.fills.is_empty() {
                continue;
            }
            let output = provisional(&self.config, &self.holdings, &day, symbol, self.next_lot);
            self.next_lot = output.next_lot;
            if output.uncovered > 0 {
                uncovered.push((symbol, output.uncovered));
            }
            entries.extend(output.entries);
            realized.extend(output.realized);
            if output.holding.is_empty() {
                self.holdings.remove(&symbol);
            } else {
                self.holdings.insert(symbol, output.holding);
            }
        }

        // Valuation (section 12.1).
        for &symbol in self.holdings.keys() {
            let facts = day.facts.securities.get(&symbol);
            if facts.is_none() {
                diagnostics.push(Diagnostic::StalePosition { symbol });
            }
            let reference = facts.and_then(|facts| facts.ref_price);
            let price = quotes
                .get(&symbol)
                .and_then(|quote| match (quote.close, reference) {
                    (Some(close), _) => Some(close),
                    (None, Some(reference)) => Some(match (quote.close_bid, quote.close_ask) {
                        (Some(bid), _) if bid > reference => bid,
                        (_, Some(ask)) if ask < reference => ask,
                        _ => reference,
                    }),
                    (None, None) => None,
                })
                .or_else(|| self.valuation.get(&symbol).copied())
                .or(reference)
                .ok_or(LedgerError::NoValuationPrice { symbol, date })?;
            self.valuation.insert(symbol, price);
        }

        // Shares sold first and not bought back (section 8.3).
        for &(symbol, shares) in &uncovered {
            if self.config.daytrade.uncovered == Uncovered::Error {
                return Err(LedgerError::UncoveredSellFirst { symbol, shares });
            }
            let fee = Money::new(divide(
                notional_units(self.valuation[&symbol], shares)
                    * i128::from(self.config.daytrade.borrow_fee_rate.numerator()),
                i128::from(self.config.daytrade.borrow_fee_rate.denominator())
                    * i128::from(Price::UNITS_PER_YUAN),
                Rounding::Floor,
            ));
            entries.push(Entry {
                date,
                settles: day.settles,
                symbol,
                order: None,
                item: Item::BorrowFee,
                amount: -fee,
                approx: true,
            });
            self.forced_owed.entry(symbol).or_insert(0);
        }
        let net: Money = entries.iter().map(|entry| entry.amount).sum();
        if net != Money::ZERO {
            *self.pending.entry(day.settles).or_default() += net;
        }

        let maintenance = self.maintenance();
        self.review_call(date, maintenance);
        self.review_forced(&day)?;
        self.last_day = Some(date);

        let pending = self.pending();
        let receivable: Money = self
            .receivables
            .iter()
            .map(|receivable| receivable.entry.amount)
            .sum();
        Ok(DayClose {
            date,
            settles: day.settles,
            entries,
            realized,
            cash: self.cash,
            pending,
            receivable,
            equity: self.cash + pending + receivable + self.position_value(),
            maintenance,
            call: self.call.is_some(),
            diagnostics,
        })
    }

    /// The value of every position at the valuation prices (section 14).
    fn position_value(&self) -> Money {
        let market = |symbol: Symbol, shares: i64| {
            self.valuation
                .get(&symbol)
                .map_or(Money::ZERO, |&price| notional(price, shares))
        };
        let positions: Money = self
            .holdings
            .iter()
            .map(|(&symbol, holding)| {
                let loans: Money = holding.margin.iter().map(|lot| lot.loan).sum();
                let held: Money = holding
                    .short
                    .iter()
                    .map(|lot| lot.margin + lot.collateral)
                    .sum();
                market(symbol, holding.cash_shares()) + market(symbol, holding.margin_shares())
                    - loans
                    + held
                    - market(symbol, holding.short_shares())
                    - market(symbol, holding.owed_shares())
            })
            .sum();
        let to_come: Money = self
            .deliveries
            .iter()
            .map(|delivery| market(delivery.symbol, delivery.shares))
            .sum();
        positions + to_come
    }

    /// The account maintenance ratio (section 12.2).
    fn maintenance(&self) -> Option<Maintenance> {
        let units = i128::from(Price::UNITS_PER_YUAN);
        let (mut numerator, mut denominator) = (0_i128, 0_i128);
        for (symbol, holding) in &self.holdings {
            let Some(&price) = self.valuation.get(symbol) else {
                continue;
            };
            for lot in &holding.margin {
                numerator += notional_units(price, lot.shares);
                denominator += i128::from(lot.loan.get()) * units;
            }
            for lot in &holding.short {
                numerator += i128::from((lot.margin + lot.collateral).get()) * units;
                denominator += notional_units(price, lot.shares);
            }
        }
        (denominator > 0).then_some(Maintenance {
            numerator,
            denominator,
        })
    }

    /// The lots whose own maintenance ratio is below the call threshold.
    fn lots_below_call(&self) -> Vec<LotId> {
        let units = i128::from(Price::UNITS_PER_YUAN);
        let threshold = self.config.credit.call_threshold;
        let mut lots = Vec::new();
        for (symbol, holding) in &self.holdings {
            let Some(&price) = self.valuation.get(symbol) else {
                continue;
            };
            let below = |numerator: i128, denominator: i128| {
                denominator > 0
                    && Maintenance {
                        numerator,
                        denominator,
                    }
                    .is_below(threshold)
            };
            lots.extend(
                holding
                    .margin
                    .iter()
                    .filter(|lot| {
                        below(
                            notional_units(price, lot.shares),
                            i128::from(lot.loan.get()) * units,
                        )
                    })
                    .map(|lot| lot.id),
            );
            lots.extend(
                holding
                    .short
                    .iter()
                    .filter(|lot| {
                        below(
                            i128::from((lot.margin + lot.collateral).get()) * units,
                            notional_units(price, lot.shares),
                        )
                    })
                    .map(|lot| lot.id),
            );
        }
        lots
    }

    /// Margin calls (section 12.3).
    fn review_call(&mut self, date: Date, maintenance: Option<Maintenance>) {
        let Some(maintenance) = maintenance else {
            self.call = None;
            return;
        };
        let credit = &self.config.credit;
        if !maintenance.is_below(credit.cancel_threshold) {
            self.call = None;
            return;
        }
        let below = maintenance.is_below(credit.call_threshold);
        match &self.call {
            None if below => {
                let wait = (credit.notice_lag_days + credit.grace_days).saturating_sub(1);
                let deadline = if wait == 0 {
                    Some(date)
                } else {
                    // Past the end of the calendar the run ends before the deadline.
                    self.calendar.trading_day_after(date, wait as usize).ok()
                };
                self.call = Some(Call {
                    deadline,
                    lots: self.lots_below_call(),
                });
                if deadline == Some(date) {
                    self.dispose_call();
                }
            }
            Some(call) if below && call.deadline.is_some_and(|deadline| date >= deadline) => {
                self.dispose_call();
            }
            _ => {}
        }
    }

    fn dispose_call(&mut self) {
        let call = self.call.take().expect("a call to dispose of");
        for lot in call.lots {
            self.forced_lots.entry(lot).or_insert(ForcedLot {
                reason: ForcedReason::MarginCall,
                attempts: 0,
            });
        }
    }

    /// Decides what must be closed on the next trading day: lots past their term,
    /// short lots on their last day to cover, and what today's forced orders left.
    fn review_forced(&mut self, day: &Day) -> Result<(), LedgerError> {
        let date = day.facts.date;
        // A forced order that was not sent, or not filled completely, is a failed
        // attempt for what it was to close.
        let failed: Vec<&ForcedPlan> = day
            .forced
            .iter()
            .filter(|plan| {
                plan.id
                    .and_then(|id| day.orders.get(&id))
                    .is_none_or(|order| {
                        order.fills.iter().map(|&(_, qty)| qty.get()).sum::<i64>()
                            < plan.order.order.qty.get()
                    })
            })
            .collect();
        let mut open: BTreeMap<LotId, (Symbol, i64)> = BTreeMap::new();
        for (&symbol, holding) in &self.holdings {
            let last_cover_day = day
                .facts
                .securities
                .get(&symbol)
                .is_some_and(|facts| facts.short_last_cover_day);
            for lot in &holding.margin {
                open.insert(lot.id, (symbol, lot.shares));
                if lot.due <= date {
                    self.forced_lots.entry(lot.id).or_insert(ForcedLot {
                        reason: ForcedReason::Expired,
                        attempts: 0,
                    });
                }
            }
            for lot in &holding.short {
                open.insert(lot.id, (symbol, lot.shares));
                let reason = if last_cover_day {
                    Some(ForcedReason::ShortSuspension)
                } else if lot.due <= date {
                    Some(ForcedReason::Expired)
                } else {
                    None
                };
                if let Some(reason) = reason {
                    self.forced_lots.entry(lot.id).or_insert(ForcedLot {
                        reason,
                        attempts: 0,
                    });
                }
            }
        }
        self.forced_lots.retain(|id, _| open.contains_key(id));
        for (id, lot) in &mut self.forced_lots {
            let missed = failed.iter().any(|plan| match &plan.purpose {
                Purpose::CloseLots(lots) => lots.contains(id),
                Purpose::Strategy | Purpose::CoverOwed => false,
            });
            lot.attempts = if missed { lot.attempts + 1 } else { 0 };
            if lot.attempts >= FORCED_ATTEMPTS {
                let (symbol, shares) = open[id];
                return Err(LedgerError::ForcedOrderUnfilled { symbol, shares });
            }
        }
        let holdings = &self.holdings;
        self.forced_owed.retain(|symbol, _| {
            holdings
                .get(symbol)
                .is_some_and(|holding| holding.owed_shares() > 0)
        });
        for (&symbol, attempts) in &mut self.forced_owed {
            let missed = failed.iter().any(|plan| {
                plan.purpose == Purpose::CoverOwed && plan.order.order.symbol == symbol
            });
            *attempts = if missed { *attempts + 1 } else { 0 };
            if *attempts >= FORCED_ATTEMPTS {
                return Err(LedgerError::ForcedOrderUnfilled {
                    symbol,
                    shares: holdings[&symbol].owed_shares(),
                });
            }
        }
        Ok(())
    }
}

/// The day-end computation for one instrument on the fills so far.
fn provisional(
    config: &Config,
    holdings: &BTreeMap<Symbol, Holding>,
    day: &Day,
    symbol: Symbol,
    next_lot: u64,
) -> CloseOutput {
    let empty = Holding::default();
    close(&CloseInput {
        date: day.facts.date,
        settles: day.settles,
        due: day.due,
        symbol,
        facts: &day.facts.securities[&symbol],
        config,
        holding: holdings.get(&symbol).unwrap_or(&empty),
        orders: &day.orders,
        fills: &day.activity[&symbol].fills,
        next_lot,
    })
}

/// The price an order is valued at before it fills: its limit, or the limit-up price
/// for a market order. `None` when there is no such price.
fn commit_price(facts: &SecurityFacts, price: OrderPrice) -> Option<Price> {
    price.limit().or(facts.limit_up)
}

/// The value of `shares` shares of an order at the price it is committed at.
fn order_value(
    _order: &NewOrder,
    facts: &SecurityFacts,
    price: OrderPrice,
    shares: i64,
) -> Option<Money> {
    commit_price(facts, price).map(|price| notional(price, shares))
}

/// Settled cash a buy under prepayment commits: its value and fee at the order price.
fn prepay_hold(
    config: &Config,
    order: &NewOrder,
    facts: &SecurityFacts,
    price: OrderPrice,
    shares: i64,
) -> Option<Money> {
    let value = order_value(order, facts, price, shares)?;
    Some(value + tax::fee(&config.fee, order.board, value))
}

/// The prepayment an order holds: for what it has filled and what is still working.
fn order_hold(
    config: &Config,
    order: &DayOrder,
    facts: &SecurityFacts,
    price: OrderPrice,
) -> Money {
    if !(order.prepaid && order.order.side == Side::Buy) {
        return Money::ZERO;
    }
    let filled: i64 = order.fills.iter().map(|&(_, qty)| qty.get()).sum();
    prepay_hold(config, &order.order, facts, price, filled + order.working).unwrap_or(Money::ZERO)
}

/// What a working part of an order is worked out from.
struct Working<'a> {
    order: &'a NewOrder,
    price: OrderPrice,
    shares: i64,
    /// The trade value the order has filled today.
    filled: Money,
}

/// The cash a working order sets aside (section 7.3): an upper bound of what its
/// further fills can take from the settlement of the day. `None` when there is no
/// price to work it out with.
fn reservation(
    config: &Config,
    facts: &SecurityFacts,
    holding: &Holding,
    day: &Day,
    working: &Working<'_>,
) -> Option<Money> {
    let Working {
        order,
        price,
        shares,
        filled,
    } = *working;
    if shares == 0 {
        return Some(Money::ZERO);
    }
    // The fee is charged on the whole order, so further fills add the difference.
    let fee = |value: Money| {
        tax::fee(&config.fee, order.board, filled + value)
            - tax::fee(&config.fee, order.board, filled)
    };
    let not_negative = |amount: Money| amount.max(Money::ZERO);
    // A buy fills at its price or below, a sale at its price or above.
    let highest = || match order.side {
        Side::Buy => commit_price(facts, price),
        Side::Sell => facts.limit_up.or(price.limit()),
    };
    let lowest = || price.limit().or(facts.limit_down);
    let sale_tax =
        |value: Money| tax::tax_rate(facts.class, day.facts.date).of(value, Rounding::Ceil);
    Some(match (order.kind, order.side) {
        (TradeKind::Cash, Side::Buy) => {
            let value = notional(highest()?, shares);
            value + fee(value)
        }
        (TradeKind::Cash | TradeKind::CashSellFirst, _) => {
            // Only a sale smaller than its own costs takes cash.
            let value = notional(lowest()?, shares);
            not_negative(fee(value) + sale_tax(value) - value)
        }
        (TradeKind::MarginBuy, _) => {
            let value = notional(highest()?, shares);
            let ratio = config.credit.margin_loan_ratio;
            let loan = divide(
                i128::from(value.get()) * i128::from(ratio.numerator()),
                i128::from(ratio.denominator()) * 1_000,
                Rounding::Floor,
            ) * 1_000;
            // The loan is rounded down to a thousand on the value that fills, which
            // can be a thousand less than on the value at the order price.
            value + fee(value) - Money::new((loan - 1_000).max(0))
        }
        (TradeKind::ShortSell, _) => {
            // The margin grows with the sale price.
            let value = notional(highest()?, shares);
            let ratio = short_margin_rate(config, facts, day.facts.date);
            Money::new(
                divide(
                    i128::from(value.get()) * i128::from(ratio.numerator()),
                    i128::from(ratio.denominator()) * 100,
                    Rounding::Ceil,
                ) * 100,
            )
        }
        (TradeKind::ShortCover, _) => {
            let value = notional(highest()?, shares);
            // The lots that give back the least for each share.
            let mut lots: Vec<(Money, i64)> = holding
                .short
                .iter()
                .map(|lot| (lot.margin + lot.collateral, lot.shares))
                .collect();
            lots.sort_by(|a, b| {
                (i128::from(a.0.get()) * i128::from(b.1))
                    .cmp(&(i128::from(b.0.get()) * i128::from(a.1)))
            });
            let (mut released, mut left) = (Money::ZERO, shares);
            for (held, lot_shares) in lots {
                let part = left.min(lot_shares);
                // Margin and collateral are each rounded down for a part of a lot.
                released += if part == lot_shares {
                    held
                } else {
                    not_negative(share_of(held, part, lot_shares) - Money::new(2))
                };
                left -= part;
                if left == 0 {
                    break;
                }
            }
            not_negative(value + fee(value) - released)
        }
        (TradeKind::MarginSell, _) => {
            let value = notional(lowest()?, shares);
            // The lots that owe the most for each share, with interest since the
            // oldest of them.
            let mut lots: Vec<(Money, i64)> = holding
                .margin
                .iter()
                .map(|lot| (lot.loan, lot.shares))
                .collect();
            lots.sort_by(|a, b| {
                (i128::from(b.0.get()) * i128::from(a.1))
                    .cmp(&(i128::from(a.0.get()) * i128::from(b.1)))
            });
            let (mut loan, mut left) = (Money::ZERO, shares);
            for (lot_loan, lot_shares) in lots {
                let part = left.min(lot_shares);
                loan += Money::new(divide(
                    i128::from(lot_loan.get()) * i128::from(part),
                    i128::from(lot_shares),
                    Rounding::Ceil,
                ));
                left -= part;
                if left == 0 {
                    break;
                }
            }
            let days = holding
                .margin
                .iter()
                .map(|lot| day.settles.epoch_days() - lot.settles.epoch_days())
                .max()
                .unwrap_or(0)
                .max(0);
            let rate = config.credit.margin_interest_rate;
            // Each lot rounds its own interest.
            let interest = Money::new(
                divide(
                    i128::from(loan.get()) * i128::from(rate.numerator()) * i128::from(days),
                    i128::from(rate.denominator()) * 365,
                    Rounding::Ceil,
                ) + i64::try_from(holding.margin.len()).expect("a number of lots"),
            );
            not_negative(loan + interest + fee(value) + sale_tax(value) - value)
        }
    })
}
