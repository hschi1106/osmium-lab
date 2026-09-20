mod cursor;
pub mod equity_quote;
mod provider;
mod query;
mod sync;
mod transport;

pub mod taifex;
pub mod tpex;
pub mod twse;

pub use cursor::{
    CursorCheckpoint, CursorError, CursorState, CursorStateMachine, PendingPage, TeralionCursor,
    TeralionRequest, TeralionTransport, TransportError,
};
pub use data_sync::PageCommitReceipt;
pub use provider::{
    MappingSelectionError, NormalizerSelectionError, PartitionNormalizerConfig, ProviderError,
    SourceSyncResult, TeralionProvider, normalizer_config_for, normalizer_mapping_for,
    prepare_cache,
};
pub use query::{
    ArchiveKind, ArchiveMarket, ArchiveTimestamp, QueryError, TERALION_INTERFACE_VERSION,
    TeralionCredential, TeralionQuery,
};
pub use sync::{PagedSyncReport, SyncError, TeralionSync};
pub use transport::{FeedArchiveTransport, TERALION_BASE_URL};
