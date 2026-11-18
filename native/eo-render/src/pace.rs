//! the driver's submission spike, steered into time the render thread
//! would spend waiting for the GPU anyway.
//!
//! The Vulkan driver on the desktop GPU it was measured on spends 2.2-4 ms inside one
//! `vkQueueSubmit` out of every 1,023 that carry command buffers (empty
//! submissions do not count; measured over 20 consecutive periods). Nothing
//! about a frame's work changes it, so it lands on whichever frame makes that
//! submission; on a GPU-heavy frame (the rain and night views, 6.5-7.5 ms of
//! GPU) that frame goes over the budget.
//!
//! Every submission made through [`submit`] is counted and timed; a slow one
//! fixes the phase (the next spike is due `SPIKE_PERIOD` submissions later).
//! When a loop is about to block on the GPU and the spike is near, [`prepay`]
//! makes tiny submissions (a 4-byte clear each) in that wait: they move the
//! count on, and the one that meets the spike is made only when the rest of
//! the wait can absorb it. The spike then costs the frames nothing; with no
//! such wait (a CPU-bound stretch) it falls where it would have. A submission made elsewhere (not counted)
//! only moves the real spike earlier than predicted: the hunt then meets it
//! sooner, or the frame that makes it shows it and the phase is fixed again.

use std::sync::Mutex;
use std::time::Instant;

/// Counted submissions from one spike to the next.
pub const SPIKE_PERIOD: u64 = 1023;
/// A submission slower than this (ms) is taken for the spike (others:
/// 0.05-1.2 ms, the spike 2.2 ms and more).
pub const SPIKE_MS: f64 = 1.8;
/// Tiny submissions are made only this close to the predicted spike.
pub const HUNT_WINDOW: u64 = 128;
/// Tiny submissions made past the predicted spike before giving up (the
/// count drifts by one now and then: a submission made outside `submit`).
const LATE_SLACK: u64 = 2;
/// The least a tiny submission is reckoned to cost (ms) when planning one.
const TINY_MIN_MS: f64 = 0.05;
/// What a spike may cost (ms) when deciding whether a wait can absorb one.
pub const SPIKE_ALLOW_MS: f64 = 3.5;
/// Kept free of the expected wait (ms).
const MARGIN_MS: f64 = 0.5;

/// Counters since the process started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaceStats {
    /// Counted submissions (with command buffers), tiny ones included.
    pub submissions: u64,
    /// Submissions slower than `SPIKE_MS`.
    pub spikes: u64,
    /// Spikes taken by a tiny submission inside a wait.
    pub prepaid: u64,
    /// Tiny submissions made.
    pub tiny: u64,
}

struct State {
    count: u64,
    /// The count at which the next spike is due.
    next: Option<u64>,
    /// Running cost of one tiny submission (ms).
    tiny_ms: f64,
    stats: PaceStats,
    buf: Option<wgpu::Buffer>,
    /// The count at the last spike.
    last: u64,
}

static STATE: Mutex<State> = Mutex::new(State { count: 0, next: None, tiny_ms: 0.1, stats: PaceStats { submissions: 0, spikes: 0, prepaid: 0, tiny: 0 }, buf: None, last: 0 });

/// `EO_PACE_LOG=1`: print every spike and the counted submissions since the last one.
fn log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("EO_PACE_LOG").is_ok())
}

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Count one submission that took `ms`; true when it was the spike.
fn note(st: &mut State, ms: f64) -> bool {
    st.count += 1;
    st.stats.submissions += 1;
    if ms >= SPIKE_MS {
        st.stats.spikes += 1;
        if log_on() {
            eprintln!("pace: spike {ms:.2} ms at counted submission {} ({} since the last; predicted {:?})", st.count, st.count - st.last, st.next);
        }
        st.last = st.count;
        st.next = Some(st.count + SPIKE_PERIOD);
        crate::hitch::mark(crate::hitch::Mark::SubmitSpike, 1);
        return true;
    }
    if log_on() && ms >= 0.8 {
        eprintln!("pace: slow submission {ms:.2} ms at counted submission {} ({} since the last spike)", st.count, st.count - st.last);
    }
    // Well past the prediction without a spike: the phase is lost until the next one.
    if st.next.is_some_and(|n| st.count > n + 8) {
        st.next = None;
    }
    false
}

/// `queue.submit`, counted and timed (a submission with no command buffers is
/// not counted: the driver does not count it either).
pub fn submit<I: IntoIterator<Item = wgpu::CommandBuffer>>(queue: &wgpu::Queue, cmds: I) -> wgpu::SubmissionIndex {
    let cmds: Vec<wgpu::CommandBuffer> = cmds.into_iter().collect();
    let counted = !cmds.is_empty();
    let t = Instant::now();
    let index = queue.submit(cmds);
    if counted {
        note(&mut state(), t.elapsed().as_secs_f64() * 1e3);
    }
    index
}

/// A submission that always counts (a tiny clear), for loops that only flush
/// queued writes (`queue.submit([])` would count or not depending on them).
pub fn submit_tick(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::SubmissionIndex {
    let buf = tiny_buffer(device);
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("submit-pacer") });
    enc.clear_buffer(&buf, 0, Some(4));
    submit(queue, [enc.finish()])
}

fn tiny_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    let mut st = state();
    st.buf
        .get_or_insert_with(|| {
            crate::hitch::mark(crate::hitch::Mark::ResourceCreated, 1);
            device.create_buffer(&wgpu::BufferDescriptor { label: Some("submit-pacer"), size: 256, usage: wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
        })
        .clone()
}

/// What a [`prepay`] did.
#[derive(Clone, Copy, Debug, Default)]
pub struct Prepay {
    /// Tiny submissions made.
    pub submissions: u32,
    /// The spike fell on one of them.
    pub spiked: bool,
}

/// Before a wait for the GPU expected to last `idle_ms`: if the spike is due
/// within `HUNT_WINDOW` counted submissions and this wait can hold every tiny
/// submission up to it and the spike itself, make them; stop at the spike.
/// (A wait that cannot hold it all is left alone: moving the count on
/// without taking the spike would only bring it closer to a frame.)
pub fn prepay(device: &wgpu::Device, queue: &wgpu::Queue, idle_ms: f64) -> Prepay {
    let t = Instant::now();
    let mut out = Prepay::default();
    let (due, tiny_ms) = {
        let st = state();
        (st.next.map(|n| n.saturating_sub(st.count)), st.tiny_ms.max(TINY_MIN_MS))
    };
    let Some(due) = due.filter(|d| (1..=HUNT_WINDOW).contains(d)) else { return out };
    if due as f64 * tiny_ms + SPIKE_ALLOW_MS + MARGIN_MS > idle_ms {
        return out;
    }
    // Up to a few past the prediction (the count drifts by one now and then).
    for _ in 0..due + LATE_SLACK {
        if t.elapsed().as_secs_f64() * 1e3 + tiny_ms + SPIKE_ALLOW_MS + MARGIN_MS > idle_ms {
            break;
        }
        let buf = tiny_buffer(device);
        let ti = Instant::now();
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("submit-pacer") });
        enc.clear_buffer(&buf, 0, Some(4));
        let cmd = enc.finish();
        let ts = Instant::now();
        queue.submit([cmd]);
        let ms = ts.elapsed().as_secs_f64() * 1e3;
        let mut st = state();
        st.stats.tiny += 1;
        out.submissions += 1;
        if note(&mut st, ms) {
            st.stats.prepaid += 1;
            out.spiked = true;
            break;
        }
        st.tiny_ms = 0.8 * st.tiny_ms + 0.2 * ti.elapsed().as_secs_f64() * 1e3;
    }
    if log_on() && out.submissions > 0 {
        let st = state();
        eprintln!(
            "pace: prepay {} tiny submissions in {:.2} ms of an expected {idle_ms:.2} ms wait (tiny {:.3} ms); spike taken {}; due in {:?}",
            out.submissions,
            t.elapsed().as_secs_f64() * 1e3,
            st.tiny_ms,
            out.spiked,
            st.next.map(|n| n.saturating_sub(st.count))
        );
    }
    out
}

/// A wait after a prepay shorter than this (ms) means the GPU did not outlast
/// the prepay: its time is charged to the frame.
pub const PREPAY_FREE_WAIT_MS: f64 = 0.25;

/// the pacer for a loop's own wait for the GPU, shared by the bench,
/// the headless play/net runs and the windowed game. Wrap the call that blocks
/// (`device.poll(Wait)` headless, `surface.get_current_texture()` in a
/// window): the prepay is made just before it, sized by the last frame's wait.
/// With no wait (a CPU-bound stretch, or a window without vsync that never
/// blocks) nothing is prepaid and the spike falls where it would have.
#[derive(Clone, Copy, Debug)]
pub struct WaitPacer {
    pub enabled: bool,
    /// The last frame's wait (prepay included), ms.
    idle_ms: f64,
    frames: u64,
}

/// What one [`WaitPacer::around`] did.
#[derive(Clone, Copy, Debug, Default)]
pub struct Waited {
    pub prepay: Prepay,
    pub prepay_ms: f64,
    /// The wrapped wait, ms.
    pub wait_ms: f64,
    /// Prepay time the wait did not absorb: count it as this frame's CPU time.
    pub charged_ms: f64,
}

impl WaitPacer {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, idle_ms: 0.0, frames: 0 }
    }

    pub fn around<R>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, wait: impl FnOnce() -> R) -> (R, Waited) {
        let tpre = Instant::now();
        let prepay = if self.enabled && self.frames >= 2 { self::prepay(device, queue, self.idle_ms) } else { Prepay::default() };
        let prepay_ms = tpre.elapsed().as_secs_f64() * 1e3;
        let tw = Instant::now();
        let r = wait();
        let wait_ms = tw.elapsed().as_secs_f64() * 1e3;
        let overran = prepay.submissions > 0 && wait_ms < PREPAY_FREE_WAIT_MS;
        self.idle_ms = if overran { wait_ms } else { prepay_ms + wait_ms };
        self.frames += 1;
        (r, Waited { prepay, prepay_ms, wait_ms, charged_ms: if overran { prepay_ms } else { 0.0 } })
    }
}

/// Counters since the process started.
pub fn stats() -> PaceStats {
    state().stats
}

/// Counted submissions until the predicted spike (None: no phase yet).
pub fn due() -> Option<u64> {
    let st = state();
    st.next.map(|n| n.saturating_sub(st.count))
}
