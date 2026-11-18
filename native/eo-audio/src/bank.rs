//! The sound bank: the clips the game plays, listed with their provenance in
//! `native/assets/audio-manifest.json` (written by
//! `native/tools/write-audio-manifest.mjs` from the automatic screen), read
//! from the folder `EO_AUDIO_ROOT` names (no default) and refused unless
//! each file's length and SHA-256 match the manifest.
//!
//! Streaming or in memory is decided per clip by length: clips longer than
//! `stream_over_s` (the ambience, rain and wind beds) stay on disk and are
//! read ahead by the stream thread; the rest are decoded once to 16-bit mono
//! in memory (the one-shots: steps, horns, doors, birds; and the engine loops,
//! which the mixer pitches and loops sample-accurately).

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MANIFEST_JSON: &str = include_str!("../../assets/audio-manifest.json");
pub const ROOT_ENV: &str = "EO_AUDIO_ROOT";

#[derive(Clone, Debug, Deserialize)]
pub struct ManifestClip {
    pub id: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub role: String,
    pub source: String,
    #[serde(default)]
    pub freesound_id: Option<u64>,
    #[serde(default)]
    pub uploader: Option<String>,
    pub licence: String,
    #[serde(default)]
    pub title: String,
    pub rate: u32,
    pub channels: u16,
    pub frames: u32,
    pub lufs: f32,
    pub peak_db: f32,
    /// Level correction toward the role's loudness target, dB.
    pub gain_db: f32,
    /// One-shot slices inside the clip (footsteps: one step each), frames [start, len].
    #[serde(default)]
    pub slices: Vec<[u32; 2]>,
    /// A loop region with its crossfade, frames [start, end, crossfade] (engine and squeal loops).
    #[serde(default, rename = "loop")]
    pub loop_region: Option<[u32; 3]>,
    /// Decode only the first frames (a loop taken from the start of a long recording).
    #[serde(default)]
    pub decode_frames: Option<u32>,
    /// Played through from disk (a bed); else decoded to memory.
    #[serde(default)]
    pub stream: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub stream_over_s: f32,
    pub clips: Vec<ManifestClip>,
}

impl Manifest {
    pub fn embedded() -> Result<Manifest> {
        serde_json::from_str(MANIFEST_JSON).context("audio-manifest.json")
    }
}

pub type ClipId = u32;

#[derive(Clone, Debug)]
pub struct Clip {
    pub id: String,
    pub role: String,
    pub rate: u32,
    pub frames: u32,
    /// Linear gain toward the role's loudness target.
    pub gain: f32,
    /// Decoded 16-bit mono, or None when the clip streams from `path`.
    pub data: Option<Arc<[i16]>>,
    pub path: PathBuf,
    pub channels: u16,
    pub slices: Vec<[u32; 2]>,
    pub loop_region: Option<[u32; 3]>,
    pub lufs: f32,
}

impl Clip {
    pub fn duration_s(&self) -> f32 {
        self.frames as f32 / self.rate.max(1) as f32
    }
    pub fn streamed(&self) -> bool {
        self.data.is_none()
    }
}

#[derive(Debug, Default)]
pub struct Bank {
    pub clips: Vec<Clip>,
    pub by_role: HashMap<String, Vec<ClipId>>,
    pub root: Option<PathBuf>,
    /// Decoded sample memory, bytes.
    pub memory_bytes: usize,
    pub streamed: usize,
    pub load_ms: f64,
    pub errors: Vec<String>,
}

impl Bank {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn role(&self, r: &str) -> &[ClipId] {
        self.by_role.get(r).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips.get(id as usize)
    }

    /// The root `EO_AUDIO_ROOT` names; None when it is not set or holds no copy.
    pub fn root_from_env() -> Option<PathBuf> {
        let p = std::env::var_os(ROOT_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)?;
        p.join("MANIFEST.tsv").is_file().then_some(p)
    }

    /// Load (verify, decode the short clips) every clip of `m` under `root`.
    /// A clip that fails its check is left out and named in `errors`.
    pub fn load(m: &Manifest, root: &Path) -> Bank {
        let t = std::time::Instant::now();
        let mut b = Bank { root: Some(root.to_path_buf()), ..Default::default() };
        for c in &m.clips {
            match load_clip(c, root, m.stream_over_s) {
                Ok(clip) => {
                    if let Some(d) = &clip.data {
                        b.memory_bytes += d.len() * 2;
                    } else {
                        b.streamed += 1;
                    }
                    let id = b.clips.len() as ClipId;
                    b.by_role.entry(clip.role.clone()).or_default().push(id);
                    b.clips.push(clip);
                }
                Err(e) => b.errors.push(format!("{}: {e:#}", c.id)),
            }
        }
        b.load_ms = t.elapsed().as_secs_f64() * 1e3;
        b
    }

    /// For tests and tools: a bank from clips made in memory.
    pub fn from_clips(clips: Vec<Clip>) -> Bank {
        let mut b = Bank::default();
        for c in clips {
            if let Some(d) = &c.data {
                b.memory_bytes += d.len() * 2;
            } else {
                b.streamed += 1;
            }
            let id = b.clips.len() as ClipId;
            b.by_role.entry(c.role.clone()).or_default().push(id);
            b.clips.push(c);
        }
        b
    }
}

fn safe_rel(rel: &str) -> Result<()> {
    if rel.is_empty() || rel.split('/').any(|c| c.is_empty() || c == "." || c == ".." || !c.chars().all(|ch| ch.is_ascii_alphanumeric() || "-_.".contains(ch))) {
        bail!("refused path {rel}");
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in d.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn load_clip(c: &ManifestClip, root: &Path, stream_over_s: f32) -> Result<Clip> {
    safe_rel(&c.path)?;
    let p = c.path.split('/').fold(root.to_path_buf(), |acc, part| acc.join(part));
    let bytes = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
    if bytes.len() as u64 != c.bytes {
        bail!("{} bytes, the manifest says {}", bytes.len(), c.bytes);
    }
    let sha = sha256_hex(&bytes);
    if sha != c.sha256 {
        bail!("SHA-256 {sha} differs from the manifest");
    }
    let reader = hound::WavReader::new(std::io::Cursor::new(&bytes)).map_err(|e| anyhow!("not a WAV: {e}"))?;
    let spec = reader.spec();
    let frames = reader.duration();
    if spec.sample_rate != c.rate || spec.channels != c.channels || frames != c.frames {
        bail!("header {} Hz x{} {} frames differs from the manifest", spec.sample_rate, spec.channels, frames);
    }
    // Beds longer than the threshold stream; everything else is decoded (a loop only as far as it needs).
    let streamed = c.stream && frames as f32 / spec.sample_rate as f32 > stream_over_s;
    let keep = c.decode_frames.unwrap_or(frames).min(frames);
    let data: Option<Arc<[i16]>> = if streamed { None } else { Some(decode_mono_i16(reader, keep)?.into()) };
    let frames = if streamed { frames } else { keep };
    Ok(Clip {
        id: c.id.clone(),
        role: c.role.clone(),
        rate: spec.sample_rate,
        frames,
        gain: 10f32.powf(c.gain_db / 20.0),
        data,
        path: p,
        channels: spec.channels,
        slices: c.slices.clone(),
        loop_region: c.loop_region,
        lufs: c.lufs,
    })
}

/// Decode a WAV to 16-bit mono (channels averaged; 24/32-bit and float scaled).
pub fn decode_mono_i16<R: std::io::Read>(mut reader: hound::WavReader<R>, max_frames: u32) -> Result<Vec<i16>> {
    let spec = reader.spec();
    let ch = spec.channels.max(1) as usize;
    let limit = max_frames.min(reader.duration()) as usize;
    let take = limit * ch;
    let mut out = Vec::with_capacity(limit);
    let mut acc = 0f32;
    let mut k = 0usize;
    let mut push = |v: f32, out: &mut Vec<i16>| {
        acc += v;
        k += 1;
        if k == ch {
            out.push(((acc / ch as f32) * 32767.0).round().clamp(-32768.0, 32767.0) as i16);
            acc = 0.0;
            k = 0;
        }
    };
    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => {
            for s in reader.samples::<f32>().take(take) {
                push(s?, &mut out);
            }
        }
        (hound::SampleFormat::Int, 16) => {
            for s in reader.samples::<i16>().take(take) {
                push(s? as f32 / 32768.0, &mut out);
            }
        }
        (hound::SampleFormat::Int, bits) => {
            let scale = 1.0 / (1u64 << (bits - 1)) as f32;
            for s in reader.samples::<i32>().take(take) {
                push(s? as f32 * scale, &mut out);
            }
        }
    }
    Ok(out)
}
