# 0012 — Evaluate review governance from the default branch

- Status: accepted
- Date: 2026-08-03
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — F-025 from the round-9 review of
  [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198)
- Acceptance PR: [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198), with
  [keeplin-srv#104](https://github.com/jsunyermias/keeplin-srv/pull/104) as its companion
- Supersedes: [0009](0009-governance-evaluated-from-the-default-branch.md) completely
- Superseded by: none

## Context and problem

Keeplin requires two conjunctive outcomes before merge: the mechanical review loop converges,
and an independent reviewer has examined the objective and diff, or the maintainer has recorded
an explicit waiver and review debt. `check-review-governance.js` evaluates the second outcome.

Today that script runs inside `.github/workflows/ci.yml`. A pull-request head controls both the
workflow definition and the script it invokes. It can delete the step, weaken its condition, or
leave the step name while replacing its behavior. The `Check, Test & Lint` job may still report
`success`.

The trusted `workflow_run` evaluator loads `check-review-loop.js` from the default branch, but it
currently reads only the successful job result. It has no evidence that the head-controlled
governance code ran. It can therefore publish a successful `Review loop converged` check when the
change being judged removed the independent-review rules.

ADR 0009 accepted the right architectural direction but is incomplete under the repository's
ADR contract: it omits compatibility, migration, failure modes, verification, and rollback; its
body contradicts its accepted lifecycle metadata; and it gives two incompatible dispositions for
the `ci.yml` step. Accepted ADR bodies are historical records, so this ADR supersedes 0009 in full
instead of rewriting its decision.

## Forces and requirements

- The rules that decide independent-review governance must come from the default branch, not the
  pull-request head they judge.
- Convergence and review governance remain conjunctive; a successful loop result alone never
  supplies independent review.
- The trusted evaluator must treat head content only as API-fetched data and must not check out,
  execute, import, or shell-interpolate it.
- The `ci.yml` governance step must have one explicit role and disposition.
- Rules and evidence must not be conflated: protecting evaluator code does not authenticate
  head-authored waiver or review-debt claims.
- Forks, malformed evidence, missing files, API failures, and ambiguous pull-request correlation
  fail closed.
- Both repositories use byte-identical evaluator and governance logic, with only genuinely
  repository-specific CI setup differing.
- No Rust wire, format, persistence, or `keeplin-core` dependency surface changes.
- Because this ADR is proposed, no dependent workflow implementation may land before maintainer
  acceptance.

## Threat model

The protected asset is enforcement of the repository's independent-review-or-maintainer-waiver
rules for the exact pull request and head commit being evaluated.

The adversary is a pull-request author or agent able to change any file on the head branch,
including `.github/workflows/ci.yml`, `.github/scripts/check-review-governance.js`, the pull-
request body, changed files, and `docs/review-debt.md`. The adversary may cause the unprivileged
head workflow to report a successful job after removing its local governance step.

The default branch's `workflow_run` definition and default-branch evaluator source are trusted
for the rules they execute. GitHub API responses and repository identity configuration are the
boundary through which the evaluator obtains head data. The decision does not trust a step name
or success result supplied by head-controlled CI as proof of governance execution.

The decision deliberately does not authenticate the facts asserted in head-controlled evidence.
A head can still author its own waiver fields and a matching `docs/review-debt.md` entry. It also
does not defend against maintainers changing the default branch, compromised GitHub
infrastructure, or credentials outside the pull-request boundary.

## Options considered

### Evaluate governance in the default-branch evaluator and retain the CI step as a fast signal

Load governance code from the default branch, evaluate it against API-fetched head evidence, and
make its success a condition of the authoritative result. Keep the current `ci.yml` step for
early author feedback, but assign it no gate authority. This duplicates evaluation intentionally:
one result is fast and head-controlled, the other authoritative and default-branch controlled.
The cost is adapter complexity and the need to diagnose two signals without confusing them.

### Remove the CI step when the trusted evaluator is implemented

There would be only one signal and no duplicate computation. Rejected because authors would wait
for the completed `workflow_run` before seeing basic template or waiver errors. It also creates an
avoidable transition in existing CI behavior. Under the chosen option, deleting or gutting the
step is a policy regression, not a legitimate implementation of this ADR.

### Require a successful named governance step from head CI

Simple, but the head controls both the name and implementation. A successful step proves only
that something with that name reported success. Rejected as circular evidence.

### Rely only on branch protection's approving-review rule

Useful as an external defense, but it does not evaluate Keeplin's recorded implementer,
independent-review assertions, evidence link, waiver fields, or review-debt entry. Branch
protection also lives outside the repository and cannot be verified by these scripts. It remains
complementary.

### Bound the policy claim and leave governance in head CI

Accurately document that the head can remove the gate. Operationally cheapest, but leaves a
mechanically fixable bypass in a required governance rule. Rejected by the maintainer in favor
of default-branch evaluation.

## Decision and justification

If accepted, review governance is evaluated by default-branch code inside the trusted
`workflow_run` evaluator.

1. The workflow fetches `check-review-governance.js` and its required evaluator support from the
   API-reported default branch, alongside `check-review-loop.js`. It never loads executable code
   from the pull-request head.
2. The adapter fetches the pull-request body, exact changed-file list, and
   `docs/review-debt.md` at the evaluated head through GitHub APIs and passes those values as data
   to the default-branch governance function.
3. `Review loop converged` succeeds only when both the review-loop result and the governance
   result pass. Its output identifies which gate failed.
4. The existing `ci.yml` governance step remains in both repositories, unchanged in behavior,
   as a nonauthoritative fast signal to the author. It is not evidence consumed by the trusted
   gate. Deleting, weakening, or gutting it is outside this decision and is a review finding, not
   a valid cleanup when implementing the authoritative gate.
5. Forks and incomplete, malformed, unreachable, or ambiguously correlated inputs fail closed.

### What this bounds

Moving the rules out of the head's reach does not move the evidence they read.

The pull-request body, changed-file list, and `docs/review-debt.md` at the evaluated commit remain
head-controlled. A pull request can still write its own maintainer waiver into
`docs/review-debt.md`, name itself, fill the waiver fields in its own body, and satisfy the waiver
path while the evaluator runs unchanged default-branch code.

The resulting claim is precise: **a head cannot edit or delete the governance rules used by the
authoritative evaluator, and it can still author the evidence those rules read.** This ADR does
not prove that the asserted review or waiver facts are true. Authenticating that evidence is a
separate decision and must not be inferred from default-branch rule isolation.

This option best satisfies the forces because it closes the code-substitution bypass without
overstating the evidence boundary and preserves rapid local feedback with an explicitly
nonauthoritative role.

## Consequences and risks

The positive consequence is that a pull-request head can no longer obtain authoritative success
by deleting or weakening its governance evaluator. One required check reports the conjunction of
loop convergence and governance, and its diagnostic output distinguishes their failures.

The principal residual risk is head-authored evidence. A fabricated waiver and matching review-
debt entry can satisfy unchanged rules; moving code alone supplies no provenance. Other failure
modes include:

- GitHub API failure, pagination error, missing `docs/review-debt.md`, malformed pull-request
  body, or multiple matching pull requests. Each must fail closed with a diagnostic result.
- Default-branch governance and the retained CI fast signal may temporarily disagree during a
  rules update. The default-branch evaluator is authoritative; the discrepancy is observable and
  corrected by coordinated rollout.
- A workflow adapter could accidentally execute head data or fetch evaluator source from the
  head. Structural tests must reject checkout, shell execution, and head-source references.
- A result could be attached to the wrong head or workflow run. Repository, pull-request, head
  SHA, workflow ID, run event, and App identity remain explicit bindings.
- Fork pull requests remain unable to satisfy the trusted write path and fail closed.

The retained `ci.yml` step adds modest duplicate execution and may be bypassed by a head, but such
a bypass cannot satisfy the authoritative evaluator merely by making the CI job green.

## Compatibility, migration, and rollback

No Rust API, wire protocol, stored note format, database schema, migration, or protocol version
changes. No `keeplin-core` pin changes. The affected surfaces are GitHub workflows, dependency-
free JavaScript, policy documents, and tests.

Rollout is coordinated across both repositories after maintainer acceptance:

1. Land byte-identical default-branch governance evaluator code and tests.
2. Land the workflow adapter changes while retaining the existing `ci.yml` fast-signal step.
3. Configure or verify any repository variables used to identify the authoritative CI workflow.
4. Make the combined evaluator check required only after the exact default-branch commit is
   available and observed working in each repository.

Open pull requests require no ledger, wire, or data migration. Their next completed pull-request
CI run is evaluated under the new default-branch rules. A partially upgraded pair of repositories
has different governance enforcement but no client/server compatibility break; coordinated
rollout minimizes that policy window.

Rollback is reverting the default-branch workflow and evaluator commits and restoring the prior
required-check configuration. The retained `ci.yml` signal continues to provide the old
head-controlled check during rollback. Any authoritative check runs already created remain
historical GitHub records and do not alter repository data. Rollback reopens F-025 and must be
recorded as a known governance regression, not represented as equivalent protection.

## Verification plan

- A fixture whose head removes or replaces the `ci.yml` governance step still fails the
  authoritative evaluator when governance evidence is incomplete.
- A fixture with valid independent-review evidence passes both gates; valid maintainer-waiver
  evidence follows the existing governance rules and passes, subject to the documented
  head-authored-evidence bound.
- Missing, malformed, or unreachable review-debt content; ambiguous pull-request association;
  fork origin; wrong head SHA; wrong workflow event; and API lookup failure each fail closed.
- Structural workflow tests prove that governance source is fetched from
  `repository.default_branch`, that no head checkout or shell interpolation exists, and that the
  trusted workflow invokes only fully pinned actions.
- A structural CI test proves the existing `check-review-governance.js` step remains as the
  nonauthoritative fast signal with unchanged behavior.
- The authoritative output separately identifies review-loop and governance failures.
- Existing review-loop, governance, bounded-history, and terminal-truncation tests continue to
  pass.
- `./scripts/check-docs.sh`, `node --test .github/scripts/*.test.js`, and
  `python3 -m unittest discover -s scripts/tests -p 'test_*.py'` pass in both repositories.
- `cmp` confirms that shared governance implementation, workflows, companions, and tests are
  byte-identical across repositories.

These checks demonstrate rule isolation and failure behavior. They do not claim to authenticate
the head-authored waiver evidence explicitly left outside this decision.

## Equivalent decision in the other repository

This file in `jsunyermias/keeplin` is canonical. The `jsunyermias/keeplin-srv` ADR registry links
keeplin ADR 0012 and records the same proposed status and server impact. After acceptance, the
paired implementation would land through [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198)
and [keeplin-srv#104](https://github.com/jsunyermias/keeplin-srv/pull/104), with byte-identical
governance scripts, evaluator workflow, and contract tests. Repository-specific Rust/PostgreSQL
CI setup may differ, but the retained governance fast signal has the same behavior. No immutable
`keeplin-core` dependency pin or shared protocol version changes.
