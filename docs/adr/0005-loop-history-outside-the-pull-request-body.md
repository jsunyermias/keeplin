# 0005 — Loop history lives outside the pull-request body

- Status: proposed
- Date: 2026-08-03
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — raised as finding F-002 by the independent review of
  [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198)
- Acceptance PR: link once the ADR is accepted
- Supersedes: none — amends the storage mechanism of
  [0004](0004-review-loop-convergence.md), whose convergence and blocking rules stand unchanged
- Superseded by: none

## Context and problem

[ADR 0004](0004-review-loop-convergence.md) gave the review loop a mechanical convergence
condition and a stagnation brake. Its compatibility section records that the ledger "lives in
pull-request bodies and `docs/review-stalls.md`". The independent review of the implementing
pull request found that this storage choice defeats the brake it was meant to enforce.

Verified behaviour, reproduced against the implementation:

- The round log is parsed out of the pull-request body, which any agent in the loop can edit.
  Deleting earlier `Round log` rows resets the non-shrink streak: a loop that had reached
  `escalated` after four non-shrinking rounds returns to `converging`.
- Deleting the findings table as well yields an empty ledger, which converges immediately.
- The parser requires only that round numbers ascend, not that they are continuous or that
  previously recorded rounds are preserved, so a rewrite is indistinguishable from a fresh
  loop.

ADR 0004's threat model anticipated that agents author the ledger and accepted that they could
misclassify a finding, with independent review as the mitigation. It did not anticipate that
the *history* is equally editable, and it therefore overclaims: it states that "indefinite
silent iteration ceases to be reachable", which the implementation does not deliver. The brake
is the one invariant that cannot rely on review as its mitigation, because its entire purpose
is to fire when review and implementation have stopped making progress on their own.

## Forces and requirements

- **The brake must survive an editing agent.** A stagnation brake whose history is written by
  the party it constrains is not a brake. This is the whole force behind the decision.
- **The findings table must stay editable.** Reviewers and implementers legitimately add
  findings and change their states every round. Making the ledger read-only would break the
  loop it serves.
- **Convergence and blocking rules from ADR 0004 do not change.** Reification as the blocking
  criterion, advisory findings, durable dismissals and the definition of convergence stand.
  Only the location of the authoritative history moves.
- **No new operational dependency.** The mechanism must not require a database, an external
  service, or a package registry at CI time.
- **Degradation must be visible.** Where the mechanism cannot work, it must fail closed and
  say so, not silently revert to trusting the body.
- **Both repositories behave identically.**

## Threat model

- **Asset:** the integrity of the loop's history — the record of what was open, and for how
  many rounds it failed to shrink.
- **Trust boundary:** between agents inside the loop, who may edit any pull-request field, and
  the maintainer, who is escalated to.
- **Adversary:** an agent optimising for a green check, including one that would not lie
  outright but would "tidy" a round log to keep it readable.
- **Capability removed by this decision:** rewriting or erasing observed loop history.
- **Capabilities retained:** everything ADR 0004's threat model already accepted — an agent can
  still classify a reifiable finding as advisory, mark a finding `resolved` without evidence,
  or write a thin dismissal citation. Those remain semantic judgments left to independent
  review, and this decision does not claim to close them.
- **New exposure:** the mechanism requires `checks: write` on one job. That token can create
  check runs on this repository. It is scoped to the `converge` job, and the job runs no
  third-party code beyond the pinned actions the workflow already uses.
- **Accepted limitation:** for a pull request from a fork, `GITHUB_TOKEN` is read-only on
  `pull_request` events, so no history can be written. This is recorded rather than worked
  around; see Consequences.

## Options considered

**A. Keep ADR 0004's storage and accept the gap.** Record F-002 as dismissed, citing the
existing threat model, and correct the overclaim. Benefit: no work, no new permission. Cost:
the stagnation brake remains defeatable by editing a Markdown table, so the change's central
promise — that a stuck loop surfaces instead of iterating — is unenforced. Rejected because the
brake is precisely the invariant that cannot fall back on review.

**B. Append-only enforcement within the body.** Require the round log to only grow. Benefit:
no new storage or permission. Cost: it cannot work — the checker has no memory of what the log
previously contained, so "only grew" is unverifiable from a single evaluation. Rejected as
incoherent rather than merely weak.

**C. Persist history in a tracked file in the repository.** Write the round log to
`docs/review-loop/<pr>.md` and require it to grow. Benefit: durable and reviewable. Cost: the
file is in the diff, so the same agent can edit it in the same commit; it merely moves the
problem and adds per-pull-request file churn to every branch. Rejected.

**D. Persist history in check-run outputs written by CI.** Each evaluation publishes a
`Review loop state` check run whose output encodes the round: the loop-state hash, the blocking
count, and the open reified finding IDs. Check runs belong to GitHub and are not editable
through the pull request, so the history an agent can reach is the rendering, not the record.
The checker reconstructs history from prior `Review loop state` runs across the pull request's
commits. Benefit: closes the gap with no new service, no new dependency, and no new file.
Cost: one extra permission, one extra check run per evaluation, and no coverage for fork pull
requests. Evidence that would change the assessment: if fork contributions become normal for
these repositories, this mechanism covers the minority of pull requests and needs replacing.

## Decision and justification

This ADR proposes option D. It states the recommendation; it is not approved until the
maintainer moves this ADR to `accepted`.

1. **The authoritative loop history is the sequence of `Review loop state` check runs** for the
   pull request's commits, written by CI. Each carries, in machine-readable form, the round
   number, the loop-state hash, the blocking count, and the open reified finding IDs observed
   at that evaluation.
2. **The `Round log` in the pull-request body becomes a rendering, not a source.** The checker
   derives the streak and the repeated-state test from the check runs. A body round log that
   disagrees with the observed history is `malformed` — the disagreement is reported rather
   than silently preferred in either direction.
3. **A finding ID that CI has observed may change state, but may not disappear.** Removing it
   from the ledger is `malformed`. This closes the "delete the findings too" path, which
   option D would otherwise leave open.
4. **The stagnation brake reads only observed history.** Deleting or renumbering body rows
   therefore cannot reset a streak or clear an escalation the loop has already earned.
5. **Where history cannot be written, the loop fails closed.** If the token cannot create a
   check run — a fork pull request being the known case — the checker reports that the history
   is unverifiable and does not converge, rather than falling back to the editable body.
6. **ADR 0004's convergence and blocking rules are unchanged.** Reification as the blocking
   criterion, the advisory disposition, durable dismissals, monotonic progress, K, and the
   escalation record in `docs/review-stalls.md` all stand exactly as decided there. This ADR
   amends only where the history is kept, and corrects 0004's claim that silent iteration was
   already unreachable.

## Consequences and risks

Positive: the brake becomes enforceable rather than advisory; the escalation record and the
streak survive an agent that rewrites the body; and the checker's message can cite the exact
prior evaluation, so a maintainer can audit the history without trusting the pull request.

Negative and residual:

- One additional check run per evaluation. These accumulate on a long-running pull request and
  are visible in the checks list. They are deliberately `neutral`, so they neither pass nor
  block on their own.
- `checks: write` is a real permission increase over the current read-only workflow, confined
  to the `converge` job.
- **Fork pull requests are not covered.** Their token is read-only, so history cannot be
  written and the loop will not converge. For these repositories, whose pull requests come from
  branches, that is currently vacuous — but it is a hard edge, and if fork contributions start,
  this decision needs revisiting rather than quietly relaxing.
- Every semantic gaming vector ADR 0004 accepted remains accepted. This decision narrows what
  an agent can rewrite; it does not make the ledger truthful.

## Compatibility, migration, and rollback

No wire, protocol, format or persistence surface is touched; `PROTOCOL_VERSION`, the storage
format versions and every migration are unaffected. The change is confined to `.github/` and
`docs/`.

Migration: a pull request open when this lands has no prior `Review loop state` runs. Its
history begins at the first evaluation after the change, exactly as a new pull request's does.
No pull request is retroactively invalidated, and no existing body content becomes invalid —
the round log keeps rendering, it simply stops being authoritative.

Rollback: revert the pull requests. The check runs left behind are inert metadata that expire
with GitHub's retention; nothing reads them once the checker is gone.

## Verification plan

Deterministic `node --test` cases that fail before the change and pass after it:

- Deleting or renumbering body round-log rows does not reset a streak or clear an escalation
  that observed history already establishes — the reification of finding F-002.
- Removing a previously observed open finding ID from the ledger is `malformed`.
- A body round log that contradicts observed history is `malformed`, naming the disagreement.
- Observed history with no prior runs behaves as round zero, so the migration path is covered.
- Unwritable history does not converge and says why, covering the fork case without a fork.
- ADR 0004's existing suite continues to pass unchanged, demonstrating that convergence and
  blocking rules were not altered.

## Equivalent decision in the other repository

Canonical here, in `jsunyermias/keeplin`, like ADR 0004, because it binds both repositories'
governance equally. `jsunyermias/keeplin-srv` registers and links it under "Canonical
cross-repository decisions" rather than copying its reasoning. No `keeplin-core` surface and no
dependency pin is affected. The paired work is two coordinated pull requests carrying the
byte-identical checker, suite and workflow job, each linking the other.
