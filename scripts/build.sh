#!/usr/bin/env bash
# Build keeplin-daemon for all configured targets.
# Requires the appropriate cross-compilation toolchains to be installed.
set -euo pipefail

BINARY="keeplin-daemon"
OUT_DIR="dist"
mkdir -p "$OUT_DIR"

TARGETS=(
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-musl"
    "x86_64-pc-windows-gnu"
    # macOS and Android targets require Apple/Android SDKs — enable as needed:
    # "x86_64-apple-darwin"
    # "aarch64-apple-darwin"
    # "aarch64-linux-android"
)

for TARGET in "${TARGETS[@]}"; do
    echo "==> Building for $TARGET …"
    cargo build --release --target "$TARGET" -p keeplin-daemon

    SRC="target/$TARGET/release/$BINARY"
    # Windows produces .exe
    if [[ "$TARGET" == *windows* ]]; then
        SRC="${SRC}.exe"
        DEST="$OUT_DIR/${BINARY}-${TARGET}.exe"
    else
        DEST="$OUT_DIR/${BINARY}-${TARGET}"
    fi

    cp "$SRC" "$DEST"
    echo "    => $DEST"
done

echo "Done. Artifacts in $OUT_DIR/"
