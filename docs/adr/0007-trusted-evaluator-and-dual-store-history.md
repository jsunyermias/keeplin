# 0007 — Trusted evaluator, verified disposal, and dual-store loop history

- Status: rejected
- Date: 2026-08-03
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — raised when [0006](0006-trusted-review-loop-history.md) proved unimplementable
  during its own implementation, recorded as finding F-017 on
  [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198)
- Acceptance PR: link once the ADR is accepted
- Supersedes: [0006](0006-trusted-review-loop-history.md)
- Superseded by: [0008](0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md)

Rejected on 2026-08-03 by two independent reviews, before acceptance — the ordering this ADR
itself asked for. Its load-bearing claim, that check runs cannot be deleted through the API, is
false in effect: `PATCH` rewrites output for any token with `checks: write`, check data is
archived at 400 days, same-name runs past 1,000 per suite are auto-deleted, and deleting a
workflow run removes its job check runs. The second review additionally found the counter's
source unspecified, so a naive implementation would recompute it from a truncated journal and
reproduce 0006's hole while the specified test still passed. ADR 0008 carries the replacement.

## Context and problem

ADR 0004 gave the review loop a computed convergence condition. ADR 0005 tried to move its
history out of the editable pull-request body and was rejected for never establishing a trusted
writer. ADR 0006 was accepted, and then failed at implementation.

The failure is worth stating exactly, because it is the third design to break on the same rock.
ADR 0006 made the history an append-only journal of App-authored comments, digest-chained so
each record names its predecessor. That detects editing a record, and deleting a record from the
middle, because a surviving successor still points at what is missing. It does not detect
deleting the **newest** record, or all of them: a chain of three with the third removed is a
perfectly valid chain of two, nothing survives that references the third, and GitHub does not
expose deleted comments. Terminal deletion is indistinguishable from a round that has not
happened yet — which is precisely the erasure the stagnation brake exists to survive. ADR 0006's
own text says that if deletion cannot be detected this way, the external append-only option must
be chosen; the implementing reviewer held it to that and stopped without writing code.

What that episode also revealed is that ADR 0006 bundled three independent properties, only one
of which was blocked:

1. **The evaluator must not be modifiable by the pull request it judges.** Today the convergence
   check runs code checked out from the pull-request head, so an agent can edit its own judge.
2. **A finding's disposal must be authorized.** Today anything may be marked `resolved` with no
   evidence at all — nothing verifies that the named check exists, let alone passes.
3. **The history must resist erasure.**

Properties 1 and 2 never depended on the journal. They were held hostage to the part that broke.

## Forces and requirements

- The evaluator's code and workflow definition must not come from the pull-request head, and it
  must never check out or execute head content.
- A reified finding must not reach `resolved` or `dismissed` on an author's say-so.
- History must survive force-push, rebase and reset, because commit-attached records do not.
- History must resist deletion **including of its most recent record**, which is where ADR 0006
  failed.
- Every guarantee this decision claims must be one the implementation can demonstrate. Three
  designs have now overclaimed; a smaller honest claim is worth more than a larger unverifiable
  one.
- No new operational dependency, credential store or paid service.
- Both repositories behave identically.

## Threat model

- **Assets:** the convergence claim, the record of open findings, and the stagnation brake.
- **Trust boundary — stated precisely, because ADR 0006 was vague here and it mattered:** the
  boundary is *what the pull request can influence*. A pull request can rewrite its own body,
  its commits, its branch, and any file in its diff — including, today, the evaluator itself.
- **Adversary:** an agent inside the loop optimizing for a green check. Not a human attacker.
- **Explicitly outside the model:** anyone holding repository administrator credentials directly.
  In a single-maintainer repository an agent given the maintainer's own token can delete
  comments, rewrite protection and forge anything; no arrangement of GitHub primitives defends
  against that, and pretending otherwise is the overclaiming this ADR exists to stop. The
  defended property is that **nothing reachable through the pull request** can erase history or
  forge convergence.
- **Retained capability:** classifying a reifiable defect as advisory, or writing a thin
  dismissal reason. Semantic judgment stays with independent review; this is a non-goal, as in
  ADR 0004.

## Options considered

**A. External append-only store.** ADR 0006's stated fallback. Strongest retention and
independence. Cost: a service, credentials, availability and operational burden on a pre-release
project that currently needs none of those. Rejected as disproportionate for the threat actually
being defended against, and recorded as the escalation path if this decision proves insufficient.

**B. Anchor history on the default branch.** The evaluator commits an observation counter to a
file or ref on `main`. Durable and protected. Cost: continuous-integration writes to the default
branch on every evaluation of every pull request, which is heavy churn and a broader permission
than the alternative. Rejected on cost, not on soundness.

**C. Scope the claim down.** Keep the comment journal, accept that terminal deletion is
invisible, and say so in `AGENTS.md`. Zero cost and honest. Rejected only because option D
appears to close the gap at comparable cost — but this remains the correct outcome if D fails
review, and it is a legitimate end state rather than a failure.

**D. Two stores whose blind spots do not overlap.** The journal stays as App-authored comments,
which survive topology rewrites. The evaluator additionally publishes the observation count in
the output of the current head's check run. Check runs cannot be deleted through the GitHub API
at all — only superseded — so a journal shorter than the count the check run reports is proof of
deletion, including terminal deletion. Comments cover what check runs cannot (rebase); check runs
cover what comments cannot (erasure). Chosen, subject to the verification below actually
demonstrating it.

## Decision and justification

ADR 0007 supersedes ADR 0006 in full. Its properties 1 and 2 are carried over unchanged in
substance; its journal design is replaced.

1. **The authoritative evaluator is a default-branch `workflow_run` workflow.** Its definition
   comes from the default branch, never the pull-request head. It reads the pull request, its
   files, its ledger and the conclusions of the completed unprivileged CI run through APIs, and
   never checks out or executes head content. Unprivileged CI continues to run tests read-only
   and holds no write token.
2. **A reified finding reaches `resolved` or `dismissed` only against a verified reference:** a
   GitHub review or comment whose author association is `MEMBER`, `OWNER` or `COLLABORATOR` and
   whose author is not the pull-request author. The evaluator records its immutable ID, author,
   and a digest of its body, and re-verifies all three on every evaluation; a mismatched digest
   or a dismissed review returns the finding to `open`. `resolved` additionally names the check
   and the successful run that prove the fix. Tombstones and the genesis record of a pull request
   predating this decision require the same verified reference.
3. **History is stored twice, and the two stores are checked against each other.** The journal is
   the digest-chained sequence of App-authored comments; the count of observations is also
   written to the output of the check run the evaluator publishes on the current head. If the
   journal is shorter than the count, or the chain is broken, or a record's digest does not
   match, the result is `history-unverifiable` and the loop fails closed. This is what closes
   terminal deletion, which ADR 0006 could not.
4. **History belongs to the pull request number, not to its commits.** Force-push, rebase and
   reset append rather than reset. A rerun is idempotent by run ID and attempt.
5. **Only explicitly named required jobs count**, each needing positive `success` evidence.
   Skipped, neutral, absent and unknown are not green.
6. **Fork pull requests cannot produce a trusted evaluation** and therefore do not converge. This
   is stated as a limitation, not worked around.
7. **The claim is bounded by the trust boundary above.** Convergence means "nothing reachable
   through this pull request contradicts it", not "no actor could have forged this".

## Consequences and risks

Positive: the evaluator stops being editable by what it judges; disposal stops being an
assertion; and erasure becomes detectable in both directions it can occur.

Negative and residual:

- The check-run counter is only as good as the claim that check runs cannot be deleted. If that
  is wrong, or if a token reachable from the pull request can update that check run's output,
  option D collapses into option C. **This is the assumption the review of this ADR must attack
  hardest**, because it is the load-bearing one and it is mine, not GitHub's documentation
  speaking.
- Two stores mean two ways to be inconsistent. Every inconsistency must fail closed, which will
  occasionally block a pull request for an infrastructural reason rather than a real finding.
- Elevated permissions on the trusted job, never exposed to a checkout, shell, head dependency,
  or any action other than GitHub's own, pinned to a full commit SHA.
- Semantic gaming remains open, as in ADR 0004.
- An administrator credential defeats everything here, by design and by admission.

## Compatibility, migration, and rollback

No wire, protocol, format or persistence surface is touched; the change is confined to
`.github/` and `docs/`. Pull requests open when this lands have no journal; their history begins
at the first evaluation after it, recorded as a genesis observation requiring a verified
maintainer reference. Rollback is reverting the pull requests; the journal comments and check
runs left behind are inert once nothing reads them.

## Verification plan

Deterministic tests, each failing before the change:

- **F-002 / terminal deletion:** a journal shorter than the count published in the check-run
  output yields `history-unverifiable`, never round zero. The existing deliberately-red test
  must turn green without being edited.
- **F-008 / trusted writer:** a record whose App, workflow, repository or schema does not match
  configuration is refused, so head-supplied or forged history cannot enter.
- **F-009 / topology:** history survives rebase and force-push, or fails closed; it never
  silently resets.
- **F-013 / disposal:** a disposal whose reference is missing, unauthorized, authored by the
  pull-request author, digest-mismatched, or a dismissed review leaves the finding open.
- **Required checks:** skipped, neutral, absent and unknown do not converge.
- ADR 0004's inherited semantics — reification, advisory, durable dismissal, monotonic progress,
  the stagnation brake — continue to pass unchanged.

**Before acceptance**, this ADR is to be reviewed by at least two model families other than its
author, specifically attacking the check-run deletion assumption in Consequences. ADR 0006 was
accepted and found unimplementable within hours; the ordering is deliberate.

## Equivalent decision in the other repository

Canonical here in `jsunyermias/keeplin`; `jsunyermias/keeplin-srv` links it and carries the
byte-identical evaluator, suite and trusted workflow, differing only where its CI genuinely
differs (PostgreSQL service, workspace test matrix). No `keeplin-core` surface or pin is
affected.
