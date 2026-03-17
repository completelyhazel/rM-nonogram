#!/usr/bin/env bash
set -e

TARGET="aarch64-unknown-linux-musl"
BINARY_NAME="entry"
OUT_DIR="../backend"

echo "──────────────────────────────────────────────"
echo "  Nonogram Fetcher — build backend"
echo "  target: $TARGET"
echo "──────────────────────────────────────────────"

# verify target
if ! rustup target list --installed | grep -q "$TARGET"; then
    echo "[+] adding target $TARGET…"
    rustup target add "$TARGET"
fi

# compile
echo "[+] release mode compilin...."
cargo build --release --target "$TARGET"

# copy directory
BUILT="target/$TARGET/release/$BINARY_NAME"
if [ -f "$BUILT" ]; then
    cp "$BUILT" "$OUT_DIR/$BINARY_NAME"
    echo "[✓] binary copied to $OUT_DIR/$BINARY_NAME"
    ls -lh "$OUT_DIR/$BINARY_NAME"
else
    echo "[✗] ERROR: didnt find binary on $BUILT"
    exit 1
fi

echo "──────────────────────────────────────────────"
echo "  FINISHED BUILD"
echo "──────────────────────────────────────────────"
