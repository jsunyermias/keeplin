# Retrospective review: PRs #178, #180 and #181

| Field | Value |
|---|---|
| Reviewed | 2026-08-01 |
| Reviewer | Codex |
| Implementer | Claude |
| Checklist | `docs/prompts/0.C-prompt-revision-seguridad.md` |
| Client scope | [keeplin#178](https://github.com/jsunyermias/keeplin/pull/178), [keeplin#180](https://github.com/jsunyermias/keeplin/pull/180), [keeplin#181](https://github.com/jsunyermias/keeplin/pull/181) |
| Server scope | [keeplin-srv#92](https://github.com/jsunyermias/keeplin-srv/pull/92), [keeplin-srv#93](https://github.com/jsunyermias/keeplin-srv/pull/93) |

## Verdict

The final head of #178 is correct for its stated refactor and has no remaining blocking
finding. The prepared-issue change in #180/#92 and the review-debt policy in #181/#93 are
internally consistent in intent, but each exposed one actionable defect in real use. Those
defects are carried by the coordinated governance-fix issues and PRs that add this review.

This review is independent: it was performed by Codex against the merged diffs and issue
objectives, not from Claude's description as the sole source.

## #178 — storage database module split

The review compared the final head `e615194` with its merge base and rechecked the findings
from the earlier review rounds. The final commit corrected every affected `StorageError`
statement, including the two additional occurrences found while applying the last requested
change. Module inventories, paths, re-exports and companion coverage agree with the resulting
tree, and the exact final head passed the required GitHub checks.

Moving the existing persistence implementation into smaller modules did not change the
schema, migration protocol, persistence guarantees or security boundary. It therefore did
not introduce an architectural decision that required a new ADR. The earlier unanswered ADR
question is resolved as “not required for this structure-only refactor.”

**Result:** no open finding; the #178 review-debt entry can be cleared.

## #180 and keeplin-srv #92 — prepared-issue format

The English issue template and the Spanish preparation prompt express the same fields and do
not contradict `AGENTS.md`; filling the structural template with Spanish prose is supported.
However, both told the author to read “the companion of any file the change creates.” That is
impossible before a new file and its companion exist. The correct bounded context is the
applicable template plus the nearest existing companion of the same supported source kind.

The #180 author evidence also said the context tooling covered 45 sources, while its merge
base contained 55 supported sources. That statement was stale author evidence, not a product
defect or a failed required check. It must be corrected in the historical PR conversation so
future readers do not reuse it as a measured fact.

**Result:** actionable documentation finding; corrected in the coordinated governance-fix
PRs, with a historical correction comment required on #180.

## #181 and keeplin-srv #93 — review waiver and debt

The policy correctly distinguishes author assertions, CI evidence, independent review and an
explicit per-PR maintainer waiver. It also correctly treats a waiver as deferred review rather
than removal of the obligation. The missing control is mechanical: a ready PR can leave both
the independent-review and waiver sections blank, and nothing checks that a waived PR changes
`docs/review-debt.md` or names itself there.

The remediation keeps draft PRs editable, but makes the existing required `Check, Test &
Lint` job validate every ready PR. It accepts only one of two paths:

1. distinct named reviewer and implementer, both review assertions checked, and a GitHub
   evidence link; or
2. all maintainer-waiver fields completed, `docs/review-debt.md` changed in the same PR, and
   an entry naming that exact repository and PR number.

Body edits and ready-for-review transitions retrigger the check. Dependency-free Node tests
cover both successful paths and the missing-evidence, self-review, incomplete-waiver,
unchanged-debt and wrong-PR failures.

**Result:** actionable governance finding; corrected in the coordinated governance-fix PRs.

## Debt disposition

All three debt entries were independently examined in this sweep. #178 has no residual
finding; the #180/#92 and #181/#93 findings have bounded remediations and verification. The
entries therefore move from Open to Cleared with this document as their durable review
evidence. This does not pre-review the new remediation diff, which still requires a reviewer
independent from Codex before its PRs leave draft state.
