# Kwiks (1998) `.PIX` format — decoded

**Status: SOLVED.** All 7 `.pix` files in the game decode to exactly `width × height`
pixels, with no leftover bytes, and the colors match the reference JPEG assets.
Working decoder: [`tools/pix2png.py`](../tools/pix2png.py).

Evidence obtained with REA 6.9.0 + Ghidra 12.1.4 on `work/kwiks2.exe`
(PE32 i386, Watcom C/C++32, no symbols). Everything marked **[O]** is observed
in the binary or in the files; what is marked **[I]** is inferred.

---

## 1. Location in the binary

| What | Where | Note |
|---|---|---|
| File name strings | `0x42505e` "COMPLET.PIX", `0x42506c` "GAMEOVER.PIX", `0x4252f8` "SPIEL.PIX", `0x425464` "INFO.PIX", `0x42546d` "FNTS.PIX", `0x425712` "EMAKERS.PIX", `0x42571e` "MAKERS.PIX" | **[O]** |
| File loading / format dispatch | `FUN_004070c4` | **[O]** |
| Recognized extensions | `0x425759`: `.TGA .PI1 .BMP .PCX .PIC .GIF .PIX` (comparison with `FUN_0040ea7b`) | **[O]** |
| RLE decoder | `FUN_00407080` | **[O]** |
| Raw copy (uncompressed) | `FUN_0040ed38` (= `memmove`) | **[O]** |
| Palette lookup → RGB565 | `FUN_0040cc28` | **[O]** |
| Palette lookup → 24/32bpp | `FUN_0040cca4`, `FUN_00406f50`, `FUN_00406fe8` | **[O]** |
| I/O (open/read/seek) | `FUN_0040ea56`, `FUN_0040c730`, `FUN_0040e721`, `FUN_0040e962` | **[O]** (semantics inferred from their use) |

`FUN_004070c4` compares the **file name extension** against the table at
`0x425759`; the `.PIX` branch (the last one) reads an 11-byte header, verifies the
`"PIX"` magic in the first 3 bytes and decodes. **[O]**

## 2. Header layout (11 bytes + length field)

| Offset | Size | Content | Evidence |
|---|---|---|---|
| 0 | 3 | `50 49 58` = `"PIX"` | **[O]** checked in `FUN_004070c4` (`local_88=='P' && local_87=='I' && local_86=='X'`) and in the 7 files |
| 3 | 1 | Version. `0x02` in all files | **[O]** the byte is not used in the code **[I]** interpretation as a version |
| 4 | 1 | Offset (from the start of the file) of the length / data field. `0x0b` = 11 in the 7 files | **[O]** `FUN_004070c4` seeks to `byte4 + DAT_00461148`; with DAT=0 this gives 11 |
| 5 | 2 | Width, u16 LE | **[O]** (`sStack_83` → `local_5c`) |
| 7 | 2 | Height, u16 LE | **[O]** (`uStack_81/uStack_80` → `local_58[0]`) |
| 9 | 1 | Flags. `0x00` in all 7 files. If ≠ 0, `FUN_004070c4` **does not decode pixels** | **[O]** branch `local_7f == '\0'`; meaning of the ≠ 0 value unknown |
| 10 | 1 | Unknown. `0x01` in all 7 files | **[O]** value; meaning **unknown** |
| 11 | 2 or 4 | Length of the compressed stream. **u16 LE if the image is exactly 320×200; u32 LE otherwise** | **[O]** `FUN_004070c4`: special branch `w==0x140 && h==200` reads 2 bytes; the generic branch reads 4. Verified in the 7 files (the size matches exactly) |
| 13/15 | n | Compressed stream (or raw pixels, see §4) | **[O]** |
| end−768 | 768 | Palette: 256 × (R, G, B), **6 bits per channel (0..63)** | **[O]** `FUN_0040e721(..., 0x300)` reads 768 bytes; values ≤ 63 in the 7 files; RGB order see §5 |

After the palette there is nothing: `file_size = end_of_stream + 768` exactly in the 7
files (0 leftover bytes). **[O]**

Observed sizes (verified byte by byte):

| File | Dim | Len. field | Value | `11/13 + len + 768` = size |
|---|---|---|---|---|
| complet.pix | 640×480 | u32 @11 | 18588 | 19371 ✓ |
| gameover.pix | 640×480 | u32 @11 | 21298 | 22081 ✓ |
| makers.pix | 640×480 | u32 @11 | 35944 | 36727 ✓ |
| emakers.pix | 640×480 | u32 @11 | 30718 | 31501 ✓ |
| info.pix | 640×480 | u32 @11 | 10238 | 11021 ✓ |
| spiel.pix | 320×200 | u16 @11 | 5388 | 6169 ✓ |
| fnts.pix | 320×200 | u16 @11 | 1920 | 2701 ✓ |

## 3. Decompression (RLE) — `FUN_00407080`

The stream produces exactly `width × height` bytes of palette indices (8 bpp),
**row order top to bottom, contiguous rows of `width` bytes** **[O]**
(the blit loops in `FUN_004070c4` traverse height rows × width pixels with
stride = width; confirmed visually: the text comes out upright).

```
output = []
while input remains:
    b = read_byte()
    if (b & 0x80) == 0:            # literal:  b = 0..0x7F
        copy b bytes from input to output
    else:                          # repetition: b = 0x80..0xFF
        value = read_byte()
        repeat (b & 0x7F) times: output.append(value)
```

Evidence: pseudocode of `FUN_00407080` (loop `while (pbVar5 < in_EAX +
unaff_EBX)`, branch `(bVar2 & 0x80) == 0` copies `bVar2` literal bytes; the
`else` branch builds `CONCAT11(value, b) & 0x7f` and writes the high byte while
counting the low one down to zero). **[O]**

Notes:
- A `0x00` control byte is a no-op (0 literals); `0x80` repeats 0 times.
  They can be used as padding. **[O]** (from the code)
- Maximum count per operation: 127 bytes. **[O]**
- The decoder stops when the input is exhausted; the sum of the operations must
  give exactly `width × height` (verified: 7/7). **[O]**

## 4. Uncompressed case

If `stream_length == width × height`, the data is **raw pixels** (uncompressed 8 bpp
indices) and the game copies it with `memmove` (`FUN_0040ed38`) instead of
going through the RLE. **[O]** (`FUN_004070c4`: `if (local_5c * local_58[0] - local_7c == 0)`
→ `FUN_0040ed38`, `else` → `FUN_00407080`; in the 320×200 branch it compares against 64000).

In the 7 game files all streams are compressed. **[O]**

**Uncertainty:** if a compressed stream happened to have length
`width × height`, the game would interpret it as raw. This is an encoder
convention, not something distinguishable when decoding. **[I]**

## 5. Palette and color

- 768 bytes at the end of the file = 256 entries × (R, G, B). **[O]**
- Each channel occupies 6 bits (values 0..63; in the 7 files the maximum is 63 and
  there are ramps like `00 00 00, 02 02 02, 04 04 04, … 3f 3f 3f`). **[O]**
- **RGB order** **[O]**:
  - `FUN_0040cc28` (16bpp path, the one used by the game at 640×480×16):
    `R5 = pal[0]>>1` → bits 11–15, `G6 = pal[1]` → bits 5–10, `B5 = pal[2]>>1` → bits 0–4 (RGB565).
  - `FUN_0040cca4` (24/32bpp path): returns
    `(pal[0]&0x3f)<<18 | (pal[1]&0x3f)<<10 | (pal[2]&0x3f)<<2` and
    `FUN_00406f50` writes the BGR bytes = `pal[2]<<2, pal[1]<<2, pal[0]<<2`.
- **Expansion to 8 bits** for PNG (documented decision):
  - **`shl2` (used by default in pix2png.py): `c8 = v << 2`.** It is literally
    the expansion of the game's 24bpp path **[O]** and reproduces the artist's
    original colors: the reference JPEGs contain exactly those
    values (see §7). **[O]**
  - `565`: what was seen on screen with the game's 16bpp mode
    (`R5=v>>1` → `(x<<3)|(x>>2)`, `G6=v` → `(g<<2)|(g>>4)`, `B5=v>>1`). **[O]**
  - `linear`: `v*255/63`. Aesthetic option; not observed in the binary. **[I]**

## 6. Complete decoding pseudocode

```
read 11-byte header
assert header[0:3] == "PIX"
w = u16le(header[5:7]); h = u16le(header[7:9])
assert header[9] == 0            # if != 0 the game does not decode pixels
off = header[4]                  # = 11 in all known files
if (w, h) == (320, 200):  n = u16le(file[off:off+2]); off += 2
else:                     n = u32le(file[off:off+4]); off += 4
stream  = file[off : off+n]
palette6 = file[off+n : off+n+768]        # 256 * RGB, 6 bits
assert off + n + 768 == file_size

if n == w*h:  indices = stream              # raw
else:         indices = RLE_decode(stream)  # §3, must yield exactly w*h

for each index i: RGB8 = (palette6[i].r << 2, palette6[i].g << 2, palette6[i].b << 2)
image = indices.reshape(h, w)              # row 0 = top
```

## 7. Validation

- **Size:** the 7 files decode to exactly `w*h` bytes and consume the
  entire file (0 leftover bytes, exact 768 palette). **[O]**
- **Dimensions:** the PNGs are exactly 640×480 (5 files) and 320×200
  (spiel, fnts). **[O]**
- **Visual content:** `complet` = "LEVEL COMPLETE!" caption, `gameover` = creature
  + "GAME OVER!", `makers`/`emakers` = DE/EN credits (readable names: Michael
  Haar, Rebecca Ludolph), `info` = HUD "Fertig/Filled % Level Punkte/Points
  Leben/Lives" + digits, `fnts` = digit font 0–9, `spiel` = 3 star
  effects. All readable, correct orientation. **[O]**
- **Color:** the reference JPEGs (original game assets) and the PNGs
  match: the yellow of `complet.png` = (252, 228, 48) vs `titel.jpg` =
  (253, 228, 48) (JPEG noise); dominant green = (0, 208, 0) in both
  (palette (0,52,0) << 2). **[O]** → confirms both the RGB order and the
  ×4 expansion.

## 8. Explicit uncertainties

1. **Byte 3** (`0x02`): probably the format version; the code does not read it. **[I]**
2. **Byte 9**: if ≠ 0 the game skips pixel decoding (branch
   `local_7f == 0` of `FUN_004070c4`). No file has it set to non-zero; it is not
   known what it represents (palette-only file? external pixels?). **[O the
   behavior; the meaning is unknown]**
3. **Byte 10** (`0x01`): no identified use. **[O the value; meaning unknown]**
4. **`DAT_00461148`** (global added to the byte-4 seek): probably the in-memory
   base of the file (0 when loading from disk). With 0 everything adds up. **[I]**
5. **u16/u32 rule of the length field**: observed as "exactly
   320×200 → u16, otherwise → u32". No files with other resolutions are known;
   it could actually be "if `w*h` fits in u16", but the code explicitly compares
   against 0x140 and 200. **[O the code; the general rule is untested]**
6. **`FUN_0040e721`** (read) and **`FUN_0040e962`** (seek): exact signatures
   inferred from their use (the Watcom register convention confuses the
   decompiler). The resulting file layout matches to the byte in the 7
   files. **[I]**
7. The `.pix` files embedded in `data.one` were extracted as-is; there is no sign of
   additional data around them (the loader opens standalone files by name). **[O]**

## 9. Using the decoder

```bash
python3 tools/pix2png.py assets/data/complet.pix -o assets/png/complet.png
python3 tools/pix2png.py --all assets/data --outdir assets/png --info
python3 tools/pix2png.py file.pix --scale 565   # 16bpp-mode colors
```

Output generated in `assets/png/` (7 PNGs: complet, emakers, fnts, gameover,
info, makers, spiel).
