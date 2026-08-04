# Review-loop history spike required by ADR 0008

This is the tracked implementation artifact for ADR 0008 option C. Run the probes in a scratch
repository owned by the maintainer; do not infer results from documentation or run them against
Keeplin. Preserve sanitized API requests/responses, actor/App identities, run IDs, suite IDs,
check-run IDs, commit SHAs and timestamps.

## Probe 1 — workflow-run deletion and API-created checks

Create a workflow run, then use a separately authenticated App to create a check run on the same
head SHA and check suite. Delete the workflow run through the Actions API and query the suite and
both check-run IDs. Establish whether the API-created check survives deletion of the workflow run
whose suite it shares.

## Probe 2 — cross-App check-run mutation

Have App A create a check run. Authenticate as distinct App B with `checks: write` and attempt a
valid `PATCH /repos/{owner}/{repo}/check-runs/{id}` changing its output. Record the response and a
refetch as App A. Establish whether GitHub enforces creator-App ownership.

## Probe 3 — dangling-SHA visibility

Create a check on a branch head, force-push so the commit is unreachable from all refs, and query
by check-run ID, suite ID and dangling SHA. Repeat after workflow-run deletion. Establish whether
checks remain listable on dangling commits and through which API paths.

## Decision gate

Publish one reproducible report covering all probes. If every option-C assumption holds, propose
a new ADR superseding 0008. If any fails or remains ambiguous, evaluate option A. Do not amend
0008 or remove `limitation_F002_terminal_truncation_undetected` before that ADR is accepted and
implemented.
