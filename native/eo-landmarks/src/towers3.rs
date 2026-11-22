//! The landmark buildings' third pass: towers2.rs's generators carried
//! further toward the real buildings (`eo-app --no-towers3` builds them as
//! towers2.rs does, the previous output), on the same site records,
//! dimensions and colours:
//!
//! | Building | What changes | Source |
//! | --- | --- | --- |
//! | All floodlit | projector floodlights in place of the lit band: each rig a set of ground- or roof-mounted projectors (position, aim, beam and field angles, axis intensity), drawn by the shader with N.L, the beam's cone and inverse-square fall-off (eo-render landmarks_surface.wgsl `lm_flood3`), so a curved face has a terminator and a roof the beams do not reach stays dark | IES Lighting Handbook, floodlighting: 50 lux (5 fc) on a light surface in dark surroundings, 150 lux in bright (the aim illuminances below); the beam and field angles of NEMA type 3-5 floods (beam 2 x 15-23 deg) INFERRED; positions INFERRED from the buildings' plans |
//! | Transamerica Pyramid | the base's inclined columns drawn with a triangular section (a prow outward), thinner, so the row of A and V frames reads; the precast a neutral white; the aluminium spire glowing from within (an interior rig: the light reaches the skin from inside) | Wikipedia, Transamerica Pyramid: 'the aluminum cap is indirectly illuminated from within'; a base 'web of diagonal trusses' four storeys tall (ASCE Civil Engineering, 2022, on the pyramid's renovation); the member section (0.75 m face, 0.85 m prow) INFERRED |
//! | Coit Tower | the shaft's 18 flutes cut crisp (a rib, two cheeks, a hollow) and deeper; a coping at the base's roof | the export's 18 flutes (landmarks-js.json coit-tower `flutes`); Wikipedia, Coit Tower: 'an unpainted reinforced concrete' fluted column. Flute depth 0.45 m INFERRED (the export's 0.035 of the radius, 0.2 m, read as a smooth cylinder from 450 m) |
//! | City Hall | the granite a light, cool grey; the dome's cladding a mid grey under its gilded ribs; floodlit facades, drum and dome | Wikipedia, San Francisco City Hall: granite from Madera County (the light grey 'Raymond' granite); albedo 0.5 INFERRED within light grey granite's 0.4-0.55 |
//! | Salesforce Tower | the fins and sunshades a painted (dielectric) pearl white; the spandrel between floors' vision glass opaque behind them (skyline-buildings.json salesforce-curtain v3 `glazeY`) | Pelli Clarke Pelli: 'clear glass with pearlescent metal accents'; Guardian Glass, Salesforce Tower project: 'neutral-looking' double glazing with high solar protection; vision glass to a 10 ft (3.05 m) ceiling of the 4.557 m storey, the rest spandrel (WBDG, curtain walls: spandrel panels hide the areas between vision glass) INFERRED |
//! | Sutro Tower | the legs' panels split at the paint bands' edges, so each band is one block of orange or white from chord to lacing; the masts' top posts thinner | the export's 11 alternating bands of international orange and white over the height (landmarks-js.json sutro-tower `bands`; FAA AC 70/7460-1M, aviation orange and white bands); post width 0.7 m INFERRED (the export's 1.8 m read as a thick red stick) |
//!
//! Every new group's material is the generator's own (`Out::materials`).
//!
//! The fourth pass (`crate::towers4_on`; `eo-app --no-towers4` builds the
//! third, the previous output) over it:
//!
//! | Building | What changes | Source |
//! | --- | --- | --- |
//! | Salesforce Tower | the crown as its parts: fins a module round it 0.61 m proud and a sunshade ring each storey (pearl painted aluminium), the perforated panels behind them to 0.82 of the crown (Day for Night's walkers through them at night, eo-render `lm_led4`), open lattice and a dark well over them | Wikipedia, Salesforce Tower (fins and sunshades up to two feet beyond the glass; the sculpture's 11,000 LEDs on the top six floors); KQED, 2018 (the LEDs light the building, seen through perforated aluminium panels); Pelli Clarke Pelli ('a transparent crown that appears to dissolve into the sky'); fin thickness and the panels' top INFERRED |
//! | Transamerica Pyramid | the spire coated aluminium (metalness 0.6, roughness 0.35) of aluminium's F0 weathered | Wikipedia ('the aluminum cap'); Real-Time Rendering 4th ed., table 9.2 (aluminium F0) |
//! | City Hall | the gilding gold leaf (metal of gold's F0, roughness 0.38); the drum's floods aimed at the colonnade's middle at 0.42 of the bright aim, so the drum's cornice no longer clips | Wikipedia, San Francisco City Hall (gilded dome trim); Real-Time Rendering 4th ed., table 9.2 (gold F0); the floods' aim INFERRED |

use crate::data::{LandmarksJs, Material};
use crate::geometry::{ALL, GroundGrid, MeshArrays, Rgb, V3, pairs, ring_centroid};
use crate::skyline::facade_material;
use crate::towers::{Out, base_y, flat, hex, n, v2};
use crate::towers2::{registry_material, with_origin};
use serde_json::Value;
use std::f64::consts::PI;

// ---- projector floodlights ----

/// The aim illuminance (lux) each projector gives on its face. IES
/// floodlighting asks 50 lux on a light surface in dark surroundings (Coit
/// Tower on its park hill) and 150 in bright (City Hall): at the game's
/// night exposure (eo-sky `night_exposure`) 50 lux on light stone (about 10
/// cd/m2) clips to display white, so both are drawn at the share of it
/// that keeps the brightest lit stone under 0.8 of display white (about 4
/// cd/m2 at the peak where beams overlap), the IES ratio between the two
/// kept as overlap (City Hall's beams overlap two to three deep): INFERRED.
pub const IES_DARK_LUX: f64 = 50.0;
pub const AIM_LUX_DARK: f64 = 5.5;
pub const AIM_LUX_BRIGHT: f64 = 10.0;
/// The luminance (cd/m2) of the Transamerica spire's skin lit from within, INFERRED (under the night exposure's clip).
pub const SPIRE_GLOW_CD: f64 = 2.4;

/// One projector of a rig.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projector {
    /// From the rig's origin (the landmark's anchor in plan, the rig's foot in y), m.
    pub at: V3,
    /// The beam axis (unit).
    pub aim: V3,
    /// Axis intensity, cd.
    pub candela: f64,
    /// Full intensity inside `inner_deg` of the axis, none past `outer_deg`.
    pub inner_deg: f64,
    pub outer_deg: f64,
    /// The share of the axis intensity spilt evenly over the forward hemisphere (the lens and housing).
    pub spill: f64,
    /// A light inside a translucent skin (it reaches the skin's inner face: no N.L sign, no specular).
    pub interior: bool,
}

fn sub3(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add3(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul3(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot3(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit3(a: V3) -> V3 {
    let l = dot3(a, a).sqrt().max(1e-12);
    mul3(a, 1.0 / l)
}

/// A projector at `at` aimed at `target`, its intensity giving `lux` on a
/// surface of normal `surface_n` at the target.
fn aimed(at: V3, target: V3, surface_n: V3, lux: f64, inner_deg: f64, outer_deg: f64, spill: f64) -> Projector {
    let d = sub3(target, at);
    let d2 = dot3(d, d);
    let aim = unit3(d);
    let cos_i = (-dot3(aim, unit3(surface_n))).max(0.15);
    Projector { at, aim, candela: lux * d2 / cos_i, inner_deg, outer_deg, spill, interior: false }
}

/// Coit Tower's rig, about its shaft's foot (the base building's roof):
/// six stations on the hilltop 28 m from the axis, 1 m over the ground (5 m
/// under the roof), each with two narrow floods, one aimed at the shaft 8 m
/// up and one 32 m up (the overlap of the two evens the inverse-square
/// fall-off from the foot to the gallery), their spill lighting the base. INFERRED.
fn coit_rig(js: &LandmarksJs) -> Vec<Projector> {
    let site = &js.buildings["coit-tower"];
    let c = v2(&site["centre"]);
    let anchor = anchor_of(js, "coit-tower");
    let r0 = n(&site["dimensions"], "shaftRadiusM");
    let below = n(&site["dimensions"], "lidarMedianM") - 1.0;
    let (ox, oz) = (c[0] - anchor[0], c[1] - anchor[1]);
    let mut out = Vec::new();
    for k in 0..6 {
        // the first toward the city (south-east), the rest every 60 deg.
        let a = (k as f64 / 6.0) * 2.0 * PI - PI / 4.0;
        let (dx, dz) = (a.cos(), a.sin());
        let at = [ox + dx * 28.0, -below, oz + dz * 28.0];
        for (h, spill) in [(8.0, 0.06), (32.0, 0.0)] {
            let target = [ox + dx * r0, h, oz + dz * r0];
            out.push(aimed(at, target, [dx, 0.0, dz], AIM_LUX_DARK, 12.0, 26.0, spill));
        }
    }
    out
}

/// City Hall's rig, about its base: a row of projectors 14 m out from each
/// facade on the ground, every 11 m, aimed square at the facade 16 m up (0.6
/// of the cornice); on the main roof eight 9 m outside the drum aimed at its
/// colonnade and eight 25 m out aimed over it at the dome's shell; four on the dome's
/// crown ring aimed at the lantern (INFERRED).
fn city_hall_rig(js: &LandmarksJs) -> Vec<Projector> {
    let site = &js.buildings["city-hall"];
    let d = &site["dimensions"];
    let fr = &site["frame"];
    let anchor = anchor_of(js, "city-hall");
    let (cx, cz, u, v, hu, hv) = (n(fr, "cx") - anchor[0], n(fr, "cz") - anchor[1], v2(&fr["u"]), v2(&fr["v"]), n(fr, "hu"), n(fr, "hv"));
    let at = |a: f64, b: f64, y: f64| -> V3 { [cx + u[0] * a + v[0] * b, y, cz + u[1] * a + v[1] * b] };
    let aim_y = 0.6 * n(d, "corniceM");
    let mut out = Vec::new();
    for (na, nb, half, off) in [(1.0, 0.0, hv, hu), (-1.0, 0.0, hv, hu), (0.0, 1.0, hu, hv), (0.0, -1.0, hu, hv)] {
        let count = ((2.0 * half) / 11.0).floor() as i64;
        for k in 0..count {
            let s = -half + (2.0 * half) * (k as f64 + 0.5) / count as f64;
            let (p, t, nrm) = if na != 0.0 {
                (at(na * (off + 14.0), s, 0.5), at(na * off, s, aim_y), [u[0] * na, 0.0, u[1] * na])
            } else {
                (at(s, nb * (off + 14.0), 0.5), at(s, nb * off, aim_y), [v[0] * nb, 0.0, v[1] * nb])
            };
            out.push(aimed(p, t, nrm, AIM_LUX_BRIGHT, 16.0, 34.0, 0.04));
        }
    }
    let (roof, drum_r, drum_top, ent, rise, lantern_r, lantern_top) =
        (n(d, "corniceM"), n(d, "drumRadiusM"), n(d, "drumTopM"), n(d, "entablatureM"), n(d, "domeRiseM"), n(d, "lanternRadiusM"), n(d, "lanternTopM"));
    let c = at(0.0, 0.0, 0.0);
    for k in 0..8 {
        let a = (k as f64 + 0.5) / 8.0 * 2.0 * PI;
        let (dx, dz) = (a.cos(), a.sin());
        let p = [c[0] + dx * (drum_r + 9.0), roof + 1.0, c[2] + dz * (drum_r + 9.0)];
        let t = [c[0] + dx * drum_r, drum_top - 4.0, c[2] + dz * drum_r];
        out.push(aimed(p, t, [dx, 0.0, dz], AIM_LUX_BRIGHT * 0.6, 18.0, 36.0, 0.03));
    }
    // eight more from the same stations aimed at the dome's shell, 0.45 up
    // its profile (the export's dome: radius rd cos(t pi/2)^0.85, rise sin(t pi/2)).
    let rd = n(d, "domeDiameterM") / 2.0;
    let t = 0.45 * PI / 2.0;
    let (dr, dy) = (rd * t.cos().powf(0.85), drum_top + ent + 1.2 + rise * t.sin());
    for k in 0..8 {
        let a = (k as f64 + 0.5) / 8.0 * 2.0 * PI;
        let (dx, dz) = (a.cos(), a.sin());
        let p = [c[0] + dx * (drum_r + 25.0), roof + 1.0, c[2] + dz * (drum_r + 25.0)];
        let tgt = [c[0] + dx * dr, dy, c[2] + dz * dr];
        out.push(aimed(p, tgt, [dx * t.sin(), t.cos(), dz * t.sin()], AIM_LUX_BRIGHT * 0.9, 10.0, 22.0, 0.0));
    }
    let crown = drum_top + ent + rise;
    for k in 0..4 {
        let a = (k as f64 + 0.25) / 4.0 * 2.0 * PI;
        let (dx, dz) = (a.cos(), a.sin());
        let p = [c[0] + dx * (lantern_r + 4.0), crown, c[2] + dz * (lantern_r + 4.0)];
        let t = [c[0] + dx * lantern_r, (crown + lantern_top) / 2.0, c[2] + dz * lantern_r];
        out.push(aimed(p, t, [dx, 0.0, dz], AIM_LUX_BRIGHT * 0.4, 20.0, 40.0, 0.03));
    }
    out
}

/// The Transamerica spire's interior rig, about the spire's foot: three
/// lights on its axis at 0.15, 0.45 and 0.75 of its height, each giving the
/// skin `SPIRE_GLOW_CD` (L = E rho / pi) where its own height crosses it.
fn transamerica_rig(js: &LandmarksJs) -> Vec<Projector> {
    let site = &js.buildings["transamerica-pyramid"];
    let d = &site["dimensions"];
    let fr = &site["frame"];
    let anchor = anchor_of(js, "transamerica-pyramid");
    let (cx, cz, fhalf) = (n(fr, "cx") - anchor[0], n(fr, "cz") - anchor[1], n(fr, "half"));
    let (h_total, spire) = (n(d, "heightM"), n(d, "spireM"));
    let half_at = |y_from_spire: f64| fhalf * (1.0 - (h_total - spire + y_from_spire) / h_total);
    let rho = crate::geometry::srgb_hex(n(d, "metalSrgb") as u32)[1];
    [0.15, 0.45, 0.75]
        .iter()
        .map(|f| {
            let y = spire * f;
            let r = half_at(y).max(0.5);
            let lux = SPIRE_GLOW_CD * PI / rho.max(0.05);
            Projector { at: [cx, y, cz], aim: [0.0, 1.0, 0.0], candela: lux * r * r / 3.0, inner_deg: 80.0, outer_deg: 90.0, spill: 1.0, interior: true }
        })
        .collect()
}

fn anchor_of(js: &LandmarksJs, id: &str) -> [f64; 2] {
    let a = &js.registry[id].anchor;
    [a["x"].as_f64().unwrap_or(0.0), a["z"].as_f64().unwrap_or(0.0)]
}

/// Every rig, in the WGSL table's order (City Hall's drum floods as the
/// switches set the pass: `crate::towers4_on`).
pub fn rigs() -> Vec<(&'static str, Vec<Projector>)> {
    let js = LandmarksJs::get();
    let mut ch = city_hall_rig(js);
    if crate::towers4_on() {
        drum4(js, &mut ch);
    }
    vec![("coit-tower", coit_rig(js)), ("city-hall", ch), ("transamerica-spire", transamerica_rig(js))]
}

/// v4: City Hall's eight drum floods aimed at the colonnade's middle (the
/// drum's top less `DRUM4_AIM_DOWN_M`) at `DRUM4_LUX_SHARE` of the bright aim
/// in place of 0.6 aimed 4 m under the drum's top: there, with the dome
/// shell's eight floods' fringe, the cornice band took two beams' cores and
/// clipped (INFERRED: the colonnade lit, the cornice in the beams' fall-off).
pub const DRUM4_AIM_DOWN_M: f64 = 11.0;
pub const DRUM4_LUX_SHARE: f64 = 0.42;

fn drum4(js: &LandmarksJs, rig: &mut [Projector]) {
    let site = &js.buildings["city-hall"];
    let d = &site["dimensions"];
    let fr = &site["frame"];
    let anchor = anchor_of(js, "city-hall");
    let (cx, cz) = (n(fr, "cx") - anchor[0], n(fr, "cz") - anchor[1]);
    let (drum_r, drum_top) = (n(d, "drumRadiusM"), n(d, "drumTopM"));
    // the drum's floods follow the facades' (the facade rows are 11 m apart along each side).
    let first = rig.iter().position(|p| (p.at[0] - cx).hypot(p.at[2] - cz) > drum_r + 8.5 && (p.at[0] - cx).hypot(p.at[2] - cz) < drum_r + 9.5);
    let Some(first) = first else { return };
    for p in rig.iter_mut().skip(first).take(8) {
        let (dx, dz) = ((p.at[0] - cx) / (drum_r + 9.0), (p.at[2] - cz) / (drum_r + 9.0));
        let target = [cx + dx * drum_r, drum_top - DRUM4_AIM_DOWN_M, cz + dz * drum_r];
        *p = aimed(p.at, target, [dx, 0.0, dz], AIM_LUX_BRIGHT * DRUM4_LUX_SHARE, 18.0, 36.0, 0.03);
    }
}

/// The projectors as three rows each of the WGSL table (eo-render
/// `LM_PROJ`): (offset xyz, cd), (aim xyz, cos field), (cos beam, spill,
/// interior 1/0, 0); and each rig's (first, count).
pub fn projector_rows() -> (Vec<[f32; 12]>, Vec<(&'static str, u32, u32)>) {
    let mut rows = Vec::new();
    let mut ranges = Vec::new();
    for (id, rig) in rigs() {
        ranges.push((id, rows.len() as u32, rig.len() as u32));
        for p in rig {
            let (ci, co) = (p.inner_deg.to_radians().cos(), p.outer_deg.to_radians().cos());
            rows.push([
                p.at[0] as f32,
                p.at[1] as f32,
                p.at[2] as f32,
                p.candela as f32,
                p.aim[0] as f32,
                p.aim[1] as f32,
                p.aim[2] as f32,
                co as f32,
                ci as f32,
                p.spill as f32,
                if p.interior { 1.0 } else { 0.0 },
                0.0,
            ]);
        }
    }
    (rows, ranges)
}

/// A material lit by a rig: `rig` = [first, origin y, count] (eo-render
/// packs it into `base` with LM_F_FLOOD).
pub fn with_rig(m: &Material, rig: &str, origin_y: f64) -> Material {
    let (_, ranges) = projector_rows();
    let (first, count) = ranges.iter().find(|r| r.0 == rig).map(|r| (r.1, r.2)).unwrap_or((0, 0));
    let mut m = m.clone();
    let mut o = m.surface_options.clone().unwrap_or_else(|| serde_json::json!({}));
    o["rig"] = serde_json::json!([first, origin_y, count]);
    m.surface_options = Some(o);
    m
}

// ---- the buildings ----

/// A member of triangular section from p0 to p1: its back face `w` wide
/// toward the building, its prow `depth` out along `out` (horizontal unit).
fn prow_member(m: &mut MeshArrays, p0: V3, p1: V3, out: [f64; 2], w: f64, depth: f64, c: Rgb) {
    let o = [out[0], 0.0, out[1]];
    let dir = unit3(sub3(p1, p0));
    let side = unit3(cross3(dir, o));
    let sec = |p: V3| [add3(sub3(p, mul3(side, w / 2.0)), mul3(o, -depth / 3.0)), add3(add3(p, mul3(side, w / 2.0)), mul3(o, -depth / 3.0)), add3(p, mul3(o, 2.0 * depth / 3.0))];
    let (a, b) = (sec(p0), sec(p1));
    let mid = mul3(add3(p0, p1), 0.5);
    for i in 0..3 {
        let j = (i + 1) % 3;
        let (q0, q1, q2, q3) = (a[i], a[j], b[j], b[i]);
        let nrm = cross3(sub3(q1, q0), sub3(q3, q0));
        let qc = mul3(add3(add3(q0, q1), add3(q2, q3)), 0.25);
        if dot3(nrm, sub3(qc, mid)) >= 0.0 {
            m.quad(q0, q1, q2, q3, c);
        } else {
            m.quad(q3, q2, q1, q0, c);
        }
    }
    for (s, sgn) in [(a, -1.0), (b, 1.0)] {
        let nrm = cross3(sub3(s[1], s[0]), sub3(s[2], s[0]));
        if dot3(nrm, mul3(dir, sgn)) >= 0.0 {
            m.tri(s[0], s[1], s[2], c);
        } else {
            m.tri(s[2], s[1], s[0], c);
        }
    }
}

/// The base truss: each side's column feet (the export's nine a side)
/// lean in pairs to the ring beam, a row of A and V frames, each column of
/// triangular section (`TRUSS_FACE_M` back, `TRUSS_PROW_M` prow).
pub const TRUSS_FACE_M: f64 = 0.75;
pub const TRUSS_PROW_M: f64 = 0.85;

/// The precast's white quartz-aggregate tone, nearer neutral than the
/// export's cream 0xd9d5cb (Wikipedia: 'white precast quartz aggregate';
/// a white-aggregate precast panel reflects about 0.6, INFERRED).
pub const TRANSAMERICA_PRECAST_SRGB: u32 = 0xcfcdc8;

pub fn transamerica(site: &Value, g: &GroundGrid) -> Out {
    transamerica_opened(site, g, false)
}

/// `transamerica`; `open` (landmark access): towers2.rs
/// `transamerica_opened`'s top level left to the observation room.
pub fn transamerica_opened(site: &Value, g: &GroundGrid, open: bool) -> Out {
    let mut site = site.clone();
    site["dimensions"]["precastSrgb"] = serde_json::json!(TRANSAMERICA_PRECAST_SRGB);
    let site = &site;
    let mut o = crate::towers2::transamerica_opened(site, g, open);
    let d = &site["dimensions"];
    let fr = &site["frame"];
    let (cx, cz, u, v, fhalf) = (n(fr, "cx"), n(fr, "cz"), v2(&fr["u"]), v2(&fr["v"]), n(fr, "half"));
    let precast_c = hex(d, "precastSrgb");
    let ring = pairs(&flat(&site["record"]["points"]));
    let base = base_y(&ring, g);
    let h_total = n(d, "heightM");
    let spire_y = h_total - n(d, "spireM");
    let top0 = n(d, "baseStoreysM");
    let half = |y: f64| (fhalf * (1.0 - y / h_total)).max(0.0);
    let at = |a: f64, b: f64, y: f64| -> V3 { [cx + u[0] * a + v[0] * b, base + y, cz + u[1] * a + v[1] * b] };
    // towers2's precast less its base columns (the members up to the ring
    // beam: every triangle whose vertices all lie under the ring beam's foot
    // and outside the glazed core), then the prow columns.
    let truss_top = top0 - 2.2;
    let core = half(0.0) - 4.5;
    if let Some((_, pre)) = o.groups.iter_mut().find(|(k, _)| k == "precast") {
        let mut m = MeshArrays::new("transamerica:precast");
        for t in pre.indices.chunks_exact(3) {
            let ps: Vec<V3> = t.iter().map(|&i| {
                let i = i as usize * 3;
                [pre.positions[i] as f64, pre.positions[i + 1] as f64, pre.positions[i + 2] as f64]
            }).collect();
            let column = ps.iter().all(|p| {
                let (dx, dz) = (p[0] - cx, p[2] - cz);
                let (a, b) = (dx * u[0] + dz * u[1], dx * v[0] + dz * v[1]);
                p[1] - base < truss_top - 0.01 && a.abs().max(b.abs()) > core + 0.5
            });
            if column {
                continue;
            }
            let i0 = t[0] as usize * 3;
            let c = [pre.colors[i0] as f64, pre.colors[i0 + 1] as f64, pre.colors[i0 + 2] as f64];
            m.tri(ps[0], ps[1], ps[2], c);
        }
        for side in 0..4 {
            let edge = |t: f64, y: f64, inset: f64| {
                let h = half(y) - inset;
                let s = t * h;
                match side {
                    0 => at(s, -h, y),
                    1 => at(h, s, y),
                    2 => at(-s, h, y),
                    _ => at(-h, -s, y),
                }
            };
            let out = match side {
                0 => [-v[0], -v[1]],
                1 => [u[0], u[1]],
                2 => [v[0], v[1]],
                _ => [-u[0], -u[1]],
            };
            let pitch = 1.0 / 4.6;
            for k in -4i32..=4 {
                let t = k as f64 * pitch;
                let foot = edge(t, 0.0, 0.6);
                for dk in [-0.5, 0.5] {
                    prow_member(&mut m, foot, edge(t + dk * pitch, truss_top, 0.6), out, TRUSS_FACE_M, TRUSS_PROW_M, precast_c);
                }
            }
            // the corner column, its prow on the diagonal.
            let diag = [(out[0] + match side { 0 => u[0], 1 => v[0], 2 => -u[0], _ => -v[0] }) * std::f64::consts::FRAC_1_SQRT_2, (out[1] + match side { 0 => u[1], 1 => v[1], 2 => -u[1], _ => -v[1] }) * std::f64::consts::FRAC_1_SQRT_2];
            prow_member(&mut m, edge(1.0, 0.0, 0.6), edge(1.0, truss_top, 0.6), diag, 1.3, 1.2, precast_c);
            m.beam(edge(-1.0, 0.35, 0.3), edge(1.0, 0.35, 0.3), 0.7, 1.4, precast_c);
        }
        *pre = m.freeze();
    }
    // the spire glows from within (its interior rig) in place of the lit band.
    if let Some(m) = registry_material("transamerica-pyramid", "metal") {
        let spire = with_rig(&facade_material("solid", m.roughness, m.metalness, None), "transamerica-spire", base + spire_y);
        o.materials.retain(|(k, _)| k != "metal");
        o.materials.push(("metal".into(), spire));
    }
    o.version = o.version.replace("+towers2", "+towers3");
    o
}

/// The fins' and sunshades' finish: 'pearlescent metal accents' (Pelli
/// Clarke Pelli) are a painted aluminium, a dielectric paint film over the
/// metal (metalness 0; a satin finish, roughness 0.35, INFERRED), in place
/// of towers2's partly metallic 0.42 / 0.25, which dimmed them to grey.
pub const SALESFORCE_FIN_ROUGHNESS: f64 = 0.35;

pub fn salesforce(site: &Value, g: &GroundGrid) -> Out {
    salesforce_opened(site, g, false)
}

/// `salesforce`; `open` (landmark access): towers2.rs `salesforce_opened`'s
/// top-floor vision glass left to the observation level.
pub fn salesforce_opened(site: &Value, g: &GroundGrid, open: bool) -> Out {
    let mut o = crate::towers2::salesforce_opened(site, g, open);
    let origin = o.stats["baseY"].as_f64().unwrap_or(0.0);
    for key in ["glass", "far"] {
        if let Some((_, m)) = o.materials.iter_mut().find(|(k, _)| k == key) {
            *m = with_origin(facade_material("salesforce-curtain", SALESFORCE_FIN_ROUGHNESS, 0.0, None), origin);
        }
    }
    o.version = o.version.replace("+towers2", "+towers3");
    o
}

/// Coit Tower's flute: a rib at the shaft's radius over `COIT_RIB_SHARE` of
/// the pitch, two cheeks, a hollow `COIT_FLUTE_DEPTH_M` deep (INFERRED).
pub const COIT_FLUTE_DEPTH_M: f64 = 0.45;
pub const COIT_RIB_SHARE: f64 = 0.34;

pub fn coit(site: &Value, g: &GroundGrid) -> Out {
    coit_opened(site, g, false)
}

/// `coit`; `open` (landmark access, access.rs) leaves out the belvedere's
/// solid band and its painted openings, which the gallery draws as a real
/// arcade, and the far item's band there (as towers.rs `coit_opened`).
pub fn coit_opened(site: &Value, g: &GroundGrid, open: bool) -> Out {
    let d = &site["dimensions"];
    let concrete_c = hex(d, "concreteSrgb");
    let shade = concrete_c.map(|c| c * 0.82);
    let opening = hex(d, "openingSrgb");
    let ring = pairs(&flat(&site["record"]["points"]));
    let [cx, cz] = v2(&site["centre"]);
    let base_ground = base_y(&ring, g);
    let top_ground = ring.iter().map(|p| g.at(p[0], p[1])).fold(f64::NEG_INFINITY, f64::max);
    let mut concrete = MeshArrays::new("coit:concrete");
    let mut openings = MeshArrays::new("coit:openings");
    let mut far = MeshArrays::new("coit:far");
    let y0 = top_ground;
    let base = y0 + n(d, "lidarMedianM");
    let r0 = n(d, "shaftRadiusM");
    let r1 = r0 - n(d, "taperDiameterM") / 2.0;
    let bel0 = y0 + n(d, "belvedereFromM");
    let bel1 = y0 + n(d, "belvedereToM");
    let top = y0 + n(d, "lidarPeakM");
    let (flutes_n, cornice_m, lr) = (n(d, "flutes").max(1.0), n(d, "corniceM"), n(d, "lanternRadiusM"));
    // the base building, and a coping round its roof.
    concrete.prism(&ring, base_ground - 0.5, base - 0.6, concrete_c, false, false);
    let rc = ring_centroid(&ring);
    let coping: Vec<[f64; 2]> = ring.iter().map(|p| [rc[0] + (p[0] - rc[0]) * 1.03, rc[1] + (p[1] - rc[1]) * 1.03]).collect();
    concrete.prism(&coping, base - 0.6, base, concrete_c, true, true);
    far.prism(&ring, base_ground - 0.5, base, concrete_c, true, false);
    // the shaft's flutes: each pitch's six corners, a rib at the shaft's
    // radius (three corners over `COIT_RIB_SHARE`), a cheek down, the
    // hollow (three corners), a cheek up into the next rib.
    let corners = [0.0, COIT_RIB_SHARE / 2.0, COIT_RIB_SHARE, COIT_RIB_SHARE + 0.12, 0.5 + COIT_RIB_SHARE / 2.0, 0.88];
    let sunk = [0.0, 0.0, 0.0, COIT_FLUTE_DEPTH_M, COIT_FLUTE_DEPTH_M, COIT_FLUTE_DEPTH_M];
    let per = corners.len();
    let segments = flutes_n as usize * per;
    let corner = |k: usize| -> (f64, f64) {
        let (pitch, j) = (k / per, k % per);
        ((pitch as f64 + corners[j]) / flutes_n * 2.0 * PI, sunk[j])
    };
    let mut rows = Vec::new();
    for k in 0..=8 {
        let y = base + ((bel0 - base) * k as f64) / 8.0;
        rows.push([r0 + ((r1 - r0) * (y - base)) / (bel1 - base), y]);
    }
    for p in 0..rows.len() - 1 {
        let [ra, ya] = rows[p];
        let [rb, yb] = rows[p + 1];
        for k in 0..segments {
            let (t0, s0) = corner(k);
            let (t1, s1) = corner((k + 1) % segments);
            let t1 = if k + 1 == segments { 2.0 * PI } else { t1 };
            let pt = |t: f64, r: f64, y: f64| [cx + t.cos() * r, y, cz - t.sin() * r];
            concrete.quad(pt(t0, ra - s0, ya), pt(t1, ra - s1, ya), pt(t1, rb - s1, yb), pt(t0, rb - s0, yb), concrete_c);
        }
    }
    if !open {
        concrete.lathe(cx, cz, &[[r1, bel0], [r1, bel1]], 48, shade, None, None);
    }
    let cornice_r = r1 + 0.7;
    concrete.lathe(cx, cz, &[[r1, bel1], [cornice_r, bel1], [cornice_r, bel1 + cornice_m], [lr + 0.6, bel1 + cornice_m], [0.0, bel1 + cornice_m + 0.01]], 48, concrete_c, None, None);
    let arches = if open { 0.0 } else { n(d, "arches") };
    for k in 0..arches as i64 {
        let a0 = ((k as f64 + 0.18) / arches) * PI * 2.0;
        let a1 = ((k as f64 + 0.82) / arches) * PI * 2.0;
        let am = (a0 + a1) / 2.0;
        let r = r1 + 0.05;
        let p = |a: f64, y: f64| [cx + a.cos() * r, y, cz - a.sin() * r];
        let sill = bel0 + 0.9;
        let spring = bel1 - 1.8;
        openings.quad(p(a0, sill), p(a1, sill), p(a1, spring), p(a0, spring), opening);
        openings.tri(p(a0, spring), p(a1, spring), p(am, bel1 - 0.6), opening);
    }
    let l0 = bel1 + cornice_m;
    concrete.lathe(cx, cz, &[[lr, l0], [lr, top - 0.8], [lr + 0.3, top - 0.8], [lr + 0.3, top - 0.3], [0.0, top]], 32, concrete_c, None, None);
    for k in 0..8 {
        let a0 = ((k as f64 + 0.25) / 8.0) * PI * 2.0;
        let a1 = ((k as f64 + 0.75) / 8.0) * PI * 2.0;
        let r = lr + 0.05;
        let p = |a: f64, y: f64| [cx + a.cos() * r, y, cz - a.sin() * r];
        openings.quad(p(a0, l0 + 0.6), p(a1, l0 + 0.6), p(a1, top - 1.2), p(a0, top - 1.2), opening);
    }
    if open {
        // the far item leaves the belvedere's band to the gallery's arcade (drawn at every distance).
        let r_bel0 = r0 + ((r1 - r0) * (bel0 - base)) / (bel1 - base);
        far.lathe(cx, cz, &[[r0, base], [r_bel0, bel0]], 16, concrete_c, None, None);
        far.lathe(cx, cz, &[[r1, bel1], [cornice_r, bel1], [cornice_r, bel1 + cornice_m], [lr, bel1 + cornice_m], [lr, top], [0.0, top]], 16, concrete_c, None, None);
    } else {
        far.lathe(cx, cz, &[[r0, base], [r1, bel1], [cornice_r, bel1], [cornice_r, bel1 + cornice_m], [lr, bel1 + cornice_m], [lr, top], [0.0, top]], 16, concrete_c, None, None);
    }
    let mut materials = vec![];
    if let Some(m) = registry_material("coit-tower", "concrete") {
        materials.push(("concrete".into(), with_rig(&m, "coit-tower", base)));
    }
    if let Some(m) = registry_material("coit-tower", "openings") {
        materials.push(("openings".into(), with_rig(&m, "coit-tower", base)));
    }
    Out {
        version: format!("{}+towers3", site["version"].as_str().unwrap_or("")),
        groups: vec![("concrete".into(), concrete.freeze()), ("openings".into(), openings.freeze())],
        far: far.freeze(),
        colliders: vec![],
        materials,
        lights: vec![],
        stats: serde_json::json!({ "baseY": base_ground, "topY": top, "shaftRadiusM": r0, "shaftFootY": base, "fluteDepthM": COIT_FLUTE_DEPTH_M }),
    }
}

/// City Hall's granite (Madera County 'Raymond' granite, a light cool grey):
/// sRGB #babdc0 (linear about 0.5) and its recesses' #9fa2a5, INFERRED.
pub const CITY_HALL_STONE_SRGB: u32 = 0xbabdc0;
pub const CITY_HALL_STONE_SHADE_SRGB: u32 = 0x9fa2a5;

/// The dome's cladding: a mid grey (sRGB #7e858c, linear about 0.23) in
/// place of the export's near-black 0x59626b, so lit it reads as a grey
/// dome with its gilded ribs, as public photographs describe it. INFERRED.
pub const CITY_HALL_DOME_SRGB: u32 = 0x7e858c;

pub fn city_hall(site: &Value, g: &GroundGrid) -> Out {
    let mut s = site.clone();
    s["dimensions"]["stoneSrgb"] = serde_json::json!(CITY_HALL_STONE_SRGB);
    s["dimensions"]["stoneShadeSrgb"] = serde_json::json!(CITY_HALL_STONE_SHADE_SRGB);
    s["dimensions"]["domeSrgb"] = serde_json::json!(CITY_HALL_DOME_SRGB);
    let mut o = crate::towers::city_hall(&s, g);
    let base = o.stats["baseY"].as_f64().unwrap_or(0.0);
    o.version = format!("{}+towers3", o.version);
    for key in ["stone", "dome"] {
        if let Some(m) = registry_material("city-hall", key) {
            o.materials.push((key.into(), with_rig(&m, "city-hall", base)));
        }
    }
    o.materials.push(("windows".into(), with_origin(facade_material("city-hall-window", 0.3, 0.1, None), base)));
    o
}

/// Sutro Tower's mast-top posts, half width (m, INFERRED).
pub const SUTRO_POST_HALF_M: f64 = 0.35;

pub fn sutro(site: &Value, g: &GroundGrid) -> Out {
    use crate::towers2::{SUTRO_CROSSBAR_DEPTH_M, SUTRO_LEVELS_FT, SUTRO_TOP_DEPTH_M, SUTRO_TOP_REACH_M};
    const FT: f64 = 0.3048;
    let d = &site["dimensions"];
    let c = [n(&site["centre"], "x"), n(&site["centre"], "z")];
    let dirs: Vec<[f64; 2]> = site["legDirections"].as_array().map(|a| a.iter().map(v2).collect()).unwrap_or_default();
    let sqrt3 = 3f64.sqrt();
    let circum = |side: f64| side / sqrt3;
    let (orange, white, concrete_c) = (hex(d, "orangeSrgb"), hex(d, "whiteSrgb"), hex(d, "concreteSrgb"));
    let (height, bands, waist, platform) = (n(d, "heightM"), n(d, "bands"), n(d, "waistM"), n(d, "platformM"));
    let (rb, rw, rt) = (circum(n(d, "baseSideM")), circum(n(d, "waistSideM")), circum(n(d, "topSideM")));
    let leg_radius_at = |h: f64| {
        if h <= waist {
            rb + (rw - rb) * h.max(0.0) / waist
        } else if h <= platform {
            rw + (rt - rw) * (h - waist) / (platform - waist)
        } else {
            rt
        }
    };
    let band_colour = |h: f64| {
        let b = ((h / height) * bands).floor().max(0.0).min(bands - 1.0) as i64;
        if b % 2 == 0 { orange } else { white }
    };
    let feet: Vec<([f64; 2], f64, f64)> = dirs
        .iter()
        .map(|[ux, uz]| {
            let (x, z) = (c[0] + ux * rb, c[1] + uz * rb);
            let gr = g.at(x, z);
            ([x, z], if gr.is_nan() { 0.0 } else { gr }, uz.atan2(*ux))
        })
        .collect();
    let base_y = feet.iter().fold(0.0, |s, f| s + f.1) / feet.len() as f64;
    let leg_point = |leg: usize, h: f64| -> V3 {
        let [ux, uz] = dirs[leg];
        let r = leg_radius_at(h);
        [c[0] + ux * r, base_y + h, c[1] + uz * r]
    };
    // the lattice's panels split at the bands' edges, so each band is one
    // colour from chord to lacing (clean blocks, not panels straddling two).
    let band_h = height / bands;
    let lattice = |mesh: &mut MeshArrays, leg: usize, h0: f64, h1: f64, width: f64, panel: f64| {
        let [ux, uz] = dirs[leg];
        let r = width / sqrt3;
        let (tx, tz) = (-uz, ux);
        let offsets = [
            [ux * r, uz * r],
            [(-ux * 0.5 + tx * sqrt3 / 2.0) * r, (-uz * 0.5 + tz * sqrt3 / 2.0) * r],
            [(-ux * 0.5 - tx * sqrt3 / 2.0) * r, (-uz * 0.5 - tz * sqrt3 / 2.0) * r],
        ];
        let at = |h: f64, k: usize| {
            let p = leg_point(leg, h);
            [p[0] + offsets[k][0], p[1], p[2] + offsets[k][1]]
        };
        let mut cuts = vec![h0];
        let mut e = ((h0 / band_h).floor() + 1.0) * band_h;
        while e < h1 - 0.5 {
            cuts.push(e);
            e += band_h;
        }
        cuts.push(h1);
        let mut s = 0usize;
        for w in cuts.windows(2) {
            let steps = ((w[1] - w[0]) / panel).round().max(1.0) as i64;
            for q in 0..steps {
                let ha = w[0] + ((w[1] - w[0]) * q as f64) / steps as f64;
                let hb = w[0] + ((w[1] - w[0]) * (q + 1) as f64) / steps as f64;
                let colour = band_colour((ha + hb) / 2.0);
                for k in 0..3 {
                    mesh.beam(at(ha, k), at(hb, k), 0.55, 0.55, colour);
                    mesh.beam(at(hb, k), at(hb, (k + 1) % 3), 0.28, 0.28, colour);
                    let (a, b) = if s % 2 == 0 { (k, (k + 1) % 3) } else { ((k + 1) % 3, k) };
                    mesh.beam(at(ha, a), at(hb, b), 0.22, 0.22, colour);
                }
                s += 1;
            }
        }
    };
    let crossbar = |mesh: &mut MeshArrays, i: usize, j: usize, h: f64, depth: f64, reach: f64, chord: f64| {
        let colour = band_colour(h);
        let (a, b) = (leg_point(i, h), leg_point(j, h));
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let len = dx.hypot(dz);
        let (ex, ez) = (dx / len, dz / len);
        let p = |s: f64, y: f64| -> V3 { [a[0] + ex * s, base_y + y, a[2] + ez * s] };
        let (s0, s1) = (-reach, len + reach);
        let (yb, yt) = (h - depth / 2.0, h + depth / 2.0);
        mesh.beam(p(s0, yb), p(s1, yb), chord, chord, colour);
        mesh.beam(p(s0, yt), p(s1, yt), chord, chord, colour);
        let panels = (((s1 - s0) / depth).round() as i64).max(2);
        for k in 0..=panels {
            let s = s0 + (s1 - s0) * k as f64 / panels as f64;
            mesh.beam(p(s, yb), p(s, yt), chord * 0.6, chord * 0.6, colour);
            if k < panels {
                let s2 = s0 + (s1 - s0) * (k + 1) as f64 / panels as f64;
                let (ya, yz) = if k % 2 == 0 { (yb, yt) } else { (yt, yb) };
                mesh.beam(p(s, ya), p(s2, yz), chord * 0.5, chord * 0.5, colour);
            }
        }
    };
    let mut steel = MeshArrays::new("sutro-tower:steel");
    let mut concrete = MeshArrays::new("sutro-tower:concrete");
    let mut far = MeshArrays::new("sutro-tower:far");
    let (foot_side, foot_top, foot_bury) = (n(d, "footingSideM"), n(d, "footingTopM"), n(d, "footingBuryM"));
    for (p, gr, yaw) in &feet {
        let half = foot_side / 2.0;
        concrete.box_(p[0], p[1], gr - foot_bury, gr + foot_top, half, half, *yaw, concrete_c, ALL);
        far.box_(p[0], p[1], gr - foot_bury, gr + foot_top, half, half, *yaw, concrete_c, ALL);
    }
    let (leg_w, mast_w, panel) = (n(d, "legWidthM"), n(d, "mastWidthM"), n(d, "panelM"));
    let mut lights = Vec::new();
    for leg in 0..3 {
        let start = feet[leg].1 + foot_top - base_y;
        lattice(&mut steel, leg, start, platform, leg_w, panel);
        lattice(&mut steel, leg, platform, height - 4.0, mast_w, panel * 0.75);
        let top = leg_point(leg, height - 22.0);
        steel.box_(top[0], top[2], top[1], base_y + height, SUTRO_POST_HALF_M, SUTRO_POST_HALF_M, feet[leg].2, band_colour(height - 1.0), ALL);
        let keys = [start, waist, platform, height];
        for s in 0..3 {
            let w = if s < 2 { leg_w * 1.1 } else { mast_w * 1.2 };
            let steps = ((keys[s + 1] - keys[s]) / (height / bands / 2.0)).round().max(1.0) as i64;
            for k in 0..steps {
                let ha = keys[s] + ((keys[s + 1] - keys[s]) * k as f64) / steps as f64;
                let hb = keys[s] + ((keys[s + 1] - keys[s]) * (k + 1) as f64) / steps as f64;
                far.beam(leg_point(leg, ha), leg_point(leg, hb), w, w, band_colour((ha + hb) / 2.0));
            }
        }
        let tip = leg_point(leg, height);
        lights.push(crate::lights::aviation([tip[0], tip[2]], tip[1]));
        let mid = leg_point(leg, SUTRO_LEVELS_FT[2] * FT);
        lights.push(crate::lights::aviation([mid[0], mid[2]], mid[1]));
    }
    let mut levels: Vec<f64> = SUTRO_LEVELS_FT.iter().map(|f| f * FT).collect();
    if let Some(top_frame) = flat(&d["ringFramesM"]).last() {
        levels.push(*top_frame);
    }
    for &h in &levels {
        for k in 0..3 {
            crossbar(&mut steel, k, (k + 1) % 3, h, SUTRO_CROSSBAR_DEPTH_M, 0.0, 0.7);
            far.beam(leg_point(k, h), leg_point((k + 1) % 3, h), 1.2, SUTRO_CROSSBAR_DEPTH_M * 0.6, band_colour(h));
        }
    }
    for k in 0..3 {
        crossbar(&mut steel, k, (k + 1) % 3, platform, SUTRO_TOP_DEPTH_M, SUTRO_TOP_REACH_M, 0.9);
        let (a, b) = (leg_point(k, platform), leg_point((k + 1) % 3, platform));
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let len = dx.hypot(dz);
        let (ex, ez) = (dx / len, dz / len);
        let r = SUTRO_TOP_REACH_M;
        far.beam([a[0] - ex * r, a[1], a[2] - ez * r], [b[0] + ex * r, b[1], b[2] + ez * r], 1.6, SUTRO_TOP_DEPTH_M * 0.7, band_colour(platform));
    }
    for h in [platform + 22.0, platform + 44.0] {
        let colour = band_colour(h);
        for k in 0..3 {
            steel.beam(leg_point(k, h), leg_point((k + 1) % 3, h), 0.6, 0.8, colour);
        }
    }
    let toward_axis = |x: f64, z: f64, min_m: f64| -> Option<[f64; 2]> {
        let (dx, dz) = (c[0] - x, c[1] - z);
        let dd = dx.hypot(dz);
        if dd < min_m { None } else { Some([dx / dd, dz / dd]) }
    };
    steel.shade_occlusion(Some(&toward_axis), 0.62, 0.74, 1.5);
    far.shade_occlusion(Some(&toward_axis), 0.62, 0.74, 1.5);
    let mut colliders = Vec::new();
    for (leg, (p, gr, yaw)) in feet.iter().enumerate() {
        let half = foot_side / 2.0;
        colliders.push(crate::Collider { part: "footing".into(), x: p[0], z: p[1], y0: gr - foot_bury, y1: gr + foot_top, hx: half, hz: half, yaw: *yaw });
        let h0 = gr + foot_top - base_y;
        let h1 = h0 + 12.0;
        let a = leg_point(leg, (h0 + h1) / 2.0);
        let leg_half = leg_w / 2.0 + 0.3;
        colliders.push(crate::Collider { part: "leg".into(), x: a[0], z: a[2], y0: base_y + h0, y1: base_y + h1, hx: leg_half, hz: leg_half, yaw: *yaw });
    }
    let materials = vec![];
    let _ = (white, orange);
    Out {
        version: format!("{}+towers3", site["version"].as_str().unwrap_or("")),
        groups: vec![("steel".into(), steel.freeze()), ("concrete".into(), concrete.freeze())],
        far: far.freeze(),
        colliders,
        materials,
        lights,
        stats: serde_json::json!({ "baseY": base_y, "topY": base_y + height, "levelsM": levels, "feet": feet.iter().map(|f| [f.0[0], f.0[1], f.1]).collect::<Vec<_>>() }),
    }
}

/// The third pass of a registered landmark building, by id, with the
/// fourth over it when the switches leave it on (`crate::towers4_on`).
pub fn build(id: &str, site: &Value, g: &GroundGrid) -> Option<Out> {
    build_v(id, site, g, crate::towers4_on())
}

/// `build` with the fourth pass on or off (`v4`; off: the third pass alone).
pub fn build_v(id: &str, site: &Value, g: &GroundGrid, v4: bool) -> Option<Out> {
    build_v_opened(id, site, g, v4, false)
}

/// `build_v`; `open`: the bands the landmark-access decks replace are left
/// out (Coit's belvedere, Salesforce's top-floor glass, Transamerica's top level).
pub fn build_v_opened(id: &str, site: &Value, g: &GroundGrid, v4: bool, open: bool) -> Option<Out> {
    if v4 {
        return Some(match id {
            "transamerica-pyramid" => transamerica4_opened(site, g, open),
            "salesforce-tower" => salesforce4_opened(site, g, open),
            "city-hall" => city_hall4(site, g),
            "coit-tower" => coit_opened(site, g, open),
            "sutro-tower" => sutro(site, g),
            _ => return None,
        });
    }
    Some(match id {
        "transamerica-pyramid" => transamerica_opened(site, g, open),
        "salesforce-tower" => salesforce_opened(site, g, open),
        "coit-tower" => coit_opened(site, g, open),
        "sutro-tower" => sutro(site, g),
        "city-hall" => city_hall(site, g),
        _ => return None,
    })
}

// ---- the fourth pass (`crate::towers4_on`; `eo-app --no-towers4` builds the third) ----

/// The spire's aluminium cladding: coated aluminium as the skyline towers'
/// (skyline.rs METAL4_*: metalness 0.6, roughness 0.35) of aluminium's
/// reflectance, F0 linear (0.913, 0.922, 0.924) (Real-Time Rendering, 4th
/// ed., table 9.2), weathered to 0.85 of it, in place of the precast-grey 0.85 metal.
pub const ALUMINIUM_F0: [f64; 3] = [0.913, 0.922, 0.924];
pub const SPIRE4_WEATHER: f64 = 0.85;
pub const SPIRE4_ROUGHNESS: f64 = crate::skyline::METAL4_ROUGHNESS;
pub const SPIRE4_METALNESS: f64 = crate::skyline::METAL4_METALNESS;

pub fn transamerica4(site: &Value, g: &GroundGrid) -> Out {
    transamerica4_opened(site, g, false)
}

/// `transamerica4`, opened for the landmark-access deck with `open` (`transamerica_opened`).
pub fn transamerica4_opened(site: &Value, g: &GroundGrid, open: bool) -> Out {
    let mut o = transamerica_opened(site, g, open);
    // the spire's vertex colour (the export's metalSrgb) to aluminium's F0.
    if let Some((_, f)) = o.groups.iter_mut().find(|(k, _)| k == "metal") {
        for c in f.colors.chunks_exact_mut(3) {
            for k in 0..3 {
                c[k] = (ALUMINIUM_F0[k] * SPIRE4_WEATHER) as f32;
            }
        }
    }
    if let Some((_, m)) = o.materials.iter_mut().find(|(k, _)| k == "metal") {
        m.roughness = SPIRE4_ROUGHNESS;
        m.metalness = SPIRE4_METALNESS;
        m.env_class = "facade-metal".into();
    }
    o.version = o.version.replace("+towers3", "+towers4");
    o
}

/// Salesforce's crown as its parts: the shaft's fins carried up the crown
/// (pearl painted aluminium, 2 ft (0.61 m) proud: Wikipedia, 'white aluminum
/// fins and perforated sunshades, which reach up to two feet beyond the glass
/// skin'), one a facade module (5 ft, 1.524 m, skyline-buildings.json
/// salesforce-curtain) round the crown, `CROWN_FIN_THICK_M` thick, and its
/// sunshades a ring each storey (4.557 m); behind them the perforated
/// aluminium panels (Wikipedia: Day for Night's LEDs on the top six floors;
/// KQED, 2018: over a hundred perforated aluminium panels, the LEDs' light
/// reflected out through them) up to `CROWN_SCREEN_TOP` of the crown, the
/// facade style salesforce-screen (their open share over the dark
/// structure; the sculpture at night); over it the fins and rings alone,
/// open to the sky (Pelli Clarke Pelli: 'a transparent crown that appears
/// to dissolve into the sky'), round a dark well. Thickness and the screen's
/// top INFERRED.
pub const CROWN_FIN_DEPTH_M: f64 = 0.61;
pub const CROWN_FIN_THICK_M: f64 = 0.12;
pub const CROWN_SCREEN_TOP: f64 = 0.82;
pub const CROWN_SHADE_THICK_M: f64 = 0.15;
/// The structure seen inside the crown and its well: a dark painted steel grey (sRGB #33383d, INFERRED).
pub const CROWN_WELL_SRGB: u32 = 0x33383d;

/// A point of a closed ring at `t` of its perimeter (by length) and the ring's outward normal there.
fn ring_at(ring: &[[f64; 2]], c: [f64; 2], t: f64) -> ([f64; 2], [f64; 2]) {
    let n = ring.len();
    let lens: Vec<f64> = (0..n).map(|i| {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        (b[0] - a[0]).hypot(b[1] - a[1])
    }).collect();
    let total: f64 = lens.iter().sum();
    let mut s = t.rem_euclid(1.0) * total;
    for i in 0..n {
        if s <= lens[i] || i == n - 1 {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let f = (s / lens[i].max(1e-9)).clamp(0.0, 1.0);
            let p = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
            let tg = [(b[0] - a[0]) / lens[i].max(1e-9), (b[1] - a[1]) / lens[i].max(1e-9)];
            let mut nn = [-tg[1], tg[0]];
            if (p[0] - c[0]) * nn[0] + (p[1] - c[1]) * nn[1] < 0.0 {
                nn = [-nn[0], -nn[1]];
            }
            return (p, nn);
        }
        s -= lens[i];
    }
    (ring[0], [1.0, 0.0])
}

fn ring_length(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    (0..n).map(|i| (ring[(i + 1) % n][0] - ring[i][0]).hypot(ring[(i + 1) % n][1] - ring[i][1])).sum()
}

/// A ring pushed out by `d` along each vertex's outward normal (the mean of its edges').
fn offset_ring(ring: &[[f64; 2]], c: [f64; 2], d: f64) -> Vec<[f64; 2]> {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (p, a, b) = (ring[i], ring[(i + n - 1) % n], ring[(i + 1) % n]);
            let mut nn = [-(b[1] - a[1]), b[0] - a[0]];
            let l = nn[0].hypot(nn[1]).max(1e-9);
            nn = [nn[0] / l, nn[1] / l];
            if (p[0] - c[0]) * nn[0] + (p[1] - c[1]) * nn[1] < 0.0 {
                nn = [-nn[0], -nn[1]];
            }
            [p[0] + nn[0] * d, p[1] + nn[1] * d]
        })
        .collect()
}

pub fn salesforce4(site: &Value, g: &GroundGrid) -> Out {
    salesforce4_opened(site, g, false)
}

/// `salesforce4`, opened for the landmark-access deck with `open` (`salesforce_opened`).
pub fn salesforce4_opened(site: &Value, g: &GroundGrid, open: bool) -> Out {
    use crate::geometry::{scale_ring, srgb_hex};
    let mut o = salesforce_opened(site, g, open);
    let d = &site["dimensions"];
    let fin_c = srgb_hex(crate::towers2::SALESFORCE_PEARL_SRGB);
    let well_c = srgb_hex(CROWN_WELL_SRGB);
    let crown_c = hex(d, "crownSrgb");
    let ring = pairs(&flat(&site["record"]["points"]));
    let c = ring_centroid(&ring);
    let base = base_y(&ring, g);
    let (h_total, top_floor) = (n(d, "heightM"), n(d, "topFloorM"));
    let (k_tf, k_top) = (n(d, "scaleAtTopFloor"), n(d, "scaleAtTop"));
    let crown_h = h_total - top_floor;
    let p = crate::towers2::SALESFORCE_CROWN_P;
    let scale_at = |y: f64| {
        let t = ((y - top_floor) / crown_h).clamp(0.0, 1.0);
        k_top + (k_tf - k_top) * (1.0 - t.powf(p)).max(0.0).powf(1.0 / p)
    };
    let at = |y: f64| scale_ring(&ring, c[0], c[1], scale_at(y));
    let bands = 18;
    let ty = |i: usize| top_floor + crown_h * ((i as f64 / bands as f64) * std::f64::consts::FRAC_PI_2).sin();
    let mut fins = MeshArrays::new("salesforce:crown");
    let mut screen = MeshArrays::new("salesforce:screen");
    // the fins: one a module round the top floor's perimeter, following the crown's profile.
    let module = crate::skyline::data().facades.get("salesforce-curtain").map(|f| f.module_m).unwrap_or(1.524);
    let nf = (ring_length(&at(top_floor)) / module).round().max(8.0) as usize;
    for f in 0..nf {
        let t = (f as f64 + 0.5) / nf as f64;
        for i in 0..bands {
            let (y0, y1) = (ty(i), ty(i + 1));
            let (p0, n0) = ring_at(&at(y0), c, t);
            let (p1, n1) = ring_at(&at(y1), c, t);
            let side = |nn: [f64; 2], k: f64| [-nn[1] * k, nn[0] * k];
            let hw = CROWN_FIN_THICK_M / 2.0;
            let corner = |pp: [f64; 2], nn: [f64; 2], out: f64, s: f64, y: f64| -> V3 {
                let sd = side(nn, s * hw);
                [pp[0] + nn[0] * out + sd[0], base + y, pp[1] + nn[1] * out + sd[1]]
            };
            let (r0, r1) = (-0.05, CROWN_FIN_DEPTH_M);
            // the two cheeks and the outer edge.
            for (s, (oa, ob)) in [(1.0, (r0, r1)), (-1.0, (r1, r0))] {
                fins.quad(corner(p0, n0, oa, s, y0), corner(p0, n0, ob, s, y0), corner(p1, n1, ob, s, y1), corner(p1, n1, oa, s, y1), fin_c);
            }
            fins.quad(corner(p0, n0, r1, 1.0, y0), corner(p0, n0, r1, -1.0, y0), corner(p1, n1, r1, -1.0, y1), corner(p1, n1, r1, 1.0, y1), fin_c);
            if i + 1 == bands {
                fins.quad(corner(p1, n1, r0, 1.0, y1), corner(p1, n1, r1, 1.0, y1), corner(p1, n1, r1, -1.0, y1), corner(p1, n1, r0, -1.0, y1), fin_c);
            }
        }
    }
    // the sunshades: a ring each storey, from the panels out to the fins' edge.
    let storey = crate::skyline::data().facades.get("salesforce-curtain").map(|f| f.storey_m).unwrap_or(4.557);
    let mut y = top_floor + storey;
    while y < h_total - 1.0 {
        let inner = at(y);
        let outer = offset_ring(&inner, c, CROWN_FIN_DEPTH_M);
        let nr = inner.len();
        let (ya, yb) = (base + y - CROWN_SHADE_THICK_M / 2.0, base + y + CROWN_SHADE_THICK_M / 2.0);
        for k in 0..nr {
            let j = (k + 1) % nr;
            let (ia, ib, oa, ob) = (inner[k], inner[j], outer[k], outer[j]);
            // top, outer edge, underside (each wound to face out).
            let q = |a: [f64; 2], b: [f64; 2], yy: f64| -> (V3, V3) { ([a[0], yy, a[1]], [b[0], yy, b[1]]) };
            let (t0, t1) = q(ia, ib, yb);
            let (t3, t2) = q(oa, ob, yb);
            let mut add = |a: V3, b: V3, cc: V3, dd: V3, want: [f64; 3]| {
                let nn = cross3(sub3(b, a), sub3(dd, a));
                if dot3(nn, want) >= 0.0 {
                    fins.quad(a, b, cc, dd, fin_c);
                } else {
                    fins.quad(dd, cc, b, a, fin_c);
                }
            };
            add(t0, t1, t2, t3, [0.0, 1.0, 0.0]);
            let (b0, b1) = q(ia, ib, ya);
            let (b3, b2) = q(oa, ob, ya);
            add(b0, b3, b2, b1, [0.0, -1.0, 0.0]);
            let mid = [(oa[0] + ob[0]) / 2.0 - c[0], 0.0, (oa[1] + ob[1]) / 2.0 - c[1]];
            add([oa[0], ya, oa[1]], [ob[0], ya, ob[1]], [ob[0], yb, ob[1]], [oa[0], yb, oa[1]], mid);
        }
        y += storey;
    }
    // the perforated panels, a hair inside the fins' roots, up to the screen's top.
    let screen_top = top_floor + CROWN_SCREEN_TOP * crown_h;
    for i in 0..bands {
        let (y0, y1) = (ty(i), ty(i + 1).min(screen_top));
        if y0 >= screen_top {
            break;
        }
        screen.loft(&scale_ring(&at(y0), c[0], c[1], 0.997), base + y0, &scale_ring(&at(y1), c[0], c[1], 0.997), base + y1, fin_c, false);
    }
    // the structure inside, seen over the panels' top: a dark floor at their top, the lip and the well as towers2.
    crate::skyline::cap(&mut screen, &scale_ring(&at(screen_top), c[0], c[1], 0.99), base + screen_top, well_c);
    let top = at(h_total);
    fins.loft(&top, base + h_total, &scale_ring(&top, c[0], c[1], 0.9), base + h_total, crown_c, false);
    o.groups.retain(|(k, _)| k != "crown");
    o.groups.push(("crown".into(), fins.freeze()));
    o.groups.push(("screen".into(), screen.freeze()));
    o.materials.retain(|(k, _)| k != "crown");
    o.materials.push(("crown".into(), crate::skyline::plain_material(SALESFORCE_FIN_ROUGHNESS, 0.0, "facade-masonry", None)));
    o.materials.push(("screen".into(), with_origin(facade_material("salesforce-screen", 0.4, 0.0, None), base + top_floor)));
    o.version = o.version.replace("+towers3", "+towers4");
    o.stats["crownFins"] = serde_json::json!(nf);
    o
}

/// City Hall's gilding: gold leaf a metal (metalness 1) of gold's
/// reflectance, F0 linear (1.000, 0.766, 0.336) (Real-Time Rendering, 4th
/// ed., table 9.2), weathered to 0.9 of it, at roughness 0.38 (leaf laid over
/// a ground's texture, INFERRED) in place of the export's 0.5 and metalness
/// 0.8, so the ribs and the lantern catch the sun;
/// the lead-grey cladding between them keeps its roughness (0.55).
/// San Francisco City Hall's dome trim is gilded (Wikipedia, San Francisco
/// City Hall: re-gilded in the 1999 restoration).
pub const GOLD_F0: [f64; 3] = [1.0, 0.766, 0.336];
pub const GILT4_WEATHER: f64 = 0.9;
pub const GILT4_ROUGHNESS: f64 = 0.38;

pub fn city_hall4(site: &Value, g: &GroundGrid) -> Out {
    let mut o = city_hall(site, g);
    let gold = crate::geometry::srgb_hex(n(&site["dimensions"], "goldSrgb") as u32);
    if let Some((_, f)) = o.groups.iter_mut().find(|(k, _)| k == "dome") {
        for c in f.colors.chunks_exact_mut(3) {
            // the gilt by its vertex colour (the export's goldSrgb); the cladding stays.
            let warm = (c[0] as f64 - c[2] as f64) / (c[0] as f64).max(1e-3);
            let gw = (gold[0] - gold[2]) / gold[0].max(1e-3);
            if warm > 0.6 * gw {
                for k in 0..3 {
                    c[k] = (GOLD_F0[k] * GILT4_WEATHER) as f32;
                }
            }
        }
    }
    // (the gilt's metalness and roughness: eo-render landmarks_surface.wgsl `LM_GILT4_*`.)
    o.version = format!("{}+towers4", o.version);
    o
}
