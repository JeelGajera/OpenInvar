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

**Superseded by R′ below.** The first run found that this criterion does not
measure what it was written to measure: `dispatch` stamps every Tier 1 edge
`Resolved`, and an edge only exists once something bound it, so a repository
with no Tier 2 files reports 100% by construction. It is a tier classification,
not a resolution rate. R is left as written — moving a threshold to match an
outcome is what pre-registering it is meant to prevent — and is not scored
again.

### R′ — Reference-level coverage · investigate below threshold

> Of the references an adapter attempted to bind, the share it bound.

Registered after the first run and before any measurement was taken with it.
This is the figure R was believed to be: the denominator now includes the
references that bound to nothing, which previously left no trace to count.

| | |
|---|---|
| Pass | median per-repository references bound ≥ 65% |
| Investigate | 50–65% |
| **Stop** | **< 50%** |

The thresholds sit well below R's because they measure a different and stricter
thing. An unbound reference is not necessarily a defect: a type from a package
whose source was never analysed is indistinguishable, to an adapter, from one
it should have found and missed. Both are references the graph cannot answer
questions about, which is what the number is for — so it is an upper bound on
what is missing, not a defect count.

Read as a trend rather than an absolute. The figure is comparable across
revisions of one repository, and only loosely across languages: each extractor
creates a different number of placeholders for the same source, so a language
with finer-grained extraction reports a lower percentage for identical code.

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
