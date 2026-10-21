#!/usr/bin/env bash
#
# Reports a named scheduled workflow whose most recent run failed - the gap
# that let six consecutive nightly Distribution failures go unnoticed for a
# week (#750). floor-health-check.yml's two sweeps only ever look at pull
# requests and issues, never at a scheduled workflow's own run history, so
# nothing was watching the guard itself.
#
# A single failure is reported once; a run of several in a row is reported
# louder, naming how many - the point is that nobody has to open a list of
# runs to tell "one bad night" from "this guard has stopped working".
#
# A file rather than shell inside floor-health-check.yml so that
# `.github/tests/scheduled-workflow-health.sh` can run it, the same way
# `wait-for-floor.sh` is tested.

set -uo pipefail

# How many of $1's most recent completed runs failed in a row, most recent
# first. Stops at the first non-failure, so a streak that has since
# recovered does not inflate today's count.
failure_streak() {
  local workflow="$1"
  local runs
  # A listing that could not be made is not a healthy streak, for the same
  # reason as wait-for-floor.sh's open_pull_requests: reading it that way
  # would let a broken `gh` - no token, an outage, a renamed workflow - pass
  # for "nothing to report" instead of failing loudly.
  # This function's stdout is captured by the caller as the streak count, so
  # the error goes to stderr instead - printed to stdout it would end up
  # silently assigned to that count rather than ever being seen.
  if ! runs=$(gh run list --repo "${REPO}" --workflow "${workflow}" --status completed \
    --limit "${HISTORY_LIMIT:-20}" --json conclusion); then
    echo "::error::Could not list runs of ${workflow}; refusing to treat that as a healthy streak." >&2
    return 1
  fi
  python3 -c '
import json, sys

runs = json.load(sys.stdin)
streak = 0
for run in runs:
    if run.get("conclusion") == "failure":
        streak += 1
    else:
        break
print(streak)
' <<< "${runs}"
}

check_scheduled_workflows() {
  : "${REPO:?}"
  # `gh` needs a token: Actions does not put one in the environment on its
  # own, and a missing token must not be indistinguishable from a healthy
  # floor.
  if [ -z "${GH_TOKEN:-}" ] && [ -z "${GITHUB_TOKEN:-}" ]; then
    echo "::error::Neither GH_TOKEN nor GITHUB_TOKEN is set, so this cannot see the watched workflows' run history."
    return 1
  fi
  local workflows="${WATCHED_SCHEDULED_WORKFLOWS:?}"
  local failed=0
  local workflow streak
  for workflow in ${workflows}; do
    streak=$(failure_streak "${workflow}") || { failed=1; continue; }
    case "${streak}" in
      ''|*[!0-9]*)
        echo "::error::Asked how many of ${workflow}'s runs failed in a row and got '${streak}'."
        failed=1
        continue
        ;;
    esac
    if [ "${streak}" -eq 0 ]; then
      echo "${workflow}: last scheduled run succeeded."
    elif [ "${streak}" -eq 1 ]; then
      echo "::error::${workflow}: the last scheduled run failed."
      failed=1
    else
      echo "::error::${workflow}: the last ${streak} scheduled runs in a row have failed - this guard has stopped working, not just had a blip."
      failed=1
    fi
  done
  return "${failed}"
}

# Sourced by the test, which then calls the function above directly with its
# own stub `gh`.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  check_scheduled_workflows
  exit $?
fi
