# `.github/workflows/adr-0015-verification.yml` — ADR 0015 live credential verification

## Purpose

The original implementation of the fourth test in verification item 9 of
[ADR 0015](../../docs/adr/0015-self-authorized-disposal-with-an-auditable-directive.md) is the test
named `evaluator_GITHUB_TOKEN_really_enumerates_the_expected_repository_principals` in
`.github/scripts/check-review-loop.test.js`. On each CI execution, that test uses the run's actual
`GITHUB_TOKEN`, exercises the worktree copy of the enumerator, compares the result with the
repository-specific literal `["jsunyermias"]`, and reports failure through the required
`Check, Test & Lint` job. It skips locally when CI or the token is absent.

This workflow adds a probe that runs on pushes to `main`, on a daily schedule even when repository
activity does not start CI, and by manual dispatch. It derives the expected singleton from
`repository.owner.login`, exercises evaluator code fetched from the API-reported default branch,
and reports failure through its own job outside the required-job set. It proves that the
evaluator's actual `GITHUB_TOKEN` can exhaustively enumerate the repository principals and that
the single-principal premise for Option C still holds.

It runs on every push to `main`, is scheduled to run daily at 06:17 UTC, and supports manual
dispatch. The push trigger makes the merge that introduces this workflow exercise its final
assertion and makes later failures visible on the affected default-branch commit. The off-hour
minute avoids concentrating the scheduled request at the start of an hour, when GitHub warns that
scheduled runs can be delayed. Scheduled workflows are best-effort and GitHub automatically
disables them after 60 days without repository activity. The schedule is an early warning only:
the evaluator independently enumerates principals on every authorization path and refuses the
affected disposition when enumeration is unknown or the single-principal premise is false.

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
block unrelated repository changes. It has neither a `pull_request` nor a
`pull_request_target` trigger, so it does not run as a pull-request check.

## Trust boundary

There is no checkout and no shell step. The action is pinned to a full commit SHA, and evaluator
code comes from the default branch. The expected login comes from the repository payload rather
than a repository-specific literal, but the singleton `[owner.login]` expectation deliberately
assumes a personally-owned repository. As ADR 0015's Decision records, moving either repository
to an organization requires revisiting this guard before relying on it. The job name is
deliberately distinct from the required CI jobs `Check, Test & Lint` and
`Knowledge graph up to date`.

## Related files

- [`review-loop-evaluator.yml`](review-loop-evaluator.yml) — production authorization adapter
  whose loader, credential, permissions, and enumerator this verification exercises.
- [`../scripts/check-review-loop.js`](../scripts/check-review-loop.js) — exports the principal
  enumeration implementation fetched from the default branch.
- [ADR 0015](../../docs/adr/0015-self-authorized-disposal-with-an-auditable-directive.md) — defines
  Option C and verification item 9.
