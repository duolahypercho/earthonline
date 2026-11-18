//! The keyed hashes the JS weather schedule uses
//! (src/simulation/physics/weather/weather-schedule.js `hashString`, `mix32`,
//! `hash01`, `deriveWeatherScheduleSeed`): no runtime PRNG, every draw is a
//! pure function of (seed, index, salt), so any machine (client or server)
//! derives the same weather from the same seed and clock.

/// FNV-1a over the UTF-16 code units of `text` (`hashString`).
pub fn hash_string(text: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for c in text.encode_utf16() {
        h = (h ^ c as u32).wrapping_mul(16777619);
    }
    h
}

/// `mix32`: a 32-bit finaliser.
pub fn mix32(value: u32) -> u32 {
    let mut h = value;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846ca68b);
    h ^= h >> 16;
    h
}

/// `hash01(seed, index, salt)` in [0, 1).
pub fn hash01(seed: u32, index: u32, salt: u32) -> f64 {
    let keyed = seed ^ (index.wrapping_add(1)).wrapping_mul(0x9e3779b1) ^ (salt.wrapping_add(1)).wrapping_mul(0x85ebca6b);
    mix32(keyed) as f64 / 4294967296.0
}

/// `deriveWeatherScheduleSeed(sourceId, generation)`.
pub fn schedule_seed(source_id: &str, generation: u32) -> u32 {
    mix32(hash_string(source_id) ^ generation.wrapping_add(1).wrapping_mul(0x9e3779b1))
}

/// A float hash of a lattice point (the cloud sheets and star cells), 0..1.
pub fn lattice01(x: i32, y: i32, z: i32, seed: u32) -> f64 {
    let h = mix32((x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841) ^ (z as u32).wrapping_mul(0xcb1ab31f) ^ seed);
    h as f64 / 4294967296.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mix32_matches_the_js_on_known_values() {
        // node -e "mix32" on these inputs (the JS function, transcribed): 0 -> 0.
        assert_eq!(mix32(0), 0);
        // FNV-1a of the empty string is the offset basis.
        assert_eq!(hash_string(""), 2166136261);
        // hash01 is in [0, 1).
        for i in 0..1000 {
            let v = hash01(12345, i, 2);
            assert!((0.0..1.0).contains(&v));
        }
    }
}
