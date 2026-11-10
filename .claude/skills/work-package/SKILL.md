---
name: work-package
description: Turn a goal, bug or plan item into one or more work-package issues that another agent can start with /issue, after researching the code and getting the user's approval.
argument-hint: <goal, bug or plan item>
disable-model-invocation: true
---

# Draft work packages

The request is the arguments you were invoked with.

A work package is an issue another agent can implement with `/issue <n>` without asking what was meant. Research first, draft, show the drafts to the user, and file nothing until the user approves. Never start implementing.

## 1. Understand the request

- Read `AGENTS.md` first. For a plan item, read its entry in the plan `AGENTS.md` names and the design document that governs it.
- If the project keeps specs or design documents (`AGENTS.md` says where), read the whole one that covers the request. Each work package implements part of it and names the parts. A spec that is not approved cannot be implemented yet: say so and stop.
- If the request adds behaviour that `AGENTS.md` says belongs in a spec or design document and none covers it, draft that first and show it with the work packages; filing waits until the user approves both.
- For a bug, find the code path. Reproduce it if that is cheap and safe; otherwise make reproducing it the first acceptance item and say it is unconfirmed.
- Read the design documents and decision records the change relies on. Note each one's status.
- If the change belongs in another repository, follow what `AGENTS.md` says about it and draft the issue for that repository.
- Check dependency APIs against the versions the project pins, not from memory.

## 2. Research the code

Read the code the change will touch until you can name its primary area: the packages or directories it is centred in. It need not list every file; `/issue` changes tests, documents, callers and generated files outside it without asking. Note the constraints in `AGENTS.md` that shape the design: generated files and the commands that regenerate them, files that must stay identical, where a new module is registered, and compatibility rules.

## 3. Check work in flight

```sh
gh issue list --state open --limit 100 --json number,title,labels,body
gh pr list --state open --json number,title,headRefName,files
```

Compare the primary area of every open issue and the changed files of every open PR with this change's primary area. For each overlap, either list the other issue under "Depends on", or state why running in parallel is safe. The files under "Files that serialise work" in `AGENTS.md` are the overlaps that must be chained: only one open issue should change each at a time.

## 4. Shape the work

- One issue is one pull request that one agent can finish and verify in a session. Split anything larger.
- Split so parallel issues have distinct primary areas. If two pieces centre on the same package, or both change a serialising file, chain them with "Depends on" instead.
- Order the issues: which can start now, which wait for which. `/orchestrate` starts an issue only once every issue it depends on has merged, so a long chain runs one issue at a time; prefer independent pieces where the design allows.

## 5. Settle the design or mark it open

- If the design is clear, state it concretely in "Scope": names, file locations, behaviour, error cases. End it with: "Proposed design. If you disagree, stop and say so before implementing."
- If a choice belongs to the responsible engineer, say so: list the options with your recommendation and add the `decision-needed` label. `/issue` refuses issues with that label until it is removed.
- A change that `AGENTS.md` says needs a spec or design-document change gets it first: make that its own issue, or the first acceptance item, and say whether its status blocks the merge.

## 6. Suggest who merges

`/orchestrate` merges a pull request whose issue carries `merge:agent` once `.fylkir/agent-merge.sh` accepts it; every other pull request waits for the responsible engineer ("Agent merges" in `AGENTS.md`). Suggest one label for each draft:

- **`merge:agent`** only when all of these are expected to hold:
  - No changed file matches `.fylkir/merge-paths`.
  - The change does not touch what triggers the security pass (the commands under "When this pass runs" in `.fylkir/security-review.md`).
  - It stays within the line cap in `.fylkir/agent-merge.sh`, not counting tests and generated files.
  - It needs no spec or design-document change and no decision of the engineer's.
  - Every acceptance item can be shown by tests, CI or commands an agent can run: nothing the engineer must dispatch, sign in to or check by hand.
- **`merge:human`** otherwise.

Write the reasoning: the condition that decides it, and for `merge:agent` which tests in the acceptance evidence would catch a defect. The script checks the mechanical conditions again before it merges; the label records that the engineer is content not to read the pull request.

## 7. Write each issue

Title: `<area>: <imperative summary>`, for example `cli: reject an unknown output format`. Labels from `gh label list`, such as `bug`, `enhancement`, `documentation` or `spike`, plus the suggested `merge:agent` or `merge:human`.

The body uses exactly the headings of `.github/ISSUE_TEMPLATE/work-package.md`:

- **Scope**: what changes and why, today's behaviour with file and function references, and the design (step 5).
- **Primary area**: the packages or directories the change is centred in.
- **Depends on**: issue numbers, or "None", plus any overlap you judged safe and why.
- **Acceptance evidence**: checkbox items. Each one names the test, command or CI step that proves it, and the observable result. Use the real dependencies `AGENTS.md` asks tests to use. Include docs, regenerated files and release-note updates where they apply. "Works correctly" is not evidence.
- **Design**: the parts of a spec it implements, and the design documents and decision records it relies on or changes, with each one's status, and whether a status blocks the merge. "None" when nothing applies.
- **Merge**: the suggested label and its reasoning (step 6).
- **References**: design-doc sections, related issues and PRs, spikes.

Use synthetic users, accounts and records in examples. No real data.

## 8. Check each draft

Before showing it, confirm that `/issue` would accept it: acceptance evidence and primary area present; dependencies either closed or listed as waiting; spec and design status addressed; no serialising file shared with another open issue and no primary area shared unless the issue says why that is safe; a merge suggestion with its reasoning.

## 9. Present and wait

Show the user every draft in full (title, labels, body), then the order: which can run in parallel and which wait for which, and which are suggested for `merge:agent`. **Stop and wait for the user's approval.** If you run as a Fylkir agent (your first message from Fylkir named your agent), make the drafts your final message, run `fylkir ask "File the <n> work-package drafts for <goal> above?" --option "File them: as drafted" --option "Revise: I'll say what to change"` and end your turn; the answer arrives as your next message. Approving a draft approves its merge label unless the user changes it. Revise and show again when asked. Do not file anything before the user says so.

## 10. File the approved issues

Create them in dependency order so later issues can name earlier ones by number. If `gh label list` lacks `merge:agent` or `merge:human`, create it first:

```sh
gh label create merge:agent --color 0e8a16 --description "The orchestrator may merge the pull request once .fylkir/agent-merge.sh accepts it"
gh label create merge:human --color d93f0b --description "The responsible engineer merges the pull request"
```

Pass each body inline with a quoted heredoc, so the shell expands nothing in it, and with a delimiter that does not occur in the body:

```sh
gh issue create --title "<title>" --label <label> --label <merge label> --body "$(cat <<'BODY'
<body>
BODY
)"
```

Report each issue's number and URL, its merge label and the order to run them in.
