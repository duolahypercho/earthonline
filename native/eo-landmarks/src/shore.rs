//! The city's built shore edge: a port of src/citygen/shoreline.js
//! `buildSeawallGeometry` (the quay wall along the shoreline rings: seaward
//! face, cap, chamfer, apron, and its relief tiers: coping stones, drip
//! soffit, control-joint panels, tide ledge, block courses and tide band),
//! with the beach rule of src/citygen/renderer.js (`nearBeach`: no wall where
//! the shore is a mapped beach, `beachWallReachM` of
//! src/render/materials/beach-surface.js), and the Ferry Plaza pier deck of
//! src/realmap/hero-waterfront-structures.js (`createFerryCard04PierDeck`,
//! `applyOracleSeawardPierFascia`, `createFerryCard04PierSheds`).
//!
//! The JS builds the wall only inside its boot window (bounds + 160 m) on
//! the window's oracle; here it is built along every ring edge of the
//! citywide oracle, split into plan chunks, and the JS's relief-tier rule
//! (the run length against `SEAWALL_RELIEF_TRIANGLE_BUDGET`) is applied per
//! chunk. A ring edge whose seaward side the elevation grid shows as dry land
//! (the county line: outside the rings, above the sea) is not a shore and
//! gets no wall.

use serde::Deserialize;
use serde_json::Value;

/// shoreline.js's seawall constants, read from its source by the export.
#[derive(Clone, Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct SeawallConsts {
    pub COPING_STONE_M: f64,
    pub COPING_PROUD_M: f64,
    pub JOINT_GAP_M: f64,
    pub CONTROL_JOINT_M: f64,
    pub CONTROL_GAP_M: f64,
    pub PANEL_PROUD_M: f64,
    pub BLOCK_L_M: f64,
    pub BLOCK_H_M: f64,
    pub BLOCK_COURSES: f64,
    pub BLOCK_MARGIN_M: f64,
    pub BLOCK_DRAFT_M: f64,
    pub BLOCK_JOINT_M: f64,
    pub BAND_H_M: f64,
    pub BAND_RECESS_M: f64,
    pub DRIP_RISE_M: f64,
    pub LEDGE_ABOVE_SEA_M: f64,
    pub LEDGE_H_M: f64,
    pub LEDGE_D_M: f64,
    pub LEDGE_ROOM_M: f64,
    pub SEAWALL_RELIEF_TRIANGLE_BUDGET: f64,
    pub RELIEF_TRIS_PER_M_BLOCKS: f64,
    pub RELIEF_TRIS_PER_M_COPING: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Blocks,
    Coping,
    Base,
}

/// What `buildSeawallGeometry` returns: positions, uvs, indices (quads of four
/// unique vertices), edge count, run length and relief tier.
#[derive(Clone, Debug, Default)]
pub struct SeawallMesh {
    pub positions: Vec<[f64; 3]>,
    pub uvs: Vec<[f64; 2]>,
    pub indices: Vec<u32>,
    pub edge_count: usize,
    pub run_length: f64,
    pub tier: Option<Tier>,
}

impl SeawallMesh {
    fn quad(&mut self, c: [[f64; 3]; 4], uv: [f64; 8]) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&c);
        self.uvs.extend_from_slice(&[[uv[0], uv[1]], [uv[2], uv[3]], [uv[4], uv[5]], [uv[6], uv[7]]]);
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// `buildSeawallGeometry(oracle, box, topAt, bottomY, copingLand, copingSea, skipEdge)`
/// over the `edges` already chosen by the caller (`forEachShoreEdge(box)`),
/// with `force_tier` replacing the run-length rule when given.
#[allow(clippy::too_many_arguments)]
pub fn build_seawall(
    k: &SeawallConsts,
    sea_level_y: f64,
    edges: &[[f64; 4]],
    is_water: &dyn Fn(f64, f64) -> bool,
    top_at: &dyn Fn(f64, f64) -> f64,
    bottom_y: f64,
    coping_land: f64,
    coping_sea: f64,
    skip_edge: Option<&dyn Fn(f64, f64, f64, f64) -> bool>,
    force_tier: Option<Tier>,
) -> Option<SeawallMesh> {
    build_seawall_with(k, sea_level_y, edges, is_water, top_at, bottom_y, coping_land, coping_sea, skip_edge, force_tier, true)
}

/// `build_seawall` without its landward face, tide band and
/// block courses (`landward` false): the quay apron (waterfront.rs) stands on
/// the wall's crest over that side, so nothing can see it.
#[allow(clippy::too_many_arguments)]
pub fn build_seawall_with(
    k: &SeawallConsts,
    sea_level_y: f64,
    edges: &[[f64; 4]],
    is_water: &dyn Fn(f64, f64) -> bool,
    top_at: &dyn Fn(f64, f64) -> f64,
    bottom_y: f64,
    coping_land: f64,
    coping_sea: f64,
    skip_edge: Option<&dyn Fn(f64, f64, f64, f64) -> bool>,
    force_tier: Option<Tier>,
    landward: bool,
) -> Option<SeawallMesh> {
    let keep: Vec<[f64; 4]> = edges.iter().copied().filter(|e| skip_edge.is_none_or(|f| !f(e[0], e[1], e[2], e[3]))).collect();
    let run_length: f64 = keep.iter().map(|e| (e[2] - e[0]).hypot(e[3] - e[1])).sum();
    let (with_blocks, with_coping) = match force_tier {
        Some(Tier::Blocks) => (true, true),
        Some(Tier::Coping) => (false, true),
        Some(Tier::Base) => (false, false),
        None => {
            let b = run_length * k.RELIEF_TRIS_PER_M_BLOCKS <= k.SEAWALL_RELIEF_TRIANGLE_BUDGET;
            (b, b || run_length * k.RELIEF_TRIS_PER_M_COPING <= k.SEAWALL_RELIEF_TRIANGLE_BUDGET)
        }
    };
    let mut m = SeawallMesh { run_length, ..Default::default() };
    for &[ax, az, bx, bz] in &keep {
        let length = (bx - ax).hypot(bz - az);
        if !(length > 0.05) {
            continue;
        }
        let top_a = top_at(ax, az);
        let top_b = top_at(bx, bz);
        let u1 = length / 4.0;
        let ux = (bx - ax) / length;
        let uz = (bz - az) / length;
        let (mut nx, mut nz) = (-uz, ux);
        let (mx, mz) = ((ax + bx) / 2.0, (az + bz) / 2.0);
        let mut water_side = 0;
        for d in [0.6, 1.4, 2.6] {
            if is_water(mx + nx * d, mz + nz * d) {
                water_side += 1;
            }
        }
        if water_side >= 2 {
            nx = -nx;
            nz = -nz;
        }
        let at = |s: f64, o: f64, y: f64| [ax + ux * s + nx * o, y, az + uz * s + nz * o];
        let crest_at = |s: f64| top_a + ((top_b - top_a) * s) / length;
        let low_crest = top_a.min(top_b);
        let v_of = |y: f64| ((y - bottom_y) / (0.1f64).max(low_crest - bottom_y)).max(0.0).min(1.0);
        // The round-12 wall: seaward face, cap, chamfer, apron.
        m.quad([at(0.0, 0.0, top_a), at(length, 0.0, top_b), at(length, 0.0, bottom_y), at(0.0, 0.0, bottom_y)], [0.0, 1.0, u1, 1.0, u1, 0.0, 0.0, 0.0]);
        m.quad([at(0.0, 0.0, top_a), at(length, 0.0, top_b), at(length, coping_land, top_b), at(0.0, coping_land, top_a)], [0.0, 0.0, u1, 0.0, u1, 0.12, 0.0, 0.12]);
        let bevel = coping_sea.max(0.05);
        m.quad([at(0.0, -bevel, top_a - bevel), at(length, -bevel, top_b - bevel), at(length, 0.0, top_b), at(0.0, 0.0, top_a)], [0.0, 0.94, u1, 0.94, u1, 1.0, 0.0, 1.0]);
        let overrun = (0.45f64).min(length * 0.45);
        let apron_w = 0.6;
        m.quad(
            [at(-overrun, 0.0, sea_level_y - 0.07), at(length + overrun, 0.0, sea_level_y - 0.07), at(length + overrun, -apron_w, sea_level_y - 0.30), at(-overrun, -apron_w, sea_level_y - 0.30)],
            [0.0, 0.05, u1, 0.05, u1, 0.0, 0.0, 0.0],
        );
        // Relief: the landward face.
        let land_face = if with_blocks { coping_land - k.BLOCK_DRAFT_M } else { coping_land };
        if !landward {
            // under the quay apron.
        } else if with_blocks {
            let band_top = k.BLOCK_COURSES * k.BLOCK_H_M + k.BLOCK_JOINT_M;
            let band_bottom = band_top + k.BAND_H_M;
            let band_face = land_face - k.BAND_RECESS_M;
            let vt = v_of(low_crest - band_top);
            let vb = v_of(low_crest - band_bottom);
            m.quad([at(0.0, land_face, top_a), at(length, land_face, top_b), at(length, land_face, top_b - band_top), at(0.0, land_face, top_a - band_top)], [0.0, 1.0, u1, 1.0, u1, vt, 0.0, vt]);
            m.quad([at(0.0, land_face, top_a), at(length, land_face, top_b), at(length, coping_land, top_b), at(0.0, coping_land, top_a)], [0.0, 0.99, u1, 0.99, u1, 1.0, 0.0, 1.0]);
            m.quad(
                [at(0.0, land_face, top_a - band_top), at(length, land_face, top_b - band_top), at(length, band_face, top_b - band_bottom), at(0.0, band_face, top_a - band_bottom)],
                [0.0, vt, u1, vt, u1, vb, 0.0, vb],
            );
            m.quad(
                [at(0.0, band_face, top_a - band_bottom), at(length, band_face, top_b - band_bottom), at(length, land_face, top_b - band_bottom), at(0.0, land_face, top_a - band_bottom)],
                [0.0, vb, u1, vb, u1, vb, 0.0, vb],
            );
            m.quad([at(0.0, land_face, top_a - band_bottom), at(length, land_face, top_b - band_bottom), at(length, land_face, bottom_y), at(0.0, land_face, bottom_y)], [0.0, vb, u1, vb, u1, 0.0, 0.0, 0.0]);
        } else {
            m.quad([at(0.0, land_face, top_a), at(length, land_face, top_b), at(length, land_face, bottom_y), at(0.0, land_face, bottom_y)], [0.0, 1.0, u1, 1.0, u1, 0.0, 0.0, 0.0]);
        }
        // Drip soffit under the chamfer's overhang.
        m.quad(
            [at(0.0, -bevel, top_a - bevel), at(length, -bevel, top_b - bevel), at(length, 0.0, top_b - bevel + k.DRIP_RISE_M), at(0.0, 0.0, top_a - bevel + k.DRIP_RISE_M)],
            [0.0, 0.90, u1, 0.90, u1, 0.92, 0.0, 0.92],
        );
        // Coping stones.
        let stones = if with_coping { ((length / k.COPING_STONE_M).round() as i64).max(1) } else { 0 };
        let stone_pitch = length / (stones.max(1) as f64);
        let half = k.JOINT_GAP_M / 2.0;
        let e = k.COPING_PROUD_M;
        let ch = e * std::f64::consts::FRAC_1_SQRT_2;
        for i in 0..stones {
            let s0 = i as f64 * stone_pitch + half;
            let s1 = (i + 1) as f64 * stone_pitch - half;
            if s1 - s0 < 0.05 {
                continue;
            }
            let (y0, y1) = (crest_at(s0), crest_at(s1));
            let (v0, v1) = (s0 / 4.0, s1 / 4.0);
            m.quad([at(s0, 0.0, y0 + e), at(s1, 0.0, y1 + e), at(s1, coping_land, y1 + e), at(s0, coping_land, y0 + e)], [v0, 0.0, v1, 0.0, v1, 0.12, v0, 0.12]);
            m.quad([at(s0, coping_land, y0), at(s1, coping_land, y1), at(s1, coping_land, y1 + e), at(s0, coping_land, y0 + e)], [v0, 0.12, v1, 0.12, v1, 0.13, v0, 0.13]);
            m.quad([at(s0, 0.0, y0), at(s0, coping_land, y0), at(s0, coping_land, y0 + e), at(s0, 0.0, y0 + e)], [v0, 0.0, v0, 0.12, v0, 0.13, v0, 0.01]);
            m.quad([at(s1, coping_land, y1), at(s1, 0.0, y1), at(s1, 0.0, y1 + e), at(s1, coping_land, y1 + e)], [v1, 0.12, v1, 0.0, v1, 0.01, v1, 0.13]);
            m.quad([at(s0, -bevel - ch, y0 - bevel + ch), at(s1, -bevel - ch, y1 - bevel + ch), at(s1, 0.0, y1 + e), at(s0, 0.0, y0 + e)], [v0, 0.94, v1, 0.94, v1, 1.0, v0, 1.0]);
            m.quad([at(s0, -bevel, y0 - bevel), at(s0, 0.0, y0), at(s0, 0.0, y0 + e), at(s0, -bevel - ch, y0 - bevel + ch)], [v0, 0.94, v0, 1.0, v0, 1.0, v0, 0.94]);
            m.quad([at(s1, 0.0, y1), at(s1, -bevel, y1 - bevel), at(s1, -bevel - ch, y1 - bevel + ch), at(s1, 0.0, y1 + e)], [v1, 1.0, v1, 0.94, v1, 0.94, v1, 1.0]);
        }
        // Seaward control-joint panels.
        let panels = ((length / k.CONTROL_JOINT_M).round() as i64).max(1);
        let panel_pitch = length / panels as f64;
        let panel_half = k.CONTROL_GAP_M / 2.0;
        let panel_bottom = sea_level_y - 0.35;
        let p = k.PANEL_PROUD_M;
        for i in 0..panels {
            let s0 = i as f64 * panel_pitch + panel_half;
            let s1 = (i + 1) as f64 * panel_pitch - panel_half;
            if s1 - s0 < 0.1 {
                continue;
            }
            let t0 = crest_at(s0) - bevel + k.DRIP_RISE_M - 0.01;
            let t1 = crest_at(s1) - bevel + k.DRIP_RISE_M - 0.01;
            if t0.min(t1) <= panel_bottom + 0.05 {
                continue;
            }
            let (v0, v1) = (s0 / 4.0, s1 / 4.0);
            m.quad([at(s0, -p, t0), at(s1, -p, t1), at(s1, -p, panel_bottom), at(s0, -p, panel_bottom)], [v0, v_of(t0), v1, v_of(t1), v1, v_of(panel_bottom), v0, v_of(panel_bottom)]);
            m.quad([at(s0, 0.0, t0), at(s0, -p, t0), at(s0, -p, panel_bottom), at(s0, 0.0, panel_bottom)], [v0, v_of(t0), v0, v_of(t0), v0, v_of(panel_bottom), v0, v_of(panel_bottom)]);
            m.quad([at(s1, -p, t1), at(s1, 0.0, t1), at(s1, 0.0, panel_bottom), at(s1, -p, panel_bottom)], [v1, v_of(t1), v1, v_of(t1), v1, v_of(panel_bottom), v1, v_of(panel_bottom)]);
        }
        // Tide ledge, only where the wall stands clear of it.
        let ledge_base = sea_level_y + k.LEDGE_ABOVE_SEA_M;
        let ledge_top = ledge_base + k.LEDGE_H_M;
        if low_crest - bevel >= ledge_top + k.LEDGE_ROOM_M {
            let nose = -(p + k.LEDGE_D_M);
            let slope_top = ledge_top + k.LEDGE_D_M * 0.5;
            m.quad([at(0.0, -p, slope_top), at(length, -p, slope_top), at(length, nose, ledge_top), at(0.0, nose, ledge_top)], [0.0, v_of(slope_top), u1, v_of(slope_top), u1, v_of(ledge_top), 0.0, v_of(ledge_top)]);
            m.quad([at(0.0, nose, ledge_top), at(length, nose, ledge_top), at(length, nose, ledge_base), at(0.0, nose, ledge_base)], [0.0, v_of(ledge_top), u1, v_of(ledge_top), u1, v_of(ledge_base), 0.0, v_of(ledge_base)]);
            m.quad([at(0.0, nose, ledge_base), at(length, nose, ledge_base), at(length, -p, ledge_base), at(0.0, -p, ledge_base)], [0.0, v_of(ledge_base), u1, v_of(ledge_base), u1, v_of(ledge_base), 0.0, v_of(ledge_base)]);
        }
        // Landward block courses in running bond (drafted-margin ashlar).
        let of = coping_land;
        let mg = k.BLOCK_MARGIN_M;
        let joint_half = k.BLOCK_JOINT_M / 2.0;
        let courses = if with_blocks && landward { k.BLOCK_COURSES as i64 } else { 0 }; // landward
        for c in 0..courses {
            let offset = if c % 2 == 1 { k.BLOCK_L_M / 2.0 } else { 0.0 };
            let rel_top = -k.BLOCK_JOINT_M - c as f64 * k.BLOCK_H_M;
            let rel_bottom = rel_top - k.BLOCK_H_M + k.BLOCK_JOINT_M;
            let mut s = -offset;
            while s < length {
                let s0 = s.max(0.0) + joint_half;
                let s1 = length.min(s + k.BLOCK_L_M) - joint_half;
                if s1 - s0 >= 3.0 * mg {
                    let (si0, si1) = (s0 + mg, s1 - mg);
                    let (yt0, yt1) = (crest_at(s0) + rel_top, crest_at(s1) + rel_top);
                    let (yb0, yb1) = (crest_at(s0) + rel_bottom, crest_at(s1) + rel_bottom);
                    let (yit0, yit1) = (crest_at(si0) + rel_top - mg, crest_at(si1) + rel_top - mg);
                    let (yib0, yib1) = (crest_at(si0) + rel_bottom + mg, crest_at(si1) + rel_bottom + mg);
                    let (v0, v1, vi0, vi1) = (s0 / 4.0, s1 / 4.0, si0 / 4.0, si1 / 4.0);
                    m.quad([at(si0, of, yit0), at(si1, of, yit1), at(si1, of, yib1), at(si0, of, yib0)], [vi0, v_of(yit0), vi1, v_of(yit1), vi1, v_of(yib1), vi0, v_of(yib0)]);
                    m.quad([at(s0, land_face, yt0), at(s1, land_face, yt1), at(si1, of, yit1), at(si0, of, yit0)], [v0, v_of(yt0), v1, v_of(yt1), vi1, v_of(yit1), vi0, v_of(yit0)]);
                    m.quad([at(si0, of, yib0), at(si1, of, yib1), at(s1, land_face, yb1), at(s0, land_face, yb0)], [vi0, v_of(yib0), vi1, v_of(yib1), v1, v_of(yb1), v0, v_of(yb0)]);
                    m.quad([at(s0, land_face, yt0), at(si0, of, yit0), at(si0, of, yib0), at(s0, land_face, yb0)], [v0, v_of(yt0), vi0, v_of(yit0), vi0, v_of(yib0), v0, v_of(yb0)]);
                    m.quad([at(si1, of, yit1), at(s1, land_face, yt1), at(s1, land_face, yb1), at(si1, of, yib1)], [vi1, v_of(yit1), v1, v_of(yt1), v1, v_of(yb1), vi1, v_of(yib1)]);
                }
                s += k.BLOCK_L_M;
            }
        }
        m.edge_count += 1;
    }
    if m.edge_count == 0 {
        return None;
    }
    m.tier = Some(if with_blocks {
        Tier::Blocks
    } else if with_coping {
        Tier::Coping
    } else {
        Tier::Base
    });
    Some(m)
}

/// The wall's far draw (a LOD the JS does not have: its wall is drawn whole
/// inside its window): the seaward face and the cap of `buildSeawallGeometry`
/// alone, two quads an edge.
pub fn far_wall(edges: &[[f64; 4]], is_water: &dyn Fn(f64, f64) -> bool, top_at: &dyn Fn(f64, f64) -> f64, bottom_y: f64, coping_land: f64) -> SeawallMesh {
    let mut m = SeawallMesh::default();
    for &[ax, az, bx, bz] in edges {
        let length = (bx - ax).hypot(bz - az);
        if !(length > 0.05) {
            continue;
        }
        let (ux, uz) = ((bx - ax) / length, (bz - az) / length);
        let (mut nx, mut nz) = (-uz, ux);
        let (mx, mz) = ((ax + bx) / 2.0, (az + bz) / 2.0);
        if [0.6, 1.4, 2.6].iter().filter(|d| is_water(mx + nx * **d, mz + nz * **d)).count() >= 2 {
            nx = -nx;
            nz = -nz;
        }
        let at = |s: f64, o: f64, y: f64| [ax + ux * s + nx * o, y, az + uz * s + nz * o];
        let (top_a, top_b) = (top_at(ax, az), top_at(bx, bz));
        let u1 = length / 4.0;
        m.quad([at(0.0, 0.0, top_a), at(length, 0.0, top_b), at(length, 0.0, bottom_y), at(0.0, 0.0, bottom_y)], [0.0, 1.0, u1, 1.0, u1, 0.0, 0.0, 0.0]);
        m.quad([at(0.0, 0.0, top_a), at(length, 0.0, top_b), at(length, coping_land, top_b), at(0.0, coping_land, top_a)], [0.0, 0.0, u1, 0.0, u1, 0.12, 0.0, 0.12]);
        m.edge_count += 1;
        m.run_length += length;
    }
    m.tier = Some(Tier::Base);
    m
}

/// A mapped beach (sf-lots-parks `greens` / `water` of kind beach) for the wall rule.
#[derive(Clone, Debug)]
pub struct BeachFeature {
    pub osm: String,
    pub polygon: Vec<[f64; 2]>,
}

/// sf-lots-parks.json.gz's beaches (greens and water of kind `beach`).
pub fn load_beaches(repo_root: &std::path::Path) -> anyhow::Result<Vec<BeachFeature>> {
    use std::io::Read;
    let bytes = std::fs::read(repo_root.join("public/data/sf/sf-lots-parks.json.gz"))?;
    let mut s = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut s)?;
    let v: Value = serde_json::from_str(&s)?;
    let mut out = Vec::new();
    for key in ["greens", "water"] {
        for g in v[key].as_array().into_iter().flatten() {
            if !g["kind"].as_str().is_some_and(|k| k.eq_ignore_ascii_case("beach")) {
                continue;
            }
            let gs = eo_data::groundscale::scale();
            let poly: Vec<[f64; 2]> = g["polygon"].as_array().map(|a| a.chunks_exact(2).map(|c| [c[0].as_f64().unwrap_or(0.0) * gs, c[1].as_f64().unwrap_or(0.0) * gs]).collect()).unwrap_or_default();
            if poly.len() >= 3 {
                out.push(BeachFeature { osm: g["osm"].as_str().unwrap_or("").to_string(), polygon: poly });
            }
        }
    }
    Ok(out)
}

/// The JS beach rule (renderer.js `nearBeach`): inside a beach polygon, or
/// within its reach (`beachWallReachM`: the wide-foreshore records' reach,
/// else the fallback) of one of its edges.
pub struct BeachRule<'a> {
    pub beaches: &'a [BeachFeature],
    pub reach_m: f64,
    pub wide_reach_m: f64,
    pub wide_sources: &'a [String],
}

impl BeachRule<'_> {
    fn inside(&self, x: f64, z: f64) -> bool {
        for b in self.beaches {
            let poly = &b.polygon;
            let mut inside = false;
            let mut j = poly.len() - 1;
            for i in 0..poly.len() {
                let (a, c) = (poly[i], poly[j]);
                if (a[1] > z) != (c[1] > z) && x < ((c[0] - a[0]) * (z - a[1])) / (c[1] - a[1]) + a[0] {
                    inside = !inside;
                }
                j = i;
            }
            if inside {
                return true;
            }
        }
        false
    }
    pub fn near(&self, x: f64, z: f64) -> bool {
        if self.inside(x, z) {
            return true;
        }
        for b in self.beaches {
            let reach = if self.wide_sources.contains(&b.osm) { self.wide_reach_m } else { self.reach_m };
            let poly = &b.polygon;
            let mut j = poly.len() - 1;
            for i in 0..poly.len() {
                let (a, c) = (poly[j], poly[i]);
                let (dx, dz) = (c[0] - a[0], c[1] - a[1]);
                let len2 = dx * dx + dz * dz;
                let t = if len2 > 0.0 { (((x - a[0]) * dx + (z - a[1]) * dz) / len2).max(0.0).min(1.0) } else { 0.0 };
                if (x - (a[0] + dx * t)).hypot(z - (a[1] + dz * t)) < reach {
                    return true;
                }
                j = i;
            }
        }
        false
    }
    /// `skipBeachEdge`: the edge's midpoint near a beach.
    pub fn skip(&self, ax: f64, az: f64, bx: f64, bz: f64) -> bool {
        self.near((ax + bx) / 2.0, (az + bz) / 2.0)
    }
}

/// Plan cell the citywide wall is split into (one LOD group each), metres.
pub const WALL_CHUNK_M: f64 = 128.0;
/// The box the JS's relief-tier rule is applied over: about the JS boot
/// window's wall box (its bounds + 160 m on each side), metres.
pub const WALL_TIER_BOX_M: f64 = 2000.0;
/// A ring edge is a shore only where the ground 4 m out on its seaward side
/// is within this of the sea datum (or outside the elevation grid).
pub const SHORE_DRY_ABOVE_SEA_M: f64 = 1.0;

/// One chunk of the citywide wall: its near draw (the relief tier of its box)
/// and its far draw (the base tier), as frozen records with face normals.
#[derive(Clone, Debug)]
pub struct WallChunk {
    pub near: crate::Frozen,
    pub far: crate::Frozen,
    pub tier: Tier,
    pub run_m: f64,
}

/// Face normals and a flat colour for a seawall mesh (three's
/// `computeVertexNormals` on its quads of four unique vertices).
pub fn freeze_wall(m: &SeawallMesh, name: &str, colour: [f64; 3]) -> crate::Frozen {
    let mut normals = vec![[0f64; 3]; m.positions.len()];
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]);
        let (cb, ab) = ([c[0] - b[0], c[1] - b[1], c[2] - b[2]], [a[0] - b[0], a[1] - b[1], a[2] - b[2]]);
        let n = [cb[1] * ab[2] - cb[2] * ab[1], cb[2] * ab[0] - cb[0] * ab[2], cb[0] * ab[1] - cb[1] * ab[0]];
        for &i in t {
            for k in 0..3 {
                normals[i as usize][k] += n[k];
            }
        }
    }
    let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut f = crate::Frozen { name: name.into(), ..Default::default() };
    for (p, n) in m.positions.iter().zip(&normals) {
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        f.positions.extend(p.iter().map(|v| *v as f32));
        f.normals.extend(n.iter().map(|v| (*v / l) as f32));
        f.colors.extend(colour.iter().map(|v| *v as f32));
        min_y = min_y.min(p[1]);
        max_y = max_y.max(p[1]);
    }
    f.indices = m.indices.clone();
    f.min_y = min_y;
    f.max_y = max_y;
    f
}

/// The citywide quay wall: every shore edge of the citywide oracle that is
/// not a beach (the JS rule) and not a dry county line, in `WALL_CHUNK_M`
/// chunks, each at the relief tier the JS's run-length rule gives its
/// `WALL_TIER_BOX_M` box, with a base-tier far draw.
#[allow(clippy::too_many_arguments)]
pub fn city_walls(
    k: &SeawallConsts,
    sea_level_y: f64,
    oracle: &eo_water::shoreline::Oracle,
    ground: &(dyn Fn(f64, f64) -> Option<f64> + Sync),
    top_at: &(dyn Fn(f64, f64) -> f64 + Sync),
    bottom_y: f64,
    beaches: &BeachRule,
    colour: [f64; 3],
) -> Vec<WallChunk> {
    let mut kept: Vec<[f64; 4]> = Vec::new();
    oracle.for_each_edge(|ax, az, bx, bz| {
        let length = (bx - ax).hypot(bz - az);
        if !(length > 0.05) {
            return;
        }
        let (ux, uz) = ((bx - ax) / length, (bz - az) / length);
        let (mut nx, mut nz) = (-uz, ux);
        let (mx, mz) = ((ax + bx) / 2.0, (az + bz) / 2.0);
        let ws = [0.6, 1.4, 2.6].iter().filter(|d| oracle.is_water(mx + nx * **d, mz + nz * **d)).count();
        if ws < 2 {
            nx = -nx;
            nz = -nz;
        }
        if ground(mx + nx * 4.0, mz + nz * 4.0).is_some_and(|g| g > sea_level_y + SHORE_DRY_ABOVE_SEA_M) {
            return;
        }
        if beaches.skip(ax, az, bx, bz) {
            return;
        }
        kept.push([ax, az, bx, bz]);
    });
    walls_on_edges(k, sea_level_y, &|x, z| oracle.is_water(x, z), &kept, top_at, bottom_y, colour, true)
}

/// the citywide wall over a chosen edge list (`city_walls`'
/// chunking and tier rule), its near tier with or without the landward side.
#[allow(clippy::too_many_arguments)]
pub fn walls_on_edges(
    k: &SeawallConsts,
    sea_level_y: f64,
    is_water: &(dyn Fn(f64, f64) -> bool + Sync),
    kept: &[[f64; 4]],
    top_at: &(dyn Fn(f64, f64) -> f64 + Sync),
    bottom_y: f64,
    colour: [f64; 3],
    landward: bool,
) -> Vec<WallChunk> {
    use std::collections::BTreeMap;
    let cell = |e: &[f64; 4], size: f64| (((e[0] + e[2]) / 2.0 / size).floor() as i64, ((e[1] + e[3]) / 2.0 / size).floor() as i64);
    let mut box_run: BTreeMap<(i64, i64), f64> = BTreeMap::new();
    let mut chunks: BTreeMap<(i64, i64), Vec<[f64; 4]>> = BTreeMap::new();
    for e in kept {
        *box_run.entry(cell(e, WALL_TIER_BOX_M)).or_default() += (e[2] - e[0]).hypot(e[3] - e[1]);
        chunks.entry(cell(e, WALL_CHUNK_M)).or_default().push(*e);
    }
    let list: Vec<(&(i64, i64), &Vec<[f64; 4]>)> = chunks.iter().collect();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
    let per = list.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let hs: Vec<_> = list
            .chunks(per)
            .map(|part| {
                let box_run = &box_run;
                s.spawn(move || {
                    part.iter()
                        .filter_map(|(_, edges)| {
                            let run = box_run[&cell(&edges[0], WALL_TIER_BOX_M)];
                            let tier = if run * k.RELIEF_TRIS_PER_M_BLOCKS <= k.SEAWALL_RELIEF_TRIANGLE_BUDGET {
                                Tier::Blocks
                            } else if run * k.RELIEF_TRIS_PER_M_COPING <= k.SEAWALL_RELIEF_TRIANGLE_BUDGET {
                                Tier::Coping
                            } else {
                                Tier::Base
                            };
                            let near = build_seawall_with(k, sea_level_y, edges, is_water, top_at, bottom_y, 0.42, 0.35, None, Some(tier), landward)?;
                            let far = far_wall(edges, is_water, top_at, bottom_y, 0.42);
                            Some(WallChunk { near: freeze_wall(&near, "seawall", colour), far: freeze_wall(&far, "seawall:far", colour), tier, run_m: near.run_length })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().expect("seawall thread panicked")).collect()
    })
}

/// `createFerryCard04PierSheds`: presentation sheds on the outline
/// (`pickOutlineAnchors`: the six southernmost and six easternmost vertices,
/// inset toward the centroid, at most `renderBudget.meshes`, kept
/// `minSeparationM` apart), each a `widthM x heightM x depthM` box on the
/// deck turned to its outline edge. Returns (positions, indices).
pub fn ferry_pier_sheds(outline: &[[f64; 2]], spec: &Value, deck_lift: f64, elevation_at: &dyn Fn(f64, f64) -> f64) -> Option<(Vec<[f64; 3]>, Vec<u32>)> {
    let n = |k: &str| crate::data::num(spec, k);
    let (w, d, h) = (n("widthM"), n("depthM"), n("heightM"));
    let picked = ferry_shed_anchors(outline, spec);
    if picked.is_empty() {
        return None;
    }
    let mut m = crate::MeshArrays::new("sheds");
    for (p, yaw) in &picked {
        let deck_y = elevation_at(p[0], p[1]) + deck_lift;
        // three's rotation.y turns local +x toward -z: the box's local x is its width.
        m.box_(p[0], p[1], deck_y, deck_y + h, w / 2.0, d / 2.0, -yaw, [1.0; 3], crate::geometry::ALL);
    }
    let pos: Vec<[f64; 3]> = m.positions.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect();
    Some((pos, m.indices))
}

/// `pickOutlineAnchors` of `createFerryCard04PierSheds`: each shed's plan
/// point and its outline edge's heading (shared with waterfront.rs).
pub fn ferry_shed_anchors(outline: &[[f64; 2]], spec: &Value) -> Vec<([f64; 2], f64)> {
    let n = |k: &str| crate::data::num(spec, k);
    let (inset, sep) = (n("insetM"), n("minSeparationM"));
    let max = spec["renderBudget"]["meshes"].as_u64().unwrap_or(0) as usize;
    let c = crate::geometry::ring_centroid(outline);
    let mut south: Vec<usize> = (0..outline.len()).collect();
    south.sort_by(|a, b| outline[*a][1].total_cmp(&outline[*b][1]));
    let mut east: Vec<usize> = (0..outline.len()).collect();
    east.sort_by(|a, b| (-outline[*a][0]).total_cmp(&-outline[*b][0]));
    let mut picked: Vec<([f64; 2], f64)> = Vec::new();
    for i in south.iter().take(6).chain(east.iter().take(6)) {
        let p = outline[*i];
        let (dx, dz) = (c[0] - p[0], c[1] - p[1]);
        let l = dx.hypot(dz);
        let l = if l == 0.0 { 1.0 } else { l };
        let placed = [p[0] + dx / l * inset, p[1] + dz / l * inset];
        if picked.iter().any(|(q, _)| (q[0] - placed[0]).hypot(q[1] - placed[1]) < sep) {
            continue;
        }
        let next = outline[(*i + 1) % outline.len()];
        picked.push((placed, (next[0] - p[0]).atan2(next[1] - p[1])));
        if picked.len() >= max {
            break;
        }
    }
    picked
}

/// The Ferry Plaza pier deck (`createFerryCard04PierDeck` + `applyOracleSeawardPierFascia`):
/// the exact outline triangulated at `elevationAt + deckLiftM`, a skin of
/// `deckThicknessM`, the underside dropped to the sea where the oracle says water.
pub struct PierDeck {
    pub top: Vec<[f64; 3]>,
    pub bottom: Vec<[f64; 3]>,
    pub indices: Vec<u32>,
}

pub fn ferry_pier_deck(outline: &[[f64; 2]], deck_lift: f64, thickness: f64, sea_level_y: f64, elevation_at: &dyn Fn(f64, f64) -> f64, is_water: &dyn Fn(f64, f64) -> bool) -> Option<PierDeck> {
    let mut ear = earcut::Earcut::<f64>::new();
    let mut tri: Vec<u32> = Vec::new();
    ear.earcut(outline.iter().copied(), &[] as &[u32], &mut tri);
    if tri.is_empty() {
        return None;
    }
    let top: Vec<[f64; 3]> = outline.iter().map(|p| [p[0], elevation_at(p[0], p[1]) + deck_lift, p[1]]).collect();
    let bottom: Vec<[f64; 3]> = top.iter().map(|p| [p[0], if is_water(p[0], p[2]) { sea_level_y - 0.2 } else { p[1] - thickness }, p[2]]).collect();
    let n = top.len() as u32;
    let mut indices = tri;
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend_from_slice(&[i, j, n + i, j, n + j, n + i]);
    }
    Some(PierDeck { top, bottom, indices })
}
