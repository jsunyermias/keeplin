# `.cargo/config.toml` — workspace Cargo configuration

## Purpose

This file applies workspace-wide Cargo settings that override Cargo's built-in defaults
for every crate in the workspace. It can optionally configure linkers for
cross-compilation targets.

## Sections

### Build profiles do **not** belong here

Cargo reads `[profile.*]` only from a manifest (`Cargo.toml`), never from
`.cargo/config.toml` — a profile placed here is silently ignored. The release profile
(`opt-level = 3`, `lto = true`, `codegen-units = 1`, `strip = true`) therefore lives in
the workspace root `Cargo.toml`. This file keeps only a comment pointing there.

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
