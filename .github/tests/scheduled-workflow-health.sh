#!/usr/bin/env bash
#
# Tests `.github/scheduled-workflow-health.sh`, added for #750: the nightly
# Distribution check failed six nights running and floor-health-check.yml
# never noticed, because neither of its sweeps ever looked at a scheduled
# workflow's own run history.
#
# Same shape as wait-for-floor.sh's test: run the real script against a
# stub `gh`, so the streak counting is exercised for real rather than
# reimplemented in the test.

set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "${here}/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

failed=0

check() { # description, expected, actual
  if [ "$2" = "$3" ]; then
    echo "  ok    $1"
  else
    echo "  FAIL  $1"
    echo "        expected: $2"
    echo "        actual  : $3"
    failed=1
  fi
}

# Stands in for the `gh` the script calls. Serves the JSON named after the
# workflow it was asked about, so one test can watch several workflows at
# once and see each one's own history rather than a shared one.
stub="${work}/stub.sh"
cat > "${stub}" <<STUB
gh() {
  if [ "\$1" = "run" ] && [ "\$2" = "list" ]; then
    local workflow=""
    while [ "\$#" -gt 0 ]; do
      if [ "\$1" = "--workflow" ]; then workflow="\$2"; fi
      shift
    done
    local var="RUNS_\${workflow//[.-]/_}"
    if [ -n "\${!var+x}" ]; then
      printf '%s' "\${!var}"
    else
      printf '%s' "\${RUNS_JSON:-[]}"
    fi
    return "\${GH_EXIT:-0}"
  fi
  return 0
}
export -f gh
source "${root}/.github/scheduled-workflow-health.sh"
check_scheduled_workflows
exit \$?
STUB

run() { # WATCHED_SCHEDULED_WORKFLOWS, then NAME=value pairs as remaining args
  local watched="$1"
  shift
  env "$@" WATCHED_SCHEDULED_WORKFLOWS="${watched}" REPO=o/r \
    bash "${stub}" > "${work}/log" 2>&1
  echo "$?"
}

success='[{"conclusion": "success", "createdAt": "2026-09-22T07:30:00Z"}]'
one_failure='[{"conclusion": "failure", "createdAt": "2026-09-22T07:30:00Z"}, {"conclusion": "success", "createdAt": "2026-09-21T07:30:00Z"}]'
six_failures='[
  {"conclusion": "failure", "createdAt": "2026-09-21T07:30:00Z"},
  {"conclusion": "failure", "createdAt": "2026-09-20T07:30:00Z"},
  {"conclusion": "failure", "createdAt": "2026-09-19T07:30:00Z"},
  {"conclusion": "failure", "createdAt": "2026-09-18T07:30:00Z"},
  {"conclusion": "failure", "createdAt": "2026-09-17T07:30:00Z"},
  {"conclusion": "failure", "createdAt": "2026-09-16T07:30:00Z"},
  {"conclusion": "success", "createdAt": "2026-09-15T07:30:00Z"}
]'

echo "== a workflow whose last scheduled run succeeded is healthy =="
check "exits 0" "0" "$(run distribution.yml RUNS_JSON="${success}")"
check "says so, not an error" "0" "$(grep -c '::error::' "${work}/log")"

echo "== a single failed run fails the check but is not called a streak =="
check "exits non-zero" "1" "$(run distribution.yml RUNS_JSON="${one_failure}")"
check "reports an error" "1" "$(grep -c '::error::' "${work}/log")"
check "does not claim a streak of consecutive runs" "0" "$(grep -c 'consecutive' "${work}/log")"

echo "== a run of six consecutive failures is distinguished from one =="
check "exits non-zero" "1" "$(run distribution.yml RUNS_JSON="${six_failures}")"
check "names the streak" "1" "$(grep -c '6 consecutive scheduled runs' "${work}/log")"
check "louder than a single failure: mentions consecutive" "1" "$(grep -c 'consecutive' "${work}/log")"

echo "== a streak stops counting at the most recent success =="
# One failure since the last success (2026-09-22), not the four further
# back before an earlier success - the streak is about now, not history.
mixed='[
  {"conclusion": "failure", "createdAt": "2026-09-22T07:30:00Z"},
  {"conclusion": "success", "createdAt": "2026-09-21T07:30:00Z"},
  {"conclusion": "failure", "createdAt": "2026-09-20T07:30:00Z"},
  {"conclusion": "failure", "createdAt": "2026-09-19T07:30:00Z"}
]'
check "exits non-zero" "1" "$(run distribution.yml RUNS_JSON="${mixed}")"
check "reports a single failure, not a streak of four" "0" "$(grep -c 'consecutive' "${work}/log")"

echo "== more than one watched workflow is checked independently =="
check "one healthy, one on a streak: still exits non-zero" "1" \
  "$(run 'distribution.yml release.yml' RUNS_distribution_yml="${success}" RUNS_release_yml="${six_failures}")"
check "names the failing one, not the healthy one" "1" \
  "$(grep -c 'release.yml has failed' "${work}/log")"
check "does not also flag the healthy one" "0" \
  "$(grep -c 'distribution.yml has failed' "${work}/log")"

echo "== a workflow with no scheduled runs yet is not a failure =="
check "exits 0" "0" "$(run distribution.yml RUNS_JSON='[]')"
check "says there is nothing yet, not an error" "1" "$(grep -c 'no scheduled runs yet' "${work}/log")"

echo "== a listing that cannot be made is never read as a clean bill of health =="
check "a failing gh call fails the check" "1" "$(run distribution.yml RUNS_JSON="${success}" GH_EXIT=1)"
check "says the listing could not be made" "1" "$(grep -c 'Could not list' "${work}/log")"

if [ "${failed}" = 0 ]; then
  echo "ALL OK"
else
  echo "SOMETHING FAILED"
fi
exit "${failed}"
