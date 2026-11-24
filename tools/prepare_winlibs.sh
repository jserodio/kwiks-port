#!/usr/bin/env bash
# Download and prepare the Windows libraries needed to cross-compile the port.
#
# Works on Linux and on Windows (Git Bash / MSYS2). Writes to .winlibs/ :
#
#   .winlibs/SDL3-<ver>/x86_64-w64-mingw32/{lib,bin}
#   .winlibs/SDL3_image-<ver>/x86_64-w64-mingw32/{lib,bin}
#   .winlibs/openmpt-bin/            libopenmpt.dll + its helper DLLs
#   .winlibs/openmpt-bin/libopenmpt.dll.a   MinGW import library
#
# The import library for libopenmpt is generated from the DLL itself, because
# openmpt.org does not publish a MinGW build.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/.winlibs"
mkdir -p "$OUT"
cd "$OUT"

SDL3_VER="${SDL3_VER:-3.4.20}"
SDL3_IMAGE_VER="${SDL3_IMAGE_VER:-3.4.8}"
OPENMPT_VER="${OPENMPT_VER:-0.8.10}"

say() { echo "==> $*"; }

# --- MinGW tools -----------------------------------------------------------
# Linux/macOS package them with a target prefix; MSYS2 ships them unprefixed.
find_tool() {
    for name in "$@"; do
        if command -v "$name" >/dev/null 2>&1; then
            command -v "$name"
            return 0
        fi
    done
    return 1
}

DLLTOOL="${MINGW_DLLTOOL:-$(find_tool x86_64-w64-mingw32-dlltool dlltool || true)}"
OBJDUMP="${MINGW_OBJDUMP:-$(find_tool x86_64-w64-mingw32-objdump objdump || true)}"
if [ -z "$DLLTOOL" ] || [ -z "$OBJDUMP" ]; then
    echo "MinGW tools not found. Install mingw-w64, for example:" >&2
    echo "  brew install mingw-w64                       (Linux/macOS)" >&2
    echo "  pacman -S mingw-w64-x86_64-binutils          (MSYS2)" >&2
    exit 1
fi

# --- SDL3 ------------------------------------------------------------------
if [ ! -d "SDL3-$SDL3_VER" ]; then
    say "Downloading SDL3 $SDL3_VER (MinGW)"
    curl -fsSL -o sdl3-mingw.tar.gz \
        "https://github.com/libsdl-org/SDL/releases/download/release-$SDL3_VER/SDL3-devel-$SDL3_VER-mingw.tar.gz"
    tar xzf sdl3-mingw.tar.gz
fi

# --- SDL3_image ------------------------------------------------------------
if [ ! -d "SDL3_image-$SDL3_IMAGE_VER" ]; then
    say "Downloading SDL3_image $SDL3_IMAGE_VER (MinGW)"
    curl -fsSL -o sdl3_image-mingw.tar.gz \
        "https://github.com/libsdl-org/SDL_image/releases/download/release-$SDL3_IMAGE_VER/SDL3_image-devel-$SDL3_IMAGE_VER-mingw.tar.gz"
    tar xzf sdl3_image-mingw.tar.gz
fi

# --- libopenmpt ------------------------------------------------------------
if [ ! -d "openmpt-bin" ]; then
    say "Downloading libopenmpt $OPENMPT_VER"
    curl -fsSL -o openmpt-dev.zip \
        "https://lib.openmpt.org/files/libopenmpt/dev/libopenmpt-$OPENMPT_VER+release.dev.windows.vs2022.zip"
    mkdir -p openmpt-bin
    unzip -o -j openmpt-dev.zip "bin/amd64/libopenmpt.dll" "bin/amd64/openmpt-*.dll" \
        -d openmpt-bin >/dev/null

    say "Generating the MinGW import library for libopenmpt"
    "$OBJDUMP" -p openmpt-bin/libopenmpt.dll > openmpt-bin/objdump.txt
    python3 - openmpt-bin/objdump.txt openmpt-bin/libopenmpt.def <<'PY'
import re, sys, pathlib
text = pathlib.Path(sys.argv[1]).read_text(errors="replace")
block = text.split("[Ordinal/Name Pointer] Table", 1)
names = []
if len(block) == 2:
    for line in block[1].splitlines():
        m = re.match(r"\s*\[\s*\d+\]\s+\+base\[\s*\d+\]\s+[0-9a-fA-F]+\s+(.+?)\s*$", line)
        if m:
            names.append(m.group(1))
if not names:
    sys.exit("could not read the export table of libopenmpt.dll")
pathlib.Path(sys.argv[2]).write_text(
    "LIBRARY libopenmpt.dll\nEXPORTS\n" + "\n".join(f"  {n}" for n in names) + "\n"
)
print(f"  {len(names)} exported symbols")
PY
    "$DLLTOOL" -d openmpt-bin/libopenmpt.def -l openmpt-bin/libopenmpt.dll.a \
        -D libopenmpt.dll
    rm -f openmpt-bin/objdump.txt
fi

# --- Stage into game/lib ---------------------------------------------------
say "Staging the import libraries into game/lib"
mkdir -p "$ROOT/game/lib"
cp "$OUT"/SDL3-*/x86_64-w64-mingw32/lib/libSDL3.dll.a "$ROOT/game/lib/"
cp "$OUT"/SDL3_image-*/x86_64-w64-mingw32/lib/libSDL3_image.dll.a "$ROOT/game/lib/"
cp "$OUT/openmpt-bin/libopenmpt.dll.a" "$ROOT/game/lib/"

ls -la "$ROOT/game/lib"
say "Done"
