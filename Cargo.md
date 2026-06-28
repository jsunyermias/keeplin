# `Cargo.toml` — workspace root

## Crate purpose

This is the Cargo workspace manifest. It declares the two member crates
(`keeplin-core` and `keeplin-daemon`) and pins shared dependency versions in
`[workspace.dependencies]` so that both crates always use the same versions of
common libraries without repeating version strings in each crate's own `Cargo.toml`.

## Workspace members

| Crate | Path | Role |
|-------|------|------|
| `keeplin-core` | `keeplin-core/` | Library: domain models, storage backends, encryption, sync engine |
| `keeplin-daemon` | `keeplin-daemon/` | Binary: gRPC server that exposes `keeplin-core` over the network |

## Workspace-level shared packages

| Package field | Value | Description |
|---------------|-------|-------------|
| `version` | `0.1.0` | Shared across all crates; bump all at once |
| `edition` | `2021` | Rust edition used by all crates |
| `authors` | `Keeplin Contributors` | Default author string |
| `license` | `MIT` | SPDX identifier |

## Runtime dependencies (shared)

| Crate | Version | Why |
|-------|---------|-----|
| `tokio` | 1 (full features) | Async runtime; used everywhere |
| `serde` | 1 (derive) | Serialisation / deserialisation of all domain types |
| `serde_json` | 1 | JSON encoding for log files, HTTP payloads, and change journal |
| `chrono` | 0.4 (serde) | UTC timestamps on all domain types |
| `uuid` | 1 (v4 + serde) | UUID v4 IDs for all entities |
| `thiserror` | 1 | Derive macros for the `StorageError` and `SyncError` enums |
| `anyhow` | 1 | Error propagation in `main.rs` and `Config::from_file` |
| `async-trait` | 0.1 | Enables `async fn` in trait definitions (Rust < 1.75 limitation) |
| `tracing` | 0.1 | Structured logging throughout the codebase |
| `tracing-subscriber` | 0.3 (env-filter) | Log formatting and filtering, configured in `main.rs` |
| `toml` | 0.8 | Parsing the `keeplin.toml` configuration file |

## Dev / build dependencies (shared)

| Crate | Version | Why |
|-------|---------|-----|
| `tempfile` | 3 | Creates temporary directories in integration tests |

## Resolver

`resolver = "2"` uses Cargo's feature-resolver v2, which avoids activating features
required by dev-dependencies in production builds. This is particularly important for
`tokio`, which is declared with `features = ["full"]` as a dev-dependency but only needs
a subset of features in library code.

## Release profile

`[profile.release]` is declared in this manifest (not in `.cargo/config.toml`, where
Cargo would ignore it): `opt-level = 3`, `lto = true`, `codegen-units = 1`, and
`strip = true` for a small, fully-optimised daemon binary.

## Related files

- `keeplin-core/Cargo.toml` — library-specific dependencies
- `keeplin-daemon/Cargo.toml` — binary-specific dependencies
- `.cargo/config.toml` — cross-compilation linker settings (profiles live here in `Cargo.toml`)
