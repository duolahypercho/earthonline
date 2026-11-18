//! The server's collision tiles: each 128 m tile is cooked once (heightfield,
//! building prisms, street surface) on a few worker threads, kept while any
//! player's world wants it, and inserted into every player's own Rapier
//! world as a copy that shares the shapes (`BuiltTile::clone`). Memory is
//! one cooked set for the area the players are in, not one per player.

use eo_player::world::{BuildingSet, BuiltTile, GroundProvider, TileFeed, TileKey, cook_tile};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

struct State {
    cooked: HashMap<TileKey, (Arc<BuiltTile>, Instant)>,
    queue: Vec<(TileKey, (f64, f64))>,
    busy: std::collections::HashSet<TileKey>,
    stop: bool,
    cook_ms: Vec<f64>,
}

pub struct SharedTiles {
    state: Arc<(Mutex<State>, Condvar)>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl SharedTiles {
    pub fn start(ground: Arc<dyn GroundProvider>, buildings: Arc<BuildingSet>, threads: usize) -> Arc<Self> {
        let state = Arc::new((Mutex::new(State { cooked: HashMap::new(), queue: vec![], busy: Default::default(), stop: false, cook_ms: vec![] }), Condvar::new()));
        let hs = (0..threads.max(1))
            .map(|k| {
                let (s, g, b) = (state.clone(), ground.clone(), buildings.clone());
                std::thread::Builder::new()
                    .name(format!("eo-server-tiles-{k}"))
                    .spawn(move || {
                        loop {
                            let key = {
                                let (m, cv) = &*s;
                                let mut st = m.lock().unwrap();
                                loop {
                                    if st.stop {
                                        return;
                                    }
                                    if let Some(i) = (0..st.queue.len()).min_by(|&a, &b| {
                                        let (ka, ca) = st.queue[a];
                                        let (kb, cb) = st.queue[b];
                                        ka.distance_to(ca.0, ca.1).total_cmp(&kb.distance_to(cb.0, cb.1))
                                    }) {
                                        let (k, _) = st.queue.swap_remove(i);
                                        st.busy.insert(k);
                                        break k;
                                    }
                                    st = cv.wait(st).unwrap();
                                }
                            };
                            let t = cook_tile(key, &*g, &b);
                            let (m, _) = &*s;
                            let mut st = m.lock().unwrap();
                            st.busy.remove(&key);
                            st.cook_ms.push(t.cook_ms);
                            st.cooked.insert(key, (Arc::new(t), Instant::now()));
                        }
                    })
                    .expect("spawn tile cooker")
            })
            .collect();
        Arc::new(Self { state, threads: hs })
    }

    /// Drop cooked tiles nobody fetched for `idle_s` (worlds holding them keep their copies).
    pub fn evict(&self, idle_s: f64) -> usize {
        let (m, _) = &*self.state;
        let mut st = m.lock().unwrap();
        let before = st.cooked.len();
        st.cooked.retain(|_, (_, t)| t.elapsed().as_secs_f64() < idle_s);
        before - st.cooked.len()
    }

    pub fn resident(&self) -> usize {
        self.state.0.lock().unwrap().cooked.len()
    }

    pub fn cook_ms(&self) -> Vec<f64> {
        self.state.0.lock().unwrap().cook_ms.clone()
    }
}

impl TileFeed for SharedTiles {
    fn fetch(&self, key: TileKey, center: (f64, f64)) -> Option<BuiltTile> {
        let (m, cv) = &*self.state;
        let mut st = m.lock().unwrap();
        if let Some((t, used)) = st.cooked.get_mut(&key) {
            *used = Instant::now();
            return Some((**t).clone());
        }
        if !st.busy.contains(&key) && !st.queue.iter().any(|q| q.0 == key) {
            st.queue.push((key, center));
            cv.notify_one();
        }
        None
    }
}

impl Drop for SharedTiles {
    fn drop(&mut self) {
        {
            let (m, cv) = &*self.state;
            m.lock().unwrap().stop = true;
            cv.notify_all();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}
