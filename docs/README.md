# Kwiks (1998) — Documentation

Reverse-engineering documentation for `work/kwiks2.exe` (PE32 i386, Watcom C/C++32, no symbols).

Labels used throughout: **[O]** = observed in the binary · **[I]** = inferred · **[?]** = unknown.

## Contents

| Document | Description |
|---|---|
| [GAME_LOGIC.md](GAME_LOGIC.md) | Game logic specification: state machine, 40 Hz tick, data model, QIX-style collision/fill, physics, scoring. |
| [PIX_FORMAT.md](PIX_FORMAT.md) | `.PIX` image format decoded: header layout, RLE decompression, palette (6-bit RGB → RGB565), validation. |
| [decomp/](decomp/) | Ghidra decompiled pseudocode (`FUN_xxxxxxxx.c`) and metadata (`.json`) per function. |

## Assets

- `gameplay_nivel1.gif` — gameplay capture of level 1.

## Tools

- [`tools/pix2png.py`](../tools/pix2png.py) — working `.PIX` decoder (see [PIX_FORMAT.md](PIX_FORMAT.md) §9).
