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

Line endings are normalized to LF. No other whitespace is normalized. This catches stale,
truncated, altered or reordered blocks, duplicate source markers/fences, orphan fences and
missing leaf fences. Synchronization replaces only a valid existing fence body and refuses
to paper over structural errors.

## Context metadata

The generated manifest records the paired paths and markers as `EXTRACTED`. Purpose,
invariants, tests, risk, indispensable dependency decisions and cross-repo contracts are
labelled `INFERRED`, because they come from authored prose or deterministic heuristics.
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
pack reproducibility.

## Related files

- `sync-companion-code` / `context-pack` — stable contributor-facing entry points.
- `tests/test_companion_tool.py` — regression suite.
- `check-docs.sh` — CI integration.
