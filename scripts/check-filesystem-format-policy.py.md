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
- If the decimal `FORMAT_VERSION` declaration cannot be found in the lifecycle module at either
  endpoint, evaluation fails closed instead of treating the constant as unchanged.
- Instead of those two artifacts, the change may cite an ADR from its commit messages or added diff
  lines. The cited file must already exist as `accepted` at the comparison base and contain the
  deliberate marker `- Filesystem-format-exception: <source> -> <target>` for the exact transition;
  policy prose that merely describes exceptions grants nothing, and accepting an exception inside
  the format-bump change is not "separately accepted" and does not pass.
- If `.github/keeplin-release-boundary.json` exists at the comparison base, its content at head must
  remain byte-identical. If it is absent at base, the introducing change may iterate on it;
  immutability begins after that change is merged into the future comparison base.
- The same base-anchored rule protects this policy script: when it exists at base, modification or
  deletion at head fails. This repository check detects ordinary deletion or weakening after the
  gate lands on the protected base, but cannot force GitHub to execute itself.

The checker intentionally does not require migrations for versions 1 through 8 and does not create
the release-boundary latch; those are separate decisions and observations in ADR 0016.

## Invocation

```text
./scripts/check-filesystem-format-policy.py <base-sha> <head-sha>
FORMAT_POLICY_BASE=<base-sha> FORMAT_POLICY_HEAD=<head-sha> ./scripts/check-filesystem-format-policy.py
```

CI supplies the pull-request base/head or push before/after SHAs after a full-history checkout.
Once the default branch contains this file, CI copies that version to a temporary path and executes
it, so a proposed edit cannot replace the policy judging the same change. Bootstrap is bounded:
when both the default branch and comparison base lack the file, CI executes the introducing head
copy; a base-present/default-missing mismatch refuses fallback. Unit tests still import the
checked-out copy to exercise proposed changes. This isolates the enforcing rule from an ordinary
pull-request edit, but does not protect against someone able to write the default branch or remove
the head-controlled workflow step.

A rerun of an already-merged working-branch push can resolve `merge-base == head` and evaluate an
empty range. Its green result is knowingly vacuous: that rerun introduces no new code, and the
merged change passed the gate against its then-current base.

## Dependencies

- `git show`, `git diff`, `git log`, and `git ls-tree` — provide immutable endpoint contents,
  changed Rust lines, ADR citations, and ADR paths; expects both supplied revisions and their
  intervening history to exist locally.
- `keeplin-core/src/storage/fs/lifecycle.rs` — owns the format constant and future migration
  dispatcher.
- `docs/adr/*.md` — accepted bounded-exception evidence.

## Used by

- `.github/workflows/ci.yml` — runs the gate before documentation and Rust checks.
- `scripts/tests/test_filesystem_format_policy.py` — mutation-tests the evaluator.

## Graph context

This operational policy tool is excluded from Graphify's code corpus and is read directly.
