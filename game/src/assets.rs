//! Loading of the original Kwiks (1998) assets.
//!
//! Responsible for decoding the `.pix` files (custom RLE, see
//! `docs/PIX_FORMAT.md`) and the original `.jpg` files into an RGBA
//! framebuffer.
//!
//! **Module contract** (used by `main.rs`, `screens.rs` and `field.rs`):
//! - [`RgbaImage`]: flat RGBA8 image in memory.
//! - [`decode_pix`] / [`load_pix_file`] / [`load_jpeg`]: decoders.
//! - [`asset_path`]: resolves paths inside `assets/`.
//!
//! Additionally [`PixHeader`] / [`parse_pix_header`] (format header),
//! [`Palette`] (256-color 6-bit palette) and [`decode_pix_indexed`] (8 bpp
//! palette indices without expansion, useful for the font in `fnts.pix`) are
//! exposed.

// Module API for `main.rs` / `screens.rs` / `field.rs`; until everything is
// wired up, we do not want dead_code warnings from partial use.
#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Palette size: 256 entries × (R, G, B).
pub const PALETTE_SIZE: usize = 768;

/// Flat RGBA8 image (row by row, top to bottom).
#[derive(Clone, Debug)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width * height * 4) as usize],
        }
    }

    /// Color at (x, y) as RGBA; out of range returns transparent.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    pub fn put_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        self.data[i..i + 4].copy_from_slice(&rgba);
    }

    /// Dimensions (width, height).
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Header of a `.pix` file (11 bytes + length field), see
/// `docs/PIX_FORMAT.md` §2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixHeader {
    /// Byte 3: probable format version (`0x02` in all files).
    pub version: u8,
    /// Byte 4: offset from the start of the file of the length field (0x0b).
    pub data_offset: u8,
    /// Width in pixels (u16 LE @5).
    pub width: u16,
    /// Height in pixels (u16 LE @7).
    pub height: u16,
    /// Byte 9: flags. If ≠ 0 the game does not decode pixels (meaning
    /// unknown, see §8).
    pub flags: u8,
    /// Byte 10: unknown (`0x01` in all files).
    pub unknown: u8,
    /// Length of the compressed stream in bytes (u16 if 320×200, u32 otherwise).
    pub stream_len: u32,
    /// Offset of the stream inside the file.
    pub stream_offset: usize,
    /// Offset of the palette (768 bytes) inside the file; it is the exact end.
    pub palette_offset: usize,
}

/// 256-color palette. The `.pix` files store 6-bit RGB (0..63) which is
/// expanded to 8 bits with `v << 2`, the expansion of the game's 24bpp path
/// (`docs/PIX_FORMAT.md` §5) and the one that reproduces the original colors
/// (the reference PNGs were generated with `--scale shl2`).
#[derive(Clone, Debug)]
pub struct Palette {
    rgb8: [[u8; 3]; 256],
}

impl Palette {
    /// Builds the palette from the last 768 bytes of the `.pix` file
    /// (256 × (R, G, B), 6 bits per channel).
    pub fn from_rgb6(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < PALETTE_SIZE {
            bail!(
                "pix: truncated palette ({} bytes, {PALETTE_SIZE} expected)",
                bytes.len()
            );
        }
        let mut rgb8 = [[0u8; 3]; 256];
        for (i, entry) in rgb8.iter_mut().enumerate() {
            let p = &bytes[i * 3..i * 3 + 3];
            // & 0x3f: the channels are 6 bits (defensive, like pix2png.py).
            *entry = [(p[0] & 0x3f) << 2, (p[1] & 0x3f) << 2, (p[2] & 0x3f) << 2];
        }
        Ok(Self { rgb8 })
    }

    /// RGB8 color of entry `index`.
    pub fn rgb8(&self, index: u8) -> [u8; 3] {
        self.rgb8[index as usize]
    }

    /// RGBA8 color (opaque alpha) of entry `index`.
    pub fn rgba8(&self, index: u8) -> [u8; 4] {
        let [r, g, b] = self.rgb8(index);
        [r, g, b, 255]
    }
}

/// A decoded `.pix`: header + 8 bpp palette indices (row by row, top to
/// bottom) + palette.
#[derive(Clone, Debug)]
pub struct PixImage {
    pub header: PixHeader,
    pub indices: Vec<u8>,
    pub palette: Palette,
}

impl PixImage {
    /// Palette index at (x, y). Out of range returns 0.
    pub fn pixel_index(&self, x: u32, y: u32) -> u8 {
        let (w, h) = (self.header.width as u32, self.header.height as u32);
        if x >= w || y >= h {
            return 0;
        }
        self.indices[(y * w + x) as usize]
    }

    /// Expands the indices with the palette into an [`RgbaImage`] (opaque alpha).
    pub fn to_rgba(&self) -> RgbaImage {
        let mut img = RgbaImage::new(self.header.width as u32, self.header.height as u32);
        for (i, &idx) in self.indices.iter().enumerate() {
            let rgba = self.palette.rgba8(idx);
            let o = i * 4;
            img.data[o..o + 4].copy_from_slice(&rgba);
        }
        img
    }
}

/// Resolves a path inside `assets/` (relative to the project root).
pub fn asset_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets").join(relative)
}

/// Parses and validates the header of a `.pix` file (11 bytes + length field).
///
/// Checks the `"PIX"` magic, data offset ≥ 11, that the length field (u16 if
/// 320×200, u32 otherwise) fits, and that the stream plus the 768-byte palette
/// reach the exact end of the file (no trailing bytes, as verified on the 7
/// real files: `docs/PIX_FORMAT.md` §2 and §7).
pub fn parse_pix_header(bytes: &[u8]) -> Result<PixHeader> {
    if bytes.len() < 11 {
        bail!(
            "pix: file too short ({} bytes; 11-byte header)",
            bytes.len()
        );
    }
    if &bytes[0..3] != b"PIX" {
        bail!("pix: bad magic (expected \"PIX\")");
    }
    let version = bytes[3];
    let data_offset = bytes[4] as usize;
    if data_offset < 11 {
        bail!("pix: data offset {data_offset} < 11 (invalid header)");
    }
    let width = u16::from_le_bytes([bytes[5], bytes[6]]);
    let height = u16::from_le_bytes([bytes[7], bytes[8]]);
    let flags = bytes[9];
    let unknown = bytes[10];

    // Length field rule (§2): u16 LE if the image is exactly 320×200,
    // u32 LE in any other case.
    let (stream_len, len_size) = if (width, height) == (320, 200) {
        if bytes.len() < data_offset + 2 {
            bail!("pix: truncated length field (u16)");
        }
        (
            u16::from_le_bytes([bytes[data_offset], bytes[data_offset + 1]]) as u32,
            2,
        )
    } else {
        if bytes.len() < data_offset + 4 {
            bail!("pix: truncated length field (u32)");
        }
        (
            u32::from_le_bytes([
                bytes[data_offset],
                bytes[data_offset + 1],
                bytes[data_offset + 2],
                bytes[data_offset + 3],
            ]),
            4,
        )
    };

    let stream_offset = data_offset + len_size;
    let palette_offset = stream_offset + stream_len as usize;
    let end = palette_offset + PALETTE_SIZE;
    if bytes.len() < end {
        bail!(
            "pix: truncated file ({} bytes, {end} needed: {stream_len}-byte stream + {PALETTE_SIZE}-byte palette)",
            bytes.len()
        );
    }
    if bytes.len() > end {
        bail!(
            "pix: {} trailing bytes after the palette (the file must end at {end})",
            bytes.len() - end
        );
    }

    Ok(PixHeader {
        version,
        data_offset: data_offset as u8,
        width,
        height,
        flags,
        unknown,
        stream_len,
        stream_offset,
        palette_offset,
    })
}

/// Decompresses the RLE stream of `FUN_00407080` (`docs/PIX_FORMAT.md` §3):
/// bit 7 of the control byte = 0 → literal of `b` bytes; = 1 → repeat the next
/// byte `b & 0x7F` times. It must produce exactly `expected` (= width × height)
/// bytes.
fn decode_rle(stream: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while i < stream.len() {
        let b = stream[i];
        i += 1;
        if b & 0x80 == 0 {
            // Literal: copy `b` bytes from the input.
            let count = b as usize;
            if i + count > stream.len() {
                bail!("pix: truncated literal run ({count} bytes)");
            }
            out.extend_from_slice(&stream[i..i + count]);
            i += count;
        } else {
            // Repeat: `b & 0x7F` copies of the next byte.
            let count = (b & 0x7f) as usize;
            if i >= stream.len() {
                bail!("pix: truncated repeat run");
            }
            let value = stream[i];
            i += 1;
            out.extend(std::iter::repeat(value).take(count));
        }
        if out.len() > expected {
            bail!("pix: RLE produces more than {expected} bytes (width × height)");
        }
    }
    if out.len() != expected {
        bail!(
            "pix: RLE produces {} bytes, {expected} expected (width × height)",
            out.len()
        );
    }
    Ok(out)
}

/// Decodes a `.pix` file (custom RLE + 6-bit RGB palette).
///
/// Layout: `"PIX"` + version + data_offset(0x0b) + w u16 + h u16 + flags +
/// unknown byte + length (u16 if 320×200, u32 otherwise) + RLE stream +
/// 768-byte palette (256 × 6-bit RGB).
///
/// If the stream length is exactly `w*h`, the data is raw uncompressed pixels
/// (§4). The palette is expanded to 8 bits with `v << 2`.
pub fn decode_pix(bytes: &[u8]) -> Result<RgbaImage> {
    Ok(decode_pix_indexed(bytes)?.to_rgba())
}

/// Like [`decode_pix`], but returns the 8 bpp indices and the palette without
/// expanding to color.
pub fn decode_pix_indexed(bytes: &[u8]) -> Result<PixImage> {
    let header = parse_pix_header(bytes)?;
    if header.flags != 0 {
        // §2/§8: if byte 9 ≠ 0 the game does not decode pixels. No real file
        // has it ≠ 0 and its meaning is unknown.
        bail!(
            "pix: flags = {:#04x} ≠ 0 (no pixel data; meaning unknown)",
            header.flags
        );
    }

    let stream_end = header.stream_offset + header.stream_len as usize;
    let stream = &bytes[header.stream_offset..stream_end];
    let palette = Palette::from_rgb6(&bytes[header.palette_offset..])?;

    let expected = header.width as usize * header.height as usize;
    let indices = if header.stream_len as usize == expected {
        // Uncompressed case (§4): the game does a memmove, no RLE involved.
        stream.to_vec()
    } else {
        decode_rle(stream, expected)?
    };

    Ok(PixImage {
        header,
        indices,
        palette,
    })
}

/// Loads and decodes a `.pix` file from disk.
pub fn load_pix_file(path: &Path) -> Result<RgbaImage> {
    let bytes = std::fs::read(path)?;
    decode_pix(&bytes)
}

/// Loads a `.jpg` from the original assets into RGBA8.
pub fn load_jpeg(path: &Path) -> Result<RgbaImage> {
    let rgba = image::open(path)?.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(RgbaImage {
        width,
        height,
        data: rgba.into_raw(),
    })
}

/// Size of the monster sprites (3 × 100×100 in `SPIEL.PIX`).
pub const MONSTER_SPRITE: u32 = 100;

/// The 3 monster sprites, cropped from `spiel.pix`.
///
/// `FUN_00402720` locks the `SPIEL.PIX` surface and copies three 100×100 u16
/// pixel blocks taken from columns 0, 100 and 200 of the first 100 rows. That
/// cropping is reproduced here over the decoded image.
///
/// **Transparency** is palette index 0, like the `0x0000` color-key of the
/// original: it is decided on the indices **before** expanding to RGB, so the
/// sprite background does not leave a dark box over the field.
pub fn monster_sprites() -> Result<Vec<RgbaImage>> {
    let path = asset_path("data/spiel.pix");
    let bytes = std::fs::read(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let sheet = crate::assets::decode_pix_indexed(&bytes)?;

    let mut out = Vec::with_capacity(3);
    for i in 0..3 {
        let mut sprite = RgbaImage::new(MONSTER_SPRITE, MONSTER_SPRITE);
        for y in 0..MONSTER_SPRITE {
            for x in 0..MONSTER_SPRITE {
                let sx = i * MONSTER_SPRITE + x;
                let idx = sheet.pixel_index(sx, y);
                // Index 0 = color-key (transparent).
                let rgba = if idx == 0 {
                    [0, 0, 0, 0]
                } else {
                    let mut c = sheet.palette.rgba8(idx);
                    c[3] = 0xff;
                    c
                };
                sprite.put_pixel(x, y, rgba);
            }
        }
        out.push(sprite);
    }
    Ok(out)
}
