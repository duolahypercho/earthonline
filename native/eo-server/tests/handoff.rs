//! Hand-off between two regions on one server: a player walks from one
//! region cell into the next. The second cell opens before the player
//! reaches it, cars and walkers crossing the line move to the region that
//! owns the ground they are on with their ids and states intact (no jump at
//! the hand-off), no id is ever held by two regions, nothing near the player
//! vanishes at the line, and the first cell closes once the player has left
//! it behind.

use eo_server::regions::{CLOSE_S, REGION_M, RegionKey, Regions, Viewer};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One server tick of the regions, with the network builder given real time (as at 60 Hz).
fn tick(regions: &mut Regions, v: Viewer) {
    regions.update(&[v]);
    let t = Instant::now();
    loop {
        if let Some(p) = regions.take_build() {
            regions.apply_build(p);
            break;
        }
        if regions.pending_tiles() == 0 || t.elapsed() > Duration::from_millis(30) {
            break;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
    regions.step();
}

#[test]
fn a_player_crossing_a_region_border_hands_off_and_the_world_is_continuous() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let source = Arc::new(eo_sim::native::load(&paths).unwrap());
    let mut regions = Regions::new(source, 1, 3);
    // Along Folsom Street, across x = 1536 (the line between cells 2 and 3).
    let border = 3.0 * REGION_M;
    let (z, x0, x1) = (-366.0, border - 260.0, border + 330.0);
    let mut v = Viewer { player: 1, x: x0, z, yaw: std::f64::consts::FRAC_PI_2 };
    let a = RegionKey::of(0, x0, z);
    let b = RegionKey::of(0, x1, z);
    assert_ne!(a, b);
    // The network streams in, the first cell opens and fills (20 s).
    for _ in 0..1200 {
        tick(&mut regions, v);
    }
    assert!(regions.find(a).is_some(), "the player's cell is open");
    let (cars0, walkers0) = regions.agents();
    assert!(cars0 > 15 && walkers0 > 15, "the first cell is populated: {cars0} cars, {walkers0} walkers");
    // Where each agent was last step: (region key, position).
    let mut last: HashMap<(bool, u32), (RegionKey, [f64; 2])> = HashMap::new();
    let mut b_opened_at: Option<f64> = None;
    let mut a_closed_at: Option<f64> = None;
    let (mut worst_vanish, mut handoffs_seen, mut handoff_jump_max) = (0usize, 0usize, 0.0f64);
    let mut crossed_a_to_b = 0usize;
    let steps = ((x1 - x0) / 1.4 * 60.0) as usize; // walking pace
    for s in 0..steps + (CLOSE_S as usize + 6) * 60 {
        if s < steps {
            v.x = x0 + (x1 - x0) * s as f64 / steps as f64;
        }
        tick(&mut regions, v);
        if b_opened_at.is_none() && regions.find(b).is_some() {
            b_opened_at = Some(v.x);
        }
        if a_closed_at.is_none() && regions.find(a).is_none() {
            a_closed_at = Some(v.x);
        }
        let mut seen: HashMap<(bool, u32), (RegionKey, [f64; 2])> = HashMap::new();
        for (ri, r) in regions.world.regions.iter().enumerate() {
            let key = regions.cells[ri].key;
            for (i, c) in r.fleet.cars.iter().enumerate() {
                assert!(seen.insert((true, c.id), (key, [r.fleet.x[i], r.fleet.z[i]])).is_none(), "car {} held by two regions", c.id);
            }
            for (i, w) in r.crowd.walkers.iter().enumerate() {
                assert!(seen.insert((false, w.key), (key, [r.crowd.x[i], r.crowd.z[i]])).is_none(), "walker {} held by two regions", w.key);
            }
        }
        let mut vanished = 0;
        for (id, (k0, p)) in &last {
            match seen.get(id) {
                None if (p[0] - v.x).hypot(p[1] - v.z) < 60.0 => vanished += 1,
                Some((k1, q)) if k1 != k0 => {
                    // A hand-off: same id; it went to the region that owns the ground it is on
                    // (unless the new region recycled it far away in the same tick: a donor 240 m from everyone).
                    handoffs_seen += 1;
                    let moved = (q[0] - p[0]).hypot(q[1] - p[1]);
                    if moved < 5.0 {
                        handoff_jump_max = handoff_jump_max.max(moved);
                        assert_eq!(RegionKey::of(0, p[0], p[1]), *k1, "handed to the region that owns the ground it was on");
                    }
                    if *k0 == a && *k1 == b {
                        crossed_a_to_b += 1;
                    }
                }
                _ => {}
            }
        }
        worst_vanish = worst_vanish.max(vanished);
        last = seen;
    }
    let st = regions.stats.clone();
    println!(
        "cell {b:?} opened with the player at x = {:.0} (border {border}); cell {a:?} closed at x = {:?}; handed off: {} cars, {} walkers ({handoffs_seen} seen, {crossed_a_to_b} from A to B, largest move across a hand-off {handoff_jump_max:.3} m); worst vanished near the player in one step {worst_vanish}",
        b_opened_at.unwrap_or(f64::NAN),
        a_closed_at.map(|x| x.round()),
        st.cars_handed_off,
        st.walkers_handed_off
    );
    let opened = b_opened_at.expect("the second cell never opened");
    assert!(opened < border - 60.0, "the second cell must open before the player reaches it (opened at {opened})");
    let closed = a_closed_at.expect("the first cell never closed after the player left it");
    assert!(closed > border + 300.0, "the first cell closed too early (player at {closed})");
    assert!(crossed_a_to_b > 0, "no agent was seen moving from A to B");
    assert!(handoff_jump_max < 1.0, "an agent moved {handoff_jump_max} m across its hand-off");
    assert!(st.handoff_moved_m_max < 1e-6, "an agent's pose changed in the hand-off itself by {} m", st.handoff_moved_m_max);
    assert_eq!(worst_vanish, 0, "agents near the player vanished");
}
