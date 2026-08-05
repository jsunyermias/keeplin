# 0015 — Self-authorized disposal with an auditable directive

- Status: proposed
- Date: 2026-08-05
- Decision owners: `jsunyermias`
- Scope: cross-repo
- Issue: [keeplin#206](https://github.com/jsunyermias/keeplin/issues/206)
- Acceptance PR: link once the ADR is accepted
- Supersedes: none. Amends the authorization precondition that
  [0008](0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md) established and
  [0013](0013-genesis-anchor-on-an-empty-journal.md) applied to genesis
- Superseded by: none

## Context and problem

### The verified defect

`.github/scripts/check-review-loop.js` disposes of a finding only through `verifyAuthorization`,
which requires a directive whose author simultaneously satisfies two conditions:

- the author is **not** the pull request author;
- the author's `author_association` is `MEMBER`, `OWNER` or `COLLABORATOR`.

A collaborator query run on 2026-08-05 against both repositories returned exactly one principal:

| Repository | Principal | Role |
|---|---|---|
| `jsunyermias/keeplin` | `jsunyermias` | admin |
| `jsunyermias/keeplin-srv` | `jsunyermias` | admin |

No app, bot or service account holds a sufficient association. The same principal opens the pull
requests. The two conditions therefore cannot be satisfied simultaneously by anyone. This resolves
acceptance criterion 0 of keeplin#206, which the issue left open: the path is *unreachable*, not
merely undocumented.

### The consequence is wider than keeplin#206 states

keeplin#206 describes the defect as blocking convergence through the synthetic `GENESIS` finding.
Measurement on a real pull request shows it is broader.
[keeplin-srv#114](https://github.com/jsunyermias/keeplin-srv/pull/114) was the first pull request
the default-branch evaluator ever evaluated. Its published journal records report:

```json
{"observation": 3, "unauthenticatedAnchor": true, "blocking": 15}
```

Fifteen blockers: `GENESIS` plus **fourteen ledger findings that were fixed and independently
verified**, each carrying the same diagnosis:

```json
"disposalError": "authorization reference is unreachable"
```

The mechanism is in `evaluateTrustedReviewLoop`: any finding whose ledger state is `resolved` or
`dismissed` and whose authorization does not verify is projected back to `reified: true,
state: "open"`. That behaviour is correct — the evaluator must not accept prose as evidence of
disposal — but combined with the unreachable authorizer it means **no finding in either
repository can ever be recorded as closed**, whether or not its pull request would otherwise
converge.

That is a stronger statement than keeplin#206 makes, and it is the fact this decision must
answer.

### Evidence of the practical outcome

Two pull requests have now merged above an open stall record rather than through the gate:
keeplin#203, cited by keeplin#206 itself, and keeplin-srv#114, whose
`docs/review-stalls.md` row remains open on `main` with all fifteen blockers named and
`Exit taken` empty. `AGENTS.md` defines an open row there as an open condition under *Definition
of done*. The gate that eleven ADRs built is being routed around because it cannot be opened.

### Facts, inferences and proposals

- **Fact**: the collaborator query result above, the evaluator's published `blocking: 15`, and the
  fourteen `authorization reference is unreachable` diagnoses.
- **Fact**: `verifyAuthorization` binds repository, pull-request number, non-author authorizer,
  exact finding, target state, and recorded reference identity, author and body digest. It does
  **not** bind a head SHA or a nonce; that limitation is recorded in
  [0014](0014-correct-the-empty-journal-decision-record.md).
- **Inference**: no configuration of the current repositories admits a qualifying authorizer
  without adding a principal.
- **Proposal**: everything under *Decision and justification*.

## Forces and requirements

- **The gate must become openable.** A convergence condition no participant can satisfy is not a
  high standard; it is a broken mechanism that trains its users to merge around it.
- **Disposal must remain deliberate and attributable.** Whatever replaces the separation must
  leave a record that a later reader can audit without the transcript of an agent session.
- **The bounds of 0008 and 0011 must not be overstated.** The journal's chain is unkeyed: it
  detects accidental corruption and casual editing, not forgery by a repository workflow carrying
  the same App identity, and terminal truncation is not detected. No option may be presented as
  closing those holes.
- **Reification must remain irreversible by shortcut.** A finding that named a mechanical check
  may not become `advisory` by relabelling.
- **The relaxation must be detectable if the project grows.** A rule justified by "one human" must
  fail loudly, not silently persist, once that premise stops holding.
- **Independent review is untouched.** `AGENTS.md` states that convergence is never a substitute
  for an independent reviewer, and that an agent does not waive that review. This decision changes
  what *convergence* requires; it changes nothing about who may review.

## Threat model

**Asset.** The claim, carried in the journal and the ledger, that a reified finding was disposed
of deliberately by someone entitled to decide.

**Trust boundary.** Between anyone who can write to the repository or its workflows, and the
evaluator running from the default branch.

**Adversaries and capabilities.**

1. *An agent or contributor wanting a pull request to converge.* Can edit the ledger and pull
   request body, and open comments. Cannot, today, produce a verifying directive.
2. *An actor with repository write access.* Can add or modify a workflow that runs under the same
   App identity, recompute the unkeyed digest chain, and manufacture convergence on a history in
   which no finding was ever reified. Can also truncate the newest journal record.

**What the current separation actually buys against each.** Against adversary 1 it is real: an
agent cannot self-dispose. Against adversary 2 it buys nothing, because that actor controls the
identity the chain is built on. 0008 and 0011 already concede this.

**Accepted leakage and non-goals.** This ADR does not authenticate history against adversary 2,
does not detect terminal truncation, and does not introduce a keyed chain. Those remain the bounds
of 0008 and 0011 and are unchanged.

**The honest statement of what is being given up.** In a repository whose only principal is the
maintainer, the author/authorizer separation defends against *the maintainer's own agents acting
without the maintainer*, not against a second human. Option 1 below removes it and replaces it
with an explicit, attributable act by the maintainer. That is a real reduction in defence against
adversary 1, and it is the cost this decision accepts.

## Options considered

### Option A — Self-authorization with an auditable directive *(recommended)*

The pull request author may authorize disposal of a finding on their own pull request, provided
the directive is recorded as a first-class, machine-readable artifact bound to that pull request
and finding.

- **Benefits.** Opens the gate with no new credential and no new principal. Keeps every disposal
  an explicit, dated, attributable act rather than an implicit consequence of editing a table.
  Preserves the evaluator's refusal to accept prose.
- **Costs.** Removes the author/authorizer separation. Against adversary 1 this is a genuine
  reduction: an agent operating with the maintainer's credentials could issue the directive. The
  mitigation is attribution and reviewability, not prevention.
- **Failure modes.** A maintainer who authorizes without reading disposes of findings by habit.
  The directive's audit trail makes that visible after the fact; nothing prevents it.
- **What would change the assessment.** A second human principal joining the project, which would
  make the separation meaningful again and this relaxation unnecessary.

### Option B — A service principal issues the directives

An app or bot account with sufficient association authorizes disposals.

- **Benefits.** Preserves the separation formally.
- **Costs.** Introduces a credential whose compromise converges any pull request. Because the
  measurement above shows *every* disposal needs authorization — not only genesis — that principal
  would have to authorize each one. Either a human drives it, in which case it is Option A with a
  credential in between, or it does not, in which case it is a rubber stamp that satisfies the
  check while providing none of the judgement the separation exists to obtain.
- **Failure modes.** The rubber-stamp outcome is the likely one, and it is worse than Option A
  because it *looks* like an independent authorization.

### Option C — Relaxed association for single-principal repositories

Declare an explicit policy exception when the repository has exactly one principal with a
sufficient association.

- **Benefits.** Same effect as Option A, framed as a bounded exception with a detectable
  precondition.
- **Costs.** The precondition must be evaluated at authorization time against live repository
  state, which adds an API dependency to the evaluator and a new failure mode when that call
  fails. Option A achieves the same outcome without that dependency, and can carry the same
  detectability as a declared condition rather than a runtime query.
- **Assessment.** Reasonable; strictly more machinery than Option A for the same result.

### Option D — Pre-seeded genesis anchor

Seed the journal on the default branch so it is never empty.

- **Assessment.** **Insufficient, and the measurement shows why.** It addresses `GENESIS` and
  leaves the fourteen ledger findings undisposable. It would turn `blocking: 15` into
  `blocking: 14` and change nothing about the underlying defect. Recorded here so the option is
  not revisited without that evidence.

### Option E — Keep the current behaviour

- **Assessment.** The status quo is that pull requests merge above open stall records, and
  `AGENTS.md` declares the default branch not-done while such a record is open. Two merges have
  now taken that route. Keeping it means the convergence rule exists only on paper.

## Decision and justification

> `proposed`. The maintainer has indicated Option A as the intended direction; that is a
> statement of intent recorded here, not an approval. Implementation remains blocked until this
> ADR is `accepted`.

**Adopt Option A.** The pull request author may authorize disposal of a finding on their own pull
request when, and only when, the authorization is carried by a directive that:

1. is authored by a principal whose `author_association` is `MEMBER`, `OWNER` or `COLLABORATOR`
   — `NONE` is never sufficient, and this is not relaxed;
2. names the exact finding ID, the exact target state, and a non-empty reason;
3. is bound to the repository and pull request number it applies to;
4. is issued strictly after the observation that reified or last changed the disposition of that
   finding, preserving the existing same-second comparator;
5. is recorded in the journal with its reference identity, author and body digest, exactly as a
   third-party directive is today.

The only condition removed is the requirement that the directive's author differ from the pull
request author. Everything else that `verifyAuthorization` binds stays bound.

**Invariants this establishes.**

- Disposal remains an explicit act with a named reason, never an inference from a table edit.
- Reification remains remembered: a finding that named a mechanical check cannot become
  `advisory` without a verified directive, and this ADR does not create a shortcut around that.
- The evaluator still refuses prose. A ledger row without a directive still projects to `open`.
- The bounds of 0008 and 0011 are unchanged and unweakened; this decision neither claims nor
  provides authenticity against a workflow sharing the App identity, nor detection of terminal
  truncation.

**Why this over the alternatives.** Option D is measurably insufficient. Option E leaves a rule
that is routed around, which is worse than a rule honestly relaxed. Option B's separation is
nominal once the same human drives the service principal, and it adds a credential whose
compromise is equivalent to convergence. Option C reaches the same place as A through a runtime
dependency that A does not need.

**What now defends against an agent self-disposing.** Not the check — the record. A directive is
dated, attributed and digest-bound, and the ledger row that cites it is part of the diff an
independent reviewer examines. `AGENTS.md` already requires that reviewer, and this ADR does not
touch that requirement. The defence moves from prevention to attributable review, and this ADR
says so rather than implying the protection survives intact.

## Consequences and risks

**Positive.** Pull requests become able to converge. The fourteen findings of keeplin-srv#114 and
its `docs/review-stalls.md` row become clearable through a defined exit rather than by merging
above them. `Review loop converged` becomes a candidate for branch protection — which remains the
maintainer's step, and only after a pull request has actually reached `converged`.

**Negative.** An agent operating with the maintainer's credentials can issue a directive that
verifies. This is a real reduction against adversary 1 and is the accepted cost.

**Residual risks.**

- The relaxation persists silently if a second principal joins and nobody revisits it. Mitigated
  by the follow-up below, not by the mechanism.
- Directive fatigue: disposing of fourteen findings on one pull request requires fourteen
  directives. If that friction leads to batch-authorizing without reading, the audit trail records
  the act but not the absence of judgement.
- This ADR does not make keeplin-srv#114's existing findings retroactively disposable. Its stall
  row stays open until directives are issued for it under an implementation of this decision.

**Observability.** The journal already records each disposal's reference, author and digest. No
new signal is required. What is missing is a way to see, at a glance, how many disposals were
self-authorized versus third-party authorized; a counter in the journal record would make the
relaxation's usage visible and is proposed as follow-up, not required for acceptance.

**Follow-up work.**

- A procedure, step by step, for issuing and recording a directive, linked from `AGENTS.md`.
- A check that fails when the repository gains a second principal with sufficient association
  while self-authorization is still enabled, so the exception cannot outlive its premise silently.
- Clearing `docs/review-stalls.md` on both repositories through a defined exit.

## Compatibility, migration, and rollback

**Wire and format compatibility.** Not applicable. This decision touches neither the collab
protocol, `PROTOCOL_VERSION`, the `Change` model, format limits, the encryption envelope, nor any
persistent store. `keeplin-core` is unaffected and its pin does not move.

**Journal compatibility.** The journal record schema is unchanged by the decision itself. If the
follow-up counter is added, it is a new digest-bound field on new records only; existing records
remain verifiable, exactly as `unauthenticatedAnchor` was introduced under 0013.

**Migration.** None. Directives issued after acceptance apply to findings already in a ledger; no
stored data is rewritten.

**Rollout ordering.** `check-review-loop.js` is shared byte-identically between the two
repositories, so the implementation lands as a coordinated pair of pull requests. Because the
evaluator runs from the default branch, the change takes effect for a repository only once merged
to its `main`, and the two repositories may briefly disagree. That window is benign: the stricter
behaviour is the current one.

**Rollback.** Reverting the implementation restores the current rule and returns every repository
to the state where no disposal verifies. Findings disposed of under this decision would project
back to `open`. Rollback is therefore safe but not free: it re-blocks anything that converged
under it.

## Verification plan

Each item must fail if the corresponding behaviour is reverted.

1. **Positive.** A directive authored by the pull request author, satisfying conditions 1–5,
   disposes of a reified finding and the blocking set shrinks. Test in
   `.github/scripts/check-review-loop.test.js`.
2. **Negative — association.** The same directive from an author with `author_association: NONE`
   is refused. This is the case the chosen policy rejects, and keeplin#206's acceptance criterion 5
   requires it to be covered by a named test.
3. **Negative — binding.** A directive naming a different finding, a different target state, a
   different pull request or a different repository is refused.
4. **Negative — ordering.** A directive issued in the same second as, or before, the observation
   that reified the finding is refused; the existing same-second comparator is unchanged.
5. **Negative — reification.** A reified finding cannot be moved to `advisory` without a
   verifying directive, and the retired-ID reservation still refuses a returning ID unreified.
6. **Failure injection.** With the authorization reference unreachable, the finding projects back
   to `reified: true, state: "open"` with its `disposalError`, exactly as today.
7. **Cross-repository.** The governance files remain byte-identical between the two repositories;
   the existing symmetry check covers this.
8. **End to end.** A real pull request issues a directive, and its `Review loop converged` check
   publishes `converged`. keeplin#206's acceptance criterion 3 requires this, and nothing short of
   a real run satisfies it.
9. **Operational.** `docs/review-stalls.md` rows on both repositories move to `Cleared` naming the
   exit taken, and keeplin#206's criterion 4 is met.

Not verifiable, and stated rather than omitted: nothing here demonstrates authenticity against a
repository workflow carrying the same App identity, or detection of terminal truncation. Those
bounds belong to 0008 and 0011 and this plan does not test what it cannot establish.

## Equivalent decision in the other repository

This ADR is canonical in `jsunyermias/keeplin`, per the registry rule that cross-repository
decisions live here. `keeplin-srv` carries no copy; its `docs/adr/README.md` links this record.
The implementation is a coordinated pair of pull requests, because `check-review-loop.js` and its
workflow are byte-identical across both repositories and the symmetry check enforces that. No
`keeplin-core` pin or version implication follows, since no shared wire or format surface is
involved.
