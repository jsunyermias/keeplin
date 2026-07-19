# `build.rs` — keeplin-daemon build script

Self-contained companion for `keeplin-daemon/build.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this
file must be able to understand and modify the module without opening anything else, so
project-wide conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in the `.rs` carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section here;
grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

This build script runs before the Rust compiler processes any source file in the crate. Its
sole responsibility is to invoke `tonic-build`, which compiles `proto/keeplin.proto` into Rust
source and writes it to Cargo's `OUT_DIR`; the generated file is later included into the crate
by `src/proto.rs` via `tonic::include_proto!("keeplin")`. **Prerequisite:** `protoc` (the
Protocol Buffers compiler) must be installed and on `PATH` — in CI it is installed via
`sudo apt-get install -y protobuf-compiler`.

---

## main

**Identification** — `fn main() -> Result<(), Box<dyn std::error::Error>>`, the build-script
entry point; marker `// md:main`.

**Code** — complete and verbatim:

```rust
// md:main
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &["proto/keeplin.proto"],
            &["proto/"],
        )?;
    Ok(())
}
```

**What it does** — Configures `tonic-build` and compiles the single service definition
`proto/keeplin.proto` at build time. `build_server(true)` generates the server-side
`KeeplinService` async trait + `KeeplinServiceServer` wrapper that `src/server.rs` implements
and `src/main.rs` registers; `build_client(true)` also generates client-side stubs so
integration tests can call the daemon over a real gRPC channel without a separate client
crate. `compile_protos` takes the proto file list (`proto/keeplin.proto`) and the include path
(`proto/`) from which `protoc` resolves any `import` statements — there are currently no
imports, but the flag must still point at a valid directory. `tonic-build` writes the
generated Rust to `$OUT_DIR/keeplin.rs`; the exact path never matters to application code
because `tonic::include_proto!("keeplin")` resolves it. The `?` propagates any compile error
(e.g. `protoc` missing) as a build failure; on success it returns `Ok(())`. A change to
`keeplin.proto` triggers a rebuild of `keeplin-daemon` only — `keeplin-core` has no build
script and no proto dependency.

**Dependencies** —
- `tonic_build::configure` — the codegen builder; expects `.build_server`/`.build_client` to
  keep emitting the `keeplin_service_server::KeeplinService` trait names that `src/server.rs`
  implements and `src/proto.rs` includes. If tonic renamed those generated items, `server.rs`
  would fail to compile — this file and `server.rs` share that generated contract.
- `tonic_build::Builder::compile_protos` — runs `protoc`; expects `protoc` on `PATH` and the
  proto file to exist at `proto/keeplin.proto`. A missing `protoc` surfaces here as a build
  error, not a silent skip.
- `std::error::Error` (boxed) — the error type propagated by `?`; expects tonic's error to
  implement it (it does).

**Used by** — Cargo runs this automatically before compiling the crate; its output is consumed
by `src/proto.rs` (`tonic::include_proto!`). No Rust caller references `main` directly.

**Repeated context** — Proto evolution is **additive-only**: add new fields with new tags,
never reuse or renumber existing tags, so old peers ignore unknown fields. The wire/protocol
version is a clean-break surface (see `ARCHITECTURE.md`), but individual proto messages evolve
additively within a version.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every companion
because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the navigation model,
the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with `graphify update .`
after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `main()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph; the coupling to `proto.rs`/`server.rs` is via generated code, not a
  graph edge) (INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated elsewhere)

- Compiles `proto/keeplin.proto` with `tonic-build` at build time; requires `protoc` on PATH.
- Proto evolution is additive-only: new fields with new tags, never reuse/renumber (old peers
  ignore unknown fields).
- `build_server(true)` must stay on: `src/server.rs` implements the generated server trait.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `fn main` | `// md:main` |
