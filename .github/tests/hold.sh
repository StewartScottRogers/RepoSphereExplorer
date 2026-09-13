#!/usr/bin/env bash
#
# Tests the `hold` step of workflow-review.yml.
#
# There is no other way to check this. The step is shell embedded in a
# workflow, it only ever runs on a real pull request, and by the time it
# is wrong a pull request has already merged that should not have. Twice
# now: #440 merged through a hold that failed silently, and #463 merged
# with changes requested and shipped a release path that could never
# fire.
#
# The stub is *stateful* on purpose. #463's test asserted a case called
# "approved, held, auto-merge label", which the real code could never
# reach because holding stripped that label - so the assertion passed
# against a state that does not exist. Here each scenario runs the step
# for real and the next assertion sees whatever the step actually did.

set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "${here}/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

python3 - "${root}/.github/workflows/workflow-review.yml" "${work}/hold.sh" <<'PY'
import sys, yaml, io
document = yaml.safe_load(io.open(sys.argv[1], encoding="utf-8"))
io.open(sys.argv[2], "w", newline="\n").write(document["jobs"]["hold"]["steps"][0]["run"])
PY

LABELS="${work}/labels"
AUTOMERGE="${work}/automerge"
export LABELS AUTOMERGE

# Stands in for the `gh` the step calls. Answers from, and writes to, the
# two files above, so label state carries from one run to the next.
gh() {
  local args="$*"
  case "$1 $2" in
    "api repos"*) printf '%s\n' "${REVIEW_STATE}" ;;
    "pr view")
      local name
      name=$(printf '%s' "${args}" | sed -n 's/.*index("\([^"]*\)").*/\1/p')
      grep -qx "${name}" "${LABELS}" 2>/dev/null && echo 0
      return 0
      ;;
    "pr edit")
      local name=${args##*-label }
      if printf '%s' "${args}" | grep -q -- "--add-label"; then
        grep -qx "${name}" "${LABELS}" 2>/dev/null || echo "${name}" >> "${LABELS}"
      else
        grep -vx "${name}" "${LABELS}" > "${LABELS}.tmp" 2>/dev/null || true
        mv "${LABELS}.tmp" "${LABELS}"
      fi
      return 0
      ;;
    "pr merge")
      if printf '%s' "${args}" | grep -q -- "--disable-auto"; then
        echo off > "${AUTOMERGE}"
      else
        echo on > "${AUTOMERGE}"
      fi
      return "${MERGE_RESULT:-0}"
      ;;
    *) return 0 ;;
  esac
}
export -f gh

failed=0

hold() { # $1 = the latest review state on this head
  REVIEW_STATE="$1" GH_TOKEN=x REPO=o/r PR=1 HEAD_SHA=abc REVIEW_OUTCOME=success \
    MERGE_RESULT="${MERGE_RESULT:-0}" bash "${work}/hold.sh" >/dev/null 2>&1
  echo "$?"
}

given() { # $1 = labels, space separated; $2 = auto-merge on|off
  : > "${LABELS}"
  for label in $1; do echo "${label}" >> "${LABELS}"; done
  echo "$2" > "${AUTOMERGE}"
}

state() {
  printf 'auto-merge=%s labels=[%s]' \
    "$(cat "${AUTOMERGE}" 2>/dev/null)" \
    "$(tr '\n' ' ' < "${LABELS}" 2>/dev/null | sed 's/ $//')"
}

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

echo "== the sequence work order #462 is about =="
echo "   labelled, held, pushed to, then approved"
given "auto-merge" on
check "held: exits non-zero" "1" "$(hold CHANGES_REQUESTED)"
check "held: auto-merge off, and the label kept" \
  "auto-merge=off labels=[auto-merge needs-human-review]" "$(state)"
check "released: exits zero" "0" "$(hold APPROVED)"
check "released: auto-merge back on, hold lifted" \
  "auto-merge=on labels=[auto-merge]" "$(state)"

echo "== approved first time, never held =="
given "auto-merge" on
check "exits zero" "0" "$(hold APPROVED)"
check "left as it was" "auto-merge=on labels=[auto-merge]" "$(state)"

echo "== a person's workflow change, never labelled =="
given "" off
check "held: exits non-zero" "1" "$(hold CHANGES_REQUESTED)"
check "held: marked for a person" "auto-merge=off labels=[needs-human-review]" "$(state)"
check "released: exits zero" "0" "$(hold APPROVED)"
check "released: not granted auto-merge it never had" \
  "auto-merge=off labels=[]" "$(state)"

echo "== no review landed on this head =="
given "auto-merge" on
check "held: exits non-zero" "1" "$(hold "")"
check "held: auto-merge off" \
  "auto-merge=off labels=[auto-merge needs-human-review]" "$(state)"

echo "== the release cannot re-enable =="
given "auto-merge needs-human-review" off
MERGE_RESULT=1
check "warns rather than failing" "0" "$(hold APPROVED)"
MERGE_RESULT=0

if [ "${failed}" = 0 ]; then
  echo "ALL OK"
else
  echo "SOMETHING FAILED"
fi
exit "${failed}"
