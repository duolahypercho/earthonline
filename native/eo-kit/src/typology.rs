//! per-building facade typology: the window module drawn per building, so
//! no two neighbours share one rhythm, and a tower's facade system drawn
//! from its family, its era and, where it is known, its identity.
//!
//! The archetype ports each write one window module (a Victorian's 3.3 m
//! bay at 0.52 of it, an Edwardian's 3.0 m at 0.48, a mid-century window
//! wall's 3.2 m mullions, a tower family's piers): across a census of the
//! city (`eo-kit/examples` census, 1 record in 7) every Edwardian, Victorian,
//! mid-century and tower record drew its archetype's one module, so the
//! city read as one tiled texture near and far. Here, after the spec is
//! written and before anything is planned (so the near rungs and the far
//! box, which both read the spec, agree):
//!
//! * *Residential and commercial fronts* (`vary`): the bay pitch, the window
//!   width share, sill and head move per building inside the ranges the
//!   city's building types show, from its own seeded draw (`<seed>|typology`,
//!   so the archetype's own random stream is untouched):
//!   - window width: a timber-era double-hung sash is about 0.75-1.0 m wide
//!     in a 2.4-3.6 m bay (SF Planning, Residential Design Guidelines,
//!     windows section: "vertically oriented, double-hung"; NPS Preservation
//!     Brief 9, The Repair of Historic Wooden Windows: sash windows of the
//!     period), a share of about 0.3-0.65;
//!   - sill height 0.6-1.1 m (the California Building Code's 24 in minimum
//!     sill for an operable window more than 72 in above grade, CBC 1015.8,
//!     and the 44 in maximum to an emergency escape opening, CBC 1031.3.1);
//!   - storey head clear 0.25-0.7 m;
//!   - a mid-century window wall's mullion pitch 1.2-3.6 m and spandrel
//!     0.8-1.3 m.
//! * *Towers* (`tower`): a family of facade systems instead of two, its
//!   module drawn per tower: a unitised curtain wall (a 5 ft, 1.52 m, or
//!   4 ft, 1.22 m, module: the common unitised panel widths, AAMA CW-DG-1
//!   Curtain Wall Design Guide), a stick curtain wall with expressed
//!   mullions, horizontal ribbon windows over deep spandrels, punched
//!   precast, precast with vertical fins, and the prewar masonry pier grid,
//!   with the pane's coating, the spandrel's finish and the stone's tone
//!   per tower. Known towers take their own system (`TOWER_IDENTITY`).
//!
//! Switch: `EO_FACADE_TYPOLOGY=off` (eo-app `--no-facade-typology`): every
//! spec as the port writes it. On, `cache_tag` names the kit tile cache.

use crate::classify::Classified;
use crate::js::*;
use crate::rng::Rng;

/// Bump when a draw changes (names the kit tile cache folder).
pub const VERSION: u32 = 1;

/// `EO_FACADE_TYPOLOGY=off` (or 0) turns the typology off (eo-app `--no-facade-typology`).
pub fn enabled() -> bool {
    !matches!(std::env::var("EO_FACADE_TYPOLOGY").as_deref(), Ok("off") | Ok("0"))
}

/// A tag for the kit tile cache key (empty when off).
pub fn cache_tag() -> String {
    if enabled() { format!("typology-v{VERSION}") } else { String::new() }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// One building's draw of its window module.
#[derive(Clone, Copy, Debug)]
struct Module {
    /// Bay pitch factor.
    bay: f64,
    /// Window width factor.
    width: f64,
    /// Sill and head clear offsets, metres.
    sill: f64,
    head: f64,
    /// Lights across a window (None: the port's).
    mullions: Option<i64>,
    /// A ribbon's spandrel offset (m) and mullion pitch factor.
    spandrel: f64,
    ribbon: f64,
}

fn module_for(c: &Classified) -> Module {
    let mut r = Rng::new(&format!("{}|typology", c.seed));
    let (u0, u1, u2, u3, u4, u5, u6) = (r.next(), r.next(), r.next(), r.next(), r.next(), r.next(), r.next());
    // Per archetype: (bay factor range, width factor range, sill range, head range).
    let (bay, width, sill, head) = match c.archetype {
        crate::classify::VICTORIAN_ROW => ((0.86, 1.14), (0.78, 1.18), (-0.14, 0.1), (-0.16, 0.1)),
        crate::classify::EDWARDIAN_FLATS => ((0.86, 1.2), (0.8, 1.25), (-0.12, 0.14), (-0.14, 0.14)),
        crate::classify::STUCCO_ROW | crate::classify::COTTAGE_GARAGE => ((0.85, 1.18), (0.78, 1.22), (-0.12, 0.16), (-0.1, 0.18)),
        crate::classify::BAY_WINDOW_APARTMENT => ((0.86, 1.16), (0.8, 1.2), (-0.1, 0.14), (-0.12, 0.14)),
        crate::classify::MIDCENTURY_STUCCO => ((0.8, 1.25), (0.75, 1.3), (-0.15, 0.15), (-0.12, 0.15)),
        _ => ((0.85, 1.2), (0.8, 1.2), (-0.1, 0.12), (-0.12, 0.12)),
    };
    let mullions = match c.archetype {
        // timber-era sashes: one meeting rail; a pair or a three-light group now and then.
        crate::classify::VICTORIAN_ROW | crate::classify::EDWARDIAN_FLATS => Some(if u4 < 0.62 { 0 } else if u4 < 0.9 { 1 } else { 2 }),
        crate::classify::STUCCO_ROW | crate::classify::BAY_WINDOW_APARTMENT => Some(if u4 < 0.45 { 0 } else if u4 < 0.85 { 1 } else { 2 }),
        crate::classify::MIDCENTURY_STUCCO => Some(if u4 < 0.5 { 0 } else { 1 }),
        _ => None,
    };
    Module {
        bay: lerp(bay.0, bay.1, u0),
        width: lerp(width.0, width.1, u1),
        sill: lerp(sill.0, sill.1, u2),
        head: lerp(head.0, head.1, u3),
        mullions,
        spandrel: lerp(-0.25, 0.25, u5),
        ribbon: lerp(0.4, 1.12, u6),
    }
}

fn scale_key(o: &mut Map<String, J>, key: &str, k: f64, lo: f64, hi: f64) {
    if let Some(v) = o.get(key).and_then(|v| v.as_f64()) {
        o.insert(key.into(), jn((v * k).clamp(lo, hi)));
    }
}

fn add_key(o: &mut Map<String, J>, key: &str, d: f64, lo: f64, hi: f64) {
    if let Some(v) = o.get(key).and_then(|v| v.as_f64()) {
        o.insert(key.into(), jn((v + d).clamp(lo.min(v), hi.max(v))));
    }
}

/// A window's own keys (a windowSpec, a bay or a bay's storey override; a
/// bay's `width` is the bay's, `window_width` false there).
fn vary_window(o: &mut Map<String, J>, m: &Module, window_width: bool) {
    scale_key(o, "widthRatio", m.width, 0.22, 0.78);
    scale_key(o, "openingWidth", m.width, 0.5, 4.0);
    if window_width {
        scale_key(o, "width", m.width, 0.5, 4.0);
    }
    add_key(o, "sillHeight", m.sill, 0.6, 1.1);
    add_key(o, "headClear", m.head, 0.25, 0.7);
    add_key(o, "head", -m.head, 1.6, 4.2);
    if let (Some(n), true) = (m.mullions, o.contains_key("mullions")) {
        if o.get("mullions").and_then(|v| v.as_f64()).is_some() {
            o.insert("mullions".into(), json!(n));
        }
    }
}

fn vary_elevation(e: &mut J, m: &Module) {
    let Some(o) = e.as_object_mut() else { return };
    scale_key(o, "bayWidth", m.bay, 1.6, 8.0);
    // A punched elevation on the planner's default window (elevation.rs
    // defaults: widthRatio 0.55, sill 0.9 m, head clear 0.5 m) takes the
    // building's draw on those.
    let punched = matches!(o.get("window").and_then(|v| v.as_str()), Some("punched") | None);
    if punched && !o.contains_key("windowSpec") {
        o.insert("windowSpec".into(), json!({ "widthRatio": 0.55, "sillHeight": 0.9, "headClear": 0.5 }));
    }
    if let Some(ws) = o.get_mut("windowSpec").and_then(|v| v.as_object_mut()) {
        vary_window(ws, m, true);
    }
    if let Some(rs) = o.get_mut("ribbonSpec").and_then(|v| v.as_object_mut()) {
        add_key(rs, "spandrelHeight", m.spandrel, 0.75, 1.4);
        add_key(rs, "headClear", 0.5 * m.head, 0.25, 0.7);
        if rs.get("mullionSpacing").and_then(|v| v.as_f64()).is_some_and(|s| s < 20.0) {
            scale_key(rs, "mullionSpacing", m.ribbon, 1.2, 3.6);
        }
    }
    // The bays an archetype fitted to its front keep their widths (a lot's
    // entry, oriel and plain bays); their windows take the building's draw.
    if let Some(bays) = o.get_mut("bays").and_then(|v| v.as_array_mut()) {
        for b in bays.iter_mut() {
            let Some(bo) = b.as_object_mut() else { continue };
            vary_window(bo, m, false);
            if let Some(st) = bo.get_mut("storeys") {
                let items: Vec<&mut J> = match st {
                    J::Object(map) => map.values_mut().collect(),
                    J::Array(a) => a.iter_mut().collect(),
                    _ => vec![],
                };
                for s in items {
                    if let Some(so) = s.as_object_mut() {
                        vary_window(so, m, false);
                    }
                }
            }
        }
    }
}

/// Vary one spec's window module in place (towers draw theirs in `tower`).
pub fn vary(c: &Classified, spec: &mut J) {
    if !enabled() || c.archetype == crate::classify::DOWNTOWN_TOWER || !spec.is_object() {
        return;
    }
    let m = module_for(c);
    let Some(els) = spec.get_mut("elevations").and_then(|v| v.as_object_mut()) else { return };
    for (k, v) in els.iter_mut() {
        if k == "byEdge" {
            if let Some(by) = v.as_object_mut() {
                for e in by.values_mut() {
                    vary_elevation(e, &m);
                }
            }
        } else {
            vary_elevation(v, &m);
        }
    }
    if let Some(meta) = spec.get_mut("archetype").and_then(|v| v.as_object_mut()) {
        meta.insert("typology".into(), json!({ "bay": m.bay, "width": m.width, "sill": m.sill, "head": m.head }));
    }
}

// ---------------------------------------------------------------------------
// Towers.
// ---------------------------------------------------------------------------

/// A tower's facade system as the tower archetype draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Tower {
    /// The system's name (the far box reads it): `curtain-flush`,
    /// `curtain-mullion`, `ribbon-band`, `precast-punched`, `precast-fin`,
    /// `masonry-pier`, `granite-bay`.
    pub system: &'static str,
    /// The archetype family it stands in for (`curtain-wall`, `punched-grid`, `prewar-masonry`).
    pub family: &'static str,
    pub wall: &'static str,
    pub spandrel: &'static str,
    pub glass: &'static str,
    /// The piers' and mullion caps' finish (the `trim` slot).
    pub trim: &'static str,
    pub spandrel_band: f64,
    pub pier_width: f64,
    pub pier_projection: f64,
    pub pier_spacing: f64,
    pub band_projection: f64,
    pub reveal: f64,
    /// A multiply on the wall (and spandrel) stone, 0xRRGGBB (None: the finish's own colour).
    pub stone_tint: Option<u32>,
}

/// Towers whose facade system is on record: (source key, system, stone tint).
///
/// * 555 California Street (1969): "clad in costly polished or rough
///   carnelian granite", with "thousands of bay windows" (Wikipedia, 555
///   California Street). Carnelian granite is a deep red-brown, polished it
///   reads near black with the sky in it; drawn as a dark brown stone (on the
///   ashlar finish, #d6cbb6, the tint draws about #3c322f, a polished dark
///   granite's diffuse reflectance of about 0.03-0.05), bronze-dark glass.
/// * Embarcadero Center 1-4 (1971-1982): "a repetition of concrete and glass
///   panels that organize the facade in narrow and tall windows and vertical
///   concrete stripes that protrude slightly from the window plane"
///   (buildingsdb.com, One Embarcadero Center): light precast with fins.
pub const TOWER_IDENTITY: [(&str, &str, Option<u32>); 5] = [
    ("288511106", "granite-bay", Some(0x4a4044)),
    ("32612608", "precast-fin", Some(0xf4f1ea)),
    ("32612520", "precast-fin", Some(0xf4f1ea)),
    ("32612538", "precast-fin", Some(0xf4f1ea)),
    ("616812910", "precast-fin", Some(0xf4f1ea)),
];

/// A coated or tinted pane, a weight: what the city's curtain walls show
/// (grey and bronze tints of the 1960s-80s, silver reflective coatings of
/// the 1980s, the blue-grey and clear low-e glass of the 2000s).
const CURTAIN_GLASS: [(&str, f64); 5] = [("glass-film", 1.0), ("glass-reflective", 1.3), ("glass-sky", 1.0), ("glass-dark", 0.8), ("glass-clear", 0.9)];
/// Spandrel and mullion finishes of a curtain wall.
const CURTAIN_METAL: [&str; 4] = ["anodized-aluminium", "painted-steel-mid", "perforated-metal", "painted-steel-light"];
/// Stone and precast faces, and a per-tower tone over each (multiplies;
/// white cement precast reads lightest, aged grey concrete darker; a buff
/// limestone and a pink-grey granite).
const STONE: [&str; 4] = ["concrete-precast", "limestone-ashlar", "cast-stone", "painted-block"];
const STONE_TONES: [u32; 6] = [0xffffff, 0xf6f2ea, 0xe8e6e2, 0xf2e6d6, 0xd9d6d0, 0xeee4dc];

fn weighted<'a>(roll: f64, list: &[(&'a str, f64)]) -> &'a str {
    let total: f64 = list.iter().map(|e| e.1).sum();
    let mut at = roll * total;
    for e in list {
        at -= e.1;
        if at < 0.0 {
            return e.0;
        }
    }
    list[list.len() - 1].0
}

/// One tower's facade system (`family`: the family the port drew; `prewar`
/// when the record is a prewar tower). None with the typology off.
pub fn tower(c: &Classified, family: &'static str) -> Option<Tower> {
    if !enabled() {
        return None;
    }
    let mut r = Rng::new(&format!("{}|tower-typology", c.seed));
    let u: Vec<f64> = (0..10).map(|_| r.next()).collect();
    let key = c.record_id.strip_prefix("sf-building-").unwrap_or(&c.record_id);
    let known = TOWER_IDENTITY.iter().find(|(id, _, _)| *id == key);
    let system: &'static str = match known {
        Some((_, s, _)) => s,
        None => match family {
            "prewar-masonry" => "masonry-pier",
            "curtain-wall" => {
                if u[0] < 0.55 {
                    "curtain-flush"
                } else if u[0] < 0.85 {
                    "curtain-mullion"
                } else {
                    "ribbon-band"
                }
            }
            _ => {
                if u[0] < 0.5 {
                    "precast-punched"
                } else if u[0] < 0.8 {
                    "precast-fin"
                } else {
                    "ribbon-band"
                }
            }
        },
    };
    let stone = STONE[(u[1] * STONE.len() as f64) as usize % STONE.len()];
    let tone = STONE_TONES[(u[2] * STONE_TONES.len() as f64) as usize % STONE_TONES.len()];
    let glass = weighted(u[3], &CURTAIN_GLASS);
    let metal = CURTAIN_METAL[(u[4] * CURTAIN_METAL.len() as f64) as usize % CURTAIN_METAL.len()];
    // Unitised panel modules: 5 ft (1.524 m) most often, 4 ft (1.219 m), 1.5 m metric, 6 ft (1.829 m).
    let pane = [1.524, 1.524, 1.219, 1.5, 1.829][(u[5] * 5.0) as usize % 5];
    let t = |i: usize, a: f64, b: f64| lerp(a, b, u[i]);
    let tw = match system {
        "curtain-flush" => Tower {
            system,
            family: "curtain-wall",
            wall: "anodized-aluminium",
            spandrel: metal,
            glass,
            trim: metal,
            spandrel_band: t(6, 0.85, 1.3),
            pier_width: t(7, 0.06, 0.1),
            pier_projection: t(8, 0.04, 0.09),
            pier_spacing: pane,
            band_projection: 0.03,
            reveal: 0.06,
            stone_tint: None,
        },
        "curtain-mullion" => Tower {
            system,
            family: "curtain-wall",
            wall: "anodized-aluminium",
            spandrel: metal,
            glass,
            trim: metal,
            spandrel_band: t(6, 0.9, 1.4),
            pier_width: t(7, 0.12, 0.3),
            pier_projection: t(8, 0.15, 0.35),
            pier_spacing: pane * if u[9] < 0.5 { 1.0 } else { 2.0 },
            band_projection: 0.06,
            reveal: 0.1,
            stone_tint: None,
        },
        "ribbon-band" => Tower {
            system,
            family: "punched-grid",
            wall: stone,
            spandrel: stone,
            glass: if u[9] < 0.5 { glass } else { "glass-clear" },
            trim: "anodized-aluminium",
            spandrel_band: t(6, 1.35, 1.9),
            pier_width: t(7, 0.08, 0.16),
            pier_projection: t(8, 0.02, 0.06),
            pier_spacing: pane,
            band_projection: t(9, 0.1, 0.2),
            reveal: 0.18,
            stone_tint: Some(tone),
        },
        "precast-fin" => Tower {
            system,
            family: "punched-grid",
            wall: "concrete-precast",
            spandrel: "concrete-precast",
            glass: if known.is_some() { "glass-dark" } else if u[9] < 0.5 { "glass-dark" } else { glass },
            trim: "concrete-precast",
            spandrel_band: t(6, 0.7, 1.05),
            pier_width: t(7, 0.32, 0.55),
            pier_projection: t(8, 0.3, 0.6),
            pier_spacing: t(5, 1.45, 2.1),
            band_projection: 0.06,
            reveal: 0.3,
            stone_tint: Some(known.and_then(|k| k.2).unwrap_or(tone)),
        },
        "granite-bay" => Tower {
            system,
            family: "prewar-masonry",
            wall: "limestone-ashlar",
            spandrel: "limestone-ashlar",
            glass: "glass-dark",
            trim: "limestone-ashlar",
            spandrel_band: 1.05,
            pier_width: 0.34,
            pier_projection: 0.12,
            pier_spacing: 1.6,
            band_projection: 0.05,
            reveal: 0.16,
            stone_tint: known.and_then(|k| k.2),
        },
        "masonry-pier" => Tower {
            system,
            family: "prewar-masonry",
            wall: if u[9] < 0.6 { "limestone-ashlar" } else { "cast-stone" },
            spandrel: "cast-stone",
            glass: "glass-clear",
            trim: "cast-stone-trim",
            spandrel_band: t(6, 1.1, 1.6),
            pier_width: t(7, 0.75, 1.3),
            pier_projection: t(8, 0.08, 0.16),
            pier_spacing: t(5, 2.4, 3.6),
            band_projection: 0.06,
            reveal: 0.3,
            stone_tint: Some(tone),
        },
        // precast-punched
        _ => Tower {
            system,
            family: "punched-grid",
            wall: stone,
            spandrel: stone,
            glass: if u[9] < 0.7 { "glass-clear" } else { glass },
            trim: stone,
            spandrel_band: t(6, 1.2, 1.7),
            pier_width: t(7, 0.7, 1.3),
            pier_projection: t(8, 0.12, 0.26),
            pier_spacing: t(5, 2.6, 4.6),
            band_projection: 0.12,
            reveal: 0.26,
            stone_tint: Some(tone),
        },
    };
    Some(tw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classified(archetype: &'static str, seed: u32) -> Classified {
        Classified { archetype, seed, ..Default::default() }
    }

    #[test]
    fn neighbours_draw_different_modules() {
        let a = module_for(&classified(crate::classify::EDWARDIAN_FLATS, 11));
        let b = module_for(&classified(crate::classify::EDWARDIAN_FLATS, 12));
        assert!((a.bay - b.bay).abs() > 1e-6 || (a.width - b.width).abs() > 1e-6);
        // Over many seeds the draw spans its range.
        let w: Vec<f64> = (0..400).map(|s| module_for(&classified(crate::classify::VICTORIAN_ROW, s)).width).collect();
        let (lo, hi) = w.iter().fold((f64::MAX, f64::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
        assert!(lo < 0.82 && hi > 1.14, "{lo} {hi}");
    }

    #[test]
    fn a_spec_varies_its_windows_and_keeps_its_fitted_bays() {
        if !enabled() {
            return;
        }
        let c = classified(crate::classify::VICTORIAN_ROW, 5);
        let mut spec = json!({
            "elevations": {
                "front": { "bayWidth": 3.3, "windowSpec": { "widthRatio": 0.52, "sillHeight": 0.82, "headClear": 0.44, "mullions": 1 },
                           "bays": [{ "width": 2.0, "storeys": { "0": "entrance" } }, { "width": 4.4, "openingWidth": 1.0 }] },
                "default": "blind",
            },
            "archetype": {},
        });
        vary(&c, &mut spec);
        let f = &spec["elevations"]["front"];
        assert_ne!(f["bayWidth"].as_f64(), Some(3.3));
        assert_eq!(f["bays"][0]["width"].as_f64(), Some(2.0));
        assert_eq!(f["bays"][1]["width"].as_f64(), Some(4.4));
        let s = f["windowSpec"]["sillHeight"].as_f64().unwrap();
        assert!((0.6..=1.1).contains(&s), "{s}");
        assert_eq!(spec["elevations"]["default"], json!("blind"));
    }

    #[test]
    fn towers_draw_several_systems_and_known_towers_their_own() {
        if !enabled() {
            return;
        }
        let mut seen = std::collections::BTreeSet::new();
        for s in 0..200 {
            let fam = if s % 3 == 0 { "punched-grid" } else { "curtain-wall" };
            let t = tower(&classified(crate::classify::DOWNTOWN_TOWER, s), fam).unwrap();
            assert!(t.pier_spacing > t.pier_width, "{t:?}");
            seen.insert(t.system);
        }
        assert!(seen.len() >= 5, "{seen:?}");
        let mut c = classified(crate::classify::DOWNTOWN_TOWER, 3);
        c.record_id = "sf-building-288511106".into();
        assert_eq!(tower(&c, "punched-grid").unwrap().system, "granite-bay");
    }
}
