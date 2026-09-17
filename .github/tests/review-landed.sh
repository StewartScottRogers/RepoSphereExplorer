#!/usr/bin/env bash
#
# Tests `.github/review-landed.sh` and the retry it gates.
#
# The reviewer finished twice without leaving a review (#483, #485) and
# both times a person had to re-run the job. The fix is a retry, and a
# retry that never fires - or one that fires when a review *did* land,
# and so reviews everything twice - would both look fine from outside.
#
# Same shape as hold.sh: run the real script against a stub `gh`, and
# read the workflow to check the wiring the script cannot see.

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

# Stands in for the `gh` the script calls. REVIEW_COUNT decides what the
# API says came back for this head.
gh() {
  case "$1 $2" in
    "api repos"*) printf '%s\n' "${REVIEW_COUNT}" ;;
    *) return 0 ;;
  esac
}
export -f gh

landed() { # $1 = how many reviews the API reports on this head
  REVIEW_COUNT="$1" REPO=o/r PR=1 HEAD_SHA=abc \
    GITHUB_OUTPUT="${work}/out" CLAUDE_LOG="${CLAUDE_LOG:-/nonexistent}" \
    bash "${root}/.github/review-landed.sh" > "${work}/log" 2>&1
  grep -oP '(?<=^landed=).*' "${work}/out" | tail -1
}

echo "== whether a review landed =="
: > "${work}/out"
check "a review on this head is a review" "true" "$(landed 1)"
: > "${work}/out"
check "three reviews still count as landed" "true" "$(landed 3)"
: > "${work}/out"
check "none on this head is none" "false" "$(landed 0)"

echo "== what it says when none landed =="
: > "${work}/out"
landed 0 > /dev/null
if grep -q "No review on abc after the first pass" "${work}/log"; then
  echo "  ok    it says so rather than failing silently"
else
  echo "  FAIL  no message about the missing review"
  failed=1
fi
if grep -q "nothing to say about what was denied" "${work}/log"; then
  echo "  ok    and says the transcript was not there"
else
  echo "  FAIL  no word about the missing transcript"
  failed=1
fi

echo "== the denied calls are printed when the transcript is there =="
cat > "${work}/transcript.json" <<'JSON'
[
  {"message": {"content": [
    {"type": "tool_use", "name": "Read", "input": {"file_path": "CLAUDE.md"}},
    {"type": "tool_result", "is_error": true,
     "content": "Claude requested permissions to use Read, but you have not granted it yet."}
  ]}}
]
JSON
: > "${work}/out"
CLAUDE_LOG="${work}/transcript.json" landed 0 > /dev/null
if grep -q "denied:.*not granted" "${work}/log"; then
  echo "  ok    the refusal reaches the log, so the cause is not a guess"
else
  echo "  FAIL  the denied call was not printed"
  echo "        --- script output ---"
  sed 's/^/        /' "${work}/log"
  failed=1
fi

echo "== a transcript in the shape the runtime really writes does not crash it =="
cat > "${work}/mixed.json" <<'JSON'
[
  "a bare string line",
  {"type": "user", "message": {"content": "plain text, not a list of blocks"}},
  {"type": "assistant", "message": "not even a dict"},
  {"type": "result", "permission_denials": [
    {"tool_name": "Bash", "tool_input": {"command": "gh pr review 1 --approve"}}
  ]}
]
JSON
: > "${work}/out"
CLAUDE_LOG="${work}/mixed.json" landed 0 > /dev/null
if grep -q "could not parse" "${work}/log"; then
  echo "  FAIL  a mixed transcript still crashes the parser"
  failed=1
else
  echo "  ok    a mixed transcript is read, not abandoned"
fi
if grep -q "denied: Bash .*gh pr review 1 --approve" "${work}/log"; then
  echo "  ok    and the runtime's own list of refusals is printed"
else
  echo "  FAIL  the permission_denials list was not printed"
  sed 's/^/        /' "${work}/log"
  failed=1
fi

echo "== the workflow wires the retry to it =="
python3 - "${root}/.github/workflows/independent-review.yml" <<'PY'
import sys, yaml, io

document = yaml.safe_load(io.open(sys.argv[1], encoding="utf-8"))
steps = next(iter(document["jobs"].values()))["steps"]
by_id = {step.get("id"): step for step in steps}

def check(description, ok):
    print(f"  {'ok   ' if ok else 'FAIL '} {description}")
    if not ok:
        sys.exit(1)

check("there is a first pass and a retry",
      "reviewer" in by_id and "retry" in by_id)

retry_if = str(by_id["retry"].get("if", ""))
check("the retry runs only when the first pass left no review",
      "first-pass.outputs.landed == 'false'" in retry_if)
check("and never when the job stood aside",
      "stand_aside == ''" in retry_if)

order = [step.get("id") for step in steps]
check("the retry comes after the check that gates it",
      order.index("retry") > order.index("first-pass"))
check("and the final guard comes after the retry",
      order.index("retry") < len(order) - 1)

first = by_id["reviewer"]["with"]
second = by_id["retry"]["with"]
check("both passes are given the same prompt and the same tools",
      first["prompt"] == second["prompt"]
      and first["claude_args"] == second["claude_args"])

tools = first["claude_args"]
check("the reviewer can read the files its prompt tells it to read",
      "Read" in tools and "Grep" in tools)
PY
if [ "$?" != 0 ]; then failed=1; fi

if [ "${failed}" = 0 ]; then
  echo "ALL OK"
else
  echo "SOMETHING FAILED"
fi
exit "${failed}"
