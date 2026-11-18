//! The deterministic order of one trading day's events (`docs/reference/replay.md`):
//! the total order of market data messages across feeds, and the timeline that
//! interleaves them with simulated order traffic, reports and timers.

mod merge;
mod timeline;

pub use merge::{MergedFeeds, MessageOrder};
pub use timeline::{Event, ScheduledClass, Timeline};
