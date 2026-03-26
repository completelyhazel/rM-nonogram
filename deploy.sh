set -euo pipefail

# config

TARGET="aarch64-unknown-linux-musl"
APP_ID="nonogram-fetcher"
APPLOAD_DIR="/home/root/xovi/exthome/appload"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DIST_DIR="$SCRIPT_DIR/dist/$APP_ID"

# argument parsing
BUILD=true
DEPLOY=true

for arg in "$@"; do
    case "$arg" in
        --build-only)  DEPLOY=false ;;
        --deploy-only) BUILD=false  ;;
        --help|-h)
            echo "Usage: $0 [--build-only | --deploy-only]"
            echo "  RM_IP env var sets the device IP (default: 10.11.99.1)"
            exit 0
            ;;
        *) echo "[!] Unknown argument: $arg"; exit 1 ;;
    esac
done

# helpers

header() { echo; echo "── $* ──────────────────────────────────────────"; }
ok()     { echo "[✓] $*"; }
fail()   { echo "[✗] $*" >&2; exit 1; }
info()   { echo "    $*"; }

# build phase

if $BUILD; then

    header "Rust backend"

    if ! rustup target list --installed | grep -q "$TARGET"; then
        info "Adding Rust target $TARGET…"
        rustup target add "$TARGET"
    fi

    (
        cd "$SCRIPT_DIR/backend"
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-gnu-gcc \
            cargo build --release --target "$TARGET"
    )

    BUILT="$SCRIPT_DIR/backend/target/$TARGET/release/entry"
    [ -f "$BUILT" ] || fail "Rust binary not found at $BUILT"
    ok "Rust binary compiled ($(du -sh "$BUILT" | cut -f1))"

    # qml resources

    header "QML resources"

    RCC=""
    for cmd in rcc6 rcc-qt6 rcc qtchooser; do
        if command -v "$cmd" &>/dev/null; then
            RCC="$cmd"; break
        fi
    done
    if [ -z "$RCC" ]; then
        RCC=$(find /usr/lib/qt6 /usr/lib/x86_64-linux-gnu/qt6 \
                   -name "rcc" -type f 2>/dev/null | head -1 || true)
    fi
    [ -n "$RCC" ] || fail "rcc not found — run: sudo apt install qt6-base-dev"

    info "Using: $RCC ($($RCC --version 2>&1 | head -1))"
    (cd "$SCRIPT_DIR/ui" && "$RCC" -binary application.qrc -o "$SCRIPT_DIR/resources.rcc")
    ok "resources.rcc compiled"

    # package

    header "Packaging"

    rm -rf "$DIST_DIR"
    mkdir -p "$DIST_DIR/backend"

    cp "$SCRIPT_DIR/manifest.json"  "$DIST_DIR/"
    cp "$SCRIPT_DIR/resources.rcc"  "$DIST_DIR/"
    cp "$BUILT"                     "$DIST_DIR/backend/entry"
    chmod +x                        "$DIST_DIR/backend/entry"

    if [ -f "$SCRIPT_DIR/icon.png" ]; then
        cp "$SCRIPT_DIR/icon.png" "$DIST_DIR/"
    else
        info "icon.png not found — AppLoad will use its default icon"
    fi

    ok "Packaged to dist/$APP_ID/"
    find "$DIST_DIR" -type f | sort | sed "s|$SCRIPT_DIR/dist/||" | while read -r f; do
        info "$f"
    done

fi  # end BUILD

# deploy phase

if $DEPLOY; then

    header "Deploy to reMarkable"

    [ -d "$DIST_DIR" ] || fail "dist/$APP_ID/ not found — run without --deploy-only first"

    # device ip
    if [ -z "${RM_IP:-}" ]; then
        read -rp "reMarkable IP [10.11.99.1]: " RM_IP
        RM_IP="${RM_IP:-10.11.99.1}"
    fi
    info "Target: root@$RM_IP"

    # quick connectivity check with a clear error if SSH auth isnt ready up yet
    if ! ssh -o ConnectTimeout=5 -o BatchMode=yes \
             "root@$RM_IP" true 2>/dev/null; then
        fail "SSH connection failed"
    fi

    # stop  xochitl before copying so there are no open file handle issues
    info "Stopping xochitl…"
    ssh "root@$RM_IP" "systemctl stop xochitl || true" 2>/dev/null

    # copy app directory
    info "Copying files…"
    ssh "root@$RM_IP" "mkdir -p '$APPLOAD_DIR/$APP_ID/backend'"
    scp -r "$DIST_DIR/." "root@$RM_IP:$APPLOAD_DIR/$APP_ID/"

    info "Restarting xochitl…"
    ssh "root@$RM_IP" "systemctl start xochitl"

    ok "Deployed to $APPLOAD_DIR/$APP_ID/"
    echo
    echo "  The app will appear in AppLoad on the device."

fi

header "Done"
