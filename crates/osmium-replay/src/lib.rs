//! The deterministic order of one trading day's events (`docs/reference/replay.md`):
//! the total order of market data messages across feeds, and the timeline that
//! interleaves them with simulated order traffic, reports and timers.

mod merge;
mod timeline;

/// The version of the event order (`docs/reference/replay.md` sections 2 and 3).
pub const ORDERING_VERSION: u32 = 1;

pub use merge::{MergedFeeds, MessageOrder};
pub use timeline::{Event, ScheduledClass, Timeline};
