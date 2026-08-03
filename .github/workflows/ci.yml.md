# `.github/workflows/ci.yml` — CI pipeline

## Purpose

This workflow validates every push and pull request by checking pull-request review
governance, formatting, compiling the workspace, running all tests, and linting with
Clippy. It must pass on all commits to `main` and on every commit to branches whose names
start with `claude/`.

## Triggers

| Event | Branches / filters |
|-------|--------------------|
| `push` | `main`, `claude/**` |
| `pull_request` | target branch: `main`; events `opened`, `synchronize`, `reopened`, `edited`, `ready_for_review` |

The workflow explicitly has read-only access to repository contents and pull-request metadata.
Its adversarial permission canary attempts to patch its own check run and fails unless GitHub
returns 403. The separate default-branch `review-loop-evaluator.yml` consumes completed runs.

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
| Prove pull-request token cannot rewrite check runs | API `PATCH` canary (pull requests only) | Requires HTTP 403; any successful mutation fails CI |
| Check pull-request review governance | `actions/github-script@v7` (non-draft pull requests only) | Requires either an independent review with evidence, or a complete maintainer waiver whose exact PR is recorded in the changed `docs/review-debt.md` |
| Install Python | `actions/setup-python@v5` (`3.12`) | Provides the standard-library runtime used by the deterministic companion checks |
| Check companion docs | `./scripts/check-docs.sh` | Enforces structure, exact source↔fence fidelity and the generated context manifest (the two-layer navigation model) |
| Test companion tooling | `python3 -m unittest discover -s scripts/tests -p 'test_*.py'` | Exercises syntax fixtures, drift/error detection, fence-only sync and reproducible packs |
| Install Rust | `dtolnay/rust-toolchain@stable` with `clippy, rustfmt` | Installs the latest stable Rust toolchain including the Clippy linter and `rustfmt` formatter |
| Cache | `Swatinem/rust-cache@v2` | Caches the Cargo registry, compiled dependencies, and build artifacts between runs to speed up subsequent builds |
| Install protoc | `sudo apt-get install -y protobuf-compiler` | Installs the Protocol Buffers compiler required by `keeplin-daemon/build.rs` |
| cargo fmt | `cargo fmt --check --all` | Verifies that all Rust source files in the workspace are formatted according to the project's `rustfmt` style. Fails the CI job if any file is not formatted. |
| cargo test (core) | `cargo test -p keeplin-core` | Runs all unit and integration tests in `keeplin-core`, including the `FsBackend`, `DbBackend`, and `EncryptedBackend` test suites |
| cargo test (daemon) | `cargo test -p keeplin-daemon` | Runs all tests in `keeplin-daemon`, including the `validate_basic_auth` unit tests in `main.rs` |
| cargo clippy | `cargo clippy --workspace --all-targets -- -D warnings` | Lints the entire workspace **including test and bench code** (matching the command the README tells contributors to run) and treats every warning as an error. Also fully subsumes the type-checking a separate `cargo check` step used to provide. |
| Install cargo-audit | `taiki-e/install-action@v2` (`tool: cargo-audit`) | Downloads a prebuilt `cargo-audit` binary; compiling it from source with `cargo install` added minutes to every run for no additional coverage |
| cargo audit | `cargo audit` | Checks `Cargo.lock` against the RustSec advisory database |
| Test pull-request governance checks | `node --test` over `check-review-governance.test.js` and `check-review-loop.test.js` | Exercises governance, trusted-evaluator isolation, verified disposal, required-job and bounded-journal behavior |

### `graph` — Knowledge graph up to date

Runs on `ubuntu-latest`, in parallel with `test` (no Rust toolchain needed). Enforces
LAYER 1 of the navigation model by generating it from the exact checked-out commit,
validating its focused corpus and reproducibility, and publishing the ignored output.

| Step | Action / Command | Purpose |
|------|-----------------|---------|
| Checkout | `actions/checkout@v4` | Clones the repository at the triggering commit |
| Install Python | `actions/setup-python@v5` (`python-version: 3.12`) | Provides the Python runtime graphify needs |
| Install graphify | `python -m pip install "graphifyy==0.9.25"` | Installs the pinned extractor used for every CI artifact |
| Generate and validate knowledge graph | `./scripts/check-graph.sh` (env `GRAPHIFY_REQUIRED=1`) | Builds twice, verifies same-tree reproducibility, corpus exclusions, cross-file edges, domain hubs, and report quality |
| Publish knowledge graph | `actions/upload-artifact@v4` | Publishes the complete `graphify-out/` directory as `knowledge-graph-<commit SHA>` for 14 days, including hidden Graphify metadata |

### Trusted convergence

The former head-controlled `converge` job is removed. The default-branch
[`review-loop-evaluator.yml`](review-loop-evaluator.yml) workflow is the authoritative evaluator
after this unprivileged workflow completes. Only `Check, Test & Lint` and
`Knowledge graph up to date` count, and each must positively report `success`.

The trusted evaluator remains a floor beneath review governance, never a substitute for an
independent reviewer. Fork pull requests deliberately fail closed rather than evaluate partial
journal evidence.

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
- The workflow runs tests for each crate separately (`-p keeplin-core`, `-p keeplin-daemon`,
  rather than `--workspace` because the suites are logically independent and
  this makes it easier to identify which crate a failure belongs to.

## Related files

- `.github/scripts/check-review-loop.js` — the convergence and stagnation evaluator
- `.github/scripts/check-review-governance.js` — the independent-review and waiver evaluator
- `docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md` — accepted decision
- `.github/workflows/review-loop-evaluator.yml` — default-branch authoritative evaluator
- `docs/review-stalls.md` — the durable record of escalated loops
- `.github/workflows/` — directory containing all GitHub Actions workflow files
- `keeplin-daemon/build.rs` — the build script that requires `protoc`
- `keeplin-daemon/proto/keeplin.proto` — compiled by `protoc` during the build
