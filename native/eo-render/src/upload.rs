//! Budgeted uploads: every streamed byte (kit texture layers, Blender models,
//! kit building packs) goes through one FIFO queue that writes at most
//! `budget` bytes per frame. A job's bulk writes (vertices, indices, texture
//! mips) are split across frames as needed; its `finish` writes (object table
//! entries, LOD-group readiness) are applied in the frame its last bulk byte
//! is written, so nothing is drawn before all of its data is on the GPU.
//!
//! Slice 2 measured 17-39 ms render-thread hitches when a batch of models (or
//! four kit texture layers) was written in one call; this replaces that.
//!
//! the bulk buffer writes are staged by default: the render thread
//! only reserves room in a persistent, mapped staging chunk and hands the
//! bytes to a copier thread; once the copier is done, the chunk's GPU copies
//! are submitted (next pump) and only then is the job finished (`Stager`).
//! Texture mips go the same way, in bands of whole rows (each band one
//! `copy_buffer_to_texture` from the chunk, rows at the copy's 256-byte
//! alignment), so a mip larger than what is left of the budget spreads over
//! pumps under the same byte and time budget as the buffers. A new streamed
//! texture needs only a `Target` and a `Renderer::upload_texture` arm.

use crate::Renderer;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Where a write lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Vertex,
    Index,
    Object,
    Group,
    /// the per-LOD-group appearance buffer.
    Age,
    /// Kit texture array `map` (0 albedo, 1 normal, 2 ORM), `layer`, `mip`.
    Texture { map: u8, layer: u32, mip: u32 },
    /// the street prop instance arena (`props::PropsGpu`).
    PropInstances,
    /// the wire segment arena (`props::PropsGpu::wire_buffer`).
    PropWires,
    /// the lamp-light slots (`props::PropsGpu::lights_buffer`).
    PropLights,
    /// one mip of one layer of a CC0 array (0-2 the material maps, 3-5
    /// the decal maps), BC blocks. Producers push offset 0 and the whole mip;
    /// the queue bands it in whole block rows through the stager, as the kit maps.
    Cc0Texture { array: u8, layer: u16, mip: u8 },
    /// the street decal grid (headers, cells) and the decal records (byte offsets).
    Cc0Grid,
    Cc0Decals,
}

/// A write's payload, owned in its natural type so nothing is copied on the
/// render thread to turn it into bytes.
pub enum Data {
    Bytes(Vec<u8>),
    Vertices(Vec<crate::scene::Vertex>),
    Words(Vec<u32>),
    Objects(Vec<crate::scene::GpuObject>),
}

impl Data {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Data::Bytes(v) => v,
            Data::Vertices(v) => bytemuck::cast_slice(v),
            Data::Words(v) => bytemuck::cast_slice(v),
            Data::Objects(v) => bytemuck::cast_slice(v),
        }
    }
}

pub struct Write {
    pub target: Target,
    /// Byte offset into the buffer (for a texture, into the mip at a
    /// whole row; producers push 0 and the whole mip, and the queue bands it).
    pub offset: u64,
    pub data: Data,
}

pub struct Job {
    /// Caller's tag, returned when the job completes.
    pub tag: u64,
    pub bulk: Vec<Write>,
    /// Applied when the last bulk byte is out (small: objects, groups).
    pub finish: Vec<Write>,
    next: usize,
    at: usize,
    /// the queue's id for the job (the stager's pieces carry it).
    id: u64,
    /// staged pieces whose GPU copies are not submitted yet.
    staged: u32,
    /// the bulk write being staged, shared with the copier's pieces.
    cur: Option<Arc<Data>>,
}

impl Job {
    pub fn new(tag: u64, bulk: Vec<Write>, finish: Vec<Write>) -> Self {
        Self { tag, bulk, finish, next: 0, at: 0, id: 0, staged: 0, cur: None }
    }
    pub fn bytes(&self) -> u64 {
        self.bulk.iter().chain(&self.finish).map(|w| w.data.bytes().len() as u64).sum()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PumpStats {
    pub bytes: u64,
    pub writes: u32,
    pub jobs_done: u32,
}

pub struct UploadQueue {
    jobs: VecDeque<Job>,
    /// Bytes allowed per `pump` (one per frame).
    pub budget: u64,
    pub max_bytes_in_a_pump: u64,
    pub total_bytes: u64,
    /// no new write starts once a pump has spent this long (at least one write a pump).
    pub time_budget: std::time::Duration,
    /// stage the bulk buffer writes (`Stager`; false: `queue.write_buffer` on this thread, as before).
    pub ring: bool,
    stager: Option<Stager>,
    next_id: u64,
}

/// largest single buffer write (so the time budget can stop between pieces).
pub const WRITE_PIECE: u64 = 1 << 20;
/// default pump time budget.
pub const DEFAULT_TIME_BUDGET: std::time::Duration = std::time::Duration::from_micros(1000);

impl UploadQueue {
    pub fn new(budget: u64) -> Self {
        Self { jobs: VecDeque::new(), budget: budget.max(4096), max_bytes_in_a_pump: 0, total_bytes: 0, time_budget: DEFAULT_TIME_BUDGET, ring: true, stager: None, next_id: 1 }
    }

    /// stage the bulk writes (true) or write them with `queue.write_buffer` here (false).
    pub fn with_ring(mut self, on: bool) -> Self {
        self.ring = on;
        self
    }

    pub fn push(&mut self, mut job: Job) {
        job.id = self.next_id;
        self.next_id += 1;
        self.jobs.push_back(job);
    }

    /// Jobs not finished (queued, or with bulk bytes still on their way to the GPU).
    pub fn pending(&self) -> usize {
        self.jobs.len()
    }

    pub fn pending_bytes(&self) -> u64 {
        self.jobs.iter().map(|j| j.bytes()).sum()
    }

    /// the stager's chunks (mapped and ready, all), times it grew, bytes staged.
    pub fn stager_stats(&self) -> Option<(usize, usize, u64, u64)> {
        self.stager.as_ref().map(|s| {
            let (ready, n) = s.chunks();
            (ready, n, s.grew, s.bytes)
        })
    }

    /// Drop queued jobs the caller no longer wants (not started, or started:
    /// a started job's partial bulk is harmless, its finish never applied).
    /// a staged job's pieces still being copied are voided (their GPU
    /// copies are never submitted, so a range freed here and reused later is
    /// written only by its new owner).
    pub fn cancel(&mut self, mut keep: impl FnMut(u64) -> bool) -> Vec<u64> {
        let _free = crate::hitch::span(crate::hitch::Phase::PayloadFree); // hitch
        let mut dropped = vec![];
        // cancelled payloads (whole packs) are freed off the render thread.
        let mut kept = VecDeque::with_capacity(self.jobs.len());
        for mut j in self.jobs.drain(..) {
            if keep(j.tag) {
                kept.push_back(j);
            } else {
                dropped.push(j.tag);
                if j.staged > 0 {
                    if let Some(s) = self.stager.as_mut() {
                        s.void_job(j.id);
                    }
                }
                if let Some(a) = j.cur.take() {
                    release(a);
                }
                bury(Box::new(j));
            }
        }
        self.jobs = kept;
        dropped
    }

    /// Write up to `budget` bytes. Buffer writes split at 4-byte boundaries;
    /// a texture mip at whole rows (it was written whole, so a mip
    /// larger than the budget went out in one pump).
    /// Returns the stats and the tags of jobs completed this call.
    pub fn pump(&mut self, renderer: &Renderer, queue: &wgpu::Queue) -> (PumpStats, Vec<u64>) {
        if !self.ring {
            return self.pump_direct(renderer, queue);
        }
        self.pump_staged(renderer, queue)
    }

    /// The bulk writes with `queue.write_buffer` on this thread (before hitch2; `--no-upload-ring`).
    fn pump_direct(&mut self, renderer: &Renderer, queue: &wgpu::Queue) -> (PumpStats, Vec<u64>) {
        let mut st = PumpStats::default();
        let mut done = vec![];
        let budget = self.budget;
        // a time budget besides the byte budget. Under CPU contention a
        // 3.9 MB frame measured 3.5 ms inside write_buffer (+2.7 ms finish,
        // 1.0 ms submit); buffer writes go in pieces of at most WRITE_PIECE
        // and no new piece starts once the pump has spent `time_budget`.
        let t0 = std::time::Instant::now();
        let time_budget = self.time_budget;
        let mut out_of_time = false;
        while let Some(job) = self.jobs.front_mut() {
            while job.next < job.bulk.len() {
                if st.bytes > 0 && t0.elapsed() >= time_budget {
                    out_of_time = true;
                    break;
                }
                let left = budget - st.bytes;
                if left == 0 {
                    break;
                }
                let w = &job.bulk[job.next];
                let remain = w.data.bytes().len() - job.at;
                match w.target {
                    Target::Texture { .. } | Target::Cc0Texture { .. } => {
                        // whole rows, as many as the budget has room
                        // for (one at least): a mip larger than the budget
                        // goes over several pumps instead of in one.
                        let row = renderer.upload_texture(w.target).filter(|t| t.bytes() == w.data.bytes().len() as u64).map(|t| t.row_bytes()).unwrap_or(remain as u64);
                        let rows = (left / row).min(WRITE_PIECE.max(row) / row);
                        if rows == 0 && st.bytes > 0 {
                            break;
                        }
                        let take = (rows.max(1) * row).min(remain as u64) as usize;
                        renderer.write(queue, w.target, w.offset + job.at as u64, &w.data.bytes()[job.at..job.at + take]);
                        st.bytes += take as u64;
                        st.writes += 1;
                        job.at += take;
                        if job.at >= w.data.bytes().len() {
                            // Free each payload as it goes out: dropping a whole
                            // finished job at once (hundreds of MB of texture mips)
                            // was a 15 ms stall in one frame.
                            free_payload(&mut job.bulk[job.next].data); // hitch
                            job.next += 1;
                            job.at = 0;
                        }
                    }
                    _ => {
                        let take = (remain as u64).min(left).min(WRITE_PIECE) as usize & !3usize; // pieces
                        if take == 0 {
                            break;
                        }
                        renderer.write(queue, w.target, w.offset + job.at as u64, &w.data.bytes()[job.at..job.at + take]);
                        st.bytes += take as u64;
                        st.writes += 1;
                        job.at += take;
                        if job.at >= w.data.bytes().len() {
                            free_payload(&mut job.bulk[job.next].data); // hitch
                            job.next += 1;
                            job.at = 0;
                        }
                    }
                }
            }
            if job.next < job.bulk.len() || out_of_time {
                break;
            }
            let finish_bytes: u64 = job.finish.iter().map(|w| w.data.bytes().len() as u64).sum();
            if st.bytes > 0 && st.bytes + finish_bytes > budget {
                break;
            }
            for w in &job.finish {
                renderer.write(queue, w.target, w.offset, w.data.bytes());
                st.writes += 1;
            }
            st.bytes += finish_bytes;
            st.jobs_done += 1;
            done.push(job.tag);
            crate::hitch::count(crate::hitch::Event::JobDone, 1); // hitch
            self.jobs.pop_front();
        }
        self.max_bytes_in_a_pump = self.max_bytes_in_a_pump.max(st.bytes);
        self.total_bytes += st.bytes;
        (st, done)
    }

    /// one pump with the stager. (1) Chunks whose bytes the copier has
    /// written are copied out in one submission. (2) Jobs whose every bulk
    /// byte is now submitted get their finish writes (`queue.write_buffer`:
    /// they reach the GPU at the next submission, after those copies) and are
    /// done. (3) Up to the budget of new bytes is staged: the render thread
    /// only reserves room in a mapped chunk and hands the copy to the copier
    /// thread (measured before: ~0.4 ms of render-thread time per MiB copied
    /// into the mapped staging memory, `write_buffer`'s own memcpy).
    fn pump_staged(&mut self, renderer: &Renderer, queue: &wgpu::Queue) -> (PumpStats, Vec<u64>) {
        let mut st = PumpStats::default();
        let mut done = vec![];
        let budget = self.budget;
        let t0 = std::time::Instant::now();
        let stager = self.stager.get_or_insert_with(|| Stager::new(renderer.gpu_device(), RING_PREWARM_CHUNKS));
        // (1) Retire what the copier finished.
        for id in stager.retire(renderer, queue, budget) {
            if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                j.staged = j.staged.saturating_sub(1);
            }
        }
        // (2) Finish the jobs at the front whose bulk is all on its way (their
        // finish writes count against this pump's budget, as before).
        while let Some(job) = self.jobs.front_mut() {
            if job.next < job.bulk.len() || job.staged > 0 {
                break;
            }
            let fb: u64 = job.finish.iter().map(|w| w.data.bytes().len() as u64).sum();
            if st.bytes > 0 && st.bytes + fb > budget {
                break;
            }
            for w in &job.finish {
                renderer.write(queue, w.target, w.offset, w.data.bytes());
                st.writes += 1;
            }
            st.bytes += fb;
            st.jobs_done += 1;
            done.push(job.tag);
            crate::hitch::count(crate::hitch::Event::JobDone, 1); // hitch
            if let Some(j) = self.jobs.pop_front() {
                if j.finish.iter().any(|w| w.data.bytes().len() >= GRAVEYARD_MIN_BYTES) {
                    bury(Box::new(j));
                }
            }
        }
        // (3) Stage new bytes, jobs in order.
        let mut staged_bytes = 0u64;
        'jobs: for job in self.jobs.iter_mut() {
            while job.next < job.bulk.len() {
                if staged_bytes > 0 && t0.elapsed() >= self.time_budget {
                    break 'jobs;
                }
                let left = budget.saturating_sub(st.bytes + staged_bytes);
                if left == 0 {
                    break 'jobs;
                }
                let target = job.bulk[job.next].target;
                let offset = job.bulk[job.next].offset;
                // (once staging started, the payload lives in `cur`)
                let total = job.cur.as_ref().map(|a| a.bytes().len()).unwrap_or_else(|| job.bulk[job.next].data.bytes().len());
                let remain = total - job.at;
                // Where this pump's piece goes, and its shape: a texture mip in
                // whole rows (a band that fits what is left of the budget, one
                // row at least), a buffer range in 4-byte words.
                let plan = if let Some(t) = renderer.upload_texture(target).filter(|t| t.bytes() == total as u64 && t.row_bytes() <= RING_CHUNK_BYTES) {
                    let row = t.row_bytes();
                    let stride = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64);
                    if left < row && st.bytes + staged_bytes > 0 {
                        break 'jobs;
                    }
                    // (a band of a block-compressed mip is whole block rows: `row` is one row of blocks.)
                    let rows = (remain as u64 / row).min((left / row).max(1)).min((RING_CHUNK_BYTES - TEXTURE_PIECE_ALIGN - row) / stride + 1);
                    let y0 = (job.at as u64 / row) as u32;
                    let tex = Dst::Texture { tex: t.texture.clone(), mip: t.mip, layer: t.layer, y0, width: t.width, rows: rows as u32, bytes_per_row: stride as u32, block: t.block };
                    Some((Rows { from: job.at, row, rows, src_stride: row, dst_stride: stride }, tex))
                } else if let Some(buf) = renderer.upload_buffer(target).filter(|_| remain % 4 == 0 && offset % 4 == 0) {
                    let take = (remain as u64).min(left).min(RING_CHUNK_BYTES) & !3;
                    if take == 0 {
                        break 'jobs;
                    }
                    Some((Rows::flat(job.at, take), Dst::Buffer { buf: buf.clone(), off: offset + job.at as u64 }))
                } else {
                    None
                };
                let Some((rows, dst)) = plan else {
                    // A texture of another shape, or bytes the GPU copy cannot take: written here, as before.
                    let w = &job.bulk[job.next];
                    let texture = matches!(w.target, Target::Texture { .. } | Target::Cc0Texture { .. });
                    if texture && remain as u64 > left && st.bytes + staged_bytes > 0 {
                        break 'jobs;
                    }
                    if !texture && job.staged > 0 {
                        // Keep the job's writes in order: wait for its staged pieces.
                        continue 'jobs;
                    }
                    renderer.write(queue, w.target, w.offset + job.at as u64, &w.data.bytes()[job.at..]);
                    staged_bytes += remain as u64;
                    st.writes += 1;
                    free_payload(&mut job.bulk[job.next].data);
                    job.next += 1;
                    job.at = 0;
                    continue;
                };
                let take = rows.bytes();
                let arc = job.cur.get_or_insert_with(|| Arc::new(std::mem::replace(&mut job.bulk[job.next].data, Data::Bytes(Vec::new())))).clone();
                let tw = std::time::Instant::now();
                if !stager.stage(job.id, arc, rows, dst) {
                    break 'jobs; // every chunk is in flight: the rest waits for a later pump
                }
                crate::hitch::note_write_to(crate::hitch_target(target), take, tw.elapsed().as_secs_f64() * 1e3);
                staged_bytes += take;
                st.writes += 1;
                job.staged += 1;
                job.at += take as usize;
                if job.at >= total {
                    if let Some(a) = job.cur.take() {
                        release(a);
                    }
                    job.next += 1;
                    job.at = 0;
                }
            }
        }
        stager.close();
        st.bytes += staged_bytes;
        self.max_bytes_in_a_pump = self.max_bytes_in_a_pump.max(st.bytes);
        self.total_bytes += st.bytes;
        (st, done)
    }
}

// ---------------------------------------------------------------------------
// the stager: persistent mapped chunks, filled by a copier thread.
// ---------------------------------------------------------------------------

/// bytes of one staging chunk (a whole pump's budget fits in one).
pub const RING_CHUNK_BYTES: u64 = 4 << 20;
/// chunks made with the stager (several pumps' worth are in flight
/// while the copier fills them and the GPU copies them out).
pub const RING_PREWARM_CHUNKS: usize = 6;
/// the stager never grows past this; staging waits instead.
pub const RING_MAX_CHUNKS: usize = 12;

const CHUNK_MAPPED: u8 = 0;
const CHUNK_IN_USE: u8 = 1;
const CHUNK_FAILED: u8 = 2;

struct RingChunk {
    buf: wgpu::Buffer,
    /// Bytes handed out since the chunk was last mapped.
    used: u64,
    state: Arc<AtomicU8>,
}

/// A range of a chunk the copier fills.
struct Piece {
    job: u64,
    done: Arc<AtomicBool>,
    /// Kept alive (and the chunk mapped) until the copier is done.
    view: Option<wgpu::BufferViewMut>,
    /// Where the piece starts in its chunk.
    src: u64,
    dst: Dst,
    /// Payload bytes (a texture band's row padding not counted).
    len: u64,
    void: bool,
}

/// Where a piece's GPU copy lands.
enum Dst {
    Buffer { buf: wgpu::Buffer, off: u64 },
    /// `rows` rows of a texture subresource from row `y0`, `bytes_per_row`
    /// apart in the chunk (the copy's 256-byte row alignment). cc0: for a
    /// block-compressed format a row is a row of `block` x `block` blocks.
    Texture { tex: wgpu::Texture, mip: u32, layer: u32, y0: u32, width: u32, rows: u32, bytes_per_row: u32, block: u32 },
}

/// where a texture piece starts in its chunk (a copy's buffer offset is a
/// multiple of the format's block size, 16 bytes for BC; 512 also meets the
/// D3D12 placement alignment, so no backend splits the copy).
pub const TEXTURE_PIECE_ALIGN: u64 = 512;

/// What a piece copies: `rows` runs of `row` bytes, `src_stride` apart in the
/// payload from `from` and `dst_stride` apart in the chunk.
#[derive(Clone, Copy)]
struct Rows {
    from: usize,
    row: u64,
    rows: u64,
    src_stride: u64,
    dst_stride: u64,
}

impl Rows {
    fn flat(from: usize, len: u64) -> Self {
        Self { from, row: len, rows: 1, src_stride: len, dst_stride: len }
    }

    /// Payload bytes.
    fn bytes(&self) -> u64 {
        self.row * self.rows
    }

    /// Chunk bytes, first row to the end of the last.
    fn span(&self) -> u64 {
        self.dst_stride * (self.rows - 1) + self.row
    }
}

/// The mapped address the copier writes to (valid while the piece's view lives).
struct SendPtr(*mut u8);
// SAFETY: the pointer is written by the copier thread only, and the view that
// keeps the mapping alive is dropped (and the chunk unmapped) only after the
// copier has set the piece's `done` flag (Release/Acquire).
unsafe impl Send for SendPtr {}

struct CopyTask {
    dst: SendPtr,
    src: Arc<Data>,
    rows: Rows,
    done: Arc<AtomicBool>,
}

pub struct Stager {
    device: wgpu::Device,
    chunks: Vec<RingChunk>,
    /// The chunk this pump stages into, and its pieces.
    open: Option<(usize, Vec<Piece>)>,
    /// Chunks waiting for the copier, oldest first (submitted in this order).
    closed: VecDeque<(usize, Vec<Piece>)>,
    tx: std::sync::mpsc::Sender<CopyTask>,
    pub grew: u64,
    pub bytes: u64,
}

impl Stager {
    pub fn new(device: &wgpu::Device, prewarm: usize) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<CopyTask>();
        std::thread::Builder::new()
            .name("eo-upload-copier".into())
            .spawn(move || {
                // Normal priority: above the streaming workers, below the render thread.
                while let Ok(t) = rx.recv() {
                    let r = t.rows;
                    for k in 0..r.rows {
                        let at = r.from + (k * r.src_stride) as usize;
                        let src = &t.src.bytes()[at..at + r.row as usize];
                        // SAFETY: `dst` addresses `r.span()` mapped bytes no one else touches until `done`.
                        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), t.dst.0.add((k * r.dst_stride) as usize), src.len()) };
                    }
                    t.done.store(true, Ordering::Release);
                    drop(t.src); // the last piece of a payload frees it here, off the render thread
                }
            })
            .expect("spawn upload copier");
        let mut s = Self { device: device.clone(), chunks: vec![], open: None, closed: VecDeque::new(), tx, grew: 0, bytes: 0 };
        for _ in 0..prewarm {
            s.add_chunk();
        }
        s
    }

    fn add_chunk(&mut self) -> usize {
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upload-stager"),
            size: RING_CHUNK_BYTES,
            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        self.chunks.push(RingChunk { buf, used: 0, state: Arc::new(AtomicU8::new(CHUNK_MAPPED)) });
        self.chunks.len() - 1
    }

    /// Chunks mapped and ready, and all chunks.
    pub fn chunks(&self) -> (usize, usize) {
        (self.chunks.iter().filter(|c| c.state.load(Ordering::Acquire) == CHUNK_MAPPED).count(), self.chunks.len())
    }

    /// Hand `rows` of `src` to the copier, bound for `dst`. False: no chunk
    /// has room (try again next pump).
    fn stage(&mut self, job: u64, src: Arc<Data>, rows: Rows, dst: Dst) -> bool {
        let span = rows.span();
        // a texture piece starts at TEXTURE_PIECE_ALIGN (BC copies need 16-byte offsets).
        let align = if matches!(dst, Dst::Texture { .. }) { TEXTURE_PIECE_ALIGN } else { wgpu::MAP_ALIGNMENT };
        let fits = |used: u64| used.next_multiple_of(align) + span <= RING_CHUNK_BYTES;
        if self.open.as_ref().is_some_and(|(k, _)| !fits(self.chunks[*k].used)) {
            self.close();
        }
        if self.open.is_none() {
            let free = self.chunks.iter().position(|c| c.state.load(Ordering::Acquire) == CHUNK_MAPPED && c.used == 0);
            let k = match free {
                Some(k) => k,
                None if self.chunks.len() < RING_MAX_CHUNKS => {
                    self.grew += 1;
                    crate::hitch::mark(crate::hitch::Mark::RingGrew, 1);
                    crate::hitch::mark(crate::hitch::Mark::ResourceCreated, 1);
                    self.add_chunk()
                }
                None => return false,
            };
            self.chunks[k].state.store(CHUNK_IN_USE, Ordering::Release);
            self.open = Some((k, vec![]));
        }
        let (k, pieces) = self.open.as_mut().unwrap();
        let c = &mut self.chunks[*k];
        let at = c.used.next_multiple_of(align);
        let Ok(mut view) = c.buf.get_mapped_range_mut(at..at + span) else { return false };
        let ptr = view.slice(..).as_raw_ptr().as_ptr() as *mut u8;
        c.used = (at + span).next_multiple_of(wgpu::MAP_ALIGNMENT);
        let done = Arc::new(AtomicBool::new(false));
        if self.tx.send(CopyTask { dst: SendPtr(ptr), src, rows, done: done.clone() }).is_err() {
            return false;
        }
        let len = rows.bytes();
        pieces.push(Piece { job, done, view: Some(view), src: at, dst, len, void: false });
        self.bytes += len;
        crate::hitch::mark(crate::hitch::Mark::RingBytes, len);
        true
    }

    /// End of a pump: the open chunk takes no more pieces.
    fn close(&mut self) {
        if let Some((k, pieces)) = self.open.take() {
            if pieces.is_empty() {
                self.chunks[k].used = 0;
                self.chunks[k].state.store(CHUNK_MAPPED, Ordering::Release);
            } else {
                self.closed.push_back((k, pieces));
            }
        }
    }

    /// A cancelled job: its pieces are never copied to the GPU.
    fn void_job(&mut self, job: u64) {
        for (_, pieces) in self.closed.iter_mut().chain(self.open.iter_mut()) {
            for p in pieces.iter_mut().filter(|p| p.job == job) {
                p.void = true;
            }
        }
    }

    /// Submit the GPU copies of the oldest chunks the copier has finished, in
    /// order; returns the job of every piece submitted (voided ones excluded).
    /// At most `budget` bytes of GPU copies a call (at least one chunk), so a
    /// copier that fell behind does not put several frames' bytes in one.
    fn retire(&mut self, renderer: &Renderer, queue: &wgpu::Queue, budget: u64) -> Vec<u64> {
        let mut jobs = vec![];
        let mut ready = vec![];
        let mut taken = 0u64;
        while self.closed.front().is_some_and(|(_, p)| p.iter().all(|p| p.done.load(Ordering::Acquire))) {
            let b: u64 = self.closed.front().map(|(_, p)| p.iter().filter(|p| !p.void).map(|p| p.len).sum()).unwrap_or(0);
            if !ready.is_empty() && taken + b > budget {
                break;
            }
            taken += b;
            ready.push(self.closed.pop_front().unwrap());
        }
        if ready.is_empty() {
            return jobs;
        }
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("upload-stager") });
        let mut bytes = 0;
        for (k, pieces) in &mut ready {
            for p in pieces.iter_mut() {
                p.view = None;
            }
            self.chunks[*k].buf.unmap();
            let chunk = &self.chunks[*k].buf;
            for p in pieces.iter() {
                if !p.void {
                    match &p.dst {
                        Dst::Buffer { buf, off } => enc.copy_buffer_to_buffer(chunk, p.src, buf, *off, p.len),
                        // `y0` and `rows` count block rows (one texel for the kit's RGBA8); the copy's
                        // origin and extent are texels, its rows per image blocks, its width the mip's physical size.
                        Dst::Texture { tex, mip, layer, y0, width, rows, bytes_per_row, block } => enc.copy_buffer_to_texture(
                            wgpu::TexelCopyBufferInfo { buffer: chunk, layout: wgpu::TexelCopyBufferLayout { offset: p.src, bytes_per_row: Some(*bytes_per_row), rows_per_image: Some(*rows) } },
                            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: *mip, origin: wgpu::Origin3d { x: 0, y: *y0 * *block, z: *layer }, aspect: wgpu::TextureAspect::All },
                            wgpu::Extent3d { width: width.next_multiple_of(*block), height: *rows * *block, depth_or_array_layers: 1 },
                        ),
                    }
                    bytes += p.len;
                }
                jobs.push(p.job);
            }
        }
        crate::pace::submit(queue, [enc.finish()]); // counted (the driver's submission spike, `pace`)
        renderer.note_streamed(bytes);
        for (k, _) in ready {
            let c = &mut self.chunks[k];
            c.used = 0;
            let state = c.state.clone();
            c.buf.map_async(wgpu::MapMode::Write, .., move |r| state.store(if r.is_ok() { CHUNK_MAPPED } else { CHUNK_FAILED }, Ordering::Release));
        }
        jobs
    }
}

/// drop a payload handle; the last one goes to the graveyard (a few MB
/// freed on the render thread measured 0.3-0.4 ms).
fn release(a: Arc<Data>) {
    if let Ok(d) = Arc::try_unwrap(a) {
        if d.bytes().len() >= GRAVEYARD_MIN_BYTES {
            bury(Box::new(d));
        }
    }
}

/// hand a written payload to the graveyard thread (freeing a few MB of
/// pages measured 0.3-0.4 ms on the render thread).
fn free_payload(d: &mut Data) {
    let _s = crate::hitch::span(crate::hitch::Phase::PayloadFree);
    let old = std::mem::replace(d, Data::Bytes(Vec::new()));
    if old.bytes().len() >= GRAVEYARD_MIN_BYTES {
        bury(Box::new(old));
    }
}

/// Payloads smaller than this are freed in place (a channel send costs more).
const GRAVEYARD_MIN_BYTES: usize = 64 << 10;

/// drop `x` on a low-priority thread of its own.
pub fn bury(x: Box<dyn Send>) {
    static GRAVE: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<Box<dyn Send>>>> = std::sync::OnceLock::new();
    let tx = GRAVE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Box<dyn Send>>();
        std::thread::Builder::new()
            .name("eo-upload-graveyard".into())
            .spawn(move || {
                crate::hitch::lower_worker_thread();
                while let Ok(dead) = rx.recv() {
                    drop(dead);
                }
            })
            .expect("spawn upload graveyard");
        std::sync::Mutex::new(tx)
    });
    if let Err(e) = tx.lock().unwrap().send(x) {
        drop(e.0);
    }
}
