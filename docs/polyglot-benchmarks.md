# Polyglot Benchmark Harness

This repository includes a deferred benchmark procedure for large external repos.

## Procedure

1. Build release binaries:

```bash
cargo build --release
```

2. Run analysis timing:

```bash
time ./target/release/openinvar analyze <repo-path>
```

3. Run representative query latency:

```bash
time ./target/release/openinvar query blast-radius <SymbolName> --depth 3
```

4. Re-run analyze after a single-file edit to approximate incremental refresh.

## Target thresholds

- Initial parse: < 15s for 500k LOC
- Query latency: < 100ms p95 on 500k LOC graph
- Incremental: < 500ms on single file change
- Startup: < 2s from persisted graph
