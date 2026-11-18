//! Camera, projection and the frustum test the cull shader mirrors.

use glam::{Mat4, Vec3, Vec4};

/// Vertical field of view of the game's city camera
/// (`new THREE.PerspectiveCamera(52, 1, 0.5, 4200)` in src/citygen/renderer.js).
pub const GAME_FOV_Y_DEG: f32 = 52.0;
pub const GAME_NEAR_M: f32 = 0.5;
pub const GAME_FAR_M: f32 = 4200.0;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: Vec3,
    /// Radians; 0 looks north (-z), positive turns toward east (+x).
    pub yaw: f32,
    /// Radians; positive looks up.
    pub pitch: f32,
    pub fov_y_deg: f32,
    pub near: f32,
    pub far: f32,
    pub aspect: f32,
    /// sub-pixel offset of the projection (NDC), the temporal resolve's jitter.
    pub jitter: [f32; 2],
}

impl Camera {
    pub fn new(eye: Vec3, yaw: f32, pitch: f32, aspect: f32) -> Self {
        Self { eye, yaw, pitch, fov_y_deg: GAME_FOV_Y_DEG, near: GAME_NEAR_M, far: GAME_FAR_M, aspect, jitter: [0.0; 2] }
    }

    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }

    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, sy)
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_to_mat4(self.eye, self.forward(), Vec3::Y)
    }

    /// Reversed-Z perspective: near maps to depth 1, far to depth 0.
    pub fn proj(&self) -> Mat4 {
        let p = glam::camera::rh::proj::directx::perspective(self.fov_y_deg.to_radians(), self.aspect, self.far, self.near);
        // the jitter moves clip x/y by jitter x w (identity at zero).
        if self.jitter == [0.0; 2] { p } else { Mat4::from_translation(Vec3::new(self.jitter[0], self.jitter[1], 0.0)) * p }
    }

    pub fn view_proj(&self) -> Mat4 {
        self.proj() * self.view()
    }
}

/// Six normalised planes (n.xyz, d) with inside where n.p + d >= 0, extracted
/// from a clip-space matrix whose depth range is [0, 1] (Gribb-Hartmann).
pub fn frustum_planes(view_proj: Mat4) -> [Vec4; 6] {
    let r0 = view_proj.row(0);
    let r1 = view_proj.row(1);
    let r2 = view_proj.row(2);
    let r3 = view_proj.row(3);
    let mut planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2];
    for p in &mut planes {
        let l = p.truncate().length();
        if l > 1e-12 {
            *p /= l;
        } else {
            // Degenerate plane (e.g. an infinite far plane): accept everything.
            *p = Vec4::new(0.0, 0.0, 0.0, 1.0);
        }
    }
    planes
}

/// CPU reference of the shader's AABB-vs-frustum test.
pub fn aabb_visible(planes: &[Vec4; 6], min: Vec3, max: Vec3) -> bool {
    planes.iter().all(|pl| {
        let n = pl.truncate();
        let p = Vec3::new(
            if n.x >= 0.0 { max.x } else { min.x },
            if n.y >= 0.0 { max.y } else { min.y },
            if n.z >= 0.0 { max.z } else { min.z },
        );
        n.dot(p) + pl.w >= 0.0
    })
}
