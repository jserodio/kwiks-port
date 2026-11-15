//! Tests of the asset pipeline (`src/assets.rs`): `.pix` decoding against the
//! reference PNGs in `assets/png/`, JPEG loading and error handling with
//! corrupt headers.
//!
//! The crate is binary-only (no `lib.rs`), so the module is included directly
//! with `#[path]` to avoid altering the rest of the project.

#[path = "../src/assets.rs"]
mod assets;

use assets::{decode_pix, decode_pix_indexed, load_jpeg, load_pix_file, parse_pix_header, Palette,
             RgbaImage, PALETTE_SIZE};
use std::path::PathBuf;

/// (file, width, height) expected for the 7 `.pix` files of the game.
const PIX_FILES: &[(&str, u32, u32)] = &[
    ("complet.pix", 640, 480),
    ("gameover.pix", 640, 480),
    ("makers.pix", 640, 480),
    ("emakers.pix", 640, 480),
    ("info.pix", 640, 480),
    ("spiel.pix", 320, 200),
    ("fnts.pix", 320, 200),
];

fn data_file(name: &str) -> PathBuf {
    assets::asset_path("data").join(name)
}

fn png_file(name: &str) -> PathBuf {
    assets::asset_path("png").join(name)
}

/// Builds a synthetic `.pix` (u16 length field if 320×200, u32 otherwise).
fn build_pix(width: u16, height: u16, stream: &[u8], palette: &[u8; PALETTE_SIZE]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"PIX");
    out.push(0x02); // version
    out.push(0x0b); // offset of the length field
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.push(0x00); // flags
    out.push(0x01); // unknown
    if (width, height) == (320, 200) {
        out.extend_from_slice(&(stream.len() as u16).to_le_bytes());
    } else {
        out.extend_from_slice(&(stream.len() as u32).to_le_bytes());
    }
    out.extend_from_slice(stream);
    out.extend_from_slice(palette);
    out
}

/// Palette with entry `i` = (i, i, i) at 6 bits (useful to check the `v << 2`
/// expansion).
fn gray_palette() -> [u8; PALETTE_SIZE] {
    let mut pal = [0u8; PALETTE_SIZE];
    for (i, entry) in pal.chunks_exact_mut(3).enumerate() {
        let v = (i & 0x3f) as u8;
        entry.copy_from_slice(&[v, v, v]);
    }
    pal
}

// ---------------------------------------------------------------------------
// Exact dimensions of the 7 `.pix` files
// ---------------------------------------------------------------------------

#[test]
fn pix_files_decode_to_exact_dimensions() {
    for &(name, w, h) in PIX_FILES {
        let img = load_pix_file(&data_file(name))
            .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert_eq!((img.width, img.height), (w, h), "{name}: dimensions");
        assert_eq!(img.data.len(), (w * h * 4) as usize, "{name}: buffer size");
    }
}

// ---------------------------------------------------------------------------
// Byte-by-byte comparison against the reference PNGs
// ---------------------------------------------------------------------------

#[test]
fn pix_files_match_reference_pngs() {
    for &(name, w, h) in PIX_FILES {
        let decoded = load_pix_file(&data_file(name))
            .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        let png_name = name.replace(".pix", ".png");
        let reference = image::open(png_file(&png_name))
            .unwrap_or_else(|e| panic!("{png_name}: {e}"))
            .to_rgba8();
        assert_eq!(reference.dimensions(), (w, h), "{png_name}: PNG dimensions");

        // Pixel by pixel (RGB channels; the reference alpha is opaque).
        for y in 0..h {
            for x in 0..w {
                let got = decoded.pixel(x, y);
                let want = reference.get_pixel(x, y).0;
                assert_eq!(
                    got[0], want[0],
                    "{name}: different R at ({x}, {y}): {got:?} vs {want:?}"
                );
                assert_eq!(
                    got[1], want[1],
                    "{name}: different G at ({x}, {y}): {got:?} vs {want:?}"
                );
                assert_eq!(
                    got[2], want[2],
                    "{name}: different B at ({x}, {y}): {got:?} vs {want:?}"
                );
                assert_eq!(got[3], 255, "{name}: alpha not opaque at ({x}, {y})");
            }
        }
    }
}

#[test]
fn decode_pix_indexed_matches_load_pix_file() {
    for &(name, w, h) in PIX_FILES {
        let bytes = std::fs::read(data_file(name)).unwrap();
        let indexed = decode_pix_indexed(&bytes).unwrap();
        assert_eq!(indexed.indices.len(), (w * h) as usize, "{name}: indices");
        assert_eq!((indexed.header.width, indexed.header.height), (w as u16, h as u16));
        let rgba = indexed.to_rgba();
        let direct = decode_pix(&bytes).unwrap();
        assert_eq!(rgba.data, direct.data, "{name}: to_rgba vs decode_pix");
    }
}

// ---------------------------------------------------------------------------
// Original JPEGs
// ---------------------------------------------------------------------------

#[test]
fn load_jpeg_titel_is_640x480() {
    let img = load_jpeg(&data_file("titel.jpg")).expect("titel.jpg");
    assert_eq!((img.width, img.height), (640, 480));
    assert_eq!(img.data.len(), 640 * 480 * 4);
}

// ---------------------------------------------------------------------------
// RLE, raw and palette (synthetic files)
// ---------------------------------------------------------------------------

#[test]
fn rle_literals_and_repeats() {
    // 2×2: literal [1, 2] + repeat of 3 twice → indices [1, 2, 3, 3].
    let mut pal = [0u8; PALETTE_SIZE];
    pal[3..6].copy_from_slice(&[63, 0, 0]); // idx 1 → (252, 0, 0)
    pal[6..9].copy_from_slice(&[0, 63, 0]); // idx 2 → (0, 252, 0)
    pal[9..12].copy_from_slice(&[0, 0, 63]); // idx 3 → (0, 0, 252)
    let pix = build_pix(2, 2, &[0x02, 1, 2, 0x82, 3], &pal);
    let img = decode_pix(&pix).expect("rle");
    assert_eq!(img.pixel(0, 0), [252, 0, 0, 255]);
    assert_eq!(img.pixel(1, 0), [0, 252, 0, 255]);
    assert_eq!(img.pixel(0, 1), [0, 0, 252, 255]);
    assert_eq!(img.pixel(1, 1), [0, 0, 252, 255]);
}

#[test]
fn raw_stream_when_len_equals_wxh() {
    // Length == w*h → raw uncompressed pixels (§4).
    let pal = gray_palette();
    let pix = build_pix(2, 2, &[0, 32, 48, 63], &pal);
    let img = decode_pix(&pix).expect("raw");
    // Palette entry i = (i, i, i) at 6 bits → i << 2.
    assert_eq!(img.pixel(0, 0), [0, 0, 0, 255]);
    assert_eq!(img.pixel(1, 0), [128, 128, 128, 255]);
    assert_eq!(img.pixel(0, 1), [192, 192, 192, 255]);
    assert_eq!(img.pixel(1, 1), [252, 252, 252, 255]);
}

#[test]
fn palette_expands_with_shl2() {
    let pal = Palette::from_rgb6(&gray_palette()).unwrap();
    assert_eq!(pal.rgb8(0), [0, 0, 0]);
    assert_eq!(pal.rgb8(1), [4, 4, 4]);
    assert_eq!(pal.rgb8(52), [208, 208, 208]); // 52 << 2 = 208
    assert_eq!(pal.rgb8(63), [252, 252, 252]);
    assert_eq!(pal.rgba8(63), [252, 252, 252, 255]);
}

#[test]
fn stream_length_field_is_u16_for_320x200() {
    // 320×200 uses a u16 field (and here also raw: 64000 == w*h).
    let pal = gray_palette();
    let mut stream = vec![0u8; 320 * 200];
    stream[0] = 63;
    stream[319] = 32;
    let pix = build_pix(320, 200, &stream, &pal);
    // With u32 the file would carry 2 extra length bytes: we check that
    // parse_pix_header interprets the field as u16.
    let header = parse_pix_header(&pix).unwrap();
    assert_eq!(header.stream_len, 64000);
    assert_eq!(header.stream_offset, 13);
    assert_eq!(header.palette_offset, 13 + 64000);
    let img = decode_pix(&pix).expect("320x200");
    assert_eq!((img.width, img.height), (320, 200));
    assert_eq!(img.pixel(0, 0), [252, 252, 252, 255]);
    assert_eq!(img.pixel(319, 0), [128, 128, 128, 255]);
}

#[test]
fn header_fields_of_real_files() {
    // complet.pix: 640×480, u32 field @11 = 18588 (docs/PIX_FORMAT.md §2).
    let bytes = std::fs::read(data_file("complet.pix")).unwrap();
    let h = parse_pix_header(&bytes).unwrap();
    assert_eq!(&bytes[0..3], b"PIX");
    assert_eq!(h.version, 0x02);
    assert_eq!(h.data_offset, 0x0b);
    assert_eq!((h.width, h.height), (640, 480));
    assert_eq!((h.flags, h.unknown), (0x00, 0x01));
    assert_eq!(h.stream_len, 18588);
    assert_eq!(h.stream_offset, 15);
    assert_eq!(h.palette_offset, 15 + 18588);
    assert_eq!(bytes.len(), h.palette_offset + PALETTE_SIZE);

    // fnts.pix: 320×200, u16 field @11 = 1920.
    let bytes = std::fs::read(data_file("fnts.pix")).unwrap();
    let h = parse_pix_header(&bytes).unwrap();
    assert_eq!((h.width, h.height), (320, 200));
    assert_eq!(h.stream_len, 1920);
    assert_eq!(h.stream_offset, 13);
    assert_eq!(bytes.len(), h.palette_offset + PALETTE_SIZE);
}

// ---------------------------------------------------------------------------
// Error handling (corrupt headers and broken streams)
// ---------------------------------------------------------------------------

#[test]
fn error_empty_and_too_short() {
    assert!(decode_pix(b"").is_err());
    assert!(decode_pix(b"PIX\x02\x0b\x02\x00").is_err()); // < 11 bytes
}

#[test]
fn error_bad_magic() {
    let pal = gray_palette();
    let mut pix = build_pix(2, 2, &[0x84, 7], &pal);
    pix[0] = b'X';
    let err = decode_pix(&pix).unwrap_err().to_string();
    assert!(err.contains("magic"), "unexpected error: {err}");
}

#[test]
fn error_flags_nonzero() {
    let pal = gray_palette();
    let mut pix = build_pix(2, 2, &[0x84, 7], &pal);
    pix[9] = 0x01;
    let err = decode_pix(&pix).unwrap_err().to_string();
    assert!(err.contains("flags"), "unexpected error: {err}");
}

#[test]
fn error_data_offset_below_header() {
    let pal = gray_palette();
    let mut pix = build_pix(2, 2, &[0x84, 7], &pal);
    pix[4] = 0x05;
    let err = decode_pix(&pix).unwrap_err().to_string();
    assert!(err.contains("offset"), "unexpected error: {err}");
}

#[test]
fn error_truncated_length_field() {
    let pal = gray_palette();
    let pix = build_pix(2, 2, &[0x84, 7], &pal);
    assert!(decode_pix(&pix[..12]).is_err()); // the u32 length does not fit
}

#[test]
fn error_truncated_file() {
    for &(name, _, _) in PIX_FILES {
        let bytes = std::fs::read(data_file(name)).unwrap();
        assert!(decode_pix(&bytes[..bytes.len() - 1]).is_err(), "{name}: truncated");
    }
}

#[test]
fn error_trailing_garbage() {
    let bytes = std::fs::read(data_file("spiel.pix")).unwrap();
    let mut extended = bytes.clone();
    extended.push(0);
    let err = decode_pix(&extended).unwrap_err().to_string();
    assert!(err.contains("trailing"), "unexpected error: {err}");
}

#[test]
fn error_rle_output_length_mismatch() {
    let pal = gray_palette();
    // 2×2 needs 4 indices; this stream produces 5.
    let pix = build_pix(2, 2, &[0x85, 9], &pal);
    let err = decode_pix(&pix).unwrap_err().to_string();
    assert!(err.contains("more than 4"), "unexpected error: {err}");

    // ... and this one produces 3.
    let pix = build_pix(2, 2, &[0x83, 9], &pal);
    let err = decode_pix(&pix).unwrap_err().to_string();
    assert!(err.contains("3 bytes"), "unexpected error: {err}");
}

#[test]
fn error_rle_truncated_runs() {
    let pal = gray_palette();
    // Repeat run with no value byte.
    let pix = build_pix(2, 2, &[0x84], &pal);
    let err = decode_pix(&pix).unwrap_err().to_string();
    assert!(err.contains("repeat"), "unexpected error: {err}");

    // Literal that promises more bytes than are left.
    let pix = build_pix(2, 2, &[0x04, 1, 2], &pal);
    let err = decode_pix(&pix).unwrap_err().to_string();
    assert!(err.contains("literal"), "unexpected error: {err}");
}

#[test]
fn error_palette_truncated() {
    let pal = gray_palette();
    let pix = build_pix(2, 2, &[0x84, 7], &pal);
    assert!(Palette::from_rgb6(&pix[..100]).is_err());
    assert!(Palette::from_rgb6(&pal).is_ok());
}

// ---------------------------------------------------------------------------
// RgbaImage (contract)
// ---------------------------------------------------------------------------

#[test]
fn rgba_image_pixel_roundtrip_and_bounds() {
    let mut img = RgbaImage::new(3, 2);
    assert_eq!(img.dimensions(), (3, 2));
    assert_eq!(img.data.len(), 3 * 2 * 4);
    img.put_pixel(2, 1, [1, 2, 3, 4]);
    assert_eq!(img.pixel(2, 1), [1, 2, 3, 4]);
    assert_eq!(img.pixel(0, 0), [0, 0, 0, 0]);
    // Out of bounds: ignored write, transparent read.
    img.put_pixel(3, 0, [9, 9, 9, 9]);
    assert_eq!(img.pixel(3, 0), [0, 0, 0, 0]);
    assert_eq!(img.pixel(0, 2), [0, 0, 0, 0]);
}
