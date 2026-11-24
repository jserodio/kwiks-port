#!/usr/bin/env bash
# Package the Kwiks port for distribution.
#
# Usage: ./package.sh [linux|windows|windows-local|launcher|all]
#
#   linux         Native build of the game        -> dist/kwiks-<ver>-linux-x86_64.tar.gz
#   windows       Native Windows build (needs a Windows machine)
#   windows-local Cross-compile the game for Windows from Linux with MinGW
#                 -> dist/kwiks-<ver>-windows-x86_64.zip
#   launcher      Freeze the launcher with PyInstaller
#                 -> dist/kwiks-launcher-<os>-x86_64.tar.gz
#   all           linux + launcher

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GAME="$ROOT/game"
LAUNCHER="$ROOT/launcher"
DIST="$ROOT/dist"
VERSION="$(grep -m1 '^version' "$GAME/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')"
PLATFORM="${1:-linux}"
PYTHON="${PYTHON:-python3}"
WINLIBS="$ROOT/.winlibs"

package_game() {
    local target="$1"
    local bin="$2"
    local out="$DIST/kwiks-$VERSION-$target"

    echo "==> Assembling $out"
    rm -rf "$out"
    mkdir -p "$out"
    cp "$bin" "$out/"
    # The original assets are not redistributed: the user brings their own copy
    # and extracts it with the launcher or tools/extract_one.py.
    cp "$ROOT/README.md" "$out/"

    if [ "$target" = "windows-x86_64" ]; then
        # Runtime DLLs the game needs next to the executable.
        cp "$WINLIBS/SDL3-3.4.20/x86_64-w64-mingw32/bin/SDL3.dll" "$out/"
        cp "$WINLIBS"/openmpt-bin/*.dll "$out/"
        (cd "$DIST" && 7z a -tzip "$(basename "$out").zip" "$(basename "$out")" >/dev/null \
            || zip -qr "$(basename "$out").zip" "$(basename "$out")")
        echo "    $DIST/$(basename "$out").zip"
    else
        (cd "$DIST" && tar czf "$(basename "$out").tar.gz" "$(basename "$out")")
        echo "    $DIST/$(basename "$out").tar.gz"
    fi
}

build_native() {
    echo "==> Building the game"
    (cd "$GAME" && cargo build --release)
    package_game "linux-x86_64" "$GAME/target/release/kwiks"
}

build_windows_native() {
    echo "==> Building the game (Windows)"
    (cd "$GAME" && cargo build --release)
    package_game "windows-x86_64" "$GAME/target/release/kwiks.exe"
}

build_windows_cross() {
    if [ ! -d "$WINLIBS" ]; then
        echo "==> Downloading and preparing the Windows libraries"
        "$ROOT/tools/prepare_winlibs.sh"
    fi
    echo "==> Cross-compiling the game for Windows (MinGW)"
    (cd "$GAME" && CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
        cargo build --release --target x86_64-pc-windows-gnu)
    package_game "windows-x86_64" \
        "$GAME/target/x86_64-pc-windows-gnu/release/kwiks.exe"
}

package_launcher() {
    local host
    case "$(uname -s)" in
        MINGW*|MSYS*|CYGWIN*) host="windows" ;;
        *) host="linux" ;;
    esac
    local out="$DIST/kwiks-launcher-$host-x86_64"

    echo "==> Freezing the launcher with PyInstaller ($PYTHON)"
    rm -rf "$LAUNCHER/build" "$LAUNCHER/dist" "$LAUNCHER"/*.spec
    (cd "$LAUNCHER" && "$PYTHON" -m PyInstaller \
        --noconfirm --clean --onefile --windowed \
        --name kwiks-launcher \
        kwiks_launcher.py)

    local exe="kwiks-launcher"
    [ -f "$LAUNCHER/dist/kwiks-launcher.exe" ] && exe="kwiks-launcher.exe"
    assemble_launcher "$out" "$LAUNCHER/dist/$exe"
}

# Package a launcher binary that has already been built (for example with
# PyInstaller under Wine) together with the sources it needs to build the port.
assemble_launcher() {
    local out="$1"
    local binary="$2"

    echo "==> Assembling $out"
    rm -rf "$out"
    mkdir -p "$out/game/src" "$out/game/tests" "$out/tools"
    cp "$binary" "$out/"
    cp "$GAME/Cargo.toml" "$GAME/Cargo.lock" "$GAME/build.rs" "$out/game/"
    cp -r "$GAME/src/." "$out/game/src/"
    cp -r "$GAME/tests/." "$out/game/tests/"
    cp -r "$ROOT/tools/." "$out/tools/"
    cp "$ROOT/README.md" "$out/"

    echo "==> Compressing"
    (cd "$DIST" && tar czf "$(basename "$out").tar.gz" "$(basename "$out")")
    echo "    $DIST/$(basename "$out").tar.gz"
}

# Build the Windows launcher from Linux/any host using Wine + a Windows Python
# that has tkinter (PyInstaller cannot cross-compile on its own).
package_launcher_windows() {
    local out="$DIST/kwiks-launcher-windows-x86_64"
    local prefix="${WINEPREFIX:-$ROOT/.wineprefix}"
    local wine="${WINE_CMD:-flatpak run --env=WINEPREFIX=$prefix --env=WINEDEBUG=-all --command=wine org.winehq.Wine}"
    local py="${WINDOWS_PYTHON:-C:\\Python312\\python.exe}"
    local exe="$LAUNCHER/kwiks-launcher.exe"

    if [ -f "$exe" ]; then
        echo "==> Using the existing $exe"
    else
        echo "==> Freezing the launcher with PyInstaller under Wine"
        (cd "$LAUNCHER" && rm -rf build dist && \
            $wine "$py" -m PyInstaller --noconfirm --clean --onefile --windowed \
                --name kwiks-launcher kwiks_launcher.py && \
            cp "dist/kwiks-launcher.exe" "$exe")
    fi

    assemble_launcher "$out" "$exe"
}

case "$PLATFORM" in
    linux)          build_native ;;
    windows)        build_windows_native ;;
    windows-local)  build_windows_cross ;;
    launcher)          package_launcher ;;
    launcher-windows)  package_launcher_windows ;;
    all)            build_native; package_launcher ;;
    *) echo "usage: $0 [linux|windows|windows-local|launcher|launcher-windows|all]" >&2; exit 1 ;;
esac

echo "==> Done"
