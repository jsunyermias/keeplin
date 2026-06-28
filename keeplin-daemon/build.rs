//! Build script for `keeplin-daemon`.
//!
//! This script runs before the Rust compiler processes any source file in the crate.
//! Its sole responsibility is to invoke `tonic-build`, which compiles
//! `proto/keeplin.proto` into Rust source code and writes it to Cargo's `OUT_DIR`.
//! The generated file is later included into the crate by `src/proto.rs` via
//! `tonic::include_proto!("keeplin")`.
//!
//! **Prerequisites:** `protoc` (the Protocol Buffers compiler) must be installed and
//! available on `PATH`. In CI, it is installed via
//! `sudo apt-get install -y protobuf-compiler`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        // Generate the server-side `KeeplinService` trait and `KeeplinServiceServer`
        // wrapper that `src/server.rs` implements and `src/main.rs` registers.
        .build_server(true)
        // Also generate client-side stubs so integration tests can call the daemon
        // over a real gRPC channel without pulling in a separate client crate.
        .build_client(true)
        .compile_protos(
            // The single proto file that defines the entire Keeplin gRPC API.
            &["proto/keeplin.proto"],
            // The include path from which `protoc` resolves any `import` statements
            // inside the proto file. Currently there are no imports, but the flag
            // must still point to a valid directory.
            &["proto/"],
        )?;
    Ok(())
}
