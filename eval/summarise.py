#!/usr/bin/env python3
"""Apply `criteria.md` to the JSON `run.py` wrote.

Separate from the runner on purpose: the thing that measures and the thing
that decides what a passing number is should not be the same file, or a
threshold can be moved in the same edit that produces the result it is judging.

Usage:
    python3 eval/summarise.py eval/results/run.json
"""

import json
import statistics
import sys

# From criteria.md. Fixed before the first run.
DETERMINISM_MUST_BE = 1.0
PRECISION_PASS = 0.10
PRECISION_STOP = 0.25
RESOLUTION_PASS = 90.0
RESOLUTION_STOP = 75.0
SECONDS_PASS = 60.0
SECONDS_STOP = 180.0


def verdict(value, pass_at, stop_at, higher_is_better):
    if higher_is_better:
        if value >= pass_at:
            return "PASS"
        return "STOP" if value < stop_at else "INVESTIGATE"
    if value <= pass_at:
        return "PASS"
    return "STOP" if value > stop_at else "INVESTIGATE"


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "eval/results/run.json"
    data = json.loads(open(path).read())
    repos = [r for r in data["repos"] if "error" not in r]

    revisions = [rev for r in repos for rev in r["revisions"]]
    analysed = [rev for rev in revisions if rev.get("ok")]
    identical = [rev for rev in analysed if rev.get("determinism", {}).get("ok")]
    pairs = [p for r in repos for p in r["pairs"]]
    with_errors = [p for p in pairs if p["errors"]]
    with_warns = [p for p in pairs if p["warns"]]
    seconds = [rev["seconds"] for rev in analysed]

    print(f"OpenInvar {data['openinvar']}")
    print(f"{len(repos)} repositories, {len(analysed)} revisions, {len(pairs)} commit pairs\n")

    # ── D: determinism ───────────────────────────────────────
    rate = len(identical) / len(analysed) if analysed else 0.0
    d = "PASS" if rate >= DETERMINISM_MUST_BE else "STOP"
    print(f"D  determinism        {len(identical)}/{len(analysed)} byte-identical   [{d}]")
    for rev in analysed:
        if not rev.get("determinism", {}).get("ok"):
            print(f"     MISMATCH {rev['sha'][:12]}")

    # ── P: precision ─────────────────────────────────────────
    p_rate = len(with_errors) / len(pairs) if pairs else 0.0
    p = verdict(p_rate, PRECISION_PASS, PRECISION_STOP, higher_is_better=False)
    print(
        f"P  precision          {len(with_errors)}/{len(pairs)} pairs with an error "
        f"finding ({p_rate:.0%})   [{p}]"
    )
    print(f"     warn-severity, advisory: {len(with_warns)}/{len(pairs)} pairs")

    # ── R: resolution ────────────────────────────────────────
    percents = [
        r["coverage"]["overall"]["percent"]
        for r in repos
        if r.get("coverage", {}).get("overall")
    ]
    if percents:
        median = statistics.median(percents)
        r_verdict = verdict(median, RESOLUTION_PASS, RESOLUTION_STOP, higher_is_better=True)
        print(f"R  resolution         median {median:.1f}%   [{r_verdict}]")
        print(
            "     NOTE: this criterion does not measure what it was written to measure.\n"
            "     dispatch stamps every Tier 1 edge Resolution::Resolved, and the figure\n"
            "     counts resolved edges, so a repository with no Tier 2 files reports\n"
            "     100% by construction. It separates Tier 1 from Tier 2 regions, which is\n"
            "     real, but says nothing about references dropped inside a Tier 1 file."
        )
    else:
        print("R  resolution         no data   [INVESTIGATE]")

    # ── T: performance ───────────────────────────────────────
    if seconds:
        slowest = max(seconds)
        t = verdict(slowest, SECONDS_PASS, SECONDS_STOP, higher_is_better=False)
        print(
            f"T  performance        slowest {slowest:.2f}s, median "
            f"{statistics.median(seconds):.2f}s   [{t}]"
        )

    print()
    for r in repos:
        cov = (r.get("coverage") or {}).get("overall") or {}
        errs = sum(p["errors"] for p in r["pairs"])
        print(
            f"  {r['repo']:8} {r['language']:8} {r['files']:5} files  "
            f"{cov.get('total', 0):6} edges  "
            f"{max((x['seconds'] for x in r['revisions'] if x.get('ok')), default=0):5.2f}s max  "
            f"{errs} error finding(s)"
        )

    stopped = d == "STOP" or p == "STOP"
    print("\n" + ("STOP — a hard criterion failed." if stopped else "CONTINUE."))
    return 1 if stopped else 0


if __name__ == "__main__":
    sys.exit(main())
