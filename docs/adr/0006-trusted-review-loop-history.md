# 0006 — Trusted review-loop history

- Status: accepted
- Date: 2026-08-03
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — maintainer-directed follow-up to the round-2 review of
  [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198)
- Acceptance PR: [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198), with [keeplin-srv#104](https://github.com/jsunyermias/keeplin-srv/pull/104) as its companion
- Supersedes: [0004](0004-review-loop-convergence.md)
- Superseded by: none

## Context and problem

ADR 0004 defines useful convergence semantics, but its implementation reads authoritative
round history from the pull-request body. An author can delete that history and reset the
stagnation brake. Rejected ADR 0005 proposed check-run output without fully binding records to
a trusted producer or defining history through topology rewrites and retention.

This decision replaces ADR 0004 as one coherent decision. GitHub's default-branch
`workflow_run` evaluator is the simplest repository-native trust boundary: its workflow comes
from the default branch, not the pull-request head, and it can evaluate completed CI without
executing untrusted pull-request code. Check runs attached only to commits cannot retain one
history across force-push or rebase, so this proposal uses authenticated pull-request comments
as the journal and check runs only to present the current-head result.

## Forces and requirements

- The writer's code and workflow cannot come from the pull-request head.
- Records identify the repository, pull request, head SHA, exact workflow/App and schema.
- History remains defined across force-push, rebase, reset, rerun and ordinary retention.
- Unreachable, ambiguous or unauthenticated prior history fails closed.
- Finding-ID mistakes have an auditable correction that never deletes history.
- Only explicitly named required jobs count; each needs positive `success` evidence.
- Current finding-state transitions cannot be forged by a pull-request author.
- Fork behavior and all capabilities granted by `checks: write` are explicit.
- Both repositories carry byte-identical evaluator code and tests.

## Threat model

Assets are the convergence claim, historical round sequence and escalation brake. The trust
boundary separates authors and head code from the default-branch workflow and its GitHub App
token. An author may rewrite commits and Markdown, trigger reruns and submit malicious files,
but must not execute code with the writer token, replace the writer, erase observations or forge
its identity.

The trusted job may read pull-request metadata, comments and workflow results; append a journal
comment; and create or update the single current-head check run. `checks: write` can in fact
create, rerequest and update any repository check run, including its name, conclusion, status,
output and annotations. This broad capability is the principal risk. It is scoped to this job
and never exposed to a checkout, shell, head dependency or third-party action other than
GitHub's maintained script action pinned to a full commit SHA. `issues: write`, required for PR
comments, is limited to appending the journal. Compromise of the default-branch workflow or
action pin can forge history and checks. Judging finding classifications is a non-goal.

## Options considered

**Keep body history.** Simple and fork-friendly, but the constrained party controls the brake.
Rejected because F-002 remains reproducible.

**Commit-attached check-run history.** Repository-native, but rebase or force-push makes old
topology undiscoverable from the new head, reruns are ambiguous, and retention can remove
evidence. Rejected as the journal; retained only for the current result.

**Default-branch `workflow_run` plus authenticated comment journal.** The definition is not
loaded from the head, receives its token only after unprivileged CI completes, and comments
survive topology rewrites. Costs are elevated permissions, correlation logic and reliance on
GitHub retention. Chosen.

**External append-only service.** Stronger retention and signing, but adds credentials,
operations, availability and cost. It becomes preferable if GitHub cannot meet retention or
repository policy forbids the writer permissions.

## Decision and justification

ADR 0006 supersedes ADR 0004 in full. The default-branch `workflow_run` evaluator
is the sole authoritative writer. Unprivileged CI runs tests read-only. The evaluator fetches
the pull request, files, ledger and exact completed run through APIs; it never checks out or
executes head content.

Each observation is an immutable comment by the repository's GitHub Actions App. Canonical JSON
schema `keeplin.review-loop/v1` contains repository ID, PR number, head SHA, workflow file path
and immutable workflow ID, run ID and attempt, App slug and installation ID, observation number,
canonical state hash, blocking count, finding IDs, required job names/results, prior-record
digest and timestamp. The evaluator accepts only comments whose API author association, App,
repository, workflow and schema match configuration. The digest chain detects deletion,
reordering and substitution. If any expected predecessor is unreachable or invalid, writing and
convergence stop.

History belongs to the PR number, not ancestry. Force-push, rebase or reset adds an observation
for the new head without discarding old ones. A rerun is distinct by run ID and attempt and is
idempotent. Comments must be retained for the PR lifetime plus the review-debt retention period.
Pagination, retention, deletion, permission, identity or digest-chain failure produces
`history-unverifiable` and fails closed. Recovery restores evidence or opens a replacement PR
with an explicit linked maintainer reset; there is no silent genesis.

Finding IDs never disappear. A mistake gets a tombstone naming the old ID, replacement ID or
`none`, reason, maintainer identity and source link. The old ID remains reserved; only a
maintainer-authored correction accepted by the trusted evaluator changes the active projection.

"Maintainer-authored" is not a description but the same verified reference required for
disposal below: a tombstone, and the genesis record of a pull request that predates this
decision, are authorized only by a GitHub review or comment whose author association is
`MEMBER`, `OWNER` or `COLLABORATOR` and whose author is not the pull-request author, recorded
by digest as in the disposal rule. Leaving the authorization channel unspecified would have
left the one operation that rewrites the projection as the only unauthenticated one.

The author-editable ledger is input, never authority for disposal. A reified finding may move
from `open` to `resolved` or `dismissed` only when its row names a GitHub review or comment ID
whose API author association is `MEMBER`, `OWNER`, or `COLLABORATOR` and whose author is not the
pull-request author. The trusted evaluator fetches that object, verifies its repository and pull
request, and records its immutable database ID, author **and a digest of the referenced body**
in the observation, because the reference is a mutable object: a comment can be edited and a
review can later be dismissed, and an ID alone would keep validating a disposal whose stated
reason no longer exists. At every re-evaluation the evaluator refetches the reference and
requires the digest to match and, for a review, its state to be active; a mismatched digest or
a dismissed review returns the finding to `open` and reports `history-unverifiable` rather than
silently preserving the disposal. `resolved` also
names the mechanical check or assertion and the successful run ID and attempt that prove the
fix; `dismissed` names the accepted ADR or priority decision in the verified maintainer
reference. Missing, deleted, ambiguous or unauthorized evidence leaves the finding open and
fails closed.

Fork PRs are supported because `workflow_run` executes from the base repository after read-only
CI. The trusted job never checks out, sources, imports or executes fork content. If a run cannot
be correlated to exactly one open PR and head SHA, it fails closed without writing.

Only `Check, Test & Lint` and `Knowledge graph up to date` are required. Their completed job
results must equal `success`; skipped, neutral, absent and unknown are non-green. Optional checks
are irrelevant unless a later accepted ADR names them. A separate early governance-test job is
not adopted because it adds another required identity without strengthening the trust boundary.

Those names are repository policy, not evidence that GitHub branch protection has the same
configuration. Before declaring convergence, the trusted evaluator queries the branch-protection
API for the pull request's base branch and compares its required status-check contexts byte for
byte with the ADR-configured set plus the evaluator's own check. Missing API permission,
rulesets whose effective requirements cannot be resolved, or any mismatch fails closed. This is
the named follow-up requirement for F-011; until ADR 0006 is accepted and implemented, ADR 0004's
job can prove only `converge.needs` and must not claim all protected checks are green.

## Consequences and risks

History survives body edits and rewritten topology, and records identify an exact producer and
schema. Forks participate without giving their code a write token. Costs are a privileged job,
comment traffic and more complex recovery. Administrators can delete comments and platform
failure halts convergence. `checks: write` is broader than intended; workflow compromise can
forge unrelated checks. Audit logging must flag journal deletion, identity/schema rejection and
writes outside the named check.

GitHub comments are not cryptographically immutable. Missing history is made observable and
fail-closed. If deletion cannot be detected with the digest chain and sequence anchor, acceptance
must choose the external append-only option instead.

## Compatibility, migration, and rollback

No application wire, note format or database migration changes. Rollout is paired: accept the
ADR, then land byte-identical evaluator/tests and coordinated workflows in both repositories.
Existing PRs require a maintainer-authorized genesis linking their last body round; its absence
fails closed. A partially upgraded repository remains governed by ADR 0004 and cannot claim ADR
0006 guarantees.

Rollback requires a new accepted ADR; journal comments remain inert evidence and are not
deleted. Loss of writer permissions or GitHub availability blocks convergence rather than
falling back to body history.

## Verification plan

- Unit tests reject skipped, neutral, absent and unknown required results and ignore optional
  checks; prove canonical framing injective; distinguish `F-001` from `F-0010`; and cover
  CommonMark `\|`, `\\|`, `\\\|` plus a terminal literal backslash.
- API-fixture integration tests cover rerun, force-push, rebase, reset, fork correlation,
  pagination, missing predecessors, wrong App/workflow/schema, tombstones, forged finding-state
  transitions, a disposal reference whose body digest no longer matches, a disposal reference
  that is a dismissed review, and a tombstone or genesis lacking a verified maintainer reference
  transitions, missing or author-owned resolution references, branch-protection mismatch and
  permission denial.
- A canary proves no head code runs with `issues: write` or `checks: write` and only the trusted
  job has them. Cross-repository tests compare JavaScript and suites byte-for-byte.
- Operational drills delete and make unreachable journal history; both must return
  `history-unverifiable`. The red F-002 test turns green only after acceptance and implementation;
  F-008 and F-009 gain identity and topology/retention tests at that time.

## Equivalent decision in the other repository

This canonical ADR lives in `jsunyermias/keeplin`. `keeplin-srv` links it and carries a paired
PR with byte-identical JavaScript and tests. No `keeplin-core` pin changes. Workflow files differ
only for repository CI; the trusted event, permissions, action pins, schema and restrictions are
equivalent and mechanically verified.
