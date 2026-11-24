//! The cars' materials (`--car-materials`): cooked with the option, every
//! shipped body's LOD 0 is the body cooked without it (the same vertices,
//! indices and material factors) plus each material's role in its flags and
//! the licence plates' own coordinates; without it nothing of either is
//! written, so the library and its cache key are as before. Bodies whose
//! files are not in this checkout are skipped.

use eo_agents::catalog::{AgentsJs, read_verified};
use eo_agents::vehicles::{self as v, LampColours, cook_body, cook_body_with};

#[test]
fn roles_and_plate_coordinates_come_with_the_option_only() {
    let paths = eo_data::DataPaths::discover().expect("repository root");
    let js = AgentsJs::load().expect("agents JS export");
    let lamps = LampColours::from_js(&js);
    let mut bodies = 0;
    for (id, b) in &js.vehicles {
        let Some(l) = b.lods.first() else { continue };
        let Ok(bytes) = read_verified(&paths.repo_root, &l.asset()) else { continue };
        bodies += 1;
        let off = cook_body(&bytes, &lamps).expect("cook");
        let on = cook_body_with(&bytes, &lamps, true).expect("cook with the cars' materials");
        // Without the option: no role, no texture coordinates (the bodies carry no textures).
        assert!(off.materials.iter().all(|m| m.flags & v::M_ROLE_MASK == 0), "{id}: a role without the option");
        assert!(off.vertices.iter().all(|x| x.uv == [0, 0]), "{id}: coordinates without the option");
        // With it: the same body.
        assert_eq!((on.opaque.len(), on.glass.len(), on.vertices.len()), (off.opaque.len(), off.glass.len(), off.vertices.len()), "{id}");
        assert_eq!(on.opaque, off.opaque, "{id}");
        assert_eq!(on.glass, off.glass, "{id}");
        for (a, c) in on.vertices.iter().zip(&off.vertices) {
            // the 8-bit normal as before; the spare byte (0 without the option) refines it.
            assert_eq!((a.pos, &a.normal[..3], a.tag), (c.pos, &c.normal[..3], c.tag), "{id}: a vertex moved");
            assert_eq!(c.normal[3], 0, "{id}: a refinement without the option");
            assert!(a.normal[3] != -128 && a.normal[3] != 0, "{id}: the refinement's code is never -128 nor 0 (the people's)");
        }
        // The cars' occluder boxes (`--car-contact`) stand inside the body's bounds, in order.
        let s = v::contact_shape(&on);
        assert!(s.clearance > 0.05 && s.clearance < s.belt && s.belt < s.top && s.top <= on.max[1] + 1e-3, "{id}: heights {s:?}");
        assert!(s.lower_half_w > 0.5 && s.lower_half_w <= on.max[0].abs().max(on.min[0].abs()) + 1e-3 && s.upper_half_w <= s.lower_half_w, "{id}: widths {s:?}");
        assert!(s.uz0 >= s.z0 - 0.1 && s.uz0 < s.uz1 && s.uz1 <= s.z1 + 0.1 && s.z0 >= on.min[2] - 1e-3 && s.z1 <= on.max[2] + 1e-3, "{id}: lengths {s:?}");
        assert!(s.tyre_half_w > 0.05 && s.tyre_half_w <= 0.25, "{id}: tyres {s:?}"); // (a truck's rear duals are the widest)
        for ((a, c), name) in on.materials.iter().zip(&off.materials).zip(&on.material_names) {
            assert_eq!(a.flags & !v::M_ROLE_MASK, c.flags, "{id} {name}: only the role is added");
            assert_eq!((a.base, a.emissive, a.params), (c.base, c.emissive, c.params), "{id} {name}");
            assert_eq!(v::role_of_flags(a.flags), v::role_of(name), "{id} {name}");
        }
        let roles: Vec<u32> = on.materials.iter().map(|m| v::role_of_flags(m.flags)).collect();
        for want in [v::ROLE_GLASS, v::ROLE_TYRE, v::ROLE_PLATE] {
            assert!(roles.contains(&want), "{id}: no material of role {want} ({:?})", on.material_names);
        }
        // Each plate's coordinates span it: 0..1 across and up, front and rear.
        let plate: Vec<_> = on.vertices.iter().filter(|x| v::role_of_flags(on.materials[(x.tag & 0xffff) as usize].flags) == v::ROLE_PLATE).collect();
        assert!(!plate.is_empty(), "{id}: no plate vertex");
        let (lo, hi) = plate.iter().fold(([u16::MAX; 2], [0u16; 2]), |(lo, hi), x| ([lo[0].min(x.uv[0]), lo[1].min(x.uv[1])], [hi[0].max(x.uv[0]), hi[1].max(x.uv[1])]));
        assert_eq!((lo, hi), ([0, 0], [65535, 65535]), "{id}: the plates' coordinates span 0..1");
        // Read left to right: u grows to the viewer's right, +x in front, -x behind.
        for nose in [true, false] {
            let side: Vec<_> = plate.iter().filter(|x| (x.pos[2] > 0.0) == nose).collect();
            assert!(!side.is_empty(), "{id}: no {} plate", if nose { "front" } else { "rear" });
            let left = side.iter().min_by(|a, b| a.pos[0].total_cmp(&b.pos[0])).unwrap();
            let right = side.iter().max_by(|a, b| a.pos[0].total_cmp(&b.pos[0])).unwrap();
            assert_eq!(right.uv[0] > left.uv[0], nose, "{id}: the {} plate reads the wrong way", if nose { "front" } else { "rear" });
        }
    }
    assert!(bodies > 0, "no traffic body's LOD 0 is in this checkout");
}

/// (`--car-materials`) The refined normal (`pack_normal_fine`) keeps
/// `pack_normal`'s bytes and reads back about ten times closer to the
/// source than the 8-bit normal (whose error a lacquer's reflection doubles
/// into ripples), every way round: at worst ~0.04 degrees against ~0.37; a
/// zero spare byte reads as the 8-bit normal (the people's).
#[test]
fn refined_normals_read_back_ten_times_closer() {
    use glam::Vec3;
    let (mut worst8, mut worst_fine) = (0f32, 0f32);
    let mut seed = 12345u64;
    let mut rnd = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    };
    for _ in 0..20000 {
        let n = Vec3::new(rnd(), rnd(), rnd());
        if n.length() < 0.1 {
            continue;
        }
        let n = n.normalize();
        let p8 = v::pack_normal(n);
        let pf = v::pack_normal_fine(n);
        assert_eq!(&p8[..3], &pf[..3]);
        let n8 = Vec3::new(p8[0] as f32, p8[1] as f32, p8[2] as f32).normalize();
        let nf = v::unpack_normal_fine(pf);
        worst8 = worst8.max(n8.dot(n).clamp(-1.0, 1.0).acos().to_degrees());
        worst_fine = worst_fine.max(nf.dot(n).clamp(-1.0, 1.0).acos().to_degrees());
    }
    assert!(worst8 > 0.2, "the 8-bit normal's worst error {worst8} deg");
    assert!(worst_fine < worst8 / 8.0 && worst_fine < 0.05, "refined {worst_fine} deg against 8-bit {worst8} deg");
    // the people's normals (no refinement) read as the 8-bit normal.
    for n in [Vec3::X, Vec3::new(0.3, -0.8, 0.52).normalize(), Vec3::new(-0.577, 0.577, -0.577)] {
        let p = v::pack_normal(n);
        let back = v::unpack_normal_fine(p);
        let n8 = Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32).normalize();
        assert!(back.dot(n8) > 0.99999, "{n}: {back} vs {n8}");
    }
}

/// (`--car-lamps`) Every shipped body with rear lamps has them found on its flank, behind its
/// rear axle, inside its bounds, at a tail lamp's height; the packing reads back to a centimetre;
/// a body without them (the cooked box) has none.
#[test]
fn rear_lamps_sit_at_the_tail() {
    let paths = eo_data::DataPaths::discover().expect("repository root");
    let js = AgentsJs::load().expect("agents JS export");
    let lamps = LampColours::from_js(&js);
    let mut found = 0;
    for (id, b) in &js.vehicles {
        let Some(l) = b.lods.first() else { continue };
        let Ok(bytes) = read_verified(&paths.repo_root, &l.asset()) else { continue };
        let body = cook_body_with(&bytes, &lamps, true).expect("cook");
        let mid = (body.min[2] + body.max[2]) * 0.5;
        let idx = body.opaque.iter().chain(body.glass.iter());
        let parts = idx.map(|&i| {
            let v = &body.vertices[i as usize];
            (v, body.materials[(v.tag & 0xffff) as usize].flags)
        });
        let Some(p) = v::rear_lamp(parts, mid) else { continue };
        found += 1;
        let rear_axle = body.pivots[2].map(|q| q[2]).unwrap_or(mid);
        assert!(p[0] > 0.2 && p[0] <= body.max[0].abs().max(body.min[0].abs()) + 1e-3, "{id}: across {p:?}");
        assert!(p[1] > 0.3 && p[1] < body.max[1], "{id}: height {p:?}");
        assert!(p[2] < rear_axle && p[2] >= body.min[2] - 1e-3, "{id}: along {p:?} (rear axle {rear_axle})");
        let back = v::rear_lamp_from_words(v::rear_lamp_words(p)).expect("a lamp");
        assert!((0..3).all(|k| (back[k] - p[k]).abs() <= 0.006), "{id}: {p:?} read back {back:?}");
    }
    assert!(found > 0 || js.vehicles.is_empty(), "no shipped body in this checkout has rear lamps");
    let boxed = v::box_body([-0.9, 0.0, -2.2], [0.9, 1.5, 2.2]);
    let parts = boxed.opaque.iter().map(|&i| {
        let v = &boxed.vertices[i as usize];
        (v, boxed.materials[(v.tag & 0xffff) as usize].flags)
    });
    assert_eq!(v::rear_lamp(parts, 0.0), None, "the box has no lamps");
    assert_eq!(v::rear_lamp_from_words([0.0, 0.0]), None);
}
