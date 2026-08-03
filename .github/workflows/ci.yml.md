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

The workflow has read-only access to repository contents, pull-request metadata and check
runs. The `checks: read` scope exists for the review-loop convergence step, which reads the
head commit's check runs to build the set of red required checks; without it that set would
read as empty and a failing pull request could be declared converged. Body edits retrigger
the workflow, so completing or removing review evidence — and editing the review ledger — is
reflected in the required check without a new commit.

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
| Test pull-request governance checks | `node --test` over `check-review-governance.test.js` and `check-review-loop.test.js` | Exercises the reviewed and maintainer-waiver paths, and the convergence, recurrence, advisory and stagnation paths, including negative cases. **Runs last**: a deliberately-red test for an open reified finding would otherwise abort the job at step one and hide whether everything else passed |

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

### `converge` — Review loop converged

Runs on `ubuntu-latest`, `needs: [test, graph]`, non-draft pull requests only. Implements
[keeplin ADR 0004](../../docs/adr/0004-review-loop-convergence.md).

| Step | Action / Command | Purpose |
|------|-----------------|---------|
| Checkout | `actions/checkout@v4` | Clones the repository at the triggering commit |
| Check review-loop convergence | `actions/github-script@v7` | Requires required checks green and zero open reified findings; escalates a stalled loop to `docs/review-stalls.md` instead of letting it iterate silently |

**Why this is a job and not a step.** Convergence asserts "the required checks are green". A
step inside `test` cannot make that claim: it runs before `cargo test`, Clippy and audit in
its own job, and while `graph` is still going, so it would be asserting greenness of work that
had not happened. `needs: [test, graph]` makes the claim checkable. `always()` keeps the job
reporting when a dependency fails, so branch protection sees a red check rather than a skipped
one.

The job name is a branch-protection required-check identifier, exactly like `graph`'s: adding
it to the workflow does not enforce it until it is added to the required-check list in
Settings → Branches.

It reads the `## Review ledger` section of the pull-request body, the changed files, and the
head commit's check runs, then decides one of five states:

| State | Meaning | Check |
|-------|---------|-------|
| `converged` | Required checks green and no reified finding open | passes |
| `awaiting-checks` | Nothing blocks, but a required check has not finished — an unfinished check is not a green check | fails |
| `converging` | The blocking set is non-empty but shrinking | fails |
| `escalated` | The loop state repeated, or the blocking set has not shrunk for `REVIEW_LOOP_STAGNATION_LIMIT` rounds | fails, and demands a `docs/review-stalls.md` `## Open` row naming this pull request *and every current blocker* |
| `malformed` | The ledger or round log contradicts the observed state | fails |

A pull request whose body has no `## Review ledger` section at all is round zero, not
malformed — ADR 0004's migration contract for pull requests opened before the ledger existed.

`REVIEW_LOOP_STAGNATION_LIMIT` is set to `3` on the step and falls back to the script's
`DEFAULT_STAGNATION_LIMIT`. The brake measures state, not elapsed time: the loop-state hash
is `sha256(normalized diff ‖ open reified finding IDs ‖ red check names)`, where the
normalized diff is the changed paths with their blob SHAs, sorted, so commit ordering does
not affect it. Fields and list entries are joined with `\x1e` and `\x1f` rather than commas,
because check-run names contain commas — `Check, Test & Lint` does.

This job is a floor beneath `Check pull-request review governance`, never a substitute for
it. The two are conjunctive: a pull request can converge and still be unmergeable for want of
an independent reviewer.

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
- `docs/adr/0004-review-loop-convergence.md` — the accepted decision this step implements
- `docs/review-stalls.md` — the durable record of escalated loops
- `.github/workflows/` — directory containing all GitHub Actions workflow files
- `keeplin-daemon/build.rs` — the build script that requires `protoc`
- `keeplin-daemon/proto/keeplin.proto` — compiled by `protoc` during the build
