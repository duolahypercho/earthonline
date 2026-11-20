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
    /// Drawn chains with a piece whose section came from the fallback rule.
    pub fallback_chains: usize,
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
    pub fn build(streets: &StreetWorld, chains: &[Chain], dims: Dims, class_lane: &dyn Fn(&str) -> f64) -> Self {
        let ctx = Ctx { streets, chains, dims: &dims, class_lane, scale: eo_data::groundscale::scale() };
        let sweeps: Vec<Option<Sweep>> = chains.iter().map(|c| draw_of(c).map(|d| sweep::sweep(&ctx, c, d))).collect();
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
        Self { sweeps, tiles, dims: Some(dims), fallback_chains, walls: vec![], render: Mutex::new(HashMap::new()), surface: Mutex::new(HashMap::new()) }
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

    /// The render mesh of one tile at one level, built (not cached).
    pub fn build_mesh(&self, chains: &[Chain], key: (i32, i32), lod: u8) -> StructureMesh {
        let mut b = Builder::default();
        let (Some(refs), Some(dims)) = (self.tiles.get(&key), self.dims.as_ref()) else { return b.m };
        let lod = lod.min(2);
        for &(ci, q0, q1) in &refs.runs[lod as usize] {
            let Some(sw) = self.sweeps[ci as usize].as_ref() else { continue };
            if sw.draw == Draw::Landmark {
                continue;
            }
            let idx = &sw.levels[lod as usize][q0 as usize..=q1 as usize];
            let last = q1 as usize + 1 == sw.levels[lod as usize].len();
            b.era = sw.era;
            if sw.draw == Draw::Tunnel {
                crate::tunnel::run(&mut b, sw, &chains[ci as usize], idx, lod, dims);
                continue;
            }
            mesh::run(&mut b, sw, &chains[ci as usize], idx, lod, dims, last);
        }
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
        let mut out = StructureSurface::default();
        let (Some(refs), Some(dims)) = (self.tiles.get(&key), self.dims.as_ref()) else { return out };
        if let Some(m) = self.mesh(chains, key, 0) {
            mesh::collide(&m, &mut out.street, &mut out.barrier);
        }
        let mut b = Builder::default();
        for &(ci, q0, q1) in &refs.runs[0] {
            let Some(sw) = self.sweeps[ci as usize].as_ref() else { continue };
            if sw.draw != Draw::Landmark {
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
