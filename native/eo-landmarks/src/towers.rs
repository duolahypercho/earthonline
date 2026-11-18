//! the registered landmark buildings, ported from their JS
//! generators (src/world/landmarks/, the checkout `EO_JS_ROOT` names):
//!
//! | Rust | JS |
//! | --- | --- |
//! | `transamerica` | transamerica-pyramid.js `buildTransamericaPyramid` (+ -site.js `transamericaFrame`, `transamericaHalfAt`) |
//! | `salesforce` | salesforce-tower.js `buildSalesforceTower` (+ -site.js `salesforceScaleAt`) |
//! | `coit` | coit-tower.js `buildCoitTower` |
//! | `sutro` | sutro-tower.js `buildSutroTower`, `sutroLegRadiusAt`, `sutroTowerPlan`, its colliders |
//! | `city_hall` | city-hall.js `buildCityHall`, `shadeColonnadeBays` |
//!
//! Every dimension and colour is the export's (`buildings` and `registry` of
//! data/landmarks-js.json: the `-site.js` modules' records, dimensions and
//! frames as the JS evaluates them); each generator is checked against the
//! node-run JS on its own ground (tests/js_oracle.rs `buildings_match_the_js`).

use crate::Collider;
use crate::geometry::{ALL, Frozen, GroundGrid, MeshArrays, Rgb, V3, pairs, ring_centroid, scale_ring, signed_area, srgb_hex};
use serde_json::Value;
use std::f64::consts::PI;

pub struct Out {
    pub version: String,
    pub groups: Vec<(String, Frozen)>,
    pub far: Frozen,
    pub colliders: Vec<Collider>,
    pub stats: Value,
}

fn n(v: &Value, k: &str) -> f64 {
    crate::data::num(v, k)
}

fn hex(v: &Value, k: &str) -> Rgb {
    srgb_hex(n(v, k) as u32)
}

fn flat(v: &Value) -> Vec<f64> {
    v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default()
}

fn v2(v: &Value) -> [f64; 2] {
    [v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0)]
}

/// The lowest ground under a ring.
fn base_y(ring: &[[f64; 2]], g: &GroundGrid) -> f64 {
    ring.iter().map(|p| g.at(p[0], p[1])).fold(f64::INFINITY, f64::min)
}

pub fn transamerica(site: &Value, g: &GroundGrid) -> Out {
    transamerica_opened(site, g, false)
}

/// `transamerica`; `open` (landmark access, access.rs) leaves out the top
/// level's walls and windows, which the observation room draws with real openings.
pub fn transamerica_opened(site: &Value, g: &GroundGrid, open: bool) -> Out {
    let d = &site["dimensions"];
    let fr = &site["frame"];
    let (cx, cz, u, v, fhalf) = (n(fr, "cx"), n(fr, "cz"), v2(&fr["u"]), v2(&fr["v"]), n(fr, "half"));
    let (precast_c, glass_c, metal_c) = (hex(d, "precastSrgb"), hex(d, "glassSrgb"), hex(d, "metalSrgb"));
    let ring = pairs(&flat(&site["record"]["points"]));
    let base = base_y(&ring, g);
    let mut precast = MeshArrays::new("transamerica:precast");
    let mut glass = MeshArrays::new("transamerica:glass");
    let mut metal = MeshArrays::new("transamerica:metal");
    let mut far = MeshArrays::new("transamerica:far");
    let h_total = n(d, "heightM");
    let spire_y = h_total - n(d, "spireM");
    let half = |y: f64| (fhalf * (1.0 - y / h_total)).max(0.0);
    let at = |a: f64, b: f64, y: f64| -> V3 { [cx + u[0] * a + v[0] * b, base + y, cz + u[1] * a + v[1] * b] };
    let square = |h: f64| -> Vec<[f64; 2]> {
        [[-h, -h], [h, -h], [h, h], [-h, h]].iter().map(|[a, b]| {
            let p = at(*a, *b, 0.0);
            [p[0], p[2]]
        }).collect()
    };
    let top0 = n(d, "baseStoreysM");
    let floor_m = n(d, "floorM");
    let band = n(d, "windowBandM");
    glass.prism(&square(half(0.0) - 4.5), base - 0.5, base + top0 - 2.0, glass_c, true, false);
    precast.loft(&square(half(top0 - 2.2)), base + top0 - 2.2, &square(half(top0)), base + top0, precast_c, false);
    for side in 0..4 {
        for k in -4i32..=4 {
            let t = k as f64 / 4.6;
            let edge = |y: f64, inset: f64| {
                let h = half(y) - inset;
                let s = t * h;
                match side {
                    0 => at(s, -h, y),
                    1 => at(h, s, y),
                    2 => at(-s, h, y),
                    _ => at(-h, -s, y),
                }
            };
            precast.beam(edge(0.0, 0.6), edge(top0 - 2.2, 0.6), 1.2, 1.2, precast_c);
        }
    }
    let mut levels = Vec::new();
    let mut y = top0;
    while y < spire_y - 0.01 {
        levels.push(y);
        y += floor_m;
    }
    levels.push(spire_y);
    for i in 0..levels.len() - 1 {
        if open && i == levels.len() - 2 {
            continue;
        }
        let (y0, y1) = (levels[i], levels[i + 1]);
        precast.loft(&square(half(y0)), base + y0, &square(half(y1)), base + y1, precast_c, false);
        let yw0 = y0 + (y1 - y0 - band) / 2.0;
        let yw1 = yw0 + band;
        for side in 0..4 {
            let h0 = half(yw0) + 0.06;
            let h1 = half(yw1) + 0.06;
            let count = ((2.0 * h0 - 2.0) / n(d, "windowPitchM")).floor().max(1.0) as i64;
            for k in 0..count {
                let s0 = -1.0 + (2.0 * k as f64 + 0.15) / count as f64;
                let s1 = -1.0 + (2.0 * k as f64 + 1.85) / count as f64;
                let p = |s: f64, h: f64, y: f64| match side {
                    0 => at(s * (h - 1.0), -h, y),
                    1 => at(h, s * (h - 1.0), y),
                    2 => at(-s * (h - 1.0), h, y),
                    _ => at(-h, -s * (h - 1.0), y),
                };
                glass.quad(p(s1, h0, yw0), p(s0, h0, yw0), p(s0, h1, yw1), p(s1, h1, yw1), glass_c);
            }
        }
    }
    let hs = half(spire_y);
    let apex = at(0.0, 0.0, h_total);
    let sq0 = [at(-hs, -hs, spire_y), at(hs, -hs, spire_y), at(hs, hs, spire_y), at(-hs, hs, spire_y)];
    let plan: Vec<[f64; 2]> = sq0.iter().map(|p| [p[0], p[2]]).collect();
    let mut sq = sq0.to_vec();
    if signed_area(&plan) > 0.0 {
        sq.reverse();
    }
    for i in 0..4 {
        metal.tri(sq[i], sq[(i + 1) % 4], apex, metal_c);
    }
    let wing_y0 = top0 + (n(d, "wingsFromFloor") - 1.0) * floor_m;
    let wing_y1 = spire_y + 12.0;
    let out = half(wing_y0);
    let mut faces = vec![u, [-u[0], -u[1]], v, [-v[0], -v[1]]];
    // Stable, as the JS sort: the two faces whose normals run nearest east-west.
    faces.sort_by(|p, q| q[0].abs().partial_cmp(&p[0].abs()).unwrap());
    for nrm in faces.iter().take(2) {
        let side = [-nrm[1], nrm[0]];
        let w = n(d, "wingWidthM") / 2.0;
        let corners = [[-w, 0.0], [w, 0.0], [w, out - half(wing_y1) + 2.0], [-w, out - half(wing_y1) + 2.0]];
        let ring2: Vec<[f64; 2]> = corners.iter().map(|[s, dd]| [cx + nrm[0] * (out - dd) + side[0] * s, cz + nrm[1] * (out - dd) + side[1] * s]).collect();
        precast.prism(&ring2, base + wing_y0, base + wing_y1, precast_c, true, false);
        far.prism(&ring2, base + wing_y0, base + wing_y1, precast_c, true, false);
        let mut y = wing_y0 + 1.0;
        while y + band < wing_y1 {
            let a = [cx + nrm[0] * (out + 0.06) + side[0] * (-w + 1.0), base + y, cz + nrm[1] * (out + 0.06) + side[1] * (-w + 1.0)];
            let b = [cx + nrm[0] * (out + 0.06) + side[0] * (w - 1.0), base + y, cz + nrm[1] * (out + 0.06) + side[1] * (w - 1.0)];
            glass.quad(b, a, [a[0], a[1] + band, a[2]], [b[0], b[1] + band, b[2]], glass_c);
            y += floor_m;
        }
    }
    // the far item stops under the opened top level (the observation room draws it at every distance).
    let far_top = if open { levels[levels.len() - 2] } else { spire_y };
    let far_levels = [0.0, top0, 60.0, 120.0, far_top];
    for i in 0..far_levels.len() - 1 {
        far.loft(&square(half(far_levels[i])), base + far_levels[i], &square(half(far_levels[i + 1])), base + far_levels[i + 1], precast_c, false);
    }
    for i in 0..4 {
        far.tri(sq[i], sq[(i + 1) % 4], apex, metal_c);
    }
    Out {
        version: site["version"].as_str().unwrap_or("").into(),
        groups: vec![("precast".into(), precast.freeze()), ("glass".into(), glass.freeze()), ("metal".into(), metal.freeze())],
        far: far.freeze(),
        colliders: vec![],
        stats: serde_json::json!({ "baseY": base, "apexY": base + h_total, "spireFromY": base + spire_y, "wingsFromY": base + wing_y0, "halfBaseM": fhalf }),
    }
}

pub fn salesforce(site: &Value, g: &GroundGrid) -> Out {
    salesforce_opened(site, g, false)
}

/// `salesforce`; `open` (landmark access, access.rs) leaves out the top
/// floor's vision glass, which the observation level draws as open glazing.
pub fn salesforce_opened(site: &Value, g: &GroundGrid, open: bool) -> Out {
    let d = &site["dimensions"];
    let (glass_c, spandrel, crown_c, gap) = (hex(d, "glassSrgb"), hex(d, "spandrelSrgb"), hex(d, "crownSrgb"), hex(d, "crownGapSrgb"));
    let ring = pairs(&flat(&site["record"]["points"]));
    let [cx, cz] = ring_centroid(&ring);
    let base = base_y(&ring, g);
    let (h_total, top_floor, floors) = (n(d, "heightM"), n(d, "topFloorM"), n(d, "floors"));
    let (k_top_floor, k_top) = (n(d, "scaleAtTopFloor"), n(d, "scaleAtTop"));
    let scale_at = |y: f64| {
        if y <= top_floor { 1.0 + (k_top_floor - 1.0) * (y / top_floor) } else { k_top_floor + (k_top - k_top_floor) * ((y - top_floor) / (h_total - top_floor)) }
    };
    let at = |y: f64| scale_ring(&ring, cx, cz, scale_at(y));
    let mut glass = MeshArrays::new("salesforce:glass");
    let mut crown = MeshArrays::new("salesforce:crown");
    let mut far = MeshArrays::new("salesforce:far");
    let floor = top_floor / floors;
    for f in 0..floors as i64 {
        let y0 = f as f64 * floor;
        let y1 = y0 + floor - 0.6;
        let y2 = y0 + floor;
        if !(open && f == floors as i64 - 1) {
            glass.loft(&at(y0), base + y0, &at(y1), base + y1, glass_c, false);
        }
        glass.loft(&at(y1), base + y1, &at(y2), base + y2, spandrel, false);
    }
    let bands = 16.0;
    for b in 0..16 {
        let y0 = top_floor + ((h_total - top_floor) * b as f64) / bands;
        let y1 = top_floor + ((h_total - top_floor) * (b + 1) as f64) / bands;
        let ym = y0 + (y1 - y0) * 0.7;
        crown.loft(&at(y0), base + y0, &at(ym), base + ym, crown_c, false);
        crown.loft(&at(ym), base + ym, &at(y1), base + y1, gap, false);
    }
    let top = at(h_total);
    crown.loft(&top, base + h_total, &scale_ring(&top, cx, cz, 0.92), base + h_total, crown_c, false);
    crown.prism(&scale_ring(&top, cx, cz, 0.92), base + h_total - 6.0, base + h_total - 0.5, gap, true, false);
    // opened, the far item leaves the top floor's vision glass to the observation level (drawn at every distance).
    let (vis0, vis1) = ((floors - 1.0) * floor, (floors - 1.0) * floor + floor - 0.6);
    let far_bands: Vec<(f64, f64, Rgb)> = if open {
        vec![(0.0, 100.0, glass_c), (100.0, 200.0, glass_c), (200.0, vis0, glass_c), (vis1, top_floor, glass_c), (top_floor, h_total, crown_c)]
    } else {
        vec![(0.0, 100.0, glass_c), (100.0, 200.0, glass_c), (200.0, top_floor, glass_c), (top_floor, h_total, crown_c)]
    };
    for (y0, y1, c) in far_bands {
        far.loft(&at(y0), base + y0, &at(y1), base + y1, c, y1 == h_total);
    }
    Out {
        version: site["version"].as_str().unwrap_or("").into(),
        groups: vec![("glass".into(), glass.freeze()), ("crown".into(), crown.freeze())],
        far: far.freeze(),
        colliders: vec![],
        stats: serde_json::json!({ "baseY": base, "topY": base + h_total, "crownFromY": base + top_floor }),
    }
}

pub fn coit(site: &Value, g: &GroundGrid) -> Out {
    coit_opened(site, g, false)
}

/// `coit`; `open` (landmark access, access.rs) leaves out the belvedere's
/// solid band and its painted openings, which the gallery draws as a real arcade.
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
    let (flutes_n, flute_depth, cornice_m, lr) = (n(d, "flutes"), n(d, "fluteDepth"), n(d, "corniceM"), n(d, "lanternRadiusM"));
    concrete.prism(&ring, base_ground - 0.5, base, concrete_c, true, false);
    far.prism(&ring, base_ground - 0.5, base, concrete_c, true, false);
    let flutes = |t: f64, r: f64| r * (1.0 - flute_depth * (0.5 + 0.5 * (flutes_n * t).cos()));
    let mut rows = Vec::new();
    for k in 0..=8 {
        let y = base + ((bel0 - base) * k as f64) / 8.0;
        rows.push([r0 + ((r1 - r0) * (y - base)) / (bel1 - base), y]);
    }
    concrete.lathe(cx, cz, &rows, (flutes_n * 4.0) as usize, concrete_c, Some(&flutes), None);
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
    Out {
        version: site["version"].as_str().unwrap_or("").into(),
        groups: vec![("concrete".into(), concrete.freeze()), ("openings".into(), openings.freeze())],
        far: far.freeze(),
        colliders: vec![],
        stats: serde_json::json!({ "baseY": base_ground, "topY": top, "shaftRadiusM": r0 }),
    }
}

struct SutroPlan {
    base_y: f64,
    feet: Vec<([f64; 2], [f64; 2], f64, f64)>,
}

pub fn sutro(site: &Value, g: &GroundGrid) -> Out {
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
    // sutroTowerPlan: each leg's foot on the ground, the base their mean.
    let feet: Vec<([f64; 2], [f64; 2], f64, f64)> = dirs
        .iter()
        .map(|[ux, uz]| {
            let (x, z) = (c[0] + ux * rb, c[1] + uz * rb);
            let gr = g.at(x, z);
            ([x, z], [*ux, *uz], if gr.is_nan() { 0.0 } else { gr }, uz.atan2(*ux))
        })
        .collect();
    let plan = SutroPlan { base_y: feet.iter().fold(0.0, |s, f| s + f.2) / feet.len() as f64, feet };
    let leg_point = |leg: usize, h: f64| -> V3 {
        let [ux, uz] = dirs[leg];
        let r = leg_radius_at(h);
        [c[0] + ux * r, plan.base_y + h, c[1] + uz * r]
    };
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
        let steps = ((h1 - h0) / panel).round().max(1.0) as i64;
        for s in 0..steps {
            let ha = h0 + ((h1 - h0) * s as f64) / steps as f64;
            let hb = h0 + ((h1 - h0) * (s + 1) as f64) / steps as f64;
            let colour = band_colour((ha + hb) / 2.0);
            for k in 0..3 {
                mesh.beam(at(ha, k), at(hb, k), 0.55, 0.55, colour);
                mesh.beam(at(hb, k), at(hb, (k + 1) % 3), 0.28, 0.28, colour);
                let (a, b) = if s % 2 == 0 { (k, (k + 1) % 3) } else { ((k + 1) % 3, k) };
                mesh.beam(at(ha, a), at(hb, b), 0.22, 0.22, colour);
            }
        }
    };
    let ring_frame = |mesh: &mut MeshArrays, h: f64, depth: f64, width: f64| {
        let colour = band_colour(h);
        for k in 0..3 {
            mesh.beam(leg_point(k, h), leg_point((k + 1) % 3, h), width, depth, colour);
        }
    };
    let mut steel = MeshArrays::new("sutro-tower:steel");
    let mut concrete = MeshArrays::new("sutro-tower:concrete");
    let mut far = MeshArrays::new("sutro-tower:far");
    let (foot_side, foot_top, foot_bury) = (n(d, "footingSideM"), n(d, "footingTopM"), n(d, "footingBuryM"));
    for (p, _, gr, yaw) in &plan.feet {
        let half = foot_side / 2.0;
        concrete.box_(p[0], p[1], gr - foot_bury, gr + foot_top, half, half, *yaw, concrete_c, ALL);
        far.box_(p[0], p[1], gr - foot_bury, gr + foot_top, half, half, *yaw, concrete_c, ALL);
    }
    let (leg_w, mast_w, panel) = (n(d, "legWidthM"), n(d, "mastWidthM"), n(d, "panelM"));
    for leg in 0..3 {
        let start = plan.feet[leg].2 + foot_top - plan.base_y;
        lattice(&mut steel, leg, start, platform, leg_w, panel);
        lattice(&mut steel, leg, platform, height - 4.0, mast_w, panel * 0.75);
        let top = leg_point(leg, height - 22.0);
        steel.box_(top[0], top[2], top[1], plan.base_y + height, 0.9, 0.9, plan.feet[leg].3, band_colour(height - 1.0), ALL);
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
    }
    let mut frames = flat(&d["ringFramesM"]);
    frames.push(platform);
    let mut previous: Option<f64> = None;
    for &h in &frames {
        let is_platform = h == platform;
        ring_frame(&mut steel, h, if is_platform { 2.4 } else { 1.2 }, if is_platform { 1.6 } else { 0.8 });
        ring_frame(&mut far, h, if is_platform { 2.4 } else { 1.2 }, if is_platform { 1.6 } else { 0.8 });
        if let Some(h0) = previous {
            let colour = band_colour((h0 + h) / 2.0);
            for k in 0..3 {
                let j = (k + 1) % 3;
                steel.beam(leg_point(k, h0), leg_point(j, h), 0.45, 0.45, colour);
                steel.beam(leg_point(j, h0), leg_point(k, h), 0.45, 0.45, colour);
            }
        }
        previous = Some(h);
    }
    for h in [platform + 22.0, platform + 44.0] {
        ring_frame(&mut steel, h, 0.8, 0.6);
    }
    let toward_axis = |x: f64, z: f64, min_m: f64| -> Option<[f64; 2]> {
        let (dx, dz) = (c[0] - x, c[1] - z);
        let dd = dx.hypot(dz);
        if dd < min_m { None } else { Some([dx / dd, dz / dd]) }
    };
    steel.shade_occlusion(Some(&toward_axis), 0.62, 0.74, 1.5);
    far.shade_occlusion(Some(&toward_axis), 0.62, 0.74, 1.5);
    let mut colliders = Vec::new();
    for (leg, (p, _, gr, yaw)) in plan.feet.iter().enumerate() {
        let half = foot_side / 2.0;
        colliders.push(Collider { part: "footing".into(), x: p[0], z: p[1], y0: gr - foot_bury, y1: gr + foot_top, hx: half, hz: half, yaw: *yaw });
        let h0 = gr + foot_top - plan.base_y;
        let h1 = h0 + 12.0;
        let a = leg_point(leg, (h0 + h1) / 2.0);
        let leg_half = leg_w / 2.0 + 0.3;
        colliders.push(Collider { part: "leg".into(), x: a[0], z: a[2], y0: plan.base_y + h0, y1: plan.base_y + h1, hx: leg_half, hz: leg_half, yaw: *yaw });
    }
    Out {
        version: site["version"].as_str().unwrap_or("").into(),
        groups: vec![("steel".into(), steel.freeze()), ("concrete".into(), concrete.freeze())],
        far: far.freeze(),
        colliders,
        stats: serde_json::json!({ "baseY": plan.base_y, "topY": plan.base_y + height, "feet": plan.feet.iter().map(|f| [f.0[0], f.0[1], f.2]).collect::<Vec<_>>() }),
    }
}

pub fn city_hall(site: &Value, g: &GroundGrid) -> Out {
    let d = &site["dimensions"];
    let fr = &site["frame"];
    let (cx, cz, u, v, hu, hv) = (n(fr, "cx"), n(fr, "cz"), v2(&fr["u"]), v2(&fr["v"]), n(fr, "hu"), n(fr, "hv"));
    let (stone_c, shade_c, dome_c, gold, window) = (hex(d, "stoneSrgb"), hex(d, "stoneShadeSrgb"), hex(d, "domeSrgb"), hex(d, "goldSrgb"), hex(d, "windowSrgb"));
    let ring = pairs(&flat(&site["record"]["points"]));
    let base = base_y(&ring, g);
    let mut stone = MeshArrays::new("city-hall:stone");
    let mut dome = MeshArrays::new("city-hall:dome");
    let mut windows = MeshArrays::new("city-hall:windows");
    let mut far = MeshArrays::new("city-hall:far");
    let y = |h: f64| base + h;
    let yaw = u[1].atan2(u[0]);
    let w_at = |a: f64, b: f64, h: f64| -> V3 { [cx + u[0] * a + v[0] * b, y(h), cz + u[1] * a + v[1] * b] };
    let bx = |mesh: &mut MeshArrays, a: f64, b: f64, h0: f64, h1: f64, hu: f64, hv: f64, colour: Rgb| {
        let p = w_at(a, b, 0.0);
        mesh.box_(p[0], p[2], y(h0), y(h1), hu, hv, yaw, colour, ALL);
    };
    #[derive(Clone, Copy)]
    struct Face {
        na: f64,
        nb: f64,
        half: f64,
        off: f64,
    }
    let faces = [Face { na: 1.0, nb: 0.0, half: hv, off: hu }, Face { na: -1.0, nb: 0.0, half: hv, off: hu }, Face { na: 0.0, nb: 1.0, half: hu, off: hv }, Face { na: 0.0, nb: -1.0, half: hu, off: hv }];
    let ew_a = u[0].abs() > v[0].abs();
    let is_front = |f: &Face| if ew_a { f.na != 0.0 } else { f.nb != 0.0 };
    let fp = |f: &Face, s: f64, e: f64, h: f64| if f.na != 0.0 { w_at(f.na * e, s * f.na, h) } else { w_at(-s * f.nb, f.nb * e, h) };
    let face_box = |mesh: &mut MeshArrays, f: &Face, s: f64, e: f64, h0: f64, h1: f64, half_along: f64, half_out: f64, colour: Rgb| {
        let a = if f.na != 0.0 { f.na * e } else { -s * f.nb };
        let b = if f.na != 0.0 { s * f.na } else { f.nb * e };
        let (hu, hv) = if f.na != 0.0 { (half_out, half_along) } else { (half_along, half_out) };
        bx(mesh, a, b, h0, h1, hu, hv, colour);
    };
    let face_quad = |mesh: &mut MeshArrays, f: &Face, s0: f64, s1: f64, e: f64, h0: f64, h1: f64, colour: Rgb| {
        mesh.quad(fp(f, s1, e, h0), fp(f, s0, e, h0), fp(f, s0, e, h1), fp(f, s1, e, h1), colour);
    };
    let column = |stone: &mut MeshArrays, f: &Face, s: f64, e: f64, h0: f64, h1: f64, r: f64| {
        let p = fp(f, s, e, 0.0);
        stone.lathe(p[0], p[2], &[[r * 1.08, y(h0)], [r, y(h0 + 0.4)], [r * 0.86, y(h1 - 0.7)], [r * 1.1, y(h1 - 0.3)], [0.0, y(h1 - 0.3) + 0.001]], 12, stone_c, None, None);
        face_box(stone, f, s, e, h0 - 0.5, h0, r * 1.3, r * 1.3, shade_c);
        face_box(stone, f, s, e, h1 - 0.3, h1, r * 1.3, r * 1.3, stone_c);
    };
    let (h_base, h_col, h_ent, h_top) = (n(d, "baseStoreyM"), n(d, "colonnadeTopM"), n(d, "entablatureTopM"), n(d, "corniceM"));
    let loggia = n(d, "loggiaDepthM");
    bx(&mut stone, 0.0, 0.0, -0.5, h_base, hu, hv, stone_c);
    bx(&mut stone, 0.0, 0.0, h_base - 0.6, h_base, hu + 0.35, hv + 0.35, shade_c);
    bx(&mut stone, 0.0, 0.0, h_base, h_col, hu - loggia, hv - loggia, shade_c);
    bx(&mut stone, 0.0, 0.0, h_col, h_ent, hu + 0.2, hv + 0.2, stone_c);
    bx(&mut stone, 0.0, 0.0, h_ent, h_top - 0.45, hu, hv, stone_c);
    bx(&mut stone, 0.0, 0.0, h_top - 0.45, h_top, hu + 0.7, hv + 0.7, shade_c);
    bx(&mut far, 0.0, 0.0, -0.5, h_top, hu, hv, stone_c);
    let portico_half = n(d, "porticoWidthM") / 2.0;
    let end_w = n(d, "endPavilionWidthM");
    let pd = n(d, "endPavilionDepthM");
    let lift = n(d, "endPavilionLiftM");
    let (bay, col_r, portico_depth, balustrade) = (n(d, "bayM"), n(d, "columnRadiusM"), n(d, "porticoDepthM"), n(d, "balustradeM"));
    for f in &faces {
        let front = is_front(f);
        let (half, e) = (f.half, f.off);
        let mut h = 0.85;
        while h < h_base - 0.6 {
            face_quad(&mut stone, f, -half, half, e + 0.03, h - 0.07, h + 0.07, shade_c);
            h += 0.85;
        }
        for sgn in [-1.0, 1.0] {
            let s = sgn * (half - end_w / 2.0);
            face_box(&mut stone, f, s, e + (pd - loggia) / 2.0, -0.5, h_top + lift, end_w / 2.0, (pd + loggia) / 2.0, stone_c);
            face_box(&mut stone, f, s, e + pd - 0.1, h_top + lift - 0.45, h_top + lift, end_w / 2.0 + 0.5, 0.7, shade_c);
            for ds in [-end_w / 2.0 + 1.2, end_w / 2.0 - 1.2] {
                face_box(&mut stone, f, s + ds, e + pd + 0.3, h_base, h_ent, 0.7, 0.3, stone_c);
            }
            for ds in [-3.2, 3.2] {
                face_quad(&mut windows, f, s + ds - 1.1, s + ds + 1.1, e + pd + 0.04, 2.0, 5.6, window);
                face_quad(&mut windows, f, s + ds - 1.1, s + ds + 1.1, e + pd + 0.04, h_base + 2.0, h_col - 2.0, window);
                face_quad(&mut windows, f, s + ds - 0.8, s + ds + 0.8, e + pd + 0.04, h_ent + 0.6, h_top - 0.6, window);
            }
        }
        let span_from = -half + end_w;
        let span_to = half - end_w;
        let count = ((span_to - span_from) / bay).floor() as i64;
        let start = span_from + ((span_to - span_from) - count as f64 * bay) / 2.0;
        for k in 0..=count {
            let s = start + k as f64 * bay;
            if !(front && s.abs() < portico_half + 0.5) {
                column(&mut stone, f, s, e - col_r - 0.25, h_base, h_col, col_r);
            }
            if k == count {
                continue;
            }
            let m = s + bay / 2.0;
            if front && m.abs() < portico_half {
                continue;
            }
            face_quad(&mut windows, f, m - 1.0, m + 1.0, e + 0.03, 2.2, 5.8, window);
            face_quad(&mut windows, f, m - 1.1, m + 1.1, e - loggia + 0.03, h_base + 1.6, h_col - 1.4, window);
            face_quad(&mut windows, f, m - 0.8, m + 0.8, e + 0.03, h_ent + 0.5, h_top - 0.9, window);
        }
        let rail_e = e + 0.25;
        let mut s = span_from + 0.35;
        while s < span_to {
            face_box(&mut stone, f, s, rail_e, h_top, h_top + balustrade - 0.15, 0.11, 0.11, stone_c);
            s += 0.7;
        }
        face_box(&mut stone, f, (span_from + span_to) / 2.0, rail_e, h_top + balustrade - 0.15, h_top + balustrade, (span_to - span_from) / 2.0, 0.3, shade_c);
        if !front {
            continue;
        }
        let pe = e + portico_depth / 2.0;
        face_box(&mut stone, f, 0.0, pe, -0.5, h_base, portico_half, portico_depth / 2.0, stone_c);
        face_box(&mut far, f, 0.0, pe, -0.5, h_ent, portico_half, portico_depth / 2.0, stone_c);
        let pc = n(d, "porticoColumns");
        for k in 0..pc as i64 {
            let s = -portico_half + 2.0 + ((2.0 * portico_half - 4.0) * k as f64) / (pc - 1.0);
            column(&mut stone, f, s, e + portico_depth - 1.1, h_base, h_col, col_r * 1.25);
        }
        face_box(&mut stone, f, 0.0, pe, h_col, h_ent, portico_half + 0.4, portico_depth / 2.0 + 0.3, stone_c);
        let ridge = h_ent + n(d, "pedimentRiseM");
        let eo = e + portico_depth + 0.3;
        let ei = e;
        let w = portico_half + 0.4;
        let (l, r, t) = (fp(f, -w, eo, h_ent), fp(f, w, eo, h_ent), fp(f, 0.0, eo, ridge));
        let (li, ri, ti) = (fp(f, -w, ei, h_ent), fp(f, w, ei, h_ent), fp(f, 0.0, ei, ridge));
        stone.tri(r, l, t, stone_c);
        stone.tri(li, ri, ti, stone_c);
        stone.quad(l, li, ti, t, shade_c);
        stone.quad(ri, r, t, ti, shade_c);
        far.tri(r, l, t, stone_c);
        stone.beam(fp(f, -w - 0.3, eo + 0.25, h_ent), fp(f, 0.0, eo + 0.25, ridge + 0.3), 0.7, 0.6, stone_c);
        stone.beam(fp(f, w + 0.3, eo + 0.25, h_ent), fp(f, 0.0, eo + 0.25, ridge + 0.3), 0.7, 0.6, stone_c);
    }
    let (rot_top, drum_top, ent_m) = (n(d, "rotundaTopM"), n(d, "drumTopM"), n(d, "entablatureM"));
    let rh = n(d, "rotundaSideM") / 2.0;
    bx(&mut stone, 0.0, 0.0, h_top - 1.0, rot_top, rh, rh, stone_c);
    bx(&mut stone, 0.0, 0.0, rot_top - 0.6, rot_top, rh + 0.5, rh + 0.5, shade_c);
    bx(&mut far, 0.0, 0.0, h_top - 1.0, rot_top, rh, rh, stone_c);
    for [a, b] in [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]] {
        for k in -3i32..=3 {
            let kk = k as f64 * 6.0;
            bx(
                &mut stone,
                if a != 0.0 { a * (rh + 0.2) } else { kk },
                if b != 0.0 { b * (rh + 0.2) } else { kk },
                h_top,
                rot_top - 0.6,
                if a != 0.0 { 0.25 } else { 0.6 },
                if b != 0.0 { 0.25 } else { 0.6 },
                shade_c,
            );
        }
    }
    let c = w_at(0.0, 0.0, 0.0);
    let r0 = n(d, "drumRadiusM");
    let inner = r0 - 2.2;
    let columns = n(d, "columns");
    let pairs_n = columns / 2.0;
    stone.lathe(c[0], c[2], &[[inner, y(rot_top)], [inner, y(drum_top)]], 48, shade_c, None, None);
    stone.lathe(c[0], c[2], &[[r0 + 1.3, y(rot_top)], [r0 + 1.3, y(rot_top + 1.6)], [inner, y(rot_top + 1.6)]], 48, stone_c, None, None);
    for k in 0..columns as i64 {
        let a = (((k / 2) as f64 + 0.5) / pairs_n) * PI * 2.0 + if k % 2 != 0 { 0.045 } else { -0.045 };
        let (x, z) = (c[0] + a.cos() * r0, c[2] - a.sin() * r0);
        stone.lathe(x, z, &[[0.72, y(rot_top + 1.6)], [0.62, y(rot_top + 2.1)], [0.54, y(drum_top - 0.6)], [0.8, y(drum_top)], [0.0, y(drum_top) + 0.001]], 10, stone_c, None, None);
    }
    for k in 0..pairs_n as i64 {
        let a0 = ((k as f64 - 0.2) / pairs_n) * PI * 2.0;
        let a1 = ((k as f64 + 0.2) / pairs_n) * PI * 2.0;
        let r = inner + 0.05;
        let p = |a: f64, h: f64| [c[0] + a.cos() * r, y(h), c[2] - a.sin() * r];
        windows.quad(p(a0, rot_top + 5.0), p(a1, rot_top + 5.0), p(a1, drum_top - 4.0), p(a0, drum_top - 4.0), window);
    }
    let e_top = drum_top + ent_m;
    stone.lathe(c[0], c[2], &[[r0 + 1.5, y(drum_top)], [r0 + 1.5, y(e_top - 0.6)], [r0 + 2.0, y(e_top - 0.6)], [r0 + 2.0, y(e_top)], [inner, y(e_top)]], 48, stone_c, None, None);
    for k in 0..120 {
        let a = (k as f64 / 120.0) * PI * 2.0;
        stone.box_(c[0] + a.cos() * (r0 + 1.7), c[2] - a.sin() * (r0 + 1.7), y(e_top), y(e_top + 1.0), 0.12, 0.12, -a, stone_c, ALL);
    }
    dome.lathe(c[0], c[2], &[[r0 + 1.9, y(e_top + 1.0)], [r0 + 1.9, y(e_top + 1.2)], [r0 + 1.5, y(e_top + 1.2)]], 48, gold, None, None);
    far.lathe(c[0], c[2], &[[r0 + 1.5, y(rot_top)], [r0 + 1.5, y(e_top)]], 16, stone_c, None, None);
    let rd = n(d, "domeDiameterM") / 2.0;
    let (lantern_r, dome_rise) = (n(d, "lanternRadiusM"), n(d, "domeRiseM"));
    let mut profile: Vec<[f64; 2]> = Vec::new();
    for k in 0..=12 {
        let t = k as f64 / 12.0;
        let r = lantern_r.max(rd * ((t * PI) / 2.0).cos().powf(0.85));
        profile.push([r, y(e_top + dome_rise * ((t * PI) / 2.0).sin())]);
    }
    dome.lathe(c[0], c[2], &profile, 64, dome_c, None, None);
    let coarse: Vec<[f64; 2]> = profile.iter().enumerate().filter(|(k, _)| k % 3 == 0).map(|(_, p)| *p).collect();
    far.lathe(c[0], c[2], &coarse, 16, dome_c, None, None);
    let ribs = n(d, "domeRibs");
    let rib_w = n(d, "ribWidthM");
    for k in 0..ribs as i64 {
        let a = (k as f64 / ribs) * PI * 2.0;
        let at = |p: usize| [c[0] + a.cos() * (profile[p][0] + 0.15), profile[p][1], c[2] - a.sin() * (profile[p][0] + 0.15)];
        for p in 0..profile.len() - 1 {
            dome.beam(at(p), at(p + 1), rib_w, 0.35, gold);
        }
    }
    dome.lathe(c[0], c[2], &[[rd + 0.2, y(e_top + 1.3)], [rd + 0.2, y(e_top + 2.1)]], 64, gold, None, None);
    let lr = lantern_r;
    let l0 = e_top + dome_rise;
    let (lantern_top, lidar_top, dome_top) = (n(d, "lanternTopM"), n(d, "lidarTopM"), n(d, "domeTopM"));
    for k in 0..8 {
        let a = ((k as f64 + 0.5) / 8.0) * PI * 2.0;
        dome.lathe(c[0] + a.cos() * lr, c[2] - a.sin() * lr, &[[0.35, y(l0)], [0.3, y(lantern_top)], [0.0, y(lantern_top) + 0.001]], 8, gold, None, None);
    }
    dome.lathe(c[0], c[2], &[[lr - 1.0, y(l0)], [lr - 1.0, y(lantern_top)]], 24, dome_c, None, None);
    let cupola_colour = |p: usize, _mid: f64| if p == 0 || p == 2 { dome_c } else { gold };
    dome.lathe(
        c[0],
        c[2],
        &[[lr - 1.0, y(lantern_top)], [lr + 0.6, y(lantern_top)], [lr + 0.6, y(lantern_top + 0.8)], [lr * 0.6, y(lidar_top - 0.3)], [0.35, y(lidar_top)], [0.25, y(dome_top - 0.6)], [0.0, y(dome_top)]],
        24,
        gold,
        None,
        Some(&cupola_colour),
    );
    far.lathe(c[0], c[2], &[[lr, y(l0)], [lr, y(lantern_top)], [0.0, y(dome_top)]], 8, dome_c, None, None);
    // shadeColonnadeBays: the loggia's occlusion baked into the stone.
    let (h0, h1) = (h_base, h_col);
    let (foot, head) = (n(d, "loggiaShadeFoot"), n(d, "loggiaShadeHead"));
    let mut v_i = 0;
    while v_i < stone.positions.len() {
        let h = stone.positions[v_i + 1] - base;
        if !(h < h0 - 0.01 || h > h1 + 0.01) {
            let (dx, dz) = (stone.positions[v_i] - cx, stone.positions[v_i + 2] - cz);
            let a = dx * u[0] + dz * u[1];
            let b = dx * v[0] + dz * v[1];
            let inside = (hu - a.abs()).min(hv - b.abs());
            let t = ((inside - 0.3) / (loggia - 0.3)).max(0.0).min(1.0);
            let deep = t * t * (3.0 - 2.0 * t);
            if deep > 0.0 {
                let up = ((h - h0) / (h1 - h0)).max(0.0).min(1.0);
                let shade = foot + (head - foot) * up;
                let k = 1.0 - deep * (1.0 - shade);
                stone.colors[v_i] *= k;
                stone.colors[v_i + 1] *= k;
                stone.colors[v_i + 2] *= k;
            }
        }
        v_i += 3;
    }
    stone.shade_occlusion(None, n(d, "soffitShade"), 0.74, 1.5);
    Out {
        version: site["version"].as_str().unwrap_or("").into(),
        groups: vec![("stone".into(), stone.freeze()), ("dome".into(), dome.freeze()), ("windows".into(), windows.freeze())],
        far: far.freeze(),
        colliders: vec![],
        stats: serde_json::json!({ "baseY": base, "corniceY": y(h_top), "domeTopY": y(dome_top), "frame": { "cx": cx, "cz": cz, "longM": 2.0 * hu, "shortM": 2.0 * hv } }),
    }
}

/// The generator of a registered landmark building, by id.
pub fn build(id: &str, site: &Value, g: &GroundGrid) -> Option<Out> {
    build_opened(id, site, g, false)
}

/// `build`; `open`: the bands the landmark-access decks replace are left out
/// (Coit's belvedere, Salesforce's top-floor glass, Transamerica's top level).
pub fn build_opened(id: &str, site: &Value, g: &GroundGrid, open: bool) -> Option<Out> {
    Some(match id {
        "transamerica-pyramid" => transamerica_opened(site, g, open),
        "salesforce-tower" => salesforce_opened(site, g, open),
        "coit-tower" => coit_opened(site, g, open),
        "sutro-tower" => sutro(site, g),
        "city-hall" => city_hall(site, g),
        _ => return None,
    })
}

/// the ids of the landmark buildings this port builds.
pub const BUILDING_IDS: [&str; 5] = ["transamerica-pyramid", "salesforce-tower", "coit-tower", "sutro-tower", "city-hall"];
