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

/// (`--car-panels`) The panels' filter: on a cylinder's panel (a door's crown, 1.2 m radius) of a
/// few centimetres' tessellation whose normals wander by up to three degrees, the filtered normals
/// wander a quarter as far from the surface's; across a box's edge (a crease past the
/// threshold) each face keeps its own normal; an unselected vertex keeps its normal.
#[test]
fn the_panels_filter_keeps_the_surface_and_its_creases() {
    use glam::Vec3;
    let radius = 1.2f32;
    let (mut pos, mut nrm, mut truth) = (vec![], vec![], vec![]);
    let mut seed = 7u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    };
    for i in 0..40 {
        for j in 0..30 {
            let a = (i as f32 - 20.0) * 0.03 / radius;
            let n = Vec3::new(a.sin(), 0.0, a.cos());
            let p = n * radius + Vec3::new(0.0, j as f32 * 0.03, 0.0);
            let wander = Vec3::new(rnd(), rnd(), rnd()) * 0.07; // up to ~2 degrees
            pos.push(p);
            nrm.push((n + wander - n * n.dot(wander)).normalize());
            truth.push(n);
        }
    }
    let area = vec![0.03f32 * 0.03; pos.len()];
    let before: f32 = nrm.iter().zip(&truth).map(|(a, b)| a.dot(*b).clamp(-1.0, 1.0).acos().to_degrees()).fold(0.0, f32::max);
    let mut sel = vec![true; pos.len()];
    sel[0] = false;
    let kept = nrm[0];
    v::filter_panel_normals(&pos, &mut nrm, &area, &sel);
    assert_eq!(nrm[0], kept, "an unselected vertex keeps its normal");
    // away from the patch's edge (a one-sided mean there leans inward by the curvature over the radius)
    let mut worst = 0.0f32;
    for i in 4..36 {
        for j in 4..26 {
            let k = i * 30 + j;
            worst = worst.max(nrm[k].dot(truth[k]).clamp(-1.0, 1.0).acos().to_degrees());
        }
    }
    assert!(before > 1.5 && worst < before * 0.25, "the wander: {before:.2} deg before, {worst:.3} after");
    // a box's edge: two faces at 90 degrees, each vertex on the edge twice (one normal a face)
    let (mut bp, mut bn) = (vec![], vec![]);
    for i in 0..10 {
        for j in 0..10 {
            bp.push(Vec3::new(i as f32 * 0.02, j as f32 * 0.02, 0.0));
            bn.push(Vec3::Z);
            bp.push(Vec3::new(0.0, j as f32 * 0.02, i as f32 * 0.02));
            bn.push(-Vec3::X);
        }
    }
    let ba = vec![0.0004f32; bp.len()];
    v::filter_panel_normals(&bp, &mut bn, &ba, &vec![true; bp.len()]);
    for (k, n) in bn.iter().enumerate() {
        let want = if k % 2 == 0 { Vec3::Z } else { -Vec3::X };
        assert!(n.dot(want) > 0.99999, "vertex {k}: {n} crossed the crease");
    }
}

/// (`--car-panels`) Cooked with the panels, a body is the body cooked without them (the same
/// vertices and indices) but for its paint's and liveries' normals, turned by at most a few degrees on average,
/// and each badge's emblem in a material of the badge role whose coordinates span its disc; without
/// the cars' materials the option does nothing.
#[test]
fn the_panels_cook_the_paint_and_the_badges_only() {
    let paths = eo_data::DataPaths::discover().expect("repository root");
    let js = AgentsJs::load().expect("agents JS export");
    let lamps = LampColours::from_js(&js);
    let mut badges = 0;
    for (id, b) in &js.vehicles {
        let Some(l) = b.lods.first() else { continue };
        let Ok(bytes) = read_verified(&paths.repo_root, &l.asset()) else { continue };
        let mats = cook_body_with(&bytes, &lamps, true).expect("cook");
        let panels = v::cook_body_opts(&bytes, &lamps, true, true).expect("cook with the panels");
        let plain = v::cook_body_opts(&bytes, &lamps, false, true).expect("cook without the materials");
        let off = cook_body(&bytes, &lamps).expect("cook");
        assert_eq!(plain.vertices.iter().map(|x| (x.pos, x.normal, x.uv, x.tag)).collect::<Vec<_>>(), off.vertices.iter().map(|x| (x.pos, x.normal, x.uv, x.tag)).collect::<Vec<_>>(), "{id}: the panels need the materials");
        assert_eq!((&panels.opaque, &panels.glass, panels.vertices.len()), (&mats.opaque, &mats.glass, mats.vertices.len()), "{id}");
        let (mut turned, mut paint) = (0.0f64, 0usize);
        for (a, c) in panels.vertices.iter().zip(&mats.vertices) {
            assert_eq!(a.pos, c.pos, "{id}: a vertex moved");
            let (ma, mc) = (&panels.materials[(a.tag & 0xffff) as usize], &mats.materials[(c.tag & 0xffff) as usize]);
            let is_paint = (mc.flags & eo_agents::M_PAINT != 0 || v::role_of_flags(mc.flags) == v::ROLE_LIVERY) && (c.tag >> 16) & 0xff == 0;
            if v::role_of_flags(ma.flags) == v::ROLE_BADGE {
                assert_eq!(v::role_of_flags(mc.flags), v::ROLE_CHROME, "{id}: a badge is chrome");
                continue;
            }
            // (a badge's material, new, may shift the later materials' local numbers: compare what they are)
            assert_eq!((a.uv, a.tag >> 16), (c.uv, c.tag >> 16), "{id}: only the badges' coordinates and material change");
            assert_eq!((ma.flags, ma.base, ma.emissive, ma.params), (mc.flags, mc.base, mc.emissive, mc.params), "{id}: a material changed");
            if is_paint {
                let (na, nc) = (v::unpack_normal_fine(a.normal), v::unpack_normal_fine(c.normal));
                turned += na.dot(nc).clamp(-1.0, 1.0).acos().to_degrees() as f64;
                paint += 1;
            } else {
                assert_eq!(a.normal, c.normal, "{id}: a normal off the paint changed");
            }
        }
        assert!(paint > 0 && turned / (paint as f64) < 3.0, "{id}: the paint's normals turned {:.2} deg on average", turned / paint.max(1) as f64);
        let badge: Vec<_> = panels.vertices.iter().filter(|x| v::role_of_flags(panels.materials[(x.tag & 0xffff) as usize].flags) == v::ROLE_BADGE).collect();
        if !badge.is_empty() {
            badges += 1;
            let hi = badge.iter().fold([0u16; 2], |h, x| [h[0].max(x.uv[0]), h[1].max(x.uv[1])]);
            let lo = badge.iter().fold([u16::MAX; 2], |l, x| [l[0].min(x.uv[0]), l[1].min(x.uv[1])]);
            assert!(lo == [0, 0] && hi == [65535, 65535], "{id}: a badge's coordinates span its disc ({lo:?}..{hi:?})");
        }
    }
    assert!(badges > 0 || js.vehicles.is_empty(), "no shipped body in this checkout has a badge");
}
