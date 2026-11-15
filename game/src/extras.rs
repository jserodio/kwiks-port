//! Extras of the Kwiks port: high scores, achievements and practice mode.
//!
//! Three long-term progression pieces used by `main.rs`:
//!
//! - [`HighScores`]: arcade-style high score table (3 letters, max 10
//!   entries, descending order).
//! - [`Achievements`]: unlockable achievements with stable ids (snake_case).
//! - [`Practice`]: practice mode settings (level, lives, initial fill).
//!
//! # Persistence
//!
//! Custom plain text (`key=value` format), with no new dependencies:
//!
//! ```text
//! KWIKS-HIGHSCORES-1
//! name=ABC score=12345 level=7 percent=80
//! ```
//!
//! ```text
//! KWIKS-ACHIEVEMENTS-1
//! first_zone=1
//! ```
//!
//! The first line is the header (magic + version); without it the file is
//! considered corrupt and the default values are returned **without
//! failing**. Corrupt lines are ignored one by one. Writes are atomic: it
//! writes to a `.tmp` and renames it, so a crash mid-write does not corrupt
//! the original file.
//!
//! The default paths are `~/.local/share/kwiks/highscores.txt` and
//! `~/.local/share/kwiks/achievements.txt`.

use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Game constants (see `docs/GAME_LOGIC.md`)
// ---------------------------------------------------------------------------

/// Number of levels in the original game (`FELD1..14`, 15 levels).
pub const LEVEL_COUNT: u32 = 15;
/// Index of the last level (0-based).
pub const MAX_LEVEL_INDEX: u32 = LEVEL_COUNT - 1;
/// Initial lives of a game.
pub const DEFAULT_LIVES: u32 = 3;
/// Maximum configurable lives in practice mode.
pub const MAX_LIVES: u32 = 99;
/// Fill percentage that completes a level (`pct >= 75` in the original).
pub const LEVEL_COMPLETE_PERCENT: u32 = 75;
/// Maximum starting fill percentage in practice mode (without completing a level).
pub const MAX_START_PERCENT: u32 = LEVEL_COMPLETE_PERCENT - 1;
/// Every multiple of this score earned grants an extra life.
pub const EXTRA_LIFE_SCORE_STEP: u32 = 5000;

// ---------------------------------------------------------------------------
// High scores
// ---------------------------------------------------------------------------

/// Maximum number of entries in the high score table.
pub const MAX_ENTRIES: usize = 10;

/// Magic + version of the high scores file.
const HIGHSCORES_MAGIC: &str = "KWIKS-HIGHSCORES-1";

/// One entry of the high score table.
#[derive(Clone, Debug, PartialEq)]
pub struct HighScore {
    /// Player name, exactly 3 characters (uppercase letters/digits, padded
    /// on the right with spaces).
    pub name: String,
    /// Total score.
    pub score: u32,
    /// Level reached (0-based).
    pub level: u32,
    /// Maximum fill percentage achieved.
    pub percent: u32,
}

/// Persistent high score table.
///
/// Invariant maintained by [`HighScores::insert`] and [`HighScores::from_str`]:
/// `entries` is sorted from highest to lowest score and never exceeds
/// [`MAX_ENTRIES`] entries.
#[derive(Clone, Debug, Default)]
pub struct HighScores {
    /// Entries sorted from highest to lowest, at most [`MAX_ENTRIES`].
    pub entries: Vec<HighScore>,
}

impl HighScores {
    /// Loads the table from `path`. If the file does not exist or is corrupt
    /// the default (empty) table is returned without propagating the error.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_str(&text),
            Err(_) => Self::default(),
        }
    }

    /// Saves the table to `path` atomically (`.tmp` + rename), creating
    /// intermediate directories when needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &self.to_string())
            .with_context(|| format!("could not save the high scores to {}", path.display()))
    }

    /// Parses the text format of the table. Header required; corrupt lines
    /// are ignored. If the header is not the expected one, the default table
    /// is returned.
    pub fn from_str(text: &str) -> Self {
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'));
        if !matches!(lines.next(), Some(line) if line == HIGHSCORES_MAGIC) {
            return Self::default();
        }
        let mut entries: Vec<HighScore> = lines.filter_map(parse_entry).collect();
        // Stable descending sort and truncation to the table limit.
        entries.sort_by(|a, b| b.score.cmp(&a.score));
        entries.truncate(MAX_ENTRIES);
        HighScores { entries }
    }

    /// Serializes the table in its text format (with header).
    pub fn to_string(&self) -> String {
        let mut out = String::from(HIGHSCORES_MAGIC);
        out.push('\n');
        for entry in &self.entries {
            let name = normalize_name(&entry.name);
            out.push_str(&format!(
                "name={} score={} level={} percent={}\n",
                name, entry.score, entry.level, entry.percent
            ));
        }
        out
    }

    /// Tries to insert `entry` into the table. Returns the position (0-based)
    /// if it makes it in, or `None` if it does not beat the last ranked entry.
    ///
    /// The name is normalized to 3 uppercase characters. A score of 0 never
    /// makes it in. With a full table it must beat (not tie) the last entry;
    /// entries tying with existing ones are placed behind them.
    pub fn insert(&mut self, mut entry: HighScore) -> Option<usize> {
        if !qualifies(self.min_score(), self.entries.len(), entry.score) {
            return None;
        }
        entry.name = normalize_name(&entry.name);
        // First entry with a strictly lower score: the new one goes right
        // before it (ties stay behind the ones already present).
        let pos = self
            .entries
            .iter()
            .position(|e| e.score < entry.score)
            .unwrap_or(self.entries.len());
        self.entries.insert(pos, entry);
        self.entries.truncate(MAX_ENTRIES);
        Some(pos)
    }

    /// `true` if `score` would make it into the table (see [`HighScores::insert`]).
    pub fn is_record(&self, score: u32) -> bool {
        qualifies(self.min_score(), self.entries.len(), score)
    }

    /// Lowest score in the table (with the invariant, the last one).
    fn min_score(&self) -> Option<u32> {
        self.entries.iter().map(|e| e.score).min()
    }
}

/// Does `score` make it into a table with `len` entries whose minimum is `min`?
fn qualifies(min: Option<u32>, len: usize, score: u32) -> bool {
    if score == 0 {
        return false; // a 0-point record is not a record
    }
    len < MAX_ENTRIES || min.map_or(true, |min| score > min)
}

/// Parses a line `name=... score=... level=... percent=...`.
/// Any missing or malformed field discards the whole line.
fn parse_entry(line: &str) -> Option<HighScore> {
    let (mut name, mut score, mut level, mut percent) = (None, None, None, None);
    for token in line.split_whitespace() {
        let (key, value) = token.split_once('=')?;
        match key {
            "name" => name = Some(normalize_name(value)),
            "score" => score = value.parse::<u32>().ok(),
            "level" => level = value.parse::<u32>().ok(),
            "percent" => percent = value.parse::<u32>().ok(),
            _ => {} // unknown keys: ignored
        }
    }
    Some(HighScore {
        name: name?,
        score: score?,
        level: level?,
        percent: percent?,
    })
}

/// Normalizes an arcade-style name: only ASCII letters/digits, uppercase,
/// exactly 3 characters (padded on the right with spaces). `"abc"` → `"ABC"`,
/// `"xy"` → `"XY "`, `"john"` → `"JOH"`.
fn normalize_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if out.len() >= 3 {
            break;
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        }
    }
    while out.len() < 3 {
        out.push(' ');
    }
    out
}

/// Default path of the high score table: `~/.local/share/kwiks/highscores.txt`.
pub fn highscores_path() -> PathBuf {
    data_dir().join("highscores.txt")
}

/// Game data directory: `~/.local/share/kwiks`.
fn data_dir() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_else(std::env::temp_dir);
    home.join(".local").join("share").join("kwiks")
}

// ---------------------------------------------------------------------------
// Achievements
// ---------------------------------------------------------------------------

/// Magic + version of the achievements file.
const ACHIEVEMENTS_MAGIC: &str = "KWIKS-ACHIEVEMENTS-1";

/// All the achievements of the game as `(id, description)`.
///
/// The ids are stable (snake_case) and are never reused: once chosen they do
/// not change even if the description does.
pub const ALL_ACHIEVEMENTS: &[(&str, &str)] = &[
    ("first_zone", "Conquer the first zone"),
    ("first_level", "Complete the first level"),
    ("five_levels", "Complete 5 levels"),
    ("all_levels", "Complete all 15 levels"),
    ("score_100k", "Reach 100 000 points"),
    ("fill_90", "Fill a level to 90%"),
    ("survive_60s", "Survive 60 seconds without dying"),
    ("no_death_level", "Complete a level without dying"),
    ("extra_life", "Earn an extra life"),
    ("fast_graphics", "Play with fast graphics (F2)"),
];

/// Player's unlockable achievements.
#[derive(Clone, Debug, Default)]
pub struct Achievements {
    /// Unlocked ids.
    unlocked: BTreeSet<String>,
}

impl Achievements {
    /// Loads the achievements from `path`. Missing or corrupt file → default
    /// state (none unlocked) without propagating the error.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_str(&text),
            Err(_) => Self::default(),
        }
    }

    /// Saves the achievements to `path` atomically (`.tmp` + rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &self.to_string())
            .with_context(|| format!("could not save the achievements to {}", path.display()))
    }

    /// Parses the achievements text format: header + `id=1` lines.
    /// Wrong header → default state. Corrupt lines are ignored.
    pub fn from_str(text: &str) -> Self {
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'));
        if !matches!(lines.next(), Some(line) if line == ACHIEVEMENTS_MAGIC) {
            return Self::default();
        }
        let mut unlocked = BTreeSet::new();
        for line in lines {
            if let Some((id, value)) = line.split_once('=') {
                let id = id.trim();
                if !id.is_empty() && value.trim() == "1" {
                    unlocked.insert(id.to_string());
                }
            }
        }
        Achievements { unlocked }
    }

    /// Serializes the achievements in their text format (with header).
    pub fn to_string(&self) -> String {
        let mut out = String::from(ACHIEVEMENTS_MAGIC);
        out.push('\n');
        for id in &self.unlocked {
            out.push_str(&format!("{id}=1\n"));
        }
        out
    }

    /// Unlocks an achievement. Returns `true` only if it was new.
    /// An empty id is ignored. Any id is accepted (including those in
    /// [`ALL_ACHIEVEMENTS`]); [`Achievements::progress`] only counts those.
    pub fn unlock(&mut self, id: &str) -> bool {
        if id.is_empty() {
            return false;
        }
        self.unlocked.insert(id.to_string())
    }

    /// `true` if the achievement is unlocked.
    pub fn is_unlocked(&self, id: &str) -> bool {
        self.unlocked.contains(id)
    }

    /// `(unlocked, total)` over [`ALL_ACHIEVEMENTS`].
    pub fn progress(&self) -> (usize, usize) {
        let done = ALL_ACHIEVEMENTS
            .iter()
            .filter(|(id, _)| self.unlocked.contains(*id))
            .count();
        (done, ALL_ACHIEVEMENTS.len())
    }
}

/// Default path of the achievements: `~/.local/share/kwiks/achievements.txt`.
pub fn achievements_path() -> PathBuf {
    data_dir().join("achievements.txt")
}

// ---------------------------------------------------------------------------
// Practice mode
// ---------------------------------------------------------------------------

/// Practice mode settings: choose the level and game conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct Practice {
    /// Starting level (0-based, `0..=14`).
    pub level: u32,
    /// Initial lives (`1..=99`, default [`DEFAULT_LIVES`]).
    pub lives: u32,
    /// Initial fill percentage (`0..=74`; 0 = from scratch).
    pub start_percent: u32,
}

impl Practice {
    /// Default settings for `level` ([`DEFAULT_LIVES`] lives, 0 fill),
    /// already clamped to valid values.
    pub fn new(level: u32) -> Self {
        let mut practice = Practice {
            level,
            lives: DEFAULT_LIVES,
            start_percent: 0,
        };
        practice.clamp();
        practice
    }

    /// Clamps the settings to valid limits: level `0..=14`, lives `1..=99`,
    /// initial fill `0..=74` (at 75% the level would already be complete).
    pub fn clamp(&mut self) {
        self.level = self.level.min(MAX_LEVEL_INDEX);
        self.lives = self.lives.clamp(1, MAX_LIVES);
        self.start_percent = self.start_percent.min(MAX_START_PERCENT);
    }
}

// ---------------------------------------------------------------------------
// Atomic writes
// ---------------------------------------------------------------------------

/// Writes `contents` to `path` atomically: creates intermediate directories,
/// writes to a `.tmp` next to the destination and renames it over it.
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("could not create {}", dir.display()))?;
        }
    }
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    let result = std::fs::write(&tmp, contents)
        .with_context(|| format!("could not write {}", tmp.display()))
        .and_then(|()| {
            std::fs::rename(&tmp, path)
                .with_context(|| format!("could not rename {} to {}", tmp.display(), path.display()))
        });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp); // best-effort cleanup
    }
    result
}
