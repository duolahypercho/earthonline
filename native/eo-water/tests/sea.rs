//! The sea surface the physics floats bodies on (eo-water `sea`): at its grid
//! points it is the drawn sea's sum (the transform eo-render's
//! `fft_matches_the_cpu_sum` holds the GPU to), between them it reads as the
//! drawn sea reads its own grid, the shore's hold stills it, and a grid is
//! cheap enough to build several times a second.

use eo_water::sea::{self, SeaField, Shore};
use eo_water::waves::{self, Plan};

const OPEN: Shore = Shore { signed_m: 1000.0, beach: 0.0 };

fn states() -> Vec<(&'static str, waves::SeaState)> {
    vec![
        ("calm morning", waves::sea_state(7.5, [0.0, 0.0], 0.0)),
        ("afternoon westerly", waves::sea_state(16.0, [0.0, 0.0], 0.0)),
        ("storm", waves::sea_state(15.0, [11.5, 4.2], 4.0)),
    ]
}

fn drawn_sum(plan: &Plan, sp: &eo_water::spectrum::Spectrum, x: f64, z: f64, t: f64) -> f64 {
    let o = eo_water::surf_strip::ocean_share(x);
    (0..sea::HULL_CASCADES).map(|c| (plan.amp_bay[c] + (plan.amp_pac[c] - plan.amp_bay[c]) * o) * sp.cascade_height_at(c, x, z, t)).sum()
}

#[test]
fn the_grid_is_the_drawn_sum() {
    for (name, st) in states() {
        let plan = Plan::new(&st);
        let sp = waves::build(&plan, &waves::gaussians());
        let field = SeaField::from_spectrum(&plan, &sp, sea::Places::with(&plan, None, None));
        let t = 123.4;
        let g = field.grid(t);
        // On the grid points of every cascade (the bay, then past the Gate): exact.
        let mut worst = 0f64;
        for (i, x0) in [-2000.0f64, -12000.0].into_iter().enumerate() {
            for k in 0..12 {
                let c = k % 3;
                let step = waves::SIZES[c] / sea::GRIDS[c] as f64;
                let (x, z) = ((x0 / step).round() * step + (k * 7) as f64 * step, (-4200.0 / step).round() * step + (k * 3 + i) as f64 * step);
                // a point on all three grids at once is rare; compare the one cascade's own field there
                let o = eo_water::surf_strip::ocean_share(x);
                let share = plan.amp_bay[c] + (plan.amp_pac[c] - plan.amp_bay[c]) * o;
                let want = share * sp.cascade_height_at(c, x, z, t);
                let mut only = plan.clone();
                for q in 0..waves::CASCADES {
                    if q != c {
                        only.amp_bay[q] = 0.0;
                        only.amp_pac[q] = 0.0;
                    }
                }
                let got = SeaField::from_spectrum(&only, &sp, sea::Places::with(&only, None, None)).grid(t).sample_field_point(x, z, OPEN).height;
                worst = worst.max((got - want).abs());
            }
        }
        println!("{name}: modes per cascade {:?}, worst |grid - drawn sum| on grid points {worst:.2e} m", field.modes());
        // (the twin keeps k and omega in f32, as the GPU: the sum's phase k.x at 2 km off the origin
        // carries their rounding, the transform the exact lattice; 0.1 mm)
        assert!(worst < 1e-4, "{name}: the grid is not the drawn sum ({worst} m)");
        // Between grid points: what the GPU draws there is its bilinear read of its 256 grids
        // (water.wgsl `w_casc_uv`); the CPU's read of its own grids keeps to it.
        let n = eo_water::spectrum::N as f64;
        let drawn = |x: f64, z: f64| -> f64 {
            let o = eo_water::surf_strip::ocean_share(x);
            (0..sea::HULL_CASCADES)
                .map(|c| {
                    let step = waves::SIZES[c] / n;
                    let (u, v) = (x / step, z / step);
                    let (fu, fv) = (u.floor(), v.floor());
                    let (tu, tv) = (u - fu, v - fv);
                    let h = |i: f64, j: f64| sp.cascade_height_at(c, i * step, j * step, t);
                    let b = (h(fu, fv) * (1.0 - tu) + h(fu + 1.0, fv) * tu) * (1.0 - tv) + (h(fu, fv + 1.0) * (1.0 - tu) + h(fu + 1.0, fv + 1.0) * tu) * tv;
                    (plan.amp_bay[c] + (plan.amp_pac[c] - plan.amp_bay[c]) * o) * b
                })
                .sum()
        };
        let (mut e2, mut h2, mut s2) = (0f64, 0f64, 0f64);
        for k in 0..120 {
            let (x, z) = (-2000.0 + k as f64 * 13.37, -4200.0 + (k % 17) as f64 * 5.71);
            let (got, want) = (g.sample_field_point(x, z, OPEN).height, drawn(x, z));
            e2 += (got - want).powi(2);
            h2 += want * want;
            s2 += (want - drawn_sum(&plan, &sp, x, z, t)).powi(2);
        }
        println!("{name}: between grid points, rms |CPU read - GPU read| {:.5} m of a {:.4} m rms sea (the GPU's own read off the exact sum: {:.4} m)", (e2 / 120.0).sqrt(), (h2 / 120.0).sqrt(), (s2 / 120.0).sqrt());
        assert!(e2.sqrt() <= 0.02 * h2.sqrt() + 1e-6, "{name}: the grid's read strays from the drawn sea's");
    }
}

#[test]
fn no_wave_no_motion() {
    let plan = Plan::new(&waves::sea_state(16.0, [0.0, 0.0], 0.0));
    let g = SeaField::new(&plan).grid(50.0);
    // within 50 m of the shore (off a beach) the drawn sea has no waves: nothing moves.
    for d in [-5.0, 0.0, 20.0, 49.0] {
        assert_eq!(g.sample(-2000.0, -4200.0, Shore { signed_m: d, beach: 0.0 }), sea::Sample::default(), "{d} m off the shore");
    }
    assert!(g.sample(-2000.0, -4200.0, OPEN).height.abs() > 0.0);
}

#[test]
fn the_same_sea_on_every_run() {
    let plan = Plan::new(&waves::sea_state(16.0, [0.0, 0.0], 0.0));
    let (a, b) = (SeaField::new(&plan).grid(77.7), SeaField::new(&plan).grid(77.7));
    for k in 0..50 {
        let (x, z) = (k as f64 * 31.0 - 900.0, -4000.0 - k as f64 * 7.0);
        assert_eq!(a.sample(x, z, OPEN), b.sample(x, z, OPEN));
    }
}

#[test]
fn a_grid_is_cheap_enough_to_build_several_times_a_second() {
    let plan = Plan::new(&waves::sea_state(16.0, [0.0, 0.0], 0.0));
    let field = SeaField::new(&plan);
    let t0 = std::time::Instant::now();
    let mut acc = 0.0;
    for k in 0..5 {
        let g = field.grid(k as f64 * 0.1);
        for i in 0..2000 {
            acc += g.sample(-2000.0 + i as f64 * 3.1, -4200.0 + (i % 7) as f64, OPEN).height;
        }
    }
    let ms = t0.elapsed().as_secs_f64() * 1e3 / 5.0;
    println!("a grid and 2000 samples: {ms:.2} ms ({acc:.1})");
    assert!(ms < 40.0, "{ms} ms a grid");
}
