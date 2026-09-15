#!/usr/bin/env python3
"""Measure OpenInvar against real repositories at real commits.

The thresholds this reports against are in `criteria.md`, fixed before the
first run. This script only measures; it does not decide, and it does not know
what a passing number is — `summarise.py` applies the criteria to the JSON this
writes, so a change to one cannot quietly move the other.

Usage:
    python3 eval/run.py --out eval/results/run.json [--commits N] [--repo NAME]
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# Real repositories, chosen for being ordinary rather than flattering: widely
# used, actively maintained, and written before OpenInvar existed, so nothing
# in them was shaped to suit it.
CORPUS = [
    {"name": "log", "language": "Rust", "url": "https://github.com/rust-lang/log"},
    {"name": "cobra", "language": "Go", "url": "https://github.com/spf13/cobra"},
    {"name": "click", "language": "Python", "url": "https://github.com/pallets/click"},
    {"name": "gson", "language": "Java", "url": "https://github.com/google/gson"},
]


def run(cmd, cwd=None, timeout=900):
    """Run a command, returning (exit code, stdout, stderr)."""
    proc = subprocess.run(
        cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout
    )
    return proc.returncode, proc.stdout, proc.stderr


def clone(entry, workdir, depth):
    """Shallow-clone enough history to walk `depth` commits."""
    target = workdir / entry["name"]
    code, _, err = run(
        ["git", "clone", "--quiet", "--depth", str(depth + 2), entry["url"], str(target)]
    )
    if code != 0:
        return None, err.strip()
    return target, None


def recent_commits(repo, count):
    """The newest `count` commits, oldest first, so pairs read forwards."""
    code, out, _ = run(["git", "-C", str(repo), "log", "--format=%H", f"-{count}"])
    if code != 0:
        return []
    return list(reversed(out.split()))


def analyze(binary, repo, sha, json_out=False):
    """Analyse one revision. Returns (seconds, exit code, stdout)."""
    cmd = [binary, "analyze", str(repo), "--at", sha]
    if json_out:
        cmd.append("--json")
    started = time.monotonic()
    code, out, err = run(cmd)
    elapsed = time.monotonic() - started
    return elapsed, code, (out if json_out else err + out)


def determinism_check(binary, repo, sha):
    """Analyse one revision twice and compare the documents byte for byte.

    The two runs land in different temporary worktrees on different paths, so a
    path leaking into the output shows up here rather than staying invisible.
    """
    _, code_a, first = analyze(binary, repo, sha, json_out=True)
    _, code_b, second = analyze(binary, repo, sha, json_out=True)
    if code_a != 0 or code_b != 0:
        return {"ok": False, "reason": "analysis failed", "bytes": 0}
    return {
        "ok": first == second,
        "reason": "" if first == second else "two runs differed",
        "bytes": len(first.encode()),
    }


def coverage_of(binary, repo):
    """Resolution coverage as `status` reports it, per language and overall.

    A plain `analyze` first, because `status` reads the *working* graph and
    `--at` deliberately never writes one — it records a revision and leaves the
    present-tense graph alone. This measures the checked-out HEAD, which is a
    real revision of a real repository and the honest thing to quote.
    """
    run([binary, "analyze", str(repo)])
    code, out, _ = run([binary, "status", str(repo)])
    if code != 0:
        return {}
    import re

    plain = re.sub(r"\x1b\[[0-9;]*m", "", out)
    overall = None
    references = None
    per_language = {}
    for line in plain.splitlines():
        m = re.search(r"References bound\s+([\d.]+)%\s+\((\d+) of (\d+)", line)
        if m:
            # Criterion R', and a different question from the line below it:
            # this denominator counts references named in source, including the
            # ones that bound to nothing and so produced no edge to count.
            references = {
                "percent": float(m.group(1)),
                "bound": int(m.group(2)),
                "attempted": int(m.group(3)),
            }
            continue
        # The unbound remainder, split by whether an adapter could show the
        # reference was never the repository's to resolve. Without this, R'
        # cannot tell a prelude name from work the adapter did not do.
        m = re.search(r"(\d+) name something outside it", line)
        if m and references is not None:
            references["outside_repository"] = int(m.group(1))
            continue
        m = re.search(r"(\d+) are unexplained", line)
        if m and references is not None:
            references["unexplained"] = int(m.group(1))
            continue
        m = re.search(r"Resolved\s+([\d.]+)%\s+\((\d+) of (\d+)", line)
        if m:
            overall = {
                "percent": float(m.group(1)),
                "resolved": int(m.group(2)),
                "total": int(m.group(3)),
            }
            continue
        m = re.match(r"\s+(\S[\w/+#]*)\s+([\d.]+)% of (\d+) edge", line)
        if m:
            per_language[m.group(1)] = {
                "percent": float(m.group(2)),
                "edges": int(m.group(3)),
            }
    return {
        "overall": overall,
        "references": references,
        "per_language": per_language,
    }


def audit_pair(binary, repo, before, after):
    """Audit one commit pair, returning findings grouped by severity."""
    code, out, _ = run([binary, "audit", str(repo), "--base", before, "--head", after, "--json"])
    if code not in (0, 1):
        return {"ran": False, "exit": code, "findings": []}
    try:
        doc = json.loads(out)
    except json.JSONDecodeError:
        return {"ran": False, "exit": code, "findings": []}

    findings = doc.get("findings", []) or []
    return {
        "ran": True,
        "exit": code,
        "errors": [f for f in findings if f.get("severity") == "error"],
        "warns": [f for f in findings if f.get("severity") == "warn"],
        "findings": findings,
    }


def file_count(repo):
    code, out, _ = run(["git", "-C", str(repo), "ls-files"])
    return len(out.split()) if code == 0 else 0


def evaluate(entry, binary, workdir, commits):
    print(f"\n=== {entry['name']} ({entry['language']})", flush=True)
    repo, err = clone(entry, workdir, commits)
    if repo is None:
        print(f"    clone failed: {err}", flush=True)
        return {"repo": entry["name"], "language": entry["language"], "error": err}

    shas = recent_commits(repo, commits)
    print(f"    {len(shas)} commits, {file_count(repo)} tracked files", flush=True)

    result = {
        "repo": entry["name"],
        "language": entry["language"],
        "url": entry["url"],
        "files": file_count(repo),
        "revisions": [],
        "pairs": [],
    }

    for sha in shas:
        elapsed, code, _ = analyze(binary, repo, sha)
        if code != 0:
            result["revisions"].append({"sha": sha, "ok": False, "seconds": elapsed})
            print(f"    {sha[:8]} analysis FAILED", flush=True)
            continue
        det = determinism_check(binary, repo, sha)
        result["revisions"].append(
            {"sha": sha, "ok": True, "seconds": round(elapsed, 2), "determinism": det}
        )
        mark = "ok " if det["ok"] else "DIFFERS"
        print(f"    {sha[:8]} {elapsed:6.2f}s  determinism {mark}", flush=True)

    result["coverage"] = coverage_of(binary, repo)

    for before, after in zip(shas, shas[1:]):
        audit = audit_pair(binary, repo, before, after)
        entry_ = {
            "before": before,
            "after": after,
            "ran": audit["ran"],
            "errors": len(audit.get("errors", [])),
            "warns": len(audit.get("warns", [])),
            "detectors": sorted({f.get("detector", "?") for f in audit.get("findings", [])}),
        }
        result["pairs"].append(entry_)
        if entry_["errors"]:
            print(
                f"    {before[:8]}..{after[:8]}  {entry_['errors']} error finding(s): "
                f"{entry_['detectors']}",
                flush=True,
            )

    shutil.rmtree(repo, ignore_errors=True)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/release/openinvar")
    parser.add_argument("--commits", type=int, default=8)
    parser.add_argument("--repo", help="run only this corpus entry")
    parser.add_argument("--out", default="eval/results/run.json")
    args = parser.parse_args()

    binary = str(Path(args.binary).resolve())
    if not os.access(binary, os.X_OK):
        sys.exit(f"no executable at {binary}; build with: cargo build --release")

    _, version, _ = run([binary, "--version"])
    corpus = [e for e in CORPUS if not args.repo or e["name"] == args.repo]

    results = {
        "openinvar": version.strip(),
        "commits_per_repo": args.commits,
        "repos": [],
    }

    with tempfile.TemporaryDirectory(prefix="openinvar-eval-") as tmp:
        workdir = Path(tmp)
        for entry in corpus:
            results["repos"].append(evaluate(entry, binary, workdir, args.commits))

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(results, indent=2, sort_keys=True) + "\n")
    print(f"\nwrote {out}")


if __name__ == "__main__":
    main()
