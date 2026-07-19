# `lib.rs` — keeplin-core crate root

Self-contained companion for `keeplin-core/src/lib.rs`. It documents **every code block
of the source file, in source order** — a reader with only this file must be able to
understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — the file's single block: the crate's module declarations. Marker
`// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
pub mod collab;
pub mod compat;
pub mod encryption;
pub mod error;
pub mod history;
pub mod interop;
pub mod linking;
pub mod links;
pub mod migrate;
pub mod models;
pub mod ordering;
pub mod storage;
pub mod sync;
```

**What it does** — The crate root of `keeplin-core`, the library every other Keeplin
crate depends on: the complete storage, linking, ordering, collaboration and
synchronisation layer. No logic — only the public module declarations. The module
map:

| Module | Role |
|--------|------|
| `collab` | `CollabBackend` decorator — client of keeplin-srv's real-time note channel (line ops, presence, cursors), with the `protocol` (wire types) and `state` (line mirror + body↔lines diff) children |
| `compat` | the keeplin-srv `GET /version` handshake: `PROTOCOL_VERSION`, the `compatible_with` rule, `negotiate` — the one place this repo defines server-protocol compatibility |
| `encryption` | `EncryptedBackend<B>`: transparent AES-256-GCM at-rest encryption decorator for any `storage::StorageBackend` |
| `error` | all error types used across the crate (`StorageError`, `SyncError`) |
| `history` | change-history reads + forward-revert on top of the entity logs |
| `interop` | vCard & iCalendar format compatibility (contacts/events as resources) |
| `linking` | `LinkingBackend<B>`: derives bookmarks/links from note bodies, resolves `#…` references, enforces alias uniqueness |
| `links` | pure bookmark/link types and the `#…` reference + `[t](###)` bookmark grammar (I/O-free) |
| `migrate` | one-shot state copy between any two backends (e.g. `FsBackend ↔ DbBackend`) |
| `models` | domain types (`Note`, `Notebook`, `Tag`, `Resource`, `Change`, …) |
| `ordering` | the inbox system notebook, pinning, manual sort keys, starring: pure placement rules + the read-modify-write operations the API surfaces call |
| `storage` | the `StorageBackend` trait plus `FsBackend` and `DbBackend` implementations and the `note_log` version-vector resolution |
| `sync` | `SyncEngine`: orchestrates a full push-then-pull sync cycle |

Intra-crate dependency shape: `error` and `links` are leaves; `models` uses
`error`+`links`; `storage::backend` uses `error`+`models`; the decorators
(`encryption`, `linking`, `collab`, daemon-side `EventBackend`/metrics) wrap any
`StorageBackend`; `sync::engine` drives one.

**Dependencies** — none beyond its own submodules.

**Used by** — everything: `keeplin-daemon` (the binary), `keeplin-srv` (pins this
crate for `note_log::resolve`, `models::Change` and the collab protocol mirror), the
crate's tests.

**Repeated context** — Crate conventions restated: no re-exports at the crate root
(imports name their origin module, e.g. `keeplin_core::models::Note`); every `.rs`
has a companion `.md` enforced by `scripts/check-docs.sh`; a new backend only
implements `StorageBackend` in a new sub-module — no changes here beyond the
declaration.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- (no symbols extracted for this file — it contributes only its file node) (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | module declarations (`pub mod …` ×13) | `// md:Overview` |
