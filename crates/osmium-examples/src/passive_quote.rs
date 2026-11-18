use osmium_book::Mode;
use osmium_strategy::{Context, ParamError, Params, Strategy, StrategyEntry, TimerId};
use osmium_types::{
    Board, Message, NewOrder, OrderEvent, OrderEventKind, OrderId, OrderPrice, Price, Request,
    Shares, Side, Symbol, TimeInForce, TradeKind,
};

pub const ENTRY: StrategyEntry = StrategyEntry {
    name: "passive_quote",
    version: "1",
    about: "Waits at the best bid of `symbol` for one board lot and offers it at the best ask.",
    build: |params| Ok(Box::new(PassiveQuote::from_params(params)?)),
};

/// Where the strategy stands within a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Nothing is working.
    Idle,
    /// A buy order waits at `price`. `moving` while a new price is on its way.
    Buying {
        id: OrderId,
        price: Price,
        moving: bool,
    },
    /// The lot is bought and offered.
    Selling { id: OrderId },
    /// No new orders today.
    Done,
}

/// Joins the best bid with one board lot; once filled, offers the lot at the best ask;
/// from 13:25 it cancels the bid and sells what it holds in the closing auction.
///
/// A passive order is filled only after the quantity that was ahead of it has traded
/// (`docs/reference/matching.md` section 6), so the results depend on the queue model
/// far more than those of a strategy that takes liquidity. Selling the lot the same
/// day is a day trade, which the account and the instrument must allow.
#[derive(Debug)]
pub struct PassiveQuote {
    symbol: Symbol,
    stage: Stage,
    /// It is 13:25 or later: only the way out is left.
    closing: bool,
}

impl PassiveQuote {
    #[must_use]
    pub const fn new(symbol: Symbol) -> Self {
        Self {
            symbol,
            stage: Stage::Idle,
            closing: false,
        }
    }

    fn from_params(params: &mut Params) -> Result<Self, ParamError> {
        Ok(Self::new(params.symbol("symbol")?))
    }

    fn order(&self, side: Side, price: Price, shares: i64) -> NewOrder {
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

    /// The price to offer `shares` at: the best ask, or the lower limit once the
    /// closing auction is all that is left, where it trades at the closing price.
    fn offer(&mut self, ctx: &mut Context<'_>, shares: i64) {
        let instrument = ctx.market().instrument(self.symbol);
        let ask = ctx
            .market()
            .book(self.symbol, Board::Regular)
            .and_then(|state| state.book().ask().best())
            .map(|level| level.price);
        let price = if self.closing {
            instrument.and_then(|instrument| instrument.limit_down)
        } else {
            ask.or_else(|| instrument.and_then(|instrument| instrument.limit_up))
        };
        self.stage = price
            .and_then(|price| {
                let order = self.order(Side::Sell, price, shares);
                ctx.submit(order)
            })
            .map_or(Stage::Done, |id| Stage::Selling { id });
    }
}

impl Strategy for PassiveQuote {
    fn on_day_start(&mut self, ctx: &mut Context<'_>) {
        // Nothing carries over: yesterday's orders have expired.
        self.stage = Stage::Idle;
        self.closing = false;
        ctx.set_timer(ctx.date().at_taipei(13, 25, 0))
            .expect("13:25 is later today");
    }

    fn on_market(&mut self, ctx: &mut Context<'_>, message: &Message) {
        if self.closing || message.symbol != self.symbol || message.feed.board() != Board::Regular {
            return;
        }
        let Some(state) = ctx.market().book(self.symbol, Board::Regular) else {
            return;
        };
        // Quote only while orders are matched as they arrive.
        if state.mode(ctx.now()) != Mode::Continuous {
            return;
        }
        let Some(bid) = state.book().bid().best() else {
            return;
        };
        match self.stage {
            Stage::Idle => {
                let lot = ctx
                    .market()
                    .instrument(self.symbol)
                    .and_then(|instrument| instrument.shares_per_lot);
                let Some(lot) = lot else {
                    return;
                };
                // Shares left from another day are not this strategy's to trade.
                if !ctx.account().position(self.symbol).is_flat() {
                    return;
                }
                let order = self.order(Side::Buy, bid.price, i64::from(lot));
                if let Some(id) = ctx.submit(order) {
                    self.stage = Stage::Buying {
                        id,
                        price: bid.price,
                        moving: false,
                    };
                }
            }
            // Follow the best bid. A new price gives up the place in the queue.
            Stage::Buying {
                id,
                price,
                moving: false,
            } if price != bid.price => {
                ctx.reprice(id, bid.price);
                self.stage = Stage::Buying {
                    id,
                    price,
                    moving: true,
                };
            }
            Stage::Buying { .. } | Stage::Selling { .. } | Stage::Done => {}
        }
    }

    fn on_order_event(&mut self, ctx: &mut Context<'_>, event: &OrderEvent) {
        match (self.stage, event.kind) {
            (Stage::Buying { id, .. }, OrderEventKind::Repriced { price }) if id == event.order => {
                self.stage = Stage::Buying {
                    id,
                    price,
                    moving: false,
                };
            }
            // A change that is refused leaves the order where it was.
            (
                Stage::Buying { id, price, .. },
                OrderEventKind::Rejected {
                    request: Request::Reprice,
                    ..
                },
            ) if id == event.order => {
                self.stage = Stage::Buying {
                    id,
                    price,
                    moving: false,
                };
            }
            (Stage::Buying { id, .. }, OrderEventKind::Filled(fill)) if id == event.order => {
                self.offer(ctx, fill.qty.get());
            }
            (Stage::Buying { id, .. }, OrderEventKind::Cancelled { .. }) if id == event.order => {
                self.stage = Stage::Done;
            }
            (Stage::Selling { id }, OrderEventKind::Filled(_)) if id == event.order => {
                // The round trip is done; start over on the next message.
                self.stage = if self.closing {
                    Stage::Done
                } else {
                    Stage::Idle
                };
            }
            // A new order that is refused never existed. Stop for the day instead of
            // sending it again on every message.
            (
                Stage::Buying { id, .. } | Stage::Selling { id },
                OrderEventKind::Rejected {
                    request: Request::New,
                    ..
                },
            ) if id == event.order => self.stage = Stage::Done,
            _ => {}
        }
    }

    fn on_timer(&mut self, ctx: &mut Context<'_>, _timer: TimerId) {
        self.closing = true;
        match self.stage {
            // If the bid is filled before the cancel arrives, the fill is offered in
            // the closing auction.
            Stage::Buying { id, .. } => ctx.cancel(id),
            Stage::Selling { id } => {
                let limit_down = ctx
                    .market()
                    .instrument(self.symbol)
                    .and_then(|instrument| instrument.limit_down);
                if let Some(limit_down) = limit_down {
                    ctx.reprice(id, limit_down);
                }
            }
            Stage::Idle | Stage::Done => self.stage = Stage::Done,
        }
    }
}
