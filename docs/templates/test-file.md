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

## Related files

- `{{path/to/code_under_test.rs}}` — the code under test
- `{{path}}` — {{a sibling test file that covers the complementary case}}
