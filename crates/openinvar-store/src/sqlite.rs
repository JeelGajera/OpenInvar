//! Graph persistence, on SQLite.
//!
//! The storage engine is deliberately boring. What matters is above it: the
//! snapshot encoder further down this file turns a graph into a
//! length-prefixed little-endian blob, and that encoding is storage-agnostic —
//! SQLite holds the same bytes RocksDB used to, in a `BLOB` column.
//!
//! Two tables replace what were two column families: `graph` holds the single
//! working graph, `revisions` holds snapshots keyed by revision. Keeping them
//! apart still means the retention sweep cannot reach the working graph.

use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::{Language, ReExportEntry, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use openinvar_core::resolver::{AliasEntry, AliasScope};
use rusqlite::{params, Connection, OptionalExtension};

/// The single row id in `graph`. The table is constrained to it.
const GRAPH_ROW_ID: i64 = 1;

/// What the database is called when [`GraphStore::open`] is handed a directory.
const DEFAULT_DB_FILENAME: &str = "graph.db";

/// Schema version, in SQLite's own `PRAGMA user_version`.
///
/// This versions how revisions are keyed and indexed, not the bytes of a
/// snapshot — [`SNAPSHOT_VERSION`] does that. A mismatch drops the revisions
/// table and starts over: a stale index pointing at snapshots that no longer
/// parse is exactly the silent corruption this is here to prevent, and a
/// revision snapshot is a cache of something reproducible from git.
const SCHEMA_VERSION: i32 = 1;

/// Bumped to 3 for the per-edge resolution byte.
///
/// A version 1, 2 or 3 snapshot carries no assertion counts, and is read back
/// as having counted none — so an audit against an old base declines to
/// conclude rather than reporting every test as emptied.
///
/// A version 1 or 2 snapshot carries no resolution, and is read back as
/// `Structural` — the weaker value — so a graph written before this existed is
/// never treated as gate-safe on the strength of a field it does not have.
/// Re-analysing rewrites it at the current version.
///
/// A snapshot before version 6 carries no split of the unbound counts, and is
/// read back as having classified none of them. That reads as "nothing here
/// vouched for any of these" rather than as "none of them are external", which
/// is the same direction every other default above leans: an old snapshot
/// understates what is explained rather than overstating it.
const SNAPSHOT_VERSION: u8 = 6;

#[derive(Debug)]
pub enum StoreError {
    Database(String),
    Serialization(String),
    SnapshotNotFound,
    /// A store directory written by a release that used RocksDB.
    ///
    /// Carried as its own variant rather than folded into `Database` because
    /// the remedy is specific and the caller should be able to print it: the
    /// graph is derived data, so the fix is to re-analyse, not to migrate.
    /// Nothing here deletes the old directory — that is the user's to do.
    LegacyStore(PathBuf),
}

impl Display for StoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(err) => write!(f, "database error: {err}"),
            Self::Serialization(err) => write!(f, "serialization error: {err}"),
            Self::SnapshotNotFound => write!(f, "snapshot not found"),
            Self::LegacyStore(path) => write!(
                f,
                "the store format changed in this release; run 'openinvar analyze' to rebuild \
                 (the old store at {} is no longer read, and can be deleted)",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err.to_string())
    }
}

/// One stored revision, and when it was stored relative to the others.
///
/// The sequence is a counter rather than a timestamp. Retention has to order
/// revisions, and a wall clock makes that ordering depend on the machine — two
/// snapshots written inside the same clock tick would be unordered, and a
/// clock adjustment would reorder history. A counter is monotonic by
/// construction, which is the only property retention needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEntry {
    pub revision: String,
    pub sequence: u64,
}

#[derive(Debug, Clone)]
pub struct GraphSnapshot {
    pub symbols: Vec<Symbol>,
    pub relationships: Vec<Relationship>,
    pub alias_chains: Vec<(String, Vec<AliasEntry>)>,
    pub file_reexports: Vec<(String, Vec<ReExportEntry>)>,
    /// Recognised assertions per symbol, sorted by symbol id.
    pub assertions: Vec<(String, u32)>,
    /// Per file, whether this build counted assertions in its language.
    ///
    /// Stored alongside the counts because a count of nothing is ambiguous
    /// without it, and a snapshot written before version 4 carries neither —
    /// which is read back as "nothing was counted", so a detector declines to
    /// conclude rather than reporting every test as emptied.
    pub assertions_counted: Vec<(String, bool)>,
    /// References each file's adapter could not place, by file.
    pub unbound_references: Vec<(String, u32)>,
    /// Of those, the ones the adapter could show lie outside the tree, by file.
    pub unbound_outside_repository: Vec<(String, u32)>,
}

pub struct GraphStore {
    conn: Connection,
}

/// The former name of [`GraphStore`], kept for one release.
///
/// Renaming the type and rewriting its fifty-six call sites in the same change
/// would bury a storage-engine swap in a diff that mostly renames things. The
/// alias goes away in 1.0.0.
pub type RocksGraphStore = GraphStore;

/// Is this a store directory written by a release that used RocksDB?
///
/// `CURRENT` is RocksDB's own marker — it names the live manifest and is
/// written by every RocksDB database — so testing for it distinguishes an old
/// store from an ordinary directory without a false positive on either. A bare
/// `is_dir()` would reject any directory, which is a legitimate thing to hand
/// this function.
pub fn is_legacy_rocksdb_store(path: &Path) -> bool {
    path.is_dir() && path.join("CURRENT").is_file()
}

impl GraphStore {
    /// Open — or create — the store at `path`.
    ///
    /// `path` is normally the database file. A *directory* is also accepted,
    /// and means "put the database inside it" — the shape every caller used
    /// when the store was RocksDB, and what the conformance tests still pass.
    ///
    /// The one directory that is refused is a RocksDB store, recognised by the
    /// `CURRENT` file it always writes. That is reported as
    /// [`StoreError::LegacyStore`] rather than deleted or quietly ignored:
    /// silently removing a directory a user pointed us at is not a thing a
    /// tool should do on their behalf, and writing a new database beside the
    /// old one would leave them wondering where their revisions went.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if is_legacy_rocksdb_store(path) {
            return Err(StoreError::LegacyStore(path.to_path_buf()));
        }

        let file = if path.is_dir() {
            path.join(DEFAULT_DB_FILENAME)
        } else {
            path.to_path_buf()
        };

        if let Some(parent) = file.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| StoreError::Database(err.to_string()))?;
            }
        }

        let conn = Connection::open(&file)?;

        // WAL so a `watch` process and a hook invocation can hold the store at
        // the same time — a real scenario, and the default rollback journal
        // makes a reader and a writer exclude each other. `synchronous=NORMAL`
        // is the matching trade: a power loss can cost the most recent commit,
        // and the most recent commit is a cache of something re-derivable in
        // under a second.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let store = Self { conn };
        store.create_schema()?;
        store.reconcile_schema_version()?;
        Ok(store)
    }

    fn create_schema(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS graph (
                 id       INTEGER PRIMARY KEY CHECK (id = 1),
                 snapshot BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS revisions (
                 revision   TEXT PRIMARY KEY,
                 sequence   INTEGER NOT NULL,
                 created_at INTEGER NOT NULL,
                 snapshot   BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_revisions_sequence
                 ON revisions(sequence);",
        )?;
        Ok(())
    }

    /// Drop the revisions table if it was written by a different layout.
    ///
    /// Reindexing costs one analyse; misreading a stale index costs a wrong
    /// answer from a command whose whole purpose is to be trusted. A brand-new
    /// database reads `user_version` 0 and is simply stamped.
    fn reconcile_schema_version(&self) -> Result<(), StoreError> {
        let stored: i32 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;

        if stored == SCHEMA_VERSION {
            return Ok(());
        }
        if stored != 0 {
            self.conn.execute("DELETE FROM revisions", [])?;
        }
        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    pub fn save_graph(&self, graph: &InvarGraph) -> Result<(), StoreError> {
        let snapshot = GraphSnapshot::from_graph(graph)?;
        self.save_snapshot(&snapshot)
    }

    pub fn load_graph(&self) -> Result<InvarGraph, StoreError> {
        let snapshot = self.load_snapshot()?;
        snapshot.into_graph()
    }

    pub fn save_snapshot(&self, snapshot: &GraphSnapshot) -> Result<(), StoreError> {
        let bytes = snapshot.to_bytes()?;
        self.conn.execute(
            "INSERT INTO graph (id, snapshot) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET snapshot = excluded.snapshot",
            params![GRAPH_ROW_ID, bytes],
        )?;
        Ok(())
    }

    pub fn load_snapshot(&self) -> Result<GraphSnapshot, StoreError> {
        let bytes: Vec<u8> = self
            .conn
            .query_row(
                "SELECT snapshot FROM graph WHERE id = ?1",
                params![GRAPH_ROW_ID],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::SnapshotNotFound)?;

        GraphSnapshot::from_bytes(&bytes)
    }

    // ── revisions ────────────────────────────────────────────

    /// Store a graph under a revision name, replacing any snapshot already
    /// there.
    ///
    /// Re-analysing the same revision overwrites rather than accumulating: two
    /// snapshots of one revision cannot both be right, and keeping the older
    /// one would let `diff` answer from a graph the working tree no longer
    /// matches. The sequence advances on overwrite too, so re-recording a
    /// revision makes it the most recent for retention.
    pub fn save_revision(
        &self,
        revision: &str,
        snapshot: &GraphSnapshot,
    ) -> Result<(), StoreError> {
        let sequence = self.next_sequence()?;
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        self.conn.execute(
            "INSERT INTO revisions (revision, sequence, created_at, snapshot)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(revision) DO UPDATE SET
                 sequence   = excluded.sequence,
                 created_at = excluded.created_at,
                 snapshot   = excluded.snapshot",
            params![revision, sequence as i64, created_at, snapshot.to_bytes()?],
        )?;
        Ok(())
    }

    pub fn load_revision(&self, revision: &str) -> Result<GraphSnapshot, StoreError> {
        let bytes: Vec<u8> = self
            .conn
            .query_row(
                "SELECT snapshot FROM revisions WHERE revision = ?1",
                params![revision],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::SnapshotNotFound)?;

        GraphSnapshot::from_bytes(&bytes)
    }

    /// Every stored revision, most recently written first.
    ///
    /// The `ORDER BY` is not decoration. SQLite guarantees no row order
    /// without one, so an unordered `SELECT` here would make `openinvar diff`
    /// and the retention sweep depend on the query planner. Ties break on the
    /// revision name so the order is total; two revisions can never share a
    /// sequence, so the tie break exists to make the ordering provably
    /// deterministic rather than to resolve a case that occurs.
    pub fn list_revisions(&self) -> Result<Vec<RevisionEntry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT revision, sequence FROM revisions
             ORDER BY sequence DESC, revision ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RevisionEntry {
                revision: row.get(0)?,
                sequence: row.get::<_, i64>(1)? as u64,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn delete_revision(&self, revision: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM revisions WHERE revision = ?1",
            params![revision],
        )?;
        Ok(())
    }

    /// Keep the `keep` most recently written revisions, dropping the rest.
    ///
    /// Returns what it removed, in the order removed, so a caller can report
    /// it rather than deleting silently. `keep` of zero removes everything,
    /// which is what a caller asking to keep nothing means.
    ///
    /// The list is read first and deleted by name rather than expressed as one
    /// `DELETE ... WHERE revision NOT IN (...)`, because the caller is owed
    /// the names in the order they went, and that ordering comes from
    /// [`Self::list_revisions`] rather than from whatever order a delete
    /// happens to visit rows in.
    pub fn prune_revisions(&self, keep: usize) -> Result<Vec<String>, StoreError> {
        let stale: Vec<String> = self
            .list_revisions()?
            .into_iter()
            .skip(keep)
            .map(|entry| entry.revision)
            .collect();

        for revision in &stale {
            self.delete_revision(revision)?;
        }
        Ok(stale)
    }

    /// The next revision sequence number.
    ///
    /// Derived from the table rather than held in a counter row: the maximum
    /// sequence plus one is the same number the counter would hold, and it
    /// cannot drift out of step with the rows it orders. Pruning lowers the
    /// maximum, which is harmless — the only property retention needs is that
    /// a newer write sorts above every row present when it was made.
    fn next_sequence(&self) -> Result<u64, StoreError> {
        let current: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM revisions",
            [],
            |row| row.get(0),
        )?;
        Ok((current as u64).saturating_add(1))
    }
}

impl GraphSnapshot {
    pub fn from_graph(graph: &InvarGraph) -> Result<Self, StoreError> {
        let mut symbols: Vec<Symbol> = graph
            .symbols
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        symbols.sort_by(|a, b| a.id.cmp(&b.id));

        let mut relationships = Vec::new();
        for edge_id in graph.graph.edge_indices() {
            let (source_idx, target_idx) = graph
                .graph
                .edge_endpoints(edge_id)
                .ok_or_else(|| StoreError::Serialization("missing edge endpoints".to_string()))?;
            let from = graph
                .graph
                .node_weight(source_idx)
                .cloned()
                .ok_or_else(|| StoreError::Serialization("missing source node".to_string()))?;
            let to = graph
                .graph
                .node_weight(target_idx)
                .cloned()
                .ok_or_else(|| StoreError::Serialization("missing target node".to_string()))?;
            let meta = graph
                .graph
                .edge_weight(edge_id)
                .ok_or_else(|| StoreError::Serialization("missing edge metadata".to_string()))?;

            relationships.push(Relationship {
                from,
                to,
                kind: meta.kind.clone(),
                alias: meta.alias.clone(),
                properties_accessed: meta.properties_accessed.clone(),
                context: meta.context.clone(),
                file: meta.file.clone(),
                line: meta.line,
                resolution: meta.resolution,
            });
        }
        relationships.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.from.cmp(&b.from))
                .then(a.to.cmp(&b.to))
        });

        let mut alias_chains: Vec<(String, Vec<AliasEntry>)> = graph
            .alias_chains
            .iter()
            .map(|entry| {
                let mut aliases = entry.value().clone();
                aliases.sort_by(|a, b| {
                    a.defined_in_file
                        .cmp(&b.defined_in_file)
                        .then(a.alias_name.cmp(&b.alias_name))
                });
                (entry.key().clone(), aliases)
            })
            .collect();
        alias_chains.sort_by(|a, b| a.0.cmp(&b.0));

        let mut file_reexports: Vec<(String, Vec<ReExportEntry>)> = graph
            .file_reexports
            .iter()
            .map(|entry| {
                let mut re_exports = entry.value().clone();
                re_exports.sort_by(|a, b| {
                    a.exported_name
                        .cmp(&b.exported_name)
                        .then(a.source_module.cmp(&b.source_module))
                });
                (entry.key().clone(), re_exports)
            })
            .collect();
        file_reexports.sort_by(|a, b| a.0.cmp(&b.0));

        // Sorted explicitly: both come from a DashMap, whose iteration order
        // varies per process, and these bytes are compared for equality.
        let mut assertions: Vec<(String, u32)> = graph
            .assertions
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();
        assertions.sort_by(|a, b| a.0.cmp(&b.0));

        let mut assertions_counted: Vec<(String, bool)> = graph
            .assertions_counted
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();
        assertions_counted.sort_by(|a, b| a.0.cmp(&b.0));

        // Sorted, like every other collection that reaches these bytes: a
        // `DashMap` iterates in whatever order it likes, and two snapshots of
        // one graph have to be byte-identical.
        let mut unbound_references: Vec<(String, u32)> = graph
            .unbound_references
            .iter()
            .map(|e| (e.key().clone(), *e.value()))
            .collect();
        unbound_references.sort_by(|a, b| a.0.cmp(&b.0));

        let mut unbound_outside_repository: Vec<(String, u32)> = graph
            .unbound_outside_repository
            .iter()
            .map(|e| (e.key().clone(), *e.value()))
            .collect();
        unbound_outside_repository.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(Self {
            symbols,
            relationships,
            alias_chains,
            file_reexports,
            assertions,
            assertions_counted,
            unbound_references,
            unbound_outside_repository,
        })
    }

    pub fn into_graph(self) -> Result<InvarGraph, StoreError> {
        let mut graph = InvarGraph::new();

        for symbol in self.symbols {
            graph.add_symbol(symbol);
        }

        for relationship in &self.relationships {
            graph.add_relationship(relationship);
        }

        for (canonical_id, aliases) in self.alias_chains {
            graph.alias_chains.insert(canonical_id, aliases);
        }

        for (file, re_exports) in self.file_reexports {
            graph.file_reexports.insert(file, re_exports);
        }

        for (symbol, count) in self.assertions {
            graph.assertions.insert(symbol, count);
        }

        for (file, count) in self.unbound_references {
            graph.unbound_references.insert(file, count);
        }
        for (file, count) in self.unbound_outside_repository {
            graph.unbound_outside_repository.insert(file, count);
        }
        for (file, counted) in self.assertions_counted {
            graph.assertions_counted.insert(file, counted);
        }

        Ok(graph)
    }

    fn to_bytes(&self) -> Result<Vec<u8>, StoreError> {
        let mut out = Vec::new();

        write_u8(&mut out, SNAPSHOT_VERSION);

        write_u32(&mut out, self.symbols.len() as u32);
        for symbol in &self.symbols {
            write_string(&mut out, &symbol.id)?;
            write_string(&mut out, &symbol.name)?;
            write_u8(&mut out, symbol_kind_to_u8(&symbol.kind));
            write_u8(&mut out, language_to_u8(&symbol.language));
            write_string(&mut out, &symbol.file)?;
            write_u32(&mut out, symbol.line_start);
            write_u32(&mut out, symbol.line_end);
            write_optional_string(&mut out, symbol.signature.as_deref())?;
        }

        write_u32(&mut out, self.relationships.len() as u32);
        for relationship in &self.relationships {
            write_string(&mut out, &relationship.from)?;
            write_string(&mut out, &relationship.to)?;
            write_u8(&mut out, relationship_kind_to_u8(&relationship.kind));
            write_optional_string(&mut out, relationship.alias.as_deref())?;
            write_u32(&mut out, relationship.properties_accessed.len() as u32);
            for prop in &relationship.properties_accessed {
                write_string(&mut out, prop)?;
            }
            write_string(&mut out, &relationship.context)?;
            write_string(&mut out, &relationship.file)?;
            write_u32(&mut out, relationship.line);
            write_u8(&mut out, resolution_to_u8(&relationship.resolution));
        }

        write_u32(&mut out, self.alias_chains.len() as u32);
        for (canonical, entries) in &self.alias_chains {
            write_string(&mut out, canonical)?;
            write_u32(&mut out, entries.len() as u32);
            for entry in entries {
                write_string(&mut out, &entry.alias_name)?;
                write_string(&mut out, &entry.defined_in_file)?;
                write_u8(&mut out, alias_scope_to_u8(&entry.scope));
            }
        }

        write_u32(&mut out, self.file_reexports.len() as u32);
        for (file, entries) in &self.file_reexports {
            write_string(&mut out, file)?;
            write_u32(&mut out, entries.len() as u32);
            for entry in entries {
                write_string(&mut out, &entry.exported_name)?;
                write_string(&mut out, &entry.source_module)?;
            }
        }

        // Version 4. Appended rather than interleaved, so the bytes an earlier
        // version wrote are still the bytes this one writes for the same graph.
        write_u32(&mut out, self.assertions.len() as u32);
        for (symbol, count) in &self.assertions {
            write_string(&mut out, symbol)?;
            write_u32(&mut out, *count);
        }

        write_u32(&mut out, self.assertions_counted.len() as u32);
        for (file, counted) in &self.assertions_counted {
            write_string(&mut out, file)?;
            write_u8(&mut out, u8::from(*counted));
        }

        write_u32(&mut out, self.unbound_references.len() as u32);
        for (file, count) in &self.unbound_references {
            write_string(&mut out, file)?;
            write_u32(&mut out, *count);
        }

        write_u32(&mut out, self.unbound_outside_repository.len() as u32);
        for (file, count) in &self.unbound_outside_repository {
            write_string(&mut out, file)?;
            write_u32(&mut out, *count);
        }

        Ok(out)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, StoreError> {
        let mut cursor = ByteCursor::new(bytes);

        let version = cursor.read_u8()?;
        if !(1..=SNAPSHOT_VERSION).contains(&version) {
            return Err(StoreError::Serialization(format!(
                "unsupported snapshot version: {version}"
            )));
        }

        let symbol_count = cursor.read_u32()? as usize;
        let mut symbols = Vec::with_capacity(symbol_count);
        for _ in 0..symbol_count {
            symbols.push(Symbol {
                id: cursor.read_string()?,
                name: cursor.read_string()?,
                kind: u8_to_symbol_kind(cursor.read_u8()?)?,
                language: u8_to_language(cursor.read_u8()?)?,
                file: cursor.read_string()?,
                line_start: cursor.read_u32()?,
                line_end: cursor.read_u32()?,
                signature: cursor.read_optional_string()?,
            });
        }

        let rel_count = cursor.read_u32()? as usize;
        let mut relationships = Vec::with_capacity(rel_count);
        for _ in 0..rel_count {
            let from = cursor.read_string()?;
            let to = cursor.read_string()?;
            let kind = u8_to_relationship_kind(cursor.read_u8()?)?;
            let alias = cursor.read_optional_string()?;
            let prop_count = cursor.read_u32()? as usize;
            let mut properties_accessed = Vec::with_capacity(prop_count);
            for _ in 0..prop_count {
                properties_accessed.push(cursor.read_string()?);
            }
            let context = cursor.read_string()?;
            let file = cursor.read_string()?;
            let line = cursor.read_u32()?;
            let resolution = if version >= 3 {
                resolution_from_u8(cursor.read_u8()?)
            } else {
                // Pre-3 snapshots predate the field. Defaulting to the weaker
                // value keeps an old graph from being read as gate-safe.
                Resolution::Structural
            };

            relationships.push(Relationship {
                from,
                to,
                kind,
                alias,
                properties_accessed,
                context,
                file,
                line,
                resolution,
            });
        }

        let alias_chain_count = cursor.read_u32()? as usize;
        let mut alias_chains = Vec::with_capacity(alias_chain_count);
        for _ in 0..alias_chain_count {
            let canonical = cursor.read_string()?;
            let entry_count = cursor.read_u32()? as usize;
            let mut entries = Vec::with_capacity(entry_count);
            for _ in 0..entry_count {
                entries.push(AliasEntry {
                    alias_name: cursor.read_string()?,
                    defined_in_file: cursor.read_string()?,
                    scope: u8_to_alias_scope(cursor.read_u8()?)?,
                });
            }
            alias_chains.push((canonical, entries));
        }

        let mut file_reexports = Vec::new();
        if version >= 2 {
            let reexport_file_count = cursor.read_u32()? as usize;
            file_reexports.reserve(reexport_file_count);
            for _ in 0..reexport_file_count {
                let file = cursor.read_string()?;
                let entry_count = cursor.read_u32()? as usize;
                let mut entries = Vec::with_capacity(entry_count);
                for _ in 0..entry_count {
                    entries.push(ReExportEntry {
                        exported_name: cursor.read_string()?,
                        source_module: cursor.read_string()?,
                    });
                }
                file_reexports.push((file, entries));
            }
        }

        let mut assertions = Vec::new();
        let mut assertions_counted = Vec::new();
        if version >= 4 {
            let count = cursor.read_u32()? as usize;
            assertions.reserve(count);
            for _ in 0..count {
                let symbol = cursor.read_string()?;
                assertions.push((symbol, cursor.read_u32()?));
            }

            let counted = cursor.read_u32()? as usize;
            assertions_counted.reserve(counted);
            for _ in 0..counted {
                let file = cursor.read_string()?;
                assertions_counted.push((file, cursor.read_u8()? != 0));
            }
        }

        // Version 5 added the unbound-reference counts. An older snapshot
        // simply has none, which reads as "this graph does not know" rather
        // than as zero — the same way `assertions` handles a version-3 file.
        let mut unbound_references = Vec::new();
        if version >= 5 {
            let count = cursor.read_u32()? as usize;
            unbound_references.reserve(count);
            for _ in 0..count {
                let file = cursor.read_string()?;
                unbound_references.push((file, cursor.read_u32()?));
            }
        }

        // Version 6 split those counts by why the reference did not bind. An
        // older snapshot has no split, which reads as "none of them were
        // classified" — not as "none of them were external".
        let mut unbound_outside_repository = Vec::new();
        if version >= 6 {
            let count = cursor.read_u32()? as usize;
            unbound_outside_repository.reserve(count);
            for _ in 0..count {
                let file = cursor.read_string()?;
                unbound_outside_repository.push((file, cursor.read_u32()?));
            }
        }

        if !cursor.is_at_end() {
            return Err(StoreError::Serialization(
                "trailing bytes found in snapshot".to_string(),
            ));
        }

        Ok(Self {
            symbols,
            relationships,
            alias_chains,
            file_reexports,
            assertions,
            assertions_counted,
            unbound_references,
            unbound_outside_repository,
        })
    }
}

struct ByteCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, StoreError> {
        if self.pos >= self.bytes.len() {
            return Err(StoreError::Serialization(
                "unexpected EOF reading u8".to_string(),
            ));
        }
        let v = self.bytes[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn read_u32(&mut self) -> Result<u32, StoreError> {
        if self.pos + 4 > self.bytes.len() {
            return Err(StoreError::Serialization(
                "unexpected EOF reading u32".to_string(),
            ));
        }
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&self.bytes[self.pos..self.pos + 4]);
        self.pos += 4;
        Ok(u32::from_le_bytes(arr))
    }

    fn read_string(&mut self) -> Result<String, StoreError> {
        let len = self.read_u32()? as usize;
        if self.pos + len > self.bytes.len() {
            return Err(StoreError::Serialization(
                "unexpected EOF reading string".to_string(),
            ));
        }
        let slice = &self.bytes[self.pos..self.pos + len];
        self.pos += len;
        String::from_utf8(slice.to_vec())
            .map_err(|err| StoreError::Serialization(format!("invalid UTF-8 string: {err}")))
    }

    fn read_optional_string(&mut self) -> Result<Option<String>, StoreError> {
        let has = self.read_u8()?;
        if has == 0 {
            Ok(None)
        } else {
            Ok(Some(self.read_string()?))
        }
    }

    fn is_at_end(&self) -> bool {
        self.pos == self.bytes.len()
    }
}

fn write_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), StoreError> {
    let bytes = value.as_bytes();
    let len = u32::try_from(bytes.len())
        .map_err(|_| StoreError::Serialization("string too large".to_string()))?;
    write_u32(out, len);
    out.extend_from_slice(bytes);
    Ok(())
}

fn write_optional_string(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), StoreError> {
    match value {
        Some(value) => {
            write_u8(out, 1);
            write_string(out, value)
        }
        None => {
            write_u8(out, 0);
            Ok(())
        }
    }
}

fn symbol_kind_to_u8(kind: &SymbolKind) -> u8 {
    match kind {
        SymbolKind::Class => 1,
        SymbolKind::Interface => 2,
        SymbolKind::TypeAlias => 3,
        SymbolKind::Function => 4,
        SymbolKind::Method => 5,
        SymbolKind::Property => 6,
        SymbolKind::Variable => 7,
        SymbolKind::Module => 8,
        SymbolKind::Enum => 9,
        SymbolKind::EnumVariant => 10,
        SymbolKind::ExternalPackage => 11,
    }
}

fn u8_to_symbol_kind(input: u8) -> Result<SymbolKind, StoreError> {
    match input {
        1 => Ok(SymbolKind::Class),
        2 => Ok(SymbolKind::Interface),
        3 => Ok(SymbolKind::TypeAlias),
        4 => Ok(SymbolKind::Function),
        5 => Ok(SymbolKind::Method),
        6 => Ok(SymbolKind::Property),
        7 => Ok(SymbolKind::Variable),
        8 => Ok(SymbolKind::Module),
        9 => Ok(SymbolKind::Enum),
        10 => Ok(SymbolKind::EnumVariant),
        11 => Ok(SymbolKind::ExternalPackage),
        other => Err(StoreError::Serialization(format!(
            "unknown symbol kind code: {other}"
        ))),
    }
}

fn language_to_u8(language: &Language) -> u8 {
    match language {
        Language::TypeScript => 1,
        Language::JavaScript => 2,
        Language::Python => 3,
        Language::Rust => 4,
        Language::Go => 5,
        Language::Java => 6,
        Language::C => 7,
        Language::Cpp => 8,
        Language::Ruby => 9,
        Language::Php => 10,
        Language::CSharp => 11,
        Language::Kotlin => 12,
        Language::Swift => 13,
        Language::Sql => 14,
    }
}

fn u8_to_language(input: u8) -> Result<Language, StoreError> {
    match input {
        1 => Ok(Language::TypeScript),
        2 => Ok(Language::JavaScript),
        3 => Ok(Language::Python),
        4 => Ok(Language::Rust),
        5 => Ok(Language::Go),
        6 => Ok(Language::Java),
        7 => Ok(Language::C),
        8 => Ok(Language::Cpp),
        9 => Ok(Language::Ruby),
        10 => Ok(Language::Php),
        11 => Ok(Language::CSharp),
        12 => Ok(Language::Kotlin),
        13 => Ok(Language::Swift),
        14 => Ok(Language::Sql),
        other => Err(StoreError::Serialization(format!(
            "unknown language code: {other}"
        ))),
    }
}

/// Resolution is persisted as a byte rather than by discriminant order, so
/// adding a variant later cannot silently reinterpret existing snapshots.
fn resolution_to_u8(resolution: &Resolution) -> u8 {
    match resolution {
        Resolution::Structural => 0,
        Resolution::Resolved => 1,
    }
}

/// An unknown byte reads as `Structural`. A snapshot written by a newer
/// OpenInvar may carry a resolution this build does not know; treating it as the
/// weakest value is the only safe reading, since the alternative is claiming
/// gate-safety on evidence this build cannot interpret.
fn resolution_from_u8(raw: u8) -> Resolution {
    match raw {
        1 => Resolution::Resolved,
        _ => Resolution::Structural,
    }
}

fn relationship_kind_to_u8(kind: &RelationshipKind) -> u8 {
    match kind {
        RelationshipKind::Imports => 1,
        RelationshipKind::Calls => 2,
        RelationshipKind::Extends => 3,
        RelationshipKind::Implements => 4,
        RelationshipKind::UsesType => 5,
        RelationshipKind::AccessesProperty => 6,
        RelationshipKind::ReExports => 7,
        RelationshipKind::Instantiates => 8,
        // Appended rather than inserted. These numbers are on disk: renumbering
        // an existing kind would silently reinterpret every snapshot written
        // before this build.
        RelationshipKind::Tests => 9,
    }
}

fn u8_to_relationship_kind(input: u8) -> Result<RelationshipKind, StoreError> {
    match input {
        1 => Ok(RelationshipKind::Imports),
        2 => Ok(RelationshipKind::Calls),
        3 => Ok(RelationshipKind::Extends),
        4 => Ok(RelationshipKind::Implements),
        5 => Ok(RelationshipKind::UsesType),
        6 => Ok(RelationshipKind::AccessesProperty),
        7 => Ok(RelationshipKind::ReExports),
        8 => Ok(RelationshipKind::Instantiates),
        9 => Ok(RelationshipKind::Tests),
        other => Err(StoreError::Serialization(format!(
            "unknown relationship kind code: {other}"
        ))),
    }
}

fn alias_scope_to_u8(scope: &AliasScope) -> u8 {
    match scope {
        AliasScope::ImportAlias => 1,
        AliasScope::ReExport => 2,
        AliasScope::BarrelReExport => 3,
        AliasScope::DefaultImport => 4,
    }
}

fn u8_to_alias_scope(input: u8) -> Result<AliasScope, StoreError> {
    match input {
        1 => Ok(AliasScope::ImportAlias),
        2 => Ok(AliasScope::ReExport),
        3 => Ok(AliasScope::BarrelReExport),
        4 => Ok(AliasScope::DefaultImport),
        other => Err(StoreError::Serialization(format!(
            "unknown alias scope code: {other}"
        ))),
    }
}

#[cfg(test)]
mod resolution_tests {
    use super::*;
    use openinvar_core::graph::InvarGraph;
    use openinvar_core::ir::{Language, Symbol, SymbolKind};

    fn symbol(id: &str, file: &str) -> Symbol {
        Symbol {
            id: id.to_string(),
            name: id.to_string(),
            kind: SymbolKind::Class,
            language: Language::TypeScript,
            file: file.to_string(),
            line_start: 1,
            line_end: 1,
            signature: None,
        }
    }

    fn edge(file: &str, resolution: Resolution) -> Relationship {
        Relationship {
            from: "a".to_string(),
            to: "b".to_string(),
            kind: RelationshipKind::Imports,
            alias: None,
            properties_accessed: vec![],
            context: "test".to_string(),
            file: file.to_string(),
            line: 1,
            resolution,
        }
    }

    fn graph() -> InvarGraph {
        let mut g = InvarGraph::new();
        g.add_symbol(symbol("a", "a.ts"));
        g.add_symbol(symbol("b", "b.java"));
        g
    }

    #[test]
    fn resolution_survives_a_round_trip_per_edge() {
        // This field is what a gate reads. When persistence dropped it, every
        // graph loaded from disk read back as structural — the safe direction,
        // so nothing broke loudly, but `blast-radius` then refused to say
        // "safe to modify" about a fully resolved repository and the feature
        // was quietly useless. That is the bug this test exists to catch.
        let mut g = graph();
        g.add_relationship(&edge("a.ts", Resolution::Resolved));
        g.add_relationship(&edge("b.java", Resolution::Structural));

        let bytes = GraphSnapshot::from_graph(&g)
            .expect("snapshot")
            .to_bytes()
            .expect("serialize");
        let restored = GraphSnapshot::from_bytes(&bytes).expect("deserialize");

        let mut got: Vec<(String, Resolution)> = restored
            .relationships
            .iter()
            .map(|r| (r.file.clone(), r.resolution))
            .collect();
        got.sort();

        assert_eq!(
            got,
            vec![
                ("a.ts".to_string(), Resolution::Resolved),
                ("b.java".to_string(), Resolution::Structural),
            ]
        );
    }

    // A version 1 or 2 payload carries no resolution byte, and `from_bytes`
    // gates the read on `version >= 3`, defaulting to Structural. That path is
    // not covered by a test here: a valid old payload cannot be produced by
    // editing a current one, because dropping the byte misaligns every section
    // after it, and hand-building one would pin the test to a format this code
    // no longer writes. The property it would assert — that a missing
    // resolution is never read as gate-safe — is covered by
    // `the_weaker_resolution_is_the_default` in openinvar-core and by the codec
    // test below.

    #[test]
    fn an_unknown_resolution_byte_reads_as_structural() {
        // A snapshot from a newer OpenInvar may carry a resolution this build
        // cannot interpret. Treating it as the weakest value is the only safe
        // reading; the alternative claims gate-safety on unread evidence.
        assert_eq!(resolution_from_u8(0), Resolution::Structural);
        assert_eq!(resolution_from_u8(1), Resolution::Resolved);
        assert_eq!(resolution_from_u8(200), Resolution::Structural);
    }
}
