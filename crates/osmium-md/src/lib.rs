//! Read-only access to marketdata (`/srv/md/v1`): layout, schema and manifest checks,
//! and each feed's messages in exchange order as exact integers.

mod decode;
mod error;
mod feed_day;
mod instruments;
mod layout;
mod manifest;
mod order;
mod parallel;
mod parquet_io;
mod reference;
mod stream;

/// The version of the decoding of marketdata into messages
/// (`docs/reference/marketdata.md`). It changes when the same files decode differently.
pub const DECODER_VERSION: u32 = 3;

pub use error::MdError;
pub use feed_day::{FeedDay, FeedDayIdentity, Messages, MissingLotSize};
pub use instruments::DayInstruments;
pub use layout::{DEFAULT_MD_ROOT, MdRoot, Table};
pub use manifest::{DayManifest, ManifestEntry, SCHEMA_VERSION};
pub use reference::{
    CalendarDay, CorporateActions, CreditQuotas, DailyQuotes, MarketEvents, ReferenceFile,
    TradingCalendar,
};
