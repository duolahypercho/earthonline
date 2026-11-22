//! The moored boats' poses each frame (`--no-boat-motion` leaves them at
//! rest): the physics' for the boats a body carries (eo-player `harbour`,
//! near the player), else floating at rest on the drawn sea within
//! `FLOAT_RADIUS_M` of the camera (`harbour::float_pose_with`), else at rest.
//! Both read the sea the water draws (eo-water `sea`, the water's plan and
//! clock), so where its waves run the boats ride them and where it lies
//! still they do. Written to the props' boat block (heave in the height,
//! yaw, roll and pitch in the instance word, eo-props `boats::afloat_word`).

use eo_player::harbour::{self, Fleet, Moored, Sea};
use eo_render::props::GpuPropInstance;
use eo_render::upload::Target;
use eo_render::{GpuContext, Renderer};
use glam::{DQuat, DVec3, EulerRot, Vec3};
use std::collections::HashMap;
use std::sync::Arc;

/// Boats past this from the camera lie at rest (a 10 m hull is a few pixels there).
pub const FLOAT_RADIUS_M: f64 = 2500.0;

pub struct BoatMotion {
    fleet: Arc<Fleet>,
    /// per fleet boat, its render model (`props_lib2::BOAT_BASE` + body).
    models: Vec<u32>,
    water: Arc<eo_water::WaterData>,
    sea: Option<Arc<Sea>>,
    state: Option<eo_water::waves::SeaState>,
    building: Option<(eo_water::waves::SeaState, std::thread::JoinHandle<eo_water::sea::SeaField>)>,
    physics: HashMap<usize, (DVec3, DQuat)>,
    instances: Vec<GpuPropInstance>,
}

impl BoatMotion {
    pub fn new(fleet: Arc<Fleet>, models: Vec<u32>, water: Arc<eo_water::WaterData>) -> Self {
        let instances = rest_instances(&fleet, &models);
        Self { fleet, models, water, sea: None, state: None, building: None, physics: HashMap::new(), instances }
    }

    /// The sea the boats float on, once built for the water's sea state.
    pub fn sea(&self) -> Option<Arc<Sea>> {
        self.sea.clone()
    }

    /// The poses the physics' bodies have this frame (fleet index, waterline middle, rotation).
    pub fn set_physics(&mut self, poses: Vec<(usize, DVec3, DQuat)>) {
        self.physics = poses.into_iter().map(|(i, p, q)| (i, (p, q))).collect();
    }

    /// Keep the sea on the water's plan: a new sea state builds its field on a thread.
    fn pump_sea(&mut self, renderer: &Renderer) {
        let Some(plan) = renderer.water.get().and_then(|w| w.plan()) else { return };
        if self.state == Some(plan.state) {
            return;
        }
        if let Some((st, job)) = self.building.take() {
            if st == plan.state && job.is_finished() {
                if let Ok(field) = job.join() {
                    self.sea = Some(Sea::new(Arc::new(field), self.water.clone() as Arc<dyn harbour::ShoreLookup>));
                    self.state = Some(st);
                }
                return;
            }
            if st == plan.state {
                self.building = Some((st, job));
                return;
            }
        }
        let st = plan.state;
        self.building = Some((st, std::thread::Builder::new().name("eo-boats-sea".into()).spawn(move || eo_water::sea::SeaField::new(&plan)).expect("spawn sea field")));
    }

    /// Pose every boat now and write the props' boat block (`block`: at, n).
    pub fn update(&mut self, ctx: &GpuContext, renderer: &Renderer, block: (u32, u32), eye: Vec3) {
        self.pump_sea(renderer);
        let Some(sea) = self.sea.clone() else { return };
        if block.1 as usize != self.instances.len() {
            return;
        }
        let t = renderer.water_clock();
        let sea_y = self.fleet.sea_y;
        for (i, m) in self.fleet.boats.iter().enumerate() {
            let pose = match self.physics.get(&i) {
                Some(p) => Some(*p),
                None if (m.pos[0] - eye.x as f64).hypot(m.pos[1] - eye.z as f64) <= FLOAT_RADIUS_M => harbour::float_pose_with(m, sea_y, |x, z| sea.try_sample(x, z, t).map(|s| s.height)),
                None => Some((DVec3::new(m.pos[0], sea_y, m.pos[1]), DQuat::from_rotation_y(m.yaw()))),
            };
            if let Some((p, q)) = pose {
                self.instances[i] = instance(p, q, self.models[i]);
            }
        }
        renderer.write(&ctx.queue, Target::PropInstances, block.0 as u64 * 48, bytemuck::cast_slice(&self.instances));
    }
}

fn instance(p: DVec3, q: DQuat, model: u32) -> GpuPropInstance {
    // q = Ry(yaw) Rx(pitch) Rz(roll): the shader turns roll, then pitch, then yaw (props_types.wgsl `pr_local`).
    let (yaw, pitch, roll) = q.to_euler(EulerRot::YXZ);
    GpuPropInstance { pos: [p.x as f32, p.y as f32, p.z as f32], yaw: yaw as f32, scale: [1.0; 3], model, variant: 0, lamp: 0, slope: 0.0, pad: eo_props::boats::afloat_word(roll, pitch) }
}

/// The fleet at rest on the still water, in its order (the props' first upload).
pub fn rest_instances(fleet: &Fleet, models: &[u32]) -> Vec<GpuPropInstance> {
    fleet.boats.iter().zip(models).map(|(m, &model)| rest_instance(m, fleet.sea_y, model)).collect()
}

fn rest_instance(m: &Moored, sea_y: f64, model: u32) -> GpuPropInstance {
    GpuPropInstance { pos: [m.pos[0] as f32, sea_y as f32, m.pos[1] as f32], yaw: m.yaw() as f32, scale: [1.0; 3], model, variant: 0, lamp: 0, slope: 0.0, pad: 0 }
}
