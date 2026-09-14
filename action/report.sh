#!/usr/bin/env bash
# Record a snapshot of each side and build the markdown report.
#
# Both sides are analyzed from the same checkout so that both snapshots land in
# one `.openinvar/graph.db`, which is what lets `report` compare them. The base is
# checked out, analyzed, and the head restored — the working tree is left as it
# was found.
set -euo pipefail

path="${OPENINVAR_PATH:-.}"
cd "$path"

# What to compare against. The pull request base is the useful default; a push
# compares against the commit it replaced.
base_ref="${OPENINVAR_BASE_REF:-}"
if [ -z "$base_ref" ]; then
  if [ "${GITHUB_EVENT_NAME:-}" = "pull_request" ]; then
    base_ref="origin/${GITHUB_BASE_REF}"
  else
    base_ref="${GITHUB_SHA:-HEAD}^"
  fi
fi

head_sha="$(git rev-parse HEAD)"

# A shallow clone has no base commit to compare against. Deepening is cheap and
# the alternative is a report that silently describes the wrong range.
if ! git rev-parse --verify --quiet "${base_ref}^{commit}" >/dev/null; then
  echo "Fetching $base_ref (shallow clone)."
  git fetch --no-tags --depth=50 origin "+refs/heads/*:refs/remotes/origin/*" >/dev/null 2>&1 || true
fi

if ! base_tip="$(git rev-parse --verify --quiet "${base_ref}^{commit}")"; then
  echo "::error::Could not resolve base revision '$base_ref'. Pass base-ref explicitly, or set fetch-depth: 0 on actions/checkout." >&2
  exit 1
fi

# Compare against the merge base, not the tip of the base branch.
#
# The tip is wrong and wrong in the worst direction: everything merged into the
# base branch since this one diverged then reads as removed by this change. On
# OpenInvar's own pull request that produced a comment claiming a file the branch
# never touched had lost most of its symbols. A confidently wrong answer is the
# one thing this tool must not produce, so the range is the branch's own work
# and nothing else.
if base_sha="$(git merge-base "$base_tip" "$head_sha" 2>/dev/null)" && [ -n "$base_sha" ]; then
  [ "$base_sha" = "$base_tip" ] || echo "Using merge base $base_sha rather than the tip of $base_ref."
else
  # No common ancestor found — an unrelated history, or a clone too shallow to
  # reach one. Saying so beats silently comparing across the whole divergence.
  echo "::warning::No merge base between $base_ref and HEAD; comparing against the branch tip. Set fetch-depth: 0 on actions/checkout for an accurate range." >&2
  base_sha="$base_tip"
fi

if [ "$base_sha" = "$head_sha" ]; then
  echo "Base and head are the same commit; nothing to compare."
  {
    echo "verdict=clean"
    echo "report-path="
  } >> "$GITHUB_OUTPUT"
  exit 0
fi

echo "Comparing ${base_sha} -> ${head_sha}"

# The base tree, snapshotted under its own SHA so the record still means
# something after the branch moves.
git checkout --quiet --force "$base_sha"
openinvar analyze . --snapshot "$base_sha" >/dev/null

git checkout --quiet --force "$head_sha"
openinvar analyze . --snapshot "$head_sha" >/dev/null

report_path="${RUNNER_TEMP:-/tmp}/openinvar-report.md"
rules_arg=()
[ -n "${OPENINVAR_RULES:-}" ] && rules_arg=(--rules "${OPENINVAR_RULES}")
# A missing rules file is a clean report having enforced nothing, which is the
# one outcome a gate must never produce silently.
[ "${OPENINVAR_REQUIRE_RULES:-false}" = "true" ] && rules_arg+=(--require-rules)

set +e
openinvar report . --base "$base_sha" --head "$head_sha" "${rules_arg[@]}" > "$report_path"
status=$?
set -e

case $status in
  0) verdict=clean ;;
  1) verdict=violated ;;
  *)
    # The report could not be produced. That is not a violation and must not be
    # reported as one — nor as a clean run.
    echo "::warning::OpenInvar could not produce a report (exit $status). Nothing was enforced." >&2
    cat "$report_path" >&2 || true
    verdict=undecided
    ;;
esac

# "undecided" also covers a report that was produced but could not decide every
# rule, which the markdown states in its first line.
if [ "$verdict" = "clean" ] && grep -q "could not be decided" "$report_path" 2>/dev/null; then
  verdict=undecided
fi

# The audit is a separate question from the rules, and its answer belongs in
# the same comment: a reviewer should not have to open a job log to find out
# that a change looks like it was made to pass a check.
audit_path="${RUNNER_TEMP:-/tmp}/openinvar-audit.txt"
set +e
openinvar audit . --base "$base_sha" --head "$head_sha" --severity "${OPENINVAR_AUDIT_SEVERITY:-error}" > "$audit_path" 2>&1
audit_status=$?
set -e

if [ $audit_status -eq 1 ]; then
  {
    echo
    echo "### Audit"
    echo
    echo "This change contains something that looks like it was made to pass a check rather than to work."
    echo
    echo '```'
    sed 's/\x1b\[[0-9;]*m//g' "$audit_path"
    echo '```'
  } >> "$report_path"
  verdict=audit
elif [ $audit_status -ne 0 ]; then
  echo "::warning::OpenInvar could not run the audit (exit $audit_status); no audit was performed." >&2
fi

echo "Verdict: $verdict"
{
  echo "verdict=$verdict"
  echo "report-path=$report_path"
} >> "$GITHUB_OUTPUT"

cat "$report_path" >> "$GITHUB_STEP_SUMMARY"
