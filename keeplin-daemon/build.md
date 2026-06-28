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

## Related files

- `keeplin-daemon/proto/keeplin.proto` — the Protocol Buffers service definition that
  this script compiles
- `keeplin-daemon/src/proto.rs` — includes the generated code into the crate
- `keeplin-daemon/src/server.rs` — implements the generated `KeeplinService` trait
- `.github/workflows/ci.yml` — installs `protoc` before running `cargo check`
