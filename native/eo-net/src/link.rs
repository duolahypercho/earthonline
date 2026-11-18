//! The datagram link: a `std::net::UdpSocket` and, for tests and benches, a
//! link conditioner that delays (latency + jitter), drops and duplicates
//! datagrams in both directions, so every protocol path runs under the
//! conditions it must survive. Jitter reorders datagrams, as a real path does.
//!
//! Sends with a conditioner go to a delivery thread that holds them until
//! their time; receives are held in a heap and handed out when due. Without
//! a conditioner both are the bare socket calls.

use std::collections::BinaryHeap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// One direction's impairment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impair {
    /// Fixed one-way delay, ms.
    pub latency_ms: f64,
    /// Uniform extra delay in [0, jitter], ms (reorders datagrams closer than this).
    pub jitter_ms: f64,
    /// Probability a datagram is lost.
    pub loss: f64,
    /// Probability a datagram is delivered twice.
    pub duplicate: f64,
}

impl Impair {
    pub const NONE: Impair = Impair { latency_ms: 0.0, jitter_ms: 0.0, loss: 0.0, duplicate: 0.0 };

    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }
}

/// Both directions (applied by the endpoint that owns the link).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conditions {
    pub send: Impair,
    pub recv: Impair,
}

impl Conditions {
    pub const NONE: Conditions = Conditions { send: Impair::NONE, recv: Impair::NONE };

    /// The owner's test profile: 80 ms round trip (40 ms each way), 20 ms of
    /// jitter each way, 2 % loss each way.
    pub fn profile_80ms() -> Self {
        let i = Impair { latency_ms: 40.0, jitter_ms: 20.0, loss: 0.02, duplicate: 0.0 };
        Conditions { send: i, recv: i }
    }

    /// A harsher path: 160 ms round trip, 40 ms jitter each way, 5 % loss, 1 % duplicates.
    pub fn profile_harsh() -> Self {
        let i = Impair { latency_ms: 80.0, jitter_ms: 40.0, loss: 0.05, duplicate: 0.01 };
        Conditions { send: i, recv: i }
    }

    /// `--net-sim LAT,JITTER,LOSS` (round-trip ms, ms each way, percent each way).
    pub fn parse(s: &str) -> Option<Self> {
        let v: Vec<f64> = s.split(',').map(|x| x.trim().parse().ok()).collect::<Option<Vec<_>>>()?;
        let (rtt, jitter, loss) = (*v.first()?, *v.get(1).unwrap_or(&0.0), *v.get(2).unwrap_or(&0.0));
        let i = Impair { latency_ms: rtt / 2.0, jitter_ms: jitter, loss: loss / 100.0, duplicate: 0.0 };
        Some(Conditions { send: i, recv: i })
    }

    pub fn is_none(&self) -> bool {
        self.send.is_none() && self.recv.is_none()
    }
}

/// xorshift64*: the conditioner's own dice (seeded, so a test run repeats).
#[derive(Clone)]
pub struct Dice(u64);

impl Dice {
    pub fn new(seed: u64) -> Self {
        Dice(seed.max(1))
    }
    pub fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct Held {
    due: Instant,
    order: u64,
    data: Vec<u8>,
    addr: SocketAddr,
}

impl PartialEq for Held {
    fn eq(&self, o: &Self) -> bool {
        self.due == o.due && self.order == o.order
    }
}
impl Eq for Held {}
impl PartialOrd for Held {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Held {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        // Min-heap on due time.
        o.due.cmp(&self.due).then(o.order.cmp(&self.order))
    }
}

struct Outbox {
    heap: BinaryHeap<Held>,
    order: u64,
    stop: bool,
}

/// Counters (bytes include the 28-byte IPv4/UDP header).
#[derive(Default, Debug, Clone, Copy)]
pub struct LinkStats {
    pub sent_packets: u64,
    pub sent_bytes: u64,
    pub recv_packets: u64,
    pub recv_bytes: u64,
    pub dropped_send: u64,
    pub dropped_recv: u64,
}

/// IPv4 + UDP header bytes, counted in every bandwidth figure.
pub const UDP_IP_OVERHEAD: u64 = 28;

pub struct Link {
    socket: UdpSocket,
    cond: Conditions,
    dice: Mutex<Dice>,
    outbox: Option<Arc<(Mutex<Outbox>, Condvar)>>,
    inbox: Mutex<(BinaryHeap<Held>, u64)>,
    thread: Option<std::thread::JoinHandle<()>>,
    stats: Mutex<LinkStats>,
    /// Test hook: while set, nothing is sent or received (a cable pulled).
    pub blackout: std::sync::atomic::AtomicBool,
}

impl Link {
    /// Bind (tests and benches bind 127.0.0.1 only).
    pub fn bind(addr: &str, cond: Conditions, seed: u64) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_read_timeout(Some(Duration::from_millis(2)))?;
        Ok(Self::from_socket(socket, cond, seed))
    }

    pub fn from_socket(socket: UdpSocket, cond: Conditions, seed: u64) -> Self {
        let (outbox, thread) = if cond.send.is_none() {
            (None, None)
        } else {
            let ob = Arc::new((Mutex::new(Outbox { heap: BinaryHeap::new(), order: 0, stop: false }), Condvar::new()));
            let s = ob.clone();
            let sock = socket.try_clone().expect("clone socket");
            let t = std::thread::Builder::new()
                .name("eo-net-delay".into())
                .spawn(move || {
                    let (m, cv) = &*s;
                    let mut g = m.lock().unwrap();
                    loop {
                        if g.stop {
                            return;
                        }
                        let now = Instant::now();
                        match g.heap.peek() {
                            Some(h) if h.due <= now => {
                                let h = g.heap.pop().unwrap();
                                drop(g);
                                let _ = sock.send_to(&h.data, h.addr);
                                g = m.lock().unwrap();
                            }
                            Some(h) => {
                                let wait = h.due - now;
                                g = cv.wait_timeout(g, wait).unwrap().0;
                            }
                            None => g = cv.wait(g).unwrap(),
                        }
                    }
                })
                .expect("spawn link delay thread");
            (Some(ob), Some(t))
        };
        Self {
            socket,
            cond,
            dice: Mutex::new(Dice::new(seed ^ 0x9E37_79B9_7F4A_7C15)),
            outbox,
            inbox: Mutex::new((BinaryHeap::new(), 0)),
            thread,
            stats: Mutex::new(LinkStats::default()),
            blackout: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.socket.local_addr().expect("local addr")
    }

    pub fn set_read_timeout(&self, d: Duration) {
        let _ = self.socket.set_read_timeout(Some(d));
    }

    pub fn stats(&self) -> LinkStats {
        *self.stats.lock().unwrap()
    }

    pub fn send(&self, data: &[u8], addr: SocketAddr) {
        if self.blackout.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        {
            let mut s = self.stats.lock().unwrap();
            s.sent_packets += 1;
            s.sent_bytes += data.len() as u64 + UDP_IP_OVERHEAD;
        }
        let Some(ob) = &self.outbox else {
            let _ = self.socket.send_to(data, addr);
            return;
        };
        let i = self.cond.send;
        let (lost, copies, delays) = {
            let mut d = self.dice.lock().unwrap();
            let lost = d.next_f64() < i.loss;
            let copies = if d.next_f64() < i.duplicate { 2 } else { 1 };
            let delays = [i.latency_ms + d.next_f64() * i.jitter_ms, i.latency_ms + d.next_f64() * i.jitter_ms];
            (lost, copies, delays)
        };
        if lost {
            self.stats.lock().unwrap().dropped_send += 1;
            return;
        }
        let (m, cv) = &**ob;
        let mut g = m.lock().unwrap();
        for d in delays.iter().take(copies) {
            g.order += 1;
            let order = g.order;
            g.heap.push(Held { due: Instant::now() + Duration::from_secs_f64(d / 1e3), order, data: data.to_vec(), addr });
        }
        cv.notify_one();
    }

    /// The next datagram, waiting at most the socket's read timeout.
    pub fn recv(&self, buf: &mut [u8]) -> Option<(usize, SocketAddr)> {
        let deadline = Instant::now() + Duration::from_millis(2);
        loop {
            if !self.cond.recv.is_none() {
                let mut ib = self.inbox.lock().unwrap();
                if ib.0.peek().is_some_and(|h| h.due <= Instant::now()) {
                    let h = ib.0.pop().unwrap();
                    let n = h.data.len().min(buf.len());
                    buf[..n].copy_from_slice(&h.data[..n]);
                    return Some((n, h.addr));
                }
            }
            match self.socket.recv_from(buf) {
                Ok((n, addr)) => {
                    if self.blackout.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }
                    {
                        let mut s = self.stats.lock().unwrap();
                        s.recv_packets += 1;
                        s.recv_bytes += n as u64 + UDP_IP_OVERHEAD;
                    }
                    if self.cond.recv.is_none() {
                        return Some((n, addr));
                    }
                    let i = self.cond.recv;
                    let (lost, copies, delays) = {
                        let mut d = self.dice.lock().unwrap();
                        (d.next_f64() < i.loss, if d.next_f64() < i.duplicate { 2 } else { 1 }, [i.latency_ms + d.next_f64() * i.jitter_ms, i.latency_ms + d.next_f64() * i.jitter_ms])
                    };
                    if lost {
                        self.stats.lock().unwrap().dropped_recv += 1;
                        continue;
                    }
                    let mut ib = self.inbox.lock().unwrap();
                    for d in delays.iter().take(copies) {
                        ib.1 += 1;
                        let order = ib.1;
                        ib.0.push(Held { due: Instant::now() + Duration::from_secs_f64(d / 1e3), order, data: buf[..n].to_vec(), addr });
                    }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if self.cond.recv.is_none() || Instant::now() >= deadline {
                        return None;
                    }
                }
                // Windows reports an ICMP port-unreachable from an earlier send as a
                // receive error (WSAECONNRESET): not ours to act on.
                Err(_) => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                }
            }
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        if let Some(ob) = &self.outbox {
            ob.0.lock().unwrap().stop = true;
            ob.1.notify_all();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
