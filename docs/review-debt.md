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

## The format this file must keep

`scripts/check-docs.sh` verifies the shape of this registry on every run, because an entry
nobody can act on is the same as no entry. Both sections below must exist with their exact
column headers, every cell must be answered, every `Change` must contain at least one
pull-request URL of the expected shape for either repository, every `Cleared` row must
carry an HTTP(S) link to the review that cleared it, and no pull request may appear more
than once or sit in both sections. Rows under any other level-two heading are misplaced and
fail with that heading named. Only the row immediately after a table header may be its
separator. Empty strings, `—`, `-`, `TBD` and `pendiente` are unanswered; a row whose cells
are all `—` marks a genuinely empty section and fails next to real entries. The check is
offline: it validates link shape, not pull-request state.

What no check can see is whether the review happened. A named reviewer and a linked thread
are a claim; the registry keeps that claim visible and attributable, which is the most a
mechanical check can honestly offer here.

## Open

| Merged | Change | Implementer | What went unreviewed | Follow-up |
|---|---|---|---|---|
| 2026-07-27 | [keeplin#186](https://github.com/jsunyermias/keeplin/pull/186) · [keeplin-srv#95](https://github.com/jsunyermias/keeplin-srv/pull/95) — `orden-04f`, the checks on this file | Claude; review-finding remediation by Codex | Kimi K3 rejected the original implementation after reproducing three verifier blind spots. This remediation addresses those findings, but the open entry remains until an independent reviewer examines the resulting diff and confirms the findings are resolved. | Kimi K3 re-review of PR #186 and companion PR #95, pending |

## Cleared

| Merged | Change | Reviewer | Review |
|---|---|---|---|
| 2026-07-26 | [keeplin#178](https://github.com/jsunyermias/keeplin/pull/178) — `orden-05` phase 1 | Codex | [2026-08-01 retrospective sweep](https://github.com/jsunyermias/keeplin/blob/main/docs/reviews/2026-08-01-governance-debt-sweep.md) |
| 2026-07-26 | [keeplin#180](https://github.com/jsunyermias/keeplin/pull/180) · [keeplin-srv#92](https://github.com/jsunyermias/keeplin-srv/pull/92) — `orden-04d` | Codex | [2026-08-01 retrospective sweep](https://github.com/jsunyermias/keeplin/blob/main/docs/reviews/2026-08-01-governance-debt-sweep.md) |
| 2026-07-26 | [keeplin#181](https://github.com/jsunyermias/keeplin/pull/181) · [keeplin-srv#93](https://github.com/jsunyermias/keeplin-srv/pull/93) — `orden-04e` | Codex | [2026-08-01 retrospective sweep](https://github.com/jsunyermias/keeplin/blob/main/docs/reviews/2026-08-01-governance-debt-sweep.md) |
