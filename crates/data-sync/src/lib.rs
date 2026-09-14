mod cache;
mod cursor;
mod partition;
mod query;
mod source_adapter;
mod storage;
mod sync;
mod transport;
mod verify;

pub use cache::{
    CACHE_FORMAT_VERSION, CacheBuildError, CacheBuilder, CacheCatalogError, CacheDescriptor,
    CacheReadError, CacheReader, CacheRecord, LocalCacheFactory, PartitionCacheCatalog,
    PartitionCacheEntry, PartitionCacheInspection, PartitionNormalizerConfig, PublishedCache,
};
pub use cursor::{
    CursorCheckpoint, CursorError, CursorState, CursorStateMachine, PageCommitReceipt, PendingPage,
    TeralionCursor, TeralionRequest, TeralionTransport, TransportError,
};
pub use partition::{
    PARTITION_LAYOUT_VERSION, PARTITION_MANIFEST_FILE, PartitionRepositoryError,
    PartitionedSourceRepository, SourcePartitionManifest, cache_instrument_root,
    cache_partition_root, partition_root,
};
pub use query::{
    ArchiveKind, ArchiveMarket, ArchiveTimestamp, QueryError, SanitizedQueryIdentity,
    TERALION_INTERFACE_VERSION, TeralionCredential, TeralionQuery,
};
pub use source_adapter::{
    MappingIdentityError, MappingSelectionError, NormalizerMappingIdentity,
    NormalizerSelectionError, SourceAdapterError, SourceAdapterRuntime, SourceSyncResult,
    normalizer_config_for, normalizer_mapping_for,
};
pub use storage::{
    CompressionPolicy, ObjectKind, PageMetadata, PublishedRevision, SourceManifest, StagedObject,
    StagedPage, StagingError, StagingRevision, ZSTD_COMPRESSION_LEVEL,
};
pub use sync::{PagedSyncReport, SyncError, TeralionSync};
pub use transport::{FeedArchiveTransport, TERALION_BASE_URL};
pub use verify::{
    LocalSourceRepository, SourceInspection, SyncDisposition, VerificationError, VerificationReport,
};
