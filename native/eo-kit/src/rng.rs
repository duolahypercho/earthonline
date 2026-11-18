//! `hashSeed` and `createRandom` from src/world/buildings/authored/kit/frame.js.

/// Stable string -> u32 (FNV-1a over UTF-16 code units, final avalanche).
/// The JS hashes `n:<value>` for a number argument: use [`hash_seed_num`].
pub fn hash_seed(text: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for c in text.encode_utf16() {
        h = (h ^ c as u32).wrapping_mul(0x01000193);
    }
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^= h >> 16;
    h
}

/// `hashSeed(number)`: the JS hashes the text `n:<number>`.
pub fn hash_seed_num(v: f64) -> u32 {
    hash_seed(&format!("n:{}", js_num(v)))
}

/// A number as JS `String(n)` writes it, for the values seeds are made of
/// (integers and short decimals; no exponent forms).
pub fn js_num(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if v == 0.0 {
        return "0".into();
    }
    // Rust's Display for f64 is the shortest round-trip form, without an exponent.
    format!("{v}")
}

/// `createRandom(seed)`: mulberry32 seeded from `hashSeed(seed) || 1`.
#[derive(Clone, Debug)]
pub struct Rng {
    a: u32,
}

impl Rng {
    /// `createRandom(text)` for a string seed.
    pub fn new(seed: &str) -> Self {
        Self::from_hash(hash_seed(seed))
    }
    /// `createRandom(number)`.
    pub fn from_num(seed: f64) -> Self {
        Self::from_hash(hash_seed_num(seed))
    }
    pub fn from_hash(h: u32) -> Self {
        Self { a: if h == 0 { 1 } else { h } }
    }
    /// The next value in [0, 1).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b79f5);
        let a = self.a;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        ((t ^ (t >> 14)) as f64) / 4294967296.0
    }
}

/// Seed value the JS passes to `createRandom`/`hashSeed`: numbers hash as
/// `n:<n>`, strings as themselves.
pub enum Seed<'a> {
    Num(f64),
    Str(&'a str),
}

pub fn hash_of(seed: Seed) -> u32 {
    match seed {
        Seed::Num(n) => hash_seed_num(n),
        Seed::Str(s) => hash_seed(s),
    }
}
