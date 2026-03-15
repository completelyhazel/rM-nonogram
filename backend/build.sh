#!/usr/bin/env bash
# =============================================================================
#  build.sh — Cross-compila el backend Rust para el reMarkable Paper Pro
#  Target: aarch64-unknown-linux-musl (estático, sin libc dinámica)
#
#  Requisitos en el host:
#    - Rust toolchain: rustup target add aarch64-unknown-linux-musl
#    - Cross-linker:   apt install gcc-aarch64-linux-gnu
#      o usar la herramienta `cross`: cargo install cross
# =============================================================================
set -e

TARGET="aarch64-unknown-linux-musl"
BINARY_NAME="entry"
OUT_DIR="../backend"

echo "──────────────────────────────────────────────"
echo "  Nonogram Fetcher — build backend"
echo "  target: $TARGET"
echo "──────────────────────────────────────────────"

# Verificar que tenemos el target instalado
if ! rustup target list --installed | grep -q "$TARGET"; then
    echo "[+] Añadiendo target $TARGET…"
    rustup target add "$TARGET"
fi

# Compilar
echo "[+] Compilando en modo release…"
cargo build --release --target "$TARGET"

# Copiar el binario al directorio de la app
BUILT="target/$TARGET/release/$BINARY_NAME"
if [ -f "$BUILT" ]; then
    cp "$BUILT" "$OUT_DIR/$BINARY_NAME"
    echo "[✓] Binario copiado a $OUT_DIR/$BINARY_NAME"
    ls -lh "$OUT_DIR/$BINARY_NAME"
else
    echo "[✗] ERROR: No se encontró el binario en $BUILT"
    exit 1
fi

echo "──────────────────────────────────────────────"
echo "  Build completado"
echo "──────────────────────────────────────────────"
