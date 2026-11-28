//! Citywide 2.5D Skyline Heightfield + Horizon Sky-Visibility / Bent-Normal /
//! Distant-Shadow Map (`native/cache/skyline-field/<key>.eosf`).
//!
//! Bakes a single `2048 x 2048` `Rgba16Float` citywide GPU texture (with 5 mip
//! levels, ~9 m/texel covering ~18.4 km across all of San Francisco) on the CPU
//! and caches it deterministically on disk with a trailing SHA-256 digest:
//!
//! - **R (`top_y`)**: Maximum world-Y surface elevation (terrain + all 170,000
//!   building footprints + LiDAR roof heights + optional landmark prisms),
//!   metres. Mip levels 1..4 are **max-pooled** so coarse distant ray-march steps
//!   (100–1600 m) never miss narrow skyline towers.
//! - **G (`sky_vis`)**: Precomputed horizon-based sky visibility (urban canyon
//!   AO, `0.0..1.0`) marched across 16 azimuths out to `HORIZON_REACH_M` (480 m)
//!   and dilated from open streets/courtyards into building footprints.
//! - **B, A (`bent_xz`)**: Precomputed 2D horizontal bent-normal / openness
//!   vector `(bx, bz)` in `[-1.0, +1.0]` pointing toward the most open sky/Bay
//!   horizon direction. Dotted with a facade normal `N.xz`, it crisply separates
//!   waterfront/exposed faces from occluded canyon faces even when backlit.
//!
//! Uniform contract (`Look.lx[15]` / `LX_SPARE`):
//!   `(origin_x, origin_z, inv_extent, max_y)` (`inv_extent == 0.0` when inert).
//! Binding contract (Group 1, `B_SKYLINE_FIELD = 24`):
//!   `texture_2d<f32>` sampled with the existing clamping linear sampler
//!   `lk_ao_samp` (zero new samplers).

use crate::ambient_bake::{f16_bits, f16_value};
use sha2::{Digest, Sha256};
use std::f64::consts::{FRAC_2_PI, FRAC_PI_2, TAU};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Default texture resolution (texels per side).
pub const DEFAULT_SIZE: usize = 2048;
/// Number of mip levels stored and uploaded (2048 down to 128).
pub const MIP_LEVELS: usize = 5;
/// Binary cache magic and version.
pub const CACHE_MAGIC: &[u8; 4] = b"EOSF";
pub const CACHE_VERSION: u32 = 1;
/// Embedded source text so any algorithm change keys into a new cache file.
pub const SOURCE: &str = include_str!("skyline_field.rs");

/// Horizon scan reach in world metres (covers full FiDi/SoMa tower canyons & hills).
pub const HORIZON_REACH_M: f64 = 480.0;
/// Azimuth directions marched per texel.
pub const HORIZON_AZIMUTHS: usize = 16;
/// Exponential march steps per azimuth.
pub const HORIZON_STEPS: usize = 18;
/// Dilation passes from open street/plaza texels inward into building footprints.
const DILATE_PASSES: usize = 6;
/// Margin added around building bounds before aligning to `ALIGN_M`.
const MARGIN_M: f64 = 384.0;
const ALIGN_M: f64 = 256.0;

/// WGSL sampling helpers for `hlod_draw.wgsl` and `look_light.wgsl`.
/// Expects `@group(1) @binding(24) var lx_skyline_field: texture_2d<f32>;`,
/// `lk_ao_samp`, and `look.lx[15]` = `(origin_x, origin_z, inv_extent, max_y)`.
pub const WGSL: &str = r#"
// Citywide 2.5D Skyline Field (skyline_field.rs):
// look.lx[15] = (origin_x, origin_z, inv_extent, max_y); inv_extent == 0 when off.
// Texel = (max_top_y, sky_vis, bent_x, bent_z).

fn sf_field_on() -> bool {
    return look.lx[15].z > 0.0;
}

fn sf_field_uv(xz: vec2<f32>) -> vec2<f32> {
    return (xz - look.lx[15].xy) * look.lx[15].z;
}

// Urban canyon sky-visibility & Bay/horizon openness at a world point.
// Returns vec2(sky_ao, openness):
//   sky_ao   in [0.16, 1.0]: directional horizon sky visibility lifted with height above neighbors
//   openness in [0.0,  1.0]: how strongly the facade faces open sky/Bay vs dense canyon
// Uses a single L2-friendly mip-1 tap (8 MB working set vs 32 MB mip-0) since horizon sky_vis
// and bent_xz are already 3x3-smoothed over 480 m reach.
fn sf_sky_ao(world: vec3<f32>, n: vec3<f32>, building_top_y: f32) -> vec2<f32> {
    if (!sf_field_on()) {
        return vec2<f32>(1.0, 0.5);
    }
    let inv_ext = look.lx[15].z;
    let cell_m = 1.0 / max(inv_ext * 2048.0, 1e-4);
    // Offset ~1.5 texels outward along wall normal into the adjacent street/air.
    let horiz = n.xz;
    let hlen = length(horiz);
    let hdir = select(vec2<f32>(0.0), horiz / max(hlen, 1e-4), hlen > 0.05);
    let uv = sf_field_uv(world.xz + hdir * (cell_m * 1.5));
    if (any(uv <= vec2<f32>(0.002)) || any(uv >= vec2<f32>(0.998))) {
        return vec2<f32>(1.0, 0.5);
    }
    // Single mip-1 sample (1024^2 = 8 MB, fits cleanly in GPU L2 cache; R is max-pooled neighbor height).
    let s0 = textureSampleLevel(lx_skyline_field, lk_ao_samp, uv, 1.0);
    // Directional horizon visibility: open-facing faces get boost, canyon-facing faces deepen.
    let dir_dot = dot(hdir, s0.ba);
    let dir_vis = clamp(s0.g + 0.38 * dir_dot, 0.14, 1.0);
    // Lift canyon AO smoothly as elevation rises past the opposing skyline / toward crown.
    let opp_h = s0.r;
    let clear_opp = smoothstep(-6.0, 26.0, world.y - opp_h);
    let has_top = building_top_y > world.y - 1.0 && building_top_y < world.y + 400.0;
    let crown_t = select(0.0, smoothstep(-28.0, -2.0, world.y - building_top_y), has_top);
    let lift = max(clear_opp * 0.78, crown_t * 0.45);
    let sky_ao = mix(dir_vis, 1.0, clamp(lift, 0.0, 1.0));
    // Roofs mostly see open sky unless tucked beneath an adjacent taller tower.
    let roof_ao = mix(1.0, clamp(0.45 + 0.55 * s0.g, 0.45, 1.0), smoothstep(2.0, 28.0, opp_h - world.y));
    let final_ao = mix(sky_ao, roof_ao, smoothstep(0.45, 0.85, n.y));
    let openness = clamp(0.5 + 0.5 * dir_dot + 0.25 * clear_opp, 0.0, 1.0);
    return vec2<f32>(clamp(final_ao, 0.16, 1.0), openness);
}

// Fast 4-step L2-resident exponential ray-marched distant soft sun shadow (0..10 km)
// using the max-pooled heightfield mip chain (mips 1..4 = 8 MB..128 KB) in channel R.
fn sf_sun_shadow(world: vec3<f32>, n: vec3<f32>, l: vec3<f32>) -> f32 {
    if (!sf_field_on() || l.y <= 0.025 || dot(n, l) <= 0.01) {
        return 1.0;
    }
    let max_y = look.lx[15].w;
    if (world.y >= max_y - 4.0) {
        return 1.0;
    }
    let lh = length(l.xz);
    if (lh < 1e-4) {
        return 1.0;
    }
    let dir = l.xz / lh;
    let slope = l.y / lh;
    // Start bias along normal + sun + up to avoid self-shadowing a building's own texel.
    let start_xz = world.xz + n.xz * 8.5 + dir * 5.0;
    let start_y = world.y + 3.5;
    var d = 26.0;
    var vis = 1.0;
    for (var i = 0; i < 4; i = i + 1) {
        let ray_y = start_y + d * slope;
        if (ray_y > max_y) {
            break;
        }
        let uv = sf_field_uv(start_xz + dir * d);
        if (any(uv <= vec2<f32>(0.001)) || any(uv >= vec2<f32>(0.999))) {
            break;
        }
        let h = textureSampleLevel(lx_skyline_field, lk_ao_samp, uv, 1.0 + f32(i)).r;
        // Soft penumbra cone (~1.8 deg effective softness to absorb coarse mip discretization).
        let pen = max(d * 0.032, 4.5);
        let s = clamp((ray_y - h) / pen + 0.5, 0.0, 1.0);
        vis = min(vis, s);
        if (vis <= 0.02) {
            return 0.0;
        }
        d = d * 2.75;
    }
    return vis;
}

// Fast 4-step L2-resident heightfield ray-march along a reflection ray `r` (unit vector)
// so downtown glass curtain walls reflect actual neighboring skyscraper silhouettes (<0.08 ms).
// Returns vec2(hit, sunlit_hint):
//   hit         in [0.0, 1.0]: 0 = unobstructed sky/Bay, 1 = reflected neighboring tower/hill
//   sunlit_hint in [0.0, 1.0]: vertical crown & sky-exposure hint on the reflected building
//                              (0 = deep shadowed canyon base, 1 = sunlit upper tower/crown)
fn sf_reflect_occlusion(world: vec3<f32>, n: vec3<f32>, r: vec3<f32>) -> vec2<f32> {
    if (!sf_field_on()) {
        return vec2<f32>(0.0, 0.0);
    }
    let max_y = look.lx[15].w;
    if (r.y > 0.82 || (world.y >= max_y - 2.0 && r.y >= 0.0)) {
        return vec2<f32>(0.0, 0.0);
    }
    let rh = length(r.xz);
    if (rh < 0.05) {
        return vec2<f32>(0.0, 0.0);
    }
    let inv_ext = look.lx[15].z;
    let cell_m = 1.0 / max(inv_ext * 2048.0, 1e-4);
    let dir = r.xz / rh;
    let slope = r.y / rh;
    // Push origin cleanly past our own building's mip-1 texel (extra margin at grazing angles)
    // to prevent self-reflection blackout on wide glass facades.
    let nh = n.xz;
    let nh_len = length(nh);
    let n_dir = select(vec2<f32>(0.0), nh / max(nh_len, 1e-4), nh_len > 0.05);
    let grazing = 1.0 - max(dot(dir, n_dir), 0.0);
    let start_xz = world.xz + n_dir * (cell_m * (1.75 + 0.85 * grazing)) + dir * (cell_m * 0.55);

    var d = 26.0;
    var mip = 1.0;
    var hit = 0.0;
    var hint = 0.0;
    for (var i = 0; i < 4; i = i + 1) {
        let ray_y = world.y + d * slope;
        if (ray_y > max_y + 4.0) {
            break;
        }
        let uv = sf_field_uv(start_xz + dir * d);
        if (any(uv <= vec2<f32>(0.001)) || any(uv >= vec2<f32>(0.999))) {
            break;
        }
        let s = textureSampleLevel(lx_skyline_field, lk_ao_samp, uv, mip);
        let h = s.r;
        // Skip open Bay/water surface (near sea level with wide-open horizon) so downward
        // reflections on waterfront towers keep bright Bay shimmer instead of building silhouettes.
        let is_water = h <= 4.0 && s.g > 0.85;
        if (!is_water) {
            let pen = max(d * 0.055, 5.5);
            let step_hit = clamp((h - ray_y) / pen + 0.35, 0.0, 1.0);
            if (step_hit > 0.01) {
                let dist_w = 1.0 - 0.32 * smoothstep(260.0, 720.0, d);
                let eff = step_hit * dist_w;
                if (eff > hit) {
                    hit = eff;
                    let rel_h = clamp(1.0 - max(h - ray_y, 0.0) / max(h, 28.0), 0.0, 1.0);
                    hint = clamp(0.55 * rel_h + 0.45 * s.g, 0.0, 1.0);
                }
            }
            if (hit >= 0.96) {
                break;
            }
        }
        d = d * 2.60;
        mip = min(mip + 0.80, 3.4);
    }
    return vec2<f32>(clamp(hit, 0.0, 1.0), hint);
}
"#;

/// Map bounds & grid resolution of a baked skyline field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldBounds {
    pub origin_x: f64,
    pub origin_z: f64,
    pub cell_m: f64,
    pub size: usize,
    pub max_y: f32,
}

impl FieldBounds {
    pub fn around(min_x: f64, min_z: f64, max_x: f64, max_z: f64, size: usize) -> Self {
        let x0 = ((min_x - MARGIN_M) / ALIGN_M).floor() * ALIGN_M;
        let z0 = ((min_z - MARGIN_M) / ALIGN_M).floor() * ALIGN_M;
        let x1 = ((max_x + MARGIN_M) / ALIGN_M).ceil() * ALIGN_M;
        let z1 = ((max_z + MARGIN_M) / ALIGN_M).ceil() * ALIGN_M;
        let span = (x1 - x0).max(z1 - z0).max(1024.0);
        let cell_m = ((span / size as f64) * 2.0).ceil() * 0.5;
        let extent = cell_m * size as f64;
        let cx = 0.5 * (x0 + x1);
        let cz = 0.5 * (z0 + z1);
        let origin_x = ((cx - 0.5 * extent) / ALIGN_M).round() * ALIGN_M;
        let origin_z = ((cz - 0.5 * extent) / ALIGN_M).round() * ALIGN_M;
        Self { origin_x, origin_z, cell_m, size, max_y: 0.0 }
    }

    pub fn extent_m(&self) -> f64 {
        self.cell_m * self.size as f64
    }

    /// Packed uniform row for `Look.lx[15]` (`LX_SPARE`).
    pub fn params_vec4(&self) -> [f32; 4] {
        [self.origin_x as f32, self.origin_z as f32, (1.0 / self.extent_m()) as f32, self.max_y]
    }

    /// World `(x, z)` to continuous texel coordinates.
    pub fn world_to_texel(&self, x: f64, z: f64) -> (f64, f64) {
        ((x - self.origin_x) / self.cell_m - 0.5, (z - self.origin_z) / self.cell_m - 0.5)
    }

    /// Texel centre `(ix, iz)` to world `(x, z)`.
    pub fn texel_to_world(&self, ix: usize, iz: usize) -> (f64, f64) {
        (self.origin_x + (ix as f64 + 0.5) * self.cell_m, self.origin_z + (iz as f64 + 0.5) * self.cell_m)
    }
}

/// CPU-side baked skyline field: `MIP_LEVELS` levels of `Rgba16Float` pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedSkylineField {
    pub bounds: FieldBounds,
    /// Half-float (`[u16; 4]` packed as little-endian bytes) per mip level 0..MIP_LEVELS.
    pub mips: Vec<Vec<u8>>,
    pub key: String,
    pub bake_ms: f32,
}

impl BakedSkylineField {
    /// Sample mip 0 at texel `(ix, iz)` decoded back to `[top_y, sky_vis, bent_x, bent_z]`.
    pub fn texel(&self, ix: usize, iz: usize) -> [f32; 4] {
        self.mip_texel(0, ix, iz)
    }

    /// Sample mip `level` at texel `(ix, iz)` decoded back to `[f32; 4]`.
    pub fn mip_texel(&self, level: usize, ix: usize, iz: usize) -> [f32; 4] {
        let s = (self.bounds.size >> level).max(1);
        let o = (iz.min(s - 1) * s + ix.min(s - 1)) * 8;
        let b = &self.mips[level];
        [0, 1, 2, 3].map(|c| f16_value(u16::from_le_bytes([b[o + 2 * c], b[o + 2 * c + 1]])))
    }

    pub fn encode(&self) -> Vec<u8> {
        let payload_bytes: usize = self.mips.iter().map(|m| m.len()).sum();
        let mut out = Vec::with_capacity(64 + payload_bytes + 32);
        out.extend_from_slice(CACHE_MAGIC);
        out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.bounds.size as u32).to_le_bytes());
        out.extend_from_slice(&(self.mips.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.bounds.origin_x.to_le_bytes());
        out.extend_from_slice(&self.bounds.origin_z.to_le_bytes());
        out.extend_from_slice(&self.bounds.cell_m.to_le_bytes());
        out.extend_from_slice(&self.bounds.max_y.to_le_bytes());
        let kb = self.key.as_bytes();
        out.extend_from_slice(&(kb.len() as u16).to_le_bytes());
        out.extend_from_slice(kb);
        for m in &self.mips {
            out.extend_from_slice(&(m.len() as u32).to_le_bytes());
            out.extend_from_slice(m);
        }
        let digest = Sha256::digest(&out);
        out.extend_from_slice(&digest);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 48 + 32 || &bytes[0..4] != CACHE_MAGIC {
            return Err("not an EOSF skyline field".into());
        }
        let (body, digest) = bytes.split_at(bytes.len() - 32);
        if Sha256::digest(body).as_slice() != digest {
            return Err("skyline field SHA-256 mismatch".into());
        }
        let mut at = 4usize;
        let take = |at: &mut usize, n: usize| -> Result<&[u8], String> {
            let s = body.get(*at..*at + n).ok_or("truncated")?;
            *at += n;
            Ok(s)
        };
        let u32_r = |at: &mut usize| -> Result<u32, String> { Ok(u32::from_le_bytes(take(at, 4)?.try_into().unwrap())) };
        let f32_r = |at: &mut usize| -> Result<f32, String> { Ok(f32::from_le_bytes(take(at, 4)?.try_into().unwrap())) };
        let f64_r = |at: &mut usize| -> Result<f64, String> { Ok(f64::from_le_bytes(take(at, 8)?.try_into().unwrap())) };

        if u32_r(&mut at)? != CACHE_VERSION {
            return Err("skyline field version mismatch".into());
        }
        let size = u32_r(&mut at)? as usize;
        let nmips = u32_r(&mut at)? as usize;
        if size == 0 || !size.is_power_of_two() || nmips == 0 || nmips > 12 {
            return Err("invalid skyline field dimensions".into());
        }
        let origin_x = f64_r(&mut at)?;
        let origin_z = f64_r(&mut at)?;
        let cell_m = f64_r(&mut at)?;
        let max_y = f32_r(&mut at)?;
        let klen = u16::from_le_bytes(take(&mut at, 2)?.try_into().unwrap()) as usize;
        let key = std::str::from_utf8(take(&mut at, klen)?).map_err(|_| "bad key utf8")?.to_string();
        let mut mips = Vec::with_capacity(nmips);
        for m in 0..nmips {
            let mlen = u32_r(&mut at)? as usize;
            let side = (size >> m).max(1);
            if mlen != side * side * 8 {
                return Err("mip byte length mismatch".into());
            }
            mips.push(take(&mut at, mlen)?.to_vec());
        }
        if at != body.len() {
            return Err("trailing bytes".into());
        }
        Ok(Self {
            bounds: FieldBounds { origin_x, origin_z, cell_m, size, max_y },
            mips,
            key,
            bake_ms: 0.0,
        })
    }
}

/// GPU resources for the citywide 2.5D skyline field.
pub struct SkylineFieldGpu {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub bounds: FieldBounds,
    /// Packed uniform row for `Look.lx[15]` (`LX_SPARE`).
    pub params: [f32; 4],
    pub bytes: u64,
}

impl SkylineFieldGpu {
    /// Create the `Rgba16Float` 2D texture and upload all mip levels.
    pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, baked: &BakedSkylineField) -> Self {
        let size = baked.bounds.size as u32;
        let mip_count = baked.mips.len() as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("skyline-field"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut bytes = 0u64;
        for (m, data) in baked.mips.iter().enumerate() {
            let side = (size >> m).max(1);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: m as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * 8), rows_per_image: Some(side) },
                wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            );
            bytes += data.len() as u64;
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view, bounds: baked.bounds, params: baked.bounds.params_vec4(), bytes }
    }
}

/// A single building or landmark prism fed into the heightfield rasterizer.
#[derive(Clone, Debug)]
pub struct HeightPrism {
    pub ring: Vec<[f64; 2]>,
    pub top_y: f32,
}

pub fn cache_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join("skyline-field").join(format!("{key}.eosf"))
}

/// Compute a deterministic 16-hex-char cache key from the source, city buildings,
/// elevation grid, optional extra prisms, LiDAR massing tag, and grid resolution.
pub fn inputs_key(city: &eo_data::City, elevation: &eo_data::ElevationGrid, extra: &[HeightPrism], size: usize) -> String {
    let mut h = Sha256::new();
    h.update(b"eo-skyline-field");
    h.update(CACHE_VERSION.to_le_bytes());
    h.update((size as u32).to_le_bytes());
    h.update(Sha256::digest(SOURCE.as_bytes()));
    h.update(eo_data::groundscale::scale().to_le_bytes());
    h.update(eo_data::groundscale::height_scale().to_le_bytes());
    h.update(eo_data::lidar_buildings::cache_tag().as_bytes());
    h.update((elevation.meta.width as u32).to_le_bytes());
    h.update((elevation.meta.height as u32).to_le_bytes());
    h.update(elevation.meta.origin_x.to_le_bytes());
    h.update(elevation.meta.origin_z.to_le_bytes());
    h.update(elevation.meta.cell_size.to_le_bytes());
    // Sample elevation stride deterministically (fast ~1 ms hash over 5M grid).
    for &v in elevation.lidar.iter().step_by(31) {
        h.update(v.to_bits().to_le_bytes());
    }
    h.update((city.buildings.len() as u32).to_le_bytes());
    for b in &city.buildings {
        h.update(b.height.to_bits().to_le_bytes());
        h.update((b.ring.len() as u32).to_le_bytes());
        if let Some(p) = b.ring.first() {
            h.update(p[0].to_bits().to_le_bytes());
            h.update(p[1].to_bits().to_le_bytes());
        }
    }
    h.update((extra.len() as u32).to_le_bytes());
    for e in extra {
        h.update(e.top_y.to_bits().to_le_bytes());
        h.update((e.ring.len() as u32).to_le_bytes());
    }
    let d = h.finalize();
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Load from `<cache_dir>/skyline-field/<key>.eosf` or bake and cache atomically.
pub fn load_or_bake(
    city: &eo_data::City,
    elevation: &eo_data::ElevationGrid,
    extra_prisms: &[HeightPrism],
    cache_dir: Option<&Path>,
    threads: usize,
) -> BakedSkylineField {
    let key = inputs_key(city, elevation, extra_prisms, DEFAULT_SIZE);
    if let Some(dir) = cache_dir {
        let p = cache_path(dir, &key);
        if let Ok(bytes) = std::fs::read(&p) {
            if let Ok(decoded) = BakedSkylineField::decode(&bytes) {
                if decoded.key == key {
                    return decoded;
                }
            }
        }
    }
    let baked = bake_city(city, elevation, extra_prisms, DEFAULT_SIZE, key.clone(), threads);
    if let Some(dir) = cache_dir {
        let p = cache_path(dir, &key);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = p.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, baked.encode()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
    baked
}

/// Bake the citywide skyline heightfield + horizon sky visibility + bent-normal map.
pub fn bake_city(
    city: &eo_data::City,
    elevation: &eo_data::ElevationGrid,
    extra_prisms: &[HeightPrism],
    size: usize,
    key: String,
    threads: usize,
) -> BakedSkylineField {
    let mut min_x = f64::MAX;
    let mut min_z = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_z = f64::MIN;
    let mut prisms = Vec::with_capacity(city.buildings.len() + extra_prisms.len());
    let lidar_table = eo_data::lidar_buildings::table();

    for b in &city.buildings {
        if b.ring.len() < 3 {
            continue;
        }
        let mut ring = Vec::with_capacity(b.ring.len());
        let mut hi_ground = f64::MIN;
        for p in &b.ring {
            let (x, z) = (p[0] as f64, p[1] as f64);
            min_x = min_x.min(x);
            min_z = min_z.min(z);
            max_x = max_x.max(x);
            max_z = max_z.max(z);
            hi_ground = hi_ground.max(elevation.ground(x, z).max(eo_data::SEA_LEVEL_Y));
            ring.push([x, z]);
        }
        let mut h = b.height as f64;
        if let Some(lm) = lidar_table.and_then(|t| t.get(&b.id)) {
            h = h.max(lm.max_h() as f64);
        }
        prisms.push(HeightPrism { ring, top_y: (hi_ground + h) as f32 });
    }
    for ep in extra_prisms {
        if ep.ring.len() < 3 {
            continue;
        }
        for p in &ep.ring {
            min_x = min_x.min(p[0]);
            min_z = min_z.min(p[1]);
            max_x = max_x.max(p[0]);
            max_z = max_z.max(p[1]);
        }
        prisms.push(ep.clone());
    }
    if !(min_x < max_x && min_z < max_z) {
        min_x = -512.0;
        min_z = -512.0;
        max_x = 512.0;
        max_z = 512.0;
    }
    let bounds = FieldBounds::around(min_x, min_z, max_x, max_z, size);
    bake_from_prisms(bounds, |x, z| elevation.ground(x, z).max(eo_data::SEA_LEVEL_Y) as f32, &prisms, key, threads)
}

/// Core pure-CPU baker (also used directly by synthetic unit tests).
pub fn bake_from_prisms<F>(mut bounds: FieldBounds, ground_fn: F, prisms: &[HeightPrism], key: String, threads: usize) -> BakedSkylineField
where
    F: Fn(f64, f64) -> f32 + Sync,
{
    let t0 = Instant::now();
    let size = bounds.size;
    let total = size * size;
    let nthreads = threads.clamp(1, 16);

    // 1. Sample bare-earth ground elevation across the grid in parallel.
    let mut ground_y = vec![0.0f32; total];
    let rows_per = size.div_ceil(nthreads);
    std::thread::scope(|s| {
        for (tid, chunk) in ground_y.chunks_mut(rows_per * size).enumerate() {
            let z_start = tid * rows_per;
            let gfn = &ground_fn;
            s.spawn(move || {
                let rows = chunk.len() / size;
                for rz in 0..rows {
                    let iz = z_start + rz;
                    let wz = bounds.origin_z + (iz as f64 + 0.5) * bounds.cell_m;
                    let row = &mut chunk[rz * size..(rz + 1) * size];
                    for (ix, slot) in row.iter_mut().enumerate() {
                        let wx = bounds.origin_x + (ix as f64 + 0.5) * bounds.cell_m;
                        *slot = gfn(wx, wz);
                    }
                }
            });
        }
    });

    // 2. Rasterize all buildings & landmarks into `top_y` (max surface height).
    let mut top_y = ground_y.clone();
    let mut xs_buf: Vec<f64> = Vec::with_capacity(32);
    for p in prisms {
        rasterize_prism(&bounds, &p.ring, p.top_y, &mut top_y, &mut xs_buf);
    }

    let mut max_y = f32::NEG_INFINITY;
    let mut is_building = vec![0u8; total];
    for i in 0..total {
        if top_y[i] > max_y {
            max_y = top_y[i];
        }
        if top_y[i] > ground_y[i] + 1.5 {
            is_building[i] = 1;
        }
    }
    bounds.max_y = if max_y.is_finite() { max_y } else { 0.0 };

    // Build coarse 32x32 block max-height mask so pure open ocean/bay texels skip marching.
    let block = 32usize;
    let bside = size.div_ceil(block);
    let mut block_max = vec![f32::NEG_INFINITY; bside * bside];
    for iz in 0..size {
        let bz = iz / block;
        for ix in 0..size {
            let bx = ix / block;
            let v = top_y[iz * size + ix];
            let slot = &mut block_max[bz * bside + bx];
            if v > *slot {
                *slot = v;
            }
        }
    }
    let reach_blocks = ((HORIZON_REACH_M / (bounds.cell_m * block as f64)).ceil() as isize).max(1);
    let sea_thresh = eo_data::SEA_LEVEL_Y as f32 + 0.75;

    // Precompute azimuth directions and exponential step distances (in texels).
    let mut dirs = [[0.0f64; 2]; HORIZON_AZIMUTHS * 4];
    for k in 0..dirs.len() {
        let a = k as f64 * TAU / dirs.len() as f64;
        dirs[k] = [a.cos(), a.sin()];
    }
    let mut step_texels = [0.0f64; HORIZON_STEPS];
    let max_texels = (HORIZON_REACH_M / bounds.cell_m).max(4.0);
    for s in 0..HORIZON_STEPS {
        let u = (s + 1) as f64 / HORIZON_STEPS as f64;
        // Quadratic spacing: dense near (1 texel), stretching smoothly out to HORIZON_REACH_M.
        step_texels[s] = (1.0 + (max_texels - 1.0) * u * u).max(1.0);
    }

    // 3. Multi-threaded horizon scan on open (non-building) texels.
    let mut ao_raw = vec![[1.0f32, 0.0, 0.0]; total];
    let mut valid = vec![0u8; total];
    let top_ref = &top_y;
    let ground_ref = &ground_y;
    let bld_ref = &is_building;
    let bmax_ref = &block_max;

    std::thread::scope(|s| {
        let ao_chunks = ao_raw.chunks_mut(rows_per * size);
        let val_chunks = valid.chunks_mut(rows_per * size);
        for (tid, (ao_ch, val_ch)) in ao_chunks.zip(val_chunks).enumerate() {
            let z_start = tid * rows_per;
            s.spawn(move || {
                let rows = ao_ch.len() / size;
                let cell_m = bounds.cell_m;
                let size_i = size as isize;
                for rz in 0..rows {
                    let iz = z_start + rz;
                    let bz = (iz / block) as isize;
                    let row_o = rz * size;
                    for ix in 0..size {
                        let idx = iz * size + ix;
                        if bld_ref[idx] != 0 {
                            continue;
                        }
                        val_ch[row_o + ix] = 1;
                        let g = ground_ref[idx];
                        // Fast skip for open bay/ocean far from any terrain or buildings.
                        if g <= sea_thresh {
                            let bx = (ix / block) as isize;
                            let mut any_high = false;
                            'blk: for dz in -reach_blocks..=reach_blocks {
                                let nz = bz + dz;
                                if nz < 0 || nz >= bside as isize {
                                    continue;
                                }
                                for dx in -reach_blocks..=reach_blocks {
                                    let nx = bx + dx;
                                    if nx < 0 || nx >= bside as isize {
                                        continue;
                                    }
                                    if bmax_ref[nz as usize * bside + nx as usize] > sea_thresh {
                                        any_high = true;
                                        break 'blk;
                                    }
                                }
                            }
                            if !any_high {
                                ao_ch[row_o + ix] = [1.0, 0.0, 0.0];
                                continue;
                            }
                        }
                        let y0 = g as f64 + 2.0;
                        let sub = ((ix.wrapping_mul(7) ^ iz.wrapping_mul(13)) & 3) as usize;
                        let mut sky_sum = 0.0f64;
                        let mut bx_sum = 0.0f64;
                        let mut bz_sum = 0.0f64;
                        for a in 0..HORIZON_AZIMUTHS {
                            let d = dirs[a * 4 + sub];
                            let mut max_tan = 0.0f64;
                            for &st in &step_texels {
                                let sx = (ix as f64 + d[0] * st).round() as isize;
                                let sz = (iz as f64 + d[1] * st).round() as isize;
                                if sx < 0 || sx >= size_i || sz < 0 || sz >= size_i {
                                    break;
                                }
                                let dh = top_ref[sz as usize * size + sx as usize] as f64 - y0;
                                if dh > 0.0 {
                                    let t = dh / (st * cell_m);
                                    if t > max_tan {
                                        max_tan = t;
                                    }
                                }
                            }
                            let inv1 = 1.0 / (1.0 + max_tan * max_tan);
                            sky_sum += inv1;
                            let w_dir = if max_tan > 0.0 {
                                (1.0 - FRAC_2_PI * (max_tan.atan() + max_tan * inv1)).clamp(0.0, 1.0)
                            } else {
                                1.0
                            };
                            bx_sum += d[0] * w_dir;
                            bz_sum += d[1] * w_dir;
                        }
                        let inv_az = 1.0 / HORIZON_AZIMUTHS as f64;
                        let sky_vis = (sky_sum * inv_az).clamp(0.0, 1.0) as f32;
                        let norm = 2.0 * inv_az * FRAC_PI_2;
                        let bent_x = (bx_sum * norm).clamp(-1.0, 1.0) as f32;
                        let bent_z = (bz_sum * norm).clamp(-1.0, 1.0) as f32;
                        ao_ch[row_o + ix] = [sky_vis, bent_x, bent_z];
                    }
                }
            });
        }
    });

    // 4. Dilate street/open horizon visibility inward into building footprint texels.
    let mut ao_cur = ao_raw;
    let mut val_cur = valid;
    let mut ao_next = ao_cur.clone();
    let mut val_next = val_cur.clone();
    for _ in 0..DILATE_PASSES {
        let mut any_filled = false;
        for iz in 0..size {
            let z0 = iz.saturating_sub(1);
            let z1 = (iz + 1).min(size - 1);
            for ix in 0..size {
                let idx = iz * size + ix;
                if val_cur[idx] != 0 {
                    continue;
                }
                let x0 = ix.saturating_sub(1);
                let x1 = (ix + 1).min(size - 1);
                let mut sum = [0.0f32; 3];
                let mut cnt = 0.0f32;
                for nz in z0..=z1 {
                    for nx in x0..=x1 {
                        let ni = nz * size + nx;
                        if val_cur[ni] != 0 {
                            let v = ao_cur[ni];
                            sum[0] += v[0];
                            sum[1] += v[1];
                            sum[2] += v[2];
                            cnt += 1.0;
                        }
                    }
                }
                if cnt > 0.0 {
                    ao_next[idx] = [sum[0] / cnt, sum[1] / cnt, sum[2] / cnt];
                    val_next[idx] = 1;
                    any_filled = true;
                }
            }
        }
        ao_cur.copy_from_slice(&ao_next);
        val_cur.copy_from_slice(&val_next);
        if !any_filled {
            break;
        }
    }
    // Deep interiors of mega-blocks not reached by dilation get a canyon default.
    for i in 0..total {
        if val_cur[i] == 0 {
            ao_cur[i] = [0.35, 0.0, 0.0];
        }
    }

    // 5. Mild 3x3 tent filter on (sky_vis, bent_x, bent_z) to remove sub-sector jitter.
    let mut ao_smooth = vec![[1.0f32, 0.0, 0.0]; total];
    let ao_ref = &ao_cur;
    std::thread::scope(|s| {
        for (tid, chunk) in ao_smooth.chunks_mut(rows_per * size).enumerate() {
            let z_start = tid * rows_per;
            s.spawn(move || {
                let rows = chunk.len() / size;
                for rz in 0..rows {
                    let iz = z_start + rz;
                    let zm = iz.saturating_sub(1);
                    let zp = (iz + 1).min(size - 1);
                    for ix in 0..size {
                        let xm = ix.saturating_sub(1);
                        let xp = (ix + 1).min(size - 1);
                        let c = ao_ref[iz * size + ix];
                        let n1 = ao_ref[zm * size + ix];
                        let n2 = ao_ref[zp * size + ix];
                        let n3 = ao_ref[iz * size + xm];
                        let n4 = ao_ref[iz * size + xp];
                        let d1 = ao_ref[zm * size + xm];
                        let d2 = ao_ref[zm * size + xp];
                        let d3 = ao_ref[zp * size + xm];
                        let d4 = ao_ref[zp * size + xp];
                        let mut out = [0.0f32; 3];
                        for k in 0..3 {
                            out[k] = (4.0 * c[k] + 2.0 * (n1[k] + n2[k] + n3[k] + n4[k]) + (d1[k] + d2[k] + d3[k] + d4[k])) * (1.0 / 16.0);
                        }
                        chunk[rz * size + ix] = out;
                    }
                }
            });
        }
    });

    // 6. Pack mip 0 and build mip 1..MIP_LEVELS (R max-pooled, GBA box-averaged).
    let mut cur_f32: Vec<[f32; 4]> = Vec::with_capacity(total);
    for i in 0..total {
        let a = ao_smooth[i];
        cur_f32.push([top_y[i], a[0], a[1], a[2]]);
    }
    let nmips = MIP_LEVELS.min((size.trailing_zeros() as usize) + 1);
    let mut mips = Vec::with_capacity(nmips);
    let mut side = size;
    for level in 0..nmips {
        let mut bytes = Vec::with_capacity(side * side * 8);
        for px in &cur_f32 {
            for &ch in px {
                bytes.extend_from_slice(&f16_bits(ch).to_le_bytes());
            }
        }
        mips.push(bytes);
        if level + 1 >= nmips || side <= 1 {
            break;
        }
        let next_side = side / 2;
        let mut next = vec![[0.0f32; 4]; next_side * next_side];
        for nz in 0..next_side {
            let z0 = nz * 2;
            for nx in 0..next_side {
                let x0 = nx * 2;
                let a = cur_f32[z0 * side + x0];
                let b = cur_f32[z0 * side + x0 + 1];
                let c = cur_f32[(z0 + 1) * side + x0];
                let d = cur_f32[(z0 + 1) * side + x0 + 1];
                next[nz * next_side + nx] = [
                    a[0].max(b[0]).max(c[0]).max(d[0]), // max-pooled height so ray-march never skips narrow towers
                    0.25 * (a[1] + b[1] + c[1] + d[1]),
                    0.25 * (a[2] + b[2] + c[2] + d[2]),
                    0.25 * (a[3] + b[3] + c[3] + d[3]),
                ];
            }
        }
        cur_f32 = next;
        side = next_side;
    }

    BakedSkylineField {
        bounds,
        mips,
        key,
        bake_ms: (t0.elapsed().as_secs_f64() * 1e3) as f32,
    }
}

/// Scanline-rasterize a 2D footprint ring into `top_y` (max update), also
/// stamping the centroid texel so narrow footprints never slip between centers.
fn rasterize_prism(bounds: &FieldBounds, ring: &[[f64; 2]], top_y: f32, grid: &mut [f32], xs: &mut Vec<f64>) {
    let n = ring.len();
    if n < 3 {
        return;
    }
    let size = bounds.size;
    let inv = 1.0 / bounds.cell_m;
    let (mut min_z, mut max_z, mut sx, mut sz) = (f64::MAX, f64::MIN, 0.0, 0.0);
    for p in ring {
        min_z = min_z.min(p[1]);
        max_z = max_z.max(p[1]);
        sx += p[0];
        sz += p[1];
    }
    // Always stamp centroid texel.
    let cx = ((sx / n as f64 - bounds.origin_x) * inv).floor() as isize;
    let cz = ((sz / n as f64 - bounds.origin_z) * inv).floor() as isize;
    if cx >= 0 && cx < size as isize && cz >= 0 && cz < size as isize {
        let slot = &mut grid[cz as usize * size + cx as usize];
        if top_y > *slot {
            *slot = top_y;
        }
    }
    let z0 = (((min_z - bounds.origin_z) * inv - 0.5).floor() as isize).max(0) as usize;
    let z1 = (((max_z - bounds.origin_z) * inv - 0.5).ceil() as isize).min(size as isize - 1);
    if z1 < z0 as isize {
        return;
    }
    for iz in z0..=(z1 as usize) {
        let wz = bounds.origin_z + (iz as f64 + 0.5) * bounds.cell_m;
        xs.clear();
        for i in 0..n {
            let a = ring[i];
            let b = ring[(i + 1) % n];
            if (a[1] <= wz && b[1] > wz) || (b[1] <= wz && a[1] > wz) {
                let t = (wz - a[1]) / (b[1] - a[1]);
                xs.push(a[0] + t * (b[0] - a[0]));
            }
        }
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(|a, b| a.total_cmp(b));
        for pair in xs.chunks_exact(2) {
            let x0 = (((pair[0] - bounds.origin_x) * inv - 0.5).ceil() as isize).max(0);
            let x1 = (((pair[1] - bounds.origin_x) * inv - 0.5).floor() as isize).min(size as isize - 1);
            if x1 < x0 {
                continue;
            }
            let row = iz * size;
            for ix in x0 as usize..=x1 as usize {
                let slot = &mut grid[row + ix];
                if top_y > *slot {
                    *slot = top_y;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_flat_ground_bakes_to_full_sky_vis_and_zero_bent_normal() {
        let bounds = FieldBounds { origin_x: -256.0, origin_z: -256.0, cell_m: 8.0, size: 64, max_y: 0.0 };
        let baked = bake_from_prisms(bounds, |_, _| 10.0, &[], "flat-test".into(), 2);
        let c = baked.texel(32, 32);
        assert!((c[0] - 10.0).abs() < 0.05, "height {c:?}");
        assert!(c[1] > 0.99, "open flat sky_vis {c:?}");
        assert!(c[2].abs() < 0.02 && c[3].abs() < 0.02, "open flat bent_xz {c:?}");
    }

    #[test]
    fn tall_tower_wall_darkens_canyon_and_bends_openness_away_from_tower() {
        // A 120 m tall block on the WEST side (x in [-160, -20], z in [-160, 160]),
        // leaving the EAST side completely open (like downtown facing the Bay).
        let bounds = FieldBounds { origin_x: -256.0, origin_z: -256.0, cell_m: 8.0, size: 64, max_y: 0.0 };
        let tower = HeightPrism {
            ring: vec![[-160.0, -160.0], [-20.0, -160.0], [-20.0, 160.0], [-160.0, 160.0]],
            top_y: 120.0,
        };
        let baked = bake_from_prisms(bounds, |_, _| 0.0, &[tower], "wall-test".into(), 2);
        // Sample just east of the tower wall (x = 0, z = 0 -> texel 32, 32).
        let east_street = baked.texel(32, 32);
        assert!(east_street[1] < 0.72, "east street sees half the sky occluded: {east_street:?}");
        assert!(east_street[2] > 0.35, "bent_x must point strongly EAST (+x) away from tower: {east_street:?}");
        assert!(east_street[3].abs() < 0.12, "bent_z near zero by symmetry: {east_street:?}");
        // Max-pooled mip 2 preserves the 120 m tower height.
        let tower_mip2 = baked.mip_texel(2, 6, 8);
        assert!(tower_mip2[0] >= 118.0, "max-pooled mip preserves tower top: {tower_mip2:?}");
        // Round-trip encode/decode and corruption check.
        let bytes = baked.encode();
        let dec = BakedSkylineField::decode(&bytes).unwrap();
        assert_eq!(dec.bounds, baked.bounds);
        assert_eq!(dec.mips, baked.mips);
        let mut bad = bytes.clone();
        bad[100] ^= 0xff;
        assert!(BakedSkylineField::decode(&bad).is_err());
    }

    #[test]
    fn narrow_street_canyon_between_two_towers_has_low_sky_vis() {
        let bounds = FieldBounds { origin_x: -256.0, origin_z: -256.0, cell_m: 8.0, size: 64, max_y: 0.0 };
        let west = HeightPrism { ring: vec![[-120.0, -160.0], [-12.0, -160.0], [-12.0, 160.0], [-120.0, 160.0]], top_y: 100.0 };
        let east = HeightPrism { ring: vec![[12.0, -160.0], [120.0, -160.0], [120.0, 160.0], [12.0, 160.0]], top_y: 100.0 };
        let baked = bake_from_prisms(bounds, |_, _| 0.0, &[west, east], "canyon-test".into(), 2);
        let street = baked.texel(32, 32);
        let open = baked.texel(32, 2);
        assert!(street[1] < 0.38, "narrow canyon between 100 m towers must be dark: {street:?}");
        assert!(open[1] > street[1] + 0.35, "open edge much brighter than canyon: open={open:?} street={street:?}");
    }
}
