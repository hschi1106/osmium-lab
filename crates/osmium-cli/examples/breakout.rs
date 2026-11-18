//! A program with its own strategy: the walk-through of
//! `docs/guide/writing-a-strategy.md`.
//!
//! ```sh
//! cargo run --release -p osmium-cli --example breakout -- strategies
//! cargo run --release -p osmium-cli --example breakout -- run run.toml --out results
//! ```

use std::process::ExitCode;

use osmium_book::Mode;
use osmium_strategy::{Context, ParamError, Params, Registry, Strategy, StrategyEntry, TimerId};
use osmium_types::{
    Board, Message, NewOrder, OrderEvent, OrderEventKind, OrderId, OrderPrice, Price, Shares, Side,
    Symbol, TimeInForce, TradeKind,
};

const ENTRY: StrategyEntry = StrategyEntry {
    name: "breakout",
    version: "1",
    about: "Buys one board lot of `symbol` when it trades at `trigger` or higher, \
            and sells it at the close.",
    build: |params| Ok(Box::new(Breakout::from_params(params)?)),
};

/// Buys one board lot the first time the instrument trades at the trigger price or
/// higher, and sells what it holds in the closing auction.
struct Breakout {
    symbol: Symbol,
    trigger: Price,
    /// Today's buy order, once it is sent. One attempt a day.
    buy: Option<OrderId>,
}

impl Breakout {
    fn from_params(params: &mut Params) -> Result<Self, ParamError> {
        Ok(Self {
            symbol: params.symbol("symbol")?,
            trigger: params.price("trigger")?,
            buy: None,
        })
    }

    fn order(&self, side: Side, price: Price, time_in_force: TimeInForce, shares: i64) -> NewOrder {
        NewOrder {
            symbol: self.symbol,
            board: Board::Regular,
            side,
            kind: TradeKind::Cash,
            price: OrderPrice::Limit(price),
            time_in_force,
            qty: Shares::new(shares),
        }
    }
}

impl Strategy for Breakout {
    fn on_day_start(&mut self, ctx: &mut Context<'_>) {
        // Orders do not carry over, so neither does what the strategy knows of them.
        self.buy = None;
        // Orders for the closing auction are collected from 13:25.
        ctx.set_timer(ctx.date().at_taipei(13, 25, 0))
            .expect("13:25 is later today");
    }

    fn on_market(&mut self, ctx: &mut Context<'_>, message: &Message) {
        // Every message of every replayed book arrives here: leave early.
        if self.buy.is_some()
            || message.symbol != self.symbol
            || message.feed.board() != Board::Regular
        {
            return;
        }
        let Some(state) = ctx.market().book(self.symbol, Board::Regular) else {
            return;
        };
        if state.mode(ctx.now()) != Mode::Continuous {
            return;
        }
        let broke_out = state
            .last_trade()
            .is_some_and(|trade| trade.price >= self.trigger);
        let lot = ctx
            .market()
            .instrument(self.symbol)
            .and_then(|instrument| instrument.shares_per_lot);
        let (true, Some(ask), Some(lot)) = (broke_out, state.book().ask().best(), lot) else {
            return;
        };
        // Take the best ask or nothing: what an IOC order does not trade at once is
        // cancelled.
        let order = self.order(Side::Buy, ask.price, TimeInForce::Ioc, i64::from(lot));
        self.buy = ctx.submit(order);
    }

    fn on_order_event(&mut self, _ctx: &mut Context<'_>, event: &OrderEvent) {
        // Reports arrive `latency.report` after they happen. A rejection says why
        // (not enough cash, an instrument that cannot be traded, ...).
        if let OrderEventKind::Rejected { request, reason } = event.kind {
            eprintln!(
                "order {} {} rejected: {}",
                event.order.0,
                request.as_str(),
                reason.as_str()
            );
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
        // The account as the reports received so far show it, in whole board lots.
        let lot = i64::from(lot);
        let shares = ctx.account().position(self.symbol).cash / lot * lot;
        if shares > 0 {
            // At the lower limit the order trades at whatever the closing price is.
            let order = self.order(Side::Sell, limit_down, TimeInForce::Rod, shares);
            ctx.submit(order);
        }
    }
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.add(ENTRY);
    registry
}

fn main() -> ExitCode {
    osmium_cli::main(&registry())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use osmium_fixture::sample;

    use super::registry;

    #[test]
    fn the_breakout_is_bought_and_sold_at_the_close() {
        let tree = sample::tree("cli-breakout");
        let dir = tree.path().join("run");
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("run.toml");
        let params = format!("symbol = \"{}\"\ntrigger = \"100.5\"", sample::SYMBOL);
        fs::write(&config, sample::config(&tree, "breakout", &params)).unwrap();
        let results = dir.join("results");
        let arguments = [
            "run".to_owned(),
            config.to_str().unwrap().to_owned(),
            "--out".to_owned(),
            results.to_str().unwrap().to_owned(),
        ];
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let status = osmium_cli::execute(&registry(), &arguments, &mut out, &mut err);
        assert_eq!(status, 0, "{}", String::from_utf8_lossy(&err));

        let events = fs::read_to_string(results.join("order_events.csv")).unwrap();
        let fills: Vec<&str> = events
            .lines()
            .filter(|line| line.contains(",Filled,"))
            .collect();
        // Each day: the trade at 100.5 triggers a purchase at the best ask, and the
        // lot is sold at the closing price.
        assert_eq!(
            fills,
            [
                "1,2026-10-15T09:00:20.001100+08:00,Filled,101,1000,,Aggressive,,,,",
                "2,2026-10-15T13:30:00.000000+08:00,Filled,100.5,1000,,AuctionBetter,TWSE_F6,107,,",
                "3,2026-10-16T09:00:20.001100+08:00,Filled,101,1000,,Aggressive,,,,",
                "4,2026-10-16T13:30:00.000000+08:00,Filled,100.5,1000,,AuctionBetter,TWSE_F6,207,,",
            ]
        );
    }
}
