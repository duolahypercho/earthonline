//! Round 2 of the modelled signal poles on real junctions (its own binary: `signals::set_arm_rise`
//! is process-wide): with the rise on, every arm the Caltrans RSP ES-7E table covers rises and its
//! heads still hang clear of the roadway (CA MUTCD 4D.09's binding minimum); off, the plan is
//! round 1's exactly. The name signs carry a cross street's legend and fit under their arm; a
//! push button stands on a pole at the height the standards table gives, its face parallel to a
//! crosswalk. The provenance's arm classes are the standards table's rows.
use eo_props::Model;
use eo_props::signal_poles::{self, NameSigns, Options, Part};
use eo_props::signals::{self, ArmClass, Fitting};

fn tiles(w: &eo_props::PropsWorld, n: usize) -> Vec<eo_props::TilePlacement> {
    w.tile_keys().into_iter().filter(|k| !w.plan_tile(*k).fittings.is_empty()).take(n).map(|k| w.plan_tile(k)).collect()
}

#[test]
fn round2_arms_rise_signs_and_buttons() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let s = signal_poles::load(&paths.repo_root).unwrap();
    // the provenance's arm classes are the table's
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let table = ArmClass::table(&ca).unwrap();
    assert_eq!(table.len(), s.dims.arms.len());
    for (t, a) in table.iter().zip(&s.dims.arms) {
        assert!((t.length - a.class.length).abs() < 1e-3 && (t.rise - a.class.rise).abs() < 1e-3 && (t.angle - a.class.angle).abs() < 1e-6, "{t:?} vs {:?}", a.class);
        assert!((a.root_radius_m * 2.0 - t.od_at_pole).abs() < 1e-3 && a.tip_radius_m < a.root_radius_m, "tapered from the sheet's diameter");
    }
    let std = eo_props::Standards::load(&ca).unwrap();

    // off: round 1's plan (no arm rises, no class)
    signals::set_arm_rise(false);
    let w0 = eo_props::load_world(&paths).unwrap();
    let off = tiles(&w0, 60);
    assert!(off.iter().flat_map(|t| t.props.iter()).filter(|p| p.model == Model::MastArm).all(|p| p.variant == 0));
    // on
    signals::set_arm_rise(true);
    let w = eo_props::load_world(&paths).unwrap();
    let on = tiles(&w, 60);
    signals::set_arm_rise(false);
    let arms: Vec<&eo_props::Prop> = on.iter().flat_map(|t| t.props.iter()).filter(|p| p.model == Model::MastArm).collect();
    assert!(arms.iter().filter(|p| p.variant > 0).count() * 2 > arms.len(), "most arms rise: {} of {}", arms.iter().filter(|p| p.variant > 0).count(), arms.len());
    for a in arms.iter().filter(|p| p.variant > 0) {
        let c = ArmClass::of(&table, a.scale[0] as f64).unwrap();
        assert_eq!(a.variant as usize, c + 1, "the arm's class is its length's");
    }
    // the props other than arms and arm heads are the same with and without the rise
    let pick = |ts: &[eo_props::TilePlacement]| -> Vec<String> {
        let mut v: Vec<String> = ts.iter().flat_map(|t| t.props.iter()).filter(|p| !matches!(p.model, Model::MastArm | Model::SignalHead)).map(|p| format!("{:?}{:?}", p.model, p.pos)).collect();
        v.sort();
        v
    };
    assert_eq!(pick(&off), pick(&on));
    // every mast head hangs over the binding minimum (the bottom of its housing over its pole's foot)
    let head_half = w.js.num("signalHardware", "headHalfHeightM");
    for t in &on {
        for h in t.props.iter().filter(|p| p.model == Model::SignalHead && p.variant == 1) {
            let pole = t.props.iter().filter(|q| q.model == Model::SignalPole).min_by(|a, b| (a.pos[0] - h.pos[0]).hypot(a.pos[2] - h.pos[2]).total_cmp(&(b.pos[0] - h.pos[0]).hypot(b.pos[2] - h.pos[2]))).unwrap();
            assert!(h.pos[1] - head_half - pole.pos[1] >= std.signal_bottom_min - 0.25, "a head's bottom {:.2} m over its pole's foot", h.pos[1] - head_half - pole.pos[1]);
        }
    }
    // a mast pole whose arms all rise ends the ES-7C distance over the highest one's connection
    for t in &on {
        for pole in t.props.iter().filter(|p| p.model == Model::SignalPole && p.variant == 1) {
            let arms: Vec<&eo_props::Prop> = t.props.iter().filter(|a| a.model == Model::MastArm && (a.pos[0] - pole.pos[0]).hypot(a.pos[2] - pole.pos[2]) < 1e-6).collect();
            if !arms.is_empty() && arms.iter().all(|a| a.variant > 0) {
                let top = pole.pos[1] + pole.scale[1] as f64;
                let arm = arms.iter().map(|a| a.pos[1]).fold(f64::NEG_INFINITY, f64::max);
                assert!(top >= arm + std.mast_pole_top_over_arm - 1e-4 && top <= arm + std.mast_pole_top_over_arm + 1.0, "pole top {top:.2}, arm {arm:.2}");
            }
        }
    }
    // fittings: legends of cross streets, buttons at the standard height on a pole of the plan
    let (mut signs, mut buttons) = (0, 0);
    for t in &on {
        for f in &t.fittings {
            match f {
                Fitting::NameSign { legend, gap, .. } => {
                    signs += 1;
                    assert!(!legend.is_empty() && gap[1] > gap[0]);
                }
                Fitting::PushButton { pole, height, .. } => {
                    buttons += 1;
                    assert_eq!(*height, std.ped_button_height);
                    assert!(t.props.iter().any(|p| p.model == Model::SignalPole && (p.pos[0] - pole[0]).hypot(p.pos[2] - pole[2]) < 1e-6), "a button on a pole of the plan");
                }
                Fitting::Luminaire { .. } => panic!("round 2 has no street lights on its poles (round 3's `signal_lamps`, off here)"),
                Fitting::PullBox { .. } => panic!("round 2 has no pull boxes (round 5's, off here)"),
            }
        }
    }
    assert!(signs > 10 && buttons > 40, "{signs} signs, {buttons} buttons");
    // drawn: each sign whose face is known and fits hangs under its arm on two brackets; each button on its pole
    let mut ns = NameSigns::default();
    for t in &on {
        for f in &t.fittings {
            if let Fitting::NameSign { legend, .. } = f {
                ns.faces.insert(legend.clone(), (40, [1.5, 0.45]));
            }
        }
    }
    let o = Options { variants: true, name_signs: Some(&ns), push_buttons: true, pole_cut: false, level_arm_diameter: false, ..Default::default() };
    let (mut plates, mut brackets, mut drawn_buttons) = (0, 0, 0);
    for t in &on {
        let hw = signal_poles::hardware(&t.props, &t.fittings, &s.dims, &o);
        plates += hw.plates.len();
        brackets += hw.added.iter().filter(|p| p.part == Part::SignBracket).count();
        drawn_buttons += hw.added.iter().filter(|p| matches!(p.part, Part::ButtonPipeP | Part::ButtonPipeN)).count();
        for pl in &hw.plates {
            // under an arm: below the arm's root height plus its rise
            assert!(t.props.iter().any(|a| a.model == Model::MastArm && pl.pos[1] < a.pos[1] + 2.5 && (a.pos[0] - pl.pos[0]).hypot(a.pos[2] - pl.pos[2]) < a.scale[0] as f64 + 0.1));
        }
    }
    assert_eq!(brackets, 2 * plates);
    assert!(plates * 2 > signs, "most signs fit: {plates} of {signs}");
    assert_eq!(drawn_buttons, buttons, "every button finds its pole");
    // the legends the atlas is drawn for cover every sign's
    let all = signals::name_sign_legends(&w.street);
    for t in &on {
        for f in &t.fittings {
            if let Fitting::NameSign { legend, .. } = f {
                assert!(all.binary_search(legend).is_ok(), "{legend} listed");
            }
        }
    }
}

/// The arms' Street Name sign faces (`sign_atlas::arm_name_signs`): every legend the city's arms
/// may carry fits the face atlas's free rows, each sign's height follows the 12 in capitals of
/// CA MUTCD 2D.45 on the FHWA D3-1a layout, its width its legend's, its cell under the inventory's.
#[test]
fn round2_name_sign_faces() {
    let Some(art) = eo_props::sign_atlas::art::SignArt::load_default() else {
        eprintln!("no sign art folder: the name signs are not drawn (skipped)");
        return;
    };
    let paths = eo_data::DataPaths::discover().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let std = eo_props::Standards::load(&ca).unwrap();
    let w = eo_props::load_world(&paths).unwrap();
    let legends = signals::name_sign_legends(&w.street);
    let faces = w.js.signs.faces.len();
    let n = eo_props::sign_atlas::arm_name_signs(&legends, &art, faces, std.overhead_name_cap).unwrap();
    assert_eq!(n.legends, legends, "every legend fits");
    assert!(n.y0 >= (faces as u32).div_ceil(eo_props::sign_atlas::SIGN_FACE_COLUMNS) * eo_props::sign_atlas::SIGN_FACE_CELL, "under the inventory's faces");
    assert_eq!(n.rgba.len(), (eo_props::sign_atlas::SIGN_FACE_SIZE * n.height * 4) as usize);
    let cap = std.overhead_name_cap;
    for (l, s) in n.legends.iter().zip(&n.sizes) {
        assert!(s[1] > 1.2 * cap && s[1] < 2.5 * cap, "{l}: height {} for capitals {cap}", s[1]);
        assert!(s[0] >= 2.0 * s[1] && s[0] < 6.0, "{l}: width {}", s[0]);
    }
    // a longer legend, a wider sign
    let i = |x: &str| n.legends.iter().position(|l| l == x);
    if let (Some(a), Some(b)) = (i("1ST ST"), n.legends.iter().position(|l| l.len() > 12)) {
        assert!(n.sizes[b][0] > n.sizes[a][0]);
    }
    for t in &n.tiles {
        let top_px = (1.0 - (t[1] + t[3])) * eo_props::sign_atlas::SIGN_FACE_SIZE as f32;
        assert!(top_px >= n.y0 as f32 - 0.5 && t[2] > 0.0 && t[3] > 0.0);
    }
}
