# Evaluation

Measuring OpenInvar's claims against real repositories at real commits, rather
than against fixtures this project wrote itself.

Fixtures are written by the same people who wrote the analyzer and resolve at
100% because they were built to. That proves the code does what its author
expected; it does not prove the tool survives contact with code nobody here
wrote. `analyze --at <rev>` is what makes the alternative possible.

| File | |
|---|---|
| [`criteria.md`](criteria.md) | What is measured and what counts as passing — **fixed before the first run** |
| [`run.py`](run.py) | Measures. Knows nothing about thresholds |
| [`summarise.py`](summarise.py) | Applies the criteria to the JSON |
| `results/` | One dated report per run |

The runner and the summariser are separate on purpose: the thing that measures
and the thing that decides what a passing number is should not live in one
file, or a threshold can be moved in the same edit that produces the result it
is judging.

## Latest

[**2026-09-14**](results/2026-09-14.md) — OpenInvar 0.2.0, 4 repositories, 32
revisions, 28 commit pairs. **CONTINUE.**

- **Determinism 32/32 byte-identical**, zero mismatches
- **0 of 28 commit pairs** produced an error-severity finding
- Slowest revision 0.71s for 313 files and 18,989 edges
- One criterion turned out to measure a tier classification rather than a
  resolution rate, and is reported as such rather than scored as the pass it
  numerically was

## Running it

```bash
cargo build --release
python3 eval/run.py --commits 8 --out eval/results/run.json
python3 eval/summarise.py eval/results/run.json
```

Repositories are cloned into a temporary directory and removed after
measurement. Network access to github.com is required.
