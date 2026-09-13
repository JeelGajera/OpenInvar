# OpenInvar MCP Tools Reference

This document describes the MCP tools OpenInvar exposes to coding agents.

## Overview

| Tool | Purpose |
|---|---|
| `get_blast_radius` | What breaks if I change symbol X? |
| `get_dependencies` | What does symbol X depend on? |
| `get_symbol_usages` | Where is symbol X referenced? |
| `refresh_graph_index` | Re-analyze and persist graph after code changes |

All query tools resolve aliases and track property-level access.

## get_blast_radius

Input:

```json
{
  "symbol": "UserPayload",
  "file": "optional/path.ts",
  "depth": 3
}
```

## get_dependencies

Input:

```json
{
  "symbol": "UserPayload",
  "file": "optional/path.ts",
  "depth": 3
}
```

## get_symbol_usages

Input:

```json
{
  "symbol": "UserPayload",
  "file": "optional/path.ts",
  "include_aliases": true
}
```

## refresh_graph_index

Rebuilds OpenInvar index and updates both on-disk snapshot and in-memory MCP server graph.

Input:

```json
{
  "path": "optional/subdir",
  "include": "src/**,packages/core/**",
  "exclude": "**/*.test.ts,dist/**",
  "respect_gitignore": true
}
```

Notes:
- `include` and `exclude` accept comma-separated multiple patterns.
- `.gitignore` is respected by default.
- Use this tool after agent edits to keep follow-up queries accurate.

## Suggested agent sequence

1. `get_blast_radius`
2. `get_symbol_usages`
3. Apply code changes
4. `refresh_graph_index`
5. Re-run `get_blast_radius`
