//! The two deterministic draws the JS simulation uses, bit for bit.
//!
//! * `Mulberry32` is `mulberry32` in src/world/map/procedural/deterministic-random.js
//!   (re-exported by src/citygen/core.js), the seeded sequence TrafficSim
//!   spawns from (`mulberry32(seedInt + 77)`).
//! * `keyed_random` is `keyedRandom(key, salt)` in src/citygen/traffic.js:
//!   FNV-1a over the UTF-16 code units of `${key}|${salt}`, then a xorshift
//!   finish. It needs no shared state, so an agent's schedule does not depend
//!   on the order agents are stepped in - which is what lets the Rust step run
//!   agents in any order, on any thread, and still replay.
//!
//! Keys are assembled from parts (`Key::str`, `Key::int`) straight into the
//! hash, so a draw allocates nothing.

/// `mulberry32(seed)`.
#[derive(Clone, Debug)]
pub struct Mulberry32 {
    state: u32,
}

impl Mulberry32 {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// The next draw in [0, 1).
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b_79f5);
        let mut r = self.state;
        r = (r ^ (r >> 15)).wrapping_mul(r | 1);
        r ^= r.wrapping_add((r ^ (r >> 7)).wrapping_mul(r | 61));
        ((r ^ (r >> 14)) as f64) / 4_294_967_296.0
    }
}

/// An FNV-1a hash being fed the characters of `${key}|${salt}`.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    h: u32,
}

impl Default for Key {
    fn default() -> Self {
        Self::new()
    }
}

impl Key {
    pub const fn new() -> Self {
        Self { h: 0x811c_9dc5 }
    }

    #[inline]
    fn byte(mut self, c: u32) -> Self {
        self.h ^= c;
        self.h = self.h.wrapping_mul(0x0100_0193);
        self
    }

    /// Append a string (ASCII keys: one UTF-16 unit per byte; other chars by code unit).
    #[inline]
    pub fn str(mut self, s: &str) -> Self {
        for unit in s.encode_utf16() {
            self = self.byte(unit as u32);
        }
        self
    }

    /// Append the decimal text of a non-negative integer, as JS template literals print it.
    #[inline]
    pub fn int(mut self, mut v: u64) -> Self {
        let mut digits = [0u8; 20];
        let mut n = 0;
        loop {
            digits[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while n > 0 {
            n -= 1;
            self = self.byte(digits[n] as u32);
        }
        self
    }

    /// The `|` between key and salt.
    #[inline]
    pub fn sep(self) -> Self {
        self.byte(b'|' as u32)
    }

    /// The finished draw in [0, 1).
    #[inline]
    pub fn draw(self) -> f64 {
        let mut h = self.h;
        h = (h ^ (h >> 15)).wrapping_mul(0x2545_f491);
        h ^= h >> 13;
        h as f64 / 4_294_967_296.0
    }
}

/// `keyedRandom(key, salt)` for plain string parts.
pub fn keyed_random(key: &str, salt: &str) -> f64 {
    Key::new().str(key).sep().str(salt).draw()
}

/// `ambient-<n>`: the activity key every ambient walker carries.
#[inline]
pub fn walker_key(n: u32) -> Key {
    Key::new().str("ambient-").int(n as u64)
}

/// `keyedRandom(walkerKey, '<prefix><n>')`, the per-leg schedule draws.
#[inline]
pub fn walker_draw(walker: u32, salt_prefix: &str, n: u32) -> f64 {
    walker_key(walker).sep().str(salt_prefix).int(n as u64).draw()
}

/// `keyedRandom(walkerKey, salt)`.
#[inline]
pub fn walker_draw_salt(walker: u32, salt: &str) -> f64 {
    walker_key(walker).sep().str(salt).draw()
}

/// `driverPaceFactor(entityId, seedInt)` in traffic.js: `keyedRandom(`${seedInt||1}:${id}`, 'driver-speed')`.
pub fn driver_pace(entity: u32, seed_int: u32, pace_min: f64, pace_span: f64) -> f64 {
    let seed = if seed_int == 0 { 1 } else { seed_int };
    let draw = Key::new().int(seed as u64).str(":").int(entity as u64).sep().str("driver-speed").draw();
    (pace_min + draw * pace_span).min(1.0)
}

/// FNV-1a over a string, `hashString` in deterministic-random.js (stable ids for tiles and regions).
pub fn hash_string(s: &str) -> u32 {
    let mut h: u32 = 2_166_136_261;
    for unit in s.encode_utf16() {
        h ^= unit as u32;
        h = h.wrapping_mul(16_777_619);
    }
    h
}
