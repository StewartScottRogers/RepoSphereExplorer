#!/usr/bin/env bash
#
# Did a review land on this exact head?
#
# Writes `landed=true` or `landed=false` to `GITHUB_OUTPUT`, and when
# nothing landed, prints what the agent was denied - which is the
# question that took two pull requests and a lot of guessing to ask.
#
# A file rather than shell inside the workflow so that
# `.github/tests/review-landed.sh` can run it. The workflows are the
# only code in this repository that nothing else tests, and two pull
# requests have already merged that should not have because of a bug in
# one (#440, #463).

set -uo pipefail

: "${REPO:?}" "${PR:?}" "${HEAD_SHA:?}"

# An approval of an earlier push is not a review of what is being merged
# now, so the head has to match. `gh api --jq` takes a filter rather than
# jq's `--arg`, so the head goes into the filter itself.
filter="[.[] | select(.commit_id == \"${HEAD_SHA}\")] | length"
landed=$(gh api "repos/${REPO}/pulls/${PR}/reviews" --jq "${filter}" || echo 0)

if [ "${landed}" -gt 0 ]; then
  echo "Reviewed: ${landed} review(s) on ${HEAD_SHA}."
  echo "landed=true" >> "${GITHUB_OUTPUT:-/dev/null}"
  exit 0
fi

echo "No review on ${HEAD_SHA} after the first pass."
echo "landed=false" >> "${GITHUB_OUTPUT:-/dev/null}"

# What the agent was actually refused. The action leaves its transcript
# here and reports only a count of denials in the summary, so without
# this the cause is a guess - which is exactly where #486 started.
log="${CLAUDE_LOG:-${RUNNER_TEMP:-/tmp}/claude-execution-output.json}"
if [ -f "${log}" ]; then
  echo "Denied tool calls from ${log}:"
  # Every tool_result the runtime marked as an error, and the input that
  # earned it. Printed rather than uploaded so it is in the log beside
  # the failure, where somebody reading the red tick will see it.
  python3 - "${log}" <<'PY' || echo "  (could not parse the transcript)"
import json, sys

try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        entries = json.load(handle)
except (OSError, ValueError) as error:
    print(f"  (could not read the transcript: {error})")
    sys.exit(0)

if isinstance(entries, dict):
    entries = [entries]

denied = []
for entry in entries:
    # The transcript mixes shapes: a line can be a bare string, and a
    # message's content can be a string rather than a list of blocks.
    # Reading every line as a dict crashed on #597 and #607, so neither
    # said what its reviewer was refused.
    if not isinstance(entry, dict):
        continue
    # The runtime's own list of refusals, when the transcript has one.
    for refusal in entry.get("permission_denials") or []:
        if isinstance(refusal, dict):
            print(f"  denied: {refusal.get('tool_name')} {str(refusal.get('tool_input'))[:300]}")
    message = entry.get("message")
    content = message.get("content") if isinstance(message, dict) else None
    if not isinstance(content, list):
        continue
    for block in content:
        if not isinstance(block, dict):
            continue
        if block.get("type") == "tool_use":
            denied.append((block.get("name"), block.get("input")))
        text = block.get("content")
        if block.get("type") == "tool_result" and block.get("is_error"):
            print(f"  denied: {str(text)[:400]}")

if not denied:
    print("  (no tool calls in the transcript)")
PY
else
  echo "No transcript at ${log}: nothing to say about what was denied."
fi

exit 0
