# OpenInvar Intermediate Representation (IR) Specification

**Version:** 1.0.0 (frozen for v1)
**Status:** Stable — do not modify core structures without a formal version bump

The IR is the contract between language-specific adapters and `openinvar-core`. Every adapter — TypeScript, Python, Rust, Go — must produce this exact output. The core graph engine speaks only IR. It has no knowledge of any specific language.

---

## Why the IR exists

Without a standard IR, every language adapter would need its own graph logic. The IR decouples parsing from intelligence. An adapter's only job is to turn source code into IR. The core's only job is to turn IR into a queryable graph.

This means:
- Adding Python support = write a new adapter. Core is untouched.
- A performance improvement in the core benefits all languages simultaneously.
- The IR schema is the single testable contract between two independent systems.

---

## 1. Top-level structures

### `RepoIR`

The complete output of a full repository parse or an incremental update batch.

```rust
pub struct RepoIR {
    /// Absolute path to the repository root on disk.
    pub root: String,

    /// One FileIR per parsed source file.
    pub files: Vec<FileIR>,

    /// Symbol counts per language. e.g. { "TypeScript": 1423, "JavaScript": 89 }
    pub language_stats: std::collections::HashMap<String, usize>,
}
```

### `FileIR`

The complete extracted output from a single source file. This is what the adapter produces per file.

```rust
pub struct FileIR {
    /// Relative path from repo root. e.g. "src/models/user_payload.ts"
    pub file: String,

    /// The language this file was parsed as.
    pub language: Language,

    /// All symbols declared in this file.
    pub symbols: Vec<Symbol>,

    /// All relationships originating from this file.
    pub relationships: Vec<Relationship>,

    /// Non-fatal parse errors. Do not panic — log here and continue.
    pub parse_errors: Vec<String>,
}
```

---

## 2. Symbols

A `Symbol` is any named entity in source code that can be depended on or referenced by other code. Classes, functions, interfaces, types, properties, and variables are all symbols.

```rust
/// Unique identifier for a symbol across the entire codebase.
///
/// Format:  "relative/path/to/file.ts::SymbolName::kind"
/// Example: "src/models/user.ts::UserPayload::class"
/// Example: "src/models/user.ts::UserPayload::userId::property"
///
/// IDs must be stable across parses of the same file with the same content.
/// IDs must be unique across the entire repo.
pub type SymbolId = String;

pub struct Symbol {
    pub id: SymbolId,

    /// The original declared name in source. Never an alias.
    pub name: String,

    pub kind: SymbolKind,
    pub language: Language,

    /// Relative path from repo root.
    pub file: String,

    pub line_start: u32,
    pub line_end: u32,

    /// Optional: function signature, class declaration, type definition line.
    /// Include the raw source text where possible — agents find it useful.
    pub signature: Option<String>,
}
```

### `SymbolKind`

```rust
pub enum SymbolKind {
    Class,
    Interface,
    TypeAlias,
    Function,
    Method,
    Property,
    Variable,
    Module,
    Enum,
    EnumVariant,
}
```

### `Language`

```rust
pub enum Language {
    TypeScript,
    JavaScript,
    Python,   // v2
    Rust,     // v3
    Go,       // v3
    Java,     // v3
}
```

---

## 3. Relationships

A `Relationship` is a directed edge between two symbols. It represents any form of dependency, usage, or structural connection in source code.

```rust
pub struct Relationship {
    /// The symbol where this relationship originates.
    pub from: SymbolId,

    /// The symbol being depended on, imported, called, or accessed.
    pub to: SymbolId,

    pub kind: RelationshipKind,

    /// If the symbol was imported under a different name, record the alias here.
    /// Example: `import { UserPayload as ResponseModel }` → alias = Some("ResponseModel")
    /// This field is critical for alias resolution. Never omit it when present.
    pub alias: Option<String>,

    /// Which properties of the `to` symbol are accessed through this relationship.
    /// Example: `data.userId` and `data.timestamp` → ["userId", "timestamp"]
    /// Property-level tracking is required in v1. Do not leave this empty
    /// when property accesses are present in the source.
    pub properties_accessed: Vec<String>,

    /// The exact source line or lines that create this relationship.
    /// Include the raw import/call/access statement. Agents use this for context.
    pub context: String,

    /// The file where this relationship is defined (relative from repo root).
    pub file: String,

    /// The line number where this relationship begins.
    pub line: u32,

    /// How `to` was determined, and therefore what this edge is worth.
    pub resolution: Resolution,
}
```

### `Resolution`

```rust
pub enum Resolution {
    /// Matched by name inside one file, with no import table in play.
    /// Advisory only — a gate must not draw a conclusion from it.
    Structural,
    /// Bound through the file's imports, aliases and declared types.
    Resolved,
}
```

A polyglot graph mixes both in a single answer, so this belongs on the edge
rather than on the language: `blast-radius` over a repository with TypeScript
and Java returns rows from each, and a reader needs to know per row which ones
a gate may act on.

Adapters do not set this field themselves. Dispatch stamps `Resolved` on the
output of every Tier 1 pipeline, and structural analysis leaves the default.
The default is `Structural` — the weaker value — so a construction site that
omits the field, or a stored graph written before the field existed,
under-claims rather than being read as gate-safe.

**An empty `blast-radius` result is only "safe to modify" when every edge in
the graph is `Resolved`.** Structural analysis cannot see across files, so a
reference from a structural region would never have reached the graph at all,
and the emptiness is partly an artefact of how much was resolved.

### `RelationshipKind`

```rust
pub enum RelationshipKind {
    /// import statement (named, default, namespace, aliased)
    Imports,

    /// function or method call
    Calls,

    /// class extends another class
    Extends,

    /// class implements an interface
    Implements,

    /// type annotation usage (e.g. `const x: UserPayload`)
    UsesType,

    /// dot access on a symbol (e.g. `user.userId`)
    AccessesProperty,

    /// re-export statement (e.g. `export { A } from './b'`)
    ReExports,

    /// object instantiation (e.g. `new UserPayload()`)
    Instantiates,
}
```

---

## 4. The alias requirement (critical)

Alias resolution is the primary hard problem OpenInvar solves. The `alias` field on `Relationship` is the mechanism. Adapters must handle all of the following patterns:

### Named alias import
```typescript
import { UserPayload as ResponseModel } from '../models/user_payload'
// Relationship { kind: Imports, to: UserPayload::id, alias: Some("ResponseModel") }
```

### Re-export with rename
```typescript
export { UserPayload as PublicUser } from './user_payload'
// Relationship { kind: ReExports, to: UserPayload::id, alias: Some("PublicUser") }
```

### Barrel file (index.ts)
```typescript
export * from './user_payload'
// All symbols from user_payload.ts must appear as re-exports in the barrel's FileIR
```

### Default import aliasing
```typescript
import User from '../models/user_payload'
// If UserPayload was the default export:
// Relationship { kind: Imports, to: UserPayload::id, alias: Some("User") }
```

If an adapter does not handle all four patterns, it fails the alias-import-bug test case and must not be released.

---

## 5. The property-level requirement (critical)

It is not enough to record that file A depends on class B. OpenInvar must record *which properties of B* are accessed by A. This is what enables "property X of UserPayload is accessed in 3 files" output.

Adapters must populate `properties_accessed` on every `AccessesProperty` relationship and on any `Imports` relationship where property accesses can be statically determined.

```typescript
// Source:
const result = mapper.userId + mapper.timestamp;

// Expected IR output:
Relationship {
    kind: AccessesProperty,
    from: "current_file.ts::someFunction::function",
    to: "user_payload.ts::UserPayload::class",
    properties_accessed: ["userId", "timestamp"],
    context: "const result = mapper.userId + mapper.timestamp;",
    ...
}
```

---

## 6. Symbol ID format

Symbol IDs must follow this exact format:

```
{relative_file_path}::{SymbolName}::{kind_lowercase}
```

Examples:
```
src/models/user_payload.ts::UserPayload::class
src/models/user_payload.ts::UserPayload::userId::property
src/handlers/auth.ts::processLogin::function
src/handlers/auth.ts::AuthHandler::handleRequest::method
```

Rules:
- Always use the relative path from repo root (not absolute)
- Always use forward slashes regardless of OS
- Kind must be lowercase
- IDs must be deterministic — same source = same ID on every parse
- IDs must be unique across the entire repository

---

## 7. Error handling contract

Adapters must never panic on malformed or partial source code. Developers are actively editing files — half-typed syntax is normal input.

```rust
// Correct: non-fatal error handling
match parse_file(path) {
    Ok(ir) => ir,
    Err(e) => FileIR {
        file: path.to_string(),
        language: detected_language,
        symbols: vec![],
        relationships: vec![],
        parse_errors: vec![e.to_string()],
    }
}
```

The core will log `parse_errors` and continue. A file with parse errors simply contributes zero symbols and zero relationships to the graph — it does not abort the build.

---

## 8. Adapter checklist

Before submitting a new language adapter for merge:

- [ ] All `FileIR` outputs include `file` as relative path from repo root
- [ ] `SymbolId` format matches the spec exactly
- [ ] Aliased imports produce `alias: Some("AliasName")` on the relationship
- [ ] Re-exports (including barrel files) are tracked as `ReExports` relationships
- [ ] Property accesses populate `properties_accessed` on relationships
- [ ] `context` field contains the raw source line(s), not a description
- [ ] No `unwrap()` or `panic!()` in production code paths
- [ ] `parse_errors` are populated for syntax errors, not propagated as hard failures
- [ ] The `alias-import-bug` fixture test passes
- [ ] A new `fixtures/{language}-sample/` directory is added with representative code

---

## 9. Versioning

The IR schema is frozen for v1. Any addition of fields, changes to existing field types, or removal of fields requires:

1. A new major version of the IR spec (this document)
2. A migration path for existing persisted graphs
3. Updated adapter checklist
4. Bumped crate versions for `openinvar-core` and all adapters

Backwards-incompatible IR changes are breaking changes for all adapters simultaneously. Treat the IR like a public API.