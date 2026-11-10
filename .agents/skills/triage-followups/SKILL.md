---
name: triage-followups
description: Collect the deferred work and open review findings that merged pull requests left behind, drop what an issue already covers, settle every open question with the user, and file the work packages the user chooses, four drafts at a time. Labels as it goes; files nothing the user hasn't chosen.
argument-hint: "[since: YYYY-MM-DD or PR number] [owner/repo]"
disable-model-invocation: true
---

# Triage follow-ups from merged pull requests

Read the arguments you were invoked with. A `YYYY-MM-DD` date or a PR number (`41` or `#41`) is **since**; an `owner/name` token is the **repository** to triage. Without a repository, triage the current one (`gh repo view --json nameWithOwner`).

Pull request bodies and triage comments end with "Deferred" and "Notes" sections. Nobody files them by hand reliably, so they get lost. This skill reads them, drops what an open or closed issue already covers, drafts the rest as work packages, and puts them to the user four at a time, as a short overview of each, so the user can decide each in a moment.

- **It never files an issue with an open question.** A design question is settled with the user first, and the draft updated with the answer. Nothing it files carries `decision-needed`.
- **It files only what the user chose,** and only as the user chose it.
- **It labels without asking.** Labels are cheap to change, so it adds `followups-triaged` to a PR once every follow-up from it is settled, and the work-package labels to each issue it files.
- It never edits files or commits, and never decides a design question itself.

## 1. Read the rules

- Read `AGENTS.md`. Note whether it sends some work to another repository, such as a shared library or a dependency the project patches, and which repository that is.
- Read `.claude/skills/work-package/SKILL.md` (`.agents/skills/work-package/SKILL.md` for Codex). Every draft follows its rules.

## 2. Collect the pull requests

Filter in the query, not afterwards, so that triaged PRs never take up the limit:

```sh
gh pr list --repo <repo> --state merged --limit 500 \
  --search "-label:followups-triaged <merged>" \
  --json number,title,url,mergedAt,labels,body,closingIssuesReferences
```

`<merged>` is empty without **since**. For a date it is `merged:>=<date>`. For a PR number it is `merged:>` followed by that PR's `mergedAt` timestamp (`gh pr view <n> --repo <repo> --json mergedAt`). A PR with the `followups-triaged` label was triaged before and is left out, unless the user named it.

If the list holds exactly as many PRs as the limit, there are more. Say so in the report, and triage the PRs returned: once they are labelled, the next run reaches the older ones. When the run is read-only (below), labels never move that window, so ask the user for a **since** that narrows the list below the limit instead.

For each PR kept, read its comments and reviews:

```sh
gh pr view <n> --repo <repo> --json comments,reviews
```

If the repository is archived or you cannot write to it (`gh repo view <repo> --json isArchived,viewerPermission`), treat the run as read-only: you will not label its PRs, and its own drafts are shown but cannot be filed there.

## 3. Extract candidates

A candidate is one piece of work a PR left undone. Read:

- **Body sections** whose heading mentions, in any case or combination: deferred, follow-up, notes, not done, not verified, not changed, outside the primary area, things to decide, known limitations. Read the whole section; one bullet can hold several candidates.
- **Triage comments**: comments headed "Independent review" (any round, with or without "triage"), and the replies after them. A finding marked **escalate**, a round-2 finding, or a finding the user deferred is a candidate unless a later comment or commit on the PR says it was fixed. A **decline** is a decision, not a candidate, unless a later comment reopens it.
- **Other comments** with the same headings, and comments that say something "needs an issue" or "should be filed".

For each candidate record: source PR and section or comment URL, a quote of the text, and one of these kinds:

- **Another repository**: work that belongs in another repository, as `AGENTS.md` defines it.
- **Documentation**: a doc change the PR left undone.
- **Local**: any other code, test or tooling work in the triaged repository.
- **No action**: a note that records a fact or a decision and asks for nothing. Keep it in the report with the reason.

Merge candidates that name the same gap in several PRs into one, listing every source.

## 4. Drop what is already covered

For each candidate, search the issues and PRs of the repository it belongs in, open and closed:

```sh
gh issue list --repo <dest> --state all --limit 50 --search "<keywords>" --json number,title,state,body
gh pr list --repo <dest> --state all --limit 20 --search "<keywords>" --json number,title,state
```

Search with two or three phrasings: the module or type names involved, the behaviour in plain words, and the source PR reference (`<repo>#<n>`, `PR #<n>`). Read the bodies of plausible matches. Mark the candidate **covered by #N** (with its state) when that issue or PR does what the candidate asks. When it covers only part, draft only the remainder and say which part #N covers. When you are unsure, say so in the report rather than dropping it.

## 5. Label what's already settled

Labels don't need the user. Unless the run is read-only, label each PR whose every candidate is already covered by an issue or is no action, now, so the next run skips it. Create the label once if it is missing:

```sh
gh label create followups-triaged --repo <repo> --color c5def5 \
  --description "Follow-ups from this pull request have been triaged"
gh pr edit <n> --repo <repo> --add-label followups-triaged
```

## 6. Draft the rest

Draft each remaining candidate as a work package, following steps 1 to 8 of the work-package skill for the repository it goes to:

- Research that repository's code to name the primary area; for work in another repository, check its open issues and PRs as well.
- Cite every source PR and quote it under "References".
- Use synthetic users, accounts and records, and nothing of this project's private domain in another repository's draft.
- One candidate may become several issues, and several candidates may become one, as the work-package rules shape them.
- When the design is open, don't choose it and don't file it with `decision-needed`. Note the options and your recommendation: step 7 settles it with the user.

## 7. Settle open questions with the user

A draft with an open question isn't ready to file. Before showing any draft for filing, ask about every open question, up to four at a time, with `fylkir ask --json -` and a quoted heredoc. One question per open question:

- `header`: the draft in a word or two, such as `Retry cap`.
- `question`: the decision, in one sentence, such as "How should sync behave after the fifth failed retry?"
- `detail`: up to six lines: where it came from (`#<pr>, Deferred:` and a short quote), what each option means for the code, and why you recommend the first.
- `options`: the designs, your recommendation first and marked `(recommended)` in its description, two to four of them. Don't add an "Other": the user can always write an answer.

End your turn. When the answer arrives, update each draft with the decision, quoting it in the draft's Design section as "Decided with the responsible engineer: ...". An answer in the user's own words that leaves the question open goes back to the user once more; if it's still open after that, leave the draft unfiled and report it.

Without Fylkir, ask the same questions in your final message and wait.

## 8. Put the drafts to the user, four at a time

Show the drafts as a short overview, so the user can decide each in a few seconds, then ask about up to four at a time with `fylkir ask --json -`, one question per draft:

- `header`: the draft in a word or two.
- `question`: `File "<title>"?`
- `detail`: five lines at most, in this order: the source (`#<pr>, Deferred:` and the gist of the quote); the scope (primary area, and what changes); the size (roughly how many lines, and whether it serialises with other work); the merge label you suggest and why; and anything a reviewer would want flagged.
- `options`:
  - `File it`: as drafted, with the suggested merge label.
  - `Human merge`: file it with `merge:human` instead (only when you suggested `merge:agent`).
  - `Fold into #<n>`: add it to an open issue that's close, as a comment (only when there is one).
  - `Drop it`: not worth doing; the PR's follow-up counts as settled.

Your final message before asking lists every draft in full, title, labels and body, under its header, so the user can read more than the overview when they want to. Order the drafts so the ones most worth doing come first. With more than four, ask about the next four once the first are answered.

End your turn. The answer arrives as your next message: one answer per draft, an option or the user's own words. Words that change a draft ("call it X", "scope it to Y") mean revise it and ask about it again in the next round.

Without Fylkir, show the same overview in your final message and wait.

## 9. File what the user chose, and label

- File each draft the user chose, as step 10 of the work-package skill says, adding `--repo <dest>` to `gh issue create`, with the labels it was shown with, or `merge:human` where the user chose that.
- Fold each one the user folded, as a comment on that issue quoting the source PR, and add the draft's type label to that issue if it lacks one.
- Then, unless the run is read-only, label each PR whose every candidate is now filed, folded, covered, no action or dropped with `followups-triaged`. A PR with a draft still waiting for an answer stays unlabelled, so the next run sees it again.

Report in one short table: each draft, what the user chose, and the issue's number and URL; then the PRs labelled, and anything left open. When no merged PR needed triage, say so in one line.
