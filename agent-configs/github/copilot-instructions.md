# OpenInvar Instructions For GitHub Copilot

This repository uses OpenInvar for deterministic code relationship analysis.

Before proposing changes that affect exported symbols, public types, DTO fields,
services, repositories, controllers, mappers, or shared utilities:

- Ask for or run `openinvar query blast-radius <SymbolName>`.
- Use `openinvar query usages <SymbolName>` before deleting or renaming.
- Use `openinvar query deps <SymbolName>` before moving or extracting code.

OpenInvar resolves aliased imports and reports property-level access. Do not rely
only on text search when reasoning about impact.

If the graph is stale or missing, run:

```bash
openinvar analyze .
```

