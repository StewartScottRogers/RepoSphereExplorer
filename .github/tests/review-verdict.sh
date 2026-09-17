#!/usr/bin/env bash
#
# Tests `.github/review-verdict.sh` and the step that relies on it.
#
# On #595 the reviewer asked for changes in a comment review and the
# required check still went green, because it only counted reviews. A
# verdict check that approves too much looks exactly like one that works,
# so every way of saying yes and no is run through it here.
#
# Same shape as hold.sh: run the real script against a stub `gh`, and read
# the workflow to check the wiring the script cannot see.

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

# Stands in for the `gh` the script calls: the reviews API answers with
# whatever REVIEWS_JSON holds.
gh() {
  case "$1 $2" in
    "api repos"*) printf '%s' "${REVIEWS_JSON}" ;;
    *) return 0 ;;
  esac
}
export -f gh

verdict() { # $1 = the reviews JSON; prints "<verdict> <exit code>"
  : > "${work}/out"
  REVIEWS_JSON="$1" REPO=o/r PR=1 HEAD_SHA=abc GITHUB_OUTPUT="${work}/out" \
    bash "${root}/.github/review-verdict.sh" > "${work}/log" 2>&1
  local code=$?
  echo "$(grep -oP '(?<=^verdict=).*' "${work}/out" | tail -1) ${code}"
}

review() { # state, body, commit, submitted
  python3 -c 'import json,sys; print(json.dumps({"state": sys.argv[1], "body": sys.argv[2], "commit_id": sys.argv[3], "submitted_at": sys.argv[4]}))' "$@"
}

echo "== a formal review decides by its state =="
check "an approval passes" "approve 0" \
  "$(verdict "[$(review APPROVED 'looks fine' abc 2026-01-01T00:00:00Z)]")"
check "a request for changes holds" "request-changes 1" \
  "$(verdict "[$(review CHANGES_REQUESTED 'no' abc 2026-01-01T00:00:00Z)]")"

echo "== a comment review decides by its first line =="
check "'Verdict: approve' passes" "approve 0" \
  "$(verdict "[$(review COMMENTED $'Verdict: approve\n\nchecked it all' abc 2026-01-01T00:00:00Z)]")"
check "'Verdict: request changes' holds" "request-changes 1" \
  "$(verdict "[$(review COMMENTED $'Verdict: request changes\n\nno test' abc 2026-01-01T00:00:00Z)]")"
check "the wording #595 was held on, in bold, holds" "request-changes 1" \
  "$(verdict "[$(review COMMENTED '**Independent review verdict: request changes** (posted as a comment)' abc 2026-01-01T00:00:00Z)]")"
check "a comment with no verdict holds" "unstated 1" \
  "$(verdict "[$(review COMMENTED 'I had a look.' abc 2026-01-01T00:00:00Z)]")"
check "'approve' further down does not count" "unstated 1" \
  "$(verdict "[$(review COMMENTED $'Notes first.\nVerdict: approve' abc 2026-01-01T00:00:00Z)]")"

echo "== only this head, and the latest review on it =="
check "an approval of an earlier push holds" "none 1" \
  "$(verdict "[$(review APPROVED 'fine' old 2026-01-01T00:00:00Z)]")"
check "no reviews at all holds" "none 1" "$(verdict '[]')"
check "a later request for changes beats an earlier approval" "request-changes 1" \
  "$(verdict "[$(review APPROVED 'fine' abc 2026-01-01T00:00:00Z), $(review COMMENTED 'Verdict: request changes' abc 2026-01-02T00:00:00Z)]")"
check "a later approval beats an earlier request for changes" "approve 0" \
  "$(verdict "[$(review CHANGES_REQUESTED 'no' abc 2026-01-01T00:00:00Z), $(review APPROVED 'now fine' abc 2026-01-02T00:00:00Z)]")"

echo "== the workflow and the prompt are wired to it =="
python3 - "${root}/.github/workflows/independent-review.yml" "${root}/.github/reviewer-prompt.md" <<'PY'
import io, sys, yaml

document = yaml.safe_load(io.open(sys.argv[1], encoding="utf-8"))
steps = next(iter(document["jobs"].values()))["steps"]
prompt = io.open(sys.argv[2], encoding="utf-8").read()

def check(description, ok):
    print(f"  {'ok   ' if ok else 'FAIL '} {description}")
    if not ok:
        sys.exit(1)

final = steps[-1]
check("the last step decides by the verdict, not by counting reviews",
      "review-verdict.sh" in str(final.get("run", "")))
check("and it still runs whatever came before it",
      str(final.get("if", "")) == "always()")
check("the prompt asks for 'Verdict: approve' on the first line",
      "Verdict: approve" in prompt)
check("and 'Verdict: request changes'",
      "Verdict: request changes" in prompt)
check("and says what to do when GitHub refuses a formal verdict",
      "--comment" in prompt)
PY
if [ "$?" != 0 ]; then failed=1; fi

if [ "${failed}" = 0 ]; then
  echo "ALL OK"
else
  echo "SOMETHING FAILED"
fi
exit "${failed}"
