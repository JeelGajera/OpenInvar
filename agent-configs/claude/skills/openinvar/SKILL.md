---
name: openinvar
description: Use OpenInvar for symbol impact analysis, dependency lookup, usages, aliases, and property-level access before modifying code.
---

# OpenInvar Skill

Use this skill when the task involves changing, deleting, renaming, moving, or understanding a code symbol.

## Workflow

1. Ensure the graph exists with `openinvar analyze .` or the `refresh_graph` MCP tool.
2. For changes to a symbol, inspect blast radius.
3. For deletion or rename, inspect usages.
4. For moving or extracting code, inspect dependencies.
5. Mention high-risk findings before editing.

## Commands

```bash
openinvar query blast-radius SymbolName
openinvar query usages SymbolName
openinvar query deps SymbolName
```

If symbols are missing because files are ignored or filtered out:

```bash
openinvar analyze . --no-gitignore
openinvar analyze . --include "src/**/*.ts"
openinvar analyze . --exclude "tests/**"
```

When MCP is available, prefer:

- `get_blast_radius`
- `get_symbol_usages`
- `get_dependencies`
- `refresh_graph`

`refresh_graph` can receive `path`, `respect_gitignore`, `include`, and `exclude`.

## Reading Results

- Aliases are real usages, even if text search misses them.
- `properties_accessed` matters for DTO/model field changes.
- Use `--file path/to/file.ts` if a symbol name is ambiguous.
