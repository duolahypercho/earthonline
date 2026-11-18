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
//! | Tower floodlights (`BridgeFeatures::night`) | the Golden Gate towers above the deck | 24 x 400 W HPS, 6 a leg at the sidewalk, aimed up; a flux-conserving mean on the legs (`gg_floodlit`) | goldengate.org "Color & Art Deco Styling" (24 sidewalk-level lights, lit 22 June 1987); ANSI C78.42 S51 (50,000 lm) |
//! | Tower lights below the roadway (`BridgeFeatures::floods`) | the Golden Gate towers' legs under the deck | 12 a tower: 4 x 150 W, 4 x 250 W, 4 x 400 W HPS, on the pier round the legs, aimed up (`gg_floodlit_below`) | goldengate.org "Color & Art Deco Styling"; ANSI C78.42 S55 (16,000 lm), S50 (27,500 lm), S51. INFERRED: their placing and aim |
//! | Suspender LEDs (`BridgeFeatures::night`) | the west span's suspender ropes, both faces | 1 cd every 0.3 m, white, a slow sequence | "The Bay Lights" (25,000 LEDs on 300 cables, 2013), "Bay Lights 360" (about 48,000-50,000, both faces, 20 March 2026). INFERRED: the intensity |
//!
//! Intensities enter the gather as the props slice's lamp practicals do
//! (`intensity` is candela times `eo_sky::night::lamp_units_per_lux`). The
//! floods and the LEDs are shaded by the landmark surfaces and the wire pass
//! (eo-render landmarks_surface.wgsl `lm_floodlit`, landmarks_wire.wgsl
//! `lm_led`), not gathered: an up-aimed flood is not a street luminaire (the
//! gather would draw its source and a pool under it), and 48,000 LEDs are not
//! lamps for the clusters.

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
fn unit_luminance(hex: u32) -> [f32; 3] {
    let c = crate::geometry::srgb_hex(hex);
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    [(c[0] / l) as f32, (c[1] / l) as f32, (c[2] / l) as f32]
}

/// Aviation red (FAA / CIE aviation red: chromaticity near x 0.70, y 0.29), as an sRGB tone.
const AVIATION_RED_SRGB: u32 = 0xff1a0a;
/// 3,000 K lamp white, as an sRGB tone.
const WARM_WHITE_SRGB: u32 = 0xffd2a1;

fn light(pos: [f64; 3], base: [f64; 3], colour: [f32; 3], candela: f64, range: f64, angle: f64, penumbra: f64) -> LandmarkLight {
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
    let g = &crate::golden_gate::data(js);
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
    let b = &crate::bay_bridge::data(js);
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

/// The Golden Gate towers' floodlighting (`BridgeFeatures::night`), as the
/// landmark surface shades it: "24 sidewalk-level lights, with six at each of
/// the four tower legs" aimed up the towers (goldengate.org, "Color & Art
/// Deco Styling"; lit since 22 June 1987), 400 W high-pressure sodium
/// floodlights. A 400 W HPS lamp gives 50,000 lm (ANSI C78.42, lamp S51);
/// INFERRED: 60 % of it in the beam and half of that landing on the leg, over
/// the leg's two along faces and its outer face above the deck: the mean
/// illuminance `lux`, flux conserved, laid on the faces from 1.4 x at the deck
/// to 0.6 x at the top (the floods stand at the deck).
#[derive(Clone, Copy, Debug)]
pub struct Floodlit {
    /// The south tower's centre on the tower line (x, z), the span's axis, the main span.
    pub origin: [f64; 2],
    pub axis: [f64; 2],
    pub span: f64,
    pub half_plane: f64,
    pub deck_y: f64,
    pub top_y: f64,
    /// The largest half sizes of a leg above the deck (along the span, across it).
    pub half_along: f64,
    pub half_across: f64,
    pub lux: f64,
    /// Linear colour at unit luminance.
    pub colour: [f32; 3],
}

/// HPS lamp white (chromaticity near x 0.52, y 0.42; ~2,100 K), as an sRGB tone. INFERRED rendition.
const HPS_SRGB: u32 = 0xffa552;
/// ANSI C78.42 S51 (400 W HPS) rated initial lumens.
pub const HPS_400W_LM: f64 = 50_000.0;
pub const FLOODS_PER_LEG: f64 = 6.0;

pub fn gg_floodlit(js: &LandmarksJs) -> Floodlit {
    let g = crate::golden_gate::data(js);
    let d = &g.dimensions;
    let site = crate::golden_gate::Site::new(&g);
    let water = js.sea_level_y;
    let deck_y = water + crate::data::num(d, "roadwayAtTowerM");
    let top_y = water + crate::data::num(d, "towerAboveWaterM");
    let above: Vec<&[f64; 4]> = g.leg_stages.iter().filter(|s| water + s[1] > deck_y).collect();
    let half_along = above.iter().map(|s| s[3] / 2.0).fold(0.0, f64::max);
    let half_across = above.iter().map(|s| s[2] / 2.0).fold(0.0, f64::max);
    // The lit area of one leg: its two along faces (the leg's width across) and its outer face (its depth along), stage by stage.
    let area: f64 = above.iter().map(|s| (2.0 * s[2] + s[3]) * ((water + s[1]).min(top_y) - (water + s[0]).max(deck_y)).max(0.0)).sum();
    let lux = FLOODS_PER_LEG * HPS_400W_LM * 0.6 * 0.5 / area.max(1.0);
    Floodlit { origin: site.south_tower, axis: site.axis, span: site.main_span_measured, half_plane: site.half_plane, deck_y, top_y, half_along, half_across, lux, colour: unit_luminance(HPS_SRGB) }
}

/// The west span's suspender LEDs (`BridgeFeatures::night`): "The Bay Lights",
/// 25,000 white LEDs on 300 vertical cables (2013; Architect, "25,000 LEDs
/// Light Up the San Francisco-Oakland Bay Bridge"), relit on 20 March 2026 as
/// "Bay Lights 360" with about 48,000-50,000 on both faces of the cables
/// (KQED; SFist). 25,000 / 300 is 83 a cable face; over a mean cable of about
/// 25 m that is one every 0.3 m. INFERRED: 1 cd an LED seen from the side
/// (a diffused white node of a few tens of lumens; at 3 cd every rope burnt
/// out white from the Embarcadero and the sequence did not read).
pub const LED_CD: f64 = 1.0;
pub const LED_PITCH_M: f64 = 0.3;

/// The west span's frame origin (W2) and axis (for the LEDs' sequence along it).
pub fn bay_axis(js: &LandmarksJs) -> ([f64; 2], [f64; 2]) {
    let f = crate::bay_bridge::frame(&js.bay_bridge_west);
    (f.origin, f.axis)
}

/// One of a Golden Gate tower's floods (`BridgeFeatures::floods`), in the
/// tower's own frame: `c` along the span from the tower's centre line, `s`
/// across it (+ to the frame's right), `y` world height.
#[derive(Clone, Copy, Debug)]
pub struct Flood {
    pub pos: [f64; 3],
    /// Unit aim (c, s, y).
    pub aim: [f64; 3],
    /// One of the 12 below the roadway (on the pier, lighting the legs up to
    /// the deck's underside), else one of the 12 at the sidewalk.
    pub below: bool,
}

/// Rated initial lumens of the 150 W and 250 W high-pressure sodium lamps
/// (ANSI C78.42 lamps S55 and S50, as the lamp makers' catalogues rate them).
pub const HPS_150W_LM: f64 = 16_000.0;
pub const HPS_250W_LM: f64 = 27_500.0;

/// The 12 lights each tower carries below the roadway: four of 150 W, four of
/// 250 W and four of 400 W, high-pressure sodium (goldengate.org, "Color &
/// Art Deco Styling"), so half that flux a leg.
pub fn gg_below_lumens_per_leg() -> f64 {
    4.0 * (HPS_150W_LM + HPS_250W_LM + HPS_400W_LM) / 2.0
}

/// The legs under the deck as the below-roadway lights light them
/// (`BridgeFeatures::floods`): from the pier's top (`base_y`) to the deck's
/// underside (`under_y`, the stiffening truss's bottom chord), the mean
/// illuminance `lux` over the same three faces a leg as above the deck, with
/// the same utilisation (0.6 of the lamps' flux in the beams, half of it on
/// the leg).
#[derive(Clone, Copy, Debug)]
pub struct FloodlitBelow {
    pub base_y: f64,
    pub under_y: f64,
    pub lux: f64,
}

pub fn gg_floodlit_below(js: &LandmarksJs) -> FloodlitBelow {
    let g = crate::golden_gate::data(js);
    let d = &g.dimensions;
    let water = js.sea_level_y;
    let deck_y = water + crate::data::num(d, "roadwayAtTowerM");
    let under_y = deck_y - crate::data::num(d, "trussDepthM");
    let base_y = water + g.leg_stages.iter().map(|s| s[0]).fold(f64::MAX, f64::min);
    let area: f64 = g.leg_stages.iter().map(|s| (2.0 * s[2] + s[3]) * ((water + s[1]).min(under_y) - (water + s[0]).max(base_y)).max(0.0)).sum();
    FloodlitBelow { base_y, under_y, lux: gg_below_lumens_per_leg() * 0.6 * 0.5 / area.max(1.0) }
}

/// A Golden Gate tower's floods as placed sources: the 12 sidewalk-level
/// lights pointed up each tower, six at each leg, and the 12 below the
/// roadway (goldengate.org, "Color & Art Deco Styling"). The sidewalk ones
/// stand on the sidewalk that wraps each leg's outside (the walkway round
/// the leg: golden_gate.rs `tower_walkways`). INFERRED: two on each of the
/// leg's three faces the sidewalk passes (its outer face and its two faces
/// along the span), at the sidewalk's outer edge less 0.3 m, at 0.45 of the
/// face's half width either side of its middle, each aimed up its own face
/// with 3 deg of tilt toward it; the below-roadway ones in the same layout on
/// the pier's top, 1 m off the leg's base. The beams' spread and the face
/// illuminance they lay are `FLOOD_*` below, `Floodlit::lux` and
/// `FloodlitBelow::lux`. The first 12 returned are the sidewalk's.
pub fn gg_tower_floods(js: &LandmarksJs) -> (Vec<Flood>, Floodlit) {
    let f = gg_floodlit(js);
    let g = crate::golden_gate::data(js);
    let d = &g.dimensions;
    let sidewalk = crate::data::num(d, "sidewalkM");
    let curb = crate::data::num(d, "curbM");
    let y = f.deck_y + curb;
    let (hp, ha, hx) = (f.half_plane, f.half_along, f.half_across);
    let out = (sidewalk - 0.3).max(0.5);
    let tilt = 3f64.to_radians();
    let up = |dc: f64, ds: f64| {
        let (sn, cs) = (tilt.sin(), tilt.cos());
        [dc * sn, ds * sn, cs]
    };
    let mut v = Vec::new();
    for side in [-1.0, 1.0] {
        let leg = side * hp;
        // the outer face (normal +side across): floods outboard of it, aimed up and in toward it.
        for k in [-0.45, 0.45] {
            v.push(Flood { pos: [k * ha, leg + side * (hx + out), y], aim: up(0.0, -side), below: false });
        }
        // the two faces along the span (normal +-c): floods beyond each, aimed back toward it.
        for along in [-1.0, 1.0] {
            for k in [-0.45, 0.45] {
                v.push(Flood { pos: [along * (ha + out), leg + k * hx, y], aim: up(-along, 0.0), below: false });
            }
        }
    }
    // Below the roadway: on the pier's top round the leg's base stage.
    let below = gg_floodlit_below(js);
    let base = g.leg_stages.iter().fold(g.leg_stages[0], |a, s| if s[0] < a[0] { *s } else { a });
    let (bx, ba) = (base[2] / 2.0, base[3] / 2.0);
    let (yb, off) = (below.base_y + 0.3, 1.0);
    for side in [-1.0, 1.0] {
        let leg = side * hp;
        for k in [-0.45, 0.45] {
            v.push(Flood { pos: [k * ba, leg + side * (bx + off), yb], aim: up(0.0, -side), below: true });
        }
        for along in [-1.0, 1.0] {
            for k in [-0.45, 0.45] {
                v.push(Flood { pos: [along * (ba + off), leg + k * bx, yb], aim: up(-along, 0.0), below: true });
            }
        }
    }
    (v, f)
}

/// The floods' beams (INFERRED: a wide-flood NEMA type 5-6 distribution,
/// full intensity within 20 deg of the aim, none past 32 deg).
pub const FLOOD_INNER_DEG: f64 = 20.0;
pub const FLOOD_OUTER_DEG: f64 = 32.0;
/// The face illuminance up the tower relative to its mean over the height
/// above the deck: 1.35 at the deck to 0.65 at the top (mean 1), the
/// floods stand at the deck, so the faces dim upward. INFERRED: the
/// floods' combined aim (not published), set to about half at the top.
pub const FLOOD_PROFILE: [f64; 2] = [1.35, -0.7];
