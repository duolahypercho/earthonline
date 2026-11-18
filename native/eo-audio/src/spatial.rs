//! 3D placement as pure functions: distance attenuation, equal-power panning
//! relative to the listener, Doppler, and the air/rear high cut. World axes are
//! the game's: x east, y up, z south, metres.

/// Speed of sound in air at 20 C, m/s.
pub const SPEED_OF_SOUND: f32 = 343.0;
/// Doppler ratios are held inside this range (a car at 100 m/s still sounds like a car).
pub const DOPPLER_MIN: f32 = 0.5;
pub const DOPPLER_MAX: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub pos: [f32; 3],
    /// Unit view direction.
    pub forward: [f32; 3],
    /// Unit up (the world's +y unless the camera rolls).
    pub up: [f32; 3],
    /// m/s, for Doppler.
    pub vel: [f32; 3],
}

impl Default for Listener {
    fn default() -> Self {
        Self { pos: [0.0; 3], forward: [0.0, 0.0, -1.0], up: [0.0, 1.0, 0.0], vel: [0.0; 3] }
    }
}

/// How a 3D source fades with distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Falloff {
    /// Full level inside this distance, m.
    pub ref_m: f32,
    /// Silent beyond this distance (a fade over its last 20 %), m.
    pub max_m: f32,
    /// 1 = inverse distance (-6 dB per doubling); 0.5 = a line source such as a road (-3 dB).
    pub rolloff: f32,
}

impl Falloff {
    pub const fn new(ref_m: f32, max_m: f32, rolloff: f32) -> Self {
        Self { ref_m, max_m, rolloff }
    }
}

#[inline]
pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
pub fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
#[inline]
pub fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = len(a);
    if l > 1e-9 { [a[0] / l, a[1] / l, a[2] / l] } else { [0.0, 0.0, 0.0] }
}

/// Distance gain: `(ref / max(d, ref))^rolloff`, faded to zero over the last
/// 20 % before `max_m`.
pub fn attenuation(d: f32, f: &Falloff) -> f32 {
    let r = f.ref_m.max(0.01);
    let g = (r / d.max(r)).powf(f.rolloff);
    let fade_start = f.max_m * 0.8;
    if d >= f.max_m {
        0.0
    } else if d > fade_start {
        let t = (d - fade_start) / (f.max_m - fade_start);
        g * (1.0 - t * t * (3.0 - 2.0 * t))
    } else {
        g
    }
}

/// Equal-power pan law: `x` in -1 (left) .. +1 (right) to (left, right) gains;
/// both 0.7071 (-3 dB) at the centre.
pub fn pan_gains(x: f32) -> (f32, f32) {
    let t = (x.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (t.cos(), t.sin())
}

/// Where a source sits for the listener: (distance m, lateral -1..1, frontal -1..1).
/// Inside `near_m` the image widens to the centre (a source on the listener has no side).
pub fn locate(l: &Listener, pos: [f32; 3], near_m: f32) -> (f32, f32, f32) {
    let v = sub(pos, l.pos);
    let d = len(v);
    if d < 1e-4 {
        return (0.0, 0.0, 1.0);
    }
    let dir = [v[0] / d, v[1] / d, v[2] / d];
    let right = normalize(cross(l.forward, l.up));
    let mut x = dot(dir, right);
    let z = dot(dir, l.forward);
    if d < near_m {
        x *= d / near_m;
    }
    (d, x, z)
}

/// Doppler ratio heard by the listener: `(c + v_l) / (c - v_s)`, with v_l the
/// listener's speed toward the source and v_s the source's speed toward the
/// listener; `factor` scales the shift (1 = physical).
pub fn doppler(l: &Listener, src_pos: [f32; 3], src_vel: [f32; 3], factor: f32) -> f32 {
    let v = sub(l.pos, src_pos);
    let d = len(v);
    if d < 1e-3 || factor <= 0.0 {
        return 1.0;
    }
    let u = [v[0] / d, v[1] / d, v[2] / d]; // source -> listener
    let c = SPEED_OF_SOUND;
    let vs = (dot(src_vel, u) * factor).clamp(-0.9 * c, 0.9 * c);
    let vl = (-dot(l.vel, u) * factor).clamp(-0.9 * c, 0.9 * c);
    ((c + vl) / (c - vs)).clamp(DOPPLER_MIN, DOPPLER_MAX)
}

/// The high cut a distant or rear source gets, Hz: the air and the ground take
/// the top off far sounds; the head shadows a source behind.
pub fn air_cutoff_hz(d: f32, frontal: f32) -> f32 {
    let air = (22_000.0 / (1.0 + d / 40.0)).clamp(1_500.0, 20_000.0);
    let rear = if frontal < 0.0 { 1.0 - 0.45 * (-frontal) } else { 1.0 };
    air * rear
}

/// One-pole low-pass coefficient for a cutoff at a sample rate (`y += a (x - y)`).
pub fn one_pole_coeff(cutoff_hz: f32, rate: f32) -> f32 {
    let c = cutoff_hz.clamp(10.0, rate * 0.49);
    1.0 - (-2.0 * std::f32::consts::PI * c / rate).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn inverse_distance_halves_per_doubling() {
        let f = Falloff::new(2.0, 1000.0, 1.0);
        assert_eq!(attenuation(0.5, &f), 1.0, "inside the reference distance: full level");
        assert!(near(attenuation(4.0, &f), 0.5, 1e-6));
        assert!(near(attenuation(8.0, &f), 0.25, 1e-6));
        // -6.02 dB per doubling.
        let db = 20.0 * (attenuation(16.0, &f) / attenuation(8.0, &f)).log10();
        assert!(near(db, -6.0206, 1e-3), "{db}");
    }

    #[test]
    fn line_source_loses_three_db_per_doubling() {
        let f = Falloff::new(1.0, 1000.0, 0.5);
        let db = 20.0 * (attenuation(20.0, &f) / attenuation(10.0, &f)).log10();
        assert!(near(db, -3.0103, 1e-3), "{db}");
    }

    #[test]
    fn silent_at_and_beyond_max_and_monotonic() {
        let f = Falloff::new(1.0, 100.0, 1.0);
        assert_eq!(attenuation(100.0, &f), 0.0);
        assert_eq!(attenuation(250.0, &f), 0.0);
        let mut last = f32::MAX;
        for i in 0..400 {
            let g = attenuation(i as f32 * 0.25, &f);
            assert!(g <= last + 1e-7, "not monotonic at {}", i as f32 * 0.25);
            last = g;
        }
        // Continuous where the fade starts.
        assert!(near(attenuation(79.999, &f), attenuation(80.001, &f), 1e-4));
    }

    #[test]
    fn equal_power_pan() {
        for i in 0..=20 {
            let x = -1.0 + i as f32 * 0.1;
            let (l, r) = pan_gains(x);
            assert!(near(l * l + r * r, 1.0, 1e-5), "power at {x}");
        }
        let (l, r) = pan_gains(0.0);
        assert!(near(l, r, 1e-6) && near(l, std::f32::consts::FRAC_1_SQRT_2, 1e-6));
        assert_eq!(pan_gains(-1.0).1.abs() < 1e-6, true);
        assert_eq!(pan_gains(1.0).0.abs() < 1e-6, true);
    }

    #[test]
    fn sides_follow_the_view() {
        // Looking north (-z): east (+x) is on the right.
        let l = Listener { forward: [0.0, 0.0, -1.0], ..Default::default() };
        let (d, x, z) = locate(&l, [10.0, 0.0, 0.0], 1.0);
        assert!(near(d, 10.0, 1e-5) && near(x, 1.0, 1e-5) && near(z, 0.0, 1e-5));
        let (_, x, _) = locate(&l, [-10.0, 0.0, 0.0], 1.0);
        assert!(near(x, -1.0, 1e-5));
        let (_, x, z) = locate(&l, [0.0, 0.0, -10.0], 1.0);
        assert!(near(x, 0.0, 1e-5) && near(z, 1.0, 1e-5), "ahead");
        // Turned to look east, the same east source is ahead.
        let l2 = Listener { forward: [1.0, 0.0, 0.0], ..Default::default() };
        let (_, x, z) = locate(&l2, [10.0, 0.0, 0.0], 1.0);
        assert!(near(x, 0.0, 1e-5) && near(z, 1.0, 1e-5));
        // Inside the near field the image narrows to the centre.
        let (_, x, _) = locate(&l, [0.25, 0.0, 0.0], 1.0);
        assert!(near(x, 0.25, 1e-5));
    }

    #[test]
    fn doppler_matches_the_formula() {
        let l = Listener::default();
        // A car 50 m east driving west (toward the listener) at 15 m/s.
        let up = doppler(&l, [50.0, 0.0, 0.0], [-15.0, 0.0, 0.0], 1.0);
        assert!(near(up, 343.0 / (343.0 - 15.0), 1e-5), "{up}");
        // Driving away.
        let down = doppler(&l, [50.0, 0.0, 0.0], [15.0, 0.0, 0.0], 1.0);
        assert!(near(down, 343.0 / (343.0 + 15.0), 1e-5), "{down}");
        // Passing broadside: no shift.
        let side = doppler(&l, [0.0, 0.0, 20.0], [15.0, 0.0, 0.0], 1.0);
        assert!(near(side, 1.0, 1e-6));
        // The listener moving toward a still source.
        let lm = Listener { vel: [10.0, 0.0, 0.0], ..Default::default() };
        let r = doppler(&lm, [50.0, 0.0, 0.0], [0.0; 3], 1.0);
        assert!(near(r, (343.0 + 10.0) / 343.0, 1e-5));
        // Clamped.
        assert_eq!(doppler(&l, [50.0, 0.0, 0.0], [-340.0, 0.0, 0.0], 1.0), DOPPLER_MAX);
    }

    #[test]
    fn far_and_rear_sources_are_duller() {
        assert!(air_cutoff_hz(200.0, 1.0) < air_cutoff_hz(20.0, 1.0));
        assert!(air_cutoff_hz(20.0, -1.0) < air_cutoff_hz(20.0, 1.0));
        assert!(air_cutoff_hz(5000.0, 1.0) >= 1_500.0 * 0.55 - 1e-3);
        let a = one_pole_coeff(1000.0, 48_000.0);
        assert!(a > 0.0 && a < 1.0);
    }
}
