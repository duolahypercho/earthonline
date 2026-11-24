//! (`--crowd`) The people's bodies on the ground: kept apart and out of the
//! vehicles (`feed::separate`), and the contact anchors each person model
//! carries for the renderer's ground occlusion (`people::add_contact_anchors`).

use eo_agents::feed::{BODY_RADIUS_M, separate};

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
    let opts = eo_agents::library::CookOptions { far_body: true };
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
