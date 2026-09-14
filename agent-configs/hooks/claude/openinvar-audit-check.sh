#!/bin/sh
# Stop — refuse to finish on an audit finding.
#
# The counterpart to openinvar-stop-check.sh. That one enforces the rules a
# repository wrote down; this one reports changes that look like they were made
# to pass a check rather than to work — which is the failure mode a rule file
# cannot describe in advance.
#
# Only exit 1 from `audit` blocks: a finding at the configured severity. Exit 2
# means the audit could not run at all, and holding an agent over a missing
# snapshot would trap it in a loop it has no way to fix.
#
# Findings are advisory by default here. The severity floor is `error`, so the
# two high-confidence error detectors block and dead-on-arrival does not; set
# OPENINVAR_AUDIT_SEVERITY=warn to hold on everything.

_openinvar_lib=""
for _candidate in \
  "$(dirname "$0")/openinvar-hook-lib.sh" \
  "$(dirname "$0")/../lib/openinvar-hook-lib.sh" \
  "$(git rev-parse --show-toplevel 2>/dev/null)/agent-configs/hooks/lib/openinvar-hook-lib.sh"
do
  if [ -r "$_candidate" ]; then
    _openinvar_lib="$_candidate"
    break
  fi
done
[ -n "$_openinvar_lib" ] || exit 0
# shellcheck source=/dev/null
. "$_openinvar_lib"

TIMEOUT="${OPENINVAR_HOOK_TIMEOUT:-30}"
SEVERITY="${OPENINVAR_AUDIT_SEVERITY:-error}"

bin=$(openinvar_bin)
[ -n "$bin" ] || exit 0
openinvar_has_graph || exit 0

root=$(openinvar_root)

output=$(openinvar_with_timeout "$TIMEOUT" "$bin" audit "$root" --base HEAD --head worktree --severity "$SEVERITY" 2>&1)
status=$?

if [ $status -eq 1 ]; then
  printf 'OpenInvar audit found changes that look like they were made to pass a check rather than to work:\n\n%s\n\nFix the change, or record a deliberate exception in the [suppress] table of openinvar.toml with a reason.\n' "$output" >&2
  exit 2
fi

# 0 = nothing found. 2 = the audit could not run. 124 = timed out. None of
# these is a finding, and reporting one for any of them would be the same
# unearned claim the audit exists to catch.
exit 0
