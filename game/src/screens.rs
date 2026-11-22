//! State machine and screens of Kwiks (M1).
//!
//! The original has no "menu" object: each screen is a *tick handler*
//! registered in `DAT_00429038` that reads the keyboard and rewrites that
//! pointer to change screens (see `docs/GAME_LOGIC.md` §1). That idea is
//! reproduced here with an enum + dispatch.
//!
//! This module is **pure**: it does not touch SDL or the graphics system
//! clock; it only draws into the 640×480 [`RgbaImage`] that `main.rs` passes
//! in and returns the transition. The internal resolution is 640×480 like the
//! original 640×480×16.
//!
//! Lifecycle (§1.2): **title → menu 1..3 / credits / attract → game →
//! (pause | level complete | game over) → title**. There are two routes that
//! in the original **close the program** (`DAT_00429038 = 0` →
//! `PostQuitMessage`): Esc in menu 2 (`0x4057ce`) and the end of the attract
//! carousel (`0x4010cb`–`0x4010d3`); they are expressed with
//! [`TickResult::Quit`]. The credits **return to menu 2** (not to attract):
//! `0x405a11`–`0x405a64` draws MENU2 and sets the handler `0x4057bc`
//! (`GAME_LOGIC.md` §1.2 is outdated on that point; the binary is
//! authoritative, audit #23).
//!
//! Evidence convention same as in `docs/GAME_LOGIC.md`:
//! **[O]** observed in the binary · **[I]** inferred · **[?]** unresolved.
//!
//! The `.PIX` files (GAMEOVER, COMPLET, MAKERS/EMAKERS, …) are loaded with
//! `assets::decode_pix` (already implemented); if it fails, there is a
//! *fallback* to the converted PNGs in `assets/png/` (same pixels,
//! `tools/pix2png.py`), useful while `decode_pix` was a stub. See
//! [`Art::pix`].

use anyhow::{Context, Result};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::assets::{self, RgbaImage};
use crate::config::Config;
use crate::extras::{Achievements, HighScores, ALL_ACHIEVEMENTS};

// ---------------------------------------------------------------------------
// Module contract (used by `main.rs`).
//
// Contract change **[O]** (the only one allowed by the screen-fidelity task):
// `TickResult` gains the variant [`TickResult::Quit`] to be able to express
// the end of the program (Esc in MENU2 and the end of the attract). `main.rs`
// only matches `TickResult::Go` with `if let`, so it still compiles unchanged;
// the hook point is marked with `TODO(main)`.
// ---------------------------------------------------------------------------

/// Game screens, per the original handler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    /// `LAB_004054ac` — TITEL.JPG
    Title,
    /// `LAB_00405590` — MENU1 / EMENU1
    Menu1,
    /// `LAB_004057bc` — MENU2 / EMENU2
    Menu2,
    /// `LAB_00405918` — MENU3 / EMENU3
    Menu3,
    /// `LAB_004059b4` — DNLOGO + wipe of MAKERS/EMAKERS
    Credits,
    /// `LAB_00401060` — attract carousel (kiosk mode): `FELD2.JPG` (entry) →
    /// `FELD4` → `FELD6` → `FELD8` → `FELD10` → `WERBUNG.JPG`; when the slides
    /// run out, **end of the program** (it does not return to the title).
    Attract,
    /// `LAB_00404734` — game (40 Hz tick)
    Game,
    /// `LAB_00404b10` — PAUSED.JPG
    Pause,
    /// `LAB_00402334` — GAMEOVER.PIX
    GameOver,
    /// Options screen (port-only): video, audio and language settings.
    Options,
    /// High scores and achievements (port-only).
    Scores,
}

/// Normalized keyboard input (the original uses `GetAsyncKeyState`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
    Pause,
    Music,
    Sfx,
    FastGraphics,
    Help,
    Options,
    Scores,
    Quit,
}

/// Result of a screen tick: draw + possible transition.
pub enum TickResult {
    /// Stays on the same screen.
    Stay,
    /// Changes screen.
    Go(Screen),
    /// End of the program: equivalent to `DAT_00429038 = 0` in the original
    /// pump (`0x4026ed`) → `PostQuitMessage(0)` (import `cs:0x4621b4`). Fired
    /// by Esc in menu 2 (`0x4057ce`) and the end of the attract carousel
    /// (`0x4010cb`–`0x4010d3`).
    ///
    /// TODO(main): `main.rs` **does not consume this variant yet** — it only
    /// checks `TickResult::Go` with `if let`, so today the game does not
    /// actually close through these routes ([`Shell::quit_requested`] is also
    /// set). Hook point: after `shell.tick(...)` in `main.rs`, do
    /// `if let TickResult::Quit = result { break 'main; }`.
    Quit,
}

// ---------------------------------------------------------------------------
// Additional types (additive; they do not alter the contract).
// ---------------------------------------------------------------------------

/// Language of switchable text and art.
///
/// The original toggles German/English with the flag `DAT_00429034` (§3.2),
/// which selects `EMENU*.JPG` vs `MENU*.JPG` and the text variant. Here ES/EN
/// is implemented: **[I]** `En` uses the English art (`EMENU*`, `EMAKERS`) and
/// `Es` uses the primary variant (`MENU*`, `MAKERS`, German art as a
/// substitute while no Spanish art exists).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    Es,
    En,
}

impl Default for Language {
    /// `KWIKS_LANG=es|en` (default `en`). **[I]**
    fn default() -> Self {
        match std::env::var("KWIKS_LANG")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "es" | "spanish" | "espanol" | "español" => Language::Es,
            _ => Language::En,
        }
    }
}

/// Entry selected in the menu carousel (Menu1/2/3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectedEntry {
    /// MENU1 — Enter starts the game (§1.2).
    Start,
    /// MENU2 — Enter opens the credits.
    Credits,
    /// MENU3 — Enter/Esc go to attract.
    Attract,
}

/// Sub-mode of the [`Screen::Game`] screen.
///
/// In the original "level complete" is **not** a separate handler: it is mode
/// 1 of the game tick (`DAT_004604fc`, §2) with the `COMPLET.PIX` overlay. It
/// is reproduced the same way: it is still `Screen::Game`, with this
/// sub-state. **[O]**
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameMode {
    /// Mode 0: game in progress.
    Play,
    /// Mode 1: level complete (FELD + COMPLET, waiting for a key).
    LevelComplete,
}

/// Internal action derived from a key within the tick (before materializing
/// the [`TickResult`]). `Quit` models `DAT_00429038 = 0`.
enum Action {
    /// No change.
    None,
    /// Screen transition.
    Go(Screen),
    /// End of the program (see [`TickResult::Quit`]).
    Quit,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Conquest threshold to complete a level: `pct >= 0x4b` (75 %) in §2. **[O]**
const LEVEL_COMPLETE_PCT: u32 = 75;

/// Number of rows on the options screen.
const OPTION_ROWS: usize = 8;

/// Top HUD strip: 640×22 object (`DAT_00460848`, §3.2). **[O]**
const HUD_HEIGHT: u32 = 22;

/// Duration of the DNLOGO → MAKERS wipe in the credits. **[I]**
/// (`FUN_0040565c` performs the wipe.) NOTE(audit #24): the original sweeps
/// **columns** of 2 px (even left→right, odd right→left) at 2 ms/step, ~1.3 s
/// in total; the row-based wipe used here is still a [DIFF] and is out of the
/// scope of this correction pass.
const CREDITS_WIPE: Duration = Duration::from_secs(2);

/// HUD formats (§6): `%3.3d %3.3d %6.6d %3.3d` =
/// (percentage, level, score, lives). **[O]** (the value↔format association is [I])
const FMT_PCT: usize = 3;
const FMT_LEVEL: usize = 3;
const FMT_SCORE: usize = 6;
const FMT_LIVES: usize = 3;

// ---------------------------------------------------------------------------
// Shell
// ---------------------------------------------------------------------------

pub struct Shell {
    pub screen: Screen,
    /// Already-decoded background image of the current screen.
    pub background: Option<RgbaImage>,

    /// ES/EN language (switchable with [`Shell::toggle_language`]).
    pub language: Language,
    /// Music ON/OFF (F8, §1.3 `DAT_004604d8`).
    pub music_on: bool,
    /// Effects ON/OFF (F9, §1.3 `DAT_004604f8`).
    pub sfx_on: bool,
    /// Fast graphics ON/OFF (F2, §1.3 `DAT_00429064`).
    ///
    /// [O] `0x402590`: `not DAT_00429064`; the observed effect is reducing the
    /// monsters' halo to `radius/4` (min. 1) in `FUN_00403170`. That effect
    /// already lives in `sim.rs` (`Sim::fast_graphics`, used by
    /// `Sim::draw_monsters`), **but the propagation is not wired up**:
    /// `main.rs` only uses this flag to unlock an achievement and never copies
    /// `shell.fast_graphics` to the `Sim`. TODO(main): `sim.fast_graphics =
    /// shell.fast_graphics` when creating the `Sim` and on every frame (audit #55).
    pub fast_graphics: bool,
    /// Quit request (§1.2: Esc in MENU2 sets the callback to 0 →
    /// `PostQuitMessage`; same when the attract runs out). It is set together
    /// with [`TickResult::Quit`]; `main.rs` does not consume it yet (see
    /// [`TickResult::Quit`]).
    pub quit_requested: bool,

    /// Persisted settings, edited by the options screen. `main.rs` owns the
    /// file and applies the video changes when they differ from what is live.
    pub options: Config,
    /// Selected row on the options screen.
    options_index: usize,
    /// Screen to return to when leaving the options.
    options_return: Screen,
    /// Persisted high-score table (shown in [`Screen::Scores`]).
    pub scores: HighScores,
    /// Persisted achievements (shown in [`Screen::Scores`]).
    pub achievements: Achievements,
    /// Screen to return to when leaving the scores screen.
    scores_return: Screen,

    // Game state (§3.2). For now this shell keeps it as a placeholder; once
    // `sim.rs` exists it will feed it.
    /// Level index, 0-based (`DAT_00460810`).
    pub level: u32,
    /// Score (`DAT_00460835`).
    pub score: u32,
    /// Lives (`DAT_00460839`, initially 3).
    pub lives: u32,
    /// Percentage of field conquered (`DAT_00429028` → FUN_004021b4).
    pub percent: u32,

    // --- internal state ---
    game_mode: GameMode,
    help_visible: bool,
    help_is_about: bool,
    /// Index of the attract slide being shown (equivalent to `DAT_00429014`:
    /// 2=FELD2 … 10=FELD10, 12=WERBUNG; ≥ 12 exhausted).
    attract_index: u32,
    credits_started: Instant,
    /// Pending menu click to be played (see [`Shell::take_click`]).
    click_pending: bool,
    /// Game-over-by-death flag (`DAT_00460508`, see [`Shell::tick`]).
    game_over_flag: bool,
    /// Flag `DAT_00460820` of mode 1: when set, the first keypress in the
    /// interstitial only discards (see [`Shell::reset_session`] and the
    /// `GameMode::LevelComplete` arm of [`Shell::handle_screen_key`]).
    level_complete_skip: bool,
    art: Art,
}

impl Shell {
    pub fn new() -> Self {
        Self {
            screen: Screen::Title,
            background: None,
            language: Language::default(),
            music_on: true,
            sfx_on: true,
            fast_graphics: false,
            quit_requested: false,
            level: 0,
            score: 0,
            lives: 3,
            percent: 0,
            game_mode: GameMode::Play,
            help_visible: false,
            help_is_about: false,
            attract_index: 2,
            credits_started: Instant::now(),
            click_pending: false,
            game_over_flag: false,
            level_complete_skip: false,
            options: Config::default(),
            options_index: 0,
            options_return: Screen::Menu1,
            scores: HighScores::from_str(""),
            achievements: Achievements::from_str(""),
            scores_return: Screen::Menu1,
            art: Art::default(),
        }
    }

    /// One step of the current screen. Returns the transition, if any.
    ///
    /// The tick always draws into `out` (640×480) and, if there is a
    /// transition, performs the entry actions and already draws the new screen
    /// (the framebuffer is blitted to the window every frame in `main.rs`).
    pub fn tick(&mut self, input: Option<Input>, out: &mut RgbaImage) -> Result<TickResult> {
        // 0. If the shell was just created there is no background loaded yet
        // (the contract starts with `background: None`); lazy init.
        if self.background.is_none() {
            let screen = self.screen;
            self.enter(screen)?;
        }

        // 1. Input: global keys (§1.3) or keys of the current screen.
        let mut action = Action::None;
        if let Some(input) = input {
            if self.help_visible
                && !matches!(
                    input,
                    Input::Help | Input::Music | Input::Sfx | Input::FastGraphics | Input::Quit
                )
            {
                // The help is a modal MessageBox: any other key closes it
                // **without being processed** — not even movement (↑/↓ do not
                // navigate, P/Esc do not transition). See [`Shell::help_open`]
                // for freezing the game (audit #35/#53).
                self.help_visible = false;
            } else {
                action = self.handle_input(input)?;
            }
        }

        // 1b. Game-over-by-death flag (§2, common block `0x4047ee`):
        // `DAT_00460508 != 0` → EXIT → `FUN_004022d0` (GAMEOVER.PIX). [O] the
        // flag is set by the death handler when the lives run out (writes
        // `0x405112`/`0x40525c`; `lives--` and `lives < 0` at `0x4051f1`). In
        // the original the flag is checked **before** Esc/P, so it has
        // priority over the tick's key; same here.
        //
        // `main.rs` already calls [`Shell::request_game_over`] when
        // `Sim::kill_player` returns `true` (the 4th death, `lives < 0`).
        // `lives == 0` used to be used as a stand-in as well, but it fired one
        // death early because the HUD receives the lives clamped to 0 (audit
        // #50); that is why the condition now lives only in the flag.
        if self.screen == Screen::Game && self.game_over_flag {
            action = Action::Go(Screen::GameOver);
        }

        // 2. Transition + entry actions.
        let result = match action {
            Action::Go(next) => {
                self.enter(next)?;
                TickResult::Go(next)
            }
            Action::Quit => {
                self.quit_requested = true;
                TickResult::Quit
            }
            Action::None => TickResult::Stay,
        };

        // 3. Level threshold (§2 LEVEL_COMPLETE, `0x4047c0`–`0x4047e8`,
        // after the mode 0 block).
        if self.screen == Screen::Game
            && self.game_mode == GameMode::Play
            && self.percent >= LEVEL_COMPLETE_PCT
        {
            // [O] 0x4047e0/0x4047e8: mode `DAT_004604fc` = 1 and flag
            // `DAT_00460820` = 0 (that is why in the normal flow the first
            // keypress of the interstitial **advances** directly).
            self.game_mode = GameMode::LevelComplete;
            self.level_complete_skip = false;
        }

        // 4. Drawing.
        self.draw(out)?;
        Ok(result)
    }

    /// Current game mode (for tests and integration).
    #[allow(dead_code)]
    pub fn game_mode(&self) -> GameMode {
        self.game_mode
    }

    /// Toggles the ES/EN language (equivalent to the flag `DAT_00429034`). **[I]**
    pub fn toggle_language(&mut self) {
        self.language = match self.language {
            Language::Es => Language::En,
            Language::En => Language::Es,
        };
        // The menu/credits art depends on the language: reload the background.
        let screen = self.screen;
        if let Err(e) = self.enter(screen) {
            eprintln!("screens: reload after language change: {e:#}");
        }
    }

    /// Entry selected in the menus.
    pub fn selected_entry(&self) -> SelectedEntry {
        match self.screen {
            Screen::Menu1 => SelectedEntry::Start,
            Screen::Menu2 => SelectedEntry::Credits,
            _ => SelectedEntry::Attract,
        }
    }

    /// Is there a pending menu click to play? (one-shot: it is consumed.)
    ///
    /// [O] the click (`FUN_004043c4`) sounds **only on the transitions of
    /// menus 1-3 and credits** — 10 places: `0x4055a1`, `0x4055e0`, `0x40561f`
    /// (menu 1), `0x4057ce`, `0x405837`, `0x40588d`, `0x4058d9` (menu 2),
    /// `0x405945`, `0x405976` (menu 3), `0x405a11` (credits) — and **never on
    /// the title** (`0x4054d9` does not call it) **nor in game over** (audit
    /// #9 and #71).
    ///
    /// TODO(main): `main.rs` currently plays `click.wav` with the inverted
    /// rule (on `Confirm` once the screen has already changed to title/menu):
    /// it sounds on the title and **not** on ↓/↑/Esc nor on menu→game/credits.
    /// When possible, replace that rule with
    /// `if shell.take_click() { audio.play_sfx("click.wav"); }` right after
    /// `shell.tick(...)` and consume this flag once per frame.
    #[allow(dead_code)] // TODO(main): consume from main.rs
    pub fn take_click(&mut self) -> bool {
        std::mem::take(&mut self.click_pending)
    }

    /// Is the help (F1) visible?
    ///
    /// [O] the real help is a modal `MessageBox` (`0x40252d`–`0x402586`,
    /// `0x40552b` on the title and `0x4048b6` in game) that **freezes the
    /// game**: the tick does not run until the dialog is closed. This module
    /// absorbs all keys while the help is open (↑/↓ do not navigate, P/Esc do
    /// not transition), but the player's movement is processed by `main.rs`
    /// via `held`/`Sim::step` and **cannot** be frozen from here.
    /// TODO(main): while `shell.help_open()`, do not run the simulation nor
    /// process game keys (audit #35).
    #[allow(dead_code)] // TODO(main): consume from main.rs
    pub fn help_open(&self) -> bool {
        self.help_visible
    }

    /// Sets the game-over flag (`DAT_00460508 = 1`) and forces the exit to
    /// [`Screen::GameOver`] on the next tick.
    ///
    /// [O] the flag is set by the death handler when the lives run out
    /// (`lives--` and game over with `lives < 0`, `0x4051f1`; writes at
    /// `0x405112` and `0x40525c`) and the tick exits to `FUN_004022d0`
    /// (GAMEOVER.PIX) in the common block (`0x4047ee`).
    ///
    /// TODO(main): call this method from `main.rs` when `Sim::kill_player`
    /// returns `true` (the 4th death, `lives < 0`, which is the original's
    /// `lives < 0` — row #49 of the audit says the port uses `lives == 0`, but
    /// the current `sim.rs` already uses `self.lives < 0`). In the meantime
    /// the tick falls back to the `lives == 0` stand-in (see [`Shell::tick`],
    /// step 1b), which fires one death early because `main.rs` clamps
    /// `Sim::lives` with `.max(0)`.
    #[allow(dead_code)] // TODO(main): consume from main.rs
    pub fn request_game_over(&mut self) {
        self.game_over_flag = true;
    }

    // -- input -------------------------------------------------------------

    /// Global keys (F1/F2/F8/F9, §1.3) + per-screen dispatch (§1.2).
    fn handle_input(&mut self, input: Input) -> Result<Action> {
        match input {
            Input::Music => {
                self.music_on = !self.music_on;
                self.options.music_on = self.music_on;
            }
            Input::Sfx => {
                self.sfx_on = !self.sfx_on;
                self.options.sfx_on = self.sfx_on;
            }
            Input::FastGraphics => {
                self.fast_graphics = !self.fast_graphics;
                self.options.fast_graphics = self.fast_graphics;
            }
            Input::Help => {
                // On the title F1 shows the "about" (Knuddel's Quest, §1.2);
                // everywhere else, the global help ("Kwiks - Hilfe").
                self.help_is_about = self.screen == Screen::Title;
                self.help_visible = !self.help_visible;
            }
            // Port-only: open the options from the title or any menu.
            Input::Options
                if matches!(
                    self.screen,
                    Screen::Title | Screen::Menu1 | Screen::Menu2 | Screen::Menu3
                ) =>
            {
                self.options_return = self.screen;
                self.options_index = 0;
                return Ok(Action::Go(Screen::Options));
            }
            // Port-only: high scores and achievements.
            Input::Scores
                if matches!(
                    self.screen,
                    Screen::Title | Screen::Menu1 | Screen::Menu2 | Screen::Menu3
                ) =>
            {
                self.scores_return = self.screen;
                return Ok(Action::Go(Screen::Scores));
            }
            // Same as Esc in MENU2: `DAT_00429038 = 0` → end of the program.
            Input::Quit => return Ok(Action::Quit),
            other => return self.handle_screen_key(other),
        }
        Ok(Action::None)
    }

    /// Change the selected option. `forward` is false for Left, true for
    /// Right/Enter. Toggles go both ways; the frame limit cycles.
    fn change_option(&mut self, forward: bool) {
        match self.options_index {
            0 => self.options.widescreen = !self.options.widescreen,
            1 => self.options.integer_scale = !self.options.integer_scale,
            2 => {
                const LIMITS: [u32; 4] = [30, 60, 120, 0];
                let current = LIMITS
                    .iter()
                    .position(|&v| v == self.options.fps_cap)
                    .unwrap_or(3);
                let next = if forward {
                    (current + 1) % LIMITS.len()
                } else {
                    (current + LIMITS.len() - 1) % LIMITS.len()
                };
                self.options.fps_cap = LIMITS[next];
            }
            3 => self.options.fullscreen = !self.options.fullscreen,
            4 => {
                self.options.language = if self.options.language == "es" {
                    "en".to_string()
                } else {
                    "es".to_string()
                };
            }
            5 => self.options.music_on = !self.options.music_on,
            6 => self.options.sfx_on = !self.options.sfx_on,
            7 => self.options.fast_graphics = !self.options.fast_graphics,
            _ => {}
        }
        // Mirror the settings that live in the shell.
        self.music_on = self.options.music_on;
        self.sfx_on = self.options.sfx_on;
        self.fast_graphics = self.options.fast_graphics;
        self.language = match self.options.language.as_str() {
            "es" => Language::Es,
            _ => Language::En,
        };
    }

    /// Text of each options row, given the current settings.
    fn option_rows(&self) -> [(&'static str, String); OPTION_ROWS] {
        let onoff = |b: bool| if b { "ON" } else { "OFF" };
        let fps = match self.options.fps_cap {
            0 => "OFF".to_string(),
            n => format!("{n} FPS"),
        };
        let lang = if self.options.language == "es" { "ES" } else { "EN" };
        [
            ("Widescreen", onoff(self.options.widescreen).to_string()),
            ("Integer scale", onoff(self.options.integer_scale).to_string()),
            ("Frame limit", fps),
            ("Fullscreen", onoff(self.options.fullscreen).to_string()),
            ("Language", lang.to_string()),
            ("Music", onoff(self.options.music_on).to_string()),
            ("Sound effects", onoff(self.options.sfx_on).to_string()),
            ("Fast graphics", onoff(self.options.fast_graphics).to_string()),
        ]
    }

    fn handle_screen_key(&mut self, input: Input) -> Result<Action> {
        use Input::*;
        let next = match self.screen {
            // Enter/Space/Esc → menu 1 (§1.2). No click: [O] the title
            // transition (`0x4054d9`) does not call `FUN_004043c4` (audit #9).
            Screen::Title => match input {
                Confirm | Cancel => Action::Go(Screen::Menu1),
                _ => Action::None,
            },
            // Menu 1 (`LAB_00405590`): every transition plays the click.
            Screen::Menu1 => match input {
                Confirm => {
                    // [O] 0x4055e0: click + `FUN_004045d4` (starts the game).
                    self.click();
                    self.reset_session();
                    Action::Go(Screen::Game)
                }
                Down => {
                    // [O] 0x405611/0x40561f: click + MENU2 → `0x4057bc`.
                    self.click();
                    Action::Go(Screen::Menu2)
                }
                Cancel => {
                    // [O] 0x405593/0x4055a1: click + `FUN_00401010` (attract).
                    self.click();
                    Action::Go(Screen::Attract)
                }
                _ => Action::None,
            },
            // Menu 2 (`LAB_004057bc`): Enter → credits · ↓ → menu 3 ·
            // ↑ → menu 1 · Esc → click + `FUN_00405a70`
            // (`DAT_00429038 = 0` → `PostQuitMessage`): **end of the program**
            // [O] `0x4057ce`.
            Screen::Menu2 => match input {
                Confirm => {
                    // [O] 0x40580d–0x405837: key-release wait + click +
                    // DNLOGO + wipe + credits (`LAB_004059b4`).
                    self.click();
                    Action::Go(Screen::Credits)
                }
                Down => {
                    // [O] 0x4058cb/0x4058d9: click + MENU3 → `0x405918`.
                    self.click();
                    Action::Go(Screen::Menu3)
                }
                Up => {
                    // [O] 0x40588d: click + MENU1 → `0x405590`.
                    self.click();
                    Action::Go(Screen::Menu1)
                }
                Cancel => {
                    // [O] 0x4057ce: click + `FUN_00405a70` → handler = 0 →
                    // end of the program (same exit as the end of attract).
                    self.click();
                    Action::Quit
                }
                _ => Action::None,
            },
            // Menu 3 (`LAB_00405918`): Esc/Enter/Space → attract · ↑ → menu 2.
            Screen::Menu3 => match input {
                Confirm | Cancel => {
                    // [O] 0x405945: click + `FUN_00401010` (attract).
                    self.click();
                    Action::Go(Screen::Attract)
                }
                Up => {
                    // [O] 0x405968/0x405976: click + MENU2 → `0x4057bc`.
                    self.click();
                    Action::Go(Screen::Menu2)
                }
                _ => Action::None,
            },
            // Credits (`LAB_004059b4`): Esc/Enter/Space → key-release wait for
            // the 3 + click + draws MENU2 (obj `0x460860`) + handler
            // `0x4057bc`: **returns to menu 2** and releases DNLOGO/MAKERS
            // [O] `0x405a11`–`0x405a64`. (Fixes the old transition to attract;
            // `GAME_LOGIC.md` §1.2 does not match the binary — audit #23.)
            Screen::Credits => match input {
                Confirm | Cancel => {
                    self.click();
                    Action::Go(Screen::Menu2)
                }
                _ => Action::None,
            },
            // Attract (`LAB_00401060`): Esc/Enter/Space do **not** exit; they
            // clear the screen and advance the slide (see
            // [`Shell::attract_advance`]). ↑/↓ are ignored [O] (`0x401060`
            // only polls Esc/Enter/Space).
            Screen::Attract => match input {
                Confirm | Cancel => self.attract_advance()?,
                _ => Action::None,
            },
            // Port-only options screen: navigate with Up/Down, change with
            // Left/Right (or Enter), leave with Esc.
            Screen::Options => match input {
                Up => {
                    if self.options_index > 0 {
                        self.options_index -= 1;
                    }
                    Action::None
                }
                Down => {
                    if self.options_index + 1 < OPTION_ROWS {
                        self.options_index += 1;
                    }
                    Action::None
                }
                Left => {
                    self.change_option(false);
                    Action::None
                }
                Right | Confirm => {
                    self.change_option(true);
                    Action::None
                }
                Cancel => {
                    self.click();
                    Action::Go(self.options_return)
                }
                _ => Action::None,
            },
            // Port-only high scores and achievements screen.
            Screen::Scores => match input {
                Cancel | Confirm | Scores => Action::Go(self.scores_return),
                _ => Action::None,
            },
            Screen::Game => match self.game_mode {
                // Mode 0: P → pause · Esc → EXIT (GAMEOVER.PIX, §2).
                GameMode::Play => match input {
                    Pause => Action::Go(Screen::Pause),
                    Cancel => Action::Go(Screen::GameOver),
                    _ => Action::None,
                },
                // Mode 1 (§2, `0x4048e2`–`0x404a2a`): Enter/Esc/Space advance
                // the level (in mode 1 Esc does not leave the game, unlike in
                // mode 0); P and the rest are ignored. **[O]**
                GameMode::LevelComplete => match input {
                    Confirm | Cancel => {
                        // [O] `0x404a0e`–`0x404a20`: if the flag
                        // `DAT_00460820` is set, the 1st keypress only clears
                        // it and returns (**discard**); the 2nd goes to NEXT
                        // LEVEL.
                        //
                        // NOTE(audit): the task summarizes this as "the
                        // original discards the first keypress", but when
                        // verifying in the binary, the normal flow enters mode
                        // 1 with the flag **at 0** (`0x4047e8`), so the first
                        // keypress advances directly — exactly what the
                        // parenthesis in row #40 warns about. The discard only
                        // fires in the residual state (mode=1, flag=1) that
                        // row #43 marks as **[?]** (Esc/75 % race or globals
                        // overlap). Here the flag semantics are reproduced
                        // as-is: it is set to 1 by `reset_session`
                        // (`FUN_004045d4`, `0x4046a2`) and NEXT LEVEL
                        // (`0x404a9f`), and cleared by LEVEL_COMPLETE (step
                        // 3 of [`Shell::tick`]); the discard therefore remains
                        // reachable the same way as in the original (see
                        // [`Shell::reset_session`]).
                        if self.level_complete_skip {
                            self.level_complete_skip = false;
                            Action::None
                        } else {
                            self.next_level()?;
                            Action::None
                        }
                    }
                    _ => Action::None,
                },
            },
            // Only P resumes (§1.2, `LAB_00404b10`).
            Screen::Pause => match input {
                Pause => Action::Go(Screen::Game),
                _ => Action::None,
            },
            // Esc/Enter/Space → title (§1.2, `LAB_00402334`). No click: the
            // original does not play it on the title nor in game over (#71).
            Screen::GameOver => match input {
                Confirm | Cancel => Action::Go(Screen::Title),
                _ => Action::None,
            },
        };
        Ok(next)
    }

    /// Marks the menu click (`FUN_004043c4`). See [`Shell::take_click`].
    fn click(&mut self) {
        self.click_pending = true;
    }

    /// Advances the attract carousel (`LAB_00401060`).
    ///
    /// [O] Esc/Enter/Space clear the screen and advance the slide (index
    /// `DAT_00429014 += 2`, fmt `FELD%d.JPG` `0x42501a`); with the index at 10
    /// the next slide is `WERBUNG.JPG` (`0x42500e`, index = 12) and, when it
    /// runs out (index ≥ 12), `0x4010cb`–`0x4010d3` releases the object and
    /// sets `DAT_00429038 = 0` → `PostQuitMessage`: **end of the program**
    /// (kiosk mode, same as Esc in menu 2). There is no automatic advance: the
    /// original handler only polls keys.
    ///
    /// The original's key-release wait (`0x401131`) cannot be reproduced
    /// here: `main.rs` passes neither the `KeyUp` nor the SDL `repeat` flag to
    /// this module (audit #60).
    fn attract_advance(&mut self) -> Result<Action> {
        if self.attract_index >= 12 {
            // Slides exhausted: handler = 0 → end of the program.
            return Ok(Action::Quit);
        }
        if self.attract_index >= 10 {
            // [O] 0x4010e3–0x4010f4: last slide = WERBUNG.JPG (advertising).
            self.attract_index = 12;
            self.background = Some(self.art.jpg("werbung")?.clone());
        } else {
            self.attract_index += 2;
            let index = self.attract_index;
            self.background = Some(self.art.feld(index)?.clone());
        }
        Ok(Action::None)
    }

    /// Game start (`FUN_004045d4` → `FUN_00404bc0`: lives=3, score=0; level
    /// `DAT_00460810` = 0). **[O]**
    ///
    /// NOTE(fidelity): `FUN_004045d4` does **not** write the mode
    /// `DAT_004604fc` (in the whole binary only `0x4047e0` and `0x404a31`
    /// write it), so a residual mode 1 carried over from the previous game
    /// survives the reset: that is the path through which the original reaches
    /// the state (mode=1, flag=1) of row #43 **[?]**, with the logo animation
    /// bouncing and the discard of the 1st keypress. That is replicated here:
    /// `game_mode` is **not** touched. The flag `DAT_00460820 = 1` is set
    /// (`0x4046a2`) and the game-over flag is cleared
    /// (`DAT_00460508 = 0`, `0x404709`).
    pub fn reset_session(&mut self) {
        self.level = 0;
        self.score = 0;
        self.lives = 3;
        self.percent = 0;
        self.game_over_flag = false; // [O] 0x404709: `DAT_00460508 = 0`
        self.level_complete_skip = true; // [O] 0x4046a2: `DAT_00460820 = 1`
    }

    /// Next level (§2 NEXT_LEVEL, `0x404a2a`): 0-based index ++
    /// (`DAT_00460810`), mode to 0 (`0x404a31`), field
    /// `FELD((level%14)+1).JPG` (§2 `FUN_00401374`), percentage to 0 and flag
    /// `DAT_00460820 = 1` (`0x404a9f`). **[O]**
    ///
    /// The "los" sound of NEXT LEVEL is missing (`FUN_00404460`, `0x404a7d`,
    /// audit #73): the effects are emitted by `main.rs` and today it only
    /// sounds when entering `Screen::Game`. TODO(main).
    fn next_level(&mut self) -> Result<()> {
        self.level = self.level.wrapping_add(1);
        self.percent = 0;
        self.game_mode = GameMode::Play;
        self.level_complete_skip = true;
        let screen = self.screen;
        self.enter(screen)
    }

    // -- screen entry ------------------------------------------------------

    /// Actions when entering a screen: background, timers and sub-modes.
    pub fn enter(&mut self, next: Screen) -> Result<()> {
        self.screen = next;
        self.help_visible = false;
        let bg = match next {
            Screen::Title => self.art.jpg("titel")?.clone(),
            Screen::Menu1 | Screen::Menu2 | Screen::Menu3 => {
                let idx = match next {
                    Screen::Menu1 => 0,
                    Screen::Menu2 => 1,
                    _ => 2,
                };
                self.art.menu(idx, self.language)?.clone()
            }
            Screen::Credits => {
                // Loads DNLOGO (§1.2) and starts the MAKERS wipe.
                self.credits_started = Instant::now();
                self.art.jpg("dnlogo")?.clone()
            }
            Screen::Attract => {
                // FUN_00401010 enters showing FELD2.JPG (str `0x425004`) with
                // the index `DAT_00429014 = 2`. **[O]**
                self.attract_index = 2;
                self.art.feld(2)?.clone()
            }
            // FELD((level%14)+1).JPG (§2 FUN_00401374). **[O]**
            Screen::Game => self.art.feld(self.level % 14 + 1)?.clone(),
            Screen::Pause => self.art.jpg("paused")?.clone(),
            Screen::GameOver => self.art.pix("gameover")?.clone(),
            // Options: plain black backdrop, the panel draws on top.
            Screen::Options => RgbaImage::new(640, 480),
            // High scores: plain black backdrop.
            Screen::Scores => RgbaImage::new(640, 480),
        };
        self.background = Some(bg);
        Ok(())
    }

    // -- drawing -----------------------------------------------------------

    fn draw(&mut self, out: &mut RgbaImage) -> Result<()> {
        match self.screen {
            Screen::Title | Screen::Menu1 | Screen::Menu2 | Screen::Menu3 | Screen::Pause => {
                if let Some(bg) = self.background.as_ref() {
                    blit(out, bg);
                }
            }
            Screen::Game => {
                // In game the field is drawn by `main.rs`; here only the HUD
                // (over the field, with a transparent background). The level
                // background is only blitted in the level-complete
                // interstitial.
                match self.game_mode {
                    GameMode::Play => self.draw_hud(out),
                    // Level complete (§2 FUN_004017cc): full FELD +
                    // COMPLET.PIX overlay (the overlay's black is the 8bpp
                    // color-key transparency). **[I]**
                    GameMode::LevelComplete => {
                        if let Some(bg) = self.background.as_ref() {
                            blit(out, bg);
                        }
                        let complet = self.art.pix("complet")?;
                        blit_keyed(out, complet, [0, 0, 0]);
                    }
                }
            }
            Screen::GameOver => {
                // GAMEOVER.PIX is blitted full-screen (§2 FUN_004022d0).
                let gameover = self.art.pix("gameover")?;
                blit(out, gameover);
            }
            Screen::Credits => {
                if let Some(bg) = self.background.as_ref() {
                    blit(out, bg);
                }
                // Wipe: MAKERS/EMAKERS is revealed top-down over DNLOGO.
                let makers = self.art.makers(self.language)?;
                let progress = self.credits_started.elapsed().as_secs_f32()
                    / CREDITS_WIPE.as_secs_f32();
                let rows = ((makers.height as f32) * progress.clamp(0.0, 1.0)) as u32;
                blit_rows(out, makers, rows);
            }
            Screen::Attract => {
                if let Some(bg) = self.background.as_ref() {
                    blit(out, bg);
                }
            }
            Screen::Options => self.draw_options(out),
            Screen::Scores => self.draw_scores(out),
        }

        if self.help_visible {
            let about = self.help_is_about;
            let music_on = self.music_on;
            let sfx_on = self.sfx_on;
            let fast = self.fast_graphics;
            let language = self.language;
            draw_help(out, about, language, music_on, sfx_on, fast);
        }
        Ok(())
    }

    /// HUD of the game screen (§6 / `FUN_00404ce8`): 22 px strip with
    /// (percentage, level, score, lives). The label positions follow the
    /// anchors of `INFO.PIX` (x ≈ 50 / 157 / 191 / 302 / 465).
    ///
    /// The original HUD uses the font from `FNTS.PIX`/`INFO.PIX`; here it is
    /// drawn with an embedded 8×8 font (basic readable; to be polished later).
    /// The level is shown 1-based **[I]** (the internal one is 0-based like
    /// `DAT_00460810`; the original's help texts talk about levels starting
    /// at 1).
    pub fn draw_hud(&self, out: &mut RgbaImage) {
        // **Transparent** background: the HUD is drawn over the field so the
        // blue frame on the top edge stays visible (design request).
        let labels = hud_labels(self.language);
        let white = [255, 255, 255, 255];
        let yellow = [252, 228, 48, 255]; // yellow from the original palette

        // Label row (y=2) and value row (y=12), as in INFO.PIX.
        draw_text_outline(out, 50, 2, 1, white, labels[0]);
        draw_text_outline(out, 191, 2, 1, white, labels[1]);
        draw_text_outline(out, 302, 2, 1, white, labels[2]);
        draw_text_outline(out, 465, 2, 1, white, labels[3]);

        draw_text_outline(out, 50, 12, 1, yellow, &num(self.percent, FMT_PCT));
        draw_text_outline(out, 78, 12, 1, yellow, "%");
        draw_text_outline(out, 191, 12, 1, yellow, &num(self.level + 1, FMT_LEVEL));
        draw_text_outline(out, 302, 12, 1, yellow, &num(self.score, FMT_SCORE));
        draw_text_outline(out, 465, 12, 1, yellow, &num(self.lives, FMT_LIVES));
    }

    /// Options screen: a panel with the settings, the selected row highlighted.
    fn draw_options(&self, out: &mut RgbaImage) {
        let white = [255, 255, 255, 255];
        let yellow = [252, 228, 48, 255];
        let grey = [170, 170, 170, 255];

        let w = 500u32;
        let h = 350u32;
        let x = (out.width.saturating_sub(w)) / 2;
        let y = (out.height.saturating_sub(h)) / 2;

        fill_rect_blend(out, x, y, w, h, [10, 10, 30], 235);
        // Frame
        fill_rect(out, x, y, w, 2, [0, 0, 255, 255]);
        fill_rect(out, x, y + h - 2, w, 2, [0, 0, 255, 255]);
        fill_rect(out, x, y, 2, h, [0, 0, 255, 255]);
        fill_rect(out, x + w - 2, y, 2, h, [0, 0, 255, 255]);

        draw_text(out, x + 24, y + 18, 2, white, "OPTIONS");

        for (i, (label, value)) in self.option_rows().iter().enumerate() {
            let ry = y + 66 + i as u32 * 26;
            let selected = i == self.options_index;
            let colour = if selected { yellow } else { white };
            if selected {
                draw_text(out, x + 12, ry, 1, colour, ">");
            }
            draw_text_outline(out, x + 30, ry, 1, colour, label);
            let vx = x + w - 30 - (value.chars().count() as u32) * 8;
            draw_text_outline(out, vx, ry, 1, colour, value);
        }

        draw_text(
            out,
            x + 24,
            y + h - 30,
            1,
            grey,
            "UP/DOWN SELECT   LEFT/RIGHT CHANGE   ESC BACK",
        );
    }

    /// High scores (left) and achievements (right).
    fn draw_scores(&self, out: &mut RgbaImage) {
        let white = [255, 255, 255, 255];
        let yellow = [252, 228, 48, 255];
        let grey = [150, 150, 150, 255];

        fill_rect(out, 0, 0, out.width, out.height, [0, 0, 0, 255]);
        draw_text(out, 24, 16, 2, white, "HIGH SCORES");
        draw_text(out, 340, 16, 2, white, "ACHIEVEMENTS");

        // --- High scores (left column) ---
        let mut y = 60;
        if self.scores.entries.is_empty() {
            draw_text(out, 24, y, 1, grey, "NO SCORES YET");
            y += 16;
        }
        for (i, e) in self.scores.entries.iter().enumerate() {
            let line = format!(
                "{:>2}. {:<3} {:>7}  LV {:>2}  {:>3}%",
                i + 1,
                e.name,
                e.score,
                e.level + 1,
                e.percent
            );
            draw_text(out, 24, y, 1, if i == 0 { yellow } else { white }, &line);
            y += 16;
        }

        // --- Achievements (right column) ---
        let (done, total) = self.achievements.progress();
        draw_text(
            out,
            340,
            48,
            1,
            yellow,
            &format!("{done}/{total} UNLOCKED"),
        );
        let mut y = 70;
        for (id, desc) in ALL_ACHIEVEMENTS.iter() {
            let unlocked = self.achievements.is_unlocked(id);
            let mark = if unlocked { "[X]" } else { "[ ]" };
            let colour = if unlocked { white } else { grey };
            let text: String = format!("{mark} {desc}").chars().take(34).collect();
            draw_text(out, 340, y, 1, colour, &text);
            y += 14;
        }

        draw_text(out, 24, out.height - 24, 1, grey, "ESC BACK");
    }
}

// ---------------------------------------------------------------------------
// Art loading (with cache).
// ---------------------------------------------------------------------------

/// Cache of decoded art. The art depends on the language in `menus`/`makers`;
/// if the language changes, those entries are reloaded (see
/// [`Art::set_language`]).
#[derive(Default)]
struct Art {
    lang: Option<Language>,
    jpgs: Vec<(&'static str, RgbaImage)>,
    pixs: Vec<(&'static str, RgbaImage)>,
    menus: [Option<RgbaImage>; 3],
    makers: Option<RgbaImage>,
    felds: [Option<RgbaImage>; 14],
}

impl Art {
    /// JPEG from `assets/data/<name>.jpg`.
    fn jpg(&mut self, name: &'static str) -> Result<&RgbaImage> {
        if let Some(i) = self.jpgs.iter().position(|(n, _)| *n == name) {
            return Ok(&self.jpgs[i].1);
        }
        let img = assets::load_jpeg(&assets::asset_path(&format!("data/{name}.jpg")))
            .with_context(|| format!("load {name}.jpg"))?;
        self.jpgs.push((name, img));
        Ok(&self.jpgs.last().unwrap().1)
    }

    /// `.PIX` file from `assets/data/`.
    ///
    /// The original `.PIX` is tried first via `assets::decode_pix`; if the
    /// load fails it falls back to the equivalent PNG in `assets/png/` (same
    /// pixels, see `tools/pix2png.py`) — a useful *fallback* while
    /// `decode_pix` was a stub. TODO(decode_pix): remove the fallback once the
    /// native decoding is confirmed to be stable.
    fn pix(&mut self, name: &'static str) -> Result<&RgbaImage> {
        if let Some(i) = self.pixs.iter().position(|(n, _)| *n == name) {
            return Ok(&self.pixs[i].1);
        }
        let from_pix =
            assets::load_pix_file(&assets::asset_path(&format!("data/{name}.pix")))
                .with_context(|| format!("load {name}.pix"));
        let img = match from_pix {
            Ok(img) => img,
            Err(_) => assets::load_jpeg(&assets::asset_path(&format!("png/{name}.png")))
                .with_context(|| format!("load {name}.png (fallback for {name}.pix)"))?,
        };
        self.pixs.push((name, img));
        Ok(&self.pixs.last().unwrap().1)
    }

    /// Playfield `FELD<index>.JPG`, 1-based index (1..=14).
    fn feld(&mut self, index: u32) -> Result<&RgbaImage> {
        let i = ((index - 1) % 14) as usize;
        if self.felds[i].is_none() {
            let img = assets::load_jpeg(&assets::asset_path(&format!("data/feld{}.jpg", i + 1)))
                .with_context(|| format!("load feld{}.jpg", i + 1))?;
            self.felds[i] = Some(img);
        }
        Ok(self.felds[i].as_ref().unwrap())
    }

    /// Invalidates the language-dependent art if it has changed.
    fn set_language(&mut self, lang: Language) {
        if self.lang != Some(lang) {
            self.menus = Default::default();
            self.makers = None;
            self.lang = Some(lang);
        }
    }

    /// Menu image (0=MENU1, 1=MENU2, 2=MENU3) per language:
    /// `En` → `EMENU*.JPG`; `Es` → `MENU*.JPG` (see [`Language`]). **[I]**
    fn menu(&mut self, index: usize, lang: Language) -> Result<&RgbaImage> {
        self.set_language(lang);
        if self.menus[index].is_none() {
            let stem = match lang {
                Language::En => ["emenu1", "emenu2", "emenu3"][index],
                Language::Es => ["menu1", "menu2", "menu3"][index],
            };
            let img = self.jpg(stem)?.clone();
            self.menus[index] = Some(img);
        }
        Ok(self.menus[index].as_ref().unwrap())
    }

    /// Credits screen: `MAKERS.PIX` / `EMAKERS.PIX` per language. **[O]**
    fn makers(&mut self, lang: Language) -> Result<&RgbaImage> {
        self.set_language(lang);
        if self.makers.is_none() {
            let stem = match lang {
                Language::En => "emakers",
                Language::Es => "makers",
            };
            let img = self.pix(stem)?.clone();
            self.makers = Some(img);
        }
        Ok(self.makers.as_ref().unwrap())
    }
}

// ---------------------------------------------------------------------------
// Drawing primitives (pure, over RgbaImage).
// ---------------------------------------------------------------------------

/// 1:1 opaque blit clipped to the destination size.
fn blit(dst: &mut RgbaImage, src: &RgbaImage) {
    blit_rows(dst, src, src.height);
}

/// Blit of the first `rows` rows of `src` (vertical wipe).
fn blit_rows(dst: &mut RgbaImage, src: &RgbaImage, rows: u32) {
    let w = dst.width.min(src.width);
    let h = dst.height.min(src.height).min(rows);
    for y in 0..h {
        let s = ((y * src.width) * 4) as usize;
        let d = ((y * dst.width) * 4) as usize;
        let n = (w * 4) as usize;
        dst.data[d..d + n].copy_from_slice(&src.data[s..s + n]);
    }
}

/// 1:1 blit taking `key` (RGB) as transparent (8bpp color-key). **[I]**
fn blit_keyed(dst: &mut RgbaImage, src: &RgbaImage, key: [u8; 3]) {
    let w = dst.width.min(src.width);
    let h = dst.height.min(src.height);
    for y in 0..h {
        for x in 0..w {
            let p = src.pixel(x, y);
            if p[0] != key[0] || p[1] != key[1] || p[2] != key[2] {
                dst.put_pixel(x, y, p);
            }
        }
    }
}

/// Opaque filled rectangle, clipped.
fn fill_rect(dst: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) {
    let x1 = dst.width.min(x + w);
    let y1 = dst.height.min(y + h);
    for yy in y..y1 {
        for xx in x..x1 {
            dst.put_pixel(xx, yy, rgba);
        }
    }
}

/// Filled rectangle with alpha blending (`alpha` 0..=255).
fn fill_rect_blend(dst: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3], alpha: u8) {
    let x1 = dst.width.min(x + w);
    let y1 = dst.height.min(y + h);
    let a = alpha as u32;
    for yy in y..y1 {
        for xx in x..x1 {
            let p = dst.pixel(xx, yy);
            dst.put_pixel(
                xx,
                yy,
                [
                    ((p[0] as u32 * (255 - a) + rgb[0] as u32 * a) / 255) as u8,
                    ((p[1] as u32 * (255 - a) + rgb[1] as u32 * a) / 255) as u8,
                    ((p[2] as u32 * (255 - a) + rgb[2] as u32 * a) / 255) as u8,
                    p[3],
                ],
            );
        }
    }
}

/// Rectangle outline (1 px).
fn draw_rect(dst: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) {
    if w == 0 || h == 0 {
        return;
    }
    for xx in x..x.saturating_add(w).min(dst.width) {
        dst.put_pixel(xx, y, rgba);
        dst.put_pixel(xx, y.saturating_add(h - 1), rgba);
    }
    for yy in y..y.saturating_add(h).min(dst.height) {
        dst.put_pixel(x, yy, rgba);
        dst.put_pixel(x.saturating_add(w - 1), yy, rgba);
    }
}

// ---------------------------------------------------------------------------
// Embedded 8×8 font (ASCII 32..=95; lowercase is drawn as uppercase).
// ---------------------------------------------------------------------------

/// One entry per character from ' ' (32) to '_' (95): 8 rows × 8 columns with
/// '#' = pixel. It is validated in the tests.
const FONT_ART: [&str; 64] = [
    // 32 ' '
    "        \n        \n        \n        \n        \n        \n        \n        ",
    // 33 '!'
    "   #    \n   #    \n   #    \n   #    \n   #    \n        \n   #    \n        ",
    // 34 '"'
    " #   #  \n #   #  \n        \n        \n        \n        \n        \n        ",
    // 35 '#'
    " #  #   \n######  \n #  #   \n #  #   \n######  \n #  #   \n        \n        ",
    // 36 '$'
    "   #    \n ###### \n##      \n #####  \n    ##  \n######  \n   #    \n        ",
    // 37 '%'
    "##   #  \n##  #   \n   #    \n  #     \n #      \n#   ##  \n#   ##  \n        ",
    // 38 '&'
    "  ##    \n #  #   \n #  #   \n  ##    \n #  # # \n#    #  \n ###  # \n        ",
    // 39 '\''
    "  ##    \n  ##    \n #      \n        \n        \n        \n        \n        ",
    // 40 '('
    "   ##   \n  ##    \n ##     \n ##     \n ##     \n  ##    \n   ##   \n        ",
    // 41 ')'
    " ##     \n  ##    \n   ##   \n   ##   \n   ##   \n  ##    \n ##     \n        ",
    // 42 '*'
    "   #    \n # # #  \n  ###   \n #####  \n  ###   \n # # #  \n   #    \n        ",
    // 43 '+'
    "        \n   #    \n   #    \n #####  \n   #    \n   #    \n        \n        ",
    // 44 ','
    "        \n        \n        \n        \n  ##    \n  ##    \n #      \n        ",
    // 45 '-'
    "        \n        \n        \n        \n #####  \n        \n        \n        ",
    // 46 '.'
    "        \n        \n        \n        \n        \n        \n  ##    \n        ",
    // 47 '/'
    "      # \n     #  \n    #   \n   #    \n  #     \n #      \n#       \n        ",
    // 48 '0'
    " #####  \n##   ## \n##  ### \n## # ## \n###  ## \n##   ## \n #####  \n        ",
    // 49 '1'
    "  ##    \n ###    \n  ##    \n  ##    \n  ##    \n  ##    \n######  \n        ",
    // 50 '2'
    " #####  \n##   ## \n     ## \n   ###  \n  ##    \n ##     \n######  \n        ",
    // 51 '3'
    " #####  \n##   ## \n     ## \n  ####  \n     ## \n##   ## \n #####  \n        ",
    // 52 '4'
    "   ###  \n  ####  \n ## ##  \n##  ##  \n######  \n   ##   \n   ##   \n        ",
    // 53 '5'
    "######  \n##      \n#####   \n     ## \n     ## \n##   ## \n #####  \n        ",
    // 54 '6'
    " #####  \n##   ## \n##      \n#####   \n##   ## \n##   ## \n #####  \n        ",
    // 55 '7'
    "######  \n     ## \n    ##  \n   ##   \n  ##    \n  ##    \n  ##    \n        ",
    // 56 '8'
    " #####  \n##   ## \n##   ## \n #####  \n##   ## \n##   ## \n #####  \n        ",
    // 57 '9'
    " #####  \n##   ## \n##   ## \n ###### \n     ## \n##   ## \n #####  \n        ",
    // 58 ':'
    "        \n        \n  ##    \n        \n        \n  ##    \n        \n        ",
    // 59 ';'
    "        \n        \n  ##    \n        \n        \n  ##    \n #      \n        ",
    // 60 '<'
    "    ##  \n   #    \n  #     \n #      \n  #     \n   #    \n    ##  \n        ",
    // 61 '='
    "        \n        \n #####  \n        \n #####  \n        \n        \n        ",
    // 62 '>'
    " ##     \n  #     \n   #    \n    #   \n   #    \n  #     \n ##     \n        ",
    // 63 '?'
    " #####  \n#     # \n     #  \n   ##   \n   #    \n        \n   #    \n        ",
    // 64 '@'
    " #####  \n#     # \n#  ###  \n# #  #  \n#  ###  \n#       \n #####  \n        ",
    // 65 'A'
    " #####  \n##   ## \n##   ## \n######  \n##   ## \n##   ## \n##   ## \n        ",
    // 66 'B'
    "#####   \n##   ## \n##   ## \n#####   \n##   ## \n##   ## \n#####   \n        ",
    // 67 'C'
    " #####  \n##   ## \n##      \n##      \n##      \n##   ## \n #####  \n        ",
    // 68 'D'
    "#####   \n##   ## \n##   ## \n##   ## \n##   ## \n##   ## \n#####   \n        ",
    // 69 'E'
    "######  \n##      \n##      \n#####   \n##      \n##      \n######  \n        ",
    // 70 'F'
    "######  \n##      \n##      \n#####   \n##      \n##      \n##      \n        ",
    // 71 'G'
    " #####  \n##   ## \n##      \n##  ### \n##   ## \n##   ## \n #####  \n        ",
    // 72 'H'
    "##   ## \n##   ## \n##   ## \n######  \n##   ## \n##   ## \n##   ## \n        ",
    // 73 'I'
    "######  \n  ##    \n  ##    \n  ##    \n  ##    \n  ##    \n######  \n        ",
    // 74 'J'
    "    ##  \n    ##  \n    ##  \n    ##  \n    ##  \n##  ##  \n ####   \n        ",
    // 75 'K'
    "##   ## \n##  ##  \n####    \n###     \n####    \n##  ##  \n##   ## \n        ",
    // 76 'L'
    "##      \n##      \n##      \n##      \n##      \n##      \n######  \n        ",
    // 77 'M'
    "##   ## \n### ### \n####### \n## # ## \n##   ## \n##   ## \n##   ## \n        ",
    // 78 'N'
    "##   ## \n###  ## \n#### ## \n## #### \n##  ### \n##   ## \n##   ## \n        ",
    // 79 'O'
    " #####  \n##   ## \n##   ## \n##   ## \n##   ## \n##   ## \n #####  \n        ",
    // 80 'P'
    "#####   \n##   ## \n##   ## \n#####   \n##      \n##      \n##      \n        ",
    // 81 'Q'
    " #####  \n##   ## \n##   ## \n##   ## \n## # ## \n##  ##  \n #### # \n        ",
    // 82 'R'
    "#####   \n##   ## \n##   ## \n#####   \n####    \n##  ##  \n##   ## \n        ",
    // 83 'S'
    " #####  \n##   ## \n ##     \n  ####  \n     ## \n##   ## \n #####  \n        ",
    // 84 'T'
    "######  \n  ##    \n  ##    \n  ##    \n  ##    \n  ##    \n  ##    \n        ",
    // 85 'U'
    "##   ## \n##   ## \n##   ## \n##   ## \n##   ## \n##   ## \n #####  \n        ",
    // 86 'V'
    "##   ## \n##   ## \n##   ## \n##   ## \n ## ##  \n  ###   \n   #    \n        ",
    // 87 'W'
    "##   ## \n##   ## \n##   ## \n## # ## \n####### \n### ### \n##   ## \n        ",
    // 88 'X'
    "##   ## \n ## ##  \n  ###   \n  ###   \n  ###   \n ## ##  \n##   ## \n        ",
    // 89 'Y'
    "##   ## \n##   ## \n ## ##  \n  ###   \n   #    \n   #    \n   #    \n        ",
    // 90 'Z'
    "######  \n    ##  \n   ##   \n  ##    \n ##     \n##      \n######  \n        ",
    // 91 '['
    " #####  \n #      \n #      \n #      \n #      \n #      \n #####  \n        ",
    // 92 '\\'
    "#       \n #      \n  #     \n   #    \n    #   \n     #  \n      # \n        ",
    // 93 ']'
    " #####  \n     #  \n     #  \n     #  \n     #  \n     #  \n #####  \n        ",
    // 94 '^'
    "   #    \n  # #   \n #   #  \n        \n        \n        \n        \n        ",
    // 95 '_'
    "        \n        \n        \n        \n        \n        \n        \n######  ",
];

/// Bit table: `font()[c - 32][row]` = 8-bit mask (bit 0 = column 0).
fn font() -> &'static [[u8; 8]; 64] {
    static FONT: OnceLock<[[u8; 8]; 64]> = OnceLock::new();
    FONT.get_or_init(|| {
        let mut table = [[0u8; 8]; 64];
        for (i, art) in FONT_ART.iter().enumerate() {
            for (r, row) in art.split('\n').enumerate() {
                if r >= 8 {
                    break;
                }
                for (c, ch) in row.chars().enumerate() {
                    if ch == '#' {
                        table[i][r] |= 1 << c;
                    }
                }
            }
        }
        table
    })
}

/// Draws text at scale `scale` (1 = 8×8 per character). Lowercase → uppercase;
/// characters outside ASCII 32..=95 are ignored (the texts are unaccented).
fn draw_text(dst: &mut RgbaImage, x: u32, y: u32, scale: u32, rgba: [u8; 4], text: &str) {
    let mut cx = x;
    for ch in text.chars() {
        let ch = ch.to_ascii_uppercase();
        let idx = ch as usize;
        if !(32..=95).contains(&idx) {
            cx += 8 * scale;
            continue;
        }
        let glyph = &font()[idx - 32];
        for (r, row) in glyph.iter().enumerate() {
            for c in 0..8u32 {
                if row & (1 << c) != 0 {
                    fill_rect(
                        dst,
                        cx + c * scale,
                        y + r as u32 * scale,
                        scale,
                        scale,
                        rgba,
                    );
                }
            }
        }
        cx += 8 * scale;
    }
}

/// Text with a **dark outline** so it can be read over the field image
/// without an opaque background (the HUD goes on top and is transparent).
fn draw_text_outline(dst: &mut RgbaImage, x: u32, y: u32, scale: u32, rgba: [u8; 4], text: &str) {
    let shadow = [0, 0, 0, 255];
    let d = scale;
    for (dx, dy) in [(-1i32, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
        let nx = x as i32 + dx * d as i32;
        let ny = y as i32 + dy * d as i32;
        if nx >= 0 && ny >= 0 {
            draw_text(dst, nx as u32, ny as u32, scale, shadow, text);
        }
    }
    draw_text(dst, x, y, scale, rgba, text);
}

/// Number with the HUD's `%N.Nd` format (§6): fixed width with leading zeros.
fn num(value: u32, width: usize) -> String {
    format!("{:0width$}", value, width = width)
}

// ---------------------------------------------------------------------------
// Per-language texts
// ---------------------------------------------------------------------------

/// HUD labels: (percentage, level, score, lives).
///
/// EN follows `INFO.PIX` literally ("Filled: % Level: Points: Lives:"); DE
/// ("Fertig: % Level: Punkte: Leben:"); the `Es` variant is our own
/// translation. **[I]**
/// HUD labels. They are always in English: the language switch only changes
/// the in-game art (``MENU*``/``EMENU*``, ``MAKERS``/``EMAKERS``) and the help
/// panel.
fn hud_labels(_lang: Language) -> [&'static str; 4] {
    ["FILLED:", "LEVEL:", "POINTS:", "LIVES:"]
}

/// Help panel (F1) or title "about".
///
/// The original shows a Win32 `MessageBox`: "Knuddel's Quest" on the title and
/// "Kwiks - Hilfe" as the global help (§1.2/§1.3). Here it is drawn as a panel
/// over the screen (allowed by the task); the content is textual. **[I]**
fn draw_help(
    out: &mut RgbaImage,
    about: bool,
    lang: Language,
    music_on: bool,
    sfx_on: bool,
    fast: bool,
) {
    let onoff = |b: bool| if b { "ON" } else { "OFF" };
    let onoff_es = |b: bool| if b { "SI" } else { "NO" };
    let (title, lines): (&str, Vec<String>) = if about {
        (
            "KWIKS",
            vec![
                "KNUDDEL'S QUEST".to_string(),
                "(C) 1998".to_string(),
                String::new(),
                match lang {
                    Language::Es => "PULSA CUALQUIER TECLA".to_string(),
                    Language::En => "PRESS ANY KEY".to_string(),
                },
            ],
        )
    } else {
        match lang {
            Language::Es => (
                "AYUDA",
                vec![
                    "FLECHAS: MOVER".to_string(),
                    "ENTER/ESPACIO: ACEPTAR".to_string(),
                    "ESC: CANCELAR / ATRACTIVO".to_string(),
                    "P: PAUSA".to_string(),
                    "F1: AYUDA".to_string(),
                    "O: OPCIONES   H: RECORDS".to_string(),
                    "F2: GRAFICOS RAPIDOS".to_string(),
                    format!("F8: MUSICA {}", onoff_es(music_on)),
                    format!("F9: SONIDO {}", onoff_es(sfx_on)),
                    format!("GRAFICOS RAPIDOS: {}", onoff_es(fast)),
                ],
            ),
            Language::En => (
                "HELP",
                vec![
                    "ARROWS: MOVE".to_string(),
                    "ENTER/SPACE: ACCEPT".to_string(),
                    "ESC: CANCEL / ATTRACT".to_string(),
                    "P: PAUSE".to_string(),
                    "F1: HELP".to_string(),
                    "O: OPTIONS   H: SCORES".to_string(),
                    "F2: FAST GRAPHICS".to_string(),
                    format!("F8: MUSIC {}", onoff(music_on)),
                    format!("F9: SOUND {}", onoff(sfx_on)),
                    format!("FAST GFX: {}", onoff(fast)),
                ],
            ),
        }
    };

    let scale = 2u32;
    let maxw = lines
        .iter()
        .map(|l| l.chars().count())
        .chain(std::iter::once(title.chars().count()))
        .max()
        .unwrap_or(0) as u32;
    let w = (maxw * 8 * scale + 32).min(600);
    let h = (lines.len() as u32 + 1) * (8 * scale + 4) + 32;
    let x = (out.width.saturating_sub(w)) / 2;
    let y = (out.height.saturating_sub(h)) / 2;

    fill_rect_blend(out, x, y, w, h, [0, 0, 0], 200);
    draw_rect(out, x, y, w, h, [252, 228, 48, 255]);
    draw_text(out, x + 16, y + 16, scale, [252, 228, 48, 255], title);
    let mut ty = y + 16 + 8 * scale + 8;
    for line in &lines {
        draw_text(out, x + 16, ty, scale, [255, 255, 255, 255], line);
        ty += 8 * scale + 4;
    }
}

// ---------------------------------------------------------------------------
// Tests (the module is pure: everything is testable without SDL).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn buf() -> RgbaImage {
        RgbaImage::new(640, 480)
    }

    fn tick(shell: &mut Shell, input: Option<Input>) -> TickResult {
        let mut out = buf();
        shell.tick(input, &mut out).unwrap()
    }

    #[test]
    fn scores_screen_opens_and_goes_back() {
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Menu1).unwrap();
        assert!(go(tick(&mut s, Some(Input::Scores)), Screen::Scores));
        assert!(go(tick(&mut s, Some(Input::Cancel)), Screen::Menu1));
    }

    #[test]
    fn scores_do_not_open_during_a_game() {
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        assert!(matches!(
            tick(&mut s, Some(Input::Scores)),
            TickResult::Stay
        ));
        assert_eq!(s.screen, Screen::Game);
    }

    #[test]
    fn options_open_from_a_menu_and_return() {
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Menu1).unwrap();
        assert!(go(tick(&mut s, Some(Input::Options)), Screen::Options));
        assert!(go(tick(&mut s, Some(Input::Cancel)), Screen::Menu1));
    }

    #[test]
    fn options_toggle_widescreen_and_frame_limit() {
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Menu1).unwrap();
        let _ = tick(&mut s, Some(Input::Options));

        let before = s.options.widescreen;
        let _ = tick(&mut s, Some(Input::Right));
        assert_ne!(s.options.widescreen, before, "Right changes the first row");

        // Down to the frame limit row and cycle it.
        let _ = tick(&mut s, Some(Input::Down));
        let _ = tick(&mut s, Some(Input::Down));
        let before = s.options.fps_cap;
        let _ = tick(&mut s, Some(Input::Right));
        assert_ne!(s.options.fps_cap, before, "the frame limit cycles");
    }

    #[test]
    fn options_language_toggle_updates_the_shell() {
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Menu1).unwrap();
        let _ = tick(&mut s, Some(Input::Options));
        // Language is row 4.
        for _ in 0..4 {
            let _ = tick(&mut s, Some(Input::Down));
        }
        let before = s.language;
        let _ = tick(&mut s, Some(Input::Right));
        assert_ne!(s.language, before, "the language toggle reaches the shell");
    }

    #[test]
    fn options_do_not_open_during_a_game() {
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        assert!(matches!(
            tick(&mut s, Some(Input::Options)),
            TickResult::Stay
        ));
        assert_eq!(s.screen, Screen::Game);
    }

    fn go(result: TickResult, screen: Screen) -> bool {
        matches!(result, TickResult::Go(s) if s == screen)
    }

    fn quit(result: TickResult) -> bool {
        matches!(result, TickResult::Quit)
    }

    #[test]
    fn font_8x8_well_formed() {
        for art in FONT_ART {
            let rows: Vec<&str> = art.split('\n').collect();
            assert_eq!(rows.len(), 8, "glyph with != 8 rows: {art:?}");
            for row in rows {
                assert_eq!(row.chars().count(), 8, "row with != 8 cols: {art:?}");
            }
        }
    }

    #[test]
    fn title_confirm_goes_to_menu1() {
        let mut s = Shell::new();
        assert_eq!(s.screen, Screen::Title);
        assert!(go(tick(&mut s, Some(Input::Confirm)), Screen::Menu1));
        assert_eq!(s.screen, Screen::Menu1);
        // Esc also goes to menu 1 (§1.2).
        let mut s = Shell::new();
        assert!(go(tick(&mut s, Some(Input::Cancel)), Screen::Menu1));
    }

    #[test]
    fn menus_navigate_with_arrow_keys() {
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Confirm)); // → Menu1
        assert!(go(tick(&mut s, Some(Input::Down)), Screen::Menu2));
        assert!(go(tick(&mut s, Some(Input::Down)), Screen::Menu3));
        assert!(go(tick(&mut s, Some(Input::Up)), Screen::Menu2));
        assert!(go(tick(&mut s, Some(Input::Up)), Screen::Menu1));
        // Up on Menu1 does not wrap (inferred: the original does not trace it).
        assert!(matches!(tick(&mut s, Some(Input::Up)), TickResult::Stay));
    }

    #[test]
    fn menu1_enter_starts_game() {
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Confirm)); // Title → Menu1
        s.score = 999;
        s.lives = 1;
        assert!(go(tick(&mut s, Some(Input::Confirm)), Screen::Game));
        assert_eq!(s.level, 0);
        assert_eq!(s.score, 0);
        assert_eq!(s.lives, 3);
        assert_eq!(s.percent, 0);
    }

    #[test]
    fn menu2_esc_closes_the_program() {
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Confirm));
        tick(&mut s, Some(Input::Down)); // → Menu2
        assert!(!s.quit_requested);
        // Esc in MENU2 → click + `FUN_00405a70` (handler = 0) →
        // `PostQuitMessage`: end of the program [O] 0x4057ce.
        assert!(quit(tick(&mut s, Some(Input::Cancel))));
        assert!(s.quit_requested);
    }

    #[test]
    fn game_pauses_and_resumes() {
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        assert!(go(tick(&mut s, Some(Input::Pause)), Screen::Pause));
        // While paused only P resumes (§1.2).
        assert!(matches!(tick(&mut s, Some(Input::Confirm)), TickResult::Stay));
        assert!(go(tick(&mut s, Some(Input::Pause)), Screen::Game));
    }

    #[test]
    fn pause_keeps_level_and_score() {
        // Bug found: going through the pause reset the game. The pause is a
        // transient state and must not touch the game state.
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        s.level = 7;
        s.score = 2326;
        s.lives = 2;
        s.percent = 42;

        assert!(go(tick(&mut s, Some(Input::Pause)), Screen::Pause));
        assert!(go(tick(&mut s, Some(Input::Pause)), Screen::Game));

        assert_eq!(s.level, 7, "the level is kept");
        assert_eq!(s.score, 2326, "the score is kept");
        assert_eq!(s.lives, 2, "the lives are kept");
        assert_eq!(s.percent, 42, "the percentage is kept");
    }

    #[test]
    fn esc_in_game_is_game_over_and_returns_to_title() {
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        assert!(go(tick(&mut s, Some(Input::Cancel)), Screen::GameOver));
        assert!(go(tick(&mut s, Some(Input::Confirm)), Screen::Title));
    }

    #[test]
    fn level_complete_at_75_percent() {
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        s.percent = 74;
        assert!(matches!(tick(&mut s, None), TickResult::Stay));
        assert_eq!(s.game_mode, GameMode::Play);
        s.percent = 75;
        assert!(matches!(tick(&mut s, None), TickResult::Stay));
        assert_eq!(s.game_mode, GameMode::LevelComplete);
        assert!(!s.level_complete_skip); // [O] 0x4047e8 clears the flag
        // Enter/Esc/Space advance the level on the first press (mode 1, §2):
        // in the normal flow there is NO discard (NOTE(audit) in
        // handle_screen_key).
        assert!(matches!(tick(&mut s, Some(Input::Confirm)), TickResult::Stay));
        assert_eq!(s.level, 1);
        assert_eq!(s.percent, 0);
        assert_eq!(s.game_mode, GameMode::Play);
        assert!(s.level_complete_skip); // [O] 0x404a9f sets it again
    }

    #[test]
    fn level_complete_discards_first_press_if_flag() {
        // [O] 0x404a0e–0x404a20: with the flag `DAT_00460820` set, the 1st
        // keypress in mode 1 only discards (clears the flag and returns) and
        // the 2nd advances. NOTE(audit): the normal flow enters mode 1 with
        // the flag at 0 (0x4047e8), so this discard only happens in the
        // residual state (mode=1, flag=1) that row #43 marks [?]: exactly what
        // the race described there leaves behind (exiting with mode=1 and
        // restarting the game; `reset_session` does not touch the mode, like
        // `FUN_004045d4`).
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        s.game_mode = GameMode::LevelComplete;
        s.level_complete_skip = true;
        // 1st keypress: only discards the flag (level unchanged).
        assert!(matches!(tick(&mut s, Some(Input::Confirm)), TickResult::Stay));
        assert_eq!(s.level, 0);
        assert!(!s.level_complete_skip);
        assert_eq!(s.game_mode, GameMode::LevelComplete);
        // 2nd keypress: NEXT LEVEL.
        assert!(matches!(tick(&mut s, Some(Input::Confirm)), TickResult::Stay));
        assert_eq!(s.level, 1);
        assert_eq!(s.game_mode, GameMode::Play);
        assert!(s.level_complete_skip);
    }

    #[test]
    fn global_keys() {
        let mut s = Shell::new();
        assert!(s.music_on && s.sfx_on && !s.fast_graphics);
        tick(&mut s, Some(Input::Music));
        tick(&mut s, Some(Input::Sfx));
        tick(&mut s, Some(Input::FastGraphics));
        assert!(!s.music_on && !s.sfx_on && s.fast_graphics);
        // F1 opens/closes the help and does not transition.
        assert!(matches!(tick(&mut s, Some(Input::Help)), TickResult::Stay));
        assert!(s.help_visible);
        assert!(matches!(tick(&mut s, Some(Input::Help)), TickResult::Stay));
        assert!(!s.help_visible);
    }

    #[test]
    fn modal_help_closes_with_any_key() {
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Help));
        assert!(s.help_visible);
        // Enter would normally go to Menu1, but the help absorbs it.
        assert!(matches!(tick(&mut s, Some(Input::Confirm)), TickResult::Stay));
        assert!(!s.help_visible);
        assert_eq!(s.screen, Screen::Title);
    }

    #[test]
    fn language_is_toggleable() {
        let mut s = Shell::new();
        s.language = Language::Es;
        tick(&mut s, Some(Input::Confirm)); // Title → Menu1 (MENU*.JPG art)
        assert_eq!(s.language, Language::Es);
        s.toggle_language();
        assert_eq!(s.language, Language::En);
        assert_eq!(s.screen, Screen::Menu1); // the background is reloaded, no transition
    }

    #[test]
    fn menu1_esc_goes_to_attract() {
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Confirm)); // → Menu1
        assert!(go(tick(&mut s, Some(Input::Cancel)), Screen::Attract));
        assert_eq!(s.attract_index, 2); // enters with FELD2 (FUN_00401010)
    }

    #[test]
    fn attract_manual_carousel_ends_by_closing_the_program() {
        // [O] LAB_00401060: Esc/Enter/Space do not exit; they advance the
        // slide FELD2 (entry) → FELD4 → FELD6 → FELD8 → FELD10 → WERBUNG.JPG
        // (index `DAT_00429014` = 12) and the next keypress sets the handler
        // to 0 → `PostQuitMessage` (end of the program, same as Esc in menu 2).
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Confirm)); // → Menu1
        tick(&mut s, Some(Input::Cancel)); // → Attract (FELD2)
        for expected in [4, 6, 8, 10, 12] {
            assert!(matches!(tick(&mut s, Some(Input::Confirm)), TickResult::Stay));
            assert_eq!(s.attract_index, expected);
            assert!(!s.quit_requested);
        }
        // Slides exhausted → end of the program.
        assert!(quit(tick(&mut s, Some(Input::Confirm))));
        assert!(s.quit_requested);
        assert_eq!(s.screen, Screen::Attract);
    }

    #[test]
    fn attract_ignores_arrow_keys() {
        // [O] 0x401060 only polls Esc/Enter/Space; ↑/↓ do nothing.
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Confirm)); // → Menu1
        tick(&mut s, Some(Input::Cancel)); // → Attract
        assert_eq!(s.attract_index, 2);
        assert!(matches!(tick(&mut s, Some(Input::Up)), TickResult::Stay));
        assert!(matches!(tick(&mut s, Some(Input::Down)), TickResult::Stay));
        assert_eq!(s.attract_index, 2);
        assert_eq!(s.screen, Screen::Attract);
    }

    #[test]
    fn credits_return_to_menu2() {
        // [O] 0x405a11–0x405a64: Esc/Enter/Space → click + draws MENU2 +
        // handler 0x4057bc: they return to **menu 2** (not to attract; fixes
        // GAME_LOGIC.md §1.2, audit #23).
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Confirm)); // → Menu1
        tick(&mut s, Some(Input::Down)); // → Menu2
        assert!(go(tick(&mut s, Some(Input::Confirm)), Screen::Credits));
        assert!(go(tick(&mut s, Some(Input::Confirm)), Screen::Menu2));
        // Esc also returns to menu 2 (with click).
        assert!(go(tick(&mut s, Some(Input::Confirm)), Screen::Credits));
        assert!(go(tick(&mut s, Some(Input::Cancel)), Screen::Menu2));
    }

    #[test]
    fn game_over_by_death_changes_screen() {
        // [O] 0x4047ee: game-over flag → EXIT → FUN_004022d0 (GAMEOVER.PIX).
        // The exact hook is `Shell::request_game_over()`, which `main.rs`
        // calls when `Sim::kill_player` returns `true` (the 4th death,
        // `lives < 0`). `lives == 0` used to be used as a stand-in, but it
        // fired one death early (audit #50).
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        s.request_game_over();
        assert!(go(tick(&mut s, None), Screen::GameOver));
    }

    #[test]
    fn game_over_by_death_flag() {
        // The exact hook (`DAT_00460508 = 1`) also forces the exit.
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        s.request_game_over();
        assert!(go(tick(&mut s, None), Screen::GameOver));
    }

    #[test]
    fn click_only_on_menu_and_credits_transitions() {
        // [O] FUN_004043c4 sounds on the 10 menu/credits transitions and
        // never on the title nor in game over (audit #71).
        let mut s = Shell::new();
        // Title → Menu1: no click.
        tick(&mut s, Some(Input::Confirm));
        assert!(!s.take_click());
        // Menu1 ↓ → Menu2: click (one-shot).
        tick(&mut s, Some(Input::Down));
        assert!(s.take_click());
        assert!(!s.take_click());
        // Menu2 → credits and credits → Menu2: click.
        tick(&mut s, Some(Input::Confirm));
        assert!(s.take_click());
        tick(&mut s, Some(Input::Confirm));
        assert!(s.take_click());
        // Menu2 ↑ → Menu1 and Menu1 Enter → game: click.
        tick(&mut s, Some(Input::Up));
        assert!(s.take_click());
        tick(&mut s, Some(Input::Confirm));
        assert!(s.take_click());
        // In game (Esc → game over) and game over → title: no click.
        tick(&mut s, Some(Input::Cancel));
        assert!(!s.take_click());
        tick(&mut s, Some(Input::Confirm));
        assert!(!s.take_click());
    }

    #[test]
    fn click_in_menu3_and_on_esc_of_menus() {
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Confirm)); // Title → Menu1 (no click)
        tick(&mut s, Some(Input::Down)); // Menu1 → Menu2
        tick(&mut s, Some(Input::Down)); // Menu2 → Menu3
        assert!(s.take_click());
        tick(&mut s, Some(Input::Up)); // Menu3 → Menu2
        assert!(s.take_click());
        // Menu2 Esc → end of the program, with click (0x4057ce).
        assert!(quit(tick(&mut s, Some(Input::Cancel))));
        assert!(s.take_click());
    }

    #[test]
    fn help_processes_no_movement_or_actions() {
        // [O] the modal help (MessageBox) freezes the game: no key is
        // processed while it is open (audit #35/#53). In the port the help
        // absorbs the key: neither ↑/↓ navigate nor P/Esc transition.
        let mut s = Shell::new();
        tick(&mut s, Some(Input::Confirm)); // → Menu1
        tick(&mut s, Some(Input::Help));
        assert!(s.help_open());
        // ↓ would normally go to Menu2; the help absorbs it.
        assert!(matches!(tick(&mut s, Some(Input::Down)), TickResult::Stay));
        assert!(!s.help_open());
        assert_eq!(s.screen, Screen::Menu1);
        // In game, P/Esc do not pause or exit while the help is open.
        tick(&mut s, Some(Input::Confirm)); // → Game
        tick(&mut s, Some(Input::Help));
        assert!(matches!(tick(&mut s, Some(Input::Pause)), TickResult::Stay));
        assert_eq!(s.screen, Screen::Game);
        tick(&mut s, Some(Input::Help));
        assert!(matches!(tick(&mut s, Some(Input::Cancel)), TickResult::Stay));
        assert_eq!(s.screen, Screen::Game);
    }

    /// Capture of every screen to PNG (only on demand):
    /// `cargo test -- --ignored screen_captures`
    #[test]
    #[ignore]
    fn screen_captures() {
        use image::ImageEncoder;
        let dir = std::path::Path::new("/tmp/opencode/kwiks_shots");
        std::fs::create_dir_all(dir).unwrap();

        let mut dump = |shell: &mut Shell, name: &str, input: Option<Input>| {
            let mut out = buf();
            let _ = shell.tick(input, &mut out).unwrap();
            let path = dir.join(format!("{name}.png"));
            let file = std::fs::File::create(&path).unwrap();
            image::codecs::png::PngEncoder::new(file)
                .write_image(&out.data, out.width, out.height, image::ExtendedColorType::Rgba8)
                .unwrap();
            println!("  {path:?}");
        };

        // Options screen (port-only).
        let mut opts = Shell::new();
        opts.enter(Screen::Options).unwrap();
        dump(&mut opts, "00_options", None);

        // High scores and achievements (port-only), with a little sample data.
        let mut scores = Shell::new();
        scores.scores = crate::extras::HighScores::from_str(
            "KWIKS-HIGHSCORES-1\n\
             name=KWK score=2326 level=2 percent=78\n\
             name=YLE score=1200 level=1 percent=75\n\
             name=ACE score=900 level=0 percent=76\n",
        );
        let _ = scores.achievements.unlock("first_zone");
        let _ = scores.achievements.unlock("first_level");
        scores.enter(Screen::Scores).unwrap();
        dump(&mut scores, "00_scores", None);

        // Attract: full carousel (one-way only: it ends up closing the program).
        let mut s = Shell::new();
        dump(&mut s, "01_title", None);
        dump(&mut s, "02_menu1", Some(Input::Confirm));
        dump(&mut s, "03_menu2", Some(Input::Down));
        dump(&mut s, "04_menu3", Some(Input::Down));
        dump(&mut s, "05_attract_feld2", Some(Input::Confirm)); // Menu3 → Attract
        dump(&mut s, "06_attract_feld4", Some(Input::Confirm));
        dump(&mut s, "07_attract_feld6", Some(Input::Confirm));
        dump(&mut s, "08_attract_feld8", Some(Input::Confirm));
        dump(&mut s, "09_attract_feld10", Some(Input::Confirm));
        dump(&mut s, "10_attract_werbung", Some(Input::Confirm));
        dump(&mut s, "11_attract_end_quit", Some(Input::Confirm));

        // Game, interstitial, pause and game over.
        let mut s = Shell::new();
        dump(&mut s, "12_title", None);
        dump(&mut s, "13_menu1", Some(Input::Confirm));
        dump(&mut s, "14_game_hud", Some(Input::Confirm)); // Menu1 → Game
        s.percent = 80;
        dump(&mut s, "15_level_complete", None);
        dump(&mut s, "16_next_level", Some(Input::Confirm));
        dump(&mut s, "17_pause", Some(Input::Pause));
        dump(&mut s, "18_resume", Some(Input::Pause));
        dump(&mut s, "19_gameover", Some(Input::Cancel));
        dump(&mut s, "20_title", Some(Input::Confirm));

        // Credits (they return to menu 2) with the wipe fully played.
        let mut s = Shell::new();
        dump(&mut s, "21_title", None);
        dump(&mut s, "22_menu1", Some(Input::Confirm));
        dump(&mut s, "23_menu2", Some(Input::Down));
        dump(&mut s, "24_credits", Some(Input::Confirm));
        std::thread::sleep(CREDITS_WIPE + Duration::from_millis(200));
        dump(&mut s, "25_credits_wipe_end", None);
        dump(&mut s, "26_menu2_after_credits", Some(Input::Confirm));

        // Help and about + EN language
        let mut s = Shell::new();
        dump(&mut s, "27_title", None);
        dump(&mut s, "28_about", Some(Input::Help));
        dump(&mut s, "29_title_without_about", Some(Input::Help));
        s.toggle_language();
        dump(&mut s, "30_menu1_en", Some(Input::Confirm));
        dump(&mut s, "31_menu2_en", Some(Input::Down));
        dump(&mut s, "32_credits_en", Some(Input::Confirm));
    }

    #[test]
    fn game_over_draw() {
        // The GAMEOVER splash must paint pixels over the buffer.
        let mut s = Shell::new();
        s.reset_session();
        s.enter(Screen::Game).unwrap();
        tick(&mut s, Some(Input::Cancel)); // → GameOver
        let mut out = buf();
        let before = out.data.clone();
        s.tick(None, &mut out).unwrap();
        assert_ne!(out.data, before, "GameOver must draw its splash");
    }
}
