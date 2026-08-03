# `.github/workflows/review-loop-evaluator.yml` — trusted review-loop evaluator

## Purpose

This default-branch `workflow_run` workflow is the authoritative evaluator required by
[ADR 0008](../../docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md).
It correlates the completed unprivileged CI run to exactly one open pull request and reads all
pull-request content and evidence through GitHub APIs. Only a completed run whose event is
`pull_request` counts as a review round; the same CI workflow's `push` runs are ignored.

The repository variable `CI_WORKFLOW_ID` must contain the numeric database ID of this
repository's `CI` workflow. The evaluator rejects a triggering run or referenced resolution
check whose API-reported workflow ID differs from that separately configured value; it never
copies the trigger's asserted identity onto fetched checks.

## Trust boundary

There is no checkout and no shell step. The sole action is GitHub's script action pinned to a
full commit SHA. It fetches `.github/scripts/check-review-loop.js` explicitly from the
repository's API-reported default branch. Pull-request files, body text, comments, reviews,
jobs and check runs remain data; none is executed, imported or interpolated into a shell.
Malformed ledger data fails before evaluation. Comment and review references are annotated with
the API request's repository and pull-request coordinates, then those coordinates are verified
again inside the evaluator.

Check-run listing is paginated. Workflow-run identity lookup failures are represented as
unverifiable evidence and explicitly fail when the affected check is cited for resolution, rather
than aborting the adapter with an uncaught exception. A present but malformed trusted-metadata
marker also fails explicitly; absence alone retains the empty-metadata default.

The job alone receives `issues: write` and `checks: write`, used to append one digest-chained
journal comment and create the current result check. Contents, actions and pull-request access
are read-only. Forks deliberately fail closed because the policy refuses partial evidence.

Workflow concurrency is grouped by pull-request number with cancellation disabled, so two
completed runs for one pull request cannot append sibling observations from the same predecessor.
Each record also carries the originating workflow run ID and attempt. Before appending, the
evaluator refuses to write a second observation for a run/attempt pair already present in the
verified journal, making delivery retries idempotent.

## Bounded journal claim

The unkeyed digest chain detects accidental corruption and casual editing that does not rebuild
the chain. It does not authenticate a record against another repository workflow: that workflow
can use the same App identity, recompute the digests, and manufacture convergence on a history in
which no finding was ever reified. Deletion is detected only when a surviving descendant names
the missing predecessor. Terminal truncation is not detected: an actor with repository write
access can delete the newest record and the evaluator reads the shorter authentic prefix as
though the missing round never happened. Consequently, protection against reclassifying a
previously reified finding as advisory applies only while the newest surviving record still
establishes that reification; truncating behind it can make the advisory state converge. This is
ADR 0008's pinned limitation, not a durable-provenance claim.

## Repository mirror

The server repository must carry this workflow and both `.github/scripts/check-review-loop.js`
files byte-identically. Each repository configures its own numeric `CI_WORKFLOW_ID` variable, so
that value is deliberately not embedded in either workflow file. Only the server's unprivileged
CI workflow may differ in Rust/PostgreSQL setup; the trigger name `CI`, required job names,
permissions, action pin, schema and evaluator logic must remain identical.
