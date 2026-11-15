//! Tests of the audio engine (`src/audio.rs`): loading the MODs with
//! libopenmpt, decoding the WAVs with `hound` and mixing music + effects.
//!
//! Everything is tested through the SDL-free path (headless):
//! `Audio::new_headless()` and `Audio::render_into` render the final PCM
//! without opening a device, so the tests run in CI with no sound card.
//!
//! The crate is binary-only (no `lib.rs`), so the module is included directly
//! with `#[path]` to avoid altering the rest of the project.

#[path = "../src/audio.rs"]
mod audio;

use audio::{audio_asset_path, decode_wav, Audio, Music, CHANNELS, SAMPLE_RATE};

/// The 3 original music modules (ProTracker M.K.).
const MODS: &[&str] = &["titel.mod", "spiel.mod", "werbe.mod"];

/// The 6 original effects with their source sample rate.
const WAVS: &[(&str, u32)] = &[
    ("click.wav", 11025),
    ("monster.wav", 10000),
    ("gefuellt.wav", 22050),
    ("los.wav", 11025),
    ("tot.wav", 11025),
    ("leben.wav", 11111),
];

fn max_abs(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
}

/// Maximum |sample| over each half of the buffer (L,R interleaved does not
/// matter: individual samples are checked).
fn max_abs_range(buf: &[f32], from: usize, to: usize) -> f32 {
    max_abs(&buf[from..to])
}

// ---------------------------------------------------------------------------
// MOD music
// ---------------------------------------------------------------------------

/// libopenmpt accepts the 3 original `.mod` files and they produce
/// non-silent PCM.
#[test]
fn mods_load_and_produce_samples() {
    for name in MODS {
        let mut music = Music::open(&audio_asset_path(name))
            .unwrap_or_else(|e| panic!("{name}: libopenmpt does not accept the module: {e:#}"));
        assert!(music.duration_secs() > 0.0, "{name}: null duration");

        let mut buf = vec![0.0f32; 4096 * CHANNELS]; // ≈ 93 ms
        music.render_into(&mut buf);
        assert!(
            max_abs(&buf) > 1e-4,
            "{name}: the first samples are silence"
        );
    }
}

/// The music loops: rendering past the end of the module keeps producing
/// audio (seek to the last quarter second and 1 s of rendering).
#[test]
fn mods_loop() {
    for name in MODS {
        let mut music = Music::open(&audio_asset_path(name))
            .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        let dur = music.duration_secs();
        music.set_position_secs(dur - 0.25);

        // 1 s of audio: the first 0.25 s reach the end of the module and the
        // rest already comes from the loop (position 0 →).
        let mut buf = vec![0.0f32; SAMPLE_RATE as usize * CHANNELS];
        music.render_into(&mut buf);
        let wrap = (SAMPLE_RATE as usize / 4) * CHANNELS; // 0.25 s
        assert!(
            max_abs_range(&buf, 0, wrap) > 1e-3,
            "{name}: silence before the loop"
        );
        assert!(
            max_abs_range(&buf, wrap, buf.len()) > 1e-3,
            "{name}: silence after the loop (does it not repeat?)"
        );
    }
}

// ---------------------------------------------------------------------------
// WAV effects
// ---------------------------------------------------------------------------

/// The 6 `.wav` files decode: correct source sample rate, duration > 0 and
/// valid PCM resampled to 44 100 Hz stereo.
#[test]
fn wavs_decode() {
    for (name, expected_rate) in WAVS {
        let sfx = decode_wav(&audio_asset_path(name))
            .unwrap_or_else(|e| panic!("{name}: could not decode: {e:#}"));

        // Sample rate of the original file.
        assert_eq!(
            sfx.source_sample_rate, *expected_rate,
            "{name}: unexpected sample rate"
        );
        assert!(sfx.source_channels >= 1, "{name}: no channels");

        // Duration > 0 (source and after resampling).
        assert!(sfx.source_frames > 0, "{name}: no source frames");
        assert!(sfx.source_duration_secs() > 0.0, "{name}: null duration");
        assert!(sfx.duration_secs() > 0.0, "{name}: null duration");

        // The resampling preserves the duration and outputs SAMPLE_RATE stereo.
        let expected = sfx.source_duration_secs() * f64::from(SAMPLE_RATE);
        assert!(
            (sfx.frames() as f64 - expected).abs() <= 2.0,
            "{name}: {} resampled frames, ~{expected} expected",
            sfx.frames()
        );
        assert_eq!(sfx.samples.len(), sfx.frames() * CHANNELS);

        // Valid and non-silent PCM.
        for &s in &sfx.samples {
            assert!(s.is_finite() && s.abs() <= 1.0, "{name}: invalid sample {s}");
        }
        assert!(max_abs(&sfx.samples) > 1e-4, "{name}: silent effect");
    }
}

// ---------------------------------------------------------------------------
// Mixing and toggles (F8/F9)
// ---------------------------------------------------------------------------

/// F8 off mutes the music without losing the position; re-enabling it resumes
/// where it was.
#[test]
fn set_music_enabled_false_mutes_without_losing_position() {
    let mut audio = Audio::new_headless().unwrap();
    audio.play_music("titel.mod").unwrap();
    assert!(audio.has_music());
    assert!(audio.is_music_enabled());

    let mut buf = vec![0.0f32; 8192]; // 4096 frames ≈ 93 ms
    audio.render_into(&mut buf);
    assert!(max_abs(&buf) > 1e-4, "the music is not playing");
    let position = audio.music_position_secs().unwrap();
    assert!(position > 0.0);

    // F8 OFF: total silence and the position does not advance.
    audio.set_music_enabled(false);
    assert!(!audio.is_music_enabled());
    audio.render_into(&mut buf);
    assert_eq!(max_abs(&buf), 0.0, "with the music off it must mute");
    let muted_position = audio.music_position_secs().unwrap();
    assert!(
        (muted_position - position).abs() < 1e-6,
        "the position advanced while off: {position} → {muted_position}"
    );

    // F8 ON: it plays again from where it was.
    audio.set_music_enabled(true);
    audio.render_into(&mut buf);
    assert!(max_abs(&buf) > 1e-4, "the music does not resume");
    let final_position = audio.music_position_secs().unwrap();
    assert!(
        final_position > position + 0.05,
        "the music did not resume where it was: {position} → {final_position}"
    );
}

/// F9 controls the effects; several can overlap at once.
#[test]
fn effects_overlap_and_can_be_muted() {
    let mut audio = Audio::new_headless().unwrap();
    audio.play_sfx("tot.wav").unwrap();
    audio.play_sfx("leben.wav").unwrap();
    assert_eq!(audio.active_sfx(), 2, "two voices must overlap");

    let mut buf = vec![0.0f32; 4096 * CHANNELS];
    audio.render_into(&mut buf);
    assert!(max_abs(&buf) > 1e-4, "the effects are not playing");
    for &s in &buf {
        assert!(s.is_finite() && s.abs() <= 1.0, "sample out of range: {s}");
    }

    // F9 OFF: silence and the voices in progress are discarded.
    audio.set_sfx_enabled(false);
    assert!(!audio.is_sfx_enabled());
    assert_eq!(audio.active_sfx(), 0);
    audio.render_into(&mut buf);
    assert_eq!(max_abs(&buf), 0.0, "with the effects off it must mute");

    // F9 ON: they play again.
    audio.set_sfx_enabled(true);
    audio.play_sfx("tot.wav").unwrap();
    audio.render_into(&mut buf);
    assert!(max_abs(&buf) > 1e-4, "the effects do not come back");
}

/// A voice ends when its samples run out (without repeating) and leaves
/// silence behind.
#[test]
fn effects_end_without_repeating() {
    let mut audio = Audio::new_headless().unwrap();
    audio.play_sfx("click.wav").unwrap(); // ≈ 0.272 s
    assert_eq!(audio.active_sfx(), 1);

    let mut buf = vec![0.0f32; 4096 * CHANNELS];
    let mut frames_render = 0usize;
    let mut audibles = 0usize;
    while audio.active_sfx() > 0 && frames_render < SAMPLE_RATE as usize {
        audio.render_into(&mut buf);
        if max_abs(&buf) > 1e-4 {
            audibles += 1;
        }
        frames_render += buf.len() / CHANNELS;
    }
    assert_eq!(audio.active_sfx(), 0, "the voice did not end");
    assert!(audibles > 0, "the effect was silent");
    // Consumed duration ≈ 0.272 s (a bit more due to rounding to blocks).
    assert!(
        frames_render >= 11_000 && frames_render <= 13_000,
        "unexpected duration: {frames_render} frames"
    );

    // Next block: silence.
    audio.render_into(&mut buf);
    assert_eq!(max_abs(&buf), 0.0);
}

/// Music + effects are mixed with saturation: never outside [-1, 1].
#[test]
fn mixing_music_and_effects_does_not_overflow() {
    let mut audio = Audio::new_headless().unwrap();
    audio.play_music("spiel.mod").unwrap();
    for _ in 0..4 {
        audio.play_sfx("los.wav").unwrap();
    }
    assert_eq!(audio.active_sfx(), 4);

    let mut buf = vec![0.0f32; 8192 * CHANNELS];
    audio.render_into(&mut buf);
    assert!(max_abs(&buf) > 1e-4, "the mix is silent");
    for &s in &buf {
        assert!(s.is_finite() && s.abs() <= 1.0, "sample out of range: {s}");
    }
}

// ---------------------------------------------------------------------------
// Engine API
// ---------------------------------------------------------------------------

/// Nonexistent names return an error and `stop_music` mutes.
#[test]
fn nonexistent_names_fail() {
    let mut audio = Audio::new_headless().unwrap();
    assert!(audio.play_music("missing.mod").is_err());
    assert!(audio.play_sfx("missing.wav").is_err());
    assert!(!audio.has_music());

    audio.play_music("werbe.mod").unwrap();
    assert!(audio.has_music());
    audio.stop_music();
    assert!(!audio.has_music());

    let mut buf = vec![0.0f32; 1024];
    audio.render_into(&mut buf);
    assert_eq!(max_abs(&buf), 0.0);
}

/// `render_into` tolerates odd lengths (the leftover sample stays at 0).
#[test]
fn render_into_odd_length_does_not_break() {
    let mut audio = Audio::new_headless().unwrap();
    audio.play_music("titel.mod").unwrap();
    audio.play_sfx("monster.wav").unwrap();

    let mut buf = vec![1.0f32; 7];
    audio.render_into(&mut buf);
    assert_eq!(buf[6], 0.0, "the leftover sample must stay at 0");
    for &s in &buf {
        assert!(s.is_finite() && s.abs() <= 1.0);
    }
}

/// The headless engine has no SDL3 output but it does render.
#[test]
fn headless_engine_renders() {
    let mut audio = Audio::new_headless().unwrap();
    assert!(!audio.output_active());
    audio.pump(); // must do nothing and not fail
    assert!(audio.is_music_enabled() && audio.is_sfx_enabled());
}
