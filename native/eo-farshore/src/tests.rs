//! tests. The ones that need the cooked terrain cook it from the
//! DEM (or read `EO_FARSHORE_TEST_CACHE`'s cooked file) and are skipped, with
//! a line saying so, when neither is there.

use super::*;
use std::sync::OnceLock;

fn repo_root() -> PathBuf {
    eo_data::DataPaths::discover().expect("repository data").repo_root
}

fn terrain() -> Option<&'static FarTerrain> {
    static T: OnceLock<Option<FarTerrain>> = OnceLock::new();
    T.get_or_init(|| {
        let cache = std::env::var("EO_FARSHORE_TEST_CACHE").ok().map(PathBuf::from);
        load_or_bake(&repo_root(), cache.as_deref()).expect("farshore cook")
    })
    .as_ref()
}

fn grid_unscaled() -> ElevationGrid {
    let p = repo_root().join("public/data/sf/sf-elevation.bin.gz");
    ElevationGrid::decode(&eo_data::read_maybe_gzip(&p).unwrap()).unwrap()
}

/// `projectLatLon` (src/world/geo/sf-frame.js): surveyed x east, z south.
fn project(lat: f64, lon: f64) -> (f64, f64) {
    let f = Frame::sf();
    ((lon - f.center_lon) * f.m_per_deg_lon, -(lat - f.center_lat) * f.m_per_deg_lat)
}

#[test]
fn projection_is_the_city_frame() {
    // The cook unprojects with the city's frame; projecting back lands on the same point.
    let f = Frame::sf();
    for (lat, lon) in [(37.8816, -121.9142), (37.9293, -122.5776), (37.8107, -122.3627), (37.70, -123.0)] {
        let (x, z) = project(lat, lon);
        let (la, lo) = f.unproject(x, z);
        assert!((la - lat).abs() < 1e-12 && (lo - lon).abs() < 1e-12, "{lat} {lon}");
    }
}

#[test]
fn cooked_file_round_trips_and_damage_is_refused() {
    let (w, h) = (64usize, 32usize);
    let header = CookHeader {
        version: COOK_VERSION.into(),
        frame: "test".into(),
        origin_x: ORIGIN_X,
        origin_z: ORIGIN_Z,
        texel_m: TEXEL_M,
        width: w,
        height: h,
        sea_level_y: SEA_LEVEL_Y,
        heights: String::new(),
        classes: String::new(),
        shape: String::new(),
        source: String::new(),
        tiles: vec![],
        city_grid: String::new(),
    };
    let heights: Vec<f32> = (0..w * h).map(|k| ((k * 37 % 2900) as f32 - 120.0) / 10.0 * 4.0).map(|v| (v * 10.0).round() / 10.0).collect();
    let classes: Vec<[u8; 4]> = (0..(w / 2) * (h / 2)).map(|k| [(k % 256) as u8, 3, 200, 9]).collect();
    let shape: Vec<[u8; 2]> = (0..(w / 2) * (h / 2)).map(|k| [(k * 7 % 256) as u8, 128]).collect();
    let t = FarTerrain { header: header.clone(), heights: heights.clone(), classes: classes.clone(), shape: shape.clone(), class_width: w / 2, class_height: h / 2, from: "test" };
    let bytes = encode(&t).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(back.header, header);
    assert_eq!(back.classes, classes);
    assert_eq!(back.shape, shape);
    for (a, b) in back.heights.iter().zip(&heights) {
        assert!((a - b).abs() < 0.051, "{a} {b}");
    }
    // A flipped byte inside the payload, and a truncated file, are refused.
    let mut raw = Vec::new();
    GzDecoder::new(&bytes[..]).read_to_end(&mut raw).unwrap();
    raw[200] ^= 0x40;
    let mut gz = GzEncoder::new(Vec::new(), Compression::new(1));
    gz.write_all(&raw).unwrap();
    assert!(decode(&gz.finish().unwrap()).is_err());
    assert!(decode(&bytes[..bytes.len() / 2]).is_err());
}

#[test]
fn starts_without_the_dem() {
    // No DEM folder and no cache: no far terrain, and no error (the game starts as before).
    let empty = std::env::temp_dir().join(format!("farshore-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty).unwrap();
    // SAFETY: the test reads the variable once, in this thread, before restoring it.
    let before = std::env::var("EO_DEM_ROOT").ok();
    unsafe { std::env::set_var("EO_DEM_ROOT", &empty) };
    let r = load_or_bake(&repo_root(), None);
    match before {
        Some(v) => unsafe { std::env::set_var("EO_DEM_ROOT", v) },
        None => unsafe { std::env::remove_var("EO_DEM_ROOT") },
    }
    let _ = std::fs::remove_dir(&empty);
    assert!(r.expect("no error without the DEM").is_none());
}

#[test]
fn hooks_use_the_same_constant() {
    // The water and the sky volume read the far layer's depth with the literal DEPTH_K.
    let k = format!("{DEPTH_K:e}");
    assert_eq!(k, "1e-9");
    for (name, src) in [
        ("water.wgsl", include_str!("../../eo-render/src/water.wgsl")),
        ("sky_volume.wgsl", include_str!("../../eo-render/src/sky_volume.wgsl")),
        ("sky_resolve.wgsl", include_str!("../../eo-render/src/sky_resolve.wgsl")),
    ] {
        let hooks: Vec<&str> = src.lines().filter(|l| l.contains("DEPTH_K")).collect();
        assert!(!hooks.is_empty(), "{name}: no farshore hook");
        for l in hooks {
            assert!(l.contains("1e-9"), "{name}: a farshore hook without DEPTH_K: {l}");
        }
    }
}

#[test]
fn known_summits_and_islands() {
    let Some(t) = terrain() else {
        eprintln!("skipped: no DEM and no cooked file");
        return;
    };
    // NAVD88 heights of named points (USGS GNIS / NGS datasheets, rounded);
    // the 32 m cook lowers a sharp summit, so the bound is generous below.
    for (name, lat, lon, navd, below, above) in [
        ("Mount Diablo", 37.8816, -121.9142, 1173.0, 40.0, 10.0),
        ("Mount Tamalpais East Peak", 37.9293, -122.5776, 784.0, 40.0, 10.0),
        ("Angel Island, Mount Livermore", 37.8616, -122.4312, 240.0, 30.0, 10.0),
        ("Grizzly Peak (Berkeley Hills)", 37.8818, -122.2339, 527.0, 40.0, 15.0),
        ("San Bruno Mountain", 37.6866, -122.4342, 400.0, 40.0, 15.0),
        ("Yerba Buena Island (city grid)", 37.8102, -122.3644, 105.0, 30.0, 15.0),
    ] {
        // The highest cooked point within 300 m (named summit coordinates are rounded).
        let (x, z) = project(lat, lon);
        let mut y = f64::MIN;
        for k in 0..=24 {
            for r in 0..=6 {
                let a = k as f64 / 24.0 * std::f64::consts::TAU;
                let d = r as f64 * 50.0;
                y = y.max(t.sample_surveyed(x + d * a.cos(), z + d * a.sin()) - NAVD88_TO_GAME_Y);
            }
        }
        println!("{name}: cooked {y:.1} m NAVD88 (named {navd} m)");
        assert!(y > navd - below && y < navd + above, "{name}: {y:.1} m, named {navd} m");
    }
    // The open bay and the Pacific are under the sea.
    for (name, lat, lon) in [("the bay off Emeryville", 37.835, -122.33), ("San Pablo Bay", 38.05, -122.40), ("the Pacific off Point Reyes", 37.90, -123.05)] {
        let (x, z) = project(lat, lon);
        let y = t.sample_surveyed(x, z);
        assert!(y < SEA_LEVEL_Y, "{name}: {y:.2}");
    }
}

/// The seam: on the ground plan (scale `s`, heights `hs`), the far terrain just
/// outside the city grid stands where the grid's own edge stands, and inside
/// it the far terrain is the grid resampled. Tolerances: the cook is 32 m
/// texels against the grid's 8 m cells, so the difference is the grid's own
/// relief inside one 32 m texel.
#[test]
fn meets_the_city_grid() {
    let Some(t) = terrain() else {
        eprintln!("skipped: no DEM and no cooked file");
        return;
    };
    // The surveyed grid (plan 1, heights 1), and the grid as the game loads it
    // (`ElevationGrid::load`: the process's ground plan, 1.25 and heights 1.25 by default).
    let paths = eo_data::DataPaths::discover().unwrap();
    let cases = [(grid_unscaled(), 1.0, 1.0), (ElevationGrid::load(&paths).unwrap(), eo_data::groundscale::scale(), eo_data::groundscale::height_scale())];
    assert_eq!((cases[1].1, cases[1].2), (1.25, 1.25), "the approved ground plan");
    for (gs, s, hs) in cases {
        let (x0, z0, x1, z1) = (gs.meta.origin_x, gs.meta.origin_z, gs.max_x(), gs.max_z());
        // Along the four edges, 2 m outside (game metres), land only (the sea is not drawn by either there).
        let mut diffs = vec![];
        let mut worst = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let n = 4000;
        for k in 0..n {
            let f = (k as f64 + 0.5) / n as f64;
            for (gx, gz, ix, iz) in [
                (x0 + f * (x1 - x0), z0 - 2.0, x0 + f * (x1 - x0), z0 + 0.1),
                (x0 + f * (x1 - x0), z1 + 2.0, x0 + f * (x1 - x0), z1 - 0.1),
                (x0 - 2.0, z0 + f * (z1 - z0), x0 + 0.1, z0 + f * (z1 - z0)),
                (x1 + 2.0, z0 + f * (z1 - z0), x1 - 0.1, z0 + f * (z1 - z0)),
            ] {
                let edge = gs.sample(ix, iz).unwrap();
                if edge <= SEA_LEVEL_Y + 0.5 {
                    continue;
                }
                let far = t.sample_game(gx, gz, s, hs);
                if (far - edge).abs() > worst.0 {
                    worst = ((far - edge).abs(), ix, iz, edge);
                }
                diffs.push((far - edge).abs());
            }
        }
        diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = diffs[diffs.len() / 2];
        let p95 = diffs[diffs.len() * 95 / 100];
        let max = *diffs.last().unwrap();
        // The grid's own relief across one 32 m cooked texel at its steepest edge: the slope there.
        let (_, wx, wz, wy) = worst;
        let g8 = |x: f64, z: f64| gs.sample(x.clamp(x0 + 0.1, x1 - 0.1), z.clamp(z0 + 0.1, z1 - 0.1)).unwrap();
        let slope = (g8(wx + 8.0, wz) - g8(wx - 8.0, wz)).abs().max((g8(wx, wz + 8.0) - g8(wx, wz - 8.0)).abs()) / 16.0;
        println!("  worst at game ({wx:.0}, {wz:.0}), grid y {wy:.1}, local grade {slope:.2}");
        println!("seam at plan {s}, heights {hs}: {} land samples on the grid's edge, |far - grid| median {p50:.2} m, p95 {p95:.2} m, max {max:.2} m", diffs.len());
        assert!(diffs.len() > 1000);
        // Tolerance: a 32 m texel against 8 m cells; the bound on the worst point is that texel's
        // reach on the steepest edge (a cliff of grade ~1 moves ~16 m across it).
        assert!(p50 < 0.5 * hs && p95 < 3.5 * hs && max < 16.0 * hs, "seam p50 {p50:.2} p95 {p95:.2} max {max:.2}");
        // Inside: the far terrain is the grid resampled (32 m texels), on the same plan.
        let mut inner = vec![];
        for k in 0..20_000u64 {
            let (u, v) = (((k * 7919) % 10_007) as f64 / 10_007.0, ((k * 104_729) % 9_973) as f64 / 9_973.0);
            let (gx, gz) = (x0 + 600.0 + u * (x1 - x0 - 1200.0), z0 + 600.0 + v * (z1 - z0 - 1200.0));
            let gy = gs.sample(gx, gz).unwrap();
            if gy <= SEA_LEVEL_Y + 0.5 {
                continue;
            }
            inner.push((t.sample_game(gx, gz, s, hs) - gy).abs());
        }
        inner.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let ip50 = inner[inner.len() / 2];
        let ip95 = inner[inner.len() * 95 / 100];
        println!("inside the grid at plan {s}: {} land samples, |far - grid| median {ip50:.2} m, p95 {ip95:.2} m", inner.len());
        assert!(ip50 < 1.0 * hs && ip95 < 4.0 * hs);
    }
}

/// The Bay Bridge's eastern touchdown (Oakland) and the Golden Gate's north
/// anchorage (Marin) on the ground plan: the far terrain is land there, at
/// the plan point the city's own transform gives the same lat/lon.
#[test]
fn bridges_land_on_the_far_shore() {
    let Some(t) = terrain() else {
        eprintln!("skipped: no DEM and no cooked file");
        return;
    };
    // The process's ground plan (`groundscale::plan`, 1.25 by default) and heights.
    let (s, hs) = (eo_data::groundscale::scale(), eo_data::groundscale::height_scale());
    for (name, lat, lon, lo, hi) in [
        ("Bay Bridge east touchdown (Oakland)", 37.8255, -122.2930, 0.0, 15.0),
        ("Golden Gate north anchorage (Marin)", 37.8324, -122.4800, 10.0, 80.0),
        ("Sausalito waterfront", 37.8590, -122.4852, 0.0, 40.0),
        ("Treasure Island", 37.8235, -122.3707, 0.0, 12.0),
    ] {
        let (x, z) = project(lat, lon);
        let p = eo_data::groundscale::plan([x, z]);
        let y = t.sample_game(p[0], p[1], s, hs);
        let navd = t.sample_surveyed(x, z) - NAVD88_TO_GAME_Y;
        println!("{name}: game ({:.0}, {:.0}), y {y:.2}, {navd:.1} m NAVD88", p[0], p[1]);
        assert!(y > SEA_LEVEL_Y && navd > lo && navd < hi, "{name}: {navd:.1} m NAVD88");
    }
}

/// The class guesses where the heights should put them (a guess, not land
/// cover: this checks the guess does what it says, not that it is right).
#[test]
fn class_guesses_where_expected() {
    let Some(t) = terrain() else {
        eprintln!("skipped: no DEM and no cooked file");
        return;
    };
    let class_at = |lat: f64, lon: f64| {
        let (x, z) = project(lat, lon);
        let i = (((x - t.header.origin_x) / (t.header.texel_m * 2.0)) as usize).min(t.class_width - 1);
        let j = (((z - t.header.origin_z) / (t.header.texel_m * 2.0)) as usize).min(t.class_height - 1);
        t.classes[j * t.class_width + i]
    };
    let mut urban = 0;
    for (name, lat, lon) in [("downtown Oakland", 37.8044, -122.2712), ("Alameda", 37.7652, -122.2416), ("Berkeley flats", 37.8670, -122.2900), ("the Sunset (SF)", 37.7530, -122.4940), ("San Mateo", 37.5630, -122.3255)] {
        let c = class_at(lat, lon);
        println!("{name}: urban {} lake {} wood {} marsh {}", c[0], c[1], c[2], c[3]);
        urban += (c[0] > 100) as u32;
    }
    for (name, lat, lon) in [("Muir Woods (Mount Tamalpais's north-east flank)", 37.8970, -122.5810), ("Redwood Regional Park (Oakland hills)", 37.8100, -122.1660), ("Mount Diablo's flank", 37.8700, -121.9300)] {
        let c = class_at(lat, lon);
        println!("{name}: urban {} lake {} wood {} marsh {}", c[0], c[1], c[2], c[3]);
    }
    // the shape plane: a summit sees the whole sky and stands above its
    // surroundings; Muir Woods' canyon sees less and lies below them.
    let shape_at = |lat: f64, lon: f64| {
        let (x, z) = project(lat, lon);
        let i = (((x - t.header.origin_x) / (t.header.texel_m * 2.0)) as usize).min(t.class_width - 1);
        let j = (((z - t.header.origin_z) / (t.header.texel_m * 2.0)) as usize).min(t.class_height - 1);
        t.shape[j * t.class_width + i]
    };
    let peak = shape_at(37.9293, -122.5776);
    let canyon = shape_at(37.8916, -122.5713);
    println!("Mount Tamalpais East Peak: sky {} valley {}; Muir Woods canyon: sky {} valley {}", peak[0], peak[1], canyon[0], canyon[1]);
    assert!(peak[0] > canyon[0] && peak[1] < 128 && canyon[1] > 128, "the shape plane: peak {peak:?}, canyon {canyon:?}");
    let lakes = t.classes.iter().filter(|c| c[1] > 127).count();
    println!("lake cells (64 m): {lakes} ({:.1} km2)", lakes as f64 * 0.064 * 0.064);
    assert!(urban >= 4, "the built-up guess misses the bay's flats");
    assert!(lakes > 100, "no hydro-flattened lake found");
}

#[test]
#[ignore]
fn print_dem_around_reservoirs() {
    let root = dem_root().expect("EO_DEM_ROOT");
    let t = dem::DemTile::read(&root.join("raw/USGS_1_n38w123.tif")).unwrap();
    for (name, lat, lon) in [("San Pablo Reservoir", 37.9230, -122.2600), ("Crystal Springs", 37.5220, -122.3540), ("Upper San Leandro", 37.7780, -122.1640), ("bay off Emeryville", 37.835, -122.33)] {
        let (fx, fy) = ((lon - t.west) / t.d_lon - 0.5, (t.north - lat) / t.d_lat - 0.5);
        let (c, r) = (fx.round() as usize, fy.round() as usize);
        println!("{name}:");
        for dr in 0..5 {
            let row: Vec<String> = (0..5).map(|dc| format!("{:9.3}", t.values[(r + dr - 2) * t.width + c + dc - 2])).collect();
            println!("  {}", row.join(" "));
        }
    }
}
