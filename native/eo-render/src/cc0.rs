//! the CC0 scanned sets (eo-assets `cc0`) on the GPU-driven path.
//!
//! * Six texture arrays in group 0 of the city pipelines (bindings 17-19,
//!   23-25): the material sets at 2048 px (albedo BC7 sRGB, normal BC5, RHM
//!   BC7: roughness, occlusion, height, metalness) and the decal sets at
//!   1024 px, mips down to one block, one anisotropic repeat sampler (20).
//! * A per-material table (21, 4 vec4 a material, indexed like the material
//!   table): which set(s) a material draws, the multiplier that keeps the
//!   material's own mean colour (the kit bake's mean, or the vertex colour),
//!   the paint-state driver, normal strength, parallax depth, crevice grime.
//!   The JS-ported per-building appearance (tint, paint, wear, weathering,
//!   ageing) still drives every wall: the scans replace the base maps only.
//! * A per-layer table (22, 3 vec4 a layer): the scans' measured means and
//!   repeat size, which the street and ground shading normalise by so their
//!   delivered tone stays the JS one.
//! * The street decal grid (26: 8 x 8 toroidal tile slots, a header and 32 x
//!   32 cells each) and the decal records (27), written per street tile with
//!   its near pack (cc0_decals.rs).
//!
//! Everything is allocated by `attach` (once, when the downloads are present
//! and the adapter has BC compression); until every layer is resident the
//! `RUN_ON` bit is clear and every material draws as before.

use crate::authored::{F_GLASS, F_TEXTURED, F_VERTEX_COLOUR, MaterialTable};
use crate::cc0_decals::{CELLS, MAX_DECALS};
use eo_assets::cc0 as ac;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Runtime switches (frame uniform `copy_step.w`; cc0.wgsl `CC0_RUN_*`).
pub const RUN_ON: u32 = 1;
pub const RUN_WALLS: u32 = 2;
pub const RUN_STREET: u32 = 4;
pub const RUN_GROUND: u32 = 8;
pub const RUN_DECALS: u32 = 16;
pub const RUN_POM: u32 = 32;
pub const RUN_TILING: u32 = 64;
pub const RUN_GRIME: u32 = 128;
pub const RUN_STAINS: u32 = 256;
pub const RUN_DEFAULT: u32 = RUN_WALLS | RUN_STREET | RUN_GROUND | RUN_DECALS | RUN_POM | RUN_TILING | RUN_GRIME | RUN_STAINS;

/// vec4s per material / per layer in the tables.
pub const MAT_VEC4: usize = 4;
pub const LAYER_VEC4: usize = 3;
/// Toroidal street-tile slots of the decal grid (8 x 8 tiles of 128 m: wider
/// than the 310 m near-pack drop radius on either side).
pub const SLOT_SIDE: i32 = 8;
pub const SLOTS: u32 = (SLOT_SIDE * SLOT_SIDE) as u32;
/// vec4<u32> words of the grid: one header per slot, then 32 x 32 cells per slot.
pub const GRID_VEC4: u64 = SLOTS as u64 * (1 + (CELLS * CELLS) as u64);
pub const DECAL_VEC4: u64 = SLOTS as u64 * MAX_DECALS as u64 * 2;

/// Material modes (cc0.wgsl).
pub const MODE_PAINTED: u32 = 1; // the scan's luminance detail; the colour is the material's
pub const MODE_NATURAL: u32 = 2; // the scan's colour, its mean the material's
pub const MODE_DETAIL: u32 = 3; // a vertex-colour material: the scan's detail around the vertex colour
pub const MODE_PLAZA: u32 = 4; // plaza paving: the set by the style's vertex colour
pub const MODE_SAND: u32 = 5; // a sand sheet: dry and wet sand by height over the sea

/// Paint-state drivers: which per-building driver picks set B over set A.
pub const DRIVE_NONE: u32 = 0;
pub const DRIVE_PAINT: u32 = 1;
pub const DRIVE_WEAR: u32 = 2;

pub fn layer_of(id: &str) -> Option<u32> {
    ac::MATERIAL_SETS.iter().position(|d| d.id == id).map(|i| i as u32)
}

pub fn decal_layer_of(id: &str) -> Option<u32> {
    ac::DECAL_SETS.iter().position(|d| d.id == id).map(|i| i as u32)
}

/// How a material class draws from the sets: set A, set B (the other paint or
/// wear state), mode, driver, normal strength, parallax depth (m), crevice
/// grime, contrast (1 = the scan's own variance around the material's mean).
#[derive(Clone, Copy, Debug)]
pub struct ClassUse {
    pub a: &'static str,
    pub b: Option<&'static str>,
    pub mode: u32,
    pub drive: u32,
    pub normal: f32,
    pub pom_m: f32,
    pub grime: f32,
    pub contrast: f32,
}

const fn cu(a: &'static str, b: Option<&'static str>, mode: u32, drive: u32, normal: f32, pom_m: f32, grime: f32, contrast: f32) -> ClassUse {
    ClassUse { a, b, mode, drive, normal, pom_m, grime, contrast }
}

/// The kit finishes the sets replace (kit-material-specs.json names, as the
/// JS uses each: painted timber is drawn as clapboard; brick, stone and
/// concrete keep their scan colour around the kit's mean). Finishes not
/// listed (glass, canvas, signage, steel, lamp lens, membrane, terracotta,
/// composition shingle, the scanned plaster atlas) keep their kit maps.
pub fn kit_class(name: &str) -> Option<ClassUse> {
    Some(match name {
        "painted-clapboard" | "wood-siding" => cu("WoodSiding009", Some("Planks010"), MODE_PAINTED, DRIVE_PAINT, 1.0, 0.0, 0.5, 0.9),
        "cedar-shingle" => cu("WoodSiding011", None, MODE_NATURAL, DRIVE_NONE, 1.0, 0.0, 0.5, 0.9),
        "painted-shingle" => cu("WoodSiding011", None, MODE_PAINTED, DRIVE_NONE, 1.0, 0.0, 0.4, 0.7),
        // (The worn plaster scan's peeled patches read as camouflage under the paint: one stucco set.)
        "painted-stucco" | "float-finish-stucco" => cu("white_stucco", None, MODE_PAINTED, DRIVE_NONE, 0.8, 0.0, 0.25, 0.8),
        "painted-block" => cu("concrete_block_wall", None, MODE_PAINTED, DRIVE_NONE, 0.9, 0.0, 0.4, 0.7),
        "brick-running-bond" => cu("red_bricks_04", None, MODE_NATURAL, DRIVE_NONE, 1.0, 0.012, 0.5, 1.0),
        "brick-common-bond" => cu("brick_wall_006", None, MODE_NATURAL, DRIVE_NONE, 1.0, 0.012, 0.5, 1.0),
        "brick-buff" => cu("yellow_bricks", None, MODE_NATURAL, DRIVE_NONE, 1.0, 0.012, 0.5, 1.0),
        // Cast stone is moulded and dressed smooth (the sandstone ashlar scan read as rubble on plinths): smooth concrete.
        "cast-stone" => cu("concrete_wall_008", None, MODE_NATURAL, DRIVE_NONE, 0.7, 0.0, 0.3, 0.7),
        "limestone-ashlar" => cu("white_sandstone_blocks_02", None, MODE_NATURAL, DRIVE_NONE, 0.9, 0.010, 0.4, 0.8),
        "cast-stone-trim" => cu("concrete_wall_008", None, MODE_NATURAL, DRIVE_NONE, 0.6, 0.0, 0.2, 0.6),
        "concrete-precast" => cu("concrete_wall_008", None, MODE_NATURAL, DRIVE_NONE, 0.9, 0.0, 0.35, 0.9),
        "corrugated-metal" => cu("corrugated_iron_02", None, MODE_NATURAL, DRIVE_NONE, 1.0, 0.0, 0.2, 0.8),
        "standing-seam-metal" => cu("box_profile_metal_sheet", None, MODE_NATURAL, DRIVE_NONE, 1.0, 0.0, 0.2, 0.7),
        "glazed-tile" => cu("rectangular_facade_tiles", None, MODE_NATURAL, DRIVE_NONE, 1.0, 0.004, 0.3, 0.8),
        "sectional-garage-door" => cu("painted_metal_shutter", None, MODE_PAINTED, DRIVE_NONE, 1.0, 0.0, 0.3, 0.8),
        "asphalt-roof" => cu("tarred_gravel", None, MODE_NATURAL, DRIVE_NONE, 1.0, 0.0, 0.2, 1.0),
        "clay-barrel-tile" => cu("clay_roof_tiles_02", None, MODE_NATURAL, DRIVE_NONE, 1.0, 0.02, 0.3, 0.9),
        _ => return None,
    })
}

/// Landmark and open-space materials (landmarks.rs keys).
/// (The paved plazas and the waterfront apron are laid in the street's footway
/// layer and drawn by the street material: cc0.wgsl `cc0_street_concrete`.)
pub fn structure_class(key: &str) -> Option<ClassUse> {
    if key.ends_with(":sand") {
        return Some(cu("coast_sand_04", Some("damp_beach_sand"), MODE_SAND, DRIVE_NONE, 1.0, 0.0, 0.0, 1.0));
    }
    if key.ends_with(":planting") {
        return Some(cu("park_dirt", None, MODE_DETAIL, DRIVE_NONE, 1.0, 0.0, 0.0, 0.9));
    }
    // The quay wall's face, the promenade seawalls, the waterfront's concrete (wall cap, fascias) and the pier decks.
    if key == "seawall" || key.contains("seawall") || key == "structure:waterfront:concrete" {
        return Some(cu("wood_textured_concrete", None, MODE_DETAIL, DRIVE_NONE, 0.8, 0.0, 0.3, 0.8));
    }
    // the shore's rock (eo-landmarks rocks.rs): the armour's quarried stone and the boulders, the scan's detail round the vertex tone
    if key == "structure:shore-rocks:armour" {
        return Some(cu("granite_wall", None, MODE_DETAIL, DRIVE_NONE, 1.0, 0.0, 0.4, 0.9));
    }
    if key == "structure:shore-rocks:boulders" {
        return Some(cu("seaside_rock", None, MODE_DETAIL, DRIVE_NONE, 1.0, 0.0, 0.4, 0.9));
    }
    if key.contains("ferry-pier-deck") {
        return Some(cu("concrete_wall_008", None, MODE_DETAIL, DRIVE_NONE, 0.8, 0.0, 0.3, 0.8));
    }
    if key.starts_with("landmark:") {
        // Bridge concrete only (anchorages, piers, approach bents): the painted steel keeps landmark-surface.js.
        return None;
    }
    None
}

/// The scans' measures, per layer.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayerInfo {
    pub mean_rgb: [f32; 3],
    pub mean_rough: f32,
    pub mean_ao: f32,
    pub mean_height: f32,
    pub size_m: [f32; 2],
}

impl LayerInfo {
    pub fn from_set(s: &ac::CookedSet) -> Self {
        Self { mean_rgb: s.mean_rgb, mean_rough: s.mean_rough, mean_ao: s.mean_ao, mean_height: s.mean_height, size_m: s.size_m }
    }
    pub fn luma(&self) -> f32 {
        0.2126 * self.mean_rgb[0] + 0.7152 * self.mean_rgb[1] + 0.0722 * self.mean_rgb[2]
    }
}

/// The per-layer table (material layers, then decal layers).
pub fn layer_table(mats: &[LayerInfo], decals: &[LayerInfo]) -> Vec<[f32; 4]> {
    let mut v = vec![];
    for l in mats.iter().chain(decals.iter()) {
        v.push([l.mean_rgb[0], l.mean_rgb[1], l.mean_rgb[2], l.luma()]);
        v.push([l.mean_rough, l.mean_ao, l.mean_height, 0.0]);
        v.push([1.0 / l.size_m[0].max(0.01), 1.0 / l.size_m[1].max(0.01), l.size_m[0], l.size_m[1]]);
    }
    v
}

/// The per-material table. `kit_means[l]`: linear mean of kit map layer l
/// (the bake each kit material drew before); `layers`: the material sets.
/// A kit material keeps its bake's mean colour: natural sets are scaled
/// channel by channel to it, painted sets contribute luminance only.
pub fn material_table(mt: &MaterialTable, kit_means: &[[f32; 3]], layers: &[LayerInfo]) -> (Vec<[f32; 4]>, usize) {
    let mut v = vec![[-1.0f32, -1.0, 0.0, 0.0]; mt.gpu.len() * MAT_VEC4];
    let mut mapped = 0;
    for (i, g) in mt.gpu.iter().enumerate() {
        let name = mt.names.get(i).map(|s| s.as_str()).unwrap_or("");
        // (`<finish>*<k>~<c>`: a kit finish drawn k times larger, its scan's variation times c.)
        let (base, rep, contrast) = crate::authored::kit_repeat(name.split('@').next().unwrap_or(""));
        let (rep, contrast) = (rep as f32, contrast as f32);
        let textured_kit = g.flags & F_TEXTURED != 0 && g.flags & F_GLASS == 0 && g.layer >= 0;
        let (c, kit_mean) = if textured_kit {
            match (kit_class(base), kit_means.get(g.layer as usize)) {
                (Some(c), Some(m)) => (c, *m),
                _ => continue,
            }
        } else if g.flags & F_VERTEX_COLOUR != 0 {
            match structure_class(name) {
                Some(c) => (c, [1.0, 1.0, 1.0]),
                None => continue,
            }
        } else {
            continue;
        };
        let Some(la) = layer_of(c.a) else { continue };
        let lb = c.b.and_then(layer_of);
        let mul = |l: u32| -> [f32; 3] {
            let li = &layers[l as usize];
            match c.mode {
                MODE_PAINTED => kit_mean.map(|k| (k / li.luma().max(1e-3)).clamp(0.02, 50.0)),
                _ => [0, 1, 2].map(|k| (kit_mean[k] / li.mean_rgb[k].max(1e-3)).clamp(0.02, 50.0)),
            }
        };
        let la_info = &layers[la as usize];
        let ma = mul(la);
        let mb = lb.map(mul).unwrap_or(ma);
        v[i * MAT_VEC4] = [la as f32, lb.map(|l| l as f32).unwrap_or(-1.0), 1.0 / (la_info.size_m[0] * rep), 1.0 / (la_info.size_m[1] * rep)];
        v[i * MAT_VEC4 + 1] = [ma[0], ma[1], ma[2], (c.mode | c.drive << 4) as f32];
        // (A scaled finish is dressed stone at a larger course, not a larger rough
        // block: its relief keeps the scan's height, so its slopes ease by the scale.)
        v[i * MAT_VEC4 + 2] = [mb[0], mb[1], mb[2], c.normal / rep.sqrt()];
        v[i * MAT_VEC4 + 3] = [c.pom_m, c.grime, c.contrast * contrast, lb.map(|l| 1.0 / (layers[l as usize].size_m[0] * rep)).unwrap_or(0.0)];
        mapped += 1;
    }
    (v, mapped)
}

/// The Golden Gate tide station's high waters over mean sea level, real
/// metres (NOAA 9414290; one definition, in eo-landmarks ground_cover, which
/// the shore's stones read too).
pub use eo_landmarks::ground_cover::{MHHW_OVER_MSL_M, MHW_OVER_MSL_M};

/// The shore ground pass's sand constants (shore_surface.wgsl `lm_beach`,
/// cc0.wgsl `cc0_beach`): on (`--no-sand-albedo` off), the measured wet-sand
/// ratio, the tide's high-water heights over the sea at the drawn height
/// scale (the ground over the sea is drawn `height_scale` times its height),
/// and the foreshore's slope the bands' height is read on (eo-water
/// `BEACH_FORESHORE_SLOPE`, the slope the carpet lays the beach at: rise over
/// run in the drawn world).
pub fn shore_ground_wgsl(on: bool, height_scale: f64) -> String {
    format!(
        "const GG2_SAND: bool = {on};\nconst GG2_WET_SAND_RATIO: f32 = {:.4};\nconst GG2_SAND_MHW_M: f32 = {:.4};\nconst GG2_SAND_MHHW_M: f32 = {:.4};\nconst GG2_FORESHORE_SLOPE: f32 = {:.6};\n",
        eo_landmarks::ground_cover::WET_SAND_RATIO,
        MHW_OVER_MSL_M * height_scale,
        MHHW_OVER_MSL_M * height_scale,
        eo_water::field::BEACH_FORESHORE_SLOPE
    )
}

/// The Fort Point seawall's coursed granite (`lm_seawall`; `--no-riprap` off
/// with the armour in front of it): the walls inside the revetment's box
/// (eo-landmarks rocks.rs `REVETMENT_BOX_SURVEYED`, drawn at the ground
/// plan), granite ashlar in courses of `ASHLAR_COURSE_M` and blocks of
/// `ASHLAR_BLOCK_M` (INFERRED: 2 ft courses, 4-6 ft stones, no survey of the
/// 1869 wall's coursing was read), drawn at the height and plan scales, in the
/// granite scan's mean albedo (rocks.rs `ARMOUR_ALBEDO`).
pub const ASHLAR_COURSE_M: f64 = 0.61;
pub const ASHLAR_BLOCK_M: [f64; 2] = [1.22, 1.83];

pub fn ashlar_wgsl(on: bool, plan: f64, hs: f64) -> String {
    let b = eo_landmarks::rocks::REVETMENT_BOX_SURVEYED.map(|v| v * plan);
    let g = eo_landmarks::rocks::ARMOUR_ALBEDO;
    format!(
        "const GG2_ASHLAR: bool = {on};\nconst GG2_ASHLAR_BOX: vec4<f32> = vec4<f32>({:.2}, {:.2}, {:.2}, {:.2});\nconst GG2_ASHLAR_COURSE_M: f32 = {:.4};\nconst GG2_ASHLAR_BLOCK_M: vec2<f32> = vec2<f32>({:.4}, {:.4});\nconst GG2_GRANITE: vec3<f32> = vec3<f32>({:.4}, {:.4}, {:.4});\n",
        b[0], b[1], b[2], b[3], ASHLAR_COURSE_M * hs, ASHLAR_BLOCK_M[0] * plan, ASHLAR_BLOCK_M[1] * plan, g[0], g[1], g[2]
    )
}

/// WGSL constants: switches, layer indices, the decal grid's layout, the
/// SF scoring grid (`caMetric('sidewalk-scoring-grid-sf')`).
pub fn wgsl_constants(scoring_m: f32) -> String {
    let mut s = String::from("\n// generated by cc0.rs wgsl_constants\n");
    s.push_str(&crate::ground2::wgsl_constants()); // the paving branch of cc0_ground_sets
    s.push_str(&shore_ground_wgsl(eo_landmarks::features::shore_ground("sand-albedo"), eo_data::groundscale::height_scale()));
    s.push_str(&ashlar_wgsl(eo_landmarks::features::shore_ground("riprap"), eo_data::groundscale::scale(), eo_data::groundscale::height_scale()));
    for (k, v) in [
        ("ON", RUN_ON),
        ("WALLS", RUN_WALLS),
        ("STREET", RUN_STREET),
        ("GROUND", RUN_GROUND),
        ("DECALS", RUN_DECALS),
        ("POM", RUN_POM),
        ("TILING", RUN_TILING),
        ("GRIME", RUN_GRIME),
        ("STAINS", RUN_STAINS),
    ] {
        s.push_str(&format!("const CC0_RUN_{k}: u32 = {v}u;\n"));
    }
    for (i, d) in ac::MATERIAL_SETS.iter().enumerate() {
        s.push_str(&format!("const CC0_L_{}: i32 = {i};\n", d.id.to_uppercase()));
    }
    for (i, d) in ac::DECAL_SETS.iter().enumerate() {
        s.push_str(&format!("const CC0_D_{}: i32 = {i};\n", d.id.to_uppercase()));
    }
    s.push_str(&format!("const CC0_MAT_LAYERS: u32 = {}u;\n", ac::MATERIAL_SETS.len()));
    s.push_str(&format!(
        "const CC0_SLOT_SIDE: i32 = {SLOT_SIDE};\nconst CC0_SLOTS: u32 = {SLOTS}u;\nconst CC0_CELLS: u32 = {}u;\nconst CC0_MAX_DECALS: u32 = {}u;\nconst CC0_TILE_M: f32 = {:.1};\n",
        CELLS,
        MAX_DECALS,
        eo_street::STREET_TILE_M
    ));
    s.push_str(&format!("const CC0_SCORING_M: f32 = {scoring_m:.6};\n"));
    // The plaza styles' tones (plaza-paving.js, landmarks-js.json openSpace.colors), linear as the street decodes them.
    let os = &eo_landmarks::LandmarksJs::get().open_space.colors;
    for (k, name) in [("BRICK", "plazaBrick"), ("STONE", "plazaStone"), ("CONCRETE", "plazaConcrete")] {
        let c = os.get(name).and_then(|h| eo_assets::kit::hex_to_linear(h)).unwrap_or([-1.0; 3]);
        s.push_str(&format!("const CC0_PLAZA_{k}: vec3<f32> = vec3<f32>({:.6}, {:.6}, {:.6});\n", c[0], c[1], c[2]));
    }
    for (k, v) in [("PAINTED", MODE_PAINTED), ("NATURAL", MODE_NATURAL), ("DETAIL", MODE_DETAIL), ("PLAZA", MODE_PLAZA), ("SAND", MODE_SAND)] {
        s.push_str(&format!("const CC0_MODE_{k}: u32 = {v}u;\n"));
    }
    for (k, v) in [
        ("WHEEL", crate::cc0_decals::DK_WHEEL),
        ("OIL_LINE", crate::cc0_decals::DK_OIL_LINE),
        ("OIL_SPOT", crate::cc0_decals::DK_OIL_SPOT),
        ("PATCH", crate::cc0_decals::DK_PATCH),
        ("CRACK", crate::cc0_decals::DK_CRACK),
        ("MANHOLE", crate::cc0_decals::DK_MANHOLE),
        ("VAULT", crate::cc0_decals::DK_VAULT),
        ("GUM", crate::cc0_decals::DK_GUM),
        ("STAIN", crate::cc0_decals::DK_STAIN),
        ("WALK_CRACK", crate::cc0_decals::DK_WALK_CRACK),
        ("TYRE", crate::cc0_decals::DK_TYRE),
        ("GRATE", crate::cc0_decals::DK_GRATE),
    ] {
        s.push_str(&format!("const CC0_DK_{k}: u32 = {v}u;\n"));
    }
    // The carriageway's tone, and the kerb-face grime's reach (caMetric('curb-height-sf-standard')).
    s.push_str(&format!("const CC0_ASPHALT_ALBEDO: f32 = {ASPHALT_ALBEDO:.6};\nconst CC0_ASPHALT_REF_LUMA: f32 = {:.6};\nconst CC0_ASPHALT_DETAIL_GAIN: f32 = {ASPHALT_DETAIL_GAIN:.6};\n", asphalt_ref_luma()));
    let t = asphalt_tint();
    s.push_str(&format!("const CC0_ASPHALT_TINT: vec3<f32> = vec3<f32>({:.6}, {:.6}, {:.6});\nconst CC0_ASPHALT_SPREAD: f32 = {ASPHALT_SPREAD:.6};\n", t[0], t[1], t[2]));
    s.push_str(&format!("const CC0_CURB_H: f32 = {:.6};\n", crate::cc0_decals::standards().curb_height_m));
    // The worn-paint scan's own stripe drawn at a standard line width (caMetric('line-width-local'), 4-6 in).
    s.push_str(&format!("const CC0_PAINT_STRIPE_M: f32 = {:.6};\n", crate::cc0_decals::standards().line_width_m.max(0.05)));
    s
}

/// The carriageway's albedo with the CC0 street on: aged asphalt pavement,
/// neutral grey. LBNL Heat Island Group (Cool Pavements): fresh asphalt
/// reflects 0.04-0.06 of sunlight, weathered asphalt about 0.10-0.18; San
/// Francisco resurfaces its streets on a cycle, so the street draws the
/// weathered band's lower end, and each street's class and age (the JS
/// palette and bleach) vary it as luminance around this.
pub const ASPHALT_ALBEDO: f32 = 0.10;
/// More of the asphalt scan's own fine variation (aggregate, binder) inside the carriageway.
pub const ASPHALT_DETAIL_GAIN: f32 = 1.6;
/// The JS class and age variation kept as luminance, compressed (an exponent
/// on its ratio to the residential mid-cycle): the JS spread (service at its
/// most bleached is 24 % over the mid) would carry the brightest streets past
/// the photographs' sunlit band.
pub const ASPHALT_SPREAD: f64 = 0.5;

/// The carriageway albedo (luminance) for a JS carriageway of linear luminance `js_luma`.
pub fn asphalt_albedo(js_luma: f64) -> f64 {
    ASPHALT_ALBEDO as f64 * (js_luma / asphalt_ref_luma() as f64).max(0.0).powf(ASPHALT_SPREAD)
}
/// How much of the noon rig's warmth the carriageway's grey cancels (0: a
/// neutral albedo; 1: a neutral sunlit radiance at noon). The renderer has no
/// white balance, so a neutral albedo under the warm key reads warm in the sun;
/// cancelling it pushes the shaded asphalt past the JS frames' blue band (the
/// sweep `delivered::cc0_carriageway_reads_dark_neutral_grey_in_sun` prints),
/// so the grey stays neutral.
pub const ASPHALT_SUN_NEUTRAL: f64 = 0.0;

/// The carriageway grey's chroma (luminance 1): neutral, or cooled by
/// `ASPHALT_SUN_NEUTRAL` of the noon key-plus-fill's warmth on an upward face.
pub fn asphalt_tint() -> [f32; 3] {
    asphalt_tint_with(ASPHALT_SUN_NEUTRAL)
}

/// `asphalt_tint` at a given share of the noon warmth cancelled.
pub fn asphalt_tint_with(share: f64) -> [f32; 3] {
    if share <= 0.0 {
        return [1.0; 3];
    }
    let f = eo_look::LookFrame::new(12.0);
    let env_class = eo_street::material::StreetMaterial::get().env_base.get("asphalt").copied().unwrap_or(1.0);
    let n = [0.0, 1.0, 0.0];
    let sh = eo_look::sh_eval(&f.env_sh, n);
    let nl = f.key_dir[1].max(0.0);
    let e = [0, 1, 2].map(|i| f.key[i] * nl + f.hemi_sky[i] + f.ambient[i] + sh[i] * f.rig.environment_intensity * env_class);
    let l = 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
    let t = e.map(|c| (l / c.max(1e-9)).powf(share));
    let lt = 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
    t.map(|c| (c / lt) as f32)
}

/// The JS carriageway's mean linear luminance the CC0 street rescales to
/// `ASPHALT_ALBEDO`: the residential class tint at the mid-cycle bleach
/// (street-surface-age.js) times the base fold, as the street shader draws it.
pub fn asphalt_ref_luma() -> f32 {
    let m = eo_street::material::StreetMaterial::get();
    let bleach = (m.surface_age.bleach_fresh + m.surface_age.bleach_aged) / 2.0;
    let srgb = m.asphalt_tint("residential").map(|v| ((v as f64 * bleach).round()) as u8);
    let base = m.pbr_sets.carriageway.base_mean_linear.unwrap_or([1.0; 3]);
    let lin = |v: u8| eo_look::math::srgb_to_linear(v as f64 / 255.0);
    let c = [lin(srgb[0]) * base[0], lin(srgb[1]) * base[1], lin(srgb[2]) * base[2]];
    (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]) as f32
}

/// The SF sidewalk scoring pitch from the California street standards table.
pub fn scoring_pitch_m() -> f32 {
    static P: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *P.get_or_init(|| {
        eo_data::DataPaths::discover()
            .ok()
            .and_then(|p| eo_data::CaStandards::load(&p).ok())
            .and_then(|s| s.metric("sidewalk-scoring-grid-sf").ok())
            .map(|v| v as f32)
            .unwrap_or(0.0)
    })
}

/// What `attach` allocates (once).
struct Res {
    mat: [wgpu::Texture; 3],
    decal: [wgpu::Texture; 3],
    mats: wgpu::Buffer,
    layers: wgpu::Buffer,
    grid: wgpu::Buffer,
    decals: wgpu::Buffer,
}

/// What `make_draw_bg` binds (cloned handles).
pub struct Bind {
    pub mat: [wgpu::TextureView; 3],
    pub decal: [wgpu::TextureView; 3],
    pub mats: wgpu::Buffer,
    pub layers: wgpu::Buffer,
    pub grid: wgpu::Buffer,
    pub decals: wgpu::Buffer,
}

pub struct Cc0Gpu {
    /// Bound in the arrays' place until every layer is written: an array bound
    /// before its layers are written would be zero-filled by wgpu at its first
    /// use (about 600 MB of clears in the first frame).
    placeholder: wgpu::Texture,
    /// Bound in the tables' and the decal grid's place until `attach`.
    dummy: wgpu::Buffer,
    res: OnceLock<Res>,
    pub sampler: wgpu::Sampler,
    /// Requested switches (`RUN_*` without `RUN_ON`).
    options: AtomicU32,
    /// Every layer resident and the tables written.
    on: AtomicBool,
    /// VRAM allocated by `attach` (bytes).
    pub bytes: AtomicU64,
}

fn array_view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })
}

fn placeholder(d: &wgpu::Device, label: &str) -> wgpu::Texture {
    d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn storage(d: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    // COPY_SRC: the upload test reads the decal grid back.
    d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false })
}

pub const FORMATS: [wgpu::TextureFormat; 3] = [wgpu::TextureFormat::Bc7RgbaUnormSrgb, wgpu::TextureFormat::Bc5RgUnorm, wgpu::TextureFormat::Bc7RgbaUnorm];

/// Bytes of a square BC array (BC7 and BC5: one byte a texel), mips to one block.
pub fn array_bytes(res: u32, layers: usize) -> u64 {
    (0..ac::mip_count(res)).map(|m| ((res >> m) as u64).pow(2)).sum::<u64>() * layers as u64
}

impl Cc0Gpu {
    pub fn new(d: &wgpu::Device) -> Self {
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cc0-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        Self {
            placeholder: placeholder(d, "cc0-placeholder"),
            dummy: storage(d, "cc0-dummy", 16),
            res: OnceLock::new(),
            sampler,
            options: AtomicU32::new(0),
            on: AtomicBool::new(false),
            bytes: AtomicU64::new(0),
        }
    }

    pub fn bind(&self) -> Bind {
        let p = || array_view(&self.placeholder);
        let Some(r) = self.res.get() else {
            let b = || self.dummy.clone();
            return Bind { mat: [p(), p(), p()], decal: [p(), p(), p()], mats: b(), layers: b(), grid: b(), decals: b() };
        };
        let on = self.on.load(Ordering::Relaxed);
        let v = |t: &wgpu::Texture| if on { array_view(t) } else { p() };
        Bind {
            mat: [v(&r.mat[0]), v(&r.mat[1]), v(&r.mat[2])],
            decal: [v(&r.decal[0]), v(&r.decal[1]), v(&r.decal[2])],
            mats: r.mats.clone(),
            layers: r.layers.clone(),
            grid: r.grid.clone(),
            decals: r.decals.clone(),
        }
    }

    pub fn attached(&self) -> bool {
        self.res.get().is_some()
    }

    /// The switches the frame uniform carries: the options, plus `RUN_ON` once ready.
    pub fn run_bits(&self) -> u32 {
        let o = self.options.load(Ordering::Relaxed);
        if self.on.load(Ordering::Relaxed) && o != 0 { o | RUN_ON } else { 0 }
    }

    pub fn set_options(&self, bits: u32) {
        self.options.store(bits & !RUN_ON, Ordering::Relaxed);
    }

    pub fn options(&self) -> u32 {
        self.options.load(Ordering::Relaxed)
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// Array `k` (0-2 the material maps, 3-5 the decal maps), once attached:
    /// the upload stager's destination for `upload::Target::Cc0Texture`.
    pub fn array(&self, k: u8) -> Option<&wgpu::Texture> {
        let r = self.res.get()?;
        if k < 3 { r.mat.get(k as usize) } else { r.decal.get(k as usize - 3) }
    }

    /// The street decal grid and records (`upload::Target::Cc0Grid` / `Cc0Decals`), once attached.
    pub fn grid_buffer(&self) -> Option<&wgpu::Buffer> {
        self.res.get().map(|r| &r.grid)
    }

    pub fn decals_buffer(&self) -> Option<&wgpu::Buffer> {
        self.res.get().map(|r| &r.decals)
    }

    /// Allocate the arrays, tables and decal buffers (the caller rebinds group 0).
    pub fn attach(&self, d: &wgpu::Device, material_count: usize) -> u64 {
        if self.res.get().is_some() {
            return 0;
        }
        let tex = |label: &str, f: wgpu::TextureFormat, res: u32, layers: u32| -> wgpu::Texture {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: layers.max(1) },
                mip_level_count: ac::mip_count(res),
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: f,
                // COPY_SRC: the upload test reads a banded mip back.
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let names = ["cc0-albedo", "cc0-normal", "cc0-rhm"];
        let dnames = ["cc0-decal-albedo", "cc0-decal-normal", "cc0-decal-rhm"];
        let mat = [0, 1, 2].map(|k| tex(names[k], FORMATS[k], ac::MATERIAL_RES, ac::MATERIAL_SETS.len() as u32));
        let decal = [0, 1, 2].map(|k| tex(dnames[k], FORMATS[k], ac::DECAL_RES, ac::DECAL_SETS.len() as u32));
        let mats = (material_count.max(1) * MAT_VEC4 * 16) as u64;
        let layers = ((ac::MATERIAL_SETS.len() + ac::DECAL_SETS.len()) * LAYER_VEC4 * 16) as u64;
        let r = Res {
            mat,
            decal,
            mats: storage(d, "cc0-mats", mats),
            layers: storage(d, "cc0-layers", layers),
            grid: storage(d, "cc0-grid", GRID_VEC4 * 16),
            decals: storage(d, "cc0-decals", DECAL_VEC4 * 16),
        };
        let bytes = 3 * (array_bytes(ac::MATERIAL_RES, ac::MATERIAL_SETS.len()) + array_bytes(ac::DECAL_RES, ac::DECAL_SETS.len())) + mats + layers + GRID_VEC4 * 16 + DECAL_VEC4 * 16;
        if self.res.set(r).is_err() {
            return 0;
        }
        self.bytes.store(bytes, Ordering::Relaxed);
        DECALS_WANTED.store(self.options() & RUN_DECALS != 0, Ordering::Relaxed);
        bytes
    }

    /// Bytes of the material arrays, the decal arrays, and the tables + decal grid and records.
    pub fn bytes_by_part(&self) -> (u64, u64, u64) {
        if !self.attached() {
            return (0, 0, 0);
        }
        let mat = 3 * array_bytes(ac::MATERIAL_RES, ac::MATERIAL_SETS.len());
        let dec = 3 * array_bytes(ac::DECAL_RES, ac::DECAL_SETS.len());
        let total = self.bytes.load(Ordering::Relaxed);
        (mat, dec, total.saturating_sub(mat + dec))
    }

    /// One budgeted write on the direct path (`--no-upload-ring`; on the
    /// staged path the stager copies the arrays' bands and the grid itself):
    /// for a texture `offset` is the band's first byte in the mip, at a whole
    /// block row.
    pub fn write(&self, q: &wgpu::Queue, target: crate::upload::Target, offset: u64, data: &[u8]) {
        let Some(r) = self.res.get() else { return };
        match target {
            crate::upload::Target::Cc0Texture { array, layer, mip } => {
                let Some(t) = self.array(array).and_then(|t| crate::UploadTexture::of(t, mip as u32, layer as u32)) else { return };
                let row = t.row_bytes();
                let rows = (data.len() as u64 / row).max(1) as u32;
                let y0 = (offset / row) as u32;
                q.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: t.texture, mip_level: t.mip, origin: wgpu::Origin3d { x: 0, y: y0 * t.block, z: t.layer }, aspect: wgpu::TextureAspect::All },
                    data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: Some(rows) },
                    wgpu::Extent3d { width: t.width.next_multiple_of(t.block), height: rows * t.block, depth_or_array_layers: 1 },
                );
            }
            crate::upload::Target::Cc0Grid => q.write_buffer(&r.grid, offset, data),
            crate::upload::Target::Cc0Decals => q.write_buffer(&r.decals, offset, data),
            _ => {}
        }
    }

    /// Write both tables and switch the sets on (the caller rebinds group 0: `Renderer::cc0_finish`).
    pub fn finish(&self, q: &wgpu::Queue, mats: &[[f32; 4]], layers: &[[f32; 4]]) {
        let Some(r) = self.res.get() else { return };
        if (mats.len() * 16) as u64 <= r.mats.size() {
            q.write_buffer(&r.mats, 0, bytemuck::cast_slice(mats));
        }
        if (layers.len() * 16) as u64 <= r.layers.size() {
            q.write_buffer(&r.layers, 0, bytemuck::cast_slice(layers));
        }
        self.on.store(true, Ordering::Relaxed);
    }

    /// Clear the slot headers (at attach: before any tile writes one).
    pub fn clear_headers(&self, q: &wgpu::Queue) {
        let Some(r) = self.res.get() else { return };
        let empty: Vec<[u32; 4]> = vec![empty_header(); SLOTS as usize];
        q.write_buffer(&r.grid, 0, bytemuck::cast_slice(&empty));
    }
}

/// Whether street tiles place decals (set when the decal buffers are attached with decals on).
pub static DECALS_WANTED: AtomicBool = AtomicBool::new(false);

/// The toroidal slot of a street tile.
pub fn slot_of(key: (i32, i32)) -> u32 {
    (key.0.rem_euclid(SLOT_SIDE) + SLOT_SIDE * key.1.rem_euclid(SLOT_SIDE)) as u32
}

/// Byte offsets in the grid buffer: a slot's header, its cells; in the decal buffer: its records.
pub fn header_offset(slot: u32) -> u64 {
    slot as u64 * 16
}
pub fn cells_offset(slot: u32) -> u64 {
    (SLOTS as u64 + slot as u64 * (CELLS * CELLS) as u64) * 16
}
pub fn decals_offset(slot: u32) -> u64 {
    slot as u64 * MAX_DECALS as u64 * 2 * 16
}

/// A tile slot header: tile key, decal count.
pub fn header(key: (i32, i32), count: u32) -> [u32; 4] {
    [key.0 as u32, key.1 as u32, count, 1]
}

pub fn empty_header() -> [u32; 4] {
    [0x8000_0000, 0x8000_0000, 0, 0]
}

/// The upload writes of one cooked set: every mip of its three maps, whole.
pub fn set_writes(kind: ac::SetKind, layer: u32, set: &mut ac::CookedSet) -> Vec<crate::upload::Write> {
    let base = if kind == ac::SetKind::Material { 0u8 } else { 3u8 };
    let mut out = vec![];
    for (map, mips) in set.maps.iter_mut().enumerate() {
        // Smallest mips first: a layer's coarse levels land before its finest.
        // Each mip whole: the upload queue bands it in block rows through the
        // stager under the pump's byte and time budget (hitch2).
        for (mip, data) in std::mem::take(mips).into_iter().enumerate().rev() {
            out.push(crate::upload::Write { target: crate::upload::Target::Cc0Texture { array: base + map as u8, layer: layer as u16, mip: mip as u8 }, offset: 0, data: crate::upload::Data::Bytes(data) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring_pitch_reads_the_standards_table() {
        // caMetric('sidewalk-scoring-grid-sf'): 3 ft.
        assert!((scoring_pitch_m() - 0.914).abs() < 1e-3, "{}", scoring_pitch_m());
    }

    /// The city shader with the CC0 code appended parses and validates (naga,
    /// on the CPU: a WGSL error would otherwise surface only when a renderer
    /// is made on a GPU).
    #[test]
    fn city_shader_with_cc0_parses_and_validates() {
        use wgpu::naga;
        let src = crate::look::city_shader_source();
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("city shader: {}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("city shader: {}", e.emit_to_string(&src)));
    }

    #[test]
    fn every_class_names_a_set() {
        for name in ["painted-clapboard", "wood-siding", "painted-stucco", "brick-running-bond", "concrete-precast", "asphalt-roof"] {
            let c = kit_class(name).unwrap();
            assert!(layer_of(c.a).is_some(), "{name}: {}", c.a);
            assert!(c.b.is_none_or(|b| layer_of(b).is_some()));
        }
        assert!(wgsl_constants(0.914).contains("CC0_L_WOODSIDING009"));
        assert!(wgsl_constants(0.914).contains("const GG2_SAND: bool"));
        assert_eq!(slot_of((-1, -1)), 63);
        assert_eq!(slot_of((8, 0)), 0);
    }

    #[test]
    fn the_tide_heights_are_drawn_at_the_height_scale() {
        // MHW 0.643 m and MHHW 0.829 m over mean sea level, x 1.25 drawn
        let w = shore_ground_wgsl(true, 1.25);
        assert!((MHW_OVER_MSL_M - 0.643).abs() < 1e-9 && (MHHW_OVER_MSL_M - 0.829).abs() < 1e-9);
        assert!(w.contains("GG2_SAND_MHW_M: f32 = 0.80"), "{w}");
        assert!(w.contains("GG2_SAND_MHHW_M: f32 = 1.03"), "{w}");
        assert!(w.contains("GG2_WET_SAND_RATIO: f32 = 0.5000"));
        assert!(shore_ground_wgsl(false, 1.25).contains("GG2_SAND: bool = false"));
        // the bands read the carpet's foreshore (1:55): the high water 44 m drawn from the shoreline
        assert!(w.contains("GG2_FORESHORE_SLOPE: f32 = 0.018182"), "{w}");
        assert!((MHW_OVER_MSL_M * 1.25 / eo_water::field::BEACH_FORESHORE_SLOPE - 44.2).abs() < 0.1);
        // the ashlar's courses at the height scale, its blocks at the plan
        let a = ashlar_wgsl(true, 1.25, 1.25);
        assert!(a.contains("GG2_ASHLAR_COURSE_M: f32 = 0.7625"), "{a}");
        assert!(a.contains("vec2<f32>(1.5250, 2.2875)"), "{a}");
    }
}
