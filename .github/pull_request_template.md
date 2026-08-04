## Summary

<!-- What changes, why, and who is affected? -->

## Linked work

- Issue:
- Companion PR:
- Accepted ADR or `not required` with reason:

## Author assertions

These are claims made by the implementer and must remain distinct from CI evidence.

- [ ] The diff is limited to the linked issue and contains no unrelated changes.
- [ ] I checked every acceptance criterion against the resulting behavior.
- [ ] Source, companion documents, Graphify corpus configuration and project documentation
      are consistent; no generated `graphify-out/` file is staged.
- [ ] Every embedded Rust fence passes the mechanical fidelity check; no companion code
      was manually truncated or paraphrased.
- [ ] Cross-repo wire/format changes have a linked companion PR, immutable core pin,
      lockstep versioning when breaking, and contract coverage.
- [ ] I assessed negative paths, failure behavior, security, persistence and recovery
      appropriate to this change.
- [ ] Every required ADR is accepted and linked; this PR does not cross an unresolved
      decision boundary.
- [ ] I have not described the change as production-ready based only on happy-path tests.

## Verification reported by the author

<!-- Record exact commands and outcomes. Use "not run — reason" where appropriate. -->

| Check | Result |
|---|---|
| `cargo fmt --all --check` | |
| `cargo clippy --workspace --all-targets -- -D warnings` | |
| `cargo test --workspace` | |
| `./scripts/check-docs.sh` | |
| `python3 -m unittest discover -s scripts/tests -p 'test_*.py'` | |
| `./scripts/context-pack manifest --check` | |
| `GRAPHIFY_REQUIRED=1 ./scripts/check-graph.sh` when applicable | |

## Verified by CI

<!-- Complete from GitHub checks, not from the author's local claims. -->

- [ ] Required checks are green on the exact head commit.
- [ ] No required check is skipped or neutral without an accepted explanation.
- CI run or check-suite link:

## Review ledger

Every finding raised in the review loop, with a stable ID and exactly one state. A finding
**blocks** only when it is *reified* — named as something that fails mechanically: a test, a
property, a contract assertion, or a `scripts/check-docs.sh` check. A finding that cannot be
reduced to a failing check is `advisory`: recorded, not blocking. A `dismissed` finding cites
the priority decision or accepted ADR that settles it, and re-raising it does not reopen it
unless the code in its area changed. See
[`docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md`](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md).

States: `open` · `resolved` · `dismissed` · `advisory`. An `open` finding must name a failing
check; leave `Reified by` as `advisory` only for a finding that is not blocking.

| ID | Round | Reified by | State | Resolution |
|---|---|---|---|---|

For `resolved` or `dismissed`, `Resolution` is compact JSON containing `referenceId`, `author`
and `bodyDigest`; `resolved` also contains `checkRunId` and one exact required `checkName`.
The referenced review/comment body must carry a `keeplin-review-loop-authorize` HTML comment
whose JSON names the exact `finding`, target `state` and non-empty `reason`. Its author must be
an independent MEMBER, OWNER or COLLABORATOR. Genesis and tombstones use states `genesis` and
`tombstone` in the metadata object below. An empty journal may evaluate without
`genesisEvidence`, but its digest-bound `unauthenticatedAnchor` remains true and synthetic
`GENESIS` remains open and reified; convergence requires the same verified directive this field
records.

<!-- keeplin-review-loop-metadata {"genesisEvidence":null,"tombstones":[]} -->

### Round log

`Blocking` is the size of `{red required checks} ∪ {open reified findings}` and must shrink
strictly each round. A repeated loop-state hash, or no shrink for 3 rounds, escalates to the
maintainer and is recorded in
[`docs/review-stalls.md`](../docs/review-stalls.md). CI prints the hash to record.
Required jobs must explicitly report `success`; skipped, neutral, missing and unknown are not
green. The App comment journal's unkeyed chain detects accidental corruption and casual edits,
not forgery by another repository workflow with the same App identity. Deletion is detected only
while a descendant survives. Terminal truncation is not detected.

| Round | Loop-state hash | Blocking |
|---|---|---|

## Independent review

Convergence is not review. This section is a separate, conjunctive requirement: a pull
request can converge above and still be unmergeable for want of an independent reviewer.

The independent reviewer receives the objective and diff; the author's explanation is not
the sole source.

- Reviewer (human or model family):
- Implementer (human or model family):
- Prompt/checklist used: `docs/prompts/0.C-prompt-revision-seguridad.md`
- [ ] Reviewer is independent from the implementer.
- [ ] Blocking findings are resolved and conversations are closed.
- Review evidence/link:

Independent review is not the implementer's to skip. If it did not happen, this PR merges
only on an explicit maintainer waiver for this PR, and merges as review debt.

- Maintainer waiver (leave empty unless the maintainer set the review aside here):
  - Where the maintainer said so:
  - What goes unreviewed:
  - Entry in `docs/review-debt.md`:
  - Follow-up issue or sweep that will carry the deferred review:

## Merge readiness

- [ ] PR is out of draft only after implementation and self-review are complete.
- [ ] Required CI is green on the merge candidate.
- [ ] Independent review is recorded, or a maintainer waiver and its review-debt entry are.
- [ ] Companion PR status is consistent with this PR.

The maintainer performs the merge.
