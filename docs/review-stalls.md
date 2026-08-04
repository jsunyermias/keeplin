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
restore the pull request's current review ledger to the exact findings asserted by that recovered
record. This carries forward findings introduced only by the malformed record rather than losing
them during replay.

The exact comparison covers the fields the Markdown ledger represents: `id`, `round`,
`reifiedBy`, `state`, `resolution`, and `reified`, which is derived deterministically from
`reifiedBy`. The adapter derives `evidence` by parsing `resolution`, so the resolution comparison
pins the evidence the ledger can carry. `disposalError` is added only by evaluator projection and
cannot be restored as a ledger column, so it is not compared separately. No non-whitespace
content may follow the recovered frame.

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
comment history. If the command reports that the ledger is not semantically identical, restore
every recovered finding in the pull-request ledger and fetch `pull.json` again.

Do not delete anything unless the verifier exits zero. A zero exit establishes the remaining
comments as an authentic verifying prefix within the journal's bounded, unkeyed threat model,
establishes predecessor continuity, and proves that the current ledger preserves every candidate
finding exactly. Keep its JSON output with the recovery evidence.

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
| — | — | — | — | — |

## Cleared

| Detected | Pull request | Stuck on | Exit | Resolution |
|---|---|---|---|---|
| 2026-08-03 | [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198) | F-002 | dismissed | Accepted [ADR 0008](adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md) bounds the claim; the three-probe follow-up is [tracked](review-loop-spike.md). |
| 2026-08-03 | [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198) | F-008<br>F-013 | resolved | Default-branch isolation and verified-disposal tests pass. |
