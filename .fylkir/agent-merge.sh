#!/usr/bin/env bash
# Checks whether a pull request may be merged by an agent and, with --merge,
# merges it. "Agent merges" in AGENTS.md describes the conditions, and
# .fylkir/merge-paths lists the paths this repository keeps for the
# responsible engineer. The path list is read from main as it is
# on origin, never from the pull request or the working tree.
#
#   .fylkir/agent-merge.sh --check <pr>   report, change nothing
#   .fylkir/agent-merge.sh --merge <pr>   merge when every condition holds
#
# It prints each unmet condition, then the verdict, and exits 0 only when the
# pull request is eligible (and, with --merge, merged), 1 when it is not and
# 2 on a usage error. It needs git, gh and jq.
set -euo pipefail

default_branch='main'
max_lines=400
blocking_labels="merge:human decision-needed hold"

usage() {
	echo "usage: $0 --check|--merge <pr>" >&2
	exit 2
}
[ $# -eq 2 ] || usage
case $1 in --check | --merge) mode=$1 ;; *) usage ;; esac
pr=$2
case $pr in '' | *[!0-9]*) usage ;; esac

# Counted towards the line cap unless a path matches one of these globs, in
# which * also matches /: tests, test data and generated files.
not_counted=(
	'test/*' 'tests/*' '*/test/*' '*/tests/*' '__tests__/*' '*/__tests__/*'
	'spec/*' '*/spec/*' 'testdata/*' '*/testdata/*' 'fixtures/*' '*/fixtures/*'
	'*_test.*' '*.test.*' '*_spec.*' '*.spec.*' 'test_*' '*/test_*'
	'*generated*' '*.snap' '*.golden'
)

git fetch --quiet --no-tags origin "$default_branch" || {
	echo "couldn't fetch $default_branch from origin" >&2
	exit 2
}
paths=$(git show "origin/$default_branch:.fylkir/merge-paths") || {
	echo ".fylkir/merge-paths is missing on $default_branch" >&2
	exit 2
}
patterns=()
while read -r line; do
	case $line in
	'' | '#'*) ;;
	*) patterns+=("$line") ;;
	esac
done <<<"$paths"

reasons=()
fail() { reasons+=("$1"); }
has_line() { grep -qxF -- "$1" <<<"$2"; }

# The pull request itself.
IFS=$'\t' read -r state draft base head author mergeable changed < <(gh pr view "$pr" \
	--json state,isDraft,baseRefName,headRefOid,author,mergeable,changedFiles \
	--jq '[.state, .isDraft, .baseRefName, .headRefOid, .author.login, .mergeable, .changedFiles] | @tsv')
[ "$state" = OPEN ] || fail "the pull request is $state, not open"
[ "$draft" = false ] || fail "the pull request is a draft"
[ "$base" = "$default_branch" ] || fail "the pull request targets $base, not $default_branch"
[ "$mergeable" = MERGEABLE ] || fail "GitHub reports it as $mergeable, not MERGEABLE"
main_sha=$(gh api "repos/{owner}/{repo}/branches/$default_branch" --jq .commit.sha)

# A merge freeze: an open issue labelled merge-freeze stops every agent merge.
freeze=$(gh issue list --state open --label merge-freeze --json number,title --jq '.[] | [.number, .title] | @tsv') ||
	fail "the merge freeze is unchecked: the issues labelled merge-freeze could not be listed"
while IFS=$'\t' read -r n title; do
	if [ -n "$n" ]; then fail "merges are frozen by issue #$n ($title)"; fi
done <<<"$freeze"

# Labels: the engineer's merge:agent on every closing issue, nothing that
# holds it back on the issues or the pull request.
pr_labels=$(gh pr view "$pr" --json labels --jq '.labels[].name')
issues=$(gh pr view "$pr" --json closingIssuesReferences --jq '.closingIssuesReferences[].number')
[ -n "$issues" ] || fail "it closes no issue"
for n in $issues; do
	labels=$(gh issue view "$n" --json labels --jq '.labels[].name')
	if ! has_line merge:agent "$labels"; then fail "issue #$n is not labelled merge:agent"; fi
	for l in $blocking_labels; do
		if has_line "$l" "$labels"; then fail "issue #$n is labelled $l"; fi
	done
done
for l in $blocking_labels; do
	if has_line "$l" "$pr_labels"; then fail "the pull request is labelled $l"; fi
done

# Required checks on the head. gh exits non-zero while checks fail or run.
checks=$(gh pr checks "$pr" --required --json name,bucket --jq '.[] | "\(.bucket)\t\(.name)"' || true)
[ -n "$checks" ] || fail "no required check is reported"
while IFS=$'\t' read -r bucket name; do
	case $bucket in pass | skipping | '') ;; *) fail "required check $name is $bucket" ;; esac
done <<<"$checks"

# The final review comment: it reviewed this head, nothing was declined,
# escalated or left open, every acceptance item is met and the security
# pass was skipped.
review=$(gh pr view "$pr" --json comments --jq "[.comments[] | select(.author.login == \"$author\" and (.body | test(\"^\\\\s*## Independent review\")))] | last | .body // \"\"")
if [ -z "$review" ]; then
	fail "it has no Independent review comment by $author"
else
	if ! grep -qF -- "$head" <<<"$review"; then fail "the last review comment did not review the head $head"; fi
	if ! grep -qE 'Outcome: fixed [0-9]+, declined 0, escalated 0, open round-2 findings 0' <<<"$review"; then
		fail "the last review comment has no Outcome with 0 declined, 0 escalated and 0 open round-2 findings"
	fi
	acceptance=$(grep -oE 'Acceptance: met [0-9]+ of [0-9]+\.' <<<"$review" | tail -n 1 || true)
	read -r _ _ met _ of <<<"${acceptance%.}"
	if [ -z "$acceptance" ] || [ "$of" -eq 0 ] || [ "$met" -ne "$of" ]; then
		fail "the last review comment does not record every acceptance item as met (${acceptance:-no Acceptance line})"
	fi
	if ! grep -qF 'Skipped: no changed file matches.' <<<"$review"; then fail "the last review comment does not show the security pass skipped"; fi
fi

# Changed paths and size.
files=$(gh pr view "$pr" --json files --jq '.files[] | "\(.additions + .deletions)\t\(.path)"')
if [ "$(grep -c . <<<"$files")" -ne "$changed" ]; then
	fail "it changes $changed files, more than gh lists; too large to check"
fi
lines=0
while IFS=$'\t' read -r n path; do
	[ -n "$path" ] || continue
	for p in "${patterns[@]}"; do
		# shellcheck disable=SC2053 # the pattern is a glob
		if [[ $path == $p ]]; then
			fail "$path matches $p in .fylkir/merge-paths"
			break
		fi
	done
	counted=1
	for p in "${not_counted[@]}"; do
		# shellcheck disable=SC2053 # the pattern is a glob
		if [[ $path == $p ]]; then
			counted=0
			break
		fi
	done
	if [ "$counted" -eq 1 ]; then lines=$((lines + n)); fi
done <<<"$files"
[ "$lines" -le "$max_lines" ] || fail "it changes $lines lines outside tests and generated files, more than $max_lines"

# The default branch: every CI run of the push that made its current commit
# has finished and passed, so merges go one at a time and each is tested on
# its own.
if ! runs=$(gh run list --branch "$default_branch" --event push --commit "$main_sha" --limit 50 \
	--json workflowName,status,conclusion,url --jq '.[] | [.workflowName, .status, .conclusion, .url] | map(if . == null or . == "" then "-" else . end) | @tsv'); then
	fail "$default_branch's CI is unchecked: its runs could not be listed"
elif [ -z "$runs" ]; then
	fail "$default_branch's current commit $main_sha has no CI run yet"
else
	while IFS=$'\t' read -r workflow run_status run_conclusion run_url; do
		if [ "$run_status" != completed ]; then
			fail "$default_branch's $workflow run has not finished ($run_status) $run_url"
		else
			case $run_conclusion in
			success | skipped | neutral) ;;
			*) fail "$default_branch is red: its $workflow run is $run_conclusion $run_url" ;;
			esac
		fi
	done <<<"$runs"
fi

if [ ${#reasons[@]} -gt 0 ]; then
	printf -- '- %s\n' "${reasons[@]}"
	echo "PR #$pr is not eligible for an agent merge."
	exit 1
fi
echo "PR #$pr at $head is eligible for an agent merge: issue labelled merge:agent, required checks pass, clean review of this head with every acceptance item met and no security pass, no path in .fylkir/merge-paths, $lines lines, $default_branch green, no merge freeze."
[ "$mode" = --merge ] || exit 0

gh pr merge "$pr" --merge --match-head-commit "$head"
gh label create agent-merged --color c2e0c6 --description "Merged by the orchestrator through .fylkir/agent-merge.sh" >/dev/null 2>&1 || true
gh pr edit "$pr" --add-label agent-merged >/dev/null
gh pr comment "$pr" --body "Agent merge of \`$head\` by \`.fylkir/agent-merge.sh\`: the closing issue is labelled \`merge:agent\`, the required checks pass, the last review comment reviewed this head with 0 declined, 0 escalated and 0 open round-2 findings, every acceptance item met and the security pass skipped, no changed path is in \`.fylkir/merge-paths\`, $lines lines outside tests and generated files (cap $max_lines), \`$default_branch\` was green, and no issue labelled \`merge-freeze\` was open." >/dev/null
echo "Merged PR #$pr."
