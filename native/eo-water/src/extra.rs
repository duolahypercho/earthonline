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
/// texel four IEEE 754 half floats (bits) R, G, B, A. The rows hold `layers`
/// fields over the same rectangle one after the other (`height / layers`
/// rows each): the field `field()` uploads has two (`LAYERS`), the measured
/// bed (`bathymetry.rs`) and the obstacles' layer (`with_obstacles`); the
/// water's `wb_set` reads layer 0 in the texture's upper half, `wb_obstacle` layer 1 in the lower.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtraField {
    pub origin: [f64; 2],
    pub cell_m: f64,
    pub width: usize,
    pub height: usize,
    pub layers: usize,
    pub texels: Vec<[u16; 4]>,
}

impl ExtraField {
    /// The field's frame as the water uniform carries it: origin x, z and
    /// 1 / extent x, z of one layer (so a world point maps to its coordinate
    /// in a layer as (xz - origin) x inverse extent).
    pub fn frame(&self) -> [f64; 4] {
        [self.origin[0], self.origin[1], 1.0 / (self.width as f64 * self.cell_m), 1.0 / (self.layer_rows() as f64 * self.cell_m)]
    }

    /// Rows per layer.
    pub fn layer_rows(&self) -> usize {
        self.height / self.layers.max(1)
    }
}

/// Layers in the uploaded field (water_common.wgsl `wb_set` and `wb_obstacle` read them so).
pub const LAYERS: usize = 2;

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
                (with_obstacles(base, &in_force()), rev)
            })
            .0,
    )
}

/// The installed field composed afresh with the obstacles registered now and
/// their revision (None without a field): for a renderer that re-uploads when
/// `obstacles::revision()` passes `composed_revision()`.
pub fn compose() -> Option<(ExtraField, u64)> {
    let base = INSTALLED.get()?;
    let rev = crate::obstacles::revision();
    Some((with_obstacles(base, &in_force()), rev))
}

/// The obstacles the field draws: the registered ones, none with --no-obstacle-foam.
fn in_force() -> Vec<crate::obstacles::Obstacle> {
    if !crate::switches::is_on(crate::switches::OBSTACLE_FOAM) {
        return Vec::new();
    }
    crate::obstacles::get().as_ref().clone()
}

/// The obstacle list's revision the composed field holds (None before `field` is first called).
pub fn composed_revision() -> Option<u64> {
    COMPOSED.get().map(|c| c.1)
}

/// The installed field (`f`, one layer) with the obstacles at or near the
/// water (top no lower than `OBSTACLE_AWASH_M` under the sea: an awash rock
/// breaks the waves too): layer 0 is `f` with each texel's land distance (A,
/// metres, as `bathymetry::LAND_DIST_CAP_M` caps it) the least of its own and
/// its distance to every obstacle's rim; layer 1 (`LAYERS`) holds, per texel,
/// R the distance to the nearest obstacle's rim alone (capped the same), G and
/// B the tidal current's wake behind the obstacles on the flood and on the
/// ebb (`wake`), A zero. Without obstacles layer 1 is R at the cap, no wake.
pub fn with_obstacles(f: &ExtraField, list: &[crate::obstacles::Obstacle]) -> ExtraField {
    let rows = f.layer_rows();
    let n = f.width * rows;
    let cap = crate::bathymetry::LAND_DIST_CAP_M;
    let h = crate::field::to_half_three;
    let half = crate::bathymetry::half_to_f32;
    let mut l0: Vec<[u16; 4]> = f.texels[..n].to_vec();
    let mut obs = vec![cap as f32; n];
    let mut wake = vec![[0f32; 2]; n];
    let centre = |i: usize, j: usize| (f.origin[0] + (i as f64 + 0.5) * f.cell_m, f.origin[1] + (j as f64 + 0.5) * f.cell_m);
    let window = |x0: f64, z0: f64, x1: f64, z1: f64| {
        let i0 = (((x0 - f.origin[0]) / f.cell_m).floor().max(0.0)) as usize;
        let j0 = (((z0 - f.origin[1]) / f.cell_m).floor().max(0.0)) as usize;
        let i1 = (((x1 - f.origin[0]) / f.cell_m).ceil() as isize).clamp(0, f.width as isize) as usize;
        let j1 = (((z1 - f.origin[1]) / f.cell_m).ceil() as isize).clamp(0, rows as isize) as usize;
        (i0, j0, i1, j1)
    };
    let list: Vec<_> = list.iter().filter(|o| o.height > -OBSTACLE_AWASH_M && o.radius > 0.0).copied().collect();
    for o in &list {
        let r = o.radius + cap;
        let (i0, j0, i1, j1) = window(o.x - r, o.z - r, o.x + r, o.z + r);
        for j in j0..j1 {
            for i in i0..i1 {
                let (x, z) = centre(i, j);
                let d = ((x - o.x).hypot(z - o.z) - o.radius).max(0.0) as f32;
                obs[j * f.width + i] = obs[j * f.width + i].min(d);
            }
        }
        // the wake, downstream on the flood (k 0) and on the ebb (k 1)
        for (k, sgn) in [(0usize, 1.0f64), (1, -1.0)] {
            let flow = [WAKE_FLOW[0] * sgn, WAKE_FLOW[1] * sgn];
            let reach = o.radius + WAKE_LENGTH_M;
            let wide = o.radius + WAKE_WIDTH_M[0] + WAKE_WIDTH_M[1] * WAKE_LENGTH_M;
            let (ex, ez) = (o.x + flow[0] * reach, o.z + flow[1] * reach);
            let (i0, j0, i1, j1) = window(o.x.min(ex) - wide, o.z.min(ez) - wide, o.x.max(ex) + wide, o.z.max(ez) + wide);
            for j in j0..j1 {
                for i in i0..i1 {
                    let (x, z) = centre(i, j);
                    let w = wake_at(x - o.x, z - o.z, flow, o.radius);
                    let q = j * f.width + i;
                    wake[q][k] = wake[q][k].max(w as f32);
                }
            }
        }
    }
    let mut l1 = Vec::with_capacity(n);
    for q in 0..n {
        let own = half(f.texels[q][3]);
        // (the wake hugs its obstacle: it fades out within one obstacle radius,
        // at least a texel, of land that is not an obstacle, so a seawall or a
        // rocky shore downstream draws no trail of its own)
        let mut fade = 0.0;
        if wake[q][0] > 0.0 || wake[q][1] > 0.0 {
            let (x, z) = centre(q % f.width, q / f.width);
            let nearest = list.iter().min_by(|a, b| (x - a.x).hypot(z - a.z).total_cmp(&(x - b.x).hypot(z - b.z)));
            let r = nearest.map_or(f.cell_m, |o| o.radius.max(f.cell_m));
            fade = smooth(0.0, r, own as f64) as f32;
        }
        if obs[q] < own {
            l0[q][3] = h(obs[q] as f64);
        }
        l1.push([h(obs[q] as f64), h((wake[q][0] * fade) as f64), h((wake[q][1] * fade) as f64), 0]);
    }
    let mut texels = l0;
    texels.extend(l1);
    ExtraField { origin: f.origin, cell_m: f.cell_m, width: f.width, height: rows * LAYERS, layers: LAYERS, texels }
}

/// The tidal current's wake behind an obstacle of `radius` at (`dx`, `dz`)
/// from its centre, the current running along `flow` (unit): a turbulent
/// white trail from the obstacle's flanks, widening and fading downstream
/// (INFERRED: visible white water over a few obstacle widths in a current
/// over about 0.5 m/s): full within `WAKE_WIDTH_M[0]` beyond the obstacle's
/// half width, gone at `WAKE_WIDTH_M[0] + WAKE_WIDTH_M[1] s` at `s` metres
/// downstream, falling as exp(-s / `WAKE_DECAY_M`), none past `WAKE_LENGTH_M`.
pub fn wake_at(dx: f64, dz: f64, flow: [f64; 2], radius: f64) -> f64 {
    let s = dx * flow[0] + dz * flow[1];
    if s <= -radius || s > radius + WAKE_LENGTH_M {
        return 0.0;
    }
    let lat = (dx * flow[1] - dz * flow[0]).abs();
    let s0 = s.max(0.0);
    let d = (lat - radius).max(0.0);
    let start = smooth(-radius, 0.0, s);
    smooth(WAKE_WIDTH_M[0] + WAKE_WIDTH_M[1] * s0, 0.5, d) * (-s0 / WAKE_DECAY_M).exp() * start
}

/// The flood current's heading as a unit vector on the ground plan (game x,
/// z): the Golden Gate's tidal axis, heading 255 degrees reversed (water.wgsl `w_tidal`); the ebb runs opposite.
pub const WAKE_FLOW: [f64; 2] = [0.966, -0.259];
/// The wake's length downstream (m, INFERRED, as above).
pub const WAKE_LENGTH_M: f64 = 160.0;
/// The wake's full half width beyond the obstacle's and its spread per metre downstream (INFERRED).
pub const WAKE_WIDTH_M: [f64; 2] = [3.0, 0.12];
/// The wake's e-folding length downstream (m, INFERRED).
pub const WAKE_DECAY_M: f64 = 90.0;

fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far under the sea an obstacle's top may lie and still break the waves (m, INFERRED).
pub const OBSTACLE_AWASH_M: f64 = 0.5;

/// The installed field as installed, without the obstacles `field()` draws into it
/// (its layer 0's R, G and B are the water's: the obstacles change only its A and layer 1).
pub fn installed() -> Option<&'static ExtraField> {
    INSTALLED.get()
}

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
        let f = ExtraField { origin: [0.0, 0.0], cell_m: 2.0, width: 20, height: 10, layers: 1, texels: vec![[h(5.0), h(1.0), h(0.0), h(60.0)]; 200] };
        let rock = crate::obstacles::Obstacle { x: 10.0, z: 10.0, radius: 3.0, height: 0.4 };
        let deep = crate::obstacles::Obstacle { x: 30.0, z: 10.0, radius: 3.0, height: -4.0 };
        let g = with_obstacles(&f, &[rock, deep]);
        assert_eq!((g.width, g.height, g.layers, g.texels.len()), (20, 20, 2, 400));
        assert_eq!(g.frame(), f.frame());
        let a = |i: usize, j: usize| crate::bathymetry::half_to_f32(g.texels[j * 20 + i][3]);
        assert_eq!(a(4, 4), 0.0); // inside the rock (texel centre (9, 9))
        assert!((a(8, 4) - (7.0f32.hypot(1.0) - 3.0)).abs() < 0.05); // its rim's distance
        assert!((a(15, 5) - (21.0f32.hypot(1.0) - 3.0)).abs() < 0.1); // the drowned rock breaks nothing (only the first rock reaches there)
        assert_eq!(g.texels[0][..3], f.texels[0][..3]);
        // layer 1: the obstacles' own distance, the same here (no surveyed land)
        let r = |i: usize, j: usize| crate::bathymetry::half_to_f32(g.texels[(10 + j) * 20 + i][0]);
        assert_eq!(r(4, 4), 0.0);
        assert!((r(8, 4) - a(8, 4)).abs() < 1e-3);
    }

    #[test]
    fn no_obstacles_still_two_layers() {
        let h = crate::field::to_half_three;
        let f = ExtraField { origin: [0.0, 0.0], cell_m: 2.0, width: 4, height: 3, layers: 1, texels: vec![[h(5.0), h(1.0), h(0.0), h(7.0)]; 12] };
        let g = with_obstacles(&f, &[]);
        assert_eq!((g.height, g.layers), (6, 2));
        assert_eq!(g.texels[..12], f.texels[..]);
        for t in &g.texels[12..] {
            assert_eq!(crate::bathymetry::half_to_f32(t[0]), crate::bathymetry::LAND_DIST_CAP_M as f32);
            assert_eq!((t[1], t[2]), (0, 0));
        }
    }

    #[test]
    fn the_wake_trails_downstream_and_stays_clear_of_other_land() {
        let h = crate::field::to_half_three;
        let (w, rows, cell) = (200usize, 100usize, 2.0);
        // open water, and a seawall (surveyed land) across the flood's path 120 m downstream
        let mut tex = vec![[h(5.0), h(1.0), h(0.0), h(60.0)]; w * rows];
        let wall_x = 100.0 + 120.0;
        for j in 0..rows {
            for i in 0..w {
                let x = (i as f64 + 0.5) * cell;
                tex[j * w + i][3] = h((wall_x - x).clamp(0.0, 60.0));
            }
        }
        let f = ExtraField { origin: [0.0, 0.0], cell_m: cell, width: w, height: rows, layers: 1, texels: tex };
        let pile = crate::obstacles::Obstacle { x: 100.0, z: 100.0, radius: 4.0, height: 2.0 };
        let g = with_obstacles(&f, &[pile]);
        let at = |x: f64, z: f64, c: usize| crate::bathymetry::half_to_f32(g.texels[(rows + (z / cell) as usize) * w + (x / cell) as usize][c]);
        // flood (G) behind the pile along the flow, not ahead of it; the ebb (B) the other way
        let down = |s: f64| (100.0 + WAKE_FLOW[0] * s, 100.0 + WAKE_FLOW[1] * s);
        let (x, z) = down(30.0);
        assert!(at(x, z, 1) > 0.5, "{}", at(x, z, 1));
        assert!(at(x, z, 2) < 1e-3);
        let (x, z) = down(-30.0);
        assert!(at(x, z, 1) < 1e-3 && at(x, z, 2) > 0.5);
        // across the flow, past the trail's width: none
        assert!(at(100.0 + 30.0 * WAKE_FLOW[0] - 30.0 * WAKE_FLOW[1], 100.0 + 30.0 * WAKE_FLOW[1] + 30.0 * WAKE_FLOW[0], 1) < 1e-3);
        // at the seawall downstream it fades out within the pile's radius of the
        // wall (a texel 1 m off it keeps under a sixth of the trail 20 m before it)
        let (x0, z0) = down((wall_x - 20.0 - 100.0) / WAKE_FLOW[0]);
        let (x1, z1) = down((wall_x - 1.0 - 100.0) / WAKE_FLOW[0]);
        assert!(at(x0, z0, 1) > 0.25, "{}", at(x0, z0, 1));
        assert!(at(x1, z1, 1) < at(x0, z0, 1) / 6.0, "{} {}", at(x1, z1, 1), at(x0, z0, 1));
    }

    #[test]
    fn none_yet_and_the_frame_inverts_the_extent() {
        assert!(field().is_none());
        let f = ExtraField { origin: [-100.0, 50.0], cell_m: 4.0, width: 25, height: 10, layers: 1, texels: vec![[0; 4]; 250] };
        assert_eq!(f.frame(), [-100.0, 50.0, 0.01, 0.025]);
        assert_eq!(IDENTITY_FRAME, [0.0, 0.0, 1.0, 1.0]);
    }
}
