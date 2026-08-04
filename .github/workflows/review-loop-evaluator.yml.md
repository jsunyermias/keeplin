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
The evaluator's journal helper escapes HTML comment delimiters inside serialized record fields
without changing their decoded values and neutralizes marker text in the appended human-readable
message, so pull-request data cannot terminate the payload comment or create a second parseable
marker inside the persisted App comment. This closes the wedge for records written after this
change. A record written before this change that already contains a raw `-->` in its serialized
JSON still fails closed; operators must use the terminal-record recovery procedure in
[`docs/review-stalls.md`](../../docs/review-stalls.md#recovering-a-terminal-malformed-journal-record).
Its default-branch recovery verifier requires an authentic terminal candidate, continuity with
the surviving head, no unaccounted frame suffix, chronologically ordered API input with
authoritative nested App attribution, and a current ledger identical in every ledger-representable
finding field before the operator may delete the malformed comment. Evaluator-only projection
diagnostics are not Markdown ledger fields.
For an unchanged `resolved` disposition, the recorded authorization reference ID, author and body
digest remain pinned while the check-run ID and name are read from the current ledger and proved
again against the current evaluator run.

The job alone receives `issues: write` and `checks: write`, used to append one digest-chained
journal comment and create the current result check. Contents, actions and pull-request access
are read-only. Forks deliberately fail closed because the policy refuses partial evidence.

Workflow concurrency is grouped by pull-request number with cancellation disabled and
`queue: max`, so delivered runs for one pull request cannot append sibling observations from the
same predecessor and GitHub retains up to 100 pending runs rather than only the newest pending
run. The queue remains bounded: once those 100 pending slots are full, additional runs are
canceled, so the journal still cannot claim that every completion is retained under unbounded
load. Each record carries the originating workflow run ID and attempt. Before appending, the
evaluator refuses to write a second observation for a run/attempt pair already present in the
verified journal. The retry still publishes the current success or failure check, and a blocked
retry calls `core.setFailed`, so idempotency cannot turn a blocked evaluation green.

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
