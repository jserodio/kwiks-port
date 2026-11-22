//! Tests of the configuration (`src/config.rs`) and video (`src/video.rs`):
//! round-trip of the `key=value` file, defaults, unknown or invalid values,
//! internal resolution per window aspect ratio (4:3, 16:9, 16:10, ultrawide)
//! and a letterbox that preserves the aspect ratio and centers (including the
//! integer scaling case).
//!
//! The crate is binary-only (no `lib.rs`), so the modules are included
//! directly with `#[path]` to avoid altering the rest of the project.

#[path = "../src/config.rs"]
mod config;
#[path = "../src/video.rs"]
mod video;

use config::{config_path, Config};
use video::{
    image_origin, internal_size, InternalSize, BASE_HEIGHT, BASE_WIDTH,
};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Clean temporary directory, unique per test (no external dependencies).
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kwiks-config-video-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Config with every field changed from the default (valid language).
fn custom_config() -> Config {
    Config {
        window_width: 1600,
        window_height: 1200,
        fullscreen: true,
        widescreen: true,
        integer_scale: false,
        fps_cap: 60,
        language: "en".to_string(),
        music_on: false,
        sfx_on: false,
        fast_graphics: true,
    }
}

// ---------------------------------------------------------------------------
// Config: defaults
// ---------------------------------------------------------------------------

#[test]
fn config_defaults() {
    let c = Config::default();
    assert_eq!(c.window_width, 1280);
    assert_eq!(c.window_height, 960);
    assert!(!c.fullscreen);
    assert!(!c.widescreen); // faithful 640×480 by default
    assert!(c.integer_scale);
    assert_eq!(c.fps_cap, 0); // uncapped
    assert_eq!(c.language, "en"); // the menu goes in English
    assert!(c.music_on);
    assert!(c.sfx_on);
}

// ---------------------------------------------------------------------------
// Config: parsing
// ---------------------------------------------------------------------------

#[test]
fn config_from_str_all_keys() {
    let text = "\
window_width=1920
window_height=1080
fullscreen=true
widescreen=true
integer_scale=false
fps_cap=75
language=en
music_on=false
sfx_on=false
";
    let c = Config::from_str(text);
    assert_eq!(c.window_width, 1920);
    assert_eq!(c.window_height, 1080);
    assert!(c.fullscreen);
    assert!(c.widescreen);
    assert!(!c.integer_scale);
    assert_eq!(c.fps_cap, 75);
    assert_eq!(c.language, "en");
    assert!(!c.music_on);
    assert!(!c.sfx_on);
}

#[test]
fn config_from_str_ignores_unknown_keys() {
    let text = "\
ultrawide_hacks=true
window_width=1600
sound_volume=11
other_key=whatever
";
    let c = Config::from_str(text);
    assert_eq!(c.window_width, 1600); // the known one is applied
    let d = Config::default();
    assert_eq!(c.window_height, d.window_height); // unknown keys do not break anything
    assert_eq!(c.widescreen, d.widescreen);
    assert_eq!(c.language, d.language);
}

#[test]
fn config_from_str_comments_blanks_and_whitespace() {
    let text = "\
# comment with = equals
; comment with ; semicolon

   widescreen = true
window_width = 1024
";
    let c = Config::from_str(text);
    assert!(c.widescreen);
    assert_eq!(c.window_width, 1024);
}

#[test]
fn config_from_str_invalid_values_keep_default() {
    let text = "\
window_width=abc
window_width=0
window_height=-10
fullscreen=maybe
fps_cap=-5
language=fr
language=
=no_key
line_without_equals
window_width
";
    let c = Config::from_str(text);
    assert_eq!(c, Config::default());
}

#[test]
fn config_from_str_bool_variants() {
    // Accepted as true.
    for v in ["true", "TRUE", "1", "yes", "On"] {
        let c = Config::from_str(&format!("widescreen={v}"));
        assert!(c.widescreen, "{v} should be true");
    }
    // Accepted as false.
    for v in ["false", "False", "0", "NO", "off"] {
        let c = Config::from_str(&format!("integer_scale={v}\nmusic_on={v}"));
        assert!(!c.integer_scale, "{v} should be false");
        assert!(!c.music_on, "{v} should be false");
    }
    // And non-boolean values change nothing.
    let c = Config::from_str("music_on=2");
    assert!(c.music_on);
}

#[test]
fn config_from_str_language_normalized() {
    assert_eq!(Config::from_str("language=EN").language, "en");
    assert_eq!(Config::from_str("language=Es").language, "es");
}

#[test]
fn config_from_str_duplicate_last_wins() {
    let c = Config::from_str("fps_cap=30\nfps_cap=144\n");
    assert_eq!(c.fps_cap, 144);
}

// ---------------------------------------------------------------------------
// Config: round-trip and file
// ---------------------------------------------------------------------------

#[test]
fn config_round_trip_to_string() {
    for cfg in [Config::default(), custom_config()] {
        let text = cfg.to_string();
        let back = Config::from_str(&text);
        assert_eq!(back, cfg, "broken round-trip for {cfg:?}\n---\n{text}");
    }
}

#[test]
fn config_save_load_round_trip_creates_dirs() {
    let dir = temp_dir("save-load");
    let path = dir.join("nested").join("other").join("kwiks.cfg");
    let cfg = custom_config();

    cfg.save(&path).expect("save config");
    assert!(path.is_file(), "the file should exist: {}", path.display());
    let text = std::fs::read_to_string(&path).expect("read file");
    assert!(text.contains("window_width=1600"));
    assert!(text.contains("widescreen=true"));

    let back = Config::load(&path);
    assert_eq!(back, cfg);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_load_missing_returns_default() {
    let dir = temp_dir("missing");
    let missing = dir.join("missing").join("kwiks.cfg");
    assert_eq!(Config::load(&missing), Config::default());
    // A directory instead of a file is not an error either.
    let _ = std::fs::create_dir_all(&dir);
    assert_eq!(Config::load(&dir), Config::default());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_path_shape() {
    let path = config_path();
    assert!(
        path.ends_with(".config/kwiks/kwiks.cfg"),
        "unexpected path: {}",
        path.display()
    );
    if let Some(home) = std::env::var_os("HOME") {
        assert!(
            path.starts_with(std::path::Path::new(&home)),
            "it should be under HOME: {}",
            path.display()
        );
    }
}

// ---------------------------------------------------------------------------
// Video: internal resolution
// ---------------------------------------------------------------------------

#[test]
fn internal_size_fidelity_4_3() {
    // Without widescreen the internal resolution is always 640×480, even if
    // the window is widescreen (fidelity to the original).
    for window in [(1280, 960), (800, 600), (1280, 720), (1920, 1080), (640, 480)] {
        assert_eq!(
            internal_size(window, false),
            InternalSize {
                width: 640,
                height: 480
            },
            "window {window:?}"
        );
    }
}

#[test]
fn internal_size_16_9() {
    for window in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
        assert_eq!(
            internal_size(window, true),
            InternalSize {
                width: 854,
                height: 480
            },
            "window {window:?}"
        );
    }
}

#[test]
fn internal_size_16_10() {
    for window in [(1280, 800), (1920, 1200), (1680, 1050)] {
        assert_eq!(
            internal_size(window, true),
            InternalSize {
                width: 768,
                height: 480
            },
            "window {window:?}"
        );
    }
}

#[test]
fn internal_size_ultrawide() {
    // 21:9 (64:27), 32:9 and "real" 21:9 at 3440×1440.
    assert_eq!(
        internal_size((2560, 1080), true),
        InternalSize {
            width: 1138,
            height: 480
        }
    );
    assert_eq!(
        internal_size((5120, 1440), true),
        InternalSize {
            width: 1708,
            height: 480
        }
    );
    assert_eq!(
        internal_size((3440, 1440), true),
        InternalSize {
            width: 1148,
            height: 480
        }
    );
}

#[test]
fn internal_size_clamps_and_degenerate() {
    // Portrait window or degenerate input: never narrower than the original.
    for window in [(720, 1280), (0, 0), (10, 10), (1, 1), (640, 480)] {
        let size = internal_size(window, true);
        assert!(
            size.width >= BASE_WIDTH && size.height == BASE_HEIGHT,
            "window {window:?} → {size:?}"
        );
    }
    assert_eq!(
        internal_size((0, 0), true),
        InternalSize {
            width: 640,
            height: 480
        }
    );
}

#[test]
fn internal_size_even_width_and_symmetric_margins() {
    // The width is even and the extra area splits into two exact halves, with
    // the original image centered (see the extended-field design decision).
    for window in [
        (1280, 720),
        (1920, 1080),
        (1280, 800),
        (2560, 1080),
        (5120, 1440),
        (1366, 768),
    ] {
        let size = internal_size(window, true);
        assert_eq!(size.height, BASE_HEIGHT);
        assert_eq!(size.width % 2, 0, "odd width for {window:?}");
        let (ox, oy) = image_origin(size);
        assert_eq!(oy, 0);
        assert_eq!(ox, (size.width - BASE_WIDTH) / 2);
        assert_eq!(
            ox,
            size.width - BASE_WIDTH - ox,
            "asymmetric margins for {window:?}"
        );
    }
}
