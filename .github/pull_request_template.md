<!--
  Fill in the sections below. Delete any that genuinely do not apply, but do not
  delete the checklist — CI enforces most of it, and a reviewer will look for it.
-->

## Summary

<!-- What does this PR change, and why? One short paragraph. -->

## Linked issues

<!-- e.g. "Resolves #128" / "Part of #125". If none, say so and why. -->

## Type of change

- [ ] Feature
- [ ] Bug fix
- [ ] Refactor (no behaviour change)
- [ ] Docs / companion-only
- [ ] Chore / tooling

## What changed

<!--
  Bullet the concrete changes, grouped by crate:
  - keeplin-core — …
  - keeplin-daemon — …
-->

## Contract & compatibility

- [ ] Every touched `.rs` has its companion `.md` updated **verbatim** (block-complete v2.3.1): one `// md:` marker per block, one Coverage-checklist row per marker, no elided fences, no non-`// md:` comments in the `.rs`.
- [ ] `scripts/check-docs.sh` passes clean.
- [ ] Proto changes (if any) are **additive** — new field numbers only, existing numbers never renumbered or reused.
- [ ] On-disk / on-wire format changes bump the relevant version (`FsBackend::FORMAT_VERSION`, `DbBackend::SCHEMA_VERSION`) with a migration step, and the change is documented in the companion.
- [ ] No stray references to "pizarra" in touched code.

## Verification

- [ ] `cargo fmt --check --all` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo test -p keeplin-core` and `cargo test -p keeplin-daemon` green.
- [ ] Tests added or updated for the new behaviour.
- [ ] `graphify update .` run and the refreshed `graphify-out/` committed (code changes only). CI (`scripts/check-graph.sh`) fails if the graph is stale; enable the auto-refresh hook once with `git config core.hooksPath .githooks`. Requires `pip install graphifyy==0.9.25`.

<!-- Paste anything a reviewer should know that the diff doesn't show:
     manual testing done, follow-ups deferred, known limitations. -->
