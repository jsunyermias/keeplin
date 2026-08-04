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
same `verifyAuthorization` path, but keeplin ADR 0013 changes only the empty-journal outcome: an
unreachable genesis directive creates a synthetic open, reified `GENESIS` blocker and evaluation
continues, while an unreachable tombstone directive still makes history unverifiable. A verified
genesis directive disposes that synthetic blocker through the existing authorization mechanism.
A previously reified finding also cannot become advisory without that verified authorization,
and reification is remembered across every surviving record rather than
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
reserved after retirement, including IDs first recorded as advisory. The evaluator checks every
surviving adjacent observation, so a historical absence followed by the ID's return is rejected
rather than hidden by a currently present tail record. Tombstone and ledger IDs are validated
before they can enter evaluator messages. Once authorization evidence
has been written for the latest disposition, later evaluations of that same disposition bind to
the recorded identity, author and body digest rather than an author-editable replacement in the
current ledger. After any intervening disposition, a later change requires a directive whose API
creation/submission time is after the first journal observation of the intervening disposition;
unchanged observations in that same disposition do not move the freshness boundary.
An observation where the ID is absent is itself an intervening disposition, so a
`dismissed -> absent -> dismissed` sequence cannot be collapsed into one dismissal run. Because
GitHub timestamps have one-second granularity, the directive must be strictly after the boundary:
a directive issued in the same second as the intervening observation is rejected. Reissue the
directive in a later second.
For an unchanged `resolved` disposition, only the authorization reference ID, author and body
digest remain pinned to the journal. Its check-run ID and name come from the current ledger and
must prove the resolution again against the current evaluator run.
Every marked payload in an authorization comment is parsed before any matching directive is
accepted. Journal serialization escapes HTML comment delimiters inside record fields without
changing their decoded values, and marker text in the human-readable message is neutralized
before the App persists the comment. This closes the wedge for records written after this change.
A record written before this change that already contains a raw `-->` in its serialized JSON
still fails closed; the terminal-record recovery procedure is in
[`docs/review-stalls.md`](../../docs/review-stalls.md#recovering-a-terminal-malformed-journal-record).
That procedure uses `check-review-loop-recovery.js` from the default branch to recover and verify
the candidate record mechanically before deletion, including predecessor continuity and exact
raw-ledger preservation. Each new journal record carries `unauthenticatedAnchor`, a digest-bound
boolean stating whether its evaluation still lacked a verified genesis directive, and
`ledgerFindings`, the digest-bound raw pre-projection ledger beside the evaluator's `findings`
projection. `GENESIS` is not a Markdown ledger ID: it participates in the open/reified blocking
set and state hash, while the boolean carries its anchor state without weakening the `F-\d{3,}`
identifier grammar or the three ledger-ID surfaces. Recovery compares the current
ledger directly with that raw snapshot across `id`, `round`, `reifiedBy`, parser-derived
`reified`, `state` and `resolution`; it never infers a raw state from a projected shape or from
operator-written replay data. Parsed `evidence` remains represented by `resolution`, so no ledger
field is dropped. A legacy candidate without `ledgerFindings` exposes only its evaluator
projection, and the command labels that `findings` output as `legacy evaluator projection` rather
than a raw snapshot. A parser-reachable legacy projection may be replayed directly. A
terminal-malformed legacy record from a failed disposition is not recoverable by this procedure:
its non-reifying `reifiedBy` cannot parse back to the projected `reified: true` / `state: open`
combination. The verifier refuses it without attempting an inverse mapping; leave the comment in
place and escalate to the maintainer.

Failed-disposition refusal anchor: stop recovery and escalate to the maintainer.

The refusal anchor check guarantees the anchor's exact text, uniqueness, and position immediately
after the failed-disposition refusal paragraph. Positive prose assertions also require that
paragraph to identify the legacy evaluator projection, state that the failed disposition is not
recoverable by this procedure, and require maintainer escalation. They do not exclude
contradictory retry advice inside that paragraph.

For records with `ledgerFindings`, the command labels
`findings` as `raw pre-projection ledger snapshot`. The candidate frame must have no non-whitespace
suffix. Recovery input must be chronological by `created_at` and comment ID. The API's nested
`performed_via_github_app` attribution is authoritative when present, and conflicting top-level
attribution is refused. The command always reports the candidate's `unauthenticatedAnchor` value;
it emits `null` for a legacy candidate that omits the field. The verifier accepts either boolean
value as digest-bound historical state and refuses a present non-boolean value before comparing
ledger findings.

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

Bounded-history anchor: terminal truncation can erase reification history and enable advisory convergence.

The anchor checker enforces raw exact-line enrolment only. It does not parse Markdown or enforce
that the anchor is visible or conspicuous in rendered prose; an exact line inside an HTML comment
or fenced code block satisfies the checker.

Neither script discharges independent review, and `check-review-loop.js` does not weaken
`check-review-governance.js`. The two gates are conjunctive as policy, not yet as mechanism:
governance runs inside the head-controlled workflow, so the trusted evaluator cannot prove it
ran. Keeplin ADR 0012 accepts moving it into the evaluator and supersedes ADR 0009;
implementation is pending in a separate pull request off `main`.

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
