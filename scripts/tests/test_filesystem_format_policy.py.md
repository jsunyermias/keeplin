# `scripts/tests/test_filesystem_format_policy.py` — filesystem format gate mutation tests

## Purpose

These tests create isolated temporary Git repositories and call the real evaluator against actual
commits. They cover unrelated changes, unsupported format bumps, substantive-evidence disclaimers,
changed and unchanged migration dispatchers, transition-named preservation tests, previously
accepted exact-transition markers, proposed and same-change exception ADRs, rejection of ADR 0016's
own policy prose as authorization, and fail-closed constant relocation. For both the release latch
and policy script they pin the endpoint rule: a file absent at base may be created and revised in
the compared history, while a file present at base must be byte-identical at head and cannot be
modified or deleted.

## Dependencies

- `scripts/check-filesystem-format-policy.py` — imported directly so fixtures exercise the shipped
  evaluator logic; expects its public constants and `evaluate` function to remain stable or these
  tests to change with them.
- `git` — creates deterministic local histories without network access.
- `tempfile` and `unittest` — isolate fixtures and provide the repository-wide Python test runner.

## Used by

- `.github/workflows/ci.yml` — discovered by `python3 -m unittest discover -s scripts/tests -p
  'test_*.py'`.

## Graph context

This operational test is excluded from Graphify's code corpus and is read directly.
