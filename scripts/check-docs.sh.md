# `scripts/check-docs.sh` — contractual-docs CI check

## Purpose

Enforces the two rules of the companion-doc system (LAYER 2 of the navigation model):

1. **Every `.rs` file has a companion `.md`** at the same path (`foo.rs` → `foo.md`).
2. **Every such companion contains a `## Graph context` section** — the file's nodes/edges in
   the committed Graphify graph, its direct dependencies/dependents with one-line inline
   summaries, and its restated invariants.

Run locally before pushing; CI runs it as the first step of the workflow and fails the build
on any violation.

## Behaviour

- Scans the whole repo for `*.rs`, pruning `target/`, `graphify-out/`, and `.git/`.
- Prints one line per violation (missing companion, or companion without the section) and
  exits `1`; exits `0` with a confirmation line when clean.
- Pure bash + find + grep — no toolchain needed, so it runs before Rust is even installed.

## Refresh procedure after large refactors

`graphify update .` rebuilds LAYER 1 (the graph, AST-only, no API key) — run it after any
large refactor and refresh the affected `## Graph context` sections from `graphify query`
output.

## Related files

- `docs/templates/source-module.md` / `test-file.md` — the templates defining the section.
- `.github/workflows/ci.yml` — where this runs in CI.
- `graphify-out/graph.json` — LAYER 1, the queryable graph the sections are sourced from.
