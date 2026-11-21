//! The street overhead (props2): trolley and streetcar contact wires, the
//! support and junction span poles and span wires that hold them, bracket
//! arms and hangers, signal controller cabinets, the kerbside regulatory sign
//! picket (and the plate stacks strapped to support poles), overhead electric
//! distribution (timber poles with cobra-head luminaires, primary, secondary
//! and communication conductors, down guys, service drops) and the Chinatown
//! lantern strings: a Rust port of `planStreetOverhead`
//! (src/render/passes/street-overhead.js) and the helpers it reads
//! (street-rails.js `streetcarTrackIndex` / `matchStreetcarRuns` /
//! `streetcarLinkRanges`, overhead-distribution.js).
//!
//! Every California value comes from `eo_data::CaStandards` (`caMetric`): the
//! GO 95 span-wire floor of the communication cables and lantern strings, the
//! SF residential luminaire mounting height, the sidewalk throughway zone of
//! `sidewalkBand`. Every JS design value (`STREET_OVERHEAD`,
//! `WIRE_RADIUS_BY_KIND`, `SIGN_VARIANTS`, `POLE_SIGN_VARIANTS`,
//! `STREETCAR_OVERHEAD`, the pole asset's wire seats, the source tables) comes
//! from `data/overhead-js.json.gz`, written by the earlier browser version's exporter
//! with the SHA-256 of each JS source. The algorithm's own inline numbers
//! (nudge lists, steps, hash thresholds) are written here as the JS writes them.
//!
//! The JS plans a 210 m window round the camera, in segments-by-distance
//! order, under instance caps. This plans the whole city per 128 m street tile,
//! deterministically, with no caps: each element is owned by one tile (a pole,
//! cabinet or sign by its position, a wire by the midpoint of its end points; a
//! contact wire is cut into one piece per support span). Where the JS lets an
//! earlier element in its window order block a later one, this uses a fixed
//! order instead: a support pole keeps clear of its own run's supports; a
//! corner pole of the supports of the node's runs; the sign picket of its own
//! run's posts, the supports (and their plate stacks) of its run and of the
//! runs at both its nodes, and those nodes' corner poles and cabinets; a
//! distribution run keeps the lower-quartile pole spacing from the unchecked
//! poles of every lower-numbered distribution run it meets at a node; the
//! streetcar track coverage is taken in plan order.
//!
//! Inputs, as the eo-props port has them: the signal poles, heads and mast
//! arms of `signals` (the JS `props` and `drawnHardware`), the SFMTA sign posts
//! of `signs` and the surveyed trees and palms of `trees` (the JS
//! `drawnProps`). Not ported: the reference-observed block overlays' poles,
//! spans, webs, banners, fences and lots (section 2c; their per-street wire
//! flags are), furniture-drawn cabinets (`drawnCabinets`), the pole-base grime
//! discs, the instance and triangle caps, the camera window.

use crate::PropsWorld;
use crate::{Model, Prop};
use anyhow::{Context, Result};
use eo_street::plan::{Entry, RampWin, segment_kerb_ramps};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};

pub const OVERHEAD_JS_GZ: &[u8] = include_bytes!("../data/overhead-js.json.gz");

// ---------------------------------------------------------------- output

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PoleKind {
    /// A mid-block support pole (holds a span wire or a bracket arm).
    Support,
    /// A junction span pole on a corner.
    Span,
    /// An overhead distribution pole (the `utilityPole-v1.glb` asset).
    Distribution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PoleMaterial {
    Steel,
    Wood,
}

/// The cobra-head luminaire a distribution pole carries (`cobraArm-v1.glb`).
#[derive(Clone, Copy, Debug)]
pub struct PoleArm {
    pub pos: [f64; 3],
    /// three.js yaw; the arm (glTF +X) points out over the street.
    pub yaw: f64,
}

#[derive(Clone, Debug)]
pub struct OverheadPole {
    pub kind: PoleKind,
    pub material: PoleMaterial,
    /// Foot of the pole (x, footway y, z).
    pub pos: [f64; 3],
    /// three.js yaw (distribution poles: the crossarm, glTF X, lies along (cos yaw, -sin yaw)).
    pub yaw: f64,
    pub height: f64,
    /// Tapered tube radii (`createOverheadPoleGeometry`); the distribution pole asset's shaft radius both.
    pub radius_base: f64,
    pub radius_top: f64,
    pub arm: Option<PoleArm>,
    /// The JS id (`overhead-pole:<segment>:support:<k>:<side>`, `overhead-pole:<node>:<corner>`, `distribution-pole:...`).
    pub id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WireKind {
    Contact,
    Span,
    Hanger,
    Bracket,
    Primary,
    Secondary,
    Comm,
    Drop,
    Guy,
    /// A Chinatown lantern string (the JS draws it as a `drop`).
    LanternString,
}

impl WireKind {
    pub fn js_name(self) -> &'static str {
        match self {
            WireKind::Contact => "contact",
            WireKind::Span => "span",
            WireKind::Hanger => "hanger",
            WireKind::Bracket => "bracket",
            WireKind::Primary => "primary",
            WireKind::Secondary => "secondary",
            WireKind::Comm => "comm",
            WireKind::Drop | WireKind::LanternString => "drop",
            WireKind::Guy => "guy",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireSystem {
    None,
    Trolley,
    Streetcar,
}

#[derive(Clone, Debug)]
pub struct Wire {
    pub kind: WireKind,
    pub system: WireSystem,
    /// World points of the sampled catenary (the JS's own samples).
    pub points: Vec<[f32; 3]>,
    /// True radius (`WIRE_RADIUS_BY_KIND`); the ribbon widens to a pixel floor and fades by the same ratio.
    pub radius_m: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Cabinet {
    /// Base centre (x, footway y, z).
    pub pos: [f64; 3],
    /// three.js yaw: local +Z (the door face) looks along the footway.
    pub yaw: f64,
    /// Width (local X), depth (local Z), height.
    pub size: [f64; 3],
    pub node: u32,
}

#[derive(Clone, Debug)]
pub struct PlacedPlate {
    /// Plate centre in world space (on the post's +Z face).
    pub centre: [f64; 3],
    pub w: f64,
    pub h: f64,
    /// Band colour name (`SIGN_COLOURS`), fill colour name, printed face id, rules count.
    pub band: Option<String>,
    pub fill: Option<String>,
    pub face: Option<String>,
    pub rules: u32,
}

/// A kerbside sign assembly (`createSignAssemblyGeometry`): a 55 mm square post
/// (none on a pole-strapped stack) with its plates on the +Z face.
#[derive(Clone, Debug)]
pub struct SignAssembly {
    /// Base centre (x, footway y, z).
    pub pos: [f64; 3],
    pub yaw: f64,
    /// Uniform instance scale.
    pub scale: f64,
    /// Index into `SIGN_VARIANTS` (`OverheadJs::sign_variants`).
    pub variant: u16,
    /// Post height (0 for a pole-strapped stack), unscaled.
    pub post_height: f64,
    /// Strapped to an overhead support pole (no post of its own).
    pub on_pole: bool,
    pub plates: Vec<PlacedPlate>,
}

/// A signalised corner's plate cluster on its host pole (`createCornerSignGeometry`).
#[derive(Clone, Copy, Debug)]
pub struct CornerSign {
    /// The round plate's centre.
    pub pos: [f64; 3],
    pub yaw: f64,
    /// Hung on a signal mast (the JS `host` is a published prop), else on a span pole.
    pub on_signal_pole: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Lantern {
    /// Foot of the lantern asset.
    pub pos: [f64; 3],
    pub yaw: f64,
}

#[derive(Clone, Debug, Default)]
pub struct OverheadTile {
    pub poles: Vec<OverheadPole>,
    pub wires: Vec<Wire>,
    pub cabinets: Vec<Cabinet>,
    pub signs: Vec<SignAssembly>,
    pub corner_signs: Vec<CornerSign>,
    pub lanterns: Vec<Lantern>,
}

// ---------------------------------------------------------------- JS data

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignPlateSpec {
    pub w: f64,
    pub h: f64,
    pub y: f64,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub band: Option<String>,
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub face: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub rules: Option<u32>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignVariant {
    pub id: String,
    pub post_height_m: f64,
    pub plates: Vec<SignPlateSpec>,
    #[serde(default)]
    pub printed: bool,
    #[serde(default)]
    pub fence: Option<String>,
}

#[derive(Deserialize, Clone, Copy, Debug)]
pub struct Seat {
    pub arm: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Deserialize)]
struct Overlay {
    #[serde(rename = "streetIds")]
    street_ids: Vec<String>,
    axis: Value,
    #[serde(rename = "trolleyWire")]
    trolley_wire: bool,
    #[serde(rename = "steelPoles")]
    steel_poles: bool,
    #[serde(rename = "extraContactLanes")]
    extra_contact_lanes: Vec<f64>,
}

/// `data/overhead-js.json.gz`.
pub struct OverheadJs {
    pub sources: BTreeMap<String, String>,
    pub consts: Value,
    pub tables: Value,
    pub sign_variants: Vec<SignVariant>,
    pub seats: Vec<Seat>,
}

impl OverheadJs {
    pub fn load() -> Result<Self> {
        let mut s = String::new();
        flate2::read::GzDecoder::new(OVERHEAD_JS_GZ).read_to_string(&mut s).context("overhead-js.json.gz")?;
        let v: Value = serde_json::from_str(&s).context("overhead-js.json")?;
        let sources: BTreeMap<String, String> = serde_json::from_value(v["sources"].clone())?;
        let sign_variants: Vec<SignVariant> = serde_json::from_value(v["consts"]["signVariants"].clone())?;
        let seats: Vec<Seat> = serde_json::from_value(v["consts"]["utilityPole"]["attach"].clone())?;
        let mut tables = v["tables"].clone();
        // groundscale (the street size, 1.25 by default): the streetcar track samples at
        // their scaled positions (the observed overlays read only their axis direction).
        if eo_data::groundscale::active() {
            for k in ["xs", "zs"] {
                eo_data::groundscale::scale_json_numbers(&mut tables["streetcar"][k]);
            }
        }
        Ok(Self { sources, consts: v["consts"].clone(), tables, sign_variants, seats })
    }
    /// `STREET_OVERHEAD[key]`.
    pub fn h(&self, key: &str) -> f64 {
        self.consts["streetOverhead"][key].as_f64().unwrap_or_else(|| panic!("overhead-js: STREET_OVERHEAD.{key}"))
    }
    pub fn h_list(&self, key: &str) -> Vec<f64> {
        self.consts["streetOverhead"][key].as_array().unwrap_or_else(|| panic!("overhead-js: STREET_OVERHEAD.{key}")).iter().filter_map(|v| v.as_f64()).collect()
    }
    pub fn wire_radius(&self, kind: WireKind) -> f32 {
        self.consts["wireRadiusByKind"][kind.js_name()].as_f64().unwrap_or(0.014) as f32
    }
    /// The distant distribution pole's crossarm boxes (`createDistantUtilityPoleGeometry`): (minX, maxX, y, z) per arm.
    pub fn crossarms(&self) -> Vec<[f64; 4]> {
        let mut arms: BTreeMap<i32, [f64; 4]> = BTreeMap::new();
        for s in &self.seats {
            let a = arms.entry(s.arm).or_insert([f64::INFINITY, f64::NEG_INFINITY, s.y, s.z]);
            a[0] = a[0].min(s.x);
            a[1] = a[1].max(s.x);
        }
        arms.into_values().collect()
    }
}

/// The California values the overhead reads (`caMetric`).
#[derive(Clone, Debug)]
pub struct OverheadStandards {
    /// GO 95 Rule 37 Table 1 Case 3 Col A: span wires, guys and messengers over urban thoroughfares.
    pub comm_floor_m: f64,
    /// SF Public Works Standard Plan 87,205 residential pole shaft height.
    pub luminaire_height_m: f64,
}

impl OverheadStandards {
    pub fn load(ca: &eo_data::CaStandards) -> Result<Self> {
        Ok(Self {
            comm_floor_m: ca.metric_key("overhead-clearance-urban-thoroughfare", "spanWireGuyMessenger")?,
            luminaire_height_m: ca.metric_key("streetlight-sf-residential-pole", "shaftHeight")?,
        })
    }
}

/// `STREET_OVERHEAD`, named.
struct H {
    min_class_rank: i32,
    contact_height: f64,
    pair_half_gap: f64,
    support_spacing: f64,
    support_sag: f64,
    sample_step: f64,
    min_run: f64,
    hardware_clear: f64,
    hardware_vertical_clear: f64,
    hardware_influence: f64,
    hardware_lateral_budget: f64,
    span_height: f64,
    span_sag_per_metre: f64,
    span_sag_min: f64,
    span_sag_max: f64,
    span_min_width: f64,
    span_max_width: f64,
    mid_span_hanger: f64,
    nudges: Vec<f64>,
    insets: Vec<f64>,
    mid_span_prop_gap: f64,
    bracket_max_reach: f64,
    wood_share: f64,
    wood_r_top: f64,
    wood_r_base: f64,
    pole_sign_share: f64,
    pole_height: f64,
    pole_r_top: f64,
    pole_r_base: f64,
    pole_footway_min: f64,
    pole_kerb_insets: Vec<f64>,
    prop_clearance: f64,
    mast_reuse_radius: f64,
    mast_min_height: f64,
    cabinet: [f64; 3],
    cabinet_kerb_gap: f64,
    cabinet_footway_min: f64,
    cabinet_stations: Vec<f64>,
    cabinet_clearance: f64,
    sign_min_class_rank: i32,
    sign_pitch: [f64; 2],
    sign_inset: [f64; 2],
    sign_post_section: f64,
    sign_footway_min: f64,
    sign_end_margin: f64,
    sign_ramp_clear: f64,
    sign_prop_clearance: f64,
    sign_spacing: f64,
    corner_sign_height: f64,
    comm_cables: usize,
    guy_lead_ratio: f64,
    join_spacings: f64,
    pole_sign_variants: [u16; 2],
    // STREETCAR_OVERHEAD
    sc_sample_step: f64,
    sc_grid_cell: f64,
    sc_match_step: f64,
    sc_match_reach: f64,
    sc_held_share_max: f64,
    sc_match_min_dot: f64,
    sc_match_min_share: f64,
    sc_same_lane: f64,
    sc_trolley_share: f64,
    sc_link_max: f64,
}

impl H {
    fn new(js: &OverheadJs) -> Self {
        let sc = |k: &str| js.consts["streetcarOverhead"][k].as_f64().unwrap_or_else(|| panic!("STREETCAR_OVERHEAD.{k}"));
        let d = |k: &str| js.consts["streetOverhead"]["distribution"][k].as_f64().unwrap_or_else(|| panic!("STREET_OVERHEAD.distribution.{k}"));
        let psv = js.consts["poleSignVariants"].as_array().expect("POLE_SIGN_VARIANTS");
        Self {
            min_class_rank: js.h("minClassRank") as i32,
            contact_height: js.h("contactHeightM"),
            pair_half_gap: js.h("contactPairHalfGapM"),
            support_spacing: js.h("supportSpacingM"),
            support_sag: js.h("supportSagM"),
            sample_step: js.h("sampleStepM"),
            min_run: js.h("minRunM"),
            hardware_clear: js.h("hardwareClearM"),
            hardware_vertical_clear: js.h("hardwareVerticalClearM"),
            hardware_influence: js.h("hardwareInfluenceM"),
            hardware_lateral_budget: js.h("hardwareLateralBudgetM"),
            span_height: js.h("spanHeightM"),
            span_sag_per_metre: js.h("spanSagPerMetre"),
            span_sag_min: js.h("spanSagMinM"),
            span_sag_max: js.h("spanSagMaxM"),
            span_min_width: js.h("spanMinWidthM"),
            span_max_width: js.h("spanMaxWidthM"),
            mid_span_hanger: js.h("midSpanHangerM"),
            nudges: js.h_list("midSpanNudgesM"),
            insets: js.h_list("midSpanInsetsM"),
            mid_span_prop_gap: js.h("midSpanPropGapM"),
            bracket_max_reach: js.h("bracketMaxReachM"),
            wood_share: js.h("woodPoleShare"),
            wood_r_top: js.h("woodPoleRadiusTopM"),
            wood_r_base: js.h("woodPoleRadiusBaseM"),
            pole_sign_share: js.h("poleSignShare"),
            pole_height: js.h("poleHeightM"),
            pole_r_top: js.h("poleRadiusTopM"),
            pole_r_base: js.h("poleRadiusBaseM"),
            pole_footway_min: js.h("poleFootwayMinM"),
            pole_kerb_insets: js.h_list("poleKerbInsetsM"),
            prop_clearance: js.h("propClearanceM"),
            mast_reuse_radius: js.h("mastReuseRadiusM"),
            mast_min_height: js.h("mastMinHeightM"),
            cabinet: [js.h("cabinetWidthM"), js.h("cabinetDepthM"), js.h("cabinetHeightM")],
            cabinet_kerb_gap: js.h("cabinetKerbGapM"),
            cabinet_footway_min: js.h("cabinetFootwayMinM"),
            cabinet_stations: js.h_list("cabinetStationsM"),
            cabinet_clearance: js.h("cabinetClearanceM"),
            sign_min_class_rank: js.h("signMinClassRank") as i32,
            sign_pitch: [js.h("signPitchMinM"), js.h("signPitchMaxM")],
            sign_inset: [js.h("signKerbInsetMinM"), js.h("signKerbInsetMaxM")],
            sign_post_section: js.h("signPostSectionM"),
            sign_footway_min: js.h("signFootwayMinM"),
            sign_end_margin: js.h("signEndMarginM"),
            sign_ramp_clear: js.h("signRampClearM"),
            sign_prop_clearance: js.h("signPropClearanceM"),
            sign_spacing: js.h("signSpacingM"),
            corner_sign_height: js.h("cornerSignHeightM"),
            comm_cables: d("commCables") as usize,
            guy_lead_ratio: d("guyLeadRatio"),
            join_spacings: d("joinSpacings"),
            pole_sign_variants: [psv[0].as_u64().unwrap() as u16, psv[1].as_u64().unwrap() as u16],
            sc_sample_step: sc("sampleStepM"),
            sc_grid_cell: sc("gridCellM"),
            sc_match_step: sc("matchStepM"),
            sc_match_reach: sc("matchReachM"),
            sc_held_share_max: sc("heldShareMax"),
            sc_match_min_dot: sc("matchMinDot"),
            sc_match_min_share: sc("matchMinShare"),
            sc_same_lane: sc("sameLaneM"),
            sc_trolley_share: sc("trolleyShareM"),
            sc_link_max: sc("linkMaxM"),
        }
    }
}

// ---------------------------------------------------------------- helpers

/// `overheadHash01(key)`: FNV-1a, then one xorshift round.
pub fn overhead_hash01(key: &str) -> f64 {
    let mut h: u32 = 0x811c9dc5;
    for c in key.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545f491);
    h ^= h >> 13;
    (h % 1000003) as f64 / 1000003.0
}

fn hash_range(key: &str, lo: f64, hi: f64) -> f64 {
    lo + (hi - lo) * overhead_hash01(key)
}

/// `sampleAlong(points, cum, s)`: point and unit tangent (no mitre).
fn sample_along(points: &[[f64; 2]], cum: &[f64], s: f64) -> (f64, f64, f64, f64) {
    let last = cum[cum.len() - 1];
    let t = s.clamp(0.0, last);
    let mut i = 1;
    while i < cum.len() - 1 && cum[i] < t {
        i += 1;
    }
    let (a, b) = (points[i - 1], points[i]);
    let mut span = cum[i] - cum[i - 1];
    if span == 0.0 {
        span = 1e-6;
    }
    let f = (t - cum[i - 1]) / span;
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let mut l = dx.hypot(dz);
    if l == 0.0 {
        l = 1e-6;
    }
    (a[0] + dx * f, a[1] + dz * f, dx / l, dz / l)
}

/// `sagFraction(station, runStart, spanLength)`.
fn sag_fraction(station: f64, run_start: f64, span_length: f64) -> f64 {
    let local = (station - run_start) / span_length;
    let u = (local - local.floor()).clamp(0.0, 1.0);
    4.0 * u * (1.0 - u)
}

/// `projectToSegment`: (s, lateral, distance).
fn project_to_segment(points: &[[f64; 2]], cum: &[f64], x: f64, z: f64) -> (f64, f64, f64) {
    let mut best = (0.0, 0.0, f64::INFINITY);
    for i in 0..points.len().saturating_sub(1) {
        let (a, b) = (points[i], points[i + 1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let mut l2 = dx * dx + dz * dz;
        if l2 == 0.0 {
            l2 = 1e-9;
        }
        let l = l2.sqrt();
        let t = (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0);
        let (px, pz) = (a[0] + dx * t, a[1] + dz * t);
        let d = (x - px).hypot(z - pz);
        if d >= best.2 {
            continue;
        }
        best = (cum[i] + l * t, (x - px) * -(dz / l) + (z - pz) * (dx / l), d);
    }
    best
}

fn f3(x: f64, y: f64, z: f64) -> [f32; 3] {
    [x as f32, y as f32, z as f32]
}

/// A plain point grid (cell 8 m).
#[derive(Default)]
struct Grid {
    cells: HashMap<(i32, i32), Vec<u32>>,
}

const GRID_M: f64 = 8.0;

impl Grid {
    fn push(&mut self, i: usize, x: f64, z: f64) {
        self.cells.entry(((x / GRID_M).floor() as i32, (z / GRID_M).floor() as i32)).or_default().push(i as u32);
    }
    fn near(&self, x: f64, z: f64, r: f64, mut f: impl FnMut(usize)) {
        for gz in ((z - r) / GRID_M).floor() as i32..=((z + r) / GRID_M).floor() as i32 {
            for gx in ((x - r) / GRID_M).floor() as i32..=((x + r) / GRID_M).floor() as i32 {
                if let Some(v) = self.cells.get(&(gx, gz)) {
                    for &i in v {
                        f(i as usize);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- inputs

/// A published signal pole (`props` of the JS: `{x, z, heightM}`).
#[derive(Clone, Copy)]
struct SignalPoleIn {
    x: f64,
    z: f64,
    h: f64,
}

/// A drawn furniture footprint (`drawnProps`): surveyed trees and palms, SFMTA sign posts.
#[derive(Clone, Copy)]
struct DrawnIn {
    x: f64,
    z: f64,
    /// `STREET_FURNITURE_KINDS[kind].radius`.
    radius: f64,
    /// `drawnFurnitureFootprints` clearance.
    clearance: f64,
    tree: bool,
}

/// Drawn signal hardware (`drawnSignalHardware`).
#[derive(Clone, Copy)]
struct HwIn {
    a: [f64; 2],
    b: [f64; 2],
    y0: f64,
    y1: f64,
    radius: f64,
}

#[derive(Default)]
struct InputTile {
    signal: Vec<SignalPoleIn>,
    drawn: Vec<DrawnIn>,
    hardware: Vec<HwIn>,
}

/// The inputs gathered round one area, gridded.
#[derive(Default)]
struct Inputs {
    signal: Vec<SignalPoleIn>,
    signal_grid: Grid,
    drawn: Vec<DrawnIn>,
    drawn_grid: Grid,
    hardware: Vec<HwIn>,
}

impl Inputs {
    fn any_signal_within(&self, x: f64, z: f64, r: f64) -> bool {
        let mut hit = false;
        self.signal_grid.near(x, z, r, |i| {
            let p = self.signal[i];
            if (p.x - x).hypot(p.z - z) < r {
                hit = true;
            }
        });
        hit
    }
    fn nearest_signal_within(&self, x: f64, z: f64, r: f64) -> Option<(SignalPoleIn, f64)> {
        let mut best: Option<(SignalPoleIn, f64)> = None;
        self.signal_grid.near(x, z, r, |i| {
            let p = self.signal[i];
            let d = (p.x - x).hypot(p.z - z);
            if d <= r && best.is_none_or(|b| d < b.1) {
                best = Some((p, d));
            }
        });
        best
    }
}

// ---------------------------------------------------------------- per-entry and per-node plans

struct TrackRun {
    track: f64,
    lane: f64,
}

struct Supports {
    contact: Vec<Wire>,
    poles: Vec<OverheadPole>,
    wires: Vec<Wire>,
    signs: Vec<SignAssembly>,
}

struct EntryRest {
    picket: Vec<SignAssembly>,
    lantern_wires: Vec<Wire>,
    lanterns: Vec<Lantern>,
}

#[derive(Default)]
struct NodePlan {
    poles: Vec<OverheadPole>,
    spans: Vec<Wire>,
    corner_signs: Vec<CornerSign>,
    cabinet: Option<Cabinet>,
}

#[derive(Clone)]
struct DistPole {
    id: String,
    x: f64,
    z: f64,
    base_y: f64,
    yaw: f64,
    rx: f64,
    rz: f64,
}

#[derive(Clone)]
struct Chain {
    entry: u32,
    side: i8,
    compass: char,
    name: String,
    poles: Vec<DistPole>,
}

struct DistEntry {
    chains: Vec<Chain>,
}

struct DistInfo {
    compass: char,
    mapped: bool,
}

/// The city-wide overhead index (built once per street world).
pub struct Overhead {
    pub js: Arc<OverheadJs>,
    pub std: OverheadStandards,
    h: H,
    /// Per entry: `segmentCarriesTrolleyWire` (None: no source id).
    wired: Vec<Option<bool>>,
    /// Per entry: the observed overlay whose street ids name it.
    overlay: Vec<Option<usize>>,
    overlays: Vec<Overlay>,
    /// Per entry: the source piece (the one whose id the run carries).
    src: Vec<usize>,
    /// Per entry: the stable hash key (segment key, else id).
    key: Vec<String>,
    dist: Vec<Option<DistInfo>>,
    lantern_way: Vec<bool>,
    /// Per entry: streetcar tracks matched along it (street-rails `matchStreetcarRuns`, entry order).
    streetcar: Vec<Vec<TrackRun>>,
    /// Streetcar link wires (`streetcarLinkRanges`), by owner tile.
    links: HashMap<(i32, i32), Vec<Wire>>,
    /// Distribution entries by street name.
    dist_by_name: HashMap<String, Vec<u32>>,
    buildings: Vec<DropBuilding>,
    building_grid: HashMap<(i32, i32), Vec<u32>>,
    input_cache: Mutex<HashMap<(i32, i32), Arc<InputTile>>>,
    node_signal_cache: Mutex<HashMap<u32, Arc<Vec<Prop>>>>,
    tree_cache: Mutex<HashMap<u32, Arc<Vec<Prop>>>>,
    support_cache: Mutex<HashMap<u32, Arc<Supports>>>,
    rest_cache: Mutex<HashMap<u32, Arc<EntryRest>>>,
    node_cache: Mutex<HashMap<u32, Arc<NodePlan>>>,
    dist_raw_cache: Mutex<HashMap<u32, Arc<DistEntry>>>,
    dist_cache: Mutex<HashMap<u32, Arc<DistEntry>>>,
    inputs_cache: Mutex<HashMap<u32, Arc<Inputs>>>,
    input_ns: std::sync::atomic::AtomicU64,
    /// Per entry: its distribution poles and the wires it owns.
    dist_wire_cache: Mutex<HashMap<u32, Arc<(Vec<OverheadPole>, Vec<Wire>)>>>,
    pub build_ms: f64,
}

/// A building a service drop can land on.
pub struct DropBuilding {
    pub ring: Vec<[f64; 2]>,
    pub height: f64,
    pub levels: f64,
}

impl DropBuilding {
    /// From the loader's buildings: height as resolved, and the JS record's
    /// `stories` (sf-data.js: a measured storey count, which the native data
    /// does not carry, else `max(1, round(height / 3.2))`).
    pub fn from_city(buildings: &[eo_data::Building]) -> Vec<DropBuilding> {
        buildings.iter().map(Self::from_building).collect()
    }
    pub fn from_building(b: &eo_data::Building) -> DropBuilding {
        let h = b.height as f64;
        DropBuilding { ring: b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect(), height: h, levels: (js_round(h / 3.2) as f64).max(1.0) }
    }
}

const BUILDING_CELL_M: f64 = 64.0;

fn cached<K: std::hash::Hash + Eq + Copy, V>(m: &Mutex<HashMap<K, Arc<V>>>, k: K, cap: usize, f: impl FnOnce() -> V) -> Arc<V> {
    if let Some(v) = m.lock().unwrap().get(&k) {
        return v.clone();
    }
    let v = Arc::new(f());
    let mut c = m.lock().unwrap();
    if c.len() > cap {
        c.clear();
    }
    c.entry(k).or_insert(v).clone()
}

fn street_way_id(street_id: Option<&str>) -> Option<u64> {
    let s = street_id?.strip_prefix("sf-street-")?;
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

impl Overhead {
    pub fn new(w: &PropsWorld, js: Arc<OverheadJs>, std: OverheadStandards, buildings: Vec<DropBuilding>) -> Self {
        let t0 = std::time::Instant::now();
        let h = H::new(&js);
        let plan = &w.street.plan;
        let segs = &w.street.city.segments;
        let tables = &js.tables;
        let wire_yes: HashSet<u64> = tables["trolleyWireYes"].as_array().unwrap().iter().filter_map(|v| v.as_u64()).collect();
        let mut dist_ways: HashMap<u64, u64> = HashMap::new();
        let dw = tables["distributionWays"].as_array().unwrap();
        for i in (0..dw.len().saturating_sub(1)).step_by(2) {
            dist_ways.insert(dw[i].as_u64().unwrap(), dw[i + 1].as_u64().unwrap());
        }
        let lantern_ways: HashSet<u64> = tables["lanternWays"].as_array().unwrap().iter().filter_map(|v| v.as_u64()).collect();
        let overlays: Vec<Overlay> = serde_json::from_value(tables["observed"].clone()).unwrap_or_default();
        let n = plan.entries.len();
        let mut src = Vec::with_capacity(n);
        let mut key = Vec::with_capacity(n);
        let mut wired = Vec::with_capacity(n);
        let mut overlay = Vec::with_capacity(n);
        let mut dist = Vec::with_capacity(n);
        let mut lantern_way = Vec::with_capacity(n);
        for e in &plan.entries {
            let si = e.pieces.iter().copied().find(|&p| segs[p].id == e.id).unwrap_or(e.road);
            let s = &segs[si];
            src.push(si);
            key.push(s.key.clone().unwrap_or_else(|| e.id.clone()));
            let sid = s.street_id.as_deref();
            let ov = sid.and_then(|id| overlays.iter().position(|o| o.street_ids.iter().any(|x| x == id)));
            overlay.push(ov);
            let way = street_way_id(sid);
            wired.push(way.map(|id| ov.is_some_and(|o| overlays[o].trolley_wire) || wire_yes.contains(&id)));
            dist.push(way.and_then(|id| dist_ways.get(&id)).map(|&code| DistInfo {
                compass: match code & 7 {
                    1 => 'N',
                    2 => 'S',
                    3 => 'E',
                    4 => 'W',
                    5 => 'B',
                    _ => '?',
                },
                mapped: code & 8 != 0,
            }));
            lantern_way.push(way.is_some_and(|id| lantern_ways.contains(&id)));
            // (a structure's leg into a junction, eo-street joins, is not drawn: no wires on it.)
            if e.structure_leg.is_some() {
                *wired.last_mut().unwrap() = None;
                *dist.last_mut().unwrap() = None;
                *lantern_way.last_mut().unwrap() = false;
            }
        }
        let mut o = Self {
            js: js.clone(),
            std,
            h,
            wired,
            overlay,
            overlays,
            src,
            key,
            dist,
            lantern_way,
            streetcar: vec![],
            links: HashMap::new(),
            dist_by_name: HashMap::new(),
            buildings,
            building_grid: HashMap::new(),
            input_cache: Mutex::new(HashMap::new()),
            node_signal_cache: Mutex::new(HashMap::new()),
            tree_cache: Mutex::new(HashMap::new()),
            support_cache: Mutex::new(HashMap::new()),
            rest_cache: Mutex::new(HashMap::new()),
            node_cache: Mutex::new(HashMap::new()),
            dist_raw_cache: Mutex::new(HashMap::new()),
            dist_cache: Mutex::new(HashMap::new()),
            inputs_cache: Mutex::new(HashMap::new()),
            input_ns: std::sync::atomic::AtomicU64::new(0),
            dist_wire_cache: Mutex::new(HashMap::new()),
            build_ms: 0.0,
        };
        let (runs, links) = build_streetcar(&o.h, &o.js, w);
        o.streetcar = runs;
        o.links = links;
        o.build_distribution_gate(w);
        for (bi, b) in o.buildings.iter().enumerate() {
            if let Some(p) = b.ring.first() {
                o.building_grid.entry(((p[0] / BUILDING_CELL_M).floor() as i32, (p[1] / BUILDING_CELL_M).floor() as i32)).or_default().push(bi as u32);
            }
        }
        o.build_ms = t0.elapsed().as_secs_f64() * 1e3;
        o
    }

    fn street_name<'a>(&self, w: &'a PropsWorld, ei: usize) -> &'a str {
        &w.street.city.segments[self.src[ei]].street_name
    }

    /// The street key of an entry's run (`segment.id` in the JS hashes).
    pub fn entry_key(&self, ei: usize) -> &str {
        &self.key[ei]
    }

    // ---- distribution gate (`opensOntoUnderground`) ------------------------
    // (streetcar: `build_streetcar` below)
}

// ---- streetcar (street-rails.js) ------------------------------------------

#[rustfmt::skip]
fn build_streetcar(h: &H, js: &OverheadJs, w: &PropsWorld) -> (Vec<Vec<TrackRun>>, HashMap<(i32, i32), Vec<Wire>>) {
        let sc = &js.tables["streetcar"];
        let arr = |k: &str| -> Vec<f64> { sc[k].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect() };
        let (xs, zs, txs, tzs, track_of) = (arr("xs"), arr("zs"), arr("txs"), arr("tzs"), arr("trackOf"));
        let ranges: Vec<(f64, usize, usize)> = sc["ranges"].as_array().unwrap().iter().map(|r| (r[0].as_f64().unwrap(), r[1].as_u64().unwrap() as usize, r[2].as_u64().unwrap() as usize)).collect();
        let cell = h.sc_grid_cell;
        let mut cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for i in 0..xs.len() {
            cells.entry(((xs[i] / cell).floor() as i64, (zs[i] / cell).floor() as i64)).or_default().push(i as u32);
        }
        let near = |x: f64, z: f64, r: f64, f: &mut dyn FnMut(usize)| {
            for gz in ((z - r) / cell).floor() as i64..=((z + r) / cell).floor() as i64 {
                for gx in ((x - r) / cell).floor() as i64..=((x + r) / cell).floor() as i64 {
                    if let Some(l) = cells.get(&(gx, gz)) {
                        for &i in l {
                            f(i as usize);
                        }
                    }
                }
            }
        };
        const COARSE_M: f64 = 64.0;
        let coarse: HashSet<(i64, i64)> = (0..xs.len()).map(|i| ((xs[i] / COARSE_M).floor() as i64, (zs[i] / COARSE_M).floor() as i64)).collect();
        let mut covered = vec![0u8; xs.len()];
        let mut runs_of: Vec<Vec<TrackRun>> = Vec::with_capacity(w.street.plan.entries.len());
        for e in &w.street.plan.entries {
            let mut out = vec![];
            let ok = e.points.len() >= 2 && e.cum.len() == e.points.len() && e.structure_leg.is_none();
            let total = if ok { e.cum[e.cum.len() - 1] } else { 0.0 };
            let run_start = total.min(e.trim_start.max(0.0));
            let run_end = run_start.max(total - e.trim_end.max(0.0));
            let run = run_end - run_start;
            if !ok || run < h.min_run {
                runs_of.push(out);
                continue;
            }
            // `matchStreetcarRuns`.
            let half = if e.half > 0.0 { e.half.max(0.8) } else { 3.2 };
            let reach = half + h.sc_match_reach;
            let stations = 2usize.max((run / h.sc_match_step).ceil() as usize + 1);
            // Quick reject: no track sample in a 64 m cell the run passes.
            let mut any = false;
            'probe: for i in 0..e.points.len() - 1 {
                let (a, b) = (e.points[i], e.points[i + 1]);
                let n = 1 + ((b[0] - a[0]).hypot(b[1] - a[1]) / 32.0).ceil() as usize;
                for k in 0..=n {
                    let t = k as f64 / n as f64;
                    let (x, z) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                    let c = ((x / COARSE_M).floor() as i64, (z / COARSE_M).floor() as i64);
                    for dz in -1..=1 {
                        for dx in -1..=1 {
                            if coarse.contains(&(c.0 + dx, c.1 + dz)) {
                                any = true;
                                break 'probe;
                            }
                        }
                    }
                }
            }
            if !any {
                runs_of.push(out);
                continue;
            }
            // track -> (laterals, first, last), in first-hit order.
            let mut hits: Vec<(f64, Vec<f64>, usize, usize)> = vec![];
            for k in 0..stations {
                let (px, pz, tx, tz) = sample_along(&e.points, &e.cum, run_start + run * k as f64 / (stations - 1) as f64);
                let mut best: Vec<(f64, f64, f64, usize)> = vec![]; // (id, along, lateral, i)
                near(px, pz, reach + h.sc_sample_step, &mut |i| {
                    let dot = tx * txs[i] + tz * tzs[i];
                    if dot.abs() < h.sc_match_min_dot {
                        return;
                    }
                    let (ex, ez) = (xs[i] - px, zs[i] - pz);
                    let along = ex * tx + ez * tz;
                    if along.abs() > h.sc_sample_step {
                        return;
                    }
                    let lateral = ex * -tz + ez * tx;
                    if lateral.abs() > reach {
                        return;
                    }
                    let id = track_of[i];
                    match best.iter_mut().find(|b| b.0 == id) {
                        Some(b) => {
                            if along.abs() < b.1.abs() {
                                *b = (id, along, lateral, i);
                            }
                        }
                        None => best.push((id, along, lateral, i)),
                    }
                });
                for b in best {
                    match hits.iter_mut().find(|x| x.0 == b.0) {
                        Some(x) => {
                            x.1.push(b.2);
                            x.2 = x.2.min(b.3);
                            x.3 = x.3.max(b.3);
                        }
                        None => hits.push((b.0, vec![b.2], b.3, b.3)),
                    }
                }
            }
            let mut runs: Vec<(f64, f64, f64, usize, usize)> = vec![]; // (id, lane, share, first, last)
            for (id, lats, first, last) in hits {
                let share = lats.len() as f64 / stations as f64;
                if share < h.sc_match_min_share {
                    continue;
                }
                let held: f64 = (first..=last).map(|i| covered[i] as f64).sum();
                if held > h.sc_held_share_max * (last - first + 1) as f64 {
                    continue;
                }
                let mut sorted = lats.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                runs.push((id, sorted[sorted.len() / 2], share, first, last));
            }
            runs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap().then(a.0.partial_cmp(&b.0).unwrap()));
            let mut kept: Vec<(f64, f64, f64, usize, usize)> = vec![];
            for r in runs {
                if !kept.iter().any(|k| (k.1 - r.1).abs() < h.sc_same_lane) {
                    kept.push(r);
                }
            }
            kept.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            for r in &kept {
                for c in covered.iter_mut().take(r.4 + 1).skip(r.3) {
                    *c = 1;
                }
                out.push(TrackRun { track: r.0, lane: r.1 });
            }
            runs_of.push(out);
        }
        // `streetcarLinkRanges` over the whole city.
        let join_m = 2.5;
        let held_near = |i: usize| {
            let mut held = false;
            near(xs[i], zs[i], join_m, &mut |j| {
                if !held && covered[j] != 0 && (xs[j] - xs[i]).hypot(zs[j] - zs[i]) <= join_m {
                    held = true;
                }
            });
            held
        };
        let radius = js.wire_radius(WireKind::Contact);
        let mut links: HashMap<(i32, i32), Vec<Wire>> = HashMap::new();
        for &(_, first, last) in &ranges {
            let mut i = first;
            while i <= last {
                if covered[i] != 0 {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < last && covered[j + 1] == 0 {
                    j += 1;
                }
                let length = (j - i + 2) as f64 * h.sc_sample_step;
                let start_held = i > first || held_near(i);
                let end_held = j < last || held_near(j);
                if length <= h.sc_link_max && start_held && end_held {
                    let (from, to) = (if i > first { i - 1 } else { i }, if j < last { j + 1 } else { j });
                    if to > from {
                        let points: Vec<[f32; 3]> = (from..=to).map(|k| f3(xs[k], road_datum(w, xs[k], zs[k]) + h.contact_height, zs[k])).collect();
                        let wire = Wire { kind: WireKind::Contact, system: WireSystem::Streetcar, points, radius_m: radius };
                        links.entry(owner_of(&wire)).or_default().push(wire);
                    }
                }
                i = j + 1;
            }
        }
        (runs_of, links)
}

/// The JS `roadDatumAt`: terrain + `roadLift`.
fn road_datum(w: &PropsWorld, x: f64, z: f64) -> f64 {
    w.street.terrain.ground(x, z) + w.street.o.road_lift
}

impl Overhead {
    fn build_distribution_gate(&mut self, w: &PropsWorld) {
        let entries = &w.street.plan.entries;
        let end_key = |name: &str, p: [f64; 2]| format!("{}|{}|{}", name, js_round(p[0]), js_round(p[1]));
        let mut underground: HashMap<String, Vec<u32>> = HashMap::new();
        let mut strung: HashMap<String, Vec<u32>> = HashMap::new();
        let ends = |ei: usize| -> [String; 2] {
            let e = &entries[ei];
            let name = self.street_name(w, ei);
            [end_key(name, e.points[0]), end_key(name, e.points[e.points.len() - 1])]
        };
        for (ei, e) in entries.iter().enumerate() {
            if self.street_name(w, ei).is_empty() || e.points.len() < 2 || e.structure_leg.is_some() {
                continue;
            }
            let into = if self.dist[ei].is_some() { &mut strung } else { &mut underground };
            for k in ends(ei) {
                into.entry(k).or_default().push(ei as u32);
            }
        }
        let mut gated = vec![];
        for ei in 0..entries.len() {
            let Some(d) = &self.dist[ei] else { continue };
            if d.mapped || self.street_name(w, ei).is_empty() || entries[ei].points.len() < 2 {
                continue;
            }
            let mut queue: std::collections::VecDeque<String> = ends(ei).into_iter().filter(|k| underground.contains_key(k)).collect();
            if queue.is_empty() {
                continue;
            }
            let mut visited: HashSet<u32> = HashSet::new();
            let mut opens = true;
            'bfs: while let Some(k) = queue.pop_front() {
                for &gap in underground.get(&k).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if !visited.insert(gap) {
                        continue;
                    }
                    for far in ends(gap as usize) {
                        if far == k {
                            continue;
                        }
                        if strung.get(&far).is_some_and(|v| v.iter().any(|&o| o != ei as u32)) {
                            opens = false;
                            break 'bfs;
                        }
                        if underground.contains_key(&far) {
                            queue.push_back(far);
                        }
                    }
                }
            }
            if opens {
                gated.push(ei);
            }
        }
        for ei in gated {
            self.dist[ei] = None;
        }
        for ei in 0..entries.len() {
            if self.dist[ei].is_some() {
                let name = self.street_name(w, ei).to_string();
                self.dist_by_name.entry(name).or_default().push(ei as u32);
            }
        }
    }

    // ---- datums -------------------------------------------------------------

    fn road_datum(&self, w: &PropsWorld, x: f64, z: f64) -> f64 {
        w.street.terrain.ground(x, z) + w.street.o.road_lift
    }

    fn footway(&self, w: &PropsWorld, x: f64, z: f64) -> f64 {
        w.ground(x, z)
    }

    // ---- inputs -------------------------------------------------------------

    fn node_signals(&self, w: &PropsWorld, ni: u32) -> Arc<Vec<Prop>> {
        cached(&self.node_signal_cache, ni, 8192, || {
            let mut v = vec![];
            crate::signals::plan_node(w, ni as usize, &mut v);
            v
        })
    }

    fn entry_trees(&self, w: &PropsWorld, ei: u32) -> Arc<Vec<Prop>> {
        cached(&self.tree_cache, ei, 8192, || crate::trees::plan_entry(w, ei as usize))
    }

    fn input_tile(&self, w: &PropsWorld, key: (i32, i32)) -> Arc<InputTile> {
        cached(&self.input_cache, key, 4096, || {
            let t0 = std::time::Instant::now();
            let t = self.plan_input_tile(w, key);
            self.input_ns.fetch_add(t0.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
            t
        })
    }

    /// Nanoseconds spent planning the inputs (signals, SFMTA posts, surveyed trees) so far.
    pub fn input_ns(&self) -> u64 {
        self.input_ns.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn plan_input_tile(&self, w: &PropsWorld, key: (i32, i32)) -> InputTile {
        {
            let mut t = InputTile::default();
            let inside = |x: f64, z: f64| eo_street::tile_key(x, z) == key;
            let radius = |k: &str| self.js.consts["furnitureRadius"][k].as_f64().unwrap_or(f64::NAN);
            let (tree_r, palm_r, sign_r) = (radius("tree"), radius("palm"), radius("signPole"));
            let clearance = |r: f64| if r.is_finite() { r + 0.55 } else { self.js.h("signPropClearanceM") };
            let mut nodes: Vec<u32> = vec![];
            let mut entries: Vec<u32> = vec![];
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if let Some(tr) = w.street.tiles.get(&(key.0 + dx, key.1 + dz)) {
                        nodes.extend(tr.nodes.iter().copied().filter(|&ni| w.street.plan.nodes[ni as usize].signalised()));
                        if dx == 0 && dz == 0 {
                            entries.extend(tr.spans.iter().map(|s| s.0));
                        }
                    }
                }
            }
            nodes.sort_unstable();
            nodes.dedup();
            entries.sort_unstable();
            entries.dedup();
            for ni in nodes {
                for p in self.node_signals(w, ni).iter() {
                    if !inside(p.pos[0], p.pos[2]) {
                        continue;
                    }
                    match p.model {
                        Model::SignalPole => t.signal.push(SignalPoleIn { x: p.pos[0], z: p.pos[2], h: p.scale[1] as f64 }),
                        // `drawnSignalHardware`: the head body band and radius, the arm along its local +X.
                        Model::SignalHead => t.hardware.push(HwIn { a: [p.pos[0], p.pos[2]], b: [p.pos[0], p.pos[2]], y0: p.pos[1] - 0.68, y1: p.pos[1] + 0.70, radius: 0.36 }),
                        Model::MastArm => {
                            let l = p.scale[0] as f64;
                            if l > 0.1 {
                                t.hardware.push(HwIn { a: [p.pos[0], p.pos[2]], b: [p.pos[0] + p.yaw.cos() * l, p.pos[2] - p.yaw.sin() * l], y0: p.pos[1] - 0.12, y1: p.pos[1] + 0.12, radius: 0.1 });
                            }
                        }
                        _ => {}
                    }
                }
            }
            for ei in entries {
                for p in self.entry_trees(w, ei).iter() {
                    if !inside(p.pos[0], p.pos[2]) {
                        continue;
                    }
                    let r = if p.model == Model::Palm { palm_r } else { tree_r };
                    t.drawn.push(DrawnIn { x: p.pos[0], z: p.pos[2], radius: r, clearance: clearance(r), tree: true });
                }
            }
            for p in crate::signs::plan_tile(w, key) {
                if p.model != Model::SignPost {
                    continue;
                }
                // A drawn sign post reserves most of a pitch (`drawnFurnitureFootprints`).
                let c = clearance(sign_r).max(self.h.sign_pitch[0] * 0.3);
                t.drawn.push(DrawnIn { x: p.pos[0], z: p.pos[2], radius: sign_r, clearance: c, tree: false });
            }
            t
        }
    }

    fn inputs_box(&self, w: &PropsWorld, min: [f64; 2], max: [f64; 2]) -> Inputs {
        let (a, b) = (eo_street::tile_key(min[0], min[1]), eo_street::tile_key(max[0], max[1]));
        let keys: Vec<(i32, i32)> = (a.1..=b.1).flat_map(|tz| (a.0..=b.0).map(move |tx| (tx, tz))).collect();
        self.inputs_keys(w, &keys)
    }

    fn inputs_keys(&self, w: &PropsWorld, keys: &[(i32, i32)]) -> Inputs {
        let mut out = Inputs::default();
        {
            for &(tx, tz) in keys {
                let t = self.input_tile(w, (tx, tz));
                for p in &t.signal {
                    out.signal_grid.push(out.signal.len(), p.x, p.z);
                    out.signal.push(*p);
                }
                for p in &t.drawn {
                    out.drawn_grid.push(out.drawn.len(), p.x, p.z);
                    out.drawn.push(*p);
                }
                out.hardware.extend(t.hardware.iter().copied());
            }
        }
        out
    }

    fn entry_inputs(&self, w: &PropsWorld, ei: usize) -> Arc<Inputs> {
        cached(&self.inputs_cache, ei as u32, 8192, || {
            // The input tiles along the run (not its bounding box: a long diagonal run's box is most of a district).
            let e = &w.street.plan.entries[ei];
            let m = e.half + e.walks[0].max(e.walks[1]) + 12.0;
            let mut keys: Vec<(i32, i32)> = vec![];
            for i in 0..e.points.len().saturating_sub(1) {
                let (a, b) = (e.points[i], e.points[i + 1]);
                let n = 1 + ((b[0] - a[0]).hypot(b[1] - a[1]) / 16.0).ceil() as usize;
                for k in 0..=n {
                    let t = k as f64 / n as f64;
                    let (x, z) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                    let (lo, hi) = (eo_street::tile_key(x - m, z - m), eo_street::tile_key(x + m, z + m));
                    for tz in lo.1..=hi.1 {
                        for tx in lo.0..=hi.0 {
                            keys.push((tx, tz));
                        }
                    }
                }
            }
            keys.sort_unstable();
            keys.dedup();
            self.inputs_keys(w, &keys)
        })
    }

    // ---- wiring rules -----------------------------------------------------

    fn approach_wired(&self, w: &PropsWorld, ni: usize, ai: usize) -> Option<bool> {
        self.wired[w.street.plan.nodes[ni].approaches[ai].entry]
    }

    fn approach_strung(&self, w: &PropsWorld, ni: usize, ai: usize) -> bool {
        let app = &w.street.plan.nodes[ni].approaches[ai];
        match self.wired[app.entry] {
            None => app.class_rank >= self.h.min_class_rank,
            Some(b) => b,
        }
    }

    /// `holdsJunctionSpan(node)` (the webbing rule of section 2 as well).
    fn node_webbed(&self, w: &PropsWorld, ni: usize) -> bool {
        let n = &w.street.plan.nodes[ni];
        if n.approaches.len() < 3 {
            return false;
        }
        let strung = (0..n.approaches.len()).filter(|&a| self.approach_strung(w, ni, a)).count();
        let any_source = (0..n.approaches.len()).any(|a| self.approach_wired(w, ni, a).is_some());
        strung >= if any_source { 1 } else { 2 }
    }

    // ---- section 1 / 1b: contact wires and their supports -------------------

    fn supports(&self, w: &PropsWorld, ei: u32) -> Arc<Supports> {
        cached(&self.support_cache, ei, 16384, || self.plan_supports(w, ei as usize))
    }

    fn plan_supports(&self, w: &PropsWorld, ei: usize) -> Supports {
        let h = &self.h;
        let mut out = Supports { contact: vec![], poles: vec![], wires: vec![], signs: vec![] };
        let e = &w.street.plan.entries[ei];
        if e.points.len() < 2 || e.cum.len() != e.points.len() {
            return out;
        }
        let wired = self.wired[ei];
        let trolley = match wired {
            Some(false) => false,
            None => e.class_rank >= h.min_class_rank,
            Some(true) => true,
        };
        let total = e.cum[e.cum.len() - 1];
        let run_start = total.min(e.trim_start.max(0.0));
        let run_end = run_start.max(total - e.trim_end.max(0.0));
        let run = run_end - run_start;
        if run < h.min_run {
            return out;
        }
        let half = if e.half > 0.0 { e.half.max(0.8) } else { 3.2 };
        let pair = [-h.pair_half_gap, h.pair_half_gap];
        // (lane, offsets, streetcar)
        let mut runs: Vec<(f64, Vec<f64>, bool)> = vec![];
        if trolley {
            let lanes: Vec<f64> = if e.oneway { vec![0.0] } else { vec![-half * 0.5, half * 0.5] };
            for l in lanes {
                runs.push((l, pair.to_vec(), false));
            }
            if let Some(ov) = self.overlay[ei]
                && e.oneway
                && !self.overlays[ov].extra_contact_lanes.is_empty()
            {
                let mid = e.frame((run_start + run_end) / 2.0, true);
                let side = observed_side_n(&self.overlays[ov], mid.nx, mid.nz);
                for &u in &self.overlays[ov].extra_contact_lanes {
                    runs.push((u * side, pair.to_vec(), false));
                }
            }
        }
        for t in &self.streetcar[ei] {
            let _ = t.track;
            if runs.iter().any(|r| r.1.iter().any(|o| (r.0 + o - t.lane).abs() < h.sc_trolley_share)) {
                continue;
            }
            runs.push((t.lane, vec![0.0], true));
        }
        if runs.is_empty() {
            return out;
        }
        let spans = 1usize.max((run / h.support_spacing).round() as usize);
        let span_length = run / spans as f64;
        let steps = 2usize.max((run / h.sample_step).ceil() as usize);
        let inputs = self.entry_inputs(w, ei);
        // `segmentHardware`.
        let reach = 6.0f64.max(if e.half > 0.0 { e.half } else { 3.2 } * 2.0 + 4.0);
        let obstacles: Vec<[f64; 6]> = inputs
            .hardware
            .iter()
            .filter_map(|it| {
                let a = project_to_segment(&e.points, &e.cum, it.a[0], it.a[1]);
                let b = if it.b == it.a { a } else { project_to_segment(&e.points, &e.cum, it.b[0], it.b[1]) };
                if a.2.min(b.2) > reach {
                    return None;
                }
                let r = it.radius;
                Some([a.0.min(b.0) - r, a.0.max(b.0) + r, a.1.min(b.1) - r, a.1.max(b.1) + r, it.y0, it.y1])
            })
            .collect();
        let contact_r = self.js.wire_radius(WireKind::Contact);
        // Span boundaries in sample indices: each contact wire is cut per support span.
        let cuts: Vec<usize> = (0..=spans).map(|j| ((j * steps) as f64 / spans as f64).round() as usize).collect();
        for (lane, offsets, streetcar) in &runs {
            let mut adjust: Vec<(f64, f64)> = vec![];
            if !obstacles.is_empty() {
                for k in 0..=steps {
                    let station = run_start + run * k as f64 / steps as f64;
                    let (px, pz, tx, tz) = sample_along(&e.points, &e.cum, station);
                    let (x, z) = (px - tz * lane, pz + tx * lane);
                    let sag = h.support_sag * sag_fraction(station, run_start, span_length);
                    let y = self.road_datum(w, x, z) + h.contact_height - sag;
                    adjust.push(self.hardware_avoidance(&obstacles, station, *lane, y, h.hardware_clear + if offsets.len() > 1 { h.pair_half_gap } else { 0.0 }));
                }
            }
            for &offset in offsets {
                let mut pts: Vec<[f32; 3]> = Vec::with_capacity(steps + 1);
                for k in 0..=steps {
                    let station = run_start + run * k as f64 / steps as f64;
                    let (px, pz, tx, tz) = sample_along(&e.points, &e.cum, station);
                    let a = adjust.get(k).copied().unwrap_or((0.0, 0.0));
                    let lateral = lane + offset + a.0;
                    let (x, z) = (px - tz * lateral, pz + tx * lateral);
                    let sag = h.support_sag * sag_fraction(station, run_start, span_length);
                    pts.push(f3(x, self.road_datum(w, x, z) + h.contact_height - sag - a.1, z));
                }
                for j in 0..spans {
                    let (a, b) = (cuts[j], cuts[j + 1]);
                    if b > a {
                        out.contact.push(Wire { kind: WireKind::Contact, system: if *streetcar { WireSystem::Streetcar } else { WireSystem::Trolley }, points: pts[a..=b].to_vec(), radius_m: contact_r });
                    }
                }
            }
        }
        self.plan_support_poles(w, ei, e, &inputs, run_start, run_end, spans, span_length, &runs, &mut out);
        out
    }

    /// `hardwareAvoidance`: (lateral push, drop).
    fn hardware_avoidance(&self, obstacles: &[[f64; 6]], station: f64, lateral: f64, wire_y: f64, need: f64) -> (f64, f64) {
        let h = &self.h;
        let (mut push, mut drop) = (0.0f64, 0.0f64);
        for ob in obstacles {
            let ds = if station < ob[0] { ob[0] - station } else if station > ob[1] { station - ob[1] } else { 0.0 };
            if ds >= h.hardware_influence {
                continue;
            }
            let clear_below = (ob[4] - h.hardware_vertical_clear) - wire_y;
            if clear_below >= 0.0 || wire_y > ob[5] {
                continue;
            }
            let outward = (ob[3] + need) - lateral;
            let inward = lateral - (ob[2] - need);
            if outward <= 0.0 || inward <= 0.0 {
                continue;
            }
            let core = h.sample_step * 0.625;
            let ramp = if ds <= core { 1.0 } else { 1.0 - (ds - core) / (h.hardware_influence - core) };
            let escape = outward.min(inward);
            if escape <= h.hardware_lateral_budget {
                let signed = (if outward <= inward { outward } else { -inward }) * ramp;
                if signed > 0.0 {
                    push = push.max(signed);
                } else {
                    push = push.min(signed);
                }
            } else {
                drop = drop.max(-clear_below * ramp);
            }
        }
        (push, drop)
    }

    /// `supportPoleAt` over the inset list: the pole foot, or None.
    #[allow(clippy::too_many_arguments)]
    fn support_pole_at(&self, w: &PropsWorld, e: &Entry, side: f64, s: f64, band: &FBand, windows: &[RampWin], inputs: &Inputs, poles: &[[f64; 2]]) -> Option<[f64; 3]> {
        for &inset in &self.h.insets {
            if let Some(p) = self.support_pole_at_inset(w, e, side, s, band, windows, inset, inputs, poles) {
                return Some(p);
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn support_pole_at_inset(&self, w: &PropsWorld, e: &Entry, side: f64, s: f64, band: &FBand, windows: &[RampWin], inset: f64, inputs: &Inputs, poles: &[[f64; 2]]) -> Option<[f64; 3]> {
        let h = &self.h;
        let lateral = e.half + inset;
        if lateral < band.furnish_inner || lateral > band.furnish_outer {
            return None;
        }
        if in_ramp(windows, s, h.sign_ramp_clear) {
            return None;
        }
        if station_in_yield_span(e, side, s, h.sign_ramp_clear) {
            return None;
        }
        let f = e.frame(s, true);
        let (x, z) = (f.x + f.nx * side * lateral * f.miter, f.z + f.nz * side * lateral * f.miter);
        if w.roadway.hit(x, z, h.pole_r_base).is_some() || w.roadway.ramp_hit(x, z, h.pole_r_base) {
            return None;
        }
        let mut blocked = false;
        inputs.drawn_grid.near(x, z, 3.0, |i| {
            let it = inputs.drawn[i];
            let gap = if it.radius.is_finite() { it.radius + h.mid_span_prop_gap } else { h.prop_clearance };
            if (it.x - x).hypot(it.z - z) < gap {
                blocked = true;
            }
        });
        if blocked || inputs.any_signal_within(x, z, h.prop_clearance) {
            return None;
        }
        if poles.iter().any(|p| (p[0] - x).hypot(p[1] - z) < 1.2) {
            return None;
        }
        Some([x, self.footway(w, x, z), z])
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_support_poles(&self, w: &PropsWorld, ei: usize, e: &Entry, inputs: &Inputs, run_start: f64, run_end: f64, spans: usize, span_length: f64, runs: &[(f64, Vec<f64>, bool)], out: &mut Supports) {
        let h = &self.h;
        let bands = [self.fband(w, e, 1.0), self.fband(w, e, -1.0)];
        let ramps = segment_kerb_ramps(e, &w.street.o);
        let key = &self.key[ei];
        let steel_only = self.overlay[ei].is_some_and(|o| self.overlays[o].steel_poles);
        let span_r = self.js.wire_radius(WireKind::Span);
        let bracket_r = self.js.wire_radius(WireKind::Bracket);
        let hanger_r = self.js.wire_radius(WireKind::Hanger);
        let mut placed: Vec<[f64; 2]> = vec![];
        for k in 0..=spans {
            let at_start = k == 0;
            let at_end = k == spans;
            if at_start && e.approach_start.is_some_and(|(ni, _)| self.node_webbed(w, ni)) {
                continue;
            }
            if at_end && e.approach_end.is_some_and(|(ni, _)| self.node_webbed(w, ni)) {
                continue;
            }
            let ideal = run_start + k as f64 * span_length;
            let mut pick: [Option<(f64, [f64; 3])>; 2] = [None, None];
            for (si, side) in [1.0f64, -1.0].into_iter().enumerate() {
                let Some(band) = &bands[si] else { continue };
                for &nudge in &h.nudges {
                    if (at_start && nudge < 0.0) || (at_end && nudge > 0.0) {
                        continue;
                    }
                    let s = ideal + nudge;
                    if s < run_start || s > run_end {
                        continue;
                    }
                    if let Some(p) = self.support_pole_at(w, e, side, s, band, &ramps[si], inputs, &placed) {
                        pick[si] = Some((s, p));
                        break;
                    }
                }
            }
            let station = match (pick[0], pick[1]) {
                (Some(a), Some(b)) => (a.0 + b.0) / 2.0,
                (Some(a), None) | (None, Some(a)) => a.0,
                (None, None) => continue,
            };
            let (px, pz, ptx, ptz) = sample_along(&e.points, &e.cum, station);
            let sag_here = h.support_sag * sag_fraction(station, run_start, span_length);
            let mut holds: Vec<[f64; 3]> = vec![];
            for (lane, offsets, _) in runs {
                for o in offsets {
                    let (x, z) = (px - ptz * (lane + o), pz + ptx * (lane + o));
                    holds.push([x, self.road_datum(w, x, z) + h.contact_height - sag_here, z]);
                }
            }
            if holds.is_empty() {
                continue;
            }
            let top_y = holds.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max) + h.mid_span_hanger;
            let mut new_poles: Vec<([f64; 3], i32)> = vec![];
            let hanger_top: Box<dyn Fn(&[f64; 3]) -> [f64; 3]>;
            if let (Some(l), Some(r)) = (pick[0], pick[1]) {
                let (a, b) = (l.1, r.1);
                let width = (b[0] - a[0]).hypot(b[2] - a[2]);
                if !(width >= h.span_min_width && width <= h.span_max_width) {
                    continue;
                }
                let sag = h.span_sag_max.min(h.span_sag_min.max(width * h.span_sag_per_metre));
                let t_of = move |p: &[f64; 3]| {
                    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
                    let mut ww = width * width;
                    if ww == 0.0 {
                        ww = 1.0;
                    }
                    (((p[0] - a[0]) * dx + (p[2] - a[2]) * dz) / ww).clamp(0.0, 1.0)
                };
                let mut lift = 0.0f64;
                for p in &holds {
                    let t = t_of(p);
                    let base = a[1] + (b[1] - a[1]) * t - sag * 4.0 * t * (1.0 - t);
                    lift = lift.max(p[1] + h.mid_span_hanger - base);
                }
                lift = lift.min(h.pole_height - 0.35);
                let (ya, yb) = (a[1] + lift, b[1] + lift);
                let steps = 4usize.max(14usize.min((width / 3.0).round() as usize));
                let at = move |t: f64| [a[0] + (b[0] - a[0]) * t, ya + (yb - ya) * t - sag * 4.0 * t * (1.0 - t), a[2] + (b[2] - a[2]) * t];
                let pts = (0..=steps).map(|i| {
                    let p = at(i as f64 / steps as f64);
                    f3(p[0], p[1], p[2])
                });
                out.wires.push(Wire { kind: WireKind::Span, system: WireSystem::None, points: pts.collect(), radius_m: span_r });
                hanger_top = Box::new(move |p: &[f64; 3]| at(t_of(p)));
                new_poles.push((a, 1));
                new_poles.push((b, -1));
            } else {
                let (single, side) = if let Some(l) = pick[0] { (l, 1) } else { (pick[1].unwrap(), -1) };
                let pole = single.1;
                let mut far: Option<[f64; 3]> = None;
                let mut reach = 0.0;
                for p in &holds {
                    let d = (p[0] - pole[0]).hypot(p[2] - pole[2]);
                    if d > reach {
                        reach = d;
                        far = Some(*p);
                    }
                }
                let Some(far) = far else { continue };
                if reach + 0.4 > h.bracket_max_reach {
                    continue;
                }
                let (ux, uz) = ((far[0] - pole[0]) / reach, (far[2] - pole[2]) / reach);
                let end = [pole[0] + ux * (reach + 0.4), top_y, pole[2] + uz * (reach + 0.4)];
                let mid = [(pole[0] + end[0]) / 2.0, top_y, (pole[2] + end[2]) / 2.0];
                out.wires.push(Wire { kind: WireKind::Bracket, system: WireSystem::None, points: vec![f3(pole[0], top_y, pole[2]), f3(end[0], end[1], end[2])], radius_m: bracket_r });
                let tie_y = (pole[1] + h.pole_height - 0.2).min(top_y + 1.1);
                out.wires.push(Wire { kind: WireKind::Bracket, system: WireSystem::None, points: vec![f3(pole[0], tie_y, pole[2]), f3(mid[0], mid[1], mid[2])], radius_m: bracket_r });
                hanger_top = Box::new(move |p: &[f64; 3]| [p[0], top_y, p[2]]);
                new_poles.push((pole, side));
            }
            for p in &holds {
                let top = hanger_top(p);
                if !(top[1] - p[1] > 0.05) {
                    continue;
                }
                out.wires.push(Wire { kind: WireKind::Hanger, system: WireSystem::None, points: vec![f3(p[0], p[1], p[2]), f3(p[0], top[1], p[2])], radius_m: hanger_r });
            }
            for (pole, side) in new_poles {
                let id = format!("overhead-pole:{key}:support:{k}:{side}");
                let wood = !steel_only && overhead_hash01(&format!("{id}:wood")) < h.wood_share;
                placed.push([pole[0], pole[2]]);
                out.poles.push(OverheadPole {
                    kind: PoleKind::Support,
                    material: if wood { PoleMaterial::Wood } else { PoleMaterial::Steel },
                    pos: pole,
                    yaw: 0.0,
                    height: h.pole_height,
                    radius_base: if wood { h.wood_r_base } else { h.pole_r_base },
                    radius_top: if wood { h.wood_r_top } else { h.pole_r_top },
                    arm: None,
                    id: id.clone(),
                });
                // A regulation stack strapped to the pole.
                if overhead_hash01(&format!("{id}:sign")) < h.pole_sign_share {
                    let flip = if overhead_hash01(&format!("{id}:face")) < 0.5 { 1.0 } else { -1.0 };
                    let yaw = (ptx * flip).atan2(ptz * flip);
                    let radius_at = if wood { h.wood_r_base + (h.wood_r_top - h.wood_r_base) * (2.3 / h.pole_height) } else { h.pole_r_base + (h.pole_r_top - h.pole_r_base) * (2.3 / h.pole_height) };
                    let off = radius_at - h.sign_post_section / 2.0 + 0.008;
                    let variant = if overhead_hash01(&format!("{id}:stack")) < 0.55 { h.pole_sign_variants[0] } else { h.pole_sign_variants[1] };
                    let (sx, sz) = (pole[0] + yaw.sin() * off, pole[2] + yaw.cos() * off);
                    if self.plate_over_road(w, sx, sz, yaw, variant) {
                        continue;
                    }
                    out.signs.push(self.assembly([sx, pole[1], sz], yaw, 1.0, variant, true));
                }
            }
        }
    }

    /// The post and both plate edges of an assembly against every drawn roadway.
    fn plate_over_road(&self, w: &PropsWorld, x: f64, z: f64, yaw: f64, variant: u16) -> bool {
        let widest = self.js.sign_variants[variant as usize].plates.iter().map(|p| p.w).fold(0.3, f64::max);
        let plate_half = 0.53 * widest;
        let (ax, az) = (yaw.cos(), -yaw.sin());
        let (fx, fz) = (yaw.sin() * 0.04, yaw.cos() * 0.04);
        w.roadway.hit(x, z, self.h.sign_post_section).is_some()
            || w.roadway.hit(x + ax * plate_half + fx, z + az * plate_half + fz, 0.02).is_some()
            || w.roadway.hit(x - ax * plate_half + fx, z - az * plate_half + fz, 0.02).is_some()
    }

    fn assembly(&self, pos: [f64; 3], yaw: f64, scale: f64, variant: u16, on_pole: bool) -> SignAssembly {
        let spec = &self.js.sign_variants[variant as usize];
        let face_z = self.h.sign_post_section / 2.0 + 0.006;
        let (s, c) = (yaw.sin(), yaw.cos());
        let plates = spec
            .plates
            .iter()
            .map(|p| {
                let (lx, ly, lz) = (p.x.unwrap_or(0.0) * scale, p.y * scale, face_z * scale);
                PlacedPlate {
                    centre: [pos[0] + lx * c + lz * s, pos[1] + ly, pos[2] - lx * s + lz * c],
                    w: p.w * scale,
                    h: p.h * scale,
                    band: p.band.clone(),
                    fill: p.fill.clone(),
                    face: p.face.clone(),
                    rules: p.rules.unwrap_or(0),
                }
            })
            .collect();
        SignAssembly { pos, yaw, scale, variant, post_height: spec.post_height_m, on_pole, plates }
    }

    fn fband(&self, w: &PropsWorld, e: &Entry, side: f64) -> Option<FBand> {
        w.band(e, side, 1.05).map(|b| FBand { walk: b.walk, furnish_inner: b.inner + 0.12, furnish_outer: b.outer - b.clear })
    }

    // ---- section 2 and 3: junction spans, corner signs, cabinets -----------

    fn node(&self, w: &PropsWorld, ni: u32) -> Arc<NodePlan> {
        cached(&self.node_cache, ni, 16384, || self.plan_node(w, ni as usize))
    }

    fn plan_node(&self, w: &PropsWorld, ni: usize) -> NodePlan {
        let mut out = NodePlan::default();
        let n = &w.street.plan.nodes[ni];
        let webbed = self.node_webbed(w, ni);
        if !webbed && !n.signalised() {
            return out;
        }
        let c = n.position;
        let inputs = self.inputs_box(w, [c[0] - 48.0, c[1] - 48.0], [c[0] + 48.0, c[1] + 48.0]);
        // The poles already standing round the node: the supports of its approaches.
        let mut standing: Vec<[f64; 2]> = vec![];
        let mut seen: Vec<usize> = vec![];
        for app in &n.approaches {
            if seen.contains(&app.entry) {
                continue;
            }
            seen.push(app.entry);
            for p in &self.supports(w, app.entry as u32).poles {
                standing.push([p.pos[0], p.pos[2]]);
            }
        }
        let h = &self.h;
        let m = n.approaches.len();
        // anchor: (x, z, y, pole index in out.poles or usize::MAX = signal host, base_y, bx, bz, radius)
        let mut anchors: Vec<Option<Option<Anchor>>> = vec![None; m];
        let mut corner_poles: Vec<Option<OverheadPole>> = vec![None; m];
        if webbed {
            let span_r = self.js.wire_radius(WireKind::Span);
            for i in 0..m {
                if !self.approach_strung(w, ni, i) {
                    continue;
                }
                let li = (i + m - 1) % m;
                for idx in [li, i] {
                    if anchors[idx].is_none() {
                        let mut placed: Vec<[f64; 2]> = standing.clone();
                        placed.extend(corner_poles.iter().flatten().map(|p| [p.pos[0], p.pos[2]]));
                        anchors[idx] = Some(self.anchor_for(w, ni, idx, &inputs, &placed));
                    }
                }
                let (Some(Some(left)), Some(Some(right))) = (anchors[li], anchors[i]) else { continue };
                let width = (right.x - left.x).hypot(right.z - left.z);
                if !(width >= h.span_min_width && width <= h.span_max_width) {
                    continue;
                }
                let sag = h.span_sag_max.min(h.span_sag_min.max(width * h.span_sag_per_metre));
                let steps = 4usize.max(14usize.min((width / 3.0).round() as usize));
                let pts = (0..=steps)
                    .map(|k| {
                        let t = k as f64 / steps as f64;
                        f3(left.x + (right.x - left.x) * t, left.y + (right.y - left.y) * t - sag * 4.0 * t * (1.0 - t), left.z + (right.z - left.z) * t)
                    })
                    .collect();
                out.spans.push(Wire { kind: WireKind::Span, system: WireSystem::None, points: pts, radius_m: span_r });
                for (idx, a) in [(li, left), (i, right)] {
                    if a.host || corner_poles[idx].is_some() {
                        continue;
                    }
                    corner_poles[idx] = Some(OverheadPole {
                        kind: PoleKind::Span,
                        material: PoleMaterial::Steel,
                        pos: [a.x, a.base_y, a.z],
                        yaw: 0.0,
                        height: h.pole_height,
                        radius_base: h.pole_r_base,
                        radius_top: h.pole_r_top,
                        arm: None,
                        id: format!("overhead-pole:{}:{}", n.id, idx),
                    });
                }
            }
            if n.signalised() {
                for (idx, a) in anchors.iter().enumerate() {
                    let Some(Some(a)) = a else { continue };
                    if !a.host && corner_poles[idx].is_none() {
                        continue;
                    }
                    let reach = a.radius + 0.035;
                    out.corner_signs.push(CornerSign { pos: [a.x - a.bx * reach, a.base_y + h.corner_sign_height, a.z - a.bz * reach], yaw: (-a.bx).atan2(-a.bz), on_signal_pole: a.host });
                }
            }
            out.poles = corner_poles.into_iter().flatten().collect();
        }
        if n.signalised() {
            let mut all_poles = standing;
            all_poles.extend(out.poles.iter().map(|p| [p.pos[0], p.pos[2]]));
            out.cabinet = self.plan_cabinet(w, ni, &inputs, &all_poles);
        }
        out
    }

    fn anchor_for(&self, w: &PropsWorld, ni: usize, ci: usize, inputs: &Inputs, poles: &[[f64; 2]]) -> Option<Anchor> {
        let h = &self.h;
        let n = &w.street.plan.nodes[ni];
        let m = n.approaches.len();
        let corner = match n.corners.get(ci) {
            Some(Some(c)) if c.radius > 0.0 => *c,
            _ => return self.mouth_anchor_for(w, ni, ci, inputs, poles),
        };
        let (a, b) = (&n.approaches[ci], &n.approaches[(ci + 1) % m]);
        let walk = a.width_ccw.max(b.width_cw);
        if walk < h.pole_footway_min {
            return None;
        }
        let (bx, bz) = (corner.bisector[0], corner.bisector[1]);
        let (kx, kz) = (corner.centre[0] - bx * corner.radius, corner.centre[1] - bz * corner.radius);
        let i0 = h.pole_kerb_insets[0];
        if let Some((p, d)) = inputs.nearest_signal_within(kx + bx * i0, kz + bz * i0, h.mast_reuse_radius)
            && d <= h.mast_reuse_radius
            && p.h >= h.mast_min_height
        {
            let base_y = self.footway(w, p.x, p.z);
            return Some(Anchor { x: p.x, z: p.z, y: base_y + h.span_height.min(p.h - 0.5), host: true, base_y, bx, bz, radius: 0.09 });
        }
        let mut stations: Vec<(f64, f64)> = h.pole_kerb_insets.iter().map(|&i| (i, 0.0)).collect();
        let (ta, tb, cc) = (corner.ta, corner.tb, corner.centre);
        let half_sweep = (((ta[0] - cc[0]) * (tb[1] - cc[1]) - (ta[1] - cc[1]) * (tb[0] - cc[0])).atan2((ta[0] - cc[0]) * (tb[0] - cc[0]) + (ta[1] - cc[1]) * (tb[1] - cc[1]))).abs() / 2.0;
        for turn in [0.2, 0.35, 0.5, 0.7, 0.9] {
            if turn > half_sweep - 0.05 {
                break;
            }
            for &inset in &h.pole_kerb_insets {
                if corner.radius - inset < 0.3 {
                    continue;
                }
                stations.push((inset, turn));
                stations.push((inset, -turn));
            }
        }
        let behind = w.street.o.curb_top_width + w.street.o.ramp_run + 0.3;
        for inset in [behind, behind + 0.4] {
            stations.push((inset, 0.0));
        }
        for (inset, turn) in stations {
            if inset > walk - 0.25 {
                continue;
            }
            let (mut x, mut z) = (kx + bx * inset, kz + bz * inset);
            if turn != 0.0 {
                let (c, s) = (turn.cos(), turn.sin());
                let reach = corner.radius - inset;
                x = cc[0] + (-bx * c + bz * s) * reach;
                z = cc[1] + (-bx * s - bz * c) * reach;
            }
            if inputs.any_signal_within(x, z, h.prop_clearance) {
                continue;
            }
            if poles.iter().any(|p| (p[0] - x).hypot(p[1] - z) < 1.2) {
                continue;
            }
            if w.roadway.hit(x, z, h.pole_r_base).is_some() || w.roadway.ramp_hit(x, z, h.pole_r_base) {
                continue;
            }
            let base_y = self.footway(w, x, z);
            return Some(Anchor { x, z, y: base_y + h.span_height, host: false, base_y, bx, bz, radius: h.pole_r_top });
        }
        None
    }

    fn mouth_anchor_for(&self, w: &PropsWorld, ni: usize, ci: usize, inputs: &Inputs, poles: &[[f64; 2]]) -> Option<Anchor> {
        let h = &self.h;
        let n = &w.street.plan.nodes[ni];
        let m = n.approaches.len();
        let (a, b) = (&n.approaches[ci], &n.approaches[(ci + 1) % m]);
        let (half, trim) = (a.half, a.trim);
        if !(half > 0.0) {
            return None;
        }
        let (px, pz) = (-a.u[1], a.u[0]);
        let sign = if px * b.u[0] + pz * b.u[1] >= 0.0 { 1.0 } else { -1.0 };
        let walk = a.width_ccw.max(a.width_cw);
        if walk < h.pole_footway_min {
            return None;
        }
        for back in [1.5, 3.0, 4.5] {
            for &inset in &h.pole_kerb_insets {
                if inset > walk - 0.25 {
                    continue;
                }
                let x = n.position[0] + a.u[0] * (trim + back) + px * sign * (half + inset);
                let z = n.position[1] + a.u[1] * (trim + back) + pz * sign * (half + inset);
                if inputs.any_signal_within(x, z, h.prop_clearance) {
                    continue;
                }
                if poles.iter().any(|p| (p[0] - x).hypot(p[1] - z) < 1.2) {
                    continue;
                }
                if w.roadway.hit(x, z, h.pole_r_base).is_some() || w.roadway.ramp_hit(x, z, h.pole_r_base) {
                    continue;
                }
                let base_y = self.footway(w, x, z);
                return Some(Anchor { x, z, y: base_y + h.span_height, host: false, base_y, bx: px * sign, bz: pz * sign, radius: h.pole_r_top });
            }
        }
        None
    }

    fn plan_cabinet(&self, w: &PropsWorld, ni: usize, inputs: &Inputs, poles: &[[f64; 2]]) -> Option<Cabinet> {
        let h = &self.h;
        let n = &w.street.plan.nodes[ni];
        let mut ordered: Vec<usize> = (0..n.approaches.len()).collect();
        ordered.sort_by(|&a, &b| {
            let (pa, pb) = (&n.approaches[a], &n.approaches[b]);
            pb.class_rank.cmp(&pa.class_rank).then_with(|| if self.key[pa.entry] < self.key[pb.entry] { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater })
        });
        let [cw, cd, _] = h.cabinet;
        for ai in ordered {
            let app = &n.approaches[ai];
            let e = &w.street.plan.entries[app.entry];
            if e.points.len() < 2 || e.cum.len() != e.points.len() {
                continue;
            }
            let half = if app.half > 0.0 { app.half.max(0.8) } else { 3.2 };
            let trim = app.trim.max(0.0);
            let run_length = if app.run_length > 0.0 { app.run_length } else { e.cum[e.cum.len() - 1] };
            let mut sides = [(1.0f64, app.width_ccw), (-1.0, app.width_cw)];
            if sides[1].1 > sides[0].1 {
                sides.swap(0, 1);
            }
            for (sign, walk) in sides {
                if walk < h.cabinet_footway_min {
                    continue;
                }
                let lateral = half + (h.cabinet_kerb_gap + cd / 2.0).max(walk * 0.5).min(walk - 0.35 - cd / 2.0);
                if lateral <= half {
                    continue;
                }
                for &station in &h.cabinet_stations {
                    let along = trim + station;
                    if along > run_length - 1.5 {
                        break;
                    }
                    let x = n.position[0] + app.u[0] * along - app.u[1] * sign * lateral;
                    let z = n.position[1] + app.u[1] * along + app.u[0] * sign * lateral;
                    if inputs.any_signal_within(x, z, h.cabinet_clearance) {
                        continue;
                    }
                    if poles.iter().any(|p| (p[0] - x).hypot(p[1] - z) < h.cabinet_clearance) {
                        continue;
                    }
                    let (ux, uz) = (app.u[0] * sign, app.u[1] * sign);
                    let (hw, hd) = (cw / 2.0, cd / 2.0);
                    let corners = [(-hw, -hd), (-hw, hd), (hw, -hd), (hw, hd)];
                    let pt = |a: f64, f: f64| (x - uz * a + ux * f, z + ux * a + uz * f);
                    if w.roadway.hit(x, z, 0.05).is_some() || corners.iter().any(|&(a, f)| {
                        let (qx, qz) = pt(a, f);
                        w.roadway.hit(qx, qz, 0.05).is_some()
                    }) {
                        continue;
                    }
                    if w.roadway.ramp_hit(x, z, 0.05) || corners.iter().any(|&(a, f)| {
                        let (qx, qz) = pt(a, f);
                        w.roadway.ramp_hit(qx, qz, 0.05)
                    }) {
                        continue;
                    }
                    return Some(Cabinet { pos: [x, self.footway(w, x, z), z], yaw: ux.atan2(uz), size: h.cabinet, node: ni as u32 });
                }
            }
        }
        None
    }

    // ---- section 4: the kerbside sign picket, and 4c lanterns ----------------

    fn rest(&self, w: &PropsWorld, ei: u32) -> Arc<EntryRest> {
        cached(&self.rest_cache, ei, 16384, || self.plan_rest(w, ei as usize))
    }

    fn plan_rest(&self, w: &PropsWorld, ei: usize) -> EntryRest {
        let mut out = EntryRest { picket: vec![], lantern_wires: vec![], lanterns: vec![] };
        let e = &w.street.plan.entries[ei];
        if e.points.len() < 2 || e.cum.len() != e.points.len() {
            return out;
        }
        if e.class_rank >= self.h.sign_min_class_rank {
            out.picket = self.plan_picket(w, ei, e);
        }
        if self.lantern_way[ei] {
            self.plan_lanterns(w, ei, e, &mut out);
        }
        out
    }

    fn plan_picket(&self, w: &PropsWorld, ei: usize, e: &Entry) -> Vec<SignAssembly> {
        let h = &self.h;
        let mut out: Vec<SignAssembly> = vec![];
        let s0 = e.trim_start.max(0.0) + h.sign_end_margin;
        let s1 = e.length - e.trim_end.max(0.0) - h.sign_end_margin;
        if !(s1 - s0 > 4.0) {
            return out;
        }
        let inputs = self.entry_inputs(w, ei);
        // What this pass already stands round the run: support poles (own and the
        // approaches' at both ends), corner poles and cabinets of both end nodes,
        // the stacks strapped to those support poles.
        let mut poles: Vec<[f64; 2]> = vec![];
        let mut cabinets: Vec<[f64; 2]> = vec![];
        let mut signs: Vec<[f64; 2]> = vec![];
        let mut entries: Vec<usize> = vec![ei];
        for (ni, _) in [e.approach_start, e.approach_end].into_iter().flatten() {
            let np = self.node(w, ni as u32);
            poles.extend(np.poles.iter().map(|p| [p.pos[0], p.pos[2]]));
            cabinets.extend(np.cabinet.iter().map(|c| [c.pos[0], c.pos[2]]));
            for app in &w.street.plan.nodes[ni].approaches {
                if !entries.contains(&app.entry) {
                    entries.push(app.entry);
                }
            }
        }
        for &other in &entries {
            let s = self.supports(w, other as u32);
            poles.extend(s.poles.iter().map(|p| [p.pos[0], p.pos[2]]));
            signs.extend(s.signs.iter().map(|p| [p.pos[0], p.pos[2]]));
        }
        let key = &self.key[ei];
        let src = &w.street.city.segments[self.src[ei]];
        let stall_pitch = src.street_type.as_deref().and_then(|t| w.js.consts["kerbStallPitch"][t].as_f64());
        let ramps = segment_kerb_ramps(e, &w.street.o);
        for (si, side) in [1i32, -1].into_iter().enumerate() {
            let sf = side as f64;
            let Some(band) = self.fband(w, e, sf) else { continue };
            if band.walk < h.sign_footway_min {
                continue;
            }
            let windows = &ramps[si];
            let face = format!("{key}:{side}");
            let mut station = s0 + hash_range(&format!("{face}:phase"), 0.0, 9.0);
            let mut i = 0;
            while i < 48 && station <= s1 {
                let k = format!("{face}:{i}");
                let pitch = match stall_pitch {
                    Some(p) => hash_range(&format!("{k}:pitch"), p * 2.0, p * 3.0),
                    None => hash_range(&format!("{k}:pitch"), h.sign_pitch[0], h.sign_pitch[1]),
                };
                let lateral = e.half + hash_range(&format!("{k}:inset"), h.sign_inset[0], h.sign_inset[1]);
                if lateral < band.furnish_inner || lateral > band.furnish_outer {
                    station += pitch;
                    i += 1;
                    continue;
                }
                let mut placed: Option<SignAssembly> = None;
                for nudge in [0.0, 4.5] {
                    let s = station + nudge;
                    if s > s1 {
                        break;
                    }
                    if in_ramp(windows, s, h.sign_ramp_clear) || station_in_yield_span(e, sf, s, h.sign_ramp_clear) {
                        continue;
                    }
                    let f = e.frame(s, true);
                    let (x, z) = (f.x + f.nx * sf * lateral * f.miter, f.z + f.nz * sf * lateral * f.miter);
                    let mut drawn_hit = false;
                    inputs.drawn_grid.near(x, z, 8.0, |j| {
                        let it = inputs.drawn[j];
                        if (it.x - x).hypot(it.z - z) < it.clearance {
                            drawn_hit = true;
                        }
                    });
                    if drawn_hit || inputs.any_signal_within(x, z, h.sign_prop_clearance) {
                        continue;
                    }
                    if poles.iter().chain(cabinets.iter()).any(|p| (p[0] - x).hypot(p[1] - z) < 1.6) || signs.iter().any(|p| (p[0] - x).hypot(p[1] - z) < h.sign_spacing) {
                        continue;
                    }
                    let roll = overhead_hash01(&format!("{k}:variant"));
                    let flip = if overhead_hash01(&format!("{k}:face")) < 0.5 { 1.0 } else { -1.0 };
                    let variant: u16 = if roll < 0.45 {
                        0
                    } else if roll < 0.75 {
                        1
                    } else if roll < 0.9 {
                        2
                    } else {
                        3
                    };
                    let yaw = (f.tx * flip).atan2(f.tz * flip) + hash_range(&format!("{k}:jit"), -0.16, 0.16);
                    if self.plate_over_road(w, x, z, yaw, variant) {
                        continue;
                    }
                    let scale = hash_range(&format!("{k}:scale"), 0.94, 1.06);
                    placed = Some(self.assembly([x, self.footway(w, x, z), z], yaw, scale, variant, false));
                    break;
                }
                if let Some(p) = placed {
                    signs.push([p.pos[0], p.pos[2]]);
                    out.push(p);
                }
                station += pitch;
                i += 1;
            }
        }
        out
    }

    fn plan_lanterns(&self, w: &PropsWorld, ei: usize, e: &Entry, out: &mut EntryRest) {
        let h = &self.h;
        let row = &self.js.consts["lantern"];
        let height = row["heightM"].as_f64().unwrap_or(0.0);
        let dims: Vec<f64> = row["dimensionsM"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default();
        if !(height > 0.0) || dims.len() < 3 {
            return;
        }
        let width_m = dims[0].max(dims[2]);
        let floor = self.std.comm_floor_m;
        let sag = h.support_sag / h.support_spacing;
        let total = e.cum[e.cum.len() - 1];
        let run_start = total.min(e.trim_start.max(0.0));
        let run_end = run_start.max(total - e.trim_end.max(0.0));
        let (reach_l, reach_r) = (e.half + e.walks[0], e.half + e.walks[1]);
        let spacing = &self.js.consts["streetLightSpacing"];
        let pitch = spacing[e.highway.as_str()]["medianM"].as_f64().or_else(|| spacing["residential"]["medianM"].as_f64()).unwrap_or(24.0);
        let key = &self.key[ei];
        let rad = self.js.wire_radius(WireKind::Drop);
        let mut s = run_start + pitch / 2.0;
        while s <= run_end {
            let f = e.frame(s, true);
            let a = [f.x + f.nx * reach_l, f.z + f.nz * reach_l];
            let b = [f.x - f.nx * reach_r, f.z - f.nz * reach_r];
            let ya = self.footway(w, a[0], a[1]) + floor + height;
            let yb = self.footway(w, b[0], b[1]) + floor + height;
            let width = (b[0] - a[0]).hypot(b[1] - a[1]);
            let drop = width * sag;
            let steps = 4usize.max((width / h.sample_step).ceil() as usize);
            let at = |t: f64| [a[0] + (b[0] - a[0]) * t, ya + (yb - ya) * t - drop * 4.0 * t * (1.0 - t), a[1] + (b[1] - a[1]) * t];
            out.lantern_wires.push(Wire {
                kind: WireKind::LanternString,
                system: WireSystem::None,
                points: (0..=steps)
                    .map(|i| {
                        let p = at(i as f64 / steps as f64);
                        f3(p[0], p[1], p[2])
                    })
                    .collect(),
                radius_m: rad,
            });
            let count = (width / (3.0 * width_m)).floor() as usize;
            for k in 1..count {
                let p = at(k as f64 / count as f64);
                out.lanterns.push(Lantern { pos: [p[0], p[1] - height, p[2]], yaw: overhead_hash01(&format!("{key}:{s}:{k}")) * std::f64::consts::PI * 2.0 });
            }
            s += pitch;
        }
    }

    // ---- section 4b: overhead distribution -----------------------------------

    /// One entry's distribution chains. `gap_against` are the poles it must keep the lower-quartile spacing from.
    fn plan_dist_entry(&self, w: &PropsWorld, ei: usize, gap_against: &[[f64; 2]]) -> DistEntry {
        let mut out = DistEntry { chains: vec![] };
        let Some(d) = &self.dist[ei] else { return out };
        let h = &self.h;
        let e = &w.street.plan.entries[ei];
        if e.points.len() < 2 || e.cum.len() != e.points.len() {
            return out;
        }
        let total = e.cum[e.cum.len() - 1];
        let run_start = total.min(e.trim_start.max(0.0));
        let run_end = run_start.max(total - e.trim_end.max(0.0));
        let run = run_end - run_start;
        if run < h.min_run {
            return out;
        }
        let spacing = self.js.consts["distributionSpacingM"].as_f64().unwrap();
        let min_gap = self.js.consts["distributionSpacingP25M"].as_f64().unwrap();
        let comm_offset = self.js.consts["utilityPole"]["shaftRadiusM"].as_f64().unwrap_or(0.0);
        let crown = self.js.consts["streetTreeMaxHalfCrown"].as_f64().unwrap();
        let (first, last) = (e.points[0], e.points[e.points.len() - 1]);
        let mut l = (last[0] - first[0]).hypot(last[1] - first[1]);
        if l == 0.0 {
            l = 1.0;
        }
        let (tx, tz) = ((last[0] - first[0]) / l, (last[1] - first[1]) / l);
        let sides: Vec<i8> = match d.compass {
            'B' => vec![1, -1],
            c => {
                let v = match c {
                    'N' => [0.0, -1.0],
                    'S' => [0.0, 1.0],
                    'E' => [1.0, 0.0],
                    'W' => [-1.0, 0.0],
                    _ => return out,
                };
                vec![if -tz * v[0] + tx * v[1] >= 0.0 { 1 } else { -1 }]
            }
        };
        let inputs = self.entry_inputs(w, ei);
        let supports: Vec<[f64; 2]> = self.supports(w, ei as u32).poles.iter().map(|p| [p.pos[0], p.pos[2]]).collect();
        let ramps = segment_kerb_ramps(e, &w.street.o);
        let key = &self.key[ei];
        let name = self.street_name(w, ei).to_string();
        let mut mine: Vec<[f64; 2]> = vec![];
        for side in sides {
            let sf = side as f64;
            let Some(band) = self.fband(w, e, sf) else { continue };
            let windows = &ramps[if side > 0 { 0 } else { 1 }];
            let n = 1usize.max((run / spacing).round() as usize);
            let mut chain = Chain { entry: ei as u32, side, compass: d.compass, name: if name.is_empty() { key.clone() } else { name.clone() }, poles: vec![] };
            for k in 0..=n {
                let ideal = run_start + run * k as f64 / n as f64;
                let mut got: Option<(f64, [f64; 3])> = None;
                'search: for crown_clear in [true, false] {
                    for &nudge in &h.nudges {
                        if (k == 0 && nudge < 0.0) || (k == n && nudge > 0.0) {
                            continue;
                        }
                        let s = ideal + nudge;
                        if s < run_start || s > run_end {
                            continue;
                        }
                        let Some(g) = self.support_pole_at(w, e, sf, s, &band, windows, &inputs, &supports) else { continue };
                        if w.roadway.hit(g[0], g[2], comm_offset).is_some() {
                            continue;
                        }
                        if mine.iter().chain(gap_against.iter()).any(|p| (p[0] - g[0]).hypot(p[1] - g[2]) < min_gap) {
                            continue;
                        }
                        if crown_clear {
                            let mut hit = false;
                            inputs.drawn_grid.near(g[0], g[2], crown, |j| {
                                let t = inputs.drawn[j];
                                if t.tree && (t.x - g[0]).hypot(t.z - g[2]) < crown {
                                    hit = true;
                                }
                            });
                            if hit {
                                continue;
                            }
                        }
                        got = Some((s, g));
                        break 'search;
                    }
                }
                let Some((s, g)) = got else { continue };
                let f = e.frame(s, true);
                let (rx, rz) = (-f.nx * sf, -f.nz * sf);
                mine.push([g[0], g[2]]);
                chain.poles.push(DistPole { id: format!("distribution-pole:{key}:{side}:{k}"), x: g[0], z: g[2], base_y: g[1], yaw: (-rz).atan2(rx), rx, rz });
            }
            if !chain.poles.is_empty() {
                out.chains.push(chain);
            }
        }
        out
    }

    /// Distribution entries sharing an end node with `ei`.
    fn dist_neighbours(&self, w: &PropsWorld, ei: usize) -> Vec<usize> {
        let e = &w.street.plan.entries[ei];
        let mut v = vec![];
        for (ni, _) in [e.approach_start, e.approach_end].into_iter().flatten() {
            for app in &w.street.plan.nodes[ni].approaches {
                if app.entry != ei && self.dist[app.entry].is_some() && !v.contains(&app.entry) {
                    v.push(app.entry);
                }
            }
        }
        v
    }

    fn dist_raw(&self, w: &PropsWorld, ei: u32) -> Arc<DistEntry> {
        cached(&self.dist_raw_cache, ei, 16384, || self.plan_dist_entry(w, ei as usize, &[]))
    }

    /// The final chains: a run keeps the lower-quartile spacing from the poles of
    /// every lower-numbered distribution run it meets at a node (their unchecked
    /// plan), as the JS keeps it from every pole planned before it.
    fn dist_final(&self, w: &PropsWorld, ei: u32) -> Arc<DistEntry> {
        cached(&self.dist_cache, ei, 16384, || {
            let mut against: Vec<[f64; 2]> = vec![];
            for nb in self.dist_neighbours(w, ei as usize) {
                if nb < ei as usize {
                    for c in &self.dist_raw(w, nb as u32).chains {
                        against.extend(c.poles.iter().map(|p| [p.x, p.z]));
                    }
                }
            }
            if against.is_empty() {
                return DistEntry { chains: self.dist_raw(w, ei).chains.clone() };
            }
            self.plan_dist_entry(w, ei as usize, &against)
        })
    }

    fn seat_at(p: &DistPole, s: &Seat) -> [f64; 3] {
        let (c, sn) = (p.yaw.cos(), p.yaw.sin());
        [p.x + s.x * c + s.z * sn, p.base_y + s.y, p.z - s.x * sn + s.z * c]
    }

    /// `hang(a, b, kind)`.
    fn hang(&self, a: [f64; 3], b: [f64; 3], kind: WireKind) -> Option<Wire> {
        let h = &self.h;
        let length = (b[0] - a[0]).hypot(b[2] - a[2]);
        if !(length > 0.5) {
            return None;
        }
        let steps = 3usize.max((length / h.sample_step).ceil() as usize);
        let sag = length * (h.support_sag / h.support_spacing);
        let points = (0..=steps)
            .map(|i| {
                let t = i as f64 / steps as f64;
                f3(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t - sag * 4.0 * t * (1.0 - t), a[2] + (b[2] - a[2]) * t)
            })
            .collect();
        Some(Wire { kind, system: WireSystem::None, points, radius_m: self.js.wire_radius(kind) })
    }

    /// `span(p, q)`: the conductors between two poles; false where the JS stops a chain.
    fn dist_span(&self, p: &DistPole, q: &DistPole, out: &mut Vec<Wire>) -> bool {
        let seats = &self.js.seats;
        for s in seats {
            match self.hang(Self::seat_at(p, s), Self::seat_at(q, s), if s.arm == 0 { WireKind::Primary } else { WireKind::Secondary }) {
                Some(wr) => out.push(wr),
                None => return false,
            }
        }
        let comm_offset = self.js.consts["utilityPole"]["shaftRadiusM"].as_f64().unwrap_or(0.0);
        let hardware_from = self.js.consts["utilityPole"]["hardwareFromM"].as_f64().unwrap_or_else(|| seats.last().map(|s| s.y).unwrap_or(0.0));
        let floor = self.std.comm_floor_m;
        let cables = self.h.comm_cables;
        for i in 0..cables {
            let hh = floor + (hardware_from - floor) * i as f64 / cables.max(1) as f64;
            let at = |pole: &DistPole| [pole.x + pole.rx * comm_offset, pole.base_y + hh, pole.z + pole.rz * comm_offset];
            match self.hang(at(p), at(q), WireKind::Comm) {
                Some(wr) => out.push(wr),
                None => return false,
            }
        }
        true
    }

    /// Chains a join may reach (same street name, any entry with an end within reach).
    fn join_candidates(&self, w: &PropsWorld, ei: usize, reach: f64) -> Vec<usize> {
        let e = &w.street.plan.entries[ei];
        let ends = [e.points[0], e.points[e.points.len() - 1]];
        let name = self.street_name(w, ei);
        let mut out = vec![];
        for &o in self.dist_by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[]) {
            let oe = &w.street.plan.entries[o as usize];
            let oends = [oe.points[0], oe.points[oe.points.len() - 1]];
            if o as usize == ei || ends.iter().any(|a| oends.iter().any(|b| (a[0] - b[0]).hypot(a[1] - b[1]) <= reach)) {
                out.push(o as usize);
            }
        }
        out
    }

    /// The wires and poles of one entry's distribution (spans, joins it owns, down guys).
    fn dist_wires(&self, w: &PropsWorld, ei: usize, poles: &mut Vec<OverheadPole>, wires: &mut Vec<Wire>) {
        let de = self.dist_final(w, ei as u32);
        if de.chains.is_empty() {
            return;
        }
        let row = &self.js.consts["utilityPole"];
        let (height, shaft) = (row["heightM"].as_f64().unwrap_or(0.0), row["shaftRadiusM"].as_f64().unwrap_or(0.0));
        let mount = self.std.luminaire_height_m;
        for c in &de.chains {
            for p in &c.poles {
                poles.push(OverheadPole {
                    kind: PoleKind::Distribution,
                    material: PoleMaterial::Wood,
                    pos: [p.x, p.base_y, p.z],
                    yaw: p.yaw,
                    height,
                    radius_base: shaft,
                    radius_top: shaft,
                    arm: Some(PoleArm { pos: [p.x + p.rx * shaft, p.base_y + mount, p.z + p.rz * shaft], yaw: p.yaw }),
                    id: p.id.clone(),
                });
            }
            for i in 0..c.poles.len().saturating_sub(1) {
                if !self.dist_span(&c.poles[i], &c.poles[i + 1], wires) {
                    break;
                }
            }
        }
        // Joins across junctions (`joinSpacings`), each owned by the lower (entry, side) chain.
        let spacing = self.js.consts["distributionSpacingM"].as_f64().unwrap();
        let join_max = spacing * self.h.join_spacings;
        let mut joined: HashSet<String> = HashSet::new();
        for o in self.join_candidates(w, ei, join_max + 1.0) {
            let other = self.dist_final(w, o as u32);
            for a in &de.chains {
                for b in &other.chains {
                    if (b.entry, b.side) == (a.entry, a.side) || a.name != b.name || a.compass != b.compass {
                        continue;
                    }
                    let (lo, hi) = if (a.entry, a.side) < (b.entry, b.side) { (a, b) } else { (b, a) };
                    let mut best: Option<(&DistPole, &DistPole, f64)> = None;
                    for p in [&lo.poles[0], &lo.poles[lo.poles.len() - 1]] {
                        for q in [&hi.poles[0], &hi.poles[hi.poles.len() - 1]] {
                            let gap = (p.x - q.x).hypot(p.z - q.z);
                            if best.is_none_or(|b| gap < b.2) {
                                best = Some((p, q, gap));
                            }
                        }
                    }
                    let Some((p, q, gap)) = best else { continue };
                    if gap > join_max || gap < 1.0 {
                        continue;
                    }
                    joined.insert(p.id.clone());
                    joined.insert(q.id.clone());
                    // Same entry, two kerbs: one pair visit owns it.
                    if (a.entry, a.side) == (lo.entry, lo.side) {
                        self.dist_span(p, q, wires);
                    }
                }
            }
        }
        // Down guys at chain ends no join carries on.
        let Some(top) = self.js.seats.iter().copied().reduce(|a, b| if b.y > a.y { b } else { a }) else { return };
        let guy_r = self.js.wire_radius(WireKind::Guy);
        for c in &de.chains {
            if c.poles.len() < 2 {
                continue;
            }
            let n = c.poles.len();
            for (end, next) in [(&c.poles[0], &c.poles[1]), (&c.poles[n - 1], &c.poles[n - 2])] {
                if joined.contains(&end.id) {
                    continue;
                }
                let (dx, dz) = (end.x - next.x, end.z - next.z);
                let mut l = dx.hypot(dz);
                if l == 0.0 {
                    l = 1.0;
                }
                let lead = top.y * self.h.guy_lead_ratio;
                let (ax, az) = (end.x + dx / l * lead, end.z + dz / l * lead);
                wires.push(Wire { kind: WireKind::Guy, system: WireSystem::None, points: vec![f3(end.x, end.base_y + top.y, end.z), f3(ax, self.footway(w, ax, az), az)], radius_m: guy_r });
            }
        }
    }

    /// Service drops from the distribution poles owned by `key`.
    fn service_drops(&self, w: &PropsWorld, key: (i32, i32), near_entries: &[u32], out: &mut Vec<Wire>) {
        let spacing = self.js.consts["distributionSpacingM"].as_f64().unwrap();
        let mut poles: Vec<DistPole> = vec![];
        for &ei in near_entries {
            if self.dist[ei as usize].is_none() {
                continue;
            }
            for c in &self.dist_final(w, ei).chains {
                poles.extend(c.poles.iter().cloned());
            }
        }
        let own: Vec<usize> = (0..poles.len()).filter(|&i| eo_street::tile_key(poles[i].x, poles[i].z) == key).collect();
        if own.is_empty() {
            return;
        }
        let max_arm = self.js.seats.iter().map(|s| s.arm).max().unwrap_or(0);
        let low: Vec<Seat> = self.js.seats.iter().copied().filter(|s| s.arm == max_arm).collect();
        if low.is_empty() {
            return;
        }
        let mut cand: Vec<u32> = vec![];
        let r = spacing + 60.0;
        for &i in &own {
            let p = &poles[i];
            for gz in ((p.z - r) / BUILDING_CELL_M).floor() as i32..=((p.z + r) / BUILDING_CELL_M).floor() as i32 {
                for gx in ((p.x - r) / BUILDING_CELL_M).floor() as i32..=((p.x + r) / BUILDING_CELL_M).floor() as i32 {
                    if let Some(v) = self.building_grid.get(&(gx, gz)) {
                        cand.extend(v.iter().copied());
                    }
                }
            }
        }
        cand.sort_unstable();
        cand.dedup();
        for bi in cand {
            let b = &self.buildings[bi as usize];
            if b.ring.len() < 3 || !(b.height > 0.0) || !(b.levels >= 1.0) {
                continue;
            }
            let p0 = b.ring[0];
            let mut best: Option<(usize, f64, f64, f64)> = None;
            for (pi, pole) in poles.iter().enumerate() {
                if (pole.x - p0[0]).abs() > spacing + 60.0 || (pole.z - p0[1]).abs() > spacing + 60.0 {
                    continue;
                }
                for i in 0..b.ring.len() {
                    let (a, c) = (b.ring[i], b.ring[(i + 1) % b.ring.len()]);
                    let (dx, dz) = (c[0] - a[0], c[1] - a[1]);
                    let mut l2 = dx * dx + dz * dz;
                    if l2 == 0.0 {
                        l2 = 1.0;
                    }
                    let t = (((pole.x - a[0]) * dx + (pole.z - a[1]) * dz) / l2).clamp(0.0, 1.0);
                    let (x, z) = (a[0] + dx * t, a[1] + dz * t);
                    let gap = (pole.x - x).hypot(pole.z - z);
                    if best.is_none_or(|bb| gap < bb.3) {
                        best = Some((pi, x, z, gap));
                    }
                }
            }
            let Some((pi, x, z, gap)) = best else { continue };
            if gap > spacing || gap < 1.0 || !own.contains(&pi) {
                continue;
            }
            let pole = &poles[pi];
            let mut seat: Option<([f64; 3], f64)> = None;
            for s in &low {
                let ws = Self::seat_at(pole, s);
                let d = (ws[0] - x).hypot(ws[2] - z);
                if seat.is_none_or(|m| d < m.1) {
                    seat = Some((ws, d));
                }
            }
            let storey = (b.height / b.levels).min(b.height - 0.3);
            let end = [x, self.footway(w, x, z) + storey, z];
            if let Some(wr) = self.hang(seat.unwrap().0, end, WireKind::Drop) {
                out.push(wr);
            }
        }
    }

    // ---- the tile ------------------------------------------------------------

    /// Everything the overhead stands in one 128 m street tile.
    pub fn plan_tile(&self, w: &PropsWorld, key: (i32, i32)) -> OverheadTile {
        let mut out = OverheadTile::default();
        let inside = |x: f64, z: f64| eo_street::tile_key(x, z) == key;
        let mut entries: Vec<u32> = vec![];
        let mut nodes: Vec<u32> = vec![];
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(t) = w.street.tiles.get(&(key.0 + dx, key.1 + dz)) {
                    entries.extend(t.spans.iter().map(|s| s.0));
                    nodes.extend(t.nodes.iter().copied());
                }
            }
        }
        entries.sort_unstable();
        entries.dedup();
        nodes.sort_unstable();
        nodes.dedup();
        let take_wires = |src: &[Wire], out: &mut Vec<Wire>| out.extend(src.iter().filter(|wr| owner_of(wr) == key).cloned());
        for &ei in &entries {
            let s = self.supports(w, ei);
            take_wires(&s.contact, &mut out.wires);
            take_wires(&s.wires, &mut out.wires);
            out.poles.extend(s.poles.iter().filter(|p| inside(p.pos[0], p.pos[2])).cloned());
            out.signs.extend(s.signs.iter().filter(|p| inside(p.pos[0], p.pos[2])).cloned());
            let r = self.rest(w, ei);
            out.signs.extend(r.picket.iter().filter(|p| inside(p.pos[0], p.pos[2])).cloned());
            take_wires(&r.lantern_wires, &mut out.wires);
            out.lanterns.extend(r.lanterns.iter().filter(|l| inside(l.pos[0], l.pos[2])).copied());
            if self.dist[ei as usize].is_some() {
                let d = cached(&self.dist_wire_cache, ei, 16384, || {
                    let (mut poles, mut wires) = (vec![], vec![]);
                    self.dist_wires(w, ei as usize, &mut poles, &mut wires);
                    (poles, wires)
                });
                out.poles.extend(d.0.iter().filter(|p| inside(p.pos[0], p.pos[2])).cloned());
                take_wires(&d.1, &mut out.wires);
            }
        }
        self.service_drops(w, key, &entries, &mut out.wires);
        for &ni in &nodes {
            let np = self.node(w, ni);
            out.poles.extend(np.poles.iter().filter(|p| inside(p.pos[0], p.pos[2])).cloned());
            take_wires(&np.spans, &mut out.wires);
            out.corner_signs.extend(np.corner_signs.iter().filter(|c| inside(c.pos[0], c.pos[2])).copied());
            out.cabinets.extend(np.cabinet.iter().filter(|c| inside(c.pos[0], c.pos[2])).copied());
        }
        if let Some(l) = self.links.get(&key) {
            out.wires.extend(l.iter().cloned());
        }
        out
    }
}

#[derive(Clone, Copy)]
struct Anchor {
    x: f64,
    z: f64,
    y: f64,
    /// Hung off a standing signal mast.
    host: bool,
    base_y: f64,
    bx: f64,
    bz: f64,
    radius: f64,
}

/// `sidewalkBand`'s furnishing zone.
struct FBand {
    walk: f64,
    furnish_inner: f64,
    furnish_outer: f64,
}

/// A wire's owner tile: the midpoint of its end points.
pub fn owner_of(w: &Wire) -> (i32, i32) {
    let (a, b) = (w.points[0], w.points[w.points.len() - 1]);
    eo_street::tile_key((a[0] as f64 + b[0] as f64) / 2.0, (a[2] as f64 + b[2] as f64) / 2.0)
}

fn in_ramp(windows: &[RampWin], s: f64, clear: f64) -> bool {
    windows.iter().any(|w| {
        let lo = (if w.flare_lo.is_finite() { w.flare_lo.min(w.lo) } else { w.lo }) - clear;
        let hi = (if w.flare_hi.is_finite() { w.flare_hi.max(w.hi) } else { w.hi }) + clear;
        s > lo && s < hi
    })
}

/// `stationInYieldSpan(segment, side, s, margin)`.
fn station_in_yield_span(e: &Entry, side: f64, s: f64, margin: f64) -> bool {
    let pad = margin.max(0.0);
    let walk = &e.walk_yield_spans[if side > 0.0 { 0 } else { 1 }];
    walk.iter().chain(e.yield_spans.iter()).any(|sp| s >= sp[0] - pad && s <= sp[1] + pad)
}

/// `observedSegmentSide(overlay, frame, 'N')`.
fn observed_side_n(o: &Overlay, nx: f64, nz: f64) -> f64 {
    let (from, to) = (&o.axis["from"], &o.axis["to"]);
    let (fx, fz, tx, tz) = (from["x"].as_f64().unwrap_or(0.0), from["z"].as_f64().unwrap_or(0.0), to["x"].as_f64().unwrap_or(0.0), to["z"].as_f64().unwrap_or(0.0));
    let mut l = (tx - fx).hypot(tz - fz);
    if l == 0.0 {
        l = 1.0;
    }
    let (ax, az) = ((tx - fx) / l, (tz - fz) / l);
    // N normal (x east, z south): (tz, -tx).
    if nx * az + nz * -ax >= 0.0 { 1.0 } else { -1.0 }
}

/// `Math.round` (half up).
fn js_round(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

// ---------------------------------------------------------------- shared entry point

type Registry = Mutex<Vec<(usize, Arc<Overhead>)>>;

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Mutex::new(vec![]))
}

/// Build (or replace) the overhead index of a props world, with the buildings service drops land on.
pub fn install(w: &PropsWorld, buildings: Vec<DropBuilding>) -> Result<Arc<Overhead>> {
    let paths = eo_data::DataPaths::discover()?;
    let std = OverheadStandards::load(&eo_data::CaStandards::load(&paths)?)?;
    let o = Arc::new(Overhead::new(w, Arc::new(OverheadJs::load()?), std, buildings));
    let id = Arc::as_ptr(&w.street) as usize;
    let mut r = registry().lock().unwrap();
    r.retain(|(k, _)| *k != id);
    r.push((id, o.clone()));
    Ok(o)
}

/// The overhead index of a props world (built on first use; service drops
/// need [`install`] with the city's buildings, else the loader's are read here).
pub fn shared(w: &PropsWorld) -> Arc<Overhead> {
    let id = Arc::as_ptr(&w.street) as usize;
    {
        let r = registry().lock().unwrap();
        if let Some((_, o)) = r.iter().find(|(k, _)| *k == id) {
            return o.clone();
        }
    }
    static BUILD: Mutex<()> = Mutex::new(());
    let _g = BUILD.lock().unwrap();
    {
        let r = registry().lock().unwrap();
        if let Some((_, o)) = r.iter().find(|(k, _)| *k == id) {
            return o.clone();
        }
    }
    let buildings = eo_data::DataPaths::discover().ok().and_then(|p| eo_data::City::load(&p).ok()).map(|c| DropBuilding::from_city(&c.buildings)).unwrap_or_default();
    install(w, buildings).expect("overhead index")
}

/// Everything the street overhead stands in one 128 m street tile (owned by
/// position); with the grade-separated roads (`--structures`), kept clear
/// of them (`clearance::clear_overhead`). The shared index and its caches
/// never see the structures: a world without them gets what it always did.
pub fn plan_tile(w: &PropsWorld, key: (i32, i32)) -> OverheadTile {
    plan_tile_stats(w, key).0
}

/// `plan_tile`, with what keeping clear of the structures did there.
pub fn plan_tile_stats(w: &PropsWorld, key: (i32, i32)) -> (OverheadTile, crate::clearance::ClearStats) {
    let mut t = shared(w).plan_tile(w, key);
    let mut stats = crate::clearance::ClearStats::default();
    if let Some(s) = w.clear_structures() {
        crate::clearance::clear_overhead(w, s, key, &mut t, &mut stats);
    }
    (t, stats)
}
