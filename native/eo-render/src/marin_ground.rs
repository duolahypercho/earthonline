//! Vista Point and the land behind the Marin abutment. Colours come from the
//! pinned NAIP survey (native/tools/measure-marin-landing.py). The detail
//! patches reuse the terrain's triangles and heights, including existing
//! relief, so the landing adds neither a second ground surface nor a route.
use super::{Detail, Lattice, PATCH_CELLS, Patch, SIDE, STEPS, cell_tri};
use crate::scene::Vertex;
use glam::Vec3;
use std::sync::OnceLock;

const FOOTPRINTS: &str = include_str!("../../assets/marin-image-footprints.json");
const HEADLANDS: &[u8] = include_bytes!("../../assets/marin-headlands.png");

/// Each half of the atlas has a different surveyed footprint in the same
/// world frame as the terrain, independent of vertex density and terrain LOD.
pub(crate) fn atlas_constants() -> String {
    let cfg: serde_json::Value = serde_json::from_str(FOOTPRINTS).expect("Marin image footprints");
    let width = cfg["width"].as_u64().expect("image width") as f64;
    let bands = cfg["bands"].as_array().expect("image bands");
    assert_eq!(bands.len(), 2);
    let scale = eo_data::groundscale::scale();
    let mut constants = String::new();
    for (name, spec) in ["WIDE", "FINE"].into_iter().zip(bands) {
        let field = |key: &str| spec[key].as_f64().expect("survey coordinate") * scale;
        let extent = width * field("metresPerTexel");
        constants += &format!(
            "const LM_MARIN_{name}: vec4<f32> = vec4<f32>({:.6}, {:.6}, {:.6}, {:.6});\n",
            field("west"), field("north"), extent, extent / 2.,
        );
    }
    constants
}

/// The street scan array already supplies sRGB trilinear anisotropic samples;
/// the terrain needs only one additional colour layer and no new bindings.
pub(crate) fn atlas_mips(res: u32) -> Option<&'static [Vec<u8>]> {
    static LEVELS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    if !eo_landmarks::golden_gate::surveyed_surfaces() {
        return None;
    }
    let levels = LEVELS.get_or_init(|| {
        let (w, h, rgba) = crate::props_lib::decode_png(HEADLANDS).expect("Marin reflectance atlas");
        let cfg: serde_json::Value = serde_json::from_str(FOOTPRINTS).expect("Marin image footprints");
        assert_eq!((w, h), (cfg["width"].as_u64().unwrap() as usize, w));
        crate::authored::mip_chain(&rgba, w, true)
    });
    let level = levels.iter().position(|bytes| bytes.len() == 4 * (res as usize).pow(2))?;
    Some(&levels[level..])
}

/// Calibrated aerial colour on both sides of the city / regional terrain seam.
pub(crate) struct Overview {
    pub origin: [f64; 2],
    pub extent: [f64; 2],
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub(crate) fn overview() -> Option<&'static Overview> {
    static IMAGE: OnceLock<Overview> = OnceLock::new();
    if !eo_landmarks::golden_gate::surveyed_surfaces() { return None; }
    Some(IMAGE.get_or_init(|| {
        let cfg: serde_json::Value = serde_json::from_str(include_str!("../../assets/marin-overview-source.json")).unwrap();
        let (width, height, rgba) = crate::props_lib::decode_png(include_bytes!("../../assets/marin-overview.png")).expect("Marin regional reflectance");
        let field = |key: &str| cfg[key].as_f64().expect("survey coordinate") * eo_data::groundscale::scale();
        let extent = [cfg["width"].as_u64().unwrap() as f64, cfg["height"].as_u64().unwrap() as f64]
            .map(|v| v * field("metresPerTexel"));
        Overview { origin: [field("west"), field("north")], extent, width, height, rgba }
    }))
}

impl Overview {
    fn sample(&self, x: f64, z: f64) -> Option<&[u8]> {
        let u = (x - self.origin[0]) / self.extent[0];
        let v = (z - self.origin[1]) / self.extent[1];
        if !(0. ..1.).contains(&u) || !(0. ..1.).contains(&v) { return None; }
        let idx = ((v * self.height as f64) as usize * self.width + (u * self.width as f64) as usize) * 4;
        Some(&self.rgba[idx..idx + 4])
    }
}

/// The landing's existing fine tiles. Draw colour and measured terrain relief
/// on the same footprint so their transition follows identical tile edges.
pub(super) fn landing_keys(l: &Lattice) -> (std::ops::Range<i64>, std::ops::Range<i64>) {
    let scale = eo_data::groundscale::scale();
    let span = PATCH_CELLS as f64 * l.cell;
    let tiles = |lo: f64, hi: f64, origin: f64| {
        ((lo * scale - origin) / span).floor() as i64..((hi * scale - origin) / span).ceil() as i64
    };
    (tiles(-5630., -5180., l.vx(0)), tiles(-6170., -5810., l.vz(0)))
}

pub(in crate::scene) struct Ground {
    origin: [f64; 2],
    step: f64,
    width: usize,
    height: usize,
    rgba: &'static [u8],
    overview: Option<&'static Overview>,
}

impl Ground {
    pub(in crate::scene) fn load() -> Self {
        let data = include_bytes!("../../assets/marin-landing.bin");
        let word = |i: usize| i32::from_le_bytes(data[i..i + 4].try_into().unwrap());
        assert_eq!(&data[..4], b"MRN1");
        let scale = eo_data::groundscale::scale();
        let ground = Self {
            origin: [word(4) as f64 * scale, word(8) as f64 * scale],
            step: word(12) as f64 * scale,
            width: word(16) as usize,
            height: word(20) as usize,
            rgba: &data[24..],
            overview: overview(),
        };
        assert_eq!(ground.rgba.len(), ground.width * ground.height * 4);
        ground
    }

    pub(in crate::scene) fn apply(&self, vertex: &mut Vertex) {
        let [x, y, z] = vertex.pos;
        if y > eo_data::SEA_LEVEL_Y as f32 {
            if let Some(rgba) = self.overview.and_then(|im| im.sample(x as f64, z as f64)) {
                Self::blend(vertex, rgba);
            }
        }
        let u = (x as f64 - self.origin[0]) / self.step;
        let v = (z as f64 - self.origin[1]) / self.step;
        if u < 0. || v < 0. || u >= (self.width - 1) as f64 || v >= (self.height - 1) as f64 {
            return;
        }
        let offset = 4 * (v as usize * self.width + u as usize);
        Self::blend(vertex, &self.rgba[offset..offset + 4]);
    }

    fn blend(vertex: &mut Vertex, rgba: &[u8]) {
        let base = vertex.color;
        // The local survey is already averaged over several aerial pixels.
        // The terrain interpolates these samples between vertices.
        let mut out = base & 0xff000000;
        for ch in 0..3 {
            let a = rgba[3] as u32;
            let val = (((base >> (8 * ch)) & 255) * (255 - a) + rgba[ch] as u32 * a + 127) / 255;
            out |= val << (8 * ch);
        }
        vertex.color = out;
        if rgba[3] > 127 {
            // The shader reads these as measured reflectance. A neutral grey
            // here can be paved road, rather than the terrain palette's rock.
            vertex.normal = (vertex.normal & 0x00ff_ffff) | (129 << 24);
            vertex.color |= 255 << 24;
        }
    }

    pub(in crate::scene) fn patches(
        &self,
        current: Option<Detail>,
        l: Lattice,
        height: &dyn Fn(i64, i64) -> f32,
        colour: &dyn Fn(f32, f32) -> u32,
    ) -> Detail {
        let mut detail = current.unwrap_or_else(|| Detail {
            patches: Vec::new(),
            keys: Default::default(),
            lattice: l,
            stats: Default::default(),
            index: Default::default(),
        });
        let (cols, rows) = landing_keys(&l);
        for pz in rows {
            for px in cols.clone() {
                if px < 0
                    || pz < 0
                    || (px + 1) * PATCH_CELLS >= l.width
                    || (pz + 1) * PATCH_CELLS >= l.height
                {
                    continue;
                }
                let key = (px, pz);
                if let Some(&index) = detail.index.get(&key) {
                    for vert in &mut detail.patches[index].verts {
                        self.apply(vert);
                    }
                    continue;
                }
                let mut verts = Vec::with_capacity(SIDE * SIDE);
                // Cache the coarse samples bordering this patch, including one
                // normal stencil, rather than querying the street carpet for each fine vertex.
                let dim = PATCH_CELLS as usize + 3;
                let heights: Vec<_> = (0..dim * dim)
                    .map(|k| {
                        height(
                            px * PATCH_CELLS + (k % dim) as i64 - 1,
                            pz * PATCH_CELLS + (k / dim) as i64 - 1,
                        )
                    })
                    .collect();
                let h = |x: usize, z: usize| heights[z * dim + x];
                let normal = |x, z| {
                    Vec3::new(
                        h(x - 1, z) - h(x + 1, z),
                        2. * l.cell as f32,
                        h(x, z - 1) - h(x, z + 1),
                    )
                    .normalize()
                };
                for j in 0..SIDE {
                    for i in 0..SIDE {
                        let (a, b) = (
                            (i / STEPS as usize).min(PATCH_CELLS as usize - 1) + 1,
                            (j / STEPS as usize).min(PATCH_CELLS as usize - 1) + 1,
                        );
                        let (u, v) = (
                            i as f32 / STEPS as f32 - (a - 1) as f32,
                            j as f32 / STEPS as f32 - (b - 1) as f32,
                        );
                        let y = cell_tri(h(a, b), h(a + 1, b), h(a, b + 1), h(a + 1, b + 1), u, v);
                        let x = (l.vx(px * PATCH_CELLS) + i as f64 * l.cell / STEPS as f64) as f32;
                        let z = (l.vz(pz * PATCH_CELLS) + j as f64 * l.cell / STEPS as f64) as f32;
                        let n = normal(a, b)
                            .lerp(normal(a + 1, b), u)
                            .lerp(normal(a, b + 1).lerp(normal(a + 1, b + 1), u), v)
                            .normalize();
                        let mut vertex = Vertex {
                            pos: [x, y, z],
                            normal: eo_assets::glb::pack_normal(n),
                            color: colour(x, z),
                        };
                        self.apply(&mut vertex);
                        verts.push(vertex);
                    }
                }
                let nv = verts.len();
                detail.keys.insert(key);
                detail.index.insert(key, detail.patches.len());
                detail.patches.push(Patch {
                    key,
                    verts,
                    relief: vec![0.; nv],
                    relieved: 0,
                    max_relief_m: 0.,
                    material: nv,
                });
                detail.stats.vertices += nv;
            }
        }
        detail.stats.patches = detail.patches.len();
        detail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_aerial_scales_register_to_the_pavement_and_mask_the_suspended_road() {
        let cfg: serde_json::Value = serde_json::from_str(FOOTPRINTS).unwrap();
        let (width, height, rgba) = crate::props_lib::decode_png(HEADLANDS).unwrap();
        let bands = cfg["bands"].as_array().unwrap();
        for (index, band) in bands.iter().enumerate() {
            let pixel = |x: f64, z: f64| {
                let step = band["metresPerTexel"].as_f64().unwrap();
                let col = ((x - band["west"].as_f64().unwrap()) / step).floor() as usize;
                let row = ((z - band["north"].as_f64().unwrap()) / step).floor() as usize;
                let start = 4 * ((row + index * height / bands.len()) * width + col);
                &rgba[start..start + 4]
            };
            // A surveyed carriageway vertex over open air, then the calibration
            // patch on US 101 on land. Linear reflectance 0.10 encodes to sRGB 89.
            assert_eq!(pixel(-5341.9, -5900.4)[3], 0);
            let road = pixel(-5570., -6129.);
            assert_eq!(road[3], 255);
            assert!(road[..3].iter().all(|&v| v.abs_diff(89) <= 6), "{road:?}");
        }
    }
}
