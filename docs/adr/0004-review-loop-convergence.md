# 0004 — Deterministic convergence and a stagnation brake for the review loop

- Status: superseded
- Date: 2026-08-03
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — this decision originates from a maintainer order rather than a tracked issue;
  the order's text and the step-0 findings that justify it are recorded in the acceptance PR
- Acceptance PR: [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198), with
  [keeplin-srv#104](https://github.com/jsunyermias/keeplin-srv/pull/104) as its companion
- Supersedes: none
- Superseded by: [0006](0006-trusted-review-loop-history.md)

## Context and problem

The implementation↔review loop defined in `AGENTS.md` ("Workflow and review independence")
and driven by `docs/prompts/0.B-prompt-implementacion-issue.md` and
`docs/prompts/0.C-prompt-revision-seguridad.md` has no mechanical termination condition.

Verified current behavior, established by reading the driver rather than inferring it:

- The only automated gate on the loop is `.github/scripts/check-review-governance.js`,
  invoked from the `Check pull-request review governance` step of
  `.github/workflows/ci.yml` on non-draft pull requests. Both repositories carry
  byte-identical copies of the script, its test suite, the pull-request template and the
  workflow step.
- That script decides between exactly two paths — a recorded independent review, or a
  complete maintainer waiver backed by `docs/review-debt.md`. It never inspects findings.
- The single condition that stands for "the review loop finished" is the pull-request
  template checkbox `- [x] Blocking findings are resolved and conversations are closed.`
  `evaluateReviewGovernance` verifies only that the box is ticked, so the loop terminates
  on an assertion by the very agents inside it.
- Nothing in either repository holds loop state. There is no round counter, no finding
  identity, no record of what a previous round found or decided, and no comparison between
  one round and the next.

Three consequences follow mechanically from that absence, and are the problem this ADR
addresses:

1. **A satisfied reviewer is not a checkable condition.** "Blocking findings are resolved"
   is an opinion rendered as a boolean. Two agents can hold it simultaneously and
   inconsistently, and CI cannot contradict either.
2. **A reviewer without memory re-raises settled findings.** Because no finding has an
   identity or a recorded disposition, a finding dismissed on priority or on an accepted
   ADR in round 2 returns in round 4 as new, and restarts the loop.
3. **A stalled loop is indistinguishable from a working one.** Nothing detects that a round
   changed nothing, so the loop can iterate indefinitely without ever surfacing to the
   maintainer that it is stuck, and without naming what it is stuck on.

`docs/review-debt.md` already establishes the repository's pattern for the adjacent
problem: when a review obligation is deferred, the deferral becomes a durable, tracked
record instead of an informal memory. The review loop has no equivalent record for the
obligation it is *not* deferring but also not discharging.

## Forces and requirements

- **Independent review stays untouched.** `AGENTS.md` states that independent review is not
  an agent's to waive, and that only the maintainer sets it aside for one named pull
  request. Nothing here may weaken, bypass or automate that requirement; a mechanical
  convergence condition is a floor added beneath it, never a substitute for it.
- **Merging remains the maintainer's act.** This decision may make a pull request
  demonstrably ready or demonstrably not ready. It may never merge one.
- **Every termination condition must be computable.** A condition an agent asserts about
  its own satisfaction is not a termination condition.
- **Findings differ in kind.** A reviewer legitimately raises concerns that cannot be
  reduced to a failing check — a readability concern, a design preference, a risk worth
  recording. Treating those as blocking makes the loop unterminable; discarding them loses
  real signal. They need a third disposition, not a binary.
- **Progress must be observable, not asserted.** A round that produces no measurable
  reduction in the blocking set is not progress, however much work it contained.
- **The brake must not be a clock.** A time or token budget rewards slow-walking and
  punishes a genuinely hard finding. The brake must measure state change.
- **Both repositories must behave identically.** The loop spans coordinated PRs; a rule
  enforced on one side only is not enforced.
- **The checker must stay dependency-free.** `actions/github-script` loads the existing
  governance script directly from the repository with no install step; a new checker that
  needs a package registry would add an operational dependency to CI.

## Threat model

Security in the cryptographic sense is not involved: no asset, key, credential or user data
is touched. The adversary model that *is* relevant is a governance one, and is stated
explicitly because the whole point of the decision is to remove discretion from agents.

- **Assets:** the integrity of the claim "this pull request converged", and the visibility
  of a stuck loop to the maintainer.
- **Trust boundary:** between agents operating inside the loop (implementer and reviewer,
  both of which may be language models) and the maintainer, who merges.
- **Adversary:** not malice, but an agent optimizing for a green check — the failure mode
  already observed, in which a loop terminates because an agent asserted it had.
- **Capabilities the adversary retains under this decision:** an agent authors the finding
  ledger, so it can classify a finding as advisory that a stricter reviewer would have
  reified, or dismiss one with a thin citation. This decision narrows discretion; it does
  not eliminate it. What it removes is *silent* discretion: every disposition is written
  down, attributed to a round, and visible in the diff the independent reviewer reads.
- **Accepted residual leakage:** a determined agent can still write a plausible-looking
  ledger. The mitigation is not mechanical — it is that the ledger is reviewed by the
  independent reviewer like any other part of the diff.
- **Explicit non-goal:** this decision does not attempt to judge whether a finding *should*
  have been reified. It only guarantees that the answer was recorded and that the loop
  cannot terminate while a reified finding is open.

## Options considered

**A. Keep the current behavior.** No new machinery; the checkbox continues to stand for
convergence. Benefit: zero cost, no new surface. Cost: all three failure modes above remain,
and the loop's termination stays an opinion. Failure mode: the loop cycles or blocks, which
is the reported behavior. Evidence that would change the assessment: none available — the
problem is structural, not a tuning issue.

**B. A wall-clock or round-count budget.** Stop the loop after N rounds or T minutes.
Benefit: trivially mechanical, one number. Cost: it is the wrong measurement. A loop making
real progress on a hard finding is killed at the same threshold as one spinning, and an
agent can satisfy the budget by producing rounds that change nothing. Failure mode: it
converts "iterating forever" into "stopping arbitrarily", which is not better. Rejected on
the explicit requirement that the brake measure state change, not elapsed effort.

**C. Require every finding to be blocking until a human clears it.** Benefit: maximally
conservative, no finding is ever lost. Cost: every advisory remark becomes a merge blocker,
so the loop becomes unterminable in practice and the pressure to under-report findings
increases. Failure mode: reviewers learn to say less. Rejected.

**D. Reification as the blocking criterion, plus a finding ledger, plus a state-hash
stagnation brake.** A finding blocks only when it is reified — expressed as something that
mechanically fails. Every finding carries a stable ID and a recorded disposition.
Convergence is required checks green and zero open reified findings. A per-round hash of the
loop state detects repetition, and a non-shrinking blocking set over K rounds escalates.
Benefit: every termination condition is computable; dismissals are durable so a memoryless
reviewer cannot restart the loop; advisory signal is preserved without blocking; the brake
measures state, not time. Cost: a new checker, a new pull-request section, and a real
authoring burden on the reviewer, who must now say how a finding fails rather than only that
it exists. Failure mode: an agent misclassifies a reifiable finding as advisory — mitigated
by review, not by machinery, as recorded in the threat model. Evidence that would change the
assessment: repeated observation that genuine blockers are being filed advisory.

## Decision and justification

Option D is chosen. The maintainer accepted this decision in the acceptance PR above; the
invariants below are binding from that point, and this decision body is historical record from
here on. A later change to any invariant requires a new superseding ADR, not an edit to this one.

1. **Reification is the blocking criterion.** A finding blocks a pull request only if it is
   reified: expressed as a named artifact that fails mechanically — a test, a property, a
   contract assertion, or a `scripts/check-docs.sh` check. A finding that cannot be reduced
   to a failing check is **advisory**: it is recorded in the ledger with its reasoning and
   does not block the merge.

2. **Convergence is a computed conjunction.** A pull request has converged when, on its
   exact head commit, the required checks are green **and** the set of open reified findings
   is empty. "The reviewer is satisfied" is never a convergence condition and is not
   accepted as one anywhere in the pipeline.

3. **Findings are identified and dispositioned.** Every finding raised in the loop is
   recorded in a **review ledger** with a stable ID, the round that raised it, how it is
   reified (or the literal `advisory`), and exactly one state: `open`, `resolved`,
   `dismissed` or `advisory`. A `dismissed` finding carries a cited reason — a priority
   decision or an accepted ADR.

4. **A dismissal is durable.** Re-raising a finding already `dismissed` with a cited reason
   does not reopen it and does not start a new round. The dismissal is reconsidered only
   when the code in the finding's own area changes. This is the mechanism that stops a
   memoryless reviewer from restarting a settled loop.

5. **Progress is monotonic.** The blocking set is
   `{red required checks} ∪ {open reified findings}`. Each round it must shrink strictly.
   A round that leaves it the same size or larger is not progress, regardless of how much
   changed elsewhere.

6. **The brake is a state hash, not a clock.** The loop state hash is
   `sha256(normalized diff ‖ sorted open reified finding IDs ‖ sorted red required check
   names)`, where the normalized diff is the pull request's changed paths with their content
   hashes, sorted, so identical content yields an identical hash regardless of commit
   ordering. If a hash repeats a previous round's hash, or the blocking set fails to shrink
   for K consecutive rounds (K is a parameter, default 3), the loop **escalates to the
   maintainer**, naming the exact finding or check that is stuck.

7. **An escalation is recorded, never silent.** The escalation is written to
   `docs/review-stalls.md`, structured like `docs/review-debt.md` and cleared the same way.
   Continuing to iterate after an escalation without recording it is prohibited, and the
   checker enforces the record rather than trusting the agent to write it.

8. **Nothing here discharges independent review.** Convergence makes a pull request eligible
   for the maintainer's merge decision. It does not tick the independent-review boxes, does
   not substitute for `docs/prompts/0.C-prompt-revision-seguridad.md`, and leaves
   `.github/scripts/check-review-governance.js` fully in force. A pull request can converge
   and still be unmergeable for want of an independent reviewer; the two gates are
   conjunctive, and the ledger itself is part of the diff the independent reviewer examines.

This option is chosen because it is the only one of the four in which every termination
condition is computable from repository state and pull-request metadata, while the
distinction reviewers actually make — "this is broken" versus "this concerns me" — survives
instead of being flattened in either direction.

## Consequences and risks

Positive:

- A pull request ends in exactly one observable state: converged, still converging with a
  strictly smaller blocking set, or escalated with a named stuck item. Indefinite silent
  iteration ceases to be reachable.
- The pull-request checkbox `Blocking findings are resolved and conversations are closed`
  becomes falsifiable: ticking it while the ledger holds an open reified finding fails CI.
- Dismissals accumulate into a record that survives the agent that made them.
- The reviewer's burden shifts productively: saying *how* a finding fails is most of the
  work of fixing it, and produces a regression test the repository keeps.

Negative and residual:

- The ledger is authored by agents inside the loop, so misclassification remains possible.
  This is narrowed, not removed, and is documented in the threat model rather than papered
  over.
- A reviewer who cannot yet express a real defect as a failing check must file it advisory
  and open a follow-up issue. The defect is then visible but not blocking — an accepted
  trade, and the reason advisory findings are recorded rather than dropped.
- Invariant 4 keys reconsideration to "the code in the finding's area changed", which is a
  judgment at the boundary. The conservative reading — reopen when in doubt — is the correct
  one and costs only an extra round.
- Two more CI surfaces (a checker and its suite) must stay identical across repositories.
  Drift between them silently weakens one side.

Observability: the checker's message names the stuck finding or check explicitly, so the
escalation is legible from the check output alone without reading the ledger.

## Compatibility, migration, and rollback

No wire, protocol, format or persistence surface is touched. `PROTOCOL_VERSION`,
`FsBackend::FORMAT_VERSION`, `DbBackend::SCHEMA_VERSION` and every PostgreSQL migration are
unaffected, and no partially upgraded system exists to consider. The change is confined to
repository governance: `.github/`, `docs/` and `AGENTS.md`.

Migration: pull requests already open when this lands have no ledger. The checker treats an
absent ledger as round zero with no findings, so an open pull request converges on its
required checks alone until its first finding is recorded. No pull request is retroactively
invalidated.

Rollout ordering across repositories: none required, because the two repositories' checkers
are independent and the rule is per-pull-request. Landing one side first weakens only that
interval, on that side.

Rollback: revert the pull requests. The prior state is the checkbox-only gate; nothing
persists that a revert would strand, because the ledger lives in pull-request bodies and
`docs/review-stalls.md`, and both are inert once the checker is gone.

## Verification plan

Acceptance evidence is a deterministic `node --test` suite that fails before the change and
passes after it, covering, at minimum:

- **Recurrence:** a finding already `dismissed` with a cited reason, raised again, does not
  reopen and does not start a new round.
- **Stagnation:** a loop whose state hash repeats escalates; a loop whose blocking set does
  not shrink for K rounds escalates at K and not before; the escalation message names the
  exact stuck finding or check.
- **Advisory:** a finding with no failing check does not block, and its presence alone does
  not prevent convergence.
- **Convergence:** declared only with required checks green and zero open reified findings;
  a ticked "blocking findings are resolved" checkbox does not convert an open reified
  finding into a converged pull request.
- **Governance non-regression:** `scripts/check-docs.sh`, the companion rules, the graph
  check and the existing governance suite stay green in both repositories, and no statement
  in `AGENTS.md` is contradicted.

Failure injection is inherent to the suite: every case above is a negative case. Operational
verification is the checker running in CI on the pull request that introduces it.

## Equivalent decision in the other repository

This decision is canonical here, in `jsunyermias/keeplin/docs/adr/0004-review-loop-convergence.md`,
because it binds both repositories and neither is subordinate to the other for governance.
`jsunyermias/keeplin-srv` registers it under "Canonical cross-repository decisions" in
`docs/adr/README.md` and links this file rather than copying its reasoning.

No dependency pin is affected: the decision touches no `keeplin-core` surface, so the
server's pinned revision of the core crate is unchanged. The paired work is two coordinated
pull requests, one per repository, each linking the other. The checker, its suite, the
pull-request template, the prompts and `docs/review-stalls.md` are byte-identical across the
two repositories, which is what makes the rule the same rule on both sides. The workflow files
are not and cannot be identical — `keeplin-srv` runs a PostgreSQL service container and a
different test matrix — so only the two governance steps and the `checks: read` permission are
copied verbatim into it. `AGENTS.md` and the changelogs are per repository and state the same
invariants in their own terms.
