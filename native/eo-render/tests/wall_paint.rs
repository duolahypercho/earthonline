//! the painted wall finishes draw on the neutral primer. The lift
//! the kit palette applies (`eo_kit::wall_paint::primer_lift`) is primer /
//! the finish's generator colour; this checks that colour against the mean
//! of the map the game actually bakes, so the drawn mean lands on the primer.

use eo_assets::kit::{self, KitLibrary};
use eo_kit::wall_paint;

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

#[test]
fn baked_painted_finishes_average_to_their_generator_colour() {
    let lib = KitLibrary::load().unwrap();
    let primer = wall_paint::PRIMER_SRGB.map(|v| srgb_to_linear(v as f32 / 255.0));
    for name in wall_paint::PAINTED_FINISHES {
        let spec = &lib.specs[name];
        if spec.atlas_id.is_some() {
            continue; // an atlas finish divides its scan by the scan's own mean: white
        }
        let src = lib.map_source(name);
        let m = kit::bake(src, &lib.specs[src], kit::RUNTIME_SEED, 64);
        let mut sum = [0f64; 3];
        for px in m.albedo.chunks_exact(4) {
            for k in 0..3 {
                sum[k] += srgb_to_linear(px[k] as f32 / 255.0) as f64;
            }
        }
        let n = (m.albedo.len() / 4) as f64;
        let mean = sum.map(|v| (v / n) as f32);
        let own = wall_paint::finish_colour(name).unwrap();
        // The drawn mean with the lift: within 12 % of the primer per channel,
        // and no channel more than 4 % warmer than another relative to it.
        let lift = [0, 1, 2].map(|k| primer[k] / own[k]);
        let drawn = [0, 1, 2].map(|k| mean[k] * lift[k]);
        let rel = [0, 1, 2].map(|k| drawn[k] / primer[k]);
        eprintln!("{name}: bake mean {mean:?}, generator {own:?}, drawn / primer {rel:?}");
        for k in 0..3 {
            assert!((rel[k] - 1.0).abs() < 0.12, "{name}: drawn {drawn:?} against the primer {primer:?}");
        }
        let (lo, hi) = (rel.iter().cloned().fold(f32::MAX, f32::min), rel.iter().cloned().fold(f32::MIN, f32::max));
        assert!(hi / lo < 1.04, "{name}: tinted {rel:?}");
    }
}
