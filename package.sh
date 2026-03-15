#!/usr/bin/env bash
# =============================================================================
#  package.sh — Compila los recursos QML y empaqueta la app para AppLoad
#
#  Genera la siguiente estructura lista para copiar al reMarkable:
#    nonogram-fetcher/
#    ├── manifest.json
#    ├── icon.png
#    ├── resources.rcc          ← QML compilado
#    └── backend/
#        └── entry              ← binario aarch64
#
#  Requisitos:
#    - rcc (Qt Resource Compiler): apt install qtbase5-dev-tools
#      o qtchooser -run-tool=rcc -qt=6 (si tienes Qt6)
#    - El binario backend/entry ya compilado (ejecuta build.sh primero)
# =============================================================================
set -e

APP_DIR="$(dirname "$0")"
OUT_DIR="$APP_DIR/dist/nonogram-fetcher"

echo "──────────────────────────────────────────────"
echo "  Nonogram Fetcher — package"
echo "──────────────────────────────────────────────"

# 1. Limpiar y crear estructura de salida
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR/backend"

# 2. Copiar manifest e icono
cp "$APP_DIR/manifest.json" "$OUT_DIR/"
if [ -f "$APP_DIR/icon.png" ]; then
    cp "$APP_DIR/icon.png" "$OUT_DIR/"
else
    echo "[!] Aviso: icon.png no encontrado, se usará el icono por defecto de AppLoad"
fi

# 3. Compilar recursos QML con rcc
RCC_CMD=""
for cmd in rcc rcc-qt6 rcc6 qtchooser; do
    if command -v "$cmd" &>/dev/null; then
        RCC_CMD="$cmd"
        break
    fi
done

if [ -z "$RCC_CMD" ]; then
    echo "[!] rcc no encontrado. Intentando con qt6-rcc como fallback…"
    RCC_CMD="qt6-rcc"
fi

echo "[+] Compilando recursos QML con $RCC_CMD…"
cd "$APP_DIR/ui"
$RCC_CMD -binary application.qrc -o "$OUT_DIR/resources.rcc"
cd "$APP_DIR"
echo "[✓] resources.rcc generado"

# 4. Copiar binario backend
if [ -f "$APP_DIR/backend/entry" ]; then
    cp "$APP_DIR/backend/entry" "$OUT_DIR/backend/entry"
    chmod +x "$OUT_DIR/backend/entry"
    echo "[✓] backend/entry copiado"
else
    echo "[✗] ERROR: backend/entry no encontrado. Ejecuta backend/build.sh primero."
    exit 1
fi

# 5. Mostrar resultado
echo ""
echo "──────────────────────────────────────────────"
echo "  Estructura final en: $OUT_DIR"
find "$OUT_DIR" -type f | sort | sed 's|.*/dist/||'
echo ""
echo "  Para instalar en el reMarkable:"
echo "  scp -r dist/nonogram-fetcher root@<IP_REMARKABLE>:/home/root/xovi/exthome/appload/"
echo "──────────────────────────────────────────────"
