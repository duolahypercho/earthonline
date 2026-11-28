//! Simulation regions on one server: the world is cut into square region
//! cells (`REGION_M`, keyed by zone and integer cell); a cell runs one
//! shared eo-sim `Region` while a player is near it.
//!
//! OWNERSHIP. A region owns the cars and walkers inside its cell. It
//! populates only inside its cell (its `Claim`), for every player within
//! `VIEW_M` of the cell (its viewers), at the single-player density scaled
//! by the share of each viewer's disc inside the cell, so two regions
//! meeting at a border hold one player's density between them. An agent
//! that drives or walks out of its cell into another active cell is HANDED
//! OFF: taken from the one region and adopted by the other with its id,
//! state and route intact (ids are unique per server: every region numbers
//! from its own block). Into an inactive cell it stays with its region.
//!
//! LIFETIME. A cell opens when a player comes within `VIEW_M` and its
//! network tile is resident; it opens WARM (every pop-in guard on) when the
//! player can already see another region's agents, cold for a player who
//! just arrived. It closes when no player has been within `KEEP_M` for
//! `CLOSE_S` (its agents are then beyond every client's interest radius).
//!
//! The network (lanes, footways, signals) is one eo-sim `Network` streamed
//! around every player on a builder thread; adding and dropping tiles is
//! applied between steps.

use eo_sim::network::TileKey as SimTile;
use eo_sim::region::{Claim, Focus, People, Region, RegionConfig};
use eo_sim::World;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

pub const REGION_M: f64 = 512.0;
/// A player within this of a cell is one of its viewers (the recycler's 120 m disc).
pub const VIEW_M: f64 = eo_sim::consts::LOCAL_LIFE_RADIUS;
/// A cell stays open while a player is within this (beyond every interest radius of its agents).
pub const KEEP_M: f64 = 320.0;
pub const CLOSE_S: f64 = 5.0;
/// The simulation network is resident within this of every player (cars
/// are drawn to 240 m and recycled from 240 m; their routes need road beyond).
pub const NET_RADIUS_M: f64 = 400.0;
/// Agents are handed off once this far inside another cell (no ping-pong on the line).
pub const HANDOFF_MARGIN_M: f64 = 2.0;
/// Ids per region block (cars and walkers each).
pub const ID_BLOCK: u32 = 1 << 16;
pub const ID_BLOCKS: u32 = (1 << 30) / ID_BLOCK;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionKey {
    pub zone: u16,
    pub x: i32,
    pub z: i32,
}

impl RegionKey {
    pub fn of(zone: u16, x: f64, z: f64) -> Self {
        RegionKey { zone, x: (x / REGION_M).floor() as i32, z: (z / REGION_M).floor() as i32 }
    }
    pub fn claim(&self) -> Claim {
        let (x0, z0) = (self.x as f64 * REGION_M, self.z as f64 * REGION_M);
        Claim { min: [x0, z0], max: [x0 + REGION_M, z0 + REGION_M] }
    }
}

/// What the regions need to know about a player each tick.
#[derive(Clone, Copy, Debug)]
pub struct Viewer {
    pub player: u32,
    pub x: f64,
    pub z: f64,
    pub yaw: f64,
}

pub struct Cell {
    pub key: RegionKey,
    pub car_block: u32,
    pub walker_block: u32,
    /// The last update (tick) a player was within `KEEP_M`.
    last_near: u64,
    pub opened: u64,
    pub warm: bool,
}

/// A network tile built on the builder thread, waiting to be admitted.
pub struct PendingBuild {
    build: eo_sim::NetworkBuild,
    tiles: Vec<SimTile>,
    ms: f64,
}

#[derive(Clone, Debug, Default)]
pub struct RegionStats {
    pub opened: u64,
    pub closed: u64,
    pub cars_handed_off: u64,
    pub walkers_handed_off: u64,
    pub net_tiles_added: u64,
    pub net_tiles_removed: u64,
    pub net_apply_ms_max: f64,
    /// `Network::add` alone (the rest of an apply is telling every region).
    pub net_add_ms_max: f64,
    pub net_build_ms_max: f64,
    pub step_wall_ms: f64,
    pub step_cpu_ms: f64,
    /// How far an agent's pose moved in the hand-off itself (take, adopt): must be 0.
    pub handoff_moved_m_max: f64,
    pub updates: u64,
}

pub struct Regions {
    pub zone: u16,
    pub world: World,
    /// Parallel to `world.regions`.
    pub cells: Vec<Cell>,
    source: Arc<eo_sim::native::NativeSource>,
    seed: u32,
    next_block: u32,
    req_tx: Sender<Vec<SimTile>>,
    build_rx: Receiver<(eo_sim::NetworkBuild, Vec<SimTile>, f64)>,
    requested: HashSet<SimTile>,
    last_drop: Instant,
    pub stats: RegionStats,
    _builder: std::thread::JoinHandle<()>,
}

fn cell_seed(k: RegionKey) -> u32 {
    let mut h = 0x811c_9dc5u32;
    for v in [k.zone as u32, k.x as u32, k.z as u32] {
        for b in v.to_le_bytes() {
            h = (h ^ b as u32).wrapping_mul(0x0100_0193);
        }
    }
    h
}

impl Regions {
    pub fn new(source: Arc<eo_sim::native::NativeSource>, seed: u32, threads: usize) -> Self {
        let mut world = World::new(source.new_network());
        world.set_threads(threads);
        let (req_tx, req_rx) = channel::<Vec<SimTile>>();
        let (build_tx, build_rx) = channel();
        let src = source.clone();
        let builder = std::thread::Builder::new()
            .name("eo-server-network".into())
            .spawn(move || {
                while let Ok(tiles) = req_rx.recv() {
                    let t = Instant::now();
                    let b = src.build(&tiles);
                    if build_tx.send((b, tiles, t.elapsed().as_secs_f64() * 1e3)).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn network builder");
        Self { zone: 0, world, cells: vec![], source, seed, next_block: 1, req_tx, build_rx, requested: HashSet::new(), last_drop: Instant::now(), stats: RegionStats::default(), _builder: builder }
    }

    pub fn active(&self) -> usize {
        self.cells.len()
    }

    /// Network tiles requested and not yet admitted.
    pub fn pending_tiles(&self) -> usize {
        self.requested.len()
    }

    pub fn find(&self, key: RegionKey) -> Option<usize> {
        self.cells.iter().position(|c| c.key == key)
    }

    fn alloc_block(&mut self, used: &HashSet<u32>) -> u32 {
        for _ in 0..ID_BLOCKS {
            let b = self.next_block;
            self.next_block = (self.next_block + 1) % ID_BLOCKS;
            if self.next_block == 0 {
                self.next_block = 1;
            }
            if !used.contains(&b) {
                return b;
            }
        }
        panic!("no free id block");
    }

    fn blocks_in_use(&self) -> HashSet<u32> {
        let mut s = HashSet::new();
        for (r, c) in self.world.regions.iter().zip(&self.cells) {
            s.insert(c.car_block);
            s.insert(c.walker_block);
            for car in &r.fleet.cars {
                s.insert(car.id / ID_BLOCK);
            }
            for w in &r.crowd.walkers {
                s.insert(w.key / ID_BLOCK);
            }
        }
        s
    }

    /// Stream the network around the players (asynchronously) and apply what has been built.
    /// The next built network tile, if one is ready (at most one is admitted a tick).
    pub fn take_build(&mut self) -> Option<PendingBuild> {
        self.build_rx.try_recv().ok().map(|(build, tiles, ms)| PendingBuild { build, tiles, ms })
    }

    /// Admit a built tile: `Network::add` plans its junctions (milliseconds for
    /// a dense tile), then every region re-decides what the change touched.
    /// The server runs this beside the players' step (they share nothing).
    pub fn apply_build(&mut self, p: PendingBuild) {
        let t = Instant::now();
        for k in &p.tiles {
            self.requested.remove(k);
        }
        if p.tiles.iter().any(|k| !self.world.net.tiles.contains_key(k)) {
            self.world.net.add(p.build);
            self.stats.net_add_ms_max = self.stats.net_add_ms_max.max(t.elapsed().as_secs_f64() * 1e3);
            self.stats.net_tiles_added += p.tiles.len() as u64;
            for r in self.world.regions.iter_mut() {
                r.network_changed(&self.world.net);
            }
        }
        self.stats.net_build_ms_max = self.stats.net_build_ms_max.max(p.ms);
        self.stats.net_apply_ms_max = self.stats.net_apply_ms_max.max(t.elapsed().as_secs_f64() * 1e3);
    }

    fn stream(&mut self, viewers: &[Viewer]) {
        let mut want: Vec<(f64, SimTile)> = vec![];
        for v in viewers {
            for k in self.source.resident_tiles(v.x, v.z, NET_RADIUS_M) {
                if !self.world.net.tiles.contains_key(&k) && !self.requested.contains(&k) {
                    let o = k.origin();
                    let tm = eo_sim::network::TILE_M;
                    want.push(((o[0] + tm / 2.0 - v.x).hypot(o[1] + tm / 2.0 - v.z), k));
                }
            }
        }
        if !want.is_empty() {
            // Nearest a player first, in pairs, so the tile under a player arrives soon.
            want.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for (_, k) in want {
                let mut delivery = self.source.delivery_tiles(k);
                delivery.retain(|tile| !self.world.net.tiles.contains_key(tile) && self.requested.insert(*tile));
                if !delivery.is_empty() {
                    let _ = self.req_tx.send(delivery);
                }
            }
        }
        // Drop far tiles, batched, at most every 2 s.
        if self.last_drop.elapsed().as_secs_f64() > 2.0 {
            self.last_drop = Instant::now();
            let keep: HashSet<_> = viewers.iter().flat_map(|v| self.source.resident_tiles(v.x, v.z, NET_RADIUS_M + eo_sim::network::TILE_M)).collect();
            let far: Vec<SimTile> = self
                .world
                .net
                .tiles
                .keys()
                .copied()
                .filter(|t| !keep.contains(t))
                .collect();
            if !far.is_empty() {
                let t = Instant::now();
                for k in &far {
                    self.world.net.remove_tile(*k);
                }
                for r in self.world.regions.iter_mut() {
                    r.network_changed(&self.world.net);
                }
                self.stats.net_tiles_removed += far.len() as u64;
                self.stats.net_apply_ms_max = self.stats.net_apply_ms_max.max(t.elapsed().as_secs_f64() * 1e3);
            }
        }
    }

    /// Open, feed and close cells for this tick's players, and hand agents across borders.
    pub fn update(&mut self, viewers: &[Viewer]) {
        self.stream(viewers);
        self.stats.updates += 1;
        let now = self.stats.updates;
        // Open: every cell within VIEW_M of a player whose network tile is resident.
        let mut used: Option<HashSet<u32>> = None;
        for v in viewers {
            if !self.world.net.tiles.contains_key(&SimTile::of(v.x, v.z)) {
                continue;
            }
            let lo = RegionKey::of(self.zone, v.x - VIEW_M, v.z - VIEW_M);
            let hi = RegionKey::of(self.zone, v.x + VIEW_M, v.z + VIEW_M);
            for x in lo.x..=hi.x {
                for z in lo.z..=hi.z {
                    let key = RegionKey { zone: self.zone, x, z };
                    if key.claim().distance(v.x, v.z) > VIEW_M || self.find(key).is_some() {
                        continue;
                    }
                    // Warm when this player already sees another region's agents.
                    let warm = self.cells.iter().any(|c| c.key.claim().distance(v.x, v.z) < KEEP_M);
                    let used = used.get_or_insert_with(|| self.blocks_in_use());
                    let car_block = self.alloc_block(used);
                    used.insert(car_block);
                    let walker_block = self.alloc_block(used);
                    used.insert(walker_block);
                    self.open(key, car_block, walker_block, warm, v);
                }
            }
        }
        // Viewers, focus, close.
        let mut close = vec![];
        for (i, c) in self.cells.iter_mut().enumerate() {
            let claim = c.key.claim();
            let r = &mut self.world.regions[i];
            let vs = r.viewers.get_or_insert_with(Vec::new);
            vs.clear();
            let mut near = false;
            for v in viewers {
                let d = claim.distance(v.x, v.z);
                if d <= VIEW_M {
                    vs.push(Focus { x: v.x, z: v.z, yaw: v.yaw });
                }
                near |= d <= KEEP_M;
            }
            if let Some(f) = vs.first() {
                r.focus = *f;
            }
            if near {
                c.last_near = now;
            } else if (now - c.last_near) as f64 > CLOSE_S * eo_net::proto::TICK_HZ as f64 {
                close.push(i);
            }
        }
        for i in close.into_iter().rev() {
            self.world.regions.swap_remove(i);
            self.cells.swap_remove(i);
            self.stats.closed += 1;
        }
        self.renumber_exhausted();
        self.handoff();
    }

    fn renumber_exhausted(&mut self) {
        let mut need = vec![];
        for (i, c) in self.cells.iter().enumerate() {
            let r = &self.world.regions[i];
            if r.fleet.next_id >= (c.car_block + 1) * ID_BLOCK {
                need.push((i, true));
            }
            if r.crowd.next_key >= (c.walker_block + 1) * ID_BLOCK {
                need.push((i, false));
            }
        }
        if need.is_empty() {
            return;
        }
        let mut used = self.blocks_in_use();
        for (i, cars) in need {
            let b = self.alloc_block(&used);
            used.insert(b);
            if cars {
                self.cells[i].car_block = b;
                self.world.regions[i].fleet.next_id = b * ID_BLOCK;
            } else {
                self.cells[i].walker_block = b;
                self.world.regions[i].crowd.next_key = b * ID_BLOCK;
            }
        }
    }

    fn open(&mut self, key: RegionKey, car_block: u32, walker_block: u32, warm: bool, v: &Viewer) {
        let mut cfg = RegionConfig::game_default(self.seed);
        cfg.vehicle_pool = 0;
        cfg.spawn_radius = Some(0.0);
        cfg.people = People::Bubble(eo_sim::consts::CrowdBubble::for_level(eo_sim::consts::DEFAULT_PEOPLE_LEVEL));
        let focus = Focus { x: v.x, z: v.z, yaw: v.yaw };
        let mut r = Region::with_id_base(&self.world.net, cell_seed(key), cfg, focus, car_block * ID_BLOCK);
        r.crowd.next_key = walker_block * ID_BLOCK;
        r.claim = Some(key.claim());
        r.viewers = Some(vec![focus]);
        if warm {
            r.warm();
        }
        self.world.regions.push(r);
        let now = self.stats.updates;
        self.cells.push(Cell { key, car_block, walker_block, last_near: now, opened: now, warm });
        self.stats.opened += 1;
    }

    /// Agents inside another active cell move to that cell's region.
    fn handoff(&mut self) {
        if self.cells.len() < 2 {
            return;
        }
        let index: HashMap<RegionKey, usize> = self.cells.iter().enumerate().map(|(i, c)| (c.key, i)).collect();
        let zone = self.zone;
        let target = |x: f64, z: f64, own: RegionKey| -> Option<usize> {
            let k = RegionKey::of(zone, x, z);
            if k == own {
                return None;
            }
            let c = k.claim();
            let inset = (x - c.min[0]).min(c.max[0] - x).min(z - c.min[1]).min(c.max[1] - z);
            if inset < HANDOFF_MARGIN_M {
                return None;
            }
            index.get(&k).copied()
        };
        let mut cars: Vec<(usize, usize, usize)> = vec![];
        let mut walkers: Vec<(usize, usize, usize)> = vec![];
        for (ri, r) in self.world.regions.iter().enumerate() {
            let own = self.cells[ri].key;
            for i in 0..r.fleet.len() {
                if let Some(t) = target(r.fleet.x[i], r.fleet.z[i], own) {
                    cars.push((ri, i, t));
                }
            }
            for i in 0..r.crowd.walkers.len() {
                if let Some(t) = target(r.crowd.x[i], r.crowd.z[i], own) {
                    walkers.push((ri, i, t));
                }
            }
        }
        // Take in descending index order per region (swap_remove keeps lower indices valid).
        cars.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        walkers.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        for (ri, i, t) in cars {
            let h = self.world.regions[ri].fleet.take(i);
            let before = [h.pose[0], h.pose[1]];
            let k = self.world.regions[t].fleet.adopt(&self.world.net, h);
            let f = &self.world.regions[t].fleet;
            self.stats.handoff_moved_m_max = self.stats.handoff_moved_m_max.max((f.x[k] - before[0]).hypot(f.z[k] - before[1]));
            self.stats.cars_handed_off += 1;
        }
        for (ri, i, t) in walkers {
            let (w, x, z) = self.world.regions[ri].crowd.take(i);
            self.world.regions[t].crowd.adopt(w, x, z);
            self.stats.walkers_handed_off += 1;
        }
    }

    /// One fixed step of every region (in parallel on the simulation pool).
    pub fn step(&mut self) {
        self.world.step();
        self.stats.step_wall_ms = self.world.last.wall_ns as f64 / 1e6;
        self.stats.step_cpu_ms = self.world.last.cpu_ns as f64 / 1e6;
    }

    pub fn agents(&self) -> (usize, usize) {
        self.world.regions.iter().fold((0, 0), |a, r| (a.0 + r.fleet.len(), a.1 + r.crowd.walkers.len()))
    }

    pub fn source(&self) -> &Arc<eo_sim::native::NativeSource> {
        &self.source
    }
}
