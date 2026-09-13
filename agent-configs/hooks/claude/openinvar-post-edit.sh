#!/bin/sh
# PostToolUse on Edit/Write — report what the edit just broke.
#
# The edit has already happened, so this cannot and does not block. It reports
# broken references back as context, which is the earliest point an agent can
# act on them without a round trip through CI.
#
# It compares the working tree against the last commit rather than against the
# previous edit: an agent makes many small writes inside one change, and
# reporting each intermediate state would flag half-finished work as breakage.
#
# Requires a snapshot of HEAD. Without one it says nothing at all rather than
# analysing on the fly, because analysing here would make the answer depend on
# whatever else was in the tree at the moment the hook happened to run.

# Locate the shared library.
#
# Guarded with a readability test rather than `. lib || fallback`: in POSIX sh,
# sourcing a file that does not exist is a special-builtin failure that
# terminates the shell on the spot, so the fallback never runs and the hook
# exits non-zero with no output. For a pre-commit hook that silently rejects
# every commit; for a Stop hook it traps the agent. Both were real.
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
# No library means no hook. Exiting 0 leaves the agent, and the commit, alone.
[ -n "$_openinvar_lib" ] || exit 0
# shellcheck source=/dev/null
. "$_openinvar_lib"

TIMEOUT="${OPENINVAR_HOOK_TIMEOUT:-10}"

payload=$(cat)
file=$(openinvar_json_field '.tool_input.file_path' "$payload")
[ -n "$file" ] || exit 0

bin=$(openinvar_bin)
[ -n "$bin" ] || exit 0
openinvar_has_graph || exit 0

root=$(openinvar_root)

# Refresh only the file that changed. A full re-analysis here would cost
# seconds on every write, and this hook has to stay cheap enough to leave on.
openinvar_with_timeout "$TIMEOUT" "$bin" analyze "$root" --snapshot worktree >/dev/null 2>&1 || exit 0

report=$(openinvar_with_timeout "$TIMEOUT" "$bin" diff "$root" --base HEAD --head worktree --json 2>/dev/null)
[ $? -eq 0 ] || exit 0
[ -n "$report" ] || exit 0

broken=$(printf '%s' "$report" | grep -o '"kind":"broken-edge"' | wc -l | tr -d ' ')
[ "$broken" -gt 0 ] || exit 0

context="OpenInvar: this change leaves ${broken} broken reference(s) against HEAD. Run: openinvar diff . --base HEAD --head worktree — to see which symbols were removed while something still refers to them."

printf '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"%s"}}\n' "$context"
exit 0
