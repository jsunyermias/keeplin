# Pull-request governance checks

`check-review-governance.js` enforces the two permitted paths through the pull-request
template when a pull request leaves draft state:

- an independent reviewer, a distinct implementer, both completed review assertions and a
  GitHub evidence link; or
- every maintainer-waiver field, a change to `docs/review-debt.md`, and an entry that names
  the exact pull request.

`check-review-loop.js` enforces the loop's termination condition from
[keeplin ADR 0008](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md): a pull request converges
when its explicit workflow dependencies succeed and no *reified* finding is open. A finding is reified when
it names something that fails mechanically — a test, a property, a contract assertion, a
`scripts/check-docs.sh` check; a finding that cannot be reduced to a failing check is
`advisory` and does not block. It reads the `## Review ledger` section of the pull-request
body and returns one of four states:

- `converged` — explicit workflow dependencies succeeded, no open reified finding (the only passing state);
- `awaiting-checks` — nothing blocks, but a required check has not finished. A check that has
  not completed is not a green check, so this does not pass;
- `converging` — the blocking set is non-empty but shrinking;
- `escalated` — the loop-state hash repeated, or the blocking set has not shrunk for
  `REVIEW_LOOP_STAGNATION_LIMIT` rounds (3), so the stall must be recorded in
  `docs/review-stalls.md` naming that pull request;
- `malformed` — the ledger or round log contradicts the state CI observes, including a ticked
  "Blocking findings are resolved" box while a reified finding is still open.

The brake measures state, not time. SHA-256 receives canonical JSON containing normalized
changed-file tuples, sorted open finding IDs and sorted red check names, so delimiter bytes
inside filenames or names cannot make different states collide.

Three behaviours are easy to get wrong and are fixed deliberately:

- **Convergence needs positive evidence.** The trusted evaluator reads the completed CI run's
  jobs through the API and requires the two explicitly named jobs to equal `success`. Skipped,
  neutral, absent and unknown block; optional checks are outside this explicit required set.
- **A pull request with no ledger section is round zero**, per ADR 0004's migration contract —
  not malformed. It still has to pass its required checks.
- **A stall record must name what is stuck.** The `## Open` table of `docs/review-stalls.md`
  needs a row for the exact pull request whose `Stuck on` cell contains every blocker as an
  exact `<br>`-delimited token. Punctuation remains part of a name, and `F-0010` is not `F-001`.

The dependency result proves only the jobs named by `converge.needs`. GitHub branch-protection
required checks are configured outside this repository, so this script cannot prove that the
workflow dependency list and branch protection agree. It deliberately makes no broader claim.

The default-branch `workflow_run` workflow is authoritative. It rejects malformed ledger
parses and open findings without a named mechanical check. It verifies App, configured CI
workflow, repository, pull-request and schema identity; collaborator authorization directives
and body digests; and,
for `resolved`, a successful named check bound to the evaluated head, workflow and App. Missing,
changed, dismissed or unreachable evidence reopens the finding. Genesis and tombstones use the
same authorization path. A previously reified finding also cannot become advisory without that
verified authorization, and reification is remembered across every surviving record rather than
only the newest one, so retiring an ID with an authorized tombstone does not let it return
unreified. A finding that names a mechanical check cannot simultaneously use `advisory` state.
Forks deliberately fail closed.

Only `pull_request` runs of the configured CI workflow are eligible as review rounds; its parallel
`push` run is ignored. Evaluations are serialized by pull-request number with `queue: max`, which
retains up to 100 pending runs instead of replacing all but the newest one. The queue is bounded;
additional runs are canceled when it is full, so the journal does not claim unbounded retention of
every completion. Journal append is idempotent for each delivered workflow run ID and attempt; a
retry republishes its success or failure result and a blocked retry fails the workflow without
appending again. Every journal marker occurrence in a configured-App comment is parsed, so a
malformed second payload fails closed instead of disappearing. Every ID observed in surviving history remains
reserved after retirement, including IDs first recorded as advisory. Tombstone and ledger IDs
are validated before they can enter evaluator messages. Once authorization evidence
has been written for the latest disposition, later evaluations of that same disposition bind to
the recorded identity, author and body digest rather than an author-editable replacement in the
current ledger. If a finding is subsequently reopened, a later disposition requires a directive
whose API creation/submission time is after the journal comment that recorded the reopening.
Every marked payload in an authorization comment is parsed before any matching directive is
accepted. Journal serialization escapes marker text inside record fields, and marker text in the
human-readable message is neutralized before the App persists the comment.

The App comment journal's guarantees are bounded: its unkeyed digest chain detects accidental
corruption and casual editing that does not rebuild the chain. It does not authenticate records
against another repository workflow, which can use the same App identity, recompute the digests,
and manufacture convergence on a history in which no finding was ever reified. Deletion is
detected only if a surviving descendant commits to the missing record. A repository writer can
delete the newest records and the shorter prefix evaluates as if those rounds never happened.
Terminal truncation is not detected: it can erase the record that established reification, after
which the shorter authentic prefix may converge with that finding advisory. The limitation tests pin both
this consequence and the positive journal fixture rather than claiming resistance ADR 0008
deliberately does not provide.

Neither script discharges independent review, and `check-review-loop.js` does not weaken
`check-review-governance.js`. The two gates are conjunctive as policy, not yet as mechanism:
governance runs inside the head-controlled workflow, so the trusted evaluator cannot prove it
ran. ADR 0009 accepts moving it into the evaluator; implementation is pending in a dedicated
pull request.

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
