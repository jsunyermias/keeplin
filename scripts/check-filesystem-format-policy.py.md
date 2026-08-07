# `scripts/check-filesystem-format-policy.py` — filesystem format policy gate

## Purpose

This standard-library-only CI checker enforces ADR 0016 item 8 over an exact Git base/head pair.
It is deliberately syntactic. Passing proves that required evidence is visibly present; it does
not prove that a migration or test actually preserves data, which remains an independent review
obligation and is stated in both success and failure output.

## Checks

- If `FsBackend::FORMAT_VERSION` changes, `apply_format_migration` must be newly added or have a
  different complete function body and the Rust diff must add a test whose name contains both
  `preserv` and the exact `v<source>_to_v<target>` transition.
- Instead of those two artifacts, the change may cite an ADR from its commit messages or added diff
  lines. The cited file must already exist as `accepted` at the comparison base and explicitly name
  both `FORMAT_VERSION` and a `bounded exception`; accepting the exception inside the format-bump
  change is not "separately accepted" and does not pass.
- Once `.github/keeplin-release-boundary.json` exists in any commit after the comparison base, every
  subsequent compared commit must retain byte-identical content. This catches deletion or mutation
  even when the latch was first added earlier in the same pull-request branch.

The checker intentionally does not require migrations for versions 1 through 8 and does not create
the release-boundary latch; those are separate decisions and observations in ADR 0016.

## Invocation

```text
./scripts/check-filesystem-format-policy.py <base-sha> <head-sha>
FORMAT_POLICY_BASE=<base-sha> FORMAT_POLICY_HEAD=<head-sha> ./scripts/check-filesystem-format-policy.py
```

CI supplies the pull-request base/head or push before/after SHAs after a full-history checkout.

## Dependencies

- `git show`, `git diff`, `git log`, `git rev-list`, and `git ls-tree` — provide immutable revision
  contents, changed Rust lines, ADR citations, commit order, and ADR paths; expects both supplied
  revisions and their intervening history to exist locally.
- `keeplin-core/src/storage/fs/lifecycle.rs` — owns the format constant and future migration
  dispatcher.
- `docs/adr/*.md` — accepted bounded-exception evidence.

## Used by

- `.github/workflows/ci.yml` — runs the gate before documentation and Rust checks.
- `scripts/tests/test_filesystem_format_policy.py` — mutation-tests the evaluator.

## Graph context

This operational policy tool is excluded from Graphify's code corpus and is read directly.
