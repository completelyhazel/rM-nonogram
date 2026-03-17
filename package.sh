#!/usr/bin/env bash
set -e

APP_DIR="$(dirname "$0")"
OUT_DIR="$APP_DIR/dist/nonogram-fetcher"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR/backend"

cp "$APP_DIR/manifest.json" "$OUT_DIR/"
if [ -f "$APP_DIR/icon.png" ]; then
    cp "$APP_DIR/icon.png" "$OUT_DIR/"
else
    echo "[!] Warning: icon.png not found, AppLoad will use its default icon"
fi

RCC_CMD=""
for cmd in rcc rcc-qt6 rcc6 qtchooser; do
    if command -v "$cmd" &>/dev/null; then
        RCC_CMD="$cmd"
        break
    fi
done

if [ -z "$RCC_CMD" ]; then
    echo "[!] rcc not found, falling back to qt6-rcc..."
    RCC_CMD="qt6-rcc"
fi

echo "[+] Compiling QML resources with $RCC_CMD..."
cd "$APP_DIR/ui"
$RCC_CMD -binary application.qrc -o "$OUT_DIR/resources.rcc"
cd "$APP_DIR"
echo "[✓] resources.rcc generated"

if [ -f "$APP_DIR/backend/entry" ]; then
    cp "$APP_DIR/backend/entry" "$OUT_DIR/backend/entry"
    chmod +x "$OUT_DIR/backend/entry"
    echo "[✓] backend/entry copied"
else
    echo "[✗] ERROR: backend/entry not found. Run backend/build.sh first."
    exit 1
fi

echo ""
echo "----------------------------------------------"
echo "  Output: $OUT_DIR"
find "$OUT_DIR" -type f | sort | sed 's|.*/dist/||'
echo ""
echo "  To install on the reMarkable:"
echo "  scp -r dist/nonogram-fetcher root@<REMARKABLE_IP>:/home/root/xovi/exthome/appload/"
echo "----------------------------------------------"