use osmium_strategy::{Context, ParamError, Params, Strategy, StrategyEntry, TimerId};
use osmium_types::{Board, NewOrder, OrderPrice, Shares, Side, Symbol, TimeInForce, TradeKind};

pub const ENTRY: StrategyEntry = StrategyEntry {
    name: "open_to_close",
    version: "1",
    about: "Buys `lots` board lots of `symbol` at the open and sells them at the close.",
    build: |params| Ok(Box::new(OpenToClose::from_params(params)?)),
};

/// Buys in the opening auction and sells in the closing auction of the same day.
///
/// Selling shares bought the same day is a day trade: the account and the instrument
/// must allow it (`docs/reference/accounting.md` section 8), otherwise the sale is
/// rejected and the shares are sold at the next close.
#[derive(Debug)]
pub struct OpenToClose {
    symbol: Symbol,
    lots: i64,
}

impl OpenToClose {
    #[must_use]
    pub const fn new(symbol: Symbol, lots: i64) -> Self {
        Self { symbol, lots }
    }

    fn from_params(params: &mut Params) -> Result<Self, ParamError> {
        let symbol = params.symbol("symbol")?;
        let lots = params.integer_or("lots", 1)?;
        if lots <= 0 {
            return Err(ParamError::new("lots", "must be positive"));
        }
        Ok(Self::new(symbol, lots))
    }

    fn order(&self, side: Side, price: osmium_types::Price, shares: i64) -> NewOrder {
        NewOrder {
            symbol: self.symbol,
            board: Board::Regular,
            side,
            kind: TradeKind::Cash,
            price: OrderPrice::Limit(price),
            time_in_force: TimeInForce::Rod,
            qty: Shares::new(shares),
        }
    }
}

impl Strategy for OpenToClose {
    fn on_day_start(&mut self, ctx: &mut Context<'_>) {
        let Some(instrument) = ctx.market().instrument(self.symbol) else {
            return;
        };
        let (Some(lot), Some(limit_up)) = (instrument.shares_per_lot, instrument.limit_up) else {
            return;
        };
        // Orders for the closing auction are collected from 13:25.
        ctx.set_timer(ctx.date().at_taipei(13, 25, 0))
            .expect("13:25 is later today");
        // Buy only when yesterday's shares were sold.
        if ctx.account().position(self.symbol).is_flat() {
            let order = self.order(Side::Buy, limit_up, self.lots * i64::from(lot));
            ctx.submit(order);
        }
    }

    fn on_timer(&mut self, ctx: &mut Context<'_>, _timer: TimerId) {
        let Some(instrument) = ctx.market().instrument(self.symbol) else {
            return;
        };
        let (Some(lot), Some(limit_down)) = (instrument.shares_per_lot, instrument.limit_down)
        else {
            return;
        };
        // What the reports have shown so far, in whole board lots.
        let held = ctx.account().position(self.symbol).cash;
        let lot = i64::from(lot);
        let shares = held / lot * lot;
        if shares > 0 {
            let order = self.order(Side::Sell, limit_down, shares);
            ctx.submit(order);
        }
    }
}
