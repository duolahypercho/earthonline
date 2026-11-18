//! Street furniture drawn in code where the kind's GLB is not in the
//! checkout (`public/assets/street-props/*-v1.glb`: the generated models are
//! not shipped, so every such kind drew as a 0.3 m grey box at its height).
//! Each kind keeps the furniture walk's placement and the library's LOD
//! edges; only the mesh changes. `--no-furniture-models` draws the boxes, as
//! before.
//!
//! Sizes, in order of precedence:
//! - The street lights' shaft, base and arm: `streetlight-sf-residential-pole`
//!   of the standards table (SF Public Works Standard Plan 87,205: a round
//!   concrete shaft 8.2 in tapering to 5.0 in, 25 ft, on a 15 in base, with a
//!   6 ft Type K arm), read through eo-data. The column light's lantern
//!   centre is the furniture export's `lampHead.column.heightM` (where the
//!   lamp's light is placed), so the lit head and the drawn one agree.
//! - Sign plates on posts: their lower edge at least `sign-mounting-height-urban`
//!   (`min` for a primary sign) over the walk.
//! - Every other kind: its height in the furniture export's kind table
//!   (eo-props furniture-js `kinds[].height`) and its plan size from the
//!   street-prop manifest's recorded `dimensionsM`
//!   (public/assets/street-props/street-props-v1.provenance.json), so the
//!   kind fills the footprint the walk reserves for it.
//! - Part proportions inside those envelopes (collars, caps, slats, glazing)
//!   are INFERRED from common SF street hardware; colours are INFERRED
//!   paint tones unless noted.
//!
//! Model frame (as the library's other kinds): origin at the foot, +Y up,
//! -Z toward the carriageway (the lamps' `toward`: eo-props `lamp_light`).

use super::*;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(true);

/// The procedural kinds in place of the missing GLBs' boxes (`--no-furniture-models`: the boxes, as before).
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// The materials the procedural kinds draw with (base colour is the vertex colour).
pub(super) struct Mats {
    pub paint: u32,
    pub metal: u32,
    pub matte: u32,
    /// Float glass: a dielectric whose reflection is its Fresnel term (F0 4 %), nearly smooth.
    pub glass: u32,
    /// A luminaire's prismatic refractor or frosted globe: a rougher dielectric.
    pub lens: u32,
    /// Galvanised steel (a zinc coat, a dull metal).
    pub zinc: u32,
}

impl Mats {
    pub(super) fn new(materials: &mut Vec<PropMaterial>, paint: u32, metal: u32, matte: u32) -> Self {
        let white = [255u8; 3];
        let glass = push_mat(materials, white, 0.06, 0.0, 0, 0);
        let lens = push_mat(materials, white, 0.3, 0.0, 0, 0);
        let zinc = push_mat(materials, white, 0.5, 0.85, 0, 0);
        Self { paint, metal, matte, glass, lens, zinc }
    }
}

/// The sizes read from the standards table and the furniture export.
#[derive(Clone, Copy, Debug)]
pub struct Sizes {
    /// Standard Plan 87,205: shaft height, arm length, base height (m).
    pub shaft_m: f32,
    pub arm_m: f32,
    pub base_m: f32,
    /// The column light's lantern centre (`lampHead.column.heightM`).
    pub column_head_m: f32,
    /// The lowest edge of a primary sign over the walk.
    pub sign_bottom_m: f32,
}

/// 8.2 in and 5.0 in: the 87,205 shaft's diameters at its foot and head (the
/// standards entry's condition text; its metric value carries the heights only).
const SHAFT_FOOT_DIA_M: f32 = 8.2 * 0.0254;
const SHAFT_HEAD_DIA_M: f32 = 5.0 * 0.0254;

impl Sizes {
    /// From the repository's standards table and the furniture export.
    pub fn load(repo: &std::path::Path) -> anyhow::Result<Self> {
        let ca = eo_data::CaStandards::load(&eo_data::DataPaths { repo_root: repo.to_path_buf() })?;
        let pole = "streetlight-sf-residential-pole";
        let fj = eo_props::furniture::data::FurnitureJs::load()?;
        let column = fj.consts.lamp_head.get("column").map(|h| h.height_m).ok_or_else(|| anyhow::anyhow!("the furniture export has no lampHead.column"))?;
        Ok(Self {
            shaft_m: ca.metric_key(pole, "shaftHeight")? as f32,
            arm_m: ca.metric_key(pole, "armLength")? as f32,
            base_m: ca.metric_key(pole, "baseHeight")? as f32,
            column_head_m: column as f32,
            sign_bottom_m: ca.metric_key("sign-mounting-height-urban", "min")? as f32,
        })
    }
}

/// Build kind `name`'s three levels and register them as model `id`; false when this module has no such kind.
pub(super) fn build(out: &mut Out, name: &str, id: u32, lod: [f32; 4], cast: u32, m: &Mats, s: &Sizes) -> bool {
    let levels = match name {
        "bollard" => bollard(out, m),
        "hydrant" => hydrant(out, m),
        "parkingMeter" => parking_meter(out, m),
        "payStation" => pay_station(out, m),
        "signPole" => sign_pole(out, m, s),
        "wasteBin" => waste_bin(out, m),
        "bikeRack" => bike_rack(out, m),
        "newsBox" => news_box(out, m),
        "mailbox" => mailbox(out, m),
        "lampCobra" => cobra_light(out, m, s, false),
        "lampBanner" => cobra_light(out, m, s, true),
        "lampHistoric" => column_light(out, m, s),
        "bench" => bench(out, m),
        "transitShelter" => shelter(out, m),
        "busStopFlag" => stop_flag(out, m, s),
        _ => return false,
    };
    out.set(id, levels, lod, cast, 0.0);
    true
}

// ---------------------------------------------------------------- shapes

fn hex(h: &str) -> [u8; 3] {
    tone(h, 1.0)
}

/// A solid of revolution with its profile's own normals (`lathe`), closed at r = 0 ends.
fn rev(b: &mut Builder, m: u32, profile: &[[f32; 2]], sides: u32, col: [u8; 3], at: [f32; 3]) {
    lathe(b, m, profile, sides, col, at);
}

/// A shaft with `flutes` shallow grooves (`depth` of the radius), flat-shaded, open ends.
#[allow(clippy::too_many_arguments)]
fn fluted(b: &mut Builder, m: u32, at: [f32; 3], y0: f32, y1: f32, r0: f32, r1: f32, flutes: u32, depth: f32, col: [u8; 3]) {
    let n = flutes * 2;
    for i in 0..n {
        let (a0, a1) = (i as f32 / n as f32 * TAU, (i + 1) as f32 / n as f32 * TAU);
        let k = |j: u32| if j % 2 == 0 { 1.0 } else { 1.0 - depth };
        let p = |a: f32, r: f32, y: f32| [at[0] + a.cos() * r, at[1] + y, at[2] + a.sin() * r];
        let q = [p(a0, r0 * k(i), y0), p(a1, r0 * k(i + 1), y0), p(a1, r1 * k(i + 1), y1), p(a0, r1 * k(i), y1)];
        let e1 = glam::Vec3::from(q[1]) - glam::Vec3::from(q[0]);
        let e2 = glam::Vec3::from(q[3]) - glam::Vec3::from(q[0]);
        let mut nrm = e2.cross(e1).normalize_or_zero();
        let am = (a0 + a1) / 2.0;
        if nrm.dot(glam::Vec3::new(am.cos(), 0.0, am.sin())) < 0.0 {
            nrm = -nrm;
        }
        let base = b.v.len() as u32;
        for v in q {
            b.vert(v, nrm.to_array(), [0.0, 0.0], col, m);
        }
        b.tri(false, base, base + 2, base + 1);
        b.tri(false, base, base + 3, base + 2);
    }
}

/// An ellipsoid (radii `r`) about `c`, between `from` and `to` of its height (0 = bottom, 1 = top).
#[allow(clippy::too_many_arguments)]
fn ellipsoid(b: &mut Builder, m: u32, c: [f32; 3], r: [f32; 3], rings: u32, segs: u32, from: f32, to: f32, col: [u8; 3]) {
    let base = b.v.len() as u32;
    for k in 0..=rings {
        let t = from + (to - from) * k as f32 / rings as f32;
        let phi = PI * (1.0 - t); // 0 at the top
        for j in 0..=segs {
            let th = j as f32 / segs as f32 * TAU;
            let u = [phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin()];
            let p = [c[0] + u[0] * r[0], c[1] + u[1] * r[1], c[2] + u[2] * r[2]];
            let n = [u[0] / r[0], u[1] / r[1], u[2] / r[2]];
            b.vert(p, n, [0.0, 0.0], col, m);
        }
    }
    for k in 0..rings {
        for j in 0..segs {
            let a = base + k * (segs + 1) + j;
            let d = a + segs + 1;
            b.tri(false, a, a + 1, d);
            b.tri(false, a + 1, d + 1, d);
        }
    }
}

/// A round member through `pts` (a polyline), each leg its own tube.
fn bent(b: &mut Builder, m: u32, pts: &[[f32; 3]], r: f32, sides: u32, col: [u8; 3]) {
    for w in pts.windows(2) {
        tube(b, m, w[0], w[1], r, sides, col);
    }
}

// ---------------------------------------------------------------- the kinds

/// A cast bollard: kind height 0.92 m, 0.187 m across (the manifest's envelope):
/// a base flange, a plain body, a raised collar and a domed cap.
fn bollard(out: &mut Out, m: &Mats) -> [u32; 3] {
    let col = hex("#2c2f31");
    let prof: [[f32; 2]; 15] = [
        [0.0, 0.0],
        [0.094, 0.0],
        [0.094, 0.028],
        [0.086, 0.046],
        [0.083, 0.08],
        [0.08, 0.6],
        [0.0905, 0.615],
        [0.0905, 0.68],
        [0.08, 0.695],
        [0.079, 0.79],
        [0.086, 0.8],
        [0.08, 0.828],
        [0.064, 0.875],
        [0.036, 0.906],
        [0.0, 0.92],
    ];
    rev(&mut out.b, m.paint, &prof, 16, col, [0.0; 3]);
    // a retro-reflective band under the collar (INFERRED: the common white sheeting band).
    rev(&mut out.b, m.lens, &[[0.0812, 0.53], [0.0812, 0.58]], 16, hex("#c9cbc6"), [0.0; 3]);
    let n = out.finish();
    let mid: Vec<[f32; 2]> = prof.iter().copied().step_by(2).chain(std::iter::once([0.0, 0.92])).collect();
    rev(&mut out.b, m.paint, &mid, 8, col, [0.0; 3]);
    let md = out.finish();
    out.b.cylinder([0.0; 3], 0.0, 0.92, 0.085, 0.07, 6, col, m.paint, true);
    let f = out.finish();
    [n, md, f]
}

/// A post hydrant: kind height 0.78 m, the manifest's 0.50 x 0.44 m envelope.
/// White as the JS's load-time repaint (`whitenHydrantPaint`: 236, 235, 228).
fn hydrant(out: &mut Out, m: &Mats) -> [u32; 3] {
    let white = [236u8, 235, 228];
    let nut = tone("#d9d8d0", 0.9);
    let prof: [[f32; 2]; 15] = [
        [0.0, 0.0],
        [0.16, 0.0],
        [0.16, 0.035],
        [0.12, 0.055],
        [0.11, 0.09],
        [0.108, 0.5],
        [0.124, 0.515],
        [0.124, 0.555],
        [0.114, 0.575],
        [0.108, 0.62],
        [0.092, 0.675],
        [0.055, 0.715],
        [0.034, 0.725],
        [0.03, 0.765],
        [0.0, 0.78],
    ];
    let parts = |b: &mut Builder, sides: u32| {
        rev(b, m.paint, &prof, sides, white, [0.0; 3]);
        // two hose nozzles across, the pumper nozzle toward the road.
        for sx in [-1.0f32, 1.0] {
            cyl(b, m.paint, 0.042, 0.042, 0.1, sides.min(10), white, [sx * 0.16, 0.45, 0.0], Some('x'));
            cyl(b, m.paint, 0.055, 0.055, 0.03, sides.min(10), white, [sx * 0.225, 0.45, 0.0], Some('x'));
            bx(b, m.paint, 0.025, 0.04, 0.04, "#d9d8d0", sx * 0.25, 0.45, 0.0, 0.9);
        }
        cyl(b, m.paint, 0.065, 0.065, 0.1, sides.min(12), white, [0.0, 0.4, -0.15], Some('z'));
        cyl(b, m.paint, 0.078, 0.078, 0.03, sides.min(12), white, [0.0, 0.4, -0.212], Some('z'));
        b.cuboid([-0.022, 0.378, -0.245], [0.022, 0.422, -0.226], nut, m.paint);
    };
    parts(&mut out.b, 16);
    let n = out.finish();
    parts(&mut out.b, 8);
    let md = out.finish();
    out.b.cylinder([0.0; 3], 0.0, 0.72, 0.13, 0.08, 6, white, m.paint, true);
    let f = out.finish();
    [n, md, f]
}

/// A single-space meter: kind height 1.25 m, the manifest's 0.33 x 0.22 m envelope.
fn parking_meter(out: &mut Out, m: &Mats) -> [u32; 3] {
    let pole = hex(pal("meterPole"));
    let head = pal("meterBoxGrey");
    let parts = |b: &mut Builder, sides: u32, near: bool| {
        rev(b, m.metal, &[[0.0, 0.0], [0.07, 0.0], [0.07, 0.02], [0.036, 0.04], [0.034, 0.92], [0.05, 0.93], [0.0, 0.94]], sides, pole, [0.0; 3]);
        bx(b, m.paint, 0.22, 0.24, 0.16, head, 0.0, 1.06, 0.0, 1.0);
        // the dome over the head, its axis across the street.
        cyl(b, m.paint, 0.11, 0.11, 0.16, sides, tone(head, 1.0), [0.0, 1.18, 0.0], Some('z'));
        if near {
            for sz in [-1.0f32, 1.0] {
                bx(b, m.glass, 0.13, 0.08, 0.006, "#1a2328", 0.0, 1.12, sz * 0.083, 1.0);
                bx(b, m.paint, 0.08, 0.05, 0.006, "#5d6264", 0.0, 1.0, sz * 0.083, 1.0);
            }
            bx(b, m.glass, 0.16, 0.006, 0.12, "#1c2a3e", 0.0, 1.292, 0.0, 1.0);
        }
    };
    parts(&mut out.b, 12, true);
    let n = out.finish();
    parts(&mut out.b, 6, false);
    let md = out.finish();
    out.b.cylinder([0.0; 3], 0.0, 0.95, 0.035, 0.035, 4, pole, m.metal, false);
    bx(&mut out.b, m.paint, 0.22, 0.34, 0.16, head, 0.0, 1.1, 0.0, 1.0);
    let f = out.finish();
    [n, md, f]
}

/// A pay station: charcoal (the JS's `charcoalPayStationPaint`), the manifest's
/// 0.6 x 0.32 m plan at the JS's upright stretch (1.24 m x 1.15).
fn pay_station(out: &mut Out, m: &Mats) -> [u32; 3] {
    let h = 1.24 * PAY_STATION_STRETCH[1];
    let body = "#2f3336";
    let parts = |b: &mut Builder, near: bool| {
        bx(b, m.matte, 0.5, 0.05, 0.3, "#55585a", 0.0, 0.025, 0.0, 1.0);
        bx(b, m.paint, 0.44, h - 0.12, 0.26, body, 0.0, 0.05 + (h - 0.12) / 2.0, 0.0, 1.0);
        bx(b, m.paint, 0.5, 0.07, 0.32, body, 0.0, h - 0.035, 0.0, 0.85);
        if near {
            // screen, keypad, card reader and the blue pay-here label on the walk side (+Z).
            bx(b, m.glass, 0.2, 0.15, 0.008, "#15212a", 0.0, h - 0.3, 0.134, 1.0);
            bx(b, m.paint, 0.14, 0.1, 0.012, "#4b5053", 0.0, h - 0.5, 0.136, 1.0);
            bx(b, m.paint, 0.06, 0.08, 0.02, "#1d2022", 0.13, h - 0.47, 0.14, 1.0);
            bx(b, m.paint, 0.36, 0.08, 0.006, "#2a5a9e", 0.0, h - 0.13, 0.134, 1.0);
            bx(b, m.paint, 0.18, 0.05, 0.03, "#1d2022", 0.0, 0.42, 0.14, 1.0);
            // the solar panel on the hood.
            bx_rot(b, m.glass, 0.44, 0.012, 0.28, hex("#1b2738"), 0.0, -0.2, [0.0, h + 0.02, 0.0]);
        }
    };
    parts(&mut out.b, true);
    let n = out.finish();
    parts(&mut out.b, false);
    let md = out.finish();
    bx(&mut out.b, m.paint, 0.48, h, 0.3, body, 0.0, h / 2.0, 0.0, 1.0);
    let f = out.finish();
    [n, md, f]
}

/// The corner sign pole carrying the street-name blades: kind height 3.124 m;
/// a galvanised round post (2 3/8 in pipe, INFERRED), the blades crossed at its
/// head, each green with a white legend and border (`street-name-sign-letter-height`
/// condition: white legend on green), the lower blade's edge over `sign_bottom_m`.
fn sign_pole(out: &mut Out, m: &Mats, s: &Sizes) -> [u32; 3] {
    let top = 3.124f32;
    let zinc = hex("#9aa0a2");
    let (bw, bh) = (0.76f32, 0.2f32); // 30 in x 8 in blades (INFERRED for 6 in capitals)
    let lower = (top - 0.28).max(s.sign_bottom_m + bh / 2.0);
    let blade = |b: &mut Builder, y: f32, along_x: bool, near: bool| {
        let (w, d) = if along_x { (bw, 0.012) } else { (0.012, bw) };
        b.cuboid([-w / 2.0, y - bh / 2.0, -d / 2.0], [w / 2.0, y + bh / 2.0, d / 2.0], hex("#e9e8e1"), m.paint);
        let (gw, gd) = if along_x { (bw - 0.03, 0.016) } else { (0.016, bw - 0.03) };
        b.cuboid([-gw / 2.0, y - bh / 2.0 + 0.015, -gd / 2.0], [gw / 2.0, y + bh / 2.0 - 0.015, gd / 2.0], hex("#1f6a45"), m.paint);
        if near {
            // the legend: letter blocks of hashed widths (a name and a suffix), both faces.
            let mut x = -bw / 2.0 + 0.07;
            for (i, w) in [0.05f32, 0.04, 0.045, 0.04, 0.038, 0.05, 0.0, 0.04, 0.03].iter().enumerate() {
                if *w > 0.0 {
                    let hgt = if i == 0 || i == 7 { 0.105 } else { 0.08 };
                    let (lw, ld) = if along_x { (*w, 0.02) } else { (0.02, *w) };
                    let c = if along_x { [x + w / 2.0, 0.0] } else { [0.0, x + w / 2.0] };
                    b.cuboid([c[0] - lw / 2.0, y - 0.05, c[1] - ld / 2.0], [c[0] + lw / 2.0, y - 0.05 + hgt, c[1] + ld / 2.0], hex("#eeeee8"), m.paint);
                }
                x += w + 0.016;
            }
        }
    };
    let parts = |b: &mut Builder, sides: u32, near: bool| {
        rev(b, m.zinc, &[[0.0, 0.0], [0.05, 0.0], [0.05, 0.03], [0.0302, 0.05], [0.0302, top], [0.0, top + 0.02]], sides, zinc, [0.0; 3]);
        blade(b, lower, true, near);
        blade(b, lower + bh + 0.03, false, near);
        b.cuboid([-0.04, lower + bh / 2.0 - 0.01, -0.04], [0.04, lower + bh / 2.0 + 0.04, 0.04], hex("#7d8385"), m.zinc);
    };
    parts(&mut out.b, 10, true);
    let n = out.finish();
    parts(&mut out.b, 6, false);
    let md = out.finish();
    out.b.cylinder([0.0; 3], 0.0, top, 0.03, 0.03, 4, zinc, m.zinc, false);
    blade(&mut out.b, lower, true, false);
    let f = out.finish();
    [n, md, f]
}

/// A slatted steel litter receptacle: kind height 0.94 m, the manifest's 0.59 m plan.
/// Dark green (INFERRED: the city's receptacle paint).
fn waste_bin(out: &mut Out, m: &Mats) -> [u32; 3] {
    let green = "#2c3d32";
    let parts = |b: &mut Builder, slats: u32, sides: u32| {
        let c = hex(green);
        rev(b, m.paint, &[[0.0, 0.0], [0.28, 0.0], [0.28, 0.06], [0.262, 0.07], [0.0, 0.07]], sides, c, [0.0; 3]);
        // the liner behind the slats.
        b.cylinder([0.0; 3], 0.07, 0.78, 0.25, 0.25, sides, tone("#1a1d1b", 1.0), m.matte, false);
        for k in 0..slats {
            let a = k as f32 / slats as f32 * TAU;
            let (sa, ca) = a.sin_cos();
            let from = b.v.len();
            b.cuboid([-0.018, 0.07, -0.009], [0.018, 0.78, 0.009], c, m.paint);
            for v in &mut b.v[from..] {
                let [x, y, z] = v.pos;
                let z = z + 0.27;
                v.pos = [x * ca + z * sa, y, -x * sa + z * ca];
                let nn = [v.normal[0] as f32, v.normal[2] as f32];
                let rx = nn[0] * ca + nn[1] * sa;
                let rz = -nn[0] * sa + nn[1] * ca;
                v.normal = [rx.round() as i8, v.normal[1], rz.round() as i8, 0];
            }
        }
        for y in [0.2f32, 0.72] {
            rev(b, m.paint, &[[0.276, y - 0.02], [0.284, y], [0.276, y + 0.02]], sides, c, [0.0; 3]);
        }
        // the domed lid over its opening band.
        rev(b, m.paint, &[[0.0, 0.78], [0.29, 0.78], [0.29, 0.8], [0.0, 0.8]], sides, c, [0.0; 3]);
        rev(b, m.matte, &[[0.2, 0.8], [0.2, 0.85]], sides, tone("#0e100f", 1.0), [0.0; 3]);
        rev(b, m.paint, &[[0.0, 0.85], [0.295, 0.85], [0.29, 0.875], [0.24, 0.91], [0.14, 0.932], [0.0, 0.94]], sides, c, [0.0; 3]);
    };
    parts(&mut out.b, 24, 20);
    let n = out.finish();
    parts(&mut out.b, 12, 10);
    let md = out.finish();
    out.b.cylinder([0.0; 3], 0.0, 0.85, 0.28, 0.28, 8, hex(green), m.paint, false);
    rev(&mut out.b, m.paint, &[[0.0, 0.85], [0.295, 0.85], [0.0, 0.94]], 8, hex(green), [0.0; 3]);
    let f = out.finish();
    [n, md, f]
}

/// Two inverted-U hoops (the manifest's 1.9 x 0.74 m plan, 0.92 m tall), galvanised tube.
fn bike_rack(out: &mut Out, m: &Mats) -> [u32; 3] {
    let zinc = hex("#a3a8aa");
    let (h, half, r) = (0.92f32, 0.3f32, 0.024f32);
    let parts = |b: &mut Builder, sides: u32, segs: u32| {
        for x in [-0.475f32, 0.475] {
            // the arc from the -z leg over the top to the +z leg.
            let mut legs = vec![[x, 0.0, -half], [x, h - half, -half]];
            for i in 1..segs {
                let a = PI * i as f32 / segs as f32;
                legs.push([x, h - half + a.sin() * half, -half * a.cos()]);
            }
            legs.extend([[x, h - half, half], [x, 0.0, half]]);
            bent(b, m.zinc, &legs, r, sides, zinc);
            for z in [-half, half] {
                cyl(b, m.zinc, 0.06, 0.06, 0.01, sides.max(6), zinc, [x, 0.005, z], None);
            }
        }
    };
    parts(&mut out.b, 8, 10);
    let n = out.finish();
    parts(&mut out.b, 4, 4);
    let md = out.finish();
    parts(&mut out.b, 3, 2);
    let f = out.finish();
    [n, md, f]
}

/// A newspaper rack on its pedestal: kind height 1.18 m, the manifest's 0.58 x 0.44 m plan.
fn news_box(out: &mut Out, m: &Mats) -> [u32; 3] {
    let body = "#3a6b96";
    let parts = |b: &mut Builder, near: bool| {
        bx(b, m.metal, 0.46, 0.02, 0.38, "#3c4043", 0.0, 0.01, 0.0, 1.0);
        bx(b, m.metal, 0.07, 0.4, 0.07, "#3c4043", 0.0, 0.22, 0.0, 1.0);
        bx(b, m.paint, 0.54, 0.6, 0.42, body, 0.0, 0.72, 0.0, 1.0);
        // the sloped lid toward the walk (+Z).
        bx_rot(b, m.paint, 0.56, 0.05, 0.44, hex(body), 0.0, 0.18, [0.0, 1.07, 0.0]);
        if near {
            bx(b, m.glass, 0.42, 0.24, 0.008, "#202a30", 0.0, 0.85, 0.214, 1.0);
            bx(b, m.paint, 0.46, 0.04, 0.03, "#2d5679", 0.0, 0.71, 0.215, 1.0);
            bx(b, m.metal, 0.12, 0.09, 0.05, "#8d9294", 0.17, 1.02, 0.19, 1.0);
        }
    };
    parts(&mut out.b, true);
    let n = out.finish();
    parts(&mut out.b, false);
    let md = out.finish();
    bx(&mut out.b, m.paint, 0.54, 0.7, 0.42, body, 0.0, 0.77, 0.0, 1.0);
    bx(&mut out.b, m.metal, 0.07, 0.42, 0.07, "#3c4043", 0.0, 0.21, 0.0, 1.0);
    let f = out.finish();
    [n, md, f]
}

/// A collection box on its legs: kind height 1.2 m, the manifest's 0.56 x 0.48 m
/// plan, an arched top; blue (INFERRED: the postal service's box blue).
fn mailbox(out: &mut Out, m: &Mats) -> [u32; 3] {
    let blue = "#1f4c8a";
    let (w, d) = (0.5f32, 0.46f32);
    let parts = |b: &mut Builder, sides: u32, near: bool| {
        for (sx, sz) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            bx(b, m.paint, 0.04, 0.22, 0.04, blue, sx * (w / 2.0 - 0.03), 0.11, sz * (d / 2.0 - 0.03), 0.8);
        }
        bx(b, m.paint, w, 0.8, d, blue, 0.0, 0.62, 0.0, 1.0);
        // the arch: a half cylinder across the box's width, its axis through the depth.
        let from = b.v.len();
        cyl(b, m.paint, w / 2.0, w / 2.0, d, sides, hex(blue), [0.0, 0.0, 0.0], Some('z'));
        for v in &mut b.v[from..] {
            v.pos[1] = v.pos[1].max(0.0) * 0.5 + 1.02;
        }
        if near {
            // the pull-down chute and its handle on the walk side, the decal plate.
            bx(b, m.paint, 0.38, 0.13, 0.02, blue, 0.0, 0.93, d / 2.0 + 0.01, 0.82);
            bx(b, m.metal, 0.16, 0.02, 0.025, "#8f9395", 0.0, 0.88, d / 2.0 + 0.03, 1.0);
            bx(b, m.paint, 0.26, 0.18, 0.004, "#e8e6de", 0.0, 0.62, d / 2.0 + 0.002, 1.0);
            bx(b, m.paint, 0.2, 0.05, 0.005, "#b8312f", 0.0, 0.62, d / 2.0 + 0.004, 1.0);
        }
    };
    parts(&mut out.b, 16, true);
    let n = out.finish();
    parts(&mut out.b, 6, false);
    let md = out.finish();
    bx(&mut out.b, m.paint, w, 0.92, d, blue, 0.0, 0.68, 0.0, 1.0);
    let f = out.finish();
    [n, md, f]
}

/// The district street light: Standard Plan 87,205's round concrete shaft
/// (8.2 in to 5.0 in over its 25 ft, on its 15 in base) with the 6 ft Type K
/// arm rising toward the road (-Z) and a cobra-head luminaire on it (housing
/// 0.76 x 0.34 m, INFERRED from common cobra heads). `banner`: two banner
/// arms on the walk side of the shaft with their banners.
fn cobra_light(out: &mut Out, m: &Mats, s: &Sizes, banner: bool) -> [u32; 3] {
    let concrete = hex("#a6a39b");
    let arm = hex("#8c9194");
    let housing = hex("#9da2a4");
    let (r0, r1) = (SHAFT_FOOT_DIA_M / 2.0, SHAFT_HEAD_DIA_M / 2.0);
    let top = s.base_m + s.shaft_m;
    let r_at = |y: f32| r0 + (r1 - r0) * ((y - s.base_m) / s.shaft_m).clamp(0.0, 1.0);
    // the arm: off the shaft 0.45 m under its head, out its 6 ft, rising 0.35 m (INFERRED rise).
    let a0 = [0.0, top - 0.45, -r_at(top - 0.45)];
    let a1 = [0.0, top - 0.1, -(r_at(top - 0.45) + s.arm_m * 0.55)];
    let a2 = [0.0, top - 0.02, -(r_at(top - 0.45) + s.arm_m)];
    let head_c = [0.0, a2[1] - 0.06, a2[2] - 0.36];
    let parts = |b: &mut Builder, sides: u32, near: bool| {
        // the base: a chamfered collar to the base height.
        rev(b, m.matte, &[[0.0, 0.0], [0.17, 0.0], [0.17, s.base_m - 0.06], [r0 + 0.02, s.base_m], [r0, s.base_m + 0.01]], sides, tone("#a6a39b", 0.92), [0.0; 3]);
        b.cylinder([0.0; 3], s.base_m, top, r0, r1, sides, concrete, m.matte, true);
        // the arm's clamp, the arm, the luminaire.
        cyl(b, m.metal, r_at(top - 0.45) + 0.02, r_at(top - 0.45) + 0.02, 0.18, sides, arm, [0.0, top - 0.45, 0.0], None);
        bent(b, m.metal, &[a0, a1, a2], 0.032, sides.min(8), arm);
        ellipsoid(b, m.paint, head_c, [0.17, 0.1, 0.38], if near { 6 } else { 3 }, sides, 0.5, 1.0, housing);
        ellipsoid(b, m.lens, [head_c[0], head_c[1] + 0.005, head_c[2] - 0.02], [0.14, 0.07, 0.3], if near { 4 } else { 2 }, sides, 0.0, 0.5, hex("#e4e5df"));
        if banner {
            let pole_r = |y: f32| r_at(y) + 0.015;
            for y in [top - 3.1, top - 1.4] {
                cyl(b, m.metal, pole_r(y) + 0.01, pole_r(y) + 0.01, 0.1, sides, arm, [0.0, y, 0.0], None);
                tube(b, m.metal, [pole_r(y), y, 0.0], [0.72, y, 0.0], 0.016, 6, arm);
                if near {
                    cyl(b, m.metal, 0.024, 0.024, 0.03, 6, arm, [0.735, y, 0.0], Some('x'));
                }
            }
            // the banner: the overhead export's banner tones (`bannerPale`, `bannerInk`).
            let (yb, yt) = (top - 3.05, top - 1.45);
            bx(b, m.matte, 0.6, yt - yb, 0.008, "#a14a79", 0.42, (yb + yt) / 2.0, 0.0, 1.0);
            if near {
                bx(b, m.matte, 0.5, 0.16, 0.012, "#c8b4a1", 0.42, yt - 0.32, 0.0, 1.0);
                bx(b, m.matte, 0.36, 0.05, 0.012, "#c8b4a1", 0.42, yb + 0.3, 0.0, 1.0);
            }
        }
    };
    parts(&mut out.b, 14, true);
    let n = out.finish();
    parts(&mut out.b, 7, false);
    let md = out.finish();
    out.b.cylinder([0.0; 3], 0.0, top, r0 + 0.02, r1, 5, concrete, m.matte, false);
    tube(&mut out.b, m.metal, a0, a2, 0.035, 3, arm);
    bx(&mut out.b, m.paint, 0.3, 0.14, 0.7, "#9da2a4", head_c[0], head_c[1], head_c[2], 1.0);
    if banner {
        bx(&mut out.b, m.matte, 0.6, 1.6, 0.01, "#a14a79", 0.42, top - 2.25, 0.0, 1.0);
    }
    let f = out.finish();
    [n, md, f]
}

/// The column light: a fluted cast post on a moulded base with an acorn
/// globe whose centre is the light's own head height (`column_head_m`); dark
/// green-black paint (INFERRED).
fn column_light(out: &mut Out, m: &Mats, s: &Sizes) -> [u32; 3] {
    let iron = hex("#252f2a");
    let hc = s.column_head_m;
    let (g_r, g_h) = (0.2f32, 0.3f32);
    let shaft_top = hc - g_h - 0.2;
    let parts = |b: &mut Builder, sides: u32, near: bool| {
        rev(
            b,
            m.paint,
            &[[0.0, 0.0], [0.23, 0.0], [0.23, 0.07], [0.205, 0.1], [0.2, 0.16], [0.17, 0.22], [0.16, 0.55], [0.14, 0.86], [0.17, 0.9], [0.17, 0.96], [0.11, 1.02], [0.095, 1.08]],
            sides,
            iron,
            [0.0; 3],
        );
        if near {
            fluted(b, m.paint, [0.0; 3], 1.08, shaft_top, 0.085, 0.066, 8, 0.12, iron);
        } else {
            b.cylinder([0.0; 3], 1.08, shaft_top, 0.085, 0.066, sides, iron, m.paint, false);
        }
        // the capital, the globe holder, the globe, the cap and finial.
        rev(b, m.paint, &[[0.066, shaft_top], [0.1, shaft_top + 0.04], [0.1, shaft_top + 0.09], [0.13, shaft_top + 0.13], [0.13, shaft_top + 0.17], [0.08, shaft_top + 0.2], [0.0, shaft_top + 0.2]], sides, iron, [0.0; 3]);
        ellipsoid(b, m.lens, [0.0, hc, 0.0], [g_r, g_h, g_r], if near { 8 } else { 4 }, sides, 0.0, 1.0, hex("#ebe7da"));
        rev(b, m.paint, &[[0.0, hc + g_h - 0.06], [0.16, hc + g_h - 0.04], [0.12, hc + g_h + 0.04], [0.035, hc + g_h + 0.1], [0.035, hc + g_h + 0.22], [0.0, hc + g_h + 0.3]], sides, iron, [0.0; 3]);
    };
    parts(&mut out.b, 16, true);
    let n = out.finish();
    parts(&mut out.b, 7, false);
    let md = out.finish();
    out.b.cylinder([0.0; 3], 0.0, shaft_top + 0.2, 0.16, 0.07, 5, iron, m.paint, false);
    ellipsoid(&mut out.b, m.lens, [0.0, hc, 0.0], [g_r, g_h, g_r], 3, 5, 0.0, 1.0, hex("#ebe7da"));
    let f = out.finish();
    [n, md, f]
}

/// A bench: the manifest's 1.09 x 0.6 m plan and 0.86 m back; two cast end
/// frames, three seat slats at 0.45 m (INFERRED: the common 17-19 in seat) and
/// two back slats; the seat faces the road (-Z).
fn bench(out: &mut Out, m: &Mats) -> [u32; 3] {
    let frame = hex("#2a2d2b");
    let wood = "#8a6946";
    let parts = |b: &mut Builder, near: bool| {
        for x in [-0.47f32, 0.47] {
            // front leg, rear leg rising into the back support, the seat rail.
            bx_rot(b, m.paint, 0.05, 0.46, 0.05, frame, 0.0, 0.06, [x, 0.23, -0.2]);
            bx_rot(b, m.paint, 0.05, 0.88, 0.05, frame, 0.0, -0.2, [x, 0.43, 0.2]);
            bx_rot(b, m.paint, 0.05, 0.04, 0.5, frame, 0.0, 0.0, [x, 0.43, 0.0]);
            if near {
                bx_rot(b, m.paint, 0.07, 0.02, 0.1, frame, 0.0, 0.0, [x, 0.01, -0.21]);
                bx_rot(b, m.paint, 0.07, 0.02, 0.1, frame, 0.0, 0.0, [x, 0.01, 0.16]);
            }
        }
        let slats = if near { 3 } else { 1 };
        for k in 0..slats {
            let (z, d) = if near { (-0.17 + k as f32 * 0.15, 0.12) } else { (-0.02, 0.44) };
            bx(b, m.matte, 1.09, 0.04, d, wood, 0.0, 0.47, z, 1.0 - 0.04 * k as f32);
        }
        for (y, z) in if near { vec![(0.62f32, 0.255f32), (0.78, 0.29)] } else { vec![(0.7, 0.27)] } {
            bx_rot(b, m.matte, 1.09, if near { 0.11 } else { 0.28 }, 0.035, tone(wood, 0.96), 0.0, -0.2, [0.0, y, z]);
        }
    };
    parts(&mut out.b, true);
    let n = out.finish();
    parts(&mut out.b, false);
    let md = out.finish();
    bx(&mut out.b, m.matte, 1.09, 0.06, 0.44, wood, 0.0, 0.46, -0.02, 1.0);
    bx_rot(&mut out.b, m.matte, 1.09, 0.3, 0.04, tone(wood, 0.96), 0.0, -0.2, [0.0, 0.7, 0.27]);
    let f = out.finish();
    [n, md, f]
}

/// A transit shelter: the manifest's 3.6 x 1.29 m plan, 2.6 m tall; a painted
/// steel frame cantilevering the roof from the rear posts, a glazed back and
/// end screen, the lit advertising panel at the far end, a perch bench.
fn shelter(out: &mut Out, m: &Mats) -> [u32; 3] {
    let steel = "#5c6367";
    let (w, d, h) = (3.6f32, 1.29f32, 2.6f32);
    let zb = d / 2.0 - 0.08;
    let parts = |b: &mut Builder, near: bool| {
        for x in [-w / 2.0 + 0.08, w / 2.0 - 0.08] {
            bx(b, m.paint, 0.08, h - 0.12, 0.12, steel, x, (h - 0.12) / 2.0, zb, 1.0);
        }
        // the roof: a thin canopy over the whole plan, its fascia band.
        bx(b, m.paint, w, 0.06, d, steel, 0.0, h - 0.09, 0.0, 1.05);
        bx(b, m.paint, w, 0.1, 0.03, steel, 0.0, h - 0.07, -d / 2.0 + 0.015, 0.9);
        // the glazed back and the end screen (dark float glass over a frame).
        bx(b, m.glass, w - 0.3, 2.0, 0.012, "#3a4a52", 0.0, 1.22, zb, 1.0);
        bx(b, m.glass, 0.012, 2.0, d - 0.35, "#3a4a52", -w / 2.0 + 0.09, 1.22, 0.1, 1.0);
        // the advertising panel at the other end, lit white.
        bx(b, m.paint, 0.14, 1.85, 1.15, "#4e5559", w / 2.0 - 0.14, 1.2, 0.0, 1.0);
        bx(b, m.lens, 0.006, 1.7, 1.02, "#ece9df", w / 2.0 - 0.215, 1.2, 0.0, 1.0);
        if near {
            for x in [-1.1f32, 0.0, 1.1] {
                bx(b, m.paint, 0.04, 2.0, 0.04, steel, x, 1.22, zb - 0.03, 1.0);
            }
            bx(b, m.paint, w - 0.3, 0.05, 0.05, steel, 0.0, 0.24, zb - 0.03, 1.0);
            bx(b, m.paint, w - 0.3, 0.05, 0.05, steel, 0.0, 2.22, zb - 0.03, 1.0);
            bx(b, m.metal, 1.6, 0.04, 0.3, "#80878a", -0.6, 0.62, zb - 0.2, 1.0);
            for x in [-1.3f32, 0.1] {
                bx(b, m.paint, 0.04, 0.6, 0.04, steel, x, 0.3, zb - 0.06, 1.0);
            }
            // the route map case on the inside of the end screen.
            bx(b, m.paint, 0.02, 0.6, 0.45, "#e2dfd5", -w / 2.0 + 0.1, 1.45, 0.25, 1.0);
        }
    };
    parts(&mut out.b, true);
    let n = out.finish();
    parts(&mut out.b, false);
    let md = out.finish();
    bx(&mut out.b, m.paint, w, 0.08, d, steel, 0.0, h - 0.08, 0.0, 1.0);
    bx(&mut out.b, m.glass, w - 0.1, h - 0.4, 0.04, "#3a4a52", 0.0, (h - 0.4) / 2.0 + 0.2, zb, 1.0);
    bx(&mut out.b, m.paint, 0.14, 1.85, 1.15, "#4e5559", w / 2.0 - 0.14, 1.2, 0.0, 1.0);
    let f = out.finish();
    [n, md, f]
}

/// The bus stop flag: a galvanised post, 2.9 m (the manifest), the flag across
/// the kerb line facing along the street (both ways) with its lower edge over
/// `sign_bottom_m`, and a yellow band on the post (INFERRED: the transit
/// agency's stop marking).
fn stop_flag(out: &mut Out, m: &Mats, s: &Sizes) -> [u32; 3] {
    let top = 2.9f32;
    let zinc = hex("#9aa0a2");
    let (fw, fh) = (0.3f32, 0.45f32);
    let fy = (top - fh / 2.0 - 0.05).max(s.sign_bottom_m + fh / 2.0);
    let parts = |b: &mut Builder, sides: u32, near: bool| {
        rev(b, m.zinc, &[[0.0, 0.0], [0.05, 0.0], [0.05, 0.03], [0.0302, 0.05], [0.0302, top], [0.0, top + 0.02]], sides, zinc, [0.0; 3]);
        rev(b, m.paint, &[[0.0315, 1.5], [0.0315, 1.8]], sides, hex("#d9a631"), [0.0; 3]);
        // the flag on the road side of the post (-Z), its faces along the street (+-X).
        b.cuboid([-0.006, fy - fh / 2.0, -0.04 - fw], [0.006, fy + fh / 2.0, -0.04], hex("#ecebe4"), m.paint);
        b.cuboid([-0.008, fy + fh / 2.0 - 0.12, -0.04 - fw + 0.015], [0.008, fy + fh / 2.0 - 0.015, -0.055], hex("#b3282d"), m.paint);
        if near {
            b.cuboid([-0.008, fy - 0.08, -0.04 - fw + 0.05], [0.008, fy - 0.03, -0.09], hex("#2b2e31"), m.paint);
            b.cuboid([-0.008, fy - 0.17, -0.04 - fw + 0.08], [0.008, fy - 0.13, -0.12], hex("#2b2e31"), m.paint);
        }
        b.cuboid([-0.02, fy - fh / 2.0 + 0.02, -0.06], [0.02, fy - fh / 2.0 + 0.06, 0.0], hex("#7d8385"), m.zinc);
        b.cuboid([-0.02, fy + fh / 2.0 - 0.06, -0.06], [0.02, fy + fh / 2.0 - 0.02, 0.0], hex("#7d8385"), m.zinc);
    };
    parts(&mut out.b, 10, true);
    let n = out.finish();
    parts(&mut out.b, 6, false);
    let md = out.finish();
    out.b.cylinder([0.0; 3], 0.0, top, 0.03, 0.03, 4, zinc, m.zinc, false);
    out.b.cuboid([-0.006, fy - fh / 2.0, -0.04 - fw], [0.006, fy + fh / 2.0, -0.04], hex("#ecebe4"), m.paint);
    let f = out.finish();
    [n, md, f]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shaft_taper_is_the_standard_plans() {
        assert!((SHAFT_FOOT_DIA_M - 0.20828).abs() < 1e-5);
        assert!((SHAFT_HEAD_DIA_M - 0.127).abs() < 1e-5);
    }
}
