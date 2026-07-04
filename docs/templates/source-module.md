<!--
  TEMPLATE: companion doc for a source module (`foo.rs` -> `foo.md`, same directory).
  Use for any `.rs` file that carries real logic (a backend, a decorator, an engine, a
  server surface). For a crate root that only wires modules, use `crate-root.md` instead.
  Delete every HTML comment and every section that does not apply before committing.
-->
# `{{path/to/module.rs}}` — {{one-line purpose}}

## Purpose

<!-- One short paragraph: what this module is responsible for and where it sits in the
     system. Answer "why does this file exist?" for someone who has never seen it. -->
{{What the module does, in two or three sentences. Name the key type(s) it defines and the
trait(s) it implements or the decorator it is.}}

## Key types

<!-- The shape of the module: the central type(s), what they define/implement, and how they
     compose. A short inventory table is the house style; fence a signature when it clarifies.
     If the type implements `StorageBackend` (or wraps one), say where it sits in the decorator
     stack; if the module is pure (no I/O), say that. -->
| Type | Kind | Description |
|------|------|-------------|
| `{{Type}}` | {{struct/enum/trait}} | {{what it is / the trait it implements / the decorator it is}} |

## Public API

<!-- The functions/methods a caller uses, grouped or listed with a one-line contract each.
     A table (or `### fn signature` subsections for the load-bearing ones) is the house style.
     Note anything non-obvious about a signature: what it validates, when it errors, what it
     ignores (e.g. "sets `updated_at = now()`, ignoring the client value"). -->
| Function | Description |
|----------|-------------|
| `{{fn(...) -> ...}}` | {{what it does; its pre/post-conditions or error cases}} |

## {{Module-specific mechanism}}

<!-- Add one or more sections named for THIS module's load-bearing mechanism — the thing a
     reader must hold in their head to change the file safely. Examples actually used in this
     repo: "Directory layout", "Database schema", "WebSocket protocol", "The note model —
     per-device logs + version-vector merge", "Concurrency — `note_write_lock`", "Atomic write
     pattern", "`apply_change` — all N variants", "Startup security checks". Fold invariants,
     edge cases, and the locking/sync discipline into the relevant section rather than into
     fixed generic headings. Delete this placeholder heading; use real names. -->
{{The algorithm, on-disk/on-wire layout, lock order, state machine, or resolution rule — plus
the invariants that must stay true and the deliberate edge cases handled here.}}

## Design notes

<!-- Rationale and rejected alternatives. Why this shape and not the obvious other one. -->
- {{Why a decision was made this way; the alternative that was rejected and why.}}

## Related files

- `{{path}}` — {{one-line reason a reader jumps here next}}
- `ARCHITECTURE.md` / `SECURITY.md` — {{the shared concept this module relies on}}
