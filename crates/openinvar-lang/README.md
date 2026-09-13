# openinvar-lang

Every language OpenInvar understands, in one crate.

`openinvar-lang` parses source files and resolves the references between them. It
owns imports, aliases, and declared-type binding for property attribution —
where the quality of a code graph actually lives. Parsing itself is tree-sitter
and is a solved, vendored problem.

Until 1.0.0 this was six crates: `openinvar-adapter-{ts,python,rust,go,c}` and
`openinvar-adapter-dispatch`. They versioned in lockstep, had exactly one
consumer between them, and lengthened the crates.io publish chain with every
language added. See `src/lang/mod.rs` for the full reasoning.

## Layout

```
src/
  dispatch.rs      routes files to the module that owns their language,
                   runs language groups in parallel, merges into one RepoIR
  lang/
    typescript/    .ts .tsx .js .jsx .mts .cts .mjs .cjs .vue .svelte .astro
    python/        .py .pyi
    rust/          .rs
    go/            .go
    c/             .c .h .cpp .cc .cxx .hpp .hxx .hh
```

## Features

One per language, all on by default:

```toml
openinvar-lang = { version = "0.2", default-features = false, features = ["python"] }
```

`typescript` covers JavaScript and the framework file types; `c` covers C++.
`supported_languages()` is built from the enabled features, so it reports what a
build can actually analyse rather than what the source tree contains. A build
skips files in languages it does not carry rather than failing on them.

## Adding a language

A module under `src/lang/` and a feature in `Cargo.toml`. Nothing else: no new
crate, no manifest, no entry in the publish workflow.

Budget for resolution, not parsing. Every bug 0.2.0 fixed was a resolution bug
— a hardcoded receiver name, includes producing zero edges, imports pointing at
an arbitrary package member — and none was a parse bug.
