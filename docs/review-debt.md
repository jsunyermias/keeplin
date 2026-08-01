# Review debt

Every change needs an independent reviewer: the implementer is not the only reviewer. Only
the maintainer sets that aside, for one named pull request at a time, and doing so defers the
review rather than removing it. This file is the durable record of what is currently
deferred, so a decision taken under time or token pressure stays visible afterwards.

An agent never waives review and never opens an entry on its own initiative. It writes one
down when the maintainer's decision has already been made, in the same session the change
merges. The obligation to state what goes unchecked *before* that merge lives in `AGENTS.md`,
under "Independent review is not an agent's to waive".

## How an entry is cleared

An independent reviewer — a person, or a model family other than the one that implemented the
change — examines the merged diff against its issue objective with
`docs/prompts/0.C-prompt-revision-seguridad.md`, exactly as it would an open pull request.
Findings become issues on their own merit. The entry then moves to Cleared with a link to
that review.

A re-read by the implementing family does not clear an entry, and neither does a green CI
run: both are what the waiver already had.

## Open

| Merged | Change | Implementer | What went unreviewed | Follow-up |
|---|---|---|---|---|
| — | — | — | — | — |

## Cleared

| Merged | Change | Reviewer | Review |
|---|---|---|---|
| 2026-07-26 | [keeplin#178](https://github.com/jsunyermias/keeplin/pull/178) — `orden-05` phase 1 | Codex | [2026-08-01 retrospective sweep](reviews/2026-08-01-governance-debt-sweep.md) |
| 2026-07-26 | [keeplin#180](https://github.com/jsunyermias/keeplin/pull/180) · [keeplin-srv#92](https://github.com/jsunyermias/keeplin-srv/pull/92) — `orden-04d` | Codex | [2026-08-01 retrospective sweep](reviews/2026-08-01-governance-debt-sweep.md) |
| 2026-07-26 | [keeplin#181](https://github.com/jsunyermias/keeplin/pull/181) · [keeplin-srv#93](https://github.com/jsunyermias/keeplin-srv/pull/93) — `orden-04e` | Codex | [2026-08-01 retrospective sweep](reviews/2026-08-01-governance-debt-sweep.md) |
