//! A persistent worker pool for the server's per-tick parallel loops (every
//! player's controller, every client's snapshot): threads parked on a
//! condvar, indices handed out by an atomic cursor, the calling thread
//! working too. No allocation and no thread creation per call.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

#[derive(Clone, Copy)]
struct Job {
    f: *const (dyn Fn(usize) + Sync),
    n: usize,
}
// SAFETY: a Job is only dereferenced between `run` publishing it and `run`
// returning, and `run` keeps the closure alive for that whole interval.
unsafe impl Send for Job {}

struct State {
    generation: u64,
    job: Option<Job>,
    busy: usize,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    start: Condvar,
    done: Condvar,
    cursor: AtomicUsize,
}

pub struct Pool {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

fn work(shared: &Shared, job: Job) {
    loop {
        let i = shared.cursor.fetch_add(1, Ordering::Relaxed);
        if i >= job.n {
            break;
        }
        // SAFETY: see `Job`.
        unsafe { (*job.f)(i) };
    }
}

impl Pool {
    /// `helpers` threads besides the caller.
    pub fn new(helpers: usize, name: &str) -> Self {
        let shared = Arc::new(Shared { state: Mutex::new(State { generation: 0, job: None, busy: 0, shutdown: false }), start: Condvar::new(), done: Condvar::new(), cursor: AtomicUsize::new(0) });
        let threads = (0..helpers)
            .map(|k| {
                let s = shared.clone();
                std::thread::Builder::new()
                    .name(format!("{name}-{k}"))
                    .spawn(move || {
                        let mut seen = 0u64;
                        loop {
                            let job = {
                                let mut st = s.state.lock().unwrap();
                                while st.generation == seen && !st.shutdown {
                                    st = s.start.wait(st).unwrap();
                                }
                                if st.shutdown {
                                    return;
                                }
                                seen = st.generation;
                                st.job
                            };
                            if let Some(job) = job {
                                work(&s, job);
                            }
                            let mut st = s.state.lock().unwrap();
                            st.busy -= 1;
                            if st.busy == 0 {
                                s.done.notify_all();
                            }
                        }
                    })
                    .expect("spawn pool worker")
            })
            .collect();
        Pool { shared, threads }
    }

    pub fn threads(&self) -> usize {
        self.threads.len() + 1
    }

    /// Run `f(0..n)` across the pool; returns when every call has.
    pub fn run(&self, n: usize, f: &(dyn Fn(usize) + Sync)) {
        if n == 0 {
            return;
        }
        if n == 1 || self.threads.is_empty() {
            for i in 0..n {
                f(i);
            }
            return;
        }
        // SAFETY: the lifetime is erased only for the duration of this call.
        let f: &'static (dyn Fn(usize) + Sync) = unsafe { std::mem::transmute(f) };
        let job = Job { f, n };
        self.shared.cursor.store(0, Ordering::Relaxed);
        {
            let mut st = self.shared.state.lock().unwrap();
            st.job = Some(job);
            st.busy = self.threads.len();
            st.generation += 1;
            self.shared.start.notify_all();
        }
        work(&self.shared, job);
        let mut st = self.shared.state.lock().unwrap();
        while st.busy > 0 {
            st = self.shared.done.wait(st).unwrap();
        }
        st.job = None;
    }

    /// `f` on every item, in parallel (each item to exactly one thread).
    pub fn for_each_mut<T: Send>(&self, items: &mut [T], f: impl Fn(usize, &mut T) + Sync) {
        struct P<T>(*mut T);
        unsafe impl<T> Send for P<T> {}
        unsafe impl<T> Sync for P<T> {}
        let p = P(items.as_mut_ptr());
        let p = &p;
        // SAFETY: every index is claimed by exactly one thread through the cursor.
        self.run(items.len(), &|i| unsafe { f(i, &mut *p.0.add(i)) });
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        {
            let mut st = self.shared.state.lock().unwrap();
            st.shutdown = true;
            self.shared.start.notify_all();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_item_once() {
        let pool = super::Pool::new(7, "t");
        let mut v = vec![0u64; 10_000];
        for round in 0..20 {
            pool.for_each_mut(&mut v, |i, x| *x += i as u64 + round);
        }
        for (i, x) in v.iter().enumerate() {
            assert_eq!(*x, 20 * i as u64 + (0..20).sum::<u64>());
        }
    }
}
