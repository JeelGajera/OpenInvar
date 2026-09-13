# openinvar-cli

Command-line interface for OpenInvar.

`openinvar-cli` is the user-facing binary (`openinvar`) that drives analysis, querying, live watch updates, status, and MCP serving.

## Commands

- `openinvar analyze <path> [--include <csv>] [--exclude <csv>] [--no-gitignore]`
- `openinvar query blast-radius <symbol> [--file <path>] [--depth <n>]`
- `openinvar query usages <symbol> [--file <path>]`
- `openinvar query deps <symbol> [--file <path>] [--depth <n>]`
- `openinvar watch <path> [--include <csv>] [--exclude <csv>] [--no-gitignore]`
- `openinvar serve [--stdio | --port 7700]`
- `openinvar status <path>`

## Scan filtering

- `.gitignore` is respected by default for `analyze` and `watch`.
- `--include` and `--exclude` accept comma-separated multiple patterns.
- `--exclude` has higher priority than `--include`.

Example:

```bash
openinvar analyze . --include "src/**,packages/api/**" --exclude "**/*.test.ts,dist/**"
```

## Languages

`analyze` and `watch` index TypeScript/JavaScript, Python, Rust, Go and C/C++ in
a single pass. Files are routed to the adapter that owns their language, and
imports resolve within one language — a Python module importing a TypeScript
file through a build step is not linked.

## Notes

- This crate orchestrates `openinvar-core`, `openinvar-adapter-dispatch`, `openinvar-store`, and `openinvar-mcp`.
- Query behavior is deterministic and backed by persisted graph data.
