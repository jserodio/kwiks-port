//! Internal resolution and window scaling (letterbox).
//!
//! The original game is 640×480×16 fullscreen. The port keeps 640×480 as the
//! default internal resolution and scales the framebuffer to the window
//! preserving the aspect ratio (letterbox), just as `main.rs` does with
//! `set_logical_size`. This module concentrates the pure geometry (no SDL) so
//! it can be unit tested:
//!
//! - [`internal_size`]: internal field resolution for a given window.
//! - [`letterbox`]: centered destination rectangle that preserves the aspect ratio.
//! - [`integer_scale`]: maximum integer scale that fits in the window.
//!
//! # Extended widescreen field — design decision **[D]**
//!
//! The level image (`FELD*.JPG`) is 640×480 and the original always plays at
//! 4:3. The port's goal is an "extended field" on wide screens: more playable
//! area at the sides, without stretching the image. With `widescreen` enabled
//! the internal resolution grows **in width only** (the height stays at 480 and
//! the width follows the window's aspect ratio): 854×480 for 16:9, 768×480 for
//! 16:10, 1138×480 for 21:9, etc. The internal width is always even so the
//! extra area is split exactly between the two sides.
//!
//! What the extra area is (**[D]**, port decision, not an observation from the
//! original — the original had no extended field):
//!
//! - The extra area at the sides is **empty (water) field that the player can
//!   conquer**, not decoration: monsters roam it, the player draws lines in it
//!   and it counts toward the fill. The original 640×480 image is placed
//!   centered (see [`image_origin`]) and the margins
//!   `(width − 640) / 2` are symmetric.
//! - When conquering a zone that falls in the margins, **the image border is
//!   revealed extended outwards** (the outer column/row is repeated).
//!   Repeating the border is chosen over leaving a flat color fill because:
//!   (1) the engine (`Field::fill_from`) always reveals
//!   `image.pixel(x, y)`, so no pixel needs a special "no image" case;
//!   (2) the game's reward is revealing artwork and flat blocks at the sides
//!   would break it visually; (3) the rules stay homogeneous across the whole
//!   field: blue = not conquered, artwork = conquered. Known risk: if the
//!   original art had a recognizable motif right at the border, extending it
//!   would "tile" it; the FELD*.JPG files are textured backgrounds, so the
//!   effect is acceptable.
//! - **Integration**: when loading the level, expand the 640×480 image to the
//!   internal size by repeating the outer column/row (the original image in
//!   the center, see [`image_origin`]) and pass the already-expanded image to
//!   `Field::new`. This way the field, the rendering and the fill percentage
//!   work unchanged.
//! - The level threshold (75 %) is still computed over the total area of the
//!   internal field (`Field::fill_ratio`), so in widescreen there is more
//!   surface to conquer at the same percentage. **[D]** Accepted for rule
//!   uniformity; the alternative (fixed quota = 75 % of 640×480 px) would keep
//!   the per-level duration and remains a future refinement.
//! - The HUD (22 px top strip) spans the whole internal width (already the
//!   case: `screens::draw_hud` fills `out.width`).

/// Internal resolution of the playfield.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InternalSize {
    pub width: u32,
    pub height: u32,
}

/// Internal width of the original game (640×480×16).
pub const BASE_WIDTH: u32 = 640;

/// Internal height of the original game (640×480×16).
pub const BASE_HEIGHT: u32 = 480;

/// Computes the internal resolution: faithful 640×480, or extended to the
/// window's aspect ratio when `widescreen` is enabled (the height stays at 480
/// and the width is enlarged; e.g. 854×480 for 16:9).
///
/// With `widescreen` the width is `ceil(480 × window_width / window_height)`
/// rounded up to the nearest even number (never below 640): `ceil` is used so
/// the field is never narrower than the window's aspect ratio (this way the
/// window is filled side to side and only sub-pixel vertical bars remain) and
/// the even rounding splits the extra area into two exact halves. Degenerate
/// windows (0×0, portrait) return 640×480.
pub fn internal_size(window: (u32, u32), widescreen: bool) -> InternalSize {
    if !widescreen {
        return InternalSize {
            width: BASE_WIDTH,
            height: BASE_HEIGHT,
        };
    }
    let ww = window.0.max(1) as u64;
    let wh = window.1.max(1) as u64;
    // ceil(480 * ww / wh), rounded up to the nearest even number.
    let mut w = (BASE_HEIGHT as u64 * ww + wh - 1) / wh;
    w += w & 1;
    InternalSize {
        width: w.max(BASE_WIDTH as u64) as u32,
        height: BASE_HEIGHT,
    }
}

/// Origin (x, y) where the original 640×480 image is centered inside the
/// internal field (equal margins left and right; see the extended-field design
/// decision in the module documentation).
pub fn image_origin(size: InternalSize) -> (u32, u32) {
    ((size.width.saturating_sub(BASE_WIDTH)) / 2, 0)
}

/// Maximum integer scale possible inside `dst` (0 if the content does not fit
/// even at 1:1 scale).
pub fn integer_scale(src: (u32, u32), dst: (u32, u32)) -> u32 {
    let (sw, sh) = (src.0.max(1), src.1.max(1));
    (dst.0 / sw).min(dst.1 / sh)
}

/// Destination rectangle to scale the internal content to the window,
/// preserving the aspect ratio (letterbox). Returns (x, y, w, h).
///
/// - `integer_scale = true`: uses the largest integer scale that fits (see
///   [`integer_scale`]) and centers the result. If the window is smaller than
///   the content no integer ≥ 1 fits and it falls back to fractional fit
///   (**[D]**: filling the window is better than leaving a 0-sized rectangle).
/// - `integer_scale = false`: fractional fit to the largest possible size
///   without distortion (the limiting dimension is filled completely).
///
/// Centering uses integer division: if an odd pixel is left over it stays on
/// the right/bottom (bias of at most 1 px toward the top-left). A zero-sized
/// window returns (0, 0, 0, 0).
pub fn letterbox(
    src: (u32, u32),
    dst: (u32, u32),
    integer_scale: bool,
) -> (i32, i32, u32, u32) {
    let (sw, sh) = (src.0.max(1) as u64, src.1.max(1) as u64);
    let (dw, dh) = (dst.0 as u64, dst.1 as u64);
    if dw == 0 || dh == 0 {
        return (0, 0, 0, 0);
    }

    // Integer scale if it fits (and at least 1:1); otherwise fractional fit.
    let k = self::integer_scale(src, dst) as u64;
    let (w, h) = if integer_scale && k >= 1 {
        (k * sw, k * sh)
    } else if dw * sh <= dh * sw {
        // Width-limited: fill `dw` and round the height.
        (dw, ((dw * sh + sw / 2) / sw).max(1))
    } else {
        // Height-limited: fill `dh` and round the width.
        (((dh * sw + sh / 2) / sh).max(1), dh)
    };

    let w = w.min(dw);
    let h = h.min(dh);
    let x = ((dw - w) / 2) as i32;
    let y = ((dh - h) / 2) as i32;
    (x, y, w as u32, h as u32)
}
