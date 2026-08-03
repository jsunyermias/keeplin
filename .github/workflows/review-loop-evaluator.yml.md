# `.github/workflows/review-loop-evaluator.yml` — trusted review-loop evaluator

## Purpose

This default-branch `workflow_run` workflow is the authoritative evaluator required by
[ADR 0008](../../docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md).
It correlates the completed unprivileged CI run to exactly one open pull request and reads all
pull-request content and evidence through GitHub APIs.

## Trust boundary

There is no checkout and no shell step. The sole action is GitHub's script action pinned to a
full commit SHA. It fetches `.github/scripts/check-review-loop.js` explicitly from the
repository's API-reported default branch. Pull-request files, body text, comments, reviews,
jobs and check runs remain data; none is executed, imported or interpolated into a shell.

The job alone receives `issues: write` and `checks: write`, used to append one digest-chained
journal comment and create the current result check. Contents, actions and pull-request access
are read-only. Forks deliberately fail closed because the policy refuses partial evidence.

## Bounded journal claim

Editing a record is detected by its digest. Deletion is detected only when a surviving
descendant names the missing predecessor. Terminal truncation is not detected: an actor with
repository write access can delete the newest record and the evaluator reads the shorter
authentic prefix as though the missing round never happened.

## Repository mirror

The server repository must carry this workflow and both `.github/scripts/check-review-loop.js`
files byte-identically. Only its unprivileged CI workflow may differ in Rust/PostgreSQL setup;
the trigger name `CI`, required job names, permissions, action pin, schema and evaluator logic
must remain identical.
