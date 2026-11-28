//! Contact between authoritative vehicles integrated in different streaming worlds.
use super::Vehicle;
use crate::CollisionWorld;
use glam::{DQuat, DVec3};
use rapier3d::{
    parry::query::{ContactManifold, details::contact_manifold_cuboid_cuboid},
    prelude::*,
};

fn inverse_mass_at(body: &RigidBody, at: Vector, axis: Vector) -> f32 {
    let m = body.mass_properties();
    let arm = (at - body.center_of_mass()).cross(axis);
    axis.dot(m.effective_inv_mass * axis) + arm.dot(m.effective_world_inv_inertia * arm)
}

fn contact(
    pa: &Pose,
    sa: &Cuboid,
    pb: &Pose,
    sb: &Cuboid,
    reach: f32,
) -> Option<(Vector, Vector, f32)> {
    let mut contacts = ContactManifold::<(), ()>::new();
    contact_manifold_cuboid_cuboid(&pa.inv_mul(pb), sa, sb, reach, &mut contacts);
    if contacts.points.is_empty() {
        return None;
    }
    let mut at = Vector::ZERO;
    let mut gap = f32::INFINITY;
    for c in &contacts.points {
        at += (*pa * c.local_p1 + *pb * c.local_p2) * 0.5;
        gap = gap.min(c.dist);
    }
    Some((
        pa.rotation * contacts.local_n1,
        at / contacts.points.len() as f32,
        gap,
    ))
}

/// Test the scripted body against a real chassis before spending a dynamic body on it.
pub(crate) fn approaching_traffic(
    car: &Vehicle,
    world: &CollisionWorld,
    centre: DVec3,
    rotation: DQuat,
    half: DVec3,
    velocity: DVec3,
) -> bool {
    let collider = &world.physics.colliders[car.collider];
    let body = &world.physics.bodies[car.body];
    let Some(shape) = collider.shape().as_cuboid() else {
        return false;
    };
    let dt = crate::tuning::FIXED_STEP_S as f32;
    let velocity = Vector::from_array(velocity.as_vec3().to_array());
    let pose = Pose::from_parts(
        Vector::from_array(centre.as_vec3().to_array()),
        Rotation::from_array(rotation.as_quat().to_array()),
    );
    let bounds = Cuboid::new(Vector::from_array(half.as_vec3().to_array()));
    let reach = (body.linvel() - velocity).length() * dt + 0.004;
    contact(collider.position(), shape, &pose, &bounds, reach).is_some_and(|(normal, at, gap)| {
        let closing = (body.velocity_at_point(at) - velocity).dot(normal);
        closing > 0.01 && closing * dt >= gap
    })
}

/// Apply one pair of equal and opposite impulses about the centre of the
/// overlapping bumper/side faces. Isolated server worlds keep their own road
/// colliders and continue to integrate on the same fixed step.
pub fn resolve_peer_contact(
    a: &Vehicle,
    wa: &mut CollisionWorld,
    b: &Vehicle,
    wb: &mut CollisionWorld,
) -> bool {
    let (ca, cb) = (
        &wa.physics.colliders[a.collider],
        &wb.physics.colliders[b.collider],
    );
    let (Some(sa), Some(sb)) = (ca.shape().as_cuboid(), cb.shape().as_cuboid()) else {
        return false;
    };
    let (ba, bb) = (
        &mut wa.physics.bodies[a.body],
        &mut wb.physics.bodies[b.body],
    );
    if !ba.is_enabled() || !bb.is_enabled() {
        return false;
    }
    let dt = crate::tuning::FIXED_STEP_S as f32;
    let approach = (ba.linvel() - bb.linvel()).length() * dt + 0.004;
    let Some((normal, at, gap)) = contact(ca.position(), sa, cb.position(), sb, approach) else {
        return false;
    };
    let relative = bb.velocity_at_point(at) - ba.velocity_at_point(at);
    let closing = -relative.dot(normal);
    // A resting overlap at a shared spawn cannot create kinetic energy.
    if closing <= 0.01 || closing * dt < gap {
        return false;
    }
    let normal_mass = inverse_mass_at(ba, at, normal) + inverse_mass_at(bb, at, normal);
    if normal_mass <= 0.0 {
        return false;
    }
    let magnitude = (1.0 + ca.restitution().min(cb.restitution())) * closing / normal_mass;
    let tangent = (relative + normal * closing).normalize_or_zero();
    let tangent_mass = inverse_mass_at(ba, at, tangent) + inverse_mass_at(bb, at, tangent);
    let friction = if tangent_mass > 0.0 {
        (relative.dot(tangent) / tangent_mass)
            .min(magnitude * (ca.friction() * cb.friction()).sqrt())
    } else {
        0.0
    };
    let impulse = normal * magnitude - tangent * friction;
    ba.apply_impulse_at_point(-impulse, at, true);
    bb.apply_impulse_at_point(impulse, at, true);
    true
}
