//! `cargo run --release -p eo-landmarks --example access_shots > decks.json`:
//! a shots file (eo-app `--shots`) for every landmark deck (access.rs), on
//! the default ground plan: `<deck>_deck`, the deck with a person standing at
//! its railing (the player's body, eo-app's `person` key), and the view from
//! the deck at a standing eye. Where a real-world reference view is taken
//! from that deck (Coit Tower, Salesforce Tower, the Golden Gate's south
//! tower), the view keeps the reference's name, heading, tilt and lens, with
//! the eye moved onto the deck.

use serde_json::json;

/// Reference views taken from a deck: (deck, name, heading, tilt from straight down, vertical lens).
const REFERENCE: [(&str, &str, f64, f64, f64); 3] = [
    ("coit-gallery", "view_from_coit_deck", 160.0, 85.0, 35.0),
    ("salesforce-ohana", "view_from_salesforce_top", 300.0, 80.0, 35.0),
    ("golden-gate-south-tower", "view_from_golden_gate_south_tower", 120.0, 80.0, 35.0),
];

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
        let feet = d.view.feet;
        let eye = [feet[0], feet[1] + eo_landmarks::access::EYE_M, feet[2]];
        let (name, heading, pitch, fov) = match REFERENCE.iter().find(|r| r.0 == d.id) {
            Some(r) => (r.1.to_string(), r.2, r.3 - 90.0, r.4),
            None => (format!("{short}_view"), d.view.heading_deg, d.view_pitch_deg, 50.0),
        };
        shots.push(json!({
            "name": name,
            "eye": eye, "yaw": heading, "pitch": pitch, "fov": fov,
            "time_of_day": 13, "weather": "clear",
            "what": format!("{}: the view from the deck, a standing eye at the railing", d.name),
        }));
    }
    println!("{}", serde_json::to_string_pretty(&shots)?);
    Ok(())
}
