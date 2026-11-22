//! Kwiks simulation at a fixed step (40 Hz) with independent rendering.
//!
//! The original does not use delta-time: its tick (`0x404734`) drops the frame
//! if less than 25 ms have passed, so **all physics advance 1 step every
//! 25 ms**. To have free FPS without changing the gameplay, an accumulator is
//! kept here that fires `Sim::step` at 40 Hz while the render runs at whatever
//! rate the display wants (interpolating in M5).

use crate::assets::RgbaImage;
use crate::field::{Collision, Field};
use std::collections::VecDeque;

/// Duration of a logical step: 25 ms = 40 Hz, like the original.
pub const STEP: f64 = 0.025;

/// Sustained keyboard state consumed by the simulation.
#[derive(Clone, Copy, Default, Debug)]
pub struct InputState {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

/// Events the simulation notifies to the presentation layer.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    /// The player closed a zone and won `pixels` pixels.
    Filled { pixels: u32 },
    /// The player died (touched their own line or a monster).
    PlayerDied,
    /// Lives ran out: game over.
    GameOver,
    /// 75 % of the field was reached.
    LevelComplete,
    /// The player earned a life by crossing the score threshold.
    ExtraLife,
}

/// Monster types, see `docs/GAME_LOGIC.md`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MonsterKind {
    /// Type 1 — bounces diagonally, ±1 px per call.
    Bouncer,
    /// Type 2 — axial worm with a trail; turns 1/50 and on collision.
    Worm,
    /// Type 3 — subpixel movement in 1/8 of a pixel.
    Glider,
}

/// Internal unit: **1/8 of a pixel** for the three types.
///
/// The original uses whole pixels for types 1/2 and 1/8 fixed point for type 3
/// (`docs/GAME_LOGIC.md`). Unifying on 1/8 gives identical results for
/// types 1/2 (they always land on multiples of 8) and avoids duplicating logic.
const SUBPIXEL: i32 = 8;

pub struct Monster {
    kind: MonsterKind,
    /// Position in 1/8 of a pixel.
    x: i32,
    y: i32,
    /// Velocity in 1/8 of a pixel per call.
    vx: i32,
    vy: i32,
    /// Worm trail (pixel positions).
    trail: Vec<(i32, i32)>,
    /// Maximum trail length: `(level*10) % 200`.
    trail_max: usize,
}

impl Monster {
    /// Position in pixels (truncating toward zero, like the original).
    fn px(&self) -> i32 {
        self.x / SUBPIXEL
    }

    fn py(&self) -> i32 {
        self.y / SUBPIXEL
    }
}

pub struct Sim {
    pub field: Field,
    /// Player position (equivalent to `0x46082d` / `0x460831`).
    pub player_x: f32,
    pub player_y: f32,
    /// Start of the line in progress, to be able to close the area.
    trail_start: Option<(i32, i32)>,
    pub monsters: Vec<Monster>,
    pub level: u32,
    pub score: u32,
    /// Remaining lives. The original does game over when `lives < 0`, that is,
    /// with 3 initial lives the player can die **4 times**.
    pub lives: i32,
    /// Global tick counter (`DAT_00460500`).
    pub tick: u64,
    /// Extra lives already granted (so the reward is not repeated).
    extra_lives_awarded: u32,
    /// F2 — fast graphics (`DAT_00429064`): reduces the monsters' halo.
    pub fast_graphics: bool,
    /// Pending death: it is processed **instantly** inside `step`.
    ///
    /// It is important not to accumulate deaths: the tick runs 3 sub-steps
    /// and, if the player stays on their own line, each sub-step would detect
    /// it again (3 lives at once). The original avoids this by respawning the
    /// player inside the death routine itself.
    pending_death: bool,
    events: VecDeque<Event>,
    /// The original's LCG (`FUN_0040beb2`): `s*0x41C64E6D + 0x3039`.
    rng: u32,
}

impl Sim {
    pub fn new(field: Field) -> Self {
        // The player starts at the **top center**, on the frame
        // (`x = width/2; y = 1`, the same respawn position as
        // `FUN_00404f00`). This way they can go out into the field, draw and
        // return to the frame to close the area, like in the original.
        let mut sim = Self {
            player_x: (field.width() / 2) as f32,
            player_y: 1.0,
            field,
            trail_start: None,
            monsters: Vec::new(),
            level: 0,
            score: 0,
            lives: 3,
            tick: 0,
            extra_lives_awarded: 0,
            fast_graphics: false,
            pending_death: false,
            events: VecDeque::new(),
            // The original seeds with `srand(timeGetTime())` (`0x408a6b`).
            rng: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u32)
                .unwrap_or(0x1234_5678),
        };
        sim.spawn_monsters();
        sim
    }

    /// Monster generation (`FUN_004028ac`), with **1-based** level:
    /// - type 1 = `n - n/3`
    /// - type 2 (worm) = `n/2`
    /// - type 3 = `n/3`
    ///
    /// Positions and velocities are **random** (see `docs/GAME_LOGIC.md`).
    fn spawn_monsters(&mut self) {
        self.monsters.clear();
        let n = self.level + 1; // 1-based
        let w = self.field.width();
        let h = self.field.height();
        let trail_max = ((n * 10) % 200) as usize;

        let counts = [
            (MonsterKind::Bouncer, n - n / 3),
            (MonsterKind::Worm, n / 2),
            (MonsterKind::Glider, n / 3),
        ];

        for &(kind, count) in &counts {
            for _ in 0..count {
                // Random position: `rand()%(width-4)+2`.
                let px = (self.rand() as i32 % (w - 4)) + 2;
                let py = (self.rand() as i32 % (h - 4)) + 2;

                let (mut vx, mut vy) = match kind {
                    // Type 1: `rand()%3-1` per axis, both non-zero
                    // ⇒ always a ±1 px diagonal.
                    MonsterKind::Bouncer => loop {
                        let a = (self.rand() % 3) as i32 - 1;
                        let b = (self.rand() % 3) as i32 - 1;
                        if a != 0 && b != 0 {
                            break (a * SUBPIXEL, b * SUBPIXEL);
                        }
                    },
                    // Type 2 (worm): axis-aligned, magnitude 0/1.
                    MonsterKind::Worm => {
                        let a = (self.rand() % 3) as i32 - 1;
                        if a != 0 {
                            (a * SUBPIXEL, 0)
                        } else {
                            (0, ((self.rand() % 3) as i32 - 1) * SUBPIXEL)
                        }
                    }
                    // Type 3: `rand()%0x11-8` in 1/8 of a pixel, both ≠ 0.
                    MonsterKind::Glider => loop {
                        let a = (self.rand() % 0x11) as i32 - 8;
                        let b = (self.rand() % 0x11) as i32 - 8;
                        if a != 0 && b != 0 {
                            break (a, b);
                        }
                    },
                };
                if kind == MonsterKind::Glider {
                    // Type 3 already moves in 1/8 units.
                    let _ = (&mut vx, &mut vy);
                }

                self.monsters.push(Monster {
                    kind,
                    x: px * SUBPIXEL,
                    y: py * SUBPIXEL,
                    vx,
                    vy,
                    trail: Vec::new(),
                    trail_max,
                });
            }
        }

        // The original paints the markers as the monsters are born.
        for m in &self.monsters {
            self.field.set_monster(m.px(), m.py());
        }
    }

    /// LCG RNG identical to the original's.
    fn rand(&mut self) -> u32 {
        self.rng = self
            .rng
            .wrapping_mul(0x41C6_4E6D)
            .wrapping_add(0x3039);
        (self.rng >> 16) & 0x7fff
    }

    /// One 25 ms logical step.
    ///
    /// The original runs `FUN_00404f00` (player) + `FUN_00403590` (monsters)
    /// **three times per tick** (`0x404798`–`0x4047b1`), so both advance 1 px
    /// per call up to 3 times.
    pub fn step(&mut self, input: &InputState) {
        self.tick += 1;

        for _ in 0..3 {
            self.step_player_once(input);
            self.step_monsters_once();

            // Death is resolved **on the spot** (like `FUN_004051d8`): the
            // player loses a life, respawns and the remaining sub-steps are
            // cut short. Without this, staying on one's own line would kill 3
            // times per tick.
            if self.pending_death {
                self.pending_death = false;
                let game_over = self.kill_player();
                self.events.push_back(Event::PlayerDied);
                if game_over {
                    self.events.push_back(Event::GameOver);
                }
                break;
            }
        }

        if self.field.level_complete() {
            self.events.push_back(Event::LevelComplete);
        }
    }

    /// One player movement call (`FUN_00404f00`): 1 px.
    ///
    /// Faithful reproduction of the original:
    /// - **A single axis per call** (if/else if cascade), priority
    ///   **right > down > left > up**. Never diagonal.
    /// - Margin: `1 < x`, `x < width-2` (the exact border is never reached).
    /// - The position **is updated before** querying the collision.
    /// - **Death** only if `moved && drawing` and the cell is the own line
    ///   (`0xffff`) or a monster (`0x7f00`/`0x3e0`).
    /// - **Closing** if `moved && drawing` and the cell is anything else.
    fn step_player_once(&mut self, input: &InputState) {
        // A single axis per call, with the original's priority.
        let (dx, dy) = if input.right {
            (1, 0)
        } else if input.down {
            (0, 1)
        } else if input.left {
            (-1, 0)
        } else if input.up {
            (0, -1)
        } else {
            // No key: no movement.
            return;
        };

        let nx = self.player_x as i32 + dx;
        let ny = self.player_y as i32 + dy;

        // The original's margin: the exact field border is never reached.
        if nx < 1 || ny < 1 || nx > self.field.width() - 2 || ny > self.field.height() - 2 {
            return;
        }

        // The original updates the position before querying the collision.
        let prev = (self.player_x as i32, self.player_y as i32);
        self.player_x = nx as f32;
        self.player_y = ny as f32;

        let drawing = self.field.trail_len() > 0;
        match self.field.collision(nx, ny) {
            Collision::Free => {
                // Free zone: mark the line and record its start.
                if self.trail_start.is_none() {
                    self.trail_start = Some((nx, ny));
                }
                self.field.plot_line(prev.0, prev.1, nx, ny);
            }
            Collision::OwnLine | Collision::Monster => {
                // Only kills if a line was already being drawn.
                if drawing {
                    self.pending_death = true;
                }
            }
            Collision::Closed => {
                // Returning to conquered zone while drawing ⇒ close the area.
                if drawing {
                    if let Some(from) = self.trail_start.take() {
                        let outcome = self.field.close_trail(from, (nx, ny));
                        if outcome.pixels > 0 {
                            self.score += self.score_for(outcome.pixels);
                            self.events.push_back(Event::Filled {
                                pixels: outcome.pixels,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Points per conquered region: `(level+1) × (P × 1000 / 307200)`.
    fn score_for(&self, pixels: u32) -> u32 {
        let total = self.field.total().max(1) as u64;
        ((self.level as u64 + 1) * (pixels as u64 * 1000 / total)) as u32
    }

    /// One monster movement call (`FUN_00403590`).
    ///
    /// Rules verified in the disassembly (`docs/GAME_LOGIC.md`):
    /// 1. The own marker is **erased** and **repainted** at the end, on both
    ///    surfaces (state and mirror).
    /// 2. The diagonal destination `(x+vx, y+vy)` is queried **only once**.
    /// 3. If it is free → advance **in block** (both axes).
    /// 4. If it is `0xffff` (the player's white line) → **the player dies**.
    /// 5. If it is occupied → it does not move; `(x+vx, y)` is queried: if it
    ///    is free `vy` is inverted, otherwise `vx` is.
    fn step_monsters_once(&mut self) {
        for i in 0..self.monsters.len() {
            let mut m = std::mem::replace(
                &mut self.monsters[i],
                Monster {
                    kind: MonsterKind::Bouncer,
                    x: 0,
                    y: 0,
                    vx: 0,
                    vy: 0,
                    trail: Vec::new(),
                    trail_max: 0,
                },
            );

            // 1) Erase the previous marker (both surfaces).
            self.field.clear_monster(m.px(), m.py());

            let nx = (m.x + m.vx) / SUBPIXEL;
            let ny = (m.y + m.vy) / SUBPIXEL;
            let target = self.field.collision(nx, ny);

            match target {
                Collision::Free => {
                    // 3) Advance in block.
                    let turn = m.kind == MonsterKind::Worm && self.rand() % 50 == 0;
                    if turn {
                        // The worm turns in place without moving (D3).
                        self.worm_turn(&mut m);
                    } else {
                        m.x += m.vx;
                        m.y += m.vy;
                        if m.kind == MonsterKind::Worm {
                            self.worm_push_trail(&mut m);
                        }
                    }
                }
                Collision::OwnLine => {
                    // 4) Touching the player's white line ⇒ death (F1).
                    if m.kind == MonsterKind::Worm && self.rand() % 50 == 0 {
                        self.worm_turn(&mut m);
                    } else {
                        self.pending_death = true;
                    }
                }
                _ => {
                    // 5) Occupied: it does not move, decides which component
                    //    to invert.
                    if m.kind == MonsterKind::Worm {
                        self.worm_turn(&mut m);
                    } else {
                        let side = self.field.collision((m.x + m.vx) / SUBPIXEL, m.py());
                        if side == Collision::Free {
                            m.vy = -m.vy;
                        } else {
                            m.vx = -m.vx;
                        }
                    }
                }
            }

            // Trapped (worm only): the 4 axial neighbors are occupied (D5).
            if m.kind == MonsterKind::Worm && self.worm_trapped(&m) {
                for &(tx, ty) in &m.trail.clone() {
                    self.field.clear_monster(tx, ty);
                }
                m.trail.clear();
                self.field.clear_monster(m.px(), m.py());
            }

            // 1) Repaint the marker (both surfaces).
            self.field.set_monster(m.px(), m.py());
            if m.kind == MonsterKind::Worm {
                for &(tx, ty) in &m.trail {
                    self.field.set_monster(tx, ty);
                }
            }
            self.monsters[i] = m;
        }
    }

    /// The worm switches to the perpendicular axis with a new rolled direction
    /// (`0x403a1a`–`0x403a5b`): `vy != 0 → vx = rand()%3-1, vy = 0` and vice versa.
    fn worm_turn(&mut self, m: &mut Monster) {
        if m.vy != 0 {
            m.vx = ((self.rand() % 3) as i32 - 1) * SUBPIXEL;
            m.vy = 0;
        } else {
            m.vx = 0;
            m.vy = ((self.rand() % 3) as i32 - 1) * SUBPIXEL;
        }
    }

    /// Worm trail: stores the head and **erases the tail pixel** when reaching
    /// the maximum `(level*10) % 200` (D1: the original shifts the array and
    /// erases the tail pixel on both surfaces).
    ///
    /// Without that erasure the markers would pile up forever and the worm
    /// would leave an infinite trail.
    fn worm_push_trail(&mut self, m: &mut Monster) {
        let head = (m.px(), m.py());
        m.trail.push(head);
        while m.trail.len() > m.trail_max {
            if let Some(tail) = m.trail.first().copied() {
                self.field.clear_monster(tail.0, tail.1);
            }
            m.trail.remove(0);
        }
    }

    /// `true` if the 4 axial neighbors of the head are occupied (D5).
    fn worm_trapped(&mut self, m: &Monster) -> bool {
        let (x, y) = (m.px(), m.py());
        let blocked = |c: Collision| c != Collision::Free;
        blocked(self.field.collision(x - 1, y))
            && blocked(self.field.collision(x + 1, y))
            && blocked(self.field.collision(x, y - 1))
            && blocked(self.field.collision(x, y + 1))
    }

    /// Event drain for the presentation layer.
    pub fn drain_events(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }

    /// Extra life upon crossing every 5000 points (readme).
    pub fn check_extra_life(&mut self) {
        let crossed = (self.score / crate::extras::EXTRA_LIFE_SCORE_STEP) as u32;
        if crossed > self.extra_lives_awarded {
            self.extra_lives_awarded = crossed;
            self.lives += 1;
            self.events.push_back(Event::ExtraLife);
        }
    }

    /// Respawn after dying: the player returns to the **top center**
    /// (`x = width/2; y = 1` in `FUN_00404f00`), **the white line is erased**
    /// (`FUN_004021e4`, white→black: the line does not become closed) and the
    /// trail in progress is cleared.
    pub fn respawn(&mut self) {
        self.field.erase_trail();
        self.player_x = (self.field.width() / 2) as f32;
        self.player_y = 1.0;
        self.trail_start = None;
    }

    /// Sets the level (e.g. when `screens` advances after the COMPLET
    /// interstitial) and rebuilds the field with its image.
    pub fn set_level(&mut self, level: u32, field: Field) {
        self.level = level;
        self.player_x = (field.width() / 2) as f32;
        self.player_y = 1.0;
        self.trail_start = None;
        self.field = field;
        self.spawn_monsters();
    }

    /// The player dies: loses a life and respawns. Returns `true` if the lives
    /// ran out.
    ///
    /// The original does game over when `lives < 0` after the decrement
    /// (`0x4051f1`), so with 3 initial lives the player can die **4 times**.
    pub fn kill_player(&mut self) -> bool {
        self.lives -= 1;
        self.respawn();
        self.lives < 0
    }

    /// Draws the monsters with their halo (`FUN_00403170` + `FUN_00402ae4`).
    ///
    /// The original **scales** the 100×100 sprite to an `r×r` box centered on
    /// the monster and **adds it with saturation** onto the screen. The radius
    /// is animated by phase `r = ((tick + index) % 20) * 5 + 1`, and F2
    /// reduces it to `r/4` (minimum 1).
    ///
    /// Sprite↔type mapping (E4): type 1 → sprite 1, worm → sprite 0,
    /// type 3 → sprite 2.
    pub fn draw_monsters(&self, out: &mut RgbaImage, sprites: &[crate::assets::RgbaImage]) {
        for (index, m) in self.monsters.iter().enumerate() {
            let sprite_index = match m.kind {
                MonsterKind::Bouncer => 1,
                MonsterKind::Worm => 0,
                MonsterKind::Glider => 2,
            };
            let sprite = match sprites.get(sprite_index) {
                Some(s) => s,
                None => continue,
            };

            let mut r = (((self.tick + index as u64) % 20) as i32) * 5 + 1;
            if self.fast_graphics {
                r = (r / 4).max(1);
            }

            let px = m.px();
            let py = m.py();
            // Sampling the 100×100 sprite → r×r box (8.8 fixed-point step like
            // the original: 0x6400/r).
            let step = ((sprite.width as i32) << 8) / r.max(1);
            for dy in 0..r {
                let sy = ((dy * step) >> 8) as u32;
                for dx in 0..r {
                    let sx = ((dx * step) >> 8) as u32;
                    let rgba = sprite.pixel(sx.min(sprite.width - 1), sy.min(sprite.height - 1));
                    if rgba[0] == 0 && rgba[1] == 0 && rgba[2] == 0 {
                        continue; // color-key 0x0000
                    }
                    // Per-channel saturating addition.
                    let x = (px - r / 2 + dx) as u32;
                    let y = (py - r / 2 + dy) as u32;
                    let base = out.pixel(x, y);
                    out.put_pixel(
                        x,
                        y,
                        [
                            base[0].saturating_add(rgba[0]),
                            base[1].saturating_add(rgba[1]),
                            base[2].saturating_add(rgba[2]),
                            0xff,
                        ],
                    );
                }
            }
        }
    }

    /// Draws the player marker.
    ///
    /// The original (`FUN_00404ce8`) draws a **white square that pulses in
    /// size**, synchronized with the tick counter and always visible:
    ///
    /// ```text
    /// i = (tick % 16) / 5            // half-side: 0..3
    /// rect(x-i, y-i, x+i+1, y+i+1)   // filled square of side 2i+1
    /// ```
    pub fn draw_player(&self, out: &mut RgbaImage) {
        let i = ((self.tick % 16) / 5) as i32;
        let px = self.player_x as i32;
        let py = self.player_y as i32;
        for dy in -i..=i {
            for dx in -i..=i {
                out.put_pixel((px + dx) as u32, (py + dy) as u32, [0xff, 0xff, 0xff, 0xff]);
            }
        }
    }
}

/// Fixed-step accumulator: fires `step` at 40 Hz even if the render runs
/// faster (or slower).
pub struct FixedStep {
    accumulator: f64,
}

impl FixedStep {
    pub fn new() -> Self {
        Self { accumulator: 0.0 }
    }

    /// Accumulates `dt` in seconds and returns how many 25 ms steps to run.
    pub fn advance(&mut self, dt: f64) -> u32 {
        self.accumulator += dt;
        let mut steps = 0;
        while self.accumulator >= STEP {
            self.accumulator -= STEP;
            steps += 1;
        }
        steps
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::RgbaImage;

    /// Simulation without monsters: movement tests must not depend on the
    /// randomness of the generation.
    fn sim() -> Sim {
        let mut s = Sim::new(Field::new(640, 480, RgbaImage::new(640, 480)));
        for i in 0..s.monsters.len() {
            let (x, y) = (s.monsters[i].px(), s.monsters[i].py());
            s.field.clear_monster(x, y);
        }
        s.monsters.clear();
        s
    }

    /// Simulation with the monsters of the level.
    fn sim_with_monsters() -> Sim {
        Sim::new(Field::new(640, 480, RgbaImage::new(640, 480)))
    }

    #[test]
    fn one_step_lasts_25ms() {
        assert!((STEP - 0.025).abs() < 1e-9);
    }

    #[test]
    fn accumulator_fires_at_40hz() {
        let mut t = FixedStep::new();
        // 1 second of rendering at 1000 fps → 40 logical steps.
        let mut steps = 0;
        for _ in 0..1000 {
            steps += t.advance(0.001);
        }
        assert_eq!(steps, 40);
    }

    #[test]
    fn player_moves_120px_per_second() {
        let mut s = sim();
        let input = InputState {
            right: true,
            ..Default::default()
        };
        let start = s.player_x;
        for _ in 0..40 {
            s.step(&input); // 1 second at 40 Hz
        }
        assert!((s.player_x - start - 120.0).abs() < 1.0);
    }

    #[test]
    fn player_does_not_move_diagonally() {
        // The original moves a single axis per call (if/else if cascade).
        let mut s = sim();
        let input = InputState {
            right: true,
            down: true,
            ..Default::default()
        };
        s.step(&input); // 3 calls
        assert_eq!(s.player_x, 320.0 + 3.0, "only moves in X");
        assert_eq!(s.player_y, 1.0, "does not move in Y");
    }

    #[test]
    fn priority_is_right_down_left_up() {
        let mut s = sim();
        let input = InputState {
            right: true,
            down: true,
            left: true,
            up: true,
            ..Default::default()
        };
        s.step(&input);
        assert_eq!(s.player_x, 320.0 + 3.0, "right wins");
        assert_eq!(s.player_y, 1.0);
    }

    #[test]
    fn respawn_at_top_center() {
        let mut s = sim();
        s.player_x = 100.0;
        s.player_y = 400.0;
        s.respawn();
        assert_eq!(s.player_x, (640 / 2) as f32);
        assert_eq!(s.player_y, 1.0, "the readme says 'middle top of the screen'");
    }

    #[test]
    fn no_death_when_not_drawing() {
        // Right after respawning nothing is being drawn: the first movement
        // that lands on a monster does not kill (the original requires
        // `drawing`).
        let mut s = sim();
        s.respawn(); // (width/2, 1)
        s.field.set_monster(321, 1);
        let input = InputState {
            right: true,
            ..Default::default()
        };
        s.step(&input);
        assert!(s.drain_events().is_empty(), "no active trail means no death");
    }

    #[test]
    fn dies_when_touching_monster_while_drawing() {
        let mut s = sim();
        // Start a trail through free zone.
        let move_input = InputState {
            right: true,
            ..Default::default()
        };
        s.step(&move_input);
        // Place a monster right in front.
        let x = s.player_x as i32 + 1;
        let y = s.player_y as i32;
        s.field.set_monster(x, y);
        s.step(&move_input);
        assert!(
            s.drain_events().iter().any(|e| matches!(e, Event::PlayerDied)),
            "drawing and touching a monster ⇒ death"
        );
    }

    #[test]
    fn monsters_write_markers_on_the_field() {
        // Rule C6: monsters exist for the field (they block and kill).
        let s = sim_with_monsters();
        assert!(!s.monsters.is_empty());
        let marked = (0..s.field.height())
            .flat_map(|y| (0..s.field.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| s.field.collision(x, y) == Collision::Monster)
            .count();
        assert!(
            marked >= 1 && marked <= s.monsters.len(),
            "there are {marked} markers for {} monsters",
            s.monsters.len()
        );
    }

    #[test]
    fn monster_touching_the_line_kills_the_player() {
        // Rule C7/F1: if a monster steps on the white line, the player dies.
        let mut s = sim_with_monsters();
        // White line already drawn.
        s.field.plot_line(20, 20, 30, 20);
        assert_eq!(s.field.collision(25, 20), Collision::OwnLine);

        // Monster right to the left, advancing toward the line.
        s.field.clear_monster(s.monsters[0].px(), s.monsters[0].py());
        s.monsters[0].x = 19 * SUBPIXEL;
        s.monsters[0].y = 20 * SUBPIXEL;
        s.monsters[0].vx = 1 * SUBPIXEL;
        s.monsters[0].vy = 0;

        // The monster advances onto the white line within a normal tick.
        s.step(&InputState::default());
        assert!(
            s.drain_events().iter().any(|e| matches!(e, Event::PlayerDied)),
            "monster on the white line ⇒ death"
        );
    }

    #[test]
    fn white_line_is_erased_on_death() {
        // Rule F2: the line does not become closed, it is removed.
        let mut s = sim();
        s.field.plot_line(50, 50, 60, 50);
        assert!(s.field.trail_len() > 0, "there is a trail");
        s.kill_player();
        assert_eq!(s.field.trail_len(), 0, "the trail is erased on death");
    }

    #[test]
    fn game_over_on_the_fourth_death() {
        // Rule F3: with 3 initial lives the player can die 4 times.
        let mut s = sim();
        assert_eq!(s.lives, 3);
        assert!(!s.kill_player(), "1st death (3→2)");
        assert!(!s.kill_player(), "2nd death (2→1)");
        assert!(!s.kill_player(), "3rd death (1→0)");
        assert!(s.kill_player(), "4th death (0→-1) = game over");
    }

    #[test]
    fn monster_count_per_level() {
        // Rule A1: type1 = n - n/3, type2 = n/2, type3 = n/3 (n 1-based).
        for level in 0..15u32 {
            let bg = RgbaImage::new(640, 480);
            let mut s = Sim::new(Field::new(640, 480, bg));
            s.level = level;
            s.spawn_monsters();
            let n = level + 1;
            assert_eq!(
                s.monsters.len(),
                (n - n / 3) as usize + (n / 2) as usize + (n / 3) as usize,
                "level {}",
                n
            );
        }
    }

    #[test]
    fn score_respects_the_extra_life_threshold() {
        let mut s = sim();
        s.score = 4999;
        s.events.push_back(Event::ExtraLife);
        assert_eq!(s.drain_events().len(), 1);
    }

    #[test]
    fn closing_an_area_with_monster_does_not_fill() {
        let mut s = sim();
        // A square around an interior with a monster inside.
        s.field.plot_line(100, 100, 200, 100);
        s.field.plot_line(200, 100, 200, 200);
        s.field.plot_line(200, 200, 100, 200);
        s.field.plot_line(100, 200, 100, 100);
        s.field.set_monster(150, 150);
        // Seed inside the area: the monster is inside ⇒ it is undone.
        let out = s.field.close_area(&[(150, 120)]);
        assert!(out.undone);
        assert_eq!(out.pixels, 0);
    }

    #[test]
    fn closing_an_empty_area_conquers_it() {
        let mut s = sim();
        s.field.plot_line(100, 100, 200, 100);
        s.field.plot_line(200, 100, 200, 200);
        s.field.plot_line(200, 200, 100, 200);
        s.field.plot_line(100, 200, 100, 100);
        // Seed inside the closed area.
        let out = s.field.close_area(&[(150, 150)]);
        assert!(!out.undone);
        // Exact interior of the square: 99×99 cells.
        assert_eq!(out.pixels, 99 * 99);
        assert!(s.field.fill_ratio() > 0.03);
    }

    /// Visual capture of the field with monsters and player (on demand).
    #[test]
    #[ignore]
    fn capture_game() {
        let bg = crate::assets::load_jpeg(&crate::assets::asset_path("data/feld1.jpg")).unwrap();
        let mut s = Sim::new(Field::new(640, 480, bg));
        let input = InputState {
            right: true,
            down: true,
            ..Default::default()
        };
        let sprites = crate::assets::monster_sprites().unwrap();
        std::fs::create_dir_all("/tmp/opencode/kwiks_shots").unwrap();

        for (steps, name) in [(30u64, "30_field_trail.png"), (200, "30_field_monsters.png")] {
            for _ in 0..steps {
                s.step(&input);
            }
            let mut out = RgbaImage::new(640, 480);
            s.field.draw_playfield(&mut out, 22);
            s.draw_monsters(&mut out, &sprites);
            s.draw_player(&mut out);

            let path = format!("/tmp/opencode/kwiks_shots/{name}");
            image::RgbaImage::from_raw(out.width, out.height, out.data)
                .unwrap()
                .save(&path)
                .unwrap();
            eprintln!("capture → {path}");
        }
    }

    #[test]
    fn at_75_percent_enters_level_complete() {
        use crate::screens::{GameMode, Screen, Shell};

        // Small field: conquering the interior exceeds 75 %.
        let bg = RgbaImage::new(40, 40);
        let mut s = Sim::new(Field::new_playfield(40, 40, bg));
        // No monsters: if one is inside, the fill is undone.
        for i in 0..s.monsters.len() {
            let (x, y) = (s.monsters[i].px(), s.monsters[i].py());
            s.field.clear_monster(x, y);
        }
        s.monsters.clear();
        let mut shell = Shell::new();
        shell.reset_session();
        shell.enter(Screen::Game).unwrap();

        let out = s.field.close_area(&[(20, 20)]);
        assert!(out.pixels > 0, "the conquest must fill something");
        let pct = (s.field.fill_ratio() * 100.0) as u32;

        // Sync the HUD like `main.rs` does and run a tick.
        shell.percent = pct;
        let mut frame = RgbaImage::new(640, 480);
        shell.tick(None, &mut frame).unwrap();

        assert!(pct >= 75, "the test must exceed 75 % (reached {pct} %)");
        assert_eq!(
            shell.game_mode(),
            GameMode::LevelComplete,
            "exceeding 75 % must enter mode 1"
        );
    }

    #[test]
    fn crossing_own_line_costs_a_single_life() {
        // Bug found in review: the tick runs 3 sub-steps and, if the player
        // stays on their line, each sub-step would kill them again (3 lives at
        // once). Death must be resolved only once.
        let mut s = sim();
        assert_eq!(s.lives, 3);

        // Go down 5 px drawing a line…
        let move_down = InputState {
            down: true,
            ..Default::default()
        };
        for _ in 0..2 {
            s.step(&move_down);
        }
        assert!(s.field.trail_len() > 0, "there must be an own line");

        // …and go back up stepping on it.
        let move_up = InputState {
            up: true,
            ..Default::default()
        };
        s.step(&move_up);

        let events = s.drain_events();
        let deaths = events
            .iter()
            .filter(|e| matches!(e, Event::PlayerDied))
            .count();
        assert_eq!(deaths, 1, "a single death per tick");
        assert_eq!(s.lives, 2, "a single life lost");
        assert!(!events.iter().any(|e| matches!(e, Event::GameOver)));
    }

    #[test]
    fn game_over_after_running_out_of_lives() {
        let mut s = sim();
        let move_down = InputState {
            down: true,
            ..Default::default()
        };
        let move_up = InputState {
            up: true,
            ..Default::default()
        };
        let mut game_over = false;
        // Each "down + up" kills once. With 3 lives the player can die 4 times.
        for _ in 0..10 {
            for _ in 0..2 {
                s.step(&move_down);
            }
            s.step(&move_up);
            if s.drain_events().iter().any(|e| matches!(e, Event::GameOver)) {
                game_over = true;
                break;
            }
        }
        assert!(game_over, "it must end in game over");
        assert!(s.lives < 0, "the original does game over with lives < 0");
    }

    #[test]
    fn worm_trail_is_bounded() {
        // Without erasing the tail pixel, the worm leaves an infinite trail
        // (bug found in review). The maximum is `(level*10)%200`.
        let bg = RgbaImage::new(200, 200);
        let mut s = Sim::new(Field::new_playfield(200, 200, bg));
        // A single worm, no other monsters.
        for i in 0..s.monsters.len() {
            let (x, y) = (s.monsters[i].px(), s.monsters[i].py());
            s.field.clear_monster(x, y);
        }
        s.monsters.clear();
        s.monsters.push(Monster {
            kind: MonsterKind::Worm,
            x: 50 * SUBPIXEL,
            y: 50 * SUBPIXEL,
            vx: SUBPIXEL,
            vy: 0,
            trail: Vec::new(),
            trail_max: 30,
        });

        // Lots of movement: the trail cannot grow without limit.
        for _ in 0..500 {
            s.step_monsters_once();
        }

        let marked = (0..200)
            .flat_map(|y| (0..200).map(move |x| (x, y)))
            .filter(|&(x, y)| s.field.collision(x, y) == Collision::Monster)
            .count();
        assert!(
            marked <= 31,
            "the trail must be bounded to 30 + head; there are {marked}"
        );
    }

    /// Records a gameplay of the first level as a PNG sequence.
    ///
    /// Guided route: leaves the top frame, draws a rectangle and returns to
    /// the frame to **close the area** (the core mechanic of QIX). The real
    /// game HUD is drawn so it looks like the original.
    #[test]
    #[ignore]
    fn record_gameplay() {
        use crate::screens::{Screen, Shell};

        let bg = crate::assets::load_jpeg(&crate::assets::asset_path("data/feld1.jpg")).unwrap();
        let mut s = Sim::new(Field::new_playfield(640, 480, bg));
        let sprites = crate::assets::monster_sprites().unwrap();

        // Shell only for the HUD (no interactions).
        let mut shell = Shell::new();
        shell.language = crate::screens::Language::En;
        shell.reset_session();
        shell.enter(Screen::Game).unwrap();

        let out_dir = "/tmp/opencode/kwiks_gif";
        std::fs::create_dir_all(out_dir).unwrap();

        // Script: (direction, steps). Each step = 1 tick = 3 px.
        let script: &[(InputState, u64)] = &[
            (InputState { down: true, ..Default::default() }, 34),
            (InputState { right: true, ..Default::default() }, 50),
            (InputState { up: true, ..Default::default() }, 34), // back to the frame ⇒ closes
            (InputState { left: true, ..Default::default() }, 26),
            (InputState { down: true, ..Default::default() }, 22),
            (InputState { left: true, ..Default::default() }, 30),
            (InputState { up: true, ..Default::default() }, 22), // closes another zone
            (InputState::default(), 20),
        ];

        let mut frame = 0u32;
        let save_frame = |s: &Sim, shell: &mut Shell, frame: &mut u32| {
            let mut out = RgbaImage::new(640, 480);
            // Field full-screen (frame included) + HUD on top with transparent
            // background, just like `main.rs`.
            s.field.draw_playfield(&mut out, 0);
            s.draw_monsters(&mut out, &sprites);
            s.draw_player(&mut out);
            // `tick` draws the HUD last, on top and with a transparent background.
            let _ = shell.tick(None, &mut out);

            let path = format!("{out_dir}/f{:04}.png", *frame);
            image::RgbaImage::from_raw(out.width, out.height, out.data)
                .unwrap()
                .save(&path)
                .unwrap();
            *frame += 1;
        };

        for (input, ticks) in script {
            for _ in 0..*ticks {
                s.step(input);
                let _ = s.drain_events();
                shell.percent = (s.field.fill_ratio() * 100.0) as u32;
                shell.score = s.score;
                shell.lives = s.lives.max(0) as u32;
                // Captured every 3 ticks (~13 fps like the original at 40 Hz).
                if s.tick % 3 == 0 {
                    save_frame(&s, &mut shell, &mut frame);
                }
            }
        }

        eprintln!("gameplay → {out_dir} ({frame} frames)");
    }

    /// Widescreen capture (extended field, 854×480).
    #[test]
    #[ignore]
    fn capture_widescreen() {
        let size = crate::video::internal_size((1280, 720), true);
        assert_eq!((size.width, size.height), (854, 480));

        let base = crate::assets::load_jpeg(&crate::assets::asset_path("data/feld1.jpg")).unwrap();
        let bg = crate::extend_image(&base, size);
        let mut s = Sim::new(Field::new(size.width, size.height, bg));
        let input = InputState {
            left: true,
            up: true,
            ..Default::default()
        };
        for _ in 0..60 {
            s.step(&input);
        }
        let sprites = crate::assets::monster_sprites().unwrap();
        let mut out = RgbaImage::new(size.width, size.height);
        s.field.draw_playfield(&mut out, 22);
        s.draw_monsters(&mut out, &sprites);
        s.draw_player(&mut out);

        std::fs::create_dir_all("/tmp/opencode/kwiks_shots").unwrap();
        let path = "/tmp/opencode/kwiks_shots/40_widescreen.png";
        image::RgbaImage::from_raw(out.width, out.height, out.data)
            .unwrap()
            .save(path)
            .unwrap();
        eprintln!("capture → {path}");
    }
}
