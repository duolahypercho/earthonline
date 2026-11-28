//! Round 4 of the signal poles on real junctions (its own binary: the switches are process-wide).
//!
//! Every level mast arm as low as the California MUTCD lets what hangs from it go, city-wide:
//! each vehicle head's bottom (its backplate) at least the binding 15 ft over the roadway under
//! it (4D.09), each Street Name sign's bottom at least the binding 17 ft over the roadway under
//! it (2A.15), nothing on the pole under the arm's root plate; off (`--no-arm-lower`), the
//! game's arm height. A junction that meets a State route only through a freeway ramp is a city
//! junction (its arms level); off (`--no-ramp-level`), round 3.
use eo_props::signal_poles::{self, NameSigns, Options};
use eo_props::signals::{self, Fitting};
use eo_props::{Model, Prop};

/// One test (the switches are process-wide): the lowered arms, then the ramp junctions.
#[test]
fn round4_lowered_level_arms_and_ramp_junctions() {
    // the heads as the game draws them (the modelled heads and street props v2)
    eo_props::signal_models::set_signal_models(true);
    eo_props::props_v2::set_street_props_v2(true);
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = eo_props::load_world(&paths).unwrap();
    level_arms(&w);
    ramp_junctions(&w);
}

fn level_arms(w: &eo_props::PropsWorld) {
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = w;
    let std = &w.std;
    let dims = signal_poles::load(&paths.repo_root).unwrap().dims;
    // the drawn name signs: every legend's D3-1 blade at the overhead letter height
    let art = eo_props::sign_atlas::art::SignArt::load_default().unwrap();
    let mut ns = NameSigns::default();
    let mut blade: f64 = 0.0;
    for (i, l) in signals::name_sign_legends(&w.street).into_iter().enumerate() {
        let size = art.blade_size(&l, std.overhead_name_cap).unwrap();
        blade = blade.max(size[1]);
        ns.faces.insert(l, (i as u32, size));
    }
    // as eo-app: the thickest arm's radius at the pole, the hang, the tallest blade; the widths
    let thickest = dims.arms.iter().map(|a| a.root_radius_m).fold(dims.arm_root_radius_m, f64::max);
    signals::set_name_sign_drop(thickest + signal_poles::SIGN_HANG_M + blade);
    signals::set_name_sign_widths(Some(ns.faces.iter().map(|(l, f)| (l.clone(), f.1[0])).collect()));
    signals::set_rise_clearance(true);
    signals::set_arm_rise(true);
    signals::set_arm_rise_routes(true);
    signals::set_ramp_level(true);
    let down = signals::head_reach_down(w.js.num("signalHardware", "headHalfHeightM"));
    let arm_h = w.js.num("signalHardware", "mastArmHeightM");
    let o = Options { name_signs: Some(&ns), level_arm_diameter: true, ..Default::default() };

    let (mut arms, mut lowered, mut raised, mut heads, mut signs) = (0, 0, 0, 0, 0);
    let (mut lo, mut hi, mut drop_sum) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
    let mut overs: Vec<(f64, f64)> = vec![];
    let (mut rising, mut rising_raised) = (0, 0);
    let (mut head_gap, mut sign_gap) = (f64::INFINITY, f64::INFINITY);
    let (mut rising_head_gap, mut rising_sign_gap) = (f64::INFINITY, f64::INFINITY);
    for (ni, n) in w.street.plan.nodes.iter().enumerate() {
        if !n.signalised() {
            continue;
        }
        signals::set_arm_lower(false);
        signals::set_rise_clearance(false);
        let before = signals::plan_node_parts(w, ni);
        signals::set_arm_lower(true);
        signals::set_rise_clearance(true);
        let (mut props, mut fittings): (Vec<Prop>, Vec<Fitting>) = (vec![], vec![]);
        signals::plan_node_with(w, ni, &mut props, &mut fittings);
        let np = signals::plan_node_parts(w, ni);
        // off: every level arm at the game's arm height over its pole's foot
        for a in before.arms.iter().filter(|a| a.class.is_none()) {
            let pole = before.poles.iter().find(|p| p.id == a.pole).unwrap();
            assert!((a.y - pole.base_y - arm_h).abs() < 1e-9, "off: the game's arm height");
        }
        for (a, b) in np.arms.iter().zip(&before.arms) {
            if a.class.is_some() {
                assert!(a.y >= b.y - 1e-9, "a rising arm is only raised");
                rising += 1;
                if a.y > b.y + 1e-9 {
                    rising_raised += 1;
                }
                continue;
            }
            arms += 1;
            let pole = np.poles.iter().find(|p| p.id == a.pole).unwrap();
            let over = a.y - pole.base_y;
            overs.push((over, a.y - b.y));
            lo = lo.min(over);
            hi = hi.max(over);
            drop_sum += b.y - a.y;
            if a.y < b.y - 1e-9 {
                lowered += 1;
            } else if a.y > b.y + 1e-9 {
                raised += 1;
            }
            // nothing the pole carries under the arm's root plate
            for hd in np.heads.iter().filter(|h| h.pole == a.pole && h.mount != 1) {
                assert!(hd.y + down < a.y - 0.25, "node {ni}: a pole head's top {:.2} reaches the arm {:.2}", hd.y + down, a.y);
            }
        }
        // drawn: every vehicle head on a level arm and every name sign under one clear of the
        // binding minimum (a rising arm's are counted apart: ES-7E's, as round 3 drew them)
        let level = |p: [f64; 3], yaw: f64| np.arms.iter().any(|a| a.class.is_none() && (a.x - p[0]).hypot(a.z - p[2]) < 0.05 && (a.yaw - yaw).abs() < 1e-9);
        let on_level = |x: f64, z: f64| {
            np.arms.iter().filter(|a| a.class.is_none()).any(|a| {
                let u = [a.yaw.cos(), -a.yaw.sin()];
                let (vx, vz) = (x - a.x, z - a.z);
                (vx * u[1] - vz * u[0]).abs() < 0.5 && (-0.05..=a.length + 0.05).contains(&(vx * u[0] + vz * u[1]))
            })
        };
        let level_signs: Vec<Fitting> = fittings.iter().filter(|f| matches!(f, Fitting::NameSign { arm, yaw, .. } if level(*arm, *yaw))).cloned().collect();
        let rising_signs: Vec<Fitting> = fittings.iter().filter(|f| matches!(f, Fitting::NameSign { arm, yaw, .. } if !level(*arm, *yaw))).cloned().collect();
        for h in props.iter().filter(|p| p.model == Model::SignalHead && p.variant == 1) {
            let gap = h.pos[1] - down - w.ground(h.pos[0], h.pos[2]) - std.signal_bottom_min;
            if on_level(h.pos[0], h.pos[2]) {
                heads += 1;
                head_gap = head_gap.min(gap);
            } else {
                rising_head_gap = rising_head_gap.min(gap);
            }
        }
        for pl in &signal_poles::hardware(&props, &rising_signs, &dims, &o).plates {
            rising_sign_gap = rising_sign_gap.min(pl.pos[1] - pl.scale[1] as f64 / 2.0 - w.ground(pl.pos[0], pl.pos[2]) - std.sign_overhead_min);
        }
        let hw = signal_poles::hardware(&props, &level_signs, &dims, &o);
        for pl in &hw.plates {
            signs += 1;
            let (s, c) = pl.yaw.sin_cos();
            // across the plate's width (its face's local +X: (cos yaw, -sin yaw))
            for t in [-0.5, 0.0, 0.5] {
                let x = pl.pos[0] + c * t * pl.scale[0] as f64;
                let z = pl.pos[2] - s * t * pl.scale[0] as f64;
                sign_gap = sign_gap.min(pl.pos[1] - pl.scale[1] as f64 / 2.0 - w.ground(x, z) - std.sign_overhead_min);
            }
        }
    }
    println!(
        "level arms {arms}: {lowered} lowered, {raised} raised; arm over its pole's foot {lo:.2}..{hi:.2} m (was {arm_h:.2}), mean drop {:.2} m; \
         {heads} arm heads clear by >= {head_gap:.3} m, {signs} name signs by >= {sign_gap:.3} m",
        drop_sum / arms as f64
    );
    overs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let q = |f: f64| overs[((overs.len() - 1) as f64 * f) as usize].0;
    println!(
        "arm over its pole's foot: 10% {:.2}, median {:.2}, 90% {:.2} m; raised by > 0.5 m: {}, > 1 m: {}",
        q(0.1), q(0.5), q(0.9), overs.iter().filter(|o| o.1 > 0.5).count(), overs.iter().filter(|o| o.1 > 1.0).count()
    );
    println!("rising (State-route) arms with their clearance: heads clear by >= {rising_head_gap:.3} m, name signs by >= {rising_sign_gap:.3} m; {rising_raised} of {rising} raised");
    assert!(rising_head_gap >= -1e-6 && rising_sign_gap >= -1e-6, "a rising arm's head or sign under the binding minimum");
    assert!(arms > 500 && lowered * 10 > arms * 8, "most level arms lowered");
    assert!(head_gap >= -1e-6, "a head under CA MUTCD 4D.09's 15 ft: {head_gap:.3} m");
    assert!(sign_gap >= -1e-6, "a name sign under CA MUTCD 2A.15's 17 ft: {sign_gap:.3} m");
    assert!(heads > 1000 && signs > 100);
    // the lowest an arm hangs is set by them: some head or sign at its minimum
    assert!(head_gap.min(sign_gap) < 1e-6);
    signals::set_arm_lower(false);
}

fn ramp_junctions(w: &eo_props::PropsWorld) {
    signals::set_arm_rise(true);
    signals::set_arm_rise_routes(true);
    let mut changed = vec![];
    let mut arms = 0;
    for (ni, n) in w.street.plan.nodes.iter().enumerate() {
        if !n.signalised() {
            continue;
        }
        signals::set_ramp_level(false);
        let before = signals::on_state_route(&w.street, n);
        let rising = signals::plan_node_parts(w, ni).arms.iter().filter(|a| a.class.is_some()).count();
        signals::set_ramp_level(true);
        let after = signals::on_state_route(&w.street, n);
        assert!(!after || before, "the ramp rule only takes junctions away");
        if before && !after {
            assert!(signals::plan_node_parts(w, ni).arms.iter().all(|a| a.class.is_none()), "node {ni}: its arms level");
            let mut names: Vec<String> = n.approaches.iter().map(|a| w.street.city.segments[w.street.plan.entries[a.entry].road].street_name.clone()).filter(|s| !s.is_empty() && !s.starts_with("SF Rd")).collect();
            names.sort();
            names.dedup();
            changed.push((ni, names.join(" / "), rising));
            arms += rising;
        }
    }
    signals::set_ramp_level(false);
    for c in &changed {
        println!("node {}: {} ({} rising arms now level)", c.0, c.1, c.2);
    }
    println!("{} junctions, {arms} arms", changed.len());
    assert!(!changed.is_empty() && arms > 0);
    // 5th & Harrison (an I-80 on-ramp) is one of them
    assert!(changed.iter().any(|c| c.1.contains("5th Street") && c.1.contains("Harrison Street")));
}
