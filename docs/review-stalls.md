# Review stalls

The implementation↔review loop converges mechanically: a pull request is done when its
required checks are green and no reified finding is open. When instead the loop stops making
progress, it escalates to the maintainer rather than iterating quietly. This file is the
durable record of what is currently stuck, so a loop that ran out of ideas stays visible
instead of being absorbed into another round.

The convergence rule, the blocking criterion and the bounded stagnation brake are decided in
[`docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md`](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md) and enforced
by `.github/scripts/check-review-loop.js`.

## What counts as a stall

The loop state is SHA-256 over canonical JSON containing normalized changed-file tuples, open
reified finding IDs and non-successful required job names. A stall is either of:

- **A repeated state.** The current hash equals an earlier round's hash: the round changed
  nothing that matters to convergence.
- **No monotonic progress.** The blocking set `{red required checks} ∪ {open reified
  findings}` has not shrunk strictly for `REVIEW_LOOP_STAGNATION_LIMIT` rounds (3 by default).

For a journal that began without verified genesis authorization, synthetic `GENESIS` is one open
reified finding in that set. Its state is persisted separately from Markdown ledger findings as
the digest-bound `unauthenticatedAnchor` boolean on each new journal record. This special ID does
not enter the ledger's `F-\d{3,}` namespace or its retired-ID reservation; a verified genesis
directive closes it through the ordinary authorization verifier.

The brake measures state, not elapsed time or token budget. A slow round that shrinks the
blocking set is progress; a fast round that does not is not.

The accepted brake has two residual accounting limitations. Alternating between different
blocking sets can reset the non-shrinking streak even when the loop makes no lasting progress.
Also, records created by the legacy pull-request path used every observed red check while the
trusted path uses only its explicitly named required jobs, so a migrated journal can compare
blocking counts drawn from different domains. These are advisory limitations of the accepted
ADR 0008 design; neither changes the current convergence rule.

## How an entry is opened

CI fails the pull request with the exact finding or check that is stuck and refuses to pass
until the stall is recorded here naming that pull request. An agent does not decide to
escalate and does not decide to keep going: continuing to iterate after a stall without this
record is prohibited. Write the entry in the same session the stall is detected.

An entry names what is stuck, not a narrative of the rounds. If the stuck item is a reified
finding, give its ID and the check that fails. If it is a red required check, name the check.

This is enforced, not merely asked for: the checker parses the `## Open` table below and
requires a row for the exact pull request whose `Stuck on` cell names **every** current blocker
as an exact token. Separate multiple blockers with `<br>`; commas and semicolons are ordinary
characters inside a blocker name. Thus `F-0010` does not name `F-001`. A
mention of the pull request elsewhere in
this file, or a row under `Cleared`, does not satisfy it. Naming the pull request without
naming what stuck it would have told the maintainer nothing.

The journal's unkeyed digest chain detects accidental corruption and casual editing that does not
rebuild the chain. It does not authenticate records against another repository workflow, which
can use the same App identity, recompute the digests, and manufacture convergence on a history in
which no finding was ever reified. A deleted record is detected when a surviving descendant names
it. An actor with repository write access can delete the newest record and the evaluator reads the
shorter prefix as though the missing round never happened. Terminal truncation is not detected:
it can erase the record that established reification, after which the shorter authentic prefix
may converge with that finding advisory. This dismissed F-002 limitation remains tracked by
[`docs/review-loop-spike.md`](review-loop-spike.md).

Bounded-history anchor: terminal truncation can erase reification history and enable advisory convergence.

## Recovering a terminal malformed journal record

Journal serialization prevents records written after the delimiter-escaping fix from persisting
a raw `-->` inside their JSON. A malformed record written before that change still fails closed;
the fix is forward-only and does not rewrite existing issue comments.

The malformed record must be terminal. It must be the newest review-loop journal comment and no
surviving descendant may name its digest. Before deleting it, pass every earlier configured-App
journal comment, excluding only the candidate, through the default-branch evaluator's
`verifyJournal` with the repository's exact configured identity. Recover the candidate's complete
JSON record as well: its carried digest must verify, its observation must immediately follow the
surviving head, and its declared `priorDigest` must equal that head's digest. Before deletion,
restore the pull request's current review ledger to the raw pre-projection ledger recorded in the
candidate's digest-bound `ledgerFindings` field. This carries forward findings introduced only by
the malformed record without asking the recovery verifier to invert the evaluator's lossy
projection.

The comparison covers the fields the Markdown ledger represents: `id`, `round`, `reifiedBy`,
parser-derived `reified`, `state`, and `resolution`. The adapter derives `evidence` by parsing
`resolution`, so the resolution comparison pins the evidence the ledger can carry.
`disposalError` exists only in the separate evaluator `findings` projection and is not a Markdown
ledger column. A legacy candidate without `ledgerFindings` exposes only its projected values. A
parser-reachable projection may be replayed directly, but a terminal-malformed legacy record from
a failed disposition is not recoverable by this procedure: its projected `reified: true` /
`state: open` values retain a non-reifying `reifiedBy`, while the Markdown parser derives
`reified` from `reifiedBy` and therefore can never reproduce that combination. The verifier must
refuse it. Do not keep rewriting or refetching the ledger; leave the comment in place and escalate
the permanently unreachable recovery candidate to the maintainer. No non-whitespace content may
follow the recovered frame.

Failed-disposition refusal anchor: stop recovery and escalate to the maintainer.

The refusal anchor check guarantees the anchor's exact text, uniqueness, and position immediately
after the failed-disposition refusal paragraph. Positive prose assertions also require that
paragraph to identify the legacy candidate's projected values, state that the failed disposition
is not recoverable and the verifier must refuse it, forbid continued ledger rewriting or
refetching, and require maintainer escalation. They do not exclude contradictory retry advice
inside that paragraph.

The downloaded comment array must be chronological by both `created_at` and comment ID. When the
API provides nested `performed_via_github_app` attribution, it is authoritative; a conflicting
top-level `app_slug` or `app_id` is refused. These checks harden the operator gate's input
attribution and terminality assumptions inside the already documented bounded App-identity threat
model; they do not establish a new provenance guarantee.

The default-branch `check-review-loop-recovery.js` performs all of those checks mechanically. Use
a clean temporary directory, substitute the repository, pull request and candidate issue-comment
ID, and run exactly:

```sh
recovery_repo=jsunyermias/keeplin
recovery_pull=198
recovery_candidate_comment=123456789
recovery_dir=$(mktemp -d)
recovery_default=$(gh api "repos/$recovery_repo" --jq .default_branch)
recovery_repository_id=$(gh api "repos/$recovery_repo" --jq .id)
recovery_workflow_id=$(gh variable get CI_WORKFLOW_ID --repo "$recovery_repo")
gh api --method GET "repos/$recovery_repo/contents/.github/scripts/check-review-loop.js" -f ref="$recovery_default" --jq .content | base64 --decode > "$recovery_dir/check-review-loop.js"
gh api --method GET "repos/$recovery_repo/contents/.github/scripts/check-review-loop-recovery.js" -f ref="$recovery_default" --jq .content | base64 --decode > "$recovery_dir/check-review-loop-recovery.js"
gh api "repos/$recovery_repo/pulls/$recovery_pull" > "$recovery_dir/pull.json"
gh api --paginate --slurp "repos/$recovery_repo/issues/$recovery_pull/comments?per_page=100" | jq 'add' > "$recovery_dir/comments.json"
node "$recovery_dir/check-review-loop-recovery.js" \
  --pull "$recovery_dir/pull.json" \
  --comments "$recovery_dir/comments.json" \
  --candidate-comment-id "$recovery_candidate_comment" \
  --repository-id "$recovery_repository_id" \
  --workflow-id "$recovery_workflow_id" \
  --app-slug github-actions \
  --app-id 15368
```

`repositoryId` comes from the repository API's `id`; `workflowId` comes from the repository's
`CI_WORKFLOW_ID` variable. `appSlug` (`github-actions`) and `appId` (`15368`) come from the
`config` object in the default-branch `.github/workflows/review-loop-evaluator.yml`; if that
configuration changes, use its exact current values rather than the literals above. The pull API
response supplies the current ledger, and the paginated issue-comment API supplies the complete
comment history. The command's `findingsSource` labels `findings` as either the candidate's
`raw pre-projection ledger snapshot` (records with `ledgerFindings`) or its `legacy evaluator
projection` (records without it); `projectedFindings` is the evaluator output kept separately for
diagnosis. The command always reports the candidate's `unauthenticatedAnchor` value and emits
`null` for a legacy candidate that omits the field. The flag is digest-bound historical state, not
a Markdown ledger field, so it is preserved in recovery output but excluded from the
`ledgerFindings` semantic comparison.
For a raw snapshot, if the command reports that the ledger is not semantically
identical, restore every raw finding from `findings` in the pull-request ledger and fetch
`pull.json` again. A failed-disposition legacy projection is the permanently unreachable case
described above, so repeated restoration cannot make it pass and requires maintainer escalation.

Do not delete anything unless the verifier exits zero. A zero exit proves that the current ledger
matches the candidate snapshot identified by `findingsSource` and establishes an authentic verifying prefix.
It proves that prefix from the remaining comments only within the journal's bounded, unkeyed
threat model, and also establishes predecessor continuity. Keep its JSON output with the recovery
evidence.

After that verification, delete only the terminal malformed comment and rerun the affected CI
evaluation. The evaluator derives the round again from the current pull request, checks and
authorizations, and the round is then re-recorded as the next observation. If the malformed
record is not terminal, the remaining prefix does not verify, or the claimed observation cannot
be established, do not delete anything: restore the missing evidence or escalate to the
maintainer. Deleting a non-terminal record leaves a surviving descendant whose predecessor link
cannot verify and does not recover the pull request.

Disposition authorizations have a separate strict time boundary. GitHub's API timestamps have
one-second granularity, and a directive issued in the same second as the intervening observation
is not strictly after it, so the evaluator rejects it. Reissue the directive in a later second.

## How an entry is cleared

A stall is the maintainer's to resolve, and there are exactly three exits:

1. **Fix what fails** — the reified finding's check goes green, the blocking set shrinks, and
   the loop resumes normally.
2. **Dismiss the finding with a cited reason** — a priority decision or an accepted ADR. The
   finding moves to `dismissed` in the ledger with that citation and does not reopen unless the
   code in its area changes.
3. **Reclassify it as advisory** — only when it genuinely cannot be reduced to a failing
   check and an independent authorized reference carries the machine-readable reclassification
   directive. It is then recorded, tracked as a follow-up issue, and no longer blocking. That
   authorization requirement is bounded by the truncation limit above, and the "only" holds no
   further: it is enforced while the record establishing reification survives. An actor with
   repository write access who deletes the newest records reaches a shorter authentic prefix in
   which the finding was never reified, and can file it advisory there with no authorization at
   all.

Recording a stall here does not make the pull request mergeable, and neither does a green CI
run on an unrelated commit: convergence still requires zero open reified findings. Move the
entry to Cleared with the exit that was taken and a link to it.

## Open

| Detected | Pull request | Stuck on | Rounds without progress | Exit taken |
|---|---|---|---|---|
| 2026-08-04 | [keeplin#203](https://github.com/jsunyermias/keeplin/pull/203) | GENESIS | 3 | |
| 2026-08-05 | [keeplin#215](https://github.com/jsunyermias/keeplin/pull/215) | GENESIS | 3 | |
| 2026-08-05 | [keeplin#217](https://github.com/jsunyermias/keeplin/pull/217) | GENESIS | 2 | |

For [keeplin#203](https://github.com/jsunyermias/keeplin/pull/203), `GENESIS` is the
unauthenticated genesis anchor. No procedure for authorizing a genesis directive exists in either
repository, so this stall has no exit available to the author.

[keeplin#215](https://github.com/jsunyermias/keeplin/pull/215) is stuck on the same
`GENESIS` anchor and for the same reason. The pull request proposes
[ADR 0015](adr/0015-self-authorized-disposal-with-an-auditable-directive.md), which decides how a
genesis directive may be authorized; until that ADR is accepted and implemented, the exit it
describes does not yet exist, so this stall likewise has no exit available to the author.

[keeplin#217](https://github.com/jsunyermias/keeplin/pull/217) is stuck on `GENESIS` too, and it is
the pull request that **implements** the exit. It escalated on the repeated-state brake rather than
the non-shrinking one: its second observation carried a loop-state hash byte-identical to its first,
because a second evaluation ran against an unchanged tree. The blocking set was never going to
shrink in between.

The reason it cannot open its own gate is mechanical, not a judgement call. The evaluator runs from
the **default branch**, so the authorization path that would close `GENESIS` is the one on `main` —
not the one this pull request adds. It becomes available to the *next* pull request once this one
merges, and to this one never. Recorded here so that a reader who sees an implementation blocked by
the defect it fixes knows it was understood rather than overlooked.

[keeplin#232](https://github.com/jsunyermias/keeplin/pull/232) escalated on the repeated-state
brake and cleared in the same session, and it is kept here because its root cause is one this
file's readers can hit again. The cause was in the pull-request body rather than in the tree: the
`Review evidence/link` field named only the round-1 reviewer's chat session, and the governance
checker requires a `github.com` URL there. With no such link the review path cannot be satisfied,
so the checker falls through to demanding a complete maintainer waiver — reporting a missing
waiver field, which is the symptom rather than the cause — and `Check, Test & Lint` goes red on a
body field while every test passes.

It survived three green runs first, because that governance step only runs on `pull_request`
events and every run until the pull request was marked ready for review had been a push. A body
field that fails the loop is therefore first exercised late, after the author has stopped watching
for it.

`F-001`, `F-002`, `F-005`, `F-006` and `F-007` were stuck only as a consequence: a `resolved`
finding may not cite a check that failed, so all five reopened the moment the job went red, and
their directives then predated the reopening. Nothing about the findings or their fixes changed.
The third observation repeated the second's hash because the intervening evaluation ran against an
unchanged tree, which is the brake behaving correctly — the loop was not going to make progress
without a change, and the change needed was to the body.

## Cleared

| Detected | Pull request | Stuck on | Exit | Resolution |
|---|---|---|---|---|
| 2026-08-03 | [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198) | F-002 | dismissed | Accepted [ADR 0008](adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md) bounds the claim; the three-probe follow-up is [tracked](review-loop-spike.md). |
| 2026-08-03 | [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198) | F-008<br>F-013 | resolved | Default-branch isolation and verified-disposal tests pass. |
| 2026-08-08 | [keeplin#232](https://github.com/jsunyermias/keeplin/pull/232) | Check, Test & Lint<br>F-001<br>F-002<br>F-005<br>F-006<br>F-007 | resolved | The `Review evidence/link` field now names the [disposal comment](https://github.com/jsunyermias/keeplin/pull/232#issuecomment-5225489719) on `github.com`, so the governance checker's review path is satisfied and the job is green; the five findings were re-authorized by [directives](https://github.com/jsunyermias/keeplin/pull/232#issuecomment-5225538632) issued after the reopening. |
