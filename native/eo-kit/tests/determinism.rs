//! Same record, same seed: same plan and same bytes, on any thread, in any order.

use eo_kit::city::KitCity;
use eo_kit::pipeline::{build_rung, plan_building};

fn mesh_bytes(kc: &KitCity, i: usize) -> Vec<u8> {
    let p = plan_building(&kc.records[i], &kc.world, i as u32, 12.5).expect("plans");
    let mut out = vec![];
    for rung in [0, 1, 2] {
        let m = build_rung(&p, rung);
        for (name, tint, sink) in &m.groups {
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&tint.unwrap_or(u32::MAX).to_le_bytes());
            for v in &sink.positions {
                for c in v {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            for n in &sink.normals {
                for c in n {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            for i in &sink.indices {
                out.extend_from_slice(&i.to_le_bytes());
            }
        }
    }
    out
}

#[test]
fn same_record_gives_the_same_bytes_on_every_thread_and_order() {
    let root = eo_kit::repo_root().expect("repo root");
    let (_city, kc) = KitCity::load(&root).expect("city");
    // Every 997th non-excluded building: all archetypes appear.
    let picks: Vec<usize> = (0..kc.records.len()).step_by(997).filter(|i| !kc.excluded.contains(i)).collect();
    let first: Vec<Vec<u8>> = picks.iter().map(|&i| mesh_bytes(&kc, i)).collect();
    // Again, reversed, on 8 threads.
    let second: Vec<(usize, Vec<u8>)> = std::thread::scope(|s| {
        let chunks: Vec<Vec<usize>> = picks.iter().rev().copied().collect::<Vec<_>>().chunks(picks.len().div_ceil(8)).map(|c| c.to_vec()).collect();
        let hs: Vec<_> = chunks.into_iter().map(|c| s.spawn({ let kc = &kc; move || c.into_iter().map(|i| (i, mesh_bytes(kc, i))).collect::<Vec<_>>() })).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let mut differing = 0;
    for (i, bytes) in &second {
        let k = picks.iter().position(|p| p == i).unwrap();
        if &first[k] != bytes {
            differing += 1;
        }
    }
    let total: usize = first.iter().map(|b| b.len()).sum();
    eprintln!("{} buildings, {} bytes of meshes, {differing} differ", picks.len(), total);
    assert!(picks.len() > 150);
    assert_eq!(differing, 0);
}
