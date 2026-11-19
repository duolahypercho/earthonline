//! the exposure field on the real shore and ground (sf-city.json.gz,
//! sf-elevation.bin.gz): the open Pacific exposed, the strait between, the
//! inner bay sheltered from the swell, the city's lee calmer than the Gate.
//! `EO_WATER_EXPOSURE_DUMP=<dir>` also writes the three channels as PGM images.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    if let Ok(r) = std::env::var("EO_REPO_ROOT") {
        return PathBuf::from(r);
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !p.join("public/data/sf/sf-city.json.gz").exists() {
        assert!(p.pop(), "repo root not found (set EO_REPO_ROOT)");
    }
    p
}

// game metres at the 1.25 ground plan
fn game(lon: f64, lat: f64) -> (f64, f64) {
    let s = eo_water::ground_scale();
    ((lon + 122.4194) * 87_986.247_476_406_54 * s, (37.778 - lat) * 110_574.0 * s)
}

#[test]
fn the_real_sea_is_exposed_where_it_should_be() {
    let root = repo_root();
    let data = eo_water::load(&root, None).expect("water data");
    let e = eo_water::exposure::installed().expect("the exposure installed by load");
    let at = |name: &str, lon: f64, lat: f64| {
        let (x, z) = game(lon, lat);
        let v = e.at(x, z);
        eprintln!("{name:28} ({x:7.0}, {z:7.0}) swell {:.2} wind {:.2} fetch {:.2} water {:.2} (field {:.0} m)", v[0], v[1], v[2], v[3], data.field.signed_exact(x, z));
        v
    };
    let pacific = at("Pacific off Ocean Beach", -122.56, 37.76);
    let mouth = at("Gate mouth (Point Bonita)", -122.515, 37.81);
    let span = at("Golden Gate mid-span", -122.4783, 37.8199);
    let fort = at("off Fort Point", -122.4745, 37.8125);
    let crissy = at("off Crissy Field", -122.455, 37.8085);
    let baker = at("off Baker Beach", -122.4845, 37.795);
    let alcatraz_e = at("east of Alcatraz", -122.405, 37.827);
    let aquatic = at("Aquatic Park", -122.4225, 37.8095);
    let marin = at("Marin Headlands (land)", -122.49, 37.835);
    // the station's open water, 300 m north of NOAA 9414290 (where the ratio is taken)
    let station = at("NOAA 9414290's open water", -122.4659, 37.8063 + 300.0 / 110_574.0);
    if let Ok(dir) = std::env::var("EO_WATER_EXPOSURE_DUMP") {
        for (c, name) in ["swell", "wind", "fetch", "water"].iter().enumerate() {
            let mut b = format!("P5\n{} {}\n255\n", e.width, e.height).into_bytes();
            b.extend(e.texels.iter().map(|t| t[c]));
            std::fs::write(PathBuf::from(&dir).join(format!("exposure-{name}.pgm")), b).unwrap();
        }
    }
    assert!(pacific[0] > 0.9 && pacific[3] > 0.5, "open Pacific {pacific:?}");
    assert!(mouth[0] > 0.6, "the Gate's mouth {mouth:?}");
    assert!(baker[0] > 0.5, "Baker Beach faces the swell {baker:?}");
    assert!(span[0] > 0.05 && span[0] < mouth[0] + 0.05, "the strait between {span:?}");
    assert!(alcatraz_e[0] < 0.15, "the inner bay sheltered {alcatraz_e:?}");
    assert!(crissy[0] < span[0], "Crissy behind the narrows {crissy:?} {span:?}");
    assert!(aquatic[1] < span[1], "the city's lee calmer than the Gate {aquatic:?} {span:?}");
    assert!(span[1] >= 0.95, "the Gate's wind at least the station's {span:?}");
    assert!(marin[3] < 0.5, "Marin is land {marin:?}");
    // (1 at the station's cell; read bilinearly between the 100 m cells here)
    assert!((station[1] - 1.0).abs() < 0.08, "the station's wind is the station's {station:?}");
    let _ = fort;
}
