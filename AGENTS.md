# terra

<!-- For the project: describe terra here in a few sentences, and say where its design documents and specs live. -->

This file is the single set of agent instructions. `CLAUDE.md` only imports it, so Claude Code and Codex read the same rules.

## Ground rules

These apply to humans and coding agents.

- **Conventional Commits.** Every commit message follows `type(scope): summary`. Types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `build`, `ci`, `chore`. Scope is the package, module or area. Summary is imperative, lower case, no trailing period. Use the body to explain why.
- **No co-author trailers.** Do not add `Co-Authored-By`, `Claude-Session`, or generated-with attribution lines to commit messages or pull request descriptions. The author of the commit is the human who owns the branch.
- **One task, one branch, one worktree.** Every Fylkir agent works in its own worktree on its own branch, `fylkir/<name>-<id>`, started from the latest `main`; keep that branch. A human working by hand branches from current `main` and names the branch `<type>/<short-description>`. Reference the GitHub issue in the PR, not in every commit. Start in the issue's primary area and change whatever else the work needs; the PR lists the files outside it.
- **Do not guess upstream APIs.** Check every dependency call against the pinned version's source or documentation, not against memory.
- **Synthetic data only.** Examples, fixtures and tests use generic synthetic users, accounts and records. No real credentials, personal data or customer data.
- **No incidental side effects on import or construction.** No registration, automatic migrations or network access when a module is loaded or an object is built. Lifecycle is explicit.
- **No retries until green.** No automatic retries and no rerunning a failed check until it passes. A flaky test is a `bug` issue with the seed or order that reproduces it; `/flake` finds them.
- **Never push to `main`, tag, release or force-push.** Agents merge only as "Agent merges" below allows.

## Commands

<!-- For the project: how to build, run and test it, and how to start its development environment. -->

```sh
make check    # the checks CI runs
make fmt      # format the workspace
```

## Design documents

- `docs/brief.md` is the product brief. Its §11 phases are the plan.
- `docs/decisions/` holds decision records, `NNNN-<slug>.md`, in the format `docs/decisions/README.md` gives. A record is Accepted when the responsible engineer merges it.
- `docs/design/` holds design documents.
- A change that settles one of the brief's §13 open questions needs a decision record or a design document, which only the responsible engineer approves.

## Testing

<!-- For the project: what tests must use (for example a real database rather than mocks), and which tests need a development environment. -->

- Tests are deterministic and run in `make check`; none is hidden behind a flag CI doesn't set.
- A skipped test names the issue that removes the skip.
- Audio fixtures are synthetic: tones generated with ffmpeg, tagged with generic names such as "Artist A". Keep them small, and commit them together with the script that regenerates them.
- Tests never need an audio device or the network.

## Checks before a pull request

Run these once before the first push, and again only after changing code; CI runs the rest, and the review runs alongside it. Do not merge `main` and rerun the checks only because `main` moved: each push to `main` tests the combined code.

- `make check` passes.
<!-- For the project: list any check that applies only to some paths, such as a code generator that must leave no diff. -->

## Files that serialise work

Only one open issue should change each of these at a time; otherwise chain the issues with "Depends on". Two branches can each pass CI with them and still break `main`.

<!-- For the project: list the files here, such as database migrations or a schema with a sequence number. -->
- `Cargo.toml` at the root: the workspace manifest.
- `crates/terra-storage/migrations/`: migrations are numbered in sequence, so two branches can each add the same number.

Other shared files, such as dependency manifests, lockfiles and generated code, end at worst in a merge conflict or a regeneration, which the later pull request resolves.

## Agent workflow

Fylkir runs the agents. Each one runs in its own worktree, and those that change code on their own branch; `.fylkir/project.toml` holds the worktree setup and the roles below, and `fylkir roles` lists them with their provider. A Claude Code agent invokes a skill as `/issue 42`, a Codex agent as `$issue 42`. The skills are in `.claude/skills/`, with a copy in `.agents/skills/` for Codex; keep the two the same.

| Role | Provider | Runs | Started by |
| --- | --- | --- | --- |
| `orchestrator` | Claude Code | `/orchestrate [workers]` | the responsible engineer: `fylkir spawn --role orchestrator "/orchestrate 3"` |
| `planner` | Claude Code | `/work-package <goal>` | the responsible engineer: `fylkir spawn --role planner "/work-package <goal>"` |
| `implementer` | Claude Code | `/issue <n>` | `/orchestrate`, one per issue |
| `reviewer` | Codex | `.fylkir/review.md` | `/issue`, at the pull request's head commit |
| `security-reviewer` | Codex | `.fylkir/security-review.md` | `/issue`, when the security pass applies |
| `triager` | Claude Code | `/triage-followups [since]` | `/orchestrate`, or the responsible engineer |
| `flake-hunter` | Claude Code | `/flake <test or CI run>` | the responsible engineer: `fylkir spawn --role flake-hunter "/flake <test or run>"` |

Issues use the work-package template (`.github/ISSUE_TEMPLATE/work-package.md`): scope, primary area, dependencies, acceptance evidence, design and merge (`merge:agent` or `merge:human`, with the reasoning). Parallel issues keep to different primary areas. The `planner` and `triager` roles get no branch: a checkout of the newest `main`, without the worktree setup, because they change no file.

- `/work-package <goal>` turns a goal, bug or plan item into work packages: it researches the code and open work, drafts the issues with a merge suggestion and files them only after the responsible engineer approves. Open design questions get the `decision-needed` label, which `/issue` refuses. Run it in your own Claude Code or Codex session, or as a Fylkir agent with the `planner` role.
- `/orchestrate [n]`, run as the `orchestrator` role, starts an `implementer` for every ready issue (up to n at once, default 3, never two with overlapping primary areas). An issue whose dependencies are open waits until they merge. It reports a red `main` first and starts no workers until it is green, updates a PR from `main` only when it conflicts, merges what `.fylkir/agent-merge.sh` accepts and opens a revert when such a merge turns `main` red, relays handoffs and the responsible engineer's answers, and archives merged work. It never decides. Fylkir holds new turns near a usage limit by itself, so the orchestrator never checks usage.
- `/issue <n>` implements the work package in the implementer's worktree, runs the checks above, opens the PR and, while CI runs, starts an independent `reviewer` with `fylkir spawn --revision <head>`, so the reviewer reads its own detached checkout of exactly that commit. The reviewer ends its turn with its result as one JSON object matching `.fylkir/review-schema.json`, which Fylkir passes back to the implementer. A change that matches "When this pass runs" in `.fylkir/security-review.md` gets a `security-reviewer` alongside it. The implementer posts the triaged review on the PR, fixes the findings it agrees with, has the fixes reviewed once more, posts that triage and stops with declined, escalated and second-round findings, through a Fylkir handoff to the responsible engineer. Two review rounds are the cap; further rounds only on request.
- `/triage-followups [since]` collects the deferred work and open review findings that merged PRs left in their bodies and triage comments, drops what an issue already covers and drafts the rest as work packages; `/orchestrate` starts it when a merged PR lacks the `followups-triaged` label.
- `/flake <test or CI run>` reproduces a flaky test with a recorded seed, finds its cause and drafts a `bug` work package.

`/work-package`, `/triage-followups` and `/flake` change no file and file nothing before the responsible engineer approves. Reviewers never edit files. When an agent needs the responsible engineer to decide something, it runs `fylkir ask` with the options it sees, which the engineer answers in the TUI; for anything else it raises `fylkir handoff request --target human`, which appears in the attention queue, and ends its turn; the engineer answers with a message to that agent.

Codex reviewers run in Codex's sandbox, which may have no network, so they review from Git and the files alone: the implementer puts the issue and earlier review comments in the reviewer's prompt. `.codex/rules/review.rules` forbids the commands that would write to Git or GitHub. Codex applies a project's rules only when you trust the project, and has no rules per role: they apply to every Codex agent here. If you run another role on Codex, such as the implementer, give that role `--ignore-rules` in Fylkir's own configuration (`[codex.roles.implementer]`), not here.

### Agent merges

`/orchestrate` merges a pull request when `.fylkir/agent-merge.sh --merge <pr>` accepts it:

- its issue carries `merge:agent`, which `/work-package` suggests and the responsible engineer approves with the draft, and nothing carries `merge:human`, `decision-needed` or `hold`;
- it targets `main`, is not a draft, the required checks pass on its head, and it has no conflict;
- the last `Independent review` comment reviewed that head, with nothing declined, escalated or open in round 2, every acceptance item met and the security pass skipped;
- no changed path matches `.fylkir/merge-paths` (dependencies, CI, the agent workflow, migrations);
- it changes at most 400 lines outside tests and generated files;
- every CI run of the latest push to `main` passed, so merges go one at a time;
- no open issue carries `merge-freeze`, which the responsible engineer opens to stop agent merges, for example before a deploy.

The gate needs required status checks on `main` (branch protection or a ruleset); without them it accepts nothing. When an agent-merged pull request turns `main` red, `/orchestrate` opens a revert for the responsible engineer to merge. Every other merge, every tag and release, and every deploy stays with the responsible engineer. A change to `.fylkir/merge-paths` or the script is itself a human merge.
