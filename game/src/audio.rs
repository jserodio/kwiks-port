//! Audio engine of the Kwiks port.
//!
//! Plays the original MOD music (`titel.mod`, `spiel.mod`, `werbe.mod`) with
//! libopenmpt (crate `openmpt`) and the WAV effects (`click.wav`,
//! `monster.wav`, `gefuellt.wav`, `los.wav`, `tot.wav`, `leben.wav`) with
//! `hound`, mixing both into a stereo f32 buffer at 44 100 Hz that is pumped
//! to SDL3 with `sdl3::audio`.
//!
//! Replaces the original `AUDIOW32.DLL` wrappers (AOpenAudio /
//! ALoadModuleFile / APlayModule / ASetModuleVolume / ACloseAudio) over
//! DirectSound and the 6 DirectSound buffers for the WAVs (see
//! `docs/GAME_LOGIC.md` §1.3): F8 = music on/off, F9 = effects on/off.
//!
//! Architecture (separated so it can be tested without SDL):
//! - [`Music`] (libopenmpt) and [`decode_wav`] (hound) decode to f32 PCM.
//! - The internal mixer combines music + effects with their gains and sums
//!   with saturation; all of it is rendered by [`Audio::render_into`] into a
//!   buffer, without touching SDL (usable in CI/headless).
//! - [`Audio::pump`] only sends those blocks to the SDL3 `AudioStream` queue,
//!   without blocking the main loop.
//!
//! Mixing details:
//! - Music at 50 % ([`MUSIC_GAIN`]) and effects at 100 % ([`SFX_GAIN`]),
//!   summed with per-sample saturation to the `[-1, 1]` range.
//! - The music loops forever (`openmpt` repeat = -1).
//! - F8 off **mutes without losing the position**: while off the module does
//!   not advance, so re-enabling it resumes where it was.
//! - F9 off mutes and discards any effects currently playing.
//! - Several effects can overlap (up to [`MAX_VOICES`] voices).

// The module is used from the binary and from `tests/audio.rs` (included with
// `#[path]`); not all of its public items are used in both contexts.
#![allow(dead_code)]

use anyhow::{anyhow, Context, Result};
use openmpt::module::{Logger, Module};
use sdl3::audio::{AudioFormat, AudioSpec, AudioStreamOwner};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Sample rate of the whole engine (that of the SDL3 stream).
pub const SAMPLE_RATE: u32 = 44_100;
/// Output channels (interleaved stereo L,R).
pub const CHANNELS: usize = 2;
/// Music gain in the mix (ratio, 0.5 = 50 %).
pub const MUSIC_GAIN: f32 = 0.5;
/// Effects gain in the mix (1.0 = 100 %).
pub const SFX_GAIN: f32 = 1.0;
/// Maximum number of effects playing at the same time.
pub const MAX_VOICES: usize = 16;

/// Frames per internal render block (1024 frames = 2048 samples ≈ 23 ms).
const CHUNK_FRAMES: usize = 1024;
/// (Interleaved) samples per internal render block.
const CHUNK_SAMPLES: usize = CHUNK_FRAMES * CHANNELS;
/// Target audio queued in SDL3 (~100 ms of playback).
const TARGET_QUEUE_BYTES: i32 = (SAMPLE_RATE as usize * CHANNELS * std::mem::size_of::<f32>() / 10)
    as i32;
/// Maximum number of blocks that [`Audio::pump`] queues per call (≈ 230 ms).
const MAX_CHUNKS_PER_PUMP: usize = 10;

/// Saturating sum: `acc + add` clamped to `[-1, 1]`.
fn sat_add(acc: f32, add: f32) -> f32 {
    (acc + add).clamp(-1.0, 1.0)
}

/// Resolves an audio file name (e.g. `"titel.mod"`) inside the project's
/// `assets/audio/`.
pub fn audio_asset_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assets/audio")
        .join(name)
}

// ---------------------------------------------------------------------------
// MOD music (libopenmpt)
// ---------------------------------------------------------------------------

/// MOD module loaded with libopenmpt, renderable to stereo f32 PCM at
/// 44 100 Hz in an infinite loop.
///
/// [`Music::render_into`] fills the buffer (overwrites it) with "raw" module
/// PCM, with no gain or mixing.
pub struct Music {
    module: Module,
    /// Render block for libopenmpt (uses the `Vec`'s *capacity*).
    scratch: Vec<f32>,
    /// Rendered samples pending consumption (interleaved).
    pending: Vec<f32>,
    pending_pos: usize,
}

impl Music {
    /// Loads a `.mod` (or any other format supported by libopenmpt) from disk.
    pub fn open(path: &Path) -> Result<Music> {
        let data = std::fs::read(path)
            .with_context(|| format!("read module {}", path.display()))?;
        Self::from_bytes(data).with_context(|| format!("load module {}", path.display()))
    }

    /// Loads a module from memory. libopenmpt accepts M.K. (ProTracker) and
    /// many other tracker formats.
    pub fn from_bytes(data: Vec<u8>) -> Result<Music> {
        let mut buffer = data;
        let mut module = Module::create_from_memory(&mut buffer, Logger::None, &[])
            .map_err(|_| anyhow!("libopenmpt rejects the module"))?;
        // -1 = repeat forever (the original left the music looping).
        module.set_repeat_count(-1);
        Ok(Music {
            module,
            scratch: Vec::with_capacity(CHUNK_SAMPLES),
            pending: Vec::new(),
            pending_pos: 0,
        })
    }

    /// Fills `buf` with interleaved stereo f32 PCM from the module (no gain).
    /// If the buffer has an odd length, the last sample is ignored.
    pub fn render_into(&mut self, buf: &mut [f32]) {
        let samples = buf.len() / CHANNELS * CHANNELS;
        let out = &mut buf[..samples];
        let mut pos = 0;
        while pos < out.len() {
            if self.pending_pos < self.pending.len() {
                let n = (out.len() - pos).min(self.pending.len() - self.pending_pos);
                out[pos..pos + n]
                    .copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + n]);
                self.pending_pos += n;
                pos += n;
                if self.pending_pos >= self.pending.len() {
                    self.pending.clear();
                    self.pending_pos = 0;
                }
            } else {
                // libopenmpt writes up to `capacity()/2` frames into `scratch`.
                self.scratch.clear();
                let frames = self
                    .module
                    .read_interleaved_float_stereo(SAMPLE_RATE as i32, &mut self.scratch);
                if frames == 0 {
                    // End of the module (should not happen with repeat = -1).
                    break;
                }
                let n = frames * CHANNELS;
                // The `n` samples were just written by libopenmpt.
                debug_assert!(n <= self.scratch.capacity());
                unsafe { self.scratch.set_len(n) };
                self.pending.extend_from_slice(&self.scratch[..n]);
            }
        }
        out[pos..].fill(0.0);
    }

    /// Current playback position, in seconds.
    pub fn position_secs(&mut self) -> f64 {
        self.module.get_position_seconds()
    }

    /// Seeks to a position (seconds). Returns the actual position reached.
    pub fn set_position_secs(&mut self, secs: f64) -> f64 {
        self.module.set_position_seconds(secs)
    }

    /// Approximate duration of the module, in seconds.
    pub fn duration_secs(&mut self) -> f64 {
        self.module.get_duration_seconds()
    }
}

// ---------------------------------------------------------------------------
// WAV effects (hound)
// ---------------------------------------------------------------------------

/// WAV effect decoded into memory, already converted to interleaved stereo
/// f32 PCM at [`SAMPLE_RATE`] (the source file may be mono or at another
/// sample rate: it is resampled with linear interpolation).
pub struct DecodedSfx {
    /// Sample rate of the original file.
    pub source_sample_rate: u32,
    /// Channels of the original file.
    pub source_channels: u16,
    /// Frames of the original file.
    pub source_frames: usize,
    /// Interleaved stereo f32 PCM at [`SAMPLE_RATE`] (even length).
    pub samples: Vec<f32>,
}

impl DecodedSfx {
    /// Frames already resampled to [`SAMPLE_RATE`].
    pub fn frames(&self) -> usize {
        self.samples.len() / CHANNELS
    }

    /// Duration of the effect in seconds (after resampling).
    pub fn duration_secs(&self) -> f64 {
        self.frames() as f64 / SAMPLE_RATE as f64
    }

    /// Duration of the original file in seconds.
    pub fn source_duration_secs(&self) -> f64 {
        self.source_frames as f64 / self.source_sample_rate as f64
    }
}

/// Decodes a `.wav` (integer or float PCM) into [`DecodedSfx`].
pub fn decode_wav(path: &Path) -> Result<DecodedSfx> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("open WAV {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_rate == 0 {
        return Err(anyhow!("{}: invalid sample rate", path.display()));
    }
    let channels = spec.channels.max(1) as usize;

    // 1) Decode to f32 normalized to [-1, 1].
    let mut raw: Vec<f32> = Vec::new();
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in reader.samples::<f32>() {
                raw.push(s.with_context(|| format!("read samples from {}", path.display()))?);
            }
        }
        hound::SampleFormat::Int => {
            // hound delivers the integers unscaled (8 bits as [-128, 127],
            // 16 bits as [-32768, 32767], ...); we normalize by bit depth.
            let scale = 1.0 / ((1u64 << (spec.bits_per_sample.max(1) - 1)) as f32);
            if spec.bits_per_sample <= 16 {
                for s in reader.samples::<i16>() {
                    raw.push(
                        s.with_context(|| format!("read samples from {}", path.display()))?
                            as f32
                            * scale,
                    );
                }
            } else {
                for s in reader.samples::<i32>() {
                    raw.push(
                        s.with_context(|| format!("read samples from {}", path.display()))?
                            as f32
                            * scale,
                    );
                }
            }
        }
    }

    let source_frames = raw.len() / channels;
    if source_frames == 0 {
        return Err(anyhow!("{}: WAV has no samples", path.display()));
    }

    Ok(DecodedSfx {
        source_sample_rate: spec.sample_rate,
        source_channels: spec.channels,
        source_frames,
        samples: resample_stereo(&raw, channels, spec.sample_rate),
    })
}

/// Converts interleaved PCM of `channels` channels to interleaved stereo at
/// [`SAMPLE_RATE`] with linear interpolation. More than 2 channels is folded
/// down to mono.
fn resample_stereo(raw: &[f32], channels: usize, src_rate: u32) -> Vec<f32> {
    let src_frames = raw.len() / channels;
    let out_frames =
        ((src_frames as u64 * SAMPLE_RATE as u64 + src_rate as u64 / 2) / src_rate as u64) as usize;
    let mut out = Vec::with_capacity(out_frames * CHANNELS);
    for k in 0..out_frames {
        let pos = k as f64 * src_rate as f64 / SAMPLE_RATE as f64;
        let i0 = (pos.floor() as usize).min(src_frames - 1);
        let i1 = (i0 + 1).min(src_frames - 1);
        let frac = (pos - i0 as f64) as f32;
        let (l0, r0) = frame_stereo(raw, channels, i0);
        let (l1, r1) = frame_stereo(raw, channels, i1);
        out.push(l0 + (l1 - l0) * frac);
        out.push(r0 + (r1 - r0) * frac);
    }
    out
}

/// Returns the stereo pair (L, R) of frame `idx` of an interleaved buffer.
fn frame_stereo(raw: &[f32], channels: usize, idx: usize) -> (f32, f32) {
    match channels {
        1 => {
            let v = raw[idx];
            (v, v)
        }
        2 => (raw[idx * 2], raw[idx * 2 + 1]),
        _ => {
            let base = idx * channels;
            let sum: f32 = raw[base..base + channels].iter().sum();
            let v = sum / channels as f32;
            (v, v)
        }
    }
}

// ---------------------------------------------------------------------------
// Mixer (no SDL)
// ---------------------------------------------------------------------------

/// A playing effect voice.
struct Voice {
    data: Arc<DecodedSfx>,
    /// Current interleaved sample position.
    pos: usize,
}

/// Music + effects with their gains and the F8/F9 toggles. Does not depend on SDL.
struct Mixer {
    music: Option<Music>,
    music_enabled: bool,
    sfx_enabled: bool,
    sfx_cache: HashMap<String, Arc<DecodedSfx>>,
    voices: Vec<Voice>,
    /// Temporary buffer for the music PCM.
    music_tmp: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            music: None,
            music_enabled: true,
            sfx_enabled: true,
            sfx_cache: HashMap::new(),
            voices: Vec::new(),
            music_tmp: Vec::new(),
        }
    }
}

impl Mixer {
    /// Renders the mix (music + effects) into `buf`, interleaved stereo f32.
    /// Saturating sum to `[-1, 1]`. Odd lengths: the last sample stays at 0.
    fn render_into(&mut self, buf: &mut [f32]) {
        buf.fill(0.0);
        let samples = buf.len() / CHANNELS * CHANNELS;
        let out = &mut buf[..samples];

        // Music (F8). When off the module does not advance: the position is
        // preserved exactly until it is re-enabled.
        if self.music_enabled {
            if let Some(music) = self.music.as_mut() {
                self.music_tmp.resize(out.len(), 0.0);
                music.render_into(&mut self.music_tmp[..out.len()]);
                for (o, m) in out.iter_mut().zip(self.music_tmp.iter()) {
                    *o = sat_add(*o, *m * MUSIC_GAIN);
                }
            }
        }

        // Effects (F9), several overlapping.
        if self.sfx_enabled {
            let mut i = 0;
            while i < self.voices.len() {
                let mut voice = self.voices.remove(i);
                let data = &voice.data.samples;
                let take = (data.len() - voice.pos).min(out.len());
                for (o, s) in out.iter_mut().zip(&data[voice.pos..voice.pos + take]) {
                    *o = sat_add(*o, *s * SFX_GAIN);
                }
                voice.pos += take;
                if voice.pos < data.len() {
                    self.voices.insert(i, voice);
                    i += 1;
                }
                // If it finished, it is not put back into the list.
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SDL3 output
// ---------------------------------------------------------------------------

/// SDL3 device + stream. Owns the SDL handles so they live as long as the
/// output does.
struct SdlOut {
    _sdl: sdl3::Sdl,
    _audio: sdl3::AudioSubsystem,
    stream: AudioStreamOwner,
}

impl SdlOut {
    /// Initializes SDL3 audio and opens the default playback device with a
    /// stereo f32 stream at 44 100 Hz, and starts it.
    fn open() -> Result<SdlOut> {
        let sdl = sdl3::init().context("init SDL3")?;
        let audio = sdl.audio().context("SDL3 audio subsystem")?;
        let spec = AudioSpec::new(
            Some(SAMPLE_RATE as i32),
            Some(CHANNELS as i32),
            Some(AudioFormat::f32_sys()),
        );
        let device = audio
            .open_playback_device(&spec)
            .context("open SDL3 audio device")?;
        let stream = device
            .open_device_stream(Some(&spec))
            .context("create SDL3 audio stream")?;
        stream.resume().context("start SDL3 audio stream")?;
        Ok(SdlOut {
            _sdl: sdl,
            _audio: audio,
            stream,
        })
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Audio engine: MOD music + WAV effects mixed and sent to SDL3.
///
/// Typical use from the main loop:
/// ```ignore
/// let mut audio = Audio::new()?;
/// audio.play_music("titel.mod")?;
/// loop {
///     // ... frame logic ...
///     audio.play_sfx("click.wav")?; // when appropriate
///     audio.pump();                 // once per frame
/// }
/// ```
pub struct Audio {
    mixer: Mixer,
    out: Option<SdlOut>,
    /// Render block for `pump()`.
    chunk: Vec<f32>,
}

impl Audio {
    /// Initializes SDL3 audio and the MOD player.
    ///
    /// If SDL3 or the output device are not available (e.g. CI/headless with
    /// no sound card) a warning is printed to stderr and it continues without
    /// output ([`Audio::output_active`] returns `false`): mixing still works
    /// through [`Audio::render_into`] and [`Audio::pump`] does nothing. An
    /// error is only propagated if something unexpected fails when setting up
    /// the engine.
    pub fn new() -> anyhow::Result<Self> {
        let mut engine = Self::new_headless()?;
        match SdlOut::open() {
            Ok(out) => engine.out = Some(out),
            Err(e) => {
                eprintln!("warning: no SDL3 audio output ({e:#}); continuing silently");
            }
        }
        Ok(engine)
    }

    /// Engine without SDL3 output (for tests and CI/headless). All decoding
    /// and mixing works the same; [`Audio::render_into`] renders the resulting
    /// PCM.
    pub fn new_headless() -> anyhow::Result<Self> {
        Ok(Audio {
            mixer: Mixer::default(),
            out: None,
            chunk: vec![0.0; CHUNK_SAMPLES],
        })
    }

    /// Starts playing a MOD on a loop (file name inside `assets/audio/`, e.g.
    /// `"titel.mod"`). If there was already music, it is replaced once the new
    /// one has been loaded successfully.
    pub fn play_music(&mut self, name: &str) -> anyhow::Result<()> {
        let music = Music::open(&audio_asset_path(name))?;
        self.mixer.music = Some(music);
        Ok(())
    }

    /// Stops the music (a later playback starts from the beginning).
    pub fn stop_music(&mut self) {
        self.mixer.music = None;
    }

    /// Music on/off (F8). When turning off, it mutes without losing the
    /// position: the module does not advance while off and re-enabling it
    /// resumes where it was.
    pub fn set_music_enabled(&mut self, on: bool) {
        self.mixer.music_enabled = on;
    }

    /// Effects on/off (F9). When turning off, any effects currently playing
    /// are muted and discarded.
    pub fn set_sfx_enabled(&mut self, on: bool) {
        self.mixer.sfx_enabled = on;
        if !on {
            self.mixer.voices.clear();
        }
    }

    /// Plays a WAV effect (name inside `assets/audio/`, e.g. `"click.wav"`).
    /// Overlapping is allowed: each call opens a new voice.
    ///
    /// It is decoded into memory the first time and cached. With effects off
    /// (F9) nothing is heard, but an invalid name still returns an error.
    pub fn play_sfx(&mut self, name: &str) -> anyhow::Result<()> {
        let data = match self.mixer.sfx_cache.get(name) {
            Some(d) => Arc::clone(d),
            None => {
                let d = Arc::new(decode_wav(&audio_asset_path(name))?);
                self.mixer.sfx_cache.insert(name.to_string(), Arc::clone(&d));
                d
            }
        };
        if self.mixer.sfx_enabled {
            if self.mixer.voices.len() >= MAX_VOICES {
                // Anti-overflow: when saturated, the oldest voice is stolen.
                self.mixer.voices.remove(0);
            }
            self.mixer.voices.push(Voice { data, pos: 0 });
        }
        Ok(())
    }

    /// Pumps the audio to the SDL3 output; call once per frame.
    ///
    /// Renders blocks and queues them in the `AudioStream` until there is
    /// ~100 ms of headroom (with a per-call cap); it never blocks the main
    /// loop. With no active output it does nothing.
    pub fn pump(&mut self) {
        let Some(out) = self.out.as_mut() else {
            return;
        };
        let mut queued = out.stream.queued_bytes().unwrap_or(0).max(0);
        for _ in 0..MAX_CHUNKS_PER_PUMP {
            if queued >= TARGET_QUEUE_BYTES {
                break;
            }
            self.mixer.render_into(&mut self.chunk);
            if let Err(e) = out.stream.put_data_f32(&self.chunk) {
                eprintln!("warning: could not queue audio ({e}); output disabled");
                self.out = None;
                return;
            }
            queued += self.chunk.len() as i32 * std::mem::size_of::<f32>() as i32;
        }
    }

    /// Renders the final mix (music + effects, with gains and saturation) into
    /// `buf`, interleaved stereo f32 PCM at [`SAMPLE_RATE`]. Does not use SDL.
    pub fn render_into(&mut self, buf: &mut [f32]) {
        self.mixer.render_into(buf);
    }

    /// Music position in seconds (no music: `None`).
    pub fn music_position_secs(&mut self) -> Option<f64> {
        self.mixer.music.as_mut().map(|m| m.position_secs())
    }

    /// Is there music loaded?
    pub fn has_music(&self) -> bool {
        self.mixer.music.is_some()
    }

    /// Is the music on (F8)?
    pub fn is_music_enabled(&self) -> bool {
        self.mixer.music_enabled
    }

    /// Are the effects on (F9)?
    pub fn is_sfx_enabled(&self) -> bool {
        self.mixer.sfx_enabled
    }

    /// Number of effects currently playing (overlapping voices).
    pub fn active_sfx(&self) -> usize {
        self.mixer.voices.len()
    }

    /// Is the SDL3 output active?
    pub fn output_active(&self) -> bool {
        self.out.is_some()
    }
}
