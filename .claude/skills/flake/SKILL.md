---
name: flake
description: Reproduce a flaky test with a recorded seed or order, and the race detector where the language has one, find its minimal cause, and draft a bug work package with the evidence. Changes no file and files nothing without the user's approval.
argument-hint: "<test name, or CI run URL or ID>"
disable-model-invocation: true
---

# Hunt a flaky test

The test or CI run to investigate is the arguments you were invoked with.

A flaky test is a bug with a cause. This skill reproduces it, records the seed or order that triggers it, finds the smallest explanation, and drafts a `bug` work package. It never retries, reruns until green, skips the test, or loosens a timeout to hide the failure, and it never edits, commits or stashes in the worktree. Experiments that need code changes happen in a scratch copy outside it.

Record `git rev-parse HEAD` and `git status --porcelain` now; both must be unchanged at the end.

## 1. Read the rules

Read `AGENTS.md`: its testing rules, how `make check` runs the tests, whether some tests need a development environment, and how a bug is filed. Read `.claude/skills/work-package/SKILL.md` (`.agents/skills/work-package/SKILL.md` for Codex); the draft follows its rules.

## 2. Identify the failure

- **A CI run** (URL or ID): `gh run view <id> --json jobs,headSha,event,createdAt` and `gh run view <id> --log-failed`. Note the commit, the job, the toolchain and runner versions, the failing test and its module, the full failure output, and any seed or test order the run printed.
- **A test name**: find it (`git grep -n '<test name>'`). Check recent CI runs for the same test failing (`gh run list --limit 50 --json databaseId,conclusion,headSha,workflowName`, then `--log-failed` on failures).

Read the test and the code it exercises before running anything. Find out how the project's test runner randomises order and seeds randomness, how to fix the seed, how to repeat a test, and whether the language has a race or thread sanitizer; check this against the runner's documentation or `--help`, not from memory.

## 3. Reproduce

Run the test as CI runs it, with randomised order and the race detector or sanitizer where they exist, in separate invocations so each gets its own seed. Keep every output under a scratch directory outside the worktree:

```sh
scratch=$(mktemp -d)
for i in $(seq 1 20); do
  <test command for the one test, with random order and a printed seed> >"$scratch/run-$i.txt" 2>&1 || { echo "failed: run $i"; break; }
done
```

- Record the seed of each failing run, the command and how many of how many runs failed.
- Start with the single test; if it never fails alone, run its whole file or module: an order-dependent failure needs the tests before it. Raise the repeat count and the loop count before concluding it does not reproduce, and vary parallelism (one worker, many workers) when the failure looks like a timing one.
- Tests that need the development environment `AGENTS.md` describes run with that environment up.
- Confirm with the seed: rerun with the failing seed fixed. Say whether the seed alone reproduces it deterministically or only raises the rate.

If it does not reproduce after a reasonable budget (say a few thousand test executions), stop there: report what you ran and the counts, and make reproducing it the draft's first acceptance item, marked unconfirmed.

## 4. Find the minimal cause

Explain the failure by the smallest mechanism the evidence supports, and show the evidence. Common causes:

- **Data race**: a race detector's or sanitizer's report names both accesses; cite them with `file:line`. A read-after-swap race (a reader keeps a reference while a writer swaps or reuses it) shows as a race between the swap and the later read.
- **Order dependence**: shared global state, a registry, environment variables, files or database rows left by an earlier test. Narrow it with the failing seed: run the failing test after each earlier test of that order, in pairs, until one pair fails.
- **Timing**: sleeps, short timeouts, polling without a deadline, or assumptions about scheduling. Show the window, for example with timestamps from a scratch copy.
- **Nondeterminism**: hash or map iteration order, time zone or clock, random values without a seed, unordered query results.
- **External state**: shared ports, files, a database or schema used by parallel tests or processes.

Change code only in a scratch copy outside the worktree (`git worktree` is shared state; use `git archive HEAD | tar -x -C "$scratch/src"`). A scratch change that makes the failure disappear under the same seed and count, and returns when reverted, is the strongest evidence; record its diff in the draft as a suggested fix, not as applied.

## 5. Draft the bug

Draft one work package following steps 1 to 8 of the work-package skill, with the `bug` label. Besides the template's headings it states:

- The failing test and its module, the CI run or the local command that first showed it, and the commit.
- The seed, the reproducing command, and the rate (`failed k of n runs`), with the relevant failure and race-detector output quoted.
- The cause with `file:line` references, and the scratch experiment that confirms it.
- Primary area: the module of the code and test the fix touches.
- Acceptance evidence that does not depend on luck: the reproducing command with the seed passes at a count where it failed before, and a test that fails without the fix where one can be written. Retries, skips, longer timeouts and reruns are not fixes.

## 6. Present and stop

Check `git rev-parse HEAD` and `git status --porcelain` against the values recorded at the start and say that they match; if they don't, report what changed. Then show the user, in your final message: the seed, the cause, the evidence, and the draft in full (title, labels, body), followed by `Dry run: no file changed, no issue filed.` Delete the scratch directory or tell the user where it is.

**Stop and wait.** If you run as a Fylkir agent (your first message from Fylkir named your agent), run `fylkir ask "Flake in <test>: file the bug draft above?" --option "File it: as drafted" --option "Revise: I'll say what to change"` and end your turn without `fylkir yield`; the answer arrives as your next message. File the draft only after the user approves it, as step 10 of the work-package skill says, and report its number and URL.
