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

# How many pull requests are open and waiting to land. A shift branches from
# `main` as it is when the shift starts, so starting one while a pull request
# is still open produces a branch without it that then conflicts with it -
# which is why `day-shift.yml` refuses to dispatch while one is open, and why
# the shift's own self-chain waits.
#
# Neither guard covers `factory-shift.yml`'s own `0 3 * * *` schedule, and a
# scheduled run checks nothing: on 2026-09-24 the 08:04 scheduled shift opened
# #769 while #768 had been open since the night before. `claude.yml`'s
# label-triggered path has never had such a guard at all. Both call this
# script, so the rule lives here once.
#
# A listing that cannot be made is not an empty floor, for the same reason as
# below.
open_pull_requests() {
  local count
  if ! count=$(gh pr list --repo "${REPO}" --label auto-merge --state open     --json number --jq 'length'); then
    echo "::error::Could not list open pull requests; refusing to treat that as none."
    return 1
  fi
  case "${count}" in
    ''|*[!0-9]*)
      echo "::error::Asked how many pull requests are open and got '${count}'."
      return 1
      ;;
  esac
  printf '%s' "${count}"
}

wait_for_floor() {
  : "${REPO:?}" "${RUN_ID:?}"
  # `gh` needs a token: Actions does not put one in the environment on its
  # own. Checked here rather than left to `gh`, because a missing token used
  # to be indistinguishable from an empty floor - see the loud failure below.
  if [ -z "${GH_TOKEN:-}" ] && [ -z "${GITHUB_TOKEN:-}" ]; then
    echo "::error::Neither GH_TOKEN nor GITHUB_TOKEN is set, so this step cannot see the rest of the floor."
    echo "::error::Set GH_TOKEN on the step that calls this script; without it the wait would pass every run straight through."
    return 1
  fi
  local workflows="${WORKFLOWS:-claude.yml factory-shift.yml}"
  local poll_seconds="${POLL_SECONDS:-30}"
  local max_attempts="${MAX_ATTEMPTS:-600}"

  local attempt pages workflow page ahead
  for attempt in $(seq 1 "${max_attempts}"); do
    pages=()
    for workflow in ${workflows}; do
      # Never `|| page="[]"`. A failed listing is not an empty floor: read
      # that way, a broken `gh` - no token, an outage, a renamed workflow -
      # would look exactly like "nobody is ahead of you", and every run would
      # be waved through while the step reported a clear floor. That is the
      # fault this whole script exists to remove, so it fails loudly instead.
      if ! page=$(gh run list --repo "${REPO}" --workflow "${workflow}" --limit 100 \
        --json databaseId,status); then
        echo "::error::Could not list runs of ${workflow}; refusing to treat that as an empty floor."
        return 1
      fi
      pages+=("${page}")
    done

    ahead=$(runs_across_the_floor "${pages[@]}" | oldest_unfinished_run_ahead_of_us)
    if [ -z "${ahead}" ]; then
      # Nothing is building. One more question before going: is anything
      # waiting to land? Skipped where the caller is not a build - a review
      # answering a comment has nothing to branch from and must not wait for
      # the pull request it is reviewing.
      if [ "${WAIT_FOR_PULL_REQUESTS:-yes}" = "yes" ]; then
        local open
        open=$(open_pull_requests) || return 1
        if [ "${open}" -gt 0 ]; then
          echo "Floor is clear but ${open} pull request(s) are open; waiting (attempt ${attempt}/${max_attempts})..."
          sleep "${poll_seconds}"
          continue
        fi
      fi
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
