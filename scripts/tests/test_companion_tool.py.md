# `scripts/tests/test_companion_tool.py` — fidelity-tool regression suite

## What is tested

Temporary copies of the checked-in fixtures exercise every supported block shape and every
failure required by issue #147. Tests call the standard-library implementation directly;
they use no network, API, Rust compiler or LLM.

The suite proves valid function/attribute/impl/container/test-module extraction, one-line
source drift, stale/truncated/orphan/duplicate/reordered fences, duplicate source markers,
repeatable read-only check mode, fence-only synchronization with prose preservation, and
byte-for-byte identical ZIP output across two builds.

Dedicated positive and negative fixtures also prove all four `orden-04b` guarantees:
stable cross-repo contract identifiers are symmetric while incidental repo prose is
ignored; authored/extracted invariants and explicit dependency `expects:` clauses are
covered while ambiguous prose is ignored; only BOM/shebang/blank/inner-attribute
scaffolding may precede the first marker; and valid mixed-EOL fence sync preserves every
byte outside the body while invalid structure performs no write.

The `orden-04c` fixtures add positive and negative coverage for `# md:` shell sources,
extensionless shell launchers, uncovered shell preambles, unsupported-format exclusion,
and byte-preserving fence synchronization. SQL fixtures prove that a migration requires
one companion with one complete verbatim `sql` fence, that stale content is repaired only
in the companion, and that adding `-- md:` markers is rejected.

## Fixtures

Fixtures intentionally use `.fixture` suffixes so repo-wide companion discovery does not
mistake test data for product source. Each source/companion case is copied or decoded into
an isolated temporary repository with real suffixes. Mixed-EOL byte sequences are stored
as JSON escapes so the checked-in fixture itself is portable across Git clients.

## Run

`python3 -m unittest discover -s scripts/tests -p 'test_*.py'`

## Related files

- `../companion_tool.py` — code under test.
- `../sync-companion-code` and `../context-pack` — contributor-facing launchers.
