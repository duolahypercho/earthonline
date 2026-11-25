//! (`--lamp-units`) The street lamps as a city's units rather than one
//! dimmer: each lamp switches on by its own control as the sky darkens past
//! its level, and carries its own light output, colour bin and fault. Pure
//! functions and the shaders' constants; the renderer applies them (eo-render
//! look.rs `LookGpu::lamp_units`, sky_lights.wgsl, hlod_lights.wgsl,
//! props_draw.wgsl). Without the switch nothing here runs.
//!
//! Why. The JS rule (`night::lamps_on`, 1 - smoothstep(-3, 8, sun altitude))
//! turns every lamp of the city up together as one dimmer: at the golden hour
//! (sun +6 deg) every lamp burns at 9 % in full sun, so the lenses and their
//! sprites read as identical blown points in daylight and a perfectly regular
//! dotted line from the air. A street lamp is on or off (its photocontrol or
//! its time switch), and the lamps of a street differ unit to unit.
//!
//! | What | Value | Source |
//! | --- | --- | --- |
//! | Switching | each unit's turn-on level log-uniform between the clear sky's horizontal illuminance with the sun 1 deg up (~1,090 lux) and 2 deg down (189 lux), against that illuminance times the weather's dome brightness | INFERRED: the city's lamps come on around sunset, unit to unit over a few minutes (photocontrols' turn-on tolerance, ANSI C136.10: 1.0-3.0 fc at the cell, and their orientation and dirt); the band's ends set so every lamp is off with the sun up a degree and on with it two below; `photo::clear_global_lux` the sky's illuminance |
//! | Light output | uniform in [0.8, 1.2] of the design | IES RP-8-18 light loss: lumen depreciation and dirt take a unit to ~0.8 of new over its life, a new unit runs ~1.1-1.2 of the maintained design value; mean 1 keeps the RP-8 pool |
//! | Colour bin | +-175 K around the lamp's CCT, linear in that | ANSI C78.377-2017: a nominal 3000 K LED's quadrangle spans 2870-3220 K (7-step) |
//! | Failed | 2 % of units dark at night | INFERRED: a maintained city's outage share at any time (municipal outage reports, 1-3 %) |

use crate::photo::clear_global_lux;

/// The sun altitudes whose clear-sky illuminances bound the units' turn-on levels (deg).
pub const ON_SUN_HIGH_DEG: f64 = 1.0;
pub const ON_SUN_LOW_DEG: f64 = -2.0;
/// A unit's light output over the design (IES RP-8 light loss).
pub const OUTPUT_LO: f64 = 0.8;
pub const OUTPUT_HI: f64 = 1.2;
/// A unit's CCT bin either side of its lamp's nominal (K; ANSI C78.377 at 3000 K).
pub const CCT_BIN_K: f64 = 175.0;
/// The nominal CCT the bins are measured against (K; SF's LED street lamps, `cine_lamps::SF_LED_CCT_K`).
pub const NOMINAL_CCT_K: f64 = crate::lights::SF_LED_CCT_K;
/// The share of units dark with a fault at night.
pub const FAILED_SHARE: f64 = 0.02;

/// The horizontal illuminance (lux) the units' turn-on levels span: (low, high).
pub fn turn_on_band_lux() -> (f64, f64) {
    (clear_global_lux(ON_SUN_LOW_DEG), clear_global_lux(ON_SUN_HIGH_DEG))
}

/// The share of the city's units switched on (0..1) with the sun at
/// `sun_alt_deg` under a sky of dome brightness `dome` (the weather's: 1 clear,
/// lower overcast: the lamps come on earlier under cloud). Each unit is on when
/// the sky's illuminance is under its level; the levels are log-uniform over
/// `turn_on_band_lux`, so the share is the log position of the sky in the band.
pub fn population_on(sun_alt_deg: f64, dome: f64) -> f64 {
    let e = clear_global_lux(sun_alt_deg) * dome.clamp(1e-3, 1.0);
    let (lo, hi) = turn_on_band_lux();
    ((hi / e).ln() / (hi / lo).ln()).clamp(0.0, 1.0)
}

/// The CCT bin's tint at `cct` over the nominal's (linear sRGB, unit luminance
/// ratio; the colour the eye adapted to the nominal sees).
fn bin_tint(cct: f64) -> [f64; 3] {
    let d = crate::lights::degree_of_adaptation(crate::lights::NIGHT_ADAPTING_CD_M2, crate::lights::DARK_SURROUND_F);
    let c = crate::shops::adapted_source(cct, NOMINAL_CCT_K, d);
    let n = crate::shops::adapted_source(NOMINAL_CCT_K, NOMINAL_CCT_K, d);
    [c[0] / n[0], c[1] / n[1], c[2] / n[2]]
}

/// The warm and cool ends of the bins as tints on a lamp's colour.
pub fn bin_tints() -> ([f64; 3], [f64; 3]) {
    (bin_tint(NOMINAL_CCT_K - CCT_BIN_K), bin_tint(NOMINAL_CCT_K + CCT_BIN_K))
}

/// A unit's seed from its post's foot (eo-app props: the props lamps and their
/// prop instance carry it, so a unit's light and its lens agree).
pub fn seed_of(base: [f64; 3]) -> u32 {
    let x = (base[0] * 100.0).round() as i64 as u64;
    let z = (base[2] * 100.0).round() as i64 as u64;
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in x.to_le_bytes().into_iter().chain(z.to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    ((h >> 32) as u32) ^ (h as u32) | 1
}

/// The shaders' hash (PCG, Jarzynski & Olano 2020), as WGSL's `lu_hash`.
pub fn hash(x: u32) -> u32 {
    let s = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277_803_737);
    (w >> 22) ^ w
}

fn u01(x: u32) -> f64 {
    (hash(x) >> 8) as f64 / 16_777_216.0
}

/// A unit's light at the population share `p`, as the shaders' `lu_factor`:
/// 0 off or failed, else its output over `p` (the renderer's lamp scales carry
/// `p`, so the units' mean is the population's light).
pub fn factor(seed: u32, p: f64) -> f64 {
    let on = u01(seed) < p;
    let failed = u01(seed ^ 0x9e37_79b9) < FAILED_SHARE;
    let out = OUTPUT_LO + (OUTPUT_HI - OUTPUT_LO) * u01(seed ^ 0x85eb_ca6b);
    if on && !failed { out / p.max(1e-4) } else { 0.0 }
}

/// The shaders' constants and functions (`LU_ON` and `LU_BIT` are the renderer's).
pub fn wgsl() -> String {
    let (warm, cool) = bin_tints();
    let v3 = |v: [f64; 3]| format!("vec3<f32>({:.6}, {:.6}, {:.6})", v[0], v[1], v[2]);
    format!(
        "\n// generated by eo-sky lamp_units::wgsl
const LU_OUTPUT: vec2<f32> = vec2<f32>({OUTPUT_LO:.4}, {OUTPUT_HI:.4});
const LU_FAILED_SHARE: f32 = {FAILED_SHARE:.4};
const LU_TINT_WARM: vec3<f32> = {};
const LU_TINT_COOL: vec3<f32> = {};
fn lu_hash(x: u32) -> u32 {{
    let s = x * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}}
fn lu_u01(x: u32) -> f32 {{
    return f32(lu_hash(x) >> 8u) / 16777216.0;
}}
/// A unit's seed from where it stands (the fallback and far lamps: the same f32 position in both).
fn lu_seed_at(p: vec3<f32>) -> u32 {{
    return lu_hash(bitcast<u32>(p.x) ^ lu_hash(bitcast<u32>(p.z) + 1759714724u)) | 1u;
}}
/// (`--lamp-units`) the unit's light at the population share `p` (`skyu.night.x`):
/// 0 off or failed, else its output over `p`; 1 with the switch off.
fn lu_factor(seed: u32, p: f32, on: bool) -> f32 {{
    if (!LU_ON || !on) {{
        return 1.0;
    }}
    let lit = lu_u01(seed) < p;
    let failed = lu_u01(seed ^ 2654435769u) < LU_FAILED_SHARE;
    let out = mix(LU_OUTPUT.x, LU_OUTPUT.y, lu_u01(seed ^ 2246822507u));
    return select(0.0, out / max(p, 1e-4), lit && !failed);
}}
/// (`--lamp-units`) the unit's colour bin as a tint on its lamp's colour (1 off).
fn lu_tint(seed: u32, on: bool) -> vec3<f32> {{
    if (!LU_ON || !on) {{
        return vec3<f32>(1.0);
    }}
    return mix(LU_TINT_WARM, LU_TINT_COOL, lu_u01(seed ^ 3266489909u));
}}
",
        v3(warm),
        v3(cool)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unit_is_off_in_sun_and_on_by_dusk() {
        assert_eq!(population_on(6.0, 1.0), 0.0, "golden hour: no unit on");
        assert_eq!(population_on(ON_SUN_HIGH_DEG + 0.01, 1.0), 0.0);
        assert_eq!(population_on(-2.3, 1.0), 1.0, "the blue hour (embarcadero's -2.3 deg): all on");
        assert_eq!(population_on(-20.0, 1.0), 1.0);
        let mid = population_on(-0.5, 1.0);
        assert!(mid > 0.2 && mid < 0.8, "around sunset some are on: {mid}");
        // monotone as the sun sets
        let mut last = 0.0;
        for k in 0..=60 {
            let p = population_on(3.0 - k as f64 * 0.1, 1.0);
            assert!(p >= last - 1e-12);
            last = p;
        }
    }

    #[test]
    fn an_overcast_sky_turns_them_on_earlier() {
        assert!(population_on(0.0, 0.3) > population_on(0.0, 1.0));
        assert_eq!(population_on(0.0, 0.2), 1.0, "a dark overcast sunset: all on");
    }

    #[test]
    fn the_units_mean_is_the_populations_light() {
        for p in [0.1, 0.5, 1.0] {
            let n = 20000u32;
            let mean = (1..=n).map(|i| factor(hash(i) | 1, p)).sum::<f64>() / n as f64;
            let want = (1.0 - FAILED_SHARE) * (OUTPUT_LO + OUTPUT_HI) / 2.0;
            assert!((mean - want).abs() < 0.03, "p {p}: mean {mean} want {want}");
        }
        assert_eq!(factor(12345, 0.0), 0.0);
    }

    #[test]
    fn the_bins_straddle_the_nominal() {
        let (warm, cool) = bin_tints();
        assert!(warm[0] > 1.0 && warm[2] < 1.0, "warm: more red, less blue {warm:?}");
        assert!(cool[0] < 1.0 && cool[2] > 1.0, "cool: less red, more blue {cool:?}");
        for t in [warm, cool] {
            for c in t {
                assert!((c - 1.0).abs() < 0.2, "a bin is a small shift: {t:?}");
            }
        }
    }

    #[test]
    fn seeds_are_stable_and_spread() {
        let a = seed_of([1712.49, 19.1, -1438.84]);
        assert_eq!(a, seed_of([1712.4901, 0.0, -1438.8402]), "the height and sub-cm noise do not move it");
        assert_ne!(a, seed_of([1712.50, 19.1, -1438.84]));
        // the factor is a pure function of the seed (the same unit every frame and run)
        assert_eq!(factor(a, 0.7), factor(a, 0.7));
    }
}
