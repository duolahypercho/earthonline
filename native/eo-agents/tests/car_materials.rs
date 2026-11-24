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
            assert_eq!((a.pos, a.normal, a.tag), (c.pos, c.normal, c.tag), "{id}: a vertex moved");
        }
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
