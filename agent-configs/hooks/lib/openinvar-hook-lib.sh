#!/bin/sh
# Shared helpers for OpenInvar hooks.
#
# Every rule in here exists because a hook that misbehaves once gets deleted,
# and a deleted hook enforces nothing. The order of priorities is: never hang,
# never block on a tool problem, never lie about what was checked. Being
# informative comes fourth.

# Where the repository root is, from the hook's working directory.
openinvar_root() {
  git rev-parse --show-toplevel 2>/dev/null || pwd
}

# The openinvar binary, or empty if there is none.
#
# Not an error when absent: a repository is shared by people who have not
# installed OpenInvar, and their commits are not ours to reject.
openinvar_bin() {
  if [ -n "$OPENINVAR_BIN" ] && [ -x "$OPENINVAR_BIN" ]; then
    printf '%s' "$OPENINVAR_BIN"
    return 0
  fi
  command -v openinvar 2>/dev/null
}

# Whether a graph exists to query.
#
# Without one every answer would be "nothing depends on this", which is the
# single most dangerous thing these hooks could say.
openinvar_has_graph() {
  # A file, not a directory: the store is SQLite. Testing for the directory an
  # older release wrote would report "no graph" forever after an upgrade, and
  # these hooks fail open — so the hook would go quiet rather than go wrong,
  # which is the failure nobody notices.
  [ -f "$(openinvar_root)/.openinvar/graph.db" ]
}

# Run a command under a wall-clock limit.
#
# `timeout` is GNU coreutils and is missing on stock macOS, so fall back to a
# background process and a poll. A hook that waits on a slow query is a hook
# that gets switched off, and the fallback matters more than its tidiness.
openinvar_with_timeout() {
  seconds="$1"
  shift
  if command -v timeout >/dev/null 2>&1; then
    timeout "$seconds" "$@"
    return $?
  fi

  "$@" &
  pid=$!
  waited=0
  while kill -0 "$pid" 2>/dev/null; do
    if [ "$waited" -ge "$seconds" ]; then
      kill -TERM "$pid" 2>/dev/null
      return 124
    fi
    sleep 1
    waited=$((waited + 1))
  done
  wait "$pid"
}

# Read one string field out of a flat JSON object on stdin.
#
# `jq` when it is there. The fallback handles the one shape these hooks need —
# a quoted string value — and deliberately returns nothing rather than a guess
# when it cannot: an empty path makes the hook say "no file", which is safe,
# while a wrong path makes it report another file's dependents, which is not.
openinvar_json_field() {
  field="$1"
  payload="$2"
  if command -v jq >/dev/null 2>&1; then
    printf '%s' "$payload" | jq -r "$field // empty" 2>/dev/null
    return 0
  fi
  # Only the leaf name is used by the fallback; nesting is not parsed.
  leaf=$(printf '%s' "$field" | sed 's/.*\.//')
  printf '%s' "$payload" \
    | tr ',' '\n' \
    | grep "\"$leaf\"[[:space:]]*:" \
    | head -1 \
    | sed 's/.*"'"$leaf"'"[[:space:]]*:[[:space:]]*"\(.*\)"[^"]*$/\1/' \
    | sed 's/\\\\\//\//g'
}
