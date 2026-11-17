"""Core logic for the Kwiks launcher (no GUI code here).

Four steps, in the order the launcher uses them:

1. :func:`extract_assets` — read the original 1998 game's ``.one`` containers and
   write the graphics and audio to ``assets/data`` and ``assets/audio``.
2. :func:`check_dependencies` — what the port needs at build or run time.
3. :func:`build_project` — run ``cargo build --release`` and stream the output.
4. :func:`run_game` — start the built game.

Everything is written to work the same on Linux and Windows.
"""

from __future__ import annotations

import os
import platform
import shutil
import struct
import subprocess
import sys
import zipfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Optional

# --- The original ".one" container -----------------------------------------
#
#   char[2]  "OO"
#   u32      entry count
#   count x { char name[1024]; u32 offset; u32 size }   # stride 0x408
#
# Names are XOR-encoded with 0xDF; digits are stored as 0x10 + digit, the dot as
# 0x0E. See docs/PIX_FORMAT.md for the .PIX files themselves.
NAME_STRIDE = 0x408
NAME_LEN = 0x400

IS_WINDOWS = platform.system() == "Windows"
IS_MACOS = platform.system() == "Darwin"

Log = Callable[[str], None]


def _noop(_: str) -> None:
    pass


def decode_name(raw: bytes) -> str:
    out = []
    for c in raw.split(b"\x00")[0]:
        v = c ^ 0xDF
        if v == 0x0E:
            out.append(".")
        elif 32 <= v < 127:
            out.append(chr(v))
        elif 0x10 <= v <= 0x19:
            out.append(chr(ord("0") + v - 0x10))
        else:
            out.append(f"_{v:02x}")
    return "".join(out)


@dataclass
class Entry:
    name: str
    offset: int
    size: int


def read_entries(blob: bytes) -> list[Entry]:
    if blob[:2] != b"OO":
        raise ValueError("not an OO container")
    count = struct.unpack_from("<I", blob, 2)[0]
    entries = []
    for i in range(count):
        base = 6 + i * NAME_STRIDE
        name = decode_name(blob[base : base + NAME_LEN])
        offset, size = struct.unpack_from("<II", blob, base + NAME_LEN)
        entries.append(Entry(name, offset, size))
    return entries


# ---------------------------------------------------------------------------
# Locations
# ---------------------------------------------------------------------------


def project_root() -> Path:
    """Folder that holds ``game/``, ``assets/`` and ``tools/``.

    Frozen with PyInstaller the launcher sits inside that folder; running from
    source it sits in ``launcher/`` one level below it.
    """
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent.parent


def game_dir() -> Path:
    return project_root() / "game"


def assets_dir() -> Path:
    return project_root() / "assets"


# ---------------------------------------------------------------------------
# 1. Assets
# ---------------------------------------------------------------------------


@dataclass
class ExtractResult:
    data_files: int = 0
    audio_files: int = 0
    warnings: list[str] = field(default_factory=list)


def find_containers(source: Path) -> tuple[Optional[Path], Optional[Path]]:
    """Locate ``data.one`` and ``audio.one`` in a ZIP, a folder or a direct path."""
    data: Optional[Path] = None
    audio: Optional[Path] = None

    if source.is_file() and source.suffix.lower() == ".zip":
        cache = project_root() / ".launcher_cache"
        cache.mkdir(parents=True, exist_ok=True)
        try:
            with zipfile.ZipFile(source) as zf:
                for n in zf.namelist():
                    low = n.lower()
                    if low.endswith("data.one"):
                        data = Path(zf.extract(n, cache))
                    elif low.endswith("audio.one"):
                        audio = Path(zf.extract(n, cache))
        except zipfile.BadZipFile as e:
            raise ValueError(f"That file is not a readable ZIP: {e}") from e
        return data, audio

    if source.is_dir():
        for p in source.rglob("*"):
            low = p.name.lower()
            if low == "data.one":
                data = p
            elif low == "audio.one":
                audio = p
        return data, audio

    if source.is_file():
        low = source.name.lower()
        if low == "data.one":
            return source, None
        if low == "audio.one":
            return None, source

    return None, None


def extract_container(blob: bytes, out_dir: Path) -> int:
    out_dir.mkdir(parents=True, exist_ok=True)
    n = 0
    for e in read_entries(blob):
        if e.offset + e.size > len(blob):
            continue
        (out_dir / e.name.lower()).write_bytes(blob[e.offset : e.offset + e.size])
        n += 1
    return n


def extract_assets(source: Path, log: Log = _noop) -> ExtractResult:
    """Extract the original game's graphics and audio into ``assets/``."""
    res = ExtractResult()
    data, audio = find_containers(source)

    if data is None and audio is None:
        raise FileNotFoundError(
            "We could not find the game files (data.one and audio.one) in what "
            "you selected. Please choose the ZIP you downloaded, or the folder "
            "where you unzipped the game."
        )

    if data is not None:
        log(f"Reading {data.name} ...")
        res.data_files = extract_container(data.read_bytes(), assets_dir() / "data")
        log(f"  -> {res.data_files} pictures and sprites")

    if audio is not None:
        log(f"Reading {audio.name} ...")
        res.audio_files = extract_container(audio.read_bytes(), assets_dir() / "audio")
        log(f"  -> {res.audio_files} music and sound files")

    if res.data_files == 0 or res.audio_files == 0:
        res.warnings.append(
            "Only part of the game files was found. The game needs both "
            "data.one and audio.one to run."
        )
    return res


def assets_status() -> tuple[bool, str]:
    data = assets_dir() / "data"
    audio = assets_dir() / "audio"
    n_data = len(list(data.glob("*"))) if data.is_dir() else 0
    n_audio = len(list(audio.glob("*"))) if audio.is_dir() else 0
    ok = n_data >= 30 and n_audio >= 9
    return ok, f"{n_data} pictures, {n_audio} sound files"


# ---------------------------------------------------------------------------
# 2. Requirements
# ---------------------------------------------------------------------------


@dataclass
class DepStatus:
    name: str
    ok: bool
    detail: str
    fix_hint: str = ""


def _lib_dirs() -> list[Path]:
    if IS_WINDOWS:
        return [
            project_root() / "lib",
            game_dir() / "lib",
            Path(sys.executable).resolve().parent,
            Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32",
        ]
    return [
        Path("/usr/lib64"),
        Path("/usr/lib"),
        Path("/usr/local/lib"),
        Path("/lib/x86_64-linux-gnu"),
        Path("/usr/lib/x86_64-linux-gnu"),
        Path("/opt/homebrew/lib"),
        Path("/home/linuxbrew/.linuxbrew/lib"),
        game_dir() / "lib",
    ]


def find_library(names: list[str]) -> Optional[Path]:
    """First existing file among ``names`` in the usual library folders."""
    for d in _lib_dirs():
        for n in names:
            p = d / n
            if p.exists():
                return p
    if not IS_WINDOWS:
        import ctypes.util

        for n in names:
            found = ctypes.util.find_library(n.split(".so")[0].removeprefix("lib"))
            if found:
                return Path(found)
    return None


def check_dependencies() -> list[DepStatus]:
    out: list[DepStatus] = []

    cargo = shutil.which("cargo")
    out.append(
        DepStatus(
            "Rust compiler",
            cargo is not None,
            "found" if cargo else "not installed",
            "" if cargo else "Install it from https://rustup.rs and press Check again.",
        )
    )

    if IS_WINDOWS:
        libs = [
            ("SDL3", ["SDL3.dll", "SDL3.lib", "libSDL3.dll.a"]),
            ("SDL3_image", ["SDL3_image.dll", "SDL3_image.lib"]),
            ("Music library (libopenmpt)", ["openmpt.dll", "libopenmpt.dll", "libopenmpt.lib"]),
        ]
    elif IS_MACOS:
        libs = [
            ("SDL3", ["libSDL3.dylib", "libSDL3.0.dylib"]),
            ("SDL3_image", ["libSDL3_image.dylib"]),
            ("Music library (libopenmpt)", ["libopenmpt.dylib"]),
        ]
    else:
        libs = [
            ("SDL3", ["libSDL3.so.0", "libSDL3.so"]),
            ("SDL3_image", ["libSDL3_image.so.0", "libSDL3_image.so"]),
            ("Music library (libopenmpt)", ["libopenmpt.so.0", "libopenmpt.so"]),
        ]

    hint = (
        "On Windows, download SDL3 and libopenmpt and put the .dll files in the game folder."
        if IS_WINDOWS
        else "On Fedora/Bazzite: sudo dnf install sdl3 sdl3_image libopenmpt"
    )
    for label, names in libs:
        found = find_library(names)
        out.append(
            DepStatus(
                label,
                found is not None,
                str(found) if found else "not found",
                "" if found else hint,
            )
        )
    return out


def missing_link_shims() -> list[str]:
    """Libraries the build expects in ``game/lib`` and that are not there yet."""
    if IS_WINDOWS:
        need = {
            "SDL3.dll": ["SDL3.dll", "SDL3.lib"],
            "SDL3_image.dll": ["SDL3_image.dll", "SDL3_image.lib"],
            "openmpt.dll": ["openmpt.dll", "libopenmpt.dll"],
        }
    else:
        need = {
            "libSDL3.so": ["libSDL3.so.0", "libSDL3.so"],
            "libSDL3_image.so": ["libSDL3_image.so.0", "libSDL3_image.so"],
            "libopenmpt.so": ["libopenmpt.so.0", "libopenmpt.so"],
        }
    lib_dir = game_dir() / "lib"
    missing = []
    for shim, names in need.items():
        if not (lib_dir / shim).exists() and find_library(names):
            missing.append(shim)
    return missing


def create_link_shims(log: Log = _noop) -> int:
    """Create the ``game/lib`` links the build script looks for.

    This is done automatically before building, so the user never has to.
    """
    if IS_WINDOWS:
        need = {
            "SDL3.dll": ["SDL3.dll", "SDL3.lib"],
            "SDL3_image.dll": ["SDL3_image.dll", "SDL3_image.lib"],
            "openmpt.dll": ["openmpt.dll", "libopenmpt.dll"],
        }
    else:
        need = {
            "libSDL3.so": ["libSDL3.so.0", "libSDL3.so"],
            "libSDL3_image.so": ["libSDL3_image.so.0", "libSDL3_image.so"],
            "libopenmpt.so": ["libopenmpt.so.0", "libopenmpt.so"],
        }
    lib_dir = game_dir() / "lib"
    lib_dir.mkdir(parents=True, exist_ok=True)
    made = 0
    for shim, names in need.items():
        target = find_library(names)
        dest = lib_dir / shim
        if target is None or dest.exists() or dest.is_symlink():
            continue
        try:
            if IS_WINDOWS:
                shutil.copy2(target, dest)
                log(f"  copied {shim}")
            else:
                dest.symlink_to(target)
                log(f"  linked {shim} -> {target}")
            made += 1
        except OSError as e:
            log(f"  could not prepare {shim}: {e}")
    return made


# ---------------------------------------------------------------------------
# 3. Build
# ---------------------------------------------------------------------------


def build_project(log: Log = _noop, release: bool = True) -> int:
    """Run ``cargo build`` in ``game/`` and stream its output to ``log``."""
    if not (game_dir() / "Cargo.toml").exists():
        log("Could not find the game source in this folder.")
        return 1

    # Make sure the linker can find SDL3 and libopenmpt.
    n = create_link_shims(log)
    if n:
        log(f"Prepared {n} missing libraries automatically.")

    cmd = ["cargo", "build"]
    if release:
        cmd.append("--release")
    log("$ " + " ".join(cmd))

    try:
        proc = subprocess.Popen(
            cmd,
            cwd=game_dir(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
    except FileNotFoundError:
        log("The Rust compiler (cargo) is not installed.")
        return 1

    assert proc.stdout is not None
    for line in proc.stdout:
        log(line.rstrip("\n"))
    proc.wait()
    log(f"Build finished with exit code {proc.returncode}.")
    return proc.returncode


def built_binary() -> Optional[Path]:
    for name in ("kwiks.exe", "kwiks"):
        for profile in ("release", "debug"):
            for sub in (
                game_dir() / "target" / profile / name,
                game_dir() / "target" / "x86_64-pc-windows-gnu" / profile / name,
            ):
                if sub.exists():
                    return sub
    return None


# ---------------------------------------------------------------------------
# 4. Play
# ---------------------------------------------------------------------------


def run_game(log: Log = _noop) -> Optional[subprocess.Popen]:
    binary = built_binary()
    if binary is None:
        log("The game has not been built yet.")
        return None
    log(f"Starting the game: {binary.name}")
    env = os.environ.copy()
    env.setdefault("KWIKS_ASSETS", str(assets_dir()))
    return subprocess.Popen([str(binary)], cwd=project_root(), env=env)
