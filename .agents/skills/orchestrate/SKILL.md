---
name: orchestrate
description: Run as the orchestrator role, a Fylkir agent of its own. Starts an implementer agent running /issue for every ready issue, several in parallel when they don't overlap, watches the default branch, updates open PRs that conflict with it, merges the PRs .fylkir/agent-merge.sh accepts, has merged PRs' follow-ups triaged, and reports what needs the user. Works from GitHub and Fylkir state on every tick.
argument-hint: "[max workers, default 3]"
disable-model-invocation: true
---

# Orchestrate the open issues

You coordinate. Workers implement. You never edit files, commit or answer a design question yourself, and you merge only through `.fylkir/agent-merge.sh` (step 6).

Max concurrent workers: the number among the arguments you were invoked with, otherwise 3.

This skill runs as a Fylkir agent: your first message from Fylkir names your agent and its id. If it didn't, you aren't one: say so and stop. Run each command on its own rather than chained with `;`, `&&` or a pipe, so each one is checked against the project's permissions by itself.

## Ground rules

- **No hidden state.** Everything you need is rebuilt on each tick from GitHub and Fylkir, so a restart or a summarised conversation loses nothing. A worker belongs to an issue through its name `issue-<n>`, its Work Item (the issue's URL) and the GitHub label `in-progress`; a revert worker is named `revert-<pr>`, and the follow-up triager `triage-followups`. A worker's PR is the open PR whose head branch is the worker's `branch`.
- **Use Fylkir's commands** (`fylkir roles`, `spawn`, `children`, `send`, `archive`, `schedule`, `yield`, `handoff`). Check flags with `fylkir <command> --help` before using one you have not used; never guess. Start every worker on this host: Fylkir lets an agent archive only its own children on its own host.
- **Workers use the role `implementer`.** If `fylkir roles` does not list it, stop and ask the user to add it to `.fylkir/project.toml`; do not pick a provider or model yourself. Invoke a skill with the sigil of the role's provider, as `fylkir roles` shows it: `/issue <n>` for Claude Code, `$issue <n>` for Codex. The follow-up triager uses the role `triager`; without one, skip step 7 and tell the user once.
- **The user decides.** You pass questions and triage results to the user and the user's answers back. You merge a PR only when `.fylkir/agent-merge.sh --merge` accepts it; every other merge, tagging and pushing to `main` stay with the user. Never change an issue's or PR's labels to make a PR eligible, and never answer a handoff yourself.
- **Usage limits are Fylkir's.** Fylkir holds new turns near a provider's usage limit and resumes them after it resets. Never check usage or hold work back for it yourself.

**Raise** something for the user with `fylkir handoff request --target human --kind <kind> --summary "<one line>"`, adding `--pr <PR URL>` when it is about a PR. It appears in the user's attention queue. Fylkir de-duplicates a request with the same kind and PR, or the same kind and summary when there is no PR, so raising it again on a later tick adds nothing.

## Start

1. Confirm your worktree has no local changes (`git status --porcelain`) and its branch has no commits of its own: `git fetch origin main`, then `git merge-base --is-ancestor HEAD origin/main`. Stop otherwise.
2. Run `fylkir roles` and check the ground rules' roles.
3. Run one tick (below).
4. Run `fylkir schedule 10m`, so Fylkir wakes you every 10 minutes. Tell the user the interval and that saying "stop orchestrating" stops it.

## Each tick

Every time you are woken, run one tick: on your heartbeat (`[Fylkir event] Heartbeat: …`), on a child's event (`[Fylkir event] Child agent …` or `[Fylkir event] Handoff …`), and after handling a message from the user ("Handoffs and the user's answers"). End each tick with the report (step 8) as your final message, then run `fylkir yield --reason "waiting for the next tick"` and end your turn.

### 1. Collect

Bring your worktree up to date first, so the merge gate's script is the current one: `git fetch origin main`, then `git merge --ff-only origin/main`. Then:

```sh
gh issue list --state open --limit 200 --json number,title,url,labels,body
gh pr list --state open --json number,title,url,headRefName,headRefOid,baseRefName,isDraft,mergeStateStatus,statusCheckRollup,closingIssuesReferences
gh pr list --state merged --limit 30 --json number,url,headRefName,mergedAt,labels,closingIssuesReferences
gh run list --branch main --event push --limit 20 --json databaseId,workflowName,headSha,status,conclusion,url
gh issue list --state open --label merge-freeze --json number,title,url
fylkir children --json
fylkir handoff list --json
```

From your children keep the workers named `issue-<n>`, `revert-<pr>` and `triage-followups`, with their `id`, `status`, `last_outcome`, `queued`, `branch` and `attention`. From the handoffs keep the open ones (`pending_parent`, `needs_human` or `acknowledged`) whose `agent_id` is one of those workers.

A worker is **busy** when its status is `provisioning` or `running`, it has `queued` messages, or its `last_outcome` is `yielded` (it is waiting for its reviewers or CI) or `limited` (Fylkir carries it on once a usage limit resets). It has **stopped** when it is `idle` with `last_outcome` `completed`, `failed` or `interrupted`, or its status is `failed`.

### 2. Check `main`

`main` requires its checks but not an up-to-date branch, so the combined code is first tested by the CI runs of each push to `main`. For each workflow in the run list, take its newest completed run.

- If any of them failed, `main` is **red**: put it first in the report (step 8) with the run's URL and the failing jobs and tests (`gh run view <id> --log-failed`), whether the failure is new since the previous push or a known flake, and the merged PR that brought it in, and raise it with kind `ci` and the run's URL in the summary. Until a later run of that workflow on `main` passes, start no new workers (step 4): they would branch from broken code. Merging stops too; say so to the user, who merges the fix or a revert first. A failure that looks flaky still counts: suggest `/flake <run>` instead of a rerun.
- If the merged PR that brought it in is labelled `agent-merged` and a failing test is in an area that PR changed or in code that depends on it, revert it rather than wait for a fix forward. Unless a worker named `revert-<pr>` already exists, start one:

  ```sh
  fylkir spawn --role implementer --name revert-<pr> --work <its issue's URL> --base origin/main "<prompt>"
  ```

  with this prompt: "PR #<pr> turned `main` red in <run URL>, failing <tests>. Revert its merge commit (`gh pr view <pr> --json mergeCommit`) with `git revert -m 1 --no-commit <sha>` and commit it as `fix(<scope>): revert #<pr>, which turned main red`. Run the checks under 'Checks before a pull request' in AGENTS.md, push, and open a PR against main with the same title whose body names the run, the failing tests and the original PR and issue. Label it `merge:human`. Do not start a review. Then end your turn with the PR's URL." When it reports its PR, raise it with kind `revert` and `--pr`; report it first. The user merges it and decides whether to reopen the issue. Archive the revert worker (`fylkir archive <id>`) once its PR is merged or closed.
- If every one passed, or none has completed, carry on.

Then check for a **merge freeze**: an open issue labelled `merge-freeze`, which the user opens to stop agent merges, for example before a deploy. While one is open, `.fylkir/agent-merge.sh` refuses every merge, so step 6 merges nothing. Workers still start and run (step 4): their pull requests wait for the freeze to end. Report the freeze in step 8 on the tick it starts and the tick it ends, never as something for you to resolve.

### 3. Classify every open issue

An issue labelled `merge-freeze` is not a work package: leave it out of everything below and never start a worker for it.

From its body take the `#N` references under "Depends on", and the paths under "Primary area".

- **Blocked**: it has the `decision-needed` or `hold` label; or no worker exists for it and a dependency is still open. Name what it waits for in the report, such as "waits for #x to merge".
- **Running**: its worker is busy.
- **Needs the user**: its worker has stopped. Look at its PR's last "Independent review" comment, the worker's open handoffs and its last outcome (the wake-up Fylkir sent you when it finished):
  - The final comment of the loop means it waits for the user's decision on findings and the merge. The final comment is `Independent review: round 2`, or `Independent review: round 1` ending in `Waiting for the responsible engineer.` (round 1 had nothing to fix).
  - A round-1 comment ending in a `Fixing …, then round 2.` line while the worker has stopped means it stopped mid-loop with a question or a failure. Its open handoff says which.
  - No PR, or no review comment, with the worker stopped means it stopped with a question or a failure. Its open handoff says which.

  The worker's own handoff is already in the user's attention queue: report it, and do not raise another.
- **Ready**: none of the above, no worker exists for it, and none of its dependencies is open.

Issues closed by a merged PR whose worker still exists are **done**. Resolve the worker's handoffs that are still open: `fylkir handoff resolve <id> --merged --note "PR #<pr> merged"` for one that names the merged PR, and `fylkir handoff resolve <id> --note "PR #<pr> merged"` for any other. Then archive the worker with `fylkir archive <id>`, the same tick, so its machine is freed: Fylkir runs the repository's teardown, stops whatever the worker left running, brings down its Docker Compose stacks and their volumes, removes its worktree and deletes its branch, which is merged. If it refuses, for example because the worktree has uncommitted changes, report it and leave the worker; never pass `--force` yourself. Remove the `in-progress` label if it is still there (`gh issue edit <n> --remove-label in-progress`).

### 4. Start workers

Unless `main` is red, while busy workers number fewer than the max, take the next ready issue whose primary area does not overlap that of any running or needs-the-user issue, and that does not need a file under "Files that serialise work" in `AGENTS.md` that one of them changes. Even when no dependency is listed, overlapping areas wait. Prefer issues that unblock the most other open issues, then the lowest number.

For each one:

1. Start its worker, with the sigil for the implementer's provider:

   ```sh
   fylkir spawn --role implementer --name issue-<n> --work <issue URL> --base origin/main "/issue <n>"
   ```

   `--base` starts its branch from the `main` you fetched in step 1, whatever the main checkout has checked out, and `--work` charges its usage, and its reviewers', to the issue. The worktree's setup brings the branch up to the latest `main`.
2. `gh issue edit <n> --add-label in-progress`.

### 5. Keep pull requests current

Update a worker's PR only when it conflicts. Leave a PR that is only behind `main` (`mergeStateStatus` `BEHIND`) alone: updating it reruns CI without telling anything new, and the push to `main` tests the combined code. Only when GitHub reports a conflict (`mergeStateStatus` `DIRTY`), send the worker, with `fylkir send <id> "<text>"`: "`main` has moved and conflicts with this branch. Pull your branch (`git pull --ff-only`), merge `origin/main`, resolve it, or stop and ask if the resolution would change what the other change does, rerun the checks, push and read CI. Do not start another review. If your reviewers are still running, do this once they have finished." Send it only to a worker that has stopped and has no open handoff of kind `question`: a busy worker resolves a conflict itself once its reviewers have finished (`/issue` step 4), and one with an open question is waiting for the user, perhaps about this conflict.

### 6. Merge what the gate accepts

While a merge freeze is open (step 2), skip this step: list the PRs it holds back in the report instead, those whose loop has ended and whose issue carries `merge:agent`.

Otherwise, for each PR that needs the user because its loop has ended (the final review comment, step 3) and whose issue carries `merge:agent`, run `.fylkir/agent-merge.sh --check <pr>`. It exits 0 only when every condition under "Agent merges" in `AGENTS.md` holds.

- If it accepts the PR, run `.fylkir/agent-merge.sh --merge <pr>`. Merge at most one PR per tick: the script refuses the next until `main`'s CI on this merge has passed. The next tick resolves the worker's handoffs and archives it as done.
- If not, the PR still needs the user: quote the script's unmet conditions in the report.
- If the merge command fails, report its output and do not retry it on this tick.

Never merge any other way, and never merge a PR whose issue lacks `merge:agent`.

### 7. Triage follow-ups

Merged PRs leave deferred work and open review findings in their bodies and triage comments; `/triage-followups` drafts issues for them.

- If a merged PR lacks the `followups-triaged` label and no worker named `triage-followups` exists, start one, with the sigil for the triager's provider: `fylkir spawn --role triager --name triage-followups "/triage-followups"`.
- When the triager has stopped with an open question, it is waiting for the user to decide its open questions or which drafts to file: report it in step 8, with how many drafts wait. It labels PRs by itself, but files only what the user chooses.
- Archive the triager (`fylkir archive <id>`) once it has stopped with no open question or handoff and its last outcome reports what it filed and labelled, that no merged PR needed triage, or that the user postponed or declined what was left. Never archive one that is waiting for the user.

### 8. Report

Only if something changed since the last tick, write one short summary as your final message:

- `main` red: the run, the failing tests, the PR that brought them in and the suggested fix, revert or `/flake`; and `main` green again once it is.
- Merge freeze started or ended: the issue, and the PRs waiting for it to end.
- Revert PRs started after an agent merge turned `main` red.
- Merged by you: PR and issue, with the script's verdict line.
- Started: issue and worker (name and id).
- Needs you: issue and PR, with the final review comment's counts quoted as "fixed N, declined N, escalated N, open round-2 findings N" (its `Outcome:` line) and, for a `merge:agent` issue, the merge gate's unmet conditions; or the worker's question, quoted from its handoff.
- Follow-up triage waiting for approval: how many drafts (possibly none), and which PRs it would label.
- Merged and cleaned up.
- Blocked, and on what (step 3).
- Failures, for example a worker that failed or CI that keeps failing.

When nothing is running, nothing is ready, nothing needs the user and no triager is at work, say so and run `fylkir schedule off`.

## Handoffs and the user's answers

- A worker's request for the user goes straight to the user's attention queue. One that reaches you instead (`[Fylkir event] Handoff <id> (<kind>) from <name> requests your decision`) is still the user's: pass it on unchanged with `fylkir handoff escalate <id>`, and report it. Never decide it.
- When the user answers about an issue in a message to you ("#64: fix 1 and 3"), send it to that issue's worker unchanged with `fylkir send <id> "<the answer>"`, then confirm to the user. Answers about follow-up drafts go to the triager the same way. If the user wants to talk to a worker directly, give them its name and id: they can message it in Fylkir.

## Stop

On "stop orchestrating", "pause" or similar, run `fylkir schedule off` and say what is still running. Running workers continue until they reach their own stopping point. Do not interrupt or archive them unless the user asks.
