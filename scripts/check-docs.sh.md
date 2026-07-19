# `scripts/check-docs.sh` — contractual-docs CI check

## Purpose

The mechanical arbiter of the block-complete companion-doc contract (LAYER 2 of the
navigation model, `docs/templates/source-module.md` v2.3). It is dependency-free bash
(`find` + `grep` + `awk` only) so it runs as the very first CI step, before any Rust
toolchain exists. Run it locally before pushing; CI fails the build on any violation.

## What it checks

For every `.rs` file in the repo (pruning `target/`, `graphify-out/`, `.git/`), it verifies:

1. **Companion exists** — a `.md` at the same path (`foo.rs` → `foo.md`).
2. **`## Graph context` section** — the companion carries it (LAYER 1 ↔ LAYER 2 link).
3. **Markers present and mirrored** — the `.rs` has at least one `// md:` marker, and
   every marker also appears verbatim in the companion (grep both directions).
4. **No duplicate markers** — each `// md:` marker occurs exactly once in the `.rs`
   (HARD RULE 4: one marker per block).
5. **Coverage-checklist correspondence** — the companion has a `## Coverage checklist`
   whose data-row count equals the marker count. Grouped rows (e.g. `| 5-17 |`) fail,
   because they collapse several blocks into one row.
6. **No elision inside `` ```rust `` fences** — the companion's rust fences contain none
   of `// ...`, `// snip`, `// rest unchanged`, `// as before`, `/* ... */` (HARD RULE 2:
   code is embedded complete, never shortened).
7. **Uncommented-code convention** — the only comment lines allowed in the `.rs` are
   `// md:` markers; any other `//`, `///`, `//!` or `/* */` comment fails (HARD RULE 9:
   all explanation lives in the companion, so fences never contain doc comments).

## What it deliberately does NOT verify

**Verbatim fence fidelity.** The script confirms marker/row/section *correspondence*
mechanically, but it does not — and cannot cheaply — confirm that each fence's body is
character-for-character identical to the marked block in the `.rs`. That is the author's
self-check (HARD RULE 7): re-read the `.rs` top to bottom and confirm every fence matches
its block, whitespace included. A file can pass this script and still be an incomplete
migration if a fence was paraphrased.

## Known caveat

Check 7's trailing-comment pattern can false-positive on a string literal that contains
` // ` (e.g. a URL). If that ever fires on a legitimate string, reword the string — the
script is the arbiter and must not be weakened to accommodate it.

## Behaviour

- Prints one line per violation and exits `1`; prints a single confirmation line and
  exits `0` when clean.
- Repo-wide: during a migration it keeps reporting not-yet-migrated files, which is
  expected. The bar for finishing one file is that *that* file produces zero violations;
  the bar for finishing the migration is the whole script passing clean.

## Refresh procedure after large refactors

`graphify update .` rebuilds LAYER 1 (the graph, AST-only, no API key) — run it after any
large refactor and refresh the affected `## Graph context` sections from `graphify query`
output.

## Related files

- `docs/templates/source-module.md` — v2.3 block-complete template; its 9 HARD RULES are
  what this script enforces.
- `.github/workflows/ci.yml` — where this runs in CI (first step).
- `graphify-out/graph.json` — LAYER 1, the queryable graph the Graph context sections
  are sourced from.
