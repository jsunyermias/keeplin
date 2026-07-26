# `proto.rs` — generated Protocol Buffers / gRPC code

Self-contained companion for `keeplin-daemon/src/proto.rs`. It documents **every code block of
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

**Identification** — the file's single block: the generated-code module. Marker
`// md:Overview` (on the `pub mod keeplin` item — this file has no imports).

**Code** — complete and verbatim:

```rust
// md:Overview
#[allow(dead_code, clippy::all)]
pub mod keeplin {
    tonic::include_proto!("keeplin");
}
```

**What it does** — Splices in the Rust produced at compile time by `tonic-build`
(see `build.rs`) from `proto/keeplin.proto`; the code is written to Cargo's
`OUT_DIR` and included here. **Never edit the generated code** — change
`proto/keeplin.proto` instead (its own companion `proto/keeplin.proto.md`
documents the service surface). Notable exports: the protobuf message structs
(`keeplin::Note`, `Notebook`, `Tag`, `Resource`, …);
`keeplin::keeplin_service_server::KeeplinService` — the async trait
`src/server.rs` implements; `KeeplinServiceServer` — the tonic wrapper
registered in `src/main.rs`; `keeplin::SyncProgress` /
`sync_progress::Stage` — the server-streaming `Sync` RPC types. The `#[allow]`
silences `dead_code` (client stubs from `build_client(true)` the daemon never
calls) and `clippy::all` (generated code does not follow hand-written style).

**Dependencies** — `tonic` (`include_proto!`), `build.rs` + `protoc` at build
time.

**Used by** — `server.rs` (implements the service trait and converts
proto ↔ model types), `main.rs` (registers the server).

**Repeated context** — protobuf evolution follows the same additive-only rule
as the sync journal: field numbers are never reused, old peers ignore unknown
fields (proto3 defaults), and a breaking change is expressed as a
`PROTOCOL_VERSION`-style break, not dual formats.

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

- `keeplin` (generated module) — defined here (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `proto/keeplin.proto` via `build.rs` (INFERRED — generated at build time)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/server.rs` — implements the generated service trait (EXTRACTED)
- `keeplin-daemon/src/main.rs` — registers `KeeplinServiceServer` (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `pub mod keeplin { tonic::include_proto!… }` | `// md:Overview` |
