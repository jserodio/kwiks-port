//! Tests of the extras (`src/extras.rs`): high scores, achievements and
//! practice mode.
//!
//! The crate is binary-only (no `lib.rs`), so the module is included directly
//! with `#[path]` to avoid altering the rest of the project.
//!
//! Persistence is tested with real temporary files under
//! `std::env::temp_dir()`, including tolerance for missing or corrupt files
//! and atomic writes.

#[path = "../src/extras.rs"]
mod extras;

use extras::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Counter for temporary file names unique across tests running in parallel.
static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Unique temporary path (the file/directory does not exist yet).
fn tmp_path(tag: &str) -> PathBuf {
    let unique = format!(
        "kwiks-extras-{}-{}-{}",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::SeqCst),
        tag
    );
    std::env::temp_dir().join(unique)
}

/// Best-effort cleanup of temporary paths used by a test.
fn cleanup(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir_all(path);
    }
}

fn entry(name: &str, score: u32, level: u32, percent: u32) -> HighScore {
    HighScore {
        name: name.to_string(),
        score,
        level,
        percent,
    }
}

/// Builds a table with `count` entries of score `100, 200, ...` (the highest
/// one last) and returns it sorted in descending order.
fn filled_scores(count: usize) -> HighScores {
    let mut scores = HighScores::default();
    for i in 0..count {
        let inserted = scores.insert(entry("BOT", ((i + 1) * 100) as u32, 1, 50));
        assert!(inserted.is_some(), "entry {i} must make it in");
    }
    scores
}

// ---------------------------------------------------------------------------
// High scores: structure and insertion
// ---------------------------------------------------------------------------

#[test]
fn highscores_default_is_empty() {
    let scores = HighScores::default();
    assert!(scores.entries.is_empty());
    assert_eq!(scores.to_string(), "KWIKS-HIGHSCORES-1\n");
}

#[test]
fn highscores_insert_returns_position_sorted_descending() {
    let mut scores = HighScores::default();
    assert_eq!(scores.insert(entry("ABC", 100, 1, 50)), Some(0));
    assert_eq!(scores.insert(entry("DEF", 300, 2, 60)), Some(0));
    assert_eq!(scores.insert(entry("GHI", 200, 1, 55)), Some(1));
    let order: Vec<u32> = scores.entries.iter().map(|e| e.score).collect();
    assert_eq!(order, vec![300, 200, 100]);
    // The returned positions correspond to the already-sorted table.
    assert_eq!(scores.entries[1].name, "GHI");
}

#[test]
fn highscores_limit_10_entries_drops_lowest() {
    let mut scores = filled_scores(10);
    assert_eq!(scores.entries.len(), MAX_ENTRIES);
    assert_eq!(scores.entries[0].score, 1000);
    assert_eq!(scores.entries[9].score, 100);

    // A better score makes it in and kicks out the last one.
    assert_eq!(scores.insert(entry("NEW", 550, 3, 70)), Some(5));
    assert_eq!(scores.entries.len(), MAX_ENTRIES);
    assert_eq!(scores.entries[9].score, 100 + 100); // the 100 one is gone

    // A worse one does not make it in and the table stays intact.
    let before = scores.entries.clone();
    assert_eq!(scores.insert(entry("LOW", 50, 0, 10)), None);
    assert_eq!(scores.entries, before);
}

#[test]
fn highscores_from_str_sorts_and_truncates_to_10() {
    let mut text = String::from("KWIKS-HIGHSCORES-1\n");
    for i in (1..=12).rev() {
        text.push_str(&format!("name=P{i} score={} level=1 percent=50\n", i * 100));
    }
    let scores = HighScores::from_str(&text);
    assert_eq!(scores.entries.len(), MAX_ENTRIES);
    assert_eq!(scores.entries[0].score, 1200);
    assert_eq!(scores.entries[9].score, 300); // 100 and 200 truncated
}

#[test]
fn highscores_zero_score_never_qualifies() {
    let mut scores = HighScores::default();
    assert!(!scores.is_record(0));
    assert_eq!(scores.insert(entry("NIL", 0, 0, 0)), None);
    assert!(scores.entries.is_empty());
    // ...but with a full table and minimum 0 that case cannot happen.
}

// ---------------------------------------------------------------------------
// High scores: is_record at the edges
// ---------------------------------------------------------------------------

#[test]
fn highscores_is_record_empty_table() {
    let scores = HighScores::default();
    assert!(!scores.is_record(0), "0 points is not a record");
    assert!(scores.is_record(1));
    assert!(scores.is_record(u32::MAX));
}

#[test]
fn highscores_is_record_partial_table() {
    let scores = filled_scores(3); // 300, 200, 100
    assert!(scores.is_record(1));
    assert!(scores.is_record(100), "with free slots ties also qualify");
    assert!(scores.is_record(999));
    assert!(!scores.is_record(0));
}

#[test]
fn highscores_is_record_full_table() {
    let scores = filled_scores(MAX_ENTRIES); // minimum = 100
    assert!(!scores.is_record(99));
    assert!(!scores.is_record(100), "tying with the last one is not a record");
    assert!(scores.is_record(101));
}

#[test]
fn highscores_insert_tie_on_partial_table_goes_behind() {
    let mut scores = HighScores::default();
    assert_eq!(scores.insert(entry("AAA", 100, 1, 50)), Some(0));
    // Ties with AAA: it makes it in and is placed behind.
    assert_eq!(scores.insert(entry("BBB", 100, 2, 60)), Some(1));
    assert_eq!(scores.entries[0].name, "AAA");
    assert_eq!(scores.entries[1].name, "BBB");
    // Ties above both: in front.
    assert_eq!(scores.insert(entry("CCC", 100, 3, 70)), Some(2));
    assert_eq!(scores.entries.len(), 3);
}

#[test]
fn highscores_insert_tie_on_full_table_rejected() {
    let mut scores = filled_scores(MAX_ENTRIES);
    let before = scores.entries.clone();
    assert_eq!(scores.insert(entry("TIE", 100, 5, 90)), None);
    assert_eq!(scores.entries, before);
    assert_eq!(scores.insert(entry("WIN", 101, 5, 90)), Some(9));
}

// ---------------------------------------------------------------------------
// High scores: arcade-style names
// ---------------------------------------------------------------------------

#[test]
fn highscore_names_normalized_to_3_uppercase() {
    let mut scores = HighScores::default();
    scores.insert(entry("abc", 100, 1, 50));
    scores.insert(entry("a1b2c3", 200, 1, 50));
    scores.insert(entry("xy", 300, 1, 50));
    scores.insert(entry("léa!", 400, 1, 50)); // non-ASCII characters are dropped
    let names: Vec<&str> = scores.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["LA ", "XY ", "A1B", "ABC"]);
    // All exactly 3 bytes.
    assert!(scores.entries.iter().all(|e| e.name.len() == 3));
}

#[test]
fn highscore_names_normalized_on_parse() {
    let scores = HighScores::from_str(
        "KWIKS-HIGHSCORES-1\nname=abc score=10 level=1 percent=5\n",
    );
    assert_eq!(scores.entries[0].name, "ABC");
    // Empty name: padded with spaces.
    let scores = HighScores::from_str("KWIKS-HIGHSCORES-1\nname= score=10 level=1 percent=5\n");
    assert_eq!(scores.entries[0].name, "   ");
}

// ---------------------------------------------------------------------------
// High scores: text format and persistence
// ---------------------------------------------------------------------------

#[test]
fn highscores_text_roundtrip() {
    let mut scores = HighScores::default();
    scores.insert(entry("abc", 12345, 7, 80));
    scores.insert(entry("XYZ", 6789, 3, 76));
    scores.insert(entry("P1!", 1, 0, 1));
    let text = scores.to_string();
    let parsed = HighScores::from_str(&text);
    // The already-normalized names of the table survive the round-trip.
    assert_eq!(parsed.entries, scores.entries);
    // And the text is a fixed point: serializing again gives the same.
    assert_eq!(parsed.to_string(), text);
}

#[test]
fn highscores_roundtrip_empty_table() {
    let scores = HighScores::default();
    let parsed = HighScores::from_str(&scores.to_string());
    assert!(parsed.entries.is_empty());
    assert_eq!(parsed.to_string(), scores.to_string());
}

#[test]
fn highscores_from_str_skips_corrupt_lines() {
    let text = "\
KWIKS-HIGHSCORES-1
name=OK score=500 level=2 percent=75
this is not a valid line
name=NO_SCORE level=1 percent=1
name=BAD score=nan level=1 percent=1
name=EXTRA score=1 level=1
name=OK2 score=100 level=1 percent=50 unknown_key=x
";
    let scores = HighScores::from_str(text);
    let names: Vec<&str> = scores.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["OK ", "OK2"]); // descending: 500 before 100
    assert_eq!(scores.entries[0].score, 500);
    assert_eq!(scores.entries[1].score, 100);
}

#[test]
fn highscores_from_str_corrupt_header_returns_default() {
    for text in [
        "",
        "\n\n",
        "KWIKS-HIGHSCORES-2\nname=AAA score=1 level=1 percent=1\n",
        "garbage without header\nname=AAA score=1 level=1 percent=1\n",
        "KWIKS-ACHIEVEMENTS-1\n", // header of another file
    ] {
        let scores = HighScores::from_str(text);
        assert!(scores.entries.is_empty(), "it must return defaults: {text:?}");
    }
    // Comments and blank lines before the header are tolerated.
    let scores = HighScores::from_str(
        "# comment\n\nKWIKS-HIGHSCORES-1\nname=AAA score=7 level=1 percent=1\n",
    );
    assert_eq!(scores.entries.len(), 1);
}

#[test]
fn highscores_load_missing_or_corrupt_file_returns_default() {
    let missing = tmp_path("missing.txt");
    assert!(HighScores::load(&missing).entries.is_empty());

    let corrupt = tmp_path("corrupt.txt");
    std::fs::write(&corrupt, "total corruption\n\x00\x01").unwrap();
    assert!(HighScores::load(&corrupt).entries.is_empty());
    cleanup(&[missing, corrupt]);
}

#[test]
fn highscores_save_load_roundtrip_file() {
    let path = tmp_path("save-load.txt");
    let mut scores = HighScores::default();
    scores.insert(entry("abc", 777, 4, 88));
    scores.save(&path).unwrap();

    let loaded = HighScores::load(&path);
    assert_eq!(loaded.entries, scores.entries);

    // Overwriting with fewer entries also works (rename over the destination).
    let empty = HighScores::default();
    empty.save(&path).unwrap();
    assert!(HighScores::load(&path).entries.is_empty());
    cleanup(&[path]);
}

#[test]
fn highscores_save_is_atomic_and_creates_dirs() {
    let dir = tmp_path("nested-dir");
    let path = dir.join("sub").join("highscores.txt");
    let mut scores = HighScores::default();
    scores.insert(entry("TMP", 42, 1, 10));
    scores.save(&path).unwrap();

    assert!(path.is_file(), "the destination must exist");
    // No leftover .tmp after the atomic write.
    let tmp_leftover = dir.join("sub").join("highscores.txt.tmp");
    assert!(!tmp_leftover.exists(), "the .tmp must not be left behind");
    assert_eq!(HighScores::load(&path).entries, scores.entries);
    cleanup(&[dir]);
}

#[test]
fn highscores_path_layout() {
    let path = highscores_path();
    assert!(path.ends_with(".local/share/kwiks/highscores.txt"));
    let achievements = achievements_path();
    assert!(achievements.ends_with(".local/share/kwiks/achievements.txt"));
    // Both share the directory.
    assert_eq!(path.parent(), achievements.parent());
}

// ---------------------------------------------------------------------------
// Achievements
// ---------------------------------------------------------------------------

#[test]
fn achievements_catalog_is_valid() {
    assert_eq!(ALL_ACHIEVEMENTS.len(), 10);
    let mut seen = std::collections::BTreeSet::new();
    for (id, description) in ALL_ACHIEVEMENTS {
        assert!(!id.is_empty(), "empty id");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && !id.starts_with('_')
                && !id.ends_with('_')
                && !id.contains("__"),
            "id is not snake_case: {id}"
        );
        assert!(seen.insert(*id), "duplicate id: {id}");
        assert!(!description.is_empty(), "empty description for {id}");
    }
}

#[test]
fn achievements_start_empty_with_progress() {
    let achievements = Achievements::default();
    assert_eq!(achievements.progress(), (0, ALL_ACHIEVEMENTS.len()));
    assert!(!achievements.is_unlocked("first_zone"));
}

#[test]
fn achievements_unlock_only_once() {
    let mut achievements = Achievements::default();
    assert!(achievements.unlock("first_zone"), "the first unlock is new");
    assert!(!achievements.unlock("first_zone"), "it was already unlocked");
    assert!(!achievements.unlock("first_zone"));
    assert!(achievements.is_unlocked("first_zone"));
    assert!(!achievements.unlock(""), "an empty id is ignored");
    assert_eq!(achievements.progress(), (1, ALL_ACHIEVEMENTS.len()));
}

#[test]
fn achievements_progress_counts_known_ids() {
    let mut achievements = Achievements::default();
    for id in ["first_zone", "five_levels", "fast_graphics"] {
        assert!(achievements.unlock(id));
    }
    // An id outside the catalog does not count toward progress.
    assert!(achievements.unlock("made_up_achievement"));
    assert!(achievements.is_unlocked("made_up_achievement"));
    assert_eq!(achievements.progress(), (3, ALL_ACHIEVEMENTS.len()));

    // Unlocking them all gives the total.
    let mut full = Achievements::default();
    for (id, _) in ALL_ACHIEVEMENTS {
        assert!(full.unlock(id));
    }
    assert_eq!(full.progress(), (ALL_ACHIEVEMENTS.len(), ALL_ACHIEVEMENTS.len()));
    // None of those ids is "new" a second time.
    for (id, _) in ALL_ACHIEVEMENTS {
        assert!(!full.unlock(id));
    }
}

#[test]
fn achievements_text_roundtrip() {
    let mut achievements = Achievements::default();
    achievements.unlock("first_zone");
    achievements.unlock("score_100k");
    let text = achievements.to_string();
    let parsed = Achievements::from_str(&text);
    assert_eq!(parsed.to_string(), text);
    assert!(parsed.is_unlocked("first_zone"));
    assert!(parsed.is_unlocked("score_100k"));
    assert!(!parsed.is_unlocked("all_levels"));
    assert_eq!(parsed.progress(), (2, ALL_ACHIEVEMENTS.len()));
}

#[test]
fn achievements_from_str_skips_corrupt_lines() {
    let text = "\
KWIKS-ACHIEVEMENTS-1
first_zone=1
garbage line without equals
five_levels=1
fast_graphics=0
=1
extra_life=true
no_death_level=1
";
    let achievements = Achievements::from_str(text);
    assert!(achievements.is_unlocked("first_zone"));
    assert!(achievements.is_unlocked("five_levels"));
    assert!(achievements.is_unlocked("no_death_level"));
    assert!(!achievements.is_unlocked("fast_graphics"), "=0 does not unlock");
    assert!(!achievements.is_unlocked("extra_life"), "only =1 unlocks");
    assert!(!achievements.is_unlocked(""), "empty id discarded");
    assert_eq!(achievements.progress(), (3, ALL_ACHIEVEMENTS.len()));
}

#[test]
fn achievements_from_str_corrupt_header_returns_default() {
    for text in [
        "",
        "garbage\nfirst_zone=1\n",
        "KWIKS-ACHIEVEMENTS-2\nfirst_zone=1\n",
        "KWIKS-HIGHSCORES-1\nname=AAA score=1 level=1 percent=1\n",
    ] {
        let achievements = Achievements::from_str(text);
        assert_eq!(
            achievements.progress(),
            (0, ALL_ACHIEVEMENTS.len()),
            "it must return defaults: {text:?}"
        );
    }
}

#[test]
fn achievements_load_missing_or_corrupt_file_returns_default() {
    let missing = tmp_path("missing-ach.txt");
    let fresh = Achievements::load(&missing);
    assert_eq!(fresh.progress(), (0, ALL_ACHIEVEMENTS.len()));

    let corrupt = tmp_path("corrupt-ach.txt");
    std::fs::write(&corrupt, "\u{feff}I am not an achievements file").unwrap();
    assert_eq!(
        Achievements::load(&corrupt).progress(),
        (0, ALL_ACHIEVEMENTS.len())
    );
    cleanup(&[missing, corrupt]);
}

#[test]
fn achievements_save_load_roundtrip_file() {
    let dir = tmp_path("ach-dir");
    let path = dir.join("achievements.txt"); // also tests create_dir_all
    let mut achievements = Achievements::default();
    for (id, _) in ALL_ACHIEVEMENTS.iter().take(4) {
        assert!(achievements.unlock(id));
    }
    achievements.save(&path).unwrap();

    let loaded = Achievements::load(&path);
    assert_eq!(loaded.progress(), (4, ALL_ACHIEVEMENTS.len()));
    assert_eq!(loaded.to_string(), achievements.to_string());

    // No leftover .tmp (atomic write).
    assert!(!dir.join("achievements.txt.tmp").exists());
    cleanup(&[dir]);
}

// ---------------------------------------------------------------------------
// Practice mode
// ---------------------------------------------------------------------------

#[test]
fn practice_new_defaults() {
    let practice = Practice::new(0);
    assert_eq!(
        practice,
        Practice {
            level: 0,
            lives: DEFAULT_LIVES,
            start_percent: 0,
        }
    );
    assert_eq!(DEFAULT_LIVES, 3);
    let last = Practice::new(MAX_LEVEL_INDEX);
    assert_eq!(last.level, 14);
}

#[test]
fn practice_new_clamps_level() {
    assert_eq!(Practice::new(LEVEL_COUNT - 1).level, 14);
    assert_eq!(Practice::new(LEVEL_COUNT).level, MAX_LEVEL_INDEX);
    assert_eq!(Practice::new(u32::MAX).level, MAX_LEVEL_INDEX);
}

#[test]
fn practice_clamp_limits() {
    let mut practice = Practice::new(0);
    practice.level = 99;
    practice.lives = 0;
    practice.start_percent = 100;
    practice.clamp();
    assert_eq!(practice.level, MAX_LEVEL_INDEX);
    assert_eq!(practice.lives, 1);
    assert_eq!(practice.start_percent, MAX_START_PERCENT);

    practice.level = u32::MAX;
    practice.lives = u32::MAX;
    practice.start_percent = u32::MAX;
    practice.clamp();
    assert_eq!(practice.level, MAX_LEVEL_INDEX);
    assert_eq!(practice.lives, MAX_LIVES);
    assert_eq!(practice.start_percent, MAX_START_PERCENT);
}

#[test]
fn practice_clamp_keeps_valid_values() {
    let mut practice = Practice {
        level: 7,
        lives: 3,
        start_percent: 74,
    };
    practice.clamp();
    assert_eq!(
        practice,
        Practice {
            level: 7,
            lives: 3,
            start_percent: 74,
        }
    );
    // Exact lower and upper limits.
    let mut practice = Practice {
        level: MAX_LEVEL_INDEX,
        lives: MAX_LIVES,
        start_percent: MAX_START_PERCENT,
    };
    practice.clamp();
    assert_eq!(practice.level, MAX_LEVEL_INDEX);
    assert_eq!(practice.lives, MAX_LIVES);
    assert_eq!(practice.start_percent, MAX_START_PERCENT);
    // With a 75 % start the level would already be complete.
    assert_eq!(MAX_START_PERCENT, LEVEL_COMPLETE_PERCENT - 1);
    assert_eq!(LEVEL_COUNT, 15);
}

#[test]
fn game_constants_match_original() {
    // Rules of the original (docs/GAME_LOGIC.md): extra life every 5000
    // points, level complete at 75 %, 15 levels, 3 initial lives.
    assert_eq!(EXTRA_LIFE_SCORE_STEP, 5000);
    assert_eq!(LEVEL_COMPLETE_PERCENT, 75);
    assert_eq!(LEVEL_COUNT, 15);
    assert_eq!(DEFAULT_LIVES, 3);
}
