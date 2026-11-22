//! The far body (`--car-detail`): every shipped traffic body's LOD 2,
//! simplified to `FAR_BODY_ERROR_M`, is a real car shape far lighter than the
//! LOD it came from, with the source's materials, wheel parts, bounds and
//! pivots, and only vertices the source had; the same source gives the same
//! body. Bodies whose files are not in this checkout are skipped.

use std::collections::BTreeSet;

use eo_agents::catalog::{AgentsJs, read_verified};
use eo_agents::vehicles::{CookedBody, FAR_BODY_ERROR_M, FAR_BODY_TRIANGLES, LampColours, box_body, cook_body, far_body};

fn lod2_bodies() -> Vec<(String, CookedBody)> {
    let paths = eo_data::DataPaths::discover().expect("repository root");
    let js = AgentsJs::load().expect("agents JS export");
    let lamps = LampColours::from_js(&js);
    js.vehicles
        .iter()
        .filter_map(|(id, b)| {
            let l = b.lods.get(2)?;
            let bytes = read_verified(&paths.repo_root, &l.asset()).ok()?;
            Some((id.clone(), cook_body(&bytes, &lamps).expect("cook LOD 2")))
        })
        .collect()
}

fn tags(b: &CookedBody, list: &[u32]) -> BTreeSet<u32> {
    list.iter().map(|&i| b.vertices[i as usize].tag).collect()
}

#[test]
fn far_bodies_are_light_car_shapes_from_their_source() {
    let bodies = lod2_bodies();
    assert!(!bodies.is_empty(), "no traffic body's LOD 2 is in this checkout");
    let boxed = box_body([-0.9, 0.0, -2.4], [0.9, 1.5, 2.4]).triangles;
    let (mut src_sum, mut far_sum) = (0u64, 0u64);
    for (id, src) in &bodies {
        let far = far_body(src, FAR_BODY_TRIANGLES, FAR_BODY_ERROR_M);
        println!("{id}: LOD 2 {} triangles -> far {} ({:.1} %), {} -> {} vertices", src.triangles, far.triangles, 100.0 * far.triangles as f64 / src.triangles as f64, src.vertices.len(), far.vertices.len());
        src_sum += src.triangles as u64;
        far_sum += far.triangles as u64;
        // lighter than its source, more than the three boxes.
        assert!(far.triangles < src.triangles, "{id}: {} not under LOD 2's {}", far.triangles, src.triangles);
        assert!(far.triangles > boxed, "{id}: {} triangles is no more than the box's {boxed}", far.triangles);
        assert_eq!(far.triangles as usize, (far.opaque.len() + far.glass.len()) / 3, "{id}: triangle count");
        // every index in range, every vertex one of the source's, every vertex used.
        let n = far.vertices.len() as u32;
        assert!(far.opaque.iter().chain(&far.glass).all(|&i| i < n), "{id}: index out of range");
        assert!(far.vertices.iter().all(|v| src.vertices.contains(v)), "{id}: a vertex the source does not have");
        let used: BTreeSet<u32> = far.opaque.iter().chain(&far.glass).copied().collect();
        assert_eq!(used.len(), far.vertices.len(), "{id}: unused vertices kept");
        // the source's materials and wheel parts, in the same lists (opaque stays opaque, glass glass).
        assert!(tags(&far, &far.opaque).is_subset(&tags(src, &src.opaque)), "{id}: opaque tags");
        assert!(tags(&far, &far.glass).is_subset(&tags(src, &src.glass)), "{id}: glass tags");
        assert_eq!(far.materials.len(), src.materials.len(), "{id}: material table");
        // the paint is still there (the instance colour reads it).
        let paint = |b: &CookedBody| b.opaque.iter().any(|&i| b.materials[(b.vertices[i as usize].tag & 0xffff) as usize].flags & eo_agents::M_PAINT != 0);
        assert_eq!(paint(&far), paint(src), "{id}: paint");
        // the source's frame.
        assert_eq!((far.min, far.max, far.pivots), (src.min, src.max, src.pivots), "{id}: bounds and pivots");
        // deterministic.
        let again = far_body(src, FAR_BODY_TRIANGLES, FAR_BODY_ERROR_M);
        assert_eq!((again.opaque, again.glass, again.vertices.len()), (far.opaque.clone(), far.glass.clone(), far.vertices.len()), "{id}: not deterministic");
    }
    println!("all bodies: LOD 2 {src_sum} triangles -> far {far_sum} ({:.1} %)", 100.0 * far_sum as f64 / src_sum as f64);
}
