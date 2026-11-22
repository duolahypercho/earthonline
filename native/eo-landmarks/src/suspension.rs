//! The parts every suspension bridge is made of: a port of
//! src/world/landmarks/suspension-bridge.js (`bridgeFrame`, `frameInward`,
//! `stations`, `cableHeightAt`, `sideSpanSag`, `mainCables`,
//! `suspenderPairs`, `stiffeningTruss`, `roadDeck`). Every size comes in
//! through the caller's spec; markings and barrier sections are read from the
//! California standards table (`caMetric`) here, as the JS reads them.

use crate::geometry::{Faces, MeshArrays, NO_BOTTOM, Rgb, V3};
use std::sync::OnceLock;

/// The bridges' fidelity features (both bridges; each on by default and
/// turned off by its eo-app switch, which restores the earlier output;
/// `--no-bridges2` turns every one off).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BridgeFeatures {
    /// `--no-bridge-scale`: the structures' heights follow the drawn
    /// terrain's height scale (`eo_data::groundscale::height_scale`), as
    /// their spans follow the ground plan, so each bridge keeps the surveyed
    /// proportions (span to tower height, deck clearance to truss depth) on
    /// the 1.25 x plan instead of standing 1 / 1.25 as tall as its own span
    /// and its shores allow.
    pub scale: bool,
    /// `--no-bridge-paint`: the cited paint colours (International Orange;
    /// the west span's aluminium grey) instead of the export's tones.
    pub paint: bool,
    /// `--no-bridge-detail`: the towers' portal bracing, setbacks and caps,
    /// the deck's painted underside and the heavier west-span members.
    pub detail: bool,
    /// `--no-bridge-wires`: the cables and suspenders drawn after the
    /// atmosphere at every distance, anti-aliased at a screen-space floor
    /// width (the far item then carries no cable beams).
    pub wires: bool,
    /// `--no-bridge-night`: the Golden Gate towers' floodlights and the west
    /// span's suspender LEDs.
    pub night: bool,
    /// `--no-bridge-air`: the wire members (drawn after the atmosphere) take
    /// the aerial perspective the volume gives every other surface at their
    /// distance, and their sun shadow is looked up clear of their own
    /// shadow-map footprint (a rope sloping up toward the sun self-shadowed
    /// in the far cascades: one cable plane of a span drew dark grey).
    pub air: bool,
    /// `--no-bridge-finish`: the paints as dielectric films (metalness 0),
    /// the bridges' own plate seams and rivet lines in place of the
    /// landmark steel's low-frequency mottle, the west span's truss in the
    /// towers' paint, the baked occlusion eased where the frame's own sun
    /// shadow and ambient occlusion now darken the members.
    pub finish: bool,
    /// `--no-bridge-members`: the members added in the second pass (cable
    /// saddles and their housings, cable bands, the Fort Point arch's open
    /// web, the pylons' pilasters, the south pier's fender, the lamp
    /// standards, the west span's deck slabs, floor beams, sway frames and
    /// laced tower bracing, the Golden Gate far item's deck as one welded strip).
    pub members: bool,
    /// `--no-bridge-floods`: the Golden Gate towers lit by their 24 floods
    /// as placed sources (lit outer and along faces, dark inner faces, the
    /// struts' shadows), the suspender LEDs' sequence in bands with dark
    /// gaps; else the flux-conserving mean and the continuous sequence.
    /// With it the towers' 12 lights below the roadway light the legs under
    /// the deck.
    pub floods: bool,
    /// `--no-bridge-curtain`: a suspender rope's coverage filter widens to
    /// the spacing between its neighbours where they close to under a few
    /// pixels on screen, so a receding curtain sums to its true mean cover
    /// (a partition of unity) instead of beating in moire bands.
    pub curtain: bool,
    /// `--no-bridge-wire-sky`: the wires' aerial perspective (`air`) takes its
    /// sky colours (the dome's and the atmosphere tables' toward the member)
    /// at the vertices of each member's segment and interpolates them along
    /// it, its distance terms per pixel (eo-render landmarks_wire.wgsl
    /// `lm_aerial_sky`, `lm_aerial_over`). Read per pixel beside look_light,
    /// they were most of the overlay's GPU time (bench golden_gate: about
    /// 1.8 ms against 0.3); the frame differs by a level of the display on
    /// some wire pixels. Off, the colours are read per pixel, as before.
    pub wire_sky: bool,
    /// `--no-bridge-shade2`: the Golden Gate's painted members in the one
    /// paint (no `DARK` bake on the struts, fins, bracing and floor beams),
    /// and the baked occlusion replaced by the sky the deck and the opposite
    /// tower leg take from a face (eo-render landmarks_surface.wgsl
    /// `lm_bridge_sky_vis`), on the indirect light only.
    pub shade2: bool,
    /// `--no-bridge-dielectric`: the paint's diffuse light counted as a
    /// dielectric film's under the physical daylight: the sea's reflected
    /// light replaces the environment's ground share (counted once), the
    /// sky's share at the environment's own intensity.
    pub dielectric: bool,
    /// `--no-bridge-haunch`: the Golden Gate's portal struts as casings
    /// nearly flush with the legs, with stepped haunch brackets in the
    /// openings' corners and a projecting top lip.
    pub haunch: bool,
    /// `--no-bridge-bracing`: the towers' bracing under the roadway: strut 5
    /// under the deck and two laced X panels down to strut 7.
    pub bracing: bool,
    /// `--no-bridge-pylons`: the concrete pylons S1, S2, N1 and N2 as two
    /// massive fluted shafts each, the side spans' tie-downs near deck level,
    /// the backstays and the anchorage housings.
    pub pylons: bool,
    /// `--no-bridge-arch2`: the Fort Point arch's four laced ribs, spandrel
    /// bents and laterals.
    pub arch2: bool,
    /// `--no-bridge-fender2`: the south pier's fender at its published size
    /// and the pier's rounded, pointed ends and stepped top.
    pub fender2: bool,
    /// `--no-bridge-crest`: the roadway's crest at midspan at its surveyed
    /// height (the cable's low point raised with it).
    pub crest: bool,
    /// `--no-bridge-collar`: the stiffening truss closed at the tower legs
    /// (end verticals, rocker-link housings) and the balconies' steel fascia.
    pub collar: bool,
    /// `--no-paint-brdf2` (the fifth pass, `--no-bridges5` / `--no-gg3-bridge`
    /// turn every one off): the Golden Gate's paint in the chromaticity of the
    /// District's named match for its formula (golden_gate.rs
    /// `PMS_173_PRINT_SRGB`, `international_orange2`), a dielectric film of the acrylic
    /// binder's Fresnel reflectance.
    pub paint2: bool,
    /// `--no-paint-weathering`: the paint's chalking, touch-up patches of
    /// fresher paint, rust bloom at rivet rows and dirt on ledges and under
    /// set-backs (landmarks_surface.wgsl `lm_weather`).
    pub weathering: bool,
    /// `--no-concrete2`: the Golden Gate's concrete (pylons, housings, piers,
    /// fender) in a weathered portland-cement albedo (golden_gate.rs
    /// `concrete_colours`), each pour lift's tone, lime leached at the lift
    /// joints and run-off staining, all zero-mean about that albedo
    /// (landmarks_surface.wgsl `lm_concrete_age`).
    pub concrete2: bool,
    /// `--no-leg-cells`: the tower legs' cellular section (stepped corners and
    /// recessed panels between cell-wide pilasters on every face) and the
    /// portal casings' own shadow and sky occlusion from the legs, the lip
    /// and the ribs past the shadow maps' reach.
    pub leg_cells: bool,
    /// `--no-cable-detail`: the cable bands' collars, the four rope legs of
    /// each suspender group looped over the band and straddling the cable,
    /// the spacing clamps round the four legs, and the lamp standards (and
    /// the night's deck luminaires in their heads) at the published 150 ft
    /// pitch, none by the tower legs, in round 2's standard form.
    pub cable_detail: bool,
    /// `--no-gg-matte` (the sixth pass, the aging: `--no-gg-aging` turns every
    /// one off, as do the earlier passes' roll-ups): the Golden Gate's coat
    /// as a chalked flat finish (golden_gate/aging.rs `PAINT_AGED_ROUGHNESS`),
    /// the far item's too.
    pub matte: bool,
    /// `--no-gg-fade`: the coat chalked toward a pale pink-grey, less where
    /// it is sheltered, more on fresher-or-older sections by the age field
    /// (landmarks_surface.wgsl `lm_gg_age_paint`).
    pub fade: bool,
    /// `--no-gg-grime`: the coat's age and tone wandering over sections a few
    /// metres across (repainted stretches), run-off streaks down the vertical
    /// faces and salt and soot at the towers' feet.
    pub grime: bool,
    /// `--no-gg-underside`: the steel under the roadway shaded by the truss's
    /// own lattice (sun and sky) and dirtier than the faces in the open.
    pub underside: bool,
    /// `--no-gg-concrete-age`: the concrete in a warm buff at the cited
    /// albedo, run-off streaks under the copings, wall-scale staining,
    /// efflorescence and dark feet (landmarks_surface.wgsl `lm_gg_age_concrete`).
    pub concrete_age: bool,
}

impl BridgeFeatures {
    pub const ALL: Self = Self {
        scale: true,
        paint: true,
        detail: true,
        wires: true,
        night: true,
        air: true,
        finish: true,
        members: true,
        floods: true,
        curtain: true,
        wire_sky: true,
        shade2: true,
        dielectric: true,
        haunch: true,
        bracing: true,
        pylons: true,
        arch2: true,
        fender2: true,
        crest: true,
        collar: true,
        paint2: true,
        weathering: true,
        concrete2: true,
        leg_cells: true,
        cable_detail: true,
        matte: true,
        fade: true,
        grime: true,
        underside: true,
        concrete_age: true,
    };
    pub const OFF: Self = Self {
        scale: false,
        paint: false,
        detail: false,
        wires: false,
        night: false,
        air: false,
        finish: false,
        members: false,
        floods: false,
        curtain: false,
        wire_sky: false,
        shade2: false,
        dielectric: false,
        haunch: false,
        bracing: false,
        pylons: false,
        arch2: false,
        fender2: false,
        crest: false,
        collar: false,
        paint2: false,
        weathering: false,
        concrete2: false,
        leg_cells: false,
        cable_detail: false,
        matte: false,
        fade: false,
        grime: false,
        underside: false,
        concrete_age: false,
    };

    /// The features a command line leaves on.
    pub fn from_args<S: AsRef<str>>(args: &[S]) -> Self {
        let has = |f: &str| args.iter().any(|a| a.as_ref() == f);
        if has("--no-bridges2") {
            return Self::OFF;
        }
        let first = has("--no-bridges3");
        let third = first || has("--no-bridges4");
        let has3 = |f: &str| first || has(f);
        let has4 = |f: &str| third || has(f);
        // the fifth pass (round 3's bridge finish): its roll-ups, every track's roll-up, and the earlier passes'.
        let fourth = third || has("--no-bridges5") || has("--no-gg3-bridge") || has("--no-gg3");
        let has5 = |f: &str| fourth || has(f);
        // the sixth pass (the aging): its roll-up and every earlier one.
        let fifth = fourth || has("--no-gg-aging");
        let has6 = |f: &str| fifth || has(f);
        let keep = |f: &str| !args.iter().any(|a| a.as_ref() == f);
        Self {
            scale: keep("--no-bridge-scale"),
            paint: keep("--no-bridge-paint"),
            detail: keep("--no-bridge-detail"),
            wires: keep("--no-bridge-wires"),
            night: keep("--no-bridge-night"),
            air: !has3("--no-bridge-air"),
            finish: !has3("--no-bridge-finish"),
            members: !has3("--no-bridge-members"),
            floods: !has3("--no-bridge-floods"),
            curtain: !has3("--no-bridge-curtain"),
            wire_sky: keep("--no-bridge-wire-sky"),
            shade2: !has4("--no-bridge-shade2"),
            dielectric: !has4("--no-bridge-dielectric"),
            haunch: !has4("--no-bridge-haunch"),
            bracing: !has4("--no-bridge-bracing"),
            pylons: !has4("--no-bridge-pylons"),
            arch2: !has4("--no-bridge-arch2"),
            fender2: !has4("--no-bridge-fender2"),
            crest: !has4("--no-bridge-crest"),
            collar: !has4("--no-bridge-collar"),
            paint2: !has5("--no-paint-brdf2"),
            weathering: !has5("--no-paint-weathering"),
            concrete2: !has5("--no-concrete2"),
            leg_cells: !has5("--no-leg-cells"),
            cable_detail: !has5("--no-cable-detail"),
            matte: !has6("--no-gg-matte"),
            fade: !has6("--no-gg-fade"),
            grime: !has6("--no-gg-grime"),
            underside: !has6("--no-gg-underside"),
            concrete_age: !has6("--no-gg-concrete-age"),
        }
    }

    /// The second pass's features off (`--no-bridges3`): the first pass's output.
    pub const FIRST: Self = Self { air: false, finish: false, members: false, floods: false, curtain: false, ..Self::THIRD };

    /// The fourth pass's features off (`--no-bridges4`): the third pass's output.
    pub const THIRD: Self = Self { shade2: false, dielectric: false, haunch: false, bracing: false, pylons: false, arch2: false, fender2: false, crest: false, collar: false, ..Self::FOURTH };

    /// The fifth pass's features off (`--no-bridges5`, `--no-gg3-bridge`, `--no-gg3`): the fourth pass's (round 2's) output.
    pub const FOURTH: Self = Self { paint2: false, weathering: false, concrete2: false, leg_cells: false, cable_detail: false, ..Self::FIFTH };

    /// The sixth pass's features off (`--no-gg-aging`): the fifth pass's (round 3's bridge finish) output.
    pub const FIFTH: Self = Self { matte: false, fade: false, grime: false, underside: false, concrete_age: false, ..Self::ALL };
}

static FEATURES: OnceLock<BridgeFeatures> = OnceLock::new();

/// The process's bridge features: pinned by `pin_features`, else read once from its arguments.
pub fn features() -> BridgeFeatures {
    *FEATURES.get_or_init(|| BridgeFeatures::from_args(&std::env::args().collect::<Vec<_>>()))
}

/// Pin the features before anything reads them (a test, a tool); false when
/// the process already reads others.
pub fn pin_features(f: BridgeFeatures) -> bool {
    *FEATURES.get_or_init(|| f) == f
}

/// The kill switches `features` reads (eo-app accepts them).
pub const SWITCHES: &[&str] = &[
    "--no-bridges2",
    "--no-bridges3",
    "--no-bridges4",
    "--no-bridge-scale",
    "--no-bridge-paint",
    "--no-bridge-detail",
    "--no-bridge-wires",
    "--no-bridge-night",
    "--no-bridge-air",
    "--no-bridge-finish",
    "--no-bridge-members",
    "--no-bridge-floods",
    "--no-bridge-curtain",
    "--no-bridge-wire-sky",
    "--no-bridge-shade2",
    "--no-bridge-dielectric",
    "--no-bridge-haunch",
    "--no-bridge-bracing",
    "--no-bridge-pylons",
    "--no-bridge-arch2",
    "--no-bridge-fender2",
    "--no-bridge-crest",
    "--no-bridge-collar",
];

/// The bridge-finish track's switch names (the fifth pass: the roll-up
/// `--no-gg3-bridge`, also `--no-bridges5`, first), all read by
/// `BridgeFeatures::from_args`; eo-app accepts them from this list (each
/// name in one list only: eo-app's `every_listed_switch_parses`). `--no-gg3`,
/// every track's roll-up, is in eo-render pace.rs's list and read here too.
pub const GG3_SWITCHES: &[&str] = &["--no-gg3-bridge", "--no-bridges5", "--no-paint-brdf2", "--no-paint-weathering", "--no-concrete2", "--no-leg-cells", "--no-cable-detail"];

/// The aging pass's switch names (the sixth pass: the roll-up `--no-gg-aging`
/// first), read by `BridgeFeatures::from_args`; eo-app accepts them from this list.
pub const AGING_SWITCHES: &[&str] = &["--no-gg-aging", "--no-gg-matte", "--no-gg-fade", "--no-gg-grime", "--no-gg-underside", "--no-gg-concrete-age"];

/// A frame along a bridge: chainage c along `axis` from `origin`, offset s to
/// its right; `offset`, when given, adds a lateral offset per chainage (the
/// Golden Gate's deck follows its carriageways' curve at each end).
pub struct Frame<'a> {
    pub origin: [f64; 2],
    pub axis: [f64; 2],
    pub across: [f64; 2],
    pub yaw: f64,
    pub offset: Option<&'a dyn Fn(f64) -> f64>,
}

impl<'a> Frame<'a> {
    pub fn new(origin: [f64; 2], axis: [f64; 2]) -> Self {
        Self { origin, axis, across: [-axis[1], axis[0]], yaw: axis[1].atan2(axis[0]), offset: None }
    }
    pub fn point(&self, c: f64, s: f64) -> [f64; 2] {
        let s = match self.offset {
            Some(f) => s + f(c),
            None => s,
        };
        [self.origin[0] + self.axis[0] * c + self.across[0] * s, self.origin[1] + self.axis[1] * c + self.across[1] * s]
    }
    #[allow(non_snake_case)]
    pub fn P(&self, c: f64, s: f64, y: f64) -> V3 {
        let [x, z] = self.point(c, s);
        [x, y, z]
    }
    pub fn chainage(&self, x: f64, z: f64) -> f64 {
        (x - self.origin[0]) * self.axis[0] + (z - self.origin[1]) * self.axis[1]
    }
    /// `frameInward`: across the axis toward the centreline, from wherever a
    /// face stands more than `min` off it.
    pub fn inward(&self) -> impl Fn(f64, f64, f64) -> Option<[f64; 2]> + use<> {
        let [ax, az] = self.across;
        let [ox, oz] = self.origin;
        move |x: f64, z: f64, min: f64| {
            let s = (x - ox) * ax + (z - oz) * az;
            if s.abs() < min {
                return None;
            }
            Some(if s > 0.0 { [-ax, -az] } else { [ax, az] })
        }
    }
}

/// One stage of a tower leg with its corners notched (`BridgeFeatures::detail`):
/// a riveted cellular leg's re-entrant corners, which draw a shadow line down
/// each corner of the leg and its setbacks. Two boxes, the inner one's top
/// 3 cm under the outer's (no coplanar tops). INFERRED: the notch, 0.18 of
/// the leg's smaller half size.
#[allow(clippy::too_many_arguments)]
pub fn notched_box(m: &mut MeshArrays, x: f64, z: f64, y0: f64, y1: f64, half_along: f64, half_across: f64, yaw: f64, colour: Rgb, faces: Faces) {
    let n = 0.18 * half_along.min(half_across);
    m.box_(x, z, y0, y1, half_along, half_across - n, yaw, colour, faces);
    m.box_(x, z, y0, y1 - 0.03, half_along - n, half_across, yaw, colour, faces);
}

pub fn stations(c0: f64, c1: f64, step: f64) -> Vec<f64> {
    let n = (((c1 - c0) / step).round() as i64).max(1);
    (0..=n).map(|i| c0 + ((c1 - c0) * i as f64) / n as f64).collect()
}

/// A main-cable run: a parabola under the chord (c0, y0)-(c1, y1), `sag` below it at mid-run.
#[derive(Clone, Copy, Debug)]
pub struct Run {
    pub c0: f64,
    pub c1: f64,
    pub y0: f64,
    pub y1: f64,
    pub sag: f64,
}

pub fn cable_height_at(runs: &[Run], c: f64) -> Option<f64> {
    for r in runs {
        if c < r.c0 - 1e-6 || c > r.c1 + 1e-6 {
            continue;
        }
        let t = ((c - r.c0) / (r.c1 - r.c0)).max(0.0).min(1.0);
        return Some(r.y0 + (r.y1 - r.y0) * t - 4.0 * r.sag * t * (1.0 - t));
    }
    None
}

pub fn side_span_sag(main_sag: f64, main_span: f64, side_span: f64) -> f64 {
    main_sag * (side_span / main_span).powi(2)
}

pub struct CableSpec {
    pub half_plane: f64,
    pub diameter: f64,
    pub colour: Rgb,
    /// The far item's beams at twice the diameter (the JS); false when the
    /// wire pass draws the cables at every distance (`BridgeFeatures::wires`).
    pub far_beams: bool,
}

/// `mainCables` (step 12, farStep 60, farScale 2).
pub fn main_cables(near: &mut MeshArrays, far: &mut MeshArrays, frame: &Frame, runs: &[Run], o: &CableSpec) {
    let (step, far_step, far_scale) = (12.0, 60.0, 2.0);
    for s in [-o.half_plane, o.half_plane] {
        for r in runs {
            let one = [*r];
            let ns = stations(r.c0, r.c1, step);
            for i in 0..ns.len() - 1 {
                let a = frame.P(ns[i], s, cable_height_at(&one, ns[i]).unwrap_or(f64::NAN));
                let b = frame.P(ns[i + 1], s, cable_height_at(&one, ns[i + 1]).unwrap_or(f64::NAN));
                if near.wire {
                    near.wire_segment(a, b, o.diameter, o.colour);
                } else {
                    near.beam(a, b, o.diameter, o.diameter, o.colour);
                }
            }
            if !o.far_beams {
                continue;
            }
            let fs = stations(r.c0, r.c1, far_step);
            for i in 0..fs.len() - 1 {
                far.beam(
                    frame.P(fs[i], s, cable_height_at(&one, fs[i]).unwrap_or(f64::NAN)),
                    frame.P(fs[i + 1], s, cable_height_at(&one, fs[i + 1]).unwrap_or(f64::NAN)),
                    o.diameter * far_scale,
                    o.diameter * far_scale,
                    o.colour,
                );
            }
        }
    }
}

pub struct SuspenderSpec {
    pub half_plane: f64,
    pub spacing: f64,
    pub diameter: f64,
    pub pair_gap: f64,
    pub colour: Rgb,
}

/// `suspenderPairs` (endClearM 6): returns the stations per plane.
pub fn suspender_pairs(near: &mut MeshArrays, frame: &Frame, runs: &[Run], attach_y: &dyn Fn(f64) -> f64, o: &SuspenderSpec) -> usize {
    let end_clear = 6.0;
    let mut count = 0;
    for r in runs {
        let n = ((r.c1 - r.c0) / o.spacing).floor() as i64;
        let offset = ((r.c1 - r.c0) - n as f64 * o.spacing) / 2.0;
        for k in 0..=n {
            let c = r.c0 + offset + k as f64 * o.spacing;
            if c - r.c0 < end_clear || r.c1 - c < end_clear {
                continue;
            }
            let Some(top) = cable_height_at(&[*r], c) else { continue };
            let bottom = attach_y(c);
            if top - bottom < 0.5 {
                continue;
            }
            for s in [-o.half_plane, o.half_plane] {
                for d in [-o.pair_gap / 2.0, o.pair_gap / 2.0] {
                    if near.wire {
                        near.wire_segment(frame.P(c + d, s, bottom), frame.P(c + d, s, top), o.diameter, o.colour);
                    } else {
                        near.beam(frame.P(c + d, s, bottom), frame.P(c + d, s, top), o.diameter, o.diameter, o.colour);
                    }
                }
            }
            count += 1;
        }
    }
    count
}

/// The cable bands and the ropes' deck sockets (`BridgeFeatures::members`)
/// at `suspender_pairs`' own stations: a band clamped round the main cable
/// where each pair loops over it (the Golden Gate's every 50 ft:
/// goldengate.org, "Design & Construction Stats"), `band_len` long and
/// `band_d` across, and a socket `socket_len` long and `socket_d` across at
/// each rope's foot.
pub struct BandSpec {
    pub half_plane: f64,
    pub spacing: f64,
    pub pair_gap: f64,
    pub cable_d: f64,
    pub band_len: f64,
    pub band_d: f64,
    pub socket_len: f64,
    pub socket_d: f64,
    pub colour: Rgb,
}

pub fn cable_bands(wires: &mut MeshArrays, frame: &Frame, runs: &[Run], attach_y: &dyn Fn(f64) -> f64, o: &BandSpec) -> usize {
    let end_clear = 6.0;
    let mut count = 0;
    for r in runs {
        let n = ((r.c1 - r.c0) / o.spacing).floor() as i64;
        let offset = ((r.c1 - r.c0) - n as f64 * o.spacing) / 2.0;
        for k in 0..=n {
            let c = r.c0 + offset + k as f64 * o.spacing;
            if c - r.c0 < end_clear || r.c1 - c < end_clear {
                continue;
            }
            let one = [*r];
            let (Some(top), Some(a), Some(b)) = (cable_height_at(&one, c), cable_height_at(&one, c - o.band_len / 2.0), cable_height_at(&one, c + o.band_len / 2.0)) else { continue };
            let bottom = attach_y(c);
            if top - bottom < 0.5 {
                continue;
            }
            for s in [-o.half_plane, o.half_plane] {
                // the band's axis on the cable's (the band sits round it, so its axis is the cable's centre).
                wires.wire_segment(frame.P(c - o.band_len / 2.0, s, a), frame.P(c + o.band_len / 2.0, s, b), o.band_d, o.colour);
                for d in [-o.pair_gap / 2.0, o.pair_gap / 2.0] {
                    wires.wire_segment(frame.P(c + d, s, bottom), frame.P(c + d, s, bottom + o.socket_len), o.socket_d, o.colour);
                }
            }
            count += 1;
        }
    }
    let _ = o.cable_d;
    count
}

/// A saddle housing over a tower leg's top (`BridgeFeatures::members`): a
/// half-cylinder cover along the span, `half_along` long either side of the
/// leg's centre and `radius` round, standing on a plinth `plinth` high over
/// the leg's top; the cable passes under it over the saddle.
#[allow(clippy::too_many_arguments)]
pub fn saddle_housing(m: &mut MeshArrays, frame: &Frame, c: f64, s: f64, top: f64, half_along: f64, radius: f64, plinth: f64, colour: Rgb) {
    let segs = 8;
    let y0 = top + plinth;
    let arc = |k: usize| {
        let t = std::f64::consts::PI * k as f64 / segs as f64;
        (radius * t.cos(), radius * t.sin())
    };
    let [x, z] = frame.point(c, s);
    m.box_(x, z, top - 0.05, y0, half_along, radius, frame.yaw, colour, NO_BOTTOM);
    for k in 0..segs {
        let ((a0, b0), (a1, b1)) = (arc(k), arc(k + 1));
        let p = |cc: f64, ds: f64, dy: f64| frame.P(cc, s + ds, y0 + dy);
        m.quad(p(c - half_along, a0, b0), p(c + half_along, a0, b0), p(c + half_along, a1, b1), p(c - half_along, a1, b1), colour);
        // the end plates (fans from the cylinder's axis).
        m.tri(p(c + half_along, 0.0, 0.0), p(c + half_along, a1, b1), p(c + half_along, a0, b0), colour);
        m.tri(p(c - half_along, 0.0, 0.0), p(c - half_along, a0, b0), p(c - half_along, a1, b1), colour);
    }
}

pub struct TrussSpec<'a> {
    pub deck_top: &'a dyn Fn(f64) -> f64,
    pub c_start: f64,
    pub c_end: f64,
    pub half_plane: f64,
    pub depth: f64,
    pub panel: f64,
    pub colour: Rgb,
    pub dark: Rgb,
    pub gap: &'a dyn Fn(f64, f64) -> bool,
    pub far_half_width: f64,
    pub far_above: f64,
    /// The members' section sizes times this (the structure scale; 1: the JS's).
    pub member: f64,
}

/// The far deck strip's station pitch (`welded_strip`): the camber's sag
/// between stations (a 1,280 m span's 3 m crown over 20 m) is under a
/// centimetre, under a pixel past the near distance.
pub const FAR_STRIP_STEP_M: f64 = 20.0;

/// `stiffeningTruss` (farStep 40).
pub fn stiffening_truss(near: &mut MeshArrays, far: &mut MeshArrays, frame: &Frame, o: &TrussSpec) {
    let h = o.half_plane;
    let depth = o.depth;
    let far_step = 40.0;
    let k = o.member;
    let top = 0.5 * k;
    let cs = stations(o.c_start, o.c_end, o.panel);
    for i in 0..cs.len() - 1 {
        let (a, b) = (cs[i], cs[i + 1]);
        let (ta, tb) = ((o.deck_top)(a), (o.deck_top)(b));
        let cut = (o.gap)(a, b);
        if !cut {
            for s in [-h, h] {
                near.beam(frame.P(a, s, ta - top), frame.P(b, s, tb - top), 0.9 * k, 1.0 * k, o.colour);
                near.beam(frame.P(a, s, ta - depth), frame.P(b, s, tb - depth), 0.9 * k, 1.0 * k, o.colour);
                near.beam(frame.P(b, s, tb - depth), frame.P(b, s, tb - top), 0.5 * k, 0.6 * k, o.colour);
                let up = i % 2 == 0;
                near.beam(
                    frame.P(a, s, if up { ta - depth } else { ta - top }),
                    frame.P(b, s, if up { tb - top } else { tb - depth }),
                    0.45 * k,
                    0.5 * k,
                    o.colour,
                );
            }
        }
        near.beam(frame.P(b, -h, tb - 1.1 * k), frame.P(b, h, tb - 1.1 * k), 0.7 * k, 1.0 * k, o.dark);
        if !cut {
            near.beam(frame.P(a, -h, ta - depth), frame.P(b, h, tb - depth), 0.35 * k, 0.35 * k, o.dark);
            near.beam(frame.P(a, h, ta - depth), frame.P(b, -h, tb - depth), 0.35 * k, 0.35 * k, o.dark);
        }
    }
    if features().members {
        welded_strip(far, frame, o.deck_top, o.c_start, o.c_end, FAR_STRIP_STEP_M, o.far_half_width, depth, o.far_above, o.colour);
    } else {
        let fs = stations(o.c_start, o.c_end, far_step);
        for i in 0..fs.len() - 1 {
            let (a, b) = (fs[i], fs[i + 1]);
            let (ta, tb) = ((o.deck_top)(a), (o.deck_top)(b));
            let [mx, mz] = frame.point((a + b) / 2.0, 0.0);
            far.box_(mx, mz, (ta + tb) / 2.0 - depth, (ta + tb) / 2.0 + o.far_above, (b - a) / 2.0, o.far_half_width, frame.yaw, o.colour, crate::geometry::ALL);
        }
    }
    for c in [o.c_start, o.c_end] {
        let t = (o.deck_top)(c);
        let [x, z] = frame.point(c, 0.0);
        near.box_(x, z, t - depth, t, 0.6, o.far_half_width, frame.yaw, o.dark, crate::geometry::ALL);
    }
}

/// Colours of a road deck.
#[derive(Clone, Copy)]
pub struct DeckColours {
    pub asphalt: Rgb,
    pub sidewalk: Rgb,
    pub soffit: Rgb,
    pub barrier: Rgb,
    pub white: Rgb,
    pub yellow: Rgb,
    pub rail: Rgb,
}

/// The California standards a deck reads (`caMetric` in suspension-bridge.js).
#[derive(Clone, Copy, Debug)]
pub struct DeckStandards {
    /// 'temporary-railing-type-k-height'
    pub k_height: f64,
    /// 'temporary-railing-type-k-width'
    pub k_width: f64,
    /// 'line-width-state-highway'
    pub line_w: f64,
    /// 'broken-line-pattern-ge45mph' line and gap
    pub broken_line: f64,
    pub broken_gap: f64,
}

impl DeckStandards {
    pub fn load(ca: &eo_data::CaStandards) -> anyhow::Result<Self> {
        Ok(Self {
            k_height: ca.metric("temporary-railing-type-k-height")?,
            k_width: ca.metric("temporary-railing-type-k-width")?,
            line_w: ca.metric("line-width-state-highway")?,
            broken_line: ca.metric_key("broken-line-pattern-ge45mph", "line")?,
            broken_gap: ca.metric_key("broken-line-pattern-ge45mph", "gap")?,
        })
    }
}

pub struct DeckSpec<'a> {
    pub y: &'a dyn Fn(f64) -> f64,
    pub c_start: f64,
    pub c_end: f64,
    pub half_width: f64,
    pub roadway_m: f64,
    pub lanes: u32,
    pub median: bool,
    pub sidewalk_m: f64,
    pub curb_m: f64,
    pub railings: bool,
    pub railing_m: f64,
    pub inner_barrier_m: f64,
    pub panel: f64,
    pub gap: &'a dyn Fn(f64, f64) -> bool,
    pub colours: DeckColours,
    pub markings: bool,
    /// The side (+1 or -1 across) whose sidewalk railing another builder
    /// draws (eo-landmarks access.rs: the Golden Gate's east sidewalk), else None.
    pub outer_railing_elsewhere: Option<f64>,
}

/// `roadDeck`: the roadway, sidewalks behind curbs, the plate's underside, a
/// median barrier, edge lines, broken lane lines and railings.
pub fn road_deck(deck: &mut MeshArrays, rail: &mut MeshArrays, frame: &Frame, ca: &DeckStandards, o: &DeckSpec) {
    let y = o.y;
    let curb = o.roadway_m / 2.0;
    let line_w = ca.line_w;
    let median_half = if o.median { ca.k_width / 2.0 + 0.3 } else { 0.0 };
    let cs = stations(o.c_start, o.c_end, o.panel);
    let colours = o.colours;
    let stripe = |deck: &mut MeshArrays, c0: f64, c1: f64, s: f64, colour: Rgb| {
        let (y0, y1) = (y(c0) + 0.02, y(c1) + 0.02);
        deck.quad(frame.P(c0, s - line_w / 2.0, y0), frame.P(c0, s + line_w / 2.0, y0), frame.P(c1, s + line_w / 2.0, y1), frame.P(c1, s - line_w / 2.0, y1), colour);
    };
    for i in 0..cs.len() - 1 {
        let (a, b) = (cs[i], cs[i + 1]);
        let (ta, tb) = (y(a), y(b));
        deck.quad(frame.P(a, -curb, ta), frame.P(a, curb, ta), frame.P(b, curb, tb), frame.P(b, -curb, tb), colours.asphalt);
        let cut = (o.gap)(a, b);
        if o.sidewalk_m > 0.0 && !cut {
            for sgn in [-1.0, 1.0] {
                let inner = sgn * curb;
                let outer = sgn * o.half_width;
                let (p, q) = if sgn < 0.0 { (outer, inner) } else { (inner, outer) };
                deck.quad(frame.P(a, p, ta + o.curb_m), frame.P(a, q, ta + o.curb_m), frame.P(b, q, tb + o.curb_m), frame.P(b, p, tb + o.curb_m), colours.sidewalk);
                let (c0, c1, t0, t1) = if sgn > 0.0 { (b, a, tb, ta) } else { (a, b, ta, tb) };
                deck.quad(frame.P(c0, inner, t0), frame.P(c1, inner, t1), frame.P(c1, inner, t1 + o.curb_m), frame.P(c0, inner, t0 + o.curb_m), colours.sidewalk);
            }
        }
        deck.quad(frame.P(b, -o.half_width, tb - 0.3), frame.P(b, o.half_width, tb - 0.3), frame.P(a, o.half_width, ta - 0.3), frame.P(a, -o.half_width, ta - 0.3), colours.soffit);
        if o.median {
            let [mx, mz] = frame.point((a + b) / 2.0, 0.0);
            deck.box_(mx, mz, (ta + tb) / 2.0, (ta + tb) / 2.0 + ca.k_height, (b - a) / 2.0, ca.k_width / 2.0, frame.yaw, colours.barrier, NO_BOTTOM);
        }
        if o.markings {
            if o.median {
                for sgn in [-1.0, 1.0] {
                    stripe(deck, a, b, sgn * (median_half + 0.15), colours.yellow);
                    stripe(deck, a, b, sgn * (curb - 0.3), colours.white);
                }
            } else {
                stripe(deck, a, b, -(curb - 0.3), colours.yellow);
                stripe(deck, a, b, curb - 0.3, colours.white);
            }
        }
    }
    if o.markings {
        let mut lines = Vec::new();
        if o.median {
            let per = o.lanes as f64 / 2.0;
            let lane_w = (curb - median_half) / per;
            for sgn in [-1.0, 1.0] {
                let mut k = 1.0;
                while k < per {
                    lines.push(sgn * (median_half + k * lane_w));
                    k += 1.0;
                }
            }
        } else {
            let lane_w = o.roadway_m / o.lanes as f64;
            for k in 1..o.lanes {
                lines.push(-curb + k as f64 * lane_w);
            }
        }
        let cycle = ca.broken_line + ca.broken_gap;
        let mut c = o.c_start + ca.broken_gap / 2.0;
        while c + ca.broken_line < o.c_end {
            for &s in &lines {
                stripe(deck, c, c + ca.broken_line, s, colours.white);
            }
            c += cycle;
        }
    }
    if !o.railings {
        return;
    }
    let rs = stations(o.c_start, o.c_end, o.panel / 2.0);
    for i in 0..rs.len() - 1 {
        let (a, b) = (rs[i], rs[i + 1]);
        let (ta, tb) = (y(a) + o.curb_m, y(b) + o.curb_m);
        let cut = (o.gap)(a, b);
        for sgn in [-1.0, 1.0] {
            let si = sgn * (curb + 0.15);
            rail.beam(frame.P(a, si, ta - o.curb_m + o.inner_barrier_m), frame.P(b, si, tb - o.curb_m + o.inner_barrier_m), 0.25, 0.25, colours.rail);
            if cut {
                continue;
            }
            rail.beam(frame.P(b, si, tb - o.curb_m), frame.P(b, si, tb - o.curb_m + o.inner_barrier_m), 0.2, 0.2, colours.rail);
            if o.sidewalk_m <= 0.0 || o.outer_railing_elsewhere == Some(sgn) {
                continue;
            }
            let s = sgn * (o.half_width - 0.15);
            rail.beam(frame.P(a, s, ta + o.railing_m), frame.P(b, s, tb + o.railing_m), 0.12, 0.12, colours.rail);
            rail.beam(frame.P(a, s, ta + o.railing_m * 0.5), frame.P(b, s, tb + o.railing_m * 0.5), 0.08, 0.08, colours.rail);
            rail.beam(frame.P(b, s, tb), frame.P(b, s, tb + o.railing_m), 0.14, 0.14, colours.rail);
        }
    }
}

#[allow(dead_code)]
pub(crate) fn faces_all() -> Faces {
    crate::geometry::ALL
}

/// The coat's roughness (`finish`): the Golden Gate's coat is an acrylic
/// topcoat over an inorganic zinc primer (goldengate.org, "Painting the
/// Bridge"), a low-sheen film; the west span's aluminium finish coat
/// (bay_bridge.rs `STEEL_SRGB`) weathers to a matte grey. INFERRED: 0.6 (the registry's 0.45-0.55 dropped under
/// 0.5 with the landmark steel's 0.22 spread, where the sky's sharp
/// reflection mixes in: the clouds drew as pink-white blotches on the struts).
pub const PAINT_ROUGHNESS: f64 = 0.6;

/// The sea's albedo under the bridges (`finish`): 0.06 for a sun over 40 deg
/// in a clear sky (Payne 1972, "Albedo of the Sea Surface", J. Atmos. Sci.
/// 29, 959-970; it rises toward a low sun, so this is the noon value and an
/// underestimate in the morning and evening). The members' faces turned
/// down take the light the sea reflects (eo-render landmarks_surface.wgsl
/// `lm_bridge_bounce_e`): the struts' undersides and the cables seen from
/// below drew black.
pub const SEA_ALBEDO: f64 = 0.06;

/// The decks' lines as the builds laid them (`BridgeFeatures::finish`), for
/// the render's analytic deck shadow (eo-render landmarks_surface.wgsl
/// `lm_deck_key`): the sun's shadow maps reach 160 m from the eye
/// (eo-look shadow.rs `MAX_FAR_M`), and past them a member under a deck, the
/// stiffening truss's bracing, the floor beams, the lower deck, took the
/// full sun through the deck above it. The Golden Gate's: south way end
/// (chainage, height), the main span, the tower and mid-span heights, the
/// north way end, the deck's half width; the west span's: its ends, their
/// heights, the crest, the grade, the step down to the lower deck, the
/// decks' half width. Chainage on each bridge's own frame.
static GG_DECK: OnceLock<[f64; 8]> = OnceLock::new();
static BAY_DECK: OnceLock<[f64; 8]> = OnceLock::new();

pub fn record_deck_line(id: &str, v: [f64; 8]) {
    let _ = if id == BRIDGE_IDS[0] { GG_DECK.set(v) } else { BAY_DECK.set(v) };
}

/// A bridge's deck line, once its build has run.
pub fn deck_line(id: &str) -> Option<[f64; 8]> {
    if id == BRIDGE_IDS[0] { GG_DECK.get().copied() } else { BAY_DECK.get().copied() }
}

/// The two bridges' ids.
pub const BRIDGE_IDS: [&str; 2] = ["golden-gate-bridge", "bay-bridge-west"];

/// `BridgeFeatures::finish`: a bridge draw's material as the coat it is. A
/// paint film is a dielectric (a binder of refractive index about 1.5, F0
/// 0.04: the specular is the film's, uncoloured, and the pigment is all in
/// the diffuse), so the painted steel, the painted cables and the far item
/// take metalness 0 (the registry's 0.2 and 0.35 tinted the sky's reflection
/// with the paint and added it to the diffuse: the sunlit faces went pink-white);
/// the painted steel is marked as the bridges' own (`bridgePaint`: its plate
/// seams and rivet rows in place of the landmark steel's mottle) and so is
/// the bridges' concrete (the intertidal band on the piers).
pub fn finish_material(id: &str, key: &str, m: &mut crate::data::Material) {
    if !features().finish || !BRIDGE_IDS.contains(&id) {
        return;
    }
    let painted = m.surface.as_deref() == Some("painted-steel");
    if painted || m.wire || key == "far" {
        m.metalness = 0.0;
        m.roughness = m.roughness.max(PAINT_ROUGHNESS);
    }
    if key == "far" {
        // the far item is the painted structure seen from afar (the registry's far batch was masonry-rough).
        m.roughness = 0.6;
        if id == BRIDGE_IDS[1] {
            // the west span's far item takes the bridges' coat (its sea light, the deck's shadow; the
            // Golden Gate's far item has its own floodlit surface, eo-render landmarks.rs).
            m.surface = Some("painted-steel".into());
        }
    }
    if m.surface.as_deref() == Some("painted-steel") || m.surface.as_deref() == Some("concrete") {
        m.surface_options = Some(serde_json::json!({ "bridge": true }));
    }
}

/// The baked occlusion a bridge's members take (`MeshArrays::shade_occlusion`:
/// faces turned down, faces turned in). With `finish` the frame's own sun
/// shadow and screen-space occlusion darken the members where they are
/// shadowed or enclosed, so the bake keeps only what those miss (the sky a
/// face turned down does not see, the interior's inter-reflection deficit).
/// INFERRED: 0.82 down, 0.88 in, from the first pass's 0.62 / 0.74 (the
/// struts' undersides were crushed to black). The Golden Gate with `shade2`
/// bakes none (golden_gate.rs `build`); the west span keeps this.
pub fn occlusion(down: f64, inward: f64) -> (f64, f64) {
    if features().finish { (down.max(0.82), inward.max(0.88)) } else { (down, inward) }
}

/// A built-up member laced in its own plane (`BridgeFeatures::members`):
/// two chords `depth` apart about the axis p0-p1, offset along `in_plane`
/// (unit, perpendicular to the axis), joined by single lacing bars
/// zig-zagging at `pitch`; the chords `chord` square, the bars `lace`
/// square. The open web shows the sky through it where a solid plank did not.
#[allow(clippy::too_many_arguments)]
pub fn laced(m: &mut MeshArrays, p0: V3, p1: V3, in_plane: V3, depth: f64, chord: f64, lace: f64, pitch: f64, colour: Rgb) {
    let h = (depth - chord) / 2.0;
    let off = |p: V3, k: f64| [p[0] + in_plane[0] * k, p[1] + in_plane[1] * k, p[2] + in_plane[2] * k];
    let len = ((p1[0] - p0[0]).powi(2) + (p1[1] - p0[1]).powi(2) + (p1[2] - p0[2]).powi(2)).sqrt();
    if !(len > 1e-6) {
        return;
    }
    let normal = {
        let a = [(p1[0] - p0[0]) / len, (p1[1] - p0[1]) / len, (p1[2] - p0[2]) / len];
        [a[1] * in_plane[2] - a[2] * in_plane[1], a[2] * in_plane[0] - a[0] * in_plane[2], a[0] * in_plane[1] - a[1] * in_plane[0]]
    };
    for k in [-h, h] {
        m.beam_up(off(p0, k), off(p1, k), chord, chord, colour, Some(in_plane));
    }
    let n = ((len / pitch).round() as usize).max(1);
    let at = |t: f64| [p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t, p0[2] + (p1[2] - p0[2]) * t];
    for i in 0..n {
        let (a, b) = (at(i as f64 / n as f64), at((i + 1) as f64 / n as f64));
        let (ka, kb) = if i % 2 == 0 { (-h, h) } else { (h, -h) };
        m.beam_up(off(a, ka), off(b, kb), lace, lace * 0.5, colour, Some(normal));
    }
    // batten plates at the ends.
    for t in [0.0, 1.0] {
        let p = at(t);
        m.beam_up(off(p, -h - chord / 2.0), off(p, h + chord / 2.0), chord * 1.6, chord * 0.6, colour, Some(normal));
    }
}

/// The far item's deck as one strip welded along the deck's curve
/// (`BridgeFeatures::members`): cross-sections every `step` from `c0` to
/// `c1` joined face to face (the 40 m boxes at their mean height stepped
/// along the camber), the top `above` over the deck, the bottom `depth`
/// under it, `half` either side.
#[allow(clippy::too_many_arguments)]
pub fn welded_strip(far: &mut MeshArrays, frame: &Frame, deck_top: &dyn Fn(f64) -> f64, c0: f64, c1: f64, step: f64, half: f64, depth: f64, above: f64, colour: Rgb) {
    let cs = stations(c0, c1, step);
    let ring = |c: f64| {
        let t = deck_top(c);
        [frame.P(c, -half, t - depth), frame.P(c, half, t - depth), frame.P(c, half, t + above), frame.P(c, -half, t + above)]
    };
    for i in 0..cs.len() - 1 {
        let (a, b) = (ring(cs[i]), ring(cs[i + 1]));
        for k in 0..4 {
            let j = (k + 1) % 4;
            far.quad(a[k], b[k], b[j], a[j], colour);
        }
    }
    let (a, b) = (ring(cs[0]), ring(cs[cs.len() - 1]));
    far.quad(a[0], a[1], a[2], a[3], colour);
    far.quad(b[3], b[2], b[1], b[0], colour);
}
