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
//! A list may take its floor from another list's bound (the late
//! opaque list from the opaque list: after a fast turn every visible object
//! can be late). A list can therefore miss objects only in the frames between
//! a jump of its count past its bound and the read-back that sees it; `Stats`
//! counts those overflows, and `take_overflow` tells the list's owner (the
//! probe captures its faces again).
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
}

struct Entry {
    counts: wgpu::Buffer,
    /// Bytes copied from the count buffer (up to the last list's word).
    copy_bytes: u64,
    lists: Vec<List>,
    readbacks: Vec<Readback>,
    /// Per frame read back, the count of every list (newest last).
    history: VecDeque<Vec<u32>>,
    /// Encoded frames still requiring conservative recovery counts.
    full_left: u64,
    /// An overflow not yet taken by the owner (`take_overflow`).
    overflow_pending: bool,
    stats: Stats,
}

struct State {
    entries: Vec<Entry>,
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

fn draw_reserve(visible: u32, capacity: u32) -> u32 {
    visible.saturating_add(visible / 2).saturating_add(MARGIN).min(capacity)
}

fn early_reserve(early: u32, late: u32, capacity: u32) -> u32 {
    // The passes partition the same opaque set. Every object the late pass
    // discovered can already be in the early pass by the next read-back.
    draw_reserve(early.saturating_add(late), capacity)
}

impl Counts {
    pub fn new(device: &wgpu::Device) -> Arc<Self> {
        Arc::new(Self { device: device.clone(), state: Mutex::new(State { entries: vec![] }) })
    }

    /// Register a count buffer (it needs `COPY_SRC`) and the lists it counts.
    pub fn register(self: &Arc<Self>, label: &'static str, counts: &wgpu::Buffer, lists: Vec<List>) -> CountedLists {
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
            })
            .collect();
        let n = lists.len();
        let full: Vec<u32> = lists.iter().map(|l| l.max).collect();
        let mut st = self.state.lock().unwrap();
        st.entries.push(Entry {
            counts: counts.clone(),
            copy_bytes,
            lists,
            readbacks,
            history: VecDeque::new(),
            full_left: 0,
            overflow_pending: false,
            stats: Stats { label, bounds: full, peak_counts: vec![0; n], peak_bounds: vec![0; n], encoded: vec![0; n], ..Default::default() },
        });
        CountedLists { counts: self.clone(), id: st.entries.len() - 1 }
    }

    /// Per frame, before any list is zeroed or drawn: harvest the read-backs
    /// and fix every list's bound for this frame.
    pub fn begin_frame(&self) {
        let mut st = self.state.lock().unwrap();
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
                        e.history.push_back(counts);
                        if e.history.len() > WINDOW {
                            e.history.pop_front();
                        }
                    }
                    _ => {}
                }
            }
            let full = e.history.is_empty() || e.full_left > 0;
            let mut bounds: Vec<u32> = e
                .lists
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    if full {
                        l.max
                    } else {
                        let peak = e.history.iter().map(|h| h[i]).max().unwrap_or(0);
                        draw_reserve(peak, l.max)
                    }
                })
                .collect();
            for (i, l) in e.lists.iter().enumerate() {
                if let Some(f) = l.floor {
                    if let Some(last) = e.history.back() {
                        bounds[f] = bounds[f].max(early_reserve(last[f], last[i], e.lists[f].max));
                    }
                    bounds[i] = bounds[i].max(bounds[f]).min(l.max);
                }
            }
            for (i, b) in bounds.into_iter().enumerate() {
                e.stats.bounds[i] = b;
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
            if !state.entries.iter().any(|e| e.history.is_empty() || e.full_left != 0) {
                return;
            }
            state.entries.iter().map(|e| (e.counts.clone(), e.copy_bytes)).collect()
        };
        let size: u64 = sources.iter().map(|(_, bytes)| bytes).sum();
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("count recovery"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        cull(&mut enc);
        let mut offset = 0;
        for (source, bytes) in &sources {
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
            for (e, (_, bytes)) in state.entries.iter_mut().zip(&sources) {
                if e.history.is_empty() || e.full_left > 0 {
                    let counts: Vec<u32> = e.lists.iter().map(|l| words[start + l.word as usize].min(l.max)).collect();
                    for (i, l) in e.lists.iter().enumerate() {
                        let upper = l.floor.map_or(counts[i], |early| counts[early].max(counts[i]));
                        e.stats.bounds[i] = draw_reserve(upper, l.max);
                    }
                    if e.history.is_empty() {
                        e.history.push_back(counts);
                    }
                }
                start += *bytes as usize / 4;
            }
        }
        buf.unmap();
    }

    /// At the start of the frame's commands, before any cull: zero every list
    /// up to its bound, so the slots past this frame's count draw nothing.
    pub fn encode_zero(&self, enc: &mut wgpu::CommandEncoder) {
        let st = self.state.lock().unwrap();
        for e in &st.entries {
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
        let mut st = self.state.lock().unwrap();
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
            let bounds = e.stats.bounds.clone();
            if let Some(rb) = e.readbacks.iter_mut().find(|rb| rb.stage.load(Ordering::Acquire) == FREE) {
                enc.copy_buffer_to_buffer(&e.counts, 0, &rb.buf, 0, e.copy_bytes);
                rb.bounds = bounds;
                rb.stage.store(COPIED, Ordering::Release);
            }
        }
    }

    /// After a cut, recover every list until the read-backs see the new view.
    pub fn force_full(&self) {
        let mut st = self.state.lock().unwrap();
        for e in st.entries.iter_mut() {
            e.full_left = e.full_left.max(FULL_FRAMES);
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
}
