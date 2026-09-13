#!/bin/sh
# PreToolUse on Edit/Write — say what the file about to change reaches.
#
# This hook never blocks. It answers a question the agent did not ask, and an
# unasked question is not grounds for refusing an edit. Its whole job is to put
# the blast radius in front of the model before the write, so a rename that
# breaks fourteen callers is visible at the moment it is decided rather than
# when CI fails.
#
# Input: Claude Code hook JSON on stdin, with tool_input.file_path.
# Output: exit 0, with hookSpecificOutput.additionalContext when there is
# something worth saying. Silence when there is not — an agent's context window
# is not free.

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

TIMEOUT="${OPENINVAR_HOOK_TIMEOUT:-5}"

payload=$(cat)
file=$(openinvar_json_field '.tool_input.file_path' "$payload")

[ -n "$file" ] || exit 0

bin=$(openinvar_bin)
[ -n "$bin" ] || exit 0
openinvar_has_graph || exit 0

summary=$(openinvar_with_timeout "$TIMEOUT" "$bin" impact "$file" --json 2>/dev/null)
status=$?

# A timeout, a crash, or an unparseable answer all mean the same thing: this
# hook has nothing to contribute to this edit. None of them is the agent's
# problem, and none of them is a reason to interrupt it.
[ $status -eq 0 ] || exit 0
[ -n "$summary" ] || exit 0

context=$(printf '%s' "$summary" | OPENINVAR_FILE="$file" awk '
  # Reads only the scalar count fields, whose keys appear exactly once.
  # Counting occurrences of "file" instead would also count one per edge.
  function jnum(key,   pattern) {
    pattern = "\"" key "\":[0-9]+"
    if (match($0, pattern)) {
      # The matched prefix is a quote, the key, a quote and a colon:
      # length(key) + 3 characters before the digits begin.
      return substr($0, RSTART + length(key) + 3, RLENGTH - length(key) - 3) + 0
    }
    return 0
  }
  {
    defined = jnum("symbols_defined")
    files   = jnum("dependent_file_count")
    edges   = jnum("edge_count")
    blind   = jnum("blind_spots")
    gate    = ($0 ~ /"gate_safe":true/)

    if (defined == 0) exit 0

    if (files == 0) {
      # Only worth saying when the graph could have missed something.
      if (blind > 0) {
        printf "OpenInvar: nothing recorded as depending on %s, but %d file(s) were analyzed within-file only, so a cross-file reference could exist without appearing here.\n", ENVIRON["OPENINVAR_FILE"], blind
      }
      exit 0
    }

    printf "OpenInvar blast radius for %s: %d file(s) and %d reference(s) depend on symbols defined here.", ENVIRON["OPENINVAR_FILE"], files, edges
    if (!gate) printf " Some of those edges are structural, so treat them as advisory."
    printf " Check `openinvar impact` before renaming or removing anything in this file.\n"
  }
')

[ -n "$context" ] || exit 0

# Escape for embedding in the JSON string below.
context=$(printf '%s' "$context" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ')

printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"%s"}}\n' "$context"
exit 0
