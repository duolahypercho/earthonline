//! The real-time mixer. It runs on the audio thread (the device callback, the
//! null output's thread, or the offline renderer) and never allocates, locks
//! or blocks there: commands arrive through a wait-free ring, stream chunks
//! through one ring per stream slot, and its counters leave through atomics.
//!
//! Model: up to `MAX_EMITTERS` playing sources ("emitters"); every 256 frames
//! the `MAX_VOICES` most audible (level estimate x 4^priority, with hysteresis)
//! are mixed, the rest run virtual (their playheads advance, so a loop comes
//! back in phase and a one-shot ends on time). Each mixed voice: 4-point
//! Hermite resampling (pitch x Doppler x source rate / output rate), a
//! one-pole high cut (air, rear, and the caller's muffle), equal-power pan,
//! gains ramped every 64 frames. Buses scale their voices; the master runs a
//! peak limiter at -0.9 dBFS.

use crate::bank::{Bank, ClipId};
use crate::spatial::{self, Falloff, Listener};
use crate::stream::{self, Chunk, Request, SlotEnds};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

pub const MAX_EMITTERS: usize = 384;
pub const MAX_VOICES: usize = 64;
/// Frames between parameter steps (gains, rate and filter ramp across them).
pub const STEP: usize = 64;
/// Steps between voice selections.
const SELECT_EVERY: u32 = 4;
const MAP: usize = 1024;
const NO_SLOT: u16 = u16::MAX;
/// Stream read window per slot (frames kept around the playhead).
const WIN: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Bus {
    Ambience = 0,
    Traffic = 1,
    Footsteps = 2,
    Effects = 3,
    Ui = 4,
}
pub const BUSES: usize = 5;
pub const BUS_NAMES: [&str; BUSES] = ["ambience", "traffic", "footsteps", "effects", "ui"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// A clip range: frames `[start, start + len)` (`len` 0: to the end). With
    /// `looping`, the clip's own loop region (else the whole range with a 60 ms
    /// crossfade) repeats until stopped.
    Clip { clip: ClipId, start: u32, len: u32, looping: bool },
    /// Broadband noise (tyre and road roar), shaped by the voice's high cut.
    Noise { seed: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spatial {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub falloff: Falloff,
    /// Doppler scale (1 physical, 0 none).
    pub doppler: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    pub source: Source,
    pub bus: Bus,
    /// 0 (first to go virtual) ..= 7 (never, while it is audible).
    pub priority: u8,
    pub gain: f32,
    pub pitch: f32,
    /// 2D sources: -1 left .. 1 right.
    pub pan: f32,
    pub spatial: Option<Spatial>,
    pub fade_in_s: f32,
    /// High cut, Hz (20 kHz: open).
    pub lowpass_hz: f32,
}

impl Play {
    pub fn clip(clip: ClipId, bus: Bus) -> Self {
        Play { source: Source::Clip { clip, start: 0, len: 0, looping: false }, bus, priority: 3, gain: 1.0, pitch: 1.0, pan: 0.0, spatial: None, fade_in_s: 0.0, lowpass_hz: 20_000.0 }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Cmd {
    Play { id: u32, p: Play },
    /// A 3D source moved (and its level, pitch and cut).
    Update { id: u32, pos: [f32; 3], vel: [f32; 3], gain: f32, pitch: f32, lowpass_hz: f32 },
    /// A 2D source's level, pitch and pan.
    Set { id: u32, gain: f32, pitch: f32, pan: f32 },
    Stop { id: u32, fade_s: f32 },
    Listener(Listener),
    Bus { bus: Bus, gain: f32 },
    Master(f32),
    StopAll { fade_s: f32 },
}

/// What the audio thread reports (read from any thread).
#[derive(Default, Debug)]
pub struct Stats {
    pub mixed: AtomicU32,
    pub active: AtomicU32,
    pub virtual_: AtomicU32,
    pub max_mixed: AtomicU32,
    pub max_active: AtomicU32,
    pub frames: AtomicU64,
    /// Audio-thread time spent mixing, ns, summed.
    pub render_ns: AtomicU64,
    pub underruns: AtomicU32,
    /// Frames the limiter held more than 1 dB down.
    pub limited_frames: AtomicU64,
    /// Peak output (x 1e6) since the last read of `take_peak`.
    pub peak_micro: AtomicU32,
    pub dropped_cmds: AtomicU32,
    pub refused_plays: AtomicU32,
    pub streams: AtomicU32,
    pub rate: AtomicU32,
}

impl Stats {
    pub fn take_peak(&self) -> f32 {
        self.peak_micro.swap(0, Ordering::Relaxed) as f32 * 1e-6
    }
    /// Mixing cost as a share of real time (0.01 = 1 % of one core).
    pub fn load(&self) -> f64 {
        let f = self.frames.load(Ordering::Relaxed) as f64;
        let r = self.rate.load(Ordering::Relaxed).max(1) as f64;
        if f == 0.0 { 0.0 } else { self.render_ns.load(Ordering::Relaxed) as f64 * 1e-9 / (f / r) }
    }
}

struct Window {
    buf: Vec<f32>,
    /// Source frame of `buf[0]`.
    start: u64,
    generation: u32,
    ended: bool,
}

#[derive(Clone, Copy)]
struct Em {
    id: u32,
    p: Play,
    // targets
    pos: [f32; 3],
    vel: [f32; 3],
    gain_t: f32,
    pitch_t: f32,
    pan_t: f32,
    lp_t: f32,
    // playback
    head: f64,
    lo: u32,
    hi: u32,
    looping: Option<[u32; 3]>,
    clip_rate: f32,
    stream: u16,
    // envelopes
    fade: f32,
    fade_step: f32,
    stopping: bool,
    vgain: f32,
    chosen: bool,
    mixed: bool,
    // smoothed
    gl: f32,
    gr: f32,
    rate: f32,
    lp_a: f32,
    z1: f32,
    z2: f32,
    rng: u32,
    est: f32,
    started: bool,
    /// Not yet chosen once: a new voice starts at full voice gain (its own fade-in shapes it).
    fresh: bool,
    // this step's terms (left, right, rate, cut)
    t: [f32; 4],
}

pub struct Mixer {
    rate: f32,
    cmds: rtrb::Consumer<Cmd>,
    bank_cell: Arc<OnceLock<Arc<Bank>>>,
    bank: Option<Arc<Bank>>,
    pub stats: Arc<Stats>,
    ems: Vec<Em>,
    map: Vec<u16>,
    listener: Listener,
    bus_t: [f32; BUSES],
    bus_g: [f32; BUSES],
    master_t: f32,
    master_g: f32,
    lim_g: f32,
    lim_rel: f32,
    acc: Vec<f32>,
    step_count: u32,
    streams: Vec<SlotEnds>,
    stream_owner: Vec<u32>,
    stream_gen: Vec<u32>,
    windows: Vec<Window>,
    peak: f32,
    /// Select at the next step (a new source, or a stream's first data).
    reselect: bool,
}

#[inline]
fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

#[inline]
fn at(d: &[i16], i: i64, lo: i64, hi: i64) -> f32 {
    if i < lo || i >= hi { 0.0 } else { d[i as usize] as f32 * (1.0 / 32768.0) }
}

#[inline]
fn sample_mem(d: &[i16], p: f64, lo: i64, hi: i64) -> f32 {
    let i = p.floor() as i64;
    let t = (p - i as f64) as f32;
    hermite(at(d, i - 1, lo, hi), at(d, i, lo, hi), at(d, i + 1, lo, hi), at(d, i + 2, lo, hi), t)
}

impl Mixer {
    pub fn new(rate: u32, cmds: rtrb::Consumer<Cmd>, bank: Arc<OnceLock<Arc<Bank>>>, streams: Vec<SlotEnds>, stats: Arc<Stats>) -> Self {
        stats.rate.store(rate, Ordering::Relaxed);
        let empty = Em {
            id: 0,
            p: Play::clip(0, Bus::Effects),
            pos: [0.0; 3],
            vel: [0.0; 3],
            gain_t: 0.0,
            pitch_t: 1.0,
            pan_t: 0.0,
            lp_t: 20_000.0,
            head: 0.0,
            lo: 0,
            hi: 0,
            looping: None,
            clip_rate: 1.0,
            stream: NO_SLOT,
            fade: 0.0,
            fade_step: 0.0,
            stopping: false,
            vgain: 0.0,
            chosen: false,
            mixed: false,
            gl: 0.0,
            gr: 0.0,
            rate: 1.0,
            lp_a: 1.0,
            z1: 0.0,
            z2: 0.0,
            rng: 1,
            est: 0.0,
            started: false,
            fresh: true,
            t: [0.0, 0.0, 1.0, 20_000.0],
        };
        let n = streams.len();
        Mixer {
            rate: rate as f32,
            cmds,
            bank_cell: bank,
            bank: None,
            stats,
            ems: vec![empty; MAX_EMITTERS],
            map: vec![NO_SLOT; MAP],
            listener: Listener::default(),
            bus_t: [1.0; BUSES],
            bus_g: [1.0; BUSES],
            master_t: 1.0,
            master_g: 1.0,
            lim_g: 1.0,
            lim_rel: 1.0 - (-1.0 / (0.08 * rate as f32)).exp(),
            acc: vec![0.0; STEP * 2],
            step_count: 0,
            streams,
            stream_owner: vec![0; n],
            stream_gen: vec![0; n],
            windows: (0..n).map(|_| Window { buf: Vec::with_capacity(WIN + stream::CHUNK), start: 0, generation: 0, ended: false }).collect(),
            peak: 0.0,
            reselect: true,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate as u32
    }

    fn find(&self, id: u32) -> Option<usize> {
        if id == 0 {
            return None;
        }
        let s = self.map[id as usize % MAP];
        if s != NO_SLOT && self.ems[s as usize].id == id {
            return Some(s as usize);
        }
        self.ems.iter().position(|e| e.id == id)
    }

    fn free(&mut self, k: usize) {
        let id = self.ems[k].id;
        if self.map[id as usize % MAP] == k as u16 {
            self.map[id as usize % MAP] = NO_SLOT;
        }
        let s = self.ems[k].stream;
        if s != NO_SLOT {
            let s = s as usize;
            self.stream_owner[s] = 0;
            self.stream_gen[s] = self.stream_gen[s].wrapping_add(1);
            let _ = self.streams[s].req.push(Request { generation: self.stream_gen[s], clip: stream::STOP_CLIP, start: 0, end: 0 });
        }
        self.ems[k].id = 0;
        self.ems[k].stream = NO_SLOT;
        self.ems[k].mixed = false;
    }

    fn start(&mut self, id: u32, p: Play) {
        let Some(bank) = self.bank.clone() else {
            self.stats.refused_plays.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let Some(k) = self.ems.iter().position(|e| e.id == 0) else {
            self.stats.refused_plays.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let mut e = self.ems[k];
        e.id = id;
        e.p = p;
        e.gain_t = p.gain;
        e.pitch_t = p.pitch;
        e.pan_t = p.pan;
        e.lp_t = p.lowpass_hz;
        if let Some(s) = p.spatial {
            e.pos = s.pos;
            e.vel = s.vel;
        }
        e.fade = if p.fade_in_s > 0.0 { 0.0 } else { 1.0 };
        e.fade_step = if p.fade_in_s > 0.0 { 1.0 / (p.fade_in_s * self.rate) } else { 0.0 };
        e.stopping = false;
        e.vgain = 0.0;
        e.chosen = false;
        e.mixed = false;
        e.gl = 0.0;
        e.gr = 0.0;
        e.z1 = 0.0;
        e.z2 = 0.0;
        e.stream = NO_SLOT;
        e.looping = None;
        e.started = false;
        e.fresh = true;
        e.est = 0.0;
        match p.source {
            Source::Clip { clip, start, len, looping } => {
                let Some(c) = bank.clip(clip) else {
                    self.stats.refused_plays.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let lo = start.min(c.frames);
                let hi = if len == 0 { c.frames } else { (start + len).min(c.frames) };
                if hi <= lo + 4 {
                    self.stats.refused_plays.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                e.lo = lo;
                e.hi = hi;
                e.head = lo as f64;
                e.clip_rate = c.rate as f32 / self.rate;
                e.rate = e.clip_rate * p.pitch;
                if looping {
                    let xf = (0.06 * c.rate as f32) as u32;
                    e.looping = Some(c.loop_region.unwrap_or([lo + xf.min((hi - lo) / 4), hi, xf.min((hi - lo) / 4)]));
                }
                if c.streamed() {
                    let Some(s) = self.stream_owner.iter().position(|&o| o == 0) else {
                        self.stats.refused_plays.fetch_add(1, Ordering::Relaxed);
                        return;
                    };
                    self.stream_owner[s] = id;
                    self.stream_gen[s] = self.stream_gen[s].wrapping_add(1);
                    let g = self.stream_gen[s];
                    if self.streams[s].req.push(Request { generation: g, clip, start: lo, end: hi }).is_err() {
                        self.stream_owner[s] = 0;
                        self.stats.refused_plays.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    let w = &mut self.windows[s];
                    w.buf.clear();
                    w.start = lo as u64;
                    w.generation = g;
                    w.ended = false;
                    e.stream = s as u16;
                    e.looping = None; // a streamed bed is played through; the director crossfades the next
                }
            }
            Source::Noise { seed } => {
                e.lo = 0;
                e.hi = u32::MAX;
                e.head = 0.0;
                e.clip_rate = 1.0;
                e.rate = 1.0;
                e.rng = seed | 1;
            }
        }
        self.ems[k] = e;
        self.map[id as usize % MAP] = k as u16;
        self.reselect = true;
    }

    fn apply(&mut self, c: Cmd) {
        match c {
            Cmd::Play { id, p } => self.start(id, p),
            Cmd::Update { id, pos, vel, gain, pitch, lowpass_hz } => {
                if let Some(k) = self.find(id) {
                    let e = &mut self.ems[k];
                    e.pos = pos;
                    e.vel = vel;
                    e.gain_t = gain;
                    e.pitch_t = pitch;
                    e.lp_t = lowpass_hz;
                }
            }
            Cmd::Set { id, gain, pitch, pan } => {
                if let Some(k) = self.find(id) {
                    let e = &mut self.ems[k];
                    e.gain_t = gain;
                    e.pitch_t = pitch;
                    e.pan_t = pan;
                }
            }
            Cmd::Stop { id, fade_s } => {
                if let Some(k) = self.find(id) {
                    self.stop_slot(k, fade_s);
                }
            }
            Cmd::Listener(l) => self.listener = l,
            Cmd::Bus { bus, gain } => self.bus_t[bus as usize] = gain.max(0.0),
            Cmd::Master(g) => self.master_t = g.max(0.0),
            Cmd::StopAll { fade_s } => {
                for k in 0..self.ems.len() {
                    if self.ems[k].id != 0 {
                        self.stop_slot(k, fade_s);
                    }
                }
            }
        }
    }

    fn stop_slot(&mut self, k: usize, fade_s: f32) {
        let r = self.rate;
        let e = &mut self.ems[k];
        if fade_s <= 0.0 || !e.mixed {
            self.free(k);
        } else {
            e.stopping = true;
            e.fade_step = -1.0 / (fade_s * r);
        }
    }

    /// Spatial terms of one emitter for this step: (left, right) gain before
    /// the voice fade, the playback rate, the high-cut coefficient.
    fn terms(&self, e: &Em, bank_gain: f32) -> (f32, f32, f32, f32) {
        let bus = self.bus_g[e.p.bus as usize];
        let rate_scale = e.clip_rate * e.pitch_t.clamp(0.1, 4.0);
        match e.p.spatial {
            Some(s) => {
                let (d, x, z) = spatial::locate(&self.listener, e.pos, s.falloff.ref_m.max(0.5));
                let g = spatial::attenuation(d, &s.falloff) * e.gain_t * bus * bank_gain;
                let (l, r) = spatial::pan_gains(x);
                let dop = if s.doppler > 0.0 { spatial::doppler(&self.listener, e.pos, e.vel, s.doppler) } else { 1.0 };
                let cut = spatial::air_cutoff_hz(d, z).min(e.lp_t);
                (g * l, g * r, rate_scale * dop, cut)
            }
            None => {
                let g = e.gain_t * bus * bank_gain;
                let (l, r) = spatial::pan_gains(e.pan_t);
                (g * l, g * r, rate_scale, e.lp_t)
            }
        }
    }

    fn select(&mut self) {
        // Score = level estimate x 4^priority; current voices keep a 1.5x hold.
        let mut scores: [(f32, u16); MAX_EMITTERS] = [(0.0, 0); MAX_EMITTERS];
        let mut n = 0;
        for (k, e) in self.ems.iter().enumerate() {
            if e.id == 0 {
                continue;
            }
            let mut s = e.est * 4f32.powi(e.p.priority.min(7) as i32);
            if e.chosen {
                s *= 1.5;
            }
            if e.est > 1e-5 && (e.stream == NO_SLOT || self.windows[e.stream as usize].buf.len() > 0 || e.started) {
                scores[n] = (s, k as u16);
                n += 1;
            }
        }
        let list = &mut scores[..n];
        if n > MAX_VOICES {
            list.select_nth_unstable_by(MAX_VOICES - 1, |a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        }
        for e in self.ems.iter_mut() {
            e.chosen = false;
        }
        for &(_, k) in list.iter().take(MAX_VOICES) {
            self.ems[k as usize].chosen = true;
        }
    }

    /// Fill `out` (interleaved, `channels` per frame; the first two get the
    /// stereo mix, any others silence).
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let t0 = std::time::Instant::now();
        let channels = channels.max(1);
        if self.bank.is_none() {
            if let Some(b) = self.bank_cell.get() {
                self.bank = Some(b.clone());
            }
        }
        let mut applied = 0;
        while let Ok(c) = self.cmds.pop() {
            self.apply(c);
            applied += 1;
            if applied > 8192 {
                break;
            }
        }
        let frames = out.len() / channels;
        let mut done = 0;
        let mut acc = std::mem::take(&mut self.acc);
        while done < frames {
            let n = (frames - done).min(STEP);
            self.step(&mut acc[..n * 2], n);
            for i in 0..n {
                let o = &mut out[(done + i) * channels..(done + i + 1) * channels];
                if channels == 1 {
                    o[0] = 0.5 * (acc[2 * i] + acc[2 * i + 1]);
                } else {
                    o[0] = acc[2 * i];
                    o[1] = acc[2 * i + 1];
                    for x in o[2..].iter_mut() {
                        *x = 0.0;
                    }
                }
            }
            done += n;
        }
        self.acc = acc;
        let st = &self.stats;
        st.frames.fetch_add(frames as u64, Ordering::Relaxed);
        st.render_ns.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        let pk = (self.peak * 1e6) as u32;
        st.peak_micro.fetch_max(pk, Ordering::Relaxed);
        self.peak = 0.0;
    }

    fn step(&mut self, acc: &mut [f32], n: usize) {
        for x in acc.iter_mut() {
            *x = 0.0;
        }
        // Buses and master move toward their targets once a step (~1.3 ms; 20 ms time constant).
        let k = 1.0 - (-(n as f32) / (0.02 * self.rate)).exp();
        for b in 0..BUSES {
            self.bus_g[b] += (self.bus_t[b] - self.bus_g[b]) * k;
        }
        self.master_g += (self.master_t - self.master_g) * k;
        let Some(bank) = self.bank.clone() else {
            return;
        };
        // Estimates for the selection, then the selection.
        for i in 0..self.ems.len() {
            if self.ems[i].id == 0 {
                continue;
            }
            let e = self.ems[i];
            let bank_gain = match e.p.source {
                Source::Clip { clip, .. } => bank.clip(clip).map(|c| c.gain).unwrap_or(1.0),
                Source::Noise { .. } => 1.0,
            };
            let (l, r, rate, cut) = self.terms(&e, bank_gain);
            self.ems[i].est = (l * l + r * r).sqrt() * e.fade.max(0.05);
            self.ems[i].t = [l, r, rate, cut];
        }
        if self.step_count % SELECT_EVERY == 0 || self.reselect {
            self.reselect = false;
            self.select();
        }
        self.step_count = self.step_count.wrapping_add(1);
        let vstep = n as f32 / (0.012 * self.rate);
        let (mut mixed, mut active) = (0u32, 0u32);
        for i in 0..self.ems.len() {
            if self.ems[i].id == 0 {
                continue;
            }
            active += 1;
            let mut e = self.ems[i];
            let data: Option<&[i16]> = match e.p.source {
                Source::Clip { clip, .. } => match bank.clip(clip) {
                    Some(c) => c.data.as_deref(),
                    None => {
                        self.free(i);
                        continue;
                    }
                },
                Source::Noise { .. } => None,
            };
            let [tl, tr, trate, cut] = e.t;
            // Voice fade toward chosen / not chosen.
            let vt = if e.chosen { 1.0 } else { 0.0 };
            let v0 = if e.fresh && e.chosen { 1.0 } else { e.vgain };
            let v1 = if vt > v0 { (v0 + vstep).min(1.0) } else if vt < v0 { (v0 - vstep).max(0.0) } else { v0 };
            e.vgain = v1;
            if e.chosen {
                e.fresh = false;
            }
            // Fade envelope across the step.
            let f0 = e.fade;
            let f1 = (f0 + e.fade_step * n as f32).clamp(0.0, 1.0);
            e.fade = f1;
            let tgl = tl * v1 * f1;
            let tgr = tr * v1 * f1;
            let lp_a = if cut >= 18_000.0 { 1.0 } else { spatial::one_pole_coeff(cut, self.rate) };
            let mix = v0 > 0.0 || v1 > 0.0;
            let mut ended = false;
            if mix {
                mixed += 1;
                e.mixed = true;
                let inv = 1.0 / n as f32;
                let (dgl, dgr) = ((tgl - e.gl) * inv, (tgr - e.gr) * inv);
                let dr = (trate - e.rate) * inv;
                let da = (lp_a - e.lp_a) * inv;
                let (mut gl, mut gr, mut rate, mut a) = (e.gl, e.gr, e.rate, e.lp_a);
                let (mut z1, mut z2) = (e.z1, e.z2);
                match (e.p.source, data) {
                    (Source::Noise { .. }, _) => {
                        let mut s = e.rng;
                        // White noise; two one-poles make a road roar. Gain makes up the
                        // first pole's loss of power so the level follows `gain`.
                        let make_up = (1.0 / (lp_a / (2.0 - lp_a)).max(1e-4)).sqrt().min(30.0);
                        for j in 0..n {
                            s ^= s << 13;
                            s ^= s >> 17;
                            s ^= s << 5;
                            let w = (s as f32 * (1.0 / 2147483648.0)) - 1.0;
                            z1 += a * (w - z1);
                            z2 += a * (z1 - z2);
                            let y = z2 * make_up;
                            acc[2 * j] += y * gl;
                            acc[2 * j + 1] += y * gr;
                            gl += dgl;
                            gr += dgr;
                            a += da;
                        }
                        e.rng = s;
                    }
                    (Source::Clip { .. }, Some(d)) => {
                        let (lo, hi) = (e.lo as i64, e.hi as i64);
                        let mut p = e.head;
                        let lp = e.looping;
                        for j in 0..n {
                            let mut x = sample_mem(d, p, lo, hi);
                            if let Some([ls, le, xf]) = lp {
                                let (ls, le, xf) = (ls as f64, le as f64, xf as f64);
                                if xf > 0.0 && p >= le - xf {
                                    let t = ((p - (le - xf)) / xf).clamp(0.0, 1.0) as f32;
                                    let y = sample_mem(d, p - (le - ls), lo, hi);
                                    x = x * (1.0 - t).sqrt() + y * t.sqrt();
                                }
                            }
                            z1 += a * (x - z1);
                            acc[2 * j] += z1 * gl;
                            acc[2 * j + 1] += z1 * gr;
                            gl += dgl;
                            gr += dgr;
                            a += da;
                            p += rate as f64;
                            rate += dr;
                            if let Some([ls, le, _]) = lp {
                                if p >= le as f64 {
                                    p -= (le - ls) as f64;
                                }
                            } else if p >= (hi - 1) as f64 {
                                ended = true;
                                break;
                            }
                        }
                        e.head = p;
                    }
                    (Source::Clip { .. }, None) => {
                        // Streamed: pull chunks into the window as the head needs them.
                        let s = e.stream as usize;
                        if s == NO_SLOT as usize {
                            ended = true;
                        } else {
                            let need = e.head + (trate.max(e.rate) as f64) * n as f64 + 4.0;
                            let have = self.fill(s, need as u64);
                            let w = &self.windows[s];
                            let base = w.start as f64;
                            let len = w.buf.len() as i64;
                            if !have && !w.ended {
                                // Not read yet: hold the head (silence) rather than skip.
                                if e.started {
                                    self.stats.underruns.fetch_add(1, Ordering::Relaxed);
                                }
                            } else {
                                e.started = true;
                                let mut p = e.head;
                                for j in 0..n {
                                    let q = p - base;
                                    let qi = q.floor() as i64;
                                    if qi + 2 >= len {
                                        if w.ended {
                                            ended = true;
                                        }
                                        break;
                                    }
                                    let t = (q - qi as f64) as f32;
                                    let g = |i: i64| if i < 0 || i >= len { 0.0 } else { w.buf[i as usize] };
                                    let x = hermite(g(qi - 1), g(qi), g(qi + 1), g(qi + 2), t);
                                    z1 += a * (x - z1);
                                    acc[2 * j] += z1 * gl;
                                    acc[2 * j + 1] += z1 * gr;
                                    gl += dgl;
                                    gr += dgr;
                                    a += da;
                                    p += rate as f64;
                                    rate += dr;
                                }
                                e.head = p;
                                self.trim(s, p as u64);
                            }
                        }
                    }
                }
                e.gl = tgl;
                e.gr = tgr;
                e.rate = trate;
                e.lp_a = lp_a;
                e.z1 = z1;
                e.z2 = z2;
                if v1 == 0.0 {
                    e.mixed = false;
                }
            } else {
                // Virtual: the head moves as if heard.
                e.gl = 0.0;
                e.gr = 0.0;
                e.rate = trate;
                e.lp_a = lp_a;
                e.mixed = false;
                match e.p.source {
                    Source::Noise { .. } => {}
                    Source::Clip { .. } => {
                        if e.stream != NO_SLOT {
                            // Keep the stream in step (its data is dropped).
                            let s = e.stream as usize;
                            // Not heard yet: wait for its first data (then it is chosen); heard before: keep time.
                            if !e.started {
                                if self.fill(s, e.head as u64 + 8) {
                                    self.reselect = true;
                                }
                            } else {
                                e.head += (trate as f64) * n as f64;
                                let have = self.fill(s, e.head as u64 + 4);
                                self.trim(s, e.head as u64);
                                if !have && self.windows[s].ended {
                                    ended = true;
                                }
                            }
                        } else {
                            e.head += (trate as f64) * n as f64;
                            if let Some([ls, le, _]) = e.looping {
                                while e.head >= le as f64 {
                                    e.head -= (le - ls) as f64;
                                }
                            } else if e.head >= (e.hi - 1) as f64 {
                                ended = true;
                            }
                        }
                    }
                }
            }
            let faded_out = e.stopping && e.fade <= 0.0;
            self.ems[i] = e;
            if ended || faded_out {
                self.free(i);
            }
        }
        // Master and limiter.
        let thr = 0.9f32;
        let mut limited = 0u64;
        for j in 0..n {
            let l = acc[2 * j] * self.master_g;
            let r = acc[2 * j + 1] * self.master_g;
            let pk = l.abs().max(r.abs());
            // Release first, then hold the peak to the threshold (instant attack).
            self.lim_g += (1.0 - self.lim_g) * self.lim_rel;
            if pk * self.lim_g > thr {
                self.lim_g = thr / pk;
            }
            if self.lim_g < 0.891 {
                limited += 1;
            }
            acc[2 * j] = l * self.lim_g;
            acc[2 * j + 1] = r * self.lim_g;
            self.peak = self.peak.max(acc[2 * j].abs()).max(acc[2 * j + 1].abs());
        }
        let st = &self.stats;
        st.mixed.store(mixed, Ordering::Relaxed);
        st.active.store(active, Ordering::Relaxed);
        st.virtual_.store(active - mixed.min(active), Ordering::Relaxed);
        st.max_mixed.fetch_max(mixed, Ordering::Relaxed);
        st.max_active.fetch_max(active, Ordering::Relaxed);
        st.limited_frames.fetch_add(limited, Ordering::Relaxed);
        st.streams.store(self.stream_owner.iter().filter(|&&o| o != 0).count() as u32, Ordering::Relaxed);
    }

    /// Pull chunks of slot `s` until its window reaches source frame `upto`
    /// (or the ring is empty). True when it does.
    fn fill(&mut self, s: usize, upto: u64) -> bool {
        let w = &mut self.windows[s];
        loop {
            if w.start + w.buf.len() as u64 >= upto {
                return true;
            }
            if w.ended {
                return false;
            }
            if w.buf.len() + stream::CHUNK > w.buf.capacity() {
                return false; // the window is full: the head is far behind (never while mixing)
            }
            let c: Chunk = match self.streams[s].chunks.pop() {
                Ok(c) => c,
                Err(_) => return false,
            };
            if c.generation != w.generation {
                continue;
            }
            w.buf.extend_from_slice(&c.data[..c.len as usize]);
            if c.last {
                w.ended = true;
            }
        }
    }

    /// Drop window frames well behind the head (keeps 8 for the interpolator).
    fn trim(&mut self, s: usize, head: u64) {
        let w = &mut self.windows[s];
        if head > w.start + 8 + (WIN as u64 / 2) {
            let k = ((head - 8 - w.start) as usize).min(w.buf.len());
            w.buf.drain(..k);
            w.start += k as u64;
        }
    }
}

/// Memory the mixer holds outside the bank, bytes.
pub fn mixer_bytes(cmd_ring: usize) -> usize {
    MAX_EMITTERS * std::mem::size_of::<Em>() + stream::MAX_STREAMS * (WIN + stream::CHUNK) * 4 + cmd_ring * std::mem::size_of::<Cmd>() + MAP * 2
}
