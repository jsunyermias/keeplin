# `scripts/companion_tool.py` — deterministic companion fidelity engine

## Purpose

Standard-library Python implementation shared by the synchronization, manifest and
context-pack launchers. It closes RULE 7 mechanically without network access or an LLM.

## Fidelity model

For each `.rs`, the tool reads unique `// md:` markers in source order. A marker followed
by a child marker is a container and must not own a fence. Every other marker is a leaf and
must own exactly one `rust` fence whose first line is that marker. The source block extends
to the next marker, preserving deliberately grouped unmarked helpers; closing braces that
belong only to surrounding containers are excluded.

Only attributes, the container declaration (including multiline `where` clauses),
braces and blank lines may appear between a container marker and its first child.
Any real item or import in that gap is reported as `UNCOVERED` and must become an
explicit leaf. Only `impl`, `mod` and `trait` are containers (RULE 6); a marker whose
sub-blocks hang off a type definition is reported as such instead of as `UNCOVERED`,
because the fix is to make the type a leaf block (RULE 5), not to mark its declaration.

Before the first marker, the only permitted scaffolding is an optional UTF-8 BOM, an
optional first-line shebang, blank lines and crate-level inner attributes (`#![...]`).
An outer attribute, import, item or other Rust code there is reported as `UNCOVERED`.

Line endings are normalized to LF only for comparison. No other whitespace is normalized.
This catches stale, truncated, altered or reordered blocks, duplicate source
markers/fences, orphan fences and missing leaf fences. Synchronization parses raw text and
replaces only the byte range occupied by a valid fence body, using that fence's own EOL;
all prose, delimiters, mixed EOL and bytes outside the body remain exactly unchanged.
Structural errors prevent every write.

## Context metadata

Manifest schema 2 records paired paths and markers as `EXTRACTED`. Purpose, tests, risk
and indispensable dependency decisions are `INFERRED`. Invariants come only from explicit
`**Invariants**` bullets and `expects:` clauses in `**Dependencies**`: bullets prefixed
with `(EXTRACTED...)` retain `EXTRACTED`, while authored bullets and dependency
expectations are `INFERRED` and carry a `basis` that distinguishes the two sources.
Ambiguous prose is never promoted to an invariant.

Cross-repo contracts come only from `**Cross-repo contracts**` bullets whose first token
is a stable lowercase backticked identifier such as `collab-wire`. The manifest stores
sorted, unique identifiers, not repository-specific prose. Incidental mentions of a repo,
client or cross-repo relationship therefore add no entry, and companion files on both
sides can emit the same contract value.

Authored dependency and dependent bullets prefixed with `(EXTRACTED...)` retain their
explicit origin, including `:` or `;` detail suffixes.
Risk is classified as normal, persistence, protocol, security or migration with the matched
terms retained as its basis.

Context packs include a generated `context-pack.json`, fixed ZIP metadata and sorted input
paths. Size/file limits are checked before writing.

## Failure behavior

All validation errors are printed to stderr and return exit code 1. Commands write no
partial companion changes when a pair has structural errors. Manifest and ZIP outputs are
derived entirely from repository files.

## Tests

`python3 -m unittest discover -s scripts/tests -p 'test_*.py'` covers functions, attributes,
leaf impls, containers, test modules, one-line drift, stale/truncated/orphan/duplicate or
reordered fences, repeatable read-only checks, sync prose preservation and byte-for-byte
pack reproducibility. It also covers uncovered container and file preambles, allowed
initial scaffolding, mixed-EOL byte preservation and refusal paths, explicit symmetric
cross-repo identifiers, incidental-prose rejection, extracted/authored/expectation
invariants, ambiguous-prose rejection, type definitions used as containers, and detailed
`EXTRACTED` origin annotations.

## Related files

- `sync-companion-code` / `context-pack` — stable contributor-facing entry points.
- `tests/test_companion_tool.py` — regression suite.
- `check-docs.sh` — CI integration.
