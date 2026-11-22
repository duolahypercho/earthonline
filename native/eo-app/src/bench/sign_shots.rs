//! the sign capture card. `EO_CAPTURE_SIGNS=1` with `--capture-props2`
//! replaces the props2 poses with poses chosen from the placement near the
//! SoMa oracle window (the same with and without the official sign art, so a
//! before / after pair is two runs of one card): a STOP plate close up, a
//! warning plate close up, a street-cleaning kerb plate close up, the printed
//! kerb picket (the overhead `pole-stack-printed` assembly) close up, and a
//! street view at play distance towards the STOP plate. Run the card again
//! with `--time-of-day` for the night pair.

use eo_props::{Model, PropsWorld};
use eo_render::camera::Camera;
use glam::Vec3;
use std::sync::Arc;

fn look_at(eye: Vec3, at: Vec3, far: f32, aspect: f32) -> Camera {
    let d = at - eye;
    let pitch = (d.y / Vec3::new(d.x, 0.0, d.z).length().max(1e-3)).atan();
    Camera { far, ..Camera::new(eye, d.x.atan2(-d.z), pitch, aspect) }
}

pub fn poses(world: &crate::World, aspect: f32, far: f32) -> Vec<(String, Camera, String)> {
    let mut out = vec![];
    let Some(street) = world.street.as_ref().map(|s| s.world.clone()) else { return out };
    let Ok(js) = eo_props::JsProps::load() else { return out };
    let Ok(ca) = eo_data::CaStandards::load(&world.paths) else { return out };
    let Ok(std) = eo_props::Standards::load(&ca) else { return out };
    let pw = PropsWorld::new(street.clone(), Arc::new(js), std); // no structures: the same poses with and without `--structures`
    let _ = pw.install_furniture(&world.city.buildings);
    let _ = eo_props::overhead::install(&pw, eo_props::overhead::DropBuilding::from_city(&world.city.buildings));
    let soma = eo_data::groundscale::plan([1600.0f64, -400.0]); // on the scaled plan
    let k = eo_street::tile_key(soma[0], soma[1]);
    let mut plates = vec![];
    let mut pickets = vec![];
    let mut poles = vec![];
    let oh_js = eo_props::overhead::OverheadJs::load().ok();
    for dz in -3..=3 {
        for dx in -3..=3 {
            let key = (k.0 + dx, k.1 + dz);
            if !street.tiles.contains_key(&key) {
                continue;
            }
            let placed = pw.plan_tile(key).props;
            poles.extend(placed.iter().filter(|p| p.model == Model::SignPole).copied());
            plates.extend(placed.into_iter().filter(|p| p.model == Model::SignPlate));
        }
    }
    // A printed picket (any `printed` sign variant: the street-cleaning / pay stack, the corner
    // turn and one-way plates, the passenger-loading post) is rarer: ring by ring out from SoMa.
    let is_kerb_stack = |s: &eo_props::overhead::SignAssembly| oh_js.as_ref().and_then(|o| o.sign_variants.get(s.variant as usize)).is_some_and(|v| v.printed);
    let (mut seen_tiles, mut seen_signs) = (0usize, 0usize);
    'rings: for r in 0..=24i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let key = (k.0 + dx, k.1 + dz);
                if street.tiles.contains_key(&key) {
                    let signs = eo_props::overhead::plan_tile(&pw, key).signs;
                    seen_tiles += 1;
                    seen_signs += signs.len();
                    pickets.extend(signs.into_iter().filter(|s| is_kerb_stack(s)));
                }
            }
        }
        if !pickets.is_empty() {
            break 'rings;
        }
    }
    eprintln!("sign card: {seen_tiles} tiles, {seen_signs} overhead sign assemblies scanned, {} printed", pickets.len());
    let face = |p: &eo_props::Prop| pw.js.signs.faces.get(p.variant as usize).map(|s| s.as_str()).unwrap_or("");
    let dist = |x: f64, z: f64| (x - soma[0]).hypot(z - soma[1]);
    let nearest = |names: &[&str]| -> Option<eo_props::Prop> {
        plates
            .iter()
            .filter(|p| names.contains(&face(p)) && p.pos[1] - pw.ground(p.pos[0], p.pos[2]) < 3.5)
            .min_by(|a, b| dist(a.pos[0], a.pos[2]).total_cmp(&dist(b.pos[0], b.pos[2])))
            .copied()
    };
    let close = |out: &mut Vec<(String, Camera, String)>, name: &str, p: eo_props::Prop, back: f32| {
        let n = Vec3::new(p.yaw.sin() as f32, 0.0, p.yaw.cos() as f32);
        let c = Vec3::new(p.pos[0] as f32, p.pos[1] as f32, p.pos[2] as f32);
        let eye = c + n * back - Vec3::Y * 0.25;
        out.push((name.into(), look_at(eye, c, far, aspect), format!("plate '{}' ({:.2} x {:.2} m) at ({:.1}, {:.1}, {:.1}), {back} m in front", face(&p), p.scale[0], p.scale[1], p.pos[0], p.pos[1], p.pos[2])));
    };
    if let Some(p) = nearest(&["stop"]) {
        close(&mut out, "sign-stop", p, 2.4);
        // Play distance: 14 m in front of the STOP plate at eye height, looking at it.
        let n = Vec3::new(p.yaw.sin() as f32, 0.0, p.yaw.cos() as f32);
        let c = Vec3::new(p.pos[0] as f32, p.pos[1] as f32, p.pos[2] as f32);
        let e = c + n * 14.0;
        let eye = Vec3::new(e.x, pw.ground(e.x as f64, e.z as f64) as f32 + 1.7, e.z);
        out.push(("sign-street".into(), look_at(eye, c, far, aspect), format!("street view 14 m in front of the STOP plate at ({:.1}, {:.1})", p.pos[0], p.pos[2])));
    }
    if let Some(p) = nearest(&["warning"]).or_else(|| nearest(&["pedestrian", "school", "schoolAhead"])) {
        close(&mut out, "sign-warning", p, 2.4);
    }
    if let Some(p) = nearest(&["streetCleaning"]) {
        close(&mut out, "sign-kerb", p, 1.8);
    }
    // The corner sign pole's street-name blades, close enough to fill the frame. The native
    // renderer draws the pole's own GLB here (the blade atlas is cooked but not drawn: see the
    // README, props2 "Not done"), so this pair documents that the blades are unchanged in game.
    if let Some(p) = poles.iter().min_by(|a, b| dist(a.pos[0], a.pos[2]).total_cmp(&dist(b.pos[0], b.pos[2]))) {
        let top = p.pos[1] as f32 + 2.6 * p.scale[1] as f32 - 0.25;
        let a = p.yaw as f32 + std::f32::consts::FRAC_PI_4;
        let c = Vec3::new(p.pos[0] as f32, top, p.pos[2] as f32);
        let eye = c + Vec3::new(a.sin(), 0.0, a.cos()) * 1.3 - Vec3::Y * 0.35;
        out.push(("sign-blade".into(), look_at(eye, c, far, aspect), format!("corner sign pole at ({:.1}, {:.1}), blades 1.3 m away", p.pos[0], p.pos[2])));
    }
    if let Some(s) = pickets.iter().min_by(|a, b| dist(a.pos[0], a.pos[2]).total_cmp(&dist(b.pos[0], b.pos[2]))) {
        // The printed stack's plates face +z of the assembly; look at the middle plate's height.
        let n = Vec3::new(s.yaw.sin() as f32, 0.0, s.yaw.cos() as f32);
        let mid = s.plates.get(s.plates.len() / 2).map(|p| p.centre).unwrap_or([s.pos[0], s.pos[1] + 2.6, s.pos[2]]);
        let c = Vec3::new(mid[0] as f32, mid[1] as f32, mid[2] as f32);
        let eye = c + n * 2.2 - Vec3::Y * 0.2;
        out.push(("sign-picket".into(), look_at(eye, c, far, aspect), format!("printed picket variant {} at ({:.1}, {:.1})", s.variant, s.pos[0], s.pos[2])));
    }
    out
}
