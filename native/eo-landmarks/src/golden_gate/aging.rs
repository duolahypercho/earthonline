//! The Golden Gate's aging pass (the sixth pass: `BridgeFeatures::matte`,
//! `fade`, `grime`, `underside`, `concrete_age`; `--no-gg-aging` turns every
//! one off and restores round 3's bridge finish). The bridge opened in 1937
//! and stands in salt fog; the owner, comparing a frame with the 2011 bluff
//! footage, asked for a structure that reads its age at 280 m to 2.4 km: a
//! chalked, near-matte coat whose tone wanders from section to section,
//! streaked under its ledges, darker and dirtier under the deck, and warm,
//! stained concrete. The values are read by eo-render (landmarks.rs
//! `bridges6_wgsl`, landmarks_surface.wgsl `lm_gg_age_*`).
//!
//! Facts used (goldengate.org, "Painting the Bridge" and "Color & Art Deco
//! Styling"): since 1965 the coat is an inorganic zinc silicate primer under
//! an acrylic emulsion topcoat; it is renewed by "routine touch up painting on
//! an on-going basis" where inspections find corrosion, so the bridge is never
//! one coat of one age; salt-laden fog attacks the film. Exterior acrylic
//! topcoats chalk under sunlight (the binder erodes, leaving loose pigment and
//! extender at the surface; the degree is rated by ASTM D4214), and chalking
//! is what drops a coat's gloss and chroma. Every amount below without its own
//! source is the owner's direction checked against the reference footage
//! (`ggloop/video/draft2`, the 2011 bluff pan, frame 100: lit tower legs at
//! HSV saturation 0.44 and luminance 0.10, the near pylon's lit face at hue
//! 42, saturation 0.21), marked so where it is set.

use std::sync::OnceLock;

/// `matte`: the coat's roughness floor (GGX alpha-roughness as the renderer
/// takes it). A chalked industrial coat is a flat finish: MPI's gloss scale
/// puts a flat ("G1, matte") finish at 0-5 gloss units at 60 deg (MPI
/// Architectural Painting Specification Manual, "Gloss and Sheen Standards"),
/// and a topcoat that chalks loses its gloss with its binder (ASTM D4214
/// rates the chalking, ASTM D523 measures the gloss). No measured
/// mapping from gloss units to a microfacet roughness exists; 0.85 is the
/// value at which the lit legs' specular highlight stops reading in the
/// matched frame (the owner's direction, checked against the footage),
/// against round 2's 0.6 for the coat when new (suspension.rs
/// `PAINT_ROUGHNESS`).
pub const PAINT_AGED_ROUGHNESS: f64 = 0.85;

/// `matte`: the far item's coat (the towers past the near items' range) at the same floor.
pub const FAR_AGED_ROUGHNESS: f64 = PAINT_AGED_ROUGHNESS;

/// `fade`: the share of a face's colour the chalk replaces on a face in the
/// open at the coat's mean age, and the chalk's colour (linear): the loose
/// pigment and the white extender together, a pale pink-grey (the coat's own
/// red diluted about five times in white). The share is the owner's direction
/// checked against the footage: it takes the lit legs' saturation from 0.58
/// toward the reference's 0.44 and their luminance up toward its 0.10.
pub const CHALK_SHARE: f64 = 0.28;
pub const CHALK_LINEAR: [f64; 3] = [0.40, 0.27, 0.25];
/// `fade`: the share a sheltered face keeps of it (turned down, or under the
/// deck: out of the sun, where an acrylic film chalks least).
pub const CHALK_SHELTERED: f64 = 0.35;

/// `grime`: the coat's age wanders over sections a few metres across (a
/// repainted stretch, a faded face): a smooth age field `AGE_PATCH_M` (across,
/// up: drawn out down the faces, as the weather works on them) and repaint
/// sections `REPAINT_M` (across, up: a face's width, a stage's height) on a share
/// `REPAINT_SHARE` of the faces, fresher (a darker, more saturated coat:
/// `REPAINT_TONE`). Sized to read at the footage's 300-800 m (8-32 px there
/// is 2.5-10 m on the towers, 0.8-3.4 m on the near pylon). INFERRED: the
/// sizes and shares, the owner's direction checked against the footage.
pub const AGE_PATCH_M: [f64; 2] = [3.5, 16.0];
pub const AGE_TONE: f64 = 0.09;
pub const REPAINT_M: [f64; 2] = [10.0, 18.0];
pub const REPAINT_SHARE: f64 = 0.12;
pub const REPAINT_TONE: f64 = -0.05;

/// `grime`: run-off streaks down vertical faces (rust and soot carried down
/// from rivet rows, cable bands and ledges): a streak field `STREAK_PITCH_M`
/// across and `STREAK_RUN_M` long, darkening by up to `STREAK_TONE`, strongest
/// within `STREAK_DECAY_M` under the ledge above and half that further down,
/// a share `STREAK_RUST` of it rust-brown. INFERRED: the owner's direction
/// checked against the footage.
pub const STREAK_PITCH_M: f64 = 2.4;
pub const STREAK_RUN_M: f64 = 30.0;
pub const STREAK_TONE: f64 = 0.24;
pub const STREAK_DECAY_M: f64 = 14.0;
pub const STREAK_RUST: f64 = 0.35;

/// `grime`: salt and soot at the towers' feet: the coat darkens to
/// `BASE_TONE` at the sea and recovers by `BASE_M` over it (the District:
/// rust attacked "from the bases of the towers"; the splash and the fog's salt
/// are thickest there). INFERRED: the height and the tone.
pub const BASE_M: f64 = 40.0;
pub const BASE_TONE: f64 = 0.72;

/// `underside`: the steel under the roadway (the stiffening truss's inner
/// members, the floor system, the laterals, the Fort Point arch's inner ribs):
/// the sun reaches a member inside the truss through the near truss plane's
/// lattice, which stops `1 - LATTICE_TRANSMIT` of it (the truss's chords, a
/// diagonal and a vertical over each panel: about a third of the panel face-on,
/// more at the sun's slant); the sky through the two truss planes and the deck
/// `UNDER_SKY` of what the open face sees (about the planes' transmittance
/// squared); and the coat there `UNDER_DIRT` darker (soot and salt that no
/// rain washes off). INFERRED: the solidity, from the drawn truss; the dirt's
/// tone, the owner's direction checked against the footage (the reference's
/// truss under the deck is a dark maroon). The dirtied coat's luminance factor,
/// 0.14 x 0.45 = 0.063 (0.14 x 0.53 = 0.074 on the outer faces), stays inside
/// the bracket golden_gate.rs cites for the coat (the stock match Fireweed's
/// light reflectance value 7, the District's printed matches 0.08-0.24).
pub const LATTICE_TRANSMIT: f64 = 0.55;
/// `underside`: the share of the under-deck dirt the truss's outer faces take
/// (they stand under the deck's edge, where the roadway's run-off drains over
/// them, but the rain reaches them). INFERRED, the owner's direction checked
/// against the footage (the reference's near truss: lit paint at luminance
/// 0.05 against the far tower's 0.11).
pub const UNDER_OUTER: f64 = 0.8;
pub const UNDER_SKY: f64 = 0.55;
pub const UNDER_DIRT: f64 = 0.45;

/// `concrete_age`: the concrete's chromaticity, a warm buff (the structure
/// sheet's "light buff-grey", taken warmer: the reference's lit pylon face
/// is hue 42, saturation 0.21 in the footage, where round 3's export grey
/// #9B968C drew hue 9, saturation 0.06), at the cited weathered albedo
/// (golden_gate.rs `CONCRETE2_LUMINANCE`; only the chromaticity changes).
/// The owner's direction checked against the footage.
pub const CONCRETE_BUFF_SRGB: u32 = 0xB5AA86;

/// `concrete_age`: run-off from the copings (the pylons' tops, the deck's
/// edge over a housing): dark grey-brown streaks `RUNOFF_PITCH_M` across,
/// darkening by up to `RUNOFF_TONE`, strongest within `RUNOFF_DECAY_M` under
/// the coping; a wall-scale band `WALL_PITCH_M` across of `WALL_TONE`;
/// efflorescence (lime leached and dried white on the face) as pale streaks
/// of `EFFLOR_SHARE` toward `EFFLOR_LINEAR`; the foot `FOOT_TONE` darker
/// within `FOOT_M` of the ground or the sea (splash, soil, the marine growth on
/// the pier). Weathered concrete's solar reflectance falls from 0.35-0.40 new
/// to 0.20-0.30 (Levinson and Akbari 2002, as golden_gate.rs cites): the
/// staining is normalised to keep each face's mean at the cited albedo. The
/// sizes and tones are the owner's direction checked against the footage.
pub const RUNOFF_PITCH_M: f64 = 1.1;
pub const RUNOFF_TONE: f64 = 0.38;
pub const RUNOFF_DECAY_M: f64 = 10.0;
pub const WALL_PITCH_M: f64 = 4.5;
pub const WALL_TONE: f64 = 0.20;
pub const EFFLOR_SHARE: f64 = 0.35;
pub const EFFLOR_LINEAR: [f64; 3] = [0.62, 0.60, 0.56];
pub const FOOT_M: f64 = 7.0;
pub const FOOT_TONE: f64 = 0.62;
/// `concrete_age`: the pier's foot at the sea (salt and marine growth): a band
/// `SEA_FOOT_M` high, darker and greener (`SEA_FOOT_LINEAR` of the face at the
/// water). INFERRED, the owner's direction checked against the footage (the
/// reference's south pier by the tower's foot: hue 110, luminance 0.03).
pub const SEA_FOOT_M: f64 = 12.0;
pub const SEA_FOOT_LINEAR: [f64; 3] = [0.42, 0.50, 0.42];

/// The pylons as built (chainage, top, half along, foot), recorded by the
/// Golden Gate's build for the concrete's run-off (`concrete_age`).
static PYLONS: OnceLock<Vec<[f64; 4]>> = OnceLock::new();

pub fn record_pylons(v: Vec<[f64; 4]>) {
    let _ = PYLONS.set(v);
}

/// The pylons, once the build has run.
pub fn pylons() -> Option<&'static [[f64; 4]]> {
    PYLONS.get().map(|v| v.as_slice())
}
