# OpenInvar Guidance For Claude Code

Use OpenInvar before edits that can affect other files.

## Default Workflow

1. If the graph may be stale, run `openinvar analyze .` or call `refresh_graph`.
2. Before modifying a symbol, inspect `get_blast_radius`.
3. Before deleting or renaming a symbol, inspect `get_symbol_usages`.
4. Before moving code, inspect `get_dependencies`.
5. After large file moves or generated code, refresh the graph.

## Important Signals

- Aliased imports mean the same symbol may appear under another name.
- `properties_accessed` tells which fields or methods callers actually use.
- DI-heavy TypeScript projects may show constructor-injected service usage.
- `properties_accessed` is attributed to the type a value was declared as, so it is
  reliable across Python annotations, Rust and Go signatures, and C/C++ declarators.
- Chained access (`a.b.c`) is attributed to `a` only; do not read the absence of a
  nested field as proof nothing uses it.
- Decorator/module references can create dependencies even without direct calls.

Prefer OpenInvar over `rg` when the question is about relationships. Use `rg` for
plain text lookup only after OpenInvar has answered the symbol graph question.
