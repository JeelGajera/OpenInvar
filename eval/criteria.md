# Evaluation criteria

**Fixed before the evaluation was run.** A gate that chooses its thresholds
after seeing the results is not a gate, so these are recorded first and the
numbers below are not to be edited to match an outcome. If a threshold turns
out to be the wrong one, the honest move is to say so in the results and
propose a new one for the *next* run — not to move it retroactively.

## Why this exists

Everything OpenInvar claims rests on properties measured so far only on
fixtures the project wrote itself. Fixtures are written by the same people who
wrote the analyzer, and they resolve at 100% because they were built to. That
proves the code does what its author expected; it does not prove the tool
survives contact with code nobody here wrote.

This measures the claims against real repositories at real commits — which is
what `analyze --at <rev>` exists to make possible.

## What is measured, and the threshold

### D — Determinism · **STOP on any failure**

> Identical input produces identical output, byte for byte.

Each revision is analysed **twice**, into two separate temporary worktrees on
different paths, and the two JSON documents are compared byte for byte.

| | |
|---|---|
| Pass | 100% of revisions byte-identical |
| **Stop** | **any single mismatch** |

No tolerance, and no "flaky" category. This is the property every other claim
depends on: a graph that disagrees with itself between runs cannot gate
anything, and one mismatch means the guarantee is not a guarantee. A failure
here stops the release regardless of how well everything else scores.

### P — Precision · investigate above threshold

> An audit finding is an accusation.

Each pair of consecutive commits from a healthy, widely-used project is
audited. These are commits that were reviewed and merged by their maintainers;
the overwhelming majority are ordinary work. A detector that fires on them is
crying wolf, and a gate that cries wolf is switched off in week two.

Ground truth is not available — we cannot know that a given commit contained no
tampering — so this measures a *rate*, not a false-positive count. The rate is
still decisive: an error-severity finding on a large fraction of ordinary
commits is disqualifying whatever the individual findings turn out to be.

| | |
|---|---|
| Pass | error-severity findings on ≤ 10% of commit pairs |
| Investigate | 10–25% |
| **Stop** | **> 25%** |

Warn-severity findings are recorded but do not gate: `dead-on-arrival` is
advisory by design.

### R — Resolution coverage on real code · investigate below threshold

> Gates fail open on regions OpenInvar cannot resolve.

Fixtures resolve at 100%. Real repositories will not, and the honest number
is the point of publishing it.

| | |
|---|---|
| Pass | median per-repository Tier 1 resolution ≥ 90% |
| Investigate | 75–90% |
| **Stop** | **< 75%** |

Below 75% the tool would be declining to decide on most of a repository, which
is honest but not useful.

### T — Performance · investigate above threshold

> Fast queries, and a gate you can run at commit time.

| | |
|---|---|
| Pass | ≤ 60s to analyse a revision of a repository under 2000 source files |
| Investigate | 60–180s |
| **Stop** | **> 180s** |

## Corpus

Real repositories, chosen for being *ordinary* rather than exotic or
flattering: widely used, actively maintained, and written before OpenInvar
existed so nothing in them was shaped to suit it. One per Tier 1 language
where practical.

The corpus is listed in `run.py` rather than here, so the code that ran and the
list it ran on cannot drift apart.

## What this deliberately does not claim

- **Not a recall measurement.** Without labelled tampering we cannot say what
  fraction of real reward-hacking the detectors catch. Precision without recall
  is half the picture, and saying so is better than implying otherwise.
- **Not a benchmark against other tools.** Nothing here compares OpenInvar to
  semgrep or a language server.
- **Not a statement about Tier 2.** Structural regions are excluded from the
  resolution figure by the tool itself; nothing here changes that.
