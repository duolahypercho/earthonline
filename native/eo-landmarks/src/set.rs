//! The landmark set: every registered landmark this port builds, as the
//! page's main draws (landmark-generators.js `buildLandmark`: the generator's
//! groups merged by the registry's `draw` mapping, each draw with its own
//! material), its far item and its underwater skirt, and the set's distance
//! rules (landmark-set.js: near meshes while the camera is within `nearM` of
//! the landmark's box, hysteresis `hysteresisM`; the far item otherwise; the
//! skirt shown with the near meshes).

use crate::data::{LandmarksJs, Material};
use crate::geometry::{Frozen, GroundGrid};
use crate::suspension::DeckStandards;
use crate::{Built, Collider, LandmarkLight, bay_bridge, golden_gate};
use anyhow::Result;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Drawn while the camera is within the near distance of the landmark (main draws, skirt).
    Near,
    /// Drawn otherwise (the far item).
    Far,
    /// Always drawn (the pier deck and sheds: one small mesh each in the JS).
    Always,
}

/// A city structure that is not a registered landmark (the Ferry Plaza pier
/// deck and its sheds): no near/far switch, no fog cap, no shadow (as the JS's).
#[derive(Clone, Debug)]
pub struct Structure {
    pub name: String,
    /// Casts a sun shadow (the JS mesh's castShadow).
    pub cast: bool,
    /// Laid in the street surface's concrete (the plaza paving: the JS lays it in the street's path layer).
    pub footway: bool,
    /// drawn in this kit material (the kit's textured finish; `None`: vertex colour).
    pub kit: Option<String>,
    pub material: Material,
    pub mesh: Frozen,
}

#[derive(Clone, Debug)]
pub struct Draw {
    /// The draw key (the registry's material key it is drawn with).
    pub key: String,
    pub role: Role,
    /// The registry's material for the key; the far item and the skirt take
    /// the far batch's (`landmark-set.js`: roughness 0.8, metalness 0.1, vertex colours).
    pub material: Material,
    pub mesh: Frozen,
}

#[derive(Clone, Debug)]
pub struct Landmark {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The noise origin of its surfaces (the registry's anchor, y 0).
    pub anchor: [f64; 2],
    /// Its near reach: the union of the main draws' bounds (the set's `entry.box`).
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
    pub draws: Vec<Draw>,
    pub colliders: Vec<Collider>,
    pub stats: serde_json::Value,
    pub build_ms: f64,
}

#[derive(Clone, Debug, Default)]
pub struct LandmarkSet {
    pub landmarks: Vec<Landmark>,
    /// The bridges' night lights (lights.rs).
    pub lights: Vec<LandmarkLight>,
    /// The towers' aviation lights (towers2.rs, skyline.rs); the renderer gathers them with `lights`.
    pub tower_lights: Vec<LandmarkLight>,
    /// The citywide quay wall (shore.rs), chunked.
    pub walls: Vec<crate::shore::WallChunk>,
    /// The seawall material (renderer.js: 0x928d84, roughness 0.93, metalness 0).
    pub wall_material: Option<Material>,
    pub structures: Vec<Structure>,
    pub build_ms: f64,
    pub walls_ms: f64,
    /// the terrain press of the built waterfront (None with the waterfront off).
    pub press: Option<std::sync::Arc<crate::waterfront::ShorePress>>,
    /// the shore edges' classification.
    pub census: Option<crate::waterfront::EdgeCensus>,
    /// pier decks and piles built.
    pub piers_built: (usize, usize),
}

/// what the set builds beyond the first landmarks slice. All on
/// by default; `eo-app --no-landmarks2` turns all of it off (the A/B).
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The waterfront (waterfront.rs): the shore-edge rule, piers, the apron, the terrain press.
    pub waterfront: bool,
    /// The registered landmark buildings (Transamerica, Salesforce, Coit, Sutro, City Hall).
    pub buildings: bool,
    /// The bridges' night lights.
    pub lights: bool,
    /// The landmark buildings' second generators (towers2.rs; `eo-app
    /// --no-towers2` builds them as the JS export's generators do).
    pub towers2: bool,
    /// The named skyline towers (skyline.rs; `eo-app --no-skyline`: the kit draws them).
    pub skyline: bool,
}

impl Default for Options {
    /// Everything on, but the switches `eo-app` sets in the environment
    /// before any thread starts (`--no-towers2`: `EO_TOWERS2=off`,
    /// `--no-skyline`: `EO_SKYLINE=off`).
    fn default() -> Self {
        let on = |var: &str| std::env::var(var).map(|v| v != "off").unwrap_or(true);
        Self { waterfront: true, buildings: true, lights: true, towers2: on("EO_TOWERS2"), skyline: on("EO_SKYLINE") }
    }
}

impl Options {
    /// As the first landmarks slice built the set.
    pub fn off() -> Self {
        Self { waterfront: false, buildings: false, lights: false, towers2: false, skyline: false }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PlazaReport {
    pub plazas: usize,
    pub skipped: Vec<(String, &'static str, f64)>,
    pub children: usize,
    pub pieces: usize,
    pub covered_by_street: usize,
    pub triangles: usize,
    pub ms: f64,
    /// (reading the greens, the plan, the footprints near the plazas, the cut pieces, laying them) in ms.
    pub stage_ms: [f64; 5],
}

/// the building records the landmark buildings take over
/// (landmark-registry.js `claims`, drawn by the landmark instead of the kit)
/// and those they exclude (`excludes`: not buildings at all, Sutro Tower's
/// lattice footprints), by the source id without its `sf-building-` prefix.
pub fn claimed_records() -> std::collections::HashSet<String> {
    let js = LandmarksJs::get();
    crate::towers::BUILDING_IDS
        .iter()
        .filter_map(|id| js.registry.get(*id))
        .flat_map(|r| r.claims.iter().chain(r.excludes.iter()))
        .map(|s| s.trim_start_matches("sf-building-").to_string())
        .collect()
}

/// `claimed_records`, with the skyline towers' records when `opts.skyline`
/// (skyline.rs draws them; the kit and the far boxes do not).
pub fn claimed_records_with(opts: Options) -> std::collections::HashSet<String> {
    let mut c = if opts.buildings { claimed_records() } else { Default::default() };
    if opts.skyline {
        c.extend(crate::skyline::claims());
    }
    c
}

/// The JS's own `roughness` / `metalness` for a city structure's material.
pub fn plain_material(roughness: f64, metalness: f64, env: &str) -> Material {
    Material { roughness, metalness, env_class: env.into(), surface: None, draw: None, wire: false, surface_options: None }
}

/// A frozen mesh with one flat face normal per triangle (no shared vertices).
pub fn flat_mesh(name: &str, positions: &[[f64; 3]], indices: &[u32], colour: [f64; 3]) -> Frozen {
    let mut f = Frozen { name: name.into(), min_y: f64::INFINITY, max_y: f64::NEG_INFINITY, ..Default::default() };
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        for p in [a, b, c] {
            f.indices.push((f.positions.len() / 3) as u32);
            f.positions.extend(p.iter().map(|x| *x as f32));
            f.normals.extend(n.iter().map(|x| (*x / l) as f32));
            f.colors.extend(colour.iter().map(|x| *x as f32));
            f.min_y = f.min_y.min(p[1]);
            f.max_y = f.max_y.max(p[1]);
        }
    }
    f
}

/// The far batch's material (landmark-set.js `admit`).
pub fn far_material() -> Material {
    Material { roughness: 0.8, metalness: 0.1, env_class: "facade-masonry".into(), surface: None, draw: None, wire: false, surface_options: None }
}

/// `mergeArrays`: several frozen records drawn as one.
pub fn merge(name: &str, list: &[&Frozen]) -> Frozen {
    let list: Vec<&&Frozen> = list.iter().filter(|r| !r.is_empty()).collect();
    let wire = !list.is_empty() && list.iter().all(|r| r.axes.is_some());
    let mut out = Frozen { name: name.into(), min_y: f64::INFINITY, max_y: f64::NEG_INFINITY, ..Default::default() };
    if wire {
        out.axes = Some(vec![]);
        out.dirs = Some(vec![]);
        out.radii = Some(vec![]);
    }
    for r in list {
        let base = (out.positions.len() / 3) as u32;
        out.positions.extend_from_slice(&r.positions);
        out.normals.extend_from_slice(&r.normals);
        out.colors.extend_from_slice(&r.colors);
        out.indices.extend(r.indices.iter().map(|i| i + base));
        if wire {
            out.axes.as_mut().unwrap().extend_from_slice(r.axes.as_ref().unwrap());
            out.dirs.as_mut().unwrap().extend_from_slice(r.dirs.as_ref().unwrap());
            out.radii.as_mut().unwrap().extend_from_slice(r.radii.as_ref().unwrap());
        }
        out.min_y = out.min_y.min(r.min_y);
        out.max_y = out.max_y.max(r.max_y);
    }
    out
}

/// The ground grid of a registered landmark, sampled through `height_at`.
pub fn ground_of(js: &LandmarksJs, id: &str, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> GroundGrid {
    js.registry[id].ground.sampled(height_at)
}

/// how far a landmark moves on the scaled ground plan,
/// `(s - 1) * anchor` (a landmark building: built at its real site, real
/// size, then moved whole); [0, 0] at scale 1 and for the bridges, whose plan
/// is scaled with the ground (`LandmarksJs::apply_ground_scale`).
pub fn ground_scale_shift(js: &LandmarksJs, id: &str) -> [f64; 2] {
    let s = eo_data::groundscale::scale();
    if s == 1.0 || crate::data::PLAN_SCALED_LANDMARKS.contains(&id) {
        return [0.0, 0.0];
    }
    let a = &js.registry[id].anchor;
    [(s - 1.0) * a["x"].as_f64().unwrap_or(0.0), (s - 1.0) * a["z"].as_f64().unwrap_or(0.0)]
}

impl Landmark {
    /// move the whole landmark by `d` (x, z): draws, wire axes, colliders, bounds, noise origin.
    pub fn translate(&mut self, d: [f64; 2]) {
        if d == [0.0, 0.0] {
            return;
        }
        let (dx, dz) = (d[0] as f32, d[1] as f32);
        for dr in &mut self.draws {
            for p in dr.mesh.positions.chunks_exact_mut(3) {
                p[0] += dx;
                p[2] += dz;
            }
            if let Some(a) = dr.mesh.axes.as_mut() {
                for p in a.chunks_exact_mut(3) {
                    p[0] += dx;
                    p[2] += dz;
                }
            }
        }
        for c in &mut self.colliders {
            c.x += d[0];
            c.z += d[1];
        }
        self.box_min[0] += dx;
        self.box_min[2] += dz;
        self.box_max[0] += dx;
        self.box_max[2] += dz;
        self.anchor = [self.anchor[0] + d[0], self.anchor[1] + d[1]];
    }
}

/// Build one landmark (`buildLandmark`): the generator, then its draws.
pub fn build_landmark(js: &LandmarksJs, id: &str, ground: &GroundGrid, ca: &DeckStandards) -> Result<Landmark> {
    build_landmark_with(js, id, ground, ca, false).map(|(l, _)| l)
}

/// `build_landmark` with the landmark buildings' second generators
/// (towers2.rs) when `towers2`; also the lights a generator carries.
pub fn build_landmark_with(js: &LandmarksJs, id: &str, ground: &GroundGrid, ca: &DeckStandards, towers2: bool) -> Result<(Landmark, Vec<LandmarkLight>)> {
    let t0 = Instant::now();
    // the materials a second generator gives its own groups (in place of the registry's).
    let mut own: Vec<(String, Material)> = Vec::new();
    let mut lights: Vec<LandmarkLight> = Vec::new();
    let built: Built = match id {
        "golden-gate-bridge" => golden_gate::build(js, ground, ca),
        "bay-bridge-west" => bay_bridge::build(js, ground, ca),
        // the landmark buildings (towers.rs; towers2.rs).
        other => match if towers2 { crate::towers2::build(other, &js.buildings[other], ground) } else { crate::towers::build(other, &js.buildings[other], ground) } {
            Some(o) => {
                own = o.materials;
                lights = o.lights;
                Built { id: other.into(), version: o.version, groups: o.groups, far: o.far, skirt: Frozen::default(), colliders: o.colliders, stats: o.stats }
            }
            None => anyhow::bail!("eo-landmarks: no generator for {other}"),
        },
    };
    let own_material = |k: &str| own.iter().find(|(key, _)| key == k).map(|(_, m)| m.clone());
    let def = &js.registry[id];
    let groups: Vec<&(String, Frozen)> = built.groups.iter().filter(|(_, g)| !g.is_empty()).collect();
    let mut keys: Vec<String> = Vec::new();
    for (key, _) in &groups {
        let target = def.materials.get(key).and_then(|m| m.draw.clone()).unwrap_or_else(|| key.clone());
        if !keys.contains(&target) {
            keys.push(target);
        }
    }
    let mut draws = Vec::new();
    for target in keys {
        let list: Vec<&Frozen> = groups.iter().filter(|(k, _)| def.materials.get(k).and_then(|m| m.draw.clone()).unwrap_or_else(|| k.clone()) == target).map(|(_, g)| g).collect();
        let mesh = if list.len() == 1 { list[0].clone() } else { merge(&format!("{id}:{target}"), &list) };
        let material = own_material(&target).or_else(|| def.materials.get(&target).cloned()).unwrap_or_else(far_material);
        draws.push(Draw { key: target, role: Role::Near, material, mesh });
    }
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for d in &draws {
        for p in d.mesh.positions.chunks_exact(3) {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
    }
    if !built.far.is_empty() {
        draws.push(Draw { key: "far".into(), role: Role::Far, material: own_material("far").unwrap_or_else(far_material), mesh: built.far.clone() });
    }
    if !built.skirt.is_empty() {
        draws.push(Draw { key: "skirt".into(), role: Role::Near, material: far_material(), mesh: built.skirt.clone() });
    }
    let anchor = [def.anchor["x"].as_f64().unwrap_or(0.0), def.anchor["z"].as_f64().unwrap_or(0.0)];
    Ok((Landmark {
        id: id.into(),
        name: def.name.clone(),
        version: built.version,
        anchor,
        box_min: mn,
        box_max: mx,
        draws,
        colliders: built.colliders,
        stats: built.stats,
        build_ms: t0.elapsed().as_secs_f64() * 1e3,
    }, lights))
}

impl LandmarkSet {
    /// Every landmark this port has a generator for, on the ground `height_at`
    /// (the drawn terrain's sampler; None outside its grid, which the JS reads as 0).
    pub fn build(repo_root: &std::path::Path, height_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync), ca: &eo_data::CaStandards) -> Result<Self> {
        Self::build_with(repo_root, height_at, ca, Options::default())
    }

    /// `build` with the landmarks2 parts chosen by `opts`.
    pub fn build_with(repo_root: &std::path::Path, height_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync), ca: &eo_data::CaStandards, opts: Options) -> Result<Self> {
        let t0 = Instant::now();
        let js = LandmarksJs::get();
        let std = DeckStandards::load(ca)?;
        let mut ids = vec!["golden-gate-bridge", "bay-bridge-west"];
        if opts.buildings {
            ids.extend(crate::towers::BUILDING_IDS);
        }
        let towers2 = opts.buildings && opts.towers2;
        let (built, skyline) = std::thread::scope(|s| {
            // the named skyline towers (skyline.rs) beside the landmarks.
            let sky = opts.skyline.then(|| s.spawn(|| crate::skyline::build_all(repo_root, height_at)));
            // the groves round Coit and Sutro towers (grove.rs; `--no-grove`).
            let grove = (opts.buildings && crate::grove::grove_on()).then(|| s.spawn(|| crate::grove::build_all(repo_root, height_at, ca)));
            let hs: Vec<_> = ids
                .iter()
                .map(|id| {
                    let std = &std;
                    s.spawn(move || {
                        // groundscale (the street size, 1.25 by default): positioning only. The
                        // generator builds at its real site on the ground of its scaled
                        // site, then the whole landmark moves there (real size).
                        let d = ground_scale_shift(js, id);
                        let shifted = |x: f64, z: f64| height_at(x + d[0], z + d[1]);
                        let ground = if d == [0.0, 0.0] { ground_of(js, id, height_at) } else { ground_of(js, id, &shifted) };
                        let (mut l, mut lights) = build_landmark_with(js, id, &ground, std, towers2)?;
                        l.translate(d);
                        for k in &mut lights {
                            k.pos[0] += d[0];
                            k.pos[2] += d[1];
                            k.base[0] += d[0];
                            k.base[2] += d[1];
                        }
                        Ok::<(Landmark, Vec<LandmarkLight>), anyhow::Error>((l, lights))
                    })
                })
                .collect();
            let built = hs.into_iter().map(|h| h.join().expect("landmark build thread panicked")).collect::<Result<Vec<_>>>();
            let mut skyline = sky.map(|h| h.join().expect("skyline build thread panicked"));
            if let Some(g) = grove.map(|h| h.join().expect("grove build thread panicked")) {
                skyline = Some(match (skyline, g) {
                    (Some(Ok((mut a, mut b))), Ok((ga, gl))) => {
                        a.extend(ga);
                        b.extend(gl); // the groves' lights (grove.rs `loop_lamps`, the fourth pass)
                        Ok((a, b))
                    }
                    (None, g) => g,
                    (Some(e @ Err(_)), _) | (Some(Ok(_)), e @ Err(_)) => e,
                });
            }
            (built, skyline)
        });
        let mut tower_lights: Vec<LandmarkLight> = Vec::new();
        let mut landmarks: Vec<Landmark> = Vec::new();
        for (l, lights) in built? {
            landmarks.push(l);
            tower_lights.extend(lights);
        }
        if let Some(sk) = skyline {
            let (sl, lights) = sk?;
            landmarks.extend(sl);
            tower_lights.extend(lights);
        }
        let build_ms = t0.elapsed().as_secs_f64() * 1e3;
        // The shore edge: the citywide quay wall and the Ferry Plaza pier.
        let t1 = Instant::now();
        let sh = &js.shore;
        let boundary = eo_water::load_boundary(repo_root)?;
        let oracle = eo_water::shoreline::Oracle::citywide(&boundary).ok_or_else(|| anyhow::anyhow!("no shoreline rings"))?;
        let beaches = crate::shore::load_beaches(repo_root)?;
        let rule = crate::shore::BeachRule { beaches: &beaches, reach_m: sh.beach_wall_reach_m, wide_reach_m: sh.wide_foreshore_reach_m, wide_sources: &sh.wide_foreshore_sources };
        // renderer.js: crest = heightAt + roadLift + 0.04, foot = SEA_LEVEL_Y - 1.6.
        let top_at = |x: f64, z: f64| height_at(x, z).unwrap_or(0.0) + sh.road_lift + sh.crest_above_road_m;
        let wall_colour = crate::geometry::srgb_hex(sh.wall_srgb);
        let fp = &sh.ferry_pier;
        let lift = crate::data::num(&fp.presentation, "deckLiftM");
        let thick = crate::data::num(&fp.presentation, "deckThicknessM");
        let elev = |x: f64, z: f64| height_at(x, z).unwrap_or(0.0) + sh.road_lift;
        let mut structures = Vec::new();
        let (mut press, mut census, mut piers_built) = (None, None, (0usize, 0usize));
        let walls = if !opts.waterfront {
            let walls = crate::shore::city_walls(&sh.seawall, js.sea_level_y, &oracle, height_at, &top_at, js.sea_level_y - sh.bottom_below_sea_m, &rule, wall_colour);
            if let Some(deck) = crate::shore::ferry_pier_deck(&fp.outline, lift, thick, js.sea_level_y, &elev, &|x, z| oracle.is_water(x, z)) {
                let mut pos = deck.top.clone();
                pos.extend_from_slice(&deck.bottom);
                structures.push(Structure { cast: false, footway: false, kit: None, name: format!("ferry-pier-deck-{}", fp.source_way_id), material: plain_material(0.91, 0.02, "sidewalk"), mesh: flat_mesh("ferry-pier-deck", &pos, &deck.indices, crate::geometry::srgb_hex(fp.deck_srgb)) });
            }
            if let Some(sheds) = crate::shore::ferry_pier_sheds(&fp.outline, &fp.sheds, lift, &elev) {
                structures.push(Structure { cast: false, footway: false, kit: None, name: format!("ferry-pier-sheds-{}", fp.source_way_id), material: plain_material(0.88, 0.03, "facade-masonry"), mesh: flat_mesh("ferry-pier-sheds", &sheds.0, &sheds.1, crate::geometry::srgb_hex(fp.shed_srgb)) });
            }
            walls
        } else {
            // the waterfront as built places (waterfront.rs): the
            // shore edges classified (the county-line probe out to 64 m), the
            // piers of the citywide map as decks over the water, the quay wall
            // without its landward side under the apron, and the terrain press.
            use crate::waterfront as wf;
            let rings = wf::load_pier_rings(repo_root)?;
            let piers = wf::plan_piers(&rings, &oracle, height_at, js.sea_level_y, sh.road_lift);
            let (edges, c) = wf::classify_edges(&oracle, height_at, js.sea_level_y, &rule, &piers);
            let wall_edges: Vec<[f64; 4]> = edges.iter().filter(|e| e.pier.is_none()).map(|e| [e.a[0], e.a[1], e.b[0], e.b[1]]).collect();
            // The crest on the quay's own top (renderer.js's crest over the highest ground within 8 m).
            let crest_at = |x: f64, z: f64| wf::crest_ground(height_at, x, z) + sh.road_lift + sh.crest_above_road_m;
            let walls = crate::shore::walls_on_edges(&sh.seawall, js.sea_level_y, &|x, z| oracle.is_water(x, z), &wall_edges, &crest_at, js.sea_level_y - sh.bottom_below_sea_m, wall_colour, false);
            let concrete = crate::geometry::srgb_hex(sh.wall_srgb);
            // The apron in the street palette's pedestrian-way tone (`path`: the
            // promenade is Herb Caen Way's paving), the piers in the JS pier deck's.
            let hex = |s: &str| crate::geometry::srgb_hex(u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0x808080));
            let apron = hex(js.open_space.colors.get("path").map(|s| s.as_str()).unwrap_or("#e1e3e4"));
            let m = wf::build_meshes(&edges, &piers, &oracle, &crest_at, js.sea_level_y, 0.42, concrete, wf::UNDER_DECK_LINEAR, apron, crate::geometry::srgb_hex(fp.deck_srgb));
            piers_built = (m.decks, m.piles);
            structures.push(Structure { cast: false, footway: true, kit: None, name: "waterfront:paving".into(), material: plain_material(0.9, 0.0, "sidewalk"), mesh: m.paving });
            structures.push(Structure { cast: true, footway: false, kit: None, name: "waterfront:concrete".into(), material: Material { surface: Some("concrete".into()), ..plain_material(0.92, 0.0, "facade-masonry") }, mesh: m.concrete });
            structures.push(Structure { cast: false, footway: false, kit: None, name: "waterfront:under-deck".into(), material: plain_material(0.95, 0.0, "facade-masonry"), mesh: m.under_deck });
            // The Ferry Plaza pier's sheds (createFerryCard04PierSheds) on its
            // deck, walls and roofs in the kit's corrugated metal and membrane.
            let c0 = crate::geometry::ring_centroid(&fp.outline);
            let deck_y = piers.iter().find(|p| p.contains(c0[0], c0[1])).map(|p| p.deck_y).unwrap_or_else(|| elev(c0[0], c0[1]) + lift);
            if let Some((walls_m, roofs_m)) = wf::ferry_sheds(&fp.outline, &fp.sheds, deck_y) {
                structures.push(Structure { cast: true, footway: false, kit: Some(wf::SHED_WALL_KIT.into()), name: format!("ferry-pier-sheds-{}", fp.source_way_id), material: plain_material(0.88, 0.03, "facade-masonry"), mesh: walls_m });
                structures.push(Structure { cast: true, footway: false, kit: Some(wf::SHED_ROOF_KIT.into()), name: format!("ferry-pier-shed-roofs-{}", fp.source_way_id), material: plain_material(0.9, 0.0, "facade-masonry"), mesh: roofs_m });
            }
            press = Some(std::sync::Arc::new(wf::ShorePress::new(js.sea_level_y, &edges, &piers)));
            census = Some(c);
            walls
        };
        // the bridges' night lights (lights.rs), on the grounds they were built on.
        let lights = if opts.lights {
            // (the bridges' plan is scaled with the ground, so are their lights.)
            let gg = ground_of(js, "golden-gate-bridge", height_at);
            let bb = ground_of(js, "bay-bridge-west", height_at);
            crate::lights::bridge_lights(js, &gg, &bb)
        } else {
            Vec::new()
        };
        Ok(Self {
            landmarks,
            lights,
            tower_lights,
            walls,
            wall_material: Some(Material { surface: Some("seawall".into()), ..plain_material(0.93, 0.0, "sidewalk") }),
            structures,
            build_ms,
            walls_ms: t1.elapsed().as_secs_f64() * 1e3,
            press,
            census,
            piers_built,
        })
    }

    /// The city's paved plazas (plaza.rs): planned as the JS plans them over
    /// sf-lots-parks with `building_at` as the footprint test, their bays cut
    /// on the region's edges and `footprints`, laid at `level_at` (the footway
    /// datum, never under the ground), skipping a piece the street already
    /// paves (`paved_at` within the street's verge band of the level).
    #[allow(clippy::too_many_arguments)]
    pub fn add_plazas(
        &mut self,
        repo_root: &std::path::Path,
        building_at: &dyn Fn(f64, f64) -> bool,
        footprints: &[Vec<[f64; 2]>],
        level_at: &(dyn Fn(f64, f64) -> f64 + Sync),
        paved_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync),
        band_m: f64,
    ) -> Result<PlazaReport> {
        self.add_plazas_with(repo_root, building_at, footprints, level_at, paved_at, band_m, true)
    }

    /// `add_plazas`, each bay in its own tone (`bay_tones`) or all in the style's.
    #[allow(clippy::too_many_arguments)]
    pub fn add_plazas_with(
        &mut self,
        repo_root: &std::path::Path,
        building_at: &dyn Fn(f64, f64) -> bool,
        footprints: &[Vec<[f64; 2]>],
        level_at: &(dyn Fn(f64, f64) -> f64 + Sync),
        paved_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync),
        band_m: f64,
        bay_tones: bool,
    ) -> Result<PlazaReport> {
        let t0 = Instant::now();
        let js = LandmarksJs::get();
        let os = &js.open_space;
        let mut stage_ms = [0.0; 5];
        let mut lap = {
            let mut t = Instant::now();
            move |slot: &mut f64| {
                *slot = t.elapsed().as_secs_f64() * 1e3;
                t = Instant::now();
            }
        };
        let (greens, water) = crate::plaza::load_areas(repo_root)?;
        lap(&mut stage_ms[0]);
        let plan = crate::plaza::plan(&os.plaza, &greens, &water, Some(building_at));
        lap(&mut stage_ms[1]);
        let fps: Vec<(Vec<[f64; 2]>, crate::plaza::Bounds)> = footprints
            .iter()
            .filter(|r| r.len() >= 3)
            .map(|r| (r.clone(), crate::plaza::footprint_bounds(r)))
            .filter(|(_, b)| plan.plazas.iter().any(|p| b.min_x <= p.bounds.max_x && b.max_x >= p.bounds.min_x && b.min_z <= p.bounds.max_z && b.max_z >= p.bounds.min_z))
            .collect();
        lap(&mut stage_ms[2]);
        let bay = os.plaza_unit.max(0.5) * os.plaza_bay_units.max(1.0);
        let pieces = crate::plaza::pieces(&plan, bay, &fps);
        lap(&mut stage_ms[3]);
        let hex = |s: &str| u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
        let tone = |style: &str| crate::geometry::srgb_hex(hex(os.colors.get(&format!("plaza{}{}", &style[..1].to_uppercase(), &style[1..])).or(os.colors.get("path")).map(|s| s.as_str()).unwrap_or("#808080")));
        // Each piece's street test and heights (the street's surface tiles are
        // built on first use: most of the time), over a few threads, in order.
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
        let chunk = pieces.len().div_ceil(threads).max(1);
        let lifted: Vec<Option<Vec<[f64; 3]>>> = std::thread::scope(|s| {
            let hs: Vec<_> = pieces
                .chunks(chunk)
                .map(|part| {
                    s.spawn(move || {
                        part.iter()
                            .map(|(_, piece)| {
                                let m = crate::plaza::convex_centroid(piece);
                                let level = level_at(m[0], m[1]);
                                if paved_at(m[0], m[1]).is_some_and(|h| h - level <= band_m) {
                                    return None;
                                }
                                Some(piece.iter().map(|p| [p[0], level_at(p[0], p[1]), p[1]]).collect())
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().expect("plaza thread panicked")).collect()
        });
        let mut positions: Vec<[f64; 3]> = Vec::new();
        let mut colours: Vec<[f64; 3]> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let (mut laid, mut covered) = (0usize, 0usize);
        for ((pi, piece), lifted) in pieces.iter().zip(lifted) {
            let Some(lifted) = lifted else {
                covered += 1;
                continue;
            };
            let mut c = tone(&plan.plazas[*pi].style);
            if bay_tones {
                // each bay's own tone (street-surface-v2.js `emitPavedPlazas`:
                // `1 + ((hash32('plaza:<id>:<A.k>:<C.k>:00') % 41) - 20) / 20 x plazaUnitTone` on
                // the style's sRGB tone), the bay found from the piece's centre in the plaza frame.
                let p = &plan.plazas[*pi];
                let m = crate::plaza::convex_centroid(piece);
                let (u, v) = (p.frame, [-p.frame[1], p.frame[0]]);
                let (a, cc) = (m[0] * u[0] + m[1] * u[1], m[0] * v[0] + m[1] * v[1]);
                let key = format!("plaza:{}:{}:{}:00", p.id, (a / bay).floor() as i64, (cc / bay).floor() as i64);
                let jitter = 1.0 + (((crate::plaza::hash32(&key) % 41) as f64 - 20.0) / 20.0) * os.plaza_unit_tone;
                let lin = |v: f64| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
                let srgb = |v: f64| if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
                c = c.map(|v| lin((srgb(v) * jitter).min(1.0)));
            }
            let base = positions.len() as u32;
            for p in lifted {
                positions.push(p);
                colours.push(c);
            }
            // A convex piece, wound either way: fan it facing up (the sign of
            // the whole ring's area, as a cut can leave its first three points in line).
            let up = (0..piece.len()).map(|i| {
                let (a, b) = (piece[i], piece[(i + 1) % piece.len()]);
                a[0] * b[1] - b[0] * a[1]
            }).sum::<f64>() < 0.0;
            for k in 1..piece.len() as u32 - 1 {
                if up {
                    indices.extend_from_slice(&[base, base + k, base + k + 1]);
                } else {
                    indices.extend_from_slice(&[base, base + k + 1, base + k]);
                }
            }
            laid += 1;
        }
        let mut mesh = flat_mesh("plaza-paving", &positions, &indices, [1.0; 3]);
        // flat_mesh unshares the vertices; give each its piece's tone.
        for (t, tri) in indices.chunks_exact(3).enumerate() {
            for (k, &i) in tri.iter().enumerate() {
                let c = colours[i as usize];
                let v = 3 * t + k;
                mesh.colors[3 * v..3 * v + 3].copy_from_slice(&[c[0] as f32, c[1] as f32, c[2] as f32]);
            }
        }
        lap(&mut stage_ms[4]);
        let report = PlazaReport { plazas: plan.plazas.len(), skipped: plan.skipped.clone(), children: plan.children.len(), pieces: laid, covered_by_street: covered, triangles: mesh.triangles(), ms: t0.elapsed().as_secs_f64() * 1e3, stage_ms };
        if !mesh.is_empty() {
            // The JS lays it in the street surface's path layer (concrete); a matte paving here.
            self.structures.push(Structure { cast: false, footway: true, kit: None, name: "plaza-paving".into(), material: plain_material(0.9, 0.0, "sidewalk"), mesh });
        }
        Ok(report)
    }

    /// The Ocean Beach and Sunset Dunes seawalls (promenade.rs) on the drawn
    /// land (`land_at`) and the drawn beach (`sand_at`). Returns (stations, stairs) of each.
    pub fn add_promenades(&mut self, land_at: &dyn Fn(f64, f64) -> f64, sand_at: &dyn Fn(f64, f64) -> f64) -> [(usize, usize); 2] {
        let js = LandmarksJs::get();
        let p = &js.promenades;
        let albedo = &p["structureAlbedo"];
        let mut out = [(0, 0); 2];
        if let Some(ob) = crate::promenade::ocean_beach(p, albedo, land_at, sand_at) {
            out[0] = (ob.stations, ob.stairs);
            // ocean-beach-seawall.js: concrete 0.88 (casts), planting 0.95, sand 1.0 (receive only).
            self.structures.push(Structure { cast: true, footway: false, kit: None, name: "ocean-beach-seawall:concrete".into(), material: plain_material(0.88, 0.0, "sidewalk"), mesh: ob.concrete });
            self.structures.push(Structure { cast: false, footway: false, kit: None, name: "ocean-beach-seawall:planting".into(), material: plain_material(0.95, 0.0, "sidewalk"), mesh: ob.planting });
            self.structures.push(Structure { cast: false, footway: false, kit: None, name: "ocean-beach-seawall:sand".into(), material: plain_material(1.0, 0.0, "sidewalk"), mesh: ob.sand });
        }
        if let Some(sd) = crate::promenade::sunset_dunes(p, albedo, land_at, sand_at) {
            out[1] = (sd.stations, sd.stairs);
            // sunset-dunes-seawall.js: structure concrete at roughness 0.9, casts.
            self.structures.push(Structure { cast: true, footway: false, kit: None, name: "sunset-dunes-seawall".into(), material: plain_material(0.9, 0.0, "sidewalk"), mesh: sd.concrete });
        }
        out
    }

    pub fn triangles(&self, role: Role) -> usize {
        self.landmarks.iter().flat_map(|l| &l.draws).filter(|d| d.role == role).map(|d| d.mesh.triangles()).sum()
    }
}
