//! Earth Online's sound system: the bank of recorded clips, a real-time
//! mixer that places them in the world, and the city's sound director.
//!
//! Library: **cpal** (output device) + **hound** (WAV decode and write) +
//! **rtrb** (wait-free rings), with the mixer written here. Why not kira:
//! the game needs things kira leaves to the caller anyway (a voice limit
//! with priorities and virtual voices, Doppler on every car, per-voice air
//! absorption, streaming beds that crossfade), and it needs the exact same
//! mixer to run three ways: on the device, on a null output that keeps real
//! time without a sound card (benches, tests), and offline sample-accurate
//! for the evidence WAVs and the unit tests. cpal is the de facto Rust audio
//! output (WASAPI here); every clip the game ships is PCM WAV, so hound is
//! the whole decoder, with no codec stack to trust. See README "Sound".
//!
//! Threads: the render thread only pushes small `Cmd`s into a ring
//! ([`Engine`]); mixing runs on the device's callback thread (or the null
//! output's), decoding of long clips on `eo-audio-stream`, bank loading
//! (verify, decode) on `eo-audio-load`.
//!
//! Outputs: [`Output::Device`] (the interactive game, default), [`Output::Null`]
//! (the mixer runs at real time and its output is discarded: benches and
//! tests, which must never make a sound), [`Output::Off`] (`--no-audio`:
//! no mixer at all), and [`Offline`] (render on demand into a buffer).

pub mod bank;
pub mod city;
pub mod mixer;
pub mod spatial;
pub mod stream;

pub use bank::{Bank, ClipId};
pub use mixer::{Bus, Cmd, Mixer, Play, Source, Spatial, Stats};
pub use spatial::{Falloff, Listener};

use anyhow::{Result, anyhow};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// Called first thing on the audio's own worker threads (bank loader, stream reader,
/// null output), never on the device's callback thread. The app sets it (eo-app: the
/// renderer's worker lowering, so they stay below the render thread).
pub static WORKER_STARTED: OnceLock<fn()> = OnceLock::new();

pub(crate) fn worker_started() {
    if let Some(f) = WORKER_STARTED.get() {
        f();
    }
}

/// Commands the ring holds between two audio callbacks.
pub const CMD_RING: usize = 8192;
/// The null and offline outputs' rate.
pub const NULL_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Device,
    Null,
    Off,
}

impl Output {
    /// `--no-audio` and `--audio-null` on the command line; else `default`.
    /// Benches pass `Null` as the default and are never given the device.
    pub fn from_args(default: Output) -> Output {
        let has = |f: &str| std::env::args().any(|a| a == f);
        if has("--no-audio") {
            Output::Off
        } else if has("--audio-null") || default == Output::Null {
            Output::Null
        } else {
            default
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Output::Device => "device",
            Output::Null => "null",
            Output::Off => "off",
        }
    }
}

/// A playing source, as the caller holds it (0: none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Handle(pub u32);

impl Handle {
    pub fn is_some(self) -> bool {
        self.0 != 0
    }
}

/// The render thread's side.
pub struct Engine {
    tx: Option<rtrb::Producer<Cmd>>,
    next: u32,
    pub stats: Arc<Stats>,
    bank: Arc<OnceLock<Arc<Bank>>>,
    pub output: Output,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
    /// The device (or null) output's rate once known.
    pub rate: Arc<std::sync::atomic::AtomicU32>,
    pub device_name: Arc<OnceLock<String>>,
    pub device_error: Arc<OnceLock<String>>,
}

impl Engine {
    /// Start an output and load `manifest` from `root` on a thread (no root:
    /// silence; the game runs the same).
    pub fn start(output: Output, root: Option<std::path::PathBuf>) -> Engine {
        let bank: Arc<OnceLock<Arc<Bank>>> = Arc::new(OnceLock::new());
        let stats = Arc::new(Stats::default());
        let stop = Arc::new(AtomicBool::new(false));
        let mut e = Engine {
            tx: None,
            next: 1,
            stats: stats.clone(),
            bank: bank.clone(),
            output,
            stop: stop.clone(),
            threads: vec![],
            rate: Arc::new(0.into()),
            device_name: Arc::new(OnceLock::new()),
            device_error: Arc::new(OnceLock::new()),
        };
        if output == Output::Off {
            return e;
        }
        let (tx, rx) = rtrb::RingBuffer::new(CMD_RING);
        e.tx = Some(tx);
        // Bank: verify and decode off the render thread.
        {
            let bank = bank.clone();
            e.threads.push(
                std::thread::Builder::new()
                    .name("eo-audio-load".into())
                    .spawn(move || {
                        worker_started();
                        let b = match (root, bank::Manifest::embedded()) {
                            (Some(r), Ok(m)) => Bank::load(&m, &r),
                            (_, Err(err)) => Bank { errors: vec![format!("{err:#}")], ..Bank::empty() },
                            (None, _) => Bank::empty(),
                        };
                        if !b.errors.is_empty() {
                            eprintln!("audio: {} clips refused, first: {}", b.errors.len(), b.errors[0]);
                        }
                        let _ = bank.set(Arc::new(b));
                    })
                    .expect("spawn audio loader"),
            );
        }
        let (ends, worker) = stream::slots(bank.clone());
        e.threads.push(worker.spawn(stop.clone()));
        match output {
            Output::Device => {
                let (rate, name, err) = (e.rate.clone(), e.device_name.clone(), e.device_error.clone());
                let st = stats.clone();
                let stop2 = stop.clone();
                e.threads.push(
                    std::thread::Builder::new()
                        .name("eo-audio-host".into())
                        .spawn(move || {
                            if let Err(x) = device_thread(rx, bank, ends, st.clone(), rate.clone(), name, stop2.clone()) {
                                let _ = err.set(format!("{x:#}"));
                                eprintln!("audio: no output device ({x:#}); not playing");
                            }
                        })
                        .expect("spawn audio host"),
                );
            }
            Output::Null => {
                let mixer = Mixer::new(NULL_RATE, rx, bank, ends, stats.clone());
                e.rate.store(NULL_RATE, Ordering::Relaxed);
                e.threads.push(null_thread(mixer, stop.clone()));
            }
            Output::Off => unreachable!(),
        }
        e
    }

    /// The engine and its mixer for rendering on demand (tests, evidence WAVs).
    pub fn offline(bank: Arc<Bank>, rate: u32) -> (Engine, Offline) {
        let cell: Arc<OnceLock<Arc<Bank>>> = Arc::new(OnceLock::new());
        let _ = cell.set(bank);
        let stats = Arc::new(Stats::default());
        let (tx, rx) = rtrb::RingBuffer::new(CMD_RING);
        let (ends, worker) = stream::slots(cell.clone());
        let mixer = Mixer::new(rate, rx, cell.clone(), ends, stats.clone());
        let e = Engine {
            tx: Some(tx),
            next: 1,
            stats,
            bank: cell,
            output: Output::Null,
            stop: Arc::new(AtomicBool::new(false)),
            threads: vec![],
            rate: Arc::new(rate.into()),
            device_name: Arc::new(OnceLock::new()),
            device_error: Arc::new(OnceLock::new()),
        };
        (e, Offline { mixer, worker })
    }

    /// The bank once loaded.
    pub fn bank(&self) -> Option<&Arc<Bank>> {
        self.bank.get()
    }

    pub fn on(&self) -> bool {
        self.tx.is_some()
    }

    pub fn send(&mut self, c: Cmd) -> bool {
        match &mut self.tx {
            Some(tx) => {
                if tx.push(c).is_err() {
                    self.stats.dropped_cmds.fetch_add(1, Ordering::Relaxed);
                    false
                } else {
                    true
                }
            }
            None => false,
        }
    }

    pub fn play(&mut self, p: Play) -> Handle {
        if self.tx.is_none() {
            return Handle(0);
        }
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        if self.send(Cmd::Play { id, p }) { Handle(id) } else { Handle(0) }
    }

    pub fn update(&mut self, h: Handle, pos: [f32; 3], vel: [f32; 3], gain: f32, pitch: f32, lowpass_hz: f32) {
        if h.is_some() {
            self.send(Cmd::Update { id: h.0, pos, vel, gain, pitch, lowpass_hz });
        }
    }

    pub fn set(&mut self, h: Handle, gain: f32, pitch: f32, pan: f32) {
        if h.is_some() {
            self.send(Cmd::Set { id: h.0, gain, pitch, pan });
        }
    }

    pub fn stop(&mut self, h: Handle, fade_s: f32) {
        if h.is_some() {
            self.send(Cmd::Stop { id: h.0, fade_s });
        }
    }

    /// Commands the mixer has not taken yet.
    pub fn pending(&self) -> usize {
        self.tx.as_ref().map(|t| CMD_RING - t.slots()).unwrap_or(0)
    }

    /// Bytes held: decoded clips, stream rings and windows, the mixer's tables, the command ring.
    pub fn memory_bytes(&self) -> (usize, usize) {
        let bank = self.bank().map(|b| b.memory_bytes).unwrap_or(0);
        let fixed = if self.on() { stream::ring_bytes() + mixer::mixer_bytes(CMD_RING) } else { 0 };
        (bank, fixed)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.tx = None;
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// The mixer and the stream worker, driven by the caller.
pub struct Offline {
    pub mixer: Mixer,
    pub worker: stream::StreamWorker,
}

impl Offline {
    /// Render `frames` stereo frames (read ahead first, as the worker thread would have).
    pub fn render(&mut self, out: &mut Vec<f32>, frames: usize) {
        let mut done = 0;
        let mut buf = vec![0f32; 2 * 256];
        while done < frames {
            let n = (frames - done).min(256);
            self.worker.pump();
            self.mixer.render(&mut buf[..2 * n], 2);
            out.extend_from_slice(&buf[..2 * n]);
            done += n;
        }
    }
}

/// The null output: the mixer at real time, its output discarded.
fn null_thread(mut mixer: Mixer, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("eo-audio-null".into())
        .spawn(move || {
            worker_started();
            let block = (NULL_RATE / 100) as usize; // 10 ms
            let mut buf = vec![0f32; block * 2];
            let period = std::time::Duration::from_secs_f64(block as f64 / NULL_RATE as f64);
            let mut next = std::time::Instant::now();
            while !stop.load(Ordering::Relaxed) {
                mixer.render(&mut buf, 2);
                next += period;
                let now = std::time::Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now; // fell behind (debugger, suspend): do not race to catch up
                }
            }
        })
        .expect("spawn audio null output")
}

fn device_thread(
    rx: rtrb::Consumer<Cmd>,
    bank: Arc<OnceLock<Arc<Bank>>>,
    ends: Vec<stream::SlotEnds>,
    stats: Arc<Stats>,
    rate_out: Arc<std::sync::atomic::AtomicU32>,
    name_out: Arc<OnceLock<String>>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| anyhow!("no default output device"))?;
    let _ = name_out.set(device.description().map(|d| format!("{d:?}")).unwrap_or_else(|_| "default".into()));
    let sup = device.default_output_config().map_err(|e| anyhow!("{e}"))?;
    let config = sup.config();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    rate_out.store(rate, Ordering::Relaxed);
    let mut mixer = Mixer::new(rate, rx, bank, ends, stats);
    let err_fn = |e: cpal::Error| eprintln!("audio: stream error: {e}");
    let stream = match sup.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_output_stream::<f32, _, _>(config, move |data: &mut [f32], _| mixer.render(data, channels), err_fn, None)
            .map_err(|e| anyhow!("{e}"))?,
        cpal::SampleFormat::I16 => {
            let mut scratch = vec![0f32; 8192 * channels];
            device
                .build_output_stream::<i16, _, _>(
                    config,
                    move |data: &mut [i16], _| {
                        for chunk in data.chunks_mut(scratch.len()) {
                            let s = &mut scratch[..chunk.len()];
                            mixer.render(s, channels);
                            for (o, v) in chunk.iter_mut().zip(s.iter()) {
                                *o = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
                            }
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| anyhow!("{e}"))?
        }
        f => return Err(anyhow!("output sample format {f:?} not handled")),
    };
    stream.play().map_err(|e| anyhow!("{e}"))?;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(stream);
    Ok(())
}
