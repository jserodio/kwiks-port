//! Kwiks configuration: `key=value` file with no external dependencies.
//!
//! The original had no persistent options; this module adds a very simple
//! custom format (no `serde`, which is not in the project): one option per
//! line with the form `key=value`. Empty lines and comments (`#` or `;` at the
//! beginning of the line) are ignored, as well as unknown keys and values that
//! cannot be parsed: in those cases the option keeps its default value
//! (**[D]** robustness against hand-edited files; see the note on `language`
//! below).
//!
//! Example `kwiks.cfg`:
//!
//! ```text
//! # kwiks.cfg — Kwiks configuration
//! window_width=1920
//! window_height=1080
//! fullscreen=true
//! widescreen=true
//! integer_scale=false
//! fps_cap=0
//! language=en
//! music_on=true
//! sfx_on=true
//! fast_graphics=false
//! ```
//!
//! Semantics notes:
//! - Booleans: `true/false`, `1/0`, `yes/no`, `on/off` (case-insensitive).
//! - `language` only accepts `"es"` and `"en"` (normalized to lowercase);
//!   any other value is ignored and the default is kept. That is why the
//!   `to_string` → `from_str` round-trip is exact for valid configurations.
//! - `fps_cap = 0` means no FPS cap.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Game options persisted in `kwiks.cfg`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Window width (default 1280).
    pub window_width: u32,
    /// Window height (default 960).
    pub window_height: u32,
    /// Fullscreen.
    pub fullscreen: bool,
    /// Extended widescreen field (default false = faithful 640×480).
    pub widescreen: bool,
    /// Integer scaling to the window (default true).
    pub integer_scale: bool,
    /// FPS cap; 0 = uncapped (default 0).
    pub fps_cap: u32,
    /// Language: `"es"` | `"en"` (default `"en"`).
    pub language: String,
    /// Music enabled (default true).
    pub music_on: bool,
    /// Sound effects enabled (default true).
    pub sfx_on: bool,
    /// Fast graphics (F2): smaller monster halos.
    pub fast_graphics: bool,
}

impl Config {
    /// Default configuration: 1280×960 window (2× of 640×480), windowed
    /// (not fullscreen), faithful 640×480, integer scaling, no FPS cap,
    /// English and audio enabled.
    pub fn default() -> Self {
        Self {
            window_width: 1280,
            window_height: 960,
            fullscreen: false,
            widescreen: false,
            integer_scale: true,
            fps_cap: 0,
            language: "en".to_string(),
            music_on: true,
            sfx_on: true,
            fast_graphics: false,
        }
    }

    /// Loads the configuration from `path`. If the file does not exist or
    /// cannot be read, [`Config::default`] is returned (**[D]**: the game
    /// always starts; a missing file is not an error).
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_str(&text),
            Err(_) => Self::default(),
        }
    }

    /// Saves the configuration to `path`, creating intermediate directories
    /// if they do not exist.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create directory {}", parent.display()))?;
            }
        }
        std::fs::write(path, self.to_string())
            .with_context(|| format!("write {}", path.display()))
    }

    /// Parses the text of a `kwiks.cfg`. Starts from [`Config::default`] and
    /// applies each known `key=value` line. Unknown keys, lines without `=`
    /// and invalid values are ignored (the option keeps its previous value).
    pub fn from_str(text: &str) -> Self {
        let mut cfg = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "window_width" => {
                    if let Some(v) = parse_u32(value) {
                        if v >= 1 {
                            cfg.window_width = v;
                        }
                    }
                }
                "window_height" => {
                    if let Some(v) = parse_u32(value) {
                        if v >= 1 {
                            cfg.window_height = v;
                        }
                    }
                }
                "fullscreen" => {
                    if let Some(v) = parse_bool(value) {
                        cfg.fullscreen = v;
                    }
                }
                "widescreen" => {
                    if let Some(v) = parse_bool(value) {
                        cfg.widescreen = v;
                    }
                }
                "integer_scale" => {
                    if let Some(v) = parse_bool(value) {
                        cfg.integer_scale = v;
                    }
                }
                "fps_cap" => {
                    if let Some(v) = parse_u32(value) {
                        cfg.fps_cap = v;
                    }
                }
                "language" => match value.to_ascii_lowercase().as_str() {
                    "es" => cfg.language = "es".to_string(),
                    "en" => cfg.language = "en".to_string(),
                    _ => {} // unsupported language: ignored
                },
                "music_on" => {
                    if let Some(v) = parse_bool(value) {
                        cfg.music_on = v;
                    }
                }
                "sfx_on" => {
                    if let Some(v) = parse_bool(value) {
                        cfg.sfx_on = v;
                    }
                }
                "fast_graphics" => {
                    if let Some(v) = parse_bool(value) {
                        cfg.fast_graphics = v;
                    }
                }
                _ => {} // unknown key: ignored
            }
        }
        cfg
    }

    /// Serializes the configuration in `key=value` format (with a comment
    /// header). The output is always readable by [`Config::from_str`] and the
    /// round-trip is exact for valid languages (`es`/`en`).
    pub fn to_string(&self) -> String {
        format!(
            "# kwiks.cfg — Kwiks configuration (key=value format)\n\
             window_width={}\n\
             window_height={}\n\
             fullscreen={}\n\
             widescreen={}\n\
             integer_scale={}\n\
             fps_cap={}\n\
             language={}\n\
             music_on={}\n\
             sfx_on={}\n\
             fast_graphics={}\n",
            self.window_width,
            self.window_height,
            self.fullscreen,
            self.widescreen,
            self.integer_scale,
            self.fps_cap,
            self.language,
            self.music_on,
            self.sfx_on,
            self.fast_graphics,
        )
    }
}

/// Path of the configuration file: `~/.config/kwiks/kwiks.cfg`.
///
/// Uses `HOME` as the base (the directory is created when saving, see
/// [`Config::save`]; when reading it is not required to exist, see
/// [`Config::load`]). If `HOME` is not set, `.config/kwiks/kwiks.cfg` relative
/// to the current directory is used as a last resort.
pub fn config_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("kwiks").join("kwiks.cfg")
}

/// Text boolean: `true/1/yes/on` and `false/0/no/off`, case-insensitive.
/// Anything else is `None`.
fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Unsigned integer. Anything else is `None`.
fn parse_u32(value: &str) -> Option<u32> {
    value.parse::<u32>().ok()
}
