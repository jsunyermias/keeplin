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
column headers, every cell must be filled, every `Change` must link at least one merged
pull request of either repository, every `Cleared` row must link the review that cleared
it, and no pull request may sit in both sections. A row whose cells are all `—` marks a
genuinely empty section and fails next to real entries.

On the pull request side, the `Review independence declared` job requires either a named
independent reviewer or a complete maintainer waiver whose entry already exists here — so a
waiver and its record land in the same pull request, not in a later one that never comes.

What no check can see is whether the review happened. A named reviewer and a linked thread
are a claim; the registry keeps that claim visible and attributable, which is the most a
mechanical check can honestly offer here.

## Open

| Merged | Change | Implementer | What went unreviewed | Follow-up |
|---|---|---|---|---|
| 2026-07-26 | [keeplin#178](https://github.com/jsunyermias/keeplin/pull/178) — `orden-05` phase 1, `db.rs` split into `storage/db/` | Claude | Codex reviewed two rounds and confirmed both resolved, but never the head commit `e615194`, which fixed the last finding *and* two further instances of the same defect that the review had not caught. Its container inventories and path references therefore rest on the author's own verification. The PR also raised an ADR boundary question — relocating persistence and migration code — that no reviewer or maintainer answered. | maintainer sweep, pending |
| 2026-07-26 | [keeplin#180](https://github.com/jsunyermias/keeplin/pull/180) · [keeplin-srv#92](https://github.com/jsunyermias/keeplin-srv/pull/92) — `orden-04d`, prepared-issue format | Claude | No independent review at all: the same family proposed the rules, wrote them and self-reviewed them. Worth an outside eye on whether the new `0.A` rules contradict anything else in `AGENTS.md`, and on whether an English template filled with Spanish prose survives contact with real use. | maintainer sweep, pending |
| 2026-07-26 | [keeplin#181](https://github.com/jsunyermias/keeplin/pull/181) · [keeplin-srv#93](https://github.com/jsunyermias/keeplin-srv/pull/93) — `orden-04e`, this file and the waiver rule | Claude | No independent review: the family the rule is meant to bind is the one that wrote it. Two things need an outsider — whether the wording constrains agents in ways that are invisible from inside it, and whether a registry with no mechanical check survives contact with a busy week. Nothing verifies that a waived merge produced an entry here. | maintainer sweep, pending |

## Cleared

| Merged | Change | Reviewer | Review |
|---|---|---|---|
| — | — | — | — |
