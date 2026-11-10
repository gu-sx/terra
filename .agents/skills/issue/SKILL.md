---
name: issue
description: Implement one GitHub issue end to end in this Fylkir worktree, open a PR, get an independent review from reviewer agents at the PR's head commit, and present the triaged review to the responsible engineer.
argument-hint: <issue number>
disable-model-invocation: true
---

# Implement an issue

The issue number is the first of the arguments you were invoked with, `<n>` below. One issue is one unit of work in one worktree. Follow these steps in order and stop where a step says to stop.

This skill runs as a Fylkir agent: the reviewers are your children, and Fylkir wakes you when each one finishes. Your first message from Fylkir names your agent and its id; if it didn't, you aren't a Fylkir agent: say so and stop. Run each command on its own rather than chained with `;`, `&&` or a pipe, so each one is checked against the project's permissions by itself. Fylkir gave you your own worktree and branch, `fylkir/<name>-<id>`; keep that branch and never create another worktree.

**Stop and ask** means: say what you need, with the evidence, in your final message; run `fylkir ask "Issue #<n>: <the question>"`, with an `--option "<label>: <what it means>"` for each answer you can see, two to nine of them, and `--multi` if several can be chosen; and end your turn without `fylkir yield`. The engineer picks an option or writes an answer in the TUI, and it arrives as your next message. Continue from where you stopped.

**Wait** means: run `fylkir yield --reason "<what you wait for>"`, then end your turn. Fylkir wakes you when a child finishes a turn, when a message arrives, or on your heartbeat. A background shell job does not survive the end of a turn, so never leave one running to wait.

## 0. Start from the latest base

The **base** is `main`.

Run `git fetch origin`. If the branch is already pushed and its upstream has commits you don't have, run `git pull --ff-only` first. Then:

- If the branch has no commits of its own (`git merge-base --is-ancestor HEAD origin/main` succeeds), fast-forward it: `git merge --ff-only origin/main`.
- If it has commits and was never pushed, rebase it: `git rebase origin/main`. If the rebase conflicts, run `git rebase --abort`, then stop and ask.
- If it has commits and is already pushed, merge instead, because force-pushing is not allowed: `git merge origin/main`. On conflicts, `git merge --abort`, then stop and ask.

Never reset, stash or discard changes to get there.

## 1. Read and check the issue

Run `gh issue view <n> --comments`. Stop and ask if any of these hold:

- It has no acceptance evidence, or no primary area.
- It has the `decision-needed` label: a design question is still open for the user.
- A dependency it lists is still open (`gh issue view <dependency> --json state`), unless the issue says the overlap is safe.
- It implements a spec or design that is not approved, and the change would merge dependent implementation (`AGENTS.md` says where specs and design documents live and who approves them).
- The worktree has uncommitted changes.

Read `AGENTS.md`, then the specs, design documents and decision records the issue names and those `AGENTS.md` names for the area, before writing code.

## 2. Implement

- Start in the issue's primary area and change whatever else the work needs, such as tests, documents, callers and generated files, without asking. Note every changed file outside the primary area and why, for the PR body.
- Before changing a file under "Files that serialise work" in `AGENTS.md`, check whether an open PR changes it too (`gh pr list --state open --json number,files`). If one does, stop and ask.
- Implement what the spec and design documents say. When they are wrong, or silent on behaviour a user would see, do not settle it in code alone: propose the change to the spec or design document in this PR and say so in its body. Only the responsible engineer approves a spec or a design change, so such a PR is never an agent merge.
- Follow every rule in `AGENTS.md`.
- Verify dependency APIs against the pinned version's source or documentation; record exact versions and observed behaviour for spikes.
- Commit in small Conventional Commits with no co-author or generated-with trailers.
- Run the checks under "Checks before a pull request" in `AGENTS.md` until they pass, including those it names for the paths you changed. Never open a PR with failing checks.

## 3. Open the PR

Push the branch (`git push -u origin HEAD`) and run `gh pr create --base main` with a Conventional Commits title. The body follows `.github/pull_request_template.md`: it states what changed and why, maps each acceptance-evidence item to how it is met (test name, command and observed output), lists every changed file outside the primary area under the heading `Outside the primary area`, each with a line on why (or "None"), names the specs and design documents implemented or touched with their status, lists anything deferred under `Deferred`, and ends with `Closes #<n>`. No generated-with line. Pass the body inline with a quoted heredoc, so the shell expands nothing in it:

```sh
gh pr create --base main --title "<title>" --body "$(cat <<'BODY'
<body>
BODY
)"
```

**Screenshots.** If the change alters what a user sees, such as a page, a screen or a terminal UI, take screenshots of the new state with the project's own tools, and of the old state too when the difference is the point. Then run `fylkir attach --pr <pr> <files> --caption "<what they show>"`. It shows them in your Fylkir conversation and publishes them on `refs/fylkir/screenshots`, outside every branch, and prints Markdown that displays them. Post that Markdown as a PR comment headed `## Screenshots` (`gh pr comment <pr>`, with the body in a quoted heredoc as above). Never commit screenshots to the branch. Skip this for changes nobody sees.

## 4. Independent review, alongside CI

Start the review as soon as the PR is pushed. Do not wait for CI first, and do not merge `main` because it moved: each push to `main` reruns the checks on the combined code.

Record the head, `<sha>` below, with `git rev-parse HEAD`, confirm `git status --porcelain` is empty and that the head is pushed (`git rev-parse @{u}` prints the same SHA), and run `git fetch origin main` so the reviewers' `origin/main` is current. Each reviewer gets its own detached checkout of exactly `<sha>` through `fylkir spawn --revision`; none shares your worktree.

The reviewers run on Codex, in a sandbox that may have no network, so they cannot read GitHub. Their prompt carries what they need: the issue's title, body and comments as `gh issue view <n> --comments` printed them, and the PR's title and body. Pass it on stdin with a quoted heredoc (`-` makes `fylkir spawn` read the prompt from stdin), with a delimiter that does not occur in the text, so the shell expands nothing in it. Start the general reviewer:

```sh
fylkir spawn --role reviewer --name review-pr-<pr> --revision <sha> - <<'PROMPT'
Review pull request #<pr> at <sha>, for issue #<n>. Your checkout is detached at <sha>; the change under review is `git diff origin/main...HEAD`. CI is running on it separately. Follow .fylkir/review.md exactly. Do not modify any file.

## Issue #<n>: <title>

<the issue's body and comments>

## Pull request #<pr>: <title>

<the PR's body>
PROMPT
```

and, when the security pass applies (below), the security reviewer at the same time. `fylkir spawn` prints each child's id; note it. The reviewers inherit your Work Item, so their usage is charged to the issue.

For round 2 (step 5), start new reviewers the same way, named `review-pr-<pr>-round-2` and `security-review-pr-<pr>-round-2`, at the new head, with this paragraph added to each prompt before the issue: "This is round 2. The PR comment 'Independent review: round 1', quoted below, lists earlier findings and responses. Check that findings marked fixed are fixed by the commits it names, and report a declined finding again only if its reasoning is wrong." Then add that comment's full text under the heading `## Independent review: round 1`.

Then wait for the reviewers.

Fylkir wakes you when each reviewer finishes its turn, with a message that starts `[Fylkir event] Child agent <name> (<id>) finished a turn with status <status>. Outcome:` followed by the reviewer's final message. That message is its result. When each reviewer finishes, in either round:

- If the status is not `completed`, tell the user what happened and stop and ask. Do not substitute your own review.
- The result is the last ```` ```json ```` fenced block of the outcome: one JSON object matching the schema the brief names (`.fylkir/review-schema.json` or `.fylkir/security-review-schema.json`), with every required key, no other key and only the listed values. If there is no such block, it does not parse or match, or Fylkir clipped the outcome (it ends with `(clipped, … bytes total; see raw events)`), tell the user and stop and ask. Do not substitute your own review, and do not repair the output.
- Check its checkout: `fylkir children --json` gives its `worktree`, and `git worktree list --porcelain` must still show that worktree `detached` at `<sha>`. Then archive it with `fylkir archive <id>`, which removes the checkout and refuses when the reviewer left changes in it. If the head moved or Git refuses, the reviewer changed something: use none of it, leave its checkout for the user, tell the user what changed, and stop and ask.

While a reviewer is still running, wait again.

Fylkir does not watch CI for you. Once every reviewer of the round has finished, read `gh pr checks <pr>`. While checks are still running, set a heartbeat with `fylkir schedule 5m` and wait; on each heartbeat read `gh pr checks <pr>` again, and once they have finished run `fylkir schedule off`.

- If a check fails, read the failure (`gh run view <run-id> --log-failed`). It becomes a round-1 finding with pass `ci` (step 5), fixed with the reviewers' findings, and round 2 verifies the fix.
- If GitHub reports a conflict with the base (`gh pr view <pr> --json mergeable` is `CONFLICTING`), resolve it once the reviewers have finished: `git merge origin/main` and resolve it, or `git merge --abort` and stop and ask when the resolution would change what the other change does; rerun the local checks and push.

Triage (step 5) once every review of the round and CI have finished.

### Security pass

In each round, before starting the reviewers, decide whether the change needs the security pass: run each command in the fenced block under "When this pass runs" in `.fylkir/security-review.md`, as it is written there, and keep the commands and their output for the review comment. If no command prints a file, or the brief does not exist, skip the pass. Otherwise start the security reviewer alongside the general one, with the same prompt except its first paragraph:

```text
Security-review pull request #<pr> at <sha>, for issue #<n>. Your checkout is detached at <sha>; the change under review is `git diff origin/main...HEAD`. CI is running on it separately. Follow .fylkir/security-review.md exactly. Do not modify any file.
```

```sh
fylkir spawn --role security-reviewer --name security-review-pr-<pr> --revision <sha> - <<'PROMPT'
<the prompt>
PROMPT
```

When it finishes, make the same checks as for the general review. A round is complete when both passes, or the general review and a recorded skip, are, and CI has finished.

Run at most two review rounds per PR automatically: round 1, and round 2 only to verify round 1's fixes. Further rounds only when the user asks.

## 5. Triage, fix and verify

### Round 1

Check every finding against the code yourself and give it one verdict:

- **fix**: the default. Use it for every finding whose failure scenario is real or plausible. When you are unsure, write the test that settles it: if it fails, fix; if it passes and shows the scenario cannot happen, decline and cite the test. "Low value" or "style" is not a reason to skip a fix.
- **decline**: only for strong disagreement. Cite the code, test or doc that shows the failure scenario cannot happen, or show that the fix would contradict an approved spec or design, or the issue's scope.
- **escalate**: a real finding you may not fix alone: the fix needs a design change (a spec or design document, where `AGENTS.md` asks for one), or a change to the issue's acceptance evidence, or it is otherwise reserved for the responsible engineer. Also use it when neither the code nor a test can settle the scenario. Say what you would do and what decision it needs.

Post the triage as one PR comment (`gh pr comment <pr>`, with the body in a quoted heredoc as in step 3) headed `## Independent review: round 1`. It gives the reviewed SHA as the line `Reviewed SHA: <full SHA>` and the checkout check (each reviewer's checkout was still detached at that SHA and was removed cleanly), quotes the reviewer's summary and acceptance-criteria assessment followed by the line `Acceptance: met N of M.` (M items, N of them marked `yes`), and has a `### Security pass` section: the trigger commands and their output, then either `Skipped: no changed file matches.` or the security reviewer's summary and its `areas` as a table (area, affected, evidence). It lists the findings of both passes, most severe first, in one table with the columns finding (`file:line` and summary), pass (`review` or `security`), severity, verdict and reason. A check that failed in step 4 is a finding too, with pass `ci`, the check's name and failing test as the finding, and verdict **fix**. For **fix** the reason says how you will fix it. End the comment with one of these lines:

- `Fixing N findings, then round 2.` when any finding is **fix**, or `Fixing 1 finding, then round 2.` when exactly one is.
- `Outcome: fixed 0, declined N, escalated N, open round-2 findings 0 (no round 2). Waiting for the responsible engineer.` when none is. Skip the fixes and round 2: present (below) and **stop**.

Then fix every **fix** finding, in Conventional Commits with one or more commits per finding, each subject naming the defect, for example `fix(cli): reject a non-numeric port in the config file`. Where the finding is testable, add a test that fails without the fix. A finding that only needs the PR description changed, for example missing evidence, is fixed by editing the description (`gh pr edit <pr> --body ...`), with no commit. If a fix turns out to need a design change, stop that fix and mark the finding **escalate** in round 2's comment. Fix a failed CI check from step 4 the same way. Rerun the checks from step 2 and push; do not merge the base unless the PR conflicts with it. If no fix changed a file, skip the push and run round 2 on the same SHA. If the same check fails twice, stop and ask.

### Round 2

Right after pushing the fixes, record the new head, confirm `git status --porcelain` is empty and start, as in step 4, the round-2 reviewer and, when its trigger matches, the round-2 security pass, then wait, with the same checks when each finishes and the same reading of CI. If CI fails on the fixes, report it in the round-2 comment and to the user; do not fix it without the user. Triage its findings with the same verdicts, but **do not fix round-2 findings**; they go to the responsible engineer.

Post one PR comment headed `## Independent review: round 2` with the `Reviewed SHA:` line and the checkout check, the reviewer's summary and acceptance-criteria assessment with its `Acceptance: met N of M.` line, the `### Security pass` section as in round 1, and two tables, each covering both passes with a pass column:

- Round 1: finding, verdict and fixed in (the commit SHAs for each fixed finding, `PR description` for a fix that changed only the description, `-` otherwise), and whether round 2 confirmed the fix or still disputes the decline.
- Round 2: finding, severity, verdict and reason, as in round 1.

End it with `Outcome: fixed N, declined N, escalated N, open round-2 findings N. Waiting for the responsible engineer.`, counting round 1's verdicts and every round-2 finding.

### Present and stop

Present to the user in your final message, most severe first: round 1's outcome (fixed with commit SHAs, declined, escalated), round 2's triage when it ran, the reviewer's latest acceptance-criteria assessment, whether the security pass ran or was skipped, the output of `.fylkir/agent-merge.sh --check <pr>` (the conditions an agent merge still lacks, or that `/orchestrate` will merge it), and links to the review comments. Then run

```sh
fylkir handoff request --target human --kind review --pr <PR URL> --commit <head SHA> --summary "PR #<pr>: <the Outcome line, without its last sentence>"
```

and end your turn without `fylkir yield`. Two rounds are the cap: never start a third review or fix round-2 findings without the user.

## 6. After the user decides

The user's decision arrives as a message. Fix what the user chose, including declined or escalated findings the user overrules, rerun the checks from step 2, push, read CI as in step 4 and reply on the PR with what changed for each finding. Then present again as above, at the new head. Further review rounds happen only when the user asks. Never merge, push to `main`, tag or release, and never run `.fylkir/agent-merge.sh --merge`: `/orchestrate` merges a PR that the script accepts, and the responsible engineer every other.

## Messages from the orchestrator

`/orchestrate` may send you "`main` has moved and conflicts with this branch". If your reviewers are still running, do it once they have finished: `git pull --ff-only`, `git merge origin/main`, resolve it, or `git merge --abort` and stop and ask when the resolution would change what the other change does; rerun the checks, push without force and read CI as in step 4. Do not start another review. Then carry on where you were: if you were waiting, wait again; if you had presented and stopped, say in one line what the update changed and end your turn.
