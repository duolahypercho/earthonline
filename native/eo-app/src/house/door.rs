//! The Blender leaf and its collider share a rigid transform about the hinge.
use super::House;
use anyhow::{Result, ensure};
use eo_player::world::{CollisionWorld, MovableBox};
use eo_render::scene::{Scene, Vertex};
use glam::{DQuat, DVec2, DVec3, Quat};

struct LeafSpan {
    first: usize,
    closed: Vec<Vertex>,
}

pub struct Door {
    hinge: DVec3,
    closed_center: DVec3,
    closed_tip: DVec3,
    closed_yaw: f64,
    half_height: f64,
    half_thickness: f64,
    spans: Vec<LeafSpan>,
    collider: MovableBox,
    fraction: f64,
    opening: bool,
}

impl Door {
    pub fn new(
        house: &House,
        scene: &Scene,
        vertex_base: usize,
        collision: &mut CollisionWorld,
    ) -> Result<Self> {
        let spec = &house.door;
        let [lo, hi] = spec.bounds;
        let hinge = house.point(spec.hinge[0], spec.hinge[1], spec.hinge[2]);
        let center = house.point(
            (lo[0] + hi[0]) * 0.5,
            (lo[1] + hi[1]) * 0.5,
            (lo[2] + hi[2]) * 0.5,
        );
        let half = DVec3::new(
            (hi[0] - lo[0]) * house.across.length(),
            (hi[1] - lo[1]) * house.height_scale,
            (hi[2] - lo[2]) * house.back.length(),
        ) * 0.5;
        let yaw = -house.across.y.atan2(house.across.x);
        let mut spans = Vec::new();
        let mut found = std::collections::HashSet::new();
        for part in &house.model.parts {
            let name = &house.model.materials[part.material as usize].name;
            if !spec.materials.contains(name) {
                continue;
            }
            found.insert(name);
            let i = part.first_index as usize;
            let indices = &house.model.indices[i..i + part.index_count as usize];
            let Some(&low) = indices.iter().min() else {
                continue;
            };
            let high = *indices.iter().max().unwrap() as usize;
            let first = vertex_base + low as usize;
            spans.push(LeafSpan {
                first,
                closed: scene.vertices[first..=vertex_base + high].to_vec(),
            });
        }
        ensure!(
            !spans.is_empty() && found.len() == spec.materials.len(),
            "front door meshes missing from the verified house"
        );
        Ok(Self {
            hinge,
            closed_center: center,
            closed_tip: house.point(lo[0], (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5),
            closed_yaw: yaw,
            half_height: half.y,
            half_thickness: half.z,
            spans,
            collider: MovableBox::new(collision, half, center, yaw),
            fraction: 0.,
            opening: false,
        })
    }

    fn angle(fraction: f64) -> f64 {
        fraction * fraction * (3. - 2. * fraction) * std::f64::consts::FRAC_PI_2
    }

    fn rotate(&self, point: DVec3, angle: f64) -> DVec3 {
        self.hinge + DQuat::from_rotation_y(angle) * (point - self.hinge)
    }

    pub fn in_reach(&self, feet: DVec3) -> bool {
        let hand = feet + DVec3::Y;
        hand.distance(self.rotate(self.closed_tip, Self::angle(self.fraction))) < 2.15
    }

    pub fn toggle(&mut self, feet: DVec3) -> bool {
        if !self.in_reach(feet) {
            return false;
        }
        self.opening = !self.opening;
        true
    }

    pub fn action_hint(&self) -> &'static str {
        if self.opening {
            " - E: close front door"
        } else {
            " - E: open front door"
        }
    }

    pub fn is_shut(&self) -> bool {
        self.fraction == 0.
    }

    // Nearest point on the swinging leaf in the horizontal plane. Stop the
    // swing against an occupant; allow a reversal that increases clearance.
    fn clearance(&self, feet: DVec3, angle: f64) -> f64 {
        let tip = self.rotate(self.closed_tip, angle) - self.hinge;
        let axis = DVec2::new(tip.x, tip.z);
        let person = DVec2::new(feet.x - self.hinge.x, feet.z - self.hinge.z);
        let t = (person.dot(axis) / axis.length_squared()).clamp(0., 1.);
        (person - axis * t).length()
    }

    pub fn advance(
        &mut self,
        dt: f64,
        feet: DVec3,
        scene: &mut Scene,
        collision: &mut CollisionWorld,
    ) -> bool {
        let delta = dt / 0.45 * if self.opening { 1. } else { -1. };
        let next = (self.fraction + delta).clamp(0., 1.);
        if self.fraction == next {
            return false;
        }
        let angle = Self::angle(next);
        let previous_angle = Self::angle(self.fraction);
        let radial = self.closed_tip - self.hinge;
        let occupant = feet - self.hinge;
        // Check the closest orientation in the whole sweep. A slow frame
        // can otherwise carry the tip past a person between two updates.
        let facing = (radial.z * occupant.x - radial.x * occupant.z)
            .atan2(radial.x * occupant.x + radial.z * occupant.z);
        let nearest = facing.clamp(previous_angle.min(angle), previous_angle.max(angle));
        let clearance = self.clearance(feet, nearest);
        if feet.y < self.closed_center.y + self.half_height
            && feet.y + 1.8 > self.hinge.y
            && clearance < eo_player::tuning::capsule::RADIUS_M + self.half_thickness + 0.015
            && clearance + 0.0001 < self.clearance(feet, previous_angle)
        {
            return false;
        }
        self.fraction = next;
        self.collider.move_to(
            collision,
            self.rotate(self.closed_center, angle),
            self.closed_yaw + angle,
        );
        let rotation = Quat::from_rotation_y(angle as f32);
        let pivot = self.hinge.as_vec3();
        for span in &self.spans {
            for (live, rest) in scene.vertices[span.first..span.first + span.closed.len()]
                .iter_mut()
                .zip(&span.closed)
            {
                *live = *rest;
                if next != 0. {
                    live.pos =
                        (pivot + rotation * (glam::Vec3::from_array(rest.pos) - pivot)).to_array();
                    live.normal = (rest.normal & 0xff00_0000)
                        | eo_assets::glb::pack_normal(
                            rotation * eo_assets::glb::unpack_normal(rest.normal),
                        );
                }
            }
        }
        true
    }

    pub fn upload(&self, scene: &Scene, r: &eo_render::Renderer, q: &eo_render::wgpu::Queue) {
        for span in &self.spans {
            r.write(
                q,
                eo_render::upload::Target::Vertex,
                (span.first * std::mem::size_of::<Vertex>()) as u64,
                bytemuck::cast_slice(&scene.vertices[span.first..span.first + span.closed.len()]),
            );
        }
    }
}
