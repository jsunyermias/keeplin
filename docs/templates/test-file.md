<!--
  TEMPLATE: companion doc for a test file (`tests/foo.rs` -> `tests/foo.md`), or a large
  `#[cfg(test)]` module worth documenting. The goal is a map of *what behaviour is proven*,
  so a reader knows the guarantees without reading every assertion. Delete comments and
  unused sections before committing.
-->
# `{{tests/file.rs}}` — {{what it tests}}

## What is tested

<!-- One paragraph: the unit under test, how each test sets up (fresh tempdir / in-memory
     db / real socket), and whether anything is mocked. -->
{{The component under test and the common fixture pattern (e.g. "each test builds a fresh
`FsBackend` on a temp dir; no mocking, real filesystem").}}

## Test cases

<!-- Group by feature area. One row per test function: name, scenario, expected outcome.
     This table is the deliverable — keep it complete and current. -->
### {{Feature area}}

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `{{test_fn}}` | {{what it does}} | {{what it asserts}} |

## Fixtures and helpers

<!-- Shared setup helpers and where they come from. Delete if each test is self-contained. -->
| Utility | Source | Purpose |
|---------|--------|---------|
| `{{helper}}` | {{module/crate}} | {{what it provides}} |

## Coverage gaps

<!-- Honest list of what is deliberately *not* covered here and why (tested elsewhere, out
     of scope, hard to simulate). Keeps reviewers from assuming false guarantees. -->
- {{What is not tested here, and where it is covered instead — or why it is out of scope.}}

## Graph context

<!-- MANDATORY section (CI-enforced). Data source: the committed Graphify graph —
     `graphify query "<file or symbol>"` / `graphify explain "<concept>"` against
     graphify-out/graph.json. Label every relationship: EXTRACTED when it comes
     mechanically from the graph/AST, INFERRED when you concluded it yourself —
     never present inference as fact. The one-line inline summaries are the
     hyper-redundancy requirement: a small model given ONLY this .md must be able
     to work on the paired file safely, without following any pointer. -->

**Nodes/edges this file contributes**

- `{{Entity}}` — {{what it is; the relationships the graph holds for it, labelled EXTRACTED/INFERRED}}

**Direct dependencies** (what this file uses; one line each on what it is and why it matters here)

- `{{path}}` — {{ONE-LINE summary}} (EXTRACTED|INFERRED)

**Direct dependents** (who breaks if this file changes; one line each)

- `{{path}}` — {{ONE-LINE summary}} (EXTRACTED|INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated elsewhere)

- {{invariant}}

## Related files

- `{{path/to/code_under_test.rs}}` — the code under test
- `{{path}}` — {{a sibling test file that covers the complementary case}}
