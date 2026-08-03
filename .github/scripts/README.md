# Pull-request governance checks

`check-review-governance.js` enforces the two permitted paths through the pull-request
template when a pull request leaves draft state:

- an independent reviewer, a distinct implementer, both completed review assertions and a
  GitHub evidence link; or
- every maintainer-waiver field, a change to `docs/review-debt.md`, and an entry that names
  the exact pull request.

`check-review-loop.js` enforces the loop's termination condition from
[keeplin ADR 0004](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0004-review-loop-convergence.md): a pull request converges
when its required checks are green and no *reified* finding is open. A finding is reified when
it names something that fails mechanically — a test, a property, a contract assertion, a
`scripts/check-docs.sh` check; a finding that cannot be reduced to a failing check is
`advisory` and does not block. It reads the `## Review ledger` section of the pull-request
body and returns one of four states:

- `converged` — required checks green, no open reified finding (the only passing state);
- `converging` — the blocking set is non-empty but shrinking;
- `escalated` — the loop-state hash repeated, or the blocking set has not shrunk for
  `REVIEW_LOOP_STAGNATION_LIMIT` rounds (3), so the stall must be recorded in
  `docs/review-stalls.md` naming that pull request;
- `malformed` — the ledger or round log contradicts the state CI observes, including a ticked
  "Blocking findings are resolved" box while a reified finding is still open.

The brake measures state, not time: the loop-state hash is
`sha256(normalized diff ‖ open reified finding IDs ‖ red check names)`, with the normalized
diff being the changed paths and their blob SHAs, sorted.

Neither script discharges independent review, and `check-review-loop.js` does not weaken
`check-review-governance.js`: the two gates are conjunctive.

Both scripts are intentionally dependency-free so the workflow can load them from
`actions/github-script`. Run their regression suites locally with:

```sh
node --test \
  .github/scripts/check-review-governance.test.js \
  .github/scripts/check-review-loop.test.js
```

Draft pull requests remain exempt from both checks so authors can prepare the body
incrementally. Editing the body or marking the pull request ready triggers CI again — which is
how a ledger update re-runs the convergence check without a new commit.
