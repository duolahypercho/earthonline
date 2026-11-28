//! The rear yards' ground (`--yard-detail`, off by default;
//! `--no-yard-detail`): the yard survey's ground classes (eo-props `yards.rs`:
//! lawn, planting, paving, deck from the USDA NAIP aerial and the USGS lidar,
//! every 0.5 m of the surveyed plan) drawn by the terrain's shader where it
//! draws the open ground (draw.wgsl calls yards.wgsl `yd_ground` before
//! cc0.wgsl `cc0_ground`), so the yards are lit, shadowed and fogged as the
//! terrain is. Each class takes the tone the city's open ground already uses
//! for it (ground2.rs: `YARD` for lawn, `CANOPY` for planting, `CONCRETE` for
//! paving) and a deck the fences' wood (the kit's unpainted cedar,
//! props_yards.rs), with a CC0 scan's detail round that tone: lawn `Grass004`
//! with `leafy_grass`, planting `leafy_grass` with `forest_floor`, paving
//! `concrete_floor_worn_001`, deck `Planks010`. A cell the survey leaves out
//! (not a yard), every point off the maps, and the whole ground without the
//! scanned sets, draw as before.
//!
//! With the survey's canopy map (eo-props `GroundMap::canopy`) each class's
//! tone follows the aerial over it: a cell's aerial colour over its class's
//! median aerial colour (per channel, `RATIO_MAX` at most) scales the class's
//! tone, filtered between cells, so a lawn is patchy where its photograph is,
//! a paved yard as stained, and two classes meet in a short blend of their
//! tones, not a line. Planting with vegetation over it (the yards' shrubs and
//! trees, eo-app yards.rs) is litter and bare soil near the camera, where the
//! plants stand over it, and keeps the planting tone past them.
//!
//! The maps reach the shader as one storage buffer, group 0 binding 31 of the
//! city's draw layout (`words`): a header (the map count), per map eight
//! words (x0, z0, 1 / cell as f32 bits on the drawn plan, cell, nx, nz, the
//! first data word, the first canopy word or 0), then the classes, eight 4-bit
//! cells a word, then the canopy, a word a cell (the vegetation height in
//! 0.1 m, then the colour ratio per channel in 1 / 255 of `RATIO_MAX`).
//! Without the switch the buffer is the header alone and `YD_ON` is false:
//! the shader takes the previous path untouched.

use wgpu::util::DeviceExt;

/// Words before the first map's header.
pub const HEAD_WORDS: usize = 4;
/// Words of a map's header.
pub const MAP_WORDS: usize = 8;
/// Cells a data word holds (4 bits each).
pub const CELLS_PER_WORD: usize = 8;
/// The largest colour ratio a canopy word holds (a cell's aerial colour over its class's median).
pub const RATIO_MAX: f32 = 2.5;

/// Each class's median aerial colour (linear) over the cells that have one: the ratios' reference.
pub fn class_medians(g: &eo_props::yards::GroundMap) -> [[f32; 3]; 5] {
    let mut ch: Vec<[Vec<f32>; 3]> = (0..5).map(|_| Default::default()).collect();
    for k in 0..g.classes.len() {
        let c = g.classes[k] as usize;
        if c == 0 || c > 4 {
            continue;
        }
        if let Some(col) = g.colour(k) {
            for q in 0..3 {
                ch[c][q].push(col[q]);
            }
        }
    }
    let mut out = [[1.0f32; 3]; 5];
    for c in 1..5 {
        for q in 0..3 {
            let v = &mut ch[c][q];
            if !v.is_empty() {
                v.sort_by(|a, b| a.total_cmp(b));
                out[c][q] = v[v.len() / 2].max(1e-4);
            }
        }
    }
    out
}

/// The buffer's words for these maps (see the module comment).
pub fn words(maps: &[eo_props::yards::GroundMap]) -> Vec<u32> {
    let mut w = vec![0u32; HEAD_WORDS + MAP_WORDS * maps.len()];
    w[0] = maps.len() as u32;
    for (m, g) in maps.iter().enumerate() {
        let h = HEAD_WORDS + m * MAP_WORDS;
        let at = w.len() as u32;
        w[h] = (g.x0 as f32).to_bits();
        w[h + 1] = (g.z0 as f32).to_bits();
        w[h + 2] = ((1.0 / g.cell) as f32).to_bits();
        w[h + 3] = (g.cell as f32).to_bits();
        w[h + 4] = g.nx;
        w[h + 5] = g.nz;
        w[h + 6] = at;
        let mut data = vec![0u32; g.classes.len().div_ceil(CELLS_PER_WORD)];
        for (k, &c) in g.classes.iter().enumerate() {
            data[k / CELLS_PER_WORD] |= ((c & 15) as u32) << ((k % CELLS_PER_WORD) * 4);
        }
        w.extend(data);
        if g.canopy.len() == g.classes.len() {
            w[h + 7] = w.len() as u32;
            let med = class_medians(g);
            let q = |v: f32| ((v / RATIO_MAX).clamp(0.0, 1.0) * 255.0).round() as u32;
            // the height word is the tallest of the cell and its eight neighbours: the
            // litter under the plants (yards.wgsl) takes in a lone low cell among them.
            let (nx, nz) = (g.nx as i64, g.nz as i64);
            let tall = |k: usize| {
                let (i, j) = ((k as i64) % nx, (k as i64) / nx);
                let mut t = 0u8;
                for dj in -1..=1 {
                    for di in -1..=1 {
                        let (a, b) = (i + di, j + dj);
                        if a >= 0 && b >= 0 && a < nx && b < nz {
                            t = t.max(g.canopy[(b * nx + a) as usize][0]);
                        }
                    }
                }
                t
            };
            for (k, cell) in g.canopy.iter().enumerate() {
                let c = (g.classes[k] as usize).min(4);
                let r = match (c, g.colour(k)) {
                    (1..=4, Some(col)) => [0, 1, 2].map(|i| col[i] / med[c][i]),
                    _ => [1.0; 3],
                };
                let _ = cell;
                w.push(tall(k) as u32 | (q(r[0]) << 8) | (q(r[1]) << 16) | (q(r[2]) << 24));
            }
        }
    }
    w
}

/// Whether the shader draws the yards' ground: the switch, and a map read.
pub fn on() -> bool {
    !eo_props::yards::grounds().is_empty()
}

/// Whether a map carries its canopy (the aerial tones, the edge blend, the
/// paving's scans and the litter under the plants; `--no-yard-plants` off).
pub fn canopy() -> bool {
    eo_props::yards::grounds().iter().any(|g| g.canopy.len() == g.classes.len())
}

/// Where the litter under the yards' plants gives way to the planting's tone
/// (m): past 35-80 m a shrub's coarse levels are open billboards, and the
/// ground seen through them is the canopy's shade, not the litter (which no
/// one sees under a shrub from that far).
pub fn plants_m() -> [f32; 2] {
    [35.0, 80.0]
}

/// The ground under the yards' shrubs and trees near the camera (sRGB): leaf
/// litter on soil, the dark brown of a mulched bed (INFERRED; between the
/// forest-floor scan's mean and the park-dirt scan's), held to a saturation of
/// 0.2: under the play look's camera (its tone and print warm and saturate)
/// the first 0.33 drew an orange earth at 0.55, where photographs of the
/// city's garden beds in sun read 0.3-0.4.
pub const LITTER: [u8; 3] = [70, 63, 56];

/// The classes' tones (linear): lawn, planting, paving, deck.
pub fn tones() -> [[f32; 3]; 4] {
    let lin = |c: [u8; 3]| c.map(|v| eo_data::roof_colours::srgb_to_linear(v as f32 / 255.0));
    let deck = crate::props_yards::fence_finish().map(|f| f.0).unwrap_or(crate::ground2::CONCRETE);
    [lin(crate::ground2::YARD), lin(crate::ground2::CANOPY), lin(crate::ground2::CONCRETE), lin(deck)]
}

/// yards.wgsl with its switch (`YD_ON`) and tones (`YD_TONES`), for the city
/// shader. Without the yards (`--yard-detail` off, or no map read) a `yd_ground`
/// that leaves the ground to the terrain, and no binding 31 in the source, so the
/// city's shaders keep the code they had before.
pub fn wgsl() -> String {
    if !on() {
        return YD_STUB_WGSL.to_string();
    }
    let t = tones().map(|c| format!("vec3<f32>({:.6}, {:.6}, {:.6})", c[0], c[1], c[2])).join(", ");
    let l = LITTER.map(|v| eo_data::roof_colours::srgb_to_linear(v as f32 / 255.0));
    format!(
        "\n// generated by yards.rs wgsl\nconst YD_ON: bool = {};\nconst YD_CANOPY: bool = {};\nconst YD_TONES: array<vec3<f32>, 4> = array<vec3<f32>, 4>({t});\nconst YD_LITTER: vec3<f32> = vec3<f32>({:.6}, {:.6}, {:.6});\nconst YD_RATIO_MAX: f32 = {RATIO_MAX:.3};\nconst YD_PLANTS_M: vec2<f32> = vec2<f32>({:.1}, {:.1});\n{}",
        on(),
        canopy(),
        l[0],
        l[1],
        l[2],
        plants_m()[0],
        plants_m()[1],
        include_str!("yards.wgsl")
    )
}

/// `yd_ground` without the yards: the terrain draws its own ground.
const YD_STUB_WGSL: &str = "\n// generated by yards.rs wgsl (no yards)\nfn yd_ground(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) -> bool { return false; }\n";

/// The maps on the GPU (group 0 binding 31 of the city's draw layout).
pub struct YardGroundGpu {
    pub buf: wgpu::Buffer,
    pub bytes: u64,
}

impl YardGroundGpu {
    pub fn new(d: &wgpu::Device) -> Self {
        let maps = eo_props::yards::grounds();
        let w = words(maps);
        if !maps.is_empty() {
            eprintln!("yards: ground classes on the GPU ({} maps, {} KB)", maps.len(), w.len() * 4 / 1024);
        }
        let buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("yard-ground"), contents: bytemuck::cast_slice(&w), usage: wgpu::BufferUsages::STORAGE });
        YardGroundGpu { buf, bytes: w.len() as u64 * 4 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_pack_the_classes_as_the_shader_reads_them() {
        let g = eo_props::yards::GroundMap { x0: -10.0, z0: 5.0, cell: 0.625, nx: 3, nz: 4, classes: (0..12).map(|k| (k % 5) as u8).collect(), canopy: vec![] };
        let w = words(std::slice::from_ref(&g));
        assert_eq!(w[0], 1);
        let h = HEAD_WORDS;
        assert_eq!(f32::from_bits(w[h]), -10.0);
        assert_eq!(f32::from_bits(w[h + 1]), 5.0);
        assert!((f32::from_bits(w[h + 2]) - 1.6).abs() < 1e-6);
        assert_eq!((w[h + 4], w[h + 5]), (3, 4));
        let off = w[h + 6] as usize;
        assert_eq!(off, HEAD_WORDS + MAP_WORDS);
        // yards.wgsl `yd_class`: word k / 8, bits (k % 8) * 4.
        for k in 0..12usize {
            assert_eq!((w[off + k / 8] >> ((k % 8) * 4)) & 15, (k % 5) as u32);
        }
        assert_eq!(words(&[]), vec![0, 0, 0, 0], "off: the header alone");
        assert_eq!(w[h + 7], 0, "no canopy map: no canopy words");
        // with the canopy: a word a cell after the classes, the height then the ratios.
        let mut c = g.clone();
        c.canopy = (0..12).map(|k| [k as u8 * 5, 120, 120, 120]).collect();
        let w = words(std::slice::from_ref(&c));
        let at = w[h + 7] as usize;
        assert_eq!(at, off + 2, "after the two class words");
        assert_eq!(w.len(), at + 12);
        for k in 0..12usize {
            // the height is the tallest of the cell and its neighbours (heights rise with k).
            let (i, j) = (k % 3, k / 3);
            let tallest = ((j + 1).min(3) * 3 + (i + 1).min(2)) as u32 * 5;
            assert_eq!(w[at + k] & 255, tallest, "cell {k}");
            if c.classes[k] != 0 {
                // one colour over every class: each cell is its class's median, ratio 1.
                let r = ((w[at + k] >> 8) & 255) as f32 / 255.0 * RATIO_MAX;
                assert!((r - 1.0).abs() < 0.01, "{r}");
            }
        }
    }

    #[test]
    fn the_shader_reads_what_words_writes() {
        let s = include_str!("yards.wgsl");
        assert!(s.contains(&format!("const YD_HEAD: u32 = {}u;", HEAD_WORDS)));
        assert!(s.contains(&format!("const YD_MAP: u32 = {}u;", MAP_WORDS)));
        assert!(s.contains("@group(0) @binding(31)"));
        if !on() {
            assert!(!wgsl().contains("@binding(31)"), "without the yards the city's source has no binding 31");
        }
        assert!(include_str!("draw.wgsl").contains("yd_ground(&s, in.world, n, dpx, dpy)"), "draw.wgsl's terrain branch asks yd_ground first");
        let lib = include_str!("lib.rs");
        assert!(lib.contains("buf_entry(31, fs, storage(true))") && lib.contains("binding: 31"), "the draw layout and group carry binding 31");
    }
}
