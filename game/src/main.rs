//! Shell of the Kwiks port: SDL3 + configurable internal resolution.
//!
//! Architecture:
//! - **Free rendering**: the loop draws at whatever rate the display allows.
//! - **40 Hz simulation**: `sim::FixedStep` fires `Sim::step` every 25 ms,
//!   just like the original tick (`0x404734`), so the gameplay does not change.
//! - **Internal resolution**: faithful 640×480 or widescreen-extended
//!   (wider playfield, not a stretched image).
//! - **Extras**: high scores, achievements and practice mode.

use anyhow::{Context, Result};
use sdl3::event::Event;
use sdl3::keyboard::Keycode;
use sdl3::pixels::{Color, PixelFormat};
use sdl3::rect::Rect;
use std::time::Instant;

mod assets;
mod audio;
mod config;
mod extras;
mod field;
mod screens;
mod sim;
mod video;

use assets::RgbaImage;
use audio::Audio;
use config::Config;
use extras::{Achievements, HighScore, HighScores};
use field::Field;
use screens::{Input, Screen, Shell, TickResult};
use sim::{FixedStep, InputState, Sim};
use video::InternalSize;


fn map_key(key: Keycode) -> Option<Input> {
    Some(match key {
        Keycode::Up => Input::Up,
        Keycode::Down => Input::Down,
        Keycode::Left => Input::Left,
        Keycode::Right => Input::Right,
        Keycode::Return | Keycode::Space => Input::Confirm,
        Keycode::Escape => Input::Cancel,
        Keycode::P => Input::Pause,
        Keycode::F2 => Input::FastGraphics,
        Keycode::F8 => Input::Music,
        Keycode::F9 => Input::Sfx,
        Keycode::F1 => Input::Help,
        Keycode::O => Input::Options,
        Keycode::H => Input::Scores,
        _ => return None,
    })
}

/// Sustained arrow state for the simulation.
#[derive(Default)]
struct Held {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

impl Held {
    fn update(&mut self, key: Keycode, pressed: bool) {
        match key {
            Keycode::Up => self.up = pressed,
            Keycode::Down => self.down = pressed,
            Keycode::Left => self.left = pressed,
            Keycode::Right => self.right = pressed,
            _ => {}
        }
    }

    fn to_input(&self) -> InputState {
        InputState {
            up: self.up,
            down: self.down,
            left: self.left,
            right: self.right,
        }
    }
}

/// Image of a level: `FELD((level % 14) + 1).JPG`, like in the original.
fn level_image(level: u32) -> Result<RgbaImage> {
    let name = format!("data/feld{}.jpg", (level % 14) + 1);
    let path = assets::asset_path(&name);
    assets::load_jpeg(&path).with_context(|| format!("load {name}"))
}

/// Expands the level image to the internal size by repeating the border.
///
/// Port design decision **[D]**: in widescreen the field is wider and the
/// extra area is conquerable field; when revealed, the image border is shown
/// repeated outwards, so the original art is not stretched.
fn extend_image(src: &RgbaImage, size: InternalSize) -> RgbaImage {
    let mut out = RgbaImage::new(size.width, size.height);
    let (ox, oy) = video::image_origin(size);
    for y in 0..size.height {
        for x in 0..size.width {
            // Source coordinate "clamped" to the image border.
            let sx = x.saturating_sub(ox).min(src.width.saturating_sub(1));
            let sy = y.saturating_sub(oy).min(src.height.saturating_sub(1));
            let rgba = if x >= ox && x < ox + src.width && y >= oy && y < oy + src.height {
                src.pixel(x - ox, y - oy)
            } else {
                src.pixel(sx, sy)
            };
            out.put_pixel(x, y, rgba);
        }
    }
    out
}

/// Music for each screen, per `GAME_LOGIC.md` §1 (TITEL/SPIEL/WERBE.MOD).
fn music_for(screen: Screen) -> Option<&'static str> {
    match screen {
        Screen::Title | Screen::Menu1 | Screen::Menu2 | Screen::Menu3 | Screen::Options
        | Screen::Scores => Some("titel.mod"),
        Screen::Game | Screen::Pause => Some("spiel.mod"),
        Screen::Attract | Screen::Credits | Screen::GameOver => Some("werbe.mod"),
    }
}

fn main() -> Result<()> {
    // ---- Persisted configuration ----
    let cfg_path = config::config_path();
    let mut cfg = Config::load(&cfg_path);

    // Fail early with an actionable message instead of a bare I/O error.
    if !assets::asset_path("data/titel.jpg").exists() {
        anyhow::bail!(
            "the game files are missing from {:?}\n\
             Run the launcher and press 'Set up and build', or extract them\n\
             with tools/extract_one.py (see README).",
            assets::asset_path("")
        );
    }

    let size = video::internal_size((cfg.window_width, cfg.window_height), cfg.widescreen);
    let mut size = size;

    // ---- Video ----
    let sdl = sdl3::init()?;
    let video_sub = sdl.video()?;

    let mut builder = video_sub.window("Kwiks", cfg.window_width, cfg.window_height);
    builder.position_centered();
    if cfg.fullscreen {
        builder.fullscreen();
    }
    let window = builder.build().context("create window")?;

    let mut canvas = window.into_canvas();
    // SDL does the presentation scaling. `integer_scale` asks for whole-pixel
    // multiples (pixel-perfect); otherwise the image is letterboxed.
    let presentation = if cfg.integer_scale {
        sdl3::sys::render::SDL_RendererLogicalPresentation::INTEGER_SCALE
    } else {
        sdl3::sys::render::SDL_RendererLogicalPresentation::LETTERBOX
    };
    canvas
        .set_logical_size(size.width, size.height, presentation)
        .context("logical resolution")?;
    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(PixelFormat::RGBA32, size.width, size.height)
        .context("create texture")?;

    // ---- State ----
    let mut framebuffer = RgbaImage::new(size.width, size.height);
    let mut shell = Shell::new();
    shell.music_on = cfg.music_on;
    shell.sfx_on = cfg.sfx_on;
    shell.options = cfg.clone();
    shell.language = match cfg.language.as_str() {
        "es" => screens::Language::Es,
        _ => screens::Language::En,
    };
    // Video settings currently applied to the renderer, to detect changes made
    // from the options screen.
    let mut applied = (cfg.widescreen, cfg.integer_scale, cfg.fullscreen);

    let mut sim: Option<Sim> = None;
    let mut fixed = FixedStep::new();
    let mut held = Held::default();
    let sprites = assets::monster_sprites()?;
    let mut audio = Audio::new().ok();
    let mut current_music: Option<String> = None;

    shell.scores = HighScores::load(&extras::highscores_path());
    shell.achievements = Achievements::load(&extras::achievements_path());

    // Practice mode: KWIKS_LEVEL=0..14 starts at that level.
    let practice_level: Option<u32> = std::env::var("KWIKS_LEVEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&l| l < extras::LEVEL_COUNT);
    if let Some(level) = practice_level {
        let mut p = extras::Practice::new(level);
        p.clamp();
        eprintln!("practice mode: level {}", p.level + 1);
    }

    let mut events_pump = sdl.event_pump()?;
    let mut frames: u64 = 0;
    let started = Instant::now();
    let mut last = Instant::now();
    let mut alive_since = Instant::now();
    let mut deaths_this_level = 0u32;

    // Smoke mode (no real window): KWIKS_SMOKE=<n> exits after n frames.
    let smoke: u64 = std::env::var("KWIKS_SMOKE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    'main: loop {
        let dt = last.elapsed().as_secs_f64();
        last = Instant::now();

        let mut input = None;
        for event in events_pump.poll_iter() {
            match event {
                Event::Quit { .. } => break 'main,
                Event::KeyDown { keycode: Some(k), .. } => {
                    held.update(k, true);
                    input = map_key(k);
                }
                Event::KeyUp { keycode: Some(k), .. } => held.update(k, false),
                _ => {}
            }
        }

        // 1) The screen draws its background/HUD and decides the transition.
        let result = shell.tick(input, &mut framebuffer)?;

        // Apply video settings changed from the options screen.
        let want = (
            shell.options.widescreen,
            shell.options.integer_scale,
            shell.options.fullscreen,
        );
        if want != applied {
            if want.2 != applied.2 {
                canvas.window_mut().set_fullscreen(want.2)?;
            }
            if want.0 != applied.0 || want.1 != applied.1 {
                size = video::internal_size(
                    (shell.options.window_width, shell.options.window_height),
                    want.0,
                );
                let presentation = if want.1 {
                    sdl3::sys::render::SDL_RendererLogicalPresentation::INTEGER_SCALE
                } else {
                    sdl3::sys::render::SDL_RendererLogicalPresentation::LETTERBOX
                };
                canvas.set_logical_size(size.width, size.height, presentation)?;
                texture = texture_creator.create_texture_streaming(
                    PixelFormat::RGBA32,
                    size.width,
                    size.height,
                )?;
                framebuffer = RgbaImage::new(size.width, size.height);
                eprintln!("video: {}x{}", size.width, size.height);
            }
            applied = want;
        }

        // The original closes the program when the attract loop runs out and
        // with Esc in MENU2 (`PostQuitMessage`, `0x405a11`/`0x4057ce`).
        if let TickResult::Quit = result {
            eprintln!("end of program");
            break 'main;
        }

        if let Some(Input::FastGraphics) = input {
            if shell.achievements.unlock("fast_graphics") {
                eprintln!("achievement: fast graphics");
                let _ = shell.achievements.save(&extras::achievements_path());
            }
        }

        // The menu click only sounds on real transitions (audit #69).
        if shell.take_click() {
            if let Some(a) = audio.as_mut() {
                let _ = a.play_sfx("click.wav");
            }
        }

        if let TickResult::Go(next) = result {
            eprintln!("transition → {next:?}");
            if next == Screen::Game && sim.is_none() {
                let start_level = practice_level.unwrap_or(0);
                let image = extend_image(&level_image(start_level)?, size);
                let mut s = Sim::new(Field::new_playfield(size.width, size.height, image));
                s.level = start_level;
                shell.level = start_level;
                sim = Some(s);
                alive_since = Instant::now();
                deaths_this_level = 0;
                if let Some(a) = audio.as_mut() {
                    let _ = a.play_sfx("los.wav");
                }
            }
            // The pause is a transient state: the game is **not** destroyed.
            // Only when actually leaving (title, menus, attract, game over) is
            // the simulation closed and the high score saved.
            if next != Screen::Game && next != Screen::Pause {
                if let Some(s) = sim.take() {
                    // End of game: save the high score if it makes the table.
                    if shell.scores.is_record(s.score) {
                        let entry = HighScore {
                            name: "KWIK".to_string(),
                            score: s.score,
                            level: s.level + 1,
                            percent: (s.field.fill_ratio() * 100.0) as u32,
                        };
                        if let Some(pos) = shell.scores.insert(entry) {
                            eprintln!("new record! position {}", pos + 1);
                        }
                        let _ = shell.scores.save(&extras::highscores_path());
                    }
                }
            }
        }

        // Menu click: now decided by `screens` (`take_click`). See above.

        // 2) 40 Hz simulation + field rendering.
        //    With the help open the game freezes (audit #35).
        if shell.screen == Screen::Game && !shell.help_open() {
            if let Some(s) = sim.as_mut() {
                // `screens` advances the level after the COMPLET interstitial
                // (mode 1); here the field is rebuilt with the new image.
                if shell.level != s.level {
                    let image = extend_image(&level_image(shell.level)?, size);
                    s.set_level(
                        shell.level,
                        Field::new_playfield(size.width, size.height, image),
                    );
                    deaths_this_level = 0;
                    alive_since = Instant::now();
                    if let Some(a) = audio.as_mut() {
                        let _ = a.play_sfx("los.wav"); // sound when the level starts
                    }
                }

                // F2 reduces the monsters' halo (`FUN_00403170`).
                s.fast_graphics = shell.fast_graphics;

                let mut steps = fixed.advance(dt);
                let mut game_over = false;
                let mut level_done = false;
                while steps > 0 {
                    s.step(&held.to_input());
                    s.check_extra_life();
                    for ev in s.drain_events() {
                        match ev {
                            sim::Event::PlayerDied => {
                                // The simulation has already deducted the life
                                // and respawned; here only the sound.
                                deaths_this_level += 1;
                                alive_since = Instant::now();
                                eprintln!("death! remaining lives: {}", s.lives);
                                if let Some(a) = audio.as_mut() {
                                    let _ = a.play_sfx("tot.wav");
                                }
                            }
                            sim::Event::GameOver => {
                                game_over = true;
                                shell.request_game_over();
                            }
                            sim::Event::Filled { pixels } => {
                                eprintln!("zone conquered: {pixels} px");
                                if shell.achievements.unlock("first_zone") {
                                    let _ = shell.achievements.save(&extras::achievements_path());
                                }
                                if let Some(a) = audio.as_mut() {
                                    let _ = a.play_sfx("gefuellt.wav");
                                }
                            }
                            sim::Event::LevelComplete => level_done = true,
                            sim::Event::ExtraLife => {
                                if shell.achievements.unlock("extra_life") {
                                    let _ = shell.achievements.save(&extras::achievements_path());
                                }
                                if let Some(a) = audio.as_mut() {
                                    let _ = a.play_sfx("leben.wav");
                                }
                            }
                        }
                    }
                    steps -= 1;
                }

                // Time- and score-based achievements.
                if alive_since.elapsed().as_secs() >= 60 && shell.achievements.unlock("survive_60s") {
                    let _ = shell.achievements.save(&extras::achievements_path());
                }
                if s.score >= 100_000 && shell.achievements.unlock("score_100k") {
                    let _ = shell.achievements.save(&extras::achievements_path());
                }

                if game_over {
                    eprintln!("game over — score: {}", s.score);
                } else if level_done {
                    // The COMPLET interstitial is handled by `screens` (mode 1):
                    // it waits for a keypress and then advances `shell.level`.
                    // Here only the achievements are recorded.
                    if shell.achievements.unlock("first_level") {
                        let _ = shell.achievements.save(&extras::achievements_path());
                    }
                    if deaths_this_level == 0 && shell.achievements.unlock("no_death_level") {
                        let _ = shell.achievements.save(&extras::achievements_path());
                    }
                    if s.level + 1 >= extras::LEVEL_COUNT {
                        if shell.achievements.unlock("all_levels") {
                            let _ = shell.achievements.save(&extras::achievements_path());
                        }
                        eprintln!("game completed!");
                    } else if s.level + 1 >= 5 && shell.achievements.unlock("five_levels") {
                        let _ = shell.achievements.save(&extras::achievements_path());
                    }
                }

                // The shell HUD is fed from the simulation (the level is
                // managed by `screens`, which is what advances after COMPLET).
                shell.score = s.score;
                shell.lives = s.lives.max(0) as u32;
                shell.percent = (s.field.fill_ratio() * 100.0) as u32;
                if shell.percent >= 90 && shell.achievements.unlock("fill_90") {
                    let _ = shell.achievements.save(&extras::achievements_path());
                }

                // The field is drawn full-screen (including the blue frame).
                // In the level-complete interstitial (mode 1) it is **not**
                // drawn: it would cover the COMPLET overlay.
                if shell.game_mode() == crate::screens::GameMode::Play {
                    s.field.draw_playfield(&mut framebuffer, 0);
                    s.draw_monsters(&mut framebuffer, &sprites);
                    s.draw_player(&mut framebuffer);
                    // The HUD is drawn last, on top and with a transparent
                    // background, so it does not cover the top frame.
                    shell.draw_hud(&mut framebuffer);
                }
            }
        }

        // 3) Audio: music per screen, F8/F9 toggles and effects.
        if let Some(a) = audio.as_mut() {
            a.set_music_enabled(shell.music_on);
            a.set_sfx_enabled(shell.sfx_on);
            let wanted = music_for(shell.screen).map(str::to_string);
            if wanted != current_music {
                match &wanted {
                    Some(name) => {
                        if let Err(e) = a.play_music(name) {
                            eprintln!("music {name}: {e}");
                        }
                    }
                    None => a.stop_music(),
                }
                current_music = wanted;
            }
            a.pump();
        }

        texture.update(None, &framebuffer.data, (size.width * 4) as usize)?;
        canvas.set_draw_color(Color::RGB(0, 0, 0));
        canvas.clear();
        canvas.copy(&texture, None, Rect::new(0, 0, size.width, size.height))?;
        canvas.present();

        // Optional FPS cap (0 = uncapped).
        if shell.options.fps_cap > 0 {
            let target = 1.0 / shell.options.fps_cap as f64;
            let spent = last.elapsed().as_secs_f64();
            if spent < target {
                std::thread::sleep(std::time::Duration::from_secs_f64(target - spent));
            }
        }

        frames += 1;
        if smoke > 0 && frames >= smoke {
            break;
        }
    }

    // Save configuration and extras on exit.
    // The options screen owns the settings; keep the shell in sync.
    cfg = shell.options.clone();
    cfg.music_on = shell.music_on;
    cfg.sfx_on = shell.sfx_on;
    cfg.language = match shell.language {
        screens::Language::Es => "es".to_string(),
        screens::Language::En => "en".to_string(),
    };
    let _ = cfg.save(&cfg_path);
    let _ = shell.scores.save(&extras::highscores_path());
    let _ = shell.achievements.save(&extras::achievements_path());

    let elapsed = started.elapsed().as_secs_f64();
    let fps = frames as f64 / elapsed;
    eprintln!(
        "ok: {frames} frames in {elapsed:.2}s ({fps:.1} fps) — {}x{}",
        size.width, size.height
    );
    Ok(())
}
