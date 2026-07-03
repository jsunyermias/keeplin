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

## Structure

<!-- The shape of the module: the main type(s), how they compose, the important invariant.
     Use a table for a method/field inventory; fence a signature when it clarifies. -->
{{The central type and its role. If it implements `StorageBackend` (or wraps one), say so
and note where it sits in the decorator stack. If it is pure/`no I/O`, say that.}}

| Item | Description |
|------|-------------|
| `{{fn_or_field}}` | {{what it does / why it exists}} |

## How it works

<!-- The mechanism worth explaining: the algorithm, the on-disk/on-wire layout, the locking
     discipline, the state machine. This is the section that earns its keep. -->
{{The mechanism a reader needs to hold in their head to change this file safely — the data
layout, the merge/resolve rule, the lock order, the lazy-build-then-maintain pattern, etc.}}

## Invariants & edge cases

<!-- The things that must stay true, and the non-obvious cases already handled. A future
     editor breaks the module by violating these. Delete if the module has none worth
     calling out. -->
- {{Invariant that must hold (e.g. "single writer per device log"; "the projection is only
  ever replaced atomically").}}
- {{Edge case handled deliberately (e.g. legacy `0` sentinel; corrupt file falls back to X;
  backward-compatible serde default).}}

## Concurrency & sync

<!-- Only if relevant: which lock guards what, what is safe to call concurrently, and how a
     change made here interacts with the sync/version-vector model. Delete otherwise. -->
{{Locking discipline and how state produced here participates in sync (or why it does not).}}

## Design notes

<!-- Rationale and rejected alternatives. Why this shape and not the obvious other one. -->
- {{Why a decision was made this way; the alternative that was rejected and why.}}

## Related files

- `{{path}}` — {{one-line reason a reader jumps here next}}
- `ARCHITECTURE.md` / `SECURITY.md` — {{the shared concept this module relies on}}
