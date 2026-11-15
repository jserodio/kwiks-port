#!/usr/bin/env python3
"""pix2png.py — Decoder for the proprietary .PIX image format of "Kwiks" (1998).

Algorithm recovered from kwiks2.exe (PE32, Watcom C/C++32) with REA/Ghidra:

  * Loader / format dispatch : FUN_004070c4  (compares file extension against
                               ".TGA .PI1 .BMP .PCX .PIC .GIF .PIX" at 0x425759)
  * Header parse             : FUN_004070c4  (11-byte header, seek to byte 4)
  * RLE decompressor         : FUN_00407080
  * Raw (uncompressed) copy  : FUN_0040ed38  (memmove, used when len == w*h)
  * Palette lookup, RGB565   : FUN_0040cc28
  * Palette lookup, 24/32bpp : FUN_0040cca4  (+ FUN_00406f50 / FUN_00406fe8)

File layout (observed on the 7 shipped .pix files and cross-checked against
the decompiled code):

  offset  size  contents
  ------  ----  ---------------------------------------------------------
     0      3   magic "PIX"
     3      1   version (0x02 in all files)
     4      1   offset of the length field / image data (0x0b in all files)
     5      2   width  (u16 LE)
     7      2   height (u16 LE)
     9      1   flags (0x00 in all files; FUN_004070c4 skips pixel decode
                when != 0 — meaning otherwise unknown)
    10      1   unknown (0x01 in all files)
    11   2 | 4  compressed stream length (u16 LE when the image is exactly
                320x200, u32 LE otherwise — observed in FUN_004070c4)
    ..      n   compressed stream (see below)
     -    768   palette: 256 * (R, G, B), each channel 6 bits (0..63)

Stream decoding (FUN_00407080), producing exactly width*height bytes of
8-bit palette indices, row-major, top-down:

  while input remains:
      b = read byte
      if b & 0x80 == 0:   literal run : copy (b) raw bytes   (b = 0..0x7F)
      else:               repeat run  : write (b & 0x7F) copies of the next byte

If the stream length equals width*height the pixel data is stored raw
(no compression) — FUN_004070c4 calls memmove (FUN_0040ed38) in that case.

Palette channels are RGB, 6 bits per channel (VGA style):
  * 24bpp path (FUN_0040cca4): out bytes = (pal[2]<<2, pal[1]<<2, pal[0]<<2)
                               i.e. BGR in memory = RGB palette.  This v*4
                               expansion matches the original art colors
                               (cross-checked against the shipped JPEG assets).
  * RGB565 path (FUN_0040cc28): R = pal[0]>>1 -> bits 11..15,
                                G = pal[1]    -> bits 5..10,
                                B = pal[2]>>1 -> bits 0..4

Usage:
  python3 pix2png.py input.pix [-o out.png] [--scale shl2|565|linear] [--info]
  python3 pix2png.py --all <dir> --outdir <dir>
"""

import argparse
import os
import struct
import sys

try:
    from PIL import Image
except ImportError:  # pragma: no cover
    Image = None

MAGIC = b"PIX"


class PixError(ValueError):
    pass


def rle_decode(src: bytes, expected: int) -> bytearray:
    """FUN_00407080: decode the RLE stream into `expected` bytes."""
    out = bytearray()
    i = 0
    n = len(src)
    while i < n:
        b = src[i]
        i += 1
        if b & 0x80:
            count = b & 0x7F
            if i >= n:
                raise PixError("truncated repeat run")
            value = src[i]
            i += 1
            out += bytes([value]) * count
        else:
            if i + b > n:
                raise PixError("truncated literal run")
            out += src[i:i + b]
            i += b
        if len(out) > expected:
            raise PixError("RLE output exceeds width*height")
    if len(out) != expected:
        raise PixError(
            f"RLE output is {len(out)} bytes, expected {expected} (width*height)"
        )
    return out


def decode_pix(data: bytes):
    """Decode a .PIX file. Returns (width, height, indices, palette6, meta)."""
    if len(data) < 15 or data[0:3] != MAGIC:
        raise PixError("not a PIX file (bad magic)")
    version = data[3]
    data_off = data[4]
    width, height = struct.unpack_from("<HH", data, 5)
    flags9, byte10 = data[9], data[10]

    if flags9 != 0:
        raise PixError(f"unsupported header byte 9 = {flags9:#04x} (no pixel data?)")

    off = data_off
    if (width, height) == (320, 200):
        (stream_len,) = struct.unpack_from("<H", data, off)
        off += 2
    else:
        (stream_len,) = struct.unpack_from("<I", data, off)
        off += 4

    stream = data[off:off + stream_len]
    pal_off = off + stream_len
    palette6 = data[pal_off:pal_off + 768]
    trailing = data[pal_off + 768:]
    if len(stream) != stream_len:
        raise PixError("truncated compressed stream")
    if len(palette6) != 768:
        raise PixError("truncated palette (expected 768 bytes)")

    expected = width * height
    if stream_len == expected:
        indices = bytearray(stream)  # raw, uncompressed (FUN_0040ed38)
    else:
        indices = rle_decode(stream, expected)

    meta = {
        "version": version,
        "data_off": data_off,
        "flags9": flags9,
        "byte10": byte10,
        "stream_len": stream_len,
        "stream_off": off,
        "palette_off": pal_off,
        "trailing_bytes": len(trailing),
        "compressed": stream_len != expected,
    }
    return width, height, indices, palette6, meta


def palette_to_rgb8(palette6: bytes, scale: str) -> bytes:
    """Expand 256 * (R,G,B) 6-bit palette entries to 8-bit RGB.

    Scale "shl2" is the game's own 24/32bpp expansion (FUN_0040cca4 /
    FUN_00406f50: out = (pal[2]<<2, pal[1]<<2, pal[0]<<2) in BGR memory
    order) and reproduces the original art colors: the reference JPEG
    assets show exactly pal<<2 values (e.g. palette (0,52,0) -> (0,208,0)
    in titel.jpg / menu1.jpg).  Scale "565" reproduces what a 16bpp
    RGB565 display showed: R5 = pal[0]>>1, G6 = pal[1], B5 = pal[2]>>1
    (FUN_0040cc28), expanded back to 8 bits as (x5<<3)|(x5>>2) and
    (g6<<2)|(g6>>4).
    """
    out = bytearray(768)
    for i in range(256):
        r, g, b = (v & 0x3F for v in palette6[i * 3:i * 3 + 3])
        if scale == "shl2":
            out[i * 3:i * 3 + 3] = bytes((r << 2, g << 2, b << 2))
        elif scale == "565":
            r5, b5 = r >> 1, b >> 1
            out[i * 3:i * 3 + 3] = bytes(((r5 << 3) | (r5 >> 2),
                                          (g << 2) | (g >> 4),
                                          (b5 << 3) | (b5 >> 2)))
        elif scale == "linear":
            out[i * 3:i * 3 + 3] = bytes(((r * 255 + 31) // 63,
                                          (g * 255 + 31) // 63,
                                          (b * 255 + 31) // 63))
        else:
            raise PixError(f"unknown scale {scale!r}")
    return bytes(out)


def indices_to_image(width, height, indices, palette_rgb8):
    if Image is None:
        raise PixError("Pillow (PIL) is required to write PNG files")
    img = Image.frombytes("P", (width, height), bytes(indices))
    img.putpalette(palette_rgb8)
    return img


def convert(in_path, out_path, scale="shl2", info=False, quiet=False):
    data = open(in_path, "rb").read()
    width, height, indices, palette6, meta = decode_pix(data)
    pal8 = palette_to_rgb8(palette6, scale)
    img = indices_to_image(width, height, indices, pal8).convert("RGB")
    if out_path:
        img.save(out_path)
    if info:
        print(f"{os.path.basename(in_path)}: {width}x{height}, "
              f"stream={meta['stream_len']}B ({'compressed' if meta['compressed'] else 'raw'}), "
              f"palette@{meta['palette_off']}, trailing={meta['trailing_bytes']}")
    elif not quiet:
        print(f"{in_path} -> {out_path or '(no output)'} ({width}x{height})")
    return img


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("input", nargs="?", help="input .pix file")
    ap.add_argument("-o", "--output", help="output PNG (default: input with .png)")
    ap.add_argument("--scale", choices=["shl2", "565", "linear"], default="shl2",
                    help="6-bit palette expansion: shl2 = v*4, the game's own "
                         "24bpp expansion and the original art colors "
                         "(default); 565 = what a 16bpp RGB565 display showed; "
                         "linear = v*255/63")
    ap.add_argument("--all", dest="all_dir",
                    help="convert every .pix in this directory")
    ap.add_argument("--outdir", help="output directory for --all")
    ap.add_argument("--info", action="store_true", help="print header info")
    args = ap.parse_args(argv)

    if args.all_dir:
        outdir = args.outdir or args.all_dir
        os.makedirs(outdir, exist_ok=True)
        for name in sorted(os.listdir(args.all_dir)):
            if not name.lower().endswith(".pix"):
                continue
            out = os.path.join(outdir, os.path.splitext(name)[0] + ".png")
            convert(os.path.join(args.all_dir, name), out, args.scale,
                    info=args.info)
        return 0

    if not args.input:
        ap.error("input file required (or --all DIR)")
    out = args.output
    if out is None:
        out = os.path.splitext(args.input)[0] + ".png"
    convert(args.input, out, args.scale, info=args.info)
    return 0


if __name__ == "__main__":
    sys.exit(main())
