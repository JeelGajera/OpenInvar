# openinvar-store

Persistence and hot-cache crate for OpenInvar.

`openinvar-store` persists graph snapshots into SQLite and provides an in-memory hot query cache for fast repeated lookups.

## Responsibilities

- Save/load `InvarGraph` snapshots (`GraphStore`)
- Snapshot conversion (`GraphSnapshot`)
- Revision snapshots, and the retention sweep over them
- In-memory cache for query results (`HotQueryCache`)

## Main APIs

- `GraphStore::open(path)`
- `GraphStore::save_graph(&graph)` / `load_graph()`
- `GraphStore::save_revision(rev, &snapshot)` / `load_revision(rev)` / `list_revisions()` / `prune_revisions(keep)`
- `HotQueryCache::{new, put, get, invalidate, clear, stats}`

## Minimal usage

```rust
use std::path::Path;
use openinvar_store::GraphStore;

let store = GraphStore::open(Path::new(".openinvar/graph.db"))?;
let _graph = store.load_graph()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`open` also accepts a directory, and puts `graph.db` inside it.

## Notes

- The store is a **file**, `.openinvar/graph.db`. Releases before 0.3.0 wrote a RocksDB *directory* at `.openinvar/db`; `open` reports one of those as `StoreError::LegacyStore` rather than deleting it or writing beside it. The graph is derived data — re-run `openinvar analyze`.
- `RocksGraphStore` remains as a type alias for `GraphStore`, and `openinvar_store::rocksdb` as a module alias for `openinvar_store::sqlite`, for one release. Both go away in 1.0.0.
- Snapshot encoding is a hand-written length-prefixed binary format, not JSON, and is round-trip safe for literal backslash sequences. It is storage-agnostic — moving from RocksDB to SQLite did not change a byte of it.
- Every query that feeds output carries an explicit `ORDER BY`. SQLite guarantees no row order without one, and `list_revisions` ordering is what `openinvar diff` and retention rest on.
- Storage concerns are isolated here; graph/query logic stays in `openinvar-core`.
