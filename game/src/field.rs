//! Kwiks playfield: per-pixel state, collision and filling (flood fill).
//!
//! Reconstructs what the original does with three 640×480 buffers (see
//! `docs/GAME_LOGIC.md` §4):
//!
//! - a **u16 surface** with the drawn state (`DAT_0042901c`),
//! - its **mirror** (`DAT_00429018`) used to undo an invalid fill,
//! - the "sanitized" **image buffer** the pixels are copied from when filling
//!   (`DAT_00429024`), and
//! - the **fill counter** (`DAT_00429028`).
//!
//! Collision (`FUN_00401960`) is a **1-pixel query** whose result depends on
//! the stored value; here it is modeled with [`Cell`] and [`Collision`].

use crate::assets::RgbaImage;

/// Color of the monster marker on the state surface.
///
/// The original stores `0x7f00` (types 1/3) and `0x3e0` (worm) in RGB565.
/// `0x7f00` = R=15, G=60, B=0 ⇒ `(123, 243, 0)` when expanded to 8 bits.
pub const MARKER_COLOR: [u8; 4] = [123, 243, 0, 0xff];

/// Color of the field frame: the original paints `0x001f`, which in RGB565 is
/// **pure blue** (B=31 ⇒ 255).
pub const FRAME_COLOR: [u8; 4] = [0, 0, 255, 0xff];

/// State of a field pixel.
///
/// Equivalent to the values the original stores on the u16 surface:
/// `0x0000` free, `0xffff` own line, `0x001f` closed, `0x7f00`/`0x03e0`
/// monster markers; the remaining values are image pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Cell {
    /// Not conquered: the player can move through here.
    Empty = 0,
    /// Own white line in progress. Crossing it kills the player.
    Line = 1,
    /// Zone already closed/filled. It can be crossed.
    Closed = 2,
    /// A monster's marker (halo/trail).
    Monster = 3,
}

/// Result of a collision query, 1:1 with `FUN_00401960`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Collision {
    /// `0` — free.
    Free,
    /// `0xffff` — own line in progress.
    OwnLine,
    /// `0x1f` — closed zone.
    Closed,
    /// `0x7f00` / `0x03e0` — monster marker.
    Monster,
}

/// Playfield.
pub struct Field {
    width: i32,
    height: i32,
    /// Per-pixel state ([`Cell`] as `u8`), row by row.
    state: Vec<u8>,
    /// Mirror of the state, to undo invalid fills (like the original).
    mirror: Vec<u8>,
    /// Image revealed when a zone is conquered.
    image: RgbaImage,
    /// Conquered pixels (equivalent to `DAT_00429028`).
    filled: u32,
    /// Reusable `visited` marks for the flood fill, tagged with `visit_gen`
    /// so they do not need clearing between fills.
    visited: Vec<u32>,
    visit_gen: u32,
}

/// Result of a close/fill attempt.
#[derive(Clone, Copy, Debug)]
pub struct FillOutcome {
    /// Pixels conquered in this operation.
    pub pixels: u32,
    /// `true` if the area touched a monster and was undone (like the original).
    pub undone: bool,
}

impl Field {
    pub fn new(width: u32, height: u32, image: RgbaImage) -> Self {
        let n = (width * height) as usize;
        Self {
            width: width as i32,
            height: height as i32,
            state: vec![Cell::Empty as u8; n],
            mirror: vec![Cell::Empty as u8; n],
            image,
            filled: 0,
            visited: vec![0; n],
            visit_gen: 0,
        }
    }

    /// Complete playfield, with the **2 px frame** the original paints
    /// (`FUN_00401374`). That frame already counts as fill: in 640×480 it is
    /// 4464 px out of 307200, just as the original counter adds up.
    ///
    /// Kept separate from [`Field::new`] so the fill algorithm tests work on a
    /// clean field.
    pub fn new_playfield(width: u32, height: u32, image: RgbaImage) -> Self {
        let mut f = Self::new(width, height, image);
        f.paint_frame(2);
        f
    }

    /// Paints a closed frame of `t` pixels as already conquered zone.
    fn paint_frame(&mut self, t: i32) {
        for y in 0..self.height {
            for x in 0..self.width {
                if x < t || y < t || x >= self.width - t || y >= self.height - t {
                    if let Some(i) = self.index(x, y) {
                        if self.state[i] != Cell::Closed as u8 {
                            self.state[i] = Cell::Closed as u8;
                            self.mirror[i] = Cell::Closed as u8;
                            self.filled += 1;
                        }
                    }
                }
            }
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        Some((y * self.width + x) as usize)
    }

    /// State of the pixel. Outside the field it is considered closed (border).
    pub fn cell(&self, x: i32, y: i32) -> Cell {
        match self.index(x, y) {
            Some(i) => match self.state[i] {
                v if v == Cell::Line as u8 => Cell::Line,
                v if v == Cell::Closed as u8 => Cell::Closed,
                v if v == Cell::Monster as u8 => Cell::Monster,
                _ => Cell::Empty,
            },
            None => Cell::Closed,
        }
    }

    /// Collision query, 1:1 with `FUN_00401960`.
    pub fn collision(&self, x: i32, y: i32) -> Collision {
        match self.cell(x, y) {
            Cell::Empty => Collision::Free,
            Cell::Line => Collision::OwnLine,
            Cell::Closed => Collision::Closed,
            Cell::Monster => Collision::Monster,
        }
    }

    fn set(&mut self, x: i32, y: i32, cell: Cell) {
        if let Some(i) = self.index(x, y) {
            self.state[i] = cell as u8;
        }
    }

    /// Places a monster marker (halo or trail).
    ///
    /// The original writes the marker **on both surfaces** (state and mirror,
    /// `0x42901c` and `0x429018`) so that a fill undo does not erase it.
    /// Values: `0x7f00` types 1/3, `0x3e0` the worm.
    pub fn set_monster(&mut self, x: i32, y: i32) {
        if let Some(i) = self.index(x, y) {
            self.state[i] = Cell::Monster as u8;
            self.mirror[i] = Cell::Monster as u8;
        }
    }

    /// Erases a monster marker leaving the cell free, on both surfaces.
    pub fn clear_monster(&mut self, x: i32, y: i32) {
        if let Some(i) = self.index(x, y) {
            if self.state[i] == Cell::Monster as u8 {
                self.state[i] = Cell::Empty as u8;
            }
            if self.mirror[i] == Cell::Monster as u8 {
                self.mirror[i] = Cell::Empty as u8;
            }
        }
    }

    /// Erases the white line in progress without conquering anything
    /// (`FUN_004021e4`).
    ///
    /// This is what the original does on death: the white line **is removed**,
    /// it does not become a closed zone.
    pub fn erase_trail(&mut self) {
        for c in self.state.iter_mut() {
            if *c == Cell::Line as u8 {
                *c = Cell::Empty as u8;
            }
        }
    }

    /// Draws the own white line over the field (the player's trail).
    ///
    /// The original draws it with `FUN_0040a510` (line); here a Bresenham.
    pub fn plot_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (mut x, mut y) = (x0, y0);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.set(x, y, Cell::Line);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Snapshots the current state as a mirror, before a fill.
    pub fn snapshot(&mut self) {
        self.mirror.copy_from_slice(&self.state);
    }

    /// Restores the mirror (undoes an invalid fill).
    pub fn restore(&mut self) {
        self.state.copy_from_slice(&self.mirror);
    }

    /// Span-based fill from a seed, copying the image.
    ///
    /// Reconstructs `FUN_00401c8c`: walks the image by horizontal spans and
    /// conquers the connected empty cells. If a monster marker is touched
    /// during the walk, the area is **not** kept conquered (the original undoes
    /// it with the mirror).
    pub fn fill_from(&mut self, seed_x: i32, seed_y: i32) -> FillOutcome {
        if self.cell(seed_x, seed_y) != Cell::Empty {
            return FillOutcome {
                pixels: 0,
                undone: false,
            };
        }

        self.snapshot();

        let mut stack: Vec<(i32, i32)> = vec![(seed_x, seed_y)];
        // Reuse the scratch marks: bump the generation instead of clearing.
        if self.visited.len() != self.state.len() {
            self.visited.resize(self.state.len(), 0);
        }
        self.visit_gen = self.visit_gen.wrapping_add(1);
        if self.visit_gen == 0 {
            self.visited.fill(0);
            self.visit_gen = 1;
        }
        let gen = self.visit_gen;
        let mut touched_monster = false;
        let mut filled_now = 0u32;

        while let Some((x, y)) = stack.pop() {
            // Extend the span to the left and to the right.
            let mut left = x;
            while left > 0 && self.cell(left - 1, y) == Cell::Empty {
                left -= 1;
            }
            let mut right = x;
            while right + 1 < self.width && self.cell(right + 1, y) == Cell::Empty {
                right += 1;
            }

            for sx in left..=right {
                let i = self.index(sx, y).expect("span inside the field");
                if self.visited[i] == gen {
                    continue;
                }
                self.visited[i] = gen;

                // The span borders mark whether the area encloses a monster.
                if self.cell(sx, y - 1) == Cell::Monster
                    || self.cell(sx, y + 1) == Cell::Monster
                    || self.cell(sx - 1, y) == Cell::Monster
                    || self.cell(sx + 1, y) == Cell::Monster
                {
                    touched_monster = true;
                }

                self.state[i] = Cell::Closed as u8;
                filled_now += 1;

                for ny in [y - 1, y + 1] {
                    if ny < 0 || ny >= self.height {
                        continue;
                    }
                    // Push the span of the neighboring row hanging from this pixel.
                    let mut nx = sx;
                    if self.cell(nx, ny) == Cell::Empty {
                        while nx > 0 && self.cell(nx - 1, ny) == Cell::Empty {
                            nx -= 1;
                        }
                        stack.push((nx, ny));
                    }
                }
            }
        }

        if touched_monster {
            // The area contained a monster: it is undone, like in the original.
            self.restore();
            return FillOutcome {
                pixels: 0,
                undone: true,
            };
        }

        self.filled += filled_now;
        FillOutcome {
            pixels: filled_now,
            undone: false,
        }
    }

    /// Closing of an area after marking the line: seeds the fills.
    ///
    /// The original seeds 8 fills from the line endpoints (`FUN_00401f50`).
    /// Here the given seeds are tried and the valid fills are kept; those that
    /// touch a monster are discarded.
    ///
    /// TODO(geometry): the exact topology of the original's 8 seeds is
    /// documented as **[I]** in `GAME_LOGIC.md` §8; it should be verified
    /// dynamically before considering the port 1:1.
    pub fn close_area(&mut self, seeds: &[(i32, i32)]) -> FillOutcome {
        let mut total = 0;
        let mut undone_any = false;
        for &(x, y) in seeds {
            let out = self.fill_from(x, y);
            total += out.pixels;
            undone_any |= out.undone;
        }
        FillOutcome {
            pixels: total,
            undone: undone_any,
        }
    }

    /// Conquered pixels.
    pub fn filled(&self) -> u32 {
        self.filled
    }

    /// Cells of the own line in progress.
    pub fn trail_len(&self) -> u32 {
        self.state
            .iter()
            .filter(|&&c| c == Cell::Line as u8)
            .count() as u32
    }

    /// Closing of the line in progress: the line becomes a closed zone and the
    /// enclosed regions that do **not** contain monsters are conquered.
    ///
    /// Seeds per `FUN_00401f50`: the **4 diagonals of the player**
    /// `(px±1, py±1)` and the **4 axial neighbors of the line origin**.
    pub fn close_trail(&mut self, from: (i32, i32), to: (i32, i32)) -> FillOutcome {
        // 1) The whole white line becomes a closed zone (like the original).
        //    Those pixels **count** as fill: otherwise the percentage would be
        //    short compared to what is seen on screen.
        let mut trail_px = 0u32;
        for c in self.state.iter_mut() {
            if *c == Cell::Line as u8 {
                *c = Cell::Closed as u8;
                trail_px += 1;
            }
        }
        self.filled += trail_px;

        // 2) Seeds: 4 diagonals of the current endpoint + 4 axial neighbors of
        //    the origin (that is how `GAME_LOGIC.md` §4.3 describes them).
        let (px, py) = to;
        let (ox, oy) = from;
        let mut seeds: Vec<(i32, i32)> = vec![
            (px - 1, py - 1),
            (px + 1, py - 1),
            (px - 1, py + 1),
            (px + 1, py + 1),
            (ox - 1, oy),
            (ox + 1, oy),
            (ox, oy - 1),
            (ox, oy + 1),
        ];
        seeds.dedup();

        self.close_area(&seeds)
    }

    /// Total pixels of the field.
    pub fn total(&self) -> u32 {
        (self.width * self.height) as u32
    }

    /// Fill percentage in the range 0.0..=1.0.
    pub fn fill_ratio(&self) -> f32 {
        self.filled() as f32 / self.total() as f32
    }

    /// `true` if the level threshold has been reached (75 % in the original).
    pub fn level_complete(&self) -> bool {
        self.fill_ratio() >= 0.75
    }

    /// Draws the field leaving the top strip `[0, y0)` free (where the shell
    /// draws the HUD, like in the original).
    ///
    /// Colors verified in the binary (see `docs/GAME_LOGIC.md`): empty zone =
    /// **black `0x0000`**, frame = **blue `0x001f`**, own line = white `0xffff`,
    /// monster marker = `0x7f00` (green in RGB565).
    pub fn draw_playfield(&self, out: &mut RgbaImage, y0: u32) {
        // Hot path: one pass over the field writing RGBA straight into the
        // framebuffer. Index arithmetic is hoisted per row and the image is
        // sampled without bounds checks when its size matches the field.
        let direct_image =
            self.image.width as i32 == self.width && self.image.height as i32 == self.height;
        let out_stride = out.width as usize * 4;

        for y in (y0 as i32)..self.height {
            let state_row = (y * self.width) as usize;
            let out_row = y as usize * out_stride;
            let frame_row = y < 2 || y >= self.height - 2;

            for x in 0..self.width {
                let i = state_row + x as usize;
                let rgba = match self.state[i] {
                    v if v == Cell::Line as u8 => [0xff, 0xff, 0xff, 0xff],
                    v if v == Cell::Closed as u8 => {
                        if frame_row || x < 2 || x >= self.width - 2 {
                            FRAME_COLOR
                        } else if direct_image {
                            let o = i * 4;
                            [
                                self.image.data[o],
                                self.image.data[o + 1],
                                self.image.data[o + 2],
                                self.image.data[o + 3],
                            ]
                        } else {
                            self.image.pixel(x as u32, y as u32)
                        }
                    }
                    v if v == Cell::Monster as u8 => MARKER_COLOR,
                    _ => [0x00, 0x00, 0x00, 0xff],
                };
                let o = out_row + x as usize * 4;
                out.data[o..o + 4].copy_from_slice(&rgba);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(w: u32, h: u32) -> Field {
        Field::new(w, h, RgbaImage::new(w, h))
    }

    #[test]
    fn two_px_frame_counts_as_fill() {
        // The original starts the counter at 4464 (640×480 − 636×476).
        let f = Field::new_playfield(640, 480, RgbaImage::new(640, 480));
        assert_eq!(f.filled(), 4464);
        assert_eq!(f.total(), 307200);
    }

    #[test]
    fn per_pixel_collision_query() {
        let mut f = blank(10, 10);
        assert_eq!(f.collision(5, 5), Collision::Free);
        f.plot_line(2, 2, 6, 2);
        assert_eq!(f.collision(4, 2), Collision::OwnLine);
        f.set_monster(8, 8);
        assert_eq!(f.collision(8, 8), Collision::Monster);
        assert_eq!(f.collision(99, 99), Collision::Closed);
    }

    #[test]
    fn fills_connected_area() {
        let mut f = blank(8, 8);
        // A closed 4x4 square (2x2 interior) around (2,2).
        f.plot_line(1, 1, 4, 1);
        f.plot_line(4, 1, 4, 4);
        f.plot_line(4, 4, 1, 4);
        f.plot_line(1, 4, 1, 1);
        let out = f.fill_from(2, 2);
        assert!(!out.undone);
        assert_eq!(out.pixels, 2 * 2);
        assert!(f.level_complete() == false); // 4/64
        assert_eq!(f.collision(2, 2), Collision::Closed);
    }

    #[test]
    fn does_not_fill_if_monster_inside() {
        let mut f = blank(8, 8);
        f.plot_line(1, 1, 5, 1);
        f.plot_line(5, 1, 5, 5);
        f.plot_line(5, 5, 1, 5);
        f.plot_line(1, 5, 1, 1);
        f.set_monster(3, 3);
        let out = f.fill_from(2, 2);
        assert!(out.undone);
        assert_eq!(out.pixels, 0);
        // Nothing conquered.
        assert_eq!(f.filled(), 0);
    }

    #[test]
    fn level_threshold_at_75_percent() {
        let mut f = blank(4, 4);
        f.plot_line(0, 0, 11, 0); // top border
        for y in 0..3 {
            for x in 0..4 {
                f.fill_from(x, y);
            }
        }
        assert_eq!(f.filled(), 12);
        assert!(f.level_complete());
    }
}
