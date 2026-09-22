#!/usr/bin/env bash
#
# Fails loudly when a named scheduled workflow's most recent scheduled run
# was not a success, and louder still when several in a row were not.
#
# The nightly Distribution check failed every night from 2026-09-16 to
# 2026-09-21 and nothing on the floor said so (#750): floor-health-check.yml
# runs every 30 minutes and reports success regardless, because its two
# sweeps look at pull requests and issues and never at a scheduled
# workflow's own run history. A guard that cannot go red when the thing it
# guards has been red for six days is not a guard.
#
# A file rather than shell inside the workflow so that
# `.github/tests/scheduled-workflow-health.sh` can run it.

set -uo pipefail

: "${REPO:?}"
: "${WATCHED_SCHEDULED_WORKFLOWS:?}"

# How many of a workflow's most recent scheduled runs to look back across
# when counting a failing streak.
: "${RUN_HISTORY:=30}"

# How many of the most recent runs in $1 (a JSON array, newest first, of
# {"conclusion": ...} objects) are not "success", counting from the front
# and stopping at the first one that is - i.e. the current losing streak.
# Zero when the newest run succeeded.
failing_streak() {
  python3 -c '
import json, sys

runs = json.load(sys.stdin)
streak = 0
for run in runs:
    if run.get("conclusion") == "success":
        break
    streak += 1
print(streak)
'
}

# Checks every workflow named in $WATCHED_SCHEDULED_WORKFLOWS (space
# separated), and returns non-zero if any of them is on a losing streak.
check_scheduled_workflows() {
  local workflow runs count exit_code=0

  for workflow in ${WATCHED_SCHEDULED_WORKFLOWS}; do
    # A listing that could not be made is not a clean bill of health - see
    # wait-for-floor.sh's identical reasoning for why this refuses rather
    # than reads as "no failures".
    if ! runs=$(gh run list --repo "${REPO}" --workflow "${workflow}" --event schedule \
      --json conclusion,createdAt --limit "${RUN_HISTORY}"); then
      echo "::error::Could not list ${workflow}'s scheduled runs; a health check that cannot see a workflow's history is not watching it."
      exit_code=1
      continue
    fi

    count=$(printf '%s' "${runs}" | python3 -c 'import json, sys; print(len(json.load(sys.stdin)))')
    if [ "${count}" = 0 ]; then
      echo "${workflow}: no scheduled runs yet."
      continue
    fi

    local streak
    streak=$(printf '%s' "${runs}" | failing_streak)
    if [ "${streak}" = 0 ]; then
      echo "${workflow}: last scheduled run was a success."
      continue
    fi

    local newest
    newest=$(printf '%s' "${runs}" | python3 -c 'import json, sys; runs = json.load(sys.stdin); print(runs[0].get("conclusion") or "unknown")')
    exit_code=1
    if [ "${streak}" -ge 2 ]; then
      echo "::error::${workflow} has failed ${streak} consecutive scheduled runs (most recent conclusion: ${newest}). A guard nobody reads is not a guard; see #750."
    else
      echo "::error::${workflow}'s last scheduled run was a ${newest}."
    fi
  done

  return "${exit_code}"
}

# Sourced by the test, which then calls the function above directly with
# its own stub `gh`.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  check_scheduled_workflows
  exit $?
fi
