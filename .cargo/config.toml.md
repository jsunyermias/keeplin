# `.cargo/config.toml` — workspace Cargo configuration

## Purpose

This file applies workspace-wide Cargo settings that override Cargo's built-in defaults
for every crate in the workspace. Settings here affect build profiles and can optionally
configure linkers for cross-compilation targets.

## Sections

### `[profile.release]`

Controls how the workspace's release build (`cargo build --release`) is compiled.

| Key | Value | Effect |
|-----|-------|--------|
| `opt-level` | `3` | Maximum speed optimisation (same as the Cargo default for release; explicit here for clarity) |
| `lto` | `true` | Enables Link-Time Optimisation across the entire binary; reduces binary size and can improve runtime performance by allowing the linker to inline across crate boundaries |
| `codegen-units` | `1` | Compiles all code in a single code-generation unit; required for full LTO effectiveness and produces smaller, faster binaries at the cost of longer compile times |
| `strip` | `true` | Strips debug symbols from the final binary; reduces binary size significantly (useful for distributing binaries without debug information) |

### `[target.<triple>]` — cross-compilation (commented out)

Five cross-compilation targets are pre-configured but commented out:

| Target triple | Platform |
|---------------|----------|
| `x86_64-unknown-linux-musl` | 64-bit Linux with static musl libc |
| `aarch64-unknown-linux-musl` | 64-bit ARM Linux with static musl libc |
| `x86_64-pc-windows-gnu` | 64-bit Windows (MinGW toolchain) |
| `x86_64-apple-darwin` | 64-bit Intel macOS |
| `aarch64-apple-darwin` | 64-bit Apple Silicon macOS |
| `aarch64-linux-android` | 64-bit Android ARM |

To enable a target, uncomment the corresponding block and install the required
cross-compilation toolchain (e.g. `x86_64-linux-musl-gcc` for the musl target).

## Notes

- This file applies to all crates in the workspace simultaneously. Do not add
  crate-specific settings here; use each crate's own `Cargo.toml` instead.
- CI does not cross-compile; it uses the native `ubuntu-latest` toolchain. Cross-
  compilation is intended for use with `scripts/build.sh` to produce distribution
  binaries.
- The musl targets (`*-musl`) produce fully statically-linked binaries that run on any
  Linux distribution without requiring a matching C library version.

## Related files

- `scripts/build.sh` — shell script that iterates over the cross-compilation targets and
  calls `cargo build --release` for each
- `.github/workflows/ci.yml` — CI pipeline; does not use cross-compilation targets
