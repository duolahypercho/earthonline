//! where a long frame went, and the render thread's scheduling.
//!
//! A per-thread, per-frame breakdown of the render thread's CPU work: named
//! phases (streaming pump parts, uploads, camera uniforms, encode, finish,
//! submit), the bytes and calls of every GPU write, the streaming events of the
//! frame, and the thread's own CPU cycles over the frame (Windows
//! `QueryThreadCycleTime`), so a long frame whose thread barely ran is told
//! apart from one that did a lot of work: wall time far above the thread's
//! cycle time means the thread was not on a CPU (descheduled, or blocked in
//! the driver or the OS).
//!
//! Recording costs two `Instant::now()` a span (about 50 ns); the bench reads
//! and resets the record once a frame (`take`). Nothing here allocates.
//!
//! Also: thread priorities. The render thread (and the player step that runs
//! on it) is raised above normal and every streaming/cooking worker is lowered
//! below it, so a burst of CPU-bound work (kit packs cooking, other processes
//! compiling) cannot take the render thread's core for a scheduler quantum.

use std::cell::RefCell;
use std::time::Instant;

/// A named part of the render thread's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Phase {
    /// Kit packs: requests, evictions, arrivals into upload jobs.
    KitUpdate,
    /// Street tiles: requests, evictions, arrivals into upload jobs.
    StreetUpdate,
    /// Blender models taken off the loader channel into upload jobs.
    ModelPoll,
    /// The budgeted upload queue writing this frame's bytes.
    Uploads,
    /// Readiness writes after the uploads (kit and street groups).
    AfterPump,
    /// Upload payloads freed (written or cancelled).
    PayloadFree,
    /// The camera, cull and look uniforms (`Renderer::update`).
    CameraUpdate,
    /// Command encoder creation + recording (`Renderer::encode` and the bench's copies).
    Encode,
    /// `CommandEncoder::finish`.
    Finish,
    /// `Queue::submit`.
    Submit,
    /// The player's fixed step (input, physics) when it runs on this thread.
    PlayerStep,
    /// Collision tiles inserted/removed for the player.
    CollisionPump,
    /// The player's camera rig.
    PlayerCamera,
    /// Moving agents: simulation hand-off and instance uploads.
    Agents,
    /// the street props' pump (tiles, lamp channels, the props' writes).
    PropsPump,
    /// the parked cars handed to the agent renderer.
    ParkedAgents,
    /// the look frame (sky model, rig, weather terms) rebuilt (inside camera_update).
    LookFrame,
    /// the scene probe's per-frame update (inside camera_update).
    ProbeUpdate,
    /// the sky slice's per-frame uniforms (inside camera_update).
    SkyUpdate,
}

pub const PHASES: usize = 19;
pub const PHASE_NAMES: [&str; PHASES] = [
    "kit_update",
    "street_update",
    "model_poll",
    "uploads",
    "after_pump",
    "payload_free",
    "camera_update",
    "encode",
    "finish",
    "submit",
    "player_step",
    "collision_pump",
    "player_camera",
    "agents",
    "props_pump",
    "parked_agents",
    "camera_update.look_frame",
    "camera_update.probe",
    "camera_update.sky",
];

/// named things that happened in a frame (counts), so a long frame
/// can be matched to the amortised job or the GPU object creation it carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Mark {
    /// `create_bind_group` on the render thread during play.
    BindGroupCreated,
    /// `create_buffer` / `create_texture` on the render thread during play.
    ResourceCreated,
    /// A kit pack became resident (its LOD groups written).
    KitPackResident,
    /// A street tile became resident.
    StreetTileResident,
    /// A props tile was handed to the upload queue.
    PropsTileQueued,
    /// The scene probe drew a cube face.
    ProbeFace,
    /// The scene probe finished a capture (fill, levels, lobe on the GPU).
    ProbeCaptureDone,
    /// A probe face was held to a later frame.
    ProbeDeferred,
    /// Sun shadow cascades refreshed this frame (count).
    CascadesRefreshed,
    /// The look frame was rebuilt (sky model, rig, weather terms).
    LookFrameRebuilt,
    /// The sky's temporal history was discarded (cut, first frame, new targets).
    SkyHistoryReset,
    /// The water FFT gate flipped (on to off or off to on).
    WaterFftFlip,
    /// The light gather's bind group was rebuilt.
    LightsBindGroup,
    /// The city/probe draw bind groups were rebuilt (probe front cube flipped).
    DrawBindGroups,
    /// Bytes staged through the upload ring instead of `write_buffer`.
    RingBytes,
    /// The upload ring grew (a new chunk was created).
    RingGrew,
    /// A counted submission took the driver's periodic spike (`pace`).
    SubmitSpike,
}

pub const MARKS: usize = 17;
pub const MARK_NAMES: [&str; MARKS] = [
    "bind_group_created",
    "resource_created",
    "kit_pack_resident",
    "street_tile_resident",
    "props_tile_queued",
    "probe_face",
    "probe_capture_done",
    "probe_deferred",
    "cascades_refreshed",
    "look_frame_rebuilt",
    "sky_history_reset",
    "water_fft_flip",
    "lights_bind_group",
    "draw_bind_groups",
    "ring_bytes",
    "ring_grew",
    "submit_spike",
];

/// write targets the per-target byte counts are kept for.
pub const WRITE_TARGETS: usize = 11;
pub const WRITE_TARGET_NAMES: [&str; WRITE_TARGETS] = ["vertex", "index", "object", "group", "age", "texture", "prop_instances", "prop_wires", "prop_lights", "uniforms", "other"];

/// One frame of the render thread.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTrace {
    pub ms: [f64; PHASES],
    /// GPU writes (`write_buffer` / `write_texture`) issued this frame.
    pub write_calls: u32,
    pub write_bytes: u64,
    pub largest_write: u64,
    /// Time inside those write calls (wgpu copies into a staging allocation).
    pub write_ms: f64,
    /// Streaming events this frame.
    pub arrivals: u32,
    pub evictions: u32,
    pub jobs_done: u32,
    pub jobs_cancelled: u32,
    /// named events (see `Mark`).
    pub marks: [u64; MARKS],
    /// bytes written per target (`WRITE_TARGET_NAMES`).
    pub target_bytes: [u64; WRITE_TARGETS],
}

impl FrameTrace {
    /// non-zero marks as (name, count).
    pub fn marks(&self) -> Vec<(&'static str, u64)> {
        MARK_NAMES.iter().zip(self.marks).filter(|(_, n)| *n > 0).map(|(n, v)| (*n, v)).collect()
    }
    /// non-zero per-target write bytes as (name, bytes).
    pub fn target_bytes(&self) -> Vec<(&'static str, u64)> {
        WRITE_TARGET_NAMES.iter().zip(self.target_bytes).filter(|(_, n)| *n > 0).map(|(n, v)| (*n, v)).collect()
    }
    pub fn total_ms(&self) -> f64 {
        self.ms.iter().sum()
    }
    /// The phase that took longest, and its time.
    pub fn top(&self) -> (&'static str, f64) {
        let (k, v) = self.ms.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
        (PHASE_NAMES[k], *v)
    }
    /// Non-zero phases as (name, ms), longest first.
    pub fn phases(&self) -> Vec<(&'static str, f64)> {
        let mut v: Vec<(&'static str, f64)> = PHASE_NAMES.iter().zip(self.ms).filter(|(_, ms)| *ms > 0.0).map(|(n, ms)| (*n, ms)).collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        v
    }
}

thread_local! {
    static CURRENT: RefCell<FrameTrace> = RefCell::new(FrameTrace::default());
}

/// Times a phase until dropped.
pub struct Span {
    phase: Phase,
    t: Instant,
}

impl Drop for Span {
    fn drop(&mut self) {
        let ms = self.t.elapsed().as_secs_f64() * 1e3;
        let p = self.phase as usize;
        CURRENT.with(|c| c.borrow_mut().ms[p] += ms);
    }
}

pub fn span(phase: Phase) -> Span {
    Span { phase, t: Instant::now() }
}

/// Run `f` as `phase`.
pub fn timed<R>(phase: Phase, f: impl FnOnce() -> R) -> R {
    let _s = span(phase);
    f()
}

/// A GPU write of `bytes` that took `ms`.
pub fn note_write(bytes: u64, ms: f64) {
    note_write_to(WRITE_TARGETS - 1, bytes, ms);
}

/// a GPU write of `bytes` into target `t` (`WRITE_TARGET_NAMES`) that took `ms`.
pub fn note_write_to(t: usize, bytes: u64, ms: f64) {
    CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        c.write_calls += 1;
        c.write_bytes += bytes;
        c.largest_write = c.largest_write.max(bytes);
        c.write_ms += ms;
        c.target_bytes[t.min(WRITE_TARGETS - 1)] += bytes;
    });
}

/// count `n` of a named event this frame.
pub fn mark(m: Mark, n: u64) {
    CURRENT.with(|c| c.borrow_mut().marks[m as usize] += n);
}

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Arrival,
    Eviction,
    JobDone,
    JobCancelled,
}

pub fn count(e: Event, n: u32) {
    CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        match e {
            Event::Arrival => c.arrivals += n,
            Event::Eviction => c.evictions += n,
            Event::JobDone => c.jobs_done += n,
            Event::JobCancelled => c.jobs_cancelled += n,
        }
    });
}

/// This thread's record since the last `take`, reset.
pub fn take() -> FrameTrace {
    CURRENT.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

// ---------------------------------------------------------------------------
// Never wait for a lock a worker holds.
// ---------------------------------------------------------------------------

/// The render thread's side of a request list it shares with worker threads
/// (`Mutex<Vec<K>>`). Requests and cancellations are kept here and applied
/// with `try_lock` only: a worker preempted while holding the lock (workers
/// run below normal priority, so other processes can starve them for a
/// scheduler quantum) must not stall the frame. Measured: a 10.7 ms frame
/// spent waiting inside the kit update under CPU load (priority inversion).
pub struct Outbox<K> {
    add: Vec<K>,
    remove: std::collections::HashSet<K>,
}

impl<K: Eq + std::hash::Hash + Copy> Default for Outbox<K> {
    fn default() -> Self {
        Self { add: vec![], remove: Default::default() }
    }
}

impl<K: Eq + std::hash::Hash + Copy> Outbox<K> {
    pub fn request(&mut self, k: K) {
        self.remove.remove(&k);
        self.add.push(k);
    }
    pub fn cancel(&mut self, k: K) {
        self.add.retain(|x| *x != k);
        self.remove.insert(k);
    }
    /// Apply what is queued if the lock is free now; else keep it for the next frame.
    pub fn flush(&mut self, shared: &std::sync::Mutex<Vec<K>>) -> bool {
        if self.add.is_empty() && self.remove.is_empty() {
            return true;
        }
        let Ok(mut p) = shared.try_lock() else { return false };
        if !self.remove.is_empty() {
            p.retain(|k| !self.remove.contains(k));
        }
        p.extend(self.add.drain(..));
        self.remove.clear();
        true
    }
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
}

/// Set a value shared with workers if its lock is free now (the camera the
/// workers sort by; a frame late is harmless).
pub fn try_set<T>(shared: &std::sync::Mutex<T>, v: T) {
    if let Ok(mut g) = shared.try_lock() {
        *g = v;
    }
}

// ---------------------------------------------------------------------------
// Thread CPU cycles (Windows): how long the thread actually ran.
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetCurrentThread() -> isize;
        pub fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
        pub fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        pub fn GetThreadPriority(thread: isize) -> i32;
    }
    pub const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
    pub const THREAD_PRIORITY_HIGHEST: i32 = 2;
}

/// CPU cycles this thread has run (on macOS its CPU time in nanoseconds,
/// which `cycles_per_ms` calibrates the same way; 0 where unsupported).
pub fn thread_cycles() -> u64 {
    #[cfg(windows)]
    {
        let mut c = 0u64;
        // SAFETY: plain Win32 call on the current thread's pseudo handle.
        unsafe {
            win::QueryThreadCycleTime(win::GetCurrentThread(), &mut c);
        }
        c
    }
    #[cfg(target_os = "macos")]
    {
        // libSystem's `clock_gettime_nsec_np` (macOS 10.12+) with
        // CLOCK_THREAD_CPUTIME_ID (16 in <time.h>): the calling thread's CPU time.
        unsafe extern "C" {
            fn clock_gettime_nsec_np(clock_id: u32) -> u64;
        }
        const CLOCK_THREAD_CPUTIME_ID: u32 = 16;
        // SAFETY: a plain libSystem call that takes no pointers.
        unsafe { clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID) }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        0
    }
}

/// Page faults of the whole process so far (soft and hard; 0 where unsupported).
pub fn process_page_faults() -> u64 {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            cb: u32,
            page_fault_count: u32,
            rest: [usize; 9],
        }
        #[link(name = "psapi")]
        unsafe extern "system" {
            fn GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> isize;
        }
        let mut c = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
        // SAFETY: plain Win32 call with a correctly sized, writable struct.
        if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) } != 0 {
            return c.page_fault_count as u64;
        }
    }
    0
}

/// Thread cycles per millisecond of running, measured once by spinning
/// (the best of five 4 ms spins, so a preempted spin does not count).
pub fn cycles_per_ms() -> f64 {
    static CAL: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *CAL.get_or_init(|| {
        let mut best: f64 = 0.0;
        for _ in 0..5 {
            let c0 = thread_cycles();
            let t0 = Instant::now();
            while t0.elapsed().as_secs_f64() < 0.004 {
                std::hint::spin_loop();
            }
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            best = best.max((thread_cycles() - c0) as f64 / ms);
        }
        best
    })
}

/// Milliseconds this thread ran between two `thread_cycles` readings.
pub fn on_cpu_ms(c0: u64, c1: u64) -> f64 {
    let k = cycles_per_ms();
    if k > 0.0 { (c1.saturating_sub(c0)) as f64 / k } else { f64::NAN }
}

/// Raise the calling thread (the render thread, which also runs the player
/// step) above normal: the frame's CPU work is a few hundred microseconds
/// and must not wait a scheduler quantum behind CPU-bound work.
/// Returns whether the OS accepted it.
pub fn raise_render_thread() -> bool {
    #[cfg(windows)]
    {
        // SAFETY: plain Win32 calls on the current thread's pseudo handle.
        unsafe { win::SetThreadPriority(win::GetCurrentThread(), win::THREAD_PRIORITY_HIGHEST) != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

static LOWER_WORKERS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Whether workers started from now on lower themselves (`worker_started`).
pub fn set_worker_priority_lowered(on: bool) {
    LOWER_WORKERS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Called first thing on every streaming / cooking worker thread.
pub fn worker_started() {
    if LOWER_WORKERS.load(std::sync::atomic::Ordering::Relaxed) {
        lower_worker_thread();
    }
}

/// Lower the calling thread (a streaming / cooking worker) below normal.
pub fn lower_worker_thread() -> bool {
    #[cfg(windows)]
    {
        // SAFETY: as above.
        unsafe { win::SetThreadPriority(win::GetCurrentThread(), win::THREAD_PRIORITY_BELOW_NORMAL) != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The calling thread's priority (Win32 value; 0 normal), for the bench report.
pub fn thread_priority() -> i32 {
    #[cfg(windows)]
    {
        // SAFETY: as above.
        unsafe { win::GetThreadPriority(win::GetCurrentThread()) }
    }
    #[cfg(not(windows))]
    {
        0
    }
}
