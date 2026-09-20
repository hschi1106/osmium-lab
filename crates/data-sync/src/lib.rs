mod cache;
mod identity;
mod mapping;
mod partition;
mod storage;
mod verify;

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

pub(crate) fn decode_hex_32(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = decode_nibble(pair[0])? << 4 | decode_nibble(pair[1])?;
    }
    Some(output)
}

fn decode_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub use cache::{
    CACHE_FORMAT_VERSION, CacheBuildError, CacheBuilder, CacheCatalogError, CacheDescriptor,
    CacheReadError, CacheReader, CacheRecord, LocalCacheFactory, PartitionCacheCatalog,
    PartitionCacheEntry, PartitionCacheInspection, PublishedCache,
};
pub use identity::SourceRequestIdentity;
pub use mapping::{MappingIdentityError, NormalizerMappingIdentity};
pub use partition::{
    PARTITION_LAYOUT_VERSION, PARTITION_MANIFEST_FILE, PartitionRepositoryError,
    PartitionedSourceRepository, SourcePartitionManifest, cache_instrument_root,
    cache_partition_root, partition_root,
};
pub use storage::{
    CompressionPolicy, ObjectKind, PageCommitReceipt, PageMetadata, PublishedRevision,
    SourceManifest, SourcePage, StagedObject, StagedPage, StagingError, StagingRevision,
    ZSTD_COMPRESSION_LEVEL,
};
pub use verify::{
    LocalSourceRepository, SourceInspection, SyncDisposition, VerificationError, VerificationReport,
};
