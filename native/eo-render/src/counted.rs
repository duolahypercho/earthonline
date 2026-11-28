//! Count-driven multi-draws where the adapter has no
//! `MULTI_DRAW_INDIRECT_COUNT` (Metal).
//!
//! The renderer's long draw lists are compacted on the GPU: a cull pass
//! appends one command per visible object at an atomic slot and counts them,
//! and the draw reads that count on the GPU
//! (`multi_draw_indexed_indirect_count`). Metal has no such draw, and wgpu's
//! `multi_draw_indexed_indirect` there is a loop on the CPU of one draw per
//! command slot: a list with a slot for every object costs tens of
//! milliseconds a frame to encode.
//!
//! The emulation keeps the GPU compaction and draws a count the CPU knows when
//! it encodes, the list's bound: the largest count the list reached over the
//! last `WINDOW` frames read back, plus half again and `MARGIN`. Each list's
//! slots up to its bound are zeroed at the start of the frame, before any cull
//! fills them, so every slot past this frame's count is an empty draw. Counts
//! arrive a few frames late (a ring of `RING` copies, mapped in the next
//! frame's `begin_frame`, as the water's read-back does). A list with no count
//! read back yet, or whose read-back count went past the bound it was drawn
//! with, uses a conservative GPU cull during recovery. The cull runs without
//! camera occlusion, so its opaque count bounds both early and late draws.
//! Cuts and overflows recover for `FULL_FRAMES` frames while asynchronous
//! history catches up. A failed read keeps the full-capacity fallback.
//! A cut also starts a fresh window (`CutWindow`, on unless
//! `--no-cut-window`): the first read-back of a frame encoded after the cut's
//! replaces the counts of the view before it, which would otherwise bound the
//! new view until `WINDOW` read-backs had pushed them out. The cut's own frame
//! joins neither window: its early pass drew the old view's visible set, so its
//! late lists hold nearly everything in view, a count that only another cut
//! (recovered again) or a fast turn (which keeps the late lists' floor)
//! reaches again. With the fresh window the occlusion turning on or off is a
//! cut too: off, every object in the frustum draws, a count no read-back of
//! an occluded frame bounds (the old window covered it only while it still
//! held some earlier frame drawn without the occlusion).
//! A list may take its floor from another list's bound (the late
//! opaque list from the opaque list: after a fast turn every visible object
//! can be late). A list can therefore miss objects only in the frames between
//! a jump of its count past its bound and the read-back that sees it; `Stats`
//! counts those overflows, and `take_overflow` tells the list's owner (the
//! probe captures its faces again).
//!
//! Motion-aware bounds (`MotionBounds`, on unless `--no-motion-bounds`). A
//! late list's floor is there for a fast turn; most frames turn little. In a
//! frame where only the camera's rotation can change what is visible (the eye
//! still, the same lens, nothing streamed or written that selects objects, the
//! visibility flags kept, the occlusion on in this frame and the last, a
//! fresh read-back) and the rotation is under `STEADY_TURN_MAX` and no larger
//! than one a read-back in the window saw, a late list is drawn to its own
//! reserve instead (`ViewStep`, `Entry::tighten`). Every other frame keeps
//! the floor. An early list is drawn to the smaller of its reserve and a
//! bound the passes make exact: the early pass draws only objects the last
//! late pass found visible, which it drew early or late, so an early count is
//! at most the newest read-back's early and late counts plus the late bound
//! of every frame encoded since (`MARGIN` added), unless one of those late
//! lists overflowed (counted as such) or an object changed class (no
//! streamed write in those frames).
//!
//! Off (`--no-count-emulation`) the renderer keeps its uncompacted fallback,
//! as before the emulation. With `--bounded-draws` (bounds.rs) the lists that
//! count guards are not registered here; the rest still are.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// Read-back copies per registered count buffer.
pub const RING: usize = 4;
/// Frames of read-back counts a bound looks at.
pub const WINDOW: usize = 64;
/// Commands added to every bound past half again the recent peak.
pub const MARGIN: u32 = 256;
/// Frames of conservative recovery after a cut or an overflow, longer than
/// the read-back ring so that stale observations have time to drain.
pub const FULL_FRAMES: u64 = 8;
/// (motion-aware bounds) The largest rotation a frame may make (radians, 2
/// degrees) and still draw its late lists to their own reserve.
pub const STEADY_TURN_MAX: f32 = 0.035;
/// (motion-aware bounds) The eye's largest step (metres) in a still frame: a
/// moving eye changes levels of detail and occluders, so moving frames only
/// tighten when the window itself observed comparable motion.
pub const STEADY_STEP_M: f32 = 1e-3;
/// (motion-aware bounds) Largest per-frame eye step (metres) that can still
/// tighten late lists when the read-back window observed at least that step
/// (below the 30 m cut threshold so smooth fast orbits/flights tighten too).
pub const STEADY_MOVE_MAX_M: f32 = 25.0;
/// (motion-aware bounds) Rounding allowed over the window's largest eye step.
pub const STEP_SLACK_M: f32 = 0.05;
/// (motion-aware bounds) Rounding allowed over the window's largest rotation.
pub const TURN_SLACK: f32 = 1e-4;
/// (motion-aware bounds) A read-back older than this many encoded frames is
/// stale: the late lists keep their floor.
pub const STALE_FRAMES: u64 = 2 * RING as u64 + 2;
/// Encoded frames remembered for the early lists' exact bound.
const LOG_FRAMES: usize = 2 * RING + 8;

/// The motion-aware bounds' switch: `--no-motion-bounds` floors every late
/// list by its early one and draws every early list to its reserve, as before.
pub struct MotionBounds;

thread_local! {
    static MOTION_BOUNDS_SCOPE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

impl MotionBounds {
    /// The switch (eo-app accepts it; this crate reads it).
    pub const SWITCH: &'static str = "--no-motion-bounds";

    /// Whether a registry made now uses them (unless `--no-motion-bounds`, or a
    /// `with` scope on this thread says otherwise).
    pub fn wanted() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        MOTION_BOUNDS_SCOPE.with(|s| s.get()).unwrap_or_else(|| *ON.get_or_init(|| !std::env::args().any(|a| a == Self::SWITCH)))
    }

    /// Runs `body` with registries made meanwhile on this thread using them or
    /// not (tests: the two side by side in one process).
    pub fn with<R>(on: bool, body: impl FnOnce() -> R) -> R {
        let old = MOTION_BOUNDS_SCOPE.with(|s| s.replace(Some(on)));
        struct Restore(Option<bool>);
        impl Drop for Restore {
            fn drop(&mut self) {
                MOTION_BOUNDS_SCOPE.with(|s| s.set(self.0));
            }
        }
        let _restore = Restore(old);
        body()
    }
}

/// The fresh window on a cut's switch: `--no-cut-window` keeps the read-backs
/// of the view before a cut in the window until `WINDOW` newer ones push them
/// out, and the occlusion turning on or off is no cut, as before.
pub struct CutWindow;

thread_local! {
    static CUT_WINDOW_SCOPE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

impl CutWindow {
    /// The switch (eo-app accepts it; this crate reads it).
    pub const SWITCH: &'static str = "--no-cut-window";

    /// Whether a registry made now starts a fresh window on a cut (unless
    /// `--no-cut-window`, or a `with` scope on this thread says otherwise).
    pub fn wanted() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        CUT_WINDOW_SCOPE.with(|s| s.get()).unwrap_or_else(|| *ON.get_or_init(|| !std::env::args().any(|a| a == Self::SWITCH)))
    }

    /// Runs `body` with registries made meanwhile on this thread starting one
    /// or not (tests: the two side by side in one process).
    pub fn with<R>(on: bool, body: impl FnOnce() -> R) -> R {
        let old = CUT_WINDOW_SCOPE.with(|s| s.replace(Some(on)));
        struct Restore(Option<bool>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CUT_WINDOW_SCOPE.with(|s| s.set(self.0));
            }
        }
        let _restore = Restore(old);
        body()
    }
}

/// What changed since the last frame, for the motion-aware bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewStep {
    /// The camera's rotation since the last frame (radians).
    pub turn: f32,
    /// The eye's step since the last frame (metres).
    pub step: f32,
    /// Nothing but the camera's pose changed what the cull can select: the
    /// same lens, nothing streamed or written to the objects, their groups or
    /// the far layer's residency, the visibility flags kept.
    pub quiet: bool,
    /// When `quiet` is false only due to incremental object/group streaming writes,
    /// an upper bound on newly added objects so steady late lists can add exact
    /// headroom instead of falling back to the full early floor. `None` forces
    /// full fallback when `!quiet`.
    pub stream_added: Option<u32>,
    /// The two-phase occlusion runs in this frame.
    pub occlusion: bool,
}

/// Per-entry registration options (`Counts::register_with`).
#[derive(Clone, Copy, Debug)]
pub struct EntryOptions {
    /// Commands added past the reserve (`MARGIN` by default).
    pub margin: u32,
    /// Clear each list's command buffer up to its bound before culls run
    /// (true by default; false when the cull pass itself zeroes unused tail slots).
    pub zero_cmds: bool,
    /// Run a synchronous conservative recovery cull on cold start / cuts
    /// (true by default; false for small fixed-cap lists like props).
    pub recover_sync: bool,
}

impl Default for EntryOptions {
    fn default() -> Self {
        Self { margin: MARGIN, zero_cmds: true, recover_sync: true }
    }
}

/// One draw list counted in a registered count buffer.
#[derive(Clone, Debug)]
pub struct List {
    /// The list's count, a u32 word of the count buffer.
    pub word: u32,
    /// The command buffer and the list's first byte in it.
    pub draws: wgpu::Buffer,
    pub offset: u64,
    /// Bytes per command: 20 indexed, 16 non-indexed.
    pub stride: u64,
    /// The list's length in commands (what the exact fallback draws).
    pub max: u32,
    /// Earlier pass for the same opaque geometry. Newly visible objects drawn
    /// by this list can move to the earlier pass on the following frame.
    pub floor: Option<usize>,
}

/// What one registered count buffer's lists drew.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub label: &'static str,
    /// Read-backs harvested.
    pub readbacks: u64,
    /// Read-backs whose count went past the bound the list was drawn with.
    pub overflows: u64,
    /// Encoded frames for which all lists used their full capacity.
    pub full_frames: u64,
    /// Per list: this frame's bound, the peak count read back, the peak bound.
    pub bounds: Vec<u32>,
    pub peak_counts: Vec<u32>,
    pub peak_bounds: Vec<u32>,
    /// Encoded frames, and per list the sum of the bounds they were encoded
    /// with (the commands a frame that draws the list issues; bench reports).
    pub frames: u64,
    pub encoded: Vec<u64>,
    /// (motion-aware bounds) Encoded frames whose late lists took their own
    /// reserve, and frames with an early list under its reserve by the exact
    /// bound.
    pub steady_frames: u64,
    pub chained_frames: u64,
}

// Read-back stages (as the water's read-back): free, copied in an encoder,
// mapping, mapped.
const FREE: u8 = 0;
const COPIED: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;

struct Readback {
    buf: wgpu::Buffer,
    stage: Arc<AtomicU8>,
    /// The bounds of the frame whose counts this copy holds.
    bounds: Vec<u32>,
    /// That frame's number (`State::seq`), rotation (`ViewStep::turn`, negative
    /// without one), and eye step (`ViewStep::step`, negative without one).
    seq: u64,
    turn: f32,
    step: f32,
}

/// One encoded frame, for the early lists' exact bound.
struct Frame {
    seq: u64,
    bounds: Vec<u32>,
    occlusion: bool,
    quiet: bool,
}

struct Entry {
    counts: wgpu::Buffer,
    /// Bytes copied from the count buffer (up to the last list's word).
    copy_bytes: u64,
    lists: Vec<List>,
    margin: u32,
    zero_cmds: bool,
    recover_sync: bool,
    readbacks: Vec<Readback>,
    /// Per frame read back, the count of every list (newest last).
    history: VecDeque<Vec<u32>>,
    /// Per frame read back, its rotation (negative: none known, the
    /// recovery's counts).
    turns: VecDeque<f32>,
    /// Per frame read back, its eye step (negative: none known, the
    /// recovery's counts).
    steps: VecDeque<f32>,
    /// The newest frame read back: its number and counts.
    newest: Option<(u64, Vec<u32>)>,
    /// The last encoded frames (oldest first).
    log: VecDeque<Frame>,
    /// This frame's bounds without the motion-aware ones, and whether those
    /// changed any.
    fallback: Vec<u32>,
    tightened: bool,
    steady: bool,
    chained: bool,
    /// Encoded frames still requiring conservative recovery counts.
    full_left: u64,
    /// (`CutWindow`) The first frame (`State::seq`) of the view the window
    /// describes: the one after the last cut's own frame. A read-back of an
    /// earlier frame still counts its overflows and peaks but joins no window.
    view_from: u64,
    /// (`CutWindow`) The window still holds the view before the cut: the first
    /// read-back from `view_from` on replaces it (the window never empties).
    stale: bool,
    /// An overflow not yet taken by the owner (`take_overflow`).
    overflow_pending: bool,
    stats: Stats,
}

struct State {
    entries: Vec<Entry>,
    /// The motion-aware bounds are on (`MotionBounds`).
    motion: bool,
    /// A cut starts a fresh window (`CutWindow`).
    cut_window: bool,
    /// (`CutWindow`) The last frame's `ViewStep::occlusion`: a change is a cut.
    last_occlusion: Option<bool>,
    /// Frames encoded so far (`encode_copies`): the next one's number.
    seq: u64,
    /// This frame's view step (`begin_frame`), and the encodes since it.
    step: Option<ViewStep>,
    encodes: u32,
}

impl Entry {
    /// The motion-aware bounds on this frame's `bounds` (`motion_bounds`).
    fn tighten(&mut self, bounds: &mut [u32], step: ViewStep, seq: u64) {
        let lists: Vec<(Option<usize>, u32)> = self.lists.iter().map(|l| (l.floor, l.max)).collect();
        let seen = Seen { history: &self.history, turns: &self.turns, steps: &self.steps, newest: self.newest.as_ref(), log: &self.log };
        (self.steady, self.chained) = motion_bounds(&lists, &seen, bounds, step, seq, self.margin);
        self.tightened = self.steady || self.chained;
    }
}

/// What a registered count buffer's read-backs and encodes have seen.
struct Seen<'a> {
    history: &'a VecDeque<Vec<u32>>,
    turns: &'a VecDeque<f32>,
    steps: &'a VecDeque<f32>,
    newest: Option<&'a (u64, Vec<u32>)>,
    log: &'a VecDeque<Frame>,
}

/// The motion-aware bounds on `bounds` (this frame's reserves and floors, in a
/// frame not under recovery) of the lists `lists` (each its floor and length):
/// each late list's own reserve in a steady frame, each early list's exact
/// bound where it is smaller. Returns whether a late list and an early list
/// took them.
fn motion_bounds(lists: &[(Option<usize>, u32)], seen: &Seen<'_>, bounds: &mut [u32], step: ViewStep, seq: u64, margin: u32) -> (bool, bool) {
    let (mut steady_any, mut chained_any) = (false, false);
    let Some((k, last)) = seen.newest else { return (false, false) };
    let k = *k;
    // every frame from the newest read-back's on is logged, drawn with the occlusion,
    // and none after it streamed a write (an object's class kept).
    let since = seen.log.iter().filter(|f| f.seq >= k);
    let chain = step.occlusion
        && step.quiet
        && seq > k
        && since.clone().count() as u64 == seq - k
        && since.clone().all(|f| f.occlusion && (f.seq == k || f.quiet));
    let window_turn = seen.turns.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let window_step = seen.steps.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let moving = step.step > STEADY_STEP_M;
    let step_ok = !moving
        || (step.step <= STEADY_MOVE_MAX_M
            && step.step <= window_step + STEP_SLACK_M
            && seen.steps.iter().all(|s| *s >= 0.0));
    let stream_ok = step.quiet || step.stream_added.is_some();
    let extra_added = if step.quiet { 0 } else { step.stream_added.unwrap_or(0) };
    let steady = step.occlusion
        && stream_ok
        && seen.log.back().is_some_and(|f| f.seq + 1 == seq && f.occlusion)
        && seq > k
        && seq - k <= STALE_FRAMES
        && step_ok
        && step.turn <= STEADY_TURN_MAX
        && step.turn <= window_turn + TURN_SLACK
        && seen.turns.iter().all(|t| *t >= 0.0);
    for (i, &(floor, max)) in lists.iter().enumerate() {
        let Some(f) = floor else { continue };
        if steady {
            // its own reserve, without the early list's bound as a floor.
            // A translating eye keeps 2x peak + margin headroom against newly uncovered occluders,
            // plus exact headroom for any small incremental streaming write this frame.
            let peak = seen.history.iter().map(|h| h[i]).max().unwrap_or(0);
            let base_own = if moving {
                peak.saturating_mul(2).saturating_add(margin)
            } else {
                draw_reserve_with(peak, max, margin)
            };
            let own = base_own.saturating_add(extra_added).min(max);
            if own < bounds[i] {
                bounds[i] = own;
                steady_any = true;
            }
        }
        if chain {
            let pending = seen.log.iter().filter(|fr| fr.seq > k).fold(0u32, |a, fr| a.saturating_add(fr.bounds[i]));
            let exact = last[f].saturating_add(last[i]).saturating_add(pending).saturating_add(margin).min(lists[f].1);
            if exact < bounds[f] {
                bounds[f] = exact;
                chained_any = true;
            }
        }
    }
    (steady_any, chained_any)
}

/// Every count buffer drawn through the emulation, and their bounds this frame.
pub struct Counts {
    device: wgpu::Device,
    state: Mutex<State>,
}

/// The lists of one registered count buffer.
#[derive(Clone)]
pub struct CountedLists {
    counts: Arc<Counts>,
    id: usize,
}

fn draw_reserve_with(visible: u32, capacity: u32, margin: u32) -> u32 {
    visible.saturating_add(visible / 2).saturating_add(margin).min(capacity)
}

#[cfg(test)]
fn draw_reserve(visible: u32, capacity: u32) -> u32 {
    draw_reserve_with(visible, capacity, MARGIN)
}

fn early_reserve_with(early: u32, late: u32, capacity: u32, margin: u32) -> u32 {
    // The passes partition the same opaque set. Every object the late pass
    // discovered can already be in the early pass by the next read-back.
    draw_reserve_with(early.saturating_add(late), capacity, margin)
}

#[cfg(test)]
fn early_reserve(early: u32, late: u32, capacity: u32) -> u32 {
    early_reserve_with(early, late, capacity, MARGIN)
}

impl Counts {
    pub fn new(device: &wgpu::Device) -> Arc<Self> {
        Arc::new(Self { device: device.clone(), state: Mutex::new(State { entries: vec![], motion: MotionBounds::wanted(), cut_window: CutWindow::wanted(), last_occlusion: None, seq: 0, step: None, encodes: 0 }) })
    }

    /// Register a count buffer (it needs `COPY_SRC`) and the lists it counts.
    pub fn register(self: &Arc<Self>, label: &'static str, counts: &wgpu::Buffer, lists: Vec<List>) -> CountedLists {
        self.register_with(label, counts, lists, EntryOptions::default())
    }

    /// Register a count buffer with custom reserve margin, clear, and recovery policy.
    pub fn register_with(self: &Arc<Self>, label: &'static str, counts: &wgpu::Buffer, lists: Vec<List>, opts: EntryOptions) -> CountedLists {
        let copy_bytes = (lists.iter().map(|l| l.word).max().unwrap_or(0) as u64 + 1) * 4;
        let readbacks = (0..RING)
            .map(|k| Readback {
                buf: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("{label}-count-readback-{k}")),
                    size: copy_bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                stage: Arc::new(AtomicU8::new(FREE)),
                bounds: vec![],
                seq: 0,
                turn: -1.0,
                step: -1.0,
            })
            .collect();
        let n = lists.len();
        let full: Vec<u32> = lists.iter().map(|l| l.max).collect();
        let mut st = self.state.lock().unwrap();
        st.entries.push(Entry {
            counts: counts.clone(),
            copy_bytes,
            lists,
            margin: opts.margin,
            zero_cmds: opts.zero_cmds,
            recover_sync: opts.recover_sync,
            readbacks,
            history: VecDeque::new(),
            turns: VecDeque::new(),
            steps: VecDeque::new(),
            newest: None,
            log: VecDeque::new(),
            fallback: full.clone(),
            tightened: false,
            steady: false,
            chained: false,
            full_left: 0,
            view_from: 0,
            stale: false,
            overflow_pending: false,
            stats: Stats { label, bounds: full, peak_counts: vec![0; n], peak_bounds: vec![0; n], encoded: vec![0; n], ..Default::default() },
        });
        CountedLists { counts: self.clone(), id: st.entries.len() - 1 }
    }

    /// Per frame, before any list is zeroed or drawn: harvest the read-backs
    /// and fix every list's bound for this frame. `step`: what changed since
    /// the last frame (the motion-aware bounds; None: unknown, no such bounds).
    pub fn begin_frame(&self, step: Option<ViewStep>) {
        let mut guard = self.state.lock().unwrap();
        let st = &mut *guard;
        st.step = step;
        st.encodes = 0;
        if let Some(occlusion) = step.map(|s| s.occlusion) {
            if st.cut_window && st.last_occlusion.is_some_and(|o| o != occlusion) {
                Self::cut(st); // the occlusion turned on or off: another view's counts
            }
            st.last_occlusion = Some(occlusion);
        }
        let (seq, motion) = (st.seq, st.motion);
        for e in st.entries.iter_mut() {
            for k in 0..e.readbacks.len() {
                match e.readbacks[k].stage.load(Ordering::Acquire) {
                    COPIED => {
                        // Recorded in a frame already submitted (begin_frame runs before this frame's encode).
                        e.readbacks[k].stage.store(MAPPING, Ordering::Release);
                        let flag = e.readbacks[k].stage.clone();
                        e.readbacks[k].buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                            flag.store(if r.is_ok() { MAPPED } else { FREE }, Ordering::Release);
                        });
                    }
                    MAPPED => {
                        let words: Option<Vec<u32>> = e.readbacks[k].buf.slice(..).get_mapped_range().ok().map(|v| bytemuck::pod_collect_to_vec(&v[..]));
                        e.readbacks[k].buf.unmap();
                        e.readbacks[k].stage.store(FREE, Ordering::Release);
                        let Some(words) = words else { continue };
                        let counts: Vec<u32> = e.lists.iter().map(|l| words.get(l.word as usize).copied().unwrap_or(0).min(l.max)).collect();
                        let drawn = &e.readbacks[k].bounds;
                        if counts.iter().zip(drawn).any(|(c, b)| c > b) {
                            e.stats.overflows += 1;
                            e.overflow_pending = true;
                            e.full_left = e.full_left.max(FULL_FRAMES);
                        }
                        for (p, c) in e.stats.peak_counts.iter_mut().zip(&counts) {
                            *p = (*p).max(*c);
                        }
                        e.stats.readbacks += 1;
                        let (at, turn, step) = (e.readbacks[k].seq, e.readbacks[k].turn, e.readbacks[k].step);
                        if e.newest.as_ref().is_none_or(|(s, _)| at > *s) {
                            e.newest = Some((at, counts.clone()));
                        }
                        if at < e.view_from {
                            continue; // a frame of the view before the last cut, or the cut's own
                        }
                        if std::mem::take(&mut e.stale) {
                            // the first read-back since the cut: the window starts with it.
                            e.history.clear();
                            e.turns.clear();
                            e.steps.clear();
                        }
                        e.history.push_back(counts);
                        e.turns.push_back(turn);
                        e.steps.push_back(step);
                        if e.history.len() > WINDOW {
                            e.history.pop_front();
                            e.turns.pop_front();
                            e.steps.pop_front();
                        }
                    }
                    _ => {}
                }
            }
            let full = e.history.is_empty() || e.full_left > 0;
            let margin = e.margin;
            let mut bounds: Vec<u32> = e
                .lists
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    if full {
                        l.max
                    } else {
                        let peak = e.history.iter().map(|h| h[i]).max().unwrap_or(0);
                        draw_reserve_with(peak, l.max, margin)
                    }
                })
                .collect();
            for (i, l) in e.lists.iter().enumerate() {
                if let Some(f) = l.floor {
                    if let Some(last) = e.history.back() {
                        bounds[f] = bounds[f].max(early_reserve_with(last[f], last[i], e.lists[f].max, margin));
                    }
                    bounds[i] = bounds[i].max(bounds[f]).min(l.max);
                }
            }
            e.fallback.clone_from(&bounds);
            e.tightened = false;
            e.steady = false;
            e.chained = false;
            if let (true, false, Some(step)) = (motion, full, step) {
                e.tighten(&mut bounds, step, seq);
            }
            for (i, b) in bounds.into_iter().enumerate() {
                e.stats.bounds[i] = b;
            }
        }
    }

    /// Before an encode that follows a write the frame's `ViewStep` did not
    /// see: every list back to its bound without the motion-aware ones.
    pub fn relax(&self) {
        let mut st = self.state.lock().unwrap();
        for e in st.entries.iter_mut() {
            if e.tightened {
                e.stats.bounds.clone_from(&e.fallback);
                e.tightened = false;
                e.steady = false;
                e.chained = false;
            }
        }
    }

    /// On a cold list or a visibility discontinuity, measure a conservative
    /// bound from the GPU before encoding full-capacity Metal command lists.
    /// Ordinary frames use asynchronous history. A failed read retains the
    /// full-capacity fallback selected by begin_frame.
    pub(crate) fn prepare_recovery(&self, queue: &wgpu::Queue, cull: impl FnOnce(&mut wgpu::CommandEncoder)) {
        // Polling can run arbitrary completion callbacks. Keep the registry
        // unlocked throughout the submission, mapping and wait.
        let sources: Vec<_> = {
            let state = self.state.lock().unwrap();
            if !state.entries.iter().any(|e| e.recover_sync && (e.history.is_empty() || e.full_left != 0)) {
                return;
            }
            state.entries.iter().enumerate().filter(|(_, e)| e.recover_sync).map(|(id, e)| (id, e.counts.clone(), e.copy_bytes)).collect()
        };
        let size: u64 = sources.iter().map(|(_, _, bytes)| bytes).sum();
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("count recovery"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        cull(&mut enc);
        let mut offset = 0;
        for (_, source, bytes) in &sources {
            enc.copy_buffer_to_buffer(source, 0, &buf, offset, *bytes);
            offset += bytes;
        }
        let submitted = crate::pace::submit(queue, [enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buf.slice(..).map_async(wgpu::MapMode::Read, move |mapped| { let _ = tx.send(mapped); });
        if self.device.poll(wgpu::PollType::Wait { submission_index: Some(submitted), timeout: None }).is_err()
            || !matches!(rx.recv(), Ok(Ok(()))) {
            return;
        }
        if let Ok(bytes) = buf.slice(..).get_mapped_range() {
            let words: &[u32] = bytemuck::cast_slice(&bytes);
            let mut start = 0;
            let mut state = self.state.lock().unwrap();
            for &(id, _, bytes) in &sources {
                let e = &mut state.entries[id];
                if e.history.is_empty() || e.full_left > 0 {
                    let counts: Vec<u32> = e.lists.iter().map(|l| words[start + l.word as usize].min(l.max)).collect();
                    let margin = e.margin;
                    for (i, l) in e.lists.iter().enumerate() {
                        let upper = l.floor.map_or(counts[i], |early| counts[early].max(counts[i]));
                        e.stats.bounds[i] = draw_reserve_with(upper, l.max, margin);
                    }
                    if e.history.is_empty() {
                        e.history.push_back(counts);
                        e.turns.push_back(-1.0); // no frame's rotation: no motion-aware bound while it is in the window
                        e.steps.push_back(-1.0);
                    }
                }
                start += bytes as usize / 4;
            }
        }
        buf.unmap();
    }

    /// At the start of the frame's commands, before any cull: zero every list
    /// up to its bound, so the slots past this frame's count draw nothing.
    pub fn encode_zero(&self, enc: &mut wgpu::CommandEncoder) {
        let mut guard = self.state.lock().unwrap();
        let st = &mut *guard;
        st.encodes += 1;
        if st.encodes > 1 {
            // a second encode on the frame's bounds: the exact early bound did not
            // count the first one's late lists.
            for e in st.entries.iter_mut().filter(|e| e.tightened) {
                e.stats.bounds.clone_from(&e.fallback);
                e.tightened = false;
                e.steady = false;
                e.chained = false;
            }
        }
        for e in &st.entries {
            if !e.zero_cmds {
                continue;
            }
            for (l, &b) in e.lists.iter().zip(&e.stats.bounds) {
                if b > 0 {
                    enc.clear_buffer(&l.draws, l.offset, Some(b as u64 * l.stride));
                }
            }
        }
    }

    /// At the end of the frame's commands: copy every count buffer into a free
    /// read-back (none free: this frame is not read back).
    pub fn encode_copies(&self, enc: &mut wgpu::CommandEncoder) {
        let mut guard = self.state.lock().unwrap();
        let st = &mut *guard;
        let (seq, step) = (st.seq, st.step);
        st.seq += 1;
        for e in st.entries.iter_mut() {
            // Record what was actually encoded, after any recovery cull.
            if e.lists.iter().zip(&e.stats.bounds).all(|(l, b)| l.max == *b) {
                e.stats.full_frames += 1;
            }
            for (peak, bound) in e.stats.peak_bounds.iter_mut().zip(&e.stats.bounds) {
                *peak = (*peak).max(*bound);
            }
            e.stats.frames += 1;
            for (sum, bound) in e.stats.encoded.iter_mut().zip(&e.stats.bounds) {
                *sum += *bound as u64;
            }
            e.full_left = e.full_left.saturating_sub(1);
            e.stats.steady_frames += e.steady as u64;
            e.stats.chained_frames += e.chained as u64;
            let bounds = e.stats.bounds.clone();
            e.log.push_back(Frame { seq, bounds: bounds.clone(), occlusion: step.is_some_and(|s| s.occlusion), quiet: step.is_some_and(|s| s.quiet) });
            if e.log.len() > LOG_FRAMES {
                e.log.pop_front();
            }
            if let Some(rb) = e.readbacks.iter_mut().find(|rb| rb.stage.load(Ordering::Acquire) == FREE) {
                enc.copy_buffer_to_buffer(&e.counts, 0, &rb.buf, 0, e.copy_bytes);
                rb.bounds = bounds;
                rb.seq = seq;
                rb.turn = step.map_or(-1.0, |s| s.turn);
                rb.step = step.map_or(-1.0, |s| s.step);
                rb.stage.store(COPIED, Ordering::Release);
            }
        }
    }

    /// After a cut, recover every list until the read-backs see the new view
    /// (and with `CutWindow`, start each list's window afresh with them).
    pub fn force_full(&self) {
        Self::cut(&mut self.state.lock().unwrap());
    }

    /// A cut (`force_full`, or with `CutWindow` the occlusion turning on or off).
    fn cut(st: &mut State) {
        for e in st.entries.iter_mut() {
            e.full_left = e.full_left.max(FULL_FRAMES);
            if st.cut_window {
                e.view_from = st.seq + 1; // the next frame encoded is the cut's own
                e.stale = !e.history.is_empty();
            }
        }
    }

    /// Every registered count buffer's stats.
    pub fn stats(&self) -> Vec<Stats> {
        self.state.lock().unwrap().entries.iter().map(|e| e.stats.clone()).collect()
    }
}

impl CountedLists {
    /// This frame's bound of list `list` (in registration order).
    pub fn bound(&self, list: usize) -> u32 {
        self.counts.state.lock().unwrap().entries[self.id].stats.bounds[list]
    }

    /// Whether a read-back since the last call found a count past the bound it
    /// was drawn with (some objects were not drawn in that frame).
    pub fn take_overflow(&self) -> bool {
        std::mem::take(&mut self.counts.state.lock().unwrap().entries[self.id].overflow_pending)
    }

    /// Raise this frame's bound of list `list` to at least `n` (its capacity
    /// at most), for an owner that knows the count ahead of the read-backs
    /// (the scene probe's face, from its count pass: `probe::ProbeFaceCounts`).
    /// Call it after `Counts::begin_frame` and before the frame's commands:
    /// `encode_zero` zeroes each list up to its bound and `encode_copies`
    /// records the bound the overflow check compares with. Never lowers one.
    pub fn raise_bound(&self, list: usize, n: u32) {
        let mut st = self.counts.state.lock().unwrap();
        let e = &mut st.entries[self.id];
        let n = n.min(e.lists[list].max);
        e.stats.bounds[list] = e.stats.bounds[list].max(n);
        // (`relax` puts the fallback back: it keeps the raise too)
        if let Some(b) = e.fallback.get_mut(list) {
            *b = (*b).max(n);
        }
    }
}

/// `multi_draw_indexed_indirect_count`, or with the emulation (`lists` set)
/// list `list` drawn to its bound.
#[allow(clippy::too_many_arguments)]
pub fn draw_indexed(pass: &mut wgpu::RenderPass<'_>, lists: Option<&CountedLists>, list: usize, draws: &wgpu::Buffer, offset: u64, counts: &wgpu::Buffer, count_offset: u64, max: u32) {
    match lists {
        None => pass.multi_draw_indexed_indirect_count(draws, offset, counts, count_offset, max),
        Some(c) => {
            let b = c.bound(list).min(max);
            if b > 0 {
                pass.multi_draw_indexed_indirect(draws, offset, b);
            }
        }
    }
}

/// `multi_draw_indirect_count` (non-indexed), or with the emulation list
/// `list` drawn to its bound.
#[allow(clippy::too_many_arguments)]
pub fn draw(pass: &mut wgpu::RenderPass<'_>, lists: Option<&CountedLists>, list: usize, draws: &wgpu::Buffer, offset: u64, counts: &wgpu::Buffer, count_offset: u64, max: u32) {
    match lists {
        None => pass.multi_draw_indirect_count(draws, offset, counts, count_offset, max),
        Some(c) => {
            let b = c.bound(list).min(max);
            if b > 0 {
                pass.multi_draw_indirect(draws, offset, b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_visibility_can_transfer_to_the_early_pass() {
        // A camera rising from behind an occluder can transfer the entire
        // late set at once, before a newer early count arrives from the GPU.
        let capacity = 8192;
        assert!(draw_reserve(0, capacity) < 1200);
        assert!(early_reserve(0, 1200, capacity) >= 1200);
        assert_eq!(early_reserve(150, 0, capacity), draw_reserve(150, capacity));
        assert_eq!(early_reserve(800, 400, 1000), 1000);
    }

    const CAP: u32 = 10_000;
    const LISTS: [(Option<usize>, u32); 2] = [(None, CAP), (Some(0), CAP)];

    /// An early list (0) and its late one (1): 64 frames read back with
    /// `early` and `late` counts, each turning `turn`; the newest is frame 20,
    /// frames 20 to 22 were encoded with the late list at `late_bound`; this is
    /// frame 23.
    struct Case {
        history: VecDeque<Vec<u32>>,
        turns: VecDeque<f32>,
        steps: VecDeque<f32>,
        newest: (u64, Vec<u32>),
        log: VecDeque<Frame>,
    }

    fn case_with_step(early: u32, late: u32, turn: f32, step: f32, late_bound: u32) -> Case {
        let log = (20..23).map(|seq| Frame { seq, bounds: vec![9_000, late_bound], occlusion: true, quiet: true }).collect();
        Case {
            history: (0..WINDOW).map(|_| vec![early, late]).collect(),
            turns: (0..WINDOW).map(|_| turn).collect(),
            steps: (0..WINDOW).map(|_| step).collect(),
            newest: (20, vec![early, late]),
            log,
        }
    }

    fn case(early: u32, late: u32, turn: f32, late_bound: u32) -> Case {
        case_with_step(early, late, turn, 0.0, late_bound)
    }

    /// The bounds before the motion-aware ones (`begin_frame`): the reserves, the early floor.
    fn today(early: u32, late: u32) -> Vec<u32> {
        let e = draw_reserve(early, CAP).max(early_reserve(early, late, CAP));
        vec![e, draw_reserve(late, CAP).max(e).min(CAP)]
    }

    fn run(c: &Case, step: ViewStep, seq: u64) -> (Vec<u32>, (bool, bool)) {
        let mut b = today(c.newest.1[0], c.newest.1[1]);
        let seen = Seen { history: &c.history, turns: &c.turns, steps: &c.steps, newest: Some(&c.newest), log: &c.log };
        let took = motion_bounds(&LISTS, &seen, &mut b, step, seq, MARGIN);
        (b, took)
    }

    const STILL: ViewStep = ViewStep { turn: 0.008, step: 0.0, quiet: true, stream_added: None, occlusion: true };

    #[test]
    fn a_steady_turn_draws_the_late_list_to_its_own_reserve() {
        let c = case(6_000, 20, 0.008, 286);
        let (b, took) = run(&c, STILL, 23);
        assert_eq!(took, (true, true));
        assert_eq!(b[1], draw_reserve(20, CAP), "the late list's own reserve");
        // the newest early and late counts and the late bounds of frames 21 and 22.
        assert_eq!(b[0], 6_000 + 20 + 2 * 286 + MARGIN);
        assert!(b[0] < today(6_000, 20)[0]);
    }

    #[test]
    fn a_turn_past_the_window_or_the_limit_keeps_the_floor() {
        let c = case(6_000, 20, 0.008, 286);
        let (b, took) = run(&c, ViewStep { turn: 0.02, ..STILL }, 23);
        assert_eq!(b[1], today(6_000, 20)[1], "a faster turn than any read back");
        assert!(took.1, "the early list's exact bound does not depend on the turn");
        let fast = case(6_000, 20, 0.5, 286);
        let (b, _) = run(&fast, ViewStep { turn: STEADY_TURN_MAX * 1.5, ..STILL }, 23);
        assert_eq!(b[1], today(6_000, 20)[1], "a turn past STEADY_TURN_MAX, even one the window saw");
    }

    #[test]
    fn a_step_a_write_or_the_occlusion_off_keeps_todays_bounds() {
        let c = case(6_000, 20, 0.008, 286);
        let (b, _) = run(&c, ViewStep { step: 0.5, ..STILL }, 23);
        assert_eq!(b[1], today(6_000, 20)[1], "the eye moved");
        let (b, took) = run(&c, ViewStep { quiet: false, ..STILL }, 23);
        assert_eq!((b, took), (today(6_000, 20), (false, false)), "a streamed write");
        let (b, took) = run(&c, ViewStep { occlusion: false, ..STILL }, 23);
        assert_eq!((b, took), (today(6_000, 20), (false, false)), "no occlusion this frame");
        let mut off = case(6_000, 20, 0.008, 286);
        off.log.back_mut().unwrap().occlusion = false;
        let (b, took) = run(&off, STILL, 23);
        assert_eq!((b, took), (today(6_000, 20), (false, false)), "no occlusion last frame");
        let mut wrote = case(6_000, 20, 0.008, 286);
        wrote.log[1].quiet = false;
        let (b, took) = run(&wrote, STILL, 23);
        assert_eq!(b[0], today(6_000, 20)[0], "a write since the read-back: no exact early bound");
        assert!(took.0);
    }

    #[test]
    fn a_stale_or_recovered_read_back_keeps_the_floor() {
        let mut c = case(6_000, 20, 0.008, 286);
        c.newest.0 = 23 - STALE_FRAMES - 1;
        let (b, took) = run(&c, STILL, 23);
        assert_eq!((b, took), (today(6_000, 20), (false, false)), "stale, and frames since it not logged");
        let mut r = case(6_000, 20, 0.008, 286);
        r.turns[3] = -1.0;
        let (b, _) = run(&r, STILL, 23);
        assert_eq!(b[1], today(6_000, 20)[1], "the recovery's counts in the window");
    }

    #[test]
    fn the_exact_early_bound_covers_every_early_count() {
        // Objects come into view and leave it (a pseudo-random walk); the early
        // pass draws what the last late pass found visible, the late pass the
        // rest. Read-backs arrive three frames late, the late lists drawn to
        // their counts (no overflow): the early count never passes the bound.
        let n = 4_000usize;
        let mut seed = 0x9e37_79b9_u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut flags = vec![false; n];
        let mut counts: Vec<[u32; 2]> = vec![];
        let mut log: VecDeque<Frame> = VecDeque::new();
        let mut start = 0usize;
        for frame in 0..400u64 {
            if frame % 50 == 0 {
                start = (rand() % n as u64) as usize;
            }
            start = (start + (rand() % 7) as usize) % n;
            let len = 1_500 + (rand() % 400) as usize;
            let visible: Vec<bool> = (0..n).map(|i| (i + n - start) % n < len && rand() % 50 != 0).collect();
            let early = (0..n).filter(|&i| flags[i] && (i + n - start) % n < len).count() as u32;
            let late = (0..n).filter(|&i| visible[i] && !flags[i]).count() as u32;
            if frame >= 4 {
                let k = frame - 3;
                let newest = (k, counts[k as usize].to_vec());
                let history: VecDeque<Vec<u32>> = vec![newest.1.clone()].into();
                let turns: VecDeque<f32> = vec![0.0].into();
                let steps: VecDeque<f32> = vec![0.0].into();
                let seen = Seen { history: &history, turns: &turns, steps: &steps, newest: Some(&newest), log: &log };
                let mut b = vec![CAP, CAP];
                motion_bounds(&LISTS, &seen, &mut b, ViewStep { turn: 0.0, ..STILL }, frame, MARGIN);
                assert!(early <= b[0], "frame {frame}: early {early} past its exact bound {}", b[0]);
                assert!(b[0] < CAP);
            }
            counts.push([early, late]);
            log.push_back(Frame { seq: frame, bounds: vec![early, late], occlusion: true, quiet: true });
            if log.len() > LOG_FRAMES {
                log.pop_front();
            }
            flags = visible;
        }
    }

    #[test]
    fn a_steady_move_tightens_late_list_and_sudden_step_falls_back() {
        let c = case_with_step(6_000, 20, 0.008, 3.5, 296);
        let (b, took) = run(&c, ViewStep { step: 3.5, ..STILL }, 23);
        assert_eq!(took, (true, true));
        assert_eq!(b[1], 2 * 20 + MARGIN, "moving steady frame keeps 2x peak + margin");
        assert!(b[1] < today(6_000, 20)[1]);
        // Accelerating past window_step + STEP_SLACK_M keeps the early floor.
        let (b_fast, took_fast) = run(&c, ViewStep { step: 4.0, ..STILL }, 23);
        assert_eq!(b_fast[1], today(6_000, 20)[1]);
        assert!(!took_fast.0);
        // Step past STEADY_MOVE_MAX_M keeps the early floor even if seen in window.
        let c_far = case_with_step(6_000, 20, 0.008, STEADY_MOVE_MAX_M + 1.0, 296);
        let (b_far, took_far) = run(&c_far, ViewStep { step: STEADY_MOVE_MAX_M + 1.0, ..STILL }, 23);
        assert_eq!(b_far[1], today(6_000, 20)[1]);
        assert!(!took_far.0);
    }

    #[test]
    fn an_incremental_stream_write_keeps_steady_with_headroom() {
        let c = case_with_step(6_000, 20, 0.008, 18.0, 296);
        // Incremental streaming delta keeps late tightening with exact added headroom,
        // while keeping the early list conservative (not chained).
        let (b, took) = run(&c, ViewStep { step: 18.0, quiet: false, stream_added: Some(48), ..STILL }, 23);
        assert_eq!(took, (true, false));
        assert_eq!(b[1], 2 * 20 + MARGIN + 48);
        assert_eq!(b[0], today(6_000, 20)[0]);
        // Larger tile-sized incremental streaming delta still tightens when own < early floor.
        let (b_mid, took_mid) = run(&c, ViewStep { step: 18.0, quiet: false, stream_added: Some(MARGIN + 100), ..STILL }, 23);
        assert_eq!(took_mid, (true, false));
        assert_eq!(b_mid[1], 2 * 20 + MARGIN + MARGIN + 100);
        assert_eq!(b_mid[0], today(6_000, 20)[0]);
        // Huge streaming delta past the early floor falls back to today's bound.
        let (b_big, took_big) = run(&c, ViewStep { step: 18.0, quiet: false, stream_added: Some(today(6_000, 20)[1]), ..STILL }, 23);
        assert_eq!((b_big, took_big), (today(6_000, 20), (false, false)));
    }
}
