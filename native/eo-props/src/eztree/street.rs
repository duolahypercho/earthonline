//! street-furniture.js's street-tree table, ported: `STREET_TREE_SPECIES`,
//! `streetTreeVariantCount`, `streetTreeSpeciesId`, `streetTreeSilhouetteFor`,
//! `streetTreeVariantFor`, `streetTreeTrunkRadius`, `surveyedTreeVariant` /
//! `surveyedFormVariant`, and `buildStreetTreeGeometry` (the near / window /
//! far forms: `pruneUprightStreetStem` from surveyed-alley-tree.js,
//! `keepTrunkAndCards`, the facade pruning and street lean).
//!
//! Where the JS has model modes (`?streetTrees=`, device-tier.js
//! `streetTreeModelMode` / `streetTreeDrawCost`), they are page state there and
//! a parameter here (`StreetTreeMode`, default: what the export saw). Where it
//! has none (the export's silhouette tables are empty, `has_modes()` false),
//! the mode is ignored: every ring draws `STREET_TREE_SPECIES[variant % n]`
//! and the rings past the near one carry one variant, as that JS does.

use super::generator::{V3, js_hypot, js_max, js_min, js_round};
use super::geometry::Box3;
use super::{EzData, EzMesh, build_ez_tree};
use anyhow::{Result, anyhow};

/// The street pass's tree model mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreetTreeMode {
    /// 'legacy': one window species (street-ash) at every distance past the near ring.
    pub legacy: bool,
    /// 'genus' window/far silhouettes: 2 (broad-leaved, conifer) or 4.
    pub silhouettes: u8,
}

impl Default for StreetTreeMode {
    fn default() -> Self {
        let d = EzData::get().ok();
        StreetTreeMode {
            legacy: d.and_then(|d| d.street.default_mode.as_deref()).map(|m| m == "legacy").unwrap_or(false),
            silhouettes: d.and_then(|d| d.street.default_silhouettes).unwrap_or(2) as u8,
        }
    }
}

impl StreetTreeMode {
    fn silhouettes<'a>(&self, d: &'a EzData) -> &'a [String] {
        if self.silhouettes == 4 { &d.street.silhouettes4 } else { &d.street.silhouettes2 }
    }
}

/// `STREET_TREE_SPECIES`: ez species ids in street variant order.
pub fn street_tree_species() -> Result<&'static [String]> {
    Ok(&EzData::get()?.street.species)
}

/// `streetTreeVariantCount(lod)`.
pub fn street_tree_variant_count(lod: u32, mode: StreetTreeMode) -> Result<usize> {
    let d = EzData::get()?;
    Ok(if lod > 0 {
        if mode.legacy || !d.street.has_modes() { 1 } else { mode.silhouettes(d).len() }
    } else {
        d.street.species.len()
    })
}

/// `streetTreeSpeciesId(variant, lod)`: a species index near, a silhouette index
/// past it. Without model modes (`StreetTables::has_modes`), the species index
/// at every ring, as that JS's `buildStreetTreeGeometry` takes it.
pub fn street_tree_species_id(variant: usize, lod: u32, mode: StreetTreeMode) -> Result<&'static str> {
    let d = EzData::get()?;
    if lod > 0 && d.street.has_modes() {
        let s = mode.silhouettes(d);
        let n = street_tree_variant_count(lod, mode)?;
        return Ok(&s[variant % n]);
    }
    Ok(&d.street.species[variant % d.street.species.len()])
}

/// `streetTreeSilhouetteFor(variant)`: the window/far silhouette index for a near variant.
/// Without model modes the window/far rings draw variant 0: 0.
pub fn street_tree_silhouette_for(variant: usize, mode: StreetTreeMode) -> Result<usize> {
    let d = EzData::get()?;
    if mode.legacy || !d.street.has_modes() {
        return Ok(0);
    }
    let id = &d.street.species[variant % d.street.species.len()];
    let four = &d.street.silhouettes4[d.street.silhouette_by_species.get(id).copied().unwrap_or(0)];
    Ok(mode.silhouettes(d).iter().position(|s| s == four).unwrap_or(0))
}

/// `streetTreeVariantFor(draw, variants, footwayM, frontageYear, streetHalfM)`.
pub fn street_tree_variant_for(draw: f64, variants: usize, footway_m: Option<f64>, frontage_year: Option<f64>, street_half_m: Option<f64>) -> Result<usize> {
    let d = EzData::get()?;
    let species = &d.street.species;
    let find = |id: &str| species.iter().position(|s| s == id);
    let small = find("street-round");
    let mature = find("street-mature");
    let tall: Vec<usize> = species
        .iter()
        .enumerate()
        .filter(|(i, s)| {
            Some(*i) != small
                && Some(*i) != mature
                && !d.species.get(*s).map(|x| x.observed_only || x.surveyed_only).unwrap_or(false)
                && *i < variants
        })
        .map(|(i, _)| i)
        .collect();
    let v = variants as f64;
    let fallback = || ((draw * v).floor() % js_max(1.0, v)) as usize;
    let Some(small) = small else { return Ok(fallback()) };
    if !(variants > 1) || small >= variants || tall.is_empty() {
        return Ok(fallback());
    }
    let finite = |x: Option<f64>| x.filter(|v| v.is_finite());
    let young = match finite(frontage_year) {
        Some(y) => y >= d.street.young_frontage_year,
        None => finite(footway_m).is_some_and(|f| f < d.street.small_footway_m),
    };
    if young {
        return Ok(small);
    }
    if let Some(m) = mature {
        if m < variants && finite(frontage_year).is_some() && finite(street_half_m).is_some_and(|h| h >= d.street.mature_min_half_m) {
            return Ok(m);
        }
    }
    let n = tall.len() as f64;
    Ok(tall[((draw * n).floor() % n) as usize])
}

/// `surveyedFormVariant(form, variants, station)`.
pub fn surveyed_form_variant(form: &str, variants: usize, station: f64) -> Result<usize> {
    if !(variants > 1) {
        return Ok(0);
    }
    let d = EzData::get()?;
    let species = &d.street.species;
    let by_id = |id: &str| species.iter().position(|s| s == id);
    let plain = |id: &str| by_id(id);
    let bounded = |id: &str| by_id(id).filter(|&i| i < variants);
    let hit = match form {
        "mature" => plain("street-mature"),
        "round" => plain("street-round"),
        "pollard" => plain("street-pollard"),
        "elm" => plain("street-elm"),
        "canopy" => bounded("street-canopy"),
        "dome" => bounded("street-dome"),
        "high" => bounded("street-high"),
        "full" | "full-s" => bounded("street-full"),
        "full-wrap" => bounded("street-full-wrap"),
        _ => None,
    };
    if let Some(i) = hit {
        return Ok(i);
    }
    let tall: Vec<usize> = species.iter().enumerate().filter(|(_, s)| *s == "street-ash" || *s == "street-oak").map(|(i, _)| i).collect();
    if tall.is_empty() {
        return Ok(0);
    }
    let k = js_round(station * 7.0).abs();
    Ok(tall[(k % tall.len() as f64) as usize])
}

/// `surveyedTreeVariant(form, variants, station, leaf)`: the form's variant in
/// the leaf state an overlay observed (`leaf`, e.g. "bare"; None = in leaf).
pub fn surveyed_tree_variant(form: &str, variants: usize, station: f64, leaf: Option<&str>) -> Result<usize> {
    let base = surveyed_form_variant(form, variants, station)?;
    let d = EzData::get()?;
    let alt = leaf.and_then(|l| d.street.species.get(base).and_then(|id| d.species.get(id)).and_then(|s| s.leaf_states.get(l)));
    let alt_index = alt.and_then(|a| d.street.species.iter().position(|s| s == a));
    Ok(match alt_index {
        Some(i) if i < variants => i,
        _ => base,
    })
}

/// `streetTreeTrunkRadius(variant, lod)`: root radius of a species' trunk
/// (its near tier), metres, before the instance scale. Builds the tree; the
/// cooked library answers the same from its cache (`EzLibrary::street_tree_trunk_radius`).
pub fn street_tree_trunk_radius(variant: usize, lod: u32, mode: StreetTreeMode) -> Result<f64> {
    let id = street_tree_species_id(variant, lod, mode)?;
    Ok(build_ez_tree(id, "near", 0)?.user.trunk_radius_m)
}

/// One street form as `buildStreetTreeGeometry` returns it. `mesh.part` is
/// kept (the JS deletes `plantPart` after use; the split is also in the UV).
/// A far form keeps the vertices its dropped limbs used (unreferenced), as the JS does.
#[derive(Clone, Debug)]
pub struct StreetTreeMesh {
    pub species: String,
    pub mesh: EzMesh,
    /// The bounding sphere radius the JS sets: the geometry's, plus the wind envelope pad.
    pub bounding_radius_m: f64,
    /// `userData.streetPruned`: (facadeReachM, streetLeanM) where the species is facade-pruned.
    pub street_pruned: Option<(f64, f64)>,
}

/// `pruneUprightStreetStem` (surveyed-alley-tree.js): cap the wood below the
/// crown to a 0.125 m stem, smoothly between 3.3 m and 3.9 m; bark normals recomputed.
fn prune_upright_street_stem(m: &mut EzMesh) {
    // surveyed-alley-tree.js's own shape constants.
    const PLANE: f64 = 3.3;
    const TOP: f64 = 3.9;
    const BAND: f64 = 0.6;
    const STEM: f64 = 0.125;
    let n = m.positions.len();
    let mut protected = vec![false; n];
    for t in m.indices.chunks_exact(3) {
        if t.iter().any(|&v| m.part[v as usize] != 0) || t.iter().all(|&v| m.positions[v as usize][1] as f64 >= PLANE) {
            continue;
        }
        for &v in t {
            protected[v as usize] = true;
        }
    }
    let mut changed = false;
    for i in 0..n {
        if m.part[i] != 0 {
            continue;
        }
        let y = m.positions[i][1] as f64;
        if y >= TOP && !protected[i] {
            continue;
        }
        let (x, z) = (m.positions[i][0] as f64, m.positions[i][2] as f64);
        let radius = js_hypot(&[x, z]);
        let t = js_max(0.0, js_min(1.0, (y - PLANE) / BAND));
        let blend = t * t * (3.0 - 2.0 * t);
        let limit = if protected[i] { STEM } else { STEM + js_max(0.0, radius - STEM) * blend };
        if radius <= limit {
            continue;
        }
        m.positions[i][0] = (x * limit / radius) as f32;
        m.positions[i][2] = (z * limit / radius) as f32;
        changed = true;
    }
    if changed {
        let original = m.normals.clone();
        compute_vertex_normals(m);
        for i in 0..n {
            if m.part[i] != 0 {
                m.normals[i] = original[i];
            }
        }
    }
}

/// three's `computeVertexNormals` (indexed): area-weighted, accumulated in f32, normalized.
fn compute_vertex_normals(m: &mut EzMesh) {
    for nrm in m.normals.iter_mut() {
        *nrm = [0.0; 3];
    }
    let p = |v: u32| {
        let q = m.positions[v as usize];
        V3::new(q[0] as f64, q[1] as f64, q[2] as f64)
    };
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let cb = V3::new(c.x - b.x, c.y - b.y, c.z - b.z);
        let ab = V3::new(a.x - b.x, a.y - b.y, a.z - b.z);
        let n = V3::new(cb.y * ab.z - cb.z * ab.y, cb.z * ab.x - cb.x * ab.z, cb.x * ab.y - cb.y * ab.x);
        for &v in t {
            let o = &mut m.normals[v as usize];
            *o = [(o[0] as f64 + n.x) as f32, (o[1] as f64 + n.y) as f32, (o[2] as f64 + n.z) as f32];
        }
    }
    for o in m.normals.iter_mut() {
        let (x, y, z) = (o[0] as f64, o[1] as f64, o[2] as f64);
        let l = (x * x + y * y + z * z).sqrt();
        let l = if l == 0.0 || l.is_nan() { 1.0 } else { l };
        let s = 1.0 / l;
        *o = [(x * s) as f32, (y * s) as f32, (z * s) as f32];
    }
}

/// `keepTrunkAndCards`: a far tree keeps its leaf cards and the clear stem under them.
fn keep_trunk_and_cards(m: &mut EzMesh, clearance: f64) {
    let mut kept = Vec::with_capacity(m.indices.len());
    for t in m.indices.chunks_exact(3) {
        let leaf = t.iter().any(|&v| m.part[v as usize] as f64 > 0.5);
        let stem = t.iter().all(|&v| {
            let q = m.positions[v as usize];
            (q[1] as f64) < clearance && js_hypot(&[q[0] as f64, q[2] as f64]) < 0.3
        });
        if leaf || stem {
            kept.extend_from_slice(t);
        }
    }
    m.indices = kept;
}

/// three's `computeBoundingSphere` radius (box centre, farthest vertex).
fn bounding_radius(m: &EzMesh) -> f64 {
    let flat: Vec<f32> = m.positions.iter().flatten().copied().collect();
    let c = Box3::of(&flat).center();
    let mut max_sq = 0.0f64;
    for q in &m.positions {
        let (dx, dy, dz) = (c[0] - q[0] as f64, c[1] - q[1] as f64, c[2] - q[2] as f64);
        max_sq = js_max(max_sq, dx * dx + dy * dy + dz * dz);
    }
    max_sq.sqrt()
}

/// The street form of an already-built ez mesh (`buildStreetTreeGeometry`
/// after its `buildEzTreeGeometry` call). `species` is the id the mesh was built for.
pub(crate) fn street_form_of(species: &str, source: &EzMesh, coarse: bool, distant: bool) -> Result<StreetTreeMesh> {
    let d = EzData::get()?;
    let mut m = source.clone();
    if species == "street-oak" || (coarse && species == "street-ash") {
        prune_upright_street_stem(&mut m);
    }
    if distant {
        keep_trunk_and_cards(&mut m, d.street.min_leaf_clearance_m);
    }
    let sp = d.species(species)?;
    let mut street_pruned = None;
    if let Some(reach) = sp.facade_reach_m.filter(|r| r.is_finite() && *r > 0.0) {
        let mut max_z = 0.0f64;
        let mut top = 1e-3f64;
        for q in &m.positions {
            max_z = js_max(max_z, q[2] as f64);
            top = js_max(top, q[1] as f64);
        }
        let squeeze = if max_z > reach { reach / max_z } else { 1.0 };
        let lean = sp.street_lean_m.filter(|v| !v.is_nan()).unwrap_or(0.0);
        for q in m.positions.iter_mut() {
            let y = q[1] as f64;
            let mut z = q[2] as f64;
            if z > 0.0 {
                z *= squeeze;
            }
            z -= lean * (js_max(0.0, y) / top).powf(1.4);
            q[2] = z as f32;
        }
        street_pruned = Some((reach, lean));
    }
    let bounding_radius_m = bounding_radius(&m) + d.street.wind_bound_pad_m;
    Ok(StreetTreeMesh { species: species.to_string(), mesh: m, bounding_radius_m, street_pruned })
}

/// `buildStreetTreeGeometry(variant, coarse, distant)`: the form the street
/// pass draws for a variant (a species index near, a silhouette index past the
/// near ring). The window ring draws (coarse, !distant), the far ring
/// (coarse, distant), the near ring (!coarse, !distant); the near trees'
/// shadow proxies are the window forms.
pub fn build_street_tree_geometry(variant: usize, coarse: bool, distant: bool, mode: StreetTreeMode) -> Result<StreetTreeMesh> {
    let id = street_tree_species_id(variant, if coarse || distant { 1 } else { 0 }, mode)?;
    let tier = if distant { "distant" } else if coarse { "coarse" } else { "near" };
    let source = build_ez_tree(id, tier, 0).map_err(|e| anyhow!("{id} {tier}: {e:#}"))?;
    street_form_of(id, &source, coarse, distant)
}
