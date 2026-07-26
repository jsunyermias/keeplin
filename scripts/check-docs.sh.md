# `scripts/check-docs.sh` — contractual-docs CI check

## Purpose

The mechanical arbiter of the block-complete companion-doc contract (LAYER 2 of the
navigation model, `docs/templates/source-module.md` v2.5). Its structural checks use
standard shell tools and its exact-fidelity/context checks use standard-library Python, so
it still runs before any Rust toolchain exists. Run it locally before pushing; CI fails the
build on any violation.

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
8. **Exact fence fidelity** — `sync-companion-code --check` maps every Rust fence to one
   source leaf and compares the full marker, attributes, signature, body and whitespace.
   Only CRLF/LF line endings are normalized; all other text must match. Stale, truncated,
   reordered, missing, orphan and duplicate fences fail (HARD RULE 7). A container's
   preamble may carry only its declaration, attributes and braces: an unmarked import,
   const or nested item before the first child marker fails as UNCOVERED (HARD RULE 6).
   Before the file's first marker, only BOM, a first-line shebang, blank lines and inner
   crate attributes are scaffolding; any Rust item is UNCOVERED. Write-mode sync replaces
   raw fence-body ranges only, preserving mixed EOL and all bytes outside valid fences.
9. **Generated context index** — `context-pack manifest --check` fails if
   `docs/context-manifest.json` no longer matches the source/companion corpus.

## Known caveat

Check 7's trailing-comment pattern can false-positive on a string literal that contains
` // ` (e.g. a URL). If that ever fires on a legitimate string, reword the string — the
script is the arbiter and must not be weakened to accommodate it.

## Behaviour

- Prints one line per violation and exits `1`; prints confirmation lines and
  exits `0` when clean.
- Repo-wide: during a migration it keeps reporting not-yet-migrated files, which is
  expected. The bar for finishing one file is that *that* file produces zero violations;
  the bar for finishing the migration is the whole script passing clean.

## Refresh procedure after large refactors

After a large refactor, generate the ignored LAYER 1 output locally or download the CI
artifact for that exact commit, then refresh affected `## Graph context` sections from
`graphify query` output. Never stage `graphify-out/`.

## Related files

- `docs/templates/source-module.md` — v2.5 block-complete template; its 9 HARD RULES are
  what this script enforces.
- `sync-companion-code` / `context-pack` — exact-fidelity and context-index entry points.
- `.github/workflows/ci.yml` — runs this check and publishes LAYER 1 as a same-commit
  Graphify artifact; Graph context sections may also use an equivalent ignored local
  generation.
