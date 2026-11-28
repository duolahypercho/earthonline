//! The structures by 128 m tile (`eo_street::tile_key`): every station step
//! (a piece of a chain between two stations of a level) belongs to the tile
//! of its midpoint, every bent to the tile of its station on the
//! centreline, every end face to the tile of its end; so every triangle is
//! in exactly one tile. Meshes are built on first asking and kept (shared,
//! thread-safe), as `StreetWorld::cached_mesh` keeps the street's.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use eo_street::{StreetWorld, tile_key};

use crate::mesh::{self, Builder};
use crate::section::Dims;
use crate::sweep::{self, Ctx, Draw, Sweep};
use crate::{Chain, ChainKind, StructureMesh, StructureSurface};

/// Meshes kept before the cache is emptied (as the street's, past this many).
const CACHE_MAX: usize = 4096;

/// What one tile holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Refs {
    /// Per level: (chain, first, last) positions into that chain's level
    /// station list, a run of steps in this tile.
    pub runs: [Vec<(u32, u32, u32)>; 3],
    /// (chain, bent index).
    pub bents: Vec<(u32, u32)>,
    /// (chain, end 0 or 1).
    pub ends: Vec<(u32, u8)>,
    pub merges: [Vec<crate::merge_surface::Patch>; 3],
    pub skyway: Vec<crate::skyway_lights::Standard>,
    pub east_steel: Vec<crate::east_span::Member>,
    /// Stage A9: (wall, first, last) section positions of a wall
    /// (`walls::Wall`), a run of its steps in this tile.
    pub walls: Vec<(u32, u32, u32)>,
}

/// Every sweep and the tiles they fall in.
#[derive(Debug, Default)]
pub(crate) struct Tiles {
    pub sweeps: Vec<Option<Sweep>>,
    pub tiles: BTreeMap<(i32, i32), Refs>,
    pub dims: Option<Dims>,
    pub openings: Arc<eo_street::road_openings::RoadOpenings>,
    /// Drawn chains with a piece whose section came from the fallback rule.
    pub fallback_chains: usize,
    // Actual bridge plates replace the approximate chain sweeps on these decks.
    landmark_decks: Vec<crate::LandmarkDeck>,
    landmark_surfaces: BTreeMap<(i32, i32), StructureSurface>,
    /// Per deck and covered tile, the collision tile and index of each roadway face.
    landmark_pavement: BTreeMap<(crate::LandmarkDeck, (i32, i32)), Vec<((i32, i32), usize)>>,
    /// Stage A9: the retaining walls where the joined street meets the
    /// structures (`walls.rs`; none on a street built without the joins).
    pub walls: Vec<crate::walls::Wall>,
    render: Mutex<HashMap<((i32, i32), u8), Arc<StructureMesh>>>,
    surface: Mutex<HashMap<(i32, i32), Arc<StructureSurface>>>,
}

/// What a chain draws: built decks everything, built open roads below
/// other ways their surface, landmark decks collision only, built tunnels
/// (only with `--tunnels`) their bores, cuts and portals (`tunnel.rs`).
fn draw_of(c: &Chain) -> Option<Draw> {
    if !c.is_built() || c.heights.len() != c.stations.len() || c.stations.len() < 2 {
        return None;
    }
    match c.kind {
        ChainKind::Deck => Some(Draw::Deck),
        ChainKind::Below => Some(Draw::AtGrade),
        ChainKind::Landmark => Some(Draw::Landmark),
        ChainKind::Tunnel => Some(Draw::Tunnel),
    }
}

impl Tiles {
    pub fn build(streets: &StreetWorld, chains: &[Chain], dims: Dims, class_lane: &dyn Fn(&str) -> f64, apron: &[u32]) -> Self {
        let ctx = Ctx { streets, chains, dims: &dims, class_lane, scale: eo_data::groundscale::scale() };
        let sweeps: Vec<Option<Sweep>> = chains.iter().map(|c| {
            if apron.contains(&c.id) { None } else { draw_of(c).map(|d| sweep::sweep(&ctx, c, d)) }
        }).collect();
        let mut tiles: BTreeMap<(i32, i32), Refs> = BTreeMap::new();
        let mut fallback_chains = 0;
        for (ci, sw) in sweeps.iter().enumerate() {
            let Some(sw) = sw else { continue };
            let fb = sw.sections.iter().filter(|s| s.fallback).count();
            if sw.draw != Draw::Landmark {
                fallback_chains += usize::from(fb > 0);
            }
            for lod in 0..3usize {
                let idx = &sw.levels[lod];
                let mut q = 0;
                while q + 1 < idx.len() {
                    let key = |q: usize| {
                        let (a, b) = (&sw.st[idx[q]], &sw.st[idx[q + 1]]);
                        tile_key((a.x + b.x) / 2.0, (a.z + b.z) / 2.0)
                    };
                    let k = key(q);
                    let mut e = q + 1;
                    while e + 1 < idx.len() && key(e) == k {
                        e += 1;
                    }
                    tiles.entry(k).or_default().runs[lod].push((ci as u32, q as u32, e as u32));
                    q = e;
                }
            }
            if sw.draw == Draw::Landmark {
                continue;
            }
            let c = &chains[ci];
            for (k, b) in c.supports.iter().enumerate() {
                if b.columns.is_empty() {
                    continue;
                }
                let f = c.frame(b.s);
                tiles.entry(tile_key(f.x, f.z)).or_default().bents.push((ci as u32, k as u32));
            }
            for e in 0..2u8 {
                // (a tunnel's ends are its open cut, met by the street.)
                if sw.ends[e as usize] == crate::EndKind::Shared || sw.draw == Draw::Tunnel {
                    continue;
                }
                let st = if e == 0 { &sw.st[0] } else { sw.st.last().unwrap() };
                tiles.entry(tile_key(st.x, st.z)).or_default().ends.push((ci as u32, e));
            }
        }
        crate::merge_surface::add(chains, &sweeps, &mut tiles);
        for pole in crate::skyway_lights::place(chains, &sweeps, &dims) {
            tiles.entry(tile_key(pole.foot[0], pole.foot[2])).or_default().skyway.push(pole);
        }
        for member in crate::east_span::steel(chains, streets) {
            let key = tile_key((member.a[0] + member.b[0]) / 2.0, (member.a[2] + member.b[2]) / 2.0);
            tiles.entry(key).or_default().east_steel.push(member);
        }
        let openings = crate::tunnel_clearance::openings(&sweeps, streets, chains);
        Self { openings: Arc::new(openings), sweeps, tiles, dims: Some(dims), fallback_chains, walls: vec![], landmark_decks: Vec::new(), landmark_surfaces: BTreeMap::new(), landmark_pavement: BTreeMap::new(), render: Mutex::new(HashMap::new()), surface: Mutex::new(HashMap::new()) }
    }

    pub fn has_landmark_roads(&self) -> bool {
        !self.landmark_decks.is_empty()
    }

    /// A real landmark draws its own panels across all lanes and owns their collision.
    /// Synthetic solves without landmark geometry retain the surveyed chain surface.
    pub fn install_landmark_roads(&mut self, chains: &[Chain], roads: Vec<eo_landmarks::bridge_roads::RoadSurface>, clear: &mut crate::clear::ClearIndex) {
        for road in roads {
            crate::merge_surface::bridge_ends(&road, chains, &self.sweeps, &mut self.tiles, clear);
            self.landmark_decks.push(road.deck);
            for (triangles, road_top) in [(road.road, true), (road.barrier, false)] {
                for (index, tri) in triangles.into_iter().enumerate() {
                    let key = tile_key((tri[0][0]+tri[1][0]+tri[2][0])/3., (tri[0][2]+tri[1][2]+tri[2][2])/3.);
                    self.tiles.entry(key).or_default();
                    let tile = self.landmark_surfaces.entry(key).or_default();
                    if road_top && index < road.carriageway_faces {
                        let keys = tri.map(|p| tile_key(p[0], p[2]));
                        let xs = keys.map(|p| p.0);
                        let zs = keys.map(|p| p.1);
                        for x in *xs.iter().min().unwrap()..=*xs.iter().max().unwrap() {
                            for z in *zs.iter().min().unwrap()..=*zs.iter().max().unwrap() {
                                self.landmark_pavement.entry((road.deck, (x, z))).or_default()
                                    .push((key, tile.street.len()));
                            }
                        }
                    }
                    if road_top { tile.street.push(tri); } else { tile.barrier.push(tri); }
                }
            }
        }
    }

    pub fn landmark_road_height(&self, deck: crate::LandmarkDeck, x: f64, z: f64) -> Option<f64> {
        self.landmark_pavement.get(&(deck, tile_key(x, z)))?.iter().find_map(|(owner, index)| {
            mesh::plan_height(&self.landmark_surfaces[owner].street[*index], x, z)
        })
    }

    /// Stage A9: add retaining walls, each step to the tile of its middle
    /// (as a chain's), before any mesh is built.
    pub fn add_walls(&mut self, walls: Vec<crate::walls::Wall>) {
        for w in walls {
            let wi = self.walls.len() as u32;
            let n = w.secs.len();
            let key = |q: usize| {
                let (a, b) = (w.mid(q), w.mid(q + 1));
                tile_key((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0)
            };
            let mut q = 0;
            while q + 1 < n {
                let k = key(q);
                let mut e = q + 1;
                while e + 1 < n && key(e) == k {
                    e += 1;
                }
                self.tiles.entry(k).or_default().walls.push((wi, q as u32, e as u32));
                q = e;
            }
            self.walls.push(w);
        }
        self.render.lock().unwrap().clear();
        self.surface.lock().unwrap().clear();
    }

    /// Every tile with anything to draw or collide with, sorted.
    pub fn keys(&self) -> Vec<(i32, i32)> {
        self.tiles.keys().copied().collect()
    }

    /// A merge or abutment patch may extend past the tile of its sweep's centre.
    pub fn join_deck_height(&self, chain: u32, x: f64, z: f64) -> Option<f64> {
        let (tx, tz) = tile_key(x, z);
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(refs) = self.tiles.get(&(tx + dx, tz + dz)) else { continue };
                if let Some(y) = refs.merges[0].iter().find_map(|p| p.height_on(chain, x, z)) {
                    return Some(y);
                }
            }
        }
        None
    }

    /// The render mesh of one tile at one level, built (not cached).
    pub fn build_mesh(&self, chains: &[Chain], key: (i32, i32), lod: u8) -> StructureMesh {
        let mut b = Builder::default();
        let (Some(refs), Some(dims)) = (self.tiles.get(&key), self.dims.as_ref()) else { return b.m };
        let lod = lod.min(2);
        let mut linings = Vec::new();
        for &(ci, q0, q1) in &refs.runs[lod as usize] {
            let Some(sw) = self.sweeps[ci as usize].as_ref() else { continue };
            if sw.draw == Draw::Landmark {
                continue;
            }
            let idx = &sw.levels[lod as usize][q0 as usize..=q1 as usize];
            let last = q1 as usize + 1 == sw.levels[lod as usize].len();
            b.era = sw.era;
            if sw.draw == Draw::Tunnel {
                let lining = crate::tunnel::run(&mut b, sw, &chains[ci as usize], idx, lod, dims);
                if !lining.is_empty() { linings.push((lining, ci)); }
                continue;
            }
            mesh::run(&mut b, sw, &chains[ci as usize], idx, lod, dims, last);
        }
        for patch in &refs.merges[lod as usize] { patch.draw(&mut b); }
        for &(ci, k) in &refs.bents {
            if let Some(sw) = self.sweeps[ci as usize].as_ref() {
                let c = &chains[ci as usize];
                b.era = sw.era;
                // a shared bent's twin: its sweep and its bent's station.
                let twin = c.supports[k as usize].twin.and_then(|t| {
                    let tsw = self.sweeps.get(t.chain as usize)?.as_ref()?;
                    let ts = chains.get(t.chain as usize)?.supports.get(t.bent as usize)?.s;
                    Some((tsw, ts))
                });
                mesh::bent(&mut b, sw, c, k as usize, lod, dims, twin);
            }
        }
        crate::east_span::draw(&refs.east_steel, &mut b, false);
        crate::skyway_lights::draw(&mut b, &refs.skyway, lod);
        for &(ci, e) in &refs.ends {
            if let Some(sw) = self.sweeps[ci as usize].as_ref() {
                b.era = sw.era;
                mesh::end(&mut b, sw, e as usize, lod, dims);
            }
        }
        for &(wi, q0, q1) in &refs.walls {
            if let Some(w) = self.walls.get(wi as usize) {
                crate::walls::draw(&mut b, w, q0 as usize, q1 as usize, lod, dims);
            }
        }
        crate::tunnel_clearance::clip_concrete(&mut b.m, &self.openings, &linings, chains);
        b.m
    }

    /// The render mesh of one tile at one level, shared (built once, kept).
    pub fn mesh(&self, chains: &[Chain], key: (i32, i32), lod: u8) -> Option<Arc<StructureMesh>> {
        self.tiles.get(&key)?;
        let lod = lod.min(2);
        if let Some(m) = self.render.lock().unwrap().get(&(key, lod)) {
            return Some(m.clone());
        }
        let m = Arc::new(self.build_mesh(chains, key, lod));
        let mut cache = self.render.lock().unwrap();
        if cache.len() >= CACHE_MAX {
            cache.clear();
        }
        Some(cache.entry((key, lod)).or_insert(m).clone())
    }

    /// The collision triangles of one tile: the near level's deck surface
    /// and concrete, and the landmark decks' surface.
    pub fn build_surface(&self, chains: &[Chain], key: (i32, i32)) -> StructureSurface {
        let mut out = self.landmark_surfaces.get(&key).cloned().unwrap_or_default();
        let (Some(refs), Some(dims)) = (self.tiles.get(&key), self.dims.as_ref()) else { return out };
        if let Some(m) = self.mesh(chains, key, 0) {
            mesh::collide(&m, &mut out.street, &mut out.barrier);
        }
        let mut b = Builder::default();
        if !refs.east_steel.is_empty() || !refs.skyway.is_empty() {
            crate::east_span::draw(&refs.east_steel, &mut b, true);
            for pole in &refs.skyway {
                crate::skyway_lights::shaft(&mut b, pole);
            }
            for tri in b.m.indices.chunks_exact(3) {
                out.barrier.push([tri[0], tri[1], tri[2]].map(|i| b.m.positions[i as usize].map(f64::from)));
            }
            b = Builder::default();
        }
        for &(ci, q0, q1) in &refs.runs[0] {
            let Some(sw) = self.sweeps[ci as usize].as_ref() else { continue };
            if sw.draw != Draw::Landmark || chains[ci as usize].landmark.is_some_and(|id| self.landmark_decks.contains(&id)) {
                continue;
            }
            let idx = &sw.levels[0][q0 as usize..=q1 as usize];
            mesh::run(&mut b, sw, &chains[ci as usize], idx, 0, dims, false);
        }
        mesh::collide(&b.m, &mut out.street, &mut out.barrier);
        out
    }

    pub fn surface(&self, chains: &[Chain], key: (i32, i32)) -> Option<Arc<StructureSurface>> {
        self.tiles.get(&key)?;
        if let Some(s) = self.surface.lock().unwrap().get(&key) {
            return Some(s.clone());
        }
        let s = Arc::new(self.build_surface(chains, key));
        let mut cache = self.surface.lock().unwrap();
        if cache.len() >= CACHE_MAX {
            cache.clear();
        }
        Some(cache.entry(key).or_insert(s).clone())
    }
}

#[cfg(test)]
mod landmark_height_tests {
    use super::*;
    use crate::LandmarkDeck::{BayBridgeLower, BayBridgeUpper};
    use eo_landmarks::bridge_roads::RoadSurface;

    #[test]
    fn bridge_height_follows_the_roadway_across_tiles_and_keeps_the_decks_separate() {
        let panel = |y| [[-12., y, 0.], [6., y, 0.], [6., y, 12.]];
        let roads = [(BayBridgeUpper, 19.), (BayBridgeLower, 12.)].map(|(deck, y)| RoadSurface {
            deck,
            road: vec![panel(y), [[9., y, 2.], [11., y, 2.], [11., y, 8.]]],
            carriageway_faces: 1,
            barrier: vec![],
        });
        let mut tiles = Tiles::default();
        tiles.install_landmark_roads(&[], Vec::from(roads), &mut crate::clear::ClearIndex::default());
        // The centroid belongs to the positive tile. Both sides of x=0 carry traffic.
        for x in [-1., 1.] {
            assert!((tiles.landmark_road_height(BayBridgeUpper, x, 2.).unwrap() - 19.).abs() < 1e-9);
            assert!((tiles.landmark_road_height(BayBridgeLower, x, 2.).unwrap() - 12.).abs() < 1e-9);
        }
        assert_eq!(tiles.landmark_road_height(BayBridgeUpper, 10., 3.), None);
    }
}
