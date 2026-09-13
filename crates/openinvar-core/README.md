# openinvar-core

Language-agnostic graph engine for OpenInvar.

`openinvar-core` owns the canonical IR, graph structure, alias resolution, and query algorithms. It does not parse source code directly and does not contain language-specific logic.

## Responsibilities

- Defines the frozen IR contract (`Symbol`, `Relationship`, `FileIR`, `RepoIR`)
- Stores a directed symbol graph (`InvarGraph`)
- Resolves alias chains (import aliases, re-exports, barrel/default alias metadata)
- Provides query APIs:
  - `blast_radius`
  - `dependencies`
  - `symbol_usages`
- Supports incremental graph update plumbing
- Provides the shared symbol-id and AST-traversal helpers every adapter builds on

## Public modules

- `ir`: shared IR contract used by all adapters
- `graph`: graph container + indexes
- `index`: symbol lookup indexes
- `resolver`: alias chain ingestion and canonicalization helpers
- `query`: traversal-based query functions
- `incremental`: file-level replacement/update helpers
- `scan`: source-file discovery, filtering and extension-to-language detection
- `symbol_id`: canonical `SymbolId` construction, shared by every adapter
- `ast`: iterative, depth-bounded tree-sitter traversal helpers (feature `ast`)
- `error`: `InvarError`

## Features

- `ast` (default): shared tree-sitter traversal helpers used by every language
  adapter. Graph-only consumers such as `openinvar-store` turn this off so they do
  not link a parser.

## Minimal usage

```rust
use openinvar_core::graph::InvarGraph;
use openinvar_core::query;

let graph = InvarGraph::new();
let _ = query::blast_radius(&graph, "UserPayload", None, Some(2));
```

## Notes

- This crate is deterministic by design and contains no LLM logic.
- Language parsing belongs in adapter crates (`openinvar-adapter-ts`, `-python`, `-rust`, `-go`, `-c`), routed by `openinvar-adapter-dispatch`.
