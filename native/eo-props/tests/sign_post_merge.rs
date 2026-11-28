//! One pole per spot (`signs::set_merge_posts`, eo-app `--no-sign-post-merge`): the SFMTA
//! inventory lists one physical pole's signs under several records, and each stood its own
//! post a few centimetres from the others. Merged, no two posts of a tile stand within the merge radius,
//! and every plate the unmerged plan drew is still drawn (on the pole that took it) but for a
//! repeat of a plate the pole already carries. Its own binary: the switch is process-wide.
use eo_props::{Model, Prop};

/// SoMa and downtown tiles, among them the sign capture card's STOP (1953.7, -622.1), which
/// stood three posts within 0.15 m.
const SITES: [[f64; 2]; 4] = [[1953.7, -622.1], [2274.0, -2379.0], [1600.0, -400.0], [509.5, 780.0]];

fn plan(w: &eo_props::PropsWorld, keys: &[(i32, i32)]) -> Vec<Prop> {
    keys.iter().flat_map(|&k| eo_props::signs::plan_tile(w, k)).collect()
}

#[test]
fn one_pole_per_spot_keeps_every_plate() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = eo_props::load_world(&paths).unwrap();
    let mut keys: Vec<(i32, i32)> = SITES
        .iter()
        .flat_map(|s| {
            let k = eo_street::tile_key(s[0], s[1]);
            (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (k.0 + dx, k.1 + dz)))
        })
        .collect();
    keys.sort_unstable();
    keys.dedup();
    assert!(!eo_props::signs::merge_posts(), "off in the library unless set");
    let off = plan(&w, &keys);
    eo_props::signs::set_merge_posts(true);
    let on = plan(&w, &keys);
    eo_props::signs::set_merge_posts(false);
    let posts = |v: &[Prop]| v.iter().filter(|p| p.model == Model::SignPost).map(|p| [p.pos[0], p.pos[2]]).collect::<Vec<_>>();
    let near = |v: &[[f64; 2]], r: f64| (0..v.len()).map(|i| v[i + 1..].iter().filter(|q| (q[0] - v[i][0]).hypot(q[1] - v[i][1]) <= r).count()).sum::<usize>();
    let (p_off, p_on) = (posts(&off), posts(&on));
    println!("posts {} -> {}, posts within 0.9 m of another {} -> {}", p_off.len(), p_on.len(), near(&p_off, 0.9), near(&p_on, 0.9));
    assert!(near(&p_off, 0.9) > 0, "the sample no longer shows stacked posts");
    // posts are planned per tile of their listed point: two whose listed points lie in
    // neighbouring tiles are not merged (0.7 % of the stacked posts city-wide, pole_gaps).
    assert!(near(&p_on, 0.9) * 20 <= near(&p_off, 0.9), "posts within the merge radius: {} of {}", near(&p_on, 0.9), near(&p_off, 0.9));
    assert!(p_on.len() < p_off.len());
    // every plate is still drawn: the same face, facing the same way (20 deg), within a metre of where it stood.
    let plates = |v: &[Prop]| v.iter().filter(|p| p.model == Model::SignPlate).copied().collect::<Vec<_>>();
    let (q_off, q_on) = (plates(&off), plates(&on));
    let gap = |a: f64, b: f64| {
        let d = (a - b).rem_euclid(std::f64::consts::TAU);
        d.min(std::f64::consts::TAU - d)
    };
    for p in &q_off {
        assert!(
            q_on.iter().any(|q| q.variant == p.variant && gap(q.yaw, p.yaw) <= 20f64.to_radians() && (q.pos[0] - p.pos[0]).hypot(q.pos[2] - p.pos[2]) <= 1.0),
            "plate {} at {:?} lost",
            p.variant,
            p.pos
        );
    }
    // and every plate hangs on a post: its foot on a post's face.
    let r = w.js.consts["signPostRadiusM"].as_f64().unwrap();
    for q in &q_on {
        assert!(p_on.iter().any(|p| ((q.pos[0] - p[0]).hypot(q.pos[2] - p[1]) - r).abs() < 1e-6), "plate {} at {:?} on no post", q.variant, q.pos);
    }
}
