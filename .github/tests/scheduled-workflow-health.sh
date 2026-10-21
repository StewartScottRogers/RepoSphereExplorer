#!/usr/bin/env bash
#
# Tests `.github/scheduled-workflow-health.sh`.
#
# The nightly Distribution check failed six nights in a row (16-21
# September) before anyone noticed (#750), because floor-health-check.yml's
# sweeps only ever looked at pull requests and issues, never at a scheduled
# workflow's own run history. This runs the real script against a stub `gh`
# to prove: a single failed run is reported, a streak of failures is
# reported louder and distinguished from a single failure, an old streak
# that has since recovered is not still reported, and a listing that could
# not be made is never read as a clean bill of health.

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

# Stands in for the `gh` the script calls. Each watched workflow answers
# from its own env var, set to a JSON array of `{"conclusion": ...}` objects
# (most recent first) or to the literal FAIL, so independent workflows can
# be given independent, and independently-failing, run histories.
stub="${work}/stub.sh"
cat > "${stub}" <<STUB
gh() {
  if [ "\$1" = "run" ] && [ "\$2" = "list" ]; then
    local workflow=""
    while [ "\$#" -gt 0 ]; do
      if [ "\$1" = "--workflow" ]; then workflow="\$2"; fi
      shift
    done
    case "\$workflow" in
      distribution.yml) local answer="\${DISTRIBUTION_JSON:-[]}" ;;
      release.yml) local answer="\${RELEASE_JSON:-[]}" ;;
      *) local answer="[]" ;;
    esac
    [ "\$answer" = FAIL ] && return 1
    printf '%s' "\$answer"
    return 0
  fi
  return 0
}
export -f gh
source "${root}/.github/scheduled-workflow-health.sh"
check_scheduled_workflows
exit \$?
STUB

run() { # watched workflows, JSON for distribution.yml, JSON for release.yml
  WATCHED_SCHEDULED_WORKFLOWS="$1" DISTRIBUTION_JSON="${2:-[]}" RELEASE_JSON="${3:-[]}" \
    REPO=o/r GH_TOKEN="${GH_TOKEN_FOR_RUN-stub-token}" \
    bash "${stub}" > "${work}/log" 2>&1
  echo "$?"
}

failure='{"conclusion": "failure"}'
success='{"conclusion": "success"}'

echo "== a healthy last run reports nothing wrong =="
check "exits 0" "0" "$(run distribution.yml "[${success}]")"
check "says it succeeded" "1" "$(grep -c "succeeded" "${work}/log")"
check "raises no error" "0" "$(grep -c "::error::" "${work}/log")"

echo "== the exact fault #750 went unnoticed for six nights =="
check "a single failed run exits non-zero" "1" "$(run distribution.yml "[${failure}, ${success}]")"
check "names the failing workflow" "1" \
  "$(grep -c "::error::distribution.yml: the last scheduled run failed" "${work}/log")"

echo "== a run of failures is reported louder, distinguished from one =="
result="$(run distribution.yml "[${failure}, ${failure}, ${failure}, ${success}]")"
check "still exits non-zero" "1" "${result}"
check "names the streak, not just 'the last run'" "1" \
  "$(grep -c "::error::distribution.yml: the last 3 scheduled runs in a row have failed" "${work}/log")"
check "does not also print the single-failure message" "0" \
  "$(grep -c "the last scheduled run failed" "${work}/log")"

echo "== an old streak that has since recovered is not still reported =="
check "exits 0 once a success is the most recent run" "0" \
  "$(run distribution.yml "[${success}, ${failure}, ${failure}, ${failure}]")"
check "raises no error" "0" "$(grep -c "::error::" "${work}/log")"

echo "== workflows are checked independently =="
result="$(run "distribution.yml release.yml" "[${failure}]" "[${success}]")"
check "one failing workflow still fails the whole check" "1" "${result}"
check "reports the failing one" "1" "$(grep -c "::error::distribution.yml" "${work}/log")"
check "reports the healthy one as healthy, not failing" "1" \
  "$(grep -c "release.yml: last scheduled run succeeded" "${work}/log")"
check "does not raise an error for the healthy one" "0" "$(grep -c "::error::release.yml" "${work}/log")"

echo "== a listing that could not be made is never read as a clean bill of health =="
check "exits non-zero rather than treating a failed listing as healthy" "1" \
  "$(run distribution.yml FAIL)"
check "says the listing could not be made" "1" \
  "$(grep -c "::error::Could not list runs of distribution.yml" "${work}/log")"
check "never claims it succeeded" "0" "$(grep -c "succeeded" "${work}/log")"

echo "== no token at all fails, rather than reporting a clean bill of health =="
check "exits non-zero" "1" "$(GH_TOKEN_FOR_RUN="" run distribution.yml "[${success}]")"
check "says which environment variable is missing" "1" \
  "$(grep -c "::error::Neither GH_TOKEN nor GITHUB_TOKEN" "${work}/log")"
check "never claims it succeeded" "0" "$(grep -c "succeeded" "${work}/log")"

if [ "${failed}" = 0 ]; then
  echo "ALL OK"
else
  echo "SOMETHING FAILED"
fi
exit "${failed}"
