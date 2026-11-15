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

### 1. Launcher (recommended)

The launcher does the whole setup for you: pick the original game, extract its
assets, build the port and run it.

```bash
python3 launcher/kwiks_launcher.py
```

Or run the packaged binary from the [releases](../../releases) page.

### 2. Manual setup

#### 2.1 Original assets

This repository **does not include** the game's assets (graphics, music and
sound belong to their authors). You need a copy of **Kwiks (1998)** — freeware by
DIGITAL Nightmares — and extract them yourself:

```bash
# Put Kwiks_Win_EN.zip somewhere and extract the .one containers
python3 tools/extract_one.py /path/to/data.one  assets/data
python3 tools/extract_one.py /path/to/audio.one assets/audio

# Optional: convert the .PIX files to PNG for inspection
python3 tools/pix2png.py --all assets/data assets/png
```

The scripts live in `tools/` and the container and `.PIX` formats are documented
in `docs/PIX_FORMAT.md`.

#### 2.2 Build and run

```bash
cd game
cargo run --release
```

**Runtime dependencies** (Linux): `SDL3`, `SDL3_image` and `libopenmpt` as
shared libraries. On Fedora/Bazzite:

```bash
sudo dnf install sdl3 sdl3_image libopenmpt
```

If your distribution ships only the runtime (no headers or `.pc` files), create
the symlinks the `build.rs` expects:

```bash
mkdir -p game/lib
ln -sf /usr/lib64/libSDL3.so.0       game/lib/libSDL3.so
ln -sf /usr/lib64/libSDL3_image.so.0 game/lib/libSDL3_image.so
ln -sf /usr/lib64/libopenmpt.so.0    game/lib/libopenmpt.so
```

The launcher can create these for you (step 2 of its window).

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
./package.sh linux               # dist/kwiks-<version>-linux-x86_64.tar.gz
./package.sh launcher            # dist/kwiks-launcher-linux-x86_64.tar.gz
```

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
