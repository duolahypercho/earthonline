//! a kit building's wall paint, chosen per building by rule.
//!
//! The kit's painted wall finishes carry a warm generator colour
//! (`painted-stucco` #d9cfbd, the stucco and siding near it) that every
//! paint multiplies, and the archetype paint lists lean warm (cream, sand,
//! butter, peach), so one cream owns the city near and far. Here:
//!
//! * *Primer.* Every painted wall finish (`PAINTED_WALL_FINISHES`) draws on a
//!   neutral primer (`PRIMER_SRGB`): the renderer multiplies the finish's
//!   material colour by `primer_lift` (primer / the finish's own colour), so
//!   a paint is the colour it names on any finish. The finish keeps its maps
//!   and scans (relief, grain, boards); only the colour under the paint moves.
//! * *Paint.* A building's wall paint is a target colour picked by a
//!   weighted draw from its archetype's palette, weighted by district
//!   (`palette_for`): cool white, pale grey and pale stucco first, pastel
//!   accents, a few deeper colours; value and saturation vary more than hue.
//!   The tint written is target / primer (a multiply of at most 1).
//! * Victorian rows and Edwardian flats keep the period paint the port draws
//!   (already varied: charcoal, navy, sage, teal, oxblood). *Period primer*
//!   (on by default): the paint is the colour it names, on the primer, its
//!   value moved as a palette draw's is (`pick`): a paint's albedo is its
//!   light reflectance value (a white house paint's LRV is about 80-90, a
//!   charcoal's about 10), whatever the boards under it. Off
//!   (`EO_PERIOD_PAINT=off`, eo-app `--no-period-paint`): the paint at the
//!   luminance of its finish, as before (a white Victorian on wood siding then
//!   drew at 0.36). Chinatown's paint stays.
//! * Every other painted finish on the building without a paint (a recess, a
//!   bulkhead) takes the neutral grey of its old colour's luminance, so
//!   nothing turns brighter than before unless a palette says so. The
//!   blind-wall (`+fenestrated`) twin of a painted finish takes its paint.
//!
//! * *Body chroma* (on by default): a wall body's paint is held to CIELAB
//!   chroma `WALL_CHROMA_MAX` (25), its lightness and hue kept. The city's
//!   painted bodies are whites, creams, greys, pastels and muted period
//!   colours; strong colour is the trim's, a balcony's, an awning's and a
//!   sign's (SF Planning, Chinatown Area Plan and its design guidelines:
//!   colour as accent on balconies, eaves and trim). The port's Chinatown
//!   fronts drew whole walls in its accent paints (scarlet #b0453c, C* 50;
//!   gold #d9b24a, C* 58), the toy-town look of the round-1 aerials; capped,
//!   they read as the dusty brick red, ochre and sage of the real streets.
//!   A Chinatown front's accent paint goes on its trim (cornices, sills,
//!   surrounds), its body takes the apartment palette's paint.
//!   Off (`EO_WALL_CHROMA=off`, eo-app `--no-wall-chroma`): uncapped, the
//!   accent on the whole wall.
//!
//! Switch: `EO_WALL_PAINT=off` (eo-app `--no-wall-paint`): no primer, the
//! port's tints. On, `cache_tag` names the kit tile cache folder.

use crate::classify::Classified;
use crate::pipeline::{FENESTRATED_SUFFIX, Tints, base_material_name};
use crate::rng::hash_seed;
use eo_data::roof_colours as rc;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// Bump when the palettes or the rule change (names the kit tile cache folder).
pub const VERSION: u32 = 4;

/// The wall finishes a paint is drawn on (pipeline.rs `PAINTED_WALL_FINISHES`).
pub const PAINTED_FINISHES: [&str; 6] = ["wood-siding", "painted-stucco", "float-finish-stucco", "scanned-painted-plaster", "painted-clapboard", "painted-shingle"];

/// The neutral primer under every paint (sRGB): the brightest a paint can be.
pub const PRIMER_SRGB: [u8; 3] = [242, 242, 241];

static PINNED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Tests that compare with the JS planner pin the paint off (as they pin the
/// ground plan to 1:1): the port's own tints, no primer.
pub fn pin(on: bool) {
    PINNED.store(if on { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
}

pub fn enabled() -> bool {
    match PINNED.load(std::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => !matches!(std::env::var("EO_WALL_PAINT").as_deref(), Ok("off") | Ok("0")),
    }
}

/// A tag for the kit tile cache key (empty when off: the key is today's).
pub fn cache_tag() -> String {
    if enabled() {
        format!(
            "wall-paint-v{VERSION}{}{}{}",
            if period_primer() { "-period-primer" } else { "" },
            if chroma_cap() { format!("-chroma{WALL_CHROMA_MAX}") } else { String::new() },
            crate::presidio::cache_tag()
        )
    } else {
        String::new()
    }
}

/// The most CIELAB chroma (C*ab) a wall body's paint draws.
pub const WALL_CHROMA_MAX: u32 = 25;

/// The body chroma cap (`EO_WALL_CHROMA=off`: uncapped).
pub fn chroma_cap() -> bool {
    !matches!(std::env::var("EO_WALL_CHROMA").as_deref(), Ok("off") | Ok("0"))
}

// CIE 1976 L*a*b* of linear sRGB (D65 white, the sRGB primaries' matrix of IEC 61966-2-1).
fn lab_f(t: f32) -> f32 {
    const D: f32 = 6.0 / 29.0;
    if t > D * D * D { t.cbrt() } else { t / (3.0 * D * D) + 4.0 / 29.0 }
}

fn lab_finv(t: f32) -> f32 {
    const D: f32 = 6.0 / 29.0;
    if t > D { t * t * t } else { 3.0 * D * D * (t - 4.0 / 29.0) }
}

const WHITE_D65: [f32; 3] = [0.950_47, 1.0, 1.088_83];

/// Linear sRGB to CIELAB (L* 0-100).
pub fn cielab(c: [f32; 3]) -> [f32; 3] {
    let x = 0.412_456_4 * c[0] + 0.357_576_1 * c[1] + 0.180_437_5 * c[2];
    let y = 0.212_672_9 * c[0] + 0.715_152_2 * c[1] + 0.072_175 * c[2];
    let z = 0.019_333_9 * c[0] + 0.119_192 * c[1] + 0.950_304_1 * c[2];
    let (fx, fy, fz) = (lab_f(x / WHITE_D65[0]), lab_f(y / WHITE_D65[1]), lab_f(z / WHITE_D65[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIELAB to linear sRGB (unclamped).
pub fn cielab_to_linear(l: [f32; 3]) -> [f32; 3] {
    let fy = (l[0] + 16.0) / 116.0;
    let fx = fy + l[1] / 500.0;
    let fz = fy - l[2] / 200.0;
    let (x, y, z) = (lab_finv(fx) * WHITE_D65[0], lab_finv(fy) * WHITE_D65[1], lab_finv(fz) * WHITE_D65[2]);
    [3.240_454_2 * x - 1.537_138_5 * y - 0.498_531_4 * z, -0.969_266 * x + 1.876_010_8 * y + 0.041_556 * z, 0.055_643_4 * x - 0.204_025_9 * y + 1.057_225_2 * z]
}

/// A colour held to `max` CIELAB chroma, its L* and hue kept.
pub fn capped_chroma(c: [f32; 3], max: f32) -> [f32; 3] {
    let mut l = cielab(c);
    let ch = l[1].hypot(l[2]);
    if ch <= max {
        return c;
    }
    l[1] *= max / ch;
    l[2] *= max / ch;
    cielab_to_linear(l).map(|v| v.clamp(0.0, 1.0))
}

/// A wall body's tint (on the primer) with its drawn colour's chroma capped.
fn capped_tint(t: u32) -> u32 {
    if !chroma_cap() {
        return t;
    }
    let p = lin(PRIMER_SRGB);
    let drawn = hex_linear(t);
    let drawn = [0, 1, 2].map(|k| drawn[k] * p[k]);
    on_primer(capped_chroma(drawn, WALL_CHROMA_MAX as f32))
}

/// The timber rows' period paint drawn on the primer (`EO_PERIOD_PAINT=off`: at its finish's luminance).
pub fn period_primer() -> bool {
    !matches!(std::env::var("EO_PERIOD_PAINT").as_deref(), Ok("off") | Ok("0"))
}

fn lin(c: [u8; 3]) -> [f32; 3] {
    rc::rgb8_linear(c)
}

fn hex_linear(h: u32) -> [f32; 3] {
    lin([(h >> 16) as u8, (h >> 8) as u8, h as u8])
}

fn linear_hex(c: [f32; 3]) -> u32 {
    let b = c.map(|v| (rc::linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u32);
    (b[0] << 16) | (b[1] << 8) | b[2]
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Each kit finish's generator colour (linear) and whether it draws a scanned
/// atlas cell (whose map the renderer divides by its own mean: its colour is
/// then white), from kit-material-specs.json.
fn spec_colours() -> &'static HashMap<String, ([f32; 3], bool)> {
    static S: OnceLock<HashMap<String, ([f32; 3], bool)>> = OnceLock::new();
    S.get_or_init(|| {
        let path = eo_data::native_dir().join("eo-assets/data/kit-material-specs.json");
        let mut out = HashMap::new();
        let Ok(text) = std::fs::read_to_string(path) else { return out };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return out };
        let parse = |s: &str| u32::from_str_radix(s.trim_start_matches('#'), 16).ok().map(hex_linear);
        if let Some(m) = v["specs"].as_object() {
            for (k, s) in m {
                let Some(c) = s["p"]["color"].as_str().and_then(parse) else { continue };
                let tint = s["baseTint"].as_str().and_then(parse).unwrap_or([1.0; 3]);
                let atlas = s["atlasId"].is_string();
                let c = if atlas { tint } else { [c[0] * tint[0], c[1] * tint[1], c[2] * tint[2]] };
                out.insert(k.clone(), (c, atlas));
            }
        }
        out
    })
}

/// A painted finish's own colour as drawn without paint (linear), if it is one.
pub fn finish_colour(name: &str) -> Option<[f32; 3]> {
    let base = base_material_name(name);
    PAINTED_FINISHES.contains(&base).then(|| spec_colours().get(base).map(|c| c.0)).flatten()
}

/// The renderer's multiplier on a painted finish's material colour: primer /
/// its own colour (None: not a painted finish, or the switch is off).
pub fn primer_lift(name: &str) -> Option<[f32; 3]> {
    if !enabled() {
        return None;
    }
    let c = finish_colour(name)?;
    let p = lin(PRIMER_SRGB);
    Some([0, 1, 2].map(|k| p[k] / c[k].max(1e-3)))
}

/// What a material looks like unpainted, as drawn (linear): the primer for a
/// painted finish when the switch is on, else None (the caller's own colour).
pub fn drawn_base(name: &str) -> Option<[f32; 3]> {
    (enabled() && finish_colour(name).is_some()).then(|| lin(PRIMER_SRGB))
}

// ---------------------------------------------------------------------------
// Palettes (sRGB albedo targets, weights). Neutral, original values: painted
// stucco and timber of the kind the city's streets show, cool whites and pale
// greys first, pastels as accents, a few deeper colours.
// ---------------------------------------------------------------------------

type Entry = ([u8; 3], f32, Tone);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// whites and pale greys
    Pale,
    /// pastels
    Pastel,
    /// beiges and tans
    Earth,
    /// deeper colours
    Deep,
}

const STUCCO: [Entry; 17] = [
    ([236, 237, 234], 5.0, Tone::Pale),   // cool white
    ([233, 230, 221], 2.0, Tone::Pale),   // warm white
    ([214, 216, 215], 3.0, Tone::Pale),   // pale grey
    ([186, 189, 190], 2.0, Tone::Pale),   // light grey
    ([196, 204, 211], 1.8, Tone::Pastel), // blue grey
    ([204, 218, 229], 1.4, Tone::Pastel), // pale blue
    ([198, 209, 201], 1.0, Tone::Pastel), // muted coastal sage
    ([199, 204, 190], 0.9, Tone::Pastel), // natural sage
    ([237, 227, 186], 1.1, Tone::Pastel), // pale yellow
    ([235, 207, 188], 0.8, Tone::Pastel), // peach
    ([233, 208, 211], 0.7, Tone::Pastel), // pink
    ([222, 210, 190], 1.1, Tone::Earth),  // stone beige
    ([199, 182, 156], 0.5, Tone::Earth),  // tan
    ([160, 178, 196], 0.9, Tone::Deep),   // dusty blue
    ([158, 168, 158], 0.7, Tone::Deep),   // muted eucalyptus slate
    ([196, 146, 118], 0.5, Tone::Deep),   // terracotta
    ([142, 144, 147], 0.6, Tone::Deep),   // mid grey
];

/// Mixed-use and apartment fronts: whites, greys, creams, and muted SF colours.
const APARTMENT: [Entry; 12] = [
    ([234, 234, 231], 4.0, Tone::Pale),
    ([232, 227, 214], 2.5, Tone::Pale),
    ([208, 210, 209], 3.0, Tone::Pale),
    ([176, 179, 180], 1.5, Tone::Pale),
    ([194, 201, 207], 1.4, Tone::Pastel),
    ([233, 222, 184], 1.0, Tone::Pastel),
    ([204, 209, 199], 0.7, Tone::Pastel),
    ([226, 210, 192], 1.2, Tone::Earth),
    ([204, 188, 162], 0.9, Tone::Earth),
    ([156, 170, 182], 0.8, Tone::Deep),
    ([178, 128, 104], 0.45, Tone::Deep),
    ([140, 144, 141], 0.6, Tone::Deep),
];

/// Shopfront and civic painted walls.
const COMMERCIAL: [Entry; 11] = [
    ([232, 232, 229], 3.5, Tone::Pale),
    ([214, 214, 210], 3.0, Tone::Pale),
    ([184, 186, 186], 2.0, Tone::Pale),
    ([228, 220, 204], 1.8, Tone::Earth),
    ([204, 190, 166], 1.0, Tone::Earth),
    ([196, 204, 210], 0.9, Tone::Pastel),
    ([232, 218, 176], 0.8, Tone::Pastel),
    ([200, 214, 200], 0.5, Tone::Pastel),
    ([146, 150, 152], 0.8, Tone::Deep),
    ([168, 116, 96], 0.4, Tone::Deep),
    ([120, 140, 150], 0.4, Tone::Deep),
];

/// How a district shifts the draw: (pale, pastel, earth, deep) weight factors.
fn district_bias(district: Option<&str>) -> [f32; 4] {
    match district.unwrap_or("") {
        // Bright pastel and saturated streets.
        "Mission" | "Bernal Heights" | "Excelsior" | "Outer Mission" | "Portola" | "Visitacion Valley" | "Bayview Hunters Point" | "Oceanview/Merced/Ingleside" | "Potrero Hill" => [0.8, 1.7, 0.9, 1.8],
        // White and pale grey streets.
        "Marina" | "Pacific Heights" | "Presidio Heights" | "Seacliff" | "Russian Hill" | "Nob Hill" | "Inner Richmond" | "Outer Richmond" | "Lakeshore" => [1.4, 0.7, 1.0, 0.6],
        // The tract districts: whites with the pastel rows.
        "Sunset/Parkside" | "Inner Sunset" | "West of Twin Peaks" | "Twin Peaks" => [1.0, 1.3, 0.8, 0.8],
        _ => [1.0, 1.0, 1.0, 1.0],
    }
}

fn tone_index(t: Tone) -> usize {
    match t {
        Tone::Pale => 0,
        Tone::Pastel => 1,
        Tone::Earth => 2,
        Tone::Deep => 3,
    }
}

/// The palette an archetype draws its wall paint from (None: it keeps the port's paint).
pub fn palette_of(archetype: &str) -> Option<&'static [Entry]> {
    match archetype {
        "victorian-row" | "edwardian-flats" => None,
        "stucco-row" | "cottage-garage" | "midcentury-stucco" => Some(&STUCCO),
        "bay-window-apartment" => Some(&APARTMENT),
        _ => Some(&COMMERCIAL),
    }
}

fn unit(seed: u32, salt: &str) -> f32 {
    (hash_seed(&format!("{seed}|wall-paint|{salt}")) % 1_000_003) as f32 / 1_000_003.0
}

/// One building's wall paint: a weighted draw from the palette with the
/// district's bias, its value moved by -18 % to -2 % (weathering; a bake's grooves take a little more).
pub fn pick(palette: &[Entry], district: Option<&str>, seed: u32) -> [f32; 3] {
    let bias = district_bias(district);
    let total: f32 = palette.iter().map(|e| e.1 * bias[tone_index(e.2)]).sum();
    let mut at = unit(seed, "pick") * total;
    let mut c = palette[palette.len() - 1].0;
    for e in palette {
        at -= e.1 * bias[tone_index(e.2)];
        if at < 0.0 {
            c = e.0;
            break;
        }
    }
    let v = 0.82 + 0.16 * unit(seed, "value");
    lin(c).map(|x| x * v)
}

/// The tint that draws `target` on the primer.
fn on_primer(target: [f32; 3]) -> u32 {
    let p = lin(PRIMER_SRGB);
    linear_hex([0, 1, 2].map(|k| (target[k] / p[k]).clamp(0.0, 1.0)))
}

/// The neutral grey at a finish colour's luminance.
fn neutral_of(c: [f32; 3]) -> [f32; 3] {
    let l = luma(c);
    [l, l, l * 1.01]
}

/// Paint one building's walls (see the module notes). `materials`: the slots
/// as drawn; `tints`: the port's tints, rewritten in place.
pub fn apply(c: &Classified, materials: &BTreeMap<String, String>, own_tints: bool, tints: &mut Tints) {
    let wall = materials.get("wall").map(|s| s.as_str()).unwrap_or("");
    let wall_base = base_material_name(wall).to_string();
    let palette = palette_of(c.archetype);
    let chinatown = c.archetype == "bay-window-apartment" && own_tints;
    let district = c.civic.as_ref().and_then(|v| v.district.as_deref());
    let mut out = Tints::new();
    // The wall finish: the palette's paint, or the port's paint on the primer.
    // A Chinatown front's accent paint on its trim, the body from the apartment palette (the chroma rule).
    let accent_on_trim = chinatown && chroma_cap() && PAINTED_FINISHES.contains(&wall_base.as_str());
    if accent_on_trim {
        let accent = tints.get(&wall_base).copied().map(hex_linear);
        let trim = materials.get("trim").map(|t| base_material_name(t).to_string());
        if let (Some(accent), Some(trim)) = (accent, trim) {
            let own = crate::roof_measured::material_colour(&trim).unwrap_or([0.6; 3]);
            let want = capped_chroma(accent, 2.0 * WALL_CHROMA_MAX as f32);
            out.insert(trim, linear_hex([0, 1, 2].map(|k| (want[k] / own[k].max(1e-3)).clamp(0.0, 1.0))));
        }
    }
    if PAINTED_FINISHES.contains(&wall_base.as_str()) {
        let own = finish_colour(&wall_base).unwrap_or(lin(PRIMER_SRGB));
        let mut t = match palette {
            Some(p) if !chinatown || accent_on_trim => on_primer(pick(p, district, c.seed)),
            None if period_primer() => {
                // The period paint as the colour it names, its value moved as a palette draw's.
                let paint = tints.get(&wall_base).copied().map(hex_linear).unwrap_or([1.0; 3]);
                let v = 0.82 + 0.16 * unit(c.seed, "value");
                on_primer(paint.map(|x| x * v))
            }
            _ => {
                let paint = tints.get(&wall_base).copied().map(hex_linear).unwrap_or([1.0; 3]);
                let n = neutral_of(own);
                on_primer([0, 1, 2].map(|k| paint[k] * n[k]))
            }
        };
        if crate::typology::detail_enabled() {
            // Keep dark painted SF houses (charcoal, navy, deep slate) above 0.085 linear albedo so
            // shaded side/north elevations retain crisp siding & architectural detail instead of crushing pitch black.
            let p = lin(PRIMER_SRGB);
            let drawn = [0, 1, 2].map(|k| hex_linear(t)[k] * p[k]);
            let y = luma(drawn);
            if y < 0.085 {
                let lifted = if y > 1e-4 {
                    let k = 0.085 / y;
                    drawn.map(|v| (v * k).min(1.0))
                } else {
                    [0.085, 0.085, 0.088]
                };
                t = on_primer(lifted);
            }
        }
        out.insert(wall_base.clone(), capped_tint(t));
    }
    // Every other painted finish the building draws: its own paint (or none)
    // on the neutral grey of its old colour.
    for name in materials.values() {
        let base = base_material_name(name);
        if base == wall_base || out.contains_key(base) {
            continue;
        }
        let Some(own) = finish_colour(base) else { continue };
        let paint = tints.get(base).copied().map(hex_linear).unwrap_or([1.0; 3]);
        let n = neutral_of(own);
        out.insert(base.to_string(), on_primer([0, 1, 2].map(|k| paint[k] * n[k])));
    }
    for (k, v) in out {
        tints.insert(format!("{k}{FENESTRATED_SUFFIX}"), v);
        tints.insert(k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oklab_chroma(c: [f32; 3]) -> f32 {
        let l = rc::oklab(c);
        (l[1] * l[1] + l[2] * l[2]).sqrt()
    }

    #[test]
    fn every_painted_finish_has_a_colour_and_the_primer_is_reached() {
        for f in PAINTED_FINISHES {
            let c = finish_colour(f).unwrap_or_else(|| panic!("{f}: no spec colour"));
            assert!(luma(c) > 0.2, "{f}");
        }
        assert!(finish_colour("brick-running-bond").is_none());
        assert_eq!(finish_colour("painted-stucco+fenestrated"), finish_colour("painted-stucco"));
    }

    #[test]
    fn every_palette_colour_fits_under_the_primer() {
        for p in [&STUCCO[..], &APARTMENT[..], &COMMERCIAL[..]] {
            for e in p {
                for k in 0..3 {
                    assert!(e.0[k] <= PRIMER_SRGB[k], "{:?} over the primer", e.0);
                }
            }
        }
    }

    #[test]
    fn the_stucco_draw_is_neutral_on_average_and_varied() {
        // Over many seeds: the mean paint is near neutral (sRGB red minus
        // blue within 8; the round-1 city was +23), its chroma low on average
        // (pastel accents, not a tint everywhere), and not one colour.
        let n = 20_000;
        let (mut sum, mut light, mut chroma) = ([0f64; 3], 0usize, 0f64);
        let mut seen = std::collections::BTreeSet::new();
        for s in 0..n {
            let c = pick(&STUCCO, Some("Sunset/Parkside"), hash_seed(&format!("b{s}")));
            for k in 0..3 {
                sum[k] += c[k] as f64;
            }
            if rc::oklab(c)[0] > 0.9 {
                light += 1;
            }
            chroma += oklab_chroma(c) as f64;
            seen.insert(linear_hex(c) >> 4 & 0xf0f0f);
        }
        let m = sum.map(|v| (rc::linear_to_srgb((v / n as f64) as f32) * 255.0) as i32);
        assert!((m[0] - m[2]).abs() <= 8, "mean {m:?}");
        // Bright whites (OKLab L over 0.9) a real share, not the whole street.
        assert!(light > n / 8 && light < n / 2, "light share {}", light as f64 / n as f64);
        assert!(chroma / (n as f64) < 0.04, "mean chroma {}", chroma / n as f64);
        assert!(seen.len() > 20, "{} distinct paints", seen.len());
    }

    #[test]
    fn districts_shift_the_mix() {
        let deep = |d: &str| (0..5000).filter(|s| rc::oklab(pick(&STUCCO, Some(d), hash_seed(&format!("d{s}"))))[0] < 0.75).count();
        assert!(deep("Mission") > deep("Marina") * 3 / 2, "Mission {} Marina {}", deep("Mission"), deep("Marina"));
    }

    #[test]
    fn a_period_paint_draws_its_own_colour_on_the_primer() {
        if !enabled() || !period_primer() {
            return;
        }
        let c = Classified { archetype: "victorian-row", seed: 77, ..Default::default() };
        let mats: BTreeMap<String, String> = [("wall".to_string(), "wood-siding".to_string())].into();
        // White (no tint) and charcoal period paint on wood siding.
        for (paint, lo, hi) in [(None, 0.6, 0.9), (Some(0x5c6066u32), 0.07, 0.13)] {
            let mut tints = Tints::new();
            if let Some(p) = paint {
                tints.insert("wood-siding".into(), p);
            }
            apply(&c, &mats, true, &mut tints);
            let drawn = luma([0, 1, 2].map(|k| hex_linear(tints["wood-siding"])[k] * lin(PRIMER_SRGB)[k]));
            assert!(drawn > lo && drawn < hi, "{paint:?}: {drawn}");
        }
    }

    #[test]
    fn cielab_round_trips_and_the_cap_holds_lightness_and_hue() {
        let c = lin([0xb0, 0x45, 0x3c]); // the port's Chinatown scarlet
        let l = cielab(c);
        assert!(l[1].hypot(l[2]) > 45.0, "{l:?}");
        let back = cielab_to_linear(l);
        for k in 0..3 {
            assert!((back[k] - c[k]).abs() < 1e-4);
        }
        let capped = cielab(capped_chroma(c, 25.0));
        assert!((capped[1].hypot(capped[2]) - 25.0).abs() < 0.5, "{capped:?}");
        assert!((capped[0] - l[0]).abs() < 0.5);
        assert!((capped[2].atan2(capped[1]) - l[2].atan2(l[1])).abs() < 0.02);
        // A pale paint is untouched.
        let pale = lin([236, 237, 234]);
        assert_eq!(capped_chroma(pale, 25.0), pale);
        // White on D65 is L* 100, a* b* 0.
        let w = cielab([1.0; 3]);
        assert!((w[0] - 100.0).abs() < 0.01 && w[1].abs() < 0.01 && w[2].abs() < 0.01, "{w:?}");
    }

    #[test]
    fn a_chinatown_front_is_held_to_the_body_chroma() {
        if !enabled() || !chroma_cap() {
            return;
        }
        let c = Classified { archetype: "bay-window-apartment", seed: 9, ..Default::default() };
        let mats: BTreeMap<String, String> = [("wall".to_string(), "painted-stucco".to_string())].into();
        for paint in [0xb0453cu32, 0x4f8a6b, 0xd9b24a, 0x3f7f86] {
            let mut tints: Tints = [("painted-stucco".to_string(), paint)].into();
            apply(&c, &mats, true, &mut tints);
            let p = lin(PRIMER_SRGB);
            let t = hex_linear(tints["painted-stucco"]);
            let drawn = [0, 1, 2].map(|k| t[k] * p[k]);
            let l = cielab(drawn);
            assert!(l[1].hypot(l[2]) <= WALL_CHROMA_MAX as f32 + 1.5, "{paint:06x}: {l:?}");
        }
    }

    #[test]
    fn the_tint_draws_the_target_on_the_primer() {
        let target = lin([196, 204, 211]);
        let t = hex_linear(on_primer(target));
        let p = lin(PRIMER_SRGB);
        for k in 0..3 {
            assert!((t[k] * p[k] - target[k]).abs() < 0.01);
        }
    }

    #[test]
    fn the_lift_puts_every_finish_on_the_primer() {
        // SAFETY: tests in this module only read the switch.
        let p = lin(PRIMER_SRGB);
        if !enabled() {
            return;
        }
        for f in PAINTED_FINISHES {
            let l = primer_lift(f).unwrap();
            let c = finish_colour(f).unwrap();
            for k in 0..3 {
                assert!((l[k] * c[k] - p[k]).abs() < 1e-4);
            }
        }
        assert!(primer_lift("brick-running-bond").is_none());
    }
}
