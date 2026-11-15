#!/usr/bin/env python3
"""Extract the assets from data.one / audio.one of Kwiks (1998).

Container format (.one):
    char[2]  "OO"
    u32      n_entries
    n_entries x { char name[1024]; u32 offset; u32 size }   # stride 0x408

The names are encoded with XOR 0xDF. Digits are stored as 0x10 + digit and the
dot as 0x0E.
"""
import struct
import sys
from pathlib import Path

NAME_STRIDE = 0x408
NAME_LEN = 0x400


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


def read_entries(blob: bytes):
    if blob[:2] != b"OO":
        raise ValueError("not an OO container")
    count = struct.unpack_from("<I", blob, 2)[0]
    entries = []
    for i in range(count):
        base = 6 + i * NAME_STRIDE
        name = decode_name(blob[base : base + NAME_LEN])
        offset, size = struct.unpack_from("<II", blob, base + NAME_LEN)
        entries.append((name, offset, size))
    return entries


def extract(container: Path, outdir: Path):
    blob = container.read_bytes()
    outdir.mkdir(parents=True, exist_ok=True)
    for name, offset, size in read_entries(blob):
        target = outdir / name.lower()
        target.write_bytes(blob[offset : offset + size])
        print(f"{target}  {size} bytes")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(f"usage: {sys.argv[0]} <data.one|audio.one> <output-dir>")
    extract(Path(sys.argv[1]), Path(sys.argv[2]))
