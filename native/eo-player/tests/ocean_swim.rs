//! A submerged player uses the ordinary fixed step, the capsule and the mapped floor.
use eo_player::world::{BuildingSet, CollisionWorld, GroundProvider};
use eo_player::{Player, StepInput};
use glam::DVec3;
use std::sync::Arc;

struct TestShore;
impl GroundProvider for TestShore {
    fn ground_y(&self, _: f64, z: f64) -> f64 {
        (-6. + (z - 3.).max(0.)).min(1.)
    }
    fn water_column(&self, x: f64, z: f64) -> Option<(f64, f64)> {
        let floor = self.ground_y(x, z);
        (floor < 0.).then_some((floor, 0.))
    }
}

#[test]
fn swim_surfaces_dives_and_walks_onto_a_bank() {
    let mut world = CollisionWorld::new(Arc::new(TestShore), Arc::new(BuildingSet::empty()), 1);
    let feet = DVec3::new(0., -4., 0.);
    world.load_blocking(feet, 32.);
    let mut player = Player::new(feet, 0.);
    assert!(player.is_swimming(&world));
    for _ in 0..270 {
        player.fixed_step(
            &mut world,
            &StepInput {
                jump: true,
                ..Default::default()
            },
        );
    }
    let surface = player.camera(&world, 1. / 60., 1.6);
    assert!(surface.eye.y > 0. && surface.eye.y < 0.6);
    for _ in 0..330 {
        player.fixed_step(
            &mut world,
            &StepInput {
                crouch: true,
                ..Default::default()
            },
        );
    }
    let bottom = player.character.feet().y;
    assert!((-6.0..-5.8).contains(&bottom), "{bottom}");
    assert!(player.camera(&world, 1. / 60., 1.6).eye.y < 0.);
    for _ in 0..310 {
        player.fixed_step(
            &mut world,
            &StepInput {
                jump: true,
                ..Default::default()
            },
        );
    }
    for _ in 0..880 {
        player.fixed_step(
            &mut world,
            &StepInput {
                move_y: 1.,
                ..Default::default()
            },
        );
    }
    assert!(!player.is_swimming(&world));
    assert!(player.character.feet().z > 9. && player.character.feet().y > 0.);
}
