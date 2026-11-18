//! The interface between a strategy and a run (`docs/reference/strategy.md`).
//!
//! A strategy is called with what it could know at the replay clock: market messages
//! that have reached it, reports that have reached it, and its own timers. It answers
//! with orders, changes and timers through the [`Context`].

mod context;
mod params;
mod registry;
mod view;

pub use context::{Command, Context, Outbox, TimerError, TimerId};
pub use params::{ParamError, Params, Value};
pub use registry::{Registry, StrategyEntry};
pub use view::{Account, Market};

use osmium_types::{Message, NewOrder, OrderEvent, OrderId};

/// A trading strategy. Every method has an empty default.
///
/// The methods of one trading day are called in this order: `on_forced_order` for each
/// order the account is forced to send, `on_day_start`, then `on_market`,
/// `on_order_event` and `on_timer` along the timeline, and `on_day_end` after the
/// day's settlement is computed.
pub trait Strategy {
    /// The trading day opens, at 08:30:00, before any market message of the day.
    fn on_day_start(&mut self, ctx: &mut Context<'_>) {
        let _ = ctx;
    }

    /// The broker sends `order` for the account before the open (a margin call, an
    /// expired position, a short position past its last cover day, or shares sold
    /// first and not bought back). Its reports arrive through `on_order_event`.
    fn on_forced_order(&mut self, ctx: &mut Context<'_>, id: OrderId, order: &NewOrder) {
        let _ = (ctx, id, order);
    }

    /// A market message reaches the strategy. The market view already includes it.
    fn on_market(&mut self, ctx: &mut Context<'_>, message: &Message) {
        let _ = (ctx, message);
    }

    /// A report on one of the account's orders reaches the strategy. The account view
    /// already includes it.
    fn on_order_event(&mut self, ctx: &mut Context<'_>, event: &OrderEvent) {
        let _ = (ctx, event);
    }

    /// A timer set with [`Context::set_timer`] fires.
    fn on_timer(&mut self, ctx: &mut Context<'_>, timer: TimerId) {
        let _ = (ctx, timer);
    }

    /// The day is over and settled. Orders and timers requested here are refused:
    /// nothing carries over to the next day.
    fn on_day_end(&mut self, ctx: &mut Context<'_>) {
        let _ = ctx;
    }
}
