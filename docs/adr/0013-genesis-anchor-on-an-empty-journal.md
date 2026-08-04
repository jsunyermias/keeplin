# 0013 — What an empty review journal may do

- Status: proposed
- Date: 2026-08-04
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — found by running the trusted evaluator against the live GitHub API after
  [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198) merged
- Acceptance PR: pending maintainer acceptance
- Supersedes: none. Amends the genesis-authorization consequence of
  [0008](0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md)
- Superseded by: none

## Context and problem

The trusted evaluator injects a synthetic `GENESIS` finding whenever the journal is empty and
requires verified authorization for it, exactly as it does for a tombstone. Without an authorized
directive the evaluation returns `history-unverifiable` and refuses.

That rule was written and reviewed against fixtures. Its first execution against the live API, on
[keeplin#186](https://github.com/jsunyermias/keeplin/pull/186), produced:

```
GENESIS lacks verified authorization: authorization reference is unreachable.
```

Every pull request that predates the evaluator starts with an empty journal, so every one of them
is refused, and no procedure for authorizing a genesis record exists anywhere in either
repository. The loop cannot start.

This ADR decides what an empty journal is permitted to do. It does not revisit verified disposal,
the digest chain, or the bounded authenticity claim of [0011](0011-bounded-journal-authenticity.md).

## Forces and requirements

- A pull request must be able to reach a first evaluation without a manual step that is nowhere
  documented.
- The genesis record anchors the digest chain. Whatever is decided must state precisely what an
  unauthenticated anchor does and does not prove.
- The refusal must remain visible on the pull request. That is fixed separately in
  [keeplin#200](https://github.com/jsunyermias/keeplin/pull/200) and is required under every option
  here.
- Convergence must not become reachable through a path that is cheaper than the ones ADR 0011
  already concedes.
- Both repositories run byte-identical evaluator logic.

## Threat model

The protected asset is the claim that a converged pull request had no reified finding open across
its whole recorded history.

ADR 0011 already concedes that an actor who can **run a repository workflow with the configured App
identity** can recompute the digests and manufacture a history in which no finding was ever
reified, and that terminal truncation is undetected.

The genesis requirement adds one thing on top of that: it makes *restarting* the chain cost
something. With it, an actor who deletes every journal comment faces an empty journal that demands
an authorized directive. Without it, deleting the journal is equivalent to starting fresh.

This is the crux. Removing the requirement lowers the bar for a clean-history restart from
**"run a workflow with the App identity"** to **"delete issue comments with repository write
access"**. The second is strictly cheaper and needs no workflow at all.

The decision does not defend against a maintainer changing the default branch, compromised GitHub
infrastructure, or credentials outside the pull-request boundary.

## Options considered

### A. Allow an unauthenticated genesis on an empty journal

An empty journal starts the chain with no directive. The loop runs immediately for every existing
and future pull request, and nothing needs documenting.

Cost: the restart bar drops to deleting comments, as above. A converged result then proves only
that no reified finding was open *in the records that currently exist*, with no lower bound on how
those records came to be. ADR 0011's claim would need widening to say so.

### B. Require an authorized genesis, and document how to write one

The status quo, plus the missing runbook: what directive to publish, in what format, and who may
author it. The anchor keeps its full strength.

Cost: a manual authorization step for every pull request, forever, before the loop will say
anything at all. Combined with the fact that the refusal was invisible until #200, this is what
produced the current state where nothing works and nothing says why. It also front-loads the cost
onto every pull request, including those that will never approach convergence.

### C. Allow an unauthenticated genesis to EVALUATE, but not to CONVERGE

An empty journal starts the chain and the record is marked unauthenticated. Evaluation proceeds
normally and publishes its real state. The synthetic `GENESIS` finding stays **open and reified**
until an authorized directive exists, so `converged` is unreachable without the anchor while every
other state is reachable without it.

Cost: an authorization step is still required, but only once per pull request and only when the
author actually wants to converge. The evaluator has to carry an "unauthenticated anchor" flag
through the journal record, which is new state and needs its own tests.

## Decision and justification

**Pending maintainer decision.** The maintainer indicated a preference for option A and asked for
this ADR before implementation, which is why the body is written with all three options intact
rather than one.

The recommendation of this ADR is **option C**. It removes the blockage that motivated the
question — no pull request is refused for a missing anchor, and every one of them reports its real
state — while leaving the anchor mandatory for the single state that authorizes a merge. Option A
buys the same unblocking by paying the security cost on every pull request forever, including the
ones that would never have needed it. Option B keeps the strongest anchor and is the only option
that requires no code change, but it makes the loop unusable until a runbook exists and imposes a
step before any feedback at all.

If the maintainer accepts option A instead, this ADR must also widen ADR 0011's stated bound to
record that a clean-history restart requires only comment deletion. Accepting A without that
correction would leave a documented claim that the implementation no longer supports, which is the
defect class this project has spent thirty-two review rounds removing.

## Consequences and risks

Under A: the loop works everywhere immediately; the authenticity claim narrows and 0011 needs a
matching correction.

Under B: the anchor is strongest; the loop stays blocked until the runbook exists and is followed
per pull request.

Under C: the loop works everywhere immediately for every state except `converged`; a new
`unauthenticatedAnchor` fact enters the journal record and every consumer of it needs coverage,
including the recovery path, which currently assumes an authorized anchor.

Under all three: the refusal must publish its check run, which #200 delivers independently.

## Compatibility, migration, and rollback

No wire, format, persistence or `keeplin-core` surface is touched. Existing journals are
unaffected, because the decision only governs the empty case.

Rollback under A or C is a revert plus a new ADR; journals written in the interim would carry
records whose anchor is unauthenticated, and reverting does not retroactively authorize them. That
asymmetry is a reason to decide once rather than to try A and see.

## Verification plan

- Executable tests for the empty-journal path, not regexes over the workflow.
- Under C specifically: a converged result must be unreachable while the anchor is
  unauthenticated, and reachable immediately after an authorized directive appears — both pinned
  by tests that fail against the code preceding them.
- A live run against a real pull request before the decision is called verified. This ADR exists
  because fixture-level review passed a rule that the live API refused, and that is not a mistake
  worth repeating.

## Equivalent decision in the other repository

`keeplin-srv` runs byte-identical evaluator logic and links this decision rather than copying it,
per its own `docs/adr/README.md`.
