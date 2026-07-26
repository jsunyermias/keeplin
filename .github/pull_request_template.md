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

## Independent review

The independent reviewer receives the objective and diff; the author's explanation is not
the sole source.

- Reviewer (human or model family):
- Implementer (human or model family):
- Prompt/checklist used: `docs/prompts/0.C-prompt-revision-seguridad.md`
- [ ] Reviewer is independent from the implementer.
- [ ] Blocking findings are resolved and conversations are closed.
- Review evidence/link:

## Merge readiness

- [ ] PR is out of draft only after implementation and self-review are complete.
- [ ] Required CI is green on the merge candidate.
- [ ] Independent review is recorded.
- [ ] Companion PR status is consistent with this PR.

The maintainer performs the merge.
