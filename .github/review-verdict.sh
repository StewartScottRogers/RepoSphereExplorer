#!/usr/bin/env bash
#
# What did the reviewer decide about this exact head?
#
# Prints the verdict and exits 0 only for an approval. Anything else -
# changes requested, a review that states no verdict, or no review on this
# head at all - exits 1, so the required `independent review` check goes
# red and the pull request does not merge.
#
# Counting reviews was not enough. `main` requires no approving review, so
# a request for changes held nothing: the check only asked whether some
# review had landed. And a factory shift opens its pull request as the
# same `claude[bot]` that reviews it, which GitHub will not let approve or
# request changes on its own pull request, so the reviewer can only leave
# a plain comment review - and the verdict lives in its body. On #595 the
# reviewer asked for changes in a comment and the check still went green.
#
# So the verdict is read from the review's state when GitHub recorded one,
# and from the first line of its body (`Verdict: approve` or
# `Verdict: request changes`, as `.github/reviewer-prompt.md` asks) when it
# did not. The latest review on this head decides.
#
# A file rather than shell inside the workflow so that
# `.github/tests/review-verdict.sh` can run it.

set -uo pipefail

: "${REPO:?}" "${PR:?}" "${HEAD_SHA:?}"

reviews=$(gh api "repos/${REPO}/pulls/${PR}/reviews") || {
  echo "Could not read the reviews on #${PR}."
  echo "verdict=none" >> "${GITHUB_OUTPUT:-/dev/null}"
  exit 1
}

verdict=$(printf '%s' "${reviews}" | HEAD_SHA="${HEAD_SHA}" python3 -c '
import json, os, re, sys

reviews = [r for r in json.load(sys.stdin) if r.get("commit_id") == os.environ["HEAD_SHA"]]
if not reviews:
    print("none")
    sys.exit(0)
latest = sorted(reviews, key=lambda r: r.get("submitted_at") or "")[-1]
state = latest.get("state")
if state == "APPROVED":
    print("approve")
elif state == "CHANGES_REQUESTED":
    print("request-changes")
else:
    first = next((line for line in (latest.get("body") or "").splitlines() if line.strip()), "")
    # Tolerates the Markdown a reviewer tends to wrap a verdict in.
    first = re.sub(r"[*_`#>]", "", first).strip().lower()
    if re.match(r"(independent review )?verdict:\s*approve", first):
        print("approve")
    elif re.match(r"(independent review )?verdict:\s*request changes", first):
        print("request-changes")
    else:
        print("unstated")
')

echo "verdict=${verdict}" >> "${GITHUB_OUTPUT:-/dev/null}"
case "${verdict}" in
  approve)
    echo "Approved: the latest review on ${HEAD_SHA} approves."
    exit 0
    ;;
  request-changes)
    echo "Changes requested: the latest review on ${HEAD_SHA} asks for changes, so this does not merge."
    ;;
  unstated)
    echo "The latest review on ${HEAD_SHA} states no verdict: its first line is neither"
    echo "'Verdict: approve' nor 'Verdict: request changes'."
    ;;
  *)
    echo "No review on ${HEAD_SHA}."
    ;;
esac
exit 1
