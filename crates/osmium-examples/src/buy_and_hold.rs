use osmium_strategy::{Context, ParamError, Params, Strategy, StrategyEntry};
use osmium_types::{
    Board, NewOrder, OrderEvent, OrderEventKind, OrderPrice, Shares, Side, Symbol, TimeInForce,
    TradeKind,
};

pub const ENTRY: StrategyEntry = StrategyEntry {
    name: "buy_and_hold",
    version: "1",
    about: "Buys `lots` board lots of `symbol` in the opening auction and keeps them.",
    build: |params| Ok(Box::new(BuyAndHold::from_params(params)?)),
};

/// Buys a fixed number of board lots in the opening auction and never sells.
///
/// The order is sent before the open at the day's upper price limit, so it takes part
/// in the opening auction and is filled at the opening price. What is not filled on
/// one day is ordered again on the next, because orders do not carry over.
#[derive(Debug)]
pub struct BuyAndHold {
    symbol: Symbol,
    lots: i64,
    /// Shares bought so far.
    bought: i64,
}

impl BuyAndHold {
    #[must_use]
    pub const fn new(symbol: Symbol, lots: i64) -> Self {
        Self {
            symbol,
            lots,
            bought: 0,
        }
    }

    fn from_params(params: &mut Params) -> Result<Self, ParamError> {
        let symbol = params.symbol("symbol")?;
        let lots = params.integer_or("lots", 1)?;
        if lots <= 0 {
            return Err(ParamError::new("lots", "must be positive"));
        }
        Ok(Self::new(symbol, lots))
    }
}

impl Strategy for BuyAndHold {
    fn on_day_start(&mut self, ctx: &mut Context<'_>) {
        // Today's reference data: the lot size and the price limits.
        let Some(instrument) = ctx.market().instrument(self.symbol) else {
            return;
        };
        let (Some(lot), Some(limit_up)) = (instrument.shares_per_lot, instrument.limit_up) else {
            return;
        };
        let missing = self.lots * i64::from(lot) - self.bought;
        if missing > 0 {
            ctx.submit(NewOrder {
                symbol: self.symbol,
                board: Board::Regular,
                side: Side::Buy,
                kind: TradeKind::Cash,
                price: OrderPrice::Limit(limit_up),
                time_in_force: TimeInForce::Rod,
                qty: Shares::new(missing),
            });
        }
    }

    fn on_order_event(&mut self, _ctx: &mut Context<'_>, event: &OrderEvent) {
        // This strategy has one order a day, so every fill is a purchase.
        if let OrderEventKind::Filled(fill) = event.kind {
            self.bought += fill.qty.get();
        }
    }
}
