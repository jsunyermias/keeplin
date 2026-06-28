# `Cargo.toml` — keeplin-daemon

## Crate purpose

`keeplin-daemon` is the binary crate that exposes the Keeplin storage layer as a gRPC
service. It depends on `keeplin-core` for all domain logic and adds only the network
transport layer (tonic + Protocol Buffers), the configuration parser, and the
authentication interceptor.

## Runtime dependencies

| Crate | Version | Why |
|-------|---------|-----|
| `keeplin-core` | (path) | All domain models, storage backends, encryption, and sync engine |
| `tokio` | workspace | Async runtime for the gRPC server |
| `serde` | workspace | Used by `Config` for TOML deserialisation |
| `serde_json` | workspace | Not used directly; present for indirect dependencies |
| `chrono` | workspace | Timestamp parsing in `server.rs` |
| `uuid` | workspace | UUID parsing in RPC handlers |
| `thiserror` | workspace | Not used directly; present for `keeplin-core` re-exports |
| `anyhow` | workspace | Error propagation in `main()` and `Config::from_file` |
| `tracing` | workspace | Structured log emission in `main.rs` |
| `tracing-subscriber` | workspace | Log formatting and `RUST_LOG` filter support |
| `toml` | workspace | TOML parsing for `keeplin.toml` |
| `tonic` | 0.12 (`tls`) | gRPC framework; `tls` feature enables `ServerTlsConfig` |
| `prost` | 0.13 | Protocol Buffers runtime; used implicitly by tonic-generated code |
| `clap` | 4 (`derive`) | Command-line argument parsing for the `--config` flag |
| `futures-core` | 0.3 | `Stream` trait used in the server-streaming sync RPC |
| `tokio-stream` | 0.1 | `ReceiverStream` wrapper for the `mpsc::Receiver` in the sync RPC |
| `base64` | 0.22 | Base64 decoding of the `Authorization: Basic` header |
| `subtle` | 2 | Constant-time byte comparison for HTTP Basic Auth to prevent timing attacks |

## Dev / build dependencies

| Crate | Version | Why |
|-------|---------|-----|
| `tonic-build` | 0.12 | Compile-time Protocol Buffers → Rust code generation (runs in `build.rs`) |

## Feature flags

No user-facing feature flags. TLS support is always compiled in via `tonic`'s `tls`
feature.

## Build-time notes

- `build.rs` invokes `tonic-build` to compile `proto/keeplin.proto`. This requires
  `protoc` (the Protocol Buffers compiler) to be installed on the host.
- `prost = "0.13"` must be kept in sync with the version of `prost` that `tonic 0.12`
  depends on; if versions diverge, duplicate generated-code symbols will cause link
  errors.

## Related files

- `Cargo.toml` (workspace root) — shared dependency versions
- `keeplin-core/Cargo.toml` — the library this crate depends on
- `keeplin-daemon/build.rs` — the build script that compiles the proto file
- `keeplin-daemon/proto/keeplin.proto` — the Protocol Buffers service definition
