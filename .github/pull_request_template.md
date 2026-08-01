## Summary

<!-- What changes, why, and who is affected? -->

## Companion read before the code

Mandatory whenever the diff touches a supported source file. `Check pull-request review
governance` verifies the marker exists in the repository; a fabricated one fails.

- Companion marker preserved: `// md:...`
- Invariant it preserves:
- Test fails when the fix is reverted: Sí / No / No aplica

## Soft-rail exception

See `AGENTS.md` § "Protocolo de Excepción Secuencial". Leave `No` unless the maintainer
walked all three steps for **this** pull request.

- Soft-rail exception invoked: No
- Step 1 comment:
- Step 2 comment:
- Step 3 comment:

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
- Reviewer family (claude / codex / kimi / gemini / llama / mistral / human):
- Implementer family (claude / codex / kimi / gemini / llama / mistral / human):
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
