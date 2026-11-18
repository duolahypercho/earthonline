//! Archetype classification (src/world/buildings/archetypes/classify.js, v3.6),
//! the district/era prior it reads (district-era-prior.js), and the loader's
//! program inference that decides a record's `usage` (src/citygen/sf-data.js
//! inferProgram / inferType / inferUsage).

use crate::frame::P2;
use crate::rng::{Rng, hash_seed};
use std::io::Read;
use std::sync::OnceLock;

pub const BAY_WINDOW_APARTMENT: &str = "bay-window-apartment";
pub const VICTORIAN_ROW: &str = "victorian-row";
pub const SOMA_WAREHOUSE: &str = "soma-warehouse";
pub const DOWNTOWN_TOWER: &str = "downtown-tower";
pub const MIDCENTURY_STUCCO: &str = "midcentury-stucco";
pub const RETAIL_STRIP: &str = "retail-strip";
pub const CIVIC_CONCRETE: &str = "civic-concrete";
pub const INDUSTRIAL_SHED: &str = "industrial-shed";
pub const STUCCO_ROW: &str = "stucco-row";
pub const EDWARDIAN_FLATS: &str = "edwardian-flats";
pub const COTTAGE_GARAGE: &str = "cottage-garage";

pub const ARCHETYPE_IDS: [&str; 11] = [
    BAY_WINDOW_APARTMENT,
    VICTORIAN_ROW,
    SOMA_WAREHOUSE,
    DOWNTOWN_TOWER,
    MIDCENTURY_STUCCO,
    RETAIL_STRIP,
    CIVIC_CONCRETE,
    INDUSTRIAL_SHED,
    STUCCO_ROW,
    EDWARDIAN_FLATS,
    COTTAGE_GARAGE,
];

pub const STOREY_METRES: f64 = 3.2;

const SHED_TAGS: &[&str] = &["industrial", "warehouse", "hangar", "shed", "service", "garage", "garages", "carport"];
const CIVIC_TAGS: &[&str] = &[
    "civic", "public", "government", "hospital", "school", "university", "college", "church", "cathedral", "chapel", "museum", "train_station", "transportation",
    "stadium",
];
const NON_TRADE_AMENITIES: &[&str] = &[
    "hotel", "hostel", "motel", "guest_house", "place_of_worship", "school", "college", "university", "kindergarten", "childcare", "library", "language_school",
    "driving_school", "music_school", "hospital", "clinic", "nursing_home", "social_facility", "shelter", "fire_station", "police", "townhall", "courthouse",
    "prison", "post_depot", "community_centre", "ngo", "embassy", "public_building", "parking", "parking_entrance", "parking_space", "bicycle_parking",
    "toilets", "shower", "waste_disposal", "waste_transfer_station", "recycling", "fuel", "charging_station", "car_wash", "grave_yard", "crematorium",
    "monastery", "museum", "theatre",
];
const COMMERCIAL_TAGS: &[&str] = &["retail", "commercial", "supermarket", "kiosk", "office"];

/// Signature buildings (the template is recorded; they keep the v2 archetype).
const SIGNATURE_IDS: &[(&str, &str)] = &[
    ("24222973", "transamerica"),
    ("431972186", "salesforce-tower"),
    ("542375290", "transit-center"),
    ("28824850", "coit-tower"),
    ("288371295", "palace-of-fine-arts"),
    ("32946942", "grace-cathedral"),
    ("288511106", "555-california"),
    ("256442760", "mission-dolores"),
    ("256442765", "mission-dolores"),
    ("32863779", "old-mint"),
    ("579646390", "chase-center"),
    ("30675038", "conservatory-of-flowers"),
    ("28695389", "cal-academy"),
    ("72963089", "millennium-tower"),
    ("445566153", "181-fremont"),
];

const INDUSTRIAL_CLASSES: &[&str] = &["I", "IW", "IX", "IZ", "IG", "IDC", "G", "GG", "GZ", "PLG"];
const CHURCH_CLASSES: &[&str] = &["W"];
const SCHOOL_CLASSES: &[&str] = &["E", "EG"];
const CIVIC_CLASSES: &[&str] = &["VG", "IXG", "OG", "FIRG", "LIBG", "UG", "CG", "ECG", "N1", "N1G", "POLG"];
const PARK_CLASSES: &[&str] = &["VPUB"];
const RETAIL_CLASSES: &[&str] = &["C", "CZ", "B", "S", "T", "CM", "FS", "F2"];
pub const STORE_CLASSES: &[&str] = &["C", "CZ", "B", "S", "T", "CM", "FS", "FS5", "F2", "AC", "HC"];
const RESIDENTIAL_USES: &[&str] = &["SRES", "MRES", "COMR"];
const COMMERCIAL_USES: &[&str] = &["COMM", "COMO", "COMH", "COMR"];
const CHURCH_TAGS: &[&str] = &["church", "cathedral", "chapel"];
const SCHOOL_TAGS: &[&str] = &["school", "university", "college", "kindergarten"];
const SCHOOL_AMENITIES: &[&str] = &["school", "college", "university", "kindergarten"];
const VICTORIAN_DISTRICTS: &[&str] = &[
    "Western Addition", "Haight Ashbury", "Hayes Valley", "Pacific Heights", "Mission", "Castro/Upper Market", "Noe Valley", "Potrero Hill", "Bernal Heights",
    "Japantown", "Lone Mountain/USF", "Russian Hill", "Nob Hill", "Glen Park", "Presidio Heights", "North Beach", "Inner Richmond", "Tenderloin", "South of Market",
];
const TRACT_DISTRICTS: &[&str] = &[
    "Sunset/Parkside", "Inner Sunset", "Outer Richmond", "Inner Richmond", "West of Twin Peaks", "Excelsior", "Oceanview/Merced/Ingleside", "Visitacion Valley",
    "Portola", "Outer Mission", "Bayview Hunters Point", "Lakeshore", "Marina", "Seacliff", "Twin Peaks",
];
const RICHMOND_DISTRICTS: &[&str] = &["Outer Richmond", "Inner Richmond"];
const DOWNTOWN_CORE_DISTRICTS: &[&str] = &["Financial District/South Beach", "Tenderloin"];
const MIXED_DISTRICTS: &[&str] = &["Chinatown", "North Beach"];
const ROLL_YEAR_PLACEHOLDER: f64 = 1900.0;
const TOWER_HEIGHT_M: f64 = 40.0;
const COTTAGE_HEIGHT_M: f64 = 5.0;
const SHED_HEIGHT_M: f64 = 4.0;
const ACCESSORY_MAX_HEIGHT_M: f64 = 5.0;
const ACCESSORY_MAX_AREA_M2: f64 = 150.0;
const ROW_HOUSE_MAX_APARTMENT_AREA_M2: f64 = 250.0;
const FLATS_MIN_AREA_M2: f64 = 180.0;
const STORE_BLOCK_MIN_AREA_M2: f64 = 400.0;
const STORE_BLOCK_MAX_STOREYS: f64 = 3.0;
const MINIMAL_TRADITIONAL_FROM_YEAR: f64 = 1925.0;
const PRIOR_ACCESSORY_MAX_HEIGHT_M: f64 = 4.5;
const PRIOR_ACCESSORY_MAX_AREA_M2: f64 = 60.0;

fn has(list: &[&str], v: &str) -> bool {
    list.contains(&v)
}
fn has_opt(list: &[&str], v: Option<&str>) -> bool {
    v.map(|v| list.contains(&v)).unwrap_or(false)
}

/// A recorded ground-floor business (tenancy units, building-metadata tiles).
#[derive(Clone, Debug, Default)]
pub struct TenancyUnit {
    pub category: String,
    pub subtype: String,
    pub edge: Option<i64>,
    pub along_m: Option<f64>,
    pub width_m: Option<f64>,
}

/// What `classifyBuilding` reads from a record.
#[derive(Clone, Debug, Default)]
pub struct ClassifyInput {
    /// `record.id` as the JS writes it into a template string.
    pub id_text: String,
    pub name: String,
    /// `record.building` (the OSM tag, or the loaded record's `usage`).
    pub building: String,
    pub amenity: String,
    pub shop: String,
    pub levels: f64,
    pub height: f64,
    pub area: f64,
    /// The record's points (closing duplicate kept, as the JS has them).
    pub points: Vec<P2>,
    pub trade_source: bool,
    pub year_built: f64,
    pub tenancy: Vec<TenancyUnit>,
}

/// `classified.civic` (publishCivic).
#[derive(Clone, Debug, Default)]
pub struct Civic {
    pub joined: bool,
    pub year_built: Option<f64>,
    pub year_exact: Option<bool>,
    pub district: Option<String>,
    pub units: Option<f64>,
    pub assessor_storeys: Option<f64>,
    pub property_class: Option<String>,
    pub use_code: Option<String>,
    pub pitched_roof: Option<bool>,
    pub roof_peak_m: Option<f64>,
    pub roof_median_m: Option<f64>,
    pub roof_ratio: Option<f64>,
    pub area_m2: Option<f64>,
    pub accessory: bool,
    pub secondary: bool,
    pub prior: bool,
}

/// A recorded shop unit placed on the record's ring (`recordedShopUnits`).
#[derive(Clone, Debug, Default)]
pub struct ShopUnit {
    pub edge: usize,
    pub along_m: f64,
    pub width_m: f64,
    pub x: f64,
    pub z: f64,
    pub category: String,
    pub subtype: String,
}

/// The classifier's answer (the fields the archetype modules read).
#[derive(Clone, Debug, Default)]
pub struct Classified {
    pub archetype: &'static str,
    pub template: String,
    pub variant: String,
    pub template_basis: String,
    pub reason: String,
    pub height: f64,
    pub height_source: &'static str,
    pub storeys: f64,
    pub seed: u32,
    pub area: f64,
    pub year_built: f64,
    pub year_exact: Option<bool>,
    pub tag: String,
    pub amenity: String,
    pub frontage: f64,
    pub bbox_short_side_m: f64,
    pub bbox_long_side_m: f64,
    pub depth: f64,
    pub has_retail_ground: bool,
    pub sign_texts: Vec<String>,
    pub sign_trade: Option<String>,
    pub trade_tag: String,
    pub record_name: String,
    pub shop_units: Vec<ShopUnit>,
    pub civic: Option<Civic>,
    pub record_id: String,
    /// The Presidio rule applies (presidio.rs).
    pub presidio: bool,
}

/// `resolveHeight(record)`.
pub fn resolve_height(height: f64, levels: f64) -> (f64, &'static str) {
    if height.is_finite() && height > 1.0 {
        return (height, "osm-height");
    }
    if levels.is_finite() && levels >= 1.0 {
        return (levels * STOREY_METRES, "osm-levels");
    }
    (2.0 * STOREY_METRES, "default")
}

pub fn resolve_storey_count(h: f64) -> f64 {
    crate::js::js_round(h / STOREY_METRES).clamp(1.0, 60.0)
}

/// `footprintSpan(points)`: bbox long and short side.
pub fn footprint_span(points: &[P2]) -> (f64, f64) {
    if points.len() < 3 {
        return (0.0, 0.0);
    }
    let (mut x0, mut x1, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in points {
        x0 = x0.min(p.x);
        x1 = x1.max(p.x);
        z0 = z0.min(p.z);
        z1 = z1.max(p.z);
    }
    let (a, b) = (x1 - x0, z1 - z0);
    (a.max(b), a.min(b))
}

fn recorded_street_trade(input: &ClassifyInput) -> bool {
    !input.tenancy.is_empty()
}

/// `classifyBuildingV2(record)`.
pub fn classify_v2(input: &ClassifyInput) -> Classified {
    let tag = if input.building.is_empty() { "yes".to_string() } else { input.building.to_lowercase() };
    let amenity = input.amenity.to_lowercase();
    let area = input.area;
    let (height, height_source) = resolve_height(input.height, input.levels);
    let storeys = resolve_storey_count(height);
    let (long, short) = footprint_span(&input.points);
    let seed = hash_seed(&format!("sf-building-{}", input.id_text));
    let year_built = input.year_built;
    let has_retail_ground = input.trade_source || has(COMMERCIAL_TAGS, &tag) || (!amenity.is_empty() && !has(NON_TRADE_AMENITIES, &amenity));
    let mut c = Classified {
        archetype: "",
        height,
        height_source,
        storeys,
        seed,
        area,
        year_built,
        tag: tag.clone(),
        amenity: amenity.clone(),
        frontage: short,
        bbox_short_side_m: short,
        bbox_long_side_m: long,
        depth: long,
        has_retail_ground,
        ..Default::default()
    };
    let decide = |c: &mut Classified, a: &'static str, reason: String| {
        c.archetype = a;
        c.reason = reason;
    };
    if has(SHED_TAGS, &tag) {
        decide(&mut c, INDUSTRIAL_SHED, format!("building={tag}"));
    } else if has(CIVIC_TAGS, &tag) || ["townhall", "courthouse", "library", "police", "fire_station"].contains(&amenity.as_str()) {
        let r = if has(CIVIC_TAGS, &tag) { format!("building={tag}") } else { format!("amenity={amenity}") };
        decide(&mut c, CIVIC_CONCRETE, r);
    } else if height >= 40.0 {
        decide(&mut c, DOWNTOWN_TOWER, format!("height {height:.1} m >= 40"));
    } else if storeys <= 1.0 && area >= 800.0 {
        decide(&mut c, INDUSTRIAL_SHED, "1 storey over area".into());
    } else if storeys <= 2.0 && (has(COMMERCIAL_TAGS, &tag) || !amenity.is_empty()) && short / eo_data::groundscale::scale() >= 10.0 { // unscaled span
        decide(&mut c, RETAIL_STRIP, "<=2 storeys, trade use".into());
    } else if (3.0..=8.0).contains(&storeys) && area >= 900.0 {
        decide(&mut c, SOMA_WAREHOUSE, "loft".into());
    } else if (2.0..=5.0).contains(&storeys) {
        if short / eo_data::groundscale::scale() <= 9.0 { // unscaled span
            decide(&mut c, VICTORIAN_ROW, "narrow".into());
        } else if storeys <= 3.0 {
            decide(&mut c, BAY_WINDOW_APARTMENT, "wide".into());
        } else if year_built > 0.0 && year_built < 1930.0 {
            decide(&mut c, BAY_WINDOW_APARTMENT, "wide, pre-1930".into());
        } else {
            decide(&mut c, MIDCENTURY_STUCCO, "wide".into());
        }
    } else if storeys > 5.0 {
        decide(&mut c, MIDCENTURY_STUCCO, "below tower height".into());
    } else {
        decide(&mut c, RETAIL_STRIP, "fallthrough".into());
    }
    c
}

// ---------------------------------------------------------------------------
// District/era prior.
// ---------------------------------------------------------------------------

pub const PRIOR_YEAR_QUANTILES: [f64; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];
const PRIOR_YEAR_BASE: f64 = 1850.0;
const NEIGHBOUR_RINGS: i64 = 2;

pub struct PriorStore {
    pub cell_m: f64,
    pub origin: [f64; 2],
    pub cols: i64,
    pub rows: i64,
    pub cells: usize,
    pub delta_bytes: usize,
    pub districts: Vec<String>,
    pub slots: Vec<i32>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Prior {
    pub district: Option<String>,
    pub year_quantiles: Option<[f64; 5]>,
    pub placeholder_share: f64,
    pub multi_unit_share: f64,
    pub pitched_share: f64,
    pub residential_share: f64,
    pub commercial_share: f64,
    pub industrial_share: f64,
    pub store_share: f64,
    pub secondary_share: f64,
}

fn decode_base64(text: &str) -> Vec<u8> {
    let mut map = [255u8; 256];
    for (i, c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
        map[*c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = map[c as usize];
        if v == 255 {
            continue;
        }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

impl PriorStore {
    /// Decode `district-era-prior.data.js` (the JS module's own text).
    pub fn from_js(text: &str) -> Option<Self> {
        let meta_at = text.find("DISTRICT_ERA_PRIOR_META = ")? + "DISTRICT_ERA_PRIOR_META = ".len();
        let meta_end = meta_at + text[meta_at..].find("};")? + 1;
        let meta: serde_json::Value = serde_json::from_str(&text[meta_at..meta_end]).ok()?;
        if meta["version"] != "sf-district-era-prior-v4" {
            return None;
        }
        let bytes_at = text.find("DISTRICT_ERA_PRIOR_BYTES = '")? + "DISTRICT_ERA_PRIOR_BYTES = '".len();
        let bytes_end = bytes_at + text[bytes_at..].find('\'')?;
        let raw = decode_base64(&text[bytes_at..bytes_end]);
        let mut bytes = vec![];
        flate2::read::DeflateDecoder::new(&raw[..]).read_to_end(&mut bytes).ok()?;
        let cols = meta["cols"].as_i64()?;
        let rows = meta["rows"].as_i64()?;
        let cells = meta["cells"].as_u64()? as usize;
        let mut slots = vec![-1i32; (cols * rows) as usize];
        let mut at = 0usize;
        let mut index: i64 = -1;
        for n in 0..cells {
            let mut delta: i64 = 0;
            let mut shift = 0;
            loop {
                let b = bytes[at];
                at += 1;
                delta |= ((b & 0x7f) as i64) << shift;
                shift += 7;
                if b & 0x80 == 0 {
                    break;
                }
            }
            index += delta;
            slots[index as usize] = n as i32;
        }
        Some(Self {
            cell_m: meta["cellM"].as_f64()?,
            origin: [meta["origin"][0].as_f64()?, meta["origin"][1].as_f64()?],
            cols,
            rows,
            cells,
            delta_bytes: meta["deltaBytes"].as_u64()? as usize,
            districts: meta["districts"].as_array()?.iter().map(|d| d.as_str().unwrap_or("").to_string()).collect(),
            slots,
            bytes,
        })
    }

    fn record(&self, n: usize) -> Prior {
        let field = |f: usize| self.bytes[self.delta_bytes + f * self.cells + n];
        let k = PRIOR_YEAR_QUANTILES.len();
        let mut years = [0.0; 5];
        let mut first_null = false;
        for (i, y) in years.iter_mut().enumerate() {
            let v = field(1 + i);
            if v == 0 {
                if i == 0 {
                    first_null = true;
                }
                *y = f64::NAN;
            } else {
                *y = PRIOR_YEAR_BASE + v as f64;
            }
        }
        let hi = |i: usize| (field(1 + k + i) >> 4) as f64 / 15.0;
        let lo = |i: usize| (field(1 + k + i) & 15) as f64 / 15.0;
        let d = field(0) as usize;
        Prior {
            district: if d >= 1 { self.districts.get(d - 1).cloned() } else { None },
            year_quantiles: if first_null { None } else { Some(years) },
            placeholder_share: hi(0),
            multi_unit_share: lo(0),
            pitched_share: hi(1),
            residential_share: lo(1),
            commercial_share: hi(2),
            industrial_share: lo(2),
            store_share: hi(3),
            secondary_share: lo(3),
        }
    }

    /// `districtEraPriorAt(x, z)`.
    pub fn at(&self, x: f64, z: f64) -> Option<Prior> {
        if !x.is_finite() || !z.is_finite() {
            return None;
        }
        let ci = ((x - self.origin[0]) / self.cell_m).floor() as i64;
        let cj = ((z - self.origin[1]) / self.cell_m).floor() as i64;
        for ring in 0..=NEIGHBOUR_RINGS {
            let mut best: i32 = -1;
            let mut best_d = i64::MAX;
            for dj in -ring..=ring {
                for di in -ring..=ring {
                    if di.abs().max(dj.abs()) != ring {
                        continue;
                    }
                    let (i, j) = (ci + di, cj + dj);
                    if i < 0 || j < 0 || i >= self.cols || j >= self.rows {
                        continue;
                    }
                    let slot = self.slots[(j * self.cols + i) as usize];
                    let d = di * di + dj * dj;
                    if slot >= 0 && d < best_d {
                        best = slot;
                        best_d = d;
                    }
                }
            }
            if best >= 0 {
                return Some(self.record(best as usize));
            }
        }
        None
    }
}

/// `priorYearFor(prior, u)`.
pub fn prior_year_for(prior: &Prior, u: f64) -> Option<f64> {
    let years = prior.year_quantiles?;
    let qs = PRIOR_YEAR_QUANTILES;
    if u <= qs[0] {
        return Some(years[0]);
    }
    for i in 1..qs.len() {
        if u <= qs[i] {
            return Some(crate::js::js_round(years[i - 1] + ((u - qs[i - 1]) / (qs[i] - qs[i - 1])) * (years[i] - years[i - 1])));
        }
    }
    Some(years[qs.len() - 1])
}

static PRIOR: OnceLock<Option<PriorStore>> = OnceLock::new();

/// The prior (`data/district-era-prior.data.js`, baked in), decoded once.
pub fn prior_store() -> Option<&'static PriorStore> {
    PRIOR.get_or_init(|| PriorStore::from_js(include_str!("../data/district-era-prior.data.js"))).as_ref()
}

/// The roll evidence a footprint without its own record takes from its block.
#[derive(Clone, Debug, Default)]
struct PriorCivic {
    district: String,
    year_built: f64,
    year_exact: bool,
    use_code: String,
    property_class: Option<String>,
    roof_median_m: f64,
    area_m2: f64,
    accessory: bool,
    pitched_roof: bool,
}

fn district_era_civic(input: &ClassifyInput, base: &Classified) -> Option<PriorCivic> {
    let pts = &input.points;
    if pts.len() < 3 {
        return None;
    }
    let (mut x, mut z) = (0.0, 0.0);
    for p in pts {
        x += p.x;
        z += p.z;
    }
    let n = pts.len() as f64;
    let gs = eo_data::groundscale::scale(); // the prior's cells are in the real (unscaled) frame
    let prior = prior_store()?.at(x / n / gs, z / n / gs)?;
    let district = prior.district.clone()?;
    let mut random = Rng::from_num(hash_seed(&format!("sf-district-era-{}", input.id_text)) as f64);
    let year_roll = random.next();
    let placeholder_roll = random.next();
    let use_roll = random.next();
    let roof_roll = random.next();
    let store_roll = random.next();
    let secondary_roll = random.next();
    let low_height = base.height;
    let area = base.area;
    let placeholder = prior.year_quantiles.is_none() || placeholder_roll < prior.placeholder_share;
    let year = if placeholder { ROLL_YEAR_PLACEHOLDER } else { prior_year_for(&prior, year_roll).unwrap_or(f64::NAN) };
    let trade = base.has_retail_ground || recorded_street_trade(input);
    let (res, com, ind) = (prior.residential_share, prior.commercial_share, prior.industrial_share);
    let store = store_roll < prior.store_share;
    let mut use_code = "MISC".to_string();
    let mut property_class = None;
    if use_roll < res {
        let multi = (base.storeys >= 3.0 && area >= FLATS_MIN_AREA_M2) || use_roll / res < prior.multi_unit_share;
        use_code = if trade || store { "COMR" } else if multi { "MRES" } else { "SRES" }.into();
        if store {
            property_class = Some("FS".into());
        }
    } else if use_roll < res + com {
        use_code = "COMM".into();
        if trade || store {
            property_class = Some("C".into());
        }
    } else if use_roll < res + com + ind {
        use_code = "IND".into();
    }
    Some(PriorCivic {
        district,
        year_built: year,
        year_exact: !placeholder,
        use_code,
        property_class,
        roof_median_m: low_height,
        area_m2: area,
        accessory: (low_height < PRIOR_ACCESSORY_MAX_HEIGHT_M && area < PRIOR_ACCESSORY_MAX_AREA_M2)
            || (secondary_roll < prior.secondary_share && low_height >= ACCESSORY_MAX_HEIGHT_M),
        pitched_roof: roof_roll < prior.pitched_share,
    })
}

fn is_secondary(accessory: bool, low_height: f64, area: f64) -> bool {
    if !accessory {
        return false;
    }
    if low_height >= ACCESSORY_MAX_HEIGHT_M {
        return true;
    }
    area > ACCESSORY_MAX_AREA_M2 && low_height >= SHED_HEIGHT_M
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

struct Decision {
    template: String,
    variant: String,
    reason: String,
    basis: String,
    archetype: &'static str,
}

fn template_archetype(template: &str, variant: &str, storeys: f64, district: Option<&str>) -> &'static str {
    match template {
        "cottage-garage" => COTTAGE_GARAGE,
        "industrial-shed" | "pier-shed" => INDUSTRIAL_SHED,
        "soma-warehouse" => SOMA_WAREHOUSE,
        "church" | "school" | "civic" | "park-structure" => CIVIC_CONCRETE,
        "downtown-tower" | "glass-midrise" => DOWNTOWN_TOWER,
        "chinatown-mixed" => {
            if storeys <= 2.0 {
                RETAIL_STRIP
            } else {
                BAY_WINDOW_APARTMENT
            }
        }
        "retail-strip" => RETAIL_STRIP,
        "podium-residential" | "midcentury-stucco" | "contemporary-infill" => MIDCENTURY_STUCCO,
        "art-deco-midrise" => {
            if variant == "apartment" {
                BAY_WINDOW_APARTMENT
            } else if has_opt(DOWNTOWN_CORE_DISTRICTS, district) {
                DOWNTOWN_TOWER
            } else {
                SOMA_WAREHOUSE
            }
        }
        "victorian-row" => VICTORIAN_ROW,
        "bay-window-apartment" => BAY_WINDOW_APARTMENT,
        "edwardian-flats" => EDWARDIAN_FLATS,
        "stucco-row" => STUCCO_ROW,
        _ => MIDCENTURY_STUCCO,
    }
}

fn decide_template(input: &ClassifyInput, base: &Classified, civic: Option<&PriorCivic>) -> Decision {
    let joined = civic.is_some();
    let cls = civic.and_then(|c| c.property_class.as_deref());
    let use_ = civic.map(|c| c.use_code.as_str());
    let district = civic.map(|c| c.district.as_str());
    let year = civic.map(|c| c.year_built).filter(|y| y.is_finite());
    let exact_year = year.filter(|_| civic.map(|c| c.year_exact).unwrap_or(true));
    let placeholder = year == Some(ROLL_YEAR_PLACEHOLDER) && civic.map(|c| !c.year_exact).unwrap_or(false);
    let low_height = civic.map(|c| c.roof_median_m).filter(|h| h.is_finite() && *h > 0.0).unwrap_or(base.height);
    let footprint_area = civic.map(|c| c.area_m2).filter(|a| a.is_finite() && *a > 0.0).unwrap_or(base.area);
    let secondary = civic.map(|c| is_secondary(c.accessory, low_height, footprint_area)).unwrap_or(false);
    let storeys = base.storeys; // a prior carries no assessor storeys
    let pick = |template: &str, variant: String, reason: String| Decision {
        archetype: template_archetype(template, &variant, storeys, district),
        template: template.into(),
        variant,
        reason,
        basis: "v3".into(),
    };
    if let Some((_, sig)) = SIGNATURE_IDS.iter().find(|(id, _)| *id == input.id_text) {
        return Decision { template: "signature".into(), variant: sig.to_string(), reason: format!("signature {sig}"), basis: "signature".into(), archetype: base.archetype };
    }
    let institutional = has_opt(CHURCH_CLASSES, cls)
        || has_opt(SCHOOL_CLASSES, cls)
        || has_opt(CIVIC_CLASSES, cls)
        || has_opt(PARK_CLASSES, cls)
        || has_opt(INDUSTRIAL_CLASSES, cls)
        || use_ == Some("IND")
        || use_ == Some("GOVT");
    if civic.map(|c| c.accessory).unwrap_or(false) && !institutional && !secondary {
        return if low_height < SHED_HEIGHT_M {
            pick("cottage-garage", "accessory-shed".into(), "accessory".into())
        } else {
            pick("cottage-garage", "accessory-cottage".into(), "accessory".into())
        };
    }
    if has(CHURCH_TAGS, &base.tag) || base.amenity == "place_of_worship" || has_opt(CHURCH_CLASSES, cls) {
        if base.archetype == CIVIC_CONCRETE || joined {
            return pick("church", if cls == Some("W") { "roll" } else { "osm" }.into(), "church".into());
        }
    }
    if has(SCHOOL_TAGS, &base.tag) || has(SCHOOL_AMENITIES, &base.amenity) || has_opt(SCHOOL_CLASSES, cls) {
        if base.archetype == CIVIC_CONCRETE || joined {
            return pick("school", if has_opt(SCHOOL_CLASSES, cls) { "roll" } else { "osm" }.into(), "school".into());
        }
    }
    let Some(civic) = civic else {
        return Decision { template: base.archetype.into(), variant: "v2".into(), reason: base.reason.clone(), basis: "no-civic".into(), archetype: base.archetype };
    };
    // Port block 9900 never applies to a prior (block null).
    if has_opt(INDUSTRIAL_CLASSES, cls) || (use_ == Some("IND") && !has_opt(CIVIC_CLASSES, cls)) {
        return if storeys >= 3.0 {
            pick("soma-warehouse", "loft".into(), "industrial".into())
        } else {
            let v = if cls.map(|c| c.starts_with('G')).unwrap_or(false) || cls == Some("PLG") { "garage" } else { "shed" };
            pick("industrial-shed", v.into(), "industrial".into())
        };
    }
    if has_opt(PARK_CLASSES, cls) {
        return pick("park-structure", slug(if civic.district.is_empty() { "park" } else { &civic.district }), "park".into());
    }
    if has_opt(CIVIC_CLASSES, cls) || use_ == Some("GOVT") {
        return pick("civic", slug(cls.or(use_).unwrap_or("")), "civic".into());
    }
    if base.height >= TOWER_HEIGHT_M {
        let era = match exact_year {
            None => "undated",
            Some(y) if y < 1946.0 => "prewar",
            Some(y) if y < 1980.0 => "modern",
            _ => "glass",
        };
        return pick("downtown-tower", era.into(), "tower".into());
    }
    if has_opt(MIXED_DISTRICTS, district)
        && (2.0..=6.0).contains(&storeys)
        && (district == Some("Chinatown") || has_opt(STORE_CLASSES, cls) || has_opt(COMMERCIAL_USES, use_))
    {
        return pick("chinatown-mixed", slug(district.unwrap_or("")), "mixed".into());
    }
    if storeys <= 2.0 && (has_opt(RETAIL_CLASSES, cls) || (base.has_retail_ground && use_ != Some("SRES"))) {
        let v = match cls {
            Some("S") => "gas-station",
            Some("B") => "bank",
            Some("T") => "theatre",
            Some("FS") | Some("F2") => "flat-and-store",
            _ => "storefront",
        };
        return pick("retail-strip", v.into(), "retail".into());
    }
    if storeys <= STORE_BLOCK_MAX_STOREYS
        && has_opt(STORE_CLASSES, cls)
        && footprint_area >= STORE_BLOCK_MIN_AREA_M2
        && exact_year.map(|y| (1906.0..1990.0).contains(&y)).unwrap_or(false)
        && use_ != Some("SRES")
    {
        let v = if matches!(cls, Some("FS") | Some("F2")) { "flat-and-store" } else { "storefront" };
        return pick("retail-strip", v.into(), "store block".into());
    }
    if let Some(y) = exact_year {
        if y >= 1990.0 && storeys >= 4.0 {
            let resi_cls = cls
                .map(|c| ["Z", "LZ", "ZBM", "ZEU", "A", "A5", "A15"].iter().any(|p| c.starts_with(p)))
                .unwrap_or(false);
            return if (has_opt(RESIDENTIAL_USES, use_) && use_ != Some("COMR")) || resi_cls {
                pick("podium-residential", if y >= 2005.0 { "contemporary" } else { "nineties" }.into(), "podium".into())
            } else {
                pick("glass-midrise", slug(use_.unwrap_or("mixed")), "glass".into())
            };
        }
        if (1920.0..=1940.0).contains(&y) && storeys >= 4.0 && use_ != Some("SRES") {
            return pick("art-deco-midrise", if use_ == Some("MRES") { "apartment" } else { "commercial" }.into(), "deco".into());
        }
    }
    if has_opt(RESIDENTIAL_USES, use_) || has_opt(COMMERCIAL_USES, use_) {
        if low_height < COTTAGE_HEIGHT_M && storeys <= 1.0 {
            return if exact_year.map(|y| y >= MINIMAL_TRADITIONAL_FROM_YEAR).unwrap_or(false) {
                pick("cottage-garage", "minimal-traditional".into(), "house".into())
            } else {
                pick("cottage-garage", "cottage".into(), "house".into())
            };
        }
        if exact_year.map(|y| y < 1906.0).unwrap_or(false) || (placeholder && (has_opt(VICTORIAN_DISTRICTS, district) || civic.pitched_roof)) {
            let v = match exact_year {
                None => "undated",
                Some(y) if y < 1880.0 => "italianate",
                Some(y) if y < 1890.0 => "stick",
                _ => "queen-anne",
            };
            return pick("victorian-row", v.into(), "victorian".into());
        }
        let era = if placeholder { Some(1915.0) } else { exact_year };
        if let Some(era) = era {
            if secondary && use_ == Some("MRES") && (1935.0..1980.0).contains(&era) {
                return pick("midcentury-stucco", "garden-apartment".into(), "garden".into());
            }
            if era < 1931.0 && storeys >= 4.0 {
                return pick("bay-window-apartment", if placeholder { "undated" } else { "dated" }.into(), "flats".into());
            }
            if era < 1925.0 {
                return pick("edwardian-flats", if placeholder { "undated" } else { "dated" }.into(), "edwardian".into());
            }
            let tall = storeys.max(base.storeys);
            // A prior's `units` is always null, which passes the units test.
            if era <= 1965.0 && tall >= 3.0 && footprint_area >= FLATS_MIN_AREA_M2 && use_ == Some("MRES") {
                return if era < 1946.0 {
                    pick("bay-window-apartment", "flats".into(), "flats".into())
                } else {
                    pick("midcentury-stucco", "apartment".into(), "flats".into())
                };
            }
            if era <= 1960.0 && tall >= 4.0 && use_ == Some("MRES") && footprint_area >= ROW_HOUSE_MAX_APARTMENT_AREA_M2 {
                return pick("midcentury-stucco", "apartment".into(), "apartment block".into());
            }
            if era < 1946.0 || (era <= 1960.0 && has_opt(TRACT_DISTRICTS, district)) {
                let v = if district == Some("Marina") {
                    "marina"
                } else if era < 1946.0 {
                    "interwar"
                } else {
                    "tract"
                };
                return pick("stucco-row", v.into(), "row".into());
            }
            if era < 1980.0 {
                let v = if has_opt(RICHMOND_DISTRICTS, district) && era >= 1960.0 && storeys >= 3.0 {
                    "richmond-special"
                } else if use_ == Some("SRES") && storeys <= 2.0 {
                    "postwar-house"
                } else {
                    "apartment"
                };
                return pick("midcentury-stucco", v.into(), "postwar".into());
            }
            return pick("contemporary-infill", if use_ == Some("SRES") { "house" } else { "flats" }.into(), "infill".into());
        }
    }
    Decision { template: base.archetype.into(), variant: "v2".into(), reason: base.reason.clone(), basis: "fallthrough".into(), archetype: base.archetype }
}

/// `classifyBuilding(record)` (v3).
pub fn classify(input: &ClassifyInput) -> Classified {
    let base = classify_v2(input);
    let prior = district_era_civic(input, &base);
    let decision = decide_template(input, &base, prior.as_ref());
    let mut c = base.clone();
    c.archetype = decision.archetype;
    c.template = decision.template;
    c.variant = decision.variant;
    c.template_basis = if prior.is_some() && decision.basis == "v3" { "prior".into() } else { decision.basis };
    c.reason = decision.reason;
    let store_class = prior.as_ref().and_then(|p| p.property_class.as_deref()).map(|c| STORE_CLASSES.contains(&c)).unwrap_or(false);
    c.has_retail_ground = base.has_retail_ground || store_class || recorded_street_trade(input);
    c.trade_tag = if !input.shop.is_empty() { input.shop.clone() } else { input.amenity.clone() };
    c.record_name = input.name.clone();
    c.shop_units = recorded_shop_units(input);
    // tenancy-resolver.js unitTradeWord; tenantSignTexts stays empty (sf-tenant-signs data does not ship here).
    c.sign_trade = input.tenancy.iter().find_map(|u| {
        let key = format!("{}:{}", u.category, u.subtype);
        sign_trade_from_unit(&key).or_else(|| sign_trade_from_unit(&u.category)).map(|s| s.to_string())
    });
    c.civic = prior.as_ref().map(|p| {
        let secondary = is_secondary(p.accessory, p.roof_median_m, p.area_m2);
        Civic {
            joined: true,
            year_built: p.year_built.is_finite().then_some(p.year_built),
            year_exact: Some(p.year_exact),
            district: Some(p.district.clone()),
            units: None,
            assessor_storeys: None,
            property_class: p.property_class.clone(),
            use_code: Some(p.use_code.clone()),
            pitched_roof: Some(p.pitched_roof),
            roof_peak_m: None,
            roof_median_m: Some(p.roof_median_m),
            roof_ratio: None,
            area_m2: Some(p.area_m2),
            accessory: p.accessory && !secondary,
            secondary,
            prior: true,
        }
    });
    let exact = c.civic.as_ref().map(|v| v.year_built.is_some() && v.year_exact != Some(false)).unwrap_or(false);
    if exact {
        c.year_built = c.civic.as_ref().unwrap().year_built.unwrap();
        c.year_exact = Some(true);
    } else {
        c.year_exact = if c.civic.as_ref().map(|v| v.year_exact == Some(false)).unwrap_or(false) { Some(false) } else { None };
    }
    c
}

/// `SIGN_TRADE_FROM_UNIT` (tenancy-resolver.js).
fn sign_trade_from_unit(key: &str) -> Option<&'static str> {
    Some(match key {
        "cafe" | "restaurant:limited-service" => "CAFE",
        "restaurant" => "RESTAURANT",
        "bar" => "BAR",
        "supermarket" => "GROCERY",
        "convenience" | "retail:shop" | "retail:misc-store" | "retail:general-merchandise" => "MARKET",
        "retail:specialty-food" | "retail:food-store" => "DELI",
        "retail:bakery" => "BAKERY",
        "retail:hardware-garden" => "HARDWARE",
        "retail:dry-cleaner" | "retail:laundromat" => "CLEANERS",
        "retail:salon-personal-care" | "retail:personal-service" => "BARBER",
        "retail:hobby-books-sport" => "BOOKS",
        "retail:fitness" => "STUDIO",
        "retail:apparel" => "TAILOR",
        _ => return None,
    })
}

fn recorded_shop_units(input: &ClassifyInput) -> Vec<ShopUnit> {
    let pts = &input.points;
    let count = pts.len();
    let mut out = vec![];
    for u in &input.tenancy {
        let Some(edge) = u.edge.filter(|e| *e >= 0 && (*e as usize) < count) else { continue };
        let e = edge as usize;
        let (a, b) = (pts[e], pts[(e + 1) % count]);
        let len = (b.x - a.x).hypot(b.z - a.z);
        let len = if len == 0.0 { 1.0 } else { len };
        let along_in = u.along_m.filter(|v| *v != 0.0 && v.is_finite()).unwrap_or(len / 2.0);
        let along = len.min(along_in.max(0.0));
        out.push(ShopUnit {
            edge: e,
            along_m: along,
            width_m: u.width_m.filter(|v| v.is_finite()).unwrap_or(0.0),
            x: a.x + ((b.x - a.x) * along) / len,
            z: a.z + ((b.z - a.z) * along) / len,
            category: u.category.clone(),
            subtype: u.subtype.clone(),
        });
    }
    out
}

/// Archetype modules that draw a dwelling.
const DWELLING_ARCHETYPES: &[&str] = &[STUCCO_ROW, EDWARDIAN_FLATS, VICTORIAN_ROW, BAY_WINDOW_APARTMENT, MIDCENTURY_STUCCO, COTTAGE_GARAGE];

/// `priorDwelling(record)` on a raw record (shop, amenity, trade and tenancy withheld).
pub fn prior_dwelling(input: &ClassifyInput) -> bool {
    let bare = ClassifyInput { shop: String::new(), amenity: String::new(), trade_source: false, tenancy: vec![], ..input.clone() };
    let c = classify(&bare);
    c.template_basis == "prior"
        && DWELLING_ARCHETYPES.contains(&c.archetype)
        && !c.civic.as_ref().and_then(|v| v.property_class.as_deref()).map(|p| STORE_CLASSES.contains(&p)).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// sf-data.js inferProgram / inferType / inferUsage.
// ---------------------------------------------------------------------------

const PROGRAM_UNTAGGED_HALL_AREA: f64 = 3000.0;
const PROGRAM_BIG_BOX_AREA: f64 = 1800.0;
const PROGRAM_HALL_AMENITIES: &[&str] =
    &["conference_centre", "exhibition_centre", "events_venue", "arts_centre", "theatre", "cinema", "stadium", "arena", "bus_station", "ferry_terminal"];
const PROGRAM_HALL_BUILDINGS: &[&str] = &[
    "hall", "exhibition_centre", "conference_centre", "convention_centre", "stadium", "sports_hall", "sports_centre", "grandstand", "pavilion", "hangar",
    "train_station", "transportation", "terminal", "riding_hall",
];
const PROGRAM_WAREHOUSE_BUILDINGS: &[&str] = &["warehouse", "industrial", "factory", "manufacture", "depot", "storage", "service", "works"];
const PROGRAM_RESIDENTIAL_BUILDINGS: &[&str] =
    &["apartments", "residential", "house", "detached", "terrace", "dormitory", "semidetached_house", "bungalow", "houseboat", "cabin", "static_caravan"];

/// `PROGRAM_INHABITED_BUILDINGS` (the browser build's sf-data.js).
const PROGRAM_INHABITED_BUILDINGS: &[&str] = &[
    "apartments", "residential", "house", "detached", "terrace", "dormitory", "hotel", "motel", "hostel", "office", "mixed_use", "semidetached_house",
    "bungalow", "cabin", "static_caravan", "school", "university", "college", "hospital", "church", "cathedral", "chapel", "civic", "government", "public",
];

/// The raw source record (`building` here is the OSM tag).
pub struct RawRecord<'a> {
    pub id_text: &'a str,
    pub building: &'a str,
    pub amenity: &'a str,
    pub name: &'a str,
    pub levels: Option<f64>,
    pub height: Option<f64>,
    pub area: Option<f64>,
    pub points: &'a [P2],
}

/// `inferProgram(building, area)`.
pub fn infer_program(raw: &RawRecord, area: f64) -> &'static str {
    let amenity = raw.amenity.to_lowercase();
    let tag = raw.building.to_lowercase();
    let footprint = area;
    if tag == "parking" || tag == "carport" || amenity == "parking" {
        return "parking";
    }
    if has(PROGRAM_HALL_AMENITIES, &amenity) || has(PROGRAM_HALL_BUILDINGS, &tag) {
        return "hall";
    }
    if has(PROGRAM_WAREHOUSE_BUILDINGS, &tag) {
        return "warehouse";
    }
    let retail = tag == "retail" || tag == "commercial" || tag == "supermarket" || amenity == "marketplace";
    if amenity == "mall" {
        return "big-box";
    }
    if retail && footprint >= PROGRAM_BIG_BOX_AREA {
        return "big-box";
    }
    if has(PROGRAM_RESIDENTIAL_BUILDINGS, &tag) {
        return "residential";
    }
    if footprint >= PROGRAM_UNTAGGED_HALL_AREA && !PROGRAM_INHABITED_BUILDINGS.contains(&tag.as_str()) && amenity.is_empty() {
        return "hall";
    }
    if tag.is_empty() || tag == "yes" {
        let input = ClassifyInput {
            id_text: raw.id_text.to_string(),
            name: raw.name.to_string(),
            building: raw.building.to_string(),
            levels: raw.levels.unwrap_or(f64::NAN),
            height: raw.height.unwrap_or(f64::NAN),
            area: raw.area.unwrap_or(0.0),
            points: raw.points.to_vec(),
            year_built: 0.0,
            ..Default::default()
        };
        if prior_dwelling(&input) {
            return "residential";
        }
    }
    ""
}

/// `inferType(building, program)`.
pub fn infer_type(raw: &RawRecord, program: &str) -> &'static str {
    match program {
        "hall" => return "civic",
        "big-box" => return "shop",
        "parking" | "warehouse" => return "warehouse",
        _ => {}
    }
    const CIVIC_AMENITIES: &[&str] = &[
        "school", "university", "college", "hospital", "place_of_worship", "library", "townhall", "courthouse", "fire_station", "police", "community_centre",
        "public_building",
    ];
    if !raw.amenity.is_empty() && has(CIVIC_AMENITIES, raw.amenity) {
        return "civic";
    }
    if raw.name == "Transamerica Pyramid" {
        return "civic";
    }
    if raw.building.contains("warehouse") {
        return "warehouse";
    }
    let levels = raw.levels.unwrap_or(0.0);
    if levels >= 7.0 || raw.height.unwrap_or(0.0) >= 24.0 {
        return "tower";
    }
    if raw.building.contains("retail") {
        return "shop";
    }
    if program == "residential" {
        return "rowhouse";
    }
    if raw.building.contains("residential") { "rowhouse" } else { "midrise" }
}

/// `inferUsage(type, program)`.
pub fn infer_usage(type_: &str, program: &str) -> &'static str {
    if program == "residential" {
        return "residential";
    }
    match type_ {
        "tower" => "office",
        "rowhouse" => "residential",
        "civic" => "civic",
        "warehouse" => "industrial",
        "shop" | "retail" => "retail",
        _ => "mixed",
    }
}
