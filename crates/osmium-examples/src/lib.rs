//! Small strategies that show how a strategy is written (`docs/guide/`). They are
//! examples of the interface, not of how to make money.

mod buy_and_hold;
mod open_to_close;
mod passive_quote;

pub use buy_and_hold::BuyAndHold;
pub use open_to_close::OpenToClose;
pub use passive_quote::PassiveQuote;

use osmium_strategy::Registry;

/// Adds every example to `registry`.
pub fn register(registry: &mut Registry) {
    registry.add(buy_and_hold::ENTRY);
    registry.add(open_to_close::ENTRY);
    registry.add(passive_quote::ENTRY);
}
