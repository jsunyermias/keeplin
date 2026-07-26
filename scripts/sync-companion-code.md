# `scripts/sync-companion-code` — synchronize embedded Rust blocks

## Purpose

Thin, dependency-free launcher for the deterministic companion tool. It copies only the
contents of existing `rust` fences from their paired `.rs` blocks; prose, headings and
checklists are never rewritten. The write path operates on raw text offsets rather than a
normalized document, so mixed CRLF/LF outside a valid fence body is byte-preserved.

## Usage

- `./scripts/sync-companion-code --check` verifies the whole repository without writing.
- `./scripts/sync-companion-code path/to/file.rs` synchronizes one source/companion pair.
- `./scripts/sync-companion-code path/to/directory` synchronizes every pair below it.

The comparison normalizes line endings to LF and nothing else: indentation, blank lines,
attributes, signatures and bodies must otherwise be character-for-character identical.
Missing, duplicate and orphan fences are errors that require an author to repair the
companion structure; synchronization never invents sections or prose.

The source prefix before its first `// md:` marker may contain only an optional UTF-8 BOM,
an optional first-line shebang, blank lines and crate-level inner attributes (`#![...]`).
Other Rust code is `UNCOVERED`. A structural error causes a read-only failure: no fence is
updated.

## Related files

- `companion_tool.py` — implementation and failure rules.
- `check-docs.sh` — invokes this launcher in `--check` mode in CI.
- `../docs/templates/source-module.md` — the mechanically enforced companion contract.
