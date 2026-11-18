//! Paved greens laid as paving: a port of src/world/streets/plaza-paving.js
//! `planPavedPlazas` (which greens are plaza paving: a green the source maps
//! as `ground: 'paved'` whose kind is an open space, unless it stands mostly
//! on a building footprint, a roof garden, or over mapped water, a deck; its
//! style from the OSM surface or a hash of its id; its frame along its
//! longest edge; the region: the smallest plaza holding a point, less the
//! other greens mapped inside it and mapped water) and a reduced form of
//! street-surface-v2.js `emitPavedPlazas`: the plaza's bays, in its own frame,
//! cut on the region's mapped edges and on footprints with the module's
//! convex cutting (`cutConvexOnSegments`), laid at the footway datum
//! (`curbTopY(datum) + curbTopFall`), never under the ground. Not ported from
//! the emitter: the brick style's concrete bands, the joints and unit tones,
//! the walks the paving meets at their own height and the closures.

use serde_json::Value;

/// One mapped area: id as the JS carpet receives it (`osm-green-<id>`), kind, ground, surface, rings.
#[derive(Clone, Debug)]
pub struct Area {
    pub id: String,
    pub kind: String,
    pub ground: String,
    pub surface: String,
    pub polygon: Vec<[f64; 2]>,
    pub holes: Vec<Vec<[f64; 2]>>,
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
}

impl Bounds {
    fn of(rings: &[Vec<[f64; 2]>]) -> Self {
        let mut b = Bounds { min_x: f64::INFINITY, max_x: f64::NEG_INFINITY, min_z: f64::INFINITY, max_z: f64::NEG_INFINITY };
        for r in rings {
            for p in r {
                b.min_x = b.min_x.min(p[0]);
                b.max_x = b.max_x.max(p[0]);
                b.min_z = b.min_z.min(p[1]);
                b.max_z = b.max_z.max(p[1]);
            }
        }
        b
    }
    fn overlaps(&self, o: &Bounds) -> bool {
        self.min_x <= o.max_x && self.max_x >= o.min_x && self.min_z <= o.max_z && self.max_z >= o.min_z
    }
    fn holds(&self, x: f64, z: f64) -> bool {
        x >= self.min_x && x <= self.max_x && z >= self.min_z && z <= self.max_z
    }
}

/// plaza-paving.js's constants (read from its source by the export).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlazaConsts {
    pub plaza_kinds: Vec<String>,
    pub elevated_share: f64,
    pub elevated_sample_count: f64,
    pub styles: Vec<String>,
    pub surface_style: std::collections::BTreeMap<String, Value>,
}

#[derive(Clone, Debug)]
pub struct Plaza {
    pub id: String,
    pub kind: String,
    pub rings: Vec<Vec<[f64; 2]>>,
    pub bounds: Bounds,
    pub area: f64,
    pub frame: [f64; 2],
    pub style: String,
}

pub struct Plan {
    pub plazas: Vec<Plaza>,
    pub skipped: Vec<(String, &'static str, f64)>,
    pub children: Vec<(Vec<Vec<[f64; 2]>>, Bounds)>,
    pub water: Vec<(Vec<Vec<[f64; 2]>>, Bounds)>,
}

/// `inRingsXZ`: even-odd over the outer ring and the holes.
pub fn in_rings(rings: &[Vec<[f64; 2]>], x: f64, z: f64) -> bool {
    let mut inside = false;
    for ring in rings {
        let mut j = ring.len() - 1;
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[j]);
            if (a[1] > z) != (b[1] > z) && x < ((b[0] - a[0]) * (z - a[1])) / (b[1] - a[1]) + a[0] {
                inside = !inside;
            }
            j = i;
        }
    }
    inside
}

fn ring_area(r: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    let mut j = r.len() - 1;
    for i in 0..r.len() {
        a += r[j][0] * r[i][1] - r[i][0] * r[j][1];
        j = i;
    }
    a / 2.0
}

fn area_centroid(r: &[[f64; 2]]) -> [f64; 2] {
    let (mut a, mut cx, mut cz) = (0.0, 0.0, 0.0);
    let mut j = r.len() - 1;
    for i in 0..r.len() {
        let f = r[j][0] * r[i][1] - r[i][0] * r[j][1];
        a += f;
        cx += (r[j][0] + r[i][0]) * f;
        cz += (r[j][1] + r[i][1]) * f;
        j = i;
    }
    if !(a.abs() > 1e-9) {
        let (sx, sz) = r.iter().fold((0.0, 0.0), |s, p| (s.0 + p[0], s.1 + p[1]));
        return [sx / r.len() as f64, sz / r.len() as f64];
    }
    [cx / (3.0 * a), cz / (3.0 * a)]
}

/// `plazaFrame`: the longest edge's unit, folded into x > 0, z >= 0.
fn plaza_frame(r: &[[f64; 2]]) -> [f64; 2] {
    let (mut best, mut ux, mut uz) = (0.0, 1.0, 0.0);
    for i in 0..r.len() {
        let (a, b) = (r[i], r[(i + 1) % r.len()]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dz);
        if l > best + 1e-9 {
            best = l;
            ux = dx / l;
            uz = dz / l;
        }
    }
    let mut k = 0;
    while k < 4 && !(ux > 0.0 && uz >= 0.0) {
        let t = ux;
        ux = -uz;
        uz = t;
        k += 1;
    }
    [ux, uz]
}

/// FNV-1a over the UTF-16 code units, as the JS's `hash32` (ids are ASCII).
pub fn hash32(s: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for c in s.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

fn ring_ok(r: &[[f64; 2]]) -> bool {
    r.len() >= 3 && r.iter().all(|p| p[0].is_finite() && p[1].is_finite())
}

/// `planPavedPlazas(city, { buildingAt })`.
pub fn plan(k: &PlazaConsts, greens: &[Area], water: &[Area], building_at: Option<&dyn Fn(f64, f64) -> bool>) -> Plan {
    let greens: Vec<&Area> = greens.iter().filter(|g| ring_ok(&g.polygon)).collect();
    let water: Vec<(Vec<Vec<[f64; 2]>>, Bounds)> = water
        .iter()
        .filter(|w| w.kind != "beach" && ring_ok(&w.polygon))
        .map(|w| {
            let rings = vec![w.polygon.clone()];
            let b = Bounds::of(&rings);
            (rings, b)
        })
        .collect();
    let in_water = |x: f64, z: f64| water.iter().any(|(r, b)| b.holds(x, z) && in_rings(r, x, z));
    let mut plazas = Vec::new();
    let mut skipped = Vec::new();
    for g in &greens {
        if !(g.ground == "paved" && k.plaza_kinds.contains(&g.kind)) {
            continue;
        }
        let mut rings = vec![g.polygon.clone()];
        rings.extend(g.holes.iter().filter(|h| ring_ok(h)).cloned());
        let bounds = Bounds::of(&rings);
        let area = ring_area(&g.polygon).abs() - g.holes.iter().filter(|h| ring_ok(h)).map(|h| ring_area(h).abs()).sum::<f64>();
        if !(area > 0.0) {
            continue;
        }
        let step = (0.5f64).max((area / k.elevated_sample_count).sqrt());
        let (mut n, mut on_b, mut on_w) = (0usize, 0usize, 0usize);
        let mut z = bounds.min_z + step / 2.0;
        while z < bounds.max_z {
            let mut x = bounds.min_x + step / 2.0;
            while x < bounds.max_x {
                if in_rings(&rings, x, z) {
                    n += 1;
                    if building_at.is_some_and(|f| f(x, z)) {
                        on_b += 1;
                    }
                    if in_water(x, z) {
                        on_w += 1;
                    }
                }
                x += step;
            }
            z += step;
        }
        if n > 0 && on_b as f64 / n as f64 >= k.elevated_share {
            skipped.push((g.id.clone(), "roof", on_b as f64 / n as f64));
            continue;
        }
        if n > 0 && on_w as f64 / n as f64 >= k.elevated_share {
            skipped.push((g.id.clone(), "deck", on_w as f64 / n as f64));
            continue;
        }
        let tagged = k.surface_style.get(&g.surface.to_lowercase()).and_then(|v| v.as_str()).map(|s| s.to_string());
        let style = tagged.unwrap_or_else(|| k.styles[(hash32(&format!("plaza-style:{}", g.id)) as usize) % k.styles.len()].clone());
        plazas.push(Plaza { id: g.id.clone(), kind: g.kind.clone(), rings, bounds, area, frame: plaza_frame(&g.polygon), style });
    }
    let owner = |plazas: &Vec<Plaza>, x: f64, z: f64| -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, p) in plazas.iter().enumerate() {
            if !p.bounds.holds(x, z) || !in_rings(&p.rings, x, z) {
                continue;
            }
            best = match best {
                None => Some(i),
                Some(b) if p.area < plazas[b].area || (p.area == plazas[b].area && p.id < plazas[b].id) => Some(i),
                keep => keep,
            };
        }
        best
    };
    let mut children = Vec::new();
    for g in &greens {
        if plazas.iter().any(|p| p.id == g.id) {
            continue;
        }
        let mut rings = vec![g.polygon.clone()];
        rings.extend(g.holes.iter().filter(|h| ring_ok(h)).cloned());
        let b = Bounds::of(&rings);
        if !plazas.iter().any(|p| p.bounds.overlaps(&b)) {
            continue;
        }
        let c = area_centroid(&g.polygon);
        let Some(host) = owner(&plazas, c[0], c[1]) else { continue };
        if !(ring_area(&g.polygon).abs() < plazas[host].area) {
            continue;
        }
        children.push((rings, b));
    }
    Plan { plazas, skipped, children, water }
}

impl Plan {
    /// `regionAt`: the plaza owning the point, unless a child green or water holds it.
    pub fn region_at(&self, x: f64, z: f64) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, p) in self.plazas.iter().enumerate() {
            if !p.bounds.holds(x, z) || !in_rings(&p.rings, x, z) {
                continue;
            }
            best = match best {
                None => Some(i),
                Some(b) if p.area < self.plazas[b].area || (p.area == self.plazas[b].area && p.id < self.plazas[b].id) => Some(i),
                keep => keep,
            };
        }
        let p = best?;
        if self.children.iter().any(|(r, b)| b.holds(x, z) && in_rings(r, x, z)) || self.water.iter().any(|(r, b)| b.holds(x, z) && in_rings(r, x, z)) {
            return None;
        }
        Some(p)
    }
}

// ---- convex cutting (plaza-paving.js `cutConvexOnSegments`) ----

fn convex_area(poly: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    a / 2.0
}

pub fn convex_centroid(poly: &[[f64; 2]]) -> [f64; 2] {
    let (mut a, mut cx, mut cz) = (0.0, 0.0, 0.0);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let f = p[0] * q[1] - q[0] * p[1];
        a += f;
        cx += (p[0] + q[0]) * f;
        cz += (p[1] + q[1]) * f;
    }
    if !(a.abs() > 1e-12) {
        let (sx, sz) = poly.iter().fold((0.0, 0.0), |s, p| (s.0 + p[0], s.1 + p[1]));
        return [sx / poly.len() as f64, sz / poly.len() as f64];
    }
    [cx / (3.0 * a), cz / (3.0 * a)]
}

fn segment_crosses_convex(poly: &[[f64; 2]], ax: f64, az: f64, bx: f64, bz: f64) -> bool {
    let (dx, dz) = (bx - ax, bz - az);
    let len = dx.hypot(dz);
    if !(len > 1e-9) {
        return false;
    }
    let eps = 1e-7 * len.max(1.0);
    let (mut neg, mut pos) = (false, false);
    for p in poly {
        let s = (dx * (p[1] - az) - dz * (p[0] - ax)) / len;
        if s < -1e-6 {
            neg = true;
        } else if s > 1e-6 {
            pos = true;
        }
    }
    if !(neg && pos) {
        return false;
    }
    let sign = if convex_area(poly) >= 0.0 { 1.0 } else { -1.0 };
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let el = (q[0] - p[0]).hypot(q[1] - p[1]);
        if !(el > 1e-9) {
            continue;
        }
        let nx = (-(q[1] - p[1]) * sign) / el;
        let nz = ((q[0] - p[0]) * sign) / el;
        let num = nx * (ax - p[0]) + nz * (az - p[1]);
        let den = nx * dx + nz * dz;
        if den.abs() < 1e-12 * len {
            if num < -1e-9 {
                return false;
            }
            continue;
        }
        let t = -num / den;
        if den > 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 > t1 {
            return false;
        }
    }
    (t1 - t0) * len > eps + 1e-6
}

fn clip_half(poly: &[[f64; 2]], ax: f64, az: f64, dx: f64, dz: f64, side: f64) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::new();
    let push = |out: &mut Vec<[f64; 2]>, p: [f64; 2]| {
        if out.last().is_none_or(|l| (l[0] - p[0]).abs() > 1e-9 || (l[1] - p[1]).abs() > 1e-9) {
            out.push(p);
        }
    };
    let f = |p: [f64; 2]| side * (dx * (p[1] - az) - dz * (p[0] - ax));
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let (fp, fq) = (f(p), f(q));
        if fp >= 0.0 {
            push(&mut out, p);
        }
        if (fp > 0.0 && fq < 0.0) || (fp < 0.0 && fq > 0.0) {
            let t = fp / (fp - fq);
            push(&mut out, [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
        }
    }
    while out.len() > 1 && (out[0][0] - out[out.len() - 1][0]).abs() <= 1e-9 && (out[0][1] - out[out.len() - 1][1]).abs() <= 1e-9 {
        out.pop();
    }
    out
}

/// `cutConvexOnSegments`.
pub fn cut_convex(cell: Vec<[f64; 2]>, segments: &[[f64; 4]]) -> Vec<Vec<[f64; 2]>> {
    let mut pieces = vec![cell];
    for &[ax, az, bx, bz] in segments {
        let mut next: Option<Vec<Vec<[f64; 2]>>> = None;
        for k in 0..pieces.len() {
            let piece = &pieces[k];
            if !segment_crosses_convex(piece, ax, az, bx, bz) {
                if let Some(n) = next.as_mut() {
                    n.push(piece.clone());
                }
                continue;
            }
            if next.is_none() {
                next = Some(pieces[..k].to_vec());
            }
            let (dx, dz) = (bx - ax, bz - az);
            for side in [-1.0, 1.0] {
                let part = clip_half(piece, ax, az, dx, dz, side);
                if part.len() >= 3 && convex_area(&part).abs() > 1e-6 {
                    next.as_mut().unwrap().push(part);
                }
            }
        }
        if let Some(n) = next {
            pieces = n;
        }
    }
    pieces
}

/// The pieces of every plaza: its bays (`bay` metres in its frame) cut on the
/// region's mapped edges and on footprints, kept where the region holds the
/// piece's centroid and no footprint does. Returns (plaza index, piece).
pub fn pieces(plan: &Plan, bay: f64, footprints: &[(Vec<[f64; 2]>, Bounds)]) -> Vec<(usize, Vec<[f64; 2]>)> {
    let mut out = Vec::new();
    for (pi, p) in plan.plazas.iter().enumerate() {
        let near_fp: Vec<&(Vec<[f64; 2]>, Bounds)> = footprints.iter().filter(|(_, b)| b.overlaps(&p.bounds)).collect();
        let mut segs: Vec<[f64; 4]> = Vec::new();
        let mut add = |r: &[[f64; 2]]| {
            for i in 0..r.len() {
                let (a, b) = (r[i], r[(i + 1) % r.len()]);
                if (b[0] - a[0]).hypot(b[1] - a[1]) > 1e-6 {
                    segs.push([a[0], a[1], b[0], b[1]]);
                }
            }
        };
        for q in &plan.plazas {
            if q.bounds.overlaps(&p.bounds) {
                q.rings.iter().for_each(|r| add(r));
            }
        }
        for (rings, b) in plan.children.iter().chain(plan.water.iter()) {
            if b.overlaps(&p.bounds) {
                rings.iter().for_each(|r| add(r));
            }
        }
        for (r, _) in &near_fp {
            add(r);
        }
        let (u, v) = (p.frame, [-p.frame[1], p.frame[0]]);
        let (mut a0, mut a1, mut c0, mut c1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for q in &p.rings[0] {
            let (a, c) = (q[0] * u[0] + q[1] * u[1], q[0] * v[0] + q[1] * v[1]);
            a0 = a0.min(a);
            a1 = a1.max(a);
            c0 = c0.min(c);
            c1 = c1.max(c);
        }
        let (ia0, ic0) = ((a0 / bay).floor() as i64, (c0 / bay).floor() as i64);
        let (ia1, ic1) = ((a1 / bay).ceil() as i64, (c1 / bay).ceil() as i64);
        for ia in ia0..ia1 {
            for ic in ic0..ic1 {
                let at = |a: f64, c: f64| [u[0] * a + v[0] * c, u[1] * a + v[1] * c];
                let (a, c) = (ia as f64 * bay, ic as f64 * bay);
                let cell = vec![at(a, c), at(a + bay, c), at(a + bay, c + bay), at(a, c + bay)];
                let cb = Bounds::of(&[cell.clone()]);
                let local: Vec<[f64; 4]> = segs.iter().copied().filter(|s| s[0].max(s[2]) >= cb.min_x && s[0].min(s[2]) <= cb.max_x && s[1].max(s[3]) >= cb.min_z && s[1].min(s[3]) <= cb.max_z).collect();
                for piece in cut_convex(cell, &local) {
                    let m = convex_centroid(&piece);
                    if plan.region_at(m[0], m[1]) != Some(pi) {
                        continue;
                    }
                    if near_fp.iter().any(|(r, b)| b.holds(m[0], m[1]) && in_rings(std::slice::from_ref(r), m[0], m[1])) {
                        continue;
                    }
                    out.push((pi, piece));
                }
            }
        }
    }
    out
}

/// sf-lots-parks's greens and inland water as the JS carpet receives them.
pub fn load_areas(repo_root: &std::path::Path) -> anyhow::Result<(Vec<Area>, Vec<Area>)> {
    use std::io::Read;
    let bytes = std::fs::read(repo_root.join("public/data/sf/sf-lots-parks.json.gz"))?;
    let mut s = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut s)?;
    let v: Value = serde_json::from_str(&s)?;
    let gs = eo_data::groundscale::scale(); // the greens and water at the scaled ground plan
    let ring = |a: &Value| -> Vec<[f64; 2]> { a.as_array().map(|a| a.chunks_exact(2).map(|c| [c[0].as_f64().unwrap_or(f64::NAN) * gs, c[1].as_f64().unwrap_or(f64::NAN) * gs]).collect()).unwrap_or_default() };
    let area = |g: &Value, prefix: &str| Area {
        id: format!("{prefix}{}", g["id"].as_str().map(|s| s.to_string()).unwrap_or_else(|| g["id"].to_string())),
        kind: g["kind"].as_str().unwrap_or("").to_string(),
        ground: g["ground"].as_str().unwrap_or("").to_string(),
        surface: g["surface"].as_str().unwrap_or("").to_string(),
        polygon: ring(&g["polygon"]),
        holes: g["holes"].as_array().map(|h| h.iter().map(ring).filter(|r| r.len() >= 3).collect()).unwrap_or_default(),
    };
    let greens = v["greens"].as_array().map(|a| a.iter().map(|g| area(g, "osm-green-")).filter(|g| g.polygon.len() >= 3).collect()).unwrap_or_default();
    let water = v["water"].as_array().map(|a| a.iter().map(|g| area(g, "osm-water-")).filter(|g| g.polygon.len() >= 3).collect()).unwrap_or_default();
    Ok((greens, water))
}

pub fn footprint_bounds(ring: &[[f64; 2]]) -> Bounds {
    Bounds::of(&[ring.to_vec()])
}
