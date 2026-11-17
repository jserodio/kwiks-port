# Kwiks — modern port

A modern port of **Kwiks** (1998, DIGITAL Nightmares / Michael Haar), a freeware
**QIX** clone for Windows 95 built on DirectDraw. This port brings it to
**Linux and Windows** with Rust + SDL3, **unlocked frame rate**, **widescreen**
and **multiple resolutions**, without changing the original gameplay.

> Original game: fill the playfield up to **75 %** by enclosing areas with a
> white line. If an area contains a monster it cannot be claimed. Do not cross
> your own line and do not let a monster touch it.

---

## Getting started

### I just want to play

Download the **launcher** for your platform from the
[releases](../../releases) page, unpack it and run it:

| Platform | Download | Run |
|---|---|---|
| Windows | `kwiks-launcher-windows-x86_64.tar.gz` | `kwiks-launcher.exe` |
| Linux | `kwiks-launcher-linux-x86_64.tar.gz` | `./kwiks-launcher` |

The launcher installs the original game files, builds the port and starts it.
You only need to point it at the game ZIP you downloaded.

### Original game files

This project **does not include** the game's graphics, music or sound: they
belong to their authors. Bring your own copy of **Kwiks (1998)** — freeware by
DIGITAL Nightmares. The launcher installs its files for you.

To do it by hand instead:

```bash
python3 tools/extract_one.py /path/to/data.one  assets/data
python3 tools/extract_one.py /path/to/audio.one assets/audio
```

The `.one` container and the `.PIX` image format are documented in
`docs/PIX_FORMAT.md`.

### The launcher

It does the whole setup for you: choose the original game, and it installs its
files, builds the port and starts it.

| | |
|---|---|
| Windows | `kwiks-launcher.exe` |
| Linux | `./kwiks-launcher` |
| From source | `python3 launcher/kwiks_launcher.py` |

If something goes wrong, run `kwiks-launcher --diagnose` and include the output
when asking for help. `kwiks-launcher --setup <game.zip>` does the same without
a window.

---

## Building from source

You need the [Rust toolchain](https://rustup.rs) and the SDL3, SDL3_image and
libopenmpt libraries.

### Linux

```bash
# Fedora / Bazzite
sudo dnf install SDL3-devel SDL3_image-devel libopenmpt-devel
# Debian / Ubuntu (25.04+)
sudo apt install libsdl3-dev libsdl3-image-dev libopenmpt-dev
```

If your distribution ships only the runtime (no headers or `.pc` files), create
the symlinks the `build.rs` expects — the launcher does this for you
automatically:

```bash
mkdir -p game/lib
ln -sf /usr/lib64/libSDL3.so.0       game/lib/libSDL3.so
ln -sf /usr/lib64/libSDL3_image.so.0 game/lib/libSDL3_image.so
ln -sf /usr/lib64/libopenmpt.so.0    game/lib/libopenmpt.so
```

```bash
cd game
cargo run --release
```

### Windows

Two ways to build the game:

**On Windows** — install the libraries with [vcpkg](https://vcpkg.io), copy the
`.lib` and `.dll` files into `game/lib`, then:

```powershell
cd game
cargo build --release
```

**Cross-compile from Linux** with MinGW (this is what
`package.sh windows-local` does):

```bash
brew install mingw-w64            # or: pacman -S mingw-w64-x86_64-gcc
rustup target add x86_64-pc-windows-gnu
./tools/prepare_winlibs.sh        # downloads SDL3, SDL3_image and libopenmpt
./package.sh windows-local        # -> dist/kwiks-<version>-windows-x86_64.zip
```

`tools/prepare_winlibs.sh` also generates the MinGW import library for
libopenmpt, because openmpt.org does not publish one.

---

## Controls

Same as the original:

| Key | Action |
|---|---|
| Arrow keys | Move the player |
| Enter / Space | Confirm |
| Esc | Cancel / quit |
| P | Pause |
| F1 | Help |
| F2 | Fast graphics |
| F8 | Music on/off |
| F9 | Sound effects on/off |

---

## What is new

| | Original (1998) | This port |
|---|---|---|
| Platform | Windows 95 + DirectDraw 3 | Linux + Windows (SDL3) |
| Resolution | fixed 640×480×16 | 640×480 internal, widescreen, scaling |
| Frame rate | fixed 40 Hz | **free render** (60/120/144…) with a 40 Hz simulation |
| Music | MOD via AUDIOW32.DLL | MOD via libopenmpt |
| Extras | — | high scores, achievements, practice mode, options |

The **gameplay is identical**: the simulation still runs at exactly 40 Hz like
the original tick, so speeds and physics do not change.

---

## Architecture

```
launcher/       Launcher: extract assets, check dependencies, build, play
game/src/
  main.rs       Main loop: free render + 40 Hz simulation
  sim.rs        Simulation: player, monsters, scoring, levels
  field.rs      Field: per-pixel collision, flood fill, area closing
  screens.rs    State machine (title, menus, game, pause, game over)
  assets.rs     JPEG loading and the proprietary .PIX format
  audio.rs      MOD music (libopenmpt) + WAV effects, mixing
  config.rs     Persisted video/audio options
  video.rs      Internal resolution, widescreen and scaling
  extras.rs     High scores, achievements and practice mode
docs/
  GAME_LOGIC.md Game behaviour, recovered from the original binary
  PIX_FORMAT.md The .PIX container and its RLE encoding
tools/          Asset extraction scripts
```

The full technical specification, obtained by reverse engineering the original
binary, is in `docs/`.

---

## Development

```bash
cd game
cargo test                       # 139 tests
cargo test -- --ignored          # reference screenshots
```

Package:

```bash
./package.sh linux           # dist/kwiks-<version>-linux-x86_64.tar.gz
./package.sh windows-local   # dist/kwiks-<version>-windows-x86_64.zip
./package.sh launcher        # dist/kwiks-launcher-<os>-x86_64.tar.gz
```

Releases are built automatically by GitHub Actions
(`.github/workflows/release.yml`) whenever a `v*` tag is pushed.

---

## Credits and legal

- **Original game:** Kwiks © 1998 DIGITAL Nightmares (code: Michael Haar;
  music: Michael Haar). Released as freeware.
- **This repository contains only code.** The original assets (graphics, music,
  sound) and the 1998 executable are **not redistributed here**. Bring your own
  copy and extract it with `tools/` or the launcher.
- **This port** is a reverse engineering and reimplementation effort for
  preservation and personal use. The original assets remain the property of
  their authors.
- **Third-party software:** SDL3, libopenmpt, hound and image (Rust), and the
  IJG JPEG and ProTracker formats it reuses.
