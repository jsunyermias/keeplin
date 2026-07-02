# `.github/workflows/ci.yml` — CI pipeline

## Purpose

This workflow validates every push and pull request by checking formatting, compiling the
workspace, running all tests, and linting with Clippy. It must pass on all commits to
`main` and on every commit to branches whose names start with `claude/`.

## Triggers

| Event | Branches / filters |
|-------|--------------------|
| `push` | `main`, `claude/**` |
| `pull_request` | target branch: `main` |

## Environment variables

| Variable | Value | Purpose |
|----------|-------|---------|
| `CARGO_TERM_COLOR` | `always` | Forces colour output in Cargo commands even in a non-terminal CI environment, making build output easier to read in GitHub Actions logs |
| `RUST_BACKTRACE` | `1` | Causes Rust to print a full stack backtrace when a test panics, making failures easier to diagnose |

## Jobs

### `test` — Check, Test & Lint

Runs on `ubuntu-latest`.

| Step | Action / Command | Purpose |
|------|-----------------|---------|
| Checkout | `actions/checkout@v4` | Clones the repository at the triggering commit |
| Install Rust | `dtolnay/rust-toolchain@stable` with `clippy, rustfmt` | Installs the latest stable Rust toolchain including the Clippy linter and `rustfmt` formatter |
| Cache | `Swatinem/rust-cache@v2` | Caches the Cargo registry, compiled dependencies, and build artifacts between runs to speed up subsequent builds |
| Install protoc | `sudo apt-get install -y protobuf-compiler` | Installs the Protocol Buffers compiler required by `keeplin-daemon/build.rs` |
| cargo fmt | `cargo fmt --check --all` | Verifies that all Rust source files in the workspace are formatted according to the project's `rustfmt` style. Fails the CI job if any file is not formatted. |
| cargo test (core) | `cargo test -p keeplin-core` | Runs all unit and integration tests in `keeplin-core`, including the `FsBackend`, `DbBackend`, and `EncryptedBackend` test suites |
| cargo test (daemon) | `cargo test -p keeplin-daemon` | Runs all tests in `keeplin-daemon`, including the `validate_basic_auth` unit tests in `main.rs` |
| cargo clippy | `cargo clippy --workspace --all-targets -- -D warnings` | Lints the entire workspace **including test and bench code** (matching the command the README tells contributors to run) and treats every warning as an error. Also fully subsumes the type-checking a separate `cargo check` step used to provide. |
| Install cargo-audit | `taiki-e/install-action@v2` (`tool: cargo-audit`) | Downloads a prebuilt `cargo-audit` binary; compiling it from source with `cargo install` added minutes to every run for no additional coverage |
| cargo audit | `cargo audit` | Checks `Cargo.lock` against the RustSec advisory database |

## Caching strategy

`Swatinem/rust-cache@v2` caches the following directories between runs:

- `~/.cargo/registry/` — downloaded crate sources
- `~/.cargo/git/` — git dependencies
- `target/` — compiled build artifacts (incremental compilation cache)

The cache key is derived from the Cargo lock file and the target platform. When
`Cargo.lock` changes (a dependency was added or updated), the cache is invalidated and
rebuilt from scratch.

## Notes

- `protoc` must be installed before anything compiles the workspace (`cargo test`,
  `cargo clippy`) because `keeplin-daemon/build.rs` invokes `tonic-build`, which in turn
  calls `protoc`.
- The workflow runs tests for each crate separately (`-p keeplin-core`, `-p keeplin-daemon`)
  rather than `--workspace` because the two test suites are logically independent and this
  makes it easier to identify which crate a failure belongs to.

## Related files

- `.github/workflows/` — directory containing all GitHub Actions workflow files
- `keeplin-daemon/build.rs` — the build script that requires `protoc`
- `keeplin-daemon/proto/keeplin.proto` — compiled by `protoc` during the build
