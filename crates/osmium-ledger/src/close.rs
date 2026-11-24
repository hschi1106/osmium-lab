//! The day-end computation for one instrument: pairing, fees and tax, lots and realised
//! results (`docs/reference/accounting.md` sections 5 to 11 and 14).
//!
//! It is a pure function of the lots at the start of the day and the fills of the day,
//! so the same code gives the provisional figures during the day and the entries at
//! the close.

use std::collections::{BTreeMap, VecDeque};

use osmium_types::{
    Board, Date, Money, NewOrder, OrderId, OrderPrice, Price, Shares, Side, Symbol, TradeKind,
};

use crate::{
    amount::{Rate, Rounding, allocate, divide, notional_units, share_of, trade_value},
    config::Config,
    facts::{SecurityFacts, short_margin_ratio},
    journal::{Entry, Item, Realized, RealizedKind},
    position::{CashLot, Holding, LotId, MarginLot, OwedLot, ShortLot},
    tax,
};

/// Why an order exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Purpose {
    Strategy,
    /// Buys back shares sold without inventory on an earlier day.
    CoverOwed,
    /// Closes these lots first.
    CloseLots(Vec<LotId>),
}

/// An order the account accepted today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DayOrder {
    pub order: NewOrder,
    /// The current price; differs from `order.price` after a reprice.
    pub price: OrderPrice,
    /// Shares still at the exchange.
    pub working: i64,
    pub purpose: Purpose,
    pub fills: Vec<(Price, Shares)>,
    /// Cash set aside for the working part.
    pub reserved: Money,
    /// The order is subject to prepayment (section 4.1).
    pub prepaid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DayFill {
    pub order: OrderId,
    pub price: Price,
    pub shares: i64,
}

pub(crate) struct CloseInput<'a> {
    pub date: Date,
    /// The settlement date of today's trades.
    pub settles: Date,
    /// The day lots opened today must be closed by.
    pub due: Date,
    pub symbol: Symbol,
    pub facts: &'a SecurityFacts,
    pub config: &'a Config,
    pub holding: &'a Holding,
    pub orders: &'a BTreeMap<OrderId, DayOrder>,
    /// The fills of this instrument, in the order they happened.
    pub fills: &'a [DayFill],
    pub next_lot: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CloseOutput {
    pub holding: Holding,
    pub entries: Vec<Entry>,
    pub realized: Vec<Realized>,
    pub next_lot: u64,
    /// Shares sold without inventory today and not bought back.
    pub uncovered: i64,
}

impl CloseOutput {
    pub fn net(&self) -> Money {
        self.entries.iter().map(|entry| entry.amount).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// One leg of a cash day trade; the number identifies the pair.
    DayTrade(usize),
    /// One leg of a margin buy paired with a short sale.
    Offset(usize),
    NewCash,
    FromInventory,
    CoverOwed,
    OpenMargin,
    OpenShort,
    CloseMargin,
    CloseShort,
}

/// A part of a fill with one role.
#[derive(Debug, Clone, Copy)]
struct Piece {
    fill: usize,
    order: OrderId,
    side: Side,
    shares: i64,
    price: Price,
    role: Role,
    value: Money,
    fee: Money,
    tax: Money,
    short_fee: Money,
}

impl Piece {
    fn weight(&self) -> i128 {
        notional_units(self.price, self.shares)
    }

    /// What a sale brings in after the selling costs.
    fn proceeds(&self) -> Money {
        self.value - self.fee - self.tax - self.short_fee
    }

    /// What a purchase costs.
    fn cost(&self) -> Money {
        self.value + self.fee
    }
}

/// Unmatched quantity, oldest first: `(fill index, shares left)`.
type Unmatched = VecDeque<(usize, i64)>;

struct Pairing<'a> {
    input: &'a CloseInput<'a>,
    pieces: Vec<Piece>,
    pairs: usize,
}

impl Pairing<'_> {
    fn push(&mut self, fill: usize, shares: i64, role: Role) {
        let day_fill = &self.input.fills[fill];
        self.pieces.push(Piece {
            fill,
            order: day_fill.order,
            side: self.input.orders[&day_fill.order].order.side,
            shares,
            price: day_fill.price,
            role,
            value: Money::ZERO,
            fee: Money::ZERO,
            tax: Money::ZERO,
            short_fee: Money::ZERO,
        });
    }

    /// Pairs up to `left` shares of fill `fill` with the oldest unmatched quantity of
    /// the other side.
    fn pair(
        &mut self,
        other: &mut Unmatched,
        fill: usize,
        left: &mut i64,
        role: fn(usize) -> Role,
    ) {
        while *left > 0 {
            let Some(front) = other.front_mut() else {
                return;
            };
            let shares = (*left).min(front.1);
            let earlier = front.0;
            front.1 -= shares;
            if front.1 == 0 {
                other.pop_front();
            }
            *left -= shares;
            let pair = self.pairs;
            self.pairs += 1;
            self.push(earlier, shares, role(pair));
            self.push(fill, shares, role(pair));
        }
    }
}

/// Splits every fill into pieces by the pairing rules of sections 8.2 and 11.
fn pair(input: &CloseInput<'_>) -> Vec<Piece> {
    let (config, facts) = (input.config, input.facts);
    let day_trade = config.account.daytrade_enabled && facts.day_trade.buy_first;
    let sell_first = day_trade && facts.day_trade.sell_first;
    let offset = config.account.credit_offset_enabled
        && facts.margin_eligible
        && facts.short_eligible
        && !facts.short_suspended;
    let mut pairing = Pairing {
        input,
        pieces: Vec::with_capacity(input.fills.len()),
        pairs: 0,
    };
    let (mut cash_buys, mut cash_sells) = (Unmatched::new(), Unmatched::new());
    let (mut margin_buys, mut short_sells) = (Unmatched::new(), Unmatched::new());
    for (index, fill) in input.fills.iter().enumerate() {
        let order = &input.orders[&fill.order];
        let mut left = fill.shares;
        match (order.order.kind, order.order.side) {
            (TradeKind::Cash | TradeKind::CashSellFirst, _)
                if order.purpose == Purpose::CoverOwed =>
            {
                pairing.push(index, left, Role::CoverOwed);
            }
            (TradeKind::Cash | TradeKind::CashSellFirst, side) => {
                if !(day_trade && order.order.board == Board::Regular) {
                    let role = match side {
                        Side::Buy => Role::NewCash,
                        Side::Sell => Role::FromInventory,
                    };
                    pairing.push(index, left, role);
                    continue;
                }
                match side {
                    Side::Buy => {
                        // Only sells that may be bought back are waiting here.
                        pairing.pair(&mut cash_sells, index, &mut left, Role::DayTrade);
                        if left > 0 {
                            cash_buys.push_back((index, left));
                        }
                    }
                    Side::Sell => {
                        pairing.pair(&mut cash_buys, index, &mut left, Role::DayTrade);
                        if left > 0 && sell_first {
                            cash_sells.push_back((index, left));
                        } else if left > 0 {
                            pairing.push(index, left, Role::FromInventory);
                        }
                    }
                }
            }
            (TradeKind::MarginBuy, _) if offset => {
                pairing.pair(&mut short_sells, index, &mut left, Role::Offset);
                if left > 0 {
                    margin_buys.push_back((index, left));
                }
            }
            (TradeKind::ShortSell, _) if offset => {
                pairing.pair(&mut margin_buys, index, &mut left, Role::Offset);
                if left > 0 {
                    short_sells.push_back((index, left));
                }
            }
            (TradeKind::MarginBuy, _) => pairing.push(index, left, Role::OpenMargin),
            (TradeKind::ShortSell, _) => pairing.push(index, left, Role::OpenShort),
            (TradeKind::MarginSell, _) => pairing.push(index, left, Role::CloseMargin),
            (TradeKind::ShortCover, _) => pairing.push(index, left, Role::CloseShort),
        }
    }
    for (queue, role) in [
        (cash_buys, Role::NewCash),
        (cash_sells, Role::FromInventory),
        (margin_buys, Role::OpenMargin),
        (short_sells, Role::OpenShort),
    ] {
        for (index, left) in queue {
            pairing.push(index, left, role);
        }
    }
    let mut pieces = pairing.pieces;
    pieces.sort_by_key(|piece| piece.fill);
    pieces
}

/// Gives every piece its share of the order's trade value, fee, tax and short fee, and
/// returns the entries of those four items.
fn price_pieces(input: &CloseInput<'_>, pieces: &mut [Piece]) -> Vec<Entry> {
    let mut by_order: BTreeMap<OrderId, Vec<usize>> = BTreeMap::new();
    for (index, piece) in pieces.iter().enumerate() {
        by_order.entry(piece.order).or_default().push(index);
    }
    let class = input.facts.class;
    let mut entries = Vec::new();
    let mut entry = |order: OrderId, item: Item, amount: Money, approx: bool| {
        if amount != Money::ZERO {
            entries.push(Entry {
                date: input.date,
                settles: input.settles,
                symbol: input.symbol,
                order: Some(order),
                item,
                amount,
                approx,
            });
        }
    };
    for (&id, indices) in &by_order {
        let order = &input.orders[&id];
        let spread = |pieces: &mut [Piece],
                      indices: &[usize],
                      total: Money,
                      field: fn(&mut Piece) -> &mut Money| {
            let weights: Vec<i128> = indices
                .iter()
                .map(|&index| pieces[index].weight())
                .collect();
            for (&index, part) in indices.iter().zip(allocate(total, &weights)) {
                *field(&mut pieces[index]) = part;
            }
        };
        let value = trade_value(&order.fills);
        let fee = tax::fee(&input.config.fee, order.order.board, value);
        spread(pieces, indices, value, |piece| &mut piece.value);
        spread(pieces, indices, fee, |piece| &mut piece.fee);
        let sign = match order.order.side {
            Side::Buy => -1,
            Side::Sell => 1,
        };
        entry(id, Item::TradeValue, Money::new(sign * value.get()), false);
        entry(id, Item::Fee, -fee, false);
        if order.order.side == Side::Buy {
            continue;
        }
        let (day_trades, others): (Vec<usize>, Vec<usize>) = indices
            .iter()
            .partition(|&&index| matches!(pieces[index].role, Role::DayTrade(_)));
        let total = |indices: &[usize]| indices.iter().map(|&index| pieces[index].value).sum();
        let (day_trade_tax, other_tax) =
            tax::tax(class, input.date, total(&day_trades), total(&others));
        spread(pieces, &day_trades, day_trade_tax, |piece| &mut piece.tax);
        spread(pieces, &others, other_tax, |piece| &mut piece.tax);
        entry(
            id,
            Item::Tax,
            -(day_trade_tax + other_tax),
            input.facts.day_trade.approx && !day_trades.is_empty(),
        );
        if order.order.kind == TradeKind::ShortSell {
            let short_fee = input
                .config
                .credit
                .short_fee_rate
                .of(value, Rounding::Floor);
            spread(pieces, indices, short_fee, |piece| &mut piece.short_fee);
            entry(id, Item::ShortFee, -short_fee, true);
        }
    }
    entries
}

/// Interest on `principal` for `days` calendar days.
fn interest(principal: Money, rate: Rate, days: i32, rounding: Rounding) -> Money {
    let days = i128::from(days.max(0));
    Money::new(divide(
        i128::from(principal.get()) * i128::from(rate.numerator()) * days,
        i128::from(rate.denominator()) * 365,
        rounding,
    ))
}

/// The amounts of a piece that are still to be attributed to lots.
struct Rest {
    shares: i64,
    proceeds: Money,
    cost: Money,
}

impl Rest {
    fn of(piece: &Piece) -> Self {
        Self {
            shares: piece.shares,
            proceeds: piece.proceeds(),
            cost: piece.cost(),
        }
    }

    /// Takes the part that belongs to `shares` shares: `(proceeds, cost)`.
    fn take(&mut self, shares: i64) -> (Money, Money) {
        // Proceeds can be negative when the costs exceed a tiny sale; split what is
        // above zero and leave the rest to the last part.
        let part = |amount: Money| {
            if amount.get() >= 0 {
                share_of(amount, shares, self.shares)
            } else if shares == self.shares {
                amount
            } else {
                Money::ZERO
            }
        };
        let taken = (part(self.proceeds), part(self.cost));
        self.proceeds -= taken.0;
        self.cost -= taken.1;
        self.shares -= shares;
        taken
    }
}

/// The lots a closing piece takes from, as indices: the targeted lots first, then the
/// oldest.
fn closing_order<L>(lots: &[L], id: fn(&L) -> LotId, purpose: &Purpose) -> Vec<usize> {
    let targets: &[LotId] = match purpose {
        Purpose::CloseLots(targets) => targets,
        Purpose::Strategy | Purpose::CoverOwed => &[],
    };
    let mut order: Vec<usize> = (0..lots.len()).collect();
    order.sort_by_key(|&index| !targets.contains(&id(&lots[index])));
    order
}

struct Opened {
    order: OrderId,
    shares: i64,
    value: Money,
    fee: Money,
    tax: Money,
    short_fee: Money,
}

fn open(opened: &mut Vec<Opened>, piece: &Piece) {
    let index = opened
        .iter()
        .position(|lot| lot.order == piece.order)
        .unwrap_or_else(|| {
            opened.push(Opened {
                order: piece.order,
                shares: 0,
                value: Money::ZERO,
                fee: Money::ZERO,
                tax: Money::ZERO,
                short_fee: Money::ZERO,
            });
            opened.len() - 1
        });
    let lot = &mut opened[index];
    lot.shares += piece.shares;
    lot.value += piece.value;
    lot.fee += piece.fee;
    lot.tax += piece.tax;
    lot.short_fee += piece.short_fee;
}

/// The day-end result for one instrument.
#[allow(clippy::too_many_lines)]
pub(crate) fn close(input: &CloseInput<'_>) -> CloseOutput {
    let mut pieces = pair(input);
    let mut entries = price_pieces(input, &mut pieces);
    let (config, facts, date) = (input.config, input.facts, input.date);
    let mut holding = input.holding.clone();
    let mut realized: Vec<Realized> = Vec::new();
    let mut uncovered = 0;
    // Credit amounts by order, added to the entries at the end.
    let mut credit: BTreeMap<(OrderId, Item), Money> = BTreeMap::new();
    let mut add = |order: OrderId, item: Item, amount: Money| {
        *credit.entry((order, item)).or_default() += amount;
    };
    // The first leg of each pair, waiting for the second.
    let mut legs: BTreeMap<usize, Piece> = BTreeMap::new();
    let (mut new_cash, mut new_margin, mut new_short) = (Vec::new(), Vec::new(), Vec::new());
    let mut new_owed: Vec<OwedLot> = Vec::new();
    let mut beyond_inventory: Vec<Rest> = Vec::new();

    for piece in &pieces {
        match piece.role {
            Role::DayTrade(pair) | Role::Offset(pair) => {
                let Some(first) = legs.remove(&pair) else {
                    legs.insert(pair, *piece);
                    continue;
                };
                let (buy, sell) = match first.side {
                    Side::Buy => (&first, piece),
                    Side::Sell => (piece, &first),
                };
                let is_day_trade = matches!(piece.role, Role::DayTrade(_));
                realized.push(Realized {
                    date,
                    symbol: input.symbol,
                    kind: if is_day_trade {
                        RealizedKind::DayTrade
                    } else {
                        RealizedKind::CreditOffset
                    },
                    shares: piece.shares,
                    opened: date,
                    proceeds: sell.proceeds(),
                    cost: buy.cost(),
                    interest: Money::ZERO,
                    pnl: sell.proceeds() - buy.cost(),
                    approx: if is_day_trade {
                        facts.day_trade.approx
                    } else {
                        true
                    },
                });
            }
            Role::NewCash => open(&mut new_cash, piece),
            Role::OpenMargin => open(&mut new_margin, piece),
            Role::OpenShort => open(&mut new_short, piece),
            Role::FromInventory => {
                let mut rest = Rest::of(piece);
                while rest.shares > 0 {
                    let Some(lot) = holding.cash.first_mut() else {
                        break;
                    };
                    let shares = rest.shares.min(lot.shares);
                    let (proceeds, _) = rest.take(shares);
                    let cost = share_of(lot.cost, shares, lot.shares);
                    realized.push(Realized {
                        date,
                        symbol: input.symbol,
                        kind: RealizedKind::Cash,
                        shares,
                        opened: lot.acquired,
                        proceeds,
                        cost,
                        interest: Money::ZERO,
                        pnl: proceeds - cost,
                        approx: false,
                    });
                    lot.shares -= shares;
                    lot.cost -= cost;
                    if lot.shares == 0 {
                        holding.cash.remove(0);
                    }
                }
                if rest.shares > 0 {
                    beyond_inventory.push(rest);
                }
            }
            Role::CoverOwed => {
                let mut rest = Rest::of(piece);
                while rest.shares > 0 {
                    let Some(lot) = holding.owed.first_mut() else {
                        break;
                    };
                    let shares = rest.shares.min(lot.shares);
                    let (_, cost) = rest.take(shares);
                    let proceeds = if lot.proceeds.get() >= 0 {
                        share_of(lot.proceeds, shares, lot.shares)
                    } else if shares == lot.shares {
                        lot.proceeds
                    } else {
                        Money::ZERO
                    };
                    realized.push(Realized {
                        date,
                        symbol: input.symbol,
                        kind: RealizedKind::SellFirstCover,
                        shares,
                        opened: lot.sold,
                        proceeds,
                        cost,
                        interest: Money::ZERO,
                        pnl: proceeds - cost,
                        approx: true,
                    });
                    lot.shares -= shares;
                    lot.proceeds -= proceeds;
                    if lot.shares == 0 {
                        holding.owed.remove(0);
                    }
                }
                if rest.shares > 0 {
                    // More was bought than was owed: the rest is inventory.
                    new_cash.push(Opened {
                        order: piece.order,
                        shares: rest.shares,
                        value: rest.cost,
                        fee: Money::ZERO,
                        tax: Money::ZERO,
                        short_fee: Money::ZERO,
                    });
                }
            }
            Role::CloseMargin => {
                let purpose = &input.orders[&piece.order].purpose;
                let mut rest = Rest::of(piece);
                for index in closing_order(&holding.margin, |lot| lot.id, purpose) {
                    if rest.shares == 0 {
                        break;
                    }
                    let lot = &mut holding.margin[index];
                    let shares = rest.shares.min(lot.shares);
                    let (proceeds, _) = rest.take(shares);
                    let cost = share_of(lot.cost, shares, lot.shares);
                    let loan = share_of(lot.loan, shares, lot.shares);
                    let paid = interest(
                        loan,
                        config.credit.margin_interest_rate,
                        input.settles.epoch_days() - lot.settles.epoch_days(),
                        config.credit.interest_rounding,
                    );
                    add(piece.order, Item::MarginLoan, -loan);
                    add(piece.order, Item::MarginInterest, -paid);
                    realized.push(Realized {
                        date,
                        symbol: input.symbol,
                        kind: RealizedKind::Margin,
                        shares,
                        opened: lot.opened,
                        proceeds,
                        cost,
                        interest: -paid,
                        pnl: proceeds - cost - paid,
                        approx: true,
                    });
                    lot.shares -= shares;
                    lot.cost -= cost;
                    lot.loan -= loan;
                }
                assert_eq!(rest.shares, 0, "a margin sale beyond the margin position");
                holding.margin.retain(|lot| lot.shares > 0);
            }
            Role::CloseShort => {
                let purpose = &input.orders[&piece.order].purpose;
                let mut rest = Rest::of(piece);
                for index in closing_order(&holding.short, |lot| lot.id, purpose) {
                    if rest.shares == 0 {
                        break;
                    }
                    let lot = &mut holding.short[index];
                    let shares = rest.shares.min(lot.shares);
                    let (_, cost) = rest.take(shares);
                    let margin = share_of(lot.margin, shares, lot.shares);
                    let collateral = share_of(lot.collateral, shares, lot.shares);
                    let received = interest(
                        margin + collateral,
                        config.credit.short_credit_rate,
                        input.settles.epoch_days() - lot.settles.epoch_days(),
                        config.credit.interest_rounding,
                    );
                    add(piece.order, Item::ShortMargin, margin);
                    add(piece.order, Item::ShortCollateral, collateral);
                    add(piece.order, Item::ShortInterest, received);
                    realized.push(Realized {
                        date,
                        symbol: input.symbol,
                        kind: RealizedKind::Short,
                        shares,
                        opened: lot.opened,
                        proceeds: collateral,
                        cost,
                        interest: received,
                        pnl: collateral + received - cost,
                        approx: true,
                    });
                    lot.shares -= shares;
                    lot.margin -= margin;
                    lot.collateral -= collateral;
                }
                assert_eq!(rest.shares, 0, "a short cover beyond the short position");
                holding.short.retain(|lot| lot.shares > 0);
            }
        }
    }
    assert!(legs.is_empty(), "every pair has two legs");

    // Sales beyond the inventory take the shares bought today that are not part of a
    // day trade; what is still missing was sold without being bought back.
    for mut rest in beyond_inventory {
        for lot in &mut new_cash {
            if rest.shares == 0 {
                break;
            }
            if lot.shares == 0 {
                continue;
            }
            let shares = rest.shares.min(lot.shares);
            let (proceeds, _) = rest.take(shares);
            let whole = lot.value + lot.fee;
            let cost = share_of(whole, shares, lot.shares);
            realized.push(Realized {
                date,
                symbol: input.symbol,
                kind: RealizedKind::Cash,
                shares,
                opened: date,
                proceeds,
                cost,
                interest: Money::ZERO,
                pnl: proceeds - cost,
                approx: false,
            });
            lot.shares -= shares;
            lot.value = whole - cost;
            lot.fee = Money::ZERO;
        }
        if rest.shares > 0 {
            uncovered += rest.shares;
            new_owed.push(OwedLot {
                shares: rest.shares,
                proceeds: rest.proceeds,
                sold: date,
            });
        }
    }
    new_cash.retain(|lot| lot.shares > 0);

    let mut next_lot = input.next_lot;
    let mut lot_id = || {
        next_lot += 1;
        LotId(next_lot)
    };
    holding.cash.extend(new_cash.iter().map(|lot| CashLot {
        shares: lot.shares,
        cost: lot.value + lot.fee,
        acquired: date,
        settles: input.settles,
    }));
    for lot in &new_margin {
        // Rounded down to a thousand yuan.
        let loan = Money::new(
            divide(
                i128::from(lot.value.get())
                    * i128::from(config.credit.margin_loan_ratio.numerator()),
                i128::from(config.credit.margin_loan_ratio.denominator()) * 1_000,
                Rounding::Floor,
            ) * 1_000,
        );
        add(lot.order, Item::MarginLoan, loan);
        holding.margin.push(MarginLot {
            id: lot_id(),
            shares: lot.shares,
            cost: lot.value + lot.fee,
            loan,
            opened: date,
            settles: input.settles,
            due: input.due,
        });
    }
    for lot in &new_short {
        let ratio = short_margin_rate(config, facts, date);
        // Rounded up to a hundred yuan.
        let margin = Money::new(
            divide(
                i128::from(lot.value.get()) * i128::from(ratio.numerator()),
                i128::from(ratio.denominator()) * 100,
                Rounding::Ceil,
            ) * 100,
        );
        let collateral = lot.value - lot.tax - lot.short_fee - lot.fee;
        add(lot.order, Item::ShortMargin, -margin);
        add(lot.order, Item::ShortCollateral, -collateral);
        holding.short.push(ShortLot {
            id: lot_id(),
            shares: lot.shares,
            margin,
            collateral,
            opened: date,
            settles: input.settles,
            due: input.due,
        });
    }
    holding.owed.extend(new_owed);

    entries.extend(
        credit
            .into_iter()
            .filter(|&(_, amount)| amount != Money::ZERO)
            .map(|((order, item), amount)| Entry {
                date,
                settles: input.settles,
                symbol: input.symbol,
                order: Some(order),
                item,
                amount,
                approx: true,
            }),
    );
    entries.sort_by_key(|entry| (entry.order, entry.item));
    CloseOutput {
        holding,
        entries,
        realized,
        next_lot,
        uncovered,
    }
}

/// The short-sale margin ratio for a lot opened on `date` (sections 9.1 and 10.1).
pub(crate) fn short_margin_rate(config: &Config, facts: &SecurityFacts, date: Date) -> Rate {
    let ratio = config
        .credit
        .short_margin_ratio
        .unwrap_or_else(|| short_margin_ratio(date));
    if facts.repeated_disposition {
        ratio.plus(Rate::new(10, 100)).at_least(Rate::ONE)
    } else {
        ratio
    }
}
