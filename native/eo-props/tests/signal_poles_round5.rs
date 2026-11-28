//! Round 5 of the signal poles on real junctions: the corner hardware (its own binary: the
//! switches are process-wide).
//!
//! City-wide: one controller cabinet a signalised junction (`--no-one-cabinet` off), the walk's
//! furniture drawing none, a divided street's nodes sharing theirs; the cabinet (the MSF60 on its
//! pad, `--no-cabinet-model` off) on the sidewalk, never on the roadway or a kerb ramp, its back
//! the edge zone's offset from the kerb face; a pull box beside the poles (`--no-pull-boxes` off),
//! never on the roadway or a ramp, a pole's foundation or another box; the hardware drawing two
//! channels on each Street Name sign's back and a pull box for each fitting, none when off.
use eo_props::signal_poles::{self, NameSigns, Options, Part};
use eo_props::signals::{self, Fitting};
use eo_props::{Model, overhead};
use std::sync::Arc;

#[test]
fn round5_corner_hardware() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let poles = signal_poles::load(&paths.repo_root).unwrap();
    let d = poles.dims.clone();
    overhead::set_one_cabinet(true);
    overhead::set_cabinet_model(Some([d.cabinet_m[0], d.cabinet_m[1], d.cabinet_m[2], d.cabinet_pad_m[0], d.cabinet_pad_m[1]]));
    signals::set_pull_boxes(Some(d.pull_box_m));
    eo_props::signal_models::set_signal_models(true);
    eo_props::props_v2::set_street_props_v2(true);
    let w = Arc::new(eo_props::load_world(&paths).unwrap());
    let city = eo_data::City::load(&paths).unwrap();
    w.install_furniture(&city.buildings).unwrap();
    overhead::install(&w, overhead::DropBuilding::from_city(&city.buildings)).unwrap();
    let std = &w.std;
    // the drawn name signs (as round 4's test)
    let art = eo_props::sign_atlas::art::SignArt::load_default().unwrap();
    let mut ns = NameSigns::default();
    for (i, l) in signals::name_sign_legends(&w.street).into_iter().enumerate() {
        ns.faces.insert(l.clone(), (i as u32, art.blade_size(&l, std.overhead_name_cap).unwrap()));
    }
    let on = Options { name_signs: Some(&ns), pull_boxes: true, sign_channels: true, ..Default::default() };
    let off = Options { name_signs: Some(&ns), ..Default::default() };

    let (of, junctions) = overhead::junctions(&w);
    let mut per = vec![0usize; junctions.len()];
    let (mut walk, mut boxes, mut plates, mut channels, mut box_pieces) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let [len, wid] = d.pull_box_m;
    let po = d.cabinet_pad_m[1];
    let (cw, cd) = (d.cabinet_m[0] / 2.0 + po, d.cabinet_m[1] / 2.0 + po);
    for k in w.tile_keys() {
        let t = overhead::plan_tile(&w, k);
        for c in &t.cabinets {
            let j = of[c.node as usize];
            assert_ne!(j, u32::MAX, "a cabinet at a signalised node");
            per[j as usize] += 1;
            assert_eq!(c.size, d.cabinet_m, "the model's size");
            // local +X (its width) along the kerb, +Z (its doors) away from it
            let (s, co) = c.yaw.sin_cos();
            let (ax, az) = (co, -s);
            let (dx, dz) = (s, co);
            for (a, f) in [(-cw, -cd), (-cw, cd), (cw, -cd), (cw, cd)] {
                let (x, z) = (c.pos[0] + ax * a + dx * f, c.pos[2] + az * a + dz * f);
                assert!(w.roadway.hit(x, z, 0.0).is_none() && !w.roadway.ramp_hit(x, z, 0.0), "cabinet pad corner {x:.2},{z:.2} on the roadway or a ramp");
            }
            // its kerb side the edge zone's offset (less the planner's few centimetres) from the roadway
            for a in [-cw, 0.0, cw] {
                let (x, z) = (c.pos[0] + ax * a - dx * cd, c.pos[2] + az * a - dz * cd);
                assert!(w.roadway.hit(x, z, std.kerb_vertical_offset - 0.04).is_none(), "cabinet's back {x:.2},{z:.2} nearer the kerb than the edge zone");
            }
        }
        let tp = w.plan_tile(k);
        // nothing standing on a cabinet's pad (a sign post, a bollard, a hydrant, a tree, a pole)
        for c in &t.cabinets {
            let (s, co) = c.yaw.sin_cos();
            for p in tp.props.iter().filter(|p| !matches!(p.model, Model::TactilePad | Model::TreeBasin | Model::SignPlate | Model::SignalHead | Model::PedHead | Model::MastArm)) {
                let (dx, dz) = (p.pos[0] - c.pos[0], p.pos[2] - c.pos[2]);
                let (a, f) = (dx * co - dz * s, dx * s + dz * co);
                assert!(!(a.abs() < cw && f.abs() < cd), "{:?} at {:?} on the cabinet's pad at {:?}", p.model, p.pos, c.pos);
            }
        }
        walk += tp.props.iter().filter(|p| p.model == Model::SignalCabinet).count();
        let tile_poles: Vec<&eo_props::Prop> = tp.props.iter().filter(|p| p.model == Model::SignalPole).collect();
        let mut tile_boxes: Vec<[f64; 2]> = vec![];
        boxes += tp.fittings.iter().filter(|f| matches!(f, Fitting::PullBox { .. })).count();
        let hw = signal_poles::hardware(&tp.props, &tp.fittings, &d, &Options { cabinets: &t.cabinets, ..on });
        for pc in hw.added.iter().filter(|p| p.part == Part::PullBox) {
            let (pos, yaw) = (&pc.pos, &pc.yaw);
            let (s, co) = yaw.sin_cos();
            let (ux, uz) = (co, -s);
            for (a, b) in [(0.0, 0.0), (-len / 2.0, -wid / 2.0), (-len / 2.0, wid / 2.0), (len / 2.0, -wid / 2.0), (len / 2.0, wid / 2.0)] {
                let (x, z) = (pos[0] + ux * a - uz * b, pos[2] + uz * a + ux * b);
                assert!(w.roadway.hit(x, z, 0.0).is_none() && !w.roadway.ramp_hit(x, z, 0.0), "pull box corner {x:.2},{z:.2} on the roadway or a ramp");
            }
            for p in &tile_poles {
                let r = if p.variant == 1 { 0.381 } else { 0.24 };
                assert!((p.pos[0] - pos[0]).hypot(p.pos[2] - pos[2]) >= len / 2.0 + r - 1e-6, "pull box on a pole's foundation at {pos:?}");
            }
            assert!(tile_boxes.iter().all(|q| (q[0] - pos[0]).hypot(q[1] - pos[2]) >= len), "two pull boxes overlap at {pos:?}");
            assert!(t.cabinets.iter().all(|c| (c.pos[0] - pos[0]).hypot(c.pos[2] - pos[2]) >= cw + len / 2.0), "a pull box under a cabinet at {pos:?}");
            tile_boxes.push([pos[0], pos[2]]);
        }
        plates += hw.plates.len();
        channels += hw.added.iter().filter(|p| p.part == Part::SignChannel).count();
        box_pieces += hw.added.iter().filter(|p| p.part == Part::PullBox).count();
        let hw_off = signal_poles::hardware(&tp.props, &tp.fittings, &d, &off);
        assert!(hw_off.added.iter().all(|p| p.part != Part::SignChannel && p.part != Part::PullBox), "off: no channels, no pull boxes");
    }
    let with = per.iter().filter(|&&c| c == 1).count();
    eprintln!("round 5: {} junctions, {with} with their one cabinet; walk cabinets {walk}; pull boxes {boxes} planned, {box_pieces} drawn; name signs {plates}, channels {channels}", junctions.len());
    assert_eq!(walk, 0, "the walk's furniture draws no cabinet");
    assert!(per.iter().all(|&c| c <= 1), "at most one cabinet a junction");
    assert!(with as f64 >= 0.99 * junctions.len() as f64, "nearly every junction has its cabinet ({with} of {})", junctions.len());
    assert!(junctions.iter().any(|j| j.len() > 1), "divided streets' nodes joined");
    assert!(box_pieces as f64 > 5.0 * junctions.len() as f64, "pull boxes by the poles ({box_pieces})");
    assert!(box_pieces as f64 >= 0.97 * boxes as f64, "few planned boxes left out on another junction's pole ({box_pieces} of {boxes})");
    assert_eq!(channels, 2 * plates, "two channels on each sign's back");
}
