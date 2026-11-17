//! Reads marketdata (`/srv/md/v1`) parquet and maps it to osmium `DomainEvent`s.
//!
//! Within a feed, messages replay in exchange `seq` order. Each event's
//! `observed_time` is the running maximum `exch_ts` over the whole feed up to its
//! `seq`, so ordering rule v5 merges feeds and symbols without look-ahead. See
//! ADR-0007.

mod clock;
mod day;
mod error;
mod feed;
mod instruments;
mod layout;
mod manifest;
mod mapper;
mod parquet_io;
mod rows;

pub use day::{FeedDay, FeedDayIdentity, SymbolEventStream};
pub use error::MdError;
pub use feed::{Board, Feed};
pub use instruments::{InstrumentRecord, Instruments};
pub use layout::{DEFAULT_MD_ROOT, MdRoot, Table};
pub use manifest::{DayManifest, ManifestEntry};
