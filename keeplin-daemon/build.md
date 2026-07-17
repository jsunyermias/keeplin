# `build.rs` — keeplin-daemon build script

## Purpose

This build script runs at compile time (before the Rust compiler processes `src/`) and
uses `tonic-build` to compile the Protocol Buffers service definition at
`proto/keeplin.proto` into Rust source code. The generated Rust file is written into
Cargo's `OUT_DIR` and included into the crate via the `tonic::include_proto!` macro in
`src/proto.rs`.

## What it generates

`tonic-build` produces two categories of Rust code from `keeplin.proto`:

1. **Message types** — one Rust struct per `message` in the `.proto` file (e.g.
   `Note`, `CreateNoteRequest`, `ListNotesResponse`). Fields are mapped from proto3
   scalar types to Rust primitives.
2. **Service stubs** — a `KeeplinServiceServer` trait and the
   `keeplin_service_server::KeeplinService` async trait, which `keeplin-daemon/src/server.rs`
   implements.

## Configuration

```rust
tonic_build::configure()
    .build_server(true)   // generate server-side code (trait + registration wrapper)
    .build_client(true)   // generate client-side code (useful for integration tests)
    .compile_protos(
        &["proto/keeplin.proto"],  // input: the single proto file
        &["proto/"],               // include path: directory where imports are resolved
    )?;
```

## Build-time notes

- The build script requires `protoc` (the Protocol Buffers compiler) to be installed and
  available on `PATH`. In CI, it is installed via `sudo apt-get install protobuf-compiler`.
- The generated file is placed in `$OUT_DIR/keeplin.rs` by `tonic-build`. The exact path
  is not relevant to application code; `tonic::include_proto!("keeplin")` resolves it
  automatically.
- Changes to `keeplin.proto` trigger a rebuild of `keeplin-daemon` but not of
  `keeplin-core` (which has no build script and no proto dependency).

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `main()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Compiles `proto/keeplin.proto` with `tonic-build` at build time; requires `protoc` on PATH.
- Proto evolution is additive-only: new fields with new tags, never reuse/renumber (old peers ignore unknown fields).

## Related files

- `keeplin-daemon/proto/keeplin.proto` — the Protocol Buffers service definition that
  this script compiles
- `keeplin-daemon/src/proto.rs` — includes the generated code into the crate
- `keeplin-daemon/src/server.rs` — implements the generated `KeeplinService` trait
- `.github/workflows/ci.yml` — installs `protoc` before running `cargo check`
