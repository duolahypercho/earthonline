//! the process's memory allocator, so the render thread never waits
//! for a heap lock a worker holds.
//!
//! Measured (allocator calls timed on the render thread): single heap
//! allocations of 13-25 KB took 6.7-9.0 ms, the thread off the CPU, in the
//! frames where streaming workers (the props planner's tiles, kit packs)
//! were allocating and freeing heavily: the drive's 13.4 ms and the bay
//! bridge's 9.9 ms frames, and the same shape in `finish` and the props pump
//! (wgpu's and the driver's own allocations on the render thread). Every
//! thread shared the one process heap; its lock is a user-mode lock with no
//! priority inheritance, and the workers run below normal priority.
//!
//! Now three places memory comes from:
//! - **R**, a heap of the render thread's own (`claim_render_thread`): only
//!   the render thread allocates from it and frees into it, so its lock is
//!   never contended. A block of R freed on another thread is pushed onto a
//!   lock-free list the render thread empties at its next allocation.
//! - **W**, one heap for every other thread (workers, cookers, the copier,
//!   the graveyard). A block of W freed on the render thread is pushed onto
//!   a lock-free list that a janitor thread empties.
//! - **V**, blocks of 1 MiB and more straight from the virtual memory
//!   manager (no heap lock at all); one freed on the render thread goes to
//!   the janitor too (releasing a few MB measured 0.5-1.5 ms).
//!
//! The process heap is left to the OS, the CRT and the GPU driver. Each block
//! carries a 16-byte header (where it came from, and the offset to its base),
//! so any thread can free any block.
//!
//! On macOS (`VmBlocks`, on unless `--no-vm-blocks`) only V is kept: blocks
//! of 1 MiB and more come from the virtual memory manager (`mmap`) through
//! the same size-class cache, one freed on the render thread going to the
//! janitor, and everything smaller from the system allocator. Measured (the
//! drive, sampled): the Metal driver's command-encoder contexts take a large
//! allocation of their own each time an encoder opens, and the system
//! allocator now and then clears the chunk it hands back in the kernel
//! (`madvise`), 10-15 ms on the render thread, in whichever phase opened an
//! encoder; the streamed packs and tiles of 1 MiB and more churned the same
//! chunks. With them out of it, the drive's such frames (`props_pump` over
//! 5 ms or `finish` over 8 ms) fell from 12 in 10 runs to 2 in 11, for about
//! 200-270 MB more peak memory (the cache). No header is needed: `dealloc`
//! has the block's layout, and the switch is read once, at the first such
//! block.
//!
//! Elsewhere `EoAlloc` is the system allocator, and the heaps, lists and
//! classes below are unused.
#![cfg_attr(not(windows), allow(dead_code))]

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, Ordering};

#[cfg(windows)]
#[allow(non_snake_case)]
mod sys {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn HeapCreate(options: u32, initial: usize, maximum: usize) -> isize;
        pub fn HeapDestroy(heap: isize) -> i32;
        pub fn HeapAlloc(heap: isize, flags: u32, bytes: usize) -> *mut u8;
        pub fn HeapReAlloc(heap: isize, flags: u32, mem: *mut u8, bytes: usize) -> *mut u8;
        pub fn HeapFree(heap: isize, flags: u32, mem: *mut u8) -> i32;
        pub fn VirtualAlloc(addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
        pub fn VirtualFree(addr: *mut u8, size: usize, kind: u32) -> i32;
    }
    pub const HEAP_ZERO_MEMORY: u32 = 0x8;
    pub const MEM_COMMIT: u32 = 0x1000;
    pub const MEM_RESERVE: u32 = 0x2000;
    pub const MEM_RELEASE: u32 = 0x8000;
    pub const PAGE_READWRITE: u32 = 0x04;
}

/// Where a block came from (header word 0).
const FROM_R: usize = 1;
const FROM_W: usize = 2;
const FROM_V: usize = 3;
const HEADER: usize = 16;
/// Blocks at least this large come from the virtual memory manager.
const VIRTUAL_MIN: usize = 1 << 20;

static R_HEAP: AtomicIsize = AtomicIsize::new(0);
static W_HEAP: AtomicIsize = AtomicIsize::new(0);
/// Blocks waiting to go back to R (freed elsewhere) / W or V (freed on the render thread).
static R_DEFERRED: AtomicPtr<u8> = AtomicPtr::new(null_mut());
static W_DEFERRED: AtomicPtr<u8> = AtomicPtr::new(null_mut());
static V_DEFERRED: AtomicPtr<u8> = AtomicPtr::new(null_mut());
static JANITOR: AtomicBool = AtomicBool::new(false);

/// Freed V blocks kept for reuse, per size class (a class is 1 MiB x 2^(k/4),
/// in 64 KiB steps), so streamed packs and tiles do not map and unmap memory
/// at every turn; at most `V_CACHE_MAX` bytes are kept. Each list links its
/// free blocks through their first word; the render thread only `try_lock`s.
const V_CLASSES: usize = 48;
const V_CACHE_MAX: usize = 768 << 20;
static V_CACHE: [std::sync::Mutex<usize>; V_CLASSES] = [const { std::sync::Mutex::new(0) }; V_CLASSES];
static V_CACHED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The size of V class `k`.
fn v_class_size(k: usize) -> usize {
    let s = (VIRTUAL_MIN as f64 * 2f64.powf(k as f64 / 4.0)) as usize;
    s.next_multiple_of(64 << 10)
}

/// The smallest class that holds `total` bytes (None: larger than every class).
fn v_class(total: usize) -> Option<usize> {
    (0..V_CLASSES).find(|&k| v_class_size(k) >= total)
}

/// A cached block of class `k`, if one is free (and the list's lock is).
fn v_take(k: usize, render: bool) -> *mut u8 {
    let guard = if render { V_CACHE[k].try_lock().ok() } else { V_CACHE[k].lock().ok() };
    let Some(mut head) = guard else { return null_mut() };
    if *head == 0 {
        return null_mut();
    }
    let b = *head as *mut u8;
    // SAFETY: a free block linked by `v_keep`.
    *head = unsafe { (b as *const usize).read() };
    V_CACHED.fetch_sub(v_class_size(k), Ordering::Relaxed);
    b
}

/// Keep a freed V block of class `k` for reuse; false: the cache is full (or busy).
fn v_keep(k: usize, base: *mut u8) -> bool {
    let size = v_class_size(k);
    if V_CACHED.load(Ordering::Relaxed) + size > V_CACHE_MAX {
        return false;
    }
    let Ok(mut head) = V_CACHE[k].lock() else { return false };
    // SAFETY: the block is free and at least 1 MiB.
    unsafe { (base as *mut usize).write(*head) };
    *head = base as usize;
    V_CACHED.fetch_add(size, Ordering::Relaxed);
    true
}

/// Return a V block: to the cache, or to the system.
#[cfg(windows)]
fn v_release(from: usize, base: *mut u8) {
    let k = from >> 4;
    if !(k < V_CLASSES && v_keep(k, base)) {
        // SAFETY: a block from VirtualAlloc.
        unsafe { sys::VirtualFree(base, 0, sys::MEM_RELEASE) };
    }
}

/// The macOS V path's switch: `--no-vm-blocks` leaves every block to the
/// system allocator, as before.
pub struct VmBlocks;

/// (macOS) 0: not decided yet; 1: on; 2: off (`--no-vm-blocks`).
static VM_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

impl VmBlocks {
    /// The switch (eo-app accepts it; this module reads it).
    pub const SWITCH: &'static str = "--no-vm-blocks";

    /// Whether blocks of 1 MiB and more come from the VM (macOS; elsewhere
    /// false). Decided once, at the first such block, from the process's
    /// arguments as the C runtime holds them (reading them allocates nothing).
    pub fn on() -> bool {
        #[cfg(target_os = "macos")]
        {
            match VM_STATE.load(Ordering::Acquire) {
                1 => return true,
                2 => return false,
                _ => {}
            }
            let off = {
                // SAFETY: the C runtime's argc/argv, valid for the process's life.
                let (argc, argv) = unsafe { (*libc::_NSGetArgc(), *libc::_NSGetArgv()) };
                (0..argc.max(0) as usize).any(|i| {
                    let a = unsafe { *argv.add(i) };
                    !a.is_null() && unsafe { std::ffi::CStr::from_ptr(a) }.to_bytes() == Self::SWITCH.as_bytes()
                })
            };
            let want = if off { 2 } else { 1 };
            match VM_STATE.compare_exchange(0, want, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => want == 1,
                Err(v) => v == 1,
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

/// (macOS) Page size the VM blocks are rounded to (16 KiB on Apple silicon; a
/// multiple of 4 KiB).
#[cfg(target_os = "macos")]
const VM_PAGE: usize = 16 << 10;

/// (macOS) A block of `size` bytes at `align` comes from the VM.
#[cfg(target_os = "macos")]
fn is_v(size: usize, align: usize) -> bool {
    size >= VIRTUAL_MIN && align <= 4096 && VmBlocks::on()
}

/// (macOS) A V block's class (`V_CLASSES`: larger than every class, never
/// cached) and its mapped bytes.
#[cfg(target_os = "macos")]
fn v_extent(size: usize) -> (usize, usize) {
    match v_class(size) {
        Some(k) => (k, v_class_size(k)),
        None => (V_CLASSES, size.next_multiple_of(VM_PAGE)),
    }
}

/// (macOS) A V block of `size` bytes: a cached one, or fresh pages (zero).
#[cfg(target_os = "macos")]
unsafe fn v_alloc(size: usize, zero: bool) -> *mut u8 {
    let (k, bytes) = v_extent(size);
    if k < V_CLASSES {
        let c = v_take(k, is_render());
        if !c.is_null() {
            if zero {
                // SAFETY: the block holds at least `size` bytes, all ours.
                unsafe { std::ptr::write_bytes(c, 0, size) };
            }
            return c;
        }
    }
    // SAFETY: an anonymous private mapping; fresh pages are zero.
    let p = unsafe { libc::mmap(null_mut(), bytes, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
    if p == libc::MAP_FAILED { null_mut() } else { p as *mut u8 }
}

/// (macOS) Return a V block of class `k` and `bytes`: to the cache, or unmapped.
#[cfg(target_os = "macos")]
fn v_release(k: usize, bytes: usize, base: *mut u8) {
    if !(k < V_CLASSES && v_keep(k, base)) {
        // SAFETY: a mapping of `bytes` from `v_alloc`.
        unsafe { libc::munmap(base as *mut libc::c_void, bytes) };
    }
}

/// (macOS) Free a V block of `size` bytes: on the render thread (with the
/// janitor running) handed to the janitor, which may unmap it; elsewhere now.
#[cfg(target_os = "macos")]
unsafe fn v_free(base: *mut u8, size: usize) {
    let (k, bytes) = v_extent(size);
    if is_render() && JANITOR.load(Ordering::Relaxed) {
        // The class and size ride in the block's second and third words (the first links it).
        unsafe {
            (base as *mut usize).add(1).write(k);
            (base as *mut usize).add(2).write(bytes);
        }
        push(&V_DEFERRED, base);
    } else {
        v_release(k, bytes, base);
    }
}

thread_local! {
    static RENDER: Cell<bool> = const { Cell::new(false) };
}

fn is_render() -> bool {
    RENDER.try_with(|r| r.get()).unwrap_or(false)
}

#[cfg(windows)]
fn w_heap() -> isize {
    let h = W_HEAP.load(Ordering::Acquire);
    if h != 0 {
        return h;
    }
    // SAFETY: plain Win32 calls; a growable heap, serialized.
    let new = unsafe { sys::HeapCreate(0, 0, 0) };
    match W_HEAP.compare_exchange(0, new, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => new,
        Err(other) => {
            unsafe { sys::HeapDestroy(new) };
            other
        }
    }
}

/// Push a block's base onto a lock-free list (the first word links it).
fn push(list: &AtomicPtr<u8>, base: *mut u8) {
    let mut head = list.load(Ordering::Relaxed);
    loop {
        // SAFETY: the block is at least HEADER bytes and no longer in use.
        unsafe { (base as *mut *mut u8).write(head) };
        match list.compare_exchange_weak(head, base, Ordering::Release, Ordering::Relaxed) {
            Ok(_) => return,
            Err(h) => head = h,
        }
    }
}

/// Release every block on a list with `release(base)`.
fn drain(list: &AtomicPtr<u8>, release: impl Fn(*mut u8)) {
    let mut p = list.swap(null_mut(), Ordering::Acquire);
    while !p.is_null() {
        // SAFETY: written by `push`.
        let next = unsafe { (p as *mut *mut u8).read() };
        release(p);
        p = next;
    }
}

/// The global allocator (eo-app declares it with `#[global_allocator]`).
pub struct EoAlloc;

#[cfg(windows)]
impl EoAlloc {
    /// `from`, base pointer and the user pointer for `size` bytes at `align`.
    unsafe fn raw_alloc(size: usize, align: usize, zero: bool) -> *mut u8 {
        let pad = HEADER.max(align);
        let Some(total) = size.checked_add(pad) else { return null_mut() };
        let (from, base) = if let Some(k) = v_class(total).filter(|_| total >= VIRTUAL_MIN && align <= 4096) {
            let size = v_class_size(k);
            let cached = v_take(k, is_render());
            if !cached.is_null() {
                if zero {
                    // SAFETY: the whole block is ours.
                    unsafe { std::ptr::write_bytes(cached, 0, size) };
                }
                (FROM_V | (k << 4), cached)
            } else {
                // SAFETY: plain Win32 call; fresh pages are zero.
                (FROM_V | (k << 4), unsafe { sys::VirtualAlloc(null_mut(), size, sys::MEM_COMMIT | sys::MEM_RESERVE, sys::PAGE_READWRITE) })
            }
        } else if total >= VIRTUAL_MIN && align <= 4096 {
            // Larger than every class: straight from the system, never cached.
            (FROM_V | (V_CLASSES << 4), unsafe { sys::VirtualAlloc(null_mut(), total, sys::MEM_COMMIT | sys::MEM_RESERVE, sys::PAGE_READWRITE) })
        } else if is_render() {
            let r = R_HEAP.load(Ordering::Relaxed);
            drain(&R_DEFERRED, |b| unsafe {
                sys::HeapFree(r, 0, b);
            });
            (FROM_R, unsafe { sys::HeapAlloc(r, if zero { sys::HEAP_ZERO_MEMORY } else { 0 }, total) })
        } else {
            (FROM_W, unsafe { sys::HeapAlloc(w_heap(), if zero { sys::HEAP_ZERO_MEMORY } else { 0 }, total) })
        };
        if base.is_null() {
            return null_mut();
        }
        // The user pointer: `pad` past a 16-aligned base, rounded to `align`.
        let user = ((base as usize + HEADER).next_multiple_of(align.max(HEADER))) as *mut u8;
        // SAFETY: the header lies inside the block, before `user`.
        unsafe {
            let h = user.sub(HEADER) as *mut usize;
            h.write(from);
            h.add(1).write(user as usize - base as usize);
        }
        user
    }

    /// Where a user block came from and its base pointer.
    unsafe fn header(user: *mut u8) -> (usize, *mut u8) {
        let h = unsafe { user.sub(HEADER) } as *const usize;
        // SAFETY: written by `raw_alloc`.
        let (from, off) = unsafe { (h.read(), h.add(1).read()) };
        (from, unsafe { user.sub(off) })
    }

    unsafe fn raw_free(user: *mut u8) {
        let (from, base) = unsafe { Self::header(user) };
        let render = is_render();
        if from & 0xF == FROM_V {
            if render && JANITOR.load(Ordering::Relaxed) {
                // The class rides in the block's second word (the first links it).
                unsafe { (base as *mut usize).add(1).write(from) };
                push(&V_DEFERRED, base);
            } else {
                v_release(from, base);
            }
            return;
        }
        match from {
            FROM_R if render => unsafe {
                sys::HeapFree(R_HEAP.load(Ordering::Relaxed), 0, base);
            },
            FROM_R => push(&R_DEFERRED, base),
            FROM_W if render && JANITOR.load(Ordering::Relaxed) => push(&W_DEFERRED, base),
            FROM_W => unsafe {
                sys::HeapFree(w_heap(), 0, base);
            },
            _ => {}
        }
    }
}

#[cfg(windows)]
unsafe impl GlobalAlloc for EoAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        unsafe { Self::raw_alloc(l.size(), l.align(), false) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        unsafe { Self::raw_alloc(l.size(), l.align(), true) }
    }
    unsafe fn dealloc(&self, p: *mut u8, _l: Layout) {
        unsafe { Self::raw_free(p) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let (from, base) = unsafe { Self::header(p) };
        let render = is_render();
        let same_heap = (from == FROM_R && render) || (from == FROM_W && !render);
        // In place within the block's own heap, while it stays a heap block.
        if same_heap && l.align() <= HEADER && new + HEADER < VIRTUAL_MIN {
            let heap = if from == FROM_R { R_HEAP.load(Ordering::Relaxed) } else { w_heap() };
            let nb = unsafe { sys::HeapReAlloc(heap, 0, base, new + HEADER) };
            if nb.is_null() {
                return null_mut();
            }
            // The header moved with the block (offset HEADER, same origin).
            return unsafe { nb.add(HEADER) };
        }
        let q = unsafe { Self::raw_alloc(new, l.align(), false) };
        if !q.is_null() {
            // SAFETY: both blocks hold at least min(old, new) bytes.
            unsafe { std::ptr::copy_nonoverlapping(p, q, l.size().min(new)) };
            unsafe { Self::raw_free(p) };
        }
        q
    }
}

#[cfg(target_os = "macos")]
unsafe impl GlobalAlloc for EoAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if is_v(l.size(), l.align()) { unsafe { v_alloc(l.size(), false) } } else { unsafe { std::alloc::System.alloc(l) } }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if is_v(l.size(), l.align()) { unsafe { v_alloc(l.size(), true) } } else { unsafe { std::alloc::System.alloc_zeroed(l) } }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if is_v(l.size(), l.align()) { unsafe { v_free(p, l.size()) } } else { unsafe { std::alloc::System.dealloc(p, l) } }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let (was_v, will_v) = (is_v(l.size(), l.align()), is_v(new, l.align()));
        if !was_v && !will_v {
            return unsafe { std::alloc::System.realloc(p, l, new) };
        }
        if was_v && will_v && v_extent(l.size()) == v_extent(new) {
            return p; // the same mapping holds it (and `dealloc` will find the same class)
        }
        // SAFETY: `new` and the old alignment form a valid layout (the caller's contract).
        let q = unsafe { self.alloc(Layout::from_size_align_unchecked(new, l.align())) };
        if !q.is_null() {
            // SAFETY: both blocks hold at least min(old, new) bytes.
            unsafe { std::ptr::copy_nonoverlapping(p, q, l.size().min(new)) };
            unsafe { self.dealloc(p, l) };
        }
        q
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
unsafe impl GlobalAlloc for EoAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        unsafe { std::alloc::System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { std::alloc::System.dealloc(p, l) }
    }
}

/// Make the calling thread the render thread: its allocations come from a
/// heap of its own from now on, and the janitor that returns the other
/// blocks it frees is started. Call before the frame loop (not from inside
/// the allocator). Without `EoAlloc` as the global allocator it only marks
/// the thread.
pub fn claim_render_thread() {
    #[cfg(windows)]
    {
        if R_HEAP.load(Ordering::Acquire) == 0 {
            // SAFETY: plain Win32 call.
            let h = unsafe { sys::HeapCreate(0, 0, 0) };
            if R_HEAP.compare_exchange(0, h, Ordering::AcqRel, Ordering::Acquire).is_err() {
                unsafe { sys::HeapDestroy(h) };
            }
        }
        if !JANITOR.swap(true, Ordering::AcqRel) {
            std::thread::Builder::new()
                .name("eo-alloc-janitor".into())
                .spawn(|| {
                    loop {
                        // SAFETY: blocks pushed by `raw_free` (W and V bases).
                        drain(&W_DEFERRED, |base| unsafe {
                            sys::HeapFree(w_heap(), 0, base);
                        });
                        drain(&V_DEFERRED, |base| v_release(unsafe { (base as *const usize).add(1).read() }, base));
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                })
                .expect("spawn allocator janitor");
        }
    }
    #[cfg(target_os = "macos")]
    if !JANITOR.swap(true, Ordering::AcqRel) {
        std::thread::Builder::new()
            .name("eo-alloc-janitor".into())
            .spawn(|| {
                loop {
                    // SAFETY: V bases pushed by `v_free`, their class and size in words 1 and 2.
                    drain(&V_DEFERRED, |base| unsafe { v_release((base as *const usize).add(1).read(), (base as *const usize).add(2).read(), base) });
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            })
            .expect("spawn allocator janitor");
    }
    let _ = RENDER.try_with(|r| r.set(true));
}

/// Whether the calling thread allocates from the render heap.
pub fn is_render_thread() -> bool {
    is_render()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn fill(p: *mut u8, n: usize, seed: u8) {
        for i in 0..n {
            unsafe { p.add(i).write(seed.wrapping_add(i as u8)) };
        }
    }
    fn check(p: *const u8, n: usize, seed: u8) -> bool {
        (0..n).all(|i| unsafe { p.add(i).read() } == seed.wrapping_add(i as u8))
    }

    /// Blocks of every size class and alignment, made on the render thread
    /// and on a worker, grown and shrunk, and freed on the other thread.
    #[test]
    fn blocks_cross_threads_keep_their_bytes_and_alignment() {
        let a = EoAlloc;
        let sizes = [1usize, 15, 16, 17, 1000, 13_248, 24_704, 300_000, 2 << 20];
        let aligns = [1usize, 8, 16, 32, 64, 4096];
        let make = |seed: u8| -> Vec<(usize, usize, usize)> {
            let mut v = vec![];
            for (k, &s) in sizes.iter().enumerate() {
                for &al in &aligns {
                    let l = Layout::from_size_align(s, al).unwrap();
                    let p = unsafe { a.alloc(l) };
                    assert!(!p.is_null() && p as usize % al == 0, "size {s} align {al}");
                    fill(p, s, seed.wrapping_add(k as u8));
                    v.push((p as usize, s, al));
                }
            }
            v
        };
        let grow = |v: Vec<(usize, usize, usize)>, seed: u8| -> Vec<(usize, usize, usize)> {
            v.into_iter()
                .enumerate()
                .map(|(i, (p, s, al))| {
                    let k = (i / aligns.len()) as u8;
                    let n = if i % 2 == 0 { s * 2 + 7 } else { (s / 2).max(1) };
                    let q = unsafe { a.realloc(p as *mut u8, Layout::from_size_align(s, al).unwrap(), n) };
                    assert!(!q.is_null() && q as usize % al == 0);
                    assert!(check(q, s.min(n), seed.wrapping_add(k)), "realloc kept the bytes (size {s} -> {n}, align {al})");
                    (q as usize, n, al)
                })
                .collect()
        };
        let free = |v: Vec<(usize, usize, usize)>| {
            for (p, s, al) in v {
                unsafe { a.dealloc(p as *mut u8, Layout::from_size_align(s, al).unwrap()) };
            }
        };
        claim_render_thread();
        assert!(is_render_thread());
        let mine = grow(make(1), 1);
        let theirs = std::thread::scope(|sc| sc.spawn(|| grow(make(7), 7)).join().unwrap());
        // Render-thread blocks freed on a worker; the worker's freed here.
        std::thread::scope(|sc| sc.spawn(|| free(mine)).join().unwrap());
        free(theirs);
        // The next render-thread allocation returns the blocks the worker freed.
        let l = Layout::from_size_align(64, 8).unwrap();
        let p = unsafe { a.alloc(l) };
        assert!(R_DEFERRED.load(Ordering::Acquire).is_null(), "the render heap's deferred blocks were returned");
        unsafe { a.dealloc(p, l) };
        // The janitor returns what this thread freed of the other heaps.
        let t = std::time::Instant::now();
        while !(W_DEFERRED.load(Ordering::Acquire).is_null() && V_DEFERRED.load(Ordering::Acquire).is_null()) {
            assert!(t.elapsed().as_secs() < 5, "the janitor returned the deferred blocks");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod mac_tests {
    use super::*;

    fn fill(p: *mut u8, n: usize, seed: u8) {
        for i in (0..n).step_by(4093) {
            unsafe { p.add(i).write(seed.wrapping_add(i as u8)) };
        }
        unsafe { p.add(n - 1).write(seed) };
    }
    fn check(p: *const u8, n: usize, seed: u8) -> bool {
        (0..n).step_by(4093).all(|i| unsafe { p.add(i).read() } == seed.wrapping_add(i as u8))
    }

    /// Blocks on both sides of 1 MiB at every alignment, made on the render
    /// thread and on a worker, grown and shrunk across the line, freed on the
    /// other thread; a cached block handed out zeroed is zero; the render
    /// thread's freed VM blocks reach the janitor.
    #[test]
    fn vm_blocks_keep_their_bytes_alignment_and_zeroes() {
        assert!(VmBlocks::on(), "on without {}", VmBlocks::SWITCH);
        let a = EoAlloc;
        let sizes = [1usize, 1000, 300_000, (1 << 20) - 1, 1 << 20, (1 << 20) + 1, 3 << 20, 40 << 20];
        let aligns = [1usize, 16, 4096, 16384];
        let make = |seed: u8| -> Vec<(usize, usize, usize)> {
            let mut v = vec![];
            for (k, &s) in sizes.iter().enumerate() {
                for &al in &aligns {
                    let p = unsafe { a.alloc(Layout::from_size_align(s, al).unwrap()) };
                    assert!(!p.is_null() && p as usize % al == 0, "size {s} align {al}");
                    fill(p, s, seed.wrapping_add(k as u8));
                    v.push((p as usize, s, al));
                }
            }
            v
        };
        let grow = |v: Vec<(usize, usize, usize)>, seed: u8| -> Vec<(usize, usize, usize)> {
            v.into_iter()
                .enumerate()
                .map(|(i, (p, s, al))| {
                    let k = (i / aligns.len()) as u8;
                    let n = if i % 2 == 0 { s * 2 + 7 } else { (s / 2).max(1) };
                    let q = unsafe { a.realloc(p as *mut u8, Layout::from_size_align(s, al).unwrap(), n) };
                    assert!(!q.is_null() && q as usize % al == 0);
                    assert!(check(q, s.min(n), seed.wrapping_add(k)), "realloc kept the bytes (size {s} -> {n}, align {al})");
                    (q as usize, n, al)
                })
                .collect()
        };
        let free = |v: Vec<(usize, usize, usize)>| {
            for (p, s, al) in v {
                unsafe { a.dealloc(p as *mut u8, Layout::from_size_align(s, al).unwrap()) };
            }
        };
        // a cached block (dirty) handed out again zeroed.
        let l = Layout::from_size_align(2 << 20, 8).unwrap();
        let p = unsafe { a.alloc(l) };
        unsafe { std::ptr::write_bytes(p, 0xAB, l.size()) };
        unsafe { a.dealloc(p, l) };
        let z = unsafe { a.alloc_zeroed(l) };
        assert!((0..l.size()).step_by(997).all(|i| unsafe { z.add(i).read() } == 0), "a reused block handed out zeroed is zero");
        unsafe { a.dealloc(z, l) };
        std::thread::scope(|sc| {
            sc.spawn(|| {
                claim_render_thread();
                assert!(is_render_thread());
                let mine = grow(make(1), 1);
                let theirs = std::thread::scope(|sc| sc.spawn(|| grow(make(7), 7)).join().unwrap());
                std::thread::scope(|sc| sc.spawn(|| free(mine)).join().unwrap());
                free(theirs); // VM blocks freed here go to the janitor
            })
            .join()
            .unwrap()
        });
        let t = std::time::Instant::now();
        while !V_DEFERRED.load(Ordering::Acquire).is_null() {
            assert!(t.elapsed().as_secs() < 5, "the janitor returned the render thread's VM blocks");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
