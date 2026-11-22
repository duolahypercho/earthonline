//! Side views of the coastal ez-tree species (and any other species) at every
//! tier, rasterised on the CPU with the vegetation atlas's colour and alpha
//! test, for checking a form without the game:
//!
//!   cargo run --release -p eo-props --example coastal_preview -- <atlas.png> <out.png> [species...]
//!
//! One row per species, one column per tier (near, coarse, distant), each
//! cell scaled to the species' height; a 1 m grid line at the ground.

use eo_props::eztree::{COASTAL_SHRUBS, COASTAL_TREES, EzMesh, build_ez_tree};

const CELL: usize = 360;

fn load_atlas(path: &str) -> (usize, usize, Vec<u8>) {
    let f = std::fs::File::open(path).expect("atlas");
    let mut d = png::Decoder::new(std::io::BufReader::new(f));
    d.set_transformations(png::Transformations::EXPAND);
    let mut r = d.read_info().expect("png");
    let mut buf = vec![0u8; r.output_buffer_size().unwrap_or(0)];
    let info = r.next_frame(&mut buf).expect("frame");
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => panic!("atlas colour type"),
    };
    (w, h, rgba)
}

fn lin(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn srgb(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let s = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// Draw `m` into the cell at (ox, oy), `scale` px per metre, ground at the cell's bottom.
fn draw(img: &mut [u8], zb: &mut [f32], width: usize, ox: usize, oy: usize, scale: f32, m: &EzMesh, atlas: &(usize, usize, Vec<u8>), yaw: f32) {
    let (aw, ah, a) = atlas;
    let (s, c) = yaw.sin_cos();
    let proj = |p: [f32; 3]| -> [f32; 3] {
        let x = p[0] * c + p[2] * s;
        let z = -p[0] * s + p[2] * c;
        [ox as f32 + CELL as f32 / 2.0 + x * scale, oy as f32 + CELL as f32 - 6.0 - p[1] * scale, z]
    };
    let light = {
        let l = [-0.5f32, 0.75, 0.45];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    for t in m.indices.chunks_exact(3) {
        let v = [t[0] as usize, t[1] as usize, t[2] as usize];
        let p = v.map(|k| proj(m.positions[k]));
        let (minx, maxx) = (p.iter().map(|q| q[0]).fold(f32::MAX, f32::min).floor().max(ox as f32) as i64, p.iter().map(|q| q[0]).fold(f32::MIN, f32::max).ceil().min((ox + CELL - 1) as f32) as i64);
        let (miny, maxy) = (p.iter().map(|q| q[1]).fold(f32::MAX, f32::min).floor().max(oy as f32) as i64, p.iter().map(|q| q[1]).fold(f32::MIN, f32::max).ceil().min((oy + CELL - 1) as f32) as i64);
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-9 {
            continue;
        }
        for y in miny..=maxy {
            for x in minx..=maxx {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p[1][0] - px) * (p[2][1] - py) - (p[2][0] - px) * (p[1][1] - py)) / area;
                let w1 = ((p[2][0] - px) * (p[0][1] - py) - (p[0][0] - px) * (p[2][1] - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * p[0][2] + w1 * p[1][2] + w2 * p[2][2];
                let i = y as usize * width + x as usize;
                if z >= zb[i] {
                    continue;
                }
                let uv = [0, 1].map(|k| w0 * m.uvs[v[0]][k] + w1 * m.uvs[v[1]][k] + w2 * m.uvs[v[2]][k]);
                let tx = ((uv[0].clamp(0.0, 1.0)) * (*aw as f32 - 1.0)) as usize;
                let ty = ((1.0 - uv[1].clamp(0.0, 1.0)) * (*ah as f32 - 1.0)) as usize;
                let o = (ty * aw + tx) * 4;
                let leaf = m.part[v[0]] != 0;
                if leaf && a[o + 3] < 107 {
                    continue;
                }
                let n = [0, 1, 2].map(|k| w0 * m.normals[v[0]][k] + w1 * m.normals[v[1]][k] + w2 * m.normals[v[2]][k]);
                let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                let d = ((n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / nl).abs();
                let col = [0, 1, 2].map(|k| w0 * m.colors[v[0]][k] + w1 * m.colors[v[1]][k] + w2 * m.colors[v[2]][k]);
                zb[i] = z;
                for k in 0..3 {
                    img[i * 3 + k] = srgb(col[k] * lin(a[o + k]) * (0.3 + 0.8 * d));
                }
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let atlas = load_atlas(&args[1]);
    let out = &args[2];
    let mut ids: Vec<String> = args[3..].to_vec();
    if ids.is_empty() {
        ids = COASTAL_TREES.iter().chain(COASTAL_SHRUBS.iter()).map(|s| s.to_string()).collect();
    }
    let tiers = ["near", "coarse", "distant"];
    let (w, h) = (CELL * (tiers.len() + 1), CELL * ids.len());
    let mut img = vec![200u8; w * h * 3];
    let mut zb = vec![f32::MAX; w * h];
    for (r, id) in ids.iter().enumerate() {
        for (c, t) in tiers.iter().enumerate() {
            let m = match build_ez_tree(id, t, 0) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("{id} {t}: {e:#}");
                    continue;
                }
            };
            let span = (m.user.height_m as f32).max(2.0 * m.user.half_crown_m as f32) * 1.08;
            let scale = (CELL as f32 - 12.0) / span;
            let leaf: Vec<[f32; 3]> = (0..m.positions.len()).filter(|&k| m.part[k] != 0).map(|k| m.colors[k]).collect();
            let mean = [0, 1, 2].map(|c| leaf.iter().map(|v| v[c]).sum::<f32>() / leaf.len().max(1) as f32);
            eprintln!("{id:<22} {t:<8} mean leaf vertex colour (linear) {mean:.3?}");
            eprintln!("{id:<22} {t:<8} {:>6} tris  height {:.1} m  half crown {:.1} m  trunk r {:.2} m  cover {:.2} leaf x{:.2}", m.triangles(), m.user.height_m, m.user.half_crown_m, m.user.trunk_radius_m, m.user.crown_cover, m.user.crown_leaf_scale);
            draw(&mut img, &mut zb, w, c * CELL, r * CELL, scale, &m, &atlas, 0.0);
            if c == 0 {
                draw(&mut img, &mut zb, w, 3 * CELL, r * CELL, scale, &m, &atlas, 1.3);
            }
            // a 1 m tick row at the ground
            for x in 0..CELL {
                let y = r * CELL + CELL - 6;
                if ((x as f32 - CELL as f32 / 2.0) / scale).rem_euclid(1.0) < 1.0 / scale.max(1.0) * 1.5 {
                    let i = y * w + c * CELL + x;
                    img[i * 3..i * 3 + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }
    }
    let f = std::fs::File::create(out).expect("out");
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&img).unwrap();
}
