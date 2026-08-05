# `.github/workflows/adr-0015-verification.yml` — ADR 0015 live credential verification

## Purpose

This workflow implements the fourth test in verification item 9 of
[ADR 0015](../../docs/adr/0015-self-authorized-disposal-with-an-auditable-directive.md).
It proves that the evaluator's actual `GITHUB_TOKEN` can exhaustively enumerate the repository
principals and that the single-principal premise for Option C still holds.

It runs daily at 06:17 UTC and on manual dispatch. The off-hour minute avoids concentrating the
scheduled request at the start of an hour, when GitHub warns that scheduled runs can be delayed.
The schedule is an early warning only: the evaluator independently enumerates principals on every
authorization path and refuses the affected disposition when enumeration is unknown or the
single-principal premise is false.

## Execution contract

The workflow grants exactly the five permissions granted to
[`review-loop-evaluator.yml`](review-loop-evaluator.yml). It fetches
`.github/scripts/check-review-loop.js` from the API-reported default branch with
`repos.getContent`, evaluates it with `vm.runInNewContext`, and calls the exported
`enumerateRepositoryPrincipals`. It therefore exercises the same implementation and credential
used by authorization rather than a local checkout or a parallel pagination loop.

Success requires `ok === true` and `principals` to equal exactly the lower-cased repository owner
login as a singleton array. A forbidden, rate-limited, malformed, failed, or non-exhaustive
enumeration fails the job. A second principal also fails the job and becomes observable in the
step summary and error, while the workflow remains outside the required-job set and does not
block unrelated repository changes.

## Trust boundary

There is no checkout and no shell step. The action is pinned to a full commit SHA, evaluator code
comes from the default branch, and the expected login comes from the repository payload rather
than a repository-specific literal. The job name is deliberately distinct from the required CI
jobs `Check, Test & Lint` and `Knowledge graph up to date`.

## Related files

- [`review-loop-evaluator.yml`](review-loop-evaluator.yml) — production authorization adapter
  whose loader, credential, permissions, and enumerator this verification exercises.
- [`../scripts/check-review-loop.js`](../scripts/check-review-loop.js) — exports the principal
  enumeration implementation fetched from the default branch.
- [ADR 0015](../../docs/adr/0015-self-authorized-disposal-with-an-auditable-directive.md) — defines
  Option C and verification item 9.
