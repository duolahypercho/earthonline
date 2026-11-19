//! The water's extra field: one RGBA16F texture over a rectangle of the
//! water, beside the shore field and the exposure field, for what a later
//! pass measures over the sea (for example depth from a bathymetric survey,
//! or the surf zone). eo-render uploads it to the water, underwater, spray
//! and surf pipelines (group 0, `w_extra` and `w_extra_samp` in
//! water_common.wgsl) with its frame in the water uniform (`wv_extra`).
//! (iteration 3) The field is the measured sea bed and the swell over it
//! (`bathymetry.rs`), installed by `crate::load` when the survey folder is
//! there and `--no-bathymetry` is not given; without one `field()` answers
//! None, which uploads a 1 x 1 zero texture with the identity frame, and the
//! water draws as before.

/// An extra field: `width` x `height` texels of `cell_m` (game metres) from
/// `origin` (game x, z of texel (0, 0)'s outer corner), rows along +z, each
/// texel four IEEE 754 half floats (bits) R, G, B, A.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtraField {
    pub origin: [f64; 2],
    pub cell_m: f64,
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[u16; 4]>,
}

impl ExtraField {
    /// The field's frame as the water uniform carries it: origin x, z and
    /// 1 / extent x, z (so a world point maps to its texture coordinate as
    /// (xz - origin) x inverse extent).
    pub fn frame(&self) -> [f64; 4] {
        [self.origin[0], self.origin[1], 1.0 / (self.width as f64 * self.cell_m), 1.0 / (self.height as f64 * self.cell_m)]
    }
}

/// The frame uploaded without a field: origin 0, inverse extent 1.
pub const IDENTITY_FRAME: [f64; 4] = [0.0, 0.0, 1.0, 1.0];

static INSTALLED: std::sync::OnceLock<ExtraField> = std::sync::OnceLock::new();
static COMPOSED: std::sync::OnceLock<(ExtraField, u64)> = std::sync::OnceLock::new();

/// The extra field in force (None before `install` or without one): the
/// installed field with the obstacles registered by then (`obstacles.rs`)
/// drawn into its land distance (A), unless --no-obstacle-foam. Composed on
/// the first call (the water pass makes its texture once; obstacles set
/// later are not in it: `composed_revision` says which list it holds).
pub fn field() -> Option<&'static ExtraField> {
    let base = INSTALLED.get()?;
    Some(
        &COMPOSED
            .get_or_init(|| {
                let rev = crate::obstacles::revision();
                let list = crate::obstacles::get();
                if rev == 0 || list.is_empty() || !crate::switches::is_on(crate::switches::OBSTACLE_FOAM) {
                    return (base.clone(), rev);
                }
                (with_obstacles(base, &list), rev)
            })
            .0,
    )
}

/// The obstacle list's revision the composed field holds (None before `field` is first called).
pub fn composed_revision() -> Option<u64> {
    COMPOSED.get().map(|c| c.1)
}

/// The obstacles at or near the water (top no lower than `OBSTACLE_AWASH_M`
/// under the sea: an awash rock breaks the waves too) drawn into the field's
/// land distance (A, metres, as `bathymetry::LAND_DIST_CAP_M` caps it): each
/// texel takes the least of its own and its distance to every obstacle's rim.
pub fn with_obstacles(f: &ExtraField, list: &[crate::obstacles::Obstacle]) -> ExtraField {
    let mut out = f.clone();
    let cap = crate::bathymetry::LAND_DIST_CAP_M;
    for o in list.iter().filter(|o| o.height > -OBSTACLE_AWASH_M && o.radius > 0.0) {
        let r = o.radius + cap;
        let i0 = (((o.x - r - f.origin[0]) / f.cell_m).floor().max(0.0)) as usize;
        let j0 = (((o.z - r - f.origin[1]) / f.cell_m).floor().max(0.0)) as usize;
        let i1 = (((o.x + r - f.origin[0]) / f.cell_m).ceil() as isize).clamp(0, f.width as isize) as usize;
        let j1 = (((o.z + r - f.origin[1]) / f.cell_m).ceil() as isize).clamp(0, f.height as isize) as usize;
        for j in j0..j1 {
            for i in i0..i1 {
                let (x, z) = (f.origin[0] + (i as f64 + 0.5) * f.cell_m, f.origin[1] + (j as f64 + 0.5) * f.cell_m);
                let d = ((x - o.x).hypot(z - o.z) - o.radius).max(0.0);
                let t = &mut out.texels[j * f.width + i];
                if (d as f32) < crate::bathymetry::half_to_f32(t[3]) {
                    t[3] = crate::field::to_half_three(d);
                }
            }
        }
    }
    out
}

/// How far under the sea an obstacle's top may lie and still break the waves (m, INFERRED).
pub const OBSTACLE_AWASH_M: f64 = 0.5;

/// Install the field (the first one stays).
pub fn install(f: ExtraField) -> &'static ExtraField {
    INSTALLED.get_or_init(|| f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obstacles_draw_into_the_land_distance() {
        let h = crate::field::to_half_three;
        let f = ExtraField { origin: [0.0, 0.0], cell_m: 2.0, width: 20, height: 10, texels: vec![[h(5.0), h(1.0), h(0.0), h(60.0)]; 200] };
        let rock = crate::obstacles::Obstacle { x: 10.0, z: 10.0, radius: 3.0, height: 0.4 };
        let deep = crate::obstacles::Obstacle { x: 30.0, z: 10.0, radius: 3.0, height: -4.0 };
        let g = with_obstacles(&f, &[rock, deep]);
        let a = |i: usize, j: usize| crate::bathymetry::half_to_f32(g.texels[j * 20 + i][3]);
        assert_eq!(a(4, 4), 0.0); // inside the rock (texel centre (9, 9))
        assert!((a(8, 4) - (7.0f32.hypot(1.0) - 3.0)).abs() < 0.05); // its rim's distance
        assert!((a(15, 5) - (21.0f32.hypot(1.0) - 3.0)).abs() < 0.1); // the drowned rock breaks nothing (only the first rock reaches there)
        assert_eq!(g.texels[0][..3], f.texels[0][..3]);
    }

    #[test]
    fn none_yet_and_the_frame_inverts_the_extent() {
        assert!(field().is_none());
        let f = ExtraField { origin: [-100.0, 50.0], cell_m: 4.0, width: 25, height: 10, texels: vec![[0; 4]; 250] };
        assert_eq!(f.frame(), [-100.0, 50.0, 0.01, 0.025]);
        assert_eq!(IDENTITY_FRAME, [0.0, 0.0, 1.0, 1.0]);
    }
}
