//! (`--crowd`) The people's bodies on the ground: kept apart and out of the
//! vehicles (`feed::separate`), and the contact anchors each person model
//! carries for the renderer's ground occlusion (`people::add_contact_anchors`).

use eo_agents::feed::{BODY_RADIUS_M, PERSONAL_RADIUS_M, keep_personal_space, separate};

#[test]
fn bodies_are_kept_apart_and_out_of_vehicles() {
    // a clump: nine people within a metre, two on the same spot.
    let mut pos: Vec<[f64; 2]> = (0..9).map(|i| [(i % 3) as f64 * 0.3, (i / 3) as f64 * 0.3]).collect();
    pos.push(pos[4]);
    let before = pos.clone();
    separate(&mut pos, &[]);
    let mut worst = f64::INFINITY;
    for i in 0..pos.len() {
        for j in i + 1..pos.len() {
            worst = worst.min((pos[i][0] - pos[j][0]).hypot(pos[i][1] - pos[j][1]));
        }
    }
    assert!(worst > 1.5 * BODY_RADIUS_M, "closest pair {worst:.2} m apart");
    // nobody thrown far: the clump spreads, it does not scatter.
    for (a, b) in pos.iter().zip(&before) {
        assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1.5, "moved {:.2} m", (a[0] - b[0]).hypot(a[1] - b[1]));
    }
    // two people well apart are not touched.
    let mut two = vec![[0.0, 0.0], [2.0, 0.0]];
    separate(&mut two, &[]);
    assert_eq!(two, vec![[0.0, 0.0], [2.0, 0.0]]);
    // a person inside a bus's footprint (heading +z, half length 6, half width 1.3) is put beside it, the nearer side.
    let bus = (10.0, 10.0, 0.0, 1.0, 6.0, 1.3);
    let mut p = vec![[10.8, 13.0]];
    separate(&mut p, &[bus]);
    assert!((p[0][0] - (10.0 + 1.3 + BODY_RADIUS_M)).abs() < 1e-9 && (p[0][1] - 13.0).abs() < 1e-9, "{:?}", p[0]);
    // one pressed to its front bumper steps out ahead of it.
    let mut q = vec![[10.2, 15.9]];
    separate(&mut q, &[bus]);
    assert!((q[0][1] - (10.0 + 6.0 + BODY_RADIUS_M)).abs() < 1e-9, "{:?}", q[0]);
}

/// With a copy at EO_PEOPLE_ROOT: every person model gets its anchors, in
/// the bind pose's places (heels and toes at the ground, the pelvis at hip
/// height, the left limbs on the left), each a body part's size.
#[test]
fn contact_anchors_stand_where_the_body_is() {
    if std::env::var_os(eo_agents::rocketbox::ROOT_ENV).is_none() {
        eprintln!("no copy at {}; skipped", eo_agents::rocketbox::ROOT_ENV);
        return;
    }
    let repo = eo_data::DataPaths::discover().unwrap().repo_root;
    let people = eo_agents::library::PeopleSource::from_env();
    // the shared cache when there is one (a cook from the GLBs takes minutes), with the hyper tier's far bodies
    let cache = eo_assets::default_cache_dir();
    let opts = eo_agents::library::CookOptions { far_body: true, ..Default::default() };
    let mut lib = eo_agents::library::Library::load_opts(&repo, Some(&cache), &people, opts).unwrap();
    let bones = lib.bones.len();
    let pads: Vec<[u32; 4]> = lib.models.iter().map(|m| m.pad).collect();
    assert!(pads.iter().all(|p| *p == [0; 4]), "nothing before");
    eo_agents::people::add_contact_anchors(&mut lib);
    let n = eo_agents::people::CONTACT_JOINTS.len();
    let people_models = lib.meta.people_models.clone();
    assert!(!people_models.is_empty());
    assert_eq!(lib.bones.len(), bones + people_models.len() * n, "appended after the matrices");
    for &m in &people_models {
        let model = lib.models[m as usize];
        assert_eq!(model.pad[1] as usize, n);
        let a: Vec<[f32; 4]> = (0..n).map(|k| lib.bones[model.pad[0] as usize + k]).collect();
        let (heels, toes, shins, thighs, pelvis) = ([a[0], a[2]], [a[1], a[3]], [a[4], a[5]], [a[6], a[7]], a[8]);
        for f in heels.iter().chain(&toes) {
            assert!(f[1] < 0.2 && (0.02..0.12).contains(&f[3]), "a foot part {f:?}");
        }
        for t in toes {
            assert!(t[2] > heels[0][2].min(heels[1][2]) - 0.05, "toes ahead of the heels (+z)");
        }
        // (a robe or a long skirt hangs from the legs: its hem's vertices ride the calves)
        assert!(shins.iter().all(|s| (0.1..0.7).contains(&s[1]) && s[3] < 0.3) && thighs.iter().all(|t| (0.45..1.0).contains(&t[1]) && t[3] < 0.3), "{shins:?} {thighs:?}");
        assert!(shins.iter().zip(&thighs).all(|(s, t)| s[1] < t[1] && t[1] < pelvis[1] + 0.1), "shins under thighs under the pelvis");
        assert!((0.8..1.2).contains(&pelvis[1]) && pelvis[3] < 0.3, "pelvis {pelvis:?}");
        // left and right on opposite sides of the body's middle.
        assert!((heels[0][0] - pelvis[0]) * (heels[1][0] - pelvis[0]) < 0.0, "the feet either side");
    }
}

/// (`--crowd`) a walker over the landmarks' paving (`feed::Decks`) stands on
/// its top; off it, or over a steep face, nothing is found.
#[test]
fn decks_lift_walkers_onto_their_paving() {
    use eo_agents::feed::Decks;
    let quad = |y0: f32, y1: f32| vec![[[0.0, y0, 0.0], [10.0, y0, 0.0], [10.0, y1, 8.0]], [[0.0, y0, 0.0], [10.0, y1, 8.0], [0.0, y1, 8.0]]];
    let mut tris = quad(2.0, 2.4);
    // a wall (steep) over the same ground
    tris.push([[0.0, 0.0, 4.0], [10.0, 0.0, 4.0], [10.0, 3.0, 4.01]]);
    let d = Decks::new(tris);
    assert_eq!(d.len(), 2, "the wall is not stood on");
    let y = d.top(5.0, 4.0).unwrap();
    assert!((y - 2.2).abs() < 1e-4, "{y}");
    assert!(d.top(-0.5, 4.0).is_none() && d.top(5.0, 8.5).is_none());
    // a deck across a cell boundary (16 m cells) is found from either side
    let far = Decks::new(vec![[[14.0, 1.0, 0.0], [18.0, 1.0, 0.0], [18.0, 1.0, 4.0]]]);
    assert_eq!(far.top(17.5, 1.0), Some(1.0));
    assert_eq!(far.top(15.0, 0.5), Some(1.0));
}

#[test]
fn overlapping_decks_keep_a_walker_on_the_reachable_level() {
    use eo_agents::feed::Decks;
    let tri = |y| [[0.0, y, 0.0], [10.0, y, 0.0], [0.0, y, 10.0]];
    let decks = Decks::new(vec![tri(5.0), tri(1.0)]);
    assert_eq!(decks.support(2.0, 2.0, 1.3), Some(1.0));
    assert_eq!(decks.support(2.0, 2.0, 5.3), Some(5.0));
    assert_eq!(decks.support(2.0, 2.0, 0.3), None);
}

/// (`--people-capsules`) A capsule fitted to a limb's surface: its axis along
/// the limb, its radius the limb's, its ends drawn in from the limb's ends by
/// that radius; a part shorter than its width a sphere.
#[test]
fn capsules_fit_a_limbs_surface() {
    use eo_agents::people::fit_capsule;
    // a tilted cylinder's surface, radius 0.06, 0.4 m long, rings every 2 cm
    let axis = glam::Vec3::new(0.2, 1.0, 0.1).normalize();
    let side = axis.cross(glam::Vec3::X).normalize();
    let up = axis.cross(side);
    let base = glam::Vec3::new(0.1, 0.5, -0.05);
    let mut pts = vec![];
    for i in 0..=20 {
        for k in 0..16 {
            let a = k as f32 / 16.0 * std::f32::consts::TAU;
            pts.push((base + axis * (i as f32 * 0.02) + (side * a.cos() + up * a.sin()) * 0.06).to_array());
        }
    }
    let (a, b, r) = fit_capsule(&pts);
    let (a, b) = (glam::Vec3::from(a), glam::Vec3::from(b));
    assert!((r - 0.06).abs() < 1e-3, "radius {r}");
    assert!((b - a).normalize().dot(axis).abs() > 0.999, "along the limb");
    // the ends within a ring of the cylinder's ends, drawn in by the radius
    let (lo, hi) = (a.min(b), a.max(b));
    let (e0, e1) = (base + axis * 0.06, base + axis * (0.4 - 0.06));
    assert!((lo.distance(e0.min(e1)) < 0.03) && (hi.distance(e0.max(e1)) < 0.03), "{a} {b}");
    // a round part (a ball's surface, radius 0.1: RMS distance from any axis
    // through it sqrt(2/3) x 0.1) is a capsule hardly longer than a sphere, at its middle
    let ball: Vec<[f32; 3]> = (0..400).map(|i| {
        let z = 1.0 - (i as f32 + 0.5) / 200.0;
        let (s, t) = ((1.0 - z * z).sqrt(), i as f32 * 2.399963);
        [0.1 * s * t.cos(), 1.6 + 0.1 * z, 0.1 * s * t.sin()]
    }).collect();
    let (a, b, r) = fit_capsule(&ball);
    let (a, b) = (glam::Vec3::from(a), glam::Vec3::from(b));
    assert!((r - 0.0816).abs() < 0.003, "{r}");
    assert!(a.distance(b) < 0.05 && ((a + b) * 0.5).distance(glam::Vec3::new(0.0, 1.6, 0.0)) < 0.005, "{a} {b}");
    // a single point: a sphere of no size there
    let (a, b, r) = fit_capsule(&[[0.3, 1.0, 0.2]]);
    assert!(a == b && a == [0.3, 1.0, 0.2] && r == 0.0);
    assert_eq!(fit_capsule(&[]).2, 0.0, "nothing carried hides nothing");
}

/// With a copy at EO_PEOPLE_ROOT: every person model gets thirteen capsules
/// in the bind pose's places (the shoes at the ground, the shins under the
/// thighs under the hips, the chest over them and the head on top, the arms
/// either side), each its part's size, appended after the matrices.
#[test]
fn body_capsules_stand_where_the_body_is() {
    if std::env::var_os(eo_agents::rocketbox::ROOT_ENV).is_none() {
        eprintln!("no copy at {}; skipped", eo_agents::rocketbox::ROOT_ENV);
        return;
    }
    let repo = eo_data::DataPaths::discover().unwrap().repo_root;
    let people = eo_agents::library::PeopleSource::from_env();
    let cache = eo_assets::default_cache_dir();
    let opts = eo_agents::library::CookOptions { far_body: true, ..Default::default() };
    let mut lib = eo_agents::library::Library::load_opts(&repo, Some(&cache), &people, opts).unwrap();
    let bones = lib.bones.len();
    eo_agents::people::add_capsule_anchors(&mut lib);
    let n = eo_agents::people::CAPSULE_JOINTS.len();
    let people_models = lib.meta.people_models.clone();
    assert!(!people_models.is_empty());
    assert_eq!(lib.bones.len(), bones + people_models.len() * 2 * n, "appended after the matrices");
    for &m in &people_models {
        let model = lib.models[m as usize];
        assert_eq!(model.pad[1] as usize, n);
        let cap = |k: usize| {
            let (a, b) = (lib.bones[model.pad[0] as usize + 2 * k], lib.bones[model.pad[0] as usize + 2 * k + 1]);
            ([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5], a[3])
        };
        let ((shoe_l, rs), (shoe_r, _)) = (cap(0), cap(1));
        assert!(shoe_l[1] < 0.15 && shoe_r[1] < 0.15 && (0.02..0.1).contains(&rs), "shoes {shoe_l:?} {rs}");
        let (shin, thigh, hips, chest, head) = (cap(2).0, cap(4).0, cap(6).0, cap(7).0, cap(8).0);
        assert!(shin[1] < thigh[1] && thigh[1] < hips[1] + 0.1 && hips[1] < chest[1] && chest[1] < head[1], "{shin:?} {thigh:?} {hips:?} {chest:?} {head:?}");
        assert!((1.35..2.0).contains(&head[1]), "head {head:?}");
        for k in 0..n {
            assert!(cap(k).1 < 0.3, "part {k} radius {}", cap(k).1);
        }
        // the arms either side of the chest
        let (ua_l, ua_r) = (cap(9).0, cap(10).0);
        assert!((ua_l[0] - chest[0]) * (ua_r[0] - chest[0]) < 0.0, "the arms either side");
    }
}

/// (`--people-space`) Strangers keep Fruin's no-touch zone: a pair side by
/// side steps apart to it, a clump spreads until no two bodies overlap
/// whichever way they face (the shoulders' breadth, 0.61 m) without being
/// thrown far, and people already apart are left where they are.
#[test]
fn people_keep_their_personal_room() {
    let mut pair = vec![[0.0, 0.0], [0.5, 0.0]];
    keep_personal_space(&mut pair);
    assert!(((pair[1][0] - pair[0][0]) - 2.0 * PERSONAL_RADIUS_M).abs() < 1e-9 && pair[0][1] == 0.0, "{pair:?}");
    let mut pos: Vec<[f64; 2]> = (0..9).map(|i| [(i % 3) as f64 * 0.3, (i / 3) as f64 * 0.3]).collect();
    pos.push(pos[4]);
    let before = pos.clone();
    keep_personal_space(&mut pos);
    separate(&mut pos, &[]);
    let mut worst = f64::INFINITY;
    for i in 0..pos.len() {
        for j in i + 1..pos.len() {
            worst = worst.min((pos[i][0] - pos[j][0]).hypot(pos[i][1] - pos[j][1]));
        }
    }
    assert!(worst > 2.0 * BODY_RADIUS_M + 0.07, "closest pair {worst:.2} m apart");
    for (a, b) in pos.iter().zip(&before) {
        assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1.6, "moved {:.2} m", (a[0] - b[0]).hypot(a[1] - b[1]));
    }
    let mut two = vec![[0.0, 0.0], [1.0, 0.0]];
    keep_personal_space(&mut two);
    assert_eq!(two, vec![[0.0, 0.0], [1.0, 0.0]]);
}

/// (`--people-groups`) A social group's members keep their own formation's
/// spacing, inside a stranger's no-touch zone: a pair of one group 0.75 m
/// apart stays so, a stranger beside them is kept to the zone from both.
#[test]
fn a_group_keeps_its_own_spacing() {
    use eo_agents::feed::keep_personal_space_with;
    let mut pos = vec![[0.0, 0.0], [0.75, 0.0], [1.2, 0.0]];
    keep_personal_space_with(&mut pos, &[7, 7, 0]);
    let d = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    assert!(d(pos[2], pos[1]) >= 2.0 * PERSONAL_RADIUS_M - 1e-6 && d(pos[2], pos[0]) >= 2.0 * PERSONAL_RADIUS_M - 1e-6, "{pos:?}");
    let mut pair = vec![[0.0, 0.0], [0.75, 0.0]];
    keep_personal_space_with(&mut pair, &[3, 3]);
    assert_eq!(pair, vec![[0.0, 0.0], [0.75, 0.0]], "one group's pair keeps its spacing");
    let mut strangers = vec![[0.0, 0.0], [0.75, 0.0]];
    keep_personal_space_with(&mut strangers, &[3, 4]);
    assert!((d(strangers[0], strangers[1]) - 2.0 * PERSONAL_RADIUS_M).abs() < 1e-9);
}

#[test]
fn a_reordered_crowd_keeps_the_body_ids_of_its_regions_and_corridor() {
    use eo_agents::feed::{FeedThread, Feeder, Ground};
    use eo_agents::library::Library;
    use eo_sim::output::{AgentBuffer, AgentInstance, CorridorInstance, KIND_WALKER_BASE};
    use eo_sim::world::Frame;
    use std::time::{Duration, Instant};

    let mut lib = Library::default();
    lib.models.push(eo_agents::GpuModel::default());
    lib.meta.people_models.push(0);
    let feed = FeedThread::start(Feeder::new(lib.into(), Ground::Flat, None, 7).unwrap());
    let walker = |id, x| AgentInstance { id, kind: KIND_WALKER_BASE, pos: [x, 0., 3.], ..Default::default() };
    let mut frame = Frame {
        regions: vec![
            AgentBuffer { walkers: vec![walker(9, 1.), walker(10, 2.)], ..Default::default() },
            AgentBuffer { walkers: vec![walker(9, 11.)], ..Default::default() },
        ],
        corridor: vec![CorridorInstance { id: 9, kind: KIND_WALKER_BASE, pos: [19., 7., 3.], ..Default::default() }],
        ..Default::default()
    };
    for before_swap in [true, false] {
        if !before_swap {
            frame.regions[0].walkers.swap(0, 1);
        }
        let posted = feed.post(frame.clone());
        let start = Instant::now();
        let output = loop {
            if let Some((out, seq)) = feed.take_with_walkers().0 {
                assert_eq!(seq, posted);
                break out;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "waiting for the posted crowd frame");
            std::thread::sleep(Duration::from_millis(1));
        };
        let expected = [(9u64, [1., 0., 3.]), (10, [2., 0., 3.]),
            ((1u64 << 32) | 9, [11., 0., 3.]), ((1u64 << 63) | 9, [19., 7., 3.])];
        assert_eq!(output.walkers.len(), expected.len());
        for (id, feet) in expected {
            let slot = output.walkers.iter().find(|slot| slot.id == id).unwrap();
            assert_eq!(slot.feet, feet);
            assert_eq!(output.agents[slot.index].pos, feet);
            if id == 9 { assert_eq!(slot.index, usize::from(!before_swap)); }
        }
    }
}
