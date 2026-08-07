# `scripts/tests/test_filesystem_format_policy.py` — filesystem format gate mutation tests

## Purpose

These tests create isolated temporary Git repositories and call the real evaluator against actual
commits. They cover unrelated changes, unsupported format bumps, substantive-evidence disclaimers,
changed and unchanged migration dispatchers, transition-named preservation tests, previously
accepted exact-transition markers, proposed and same-change exception ADRs, rejection of ADR 0016's
own policy prose as authorization, fail-closed constant relocation, first latch creation, later
latch mutation, and deletion of a latch already present at the base revision. They also prove the
policy script can be introduced once but cannot subsequently be modified or deleted.

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
