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

[**2026-09-15 (0.3.0)**](results/2026-09-15-0.3.0.md) — OpenInvar 0.3.0, 4
repositories, 32 revisions, 28 commit pairs. **CONTINUE.**

- **Determinism 32/32 byte-identical**, across a `tree-sitter` upgrade of five
  minor versions and all ten grammars
- **0 of 28 commit pairs** produced an error-severity finding
- **R′ unchanged to the decimal in every repository**, which is what the three
  changes since the last run were meant not to move
- R′ is now split by *why* a reference failed to bind, and the split says the
  next work is a Java classifier: Java is 83% of the corpus's unbound
  references and classifies none of them

## Running it

```bash
cargo build --release
python3 eval/run.py --commits 8 --out eval/results/run.json
python3 eval/summarise.py eval/results/run.json
```

Repositories are cloned into a temporary directory and removed after
measurement. Network access to github.com is required.
