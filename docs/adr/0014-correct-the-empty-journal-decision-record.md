# 0014 — Correct the empty-journal decision record

- Status: proposed
- Date: 2026-08-04
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — independent review of
  [0013](0013-genesis-anchor-on-an-empty-journal.md) after it was accepted and merged
- Acceptance PR: pending maintainer acceptance
- Supersedes: [0013](0013-genesis-anchor-on-an-empty-journal.md) on four statements only: its
  threat-model statement, its deletion claim, its option-A characterisation and its compatibility
  claim. **The decision itself — option C — stands unchanged and is not revisited here.**
- Superseded by: none

## Context and problem

ADR 0013 decided that an empty review journal may evaluate without an authorized anchor but may
not converge without one. That decision is sound and its implementation was independently reviewed
and accepted.

The **document** was reviewed after acceptance and rejected. The reviewer was explicit that it was
rejecting the record, not the decision. Four of its findings identify statements in 0013 that are
wrong or unqualified, and an accepted body is immutable except for status and cross-link metadata,
so they cannot be edited in place. This ADR corrects them.

The author of 0013 wrote it, recommended option C in it, and asserted the deletion claim below as
airtight. It was not. Recording that here is the point of the correction.

## Forces and requirements

- The decision must not change. Option C is implemented and reviewed; reopening it would be a
  separate decision with its own ADR.
- The corrections must be usable by a reader of 0013, so this ADR names each corrected statement
  and gives its replacement.
- 0013's remaining content stays authoritative. This is a partial supersession, like 0011's
  treatment of 0008.

## Threat model

**Correction 1 — the protected asset was stated in a way option C contradicts.**

0013 says the protected asset is the claim that a converged pull request *"had no reified finding
open across its whole recorded history"*.

Under option C that is false by construction: every pull request beginning with an empty journal
necessarily has the synthetic `GENESIS` finding open and reified in its recorded history, until an
authorization directive disposes of it. The stated invariant and the accepted decision contradict
each other inside the same document.

The protected asset is instead: **a converged pull request has no undisposed reified finding at
the moment of convergence.** The synthetic genesis finding is subject to that rule like any other —
it must be disposed by verified authorization before `converged` is reachable — but its presence
earlier in the history is expected, not a violation.

**Correction 2 — the deletion claim was absolute where it must be bounded.**

0013 says *"comment deletion buys a fresh evaluation, never a convergence"*.

That is not airtight as written, because ADR 0008 concedes that terminal truncation is undetected.
An actor who truncates the journal to a prefix ending at a converged observation leaves a
digest-valid history, and 0013 offers no journal-tip freshness or invalidation check that would
reject it.

The accurate claim is exactly this, and no more: **deletion creates no new convergence path; it
degrades a protection ADR 0008 already declared bounded.** Truncation erodes the journal's memory
that a finding was once reified — `everReified` is built across the surviving records — and that
erosion is precisely the limitation 0008 documents and 0011 bounds.

An earlier draft of this correction defended the claim by arguing that the evaluator re-derives
each round from the current pull request, so a truncated journal could not by itself produce a
`converged` verdict. **That defence was wrong and is withdrawn.** Re-derivation protects against
evaluating a stale but intact journal; the attack manipulates the journal's *completeness*, and
re-deriving from a truncated history re-derives the truncated state. It also contradicted the
concession in the same paragraph: the erosion of reification memory only has content because the
journal feeds the verdict, so the ledger cannot simultaneously be journal-independent. Replacing
one unestablished absolute with another is the defect this ADR exists to correct, and it was
committed here first.

**Consequence for ADR 0011.** 0013 concluded that 0011 needs no amendment, and that conclusion
still holds — reaching `converged` requires a verified authorization directive, which comment
deletion cannot manufacture. But 0013 asserted it rather than establishing it.

0013 also wrote of option A that *"deleting the journal comments would have been enough to reach
`converged`"*. That is too strong and is superseded: deletion would have **lowered the required
capability** from modifying a repository workflow to deleting comments, not guaranteed convergence
on its own — the evaluator must still run and everything else must still be clean.

What the authorization directive is bound to, stated rather than assumed, because the claim above
rests on it: `verifyAuthorization` binds the directive to the repository and pull-request number,
to an authorizing author who is not the pull-request author, to a directive naming the exact
finding and target state, and to the recorded reference identity, author and body digest. It does
**not** bind to a head SHA or a nonce; freshness across dispositions is enforced separately by
comparing the directive's issue time against the recorded disposition change. A directive therefore
cannot be replayed from another pull request or repository, and cannot survive an edit to its own
body, but does remain valid for its finding and state across later commits on the same pull
request. That is the property this correction relies on, and it is the whole of it.

## Options considered

### Correct the record with a new ADR

The registry's own pattern: an accepted body is historical record, and a later accepted ADR carries
the correction. Costs one document and leaves both readable in sequence.

### Edit 0013 in place as a non-substantive correction

Cheaper, and defensible for the compatibility sentence alone. Rejected because correction 1 changes
the stated protected asset, which is substantive by any reading, and the registry forbids
rewriting an accepted body to match a later understanding.

### Leave the defects and rely on this review being remembered

Rejected. A threat model that contradicts its own decision is precisely the kind of claim this
project has spent thirty-two review rounds removing, and other documents cite 0013's threat model.

## Decision and justification

Corrections 1 and 2 above supersede the corresponding statements in 0013, together with its
option-A characterisation quoted there. Correction 3 below supersedes its compatibility claim.
Everything else in 0013 stands, including the decision itself.

**Correction 3 — the compatibility claim contradicted the consequences.** 0013 states *"No wire,
format, persistence or `keeplin-core` surface is touched"* while also stating that a new
`unauthenticatedAnchor` fact enters the journal record and that every consumer of it needs
coverage. Both cannot be true. The accurate statement is: **no Rust wire, database or
`keeplin-core` surface is touched, and no existing journal field changes; the journal record gains
one additive boolean, and records written before it are read as authenticated.**

## Consequences and risks

The decision, the implementation and the review of the implementation are unaffected.

A reader of 0013 must read this ADR alongside it for those four statements. That is the standing
cost of correcting an accepted record rather than editing it, and it is the cost the registry's
lifecycle deliberately chooses.

Two further review findings are recorded here as accepted limitations of 0013 rather than
corrections, because they ask for content rather than fix errors: 0013 does not restate the
replay-and-binding properties of the authorization directive that its central claim inherits from
ADR 0008, and its verification plan omits adversarial cases — suffix truncation to a converged
prefix, replayed or foreign directives, and old-consumer handling of the new field. The
implementation pull request pins the cases that exist today; the omitted ones are follow-up work,
not claims this ADR makes.

## Compatibility, migration, and rollback

Documentation only. No code, schema or behaviour changes. Rollback is rejecting this ADR, which
leaves 0013's defective statements standing and is recorded as such.

## Verification plan

- `./scripts/check-docs.sh` passes, which is the only mechanical check a decision record has.
- Each corrected statement in this ADR quotes the 0013 text it replaces, so the correction can be
  checked by reading rather than inferred.

## Equivalent decision in the other repository

`keeplin-srv` links this decision rather than copying it, per its own `docs/adr/README.md`.
