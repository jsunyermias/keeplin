# `scripts/sync-companion-code` — synchronize embedded Rust blocks

## Purpose

Thin, dependency-free launcher for the deterministic companion tool. It copies only the
contents of existing `rust` fences from their paired `.rs` blocks; prose, headings and
checklists are never rewritten.

## Usage

- `./scripts/sync-companion-code --check` verifies the whole repository without writing.
- `./scripts/sync-companion-code path/to/file.rs` synchronizes one source/companion pair.
- `./scripts/sync-companion-code path/to/directory` synchronizes every pair below it.

The comparison normalizes line endings to LF and nothing else: indentation, blank lines,
attributes, signatures and bodies must otherwise be character-for-character identical.
Missing, duplicate and orphan fences are errors that require an author to repair the
companion structure; synchronization never invents sections or prose.

## Related files

- `companion_tool.py` — implementation and failure rules.
- `check-docs.sh` — invokes this launcher in `--check` mode in CI.
- `../docs/templates/source-module.md` — the mechanically enforced companion contract.
