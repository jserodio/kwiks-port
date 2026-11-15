#!/usr/bin/env bash
# Package the Kwiks port for distribution.
#
# Usage: ./package.sh [linux|windows|launcher|all]
#
#   linux     Build the game and produce dist/kwiks-<version>-linux-x86_64.tar.gz
#   windows   Cross-compile with the x86_64-pc-windows-gnu toolchain
#   launcher  Freeze the launcher with PyInstaller and produce
#             dist/kwiks-launcher-linux-x86_64.tar.gz
#   all       Everything above

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GAME="$ROOT/game"
LAUNCHER="$ROOT/launcher"
DIST="$ROOT/dist"
VERSION="$(grep -m1 '^version' "$GAME/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')"
PLATFORM="${1:-linux}"

package_game() {
    local target="$1"
    local out="$DIST/kwiks-$VERSION-$target"

    echo "==> Building ($target)"
    if [ "$target" = "linux-x86_64" ]; then
        (cd "$GAME" && cargo build --release)
        BIN="$GAME/target/release/kwiks"
    else
        (cd "$GAME" && cargo build --release --target x86_64-pc-windows-gnu)
        BIN="$GAME/target/x86_64-pc-windows-gnu/release/kwiks.exe"
    fi

    echo "==> Assembling $out"
    rm -rf "$out"
    mkdir -p "$out"
    cp "$BIN" "$out/"
    # The original assets are not redistributed: the user brings their own
    # copy and extracts it with the launcher or tools/extract_one.py.
    cp "$ROOT/README.md" "$out/"

    echo "==> Compressing"
    (cd "$DIST" && tar czf "$(basename "$out").tar.gz" "$(basename "$out")")
    echo "    $DIST/$(basename "$out").tar.gz"
}

package_launcher() {
    local out="$DIST/kwiks-launcher-linux-x86_64"
    # Override with PYTHON=/path/to/python if your default interpreter has no
    # tkinter (for example the system Python on Bazzite; use the Homebrew one).
    local py="${PYTHON:-python3}"

    echo "==> Freezing the launcher with PyInstaller ($py)"
    rm -rf "$LAUNCHER/build" "$LAUNCHER/dist"
    (cd "$LAUNCHER" && "$py" -m PyInstaller \
        --noconfirm --clean --onefile --windowed \
        --name kwiks-launcher \
        kwiks_launcher.py)

    echo "==> Assembling $out"
    rm -rf "$out"
    mkdir -p "$out"
    cp "$LAUNCHER/dist/kwiks-launcher" "$out/"

    # The launcher needs the project to build: ship the sources and tools.
    mkdir -p "$out/game/src" "$out/game/tests" "$out/tools"
    cp "$GAME/Cargo.toml" "$GAME/Cargo.lock" "$GAME/build.rs" "$out/game/"
    cp -r "$GAME/src/." "$out/game/src/"
    cp -r "$GAME/tests/." "$out/game/tests/"
    cp -r "$ROOT/tools/." "$out/tools/"
    cp "$ROOT/README.md" "$out/"

    echo "==> Compressing"
    (cd "$DIST" && tar czf "$(basename "$out").tar.gz" "$(basename "$out")")
    echo "    $DIST/$(basename "$out").tar.gz"
}

case "$PLATFORM" in
    linux)    package_game "linux-x86_64" ;;
    windows)  package_game "windows-x86_64" ;;
    launcher) package_launcher ;;
    all)      package_game "linux-x86_64"; package_launcher ;;
    *) echo "usage: $0 [linux|windows|launcher|all]" >&2; exit 1 ;;
esac

echo "==> Done"
