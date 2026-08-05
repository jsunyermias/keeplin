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

**Where the evidence lives, and who can write it.** Stated explicitly because the rest of this
section cannot be evaluated without it, and because an earlier draft omitted it.

| Artifact | Location | Writer |
|---|---|---|
| Review ledger | the pull request **body** | anyone who can edit the pull request |
| Journal records | pull request **comments** | only the configured App: `verifyJournal` refuses any comment whose `app_slug`/`app_id` do not match `config` |
| Authorization directive | the **body of a review or comment** it is carried in | its author, whose `author_association` is checked |

**None of the three is in the file diff.** All three are visible on the pull request page. A
reviewer who reads the pull request sees them; a reviewer who reads only `git diff` does not.

**Trust boundary.** Between anyone who can write to the repository or its workflows, and the
evaluator running from the default branch.

**Adversaries and capabilities.**

1. *An agent or contributor wanting a pull request to converge.* Can edit the ledger and pull
   request body, and open comments. Cannot, today, produce a verifying directive.
2. *An actor with repository write access.* Can add or modify a workflow that runs under the same
   App identity, recompute the unkeyed digest chain, and manufacture convergence on a history in
   which no finding was ever reified. Can also truncate the newest journal record.

Recomputing the chain is not merely a matter of write access to a file: journal records live in
comments and `verifyJournal` refuses any comment not attributed to the configured App, so
adversary 1 cannot rewrite history without reaching that identity. The chain's weakness is that
the App identity is shared with any repository workflow, not that the records are loosely
writable.

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

- **Benefits.** Preserves the separation formally, and — this is a real advantage an earlier draft
  of this ADR collapsed away — creates a **capability boundary**. An agent operating with the
  maintainer's ordinary credentials could not authorize without also reaching the separate
  credential. Under Option A it can. A credential held behind an Environment approval, with
  least privilege and its own revocation and audit trail, is materially different from Option A,
  not "Option A with a credential in between".
- **Costs.** Operational burden: a credential to hold, rotate and protect, plus an approval step
  on a path that runs many times per pull request. Because *every* disposal needs authorization,
  not only genesis, that step recurs per finding.
- **Failure modes.** If no human judgement sits behind the step, it becomes a rubber stamp that
  *looks* like independent authorization and is therefore worse than Option A, which is at least
  honest about what it is. Whether that is the likely outcome here is a **design judgement, not a
  measured claim**: this ADR asserts it because the same person would drive both sides, and offers
  no evidence beyond that.
- **On compromise.** An earlier draft said compromising the credential "converges any pull
  request". That is overstated. Convergence still requires the required jobs green and, for
  `resolved`, a success check bound to the head, workflow, run and App. What compromise actually
  buys is arbitrary authorization of disposals, which manufactures convergence *when the other
  conditions already hold*.

### Option C — Relaxed association for single-principal repositories

Declare an explicit policy exception when the repository has exactly one principal with a
sufficient association.

- **Benefits.** Same effect as Option A, framed as a bounded exception with a detectable
  precondition.
- **Costs.** The precondition must be evaluated at authorization time against live repository
  state. That is **not a new API dependency** — an earlier draft claimed it was, wrongly: the
  evaluator already reads `author_association` from the API on this very path. It is a further
  call of the same class, with its own failure mode and the policy complexity of deciding what
  happens when it fails. Option A reaches the same outcome without it and can carry the same
  detectability as a declared condition.
- **Assessment.** Reasonable; strictly more machinery than Option A for the same result.

### Option D — Pre-seeded genesis anchor

Seed the journal on the default branch so it is never empty.

- **Assessment.** **Insufficient, and the measurement shows why.** It addresses `GENESIS` and
  leaves every ordinary finding undisposable. The durable form of the argument does not depend on
  the exact count: *pre-seeding removes at most the synthetic blocker and leaves untouched every
  ordinary blocker whose authorization does not verify.* The specific `blocking: 15` → 14 reading
  comes from keeplin-srv#114's journal and is **an external result this ADR has not had
  independently re-derived**; a reviewer should confirm it from the linked journal rather than
  from this document. Recorded so the option is not revisited without that evidence.

### Option E — Keep the current behaviour

- **Assessment.** The status quo is that pull requests merge above open stall records, and
  `AGENTS.md` declares the default branch not-done while such a record is open. Two merges have
  now taken that route. Keeping it means the convergence rule exists only on paper.

## Decision and justification

> `proposed`. The maintainer has indicated Option A as the intended direction; that is a
> statement of intent recorded here, not an approval. Implementation remains blocked until this
> ADR is `accepted`.

**Recommend Option A.** The pull request author may authorize disposal of a finding on their own
pull request when the authorization is carried by a directive satisfying **all** of the following.
This list is deliberately **not** offered as a transcription of `verifyAuthorization`: an earlier
draft said "when, and only when" and then proved incomplete, so exhaustiveness is left to the code
and the items below are grouped by what each one actually governs.

*Conditions on the reference itself, checked by `verifyAuthorization`:*

1. it is authored by a principal whose `author_association` is `MEMBER`, `OWNER` or `COLLABORATOR`
   — `NONE` is never sufficient, and this is not relaxed;
2. it names the exact finding ID, the exact target state, and a non-empty reason;
3. it is bound to the repository and pull request number it applies to;
4. it is **not** carried by a review whose state is `DISMISSED`. `verifyAuthorization` refuses one
   today and 0008 established the rule expressly — a dismissed review returns the finding to
   `open`. An earlier draft omitted this condition entirely; the omission is recorded rather than
   quietly repaired.

*A temporal rule of the ordinary-finding path, applied in `evaluateTrustedReviewLoop` and not in
`verifyAuthorization`:*

5. the directive is issued strictly after the observation that reified or last changed the
   disposition of that finding, preserving the existing same-second comparator. This rule does
   **not** apply to `GENESIS` or to tombstones today, and this ADR does not extend it to them.

*Required persistence, an effect of evaluation rather than a precondition of it:*

6. the authorization is recorded in the journal with its reference identity, author and body
   digest, exactly as a third-party directive is today. A first evaluation may take the evidence
   from the current ledger, verify it and project the closure before `publishEvaluation` writes
   the record; the journal is the durable result, not a gate the directive passes through.

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

**Why this over the alternatives.** Option D is insufficient for the reason recorded above: it
removes at most the synthetic blocker. Option E leaves a rule that is routed around, which is
worse than a rule honestly relaxed.

Option B is the serious alternative, and this recommendation does **not** rest on dismissing it.
Its capability boundary is real: a credential the agent cannot reach is materially different from
Option A, and compromising it does not by itself converge anything — the required jobs must still
be green, and a `resolved` still needs its success check bound to head, workflow, run and App. The
argument against B is narrower than an earlier draft of this ADR claimed, and both independent
reviewers caught that draft contradicting itself here. The argument is: the approval step recurs
on **every** disposal, not only genesis; the same person would stand on both sides of it in this
repository; and a step with no independent judgement behind it is worse than Option A precisely
because it *looks* like independent authorization. **That is a design judgement about this
project, not a measured finding**, and a maintainer who weighs the operational cost differently
should choose B.

Option C reaches the same place as A through an additional live lookup on the authorization path
— not a new class of dependency, since the evaluator already reads `author_association` there —
and A can carry the same detectability as a declared condition.

**What now defends against an agent self-disposing — stated precisely, because an earlier draft
overstated it.**

Nothing technical, **for an agent that controls a qualifying GitHub identity** — and in this
repository's tooling it does.

The precision matters, because an earlier draft offered the wrong evidence. Git commit authorship
(`Claude <noreply@anthropic.com>` on every commit of keeplin-srv#114) is Git metadata and proves
nothing about who can publish a review or comment under a qualifying association. The evidence
that does establish it: the agent-authored comments on that pull request are attributed to
`jsunyermias` with `author_association: OWNER`, because the tooling acts through the maintainer's
GitHub identity. That is the capability Option A stops filtering.

Under the adversary-1 definition used above — an actor able to edit the ledger and the pull
request body — the capability is implied anyway, since GitHub allows that only to the author or a
principal with write access. So against adversary 1 as defined, the technical control is
*eliminated*, not reduced. A comment-only actor with `author_association: NONE` is still refused
by condition 1, but that actor cannot touch the ledger either, so the residue is not a defence of
anything this decision protects.

What remains is a record and a requirement, and neither is enforced by the evaluator:

- **The record.** A directive is dated, attributed and digest-bound, and the ledger row citing it
  is in the pull request body. As the table above states, **neither is in the file diff**: a
  reviewer reading the pull request sees them, a reviewer reading only `git diff` does not.
  `AGENTS.md` says "the ledger is part of the diff the independent reviewer examines"; that
  sentence is imprecise in the same way this ADR's earlier draft was, and correcting it belongs to
  a documentation issue, not here.
- **The requirement.** `AGENTS.md` requires an independent reviewer and forbids an agent from
  waiving one. This ADR does not touch that. But it is procedural: **no mechanism enforces it**,
  and `check-review-governance.js` runs inside the head-controlled `ci.yml`, which
  [0012](0012-default-branch-review-governance.md) already records as weakenable by a head.

So the honest formulation is: the defence moves from a mechanism to a convention, and the
convention's own enforcement is known to be incomplete. That is the cost of Option A, and the
maintainer is accepting it knowingly or not at all.

## Consequences and risks

**Positive.** Pull requests become able to converge, and findings become recordable as closed.

**Negative.** Against adversary 1 the technical control is eliminated, not reduced, for the reason
given under *Decision*: agents here act with the maintainer's credentials as a matter of course.

**Negative — branch protection cuts both ways.** Making `Review loop converged` a required check
becomes possible, and an earlier draft listed only that upside. The risk is the other half: it
would formalize as the sole *enforced* merge control precisely the gate this decision weakens,
while independent review — the control that actually replaces it — remains procedural and
unenforced. If the maintainer adds it, that asymmetry should be a conscious choice, not a side
effect of the gate finally being openable.

**Residual risks.**

- The relaxation persists silently if a second principal joins and nobody revisits it. Mitigated
  by the follow-up below, not by the mechanism.
- Directive fatigue: disposing of fourteen findings on one pull request requires fourteen
  directives. If that friction leads to batch-authorizing without reading, the audit trail records
  the act but not the absence of judgement.
- **keeplin-srv#114 is merged, and no mechanism described here clears it.** Directives bind to a
  pull request number, and nothing in the evaluator defines whether it runs on a closed pull
  request or how directives issued against one would produce a re-evaluation. Its
  `docs/review-stalls.md` row therefore stays open, and verification item 11 below is scoped to
  what the mechanism actually supports rather than asserting an outcome with no path to it.
  Defining the post-merge route is follow-up work, listed below.

**Observability — an explicit non-guarantee.** The journal records each disposal's reference,
author and digest, so the self-authorized/third-party split **is** derivable after the fact by
comparing each directive's recorded author against the pull request author. **What acceptance
does not deliver is any aggregate, readily visible signal**: nothing surfaces that split without
reconstructing it by hand, and therefore nothing makes it easy to notice the relaxation becoming the norm, fatigue
setting in, or abuse accumulating — precisely the failure modes that matter once the control moves
from prevention to after-the-fact detection. An earlier draft said "no new signal is required",
which contradicted the sentence that followed it. A counter on each journal record would close
this; whether it ships in the acceptance PR is the maintainer's call, and it is not assumed here.

**Follow-up work.**

- A procedure, step by step, for issuing and recording a directive, linked from `AGENTS.md`.
- **The second-principal check** — one that fails when the repository gains another principal with
  sufficient association while self-authorization is enabled, so the exception cannot outlive its
  premise silently. The *Forces* section requires this relaxation to "fail loudly"; that force is
  unmet without it. **This ADR proposes that the check ship in the acceptance PR**, not later, and
  says so rather than leaving the assignment ambiguous. The maintainer may decide otherwise, but
  then the force should be struck rather than left nominally satisfied.
- **Correcting `AGENTS.md`.** Its sentence "the ledger is part of the diff the independent reviewer
  examines" is imprecise in the same way an earlier draft of this ADR was: the ledger is in the
  pull request body, not the file diff. This ADR flags it rather than editing it, and that flag
  would otherwise be an orphaned obligation — so it is listed here as work to be opened as a
  documentation issue.
- A defined post-merge route for already-merged pull requests carrying undisposed findings, so
  keeplin-srv#114's stall row has an exit.
- Clearing `docs/review-stalls.md` on both repositories through that exit.

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

An earlier draft claimed "each item must fail if the corresponding behaviour is reverted" for the
whole list, which was not true of every item. The list is now grouped by what each item actually
establishes.

**Group 1 — regression of the decision itself.** These fail if the self-authorization change is
reverted.

1. **Positive.** A directive authored by the pull request author, satisfying conditions 1–6,
   disposes of a reified finding and the blocking set shrinks. Test in
   `.github/scripts/check-review-loop.test.js`.
2. **End to end.** A real pull request issues a directive and its `Review loop converged` check
   publishes `converged`. keeplin#206's acceptance criterion 3 requires this, and nothing short of
   a real run satisfies it.

**Group 2 — invariants that must survive the change.** These fail when *their own* behaviour is
reverted, not when self-authorization is. They pin what the decision promises to leave untouched.

3. **Negative — association.** The same directive from an author with `author_association: NONE`
   is refused. This is the case the chosen policy rejects, and keeplin#206's acceptance criterion 5
   requires it to be covered by a named test.
4. **Negative — binding.** A directive naming a different finding, a different target state, a
   different pull request or a different repository is refused.
5. **Negative — ordering.** A directive issued in the same second as, or before, the observation
   that reified the finding is refused; the existing same-second comparator is unchanged.
6. **Negative — reification.** A reified finding cannot be moved to `advisory` without a
   verifying directive, and the retired-ID reservation still refuses a returning ID unreified.
7. **Negative — dismissed review.** A self-authorized directive carried by a review later set to
   `DISMISSED` returns the finding to `open`. This pins condition 4, which an earlier draft of
   this ADR omitted entirely.
8. **Failure injection.** With the authorization reference unreachable, the finding projects back
   to `reified: true, state: "open"` with its `disposalError`, exactly as today.

9. **Negative — the exception cannot outlive its premise.** With a second principal holding a
   sufficient association present, and self-authorization still enabled, the second-principal
   check fails. Without this item the *Forces* requirement that the relaxation "fail loudly" has
   no verifier, and the acceptance pull request could merge without the check while the plan
   reported nothing missing.

**Group 3 — deployment symmetry.** Byte-identity alone proves symmetry, not policy: if both
repositories reverted the change together it would still pass. It is therefore paired with a
behavioural check.

10. **Cross-repository.** The governance files remain byte-identical between the two repositories,
   **and** the Group 1 positive test passes when run against each repository's copy of
   `check-review-loop.js` independently.

**Group 4 — operational closure.** Moving a Markdown row is an administrative act that persists
whether or not the mechanism still works, so it cannot stand alone as evidence.

11. **Operational.** For each `docs/review-stalls.md` row moved to `Cleared`, a reproducible
    evaluation of the pull request it names no longer projects its blockers to `open`. The row
    move is the record of that, not the proof. **Scoped deliberately**: keeplin-srv#114 is merged
    and no route exists yet for re-evaluating a closed pull request, so this item covers pull
    requests the evaluator can still evaluate. keeplin#206's criterion 4 is met only once the
    post-merge route in *Follow-up work* exists.

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
