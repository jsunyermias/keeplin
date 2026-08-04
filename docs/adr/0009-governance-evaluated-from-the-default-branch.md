# 0009 — Review governance is evaluated from the default branch

- Status: superseded
- Date: 2026-08-03
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — F-025 from the round-9 review of
  [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198)
- Acceptance PR: [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198), with
  [keeplin-srv#104](https://github.com/jsunyermias/keeplin-srv/pull/104) as its companion
- Supersedes: none. Extends [0008](0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md)
- Superseded by: [0012](0012-default-branch-review-governance.md) completely

## Context and problem

`AGENTS.md` and `.github/scripts/README.md` state that the two automated gates are
**conjunctive**: convergence is a floor beneath independent review, never a substitute for it,
and a converged pull request with no independent reviewer is still unmergeable.

Round 9 of independent review established that this is true as policy and false as mechanism.

`check-review-governance.js` — the check that requires either a recorded independent review with
evidence, or a complete maintainer waiver naming its pull request in `docs/review-debt.md` — runs
as a step inside `ci.yml`. That workflow is defined by the pull request's own head. A head can
delete the step, weaken its condition, or leave the step name in place and gut the script it
calls. The job still completes as `Check, Test & Lint` with conclusion `success`.

The trusted evaluator then reads that job through the API and requires only that the two named
jobs report `success`. It has no evidence that governance ran at all. It can therefore publish a
successful `Review loop converged` check for a pull request whose independent-review gate was
removed by the same commit it is evaluating.

This is the same shape as the four defects this change has already corrected: an unconditional
promise that in fact holds only if something else holds. Here the unstated condition is "the head
did not touch the governance step".

## Decision

Move review-governance evaluation into the trusted default-branch evaluator.

1. `review-loop-evaluator.yml` loads `check-review-governance.js` from the **default branch**,
   exactly as it already loads `check-review-loop.js`, and evaluates it against API data: the
   pull-request body, its changed files, and `docs/review-debt.md` as of the evaluated head.
2. Convergence requires the governance result to pass. The two gates become conjunctive in the
   evaluator, where the head cannot reach the *evaluating code*. It can still reach the
   *evidence* — see "What this bounds" below, which is the load-bearing limit of this decision.
3. The `ci.yml` step remains, unchanged in behaviour, as a fast signal to the author. It is
   explicitly **not** the gate: it fails early and locally, the evaluator decides.
4. The evaluator's message names which gate failed, so a red `Review loop converged` is
   diagnosable without opening the run.

### What this bounds

This decision moves the *evaluating code* out of the head's reach. It does **not** move the
*evidence* out of the head's reach, and the difference decides what may be claimed.

Governance reads three head-controlled inputs: the pull-request body, the list of changed files,
and `docs/review-debt.md` at the evaluated commit. A pull request can therefore still write its
own maintainer waiver into `docs/review-debt.md`, name itself in it, fill the waiver fields in
its own body, and satisfy the waiver path — with the evaluator running default-branch code the
whole time. What the evaluator gains is that the *rules* cannot be edited or deleted by the
change being judged; what it does not gain is any proof that the *facts* asserted to those rules
are true.

So the accurate claim after this ADR is: **a head can no longer remove the governance rules, and
can still author the evidence they read.** Closing the second half is a separate decision about
authenticating waivers and review records — plausibly the same verified-authorization machinery
0008 already uses for finding disposal — and this ADR does not make it. Anyone implementing this
must not restate the first half as though it were both.

### What this deliberately does not do

It does not verify that the *step inside CI* ran. That was considered and rejected: the step's
name is head-controlled, so a head can keep the name and empty the script. Requiring a
successfully-named step would prove that something called `Check pull-request review governance`
reported success — not that governance was evaluated. It is a check that looks like evidence and
is not, which is worse than no check.

It does not change what governance *requires*. The reviewed path and the waiver path are as
0004 left them; only the place they are evaluated changes.

It does not change the bounded history claim of 0008. Terminal journal truncation remains
undetected, and this ADR makes no claim about it.

## Consequences

- The head can no longer weaken the independent-review *rules* by editing its own workflow.
  Removing the `ci.yml` step becomes a diff a reviewer sees, not a bypass. It can still author
  the evidence those rules read, per "What this bounds".
- `docs/review-debt.md` must be read at the evaluated head through the API rather than from a
  checkout, because the evaluator never checks out head content. The waiver path therefore
  verifies the same file the pull request actually changes.
- The evaluator gains a second reason to fail, so `Review loop converged` becomes the single
  required check that speaks for both gates. Branch protection does not need a new entry.
- Fork pull requests continue to fail closed, as under 0008.
- A pull request that legitimately deletes the CI step — for example when this ADR is
  implemented — is unaffected, because the gate no longer lives there.

## Alternatives considered

- **Bound the claim instead.** State that the conjunction is upheld by policy and branch
  protection, not by the evaluator, and record F-025 as advisory with a follow-up issue. Honest
  and free. Rejected as the primary path because it would be the sixth promise bounded rather
  than fixed in one change, and unlike terminal truncation this one has a mechanical fix
  available at moderate cost.
- **Require a successful governance step in the CI run.** Rejected above: head-controlled name,
  no proof of content.
- **Require an approving GitHub review through branch protection alone.** Complementary, not a
  substitute: it enforces that someone approved, not that the repository's own evidence and
  waiver rules were satisfied, and it is configured outside the repository where neither script
  can verify it.

## Status of implementation

None. This ADR is `accepted`, but its implementation has not landed. Implementation belongs in a
dedicated pull request that links the accepted ADR.
