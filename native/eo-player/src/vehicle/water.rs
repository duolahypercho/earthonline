//! Four displacement samples over the wet footprint, with progressive loss of cabin air.
use super::Vehicle;
use crate::{CollisionWorld, tuning};
use rapier3d::prelude::*;

pub(super) const FLOOD_TICKS: u16 = 3600;

impl Vehicle {
    /// Run before the tires. Returns whether the intake is under water.
    pub(super) fn displace_water(&mut self, world: &mut CollisionWorld) -> bool {
        let car = &mut world.physics.bodies[self.body];
        let centre = car.translation();
        let specs = &self.spec;
        let Some((_, water)) = world.ground.water_column(centre.x.into(), centre.z.into()) else {
            self.flood_ticks = self.flood_ticks.saturating_sub(1);
            return false;
        };
        // Even on a bridge above a tidal channel the body stays entirely dry.
        let half_span = 0.5 * specs.length.hypot(specs.width).hypot(specs.height);
        if f64::from(centre.y) > water + half_span {
            self.flood_ticks = self.flood_ticks.saturating_sub(1);
            return false;
        }
        let rotation = *car.rotation();
        let dt = tuning::FIXED_STEP_S as f32;
        let g = tuning::GRAVITY_MPS2 as f32;
        let mass = specs.mass_kg as f32;
        let curb = specs.curb_mass_kg as f32;
        let u = f32::from(self.flood_ticks) / f32::from(FLOOD_TICKS);
        // Effective displaced volume (fresh-water equivalent). The sealed
        // components displace less than the vehicle's mass after flooding.
        let max_buoyancy = curb * g * (1.85 - 1.43 * u);
        let height = specs.height as f32;
        let mut max_depth = 0.0_f32;
        let mut immersed = false;
        for lx in [-0.38, 0.38] {
            for lz in [-0.36, 0.36] {
                let point = centre
                    + rotation
                        * Vector::new(
                            lx * specs.width as f32,
                            -height * 0.28,
                            lz * specs.length as f32 + u * specs.com_offset_z as f32,
                        );
                let Some((_, surface)) = world.ground.water_column(point.x.into(), point.z.into())
                else {
                    continue;
                };
                let depth = surface as f32 - point.y;
                let fraction = ((depth + 0.17 * height) / (0.70 * height)).clamp(0.0, 1.0);
                if fraction == 0.0 {
                    continue;
                }
                max_depth = max_depth.max(depth);
                immersed = true;
                // The centre of pressure rises as the column immerses. Acting
                // at the bottom of a fully wet hull makes it overturn in still water.
                let pressure = point + rotation * Vector::Y * ((0.35 * fraction - 0.17) * height);
                let velocity = car.velocity_at_point(pressure);
                // Bounded quadratic drag at each corner also damps pitch and roll.
                let drag = (0.75 * fraction * dt * velocity.length()).min(0.24) * mass * 0.25;
                let impulse = Vector::Y * (max_buoyancy * 0.25 * fraction * dt) - velocity * drag;
                car.apply_impulse_at_point(impulse, pressure, true);
            }
        }
        if max_depth > height * 0.18 {
            self.flood_ticks = self.flood_ticks.saturating_add(1).min(FLOOD_TICKS);
        } else if !immersed {
            self.flood_ticks = self.flood_ticks.saturating_sub(1);
        }
        let intake = centre + rotation * Vector::new(0.0, height * 0.06, specs.front_axle_z as f32);
        world
            .ground
            .water_column(intake.x.into(), intake.z.into())
            .is_some_and(|(_, surface)| surface >= f64::from(intake.y))
    }
}
