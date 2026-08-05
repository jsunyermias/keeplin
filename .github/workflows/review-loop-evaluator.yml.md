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

Before evaluating any directive, the adapter enumerates repository collaborators with the same
workflow `GITHUB_TOKEN`, `affiliation=all`, and explicit traversal of every `Link: rel="next"`.
The resulting exhaustive set is supplied to every authorization verification. Unreadable,
rate-limited, forbidden, failed, or non-exhaustive enumeration is unknown and refuses each
disposition; it is never interpreted as zero or one principal. Owner self-authorization is
available only when the set contains no different login, regardless of whether the endpoint
includes the owner itself.

Job and check-run listings are paginated through Octokit's normalized page arrays. An empty page
is legitimate and contributes no items. A non-array page or a non-object job or check-run item is
malformed API evidence, so the adapter fails explicitly instead of treating the missing evidence
as an empty green result. Workflow-run identity lookup failures are represented as unverifiable
evidence and explicitly fail when the affected check is cited for resolution, rather than
aborting the adapter with an uncaught exception. A present but malformed trusted-metadata marker
also fails explicitly; absence alone retains the empty-metadata default.
The evaluator's journal helper escapes HTML comment delimiters inside serialized record fields
without changing their decoded values and neutralizes marker text in the appended human-readable
message, so pull-request data cannot terminate the payload comment or create a second parseable
marker inside the persisted App comment. This closes the wedge for records written after this
change. A record written before this change that already contains a raw `-->` in its serialized
JSON still fails closed; operators must use the terminal-record recovery procedure in
[`docs/review-stalls.md`](../../docs/review-stalls.md#recovering-a-terminal-malformed-journal-record).
Its default-branch recovery verifier requires an authentic terminal candidate, continuity with
the surviving head, no unaccounted frame suffix, chronologically ordered API input with
authoritative nested App attribution, and a current ledger identical to the digest-bound raw
pre-projection `ledgerFindings` snapshot before the operator may delete the malformed comment.
The workflow journals that raw snapshot beside the evaluator's `findings` projection, so recovery
does not infer a ledger state from lossy projection diagnostics or operator-written replay data.
Legacy records without the snapshot recover only by direct projection replay; ambiguous inverse
mapping to advisory is refused. Evaluator-only projection diagnostics are not Markdown ledger
fields. Each newly appended record also carries a digest-bound `unauthenticatedAnchor` boolean.
On an empty journal, a missing or invalid genesis authorization sets it to `true` and adds a
synthetic open, reified `GENESIS` blocker to the evaluation and loop-state hash. Evaluation and
journaling continue with their real state; only convergence is withheld. Once the existing
verified-authorization path accepts the genesis directive, the next record carries `false` and
the blocker closes. Legacy records without the field retain their existing authorized-anchor
interpretation: omission is read as authenticated. That is correct for genuine legacy records;
the only guard against a forged omission is the App-identity digest boundary whose limitation ADR
0011 already concedes. `GENESIS` stays outside `findingIds`, `findings`, and `ledgerFindings`,
whose strict `F-\d{3,}` and tri-surface consistency contracts are unchanged.
For an unchanged `resolved` disposition, the recorded authorization reference ID, author and body
digest remain pinned while the check-run ID and name are read from the current ledger and proved
again against the current evaluator run.

The job alone receives the exact permission set declared by the workflow. `actions: read` lists
the triggering run's jobs and verifies workflow-run identities. `checks: write` reads check-run
evidence and creates the current result check. `contents: read` fetches the evaluator from the
default branch and the candidate stall record from the pull-request head. `issues: write`
authorizes the issue-comment API used for the digest-chained journal, while
`pull-requests: write` is also required because that API call targets a pull request; the latter
also covers reading pull-request metadata, files and reviews. No other permission is granted.
`publishEvaluation` owns the journal eligibility decision: `history-unverifiable` and
`fork-refused` append no journal comment because their history cannot be trusted, but each still
creates a failing `Review loop converged` check whose summary is the evaluator's actual reason
before failing the workflow. Every other result must journal unless that workflow run attempt is
already recorded. Forks deliberately fail closed because the policy refuses partial evidence.

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
