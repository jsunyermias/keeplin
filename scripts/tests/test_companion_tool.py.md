# `scripts/tests/test_companion_tool.py` — fidelity-tool regression suite

## What is tested

Temporary copies of the checked-in fixtures exercise every supported block shape and every
failure required by issue #147. Tests call the standard-library implementation directly;
they use no network, API, Rust compiler or LLM.

The suite proves valid function/attribute/impl/container/test-module extraction, one-line
source drift, stale/truncated/orphan/duplicate/reordered fences, duplicate source markers,
repeatable read-only check mode, fence-only synchronization with prose preservation, and
byte-for-byte identical ZIP output across two builds.

## Fixtures

`fixtures/shapes.rs.fixture` and `shapes.md.fixture` intentionally use non-`.rs`/`.md`
suffixes so repo-wide companion discovery does not mistake test data for product source.
Each test copies them to an isolated temporary repository with real suffixes.

## Run

`python3 -m unittest discover -s scripts/tests -p 'test_*.py'`

## Related files

- `../companion_tool.py` — code under test.
- `../sync-companion-code` and `../context-pack` — contributor-facing launchers.
