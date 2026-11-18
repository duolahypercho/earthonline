//! the footprint box a building draws past its own levels (scene.rs, roles
//! KIT_BOX, FAR and ALWAYS), with its facade: the window rhythm, storeys,
//! glazing, paint and roof of its archetype (`eo_kit::far_facade`), drawn by
//! the city shader without a texture or a texel of atlas (far_box.wgsl).
//!
//! The mesh stays the box: the ring at the foot and at the top, the walls
//! between them and a flat cap. What a facade adds is carried in bytes the
//! box left unused:
//!
//! * every vertex's normal word holds, in its fourth byte, the window bay
//!   count reached at that corner going round the ring (an snorm8 from -127):
//!   a wall from corner a to corner b interpolates from bay `u_a` to `u_b`, so
//!   whole bays fit every wall and its windows meet its corners; a blind wall
//!   adds none. The first three bytes stay zero, so the shader keeps taking
//!   the face normal from screen derivatives as it did. A ring needing more
//!   than 254 bays gets a corner pair of its own per wall.
//! * the colour's alpha byte (255 on a plain box) names the facade class
//!   (`ALPHA_BASE` + class), the rgb the wall as the kit draws it.
//! * the object's two words a box never used (`far_first_index`,
//!   `far_index_count`: a terrain tile's coarse range) hold the rest, see
//!   [`pack`].
//!
//! Switch: `--no-far-facade` (`EO_FAR_FACADE=off`): the boxes exactly as
//! before (scene.rs builds the old mesh and colours).

use crate::scene::Vertex;
use eo_kit::far_facade::FarFacade;

/// Alpha byte of a facade box's vertices: this plus the facade class (0-7).
pub const ALPHA_BASE: u32 = 128;
/// Bay counts a ring can carry in one run (snorm8, -127 to 127).
pub const MAX_RING_BAYS: u32 = 254;

/// One building's far box facade, ready to mesh.
#[derive(Clone, Debug)]
pub struct FarBox {
    pub facade: FarFacade,
    /// Window bays per ring edge (0: blind).
    pub bays: Vec<u32>,
}

/// The facade of every building of the city (None: a plain box), worked out on every core.
pub fn facades(city: &eo_data::City) -> Vec<Option<FarBox>> {
    // The roof colours the facade reads are installed once a process (first call wins; the kit's thread may race here).
    if let Some(root) = eo_kit::repo_root() {
        eo_data::roof_colours::install(&root);
    }
    let bs = &city.buildings;
    let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).clamp(1, 16);
    let chunk = bs.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let hs: Vec<_> = bs.chunks(chunk).map(|c| s.spawn(move || c.iter().map(far_box_of).collect::<Vec<_>>())).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

/// One building's far box facade.
pub fn far_box_of(b: &eo_data::Building) -> Option<FarBox> {
    if b.ring.len() < 3 {
        return None;
    }
    let src = eo_kit::pipeline::SourceRecord {
        key: &b.id,
        points: &b.points,
        building: &b.tags.building,
        amenity: &b.tags.amenity,
        name: &b.tags.name,
        levels: b.tags.levels,
        height_raw: b.tags.height,
        area_raw: b.tags.area,
        height: b.height as f64,
    };
    let facade = eo_kit::far_facade::far_facade(&src)?;
    let bays = eo_kit::far_facade::edge_bays(&b.ring, &facade);
    Some(FarBox { facade, bays })
}

fn srgb_byte(c: f32) -> u32 {
    (eo_data::roof_colours::linear_to_srgb(c) * 255.0).round().clamp(0.0, 255.0) as u32
}

/// sRGB rgb565 (r in bits 0-4, g 5-10, b 11-15: the far layer's district roof word).
pub fn rgb565(c: [f32; 3]) -> u32 {
    let q = |v: f32, n: f32| ((eo_data::roof_colours::linear_to_srgb(v) * n).round().clamp(0.0, n)) as u32;
    q(c[0], 31.0) | (q(c[1], 63.0) << 5) | (q(c[2], 31.0) << 11)
}

/// The object's two facade words. `floor_above_base`: metres from the box's
/// foot (the object's min y) to the floor its storeys stand on.
///
/// ```text
/// w0: roof rgb565 (0-15) | floor over the foot, 0.1 m (16-23) | ground storey, 2.0 m + 0.1 m (24-29) | glass kind (30-31)
/// w1: storey, 2.0 m + 0.02 m (0-7) | top band, 0.1 m (8-13) | window width share, 1/16 (14-17)
///     | window height share, 1/16 (18-21) | sill share, 1/16 (22-25) | lights - 1 (26-27) | trim (28-30) | shop ground (31)
/// ```
pub fn pack(f: &FarFacade, floor_above_base: f32) -> (u32, u32) {
    let q = |v: f32, lo: f32, step: f32, max: u32| (((v - lo) / step).round().max(0.0) as u32).min(max);
    let share = |v: f32| ((v * 16.0 - 0.5).round().max(0.0) as u32).min(15);
    let w0 = rgb565(f.roof) | (q(floor_above_base, 0.0, 0.1, 255) << 16) | (q(f.ground_m, 2.0, 0.1, 63) << 24) | ((f.glass as u32 & 3) << 30);
    let w1 = q(f.storey_m, 2.0, 0.02, 255)
        | (q(f.top_m, 0.0, 0.1, 63) << 8)
        | (share(f.window_w) << 14)
        | (share(f.window_h) << 18)
        | (((f.sill * 16.0).round().max(0.0) as u32).min(15) << 22)
        | (((f.lights.clamp(1, 4) - 1) as u32) << 26)
        | (((f.trim & 7) as u32) << 28)
        | ((f.shop_ground as u32) << 31);
    (w0, w1)
}

fn bay_word(u: i32) -> u32 {
    ((u.clamp(-127, 127) as i8 as u8) as u32) << 24
}

/// The box of `ring` (open) from `base` to `top` with its facade: vertices
/// and local indices (walls wound as scene.rs winds the plain box, then the cap).
pub fn mesh(ring: &[[f32; 2]], base: f32, top: f32, fb: &FarBox, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, ear: &mut earcut::Earcut<f64>, tri: &mut Vec<u32>) {
    let n = ring.len();
    let f = &fb.facade;
    let rgb = srgb_byte(f.wall[0]) | (srgb_byte(f.wall[1]) << 8) | (srgb_byte(f.wall[2]) << 16);
    let color = rgb | ((ALPHA_BASE + f.class as u32) << 24);
    let total: u32 = fb.bays.iter().sum();
    let vert = |p: [f32; 2], y: f32, u: i32| Vertex { pos: [p[0], y, p[1]], normal: bay_word(u), color };
    // The cap's corners: the top ring's (the cap reads no bay byte), or corners of its own.
    let cap0: u32;
    if total <= MAX_RING_BAYS {
        // One run round the ring: corner k at bay -127 + the bays before it, the first corner repeated at the end.
        let mut u = vec![-127i32; n + 1];
        for k in 0..n {
            u[k + 1] = u[k] + fb.bays[k] as i32;
        }
        for k in 0..=n {
            verts.push(vert(ring[k % n], base, u[k]));
        }
        for k in 0..=n {
            verts.push(vert(ring[k % n], top, u[k]));
        }
        let t0 = (n + 1) as u32;
        for a in 0..n as u32 {
            let (ba, bc, ta, tc) = (a, a + 1, t0 + a, t0 + a + 1);
            idx.extend_from_slice(&[ba, bc, tc, ba, tc, ta]);
        }
        cap0 = t0;
    } else {
        // A corner pair per wall, each wall its own run.
        for a in 0..n {
            let c = (a + 1) % n;
            let ub = -127 + fb.bays[a].min(MAX_RING_BAYS) as i32;
            let o = verts.len() as u32;
            verts.push(vert(ring[a], base, -127));
            verts.push(vert(ring[c], base, ub));
            verts.push(vert(ring[c], top, ub));
            verts.push(vert(ring[a], top, -127));
            idx.extend_from_slice(&[o, o + 1, o + 2, o, o + 2, o + 3]);
        }
        cap0 = verts.len() as u32;
        for p in ring {
            verts.push(vert(*p, top, 0));
        }
    }
    // The cap (its colour is the roof word's).
    let c0 = cap0;
    let flat: Vec<[f64; 2]> = ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
    ear.earcut(flat.iter().copied(), &[] as &[u32], tri);
    for t in tri.iter() {
        idx.push(*t + c0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eo_kit::far_facade::{FacadeClass, GlassKind};

    fn facade() -> FarFacade {
        FarFacade {
            archetype: "stucco-row",
            class: FacadeClass::Punched,
            glass: GlassKind::Domestic,
            wall: [0.6, 0.55, 0.5],
            roof: [0.25, 0.25, 0.26],
            trim: 0,
            ground_m: 3.1,
            storey_m: 2.95,
            top_m: 0.45,
            bay_m: 3.0,
            window_w: 0.4,
            window_h: 0.55,
            sill: 0.3,
            lights: 2,
            shop_ground: true,
            lot_axis: None,
        }
    }

    #[test]
    fn the_words_round_trip_within_their_steps() {
        let f = facade();
        let (w0, w1) = pack(&f, 1.3);
        assert_eq!((w0 >> 16) & 255, 13);
        assert!(((2.0 + ((w0 >> 24) & 63) as f32 * 0.1) - 3.1).abs() < 0.051);
        assert_eq!(w0 >> 30, GlassKind::Domestic as u32);
        assert!(((2.0 + (w1 & 255) as f32 * 0.02) - 2.95).abs() < 0.011);
        assert!((((w1 >> 8) & 63) as f32 * 0.1 - 0.45).abs() < 0.051);
        assert!(((((w1 >> 14) & 15) as f32 + 0.5) / 16.0 - 0.4).abs() <= 1.0 / 32.0);
        assert!(((((w1 >> 18) & 15) as f32 + 0.5) / 16.0 - 0.55).abs() <= 1.0 / 32.0);
        assert_eq!((w1 >> 26) & 3, 1);
        assert_eq!(w1 >> 31, 1);
    }

    #[test]
    fn a_ring_counts_its_bays_round_and_its_cap_closes() {
        let ring = [[0.0, 0.0], [9.0, 0.0], [9.0, 6.0], [0.0, 6.0]];
        let fb = FarBox { facade: facade(), bays: vec![3, 0, 3, 2] };
        let (mut v, mut i, mut ear, mut tri) = (vec![], vec![], earcut::Earcut::new(), vec![]);
        mesh(&ring, 10.0, 22.0, &fb, &mut v, &mut i, &mut ear, &mut tri);
        let u = |k: usize| (v[k].normal >> 24) as u8 as i8 as i32;
        assert_eq!((0..5).map(u).collect::<Vec<_>>(), vec![-127, -124, -124, -121, -119]);
        // Walls (4 x 2 triangles) and the cap (2 triangles), the plain box's count, on 2 x 5 corners.
        assert_eq!(i.len(), 3 * (8 + 2));
        assert_eq!(v.len(), 10);
        assert!(i[24..].iter().all(|&k| (5..9).contains(&k)), "the cap stands on the top ring");
        assert!(v.iter().all(|x| x.color >> 24 == ALPHA_BASE + FacadeClass::Punched as u32 && x.normal & 0x00ff_ffff == 0));
        // Bounds as the plain box's: the ring at the foot and the top.
        assert!(v.iter().all(|x| x.pos[1] == 10.0 || x.pos[1] == 22.0));
    }

    #[test]
    fn a_long_ring_gets_a_run_per_wall() {
        let ring = [[0.0, 0.0], [400.0, 0.0], [400.0, 300.0], [0.0, 300.0]];
        let fb = FarBox { facade: facade(), bays: vec![133, 100, 133, 100] };
        let (mut v, mut i, mut ear, mut tri) = (vec![], vec![], earcut::Earcut::new(), vec![]);
        mesh(&ring, 0.0, 9.0, &fb, &mut v, &mut i, &mut ear, &mut tri);
        assert_eq!(v.len(), 4 * 4 + 4);
        let u = |k: usize| (v[k].normal >> 24) as u8 as i8 as i32;
        assert_eq!((u(0), u(1)), (-127, 6));
    }
}
