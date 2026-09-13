# OpenInvar Agent Guide

This guide explains how coding agents should use OpenInvar in day-to-day edit loops.

## Why this matters

Agents often miss deep alias imports and silent property-level breakage. OpenInvar provides deterministic relationship data so agents can make safer edits.

## Recommended agent flow

1. Call `get_blast_radius` before changing a symbol.
2. Call `get_symbol_usages` to enumerate all references.
3. Make changes.
4. Call `refresh_graph_index` after file updates.
5. Re-run `get_blast_radius` to verify expected impact.

## MCP tools for agents

- `get_blast_radius`
- `get_dependencies`
- `get_symbol_usages`
- `refresh_graph_index`

### `refresh_graph_index` parameters

```json
{
  "path": "optional/subdir",
  "include": "src/**,packages/api/**",
  "exclude": "**/*.test.ts,dist/**",
  "respect_gitignore": true
}
```

Use this when:
- The agent modifies source files and needs fresh graph state.
- The repo has large generated/vendor directories that should be excluded.
- The agent wants to focus analysis on a subset of the repo.

## Filtering behavior

OpenInvar scan filtering is shared behavior in `openinvar-core` and is used by CLI and MCP refresh.

- `.gitignore` rules are respected by default.
- `include` and `exclude` are comma-separated patterns.
- `exclude` wins over `include`.
- Passing no `include` means "include all supported source files".

## CLI equivalents for humans

```bash
openinvar analyze . --include "src/**,packages/core/**" --exclude "**/*.snap.ts,dist/**"
openinvar watch . --exclude "**/*.generated.ts" --no-gitignore
```

## Operational tip

If the agent is editing continuously, keep `openinvar watch` running and still use `refresh_graph_index` as an explicit sync point before high-risk refactors.
