//! The world: the resident network, the regions, one shared clock, a
//! persistent worker pool that steps regions in parallel, and an off-thread
//! runner that keeps the whole simulation off the render thread.
//!
//! FIXED STEP. `World::step` advances the shared clock by one fixed step
//! (`FIXED_STEP_S`, the JS `PHYSICS_FIXED_STEP_SECONDS`) and steps every
//! region with it. `World::advance(frame_dt)` accumulates frame time and runs
//! as many fixed steps as are due (capped), exactly like the JS simulation
//! clock - but each step here costs tens of microseconds, not milliseconds.
//!
//! DETERMINISM. A region's result depends only on the network, the clock, its
//! seed and its focus inputs: regions share nothing mutable, every random draw
//! comes from the region's own sequences or from keyed hashes, so the result
//! is the same on one thread or twenty-four (`tests/determinism.rs`).
//!
//! THREADS. `WorkerPool` is a fixed set of OS threads parked on a condvar;
//! a step hands them the region slice and they pull regions off an atomic
//! cursor (the calling thread works too). No per-step allocation, no thread
//! creation per step.
//!
//! OFF THE RENDER THREAD. `AsyncSim` moves the world onto its own thread. The
//! render thread posts the frame's focus inputs and elapsed time, and picks up
//! the newest published agent buffers through a triple buffer: its cost per
//! frame is two short mutex sections and a pointer swap, whatever the
//! population.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::consts::FIXED_STEP_S;
use crate::network::Network;
use crate::corridor::Corridors;
use crate::output::{AgentBuffer, CorridorInstance};
use crate::region::{Focus, Region};

#[derive(Clone, Copy)]
struct Task {
    regions: *mut Region,
    len: usize,
    net: *const Network,
    time: f64,
    dt: f64,
    cpu_ns: *const AtomicUsize,
}
// SAFETY: a Task is only dereferenced between `run` publishing it and `run`
// returning; `run` holds the exclusive borrow of the regions and the shared
// borrow of the network for that whole interval, and each region index is
// claimed by exactly one thread through the atomic cursor.
unsafe impl Send for Task {}

struct PoolState {
    generation: u64,
    task: Option<Task>,
    busy: usize,
    shutdown: bool,
}

struct PoolShared {
    state: Mutex<PoolState>,
    start: Condvar,
    done: Condvar,
    cursor: AtomicUsize,
}

pub struct WorkerPool {
    shared: Arc<PoolShared>,
    workers: Vec<JoinHandle<()>>,
}

fn work(shared: &PoolShared, task: Task) {
    loop {
        let i = shared.cursor.fetch_add(1, Ordering::Relaxed);
        if i >= task.len {
            break;
        }
        let t0 = Instant::now();
        // SAFETY: see `Task`.
        unsafe {
            let region = &mut *task.regions.add(i);
            region.step(&*task.net, task.time, task.dt);
            (*task.cpu_ns).fetch_add(t0.elapsed().as_nanos() as usize, Ordering::Relaxed);
        }
    }
}

impl WorkerPool {
    /// `threads` helpers besides the calling thread.
    pub fn new(threads: usize) -> Self {
        let shared = Arc::new(PoolShared {
            state: Mutex::new(PoolState { generation: 0, task: None, busy: 0, shutdown: false }),
            start: Condvar::new(),
            done: Condvar::new(),
            cursor: AtomicUsize::new(0),
        });
        let workers = (0..threads)
            .map(|k| {
                let shared = shared.clone();
                std::thread::Builder::new()
                    .name(format!("eo-sim-{k}"))
                    .spawn(move || {
                        let mut seen = 0u64;
                        loop {
                            let task = {
                                let mut st = shared.state.lock().unwrap();
                                while st.generation == seen && !st.shutdown {
                                    st = shared.start.wait(st).unwrap();
                                }
                                if st.shutdown {
                                    return;
                                }
                                seen = st.generation;
                                st.task
                            };
                            if let Some(task) = task {
                                work(&shared, task);
                            }
                            let mut st = shared.state.lock().unwrap();
                            st.busy -= 1;
                            if st.busy == 0 {
                                shared.done.notify_all();
                            }
                        }
                    })
                    .expect("spawn simulation worker")
            })
            .collect();
        WorkerPool { shared, workers }
    }

    pub fn threads(&self) -> usize {
        self.workers.len()
    }

    /// Step every region once, in parallel. Returns the summed CPU time of the region steps, ns.
    pub fn run(&self, regions: &mut [Region], net: &Network, time: f64, dt: f64) -> u64 {
        let cpu = AtomicUsize::new(0);
        let task = Task { regions: regions.as_mut_ptr(), len: regions.len(), net, time, dt, cpu_ns: &cpu };
        self.shared.cursor.store(0, Ordering::Relaxed);
        {
            let mut st = self.shared.state.lock().unwrap();
            st.task = Some(task);
            st.busy = self.workers.len();
            st.generation += 1;
            self.shared.start.notify_all();
        }
        work(&self.shared, task);
        let mut st = self.shared.state.lock().unwrap();
        while st.busy > 0 {
            st = self.shared.done.wait(st).unwrap();
        }
        st.task = None;
        cpu.load(Ordering::Relaxed) as u64
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        {
            let mut st = self.shared.state.lock().unwrap();
            st.shutdown = true;
            self.shared.start.notify_all();
        }
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StepTiming {
    /// Wall time of the last step, ns.
    pub wall_ns: u64,
    /// CPU time summed over every thread that stepped a region, ns.
    pub cpu_ns: u64,
}

pub struct World {
    pub net: Network,
    pub regions: Vec<Region>,
    pub time: f64,
    pub steps: u64,
    accumulator: f64,
    pool: Option<WorkerPool>,
    pub last: StepTiming,
    /// The corridor layer (the long roads' through traffic), stepped on this
    /// clock after the regions, with the first region's focus for its hand-over.
    pub corridors: Option<Corridors>,
}

/// The most fixed steps one `advance` runs (the JS clock's catch-up cap).
pub const MAX_STEPS_PER_ADVANCE: u32 = 8;

impl World {
    pub fn new(net: Network) -> Self {
        World { net, regions: vec![], time: 0.0, steps: 0, accumulator: 0.0, pool: None, last: StepTiming::default(), corridors: None }
    }

    /// Use `helpers` worker threads besides the stepping thread (0: step inline).
    pub fn set_threads(&mut self, helpers: usize) {
        self.pool = if helpers > 0 { Some(WorkerPool::new(helpers)) } else { None };
    }

    pub fn threads(&self) -> usize {
        1 + self.pool.as_ref().map_or(0, |p| p.threads())
    }

    /// One fixed step of every region.
    pub fn step(&mut self) {
        let dt = FIXED_STEP_S;
        self.time += dt;
        self.steps += 1;
        let t0 = Instant::now();
        let cpu = match (&self.pool, self.regions.len()) {
            (Some(pool), n) if n > 1 => pool.run(&mut self.regions, &self.net, self.time, dt),
            _ => {
                for r in self.regions.iter_mut() {
                    r.step(&self.net, self.time, dt);
                }
                t0.elapsed().as_nanos() as u64
            }
        };
        let mut cpu = cpu;
        if let Some(c) = &mut self.corridors {
            let t1 = Instant::now();
            let focus = self.regions.first().map(|r| [r.focus.x, r.focus.z]);
            c.step(dt, focus);
            cpu += t1.elapsed().as_nanos() as u64;
        }
        self.last = StepTiming { wall_ns: t0.elapsed().as_nanos() as u64, cpu_ns: cpu };
    }

    /// Run the fixed steps a frame of `frame_dt` seconds makes due. Returns how many ran.
    pub fn advance(&mut self, frame_dt: f64) -> u32 {
        self.accumulator += frame_dt.max(0.0);
        let mut n = 0;
        while self.accumulator >= FIXED_STEP_S && n < MAX_STEPS_PER_ADVANCE {
            self.accumulator -= FIXED_STEP_S;
            self.step();
            n += 1;
        }
        if n == MAX_STEPS_PER_ADVANCE {
            self.accumulator = self.accumulator.min(FIXED_STEP_S);
        }
        n
    }

    pub fn set_focus(&mut self, region: usize, focus: Focus) {
        if let Some(r) = self.regions.get_mut(region) {
            r.focus = focus;
        }
    }
}

// ---------------------------------------------------------------- off-thread runner

/// The frame the render thread reads.
#[derive(Default, Clone)]
pub struct Frame {
    pub time: f64,
    pub steps: u64,
    pub regions: Vec<AgentBuffer>,
    /// The corridor layer's vehicles and walkers (empty without one).
    pub corridor: Vec<CorridorInstance>,
    /// The last fixed step's timing on the simulation thread.
    pub timing: StepTiming,
}

impl Frame {
    fn copy_from(&mut self, world: &World) {
        self.time = world.time;
        self.steps = world.steps;
        self.timing = world.last;
        self.regions.resize_with(world.regions.len(), AgentBuffer::default);
        for (dst, r) in self.regions.iter_mut().zip(&world.regions) {
            dst.origin = r.out.origin;
            dst.time = r.out.time;
            dst.step = r.out.step;
            dst.vehicles.clear();
            dst.vehicles.extend_from_slice(&r.out.vehicles);
            dst.walkers.clear();
            dst.walkers.extend_from_slice(&r.out.walkers);
        }
        self.corridor.clear();
        if let Some(c) = &world.corridors {
            self.corridor.extend_from_slice(&c.out);
        }
    }
}

struct Mailbox {
    /// Frame time posted by the render thread and not yet simulated.
    pending_dt: f64,
    foci: Vec<Option<Focus>>,
    middle: Frame,
    fresh: bool,
    shutdown: bool,
}

struct AsyncShared {
    mail: Mutex<Mailbox>,
    wake: Condvar,
}

/// The world on its own thread, with a triple-buffered frame for the render thread.
pub struct AsyncSim {
    shared: Arc<AsyncShared>,
    front: Frame,
    thread: Option<JoinHandle<World>>,
}

impl AsyncSim {
    pub fn start(world: World) -> Self {
        Self::start_with(world, |_| {})
    }

    /// `start`, with `hook` run on the simulation thread after the
    /// foci are applied and before each advance (eo-app streams the network
    /// around the foci there, off the render thread).
    pub fn start_with(mut world: World, mut hook: impl FnMut(&mut World) + Send + 'static) -> Self {
        let regions = world.regions.len();
        let mut first = Frame::default();
        first.copy_from(&world);
        let shared = Arc::new(AsyncShared {
            mail: Mutex::new(Mailbox {
                pending_dt: 0.0,
                foci: vec![None; regions],
                middle: first.clone(),
                fresh: false,
                shutdown: false,
            }),
            wake: Condvar::new(),
        });
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("eo-sim-main".into())
            .spawn(move || {
                let mut back = Frame::default();
                let mut foci: Vec<Option<Focus>> = vec![None; regions];
                loop {
                    let dt = {
                        let mut m = s.mail.lock().unwrap();
                        while m.pending_dt <= 0.0 && !m.shutdown {
                            m = s.wake.wait(m).unwrap();
                        }
                        if m.shutdown {
                            return world;
                        }
                        foci.clone_from_slice(&m.foci);
                        for f in m.foci.iter_mut() {
                            *f = None;
                        }
                        std::mem::take(&mut m.pending_dt)
                    };
                    for (i, f) in foci.iter().enumerate() {
                        if let Some(f) = f {
                            world.set_focus(i, *f);
                        }
                    }
                    hook(&mut world); // agents
                    if world.advance(dt) > 0 {
                        back.copy_from(&world);
                        let mut m = s.mail.lock().unwrap();
                        std::mem::swap(&mut m.middle, &mut back);
                        m.fresh = true;
                    }
                }
            })
            .expect("spawn simulation thread");
        AsyncSim { shared, front: first, thread: Some(thread) }
    }

    /// Render thread, once per frame: hand over the frame's time and focus inputs.
    pub fn post(&self, frame_dt: f64, foci: &[(usize, Focus)]) {
        let mut m = self.shared.mail.lock().unwrap();
        m.pending_dt += frame_dt;
        for &(i, f) in foci {
            if let Some(slot) = m.foci.get_mut(i) {
                *slot = Some(f);
            }
        }
        self.shared.wake.notify_one();
    }

    /// Render thread: the newest published frame (swapped in if the simulation has finished one).
    pub fn latest(&mut self) -> &Frame {
        let mut m = self.shared.mail.lock().unwrap();
        if m.fresh {
            std::mem::swap(&mut m.middle, &mut self.front);
            m.fresh = false;
        }
        drop(m);
        &self.front
    }

    /// Stop the thread and take the world back.
    pub fn stop(mut self) -> World {
        self.signal_shutdown();
        self.thread.take().unwrap().join().expect("simulation thread")
    }

    fn signal_shutdown(&self) {
        let mut m = self.shared.mail.lock().unwrap();
        m.shutdown = true;
        self.shared.wake.notify_one();
    }
}

impl Drop for AsyncSim {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            self.signal_shutdown();
            let _ = t.join();
        }
    }
}
