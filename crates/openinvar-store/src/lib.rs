pub mod cache;
pub mod revision;
pub mod sqlite;

/// The former module path, kept for one release.
///
/// The storage engine moved from RocksDB to SQLite; the snapshot encoding did
/// not, so a consumer that only ever named types through this path is
/// unaffected. Removed in 1.0.0.
pub use sqlite as rocksdb;

pub use cache::{CacheStats, HotQueryCache};
pub use sqlite::{GraphSnapshot, GraphStore, RevisionEntry, RocksGraphStore, StoreError};
