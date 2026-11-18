//! Matches simulated orders against the real market messages of a trading day
//! (`docs/reference/matching.md`).
//!
//! Simulated orders never change the real market: prices, quantities and the book are
//! replayed as recorded, and an order only receives what it could certainly have had
//! under that assumption.

mod band;
mod engine;
mod instrument;
mod matcher;

/// The version of the matching rules (`docs/reference/matching.md`). It changes when
/// the same orders against the same messages give different order events.
pub const RULES_VERSION: u32 = 1;

pub use band::{PriceBand, Reference};
pub use engine::{AuctionMarginal, Diagnostics, Engine};
pub use instrument::{BookSpec, InstrumentClass, PriceLimits};
pub use matcher::{QueueAhead, RestingOrder};
