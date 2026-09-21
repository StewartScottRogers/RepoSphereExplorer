#!/usr/bin/env bash
#
# Tests `.github/wait-for-floor.sh` and the step that relies on it.
#
# On 2026-09-21 the `concurrency: dark-factory-floor` group this replaces
# cancelled nineteen of twenty work orders filed in a burst, and cancelled
# a factory shift that had been waiting its turn before any of them
# arrived (#733). The group only ever holds one *pending* run; a fresh
# trigger cancels whatever was pending, it does not queue behind it. This
# checks the queue that replaces it actually queues: an older unfinished
# run blocks, a newer one never does, and the wait ends the moment the
# older run finishes rather than only at some fixed point.
#
# Same shape as review-verdict.sh: run the real script against a stub
# `gh`, and read the workflows to check the wiring the script cannot see.

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

# Stands in for the `gh` the script calls. claude.yml's answer is fixed
# per run; factory-shift.yml's answer switches from SHIFT_JSON to
# SHIFT_CLEAR_JSON once it has been asked SHIFT_CLEARS_AT times, so a test
# can prove the wait ends the moment the floor actually clears rather than
# only at some fixed attempt.
stub="${work}/stub.sh"
cat > "${stub}" <<STUB
gh() {
  if [ "\$1" = "run" ] && [ "\$2" = "list" ]; then
    local workflow=""
    while [ "\$#" -gt 0 ]; do
      if [ "\$1" = "--workflow" ]; then workflow="\$2"; fi
      shift
    done
    if [ "\$workflow" = "claude.yml" ]; then
      printf '%s' "\${CLAUDE_JSON}"
      return 0
    fi
    local count_file="\${WORK}/calls-\${workflow}"
    local n=\$(( \$(cat "\${count_file}" 2>/dev/null || echo 0) + 1 ))
    echo "\${n}" > "\${count_file}"
    if [ "\${n}" -ge "\${SHIFT_CLEARS_AT}" ]; then
      printf '%s' "\${SHIFT_CLEAR_JSON}"
    else
      printf '%s' "\${SHIFT_JSON}"
    fi
    return 0
  fi
  return 0
}
export -f gh
source "${root}/.github/wait-for-floor.sh"
wait_for_floor
exit \$?
STUB

run() { # run_id, JSON for claude.yml, JSON for factory-shift.yml, clears-at
  rm -f "${work}"/calls-*
  # A huge default clears-at, so a test that does not care about the
  # answer changing over time gets the same SHIFT_JSON on every poll,
  # rather than the "== the wait ends the moment the floor actually
  # clears ==" case's switch-after-N-polls behaviour by accident.
  CLAUDE_JSON="$2" SHIFT_JSON="$3" SHIFT_CLEAR_JSON="[]" SHIFT_CLEARS_AT="${4:-1000000}" \
    WORK="${work}" REPO=o/r RUN_ID="$1" POLL_SECONDS=0 MAX_ATTEMPTS="${MAX_ATTEMPTS:-3}" \
    bash "${stub}" > "${work}/log" 2>&1
  echo "$?"
}

run_in_progress='[{"databaseId": 10, "status": "in_progress"}]'
run_queued='[{"databaseId": 10, "status": "queued"}]'
run_completed='[{"databaseId": 10, "status": "completed"}]'
newer_in_progress='[{"databaseId": 999, "status": "in_progress"}]'

echo "== an empty floor goes immediately =="
check "exits 0" "0" "$(run 20 '[]' '[]')"
check "says it goes now" "1" "$(grep -c "goes now" "${work}/log")"

echo "== an older, unfinished run in either workflow blocks =="
check "an older in-progress run in the other workflow blocks" "1" "$(run 20 '[]' "${run_in_progress}" 99)"
check "an older queued run in the other workflow blocks" "1" "$(run 20 '[]' "${run_queued}" 99)"
check "an older run in claude.yml blocks a factory-shift.yml run too" "1" "$(run 20 "${run_in_progress}" '[]' 1)"

echo "== an older run that already finished does not block =="
check "exits 0" "0" "$(run 20 '[]' "${run_completed}")"

echo "== a newer run never blocks the older one =="
check "exits 0 regardless of a newer run elsewhere" "0" "$(run 5 "${newer_in_progress}" '[]')"

echo "== self is excluded =="
check "our own run listed as in-progress does not block us" "0" "$(run 10 '[]' "${run_in_progress}")"

echo "== the wait ends the moment the floor actually clears =="
check "clears on the 2nd poll, not stuck at the 1st" "0" "$(MAX_ATTEMPTS=5 run 20 '[]' "${run_in_progress}" 2)"

echo "== gives up loudly rather than waiting forever =="
result="$(MAX_ATTEMPTS=2 run 20 '[]' "${run_in_progress}" 99)"
check "exits non-zero" "1" "${result}"
check "says so, naming the run it's stuck behind" "1" \
  "$(grep -c "::error::.*[Ss]tuck" "${work}/log")"

echo "== the workflows queue instead of using concurrency: =="
python3 - "${root}/.github/workflows/claude.yml" "${root}/.github/workflows/factory-shift.yml" <<'PY'
import io, sys, yaml

for path in sys.argv[1:]:
    document = yaml.safe_load(io.open(path, encoding="utf-8"))

    def check(description, ok):
        print(f"  {'ok   ' if ok else 'FAIL '} {path}: {description}")
        if not ok:
            sys.exit(1)

    check("has no `concurrency:` group left to silently drop a run",
          "concurrency" not in document)

    steps = next(iter(document["jobs"].values()))["steps"]
    wait_steps = [s for s in steps if "wait-for-floor.sh" in str(s.get("run", ""))]
    check("has exactly one step that waits for the floor", len(wait_steps) == 1)
    step = wait_steps[0]
    env = step.get("env", {})
    check("passes REPO", "REPO" in env)
    check("passes its own RUN_ID", env.get("RUN_ID") == "${{ github.run_id }}")

    checkout_index = next(i for i, s in enumerate(steps) if s.get("uses", "").startswith("actions/checkout"))
    check("waits right after checkout, before installing anything",
          steps.index(step) == checkout_index + 1)

    permissions = document.get("permissions", {})
    check("can read Actions runs to see the rest of the floor",
          permissions.get("actions") in ("read", "write"))
PY
if [ "$?" != 0 ]; then failed=1; fi

if [ "${failed}" = 0 ]; then
  echo "ALL OK"
else
  echo "SOMETHING FAILED"
fi
exit "${failed}"
