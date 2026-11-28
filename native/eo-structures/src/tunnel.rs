//! Stage A7: tunnel bores and portals (built only with `--tunnels`).
//!
//! A built tunnel chain is swept like a deck (`sweep.rs`, `Draw::Tunnel`: the
//! profile from A1 is its roadway), and each of its stations is either
//! covered (the drawn ground over the whole section stands over the bore's
//! roof) or open. Covered stretches are the bore: the roadway with its
//! lane paint, a raised walkway behind a kerb each side, walls up to the
//! spring line and a vault over the crown, lined in cream tile, a linear
//! luminaire along the crown, lights along it (`StructureWorld::tunnel_lights`).
//! Open stretches (the approach cut between the street and the portal, and
//! whole underpasses the ground never covers) are the roadway and walkways
//! between retaining walls that stand a parapet over the ground beside them
//! and reach under it. Where the two meet a portal headwall closes the
//! hillside over the bore's mouth. The ground the open stretches cut through
//! is handed to the terrain and the collision heightfield as cut polygons
//! (`StructureWorld::terrain_cuts`).
//!
//! Collision (`mesh::collide`): the roadway walks (street), the walkways,
//! walls, vault and headwalls block (barrier); the luminaire does not.
//!
//! The sizes below are structure-only (no street standard), INFERRED as
//! marked; the walkway kerb is the street's own kerb face height
//! (`StreetOptions::curb_face_height`, from `curb-height-sf-standard`).

#[path = "island_lighting.rs"]
mod island_lighting;
use crate::IslandTunnelSection;

#[path = "presidio_lining.rs"]
mod presidio;

use crate::mesh::{self, Builder, P, V3, p};
use crate::section::Dims;
use crate::sweep::{Station, Sweep};
use crate::{BORE_CLEAR_M, Chain, StructureClass};

/// The raised walkway behind the kerb each side of a bore. INFERRED: the
/// Broadway Tunnel's narrow walkway behind a low curb (reference sheet
/// 5.10, first-hand photo notes; its width is not sourced, section 1.7).
pub(crate) const WALKWAY_M: f64 = 0.9;
/// Height of the bore's walls from the walkway to the vault's spring line.
/// INFERRED, structure-only.
pub(crate) const SPRING_M: f64 = 4.0;
/// Thickness of the lining and roof over the crown (what the cover test adds
/// to the clear height). INFERRED, structure-only.
pub(crate) const ROOF_M: f64 = 0.6;
/// Portal and retaining wall thickness. INFERRED, structure-only (the plan's
/// listed "portal wall thickness").
pub(crate) const WALL_M: f64 = 0.6;
/// How far a portal headwall reaches past the bore's walls either side.
/// INFERRED, structure-only.
pub(crate) const WING_M: f64 = 3.0;
/// Retaining walls and headwalls stand this far over the ground beside
/// them (a parapet). INFERRED, structure-only.
pub(crate) const PARAPET_M: f64 = 0.45;
/// Walls reach this far under the ground they stand in. INFERRED, a drawing
/// depth.
pub(crate) const EMBED_M: f64 = 1.0;
/// Lights along the crown of a bore, this far apart. INFERRED: the real
/// Broadway Tunnel has one continuous linear luminaire along the crown
/// (sheet 5.10); point lights at this pitch stand for it.
pub(crate) const LIGHT_PITCH_M: f64 = 8.0;
/// A light hangs this far under the crown (the luminaire's face).
pub(crate) const LIGHT_DROP_M: f64 = 0.15;
/// A covered or open stretch shorter than this joins its neighbours (a
/// tolerance on the terrain's 10 m cells).
pub(crate) const STRETCH_MIN_M: f64 = 12.0;
/// A mapped bore without bare-earth cover can pass under a plaza or a building.
/// Close the deep part of that cut with a roof. INFERRED (a drawing rule).
pub(crate) const OPEN_DEEP_M: f64 = 2.0;
/// The cut's floor lies this far under the roadway (the ground the open
/// stretches cut through is lowered to it). A drawing depth.
pub(crate) const CUT_BELOW_M: f64 = 1.0;
/// A bore that meets the street (a paved end) opens this far in from its
/// end at least, so its portal stands clear of the street: the drawn
/// ground slopes from the street held under it to the hillside over one
/// whole terrain cell (8 m x the ground-plan scale), which would otherwise
/// lie across the mouth. INFERRED (a drawing reach, over one 10 m cell).
pub(crate) const PORTAL_OPEN_M: f64 = 12.0;

/// A portal's arch (stage A9): a band this wide round each bore's opening,
/// standing this far proud of the headwall's face (a cast arch ring, as the
/// city's 1914-1952 portals have). INFERRED, structure-only (no portal
/// drawing is sourced; reference sheet 1.7).
pub(crate) const ARCH_W_M: f64 = 0.8;
pub(crate) const ARCH_PROUD_M: f64 = 0.3;

/// The bore's lining tint (sRGB): glossy small cream-yellow tiles over the
/// whole wall and vault (reference sheet 5.10, Broadway Tunnel interior,
/// first-hand photo notes; no measured reflectance is sourced). INFERRED.
pub(crate) const TILE_TINT: [u8; 3] = [222, 208, 168];
/// The luminaire strip (sRGB): a light grey housing.
pub(crate) const LUMINAIRE_RGB: [u8; 3] = [200, 200, 196];

/// The tunnel's own part of a sweep.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TunnelInfo {
    /// Per sweep station: covered (the bore) or open (a cut).
    pub covered: Vec<bool>,
    /// Per sweep station: the drawn ground just outside each wall (+n, -n).
    pub beside: Vec<[f64; 2]>,
    /// Per sweep station and side (+n, -n): the offset of the wall's inside
    /// face and of its back (positive, outward from the centreline). A side
    /// that meets a twin bore close by (`twin`) stops at the middle between
    /// the two (a centre wall each half of, no walkway or wing past it).
    pub wall: Vec<[f64; 2]>,
    pub back: Vec<[f64; 2]>,
    pub twin: Vec<[bool; 2]>,
    /// The walkway kerb's height (`StreetOptions::curb_face_height`).
    pub kerb: f64,
    pub island_section: Option<crate::IslandTunnelSection>,
    /// At the upper island approaches the slab and the back of both
    /// retaining walls end at the same bearing above the lower carriageway.
    pub upper_soffits: Option<Vec<f64>>,
}

/// Half the bore's inside width (roadway and walkways) at a station, with
/// no twin beside it.
pub(crate) fn inner(st: &Station) -> f64 {
    st.half + WALKWAY_M
}

/// Two tunnel chains this close to parallel (degrees) share their middle.
const TWIN_ALONG_DEG: f64 = 30.0;

/// What the tunnel's stations are: covered where the drawn ground over the
/// section (centre and both wall backs) stands over the roof, open
/// elsewhere; short stretches join their neighbours; the ends at a street
/// open; each side's walls, stopping halfway to a twin bore (`twins`: the
/// other drawn tunnel chains) running alongside closer than the two bores'
/// widths.
pub(crate) fn info(st: &[Station], streets: &eo_street::StreetWorld, kerb: f64, paved: [bool; 2], twins: &[&Chain], island: Option<(crate::island_section::Dimensions, crate::IslandTunnelSection)>) -> TunnelInfo {
    let island_section = island.map(|(_, shape)| shape);
    let upper_soffits = island.and_then(|(dims, shape)| dims.upper_soffits(shape, st, twins));
    let crown = island_section.map_or(BORE_CLEAR_M, |section| section.crown());
    let ground = |x, z| streets.carpet_y(x, z);
    let cos = TWIN_ALONG_DEG.to_radians().cos();
    let mut wall = vec![];
    let mut back = vec![];
    let mut twin = vec![];
    for s in st {
        let nominal = inner(s);
        let (mut w, mut bk, mut tw) = ([nominal; 2], [nominal + WALL_M; 2], [false; 2]);
        for d in twins {
            let (sd, dist) = d.project(s.x, s.z);
            if sd <= 0.0 || sd >= d.length() {
                continue;
            }
            let reach = d.half_at(sd) + WALKWAY_M + WALL_M + nominal + WALL_M;
            let f = d.frame(sd);
            if dist >= reach || (d.y_at(sd) - s.y).abs() >= SPRING_M || (f.tx * s.tx + f.tz * s.tz).abs() < cos {
                continue;
            }
            let k = if (f.x - s.x) * s.nx + (f.z - s.z) * s.nz >= 0.0 { 0 } else { 1 };
            let mid = dist / 2.0;
            if mid < bk[k] {
                bk[k] = mid.max(s.half + 0.05);
                w[k] = (mid - WALL_M / 2.0).clamp(s.half, nominal);
                tw[k] = true;
            }
        }
        wall.push(w);
        back.push(bk);
        twin.push(tw);
    }
    let mut covered: Vec<bool> = st
        .iter()
        .zip(&back)
        .map(|(s, bk)| {
            let g = [bk[0], 0.0, -bk[1]].iter().map(|&u| ground(s.x + s.ox * u, s.z + s.oz * u)).fold(f64::INFINITY, f64::min);
            g >= s.y + crown + ROOF_M
        })
        .collect();
    // runs shorter than STRETCH_MIN_M flip (covered gaps first, then open ones).
    for want in [false, true] {
        let n = covered.len();
        let mut i = 0;
        while i < n {
            let mut j = i;
            while j + 1 < n && covered[j + 1] == covered[i] {
                j += 1;
            }
            let len = st[j].s - st[i].s;
            let inside = i > 0 && j + 1 < n;
            if covered[i] == want && inside && len < STRETCH_MIN_M {
                for c in covered.iter_mut().take(j + 1).skip(i) {
                    *c = !want;
                }
            }
            i = j + 1;
        }
    }
    // the mouths at the street open at least PORTAL_OPEN_M.
    if let (Some(first), Some(last)) = (st.first().map(|s| s.s), st.last().map(|s| s.s)) {
        for (c, s) in covered.iter_mut().zip(st) {
            if (paved[0] && s.s - first < PORTAL_OPEN_M) || (paved[1] && last - s.s < PORTAL_OPEN_M) {
                *c = false;
            }
        }
    }
    let beside: Vec<[f64; 2]> = st
        .iter()
        .zip(&back)
        .map(|(s, bk)| {
            let (p, n) = (bk[0] + 0.5, bk[1] + 0.5);
            [ground(s.x + s.ox * p, s.z + s.oz * p), ground(s.x - s.ox * n, s.z - s.oz * n)]
        })
        .collect();
    if !covered.iter().any(|&c| c) {
        // Bare-earth elevation excludes buildings and elevated plazas. Reconstruct
        // cover over deep stations, leaving the street mouths open.
        for (i, s) in st.iter().enumerate() {
            let at_mouth = (paved[0] && s.s - st[0].s < PORTAL_OPEN_M)
                || (paved[1] && st.last().unwrap().s - s.s < PORTAL_OPEN_M);
            covered[i] = !at_mouth && beside[i][0].max(beside[i][1]) - s.y > OPEN_DEEP_M;
        }
    }
    // Bare earth can be open sky while an elevated surface road spans
    // the bore. A short underpass has room for a roof between its ends
    // even when both normal approach-cut setbacks overlap.
    if !covered.windows(2).any(|pair| pair[0] && pair[1]) {
        for i in 1..st.len().saturating_sub(1) {
            let s = &st[i];
            let roof = s.y + crown + ROOF_M;
            let overhead = [-s.half, 0.0, s.half].iter().all(|&u| {
                let p = s.at(u, roof);
                streets.carriageway_near(p[0], p[2], roof).is_some_and(|h| h >= roof)
            });
            covered[i] |= overhead;
        }
    }
    TunnelInfo { covered, beside, wall, back, twin, kerb, island_section, upper_soffits }
}

/// The road's height at offset `u` of a station (the deck top's points).
fn road(sw: &Sweep, st: &Station, u: f64) -> f64 {
    sw.top(st, u.clamp(-st.half, st.half))
}

/// The bore's crown offset (the middle between its walls) and half span.
fn crown_of(w: [f64; 2]) -> (f64, f64) {
    ((w[0] - w[1]) / 2.0, (w[0] + w[1]) / 2.0)
}

/// The bore's inside outline at a station (sweep index `i`), from the +n
/// walkway's back up the +n wall, over the vault, down the -n wall to the
/// -n walkway's back (so the faces it sweeps look into the bore).
fn bore_outline(sw: &Sweep, st: &Station, info: &TunnelInfo, i: usize, lod: u8, box_section: bool) -> Vec<P> {
    let w = info.wall[i];
    let (wy_p, wy_n) = (road(sw, st, st.half) + info.kerb, road(sw, st, -st.half) + info.kerb);
    let spring = st.y + info.island_section.map_or(SPRING_M, |section| section.height(1.0));
    let crown = st.y + info.island_section.map_or(BORE_CLEAR_M, |section| section.crown());
    if box_section || matches!(info.island_section, Some(crate::IslandTunnelSection::Flat { .. })) {
        return vec![p(w[0], wy_p, 0.3), p(w[0], crown, 0.15), p(-w[1], crown, 0.15), p(-w[1], wy_n, 0.3)];
    }
    let (uc, a) = crown_of(w);
    let segs = match (info.island_section.is_some(), lod) {
        (true, 0) => 12,
        (true, 1) => 10,
        (true, _) => 8,
        (false, 0) => 10,
        (false, 1) => 4,
        (false, _) => 2,
    };
    let mut out = vec![p(w[0], wy_p, 0.3), p(w[0], spring, 0.2)];
    for k in 1..segs {
        let t = std::f64::consts::PI * k as f64 / segs as f64;
        let y = info.island_section.map_or(spring + (crown - spring) * t.sin(), |section| st.y + section.height(t.cos()));
        out.push(p(uc + a * t.cos(), y, 0.15));
    }
    out.push(p(-w[1], spring, 0.2));
    out.push(p(-w[1], wy_n, 0.3));
    out
}

/// The walkway each side: the kerb face and the walkway top, ordered so the
/// faces look into the bore and up.
fn walkway(sw: &Sweep, st: &Station, info: &TunnelInfo, i: usize, side: f64) -> Vec<P> {
    let h = st.half;
    let y = road(sw, st, side * h);
    let k = info.kerb;
    if side > 0.0 {
        vec![p(h, y, 0.4), p(h, y + k, 0.3), p(info.wall[i][0], y + k, 0.3)]
    } else {
        vec![p(-info.wall[i][1], y + k, 0.3), p(-h, y + k, 0.3), p(-h, y, 0.4)]
    }
}

/// A retaining wall of the open cut on one side, as a prism outline: its
/// inside face from the walkway up to a parapet over the ground beside it,
/// its top, its back down under that ground.
fn cut_wall(sw: &Sweep, st: &Station, info: &TunnelInfo, i: usize, side: f64) -> Vec<P> {
    let k = if side > 0.0 { 0 } else { 1 };
    let (w, bk) = (info.wall[i][k], info.back[i][k]);
    let beside = info.beside[i][k];
    let wy = road(sw, st, side * st.half) + info.kerb;
    let mut top = beside.max(wy) + PARAPET_M;
    if let Some(shape @ crate::IslandTunnelSection::Flat { .. }) = info.island_section {
        // The outer hillside is retained by the upper level. Extending a
        // lower wall up to that hillside cuts through a westbound lane.
        top = top.min(shape.lining_top(st.y));
    }
    // (down past the cut's floor, where the ground behind it is lowered to.)
    let mut base = beside.min(wy).min(st.y - CUT_BELOW_M) - EMBED_M;
    if let Some(soffits) = &info.upper_soffits {
        base = base.max(soffits[i]);
    }
    if side > 0.0 {
        vec![p(w, wy, 0.4), p(w, top, 0.3), p(bk, top, 0.3), p(bk, base, 0.5)]
    } else {
        vec![p(-bk, base, 0.5), p(-bk, top, 0.3), p(-w, top, 0.3), p(-w, wy, 0.4)]
    }
}

/// A cut wall's coping (stage A9): a cap `walls::COPING_H_M` deep over its
/// top, `walls::COPING_OUT_M` proud of each face, as a prism outline.
fn coping(sw: &Sweep, st: &Station, info: &TunnelInfo, i: usize, side: f64) -> Vec<P> {
    let k = if side > 0.0 { 0 } else { 1 };
    let (w, bk) = (info.wall[i][k], info.back[i][k]);
    let wy = road(sw, st, side * st.half) + info.kerb;
    let top = info.beside[i][k].max(wy) + PARAPET_M;
    let (c, h) = (crate::walls::COPING_OUT_M, crate::walls::COPING_H_M);
    if side > 0.0 {
        vec![p(w - c, top - 0.02, 0.4), p(w - c, top + h, 0.35), p(bk + c, top + h, 0.35), p(bk + c, top - 0.02, 0.4)]
    } else {
        vec![p(-bk - c, top - 0.02, 0.4), p(-bk - c, top + h, 0.35), p(-w + c, top + h, 0.35), p(-w + c, top - 0.02, 0.4)]
    }
}

/// The luminaire under the crown: a small box, its faces looking out.
fn luminaire(st: &Station, info: &TunnelInfo, i: usize) -> Vec<P> {
    let crown = st.y + info.island_section.map_or(BORE_CLEAR_M, |section| section.crown());
    let (uc, _) = crown_of(info.wall[i]);
    let (hw, d) = (0.15, 0.12);
    vec![p(uc + hw, crown - 0.02, 0.0), p(uc + hw, crown - d, 0.0), p(uc - hw, crown - d, 0.0), p(uc - hw, crown - 0.02, 0.0)]
}

/// Runs of consecutive positions of `idx` whose stations are all `want`
/// (covered or open), as position ranges (each at least one step).
fn stretches(info: &TunnelInfo, idx: &[usize], want: bool) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut q = 0;
    while q + 1 < idx.len() {
        // a step belongs to the covered bore when both its stations are.
        let step = |q: usize| (info.covered[idx[q]] && info.covered[idx[q + 1]]) == want;
        if !step(q) {
            q += 1;
            continue;
        }
        let a = q;
        while q + 1 < idx.len() && step(q) {
            q += 1;
        }
        out.push((a, q));
    }
    out
}

/// One stretch of a tunnel chain at one level (`tiles.rs`, as `mesh::run`
/// for a deck): roadway, paint, walkways; the bore over covered steps, the
/// cut's walls over open ones; a portal headwall where a covered step meets
/// an open one.
/// Returns the index range of the lining and walkways, whose openings
/// follow the tunnel topology. Portal headwalls can span several levels.
pub(crate) fn run(b: &mut Builder, sw: &Sweep, c: &Chain, idx: &[usize], lod: u8, dims: &Dims) -> std::ops::Range<usize> {
    let Some(info) = sw.tunnel.as_ref() else { return 0..0 };
    let gutter = sw.cross[1];
    let bw = dims.barrier_base;
    // the roadway, one strip per piece's tint.
    let mut start = 0;
    while start + 1 < idx.len() {
        let pi = mesh::piece_index(c, (sw.st[idx[start]].s + sw.st[idx[start + 1]].s) / 2.0);
        let mut end = start + 1;
        while end + 1 < idx.len() && mesh::piece_index(c, (sw.st[idx[end]].s + sw.st[idx[end + 1]].s) / 2.0) == pi {
            end += 1;
        }
        mesh::strip(b, sw, &idx[start..=end], StructureClass::DeckSurface, sw.asphalt[pi], &|st| mesh::deck_top(sw, st, lod, gutter, bw));
        start = end;
    }
    if lod < 2 {
        mesh::paint(b, sw, c, idx, dims, lod);
    }
    let lining_start = b.m.indices.len();
    let at = |st: &Station| station_index(sw, st);
    for side in [1.0, -1.0] {
        mesh::strip(b, sw, idx, StructureClass::Concrete, sw.tint, &|st| walkway(sw, st, info, at(st), side));
    }
    // The upper deck continues over the lower road through both mouths.
    // Each traffic level reaches daylight at a different survey station.
    if let Some(soffits) = &info.upper_soffits {
        mesh::strip(b, sw, idx, StructureClass::Concrete, sw.tint, &|st| {
            let i = at(st);
            vec![p(info.back[i][0], soffits[i], 0.3), p(-info.back[i][1], soffits[i], 0.3)]
        });
    }
    let box_section = presidio::applies(c);
    let lining = if box_section { sw.tint } else { TILE_TINT };
    // the bore.
    for (a, z) in stretches(info, idx, true) {
        let part = &idx[a..=z];
        if let Some(shape) = island_lighting::applies(info) {
            let outline = |st: &Station| bore_outline(sw, st, info, at(st), lod, false);
            if matches!(shape, IslandTunnelSection::Vault { .. }) {
                // Lower wall facing stops at the spring of the concrete vault.
                mesh::strip(b, sw, part, StructureClass::Concrete, TILE_TINT, &|st| outline(st)[..2].to_vec());
                if island_lighting::smooth_vault() {
                    island_lighting::vault(b, sw, part, shape, &outline);
                } else {
                    mesh::strip(b, sw, part, StructureClass::Concrete, sw.tint, &|st| {
                        let points = outline(st);
                        points[1..points.len()-1].to_vec()
                    });
                }
                mesh::strip(b, sw, part, StructureClass::Concrete, TILE_TINT, &|st| {
                    let points = outline(st);
                    points[points.len()-2..].to_vec()
                });
            } else {
                mesh::strip(b, sw, part, StructureClass::Concrete, sw.tint, &outline);
            }
            if lod < 2 { island_lighting::strips(b, c, sw, part, info, shape); }
            continue;
        }
        mesh::strip(b, sw, part, StructureClass::Concrete, lining, &|st| bore_outline(sw, st, info, at(st), lod, box_section));
        if lod < 2 && !box_section {
            mesh::strip(b, sw, part, StructureClass::Metal, LUMINAIRE_RGB, &|st| luminaire(st, info, at(st)));
        }
    }
    if box_section && lod < 2 {
        presidio::ceiling_lights(b, c, sw, idx);
    }
    // the open cut's walls (stage A9: with a coping).
    for (a, z) in stretches(info, idx, false) {
        let part = &idx[a..=z];
        for side in [1.0, -1.0] {
            mesh::strip(b, sw, part, StructureClass::Concrete, sw.tint, &|st| cut_wall(sw, st, info, at(st), side));
            if lod < 2 && !matches!(info.island_section, Some(crate::IslandTunnelSection::Flat { .. })) {
                mesh::strip(b, sw, part, StructureClass::Concrete, sw.tint, &|st| coping(sw, st, info, at(st), side));
            }
        }
    }
    let lining = lining_start..b.m.indices.len();
    // portals: a step from an open station to a covered one (or back).
    for q in 0..idx.len().saturating_sub(1) {
        let (i, j) = (idx[q], idx[q + 1]);
        if info.covered[i] == info.covered[j] {
            continue;
        }
        let (k, open_dir) = if info.covered[j] { (j, -1.0) } else { (i, 1.0) };
        portal(b, sw, &sw.st[k], info, k, open_dir, lod, box_section);
    }
    lining
}

/// The sweep station index of a station value (the strip hands out
/// stations by value).
fn station_index(sw: &Sweep, st: &Station) -> usize {
    match sw.st.binary_search_by(|t| t.s.total_cmp(&st.s)) {
        Ok(i) => i,
        Err(i) => i.min(sw.st.len() - 1),
    }
}

/// A portal headwall at sweep station `i` (the bore's first covered
/// station), facing the open side (`open_dir`: +1 toward increasing
/// station): the face over and beside the bore's mouth up to a parapet over
/// the hillside, out to its wings (to the middle where a twin bore is
/// beside it), its top, and its two ends; its back is in the hill.
fn portal(b: &mut Builder, sw: &Sweep, st: &Station, info: &TunnelInfo, i: usize, open_dir: f64, lod: u8, box_section: bool) {
    let t = mesh::norm([st.tx, 0.0, st.tz]);
    let out: V3 = mesh::scale(t, open_dir);
    // (its top and ends reach back over the bore as a canopy, over the
    // ground the terrain leaves out there: `TerrainCut::canopy`.)
    let back: V3 = mesh::scale(t, -open_dir * PORTAL_OPEN_M);
    let wing = |k: usize| if info.twin[i][k] { info.back[i][k] } else { info.wall[i][k] + WING_M };
    let (wp, wn) = (wing(0), wing(1));
    let ring = bore_outline(sw, st, info, i, lod, box_section);
    // the hillside over the face: the highest ground beside and over it.
    let roof = st.y + info.island_section.map_or(BORE_CLEAR_M, |section| section.crown()) + ROOF_M;
    // The lower portal ends at the intermediate slab. The upper headwall
    // carries the hillside above the westbound road in this shared opening.
    let top = if matches!(info.island_section, Some(crate::IslandTunnelSection::Flat { .. })) {
        roof
    } else {
        info.beside[i][0].max(info.beside[i][1]).max(roof) + PARAPET_M
    };
    let base = |y: f64| {
        let foot = y.min(st.y - CUT_BELOW_M) - EMBED_M;
        info.upper_soffits.as_ref().map_or(foot, |soffits| foot.max(soffits[i]))
    };
    let rgb = sw.tint;
    let at = |u: f64, y: f64| st.at(u, y);
    let v = |b: &mut Builder, q: V3, n: V3, u: f64, y: f64| b.v(q, n, rgb, 0.35, [u, y - st.y], StructureClass::Concrete);
    // over the opening: a strip between its outline and the top.
    for pair in ring.windows(2) {
        let (a, c) = (pair[0], pair[1]);
        let q = [v(b, at(a.u, a.y), out, a.u, a.y), v(b, at(a.u, top), out, a.u, top), v(b, at(c.u, top), out, c.u, top), v(b, at(c.u, c.y), out, c.u, c.y)];
        b.quad(q, out);
    }
    // beside it, out to the wings, from under the ground to the top.
    let (first, last) = (ring[0], *ring.last().unwrap());
    for (edge, far) in [(first, wp), (last, -wn)] {
        let y0 = base(edge.y.min(st.y));
        let q = [v(b, at(edge.u, y0), out, edge.u, y0), v(b, at(edge.u, top), out, edge.u, top), v(b, at(far, top), out, far, top), v(b, at(far, y0), out, far, y0)];
        b.quad(q, out);
    }
    // the top, back into the hill.
    let up = [0.0, 1.0, 0.0];
    let tq = |u: f64| mesh::add(at(u, top), back);
    let q = [v(b, at(wp, top), up, wp, top), v(b, at(-wn, top), up, -wn, top), v(b, tq(-wn), up, -wn, top), v(b, tq(wp), up, wp, top)];
    b.quad(q, up);
    // its two ends (stage A9: also where it meets a twin's at the middle,
    // whose portal may stand further in or out: the step between the two
    // headwalls is closed; where they stand together it is hidden between them).
    for (sgn, u) in [(1.0, wp), (-1.0, -wn)] {
        let n = mesh::norm([st.nx * sgn, 0.0, st.nz * sgn]);
        let y0 = base(st.y);
        let q = [v(b, at(u, y0), n, u, y0), v(b, at(u, top), n, u, top), v(b, mesh::add(at(u, top), back), n, u, top), v(b, mesh::add(at(u, y0), back), n, u, y0)];
        b.quad(q, n);
    }
    // stage A9: the bore's own arch at its mouth (each of twin bores its
    // own, the centre wall kept): a band of `ARCH_W_M` round the opening
    // from the walkways up the jambs and over the vault, standing
    // `ARCH_PROUD_M` proud of the face, and its coping along the top.
    if lod < 2 {
        let arch = |k: usize| if info.twin[i][k] { (info.back[i][k] - info.wall[i][k]).clamp(0.0, ARCH_W_M) } else { ARCH_W_M };
        portal_arch(b, st, &ring, out, rgb, arch(0), arch(1));
        let (c, h) = (crate::walls::COPING_OUT_M, crate::walls::COPING_H_M);
        let fw = mesh::scale(out, c);
        let bk = mesh::scale(t, -open_dir * (crate::tunnel::WALL_M - c).max(0.0));
        let cq = |b: &mut Builder, uu: f64, y: f64, d: V3, n: V3| v(b, mesh::add(at(uu, y), d), n, uu, y);
        // its front, top and back (the ends inside the hill or the twin's coping).
        let (u0, u1) = (wp + if info.twin[i][0] { 0.0 } else { c }, -wn - if info.twin[i][1] { 0.0 } else { c });
        let front = [cq(b, u0, top - 0.02, fw, out), cq(b, u0, top + h, fw, out), cq(b, u1, top + h, fw, out), cq(b, u1, top - 0.02, fw, out)];
        b.quad(front, out);
        let up = [0.0, 1.0, 0.0];
        let tq = [cq(b, u0, top + h, fw, up), cq(b, u1, top + h, fw, up), cq(b, u1, top + h, bk, up), cq(b, u0, top + h, bk, up)];
        b.quad(tq, up);
        let inn = mesh::scale(out, -1.0);
        let bq = [cq(b, u0, top + h, bk, inn), cq(b, u1, top + h, bk, inn), cq(b, u1, top - 0.02, bk, inn), cq(b, u0, top - 0.02, bk, inn)];
        b.quad(bq, inn);
    }
}

/// The arch band round a portal's opening `ring` (its outline in the face:
/// lateral offset, height), `w0` / `w1` wide on the +n / -n jamb and over
/// the vault, standing `ARCH_PROUD_M` toward `out` from the face.
fn portal_arch(b: &mut Builder, st: &Station, ring: &[P], out: V3, rgb: [u8; 3], w0: f64, w1: f64) {
    let n = ring.len();
    if n < 3 {
        return;
    }
    // the opening's middle (its half height over the crown's line) the band stands away from.
    let (umid, ymid) = ((ring[0].u + ring[n - 1].u) / 2.0, (ring[0].y + ring.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max)) / 2.0);
    // each point's direction away from the opening (in the face: lateral, up), its band width.
    let dirs: Vec<(f64, f64)> = (0..n)
        .map(|k| {
            let (a, c) = (ring[k.saturating_sub(1)], ring[(k + 1).min(n - 1)]);
            let (du, dy) = (c.u - a.u, c.y - a.y);
            let mut d = (dy, -du);
            let (mu, my) = (ring[k].u - umid, ring[k].y - ymid);
            if d.0 * mu + d.1 * my < 0.0 {
                d = (-d.0, -d.1);
            }
            let l = d.0.hypot(d.1).max(1e-9);
            (d.0 / l, d.1 / l)
        })
        .collect();
    let width = |k: usize| {
        let t = k as f64 / (n - 1) as f64;
        // the +n half of the outline is first: its jamb's width, then the -n jamb's.
        if t < 0.5 { w0 } else { w1 }
    };
    let at = |u: f64, y: f64, d: f64| mesh::add(st.at(u, y), mesh::scale(out, d));
    let inner: Vec<(f64, f64)> = ring.iter().map(|q| (q.u, q.y)).collect();
    let outer: Vec<(f64, f64)> = (0..n).map(|k| (ring[k].u + dirs[k].0 * width(k), ring[k].y + dirs[k].1 * width(k))).collect();
    let face_n = |k: usize, sgn: f64| mesh::norm([st.ox * dirs[k].0 * sgn, dirs[k].1 * sgn, st.oz * dirs[k].0 * sgn]);
    let vv = |b: &mut Builder, p: V3, nn: V3, uv: [f64; 2], wear: f64| b.v(p, nn, rgb, wear, uv, StructureClass::Concrete);
    for k in 0..n - 1 {
        let (a, c) = (k, k + 1);
        // the front.
        let q = [
            vv(b, at(inner[a].0, inner[a].1, ARCH_PROUD_M), out, [inner[a].0, inner[a].1 - st.y], 0.35),
            vv(b, at(outer[a].0, outer[a].1, ARCH_PROUD_M), out, [outer[a].0, outer[a].1 - st.y], 0.35),
            vv(b, at(outer[c].0, outer[c].1, ARCH_PROUD_M), out, [outer[c].0, outer[c].1 - st.y], 0.35),
            vv(b, at(inner[c].0, inner[c].1, ARCH_PROUD_M), out, [inner[c].0, inner[c].1 - st.y], 0.35),
        ];
        b.quad(q, out);
        // its inner edge (toward the opening) and outer edge.
        for (pts, sgn) in [(&inner, -1.0), (&outer, 1.0)] {
            let (na, nc) = (face_n(a, sgn), face_n(c, sgn));
            let q = [
                vv(b, at(pts[a].0, pts[a].1, 0.0), na, [pts[a].0, pts[a].1 - st.y], 0.4),
                vv(b, at(pts[a].0, pts[a].1, ARCH_PROUD_M), na, [pts[a].0, pts[a].1 - st.y], 0.4),
                vv(b, at(pts[c].0, pts[c].1, ARCH_PROUD_M), nc, [pts[c].0, pts[c].1 - st.y], 0.4),
                vv(b, at(pts[c].0, pts[c].1, 0.0), nc, [pts[c].0, pts[c].1 - st.y], 0.4),
            ];
            b.quad(q, mesh::add(na, nc));
        }
    }
    // the band's feet at the walkways.
    for k in [0, n - 1] {
        let down = [0.0, -1.0, 0.0];
        let q = [
            vv(b, at(inner[k].0, inner[k].1, 0.0), down, [inner[k].0, 0.0], 0.5),
            vv(b, at(inner[k].0, inner[k].1, ARCH_PROUD_M), down, [inner[k].0, ARCH_PROUD_M], 0.5),
            vv(b, at(outer[k].0, outer[k].1, ARCH_PROUD_M), down, [outer[k].0, ARCH_PROUD_M], 0.5),
            vv(b, at(outer[k].0, outer[k].1, 0.0), down, [outer[k].0, 0.0], 0.5),
        ];
        b.quad(q, down);
    }
}

/// The lights of a covered bore (`StructureWorld::tunnel_lights`): every
/// `LIGHT_PITCH_M` along its covered stretches, from half a pitch in, under
/// the crown.
pub(crate) fn lights(c: &Chain, sw: &Sweep) -> Vec<crate::TunnelLight> {
    let groups = light_groups(c, sw);
    if let Some((info, shape)) = sw.tunnel.as_ref().and_then(|info| Some((info, island_lighting::applies(info)?))) {
        return groups.into_iter().flat_map(|group| {
            let st = sw.lerp(group.s);
            let walls = info.wall[station_index(sw, &st)];
            island_lighting::rows(shape, &st, walls).map(|[u,y]| crate::TunnelLight {
                pos: st.at(u, y),
                flux_share: 0.5,
                ..group
            })
        }).collect();
    }
    if !presidio::applies(c) { return groups; }
    groups.into_iter().flat_map(|group| {
        let st = sw.lerp(group.s);
        presidio::fixture_offsets(&st, &group).map(|u| crate::TunnelLight {
            pos: st.at(u, group.pos[1]),
            flux_share: 0.25,
            ..group
        })
    }).collect()
}

fn light_groups(c: &Chain, sw: &Sweep) -> Vec<crate::TunnelLight> {
    let Some(info) = sw.tunnel.as_ref() else { return vec![] };
    let mut out = vec![];
    let n = sw.st.len();
    let mut i = 0;
    while i < n {
        if !info.covered[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i + 1 < n && info.covered[i + 1] {
            i += 1;
        }
        let (s0, s1) = (sw.st[a].s, sw.st[i].s);
        let mut s = s0 + LIGHT_PITCH_M / 2.0;
        while s < s1 {
            let st = sw.lerp(s);
            let y = st.y + info.island_section.map_or(BORE_CLEAR_M, |section| section.crown()) - LIGHT_DROP_M;
            let k = station_index(sw, &st).min(n - 1);
            let (uc, a) = crown_of(info.wall[k]);
            let q = st.at(uc, y);
            out.push(crate::TunnelLight { chain: c.id, s, pos: q, half_width: a, floor: st.y, flux_share: 1.0 });
            s += LIGHT_PITCH_M;
        }
        i += 1;
    }
    out
}

/// The covered stretches of a tunnel chain (`StructureWorld::bore_at`):
/// each one's stations (centreline, normal, roadway height, the inside's
/// reach to the wall backs either side) and which of its ends is a portal
/// (a covered station beside an open one: daylight comes in there; a
/// covered chain end goes on into another structure).
pub(crate) fn bores(c: &Chain, sw: &Sweep) -> Vec<crate::Bore> {
    let Some(info) = sw.tunnel.as_ref() else { return vec![] };
    let n = sw.st.len();
    let mut out = vec![];
    let mut i = 0;
    while i < n {
        if !info.covered[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i + 1 < n && info.covered[i + 1] {
            i += 1;
        }
        if i > a {
            let points: Vec<crate::BorePoint> = (a..=i).map(|k| {
                let st = &sw.st[k];
                crate::BorePoint { x: st.x, z: st.z, s: st.s, y: st.y, nx: st.ox, nz: st.oz, reach: info.back[k] }
            }).collect();
            let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in &points {
                let r = p.reach[0].max(p.reach[1]) * (p.nx.hypot(p.nz)).max(1.0);
                mn = [mn[0].min(p.x - r), mn[1].min(p.z - r)];
                mx = [mx[0].max(p.x + r), mx[1].max(p.z + r)];
            }
            out.push(crate::Bore { chain: c.id, island_section: info.island_section, s0: sw.st[a].s, s1: sw.st[i].s, portal: [a > 0, i + 1 < n], points, min: mn, max: mx });
        }
        i += 1;
    }
    out
}

/// The ground the open stretches of a tunnel cut through
/// (`StructureWorld::terrain_cuts`): one ring per open stretch whose drawn
/// ground beside it stands over the roadway somewhere, out to the walls'
/// backs, from the open stretch's first station to its last (a portal's
/// face, or the street at the chain's end).
pub(crate) fn cuts(c: &Chain, sw: &Sweep) -> Vec<crate::TerrainCut> {
    let Some(info) = sw.tunnel.as_ref() else { return vec![] };
    let n = sw.st.len();
    let mut out = vec![];
    let mut i = 0;
    while i < n {
        if info.covered[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i + 1 < n && !info.covered[i + 1] {
            i += 1;
        }
        // reach the portal face (the next or previous covered station) too.
        let (lo, hi) = (a.saturating_sub(1), (i + 1).min(n - 1));
        let span = &sw.st[lo..=hi];
        let intrudes = (lo..=hi).any(|k| info.beside[k][0].max(info.beside[k][1]) > sw.st[k].y - CUT_BELOW_M);
        if span.len() >= 2 && intrudes {
            // past an end at the street, on over the street's own stub by
            // PORTAL_OPEN_M (the drawn ground there slopes from the street held
            // under it up to the hillside over the bore across one whole cell).
            let past = |k: usize, sgn: f64| {
                let st = &sw.st[k];
                let (dx, dz) = (st.tx * PORTAL_OPEN_M * sgn, st.tz * PORTAL_OPEN_M * sgn);
                ([st.x + st.ox * info.back[k][0] + dx, st.z + st.oz * info.back[k][0] + dz], [st.x - st.ox * info.back[k][1] + dx, st.z - st.oz * info.back[k][1] + dz])
            };
            let mut left: Vec<[f64; 2]> = (lo..=hi).map(|k| sw.st[k].at(info.back[k][0], 0.0)).map(|q| [q[0], q[2]]).collect();
            let mut right: Vec<[f64; 2]> = (lo..=hi).map(|k| sw.st[k].at(-info.back[k][1], 0.0)).map(|q| [q[0], q[2]]).collect();
            let mut floor = span.iter().map(|st| st.y).fold(f64::INFINITY, f64::min);
            if lo == 0 && sw.ends[0] == crate::EndKind::Paved {
                let (l, r) = past(0, -1.0);
                left.insert(0, l);
                right.insert(0, r);
                floor = floor.min(c.ends[0].fixed.unwrap_or(floor));
            }
            if hi == n - 1 && sw.ends[1] == crate::EndKind::Paved {
                let (l, r) = past(n - 1, 1.0);
                left.push(l);
                right.push(r);
                floor = floor.min(c.ends[1].fixed.unwrap_or(floor));
            }
            let floor = floor - CUT_BELOW_M;
            let mut ring = left;
            ring.extend(right.into_iter().rev());
            let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for q in &ring {
                mn = [mn[0].min(q[0]), mn[1].min(q[1])];
                mx = [mx[0].max(q[0]), mx[1].max(q[1])];
            }
            out.push(crate::TerrainCut { chain: c.id, s0: span[0].s, s1: span[span.len() - 1].s, ring, floor, min: mn, max: mx, canopy: false });
            // each portal's canopy: PORTAL_OPEN_M back over the bore, out to
            // the headwall's wings.
            for (face, into) in [(lo, -1.0), (hi, 1.0)] {
                if !info.covered[face] {
                    continue;
                }
                let s0 = sw.st[face].s;
                let s1 = (s0 + into * PORTAL_OPEN_M).clamp(sw.st[0].s, sw.st[n - 1].s);
                let k = 6;
                let wing = |q: usize| [0, 1].map(|j| if info.twin[q][j] { info.back[q][j] } else { info.wall[q][j] + WING_M });
                let w = wing(face);
                let pts: Vec<crate::sweep::Station> = (0..=k).map(|m| sw.lerp(s0 + (s1 - s0) * m as f64 / k as f64)).collect();
                let mut ring: Vec<[f64; 2]> = pts.iter().map(|st| st.at(w[0], 0.0)).map(|q| [q[0], q[2]]).collect();
                ring.extend(pts.iter().rev().map(|st| st.at(-w[1], 0.0)).map(|q| [q[0], q[2]]));
                let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
                for q in &ring {
                    mn = [mn[0].min(q[0]), mn[1].min(q[1])];
                    mx = [mx[0].max(q[0]), mx[1].max(q[1])];
                }
                let floor = sw.st[face].y - CUT_BELOW_M;
                out.push(crate::TerrainCut { chain: c.id, s0: s0.min(s1), s1: s0.max(s1), ring, floor, min: mn, max: mx, canopy: true });
            }
        }
        i += 1;
    }
    out
}
