//! The street surface options, as the browser game's stream tiles compile with
//! them: `resolveStreetSurfaceOptions(city, streetTileSurfaceOptions(city))`
//! (src/world/streets/street-surface-v2.js, src/world/stream/street-tile-mesh.js),
//! written by the earlier browser version's exporter into `data/street-options.json`.
//!
//! There are no street dimensions written as literals in this crate (the
//! CLAUDE.md `caMetric()` rule). Every number below is read from that file;
//! the ones the JS reads through `caMetric` are re-derived here from the cited
//! table (`scripts/world-data/sources/california-street-standards.json`, the
//! source the JS table is generated from) by [`standards_check`], and the test
//! `standards` proves both equal.

use anyhow::{Context, Result, anyhow};
use eo_data::CaStandards;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

pub const OPTIONS_JSON: &str = include_str!("../data/street-options.json");

static PAVEMENT_REPAIRS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Select the street geometry before building the world (matched native captures).
pub fn set_pavement_repairs(enabled: bool) {
    PAVEMENT_REPAIRS.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

#[derive(Clone, Debug)]
pub struct StreetOptions {
    pub pavement_repairs: bool,
    pub(crate) surveyed_junction: Option<std::sync::Arc<crate::mesh::surveyed_junction::Survey>>,
    pub(crate) driveway: crate::driveway::Dimensions,
    // vertical construction
    pub road_lift: f64,
    pub cross_slope: f64,
    pub gutter_width: f64,
    pub gutter_depth: f64,
    pub curb_face_height: f64,
    pub curb_top_width: f64,
    pub curb_top_fall: f64,
    pub sidewalk_cross_slope: f64,
    pub min_sidewalk_width: f64,
    // verge bank and frontage
    pub verge_reach: f64,
    pub verge_grade_tan: f64,
    pub verge_min_reach: f64,
    pub verge_max_drop: f64,
    pub verge_undercut: f64,
    pub ground_sink: f64,
    pub frontage_reach: f64,
    pub frontage_step: f64,
    pub frontage_past_line: f64,
    // sampling, nodes and corners
    pub max_step: f64,
    pub node_snap: f64,
    pub infer_nodes: bool,
    pub corner_radius: f64,
    pub corner_min_angle_deg: f64,
    pub corner_max_angle_deg: f64,
    pub corner_arc_step_deg: f64,
    // kerb ramps
    pub ramp_width: f64,
    pub ramp_target_slope: f64,
    pub ramp_run: f64,
    pub ramp_flare_run: f64,
    pub ramp_lip_height: f64,
    pub ramp_lift: f64,
    // paint
    pub marking_lift: f64,
    pub junction_paint_lift: f64,
    pub edge_line_width: f64,
    pub centre_line_width: f64,
    pub centre_line_gap: f64,
    pub lane_line_width: f64,
    pub dash_mark: f64,
    pub dash_gap: f64,
    pub dash_mark_fast: f64,
    pub dash_gap_fast: f64,
    pub twltl_dash_mark: f64,
    pub twltl_dash_gap: f64,
    pub min_marked_half_width: f64,
    pub parking_lanes: bool,
    pub parking_max_class_rank: f64,
    pub parking_max_lane_width: f64,
    pub parking_moving_lane_min: f64,
    pub parking_min_width: f64,
    pub parking_max_width: f64,
    pub parking_line_width: f64,
    pub parking_stall_pitch: f64,
    pub edge_line_min_class_rank: f64,
    pub crosswalk_band_depth: f64,
    pub crosswalk_edge_inset: f64,
    pub crosswalk_clearance: f64,
    pub stop_bar_depth: f64,
    pub stop_bar_clearance: f64,
    pub stop_bar_edge_inset: f64,
    pub marking_box_clearance: f64,
    pub marking_solid_approach: f64,
    pub crossing_stripe_pitch: f64,
    pub crossing_stripe_width: f64,
    pub crossing_min_stripes: f64,
    pub driving_side_sign: f64,
    pub pave_grade_separated: bool,
    pub yield_clear_fraction: f64,
    pub presentation_duplicate_cover: f64,
    pub presentation_duplicate_half_slack: f64,
    pub exclude_highways: HashSet<String>,
    pub unmarked_highways: HashSet<String>,
    // CA MUTCD 3B.02 centre-line warrant (`centerline-warrant`)
    pub centre_shall_lanes: f64,
    pub centre_traveled_way_min: f64,
    pub centre_may_traveled_way_min: f64,
    // derived by the JS (ground-datum.js, lane-assignment.js, yard-floor-rule.js, ...)
    pub footway_lift: f64,
    pub open_ground_lift: f64,
    pub frontage_lift: f64,
    pub sidewalk_clear_path_min: f64,
    pub twltl_solid_to_broken: f64,
    pub line_width_local: [f64; 2],
    pub walk_probe_m: f64,
    pub bank_step_m: f64,
    pub bank_reach_m: f64,
    pub ramp_running_slope_max: f64,
    /// OSM ways of two-way streets tagged with 3+ moving lanes (sf-centre-lines.generated.js).
    pub multi_lane_two_way: Vec<u64>,
    pub floor_probe_stations: usize,
    pub front_grade_max_relief_m: f64,
    pub front_clamp_m: f64,
    pub archetype_foundation_skirt_m: f64,
    pub foundation_embed_m: f64,
    /// Joins with the grade-separated roads (`crate::joins`), handed in by a
    /// host that builds them (eo-app with `--structures`). None, the default
    /// and what `parse` reads: the street is drawn exactly as without them.
    /// Landing elevations requested by the connected bridge profiles, in the game frame.
    pub landings: Vec<([f64; 2], crate::joins::Rise)>,
    pub structures: Option<std::sync::Arc<crate::joins::StructureJoins>>,
    /// (`--street-tapers`) a run joined end to end with a narrower one (a
    /// plain joint: no junction, no trim) narrows to its width over the
    /// lane-reduction taper (`caMetric('lane-reduction-taper')` at the prima
    /// facie speed, CA MUTCD 3B.12), kerb, gutter and crown with it, its
    /// building line held, instead of a kerb jog and a lip across the road at
    /// the joint (`StreetWorld::half_at`). Off, the default: as before. The
    /// collision world reads the drawn street, so client and server must
    /// agree (the handshake compares it, eo-net `structures_code`).
    pub tapers: bool,
    /// (`--bikeways`, crate::bikeway) each kerbside bike lane drawn as the
    /// city's bike network says it is built: green only where painted green,
    /// a raised concrete island or tubular markers in its buffer, diagonal
    /// buffer markings, a two-way bikeway's centre line, the bicycle symbol
    /// and arrow after each intersection. Off, the default: as before. The
    /// island is part of the collision world (the handshake compares it).
    pub bikeways: bool,
    /// (`--survey-parking`, `StreetCity::apply_parking_survey`) a kerbside bike
    /// lane's side that the city's parking regulations or meters show cars park
    /// along, and that the street data drew with no parking lane, given one
    /// outside the lane's buffer (traffic | parking | buffer or island | bike
    /// lane | kerb), the carriageway widened by it and each footway narrowed by
    /// half of it (the building line held). Off, the default: as before.
    pub survey_parking: bool,
    /// (`--midblock-crossings`, `StreetWorld::plan_midblock`) each signalised,
    /// beaconed or flashing crossing between junctions (DataSF Traffic
    /// Signals' MIDBLOCK devices) drawn: a continental crosswalk across the
    /// run, kerb ramps both sides, limit lines for the traffic arriving, the
    /// lane lines and a bike lane's island broken across it. Off: as before.
    pub midblock: bool,
}

/// `--survey-parking` for every street world this process builds (as `set_tapers`).
static SURVEY_PARKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `--midblock-crossings` for every street world this process builds.
static MIDBLOCK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_survey_parking(on: bool) {
    SURVEY_PARKING.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn survey_parking() -> bool {
    SURVEY_PARKING.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_midblock(on: bool) {
    MIDBLOCK.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn midblock() -> bool {
    MIDBLOCK.load(std::sync::atomic::Ordering::Relaxed)
}

/// `--bikeways` for every street world this process builds (as `set_tapers`).
static BIKEWAYS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_bikeways(on: bool) {
    BIKEWAYS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn bikeways() -> bool {
    BIKEWAYS.load(std::sync::atomic::Ordering::Relaxed)
}

/// `--street-tapers` for every street world this process builds (eo-app,
/// eo-server, before the first build); every `StreetOptions::parse` reads it,
/// so the drawn street, the props world and the collision world agree.
static TAPERS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_tapers(on: bool) {
    TAPERS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn tapers() -> bool {
    TAPERS.load(std::sync::atomic::Ordering::Relaxed)
}

fn num(v: &Value, key: &str) -> Result<f64> {
    v.get(key).and_then(|x| x.as_f64()).ok_or_else(|| anyhow!("street-options.json: no number \"{key}\""))
}
fn flag(v: &Value, key: &str) -> Result<bool> {
    v.get(key).and_then(|x| x.as_bool()).ok_or_else(|| anyhow!("street-options.json: no boolean \"{key}\""))
}
fn set(v: &Value, key: &str) -> Result<HashSet<String>> {
    Ok(v.get(key)
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("street-options.json: no list \"{key}\""))?
        .iter()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect())
}

impl StreetOptions {
    pub fn load() -> Result<Self> {
        let mut options = Self::parse(OPTIONS_JSON)?;
        if !std::env::args_os().any(|arg| arg == "--no-surveyed-junction") {
            if let Some(path) = std::env::var_os("EO_SURVEYED_JUNCTION") {
                let survey = crate::mesh::surveyed_junction::Survey::read(Path::new(&path))?;
                options.surveyed_junction = Some(std::sync::Arc::new(survey));
            }
        }
        Ok(options)
    }

    pub fn load_file(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&s)
    }

    pub fn parse(json: &str) -> Result<Self> {
        let root: Value = serde_json::from_str(json)?;
        let o = root.get("options").ok_or_else(|| anyhow!("street-options.json: no options"))?;
        let d = root.get("derived").ok_or_else(|| anyhow!("street-options.json: no derived"))?;
        let l = root.get("lists").ok_or_else(|| anyhow!("street-options.json: no lists"))?;
        let w = root.get("centreLineWarrant").ok_or_else(|| anyhow!("street-options.json: no centreLineWarrant"))?;
        let lwl = d.get("lineWidthLocalM").and_then(|x| x.as_array()).ok_or_else(|| anyhow!("no lineWidthLocalM"))?;
        Ok(Self {
            driveway: crate::driveway::Dimensions::load(&CaStandards::bundled()?)?,
            pavement_repairs: PAVEMENT_REPAIRS.load(std::sync::atomic::Ordering::Relaxed),
            surveyed_junction: None,
            road_lift: num(o, "roadLift")?,
            cross_slope: num(o, "crossSlope")?,
            gutter_width: num(o, "gutterWidth")?,
            gutter_depth: num(o, "gutterDepth")?,
            curb_face_height: num(o, "curbFaceHeight")?,
            curb_top_width: num(o, "curbTopWidth")?,
            curb_top_fall: num(o, "curbTopFall")?,
            sidewalk_cross_slope: num(o, "sidewalkCrossSlope")?,
            min_sidewalk_width: num(o, "minSidewalkWidth")?,
            verge_reach: num(o, "vergeReach")?,
            verge_grade_tan: num(o, "vergeGradeTan")?,
            verge_min_reach: num(o, "vergeMinReach")?,
            verge_max_drop: num(o, "vergeMaxDrop")?,
            verge_undercut: num(o, "vergeUndercut")?,
            ground_sink: num(o, "groundSink")?,
            frontage_reach: num(o, "frontageReach")?,
            frontage_step: num(o, "frontageStep")?,
            frontage_past_line: num(o, "frontagePastLine")?,
            max_step: num(o, "maxStep")?,
            node_snap: num(o, "nodeSnap")?,
            infer_nodes: flag(o, "inferNodes")?,
            corner_radius: num(o, "cornerRadius")?,
            corner_min_angle_deg: num(o, "cornerMinAngleDeg")?,
            corner_max_angle_deg: num(o, "cornerMaxAngleDeg")?,
            corner_arc_step_deg: num(o, "cornerArcStepDeg")?,
            ramp_width: num(o, "rampWidth")?,
            ramp_target_slope: num(o, "rampTargetSlope")?,
            ramp_run: num(o, "rampRun")?,
            ramp_flare_run: num(o, "rampFlareRun")?,
            ramp_lip_height: num(o, "rampLipHeight")?,
            ramp_lift: num(o, "rampLift")?,
            marking_lift: num(o, "markingLift")?,
            junction_paint_lift: num(o, "junctionPaintLift")?,
            edge_line_width: num(o, "edgeLineWidth")?,
            centre_line_width: num(o, "centreLineWidth")?,
            centre_line_gap: num(o, "centreLineGap")?,
            lane_line_width: num(o, "laneLineWidth")?,
            dash_mark: num(o, "dashMark")?,
            dash_gap: num(o, "dashGap")?,
            dash_mark_fast: num(o, "dashMarkFast")?,
            dash_gap_fast: num(o, "dashGapFast")?,
            twltl_dash_mark: num(o, "twltlDashMark")?,
            twltl_dash_gap: num(o, "twltlDashGap")?,
            min_marked_half_width: num(o, "minMarkedHalfWidth")?,
            parking_lanes: flag(o, "parkingLanes")?,
            parking_max_class_rank: num(o, "parkingMaxClassRank")?,
            parking_max_lane_width: num(o, "parkingMaxLaneWidth")?,
            parking_moving_lane_min: num(o, "parkingMovingLaneMin")?,
            parking_min_width: num(o, "parkingMinWidth")?,
            parking_max_width: num(o, "parkingMaxWidth")?,
            parking_line_width: num(o, "parkingLineWidth")?,
            parking_stall_pitch: num(o, "parkingStallPitch")?,
            edge_line_min_class_rank: num(o, "edgeLineMinClassRank")?,
            crosswalk_band_depth: num(o, "crosswalkBandDepth")?,
            crosswalk_edge_inset: num(o, "crosswalkEdgeInset")?,
            crosswalk_clearance: num(o, "crosswalkClearance")?,
            stop_bar_depth: num(o, "stopBarDepth")?,
            stop_bar_clearance: num(o, "stopBarClearance")?,
            stop_bar_edge_inset: num(o, "stopBarEdgeInset")?,
            marking_box_clearance: num(o, "markingBoxClearance")?,
            marking_solid_approach: num(o, "markingSolidApproach")?,
            crossing_stripe_pitch: num(o, "crossingStripePitch")?,
            crossing_stripe_width: num(o, "crossingStripeWidth")?,
            crossing_min_stripes: num(o, "crossingMinStripes")?,
            driving_side_sign: num(o, "drivingSideSign")?,
            pave_grade_separated: flag(o, "paveGradeSeparated")?,
            yield_clear_fraction: num(o, "yieldClearFraction")?,
            presentation_duplicate_cover: num(o, "presentationDuplicateCover")?,
            presentation_duplicate_half_slack: num(o, "presentationDuplicateHalfSlack")?,
            exclude_highways: set(l, "excludeHighways")?,
            unmarked_highways: set(l, "unmarkedHighways")?,
            centre_shall_lanes: num(w, "shallLanes")?,
            centre_traveled_way_min: num(w, "traveledWayMin")?,
            centre_may_traveled_way_min: num(w, "mayTraveledWayMin")?,
            footway_lift: num(d, "footwayLiftM")?,
            open_ground_lift: num(d, "openGroundLiftM")?,
            frontage_lift: num(d, "frontageLiftM")?,
            sidewalk_clear_path_min: num(d, "sidewalkClearPathMinM")?,
            twltl_solid_to_broken: num(d, "twltlSolidToBrokenSeparationM")?,
            line_width_local: [
                lwl.first().and_then(|x| x.as_f64()).ok_or_else(|| anyhow!("lineWidthLocalM"))?,
                lwl.get(1).and_then(|x| x.as_f64()).ok_or_else(|| anyhow!("lineWidthLocalM"))?,
            ],
            walk_probe_m: num(d, "walkProbeM")?,
            bank_step_m: num(d, "bankStepM")?,
            bank_reach_m: num(d, "bankReachM")?,
            ramp_running_slope_max: num(d, "rampRunningSlopeMax")?,
            multi_lane_two_way: root
                .get("multiLaneTwoWay")
                .and_then(|x| x.as_array())
                .ok_or_else(|| anyhow!("street-options.json: no multiLaneTwoWay"))?
                .iter()
                .filter_map(|v| v.as_u64())
                .collect(),
            floor_probe_stations: num(d, "floorProbeStations")? as usize,
            front_grade_max_relief_m: num(d, "frontGradeMaxReliefM")?,
            front_clamp_m: num(d, "frontClampM")?,
            archetype_foundation_skirt_m: num(d, "archetypeFoundationSkirtM")?,
            foundation_embed_m: num(d, "foundationEmbedM")?,
            structures: None,
            landings: vec![],
            tapers: tapers(),
            bikeways: bikeways(),
            survey_parking: survey_parking(),
            midblock: midblock(),
        })
    }

    /// The same options with the grade-separated roads' joins (`None` and an
    /// empty set both leave the street as it is without them).
    pub fn with_structures(mut self, joins: Option<std::sync::Arc<crate::joins::StructureJoins>>) -> Self {
        self.structures = joins.filter(|j| !j.is_empty());
        self
    }

    /// Cache tag for overlays that change the drawn street surface.
    /// The tag is empty when both overlays are absent.
    pub fn structures_tag(&self) -> String {
        let mut tag = match &self.structures {
            Some(j) => {
                use std::hash::{Hash, Hasher};
                let mut digest = std::collections::hash_map::DefaultHasher::new();
                for (at, rise) in &self.landings {
                    [at[0], at[1], rise.to, rise.grade].map(f64::to_bits).hash(&mut digest);
                }
                format!("street-structure-joins-v1:{:016x}:surface-v2:{:016x}", j.digest(), digest.finish())
            },
            None => String::new(),
        };
        if let Some(survey) = &self.surveyed_junction {
            tag.push_str(&format!(":surveyed-junction-v1:{:016x}", survey.digest));
        }
        // the switches that move kerbs or cut ramps (`--street-tapers`, `--survey-parking`,
        // `--midblock-crossings`): their own keys; none of them on, the key as before
        for (on, name) in [(self.tapers, "tapers-v1"), (self.survey_parking, "survey-parking-v1"), (self.midblock, "midblock-v1")] {
            if on {
                tag.push_str(if tag.is_empty() { "street:" } else { ":" });
                tag.push_str(name);
            }
        }
        tag
    }

    /// `curbTopY` (street-surface-v2.js): kerb top above the gutter invert.
    pub fn curb_top_y(&self, datum: f64) -> f64 {
        datum - self.gutter_depth + self.curb_face_height
    }

    /// `crossSectionY`: carriageway height at lateral offset `u`.
    pub fn cross_section_y(&self, datum: f64, u: f64, half: f64) -> f64 {
        let a = u.abs().min(half);
        let crown = self.cross_slope * half;
        let gutter_start = (half - self.gutter_width).max(0.0);
        if a <= gutter_start {
            return datum + crown * (1.0 - a / half);
        }
        let lip = datum + crown * (1.0 - gutter_start / half);
        let invert = datum - self.gutter_depth;
        let t = (a - gutter_start) / (half - gutter_start).max(1e-6);
        lip + (invert - lip) * t
    }

    /// `sectionOffsets`: lateral stations of the carriageway, -half..+half.
    pub fn section_offsets(&self, half: f64) -> Vec<f64> {
        let g = (half - self.gutter_width).max(0.0);
        if g <= 0.05 { vec![-half, 0.0, half] } else { vec![-half, -g, 0.0, g, half] }
    }

    /// `footwayRiseAt`: finished footway above the kerb top `dist` back from the kerb line.
    pub fn footway_rise_at(&self, dist: f64) -> f64 {
        let d = dist.max(0.0);
        if d <= self.curb_top_width {
            return self.curb_top_fall * (d / self.curb_top_width.max(1e-6));
        }
        self.curb_top_fall + self.sidewalk_cross_slope * (d - self.curb_top_width)
    }

    /// `sidewalkSurfaceY`.
    pub fn sidewalk_surface_y(&self, datum: f64, u: f64, half: f64) -> f64 {
        let a = u.abs();
        let top = self.curb_top_y(datum);
        if a <= half { top } else { top + self.footway_rise_at(a - half) }
    }

    /// `gradedBankReach`.
    pub fn graded_bank_reach(&self, drop: f64) -> f64 {
        let full = self.verge_reach.max(0.0);
        if self.verge_grade_tan <= 1e-3 {
            return full;
        }
        let min = self.verge_min_reach.clamp(0.0, full);
        if drop <= 0.0 {
            return min;
        }
        (drop / self.verge_grade_tan).clamp(min, full)
    }
}

/// One option the JS reads through `caMetric`, re-derived here from the cited
/// table: (option, value in street-options.json, value from the table, how).
#[derive(Clone, Debug)]
pub struct StandardCheck {
    pub option: &'static str,
    pub js: f64,
    pub rust: f64,
    pub rule: &'static str,
}

/// Every caMetric-backed option, re-derived from the cited source with the
/// JS's own arithmetic. A "choice in range" (a value the JS states and checks
/// against a cited range with `caChoiceInRange`) is reported with `rust` = the
/// JS value when it lies inside the range, NaN when it does not.
pub fn standards_check(o: &StreetOptions, ca: &CaStandards) -> Result<Vec<StandardCheck>> {
    let mut out = vec![];
    let mut eq = |option, js: f64, rust: f64, rule| out.push(StandardCheck { option, js, rust, rule });
    let inside = |v: f64, lo: f64, hi: f64| if v >= lo - 1e-9 && v <= hi + 1e-9 { v } else { f64::NAN };
    let feet = 0.3048;
    let inch = feet / 12.0;
    // CA_CURB_FACE_HEIGHT_M: the SF 6 in curb read back to the whole inch.
    let curb = (ca.metric("curb-height-sf-standard")? / inch).round() * inch;
    eq("curbFaceHeight", o.curb_face_height, curb, "curb-height-sf-standard, to the whole inch");
    eq(
        "gutterWidth",
        o.gutter_width,
        inside(o.gutter_width, ca.metric_key("gutter-width-sf", "min")?, ca.metric_key("gutter-width-sf", "max")?),
        "choice inside gutter-width-sf",
    );
    eq("sidewalkCrossSlope", o.sidewalk_cross_slope, ca.metric_key("sidewalk-cross-slope-sf", "designMax")?, "sidewalk-cross-slope-sf designMax");
    eq("rampWidth", o.ramp_width, inside(o.ramp_width, ca.metric_key("curb-ramp-width", "sfMin")?, f64::INFINITY), "choice >= curb-ramp-width sfMin");
    eq(
        "rampTargetSlope",
        o.ramp_target_slope,
        inside(
            o.ramp_target_slope,
            ca.metric_key("curb-ramp-running-slope-sf", "designMin")?,
            ca.metric_key("curb-ramp-running-slope-sf", "designMax")?,
        ),
        "choice inside curb-ramp-running-slope-sf design range",
    );
    let rise = curb - o.ramp_lip_height - o.ramp_lift;
    eq("rampRun", o.ramp_run, rise / o.ramp_target_slope, "curb rise / rampTargetSlope");
    eq("rampFlareRun", o.ramp_flare_run, rise / ca.metric_key("curb-ramp-flare-slope", "sfDesignMax")?, "curb rise / curb-ramp-flare-slope sfDesignMax");
    let (lw0, lw1) = (ca.metric_key("line-width-local", "min")?, ca.metric_key("line-width-local", "max")?);
    eq("lineWidthLocal.min", o.line_width_local[0], lw0, "line-width-local min");
    eq("lineWidthLocal.max", o.line_width_local[1], lw1, "line-width-local max");
    eq("edgeLineWidth", o.edge_line_width, inside(o.edge_line_width, lw0, lw1), "choice inside line-width-local");
    eq("laneLineWidth", o.lane_line_width, inside(o.lane_line_width, lw0, lw1), "choice inside line-width-local");
    eq("centreLineWidth", o.centre_line_width, lw1, "line-width-local max (CENTRE_LINE_WIDTH_M)");
    eq("centreLineGap", o.centre_line_gap, ca.metric("double-yellow-spacing")? + lw1, "double-yellow-spacing + stroke");
    eq("dashMark", o.dash_mark, ca.metric_key("broken-line-pattern-le40mph", "line")?, "broken-line-pattern-le40mph line");
    eq("dashGap", o.dash_gap, ca.metric_key("broken-line-pattern-le40mph", "gap")?, "broken-line-pattern-le40mph gap");
    eq("dashMarkFast", o.dash_mark_fast, ca.metric_key("broken-line-pattern-ge45mph", "line")?, "broken-line-pattern-ge45mph line");
    eq("dashGapFast", o.dash_gap_fast, ca.metric_key("broken-line-pattern-ge45mph", "gap")?, "broken-line-pattern-ge45mph gap");
    eq("twltlDashMark", o.twltl_dash_mark, ca.metric_key("twltl-markings", "brokenLine")?, "twltl-markings brokenLine");
    eq("twltlDashGap", o.twltl_dash_gap, ca.metric_key("twltl-markings", "brokenGap")?, "twltl-markings brokenGap");
    eq("twltlSolidToBroken", o.twltl_solid_to_broken, ca.metric_key("twltl-markings", "solidToBrokenSeparation")?, "twltl-markings solidToBrokenSeparation");
    let lane = ca.metric_key("parking-space-marking-layout", "laneWidth")?;
    eq("parkingMinWidth", o.parking_min_width, lane - 0.005, "parking-space-marking-layout laneWidth - 5 mm");
    eq("parkingMaxWidth", o.parking_max_width, lane, "parking-space-marking-layout laneWidth");
    eq("parkingLineWidth", o.parking_line_width, lw0, "line-width-local min");
    eq("crosswalkBandDepth", o.crosswalk_band_depth, inside(o.crosswalk_band_depth, ca.metric_key("crosswalk-min-width", "general")?, f64::INFINITY), "choice >= crosswalk-min-width");
    let lim = (ca.metric_key("limit-line-width", "min")?, ca.metric_key("limit-line-width", "max")?);
    eq("stopBarDepth", o.stop_bar_depth, inside(o.stop_bar_depth, lim.0, lim.1), "choice inside limit-line-width");
    eq("stopBarClearance", o.stop_bar_clearance, ca.metric_key("limit-line-setback-from-crosswalk", "minFromCrosswalk")?, "limit-line-setback-from-crosswalk");
    let bar = ca.metric_range("crosswalk-continental-bars", "barWidth")?;
    let space = ca.metric_range("crosswalk-continental-bars", "spacing")?;
    eq("crossingStripeWidth", o.crossing_stripe_width, bar[1], "crosswalk-continental-bars barWidth max");
    eq("crossingStripePitch", o.crossing_stripe_pitch, bar[1] + inside(bar[1], space[0], space[1]), "bar + a space inside crosswalk-continental-bars spacing");
    eq("centreLineWarrant.shallLanes", o.centre_shall_lanes, ca.metric_key("centerline-warrant", "shallLanes")?, "centerline-warrant");
    eq("centreLineWarrant.traveledWayMin", o.centre_traveled_way_min, ca.metric_key("centerline-warrant", "traveledWayMin")?, "centerline-warrant");
    eq("centreLineWarrant.mayTraveledWayMin", o.centre_may_traveled_way_min, ca.metric_key("centerline-warrant", "mayTraveledWayMin")?, "centerline-warrant");
    eq("rampRunningSlopeMax", o.ramp_running_slope_max, ca.metric("ramp-running-slope-max")?, "ramp-running-slope-max");
    eq("sidewalkClearPathMin", o.sidewalk_clear_path_min, ca.metric_key("sidewalk-throughway-zone-sf", "alleyOrAdaMin")?, "sidewalk-throughway-zone-sf alleyOrAdaMin");
    // ground-datum.js footwayLiftM / openGroundLiftM from the construction.
    eq("footwayLift", o.footway_lift, o.road_lift - o.gutter_depth + o.curb_face_height + o.curb_top_fall, "footwayLiftM(roadLift, gutterDepth, curbFaceHeight, curbTopFall)");
    eq("groundSink", o.ground_sink, -o.open_ground_lift, "openGroundSinkM = -openGroundLiftM");
    Ok(out)
}
