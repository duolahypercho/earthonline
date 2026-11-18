//! `cargo run --release -p eo-landmarks --example access_shots > decks.json`:
//! a shots file (eo-app `--shots`) for every landmark deck (access.rs), on
//! the default ground plan: `<deck>_deck`, the deck with a person standing at
//! its railing (the player's body, eo-app's `person` key), and the view from
//! the deck at a standing eye. Where a real-world reference view is taken
//! from that deck (Coit Tower, Salesforce Tower, the Golden Gate's south
//! tower), the view keeps the reference's name, heading and tilt, with the
//! eye moved onto the deck (Coit: the arch's outer face, where the
//! reference's camera stands clear of the tower), and its lens narrowed for
//! the ground plan: the city's plan is drawn 1.25 times its size (the
//! approved ground-plan deviation) while the towers and the decks keep their
//! real heights, so a 35 degree lens's tangent over 1.25 (28.3 degrees)
//! puts everything the reference sees where the reference has it. The raw
//! 35 degree lens is the `__lens35` variant.

use serde_json::json;

/// Reference views taken from a deck: (deck, name, heading, tilt from
/// straight down, vertical lens, and the reference's own look-at pose
/// (lat, lon, alt, range) where its camera stands off the deck: the Golden
/// Gate view's camera stands about 370 m south of the south tower, over
/// Fort Point, at 37.8107 N, while the tower stands at 37.8140 N).
#[allow(clippy::type_complexity)]
const REFERENCE: [(&str, &str, f64, f64, f64, Option<[f64; 4]>); 3] = [
    ("coit-gallery", "view_from_coit_deck", 160.0, 85.0, 35.0, None),
    ("salesforce-ohana", "view_from_salesforce_top", 300.0, 80.0, 35.0, None),
    ("golden-gate-south-tower", "view_from_golden_gate_south_tower", 120.0, 80.0, 35.0, Some([37.80537, -122.46599, 20.0, 1192.0])),
];

/// Decks with room behind the person at the railing for a close camera.
const CLOSE: [&str; 4] = ["salesforce-ohana", "transamerica-48", "golden-gate-east-sidewalk", "bay-bridge-w2"];

/// The floor under (x, z) among a deck's walkable triangles (the highest), if any.
fn floor_at(walk: &[eo_landmarks::access::Tri], x: f64, z: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for t in walk {
        let (a, b, c) = (t[0], t[1], t[2]);
        let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
        if d.abs() < 1e-12 {
            continue;
        }
        let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
        let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
        let l3 = 1.0 - l1 - l2;
        if l1 >= -1e-9 && l2 >= -1e-9 && l3 >= -1e-9 {
            let y = l1 * a[1] + l2 * b[1] + l3 * c[1];
            best = Some(best.map_or(y, |v: f64| v.max(y)));
        }
    }
    best
}

fn main() -> anyhow::Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let elevation = eo_data::ElevationGrid::load(&paths)?;
    let set = eo_landmarks::access::AccessSet::build(&|x, z| elevation.sample(x, z));
    let mut shots = vec![];
    for d in &set.decks {
        let short = d.id.replace('-', "_");
        let (eye, at) = d.deck_camera;
        shots.push(json!({
            "name": format!("{short}_deck"),
            "eye": eye, "look_at": at, "fov": 50,
            "person": { "feet": d.view.feet, "heading": d.view.heading_deg },
            "time_of_day": 13, "weather": "clear",
            "what": format!("{}: the deck with a person standing at its railing", d.name),
        }));
        if CLOSE.contains(&d.id.as_str()) {
            // close behind the person, for the feet on the floor (the contact occlusion and shadow).
            let h = d.view.heading_deg.to_radians();
            let (fx, fz) = (h.sin(), -h.cos());
            let f = d.view.feet;
            let eye = [f[0] - fx * 3.0 + fz * 1.1, f[1] + 1.9, f[2] - fz * 3.0 - fx * 1.1];
            shots.push(json!({
                "name": format!("{short}_deck__close"),
                "eye": eye, "look_at": [f[0] + fx * 0.6, f[1] + 0.7, f[2] + fz * 0.6], "fov": 50,
                "person": { "feet": f, "heading": d.view.heading_deg },
                "time_of_day": 13, "weather": "clear",
                "what": format!("{}: the person at the railing, close, for the feet on the floor", d.name),
            }));
        }
        let feet = d.view.feet;
        let eye = d.vantage.unwrap_or([feet[0], feet[1] + eo_landmarks::access::EYE_M, feet[2]]);
        match REFERENCE.iter().find(|r| r.0 == d.id) {
            Some(r) => {
                let plan = 1.25f64;
                let lens = 2.0 * ((r.4 / 2.0f64).to_radians().tan() / plan).atan().to_degrees();
                for (name, fov) in [(r.1.to_string(), lens), (format!("{}__lens35", r.1), r.4)] {
                    shots.push(json!({
                        "name": name,
                        "eye": eye, "yaw": r.2, "pitch": r.3 - 90.0, "fov": fov,
                        "time_of_day": 13, "weather": "clear",
                        "what": format!("{}: the view from the deck at the reference's heading and tilt, lens {fov:.1} deg vertical", d.name),
                    }));
                }
                if let Some(g) = r.5 {
                    // the reference's own camera, where it does not stand on the deck.
                    shots.push(json!({
                        "name": format!("{}__refpose", r.1),
                        "geo": { "lat": g[0], "lon": g[1], "alt": g[2], "range": g[3], "fov": lens, "heading": r.2, "tilt": r.3 },
                        "time_of_day": 13, "weather": "clear",
                        "what": format!("{}: the reference's own camera pose (not on the deck), lens {lens:.1} deg vertical", d.name),
                    }));
                }
            }
            None => shots.push(json!({
                "name": format!("{short}_view"),
                "eye": eye, "yaw": d.view.heading_deg, "pitch": d.view_pitch_deg, "fov": 50.0,
                "time_of_day": 13, "weather": "clear",
                "what": format!("{}: the view from the deck, a standing eye at the railing", d.name),
            })),
        }
    }
    // A walk along the Golden Gate's east sidewalk: eight frames 25 m apart
    // (across the landmark draws' 128 m plan cells), the person on the floor
    // and the camera following 4 m behind, for the movement and the cells' seams.
    if let Some(d) = set.decks.iter().find(|d| d.id == "golden-gate-east-sidewalk") {
        let (eye, at) = d.deck_camera;
        // along the sidewalk: the walk's principal plan axis (its vertices' covariance), turned the camera's way.
        let pts: Vec<[f64; 2]> = d.walk.iter().flat_map(|t| t.iter().map(|p| [p[0], p[2]])).collect();
        let n = pts.len().max(1) as f64;
        let (mx, mz) = (pts.iter().map(|p| p[0]).sum::<f64>() / n, pts.iter().map(|p| p[1]).sum::<f64>() / n);
        let (mut sxx, mut sxz, mut szz) = (0.0, 0.0, 0.0);
        for p in &pts {
            let (a, b) = (p[0] - mx, p[1] - mz);
            sxx += a * a;
            sxz += a * b;
            szz += b * b;
        }
        let ang = 0.5 * (2.0 * sxz).atan2(sxx - szz);
        let (mut ux, mut uz) = (ang.cos(), ang.sin());
        if ux * (at[0] - eye[0]) + uz * (at[2] - eye[2]) < 0.0 {
            (ux, uz) = (-ux, -uz);
        }
        let heading = ux.atan2(-uz).to_degrees().rem_euclid(360.0);
        let start = d.view.feet;
        for k in 0..8 {
            let s = k as f64 * 25.0;
            let (x, z) = (start[0] + ux * s, start[2] + uz * s);
            let Some(y) = floor_at(&d.walk, x, z) else { continue };
            let cam = [x - ux * 4.0, y + 2.3, z - uz * 4.0];
            shots.push(json!({
                "name": format!("golden_gate_east_sidewalk_walk__f{:02}", k + 1),
                "eye": cam, "look_at": [x + ux * 6.0, y + 1.0, z + uz * 6.0], "fov": 50,
                "person": { "feet": [x, y, z], "heading": heading },
                "time_of_day": 13, "weather": "clear",
                "what": format!("Golden Gate Bridge east sidewalk: a walk, frame {} of 8, {s:.0} m along", k + 1),
            }));
        }
    }
    println!("{}", serde_json::to_string_pretty(&shots)?);
    Ok(())
}
