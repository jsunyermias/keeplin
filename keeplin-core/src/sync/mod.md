# `sync/mod.rs` — sync module root

Self-contained companion for `keeplin-core/src/sync/mod.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able
to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — the file's single block: the child-module declaration and
re-exports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
mod engine;

pub use engine::{run_sync, SyncEngine, SyncStage};
```

**What it does** — The root of the `sync` sub-module: the synchronisation engine for
Keeplin. It declares the private `engine` child module and re-exports its public
surface — `SyncEngine` (orchestrates a complete push-then-pull synchronisation cycle
for any `crate::storage::StorageBackend`: collect local changes, push to the remote
peer, pull remote changes, apply locally, update the last-sync timestamp), the
`run_sync` free function, and the `SyncStage` progress enum — so callers write
`keeplin_core::sync::SyncEngine` instead of reaching into `engine`.

**Dependencies** — `engine` (this module's child, `sync/engine.rs`).

**Used by** — the daemon (`keeplin-daemon`) and any embedder driving a manual sync
cycle; `sync/engine.rs` is otherwise private.

**Repeated context** — Module-root convention of the crate: root files declare and
re-export, never implement. Deliberately minimal so future sync strategies (e.g.
peer-to-peer) can be added as sibling modules without changing the public interface.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
**Nodes/edges this file contributes** (top symbols by cross-file degree)

- (no symbols extracted for this file — it contributes only its file node) (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `mod engine;` + re-exports | `// md:Overview` |
