//! Read-only access to marketdata (`/srv/md/v1`): layout, schema and manifest checks,
//! and each feed's messages in exchange order as exact integers.

mod decode;
mod error;
mod feed_day;
mod instruments;
mod layout;
mod manifest;
mod parquet_io;

pub use error::MdError;
pub use feed_day::{FeedDay, FeedDayIdentity, Messages, MissingLotSize};
pub use instruments::DayInstruments;
pub use layout::{DEFAULT_MD_ROOT, MdRoot, Table};
pub use manifest::{DayManifest, ManifestEntry, SCHEMA_VERSION};
