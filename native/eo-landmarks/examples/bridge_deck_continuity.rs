//! Check the driving surface against both meshes selected by the landmark renderer.
//! From `native/`, run `cargo run --release -p eo-landmarks --example bridge_deck_continuity -- --distant-bridge-decks --golden-gate-surfaces`.

use anyhow::{Context, Result, ensure};
use eo_landmarks::bridge_roads::{LandmarkDeck, surfaces};
use eo_landmarks::{Frozen, LandmarkSet, LandmarksJs, Role, suspension::DeckStandards};
use std::collections::HashSet;

type Face = [[u32; 3]; 3];

// Round collision coordinates to the renderer's precision. A different starting
// vertex still describes the same triangle. Reversing the face changes which side draws.
fn face_key(points: [[f32; 3]; 3]) -> Face {
    let mut face = points.map(|point| point.map(f32::to_bits));
    let first = (0..3).min_by_key(|&i| face[i]).unwrap();
    face.rotate_left(first);
    face
}

fn mesh_faces(mesh: &Frozen) -> impl Iterator<Item = Face> + '_ {
    mesh.indices.chunks_exact(3).map(|indices| {
        face_key(std::array::from_fn(|corner| {
            let offset = 3 * indices[corner] as usize;
            std::array::from_fn(|axis| mesh.positions[offset + axis])
        }))
    })
}

fn main() -> Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let elevation = eo_data::ElevationGrid::load(&paths)?;
    let sample = |x, z| elevation.sample(x, z);
    let standards = eo_data::CaStandards::load(&paths)?;
    let city = LandmarkSet::build(&paths.repo_root, &sample, &standards)?;
    let contact = surfaces(
        LandmarksJs::get(),
        &sample,
        &DeckStandards::load(&standards)?,
    );
    let mut missing = 0;
    for (id, levels) in [
        ("golden-gate-bridge", &[LandmarkDeck::GoldenGate][..]),
        (
            "bay-bridge-west",
            &[LandmarkDeck::BayBridgeUpper, LandmarkDeck::BayBridgeLower][..],
        ),
    ] {
        let bridge = city
            .landmarks
            .iter()
            .find(|landmark| landmark.id == id)
            .with_context(|| format!("missing landmark {id}"))?;
        let drawn = |detail| -> HashSet<_> {
            bridge
                .draws
                .iter()
                .filter(|d| d.role == detail || d.role == Role::Always)
                .flat_map(|d| mesh_faces(&d.mesh))
                .collect()
        };
        let near = drawn(Role::Near);
        let far = drawn(Role::Far);
        for level in levels {
            let road = contact
                .iter()
                .find(|surface| surface.deck == *level)
                .with_context(|| format!("missing collision surface for {}", level.name()))?;
            let total = road.road.len();
            ensure!(total > 0, "empty collision surface for {}", level.name());
            let mut matches = [0; 2];
            for triangle in &road.road {
                let key = face_key(triangle.map(|point| point.map(|v| v as f32)));
                matches[0] += usize::from(near.contains(&key));
                matches[1] += usize::from(far.contains(&key));
            }
            println!(
                "{}: {total} contact faces, {} match in the near mesh, {} match in the distant mesh",
                level.name(),
                matches[0],
                matches[1]
            );
            missing += 2 * total - matches.iter().sum::<usize>();
        }
    }
    ensure!(
        missing == 0,
        "{missing} contact faces missing across the near and distant models"
    );
    Ok(())
}
