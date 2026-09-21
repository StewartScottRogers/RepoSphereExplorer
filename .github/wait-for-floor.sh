#!/usr/bin/env bash
#
# Blocks until this run is the oldest unfinished run of the floor's two
# build entry points (claude.yml, factory-shift.yml), then returns.
#
# This replaces the `concurrency: dark-factory-floor` group the two
# workflows used to share. That group's comment said "queue instead of
# cancelling, so no triggering issue or comment is silently dropped" - but
# a concurrency group holds only one *pending* run: a fresh trigger cancels
# whatever was already pending, it does not queue behind it. On
# 2026-09-21 a burst of twenty work orders cancelled nineteen of
# themselves down to one, and the same burst also cancelled a factory
# shift that had been dispatched earlier and was waiting its turn in the
# group - it never started, before any of the twenty ran either. See #733.
#
# So every triggering run now actually starts (nothing is ever left
# pending, so nothing is ever cancelled to make room for another), and
# waits here instead: polling the other unfinished runs of both workflows
# until none of them is older than this one. Run IDs are assigned in
# strictly increasing order repository-wide, so comparing them stands in
# for comparing start times without a second query, and gives a total
# order that only one run at a time can be the front of - no two runs can
# both conclude they are the oldest.
#
# A file rather than shell inside the workflow so that
# `.github/tests/wait-for-floor.sh` can run it.

set -uo pipefail

# The oldest run ID, from the JSON on stdin, that is neither this run nor
# already completed, and is older than this run. Empty when there is none,
# meaning it is this run's turn.
oldest_unfinished_run_ahead_of_us() {
  RUN_ID="${RUN_ID}" python3 -c '
import json, os, sys

runs = json.load(sys.stdin)
self_id = int(os.environ["RUN_ID"])
ahead = [
    r["databaseId"] for r in runs
    if r["databaseId"] != self_id
    and r.get("status") != "completed"
    and r["databaseId"] < self_id
]
print(min(ahead) if ahead else "")
'
}

# Every run, queued or in progress, of both build workflows - fetched
# separately per workflow because `gh run list` cannot filter by more than
# one.
runs_across_the_floor() {
  python3 -c '
import json, sys
merged = []
for chunk in sys.argv[1:]:
    merged.extend(json.loads(chunk))
print(json.dumps(merged))
' "$@"
}

wait_for_floor() {
  : "${REPO:?}" "${RUN_ID:?}"
  local workflows="${WORKFLOWS:-claude.yml factory-shift.yml}"
  local poll_seconds="${POLL_SECONDS:-30}"
  local max_attempts="${MAX_ATTEMPTS:-600}"

  local attempt pages workflow page ahead
  for attempt in $(seq 1 "${max_attempts}"); do
    pages=()
    for workflow in ${workflows}; do
      page=$(gh run list --repo "${REPO}" --workflow "${workflow}" --limit 100 \
        --json databaseId,status 2>/dev/null) || page="[]"
      pages+=("${page}")
    done

    ahead=$(runs_across_the_floor "${pages[@]}" | oldest_unfinished_run_ahead_of_us)
    if [ -z "${ahead}" ]; then
      echo "Floor is clear: run ${RUN_ID} goes now."
      return 0
    fi
    echo "Run ${ahead} is ahead of us and still unfinished (attempt ${attempt}/${max_attempts}); waiting ${poll_seconds}s..."
    sleep "${poll_seconds}"
  done

  echo "::error::Waited $((max_attempts * poll_seconds / 60)) minutes for the floor to clear and it never did."
  echo "::error::Something ahead of run ${RUN_ID} is stuck; a human needs to look before this queue moves again."
  return 1
}

# Sourced by the test, which then calls the functions above directly with
# its own stub `gh` and small attempt/poll counts.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  wait_for_floor
  exit $?
fi
