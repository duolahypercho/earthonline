//! the bridges' night lights. The JS bridges, registry and pier
//! carry no light (README "Landmarks"); these are real-world light sources
//! placed on the ported models, each a `LandmarkLight` the sky slice's
//! clustered gather takes (eo_render::landmarks::sky_lamps).
//!
//! | Light | Where | Value | Source |
//! | --- | --- | --- | --- |
//! | Aviation obstruction lights | each tower leg's top, both bridges | 2,000 cd, aviation red, omni | FAA Advisory Circular 70/7460-1M, "Obstruction Marking and Lighting": the L-864 red obstruction beacon's night effective intensity, 2,000 cd. The beacon flashes (20-40 a minute); drawn steady here |
//! | Deck luminaires | both roadway edges, Golden Gate deck and the Bay Bridge's upper deck | a full-cutoff street luminaire: 30 lux pool peak (eo_sky photometry `LAMP_POOL_PEAK_LUX`, the street lamps' own value) from a 10 m head, 3,000 cd at the nadir; the street lamp role's colour | INFERRED head (10 m, a usual highway pole of 30-40 ft) and pitch (50 m, 5 mounting heights, the usual highway spacing ratio); the lamp model is the street slice's |
//! | Necklace lights | along the Bay Bridge west span's two main cables | 600 cd each, warm white (3,000 K), every 15 m | The west span's cable ("necklace") lights, strung on its suspension cables for the bridge's 50th anniversary in 1986. INFERRED: pitch and intensity (a 7,500 lm source radiating evenly, 7,500 / 4 pi = 600 cd; at 160 cd, a 2,000 lm lamp, the string did not show from the Embarcadero at the night exposure) |
//!
//! Intensities enter the gather as the props slice's lamp practicals do
//! (`intensity` is candela times `eo_sky::night::lamp_units_per_lux`).

use crate::LandmarkLight;
use crate::data::LandmarksJs;
use crate::geometry::GroundGrid;
use std::f64::consts::PI;

/// FAA AC 70/7460-1M, L-864 (red, night) effective intensity, candela.
pub const AVIATION_RED_CD: f64 = 2000.0;
/// Deck luminaire head over the deck, and the pitch along it, metres (INFERRED).
pub const DECK_HEAD_M: f64 = 10.0;
pub const DECK_PITCH_M: f64 = 50.0;
/// Necklace lamp intensity (candela) and pitch along a main cable (metres), INFERRED.
pub const NECKLACE_CD: f64 = 600.0;
pub const NECKLACE_PITCH_M: f64 = 15.0;

/// Linear colour at unit luminance of an sRGB hex.
pub(crate) fn unit_luminance(hex: u32) -> [f32; 3] {
    let c = crate::geometry::srgb_hex(hex);
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    [(c[0] / l) as f32, (c[1] / l) as f32, (c[2] / l) as f32]
}

/// Aviation red (FAA / CIE aviation red: chromaticity near x 0.70, y 0.29), as an sRGB tone.
const AVIATION_RED_SRGB: u32 = 0xff1a0a;
/// 3,000 K lamp white, as an sRGB tone.
pub(crate) const WARM_WHITE_SRGB: u32 = 0xffd2a1;

pub(crate) fn light(pos: [f64; 3], base: [f64; 3], colour: [f32; 3], candela: f64, range: f64, angle: f64, penumbra: f64) -> LandmarkLight {
    LandmarkLight {
        pos,
        base,
        colour_linear: colour,
        intensity: (candela * eo_sky::night::lamp_units_per_lux()) as f32,
        range_m: range as f32,
        decay: 2.0,
        angle: angle as f32,
        penumbra: penumbra as f32,
        source_lift_m: 0.0,
    }
}

/// The street luminaire as the street slice's lamps are (eo_sky lights `street_lamp`):
/// the nadir intensity for its pool, its cutoff cone, range 2.6 heads.
fn deck_lamp(at: [f64; 2], deck_y: f64) -> LandmarkLight {
    let head = DECK_HEAD_M;
    let cutoff = eo_sky::lights::STREET_CUTOFF_DEG.to_radians();
    let peak = eo_sky::lights::STREET_PEAK_DEG.to_radians();
    let c = eo_sky::lights::street_lamp_color();
    light([at[0], deck_y + head, at[1]], [at[0], deck_y, at[1]], [c[0] as f32, c[1] as f32, c[2] as f32], eo_sky::photometry::nadir_candela_for_pool(head), head * 2.6, cutoff, 1.0 - peak / cutoff)
}

fn aviation(at: [f64; 2], top_y: f64) -> LandmarkLight {
    light([at[0], top_y + 0.6, at[1]], [at[0], top_y, at[1]], unit_luminance(AVIATION_RED_SRGB), AVIATION_RED_CD, 160.0, PI / 2.0, 0.0)
}

/// Every bridge light, on the two bridges' own plans (the grounds they were built on).
pub fn bridge_lights(js: &LandmarksJs, gg_ground: &GroundGrid, bb_ground: &GroundGrid) -> Vec<LandmarkLight> {
    let mut out = Vec::new();
    let water = js.sea_level_y;
    // The Golden Gate: its four tower legs' tops and its deck's two edges.
    let g = &js.golden_gate;
    let top = water + crate::data::num(&g.dimensions, "towerAboveWaterM");
    for key in ["south", "north"] {
        let legs = &g.tower_legs[key];
        out.push(aviation(legs.east, top));
        out.push(aviation(legs.west, top));
    }
    let site = crate::golden_gate::Site::new(g);
    let ground = |x: f64, z: f64| gg_ground.at(x, z);
    let plan = crate::golden_gate::plan(js, &site, &ground);
    let half = crate::data::num(&g.dimensions, "roadwayM") / 2.0;
    let mut c = plan.c_start + DECK_PITCH_M / 2.0;
    while c < plan.c_end {
        let y = plan.deck_top(c);
        for s in [-half, half] {
            out.push(deck_lamp(site.deck_point(c, s), y));
        }
        c += DECK_PITCH_M;
    }
    // The Bay Bridge west span: its towers' leg tops, its upper deck's edges, the necklace on its main cables.
    let b = &js.bay_bridge_west;
    let bground = |x: f64, z: f64| bb_ground.at(x, z);
    let bp = crate::bay_bridge::plan(js, &bground);
    let f = crate::bay_bridge::frame(b);
    let hp = crate::data::num(&b.dimensions, "halfPlaneM");
    let btop = water + b.tower_top_above_water_m;
    for (_, _, tc) in &bp.towers {
        for s in [-hp, hp] {
            out.push(aviation(f.point(*tc, s), btop));
        }
    }
    let bhalf = crate::data::num(&b.dimensions, "roadwayM") / 2.0;
    let mut c = bp.c_a + DECK_PITCH_M / 2.0;
    while c < bp.c_b {
        let y = bp.deck_top(c);
        for s in [-bhalf, bhalf] {
            out.push(deck_lamp(f.point(c, s), y));
        }
        c += DECK_PITCH_M;
    }
    let runs = crate::bay_bridge::main_cable_runs(js, &bp);
    let warm = unit_luminance(WARM_WHITE_SRGB);
    let mut c = bp.c_a + NECKLACE_PITCH_M / 2.0;
    while c < bp.c_b {
        if let Some(y) = crate::suspension::cable_height_at(&runs, c) {
            for s in [-hp, hp] {
                let p = f.point(c, s);
                out.push(light([p[0], y - 0.6, p[1]], [p[0], y, p[1]], warm, NECKLACE_CD, 30.0, PI / 2.0, 0.0));
            }
        }
        c += NECKLACE_PITCH_M;
    }
    out
}
