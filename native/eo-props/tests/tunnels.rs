//! The tunnel bores' lights (stage A7, `--tunnels`) through the props' lamp
//! path: on the real city, every light of a bore's tile is in that tile's
//! `TilePlacement::lights`, a district luminaire under the crown aimed down
//! at the roadway, and none without `--tunnels`.

use std::sync::Arc;

use eo_props::PropsWorld;
use eo_structures::{ChainKind, StructureOptions, StructureWorld};

#[test]
fn the_bores_lights_are_district_luminaires_in_their_tiles_light_lists() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let street = eo_props::load_street_world(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let std = eo_props::Standards::load(&ca).unwrap();
    let build = |tunnels: bool| Arc::new(StructureWorld::build(&street, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions { tunnels }).unwrap());
    let (on, off) = (build(true), build(false));
    let world = |s: Arc<StructureWorld>| {
        let w = PropsWorld::new(street.clone(), Arc::new(eo_props::JsProps::load().unwrap()), std.clone()).with_structures(Some(s));
        w.install_furniture(&city.buildings).unwrap();
        eo_props::overhead::install(&w, eo_props::overhead::DropBuilding::from_city(&city.buildings)).unwrap();
        w
    };
    let (w_on, w_off) = (world(on.clone()), world(off));
    let t = on.chains().iter().find(|c| c.kind == ChainKind::Tunnel && c.is_built() && c.name().contains("Broadway")).expect("the Broadway Tunnel");
    let mid = t.frame(t.length() / 2.0);
    let key = eo_street::tile_key(mid.x, mid.z);
    let bore = on.tunnel_lights(key);
    assert!(!bore.is_empty(), "lights in the bore's middle tile");
    let lamps = w_on.tunnel_lights(key);
    assert_eq!(lamps.len(), bore.len());
    let placed = w_on.plan_tile(key).lights;
    for (l, b) in lamps.iter().zip(&bore) {
        assert_eq!(l.pos, b.pos);
        assert!((l.base[1] - b.floor).abs() < 1e-9 && l.pos[1] > b.floor + 4.0, "{l:?}");
        assert!(l.intensity > 0.0 && l.range_m > 0.0 && l.angle > 0.0);
        assert!(placed.iter().any(|p| p.pos == l.pos), "the bore light at {:?} is in the tile's list", l.pos);
    }
    // without --tunnels: none, and the tile's list is the street's alone.
    assert!(w_off.tunnel_lights(key).is_empty());
    assert_eq!(w_off.plan_tile(key).lights.len() + lamps.len(), placed.len());
    println!("Broadway Tunnel (chain {}): {} lights in tile {key:?}, {} in the city's bores", t.id, lamps.len(), on.all_tunnel_lights().len());
}
