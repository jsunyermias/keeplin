# `scripts/build.sh` — cross-compilation script

## Purpose

This script builds `keeplin-daemon` for multiple target platforms in a single invocation
and places the resulting binaries in a `dist/` directory at the workspace root. It is
intended for producing distribution-ready release binaries that do not depend on the host
system's C library (musl targets are statically linked).

## Usage

```sh
./scripts/build.sh
```

The script accepts no command-line arguments. All configuration (target list, output
directory) is hard-coded inside the script.

## Environment variables

| Variable | Required | Description |
|----------|----------|-------------|
| None | — | The script does not read any environment variables. The Rust toolchain and cross-compilers must be installed and available on `PATH` before running the script. |

## Arguments

This script takes no arguments.

## Steps

1. **Set the binary name and output directory.** Sets `BINARY="keeplin-daemon"` and
   creates `dist/` if it does not already exist.
2. **Iterate over the target list.** The enabled targets are:
   - `x86_64-unknown-linux-musl` — 64-bit Linux, statically linked
   - `aarch64-unknown-linux-musl` — 64-bit ARM Linux, statically linked
   - `x86_64-pc-windows-gnu` — 64-bit Windows (MinGW)
   - macOS and Android targets are commented out and must be enabled manually when the
     required SDKs are available.
3. **Build each target.** For each target, runs:
   ```sh
   cargo build --release --target "$TARGET" -p keeplin-daemon
   ```
4. **Determine the source path.** On Windows targets the binary has a `.exe` extension;
   on all other targets it has no extension.
5. **Copy the binary.** Copies the built binary to
   `dist/keeplin-daemon-<target>[.exe]`.
6. **Report completion.** Prints the path of the copied binary for each target, then a
   final summary line.

## Prerequisites

The following tools must be installed and on `PATH` before running the script:

| Tool | Required for |
|------|-------------|
| Rust + Cargo (stable) | All targets |
| `x86_64-linux-musl-gcc` | `x86_64-unknown-linux-musl` |
| `aarch64-linux-musl-gcc` | `aarch64-unknown-linux-musl` |
| `x86_64-w64-mingw32-gcc` | `x86_64-pc-windows-gnu` |

Add targets to the Rust toolchain with:
```sh
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-gnu
```

## Notes

- **`set -euo pipefail`** — the script aborts immediately if any command fails (`-e`),
  treats unset variables as errors (`-u`), and propagates pipe failures (`-pipefail`).
- The `protoc` (Protocol Buffers compiler) must be installed because
  `keeplin-daemon/build.rs` invokes it during compilation.
- This script is not run in CI. CI (`ci.yml`) only checks the native target.

## Related files

- `.cargo/config.toml` — defines cross-compilation linker paths (commented out; must be
  enabled to match the toolchains above)
- `keeplin-daemon/build.rs` — the build script that requires `protoc`
- `.github/workflows/ci.yml` — the CI pipeline (native target only, no cross-compilation)
