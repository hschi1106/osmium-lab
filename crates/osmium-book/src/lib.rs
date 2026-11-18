//! The disclosed state of one order book and its trading mode
//! (`docs/reference/matching.md` section 4).
//!
//! A book is one instrument on one board. Its state is what the exchange has disclosed
//! so far: nothing here looks ahead, and nothing here knows about simulated orders.

mod method;
mod state;

pub use method::{Method, MethodEvidence};
pub use state::{BookId, BookState, Collecting, Indicative, Mode, is_auction_result};
