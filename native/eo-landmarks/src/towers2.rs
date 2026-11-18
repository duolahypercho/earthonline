//! The landmark buildings' second generators: the JS export's five
//! (towers.rs, the previous output, `eo-app --no-towers2`) carried toward the
//! real buildings, on the same site records, dimensions and colours:
//!
//! | Building | What changes | Source |
//! | --- | --- | --- |
//! | Transamerica Pyramid | the base's columns as a row of triangles (a truss) round the four sides; the windows drawn as glazing that lights at night; the aluminium spire lit from within at night; an aviation light at the apex | Wikipedia, Transamerica Pyramid: 3,678 windows; 'the top of the pyramid is covered with aluminum panels'; 'a glass pyramid cap at the top encloses a red aircraft warning light'; 'the aluminum cap is indirectly illuminated from within'. The base's triangular columns: the task brief's description of the building (no published member count: the export's nine bays a side kept) |
//! | Salesforce Tower | the crown's sides curve in to its top (a rounded shoulder) instead of the export's straight taper; the curtain wall as pearlescent vertical fins and a sunshade per floor over clear glass (one shader facade, not the export's per-floor bands); the crown an open screen over the light sculpture, lit at night; lit floors; an aviation light at the top | Pelli Clarke Pelli (architectmagazine.com project gallery): 'clear glass with pearlescent metal accents', 'horizontal and vertical accents gradually taper in depth to accentuate the curved glass corners', 'the walls rise past the top floor to form a transparent crown'; Wikipedia: 'white aluminum fins and perforated sunshades', the crown's nine-storey light sculpture (11,000 LEDs on the top six floors) |
//! | Coit Tower | floodlit at night | Wikipedia, Coit Tower (lit at night); illuminance: IES floodlighting recommendation for a light surface in dark surroundings (skyline-buildings.json `floodlight`) |
//! | Sutro Tower | the legs tied only by deep horizontal crossbars at the tower's levels (no bracing between the legs), the top crossbar's arms reaching past the masts; aviation lights on the masts and the legs | levels: 180 ft (KGO's backup antenna at level 1), 380 and 560 ft (the auxiliary antennas between levels 3 and 4), Sutro Tower Inc., sutrotower.com 'Tower Technology'; 751 ft (level 6, Wikipedia); 200 m (the export's top ring frame). Crossbar depth and the top arms' reach INFERRED |
//! | City Hall | floodlit stone and dome at night; the windows light at night | floodlighting as Coit's; the export's windows |
//!
//! Every new group's material is the generator's (`Out::materials`): the
//! authored facade (surface kind 7, skyline.rs's styles) or the registry's
//! with a floodlight.

use crate::LandmarkLight;
use crate::data::{LandmarksJs, Material};
use crate::geometry::{ALL, GroundGrid, MeshArrays, V3, pairs, ring_centroid, scale_ring, signed_area};
use crate::skyline::facade_material;
use crate::towers::{Out, base_y, flat, hex, n, v2};
use serde_json::Value;

/// Floodlight illuminance, lux, on a light surface (IES Lighting Handbook,
/// floodlighting: 5 fc in dark surroundings; light stone and concrete need
/// the least of the table's values).
pub const FLOOD_DARK_LUX: f64 = 50.0;

/// The level heights of Sutro Tower's crossbars above its base, metres:
/// 180 ft, 380 ft, 560 ft (sutrotower.com), the export's 200 m ring frame and
/// its platform (751 ft, level 6).
pub const SUTRO_LEVELS_FT: [f64; 3] = [180.0, 380.0, 560.0];
/// INFERRED: a crossbar truss's depth, the top crossbar's, and how far the
/// top crossbar's arms reach past each mast.
pub const SUTRO_CROSSBAR_DEPTH_M: f64 = 3.6;
pub const SUTRO_TOP_DEPTH_M: f64 = 6.0;
pub const SUTRO_TOP_REACH_M: f64 = 6.5;
const FT: f64 = 0.3048;

fn with_flood(m: &Material, flood: [f64; 3]) -> Material {
    let mut m = m.clone();
    let mut o = m.surface_options.clone().unwrap_or_else(|| serde_json::json!({}));
    o["flood"] = serde_json::json!(flood);
    m.surface_options = Some(o);
    m
}

fn registry_material(id: &str, key: &str) -> Option<Material> {
    LandmarksJs::get().registry.get(id).and_then(|r| r.materials.get(key)).cloned()
}

/// Set the storey origin (the building's base, world y) of an authored-facade material.
fn with_origin(mut m: Material, y: f64) -> Material {
    if let Some(o) = m.surface_options.as_mut() {
        o["originY"] = serde_json::json!(y);
    }
    m
}

pub fn transamerica(site: &Value, g: &GroundGrid) -> Out {
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
    // The base: the columns of each side lean in pairs to meet under the
    // ring beam, a row of triangles (the export's nine bays a side), with a
    // column at each corner.
    let truss_top = top0 - 2.2;
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
        let pitch = 1.0 / 4.6;
        for k in -4i32..=4 {
            let t = k as f64 * pitch;
            let foot = edge(t, 0.0, 0.6);
            for dk in [-0.5, 0.5] {
                precast.beam_up(foot, edge(t + dk * pitch, truss_top, 0.6), 1.05, 1.25, precast_c, None);
            }
        }
        // the corner column (the next side's first corner is this side's last).
        precast.beam(edge(1.0, 0.0, 0.6), edge(1.0, truss_top, 0.6), 1.7, 1.7, precast_c);
        // the plinth band the feet stand on.
        precast.beam(edge(-1.0, 0.35, 0.3), edge(1.0, 0.35, 0.3), 0.7, 1.4, precast_c);
    }
    let mut levels = Vec::new();
    let mut y = top0;
    while y < spire_y - 0.01 {
        levels.push(y);
        y += floor_m;
    }
    levels.push(spire_y);
    for i in 0..levels.len() - 1 {
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
    let far_levels = [0.0, top0, 60.0, 120.0, spire_y];
    for i in 0..far_levels.len() - 1 {
        far.loft(&square(half(far_levels[i])), base + far_levels[i], &square(half(far_levels[i + 1])), base + far_levels[i + 1], precast_c, false);
    }
    let mut far_spire = MeshArrays::new("transamerica:far-spire");
    for i in 0..4 {
        far_spire.tri(sq[i], sq[(i + 1) % 4], apex, metal_c);
    }
    // the far item: its walls in the window band's facade, the spire merged in plain.
    let far = crate::set::merge("transamerica:far", &[&far.freeze(), &far_spire.freeze()]);
    // The spire's aluminium (the registry's roughness and metalness) lit from within at night.
    let metal_m = registry_material("transamerica-pyramid", "metal").map(|m| facade_material("solid", m.roughness, m.metalness, Some([FLOOD_DARK_LUX, base + spire_y, base + h_total])));
    let mut materials = vec![
        ("glass".to_string(), with_origin(facade_material("transamerica-window", 0.35, 0.0, None), base + top0)),
        ("far".to_string(), with_origin(facade_material("transamerica-far", 0.78, 0.0, None), base + top0)),
    ];
    if let Some(m) = metal_m {
        materials.push(("metal".into(), m));
    }
    Out {
        version: format!("{}+towers2", site["version"].as_str().unwrap_or("")),
        groups: vec![("precast".into(), precast.freeze()), ("glass".into(), glass.freeze()), ("metal".into(), metal.freeze())],
        far,
        colliders: vec![],
        materials,
        lights: vec![crate::lights::aviation([apex[0], apex[2]], apex[1])],
        stats: serde_json::json!({ "baseY": base, "apexY": base + h_total, "spireFromY": base + spire_y, "wingsFromY": base + wing_y0, "halfBaseM": fhalf }),
    }
}

/// Salesforce Tower's pearlescent white metal (INFERRED tone).
pub const SALESFORCE_PEARL_SRGB: u32 = 0xd9dcde;

/// The crown's curve: from the top floor's scale to the top's, the walls
/// rounding in to the top (a superellipse quarter, exponent `P`).
const SALESFORCE_CROWN_P: f64 = 2.4;

pub fn salesforce(site: &Value, g: &GroundGrid) -> Out {
    let d = &site["dimensions"];
    // The fins' and sunshades' 'pearlescent metal' (Pelli Clarke Pelli) and
    // the crown's white screen: neutral to cool whites, INFERRED (the
    // export's fin tone 0xd8d6ce reads warm beside a clear sky).
    let fin_c = crate::geometry::srgb_hex(SALESFORCE_PEARL_SRGB);
    let (crown_c, gap) = (hex(d, "crownSrgb"), hex(d, "crownGapSrgb"));
    let ring = pairs(&flat(&site["record"]["points"]));
    let [cx, cz] = ring_centroid(&ring);
    let base = base_y(&ring, g);
    let (h_total, top_floor) = (n(d, "heightM"), n(d, "topFloorM"));
    let (k_tf, k_top) = (n(d, "scaleAtTopFloor"), n(d, "scaleAtTop"));
    let crown_h = h_total - top_floor;
    let scale_at = |y: f64| {
        if y <= top_floor {
            1.0 + (k_tf - 1.0) * (y / top_floor)
        } else {
            let t = ((y - top_floor) / crown_h).clamp(0.0, 1.0);
            k_top + (k_tf - k_top) * (1.0 - t.powf(SALESFORCE_CROWN_P)).max(0.0).powf(1.0 / SALESFORCE_CROWN_P)
        }
    };
    let at = |y: f64| scale_ring(&ring, cx, cz, scale_at(y));
    let mut glass = MeshArrays::new("salesforce:glass");
    let mut crown = MeshArrays::new("salesforce:crown");
    let mut far = MeshArrays::new("salesforce:far");
    // The shaft tapers straight: a few lofts carry it, the facade's fins and
    // sunshades are the shader's (the vertex colour is the fins' pearl).
    let shaft = 8;
    for k in 0..shaft {
        let (y0, y1) = (top_floor * k as f64 / shaft as f64, top_floor * (k + 1) as f64 / shaft as f64);
        glass.loft(&at(y0), base + y0 - if k == 0 { 0.5 } else { 0.0 }, &at(y1), base + y1, fin_c, false);
    }
    // The crown, denser toward its rounded top.
    let bands = 18;
    let ty = |i: usize| top_floor + crown_h * ((i as f64 / bands as f64) * std::f64::consts::FRAC_PI_2).sin();
    for i in 0..bands {
        let (y0, y1) = (ty(i), ty(i + 1));
        crown.loft(&at(y0), base + y0, &at(y1), base + y1, fin_c, false);
    }
    // The crown is open at its top: a lip, the dark well inside, its floor 6 m down.
    let top = at(h_total);
    crown.loft(&top, base + h_total, &scale_ring(&top, cx, cz, 0.9), base + h_total, crown_c, false);
    crown.prism(&scale_ring(&top, cx, cz, 0.9), base + h_total - 6.0, base + h_total - 0.2, gap, true, false);
    for (y0, y1) in [(0.0, 100.0), (100.0, 200.0), (200.0, top_floor)] {
        far.loft(&at(y0), base + y0, &at(y1), base + y1, fin_c, false);
    }
    let fb = 6;
    for i in 0..fb {
        let (t0, t1) = (i as f64 / fb as f64, (i + 1) as f64 / fb as f64);
        let (y0, y1) = (top_floor + crown_h * (t0 * std::f64::consts::FRAC_PI_2).sin(), top_floor + crown_h * (t1 * std::f64::consts::FRAC_PI_2).sin());
        far.loft(&at(y0), base + y0, &at(y1), base + y1, crown_c, i == fb - 1);
    }
    let origin = base;
    Out {
        version: format!("{}+towers2", site["version"].as_str().unwrap_or("")),
        groups: vec![("glass".into(), glass.freeze()), ("crown".into(), crown.freeze())],
        far: far.freeze(),
        colliders: vec![],
        materials: vec![
            ("glass".into(), with_origin(facade_material("salesforce-curtain", 0.42, 0.25, None), origin)),
            ("crown".into(), with_origin(facade_material("salesforce-crown", 0.5, 0.2, None), base + top_floor)),
            ("far".into(), with_origin(facade_material("salesforce-curtain", 0.42, 0.25, None), origin)),
        ],
        lights: vec![crate::lights::aviation([cx, cz], base + h_total)],
        stats: serde_json::json!({ "baseY": base, "topY": base + h_total, "crownFromY": base + top_floor }),
    }
}

pub fn coit(site: &Value, g: &GroundGrid) -> Out {
    let mut o = crate::towers::coit(site, g);
    let base = o.stats["baseY"].as_f64().unwrap_or(0.0);
    let top = o.stats["topY"].as_f64().unwrap_or(base + 60.0);
    o.version = format!("{}+towers2", o.version);
    if let Some(m) = registry_material("coit-tower", "concrete") {
        o.materials.push(("concrete".into(), with_flood(&m, [FLOOD_DARK_LUX, base, top])));
    }
    o
}

pub fn city_hall(site: &Value, g: &GroundGrid) -> Out {
    let mut o = crate::towers::city_hall(site, g);
    let base = o.stats["baseY"].as_f64().unwrap_or(0.0);
    let top = o.stats["domeTopY"].as_f64().unwrap_or(base + 90.0);
    o.version = format!("{}+towers2", o.version);
    if let Some(m) = registry_material("city-hall", "stone") {
        o.materials.push(("stone".into(), with_flood(&m, [FLOOD_DARK_LUX, base, top])));
    }
    if let Some(m) = registry_material("city-hall", "dome") {
        o.materials.push(("dome".into(), with_flood(&m, [FLOOD_DARK_LUX, base, top])));
    }
    o.materials.push(("windows".into(), with_origin(facade_material("city-hall-window", 0.3, 0.1, None), base)));
    o
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
    // A crossbar: a horizontal truss between two legs, `depth` deep, its
    // chords carried `reach` past each leg (the top crossbar's arms).
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
    let mut lights: Vec<LandmarkLight> = Vec::new();
    for leg in 0..3 {
        let start = feet[leg].1 + foot_top - base_y;
        lattice(&mut steel, leg, start, platform, leg_w, panel);
        lattice(&mut steel, leg, platform, height - 4.0, mast_w, panel * 0.75);
        let top = leg_point(leg, height - 22.0);
        steel.box_(top[0], top[2], top[1], base_y + height, 0.9, 0.9, feet[leg].2, band_colour(height - 1.0), ALL);
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
        // FAA AC 70/7460-1M (lights.rs): an L-864 at each mast's top and on each leg at level 4.
        let tip = leg_point(leg, height);
        lights.push(crate::lights::aviation([tip[0], tip[2]], tip[1]));
        let mid = leg_point(leg, SUTRO_LEVELS_FT[2] * FT);
        lights.push(crate::lights::aviation([mid[0], mid[2]], mid[1]));
    }
    // The crossbars at the tower's levels; the top one (level 6, the
    // platform) deeper, its arms past the masts. No bracing between the legs.
    let mut levels: Vec<f64> = SUTRO_LEVELS_FT.iter().map(|f| f * FT).collect();
    if let Some(top_frame) = flat(&d["ringFramesM"]).last() {
        levels.push(*top_frame);
    }
    for &h in &levels {
        for k in 0..3 {
            crossbar(&mut steel, k, (k + 1) % 3, h, SUTRO_CROSSBAR_DEPTH_M, 0.0, 0.7);
            let colour = band_colour(h);
            far.beam(leg_point(k, h), leg_point((k + 1) % 3, h), 1.2, SUTRO_CROSSBAR_DEPTH_M * 0.6, colour);
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
    // The masts' upper frames (as the export's).
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
    Out {
        version: format!("{}+towers2", site["version"].as_str().unwrap_or("")),
        groups: vec![("steel".into(), steel.freeze()), ("concrete".into(), concrete.freeze())],
        far: far.freeze(),
        colliders,
        materials: vec![],
        lights,
        stats: serde_json::json!({ "baseY": base_y, "topY": base_y + height, "levelsM": levels, "feet": feet.iter().map(|f| [f.0[0], f.0[1], f.1]).collect::<Vec<_>>() }),
    }
}

/// The second generator of a registered landmark building, by id.
pub fn build(id: &str, site: &Value, g: &GroundGrid) -> Option<Out> {
    Some(match id {
        "transamerica-pyramid" => transamerica(site, g),
        "salesforce-tower" => salesforce(site, g),
        "coit-tower" => coit(site, g),
        "sutro-tower" => sutro(site, g),
        "city-hall" => city_hall(site, g),
        _ => return None,
    })
}
