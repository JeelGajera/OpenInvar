# OpenInvar Architecture

This document describes the internal architecture of OpenInvar — how the crates relate, how data flows from source code to query results, and the reasoning behind each design decision.

---

## Overview

OpenInvar is a Rust workspace. Each crate has exactly one responsibility. No crate reaches outside its defined scope.

```
openinvar-core              Language-agnostic graph engine. No language knowledge.
openinvar-adapter-ts        TypeScript/JavaScript → IR. No graph knowledge.
openinvar-adapter-python    Python → IR.
openinvar-adapter-rust      Rust → IR.
openinvar-adapter-go        Go → IR.
openinvar-adapter-c         C/C++ → IR.
openinvar-adapter-dispatch  Routes files to the adapter that owns them. No parsing.
openinvar-store             Graph persistence and cache. No parsing or query logic.
openinvar-mcp               MCP protocol server. Calls into core. No parsing.
openinvar-cli               Developer CLI. Orchestrates everything. No business logic.
```

Dependency direction is strictly one-way:

```
openinvar-cli
  ├── openinvar-mcp → openinvar-adapter-dispatch, openinvar-store, openinvar-core
  ├── openinvar-adapter-dispatch → the five adapters → openinvar-core
  └── openinvar-store → openinvar-core
```

`openinvar-core` has no dependencies on any other OpenInvar crate. It is the foundation.

---

## Data flow

```
Source files on disk
        ↓
   openinvar-adapter-dispatch
   (detect language per file, group by owning adapter)
        ↓
   openinvar-adapter-{ts,python,rust,go,c}
   (tree-sitter parse → FileIR per file, parallel via rayon;
    each adapter resolves imports within its own language)
        ↓
   RepoIR
   (Vec<FileIR> + language stats, in a deterministic order)
        ↓
   openinvar-core: graph builder
   (Symbol nodes inserted, Relationship edges inserted, alias chains built)
        ↓
   InvarGraph
   (petgraph DiGraph + DashMap indexes + AliasChains)
        ↓
   openinvar-store
   (serialize to RocksDB → .openinvar/db)

   On subsequent startup:
   openinvar-store deserialize → InvarGraph (< 2s, no reparse)

   On query:
   openinvar-core query.rs
   (BFS/DFS traversal of in-memory graph, < 100ms)
        ↓
   openinvar-mcp context_builder.rs
   (format result for agent consumption)
        ↓
   MCP tool response (JSON via stdio)
```

---

## openinvar-core

The heart of the system. Language-agnostic. Receives IR, builds graph, answers queries.

### ir.rs

The IR schema. `Symbol`, `Relationship`, `FileIR`, `RepoIR`. These are the types that every language adapter must produce and that every query returns results against. Frozen for v1 — see [ir-spec.md](ir-spec.md).

### graph.rs

The `InvarGraph` struct. Contains:

```rust
pub struct InvarGraph {
    // The actual directed graph. Node payload = SymbolId. Edge payload = RelationshipMeta.
    pub graph: DiGraph<SymbolId, RelationshipMeta>,

    // Fast lookup: SymbolId → NodeIndex in petgraph
    pub node_index: DashMap<SymbolId, NodeIndex>,

    // Fast lookup: symbol name → Vec<SymbolId>
    // Multiple symbols can share a name (same name in different files)
    pub name_index: DashMap<String, Vec<SymbolId>>,

    // Fast lookup: file path → Vec<SymbolId>
    pub file_index: DashMap<String, Vec<SymbolId>>,

    // Full symbol data
    pub symbols: DashMap<SymbolId, Symbol>,

    // Alias chains: canonical SymbolId → all known aliases
    pub alias_chains: DashMap<SymbolId, Vec<AliasEntry>>,

    // Per-file re-export metadata for barrel-chain resolution
    pub file_reexports: DashMap<String, Vec<ReExportEntry>>,
}
```

`DashMap` is used instead of `HashMap` throughout because graph mutations (incremental updates) may happen concurrently with read queries. `DashMap` provides lock-free concurrent access without requiring `RwLock` on the outer struct.

`petgraph` provides the directed graph structure and the BFS/DFS traversal algorithms used by the query engine.

### resolver.rs

Alias resolution. This is the most important module in the system.

When the graph is built from IR, every `Relationship` with a non-None `alias` field is registered in `alias_chains`:

```
UserPayload::class → [
    AliasEntry { alias_name: "ResponseModel", defined_in: "mappers/view_model_mapper.ts", scope: ImportAlias },
    AliasEntry { alias_name: "PublicUser", defined_in: "index.ts", scope: ReExport },
]
```

When `get_blast_radius("UserPayload")` is called, the resolver expands the query to include all known aliases before traversing the graph. This is what catches the aliased import bug.

Alias scopes:
- `ImportAlias` — `import { A as B }`
- `ReExport` — `export { A as B } from './a'`
- `BarrelReExport` — `export * from './a'` (exposes original names)
- `DefaultImport` — `import B from './a'` where B was the default export of A

### query.rs

Three query functions that traverse the graph:

**`blast_radius(symbol, depth)`**
BFS from target node, following INCOMING edges (who points at this?).
At each hop, include alias metadata from edge payload.
Returns `BlastRadiusResult` with `direct` and `aliased` groups.

**`dependencies(symbol, depth)`**
BFS from target node, following OUTGOING edges (what does this point at?).
Returns `DependencyResult`.

**`symbol_usages(symbol)`**
Looks up canonical symbol + all alias names in `name_index`.
Collects all incoming edges across all alias names.
Deduplicates by `file + line`.
Returns `UsageResult`.

### index.rs

Fast reverse lookup tables. `name_index` and `file_index` exist here as secondary indexes over the graph. They are rebuilt from the graph on load and updated incrementally on file change.

### incremental.rs

Handles partial graph updates when a file changes.

On a file-change event from the watcher:
1. Remove all nodes and edges from the changed file (using `file_index`)
2. Re-parse the changed file using the appropriate adapter
3. Insert new nodes and edges from the fresh `FileIR`
4. Rebuild alias chains for affected symbols
5. Update `name_index` and `file_index`

The rest of the graph is untouched. This is what keeps incremental update time under 500ms.

---

## openinvar-adapter-ts

TypeScript and JavaScript parser. Uses tree-sitter for parsing — no dependency on the TypeScript compiler, no `node_modules` required.

### File discovery

Source-file walking/filtering lives in `openinvar-core/src/scan.rs`, not a separate
adapter `walker.rs`. The adapter relies on scan filters and language detection to
discover supported files.

### parser.rs

Calls `tree-sitter-typescript` or `tree-sitter-javascript` on each file. Returns a tree-sitter `Tree`. Non-fatal on parse errors — logs them and returns a partial tree.

### framework_preprocessor.rs

Framework files (`.vue`, `.svelte`, `.astro`) are pre-processed before parsing.
The preprocessor blanks non-script regions with spaces while preserving newline
positions exactly. This keeps line numbers stable for diagnostics and relationship
metadata without adding additional parser dependencies.

### extractor.rs

Walks the tree-sitter AST and extracts `Symbol` and `Relationship` instances. This is the largest module. Key responsibilities:

- Detect class declarations, function declarations, interface declarations, type aliases, enum declarations
- Detect import statements (named, default, namespace, aliased, dynamic)
- Detect property accesses and function calls
- Assign stable `SymbolId` values to every found symbol
- Populate `properties_accessed` on relationships where property accesses are present

### import_resolver.rs

Handles the complex import patterns:

- `import { A as B }` — aliased named import
- `import * as NS from './module'` — namespace import
- `export { A } from './b'` — re-export
- `export * from './b'` — barrel re-export
- `export default class A` — default export
- `import A from './b'` — default import

Barrel file handling: when `index.ts` contains `export * from './user_payload'`, the resolver must follow the re-export chain and register all re-exported symbols with the barrel's path so they can be found when imported from the barrel.

---

## openinvar-adapter-python, -rust, -go, -c

Each of these follows the same shape as `openinvar-adapter-ts` — `parser.rs`,
`extractor.rs`, `scope_analyzer.rs` and an import resolver — and exposes the
same entrypoint, `analyze_files(root, files) -> Result<RepoIR, _>`. Shared
symbol-id construction and AST traversal live in `openinvar-core/src/symbol_id.rs`
and `openinvar-core/src/ast.rs` rather than being copied per adapter.

What differs is the resolution model each language needs:

| Adapter | Import model | Language-specific work |
| --- | --- | --- |
| `-python` | Relative imports, `__init__.py` re-export chains, star imports, `TYPE_CHECKING` guards | `framework.rs` classifies Pydantic / Django / dataclass models so their fields are attributed to the model |
| `-rust` | `use` paths resolved through the module that declares the name, following `pub use` chains | `module_tree.rs` builds the module graph from `mod` declarations; `macro_analyzer.rs` reads `#[derive]` and recovers field access from macro token trees |
| `-go` | Package imports resolved against `go.mod`, discovered per directory | `interface_detector.rs` matches method sets from the symbol table to detect structural interface satisfaction, per package |
| `-c` | `#include` resolved structurally to the header's module symbol; the preprocessor is not run | Grammar chosen per extension (`tree-sitter-c` vs `tree-sitter-cpp`); handles `typedef` and `using` aliases, base classes and namespace-qualified names |

### scope_analyzer.rs

Every adapter has one, and they exist for the same reason: member access must be
attributed to the type a value was *declared* as, not to the variable name. The
analyzer binds receivers to declared types from parameters, `let`/`var`
bindings, annotations and declarators, then keys properties per resolved type.
So `payload.user_id` is recorded against `UserPayload` however the variable was
named, and one struct does not inherit another's fields.

---

## openinvar-adapter-dispatch

The single entrypoint the CLI and MCP server call. Detects each file's language
via `openinvar_core::scan::detect_language_from_extension`, groups files by the
adapter that owns them (TS and JS share one, C and C++ share one), runs the
groups in parallel with rayon, and merges the `FileIR`s into a single `RepoIR`.
Files in unsupported languages are skipped.

Grouping uses an ordered key, and each group's files are sorted before analysis.
`RepoIR.files` determines the order symbols and edges are inserted into the
graph, and a deterministic graph is OpenInvar's first documented guarantee —
`HashMap` iteration order made two runs over identical input produce
differently-ordered output.

Imports resolve within one language. Each adapter analyses its own group
independently, so a Python module importing a TypeScript file through a build
step is not linked.

---

## openinvar-store

Persistence and caching.

### db.rs

Serializes the full `InvarGraph` to RocksDB at `.openinvar/db` in the repo root. Uses `serde_json` for serialization. On startup, deserializes and validates freshness by comparing file modification times against the stored index — files modified since the last store trigger a targeted re-parse rather than a full rebuild.

### cache.rs

LRU cache for frequently-queried symbols. Blast radius results for hot symbols (e.g. a base class used everywhere) are cached after first computation. Cache is invalidated for a symbol when any of its dependents change.

---

## openinvar-mcp

MCP server. Receives tool calls from agents via stdio, delegates to `openinvar-core`, formats results.

### server.rs

Uses the `rmcp` crate for MCP protocol handling. Registers three tools on startup: `get_blast_radius`, `get_dependencies`, `get_symbol_usages`. Each tool has a JSON schema generated via `schemars` so agents receive properly typed tool definitions.

### context_builder.rs

Formats query results from `openinvar-core` into agent-friendly text. The raw graph result is a Rust struct — context_builder turns it into the structured text format shown in the README. Key design goal: the output should be immediately actionable by an agent with no additional reasoning required.

### tools/

One file per MCP tool. Each file:
1. Receives the tool call parameters (JSON)
2. Validates and deserializes parameters
3. Calls the appropriate `query.rs` function
4. Passes result to `context_builder.rs`
5. Returns the formatted string as the MCP tool response

---

## openinvar-cli

Developer-facing CLI. Orchestrates the other crates. Contains no business logic.

### commands/analyze.rs

1. Discovers all source files (adapter walker)
2. Parses them in parallel (rayon + adapter parser + extractor)
3. Builds the graph (openinvar-core)
4. Persists to RocksDB (openinvar-store)
5. Prints stats: files parsed, symbols found, relationships found, time taken

### commands/query.rs

1. Loads graph from RocksDB (openinvar-store)
2. Calls the appropriate query function (openinvar-core)
3. Formats output as a terminal table

### commands/watch.rs

1. Loads graph from RocksDB
2. Starts the MCP server
3. Starts the file watcher (notify crate)
4. On file change: incremental update (openinvar-core) + persist delta + notify MCP server

### commands/serve.rs

Starts the MCP server in stdio mode. Designed to be called by agent MCP configurations directly.

---

## Performance design

### Why in-memory

Queries must return in under 100ms. Any disk or network I/O on the query path makes this impossible at scale. The graph lives entirely in RAM after startup. `petgraph` + `DashMap` together use approximately 1–2 bytes per node and 2–4 bytes per edge for the index structures. A 500k LOC TypeScript codebase produces roughly 50k–100k symbols and 200k–400k relationships — this fits comfortably in under 500MB.

### Why rayon for parsing

Parsing is embarrassingly parallel — each file is independent. `rayon` distributes file parsing across all available CPU cores automatically. A 300-API codebase with 50k LOC parses in approximately 8 seconds on a single machine because 8 cores parse 8 files simultaneously.

### Why DashMap

Graph mutations (incremental updates) and read queries must be able to happen concurrently. `DashMap` is a lock-free concurrent hash map — it shards internally so reads and writes to different symbols do not block each other. This is critical for watch mode where a file change triggers a mutation while the MCP server is simultaneously serving queries.

### Why RocksDB

RocksDB provides fast key-value storage with good compression. A serialized 100k-symbol graph occupies approximately 20–30MB on disk. Deserialization (startup after the first run) takes under 2 seconds because RocksDB's read path is heavily optimized for sequential key reads.

---

## Adding a new language adapter

Five adapters exist — TypeScript/JavaScript, Python, Rust, Go and C/C++. To add
the next one (Java is the nearest candidate; `Language::Java` is already in the
IR enum but has no adapter), the process is:

1. Create `crates/openinvar-adapter-<lang>/`
2. Add the `tree-sitter-<lang>` grammar as a dependency, and `openinvar-core`
   with the `ast` feature for the shared traversal and symbol-id helpers
3. Implement `parser.rs` — tree-sitter parse per file
4. Implement `extractor.rs` — extract symbols and relationships into IR
5. Implement `scope_analyzer.rs` — bind receivers to their declared types, so
   member access is attributed to the type rather than the variable name
6. Implement `import_resolver.rs` — resolve the language's import forms to
   canonical symbol IDs, including aliases and re-export chains
7. Expose `analyze_files(root, files) -> Result<RepoIR, _>` from `lib.rs`
8. Add `fixtures/adapter-<lang>/` with representative code, including the
   alias-import bug scenario
9. Write tests — the alias-import equivalent for the language must pass, plus a
   case in `crates/openinvar-cli/tests/regression.rs`
10. Register the language in `openinvar-adapter-dispatch`: extension detection in
    `openinvar_core::scan`, `language_rank`, `adapter_group`, `run_adapter` and
    `supported_languages`

Only the dispatch layer learns about the new adapter. The graph engine, the IR
schema and the MCP tools are not touched.

See also: [Agent Guide](./agent-guide.md)
