# OpenInvar Rule

Use OpenInvar before relationship-sensitive code edits.

Run or request OpenInvar queries before:

- changing exported classes, functions, interfaces, types, DTOs, services, repositories, controllers, or mappers
- deleting or renaming symbols
- changing model fields or payload shapes
- editing DI provider/module wiring

Prefer MCP tools:

- `get_blast_radius`
- `get_symbol_usages`
- `get_dependencies`
- `refresh_graph`

Fallback CLI:

```bash
openinvar analyze .
openinvar query blast-radius SymbolName
openinvar query usages SymbolName
openinvar query deps SymbolName
```

Aliases and property-level access are high-signal results. Use them to guide the
edit plan before touching code.

