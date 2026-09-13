#!/usr/bin/env bash
# Post the report, updating the previous comment rather than adding another.
#
# A pull request with nine identical bot comments is one where nobody reads the
# tenth, so the report carries a marker and this finds and edits it.
#
# Never fails the job. A missing permission is a reason the comment did not
# appear, not a reason to reject the change — and the report is on the job
# summary either way.
set -uo pipefail

report="${OPENINVAR_REPORT:-}"
if [ -z "$report" ] || [ ! -s "$report" ]; then
  echo "No report to post."
  exit 0
fi

number="$(jq -r '.pull_request.number // empty' "$GITHUB_EVENT_PATH" 2>/dev/null)"
if [ -z "$number" ]; then
  echo "Not a pull request; skipping the comment."
  exit 0
fi

marker="<!-- openinvar-report -->"
api="repos/${GITHUB_REPOSITORY}/issues"

existing="$(gh api "${api}/${number}/comments" --paginate \
  --jq "map(select(.body | contains(\"${marker}\"))) | .[0].id // empty" 2>/dev/null)"

if [ -n "$existing" ]; then
  if gh api --method PATCH "repos/${GITHUB_REPOSITORY}/issues/comments/${existing}" \
      -F body=@"$report" >/dev/null 2>&1; then
    echo "Updated comment $existing."
    exit 0
  fi
  echo "::warning::Could not update the existing OpenInvar comment; posting a new one." >&2
fi

if gh api --method POST "${api}/${number}/comments" -F body=@"$report" >/dev/null 2>&1; then
  echo "Posted a new comment."
else
  # The most common cause by far, and worth naming rather than leaving the
  # reader to guess from an HTTP status.
  echo "::warning::Could not post the OpenInvar comment. The workflow needs 'permissions: pull-requests: write'. The report is on the job summary." >&2
fi
exit 0
