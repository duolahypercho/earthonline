//! The moored boats (eo-props `boats`): the bodies match their provenance, and
//! the berths take boats that fit, bow in, the same every run.

use eo_landmarks::floating_docks::Berth;
use eo_props::boats::{self, BOW_GAP_M, Kind, OCCUPANCY};

fn bodies() -> Vec<boats::Body> {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    boats::load_bodies(&paths.repo_root).unwrap()
}

#[test]
fn the_bodies_match_their_provenance() {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let b = bodies();
    assert_eq!(b.len(), boats::BODIES.len());
    for body in &b {
        for l in &body.lods {
            let f = paths.repo_root.join(&l.path);
            assert_eq!(std::fs::metadata(&f).map(|m| m.len()).ok(), Some(l.bytes), "{} on disk at its recorded size", l.path);
        }
        // The published classes the bodies were built to (provenance `dimensionSources`).
        let (loa, beam) = match body.kind {
            Kind::Sail => (10.52, 3.58),
            Kind::Motor => (13.28, 4.25),
            Kind::Fishing => (12.5, 3.88),
            Kind::Jet => (3.37, 1.27),
        };
        assert!((body.loa - loa).abs() < 0.25 && (body.beam - beam).abs() < 0.1, "{}: {} x {} m", body.name, body.loa, body.beam);
        assert!(body.draft > 0.0 && body.height > 0.5);
    }
}

fn berth(x: f64, z: f64, heading: f64, length: f64, beam: f64, finger: bool) -> Berth {
    Berth { centre: [x, z], heading, length, beam, finger }
}

#[test]
fn the_berths_take_boats_that_fit() {
    eo_data::groundscale::pin(1.0);
    let b = bodies();
    // A marina far from the fishing harbour: 40 x 30 finger berths, 18 m by 5 m (spread out, so each boat finds its berth).
    let marina: Vec<Berth> = (0..1200).map(|i| berth(5000.0 + (i % 40) as f64 * 30.0, 2000.0 + (i / 40) as f64 * 30.0, 0.3, 18.0, 5.0, true)).collect();
    let moored = boats::moor(&marina, &b);
    assert_eq!(moored, boats::moor(&marina, &b), "the same every run");
    let share = moored.len() as f64 / marina.len() as f64;
    assert!((share - OCCUPANCY).abs() < 0.04, "occupancy {share:.3}");
    let count = |k: Kind, m: &[boats::Moored]| m.iter().filter(|x| b[x.body].kind == k).count();
    let (sail, motor, fishing, jet) = (count(Kind::Sail, &moored), count(Kind::Motor, &moored), count(Kind::Fishing, &moored), count(Kind::Jet, &moored));
    println!("marina: {} boats: {sail} sail, {motor} motor, {fishing} fishing, {jet} jet", moored.len());
    assert!(sail > motor && motor > fishing && fishing > 0 && jet > 0, "a recreational marina's mix");
    // Bow in: the bow `BOW_GAP_M` short of the berth's walk end.
    for m in &moored {
        let br = marina.iter().find(|x| (x.centre[0] - m.pos[0]).abs() < 10.0 && (x.centre[1] - m.pos[1]).abs() < 10.0 && x.heading == m.heading).unwrap();
        let u = [br.heading.cos(), br.heading.sin()];
        let bow = [m.pos[0] + u[0] * b[m.body].loa / 2.0, m.pos[1] + u[1] * b[m.body].loa / 2.0];
        let end = [br.centre[0] + u[0] * br.length / 2.0, br.centre[1] + u[1] * br.length / 2.0];
        assert!(((bow[0] - end[0]).hypot(bow[1] - end[1]) - BOW_GAP_M).abs() < 1e-6);
    }
    // Hyde Street Harbor (the fishing harbour): mostly the fishing fleet, no jet skis.
    let (lat, lon, _) = boats::FISHING_HARBOUR;
    let f = eo_data::Frame::sf();
    let (hx, hz) = ((lon - f.center_lon) * f.m_per_deg_lon, -(lat - f.center_lat) * f.m_per_deg_lat);
    let harbour: Vec<Berth> = (0..200).map(|i| berth(hx - 60.0 + (i % 20) as f64 * 6.0, hz - 50.0 + (i / 20) as f64 * 10.0, -2.4, 18.0, 5.0, true)).collect();
    let h = boats::moor(&harbour, &b);
    assert!(count(Kind::Fishing, &h) * 10 > h.len() * 7 && count(Kind::Jet, &h) == 0, "{} of {} fishing", count(Kind::Fishing, &h), h.len());
    // Berths too small for a yacht hold jet skis or nothing; a side tie may run into its gap.
    let small: Vec<Berth> = (0..100).map(|i| berth(9000.0 + i as f64 * 4.0, 0.0, 0.0, 5.0, 1.6, true)).collect();
    assert!(boats::moor(&small, &b).iter().all(|m| b[m.body].kind == Kind::Jet));
    let ties: Vec<Berth> = (0..300).map(|i| berth(-9000.0 + i as f64 * 14.5, 0.0, 0.0, 12.0, 4.2, false)).collect();
    let t = boats::moor(&ties, &b);
    assert!(count(Kind::Motor, &t) > 0, "a 13.3 m motor yacht side-ties in a 12 m berth and its gap");
    assert!(t.iter().all(|m| b[m.body].loa <= 12.0 + eo_landmarks::floating_docks::SIDE_TIE_GAP_M - 1.0));
}

#[test]
fn the_motion_word_packs_roll_and_pitch() {
    let decode = |w: u32| {
        let (r, p) = ((w & 0xffff) as i32, ((w >> 16) & 0x7fff) as i32);
        let r = if r >= 32768 { r - 65536 } else { r };
        let p = if p >= 16384 { p - 32768 } else { p };
        (r as f64 * boats::AFLOAT_STEP_RAD, p as f64 * boats::AFLOAT_STEP_RAD)
    };
    for (roll, pitch) in [(0.0, 0.0), (0.0523, -0.0175), (-0.31, 0.12), (1.2, -1.5)] {
        let w = boats::afloat_word(roll, pitch);
        assert_eq!(w & boats::AFLOAT_BIT, boats::AFLOAT_BIT);
        let (r, p) = decode(w);
        assert!((r - roll).abs() <= boats::AFLOAT_STEP_RAD / 2.0 + 1e-12 && (p - pitch).abs() <= boats::AFLOAT_STEP_RAD / 2.0 + 1e-12, "{roll} {pitch} -> {r} {p}");
    }
    // clamped at the word's reach
    let (r, p) = decode(boats::afloat_word(9.0, -9.0));
    assert!((r - 3.2767).abs() < 1e-9 && (p + 1.6384).abs() < 1e-9);
}
