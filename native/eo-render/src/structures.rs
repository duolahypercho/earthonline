//! Grade-separated roads on the GPU (freeways, ramps, road bridges), behind
//! eo-app's `--structures`: eo-structures' tile meshes drawn by the city
//! pipeline from the shared object table, modelled on the street tiles
//! (street.rs). Nothing here exists without the switch: no arena, object,
//! group, material, pass or cache.
//!
//! Every 128 m tile (`eo_street::tile_key`) with anything to draw is one LOD
//! group of the street's kind (`GROUP_KIND_STREET`; cull.wgsl `lod`: the near
//! level within the street's near radius once resident, the far level to its
//! mid radius, the coarse level past it), its own (a street group's `ready`
//! bit is the street's residency), with the street's roles:
//!
//!   level 0  near    `LOD_STREET_NEAR`, streamed by distance on worker
//!                    threads through the shared upload queue
//!                    (`StructuresRuntime`): asked for within the street's
//!                    near load radius, dropped past its drop radius
//!   level 1  far     `LOD_STREET_FAR`, static, written at attach
//!   level 2  coarse  `LOD_STREET_COARSE`, static
//!
//! The group's sphere holds the geometry of all three levels (decks and
//! bents are tall, and a tile's steps reach past its square).
//!
//! A tile's mesh at one level becomes up to five objects (`Part`): the deck
//! surface (`ObjectKind::StructureDeck` with the street material, so it is
//! the same asphalt, wear and wet look as the streets it joins), its paint,
//! joints and downspouts (`StructureDeck`, vertex colour, as the street's
//! paint), and its concrete, one object per concrete tint
//! (`ObjectKind::Structure`, one material per tint: `concrete_material`).
//! A concrete vertex carries no colour: its colour bytes and its normal's w
//! hold the formwork coordinates (metres) and the wear (`encode_concrete`),
//! which structures.wgsl turns into board lines, panel joints, stain streaks
//! and the scanned board-formed concrete. The concrete casts sun shadows at
//! the level its group shows (look_shadow_cull.wgsl); the deck surface, the
//! paint and the joints receive only.
//!
//! eo-app reserves the structures' part of the shared arenas after the
//! street's (`StructuresArena`): the static far and coarse levels first,
//! then room for the near packs, sized from the meshes so that it holds every
//! tile within the drop radius of any point (`StructurePrepared::near_bound`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::GpuContext;
use crate::authored::{GpuLodGroup, GpuMaterial};
use crate::kit::RangeAlloc;
use crate::scene::{GpuObject, LOD_STREET_COARSE, LOD_STREET_FAR, LOD_STREET_NEAR, MATERIAL_VERTEX_COLOUR, ObjectKind, Vertex};
use crate::street::{GROUP_KIND_STREET, StreetRadii, class as sc};
use crate::upload::{Data, Job, Target, UploadQueue, Write};
use eo_structures::{StructureClass, StructureMesh, StructureWorld};
use glam::Vec3;

/// Upload job tag of a near pack (bit 56 and nothing above it; the other
/// streams use bits 57-62).
pub const TAG_STRUCTURES: u64 = 1 << 56;

fn is_structures_tag(tag: u64) -> bool {
    tag >> 56 == 1
}

/// The formwork coordinates of a concrete vertex are quantised to this
/// (metres): 16 bits each, so `u` spans 655.35 m and `v` 652.79 m.
pub const QUANT_M: f64 = 0.01;
/// `u` (along the boards: a chain's station, kilometres long) is carried
/// modulo this period, a whole number of every repeat the shader draws along
/// it (the 1.8 m board-formed scan, the 2.4 m and 1.2 m form panels, the 0.15 m
/// boards: 597.6 = 1.8 x 332 = 2.4 x 249), so the pattern runs on across the
/// wrap; a triangle across a wrap gets vertices of its own on one base
/// (`to_gpu`), so nothing is interpolated across it.
pub const U_PERIOD_M: f64 = 597.6;
/// `v` (across the boards: a lateral offset or a height relative to the
/// deck top, both within tens of metres) is carried with this bias.
pub const V_BIAS_M: f64 = 320.0;
/// A column's `v` is its height above its base plus this: the shader tells a
/// column from a swept face by it (a swept face's `v` never comes near).
pub const COLUMN_V_M: f64 = 200.0;
/// Concrete roughness (broom- and form-finished concrete; the street's
/// footway concrete is of the same order). INFERRED.
pub const CONCRETE_ROUGHNESS: f32 = 0.88;
/// Deck surface vertices this far apart or nearer at one station belong to
/// one cross-section (the widest deck is under 40 m). INFERRED.
const SECTION_REACH_M: f32 = 60.0;
/// Objects one near pack may need beyond the bound's (the deck, its paint
/// and a few tints): the object margin of the near part.
const MAX_PARTS: usize = 8;

/// What one object of a tile draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    /// The driven deck: the street material.
    Deck,
    /// Paint, expansion joints and downspouts: vertex colour.
    Plain,
    /// Concrete of one tint (an index into `Tints`; stage A9: its
    /// structure's `eo_structures::Era` packed over it, `concrete_key`).
    Concrete(u8),
    /// Stage A8 (`--tunnels`): the same three inside a tunnel's bore
    /// (eo-structures `StructureWorld::bore_at`), lit by the bore's lamps and
    /// the daylight its portal lets in, never by the sun or the open sky
    /// (structures.wgsl `sx_bore_light`). `portal`: the tile's portal
    /// (`GpuTile::portals`, from 1) whose daylight reaches the triangles
    /// (within `PORTAL_DAY_M` of its face), 0 for those deeper. A tile
    /// without a bore has none.
    BoreDeck(u8),
    BorePlain(u8),
    BoreConcrete(u8, u8),
    /// The bore's linear luminaire under the crown: its housing glows (`--tunnels`).
    Luminaire,
}

/// The object kind's bit for a part inside a bore (`Part::object_kind`;
/// bits 0-7 are the `ObjectKind`, bit 8 `scene::BLEND_BIT`): structures.wgsl
/// lights it as the bore (`SX_BORE_BIT`); `LUMINAIRE_BIT` makes it glow.
pub const BORE_BIT: u32 = 1 << 9;
pub const LUMINAIRE_BIT: u32 = 1 << 10;
/// Stage A9: a concrete part's era (`eo_structures::Era`: 0 old, 1 rebuilt,
/// 2 new) rides the object kind's bits 11-12 (structures.wgsl `sx_era`): how
/// weathered `--grime` draws it. 0 by default (no bit set on an old
/// structure's object, nor without the era in the mesh).
pub const ERA_SHIFT: u32 = 11;
/// A concrete part's key (`Part::Concrete`, `Part::BoreConcrete`): the tint
/// index in the low `TINT_BITS`, the era over it. A tint index of an era-0
/// part is the key itself.
pub const TINT_BITS: u32 = 6;

/// The key of tint `tint` (an index into `Tints`) of era `era`.
pub fn concrete_key(tint: u8, era: u8) -> u8 {
    (tint & ((1 << TINT_BITS) - 1)) | ((era & 3) << TINT_BITS)
}

/// A concrete key's tint index and era.
pub fn tint_of_key(k: u8) -> u8 {
    k & ((1 << TINT_BITS) - 1)
}
pub fn era_of_key(k: u8) -> u8 {
    k >> TINT_BITS
}
/// A bore part's portal rides its object's two terrain-only words (read
/// only for `LOD_TERRAIN` objects; a structure's roles are the street's):
/// `far_first_index` its face's point (x, z) less the object's box minimum
/// plus `PORTAL_MARGIN_M`, in 1 / `PORTAL_QUANT` m (16 bits each),
/// `far_index_count` the angle of its direction into the bore (low 16 bits,
/// of a turn) and `PORTAL_VALID`. Without a portal both stay 0.
/// (Stage A9: a 1/64 m step over 1024 m, from 128 m before the box, so a
/// portal up to `THRESHOLD_REACH_M` from a tile's triangles stays in reach.)
pub const PORTAL_MARGIN_M: f32 = 128.0;
pub const PORTAL_QUANT: f32 = 64.0;
pub const PORTAL_VALID: u32 = 1 << 31;
/// A triangle whose middle is this far or less in from its nearer portal
/// takes that portal's daylight (structures.wgsl: the sky seen through the
/// mouth, half the open sky's at the face and a share
/// `PORTAL_R_M^2 / (PORTAL_R_M^2 + d^2)` of that `d` in: the view factor of
/// a mouth of the bore's half height from its axis; 4 % at 12 m, 1 % at 30
/// m). INFERRED (a drawing rule; no measured portal light is sourced:
/// reference sheet 3.5 gives the threshold zone's lamp levels only).
pub const PORTAL_DAY_M: f64 = 30.0;
pub const PORTAL_R_M: f64 = 3.5;
pub const PORTAL_MOUTH_SKY: f64 = 0.5;
/// A triangle whose middle is less than this in from a portal's face is
/// not the bore's (the portal headwall's face stands on it, in daylight).
pub const BORE_FACE_M: f64 = 0.25;

/// Stage A9 (`--tunnels`): the threshold zone. By day a tunnel's first
/// stretch in from each portal is lit far brighter than its interior, so an
/// eye coming in from daylight sees into it (CIE 88:2004 via reference sheet
/// 3.5): the threshold zone, one stopping distance long, lit to
/// L_th = k x L_20 (L_20 the approach's luminance seen from that distance;
/// k about 0.05 for a 60 m stopping distance) over its first half, falling
/// to 0.4 L_th over its second; then the transition zone, L_tr = L_th x
/// (1.9 + t)^-1.4 (t seconds past the threshold zone at the design speed),
/// down to the interior. By night the interior is lit to the approach road's
/// level and there is no threshold (sheet 3.5). `THRESHOLD_ZONE_M` and
/// `THRESHOLD_K` are the sheet's 60 m example (k UNVERIFIED there: a recalled
/// CIE table); L_20 is taken as the day's ground (the approach road and
/// what stands round the portal; the sky's share is left out). INFERRED.
pub const THRESHOLD_ZONE_M: f64 = 60.0;
pub const THRESHOLD_K: f64 = 0.05;
/// The transition zone's design speed (m/s): the prima facie speed limit of a
/// business or residence district (CVC 22352(b)(1), 25 mph), the streets
/// the city's tunnels carry; `None` without the standards table (the
/// threshold zone then ends at its 0.4 L_th with no transition).
pub fn threshold_speed() -> Option<f64> {
    static V: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    *V.get_or_init(|| eo_data::DataPaths::discover().ok().and_then(|p| eo_data::CaStandards::load(&p).ok()).and_then(|t| t.metric("speed-prima-facie-business-residence").ok()))
}

/// The threshold zone's level over the interior's by day: L_th / L_int =
/// k x the day's ground over the interior (`TUNNEL_DAY_STOPS` under it).
pub fn threshold_over_interior() -> f64 {
    THRESHOLD_K * TUNNEL_DAY_STOPS.exp2()
}

/// The additional light the renderer puts into a lit portal. The unlit
/// switch leaves the legacy exposure in place for dark bores.
pub(crate) fn metered_bore_zone(depth: f64, lamps_on: f64, lit: bool) -> f64 {
    static METER_THRESHOLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let enabled = *METER_THRESHOLD.get_or_init(|| {
        let mut active = false;
        for arg in std::env::args() {
            match arg.as_str() {
                "--tunnel-portal-exposure" => active = true,
                "--no-tunnel-portal-exposure" => return false,
                _ => {}
            }
        }
        active
    });
    if lit && enabled { bore_zone_light(depth, lamps_on) } else { 1.0 }
}

/// The threshold and transition zones' level at `d` m in from a portal,
/// as a share of L_th (CIE 88's curve above).
pub fn threshold_share(d: f64) -> f64 {
    let z = THRESHOLD_ZONE_M;
    if d <= z / 2.0 {
        1.0
    } else if d <= z {
        1.0 - 0.6 * (d - z / 2.0) / (z / 2.0)
    } else {
        match threshold_speed() {
            Some(v) if v > 0.0 => (1.9 + (d - z) / v).powf(-1.4),
            _ => 0.0,
        }
    }
}

/// Relative illuminance at the eye, matching the lit lining and road at its
/// station (`sx_threshold` in structures.wgsl). The clock dims the entrance
/// lighting to the interior level at night.
fn bore_zone_light(depth: f64, lamps_on: f64) -> f64 {
    let day = (threshold_over_interior() * threshold_share(depth)).max(1.0);
    day + (1.0 - day) * lamps_on.clamp(0.0, 1.0)
}

/// How far in from a portal the threshold and transition zones raise the
/// bore's lighting over the interior's (the share at which L = L_int), and
/// so how far in a bore's triangles take their portal: past this the
/// interior.
pub fn threshold_reach() -> f64 {
    let r = threshold_over_interior();
    let z = THRESHOLD_ZONE_M;
    match threshold_speed() {
        Some(v) if v > 0.0 && r > 1.0 => z + v * ((1.0 / r).powf(-1.0 / 1.4) - 1.9).max(0.0),
        _ => z,
    }
    .max(PORTAL_DAY_M)
}

/// A portal ([x, z, dx, dz]: its face's point, the unit direction into the
/// bore) as an object's two words relative to its box minimum `min` (see
/// `PORTAL_MARGIN_M`); (0, 0) when it lies out of the words' reach.
pub fn encode_portal(min: [f32; 3], p: [f32; 4]) -> (u32, u32) {
    let q = |v: f32, m: f32| ((v - m + PORTAL_MARGIN_M) * PORTAL_QUANT).round();
    let (x, z) = (q(p[0], min[0]), q(p[1], min[2]));
    if !(0.0..=65535.0).contains(&x) || !(0.0..=65535.0).contains(&z) {
        return (0, 0);
    }
    let a = (p[3].atan2(p[2]) / std::f32::consts::TAU).rem_euclid(1.0);
    let ai = ((a * 65536.0).round() as u32) & 0xffff;
    ((x as u32) | ((z as u32) << 16), ai | PORTAL_VALID)
}

/// `encode_portal`'s portal back (for tests): [x, z, dx, dz].
pub fn decode_portal(min: [f32; 3], w: (u32, u32)) -> Option<[f32; 4]> {
    if w.1 & PORTAL_VALID == 0 {
        return None;
    }
    let x = (w.0 & 0xffff) as f32 / PORTAL_QUANT + min[0] - PORTAL_MARGIN_M;
    let z = (w.0 >> 16) as f32 / PORTAL_QUANT + min[2] - PORTAL_MARGIN_M;
    let a = (w.1 & 0xffff) as f32 / 65536.0 * std::f32::consts::TAU;
    Some([x, z, a.cos(), a.sin()])
}

impl Part {
    pub fn kind(self) -> ObjectKind {
        match self {
            Part::Concrete(_) | Part::BoreConcrete(..) => ObjectKind::Structure,
            _ => ObjectKind::StructureDeck,
        }
    }

    /// The object's `kind` word: `kind()`, a bore's bits, a concrete part's era.
    pub fn object_kind(self) -> u32 {
        self.kind() as u32
            | match self {
                Part::BoreDeck(_) | Part::BorePlain(_) => BORE_BIT,
                Part::BoreConcrete(k, _) => BORE_BIT | ((era_of_key(k) as u32) << ERA_SHIFT),
                Part::Luminaire => BORE_BIT | LUMINAIRE_BIT,
                Part::Concrete(k) => (era_of_key(k) as u32) << ERA_SHIFT,
                _ => 0,
            }
    }

    /// The tile portal (from 1, `GpuTile::portals`) whose daylight it takes; 0 none.
    pub fn portal(self) -> u8 {
        match self {
            Part::BoreDeck(p) | Part::BorePlain(p) | Part::BoreConcrete(_, p) => p,
            _ => 0,
        }
    }

    /// A part's object, with its kind's bits and its portal (`portals`: the tile's).
    fn object(self, min: [f32; 3], max: [f32; 3], first_index: u32, index_count: u32, base_vertex: i32, portals: &[[f32; 4]]) -> GpuObject {
        let mut o = GpuObject::new(self.kind(), min, max, first_index, index_count, base_vertex);
        o.kind = self.object_kind();
        if let Some(p) = (self.portal() as usize).checked_sub(1).and_then(|k| portals.get(k)) {
            (o.far_first_index, o.far_index_count) = encode_portal(min, *p);
        }
        o
    }
}

/// Where the structures' groups and materials are in the renderer's tables.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    /// The first structure group (one per tile, in `StructureSet::tiles` order).
    pub group_base: u32,
    /// The street surface material (`street::street_material`).
    pub street_material: u32,
    /// The first concrete material (one per tint, in `Tints` order).
    pub concrete_material: u32,
}

impl Placement {
    pub fn material(&self, p: Part) -> u32 {
        match p {
            Part::Deck | Part::BoreDeck(_) => self.street_material,
            Part::Plain | Part::BorePlain(_) | Part::Luminaire => MATERIAL_VERTEX_COLOUR,
            Part::Concrete(k) | Part::BoreConcrete(k, _) => self.concrete_material + tint_of_key(k) as u32,
        }
    }
}

/// The structures' part of the shared vertex, index and object arenas (the
/// renderer's extra space after the street's). All zero when not reserved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StructuresArena {
    pub vertex_base: u32,
    pub vertex_capacity: u32,
    pub index_base: u32,
    pub index_capacity: u32,
    pub object_base: u32,
    pub object_capacity: u32,
}

impl StructuresArena {
    pub fn is_empty(&self) -> bool {
        self.vertex_capacity == 0 && self.index_capacity == 0 && self.object_capacity == 0
    }
}

/// An object slot of the arena with nothing in it (no indices: never drawn,
/// never a caster).
pub fn zeroed_object() -> GpuObject {
    let mut o = GpuObject::new(ObjectKind::Structure, [0.0; 3], [0.0; 3], 0, 0, 0);
    o.lod_role = LOD_STREET_NEAR;
    o.lod_group = 0;
    o
}

// ---------------------------------------------------------------------------
// Concrete tints and materials.
// ---------------------------------------------------------------------------

/// The concrete tints the meshes carry (sRGB), clustered by chromaticity:
/// eo-structures tints each chain's concrete (light warm grey; the Central
/// Viaduct's yellow-pigmented concrete; the Presidio Parkway's very light
/// concrete) and jitters each bent's brightness by a few per cent; a jittered
/// tint joins its chain's (the jitter's brightness is given back by the
/// shader's own mottling). Each cluster is one material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tints {
    pub rgb: Vec<[u8; 3]>,
}

/// Two tints closer than this in chromaticity (L1) are one. The three A2
/// tints are 0.018 or more apart; a bent's jitter moves one by under 0.006.
const TINT_CHROMA_TOL: f32 = 0.008;

fn chroma(c: [u8; 3]) -> [f32; 3] {
    let s = (c[0] as f32 + c[1] as f32 + c[2] as f32).max(1.0);
    [c[0] as f32 / s, c[1] as f32 / s, c[2] as f32 / s]
}

fn chroma_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()
}

impl Tints {
    /// Clusters from how often each exact tint occurs: the most frequent
    /// first (ties by value), each later one joining the first cluster within
    /// `TINT_CHROMA_TOL`; a cluster's tint is its most frequent member.
    pub fn from_counts(counts: &BTreeMap<[u8; 3], u64>) -> Self {
        let mut order: Vec<([u8; 3], u64)> = counts.iter().map(|(k, v)| (*k, *v)).collect();
        order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut rgb: Vec<[u8; 3]> = vec![];
        for (c, _) in order {
            if !rgb.iter().any(|r| chroma_dist(chroma(*r), chroma(c)) < TINT_CHROMA_TOL) && rgb.len() < 255 {
                rgb.push(c);
            }
        }
        Self { rgb }
    }

    /// The cluster of a tint (the nearest by chromaticity; 0 when there is none).
    pub fn index(&self, c: [u8; 3]) -> u8 {
        let q = chroma(c);
        let mut best = (0usize, f32::MAX);
        for (i, r) in self.rgb.iter().enumerate() {
            let d = chroma_dist(chroma(*r), q);
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0 as u8
    }
}

/// How a tint's concrete is finished (the material's `params.z`, read by
/// structures.wgsl), told from the tint eo-structures gave its chain:
/// a yellow-pigmented tint is the Central Viaduct's (banded rough/smooth
/// columns, reference sheet 1.2 / 2.4, Roads & Bridges 2005), a very light
/// one the new Presidio Parkway's (clean, smooth, little staining, sheet
/// 5.10), anything else the light warm grey of the older viaducts.
pub fn finish_of(c: [u8; 3]) -> u32 {
    let warm = (c[0] as f32 - c[2] as f32) / (c[1] as f32).max(1.0);
    if c == eo_structures::TUNNEL_TILE_TINT {
        // stage A8: the bore's lining (only with `--tunnels`; no deck carries this tint).
        FINISH_TILE
    } else if warm > 0.2 {
        FINISH_BANDED
    } else if c.iter().all(|&x| x > 185) {
        FINISH_CLEAN
    } else {
        FINISH_STANDARD
    }
}
/// The share of the Central Viaduct tint's saturation its material keeps. INFERRED.
pub const BANDED_SATURATION: f32 = 0.65;
pub const FINISH_STANDARD: u32 = 0;
pub const FINISH_BANDED: u32 = 1;
pub const FINISH_CLEAN: u32 = 2;
/// A tunnel bore's lining (eo-structures `TUNNEL_TILE_TINT`): glossy small
/// cream-yellow glazed tiles over the whole wall and vault (reference sheet
/// 5.10, Broadway Tunnel interior; sheet 1.7: "tiled walls and ceiling",
/// "glossy tiles"), structures.wgsl `sx_tile`.
pub const FINISH_TILE: u32 = 3;
/// The glaze's roughness: a glossy ceramic glaze reflects the luminaire as
/// a streak down the walls (sheet 5.10). INFERRED (no measured gloss is
/// sourced; glazed wall tile is the glossiest surface in the city's set).
pub const TILE_ROUGHNESS: f32 = 0.09;

fn srgb_to_linear(b: u8) -> f32 {
    let c = b as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// The material of one concrete tint: the tint (linear) as the base colour,
/// the concrete's roughness, its finish in `params.z`. Read only by
/// structures.wgsl (`ObjectKind::Structure`): no vertex colour, no texture.
pub fn concrete_material(c: [u8; 3]) -> GpuMaterial {
    let mut m = GpuMaterial::vertex_colour();
    m.flags = 0;
    let lin = [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2])];
    // the yellow-pigmented tint at a little less of its saturation: at full
    // strength it read olive in the sky-lit shade under the deck (the
    // A3 captures); the sheet gives the pigment, not a measured colour.
    let keep = if finish_of(c) == FINISH_BANDED { BANDED_SATURATION } else { 1.0 };
    let l = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
    m.base = [l + (lin[0] - l) * keep, l + (lin[1] - l) * keep, l + (lin[2] - l) * keep, 1.0];
    let rough = if finish_of(c) == FINISH_TILE { TILE_ROUGHNESS } else { CONCRETE_ROUGHNESS };
    m.params = [rough, 0.0, finish_of(c) as f32, 0.0];
    m
}

// ---------------------------------------------------------------------------
// Vertex encoding.
// ---------------------------------------------------------------------------

fn rgb_of(c: u32) -> [u8; 3] {
    [(c & 255) as u8, ((c >> 8) & 255) as u8, ((c >> 16) & 255) as u8]
}

/// A concrete vertex: `u` (already brought into [0, 655.35) m) in the red and
/// green bytes, `v + V_BIAS_M` in the blue byte and the normal's w (a snorm
/// byte, so 255 values: v = blue + 256 x (w + 127)), the wear in alpha.
/// Interpolating the bytes interpolates the coordinates (each is a linear
/// function of them), so structures.wgsl decodes them per vertex
/// (`sx_decode`) and the rasteriser interpolates metres.
pub fn encode_concrete(pos: [f32; 3], normal: [f32; 3], u: f64, v: f64, wear: u8) -> Vertex {
    let ui = (u / QUANT_M).round().clamp(0.0, 65535.0) as u32;
    let vi = ((v + V_BIAS_M) / QUANT_M).round().clamp(0.0, (254 * 256 + 255) as f64) as u32;
    let w = (vi >> 8) as i32 - 127;
    let n = eo_assets::glb::pack_normal(Vec3::from(normal));
    Vertex { pos, normal: (n & 0x00ff_ffff) | ((w as i8 as u8 as u32) << 24), color: (ui & 255) | ((ui >> 8) << 8) | ((vi & 255) << 16) | ((wear as u32) << 24) }
}

/// The formwork coordinates and the wear of a concrete vertex, as
/// structures.wgsl `sx_decode` reads them (for tests).
pub fn decode_concrete(v: &Vertex) -> (f64, f64, u8) {
    let c = v.color;
    let w = ((v.normal >> 24) as u8 as i8) as f64;
    let u = ((c & 255) + ((c >> 8) & 255) * 256) as f64 * QUANT_M;
    let vv = (((c >> 16) & 255) as f64 + (w + 127.0) * 256.0) * QUANT_M - V_BIAS_M;
    (u, vv, (c >> 24) as u8)
}

/// A street-encoded vertex (street.rs `to_gpu`): the sRGB tint and the street
/// class in the colour, the distance to the kerb line in the normal's w
/// (/ 6.35 m, -127 none).
fn street_vertex(pos: [f32; 3], normal: [f32; 3], rgb: u32, class: u8, kerb: f32) -> Vertex {
    let w: i8 = if kerb >= 0.0 { ((kerb / 6.35).min(1.0) * 127.0).round() as i8 } else { -127 };
    let n = eo_assets::glb::pack_normal(Vec3::from(normal));
    Vertex { pos, normal: (n & 0x00ff_ffff) | ((w as u8 as u32) << 24), color: (rgb & 0x00ff_ffff) | ((class as u32) << 24) }
}

/// Each deck surface vertex's distance to its deck's edge (the barrier's
/// toe), from the deck's own cross-section: the widest offset at its station
/// less its own (the street material's kerb distance: the gutter's silt and
/// grime along the barrier). Near level only (the far deck has no points
/// between its edges, so the distance would read 0 across it); -1 elsewhere.
fn deck_kerb(m: &StructureMesh) -> Vec<f32> {
    let n = m.positions.len();
    let mut kd = vec![-1.0f32; n];
    let mut by_s: HashMap<u32, Vec<usize>> = HashMap::new();
    for i in 0..n {
        if m.classes[i] == StructureClass::DeckSurface as u8 {
            by_s.entry(m.uv[i][0].to_bits()).or_default().push(i);
        }
    }
    let reach2 = SECTION_REACH_M * SECTION_REACH_M;
    for list in by_s.values() {
        for &i in list {
            let p = Vec3::from(m.positions[i]);
            let h = list.iter().filter(|&&j| Vec3::from(m.positions[j]).distance_squared(p) <= reach2).map(|&j| m.uv[j][1].abs()).fold(0.0f32, f32::max);
            kd[i] = (h - m.uv[i][1].abs()).max(0.0);
        }
    }
    kd
}

/// Whether a concrete triangle's `u` runs up it (a column: `u` is its height
/// above the base there, `v` the distance round it): the in-plane gradient
/// of `u` points up.
fn u_runs_up(p: [[f64; 3]; 3], u: [f64; 3]) -> bool {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (e1, e2) = (sub(p[1], p[0]), sub(p[2], p[0]));
    let (d1, d2) = (u[1] - u[0], u[2] - u[0]);
    let (a, b, c) = (dot(e1, e1), dot(e1, e2), dot(e2, e2));
    let det = a * c - b * b;
    if det.abs() < 1e-12 {
        return false;
    }
    let (k1, k2) = ((d1 * c - d2 * b) / det, (d2 * a - d1 * b) / det);
    let g = [k1 * e1[0] + k2 * e2[0], k1 * e1[1] + k2 * e2[1], k1 * e1[2] + k2 * e2[2]];
    let len = dot(g, g).sqrt();
    len > 1e-6 && g[1] / len > 0.7
}

/// One tile's mesh at one level as GPU vertices and an index list per part.
#[derive(Clone, Debug, Default)]
pub struct GpuTile {
    pub vertices: Vec<Vertex>,
    /// Index lists local to `vertices`, by part in `Part` order, none empty.
    pub parts: Vec<(Part, Vec<u32>)>,
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// The portals whose daylight its bore parts take (`Part::portal`, from
    /// 1): [x, z, dx, dz]. Empty without a bore.
    pub portals: Vec<[f32; 4]>,
}

impl GpuTile {
    pub fn indices(&self) -> usize {
        self.parts.iter().map(|p| p.1.len()).sum()
    }
}

/// Convert one tile's mesh (level `lod`): every referenced vertex once
/// (concrete vertices of a triangle across a `u` wrap once more for it), the
/// triangles split by part (a triangle's class is its first vertex's).
pub fn to_gpu(m: &StructureMesh, tints: &Tints, lod: u8) -> GpuTile {
    to_gpu_with(m, tints, lod, &|_| None)
}

/// `to_gpu`, a triangle inside a tunnel's bore (`bore`: how far in from
/// the nearer portal its middle is and that portal, None outside every bore;
/// stage A8, `--tunnels`) going to the bore's parts: `BoreDeck`, `BorePlain`,
/// `BoreConcrete` by the portal whose daylight reaches it (`PORTAL_DAY_M`;
/// `GpuTile::portals`), the luminaire strip (`TUNNEL_LUMINAIRE_RGB` metal in
/// a bore) to `Luminaire`. A triangle within `BORE_FACE_M` of a portal's face
/// (the headwall) is not the bore's.
pub fn to_gpu_with(m: &StructureMesh, tints: &Tints, lod: u8, bore: &dyn Fn([f64; 3]) -> Option<(f64, Option<[f64; 4]>)>) -> GpuTile {
    to_gpu_reach(m, tints, lod, bore, PORTAL_DAY_M)
}

/// `to_gpu_with`, a bore's triangles taking their portal out to `reach` in
/// from it (stage A9: `threshold_reach()`, the threshold zone's lighting;
/// the portal's daylight still stops at `PORTAL_DAY_M`, structures.wgsl).
pub fn to_gpu_reach(m: &StructureMesh, tints: &Tints, lod: u8, bore: &dyn Fn([f64; 3]) -> Option<(f64, Option<[f64; 4]>)>, reach: f64) -> GpuTile {
    let n = m.positions.len();
    let kerb = if lod == 0 { deck_kerb(m) } else { vec![-1.0; n] };
    let mut remap = vec![u32::MAX; n];
    let mut verts: Vec<Vertex> = Vec::with_capacity(n);
    let mut parts: BTreeMap<Part, Vec<u32>> = BTreeMap::new();
    let mut portals: Vec<[f32; 4]> = vec![];
    let p64 = |i: usize| m.positions[i].map(|x| x as f64);
    for t in m.indices.chunks_exact(3) {
        let i0 = t[0] as usize;
        let class = StructureClass::of(m.classes[i0]);
        let mid = {
            let (a, b, c) = (p64(t[0] as usize), p64(t[1] as usize), p64(t[2] as usize));
            [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0]
        };
        let hit = bore(mid);
        let lamp = class == Some(StructureClass::Metal) && rgb_of(m.colors[i0]) == eo_structures::TUNNEL_LUMINAIRE_RGB;
        // in the bore: the portal whose daylight reaches it (its index in the tile's list, from 1), else 0.
        let band = hit.filter(|h| h.0 >= BORE_FACE_M).map(|(d, portal)| match portal.filter(|_| d <= reach) {
            Some(p) => {
                let p = p.map(|v| v as f32);
                let k = portals.iter().position(|q| *q == p).unwrap_or_else(|| {
                    portals.push(p);
                    portals.len() - 1
                });
                (k + 1).min(255) as u8
            }
            None => 0,
        });
        let part = match (class, band) {
            (_, _) if lamp && hit.is_some() => Part::Luminaire,
            (Some(StructureClass::DeckSurface), None) => Part::Deck,
            (Some(StructureClass::DeckSurface), Some(b)) => Part::BoreDeck(b),
            (Some(StructureClass::Concrete), None) => Part::Concrete(concrete_key(tints.index(rgb_of(m.colors[i0])), m.era(i0) as u8)),
            (Some(StructureClass::Concrete), Some(b)) => Part::BoreConcrete(concrete_key(tints.index(rgb_of(m.colors[i0])), m.era(i0) as u8), b),
            (_, None) => Part::Plain,
            (_, Some(b)) => Part::BorePlain(b),
        };
        let list = parts.entry(part).or_default();
        if matches!(part, Part::Concrete(_) | Part::BoreConcrete(..)) {
            let ids = [t[0] as usize, t[1] as usize, t[2] as usize];
            let pos = ids.map(p64);
            let raw = ids.map(|i| (m.uv[i][0] as f64, m.uv[i][1] as f64));
            let column = u_runs_up(pos, raw.map(|q| q.0));
            // a column's (round, height + COLUMN_V_M); a swept face's (along, across).
            let uv = raw.map(|(a, b)| if column { (b, a + COLUMN_V_M) } else { (a, b) });
            let k = uv.map(|q| (q.0 / U_PERIOD_M).floor() as i64);
            let wear = |i: usize| (m.colors[i] >> 24) as u8;
            if k[0] == k[1] && k[1] == k[2] {
                for (j, &i) in ids.iter().enumerate() {
                    if remap[i] == u32::MAX {
                        remap[i] = verts.len() as u32;
                        verts.push(encode_concrete(m.positions[i], m.normals[i], uv[j].0 - k[j] as f64 * U_PERIOD_M, uv[j].1, wear(i)));
                    }
                    list.push(remap[i]);
                }
            } else {
                // across a wrap: three vertices of its own, on the lowest period.
                let base = *k.iter().min().unwrap() as f64 * U_PERIOD_M;
                for (j, &i) in ids.iter().enumerate() {
                    list.push(verts.len() as u32);
                    verts.push(encode_concrete(m.positions[i], m.normals[i], uv[j].0 - base, uv[j].1, wear(i)));
                }
            }
            continue;
        }
        for &i in t {
            let i = i as usize;
            if remap[i] == u32::MAX {
                remap[i] = verts.len() as u32;
                let (class, k) = match StructureClass::of(m.classes[i]) {
                    Some(StructureClass::DeckSurface) if (m.colors[i] >> 24) as u8 == eo_structures::EAST_SPAN_PAVING_ALPHA => (sc::EAST_SPAN_ROAD, kerb[i]),
                    Some(StructureClass::DeckSurface) => (sc::ASPHALT, kerb[i]),
                    Some(StructureClass::Paint) => (sc::PAINT, -1.0),
                    // joints (steel armour) and downspouts: the paint object's
                    // non-paint class (the concrete roughness, no paint wear).
                    _ => (sc::VERGE, -1.0),
                };
                verts.push(street_vertex(m.positions[i], m.normals[i], m.colors[i], class, k));
            }
            list.push(remap[i]);
        }
    }
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &verts {
        for k in 0..3 {
            mn[k] = mn[k].min(v.pos[k]);
            mx[k] = mx[k].max(v.pos[k]);
        }
    }
    if verts.is_empty() {
        (mn, mx) = ([0.0; 3], [0.0; 3]);
    }
    GpuTile { vertices: verts, parts: parts.into_iter().filter(|p| !p.1.is_empty()).collect(), min: mn, max: mx, portals }
}

// ---------------------------------------------------------------------------
// Stage A8: a tunnel bore's lining and lighting (`--tunnels`).
// ---------------------------------------------------------------------------

/// The lining's tile: US 4-1/4 in (108 mm) glazed ceramic wall tile, the
/// small (~10 cm) tile of the Broadway Tunnel (reference sheet 5.10), fitted
/// to a whole number of tiles in `U_PERIOD_M` (5520), so the grid runs on
/// across the station's wrap. INFERRED (the tile's size is read off the
/// photo notes; the standard size is common US practice).
pub const TILE_M: f64 = U_PERIOD_M / 5520.0;
/// The grout joint (1/8 in) and the tile's cushion edge rolling into it, and
/// the grout's colour (sRGB: a cement grout greyed by the bore's soot).
/// INFERRED (common US wall tile practice).
pub const GROUT_M: f64 = 0.003;
pub const TILE_EDGE_M: f64 = 0.004;
pub const GROUT_RGB: [u8; 3] = [150, 146, 136];
/// The bore's lighting. The interior zone is lit to 7 cd/m2 on the roadway
/// (the low end of the levels often chosen, 7-15; the 2-6 of long tunnels
/// below it: reference sheet 3.5), 100 lux over a road surface's q0 of 0.07
/// cd/m2/lx (sheet 2.2, CIE R3): `TUNNEL_INTERIOR_LUX`; the lamps a pitch
/// apart under the crown (`eo_structures` tunnel lights, 8 m at 5.95 m) add
/// `TUNNEL_POOL_OVERLAP` times one lamp's nadir there. By night the interior
/// is lit to about the approach road's level (sheet 3.5): the street lamps'
/// pool peak (`eo_sky::photometry::LAMP_POOL_PEAK_LUX`) under the same overlap.
pub const TUNNEL_INTERIOR_LUX: f64 = 100.0;
pub const TUNNEL_POOL_OVERLAP: f64 = 1.43;
/// By day the rig's sunlit ground is not on the lamps' photometric scale
/// (eo-sky's `lamp_units_per_lux` reads the night rig: the day's sunlit and
/// sky-lit ground comes to about 100 of its lux, where the real one is about
/// 100,000), so the day's interior is set against the day's ground instead:
/// this many stops under it (the real ratio of a 7 cd/m2 interior to a sunlit
/// road is about ten stops; seven keep the portal dark from the street, as a
/// camera exposed for the street shows it, and the interior within reach of
/// the exposure's adaptation). INFERRED (a mapping of the rig's scale, not a
/// measurement).
pub const TUNNEL_DAY_STOPS: f64 = 7.0;
/// The even interreflection in a bore lined in light glazed tile (about 0.6
/// reflectance over three quarters of its surface, the asphalt's 0.07-0.1 over
/// the rest): the light it returns of the lamps' is about a third of the
/// roadway's level. INFERRED (an even-room estimate, not a measurement).
pub const BORE_BOUNCE: f64 = 0.35;
/// The linear luminaire's diffuser luminance over the interior's roadway
/// illuminance (cd/m2 per lux): an LED tunnel luminaire behind an opal cover,
/// about 10,000 cd/m2 over the interior's 100 lux. INFERRED (no measured
/// luminaire is sourced; of the order of lit LED panels).
pub const LUMINAIRE_OVER_INTERIOR: f64 = 10_000.0 / TUNNEL_INTERIOR_LUX;
/// Where the luminaire's image falls in the glaze (structures.wgsl
/// `sx_tile`): across the bore from a wall to the crown (half the inside of
/// a bore of two 3.66 m lanes and a 0.9 m walkway: `travel-lane-width-state-
/// highway-min` and eo-structures' `WALKWAY_M`), the luminaire's face under
/// the crown (eo-structures `tunnel::luminaire`: 0.02-0.12 m under it) and
/// the half size its diffuser shows in the glaze (its 0.3 m face seen at a
/// glancing angle). INFERRED (one reach for every bore; a twin's or a wider
/// bore's streak stands a little off).
pub const BORE_REACH_M: f64 = 4.6;
pub const LUMINAIRE_FACE_M: f64 = eo_structures::BORE_CLEAR_M - 0.12;
pub const LUMINAIRE_HALF_M: f64 = 0.08;
/// The tunnel lamps' colour, linear RGB of unit luminance: 4000 K (the LED
/// tunnel practice of 4000-4500 K, reference sheet 3.4; the Planckian locus
/// at 4000 K, xy 0.3805 / 0.3768, in linear sRGB).
pub const TUNNEL_LAMP_LINEAR: [f32; 3] = [1.414, 0.924, 0.533];
/// Inside a bore the camera adapts to the bore's lighting (sky.rs): to the
/// interior's level, `BORE_EXPOSURE_UNDER` short of the whole way (a camera
/// exposed for the tunnel shows it dim: sheet 5.10 "the interior reads dim
/// and warm", the portal "blown out by daylight"), over the first
/// `BORE_ADAPT_M` in from a portal, never more than `BORE_EXPOSURE_MAX` over
/// the day's. INFERRED (a camera rule).
pub const BORE_EXPOSURE_UNDER: f64 = 0.35;
pub const BORE_ADAPT_M: f64 = 20.0;
pub const BORE_EXPOSURE_MAX: f64 = 1024.0;
/// `props::GpuLampLight::pad[0]` of a tunnel bore's lamp: lit round the
/// clock at the bore's level (sky_lights.wgsl `lights_props`).
pub const TUNNEL_LAMP_FLAG: u32 = 1;

/// The day's ground irradiance in the rig's units (the sunlit and sky-lit
/// card eo-sky's meter keys on, `night::key_on_ground` x beam + `ground_fill`).
fn day_ground(f: &eo_look::LookFrame, terms: &eo_sky::SkyTerms) -> f64 {
    use eo_sky::night;
    night::key_on_ground(f) * night::mean_beam(&terms.weather) + night::ground_fill(f, night::SIDEWALK_ENV)
}

/// The bore's interior roadway irradiance in the rig's units: by day
/// `TUNNEL_DAY_STOPS` under the day's ground, by night the approach road's
/// (`LAMP_POOL_PEAK_LUX` x `TUNNEL_POOL_OVERLAP` on the lamps' scale),
/// between them by the street lamps' clock. Sky.rs hands it to the lamps
/// (their pool, `LtParams.props.w`) and the city shader (`Sky.twi.w`: the
/// interreflection and the luminaire's glow).
pub fn bore_interior_units(f: &eo_look::LookFrame, terms: &eo_sky::SkyTerms) -> f64 {
    let day = day_ground(f, terms) * (-TUNNEL_DAY_STOPS).exp2();
    let night = eo_sky::photometry::LAMP_POOL_PEAK_LUX * TUNNEL_POOL_OVERLAP * terms.lamp_units_per_lux;
    day + (night - day) * terms.lamps_on.clamp(0.0, 1.0)
}

/// The bore's adapted exposure key (exposure x the interior's roadway
/// irradiance, rig units): the clear noon's exposure of its sunlit ground,
/// `BORE_EXPOSURE_UNDER` short. A camera that adapts to the bore shows it
/// alike by day and by night (its lamps are dimmed at night, the camera opens
/// up less).
fn bore_key() -> f64 {
    static K: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *K.get_or_init(|| {
        let mut f = eo_look::LookFrame::new(12.0);
        let t = eo_sky::apply(&mut f, &eo_sky::Conditions { hour: 12.0, weather: eo_sky::weather::preset(eo_sky::Kind::Clear), moon_day: 0.0 });
        BORE_EXPOSURE_UNDER * t.exposure_target * day_ground(&f, &t)
    })
}

/// The exposure's gain over the frame's target for an eye `depth` in from a
/// bore's nearer portal (sky.rs, in place of the meter's): from the meter's
/// at the portal to the bore's adaptation `BORE_ADAPT_M` in, which exposes the
/// interior (`bore_interior_units`) at the bore's key (`bore_key`): up by day
/// (the day's ground is far brighter), down by night (the night solve exposes
/// the unlit street, darker than the lit bore); within `BORE_EXPOSURE_MAX`
/// either way.
pub fn bore_exposure_gain(f: &eo_look::LookFrame, terms: &eo_sky::SkyTerms, meter: f64, depth: f64) -> f64 {
    bore_exposure_in_zone(f, terms, meter, depth, 1.0)
}

pub(crate) fn bore_exposure_in_zone(f: &eo_look::LookFrame, terms: &eo_sky::SkyTerms, meter: f64, depth: f64, zone: f64) -> f64 {
    let full = (bore_key() / (zone * bore_interior_units(f, terms) * terms.exposure_target).max(1e-12)).clamp(1.0 / BORE_EXPOSURE_MAX, BORE_EXPOSURE_MAX);
    let t = eo_look::math::smoothstep(0.0, BORE_ADAPT_M, depth);
    meter + (full - meter) * t
}

// ---------------------------------------------------------------------------
// Stage A9: the structures' weathering (`--grime`; structures.wgsl
// `sx_weather`, `sx_tile_weather`). Albedo and roughness only.
// ---------------------------------------------------------------------------

/// How heavily each era (`eo_structures::Era`) weathers, of the full: the
/// 1955-1971 viaducts and ramps and the 1914-1952 tunnels heavy, the
/// 2005-2008 Central Viaduct and West Approach medium, the 2012-15 Presidio
/// Parkway "very light, clean" (reference sheet 1.1, 1.2, 1.7, 5.10). The
/// hyper level (`--grime 2`) a little more. INFERRED (the build years are
/// sourced; how dirty a year makes concrete is not).
pub const ERA_WEATHER: [f64; 3] = [1.0, 0.65, 0.2];
pub const WX_HYPER: f64 = 1.2;
/// Soiling of the whole surface at full weathering: new grey concrete
/// reflects about 0.40, weathered and soiled concrete less (sheet 2.1: EPA
/// 2012, "falls with age"; 0.25-0.35 the common, UNVERIFIED, reading; W1's
/// walks at LBNL's 0.20-0.30): with the wear's own grime this brings the
/// old viaducts' light warm grey to about 0.28-0.32. The dirt's grey (linear
/// RGB multiplier: a cooler, greyer cast). INFERRED.
pub const WX_SOIL: f64 = 0.3;
pub const WX_DIRT_TINT: [f64; 3] = [0.97, 0.97, 0.95];
/// Run-off streaks: column pitch (m), darkening at full and their tint (the
/// soot and dirt the water carries, W1's facade streak tint), the share of
/// streaks bleeding rust under joints and bearings and of white leaching
/// ones (efflorescence) at full weathering (sheet 1.5, AASHTO defect 1120:
/// leaching, then rust staining). INFERRED.
pub const WX_STREAK_COL_M: f64 = 0.55;
pub const WX_STREAK: f64 = 0.72;
pub const WX_STREAK_TINT: [f64; 3] = [0.58, 0.56, 0.54];
pub const WX_RUST: f64 = 0.3;
pub const WX_LEACH: f64 = 0.35;
/// The fascia's run-off starts at the barrier's top (`v` of the band faces
/// is the height over the deck top: concrete-barrier-type-60m-height is the
/// mesh's; the band reaches 1.4 m) and gathers at the drip edge; the
/// barrier's toe takes the road's spray. INFERRED.
pub const WX_BAND_TOP: f64 = 1.1;
pub const WX_DRIP: f64 = 0.42;
pub const WX_TOE: f64 = 0.4;
/// Dirt on what faces up, and the green-brown of lichen on old copings. INFERRED.
pub const WX_TOP: f64 = 0.25;
pub const WX_MOSS: [f64; 3] = [0.82, 0.86, 0.72];
/// A bore's lining (`sx_tile_weather`): soot over the vault and round the
/// luminaire, soot in the grout, the road spray's tint over the bottom
/// metre, the share of crazed (dull) tiles, of stained tiles and their tint,
/// of replaced (whiter) tiles; seepage from the spring line
/// (`eo_structures` tunnel `SPRING_M`). INFERRED (sheet 5.10 shows a tiled
/// lining "slightly irregular"; its soiling is not described).
pub const WX_SOOT: f64 = 0.38;
pub const WX_GROUT_SOOT: f64 = 0.5;
pub const WX_SPRAY_TINT: [f64; 3] = [0.42, 0.39, 0.35];
pub const WX_TILE_DULL: f64 = 0.08;
pub const WX_TILE_STAIN: f64 = 0.012;
pub const WX_TILE_STAIN_TINT: [f64; 3] = [0.92, 0.88, 0.8];
pub const WX_TILE_NEW: f64 = 0.008;
pub const WX_SPRING_M: f64 = 4.0;

/// WGSL: the encoding's constants and structures.wgsl (appended to the city
/// shader by look.rs).
pub fn wgsl() -> String {
    let lin = |c: [u8; 3]| c.map(srgb_to_linear);
    let [g0, g1, g2] = lin(GROUT_RGB);
    let [l0, l1, l2] = TUNNEL_LAMP_LINEAR;
    let east_paving = eo_structures::east_span_pavement();
    let joints = eo_structures::east_span_paving_joints().map(|[x, z, d]| format!("vec3<f32>({x:.8}, {z:.8}, {d:.6})")).join(", ");
    let mouth_light = !std::env::args().any(|s| s == "--no-tunnel-mouth-lighting")
        && std::env::args().any(|s| s == "--tunnel-mouth-lighting");
    format!(
        "\n// generated by structures.rs wgsl\nconst SX_EAST_PAVING: bool = {east_paving};\nconst SX_PAVING_JOINTS: array<vec3<f32>, 3> = array({joints});\nconst SX_MOUTH_LIGHTING: bool = {mouth_light};\nconst SX_QUANT: f32 = {QUANT_M:.6};\nconst SX_V_BIAS: f32 = {V_BIAS_M:.3};\nconst SX_COLUMN_V: f32 = {COLUMN_V_M:.3};\nconst SX_FINISH_BANDED: u32 = {FINISH_BANDED}u;\nconst SX_FINISH_CLEAN: u32 = {FINISH_CLEAN}u;\n{}{}",
        format_args!(
            "{}const SX_FINISH_TILE: u32 = {FINISH_TILE}u;\nconst SX_TILE_M: f32 = {TILE_M:.9};\nconst SX_GROUT_M: f32 = {GROUT_M:.6};\nconst SX_TILE_EDGE_M: f32 = {TILE_EDGE_M:.6};\nconst SX_GROUT_RGB: vec3<f32> = vec3<f32>({g0:.6}, {g1:.6}, {g2:.6});\nconst SX_BORE_BIT: u32 = {BORE_BIT}u;\nconst SX_LUMINAIRE_BIT: u32 = {LUMINAIRE_BIT}u;\nconst SX_PORTAL_VALID: u32 = {PORTAL_VALID}u;\nconst SX_PORTAL_MARGIN_M: f32 = {PORTAL_MARGIN_M:.3};\nconst SX_PORTAL_QUANT: f32 = {PORTAL_QUANT:.3};\nconst SX_PORTAL_R2: f32 = {:.4};\nconst SX_PORTAL_SKY: f32 = {PORTAL_MOUTH_SKY:.4};\nconst SX_PORTAL_DAY_M: f32 = {PORTAL_DAY_M:.3};\nconst SX_BORE_BOUNCE: f32 = {BORE_BOUNCE:.4};\nconst SX_LUMINAIRE_OVER: f32 = {LUMINAIRE_OVER_INTERIOR:.3};\nconst SX_BORE_REACH_M: f32 = {BORE_REACH_M:.3};\nconst SX_LAMP_FACE_M: f32 = {LUMINAIRE_FACE_M:.3};\nconst SX_LAMP_HALF_M: f32 = {LUMINAIRE_HALF_M:.3};\nconst SX_LAMP_RGB: vec3<f32> = vec3<f32>({l0:.4}, {l1:.4}, {l2:.4});\n",
            a9_wgsl(),
            PORTAL_R_M * PORTAL_R_M
        ),
        include_str!("structures.wgsl")
    )
}

/// Stage A9's WGSL constants (the era, the weathering, the threshold zone).
fn a9_wgsl() -> String {
    let v3 = |c: [f64; 3]| format!("vec3<f32>({:.4}, {:.4}, {:.4})", c[0], c[1], c[2]);
    let mut s = format!("const SX_ERA_SHIFT: u32 = {ERA_SHIFT}u;\n");
    for (k, v) in [
        ("SX_ERA_OLD", ERA_WEATHER[0]),
        ("SX_ERA_REBUILT", ERA_WEATHER[1]),
        ("SX_ERA_NEW", ERA_WEATHER[2]),
        ("SX_WX_HYPER", WX_HYPER),
        ("SX_WX_SOIL", WX_SOIL),
        ("SX_WX_STREAK_COL_M", WX_STREAK_COL_M),
        ("SX_WX_STREAK", WX_STREAK),
        ("SX_WX_RUST", WX_RUST),
        ("SX_WX_LEACH", WX_LEACH),
        ("SX_WX_BAND_TOP", WX_BAND_TOP),
        ("SX_WX_DRIP", WX_DRIP),
        ("SX_WX_TOE", WX_TOE),
        ("SX_WX_TOP", WX_TOP),
        ("SX_WX_SOOT", WX_SOOT),
        ("SX_WX_GROUT_SOOT", WX_GROUT_SOOT),
        ("SX_WX_TILE_DULL", WX_TILE_DULL),
        ("SX_WX_TILE_STAIN", WX_TILE_STAIN),
        ("SX_WX_TILE_NEW", WX_TILE_NEW),
        ("SX_WX_SPRING_M", WX_SPRING_M),
        ("SX_TH_ZONE_M", THRESHOLD_ZONE_M),
        ("SX_TH_OVER", threshold_over_interior()),
        ("SX_TH_SPEED", threshold_speed().unwrap_or(0.0)),
    ] {
        s.push_str(&format!("const {k}: f32 = {v:.6};\n"));
    }
    for (k, v) in [("SX_WX_DIRT_TINT", WX_DIRT_TINT), ("SX_WX_STREAK_TINT", WX_STREAK_TINT), ("SX_WX_MOSS", WX_MOSS), ("SX_WX_SPRAY_TINT", WX_SPRAY_TINT), ("SX_WX_TILE_STAIN_TINT", WX_TILE_STAIN_TINT)] {
        s.push_str(&format!("const {k}: vec3<f32> = {};\n", v3(v)));
    }
    s
}

// ---------------------------------------------------------------------------
// The set: tiles, groups, the static levels.
// ---------------------------------------------------------------------------

/// One tile: its key and the bounds of everything it draws at any level.
#[derive(Clone, Copy, Debug)]
pub struct StructureTile {
    pub key: (i32, i32),
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Per level (near, far, coarse) over every tile, as converted.
#[derive(Clone, Copy, Debug, Default)]
pub struct LevelTotals {
    pub triangles: [u64; 3],
    pub vertices: [u64; 3],
    /// The largest single tile (vertices, indices, objects) per level.
    pub max_tile: [(u32, u32, u32); 3],
}

/// The tiles and what is fixed for them at load (shared with the workers).
pub struct StructureSet {
    pub world: Arc<StructureWorld>,
    pub tiles: Vec<StructureTile>,
    pub tile_of_key: HashMap<(i32, i32), u32>,
    pub tints: Tints,
    pub radii: StreetRadii,
    /// Per tile, the near pack's (vertices, indices, objects).
    pub near_sizes: Vec<(u32, u32, u32)>,
    /// The most near-pack vertices, indices and objects within the drop
    /// radius of any point (what the near part of the arena must hold).
    pub near_bound: (u32, u32, u32),
    pub totals: LevelTotals,
    /// Milliseconds to build and convert every tile at load.
    pub prepare_ms: f64,
}

impl StructureSet {
    /// Camera to the tile's footprint (its bounds in plan).
    pub fn tile_distance(&self, t: usize, cam: Vec3) -> f32 {
        let tile = &self.tiles[t];
        let dx = (tile.min[0] - cam.x).max(0.0).max(cam.x - tile.max[0]);
        let dz = (tile.min[2] - cam.z).max(0.0).max(cam.z - tile.max[2]);
        (dx * dx + dz * dz).sqrt()
    }

    pub fn tile_box(&self, t: usize) -> (Vec3, Vec3) {
        (Vec3::from(self.tiles[t].min), Vec3::from(self.tiles[t].max))
    }

    /// The tile's LOD group: its bounds' sphere (cull.wgsl takes the
    /// distance to the sphere), street kind, far level until the LOD pass runs.
    pub fn group(&self, t: usize) -> GpuLodGroup {
        let (mn, mx) = self.tile_box(t);
        let c = (mn + mx) * 0.5;
        GpuLodGroup { center: c.to_array(), radius: (mx - mn).length() * 0.5, ready: 0, detailed: 2, kind: GROUP_KIND_STREET, level: 1 }
    }
}

/// One static object (far or coarse), before it is placed.
#[derive(Clone, Copy, Debug)]
pub struct StaticObject {
    pub tile: u32,
    pub part: Part,
    pub role: u32,
    /// Relative to the static vertices and indices.
    pub first: u32,
    pub count: u32,
    pub base_vertex: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// A bore part's portal (`Part::portal`; `GpuTile::portals`).
    pub portal: Option<[f32; 4]>,
}

/// The far and coarse levels of every tile, written at attach.
#[derive(Default)]
pub struct StructureStatic {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub objects: Vec<StaticObject>,
}

/// Everything made at load for the structures to be drawn.
pub struct StructurePrepared {
    pub set: Arc<StructureSet>,
    pub statics: StructureStatic,
}

/// Near-pack sizes are bounded at camera points this far apart. INFERRED.
const BOUND_STEP_M: f32 = 32.0;
/// The near part of the arena is the bound times this (rounding up), a
/// margin for the frames a jump evicts at most a few packs a frame.
const NEAR_MARGIN: f32 = 1.25;

impl StructurePrepared {
    /// Build every tile's three levels on `threads` threads (eo-structures
    /// keeps them; the near ones are asked for again by the streaming
    /// workers), convert the far and coarse levels for the static part, and
    /// size the near part.
    pub fn prepare(world: Arc<StructureWorld>, radii: StreetRadii, threads: usize) -> Self {
        let t0 = Instant::now();
        let keys = world.tile_keys();
        let threads = threads.max(1);
        let chunk = keys.len().div_ceil(threads).max(1);
        // meshes of every level (shared Arcs) and the concrete tints of the static levels.
        type Built = Vec<[Option<Arc<StructureMesh>>; 3]>;
        let built: Built = std::thread::scope(|s| {
            let hs: Vec<_> = keys
                .chunks(chunk)
                .map(|ks| {
                    let w = &world;
                    s.spawn(move || {
                        crate::hitch::worker_started();
                        ks.iter().map(|k| [0u8, 1, 2].map(|lod| w.render_tile_shared(*k, lod))).collect::<Vec<_>>()
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let mut counts: BTreeMap<[u8; 3], u64> = BTreeMap::new();
        for levels in &built {
            for m in levels.iter().flatten() {
                for (i, &c) in m.classes.iter().enumerate() {
                    if c == StructureClass::Concrete as u8 {
                        *counts.entry(rgb_of(m.colors[i])).or_default() += 1;
                    }
                }
            }
        }
        let tints = Tints::from_counts(&counts);
        // every level converted (the near ones for their sizes only).
        let converted: Vec<[GpuTile; 3]> = std::thread::scope(|s| {
            let hs: Vec<_> = built
                .chunks(chunk)
                .map(|bs| {
                    let (tints, w) = (&tints, &world);
                    s.spawn(move || {
                        crate::hitch::worker_started();
                        bs.iter().map(|levels| [0u8, 1, 2].map(|lod| levels[lod as usize].as_deref().map(|m| to_gpu_reach(m, tints, lod, &|p| bore_depth(w, p), threshold_reach())).unwrap_or_default())).collect::<Vec<_>>()
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let tm = eo_street::STREET_TILE_M as f32;
        let mut tiles = vec![];
        let mut tile_of_key = HashMap::new();
        let mut statics = StructureStatic::default();
        let mut near_sizes = vec![];
        let mut totals = LevelTotals::default();
        for (t, (key, levels)) in keys.iter().zip(converted).enumerate() {
            let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
            for (lod, g) in levels.iter().enumerate() {
                let nv = g.vertices.len() as u32;
                let ni = g.indices() as u32;
                totals.triangles[lod] += ni as u64 / 3;
                totals.vertices[lod] += nv as u64;
                let mt = &mut totals.max_tile[lod];
                *mt = (mt.0.max(nv), mt.1.max(ni), mt.2.max(g.parts.len() as u32));
                if g.vertices.is_empty() {
                    continue;
                }
                for k in 0..3 {
                    mn[k] = mn[k].min(g.min[k]);
                    mx[k] = mx[k].max(g.max[k]);
                }
            }
            if mn[0] > mx[0] {
                // nothing drawn (a collision-only tile): its square at y 0.
                let (x0, z0) = (key.0 as f32 * tm, key.1 as f32 * tm);
                (mn, mx) = ([x0, 0.0, z0], [x0 + tm, 0.0, z0 + tm]);
            }
            let near = &levels[0];
            near_sizes.push((near.vertices.len() as u32, near.indices() as u32, near.parts.len() as u32));
            for (lod, role) in [(1usize, LOD_STREET_FAR), (2, LOD_STREET_COARSE)] {
                let g = &levels[lod];
                if g.parts.is_empty() {
                    continue;
                }
                let base_vertex = statics.vertices.len() as u32;
                statics.vertices.extend_from_slice(&g.vertices);
                for (part, idx) in &g.parts {
                    let first = statics.indices.len() as u32;
                    statics.indices.extend_from_slice(idx);
                    let portal = (part.portal() as usize).checked_sub(1).and_then(|k| g.portals.get(k)).copied();
                    statics.objects.push(StaticObject { tile: t as u32, part: *part, role, first, count: idx.len() as u32, base_vertex, min: g.min, max: g.max, portal });
                }
            }
            tile_of_key.insert(*key, t as u32);
            tiles.push(StructureTile { key: *key, min: mn, max: mx });
        }
        let mut set = StructureSet { world, tiles, tile_of_key, tints, radii, near_sizes, near_bound: (0, 0, 0), totals, prepare_ms: 0.0 };
        set.near_bound = near_bound(&set);
        set.prepare_ms = t0.elapsed().as_secs_f64() * 1e3;
        Self { set: Arc::new(set), statics }
    }

    /// The structures' LOD groups, one per tile in tile order (append them
    /// to the renderer's at `Placement::group_base`).
    pub fn groups(&self) -> Vec<GpuLodGroup> {
        (0..self.set.tiles.len()).map(|t| self.set.group(t)).collect()
    }

    /// The concrete materials, one per tint in tint order (append them to the
    /// renderer's at `Placement::concrete_material`).
    pub fn materials(&self) -> Vec<GpuMaterial> {
        self.set.tints.rgb.iter().map(|c| concrete_material(*c)).collect()
    }

    /// The arena it needs: the static levels and the near part.
    pub fn arena_needs(&self) -> (u32, u32, u32) {
        let (v, i, o) = self.set.near_bound;
        (self.statics.vertices.len() as u32 + v, self.statics.indices.len() as u32 + i, self.statics.objects.len() as u32 + o)
    }

    /// The near packs' part of `arena` (after the static levels).
    pub fn near_arena(&self, arena: StructuresArena) -> StructuresArena {
        let (sv, si, so) = (self.statics.vertices.len() as u32, self.statics.indices.len() as u32, self.statics.objects.len() as u32);
        StructuresArena {
            vertex_base: arena.vertex_base + sv,
            vertex_capacity: arena.vertex_capacity.saturating_sub(sv),
            index_base: arena.index_base + si,
            index_capacity: arena.index_capacity.saturating_sub(si),
            object_base: arena.object_base + so,
            object_capacity: arena.object_capacity.saturating_sub(so),
        }
    }

    /// The arena's object slots: the static objects placed, then empty slots
    /// for the near packs (`arena.object_capacity` in all).
    pub fn objects(&self, placement: Placement, arena: StructuresArena) -> Vec<GpuObject> {
        let mut out: Vec<GpuObject> = self
            .statics
            .objects
            .iter()
            .map(|s| {
                let mut o = s.part.object(s.min, s.max, arena.index_base + s.first, s.count, (arena.vertex_base + s.base_vertex) as i32, s.portal.as_slice());
                o.material = placement.material(s.part);
                o.lod_group = placement.group_base + s.tile;
                o.lod_role = s.role;
                o
            })
            .collect();
        out.truncate(arena.object_capacity as usize);
        out.resize(arena.object_capacity as usize, zeroed_object());
        out
    }

    /// Bytes of the static levels on the GPU (vertices, indices).
    pub fn static_bytes(&self) -> (u64, u64) {
        ((self.statics.vertices.len() * std::mem::size_of::<Vertex>()) as u64, self.statics.indices.len() as u64 * 4)
    }
}

/// The most near-pack vertices, indices and objects whose tiles are within
/// the drop radius of one camera point, over points every `BOUND_STEP_M`
/// within the 3 x 3 tiles round every tile, with `NEAR_MARGIN`.
fn near_bound(set: &StructureSet) -> (u32, u32, u32) {
    let tm = eo_street::STREET_TILE_M as f32;
    let reach = (set.radii.near_drop / tm).ceil() as i32 + 2;
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let (mut bv, mut bi, mut bo) = (0u64, 0u64, 0u64);
    let per = (tm / BOUND_STEP_M).round() as i32;
    for t in &set.tiles {
        for dz in -per..2 * per {
            for dx in -per..2 * per {
                let g = (t.key.0 * per + dx, t.key.1 * per + dz);
                if !seen.insert(g) {
                    continue;
                }
                let cam = Vec3::new((g.0 as f32 + 0.5) * BOUND_STEP_M, 0.0, (g.1 as f32 + 0.5) * BOUND_STEP_M);
                let (cx, cz) = eo_street::tile_key(cam.x as f64, cam.z as f64);
                let (mut v, mut i, mut o) = (0u64, 0u64, 0u64);
                for kz in cz - reach..=cz + reach {
                    for kx in cx - reach..=cx + reach {
                        let Some(&k) = set.tile_of_key.get(&(kx, kz)) else { continue };
                        if set.tile_distance(k as usize, cam) <= set.radii.near_drop + BOUND_STEP_M {
                            let s = set.near_sizes[k as usize];
                            v += s.0 as u64;
                            i += s.1 as u64;
                            o += s.2 as u64;
                        }
                    }
                }
                bv = bv.max(v);
                bi = bi.max(i);
                bo = bo.max(o);
            }
        }
    }
    let m = |x: u64| ((x as f32 * NEAR_MARGIN).ceil() as u64).min(u32::MAX as u64) as u32;
    (m(bv), m(bi), m(bo) + MAX_PARTS as u32)
}

// ---------------------------------------------------------------------------
// The renderer's side: the static levels written at attach.
// ---------------------------------------------------------------------------

pub struct StructuresGpu {
    set: Arc<StructureSet>,
    placement: Placement,
    arena: StructuresArena,
    written: u64,
}

impl StructuresGpu {
    /// Made by `Renderer::attach_structures`: the static levels go into the
    /// start of the arena (its objects are in the table from the start:
    /// `StructurePrepared::objects`).
    pub fn attach(ctx: &GpuContext, renderer: &crate::Renderer, prepared: &StructurePrepared, placement: Placement, arena: StructuresArena) -> Self {
        let vs = std::mem::size_of::<Vertex>() as u64;
        let (vb, ib) = prepared.static_bytes();
        // in pieces, so no single staging write is huge.
        for chunk in prepared.statics.vertices.chunks(1 << 16).enumerate() {
            let at = arena.vertex_base as u64 * vs + (chunk.0 as u64) * (1 << 16) * vs;
            renderer.write(&ctx.queue, Target::Vertex, at, bytemuck::cast_slice(chunk.1));
        }
        for chunk in prepared.statics.indices.chunks(1 << 18).enumerate() {
            let at = arena.index_base as u64 * 4 + (chunk.0 as u64) * (1 << 18) * 4;
            renderer.write(&ctx.queue, Target::Index, at, bytemuck::cast_slice(chunk.1));
        }
        Self { set: prepared.set.clone(), placement, arena, written: vb + ib }
    }

    /// The structures it draws.
    pub fn world(&self) -> &Arc<StructureWorld> {
        &self.set.world
    }

    /// How far into a tunnel's bore the eye is (stage A8, `--tunnels`; None
    /// outside every bore): the exposure adapts to the bore's lighting
    /// (sky.rs `SkyGpu::set_bore_depth`).
    pub fn eye_bore_depth(&self, eye: Vec3) -> Option<f64> {
        bore_depth(&self.set.world, [eye.x as f64, eye.y as f64, eye.z as f64]).map(|h| h.0)
    }

    /// Whether any bore is drawn (its lamps are lit round the clock).
    pub fn has_bores(&self) -> bool {
        !self.set.world.bores().is_empty()
    }

    pub fn set(&self) -> &Arc<StructureSet> {
        &self.set
    }

    pub fn placement(&self) -> Placement {
        self.placement
    }

    /// Its part of the shared arenas.
    pub fn arena(&self) -> StructuresArena {
        self.arena
    }

    /// Every frame in `Renderer::update`: nothing (the near packs stream
    /// through the app's upload queue, `StructuresRuntime`).
    pub fn update(&self, _queue: &wgpu::Queue, _cam: &crate::camera::Camera) {}

    /// GPU bytes it allocated beyond the shared arenas: none (the static
    /// levels fill the start of the arena, which is counted with the
    /// renderer's vertex and index buffers).
    pub fn bytes(&self) -> u64 {
        0
    }

    /// Bytes of the static levels it wrote into the arena at attach.
    pub fn written(&self) -> u64 {
        self.written
    }
}

// ---------------------------------------------------------------------------
// Runtime: near packs.
// ---------------------------------------------------------------------------

/// One tile's near level, cooked on a worker.
pub struct StructurePack {
    pub tile: u32,
    pub vbytes: Vec<u8>,
    pub parts: Vec<(Part, Vec<u32>)>,
    /// Its bore parts' portals (`GpuTile::portals`).
    pub portals: Vec<[f32; 4]>,
    pub nv: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub ms: f64,
    pub triangles: u64,
}

/// How far in from the nearer portal (x, y, z) lies inside a tunnel's bore,
/// and that portal (None outside every bore, and always without `--tunnels`).
pub fn bore_depth(world: &StructureWorld, p: [f64; 3]) -> Option<(f64, Option<[f64; 4]>)> {
    if world.bores().is_empty() {
        return None;
    }
    world.bore_at(p[0], p[1], p[2]).map(|h| (h.depth, h.portal))
}

pub fn cook_pack(set: &StructureSet, tile: u32) -> StructurePack {
    let t0 = Instant::now();
    let key = set.tiles[tile as usize].key;
    let g = set.world.render_tile_shared(key, 0).map(|m| to_gpu_reach(&m, &set.tints, 0, &|p| bore_depth(&set.world, p), threshold_reach())).unwrap_or_default();
    StructurePack {
        tile,
        nv: g.vertices.len() as u32,
        vbytes: bytemuck::cast_slice(&g.vertices).to_vec(),
        triangles: g.indices() as u64 / 3,
        parts: g.parts,
        portals: g.portals,
        min: g.min,
        max: g.max,
        ms: t0.elapsed().as_secs_f64() * 1e3,
    }
}

#[derive(Clone, Copy, Debug)]
struct Alloc {
    vertex: u32,
    vertices: u32,
    index: u32,
    indices: u32,
    object: u32,
    objects: u32,
}

impl Alloc {
    const NONE: Alloc = Alloc { vertex: 0, vertices: 0, index: 0, indices: 0, object: 0, objects: 0 };
}

enum Slot {
    Requested,
    Uploading(Alloc),
    Resident(Alloc),
    /// arrived while the arena was full; asked for again once it frees space.
    Deferred,
}

#[derive(Clone, Debug, Default)]
pub struct StructureStats {
    pub packs_cooked: usize,
    pub packs_uploaded: usize,
    pub packs_evicted: usize,
    pub packs_dropped_unneeded: usize,
    pub packs_skipped_arena_full: usize,
    pub worker_ms: f64,
    pub max_cook_ms: f64,
    pub resident: usize,
    pub resident_triangles: u64,
    pub resident_vertex_bytes: u64,
    pub resident_index_bytes: u64,
    pub group_bytes_written: u64,
}

/// The near packs, streamed like the street's (street.rs `StreetRuntime`):
/// requested within the near load radius of the camera (and, with view
/// detail, in the widened view or within its hold), cooked on worker
/// threads, written through the shared upload queue, their group's ready
/// bit set once on the GPU; dropped past the drop radius, at most a few a
/// frame. A pack that arrives while the arena is full is deferred until a
/// pack is evicted (its tile shows the far level meanwhile).
pub struct StructuresRuntime {
    pub set: Arc<StructureSet>,
    pub placement: Placement,
    /// The near part of the arena.
    pub arena: StructuresArena,
    pub radii: StreetRadii,
    vtx: RangeAlloc,
    idx: RangeAlloc,
    obj: RangeAlloc,
    slots: HashMap<u32, Slot>,
    pending: Arc<Mutex<Vec<u32>>>,
    outbox: crate::hitch::Outbox<u32>,
    camera: Arc<Mutex<Vec3>>,
    rx: Receiver<StructurePack>,
    stop: Arc<AtomicBool>,
    in_flight: Arc<AtomicUsize>,
    threads: Vec<std::thread::JoinHandle<()>>,
    pub stats: StructureStats,
    pub last_group_bytes: u64,
    pub updates: u64,
    view: Arc<Mutex<Option<crate::view_detail::ViewCone>>>,
    hold: crate::view_detail::ViewHold<u32>,
    pub near_out_of_view: usize,
    freed: bool,
}

impl StructuresRuntime {
    pub fn start(set: Arc<StructureSet>, placement: Placement, arena: StructuresArena, threads: usize, camera: Vec3) -> Self {
        let (tx, rx): (Sender<StructurePack>, Receiver<StructurePack>) = std::sync::mpsc::channel();
        let pending: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(vec![]));
        let cam = Arc::new(Mutex::new(camera));
        let stop = Arc::new(AtomicBool::new(false));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let view: Arc<Mutex<Option<crate::view_detail::ViewCone>>> = Arc::new(Mutex::new(None));
        let mut handles = vec![];
        for _ in 0..threads.max(1) {
            let (tx, pending, cam, stop, in_flight, set, view) = (tx.clone(), pending.clone(), cam.clone(), stop.clone(), in_flight.clone(), set.clone(), view.clone());
            handles.push(
                std::thread::Builder::new()
                    .name("eo-structures-near".into())
                    .spawn(move || {
                        crate::hitch::worker_started(); // below the render thread
                        while !stop.load(Ordering::Relaxed) {
                            let next = {
                                let c = *cam.lock().unwrap();
                                let v = *view.lock().unwrap();
                                let mut p = pending.lock().unwrap();
                                // in view first by screen importance, then nearest.
                                let best = p
                                    .iter()
                                    .enumerate()
                                    .map(|(k, &t)| {
                                        let (mn, mx) = set.tile_box(t as usize);
                                        (k, crate::view_detail::rank(v.as_ref(), mn, mx, set.tile_distance(t as usize, c)))
                                    })
                                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                                best.map(|(k, _)| {
                                    in_flight.fetch_add(1, Ordering::SeqCst);
                                    p.swap_remove(k)
                                })
                            };
                            let Some(tile) = next else {
                                std::thread::sleep(std::time::Duration::from_millis(4));
                                continue;
                            };
                            let pack = cook_pack(&set, tile);
                            in_flight.fetch_sub(1, Ordering::SeqCst);
                            if tx.send(pack).is_err() {
                                break;
                            }
                        }
                    })
                    .expect("spawn a structures near-pack worker"),
            );
        }
        let radii = set.radii;
        Self {
            vtx: RangeAlloc::new(arena.vertex_capacity),
            idx: RangeAlloc::new(arena.index_capacity),
            obj: RangeAlloc::new(arena.object_capacity),
            set,
            placement,
            arena,
            radii,
            slots: HashMap::new(),
            pending,
            outbox: Default::default(),
            camera: cam,
            rx,
            stop,
            in_flight,
            threads: handles,
            stats: StructureStats::default(),
            last_group_bytes: 0,
            updates: 0,
            view,
            hold: Default::default(),
            near_out_of_view: 0,
            freed: false,
        }
    }

    /// Whether the tile's near pack is resident (its group's ready bit).
    pub fn is_resident(&self, tile: u32) -> bool {
        matches!(self.slots.get(&tile), Some(Slot::Resident(_)))
    }

    pub fn begin_frame(&mut self) {
        self.last_group_bytes = 0;
    }

    pub fn update(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, uploads: &mut UploadQueue, cam: Vec3) {
        self.updates += 1;
        crate::hitch::try_set(&self.camera, cam); // never wait for a worker's lock
        let cone = renderer.view_cone();
        crate::hitch::try_set(&self.view, cone);
        self.near_out_of_view = 0;
        if self.freed {
            self.slots.retain(|_, s| !matches!(s, Slot::Deferred));
            self.freed = false;
        }
        let under = self.slots.values().any(|s| matches!(s, Slot::Deferred));
        let r = self.radii;
        let tm = eo_street::STREET_TILE_M;
        // a tile's bounds reach up to a station step and a deck's width past its square.
        let reach = (r.near_drop as f64 / tm).ceil() as i32 + 2;
        let (cx, cz) = eo_street::tile_key(cam.x as f64, cam.z as f64);
        let mut wanted: HashSet<u32> = HashSet::new();
        for tz in cz - reach..=cz + reach {
            for tx in cx - reach..=cx + reach {
                let Some(&t) = self.set.tile_of_key.get(&(tx, tz)) else { continue };
                let d = self.set.tile_distance(t as usize, cam);
                if d <= r.near_load || (self.slots.contains_key(&t) && d <= r.near_drop && !under) {
                    if let Some(c) = cone.as_ref() {
                        let (mn, mx) = self.set.tile_box(t as usize);
                        let held = self.hold.wanted(c, t, mn, mx);
                        if !held || (under && !c.sees_box(mn, mx)) {
                            self.near_out_of_view += 1;
                            continue;
                        }
                    }
                    wanted.insert(t);
                }
            }
        }
        if let Some(c) = cone.as_ref() {
            self.hold.prune(c);
        }
        let mut evictable = 0;
        let gone: Vec<u32> = self
            .slots
            .iter()
            .filter(|(k, _)| !wanted.contains(k))
            .filter(|(_, s)| {
                if matches!(s, Slot::Resident(_)) {
                    evictable += 1;
                    evictable <= crate::kit::EVICTIONS_PER_FRAME
                } else {
                    true
                }
            })
            .map(|(k, _)| *k)
            .collect();
        if !gone.is_empty() {
            let gone_set: HashSet<u32> = gone.iter().copied().collect();
            for k in &gone_set {
                self.outbox.cancel(*k);
            }
            uploads.cancel(|tag| !is_structures_tag(tag) || !gone_set.contains(&((tag & 0xffff_ffff) as u32)));
            for k in gone {
                match self.slots.remove(&k) {
                    Some(Slot::Resident(a)) => {
                        self.evict(renderer, queue, k, a);
                        self.stats.packs_evicted += 1;
                    }
                    Some(Slot::Uploading(a)) => {
                        self.free(a);
                        self.stats.packs_dropped_unneeded += 1;
                    }
                    _ => {}
                }
            }
        }
        for k in &wanted {
            if !self.slots.contains_key(k) {
                self.slots.insert(*k, Slot::Requested);
                self.outbox.request(*k);
            }
        }
        self.outbox.flush(&self.pending); // try_lock only
        while let Ok(mut pack) = self.rx.try_recv() {
            crate::hitch::count(crate::hitch::Event::Arrival, 1);
            self.stats.worker_ms += pack.ms;
            self.stats.max_cook_ms = self.stats.max_cook_ms.max(pack.ms);
            self.stats.packs_cooked += 1;
            if !matches!(self.slots.get(&pack.tile), Some(Slot::Requested)) {
                self.stats.packs_dropped_unneeded += 1;
                continue;
            }
            if pack.parts.is_empty() {
                // nothing at the near level here: resident at once (the level is drawn empty).
                self.slots.insert(pack.tile, Slot::Resident(Alloc::NONE));
                self.write_group(renderer, queue, pack.tile, true);
                self.stats.packs_uploaded += 1;
                continue;
            }
            match self.make_job(&mut pack) {
                Some((job, a)) => {
                    self.slots.insert(pack.tile, Slot::Uploading(a));
                    uploads.push(job);
                }
                None => {
                    self.slots.insert(pack.tile, Slot::Deferred);
                    self.stats.packs_skipped_arena_full += 1;
                }
            }
        }
    }

    fn free(&mut self, a: Alloc) {
        self.vtx.free(a.vertex, a.vertices);
        self.idx.free(a.index, a.indices);
        self.obj.free(a.object, a.objects);
        self.freed = true;
    }

    fn make_job(&mut self, p: &mut StructurePack) -> Option<(Job, Alloc)> {
        let ni = p.parts.iter().map(|q| q.1.len()).sum::<usize>() as u32;
        let objects_needed = p.parts.len() as u32;
        let v = self.vtx.alloc(p.nv)?;
        let Some(i) = self.idx.alloc(ni) else {
            self.vtx.free(v, p.nv);
            return None;
        };
        let Some(o) = self.obj.alloc(objects_needed) else {
            self.vtx.free(v, p.nv);
            self.idx.free(i, ni);
            return None;
        };
        let a = Alloc { vertex: v, vertices: p.nv, index: i, indices: ni, object: o, objects: objects_needed };
        let ar = self.arena;
        let group = self.placement.group_base + p.tile;
        let base_vertex = (ar.vertex_base + v) as i32;
        let mut objects = vec![];
        let mut idx: Vec<u32> = Vec::with_capacity(ni as usize);
        for (part, list) in &p.parts {
            let mut ob = part.object(p.min, p.max, ar.index_base + i + idx.len() as u32, list.len() as u32, base_vertex, &p.portals);
            ob.material = self.placement.material(*part);
            ob.lod_group = group;
            ob.lod_role = LOD_STREET_NEAR;
            objects.push(ob);
            idx.extend_from_slice(list);
        }
        let vs = std::mem::size_of::<Vertex>() as u64;
        let os = std::mem::size_of::<GpuObject>() as u64;
        let bulk = vec![
            Write { target: Target::Vertex, offset: (ar.vertex_base + v) as u64 * vs, data: Data::Bytes(std::mem::take(&mut p.vbytes)) },
            Write { target: Target::Index, offset: (ar.index_base + i) as u64 * 4, data: Data::Words(idx) },
        ];
        let finish = vec![Write { target: Target::Object, offset: (ar.object_base + o) as u64 * os, data: Data::Objects(objects) }];
        Some((Job::new(TAG_STRUCTURES | p.tile as u64, bulk, finish), a))
    }

    fn write_group(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, tile: u32, ready: bool) {
        // `ready` is the fifth word of the 32-byte group.
        let at = (self.placement.group_base as u64 + tile as u64) * std::mem::size_of::<GpuLodGroup>() as u64 + 16;
        renderer.write(queue, Target::Group, at, bytemuck::bytes_of(&(ready as u32)));
        self.last_group_bytes += 4;
        self.stats.group_bytes_written += 4;
    }

    fn evict(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, tile: u32, a: Alloc) {
        crate::hitch::count(crate::hitch::Event::Eviction, 1);
        self.write_group(renderer, queue, tile, false);
        if a.objects > 0 {
            let zero = vec![zeroed_object(); a.objects as usize];
            renderer.write(queue, Target::Object, (self.arena.object_base + a.object) as u64 * std::mem::size_of::<GpuObject>() as u64, bytemuck::cast_slice(&zero));
            self.last_group_bytes += (zero.len() * std::mem::size_of::<GpuObject>()) as u64;
        }
        self.account(a, false);
        self.free(a);
    }

    fn account(&mut self, a: Alloc, add: bool) {
        let tris = a.indices as u64 / 3;
        let vb = a.vertices as u64 * std::mem::size_of::<Vertex>() as u64;
        let ib = a.indices as u64 * 4;
        if add {
            self.stats.resident += 1;
            self.stats.resident_triangles += tris;
            self.stats.resident_vertex_bytes += vb;
            self.stats.resident_index_bytes += ib;
        } else {
            self.stats.resident = self.stats.resident.saturating_sub(1);
            self.stats.resident_triangles -= tris;
            self.stats.resident_vertex_bytes -= vb;
            self.stats.resident_index_bytes -= ib;
        }
    }

    pub fn after_pump(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, done: &[u64]) {
        for &tag in done {
            if !is_structures_tag(tag) {
                continue;
            }
            let tile = (tag & 0xffff_ffff) as u32;
            let Some(Slot::Uploading(a)) = self.slots.remove(&tile) else { continue };
            self.slots.insert(tile, Slot::Resident(a));
            self.write_group(renderer, queue, tile, true);
            self.account(a, true);
            self.stats.packs_uploaded += 1;
        }
    }

    pub fn settled(&self) -> bool {
        self.updates > 0
            && self.outbox.is_empty()
            && self.pending.lock().unwrap().is_empty()
            && self.in_flight.load(Ordering::SeqCst) == 0
            && self.slots.values().all(|s| matches!(s, Slot::Resident(_) | Slot::Deferred))
    }

    /// (vertices used, peak, indices used, peak, objects used, peak) of the near part.
    pub fn arena_use(&self) -> (u32, u32, u32, u32, u32, u32) {
        (self.vtx.used, self.vtx.peak, self.idx.used, self.idx.peak, self.obj.used, self.obj.peak)
    }

    /// Resident tiles, for tests: tile -> object range.
    pub fn resident(&self) -> Vec<(u32, (u32, u32))> {
        self.slots
            .iter()
            .filter_map(|(k, s)| if let Slot::Resident(a) = s { Some((*k, (self.arena.object_base + a.object, a.objects))) } else { None })
            .collect()
    }
}

impl Drop for StructuresRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_quad(class: StructureClass, rgb: [u8; 3], wear: u8, pts: [[f32; 3]; 4], uv: [[f32; 2]; 4], n: [f32; 3]) -> StructureMesh {
        let mut m = StructureMesh::default();
        for k in 0..4 {
            m.positions.push(pts[k]);
            m.normals.push(n);
            m.colors.push(rgb[0] as u32 | (rgb[1] as u32) << 8 | (rgb[2] as u32) << 16 | (wear as u32) << 24);
            m.classes.push(class as u8);
            m.uv.push(uv[k]);
        }
        m.indices = vec![0, 1, 2, 0, 2, 3];
        m
    }

    #[test]
    fn the_portal_meter_follows_the_illuminated_threshold_through_dusk() {
        let d = THRESHOLD_ZONE_M / 2.0;
        let day = threshold_over_interior();
        assert!(day > 1.0);
        assert_eq!(bore_zone_light(d, 0.0), day);
        assert_eq!(bore_zone_light(d, 1.0), 1.0);
        assert!((bore_zone_light(d, 0.5) - (day + 1.0) / 2.0).abs() < 1e-9);
        assert_eq!(bore_zone_light(threshold_reach() + 1.0, 0.0), 1.0);
    }

    #[test]
    fn concrete_coordinates_round_trip_and_interpolate() {
        for (u, v, w) in [(0.0, -320.0, 0u8), (12.34, 0.0, 17), (597.59, 5.25, 255), (655.35, 332.79, 128), (3.0, -12.5, 1), (1.0, 205.5, 99)] {
            let vert = encode_concrete([1.0, 2.0, 3.0], [0.0, -1.0, 0.0], u, v, w);
            let (du, dv, dw) = decode_concrete(&vert);
            assert!((du - u).abs() <= QUANT_M * 0.51, "u {u} -> {du}");
            assert!((dv - v).abs() <= QUANT_M * 0.51, "v {v} -> {dv}");
            assert_eq!(dw, w);
            // the normal survives in its three bytes.
            let n = eo_assets::glb::unpack_normal(vert.normal);
            assert!((n.y + 1.0).abs() < 0.01);
        }
        // the decoding is linear in the bytes: a byte-wise lerp decodes to the lerp.
        let (a, b) = (encode_concrete([0.0; 3], [0.0, 1.0, 0.0], 255.99, 0.5, 0), encode_concrete([0.0; 3], [0.0, 1.0, 0.0], 256.04, 2.62, 0));
        let byte = |x: &Vertex, s: u32| ((x.color >> s) & 255) as f64;
        let wb = |x: &Vertex| ((x.normal >> 24) as u8 as i8) as f64;
        let t = 0.3;
        let lerp = |p: f64, q: f64| p + (q - p) * t;
        let u = (lerp(byte(&a, 0), byte(&b, 0)) + 256.0 * lerp(byte(&a, 8), byte(&b, 8))) * QUANT_M;
        let v = (lerp(byte(&a, 16), byte(&b, 16)) + 256.0 * (lerp(wb(&a), wb(&b)) + 127.0)) * QUANT_M - V_BIAS_M;
        assert!((u - lerp(255.99, 256.04)).abs() < 1e-6, "{u}");
        assert!((v - lerp(0.5, 2.62)).abs() < 1e-6, "{v}");
    }

    #[test]
    fn a_soffit_quad_keeps_its_coordinates_and_a_wrap_gets_its_own_vertices() {
        let tints = Tints { rgb: vec![[176, 170, 159]] };
        // a soffit 4 m long across the period's wrap at 597.6 m of station.
        let pts = [[0.0, 10.0, 0.0], [4.0, 10.0, 0.0], [4.0, 10.0, 8.0], [0.0, 10.0, 8.0]];
        let uv = [[596.0, -4.0], [600.0, -4.0], [600.0, 4.0], [596.0, 4.0]];
        let m = mesh_quad(StructureClass::Concrete, [176, 170, 159], 80, pts, uv, [0.0, -1.0, 0.0]);
        let g = to_gpu(&m, &tints, 0);
        assert_eq!(g.parts.len(), 1);
        assert_eq!(g.parts[0].0, Part::Concrete(0));
        // both triangles cross the wrap: six vertices, each triangle's u continuous.
        assert_eq!(g.vertices.len(), 6);
        for tri in g.parts[0].1.chunks(3) {
            let us: Vec<f64> = tri.iter().map(|&i| decode_concrete(&g.vertices[i as usize]).0).collect();
            let span = us.iter().cloned().fold(f64::MIN, f64::max) - us.iter().cloned().fold(f64::MAX, f64::min);
            assert!(span <= 4.0 + 1e-6, "a triangle spans {span} m of u: {us:?}");
            for &i in tri {
                let (u, v, w) = decode_concrete(&g.vertices[i as usize]);
                assert!((0.0..655.35).contains(&u), "u {u}");
                assert!(v.abs() <= 4.0 + 1e-6);
                assert_eq!(w, 80);
            }
        }
        // away from the wrap the four corners are shared.
        let uv = [[10.0, -4.0], [14.0, -4.0], [14.0, 4.0], [10.0, 4.0]];
        let g = to_gpu(&mesh_quad(StructureClass::Concrete, [176, 170, 159], 80, pts, uv, [0.0, -1.0, 0.0]), &tints, 0);
        assert_eq!(g.vertices.len(), 4);
        let (u, v, _) = decode_concrete(&g.vertices[2]);
        assert!((u - 14.0).abs() < 0.006 && (v - 4.0).abs() < 0.006);
    }

    #[test]
    fn a_column_face_carries_its_height_in_v() {
        let tints = Tints { rgb: vec![[176, 170, 159]] };
        // a column's face: u is its height above the base, v the distance round it.
        let pts = [[0.0, 0.0, 0.0], [0.4, 0.0, 0.0], [0.4, 9.0, 0.0], [0.0, 9.0, 0.0]];
        let uv = [[0.0, 0.0], [0.0, 0.4], [9.0, 0.4], [9.0, 0.0]];
        let g = to_gpu(&mesh_quad(StructureClass::Concrete, [176, 170, 159], 40, pts, uv, [0.0, 0.0, -1.0]), &tints, 0);
        for (k, vert) in g.vertices.iter().enumerate() {
            let (u, v, _) = decode_concrete(vert);
            let y = vert.pos[1] as f64;
            assert!((v - (COLUMN_V_M + y)).abs() < 0.006, "vertex {k}: v {v}, height {y}");
            assert!(u <= 0.406, "vertex {k}: u {u}");
        }
        // a swept wall (u the station, along; v the height) is left as it is.
        let uv = [[30.0, -2.0], [30.4, -2.0], [30.4, 7.0], [30.0, 7.0]];
        let g = to_gpu(&mesh_quad(StructureClass::Concrete, [176, 170, 159], 40, pts, uv, [0.0, 0.0, -1.0]), &tints, 0);
        let (u, v, _) = decode_concrete(&g.vertices[2]);
        assert!((u - 30.4).abs() < 0.006 && (v - 7.0).abs() < 0.006);
    }

    #[test]
    fn the_deck_is_street_asphalt_with_its_edge_distance_and_paint_is_paint() {
        let tints = Tints { rgb: vec![[176, 170, 159]] };
        // a deck section of five points (-6, -5.4, 0, 5.4, 6) at stations 0 and 4.
        let mut m = StructureMesh::default();
        for s in [0.0f32, 4.0] {
            for u in [-6.0f32, -5.4, 0.0, 5.4, 6.0] {
                m.positions.push([s, 8.0, u]);
                m.normals.push([0.0, 1.0, 0.0]);
                m.colors.push(0xff40_4142);
                m.classes.push(StructureClass::DeckSurface as u8);
                m.uv.push([s, u]);
            }
        }
        for j in 0..4u32 {
            m.indices.extend_from_slice(&[j, j + 1, j + 6, j, j + 6, j + 5]);
        }
        let g = to_gpu(&m, &tints, 0);
        assert_eq!(g.parts.len(), 1);
        assert_eq!(g.parts[0].0, Part::Deck);
        assert_eq!(g.parts[0].0.kind(), ObjectKind::StructureDeck);
        for v in &g.vertices {
            assert_eq!(v.color >> 24, sc::ASPHALT as u32);
            assert_eq!(v.color & 0x00ff_ffff, 0x0040_4142);
            let kerb = ((v.normal >> 24) as u8 as i8) as f32 / 127.0 * 6.35;
            let want = 6.0 - v.pos[2].abs();
            assert!((kerb.min(6.35) - want.min(6.35)).abs() < 0.06, "edge distance {kerb} for offset {}", v.pos[2]);
        }
        // the far level has no edge distance.
        let g = to_gpu(&m, &tints, 1);
        assert!(g.vertices.iter().all(|v| (v.normal >> 24) as u8 as i8 == -127));
        let p = mesh_quad(StructureClass::Paint, [240, 240, 236], 255, [[0.0; 3], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]], [[0.0; 2]; 4], [0.0, 1.0, 0.0]);
        let g = to_gpu(&p, &tints, 0);
        assert_eq!(g.parts[0].0, Part::Plain);
        assert!(g.vertices.iter().all(|v| v.color >> 24 == sc::PAINT as u32 && (v.normal >> 24) as u8 as i8 == -127));
    }

    #[test]
    fn tints_cluster_by_chromaticity_and_jittered_bents_join_their_chain() {
        let mut counts = BTreeMap::new();
        counts.insert([176u8, 170, 159], 1000u64);
        counts.insert([186, 172, 136], 300);
        counts.insert([200, 197, 190], 200);
        // a bent's tint, 3 % brighter and 3 % darker.
        counts.insert([181, 175, 164], 20);
        counts.insert([171, 165, 154], 20);
        let t = Tints::from_counts(&counts);
        assert_eq!(t.rgb, vec![[176, 170, 159], [186, 172, 136], [200, 197, 190]]);
        assert_eq!(t.index([181, 175, 164]), 0);
        assert_eq!(t.index([171, 165, 154]), 0);
        assert_eq!(t.index([186, 172, 136]), 1);
        assert_eq!(t.index([200, 197, 190]), 2);
        assert_eq!(finish_of([176, 170, 159]), FINISH_STANDARD);
        assert_eq!(finish_of([186, 172, 136]), FINISH_BANDED);
        assert_eq!(finish_of([200, 197, 190]), FINISH_CLEAN);
        // light warm grey: new grey concrete reflects about 0.40 (reference sheet 2.1).
        let m = concrete_material([176, 170, 159]);
        let luma = 0.2126 * m.base[0] + 0.7152 * m.base[1] + 0.0722 * m.base[2];
        assert!((0.36..0.44).contains(&luma), "{luma}");
        assert!(m.base[0] > m.base[2], "warm, not blue-grey");
    }

    #[test]
    fn tags_are_told_from_the_other_streams() {
        assert!(is_structures_tag(TAG_STRUCTURES | 1234));
        for other in [crate::street::TAG_STREET | 5, crate::kit::TAG_KIT | (5 << 8), 1 << 57, 1 << 58, 1 << 60, 1 << 61, (1 << 58) | (1 << 56)] {
            assert!(!is_structures_tag(other), "{other:#x}");
        }
    }

    /// Stage A8: a triangle inside a bore goes to the bore's part of its
    /// class, with the portal whose daylight reaches it (within
    /// `PORTAL_DAY_M`; none deeper); the luminaire's metal to `Luminaire`; the
    /// portal's face (less than `BORE_FACE_M` in) and everything outside stay
    /// where they were; the portal rides the object's terrain-only words; and
    /// with no bore the tile is `to_gpu`'s, part for part and byte for byte.
    #[test]
    fn a_bores_triangles_are_its_own_parts_with_their_portal() {
        let quad = |class: StructureClass, rgb: [u8; 3], x0: f32| {
            mesh_quad(class, rgb, 40, [[x0, 0.0, -1.0], [x0 + 1.0, 0.0, -1.0], [x0 + 1.0, 0.0, 1.0], [x0, 0.0, 1.0]], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], [0.0, 1.0, 0.0])
        };
        let join = |ms: Vec<StructureMesh>| {
            let mut m = StructureMesh::default();
            for q in ms {
                let base = m.positions.len() as u32;
                m.positions.extend(q.positions);
                m.normals.extend(q.normals);
                m.colors.extend(q.colors);
                m.classes.extend(q.classes);
                m.uv.extend(q.uv);
                m.indices.extend(q.indices.iter().map(|i| i + base));
            }
            m
        };
        let tile = eo_structures::TUNNEL_TILE_TINT;
        let lamp = eo_structures::TUNNEL_LUMINAIRE_RGB;
        // x is the depth into the bore from its portal at x = 0 (outside for x < 0).
        let m = join(vec![
            quad(StructureClass::DeckSurface, [60, 60, 62], -20.0),
            quad(StructureClass::DeckSurface, [60, 60, 62], 1.0),
            quad(StructureClass::DeckSurface, [60, 60, 62], 7.0),
            quad(StructureClass::DeckSurface, [60, 60, 62], 40.0),
            quad(StructureClass::Concrete, tile, 4.0),
            quad(StructureClass::Concrete, [176, 170, 159], -0.6),
            quad(StructureClass::Paint, [240, 240, 240], 30.0),
            quad(StructureClass::Metal, lamp, 1.0),
            quad(StructureClass::Metal, [90, 90, 90], 30.0),
        ]);
        let mut counts = BTreeMap::new();
        counts.insert([176u8, 170, 159], 10u64);
        counts.insert(tile, 10);
        let tints = Tints::from_counts(&counts);
        let (grey, tiled) = (tints.index([176, 170, 159]), tints.index(tile));
        assert_ne!(grey, tiled, "the lining is a tint of its own");
        let portal = [0.0, 0.0, 1.0, 0.0];
        let bore = |p: [f64; 3]| (p[0] >= -0.2).then_some((p[0].max(0.0), Some(portal)));
        let g = to_gpu_with(&m, &tints, 0, &bore);
        let parts: Vec<Part> = g.parts.iter().map(|p| p.0).collect();
        assert_eq!(parts, vec![Part::Deck, Part::Concrete(grey), Part::BoreDeck(0), Part::BoreDeck(1), Part::BorePlain(0), Part::BoreConcrete(tiled, 1), Part::Luminaire], "{:?}", g.parts);
        assert_eq!(g.portals, vec![[0.0f32, 0.0, 1.0, 0.0]]);
        // the object words: the bore's bit (and the luminaire's), the portal in the terrain-only words.
        let min = [-30.0f32, -1.0, -1.0];
        let ob = Part::BoreDeck(1).object(min, [50.0, 1.0, 1.0], 0, 3, 0, &g.portals);
        assert_eq!(ob.kind, ObjectKind::StructureDeck as u32 | BORE_BIT);
        assert_eq!(ob.object_kind(), ObjectKind::StructureDeck as u32);
        let back = decode_portal(min, (ob.far_first_index, ob.far_index_count)).expect("a portal");
        assert!(back.iter().zip(portal).all(|(a, b)| (*a as f64 - b).abs() < 0.01), "{back:?}");
        let deep = Part::BoreDeck(0).object(min, [50.0, 1.0, 1.0], 0, 3, 0, &g.portals);
        assert_eq!((deep.far_first_index, deep.far_index_count), (0, 0));
        assert_eq!(Part::BoreConcrete(tiled, 0).object_kind() & 255, ObjectKind::Structure as u32);
        assert_eq!(Part::Luminaire.object_kind(), ObjectKind::StructureDeck as u32 | BORE_BIT | LUMINAIRE_BIT);
        assert_eq!(Part::Deck.object_kind(), ObjectKind::StructureDeck as u32);
        assert_eq!(Part::Concrete(1).object_kind(), ObjectKind::Structure as u32);
        assert_eq!(BORE_BIT & crate::scene::BLEND_BIT, 0);
        // a portal past the words' reach is none (its daylight cannot reach the object).
        assert_eq!(encode_portal([0.0; 3], [900.0, 0.0, 1.0, 0.0]), (0, 0));
        let d = decode_portal([5.0, 0.0, -3.0], encode_portal([5.0, 0.0, -3.0], [12.5, -40.25, 0.6, -0.8])).unwrap();
        assert!((d[0] - 12.5).abs() < 0.01 && (d[1] + 40.25).abs() < 0.01 && (d[2] - 0.6).abs() < 1e-3 && (d[3] + 0.8).abs() < 1e-3, "{d:?}");
        let pl = Placement { group_base: 0, street_material: 7, concrete_material: 20 };
        assert_eq!((pl.material(Part::BoreDeck(2)), pl.material(Part::BorePlain(1)), pl.material(Part::Luminaire), pl.material(Part::BoreConcrete(1, 3))), (7, MATERIAL_VERTEX_COLOUR, MATERIAL_VERTEX_COLOUR, 21));
        // no bore: to_gpu's tile, part for part and byte for byte.
        let a = to_gpu(&m, &tints, 0);
        let b = to_gpu_with(&m, &tints, 0, &|_| None);
        assert_eq!(a.parts, b.parts);
        assert!(a.portals.is_empty());
        assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&a.vertices), bytemuck::cast_slice::<Vertex, u8>(&b.vertices));
        assert!(a.parts.iter().all(|p| matches!(p.0, Part::Deck | Part::Plain | Part::Concrete(_))));
    }

    /// Stage A8: the bore's lining is glazed tile (its own finish, glossy),
    /// the decks' tints keep theirs; the bore's exposure gain is the meter's
    /// at the portal and, inside, adapts to the bore's key: up by day, down by
    /// night; the interior is seven stops under the day's ground by day and
    /// the street's level by night; the shader gets the constants.
    #[test]
    fn the_bores_lining_is_glossy_tile_and_its_exposure_adapts_to_the_bore() {
        let tile = eo_structures::TUNNEL_TILE_TINT;
        assert_eq!(finish_of(tile), FINISH_TILE);
        let m = concrete_material(tile);
        assert_eq!(m.params[2] as u32, FINISH_TILE);
        assert!(m.params[0] < 0.3 && m.params[0] == TILE_ROUGHNESS);
        let luma = 0.2126 * m.base[0] + 0.7152 * m.base[1] + 0.0722 * m.base[2];
        assert!((0.5..0.7).contains(&luma) && m.base[0] > m.base[1] && m.base[1] > m.base[2], "cream-yellow, light: {:?} {luma}", m.base);
        for c in [[176u8, 170, 159], [186, 172, 136], [200, 197, 190]] {
            assert_ne!(finish_of(c), FINISH_TILE);
            assert_eq!(concrete_material(c).params[0], CONCRETE_ROUGHNESS);
        }
        assert!((TILE_M * 5520.0 - U_PERIOD_M).abs() < 1e-9 && (TILE_M - 0.108).abs() < 0.001);
        let w = eo_sky::weather::preset(eo_sky::Kind::Clear);
        for (hour, day) in [(15.0, true), (23.5, false)] {
            let mut f = eo_look::LookFrame::new(hour);
            let terms = eo_sky::apply(&mut f, &eo_sky::Conditions { hour, weather: w, moon_day: 0.0 });
            let meter = 0.8;
            let at_portal = bore_exposure_gain(&f, &terms, meter, 0.0);
            let deep = bore_exposure_gain(&f, &terms, meter, 200.0);
            let half = bore_exposure_gain(&f, &terms, meter, BORE_ADAPT_M / 2.0);
            assert!((at_portal - meter).abs() < 1e-9, "{hour}: {at_portal}");
            let e = bore_interior_units(&f, &terms);
            // adapted, the interior shows alike at any hour (its key).
            assert!((deep * terms.exposure_target * e - bore_key()).abs() < 1e-6 * bore_key(), "{hour}: {deep}");
            if day {
                // seven stops under the day's ground: the camera opens up by about the seven, less its 1.5 short.
                assert!((e / day_ground(&f, &terms) - (-TUNNEL_DAY_STOPS).exp2()).abs() < 1e-9, "{hour}: {e}");
                assert!(deep > 20.0 && deep < 100.0 && half > meter && half < deep, "{hour}: {at_portal} {half} {deep}");
            } else {
                // the approach road's level on the lamps' scale: brighter than the unlit street the night solve exposes, so down.
                assert!((e - eo_sky::photometry::LAMP_POOL_PEAK_LUX * TUNNEL_POOL_OVERLAP * terms.lamp_units_per_lux).abs() < 1e-9, "{hour}: {e}");
                assert!(deep < 1.0 && deep > 1.0 / BORE_EXPOSURE_MAX, "{hour}: {deep}");
            }
        }
        let src = wgsl();
        for k in ["SX_FINISH_TILE", "SX_TILE_M", "SX_BORE_BIT", "SX_LUMINAIRE_BIT", "SX_PORTAL_VALID", "SX_PORTAL_R2", "SX_BORE_BOUNCE", "SX_LUMINAIRE_OVER", "SX_BORE_REACH_M", "SX_LAMP_RGB", "fn sx_bore_light", "fn sx_tile"] {
            assert!(src.contains(k), "{k}");
        }
    }

    /// Stage A9: a concrete part carries its structure's era in its key (the
    /// tint index under it, its material the tint's) and its object kind
    /// (`ERA_SHIFT`, read by structures.wgsl `sx_era_k`); an old structure's
    /// part is as before, part, material and kind; a mesh with no era is old.
    #[test]
    fn a_concrete_parts_era_rides_its_key_and_its_object_kind() {
        let tints = Tints { rgb: vec![[176, 170, 159], [200, 197, 190]] };
        for (tint, era) in [(0u8, 0u8), (1, 0), (0, 1), (1, 2)] {
            let k = concrete_key(tint, era);
            assert_eq!((tint_of_key(k), era_of_key(k)), (tint, era));
            let pl = Placement { group_base: 0, street_material: 3, concrete_material: 10 };
            assert_eq!(pl.material(Part::Concrete(k)), 10 + tint as u32);
            assert_eq!(pl.material(Part::BoreConcrete(k, 1)), 10 + tint as u32);
            assert_eq!(Part::Concrete(k).object_kind() & 255, ObjectKind::Structure as u32);
            assert_eq!((Part::Concrete(k).object_kind() >> ERA_SHIFT) & 3, era as u32);
            assert_eq!((Part::BoreConcrete(k, 1).object_kind() >> ERA_SHIFT) & 3, era as u32);
            assert_ne!(Part::BoreConcrete(k, 1).object_kind() & BORE_BIT, 0);
        }
        assert_eq!(concrete_key(1, 0), 1, "an old structure's key is its tint index");
        assert_eq!(Part::Concrete(1).object_kind(), ObjectKind::Structure as u32);
        assert_eq!(((1u32 << ERA_SHIFT) * 3) & (BORE_BIT | LUMINAIRE_BIT | crate::scene::BLEND_BIT | 255), 0);
        // a quad of a new structure's concrete, and the same with no era recorded.
        let pts = [[0.0, 10.0, 0.0], [4.0, 10.0, 0.0], [4.0, 10.0, 8.0], [0.0, 10.0, 8.0]];
        let uv = [[10.0, -4.0], [14.0, -4.0], [14.0, 4.0], [10.0, 4.0]];
        let mut m = mesh_quad(StructureClass::Concrete, [200, 197, 190], 80, pts, uv, [0.0, -1.0, 0.0]);
        let old = to_gpu(&m, &tints, 0);
        assert_eq!(old.parts[0].0, Part::Concrete(1));
        m.eras = vec![eo_structures::Era::New as u8; 4];
        let new = to_gpu(&m, &tints, 0);
        assert_eq!(new.parts[0].0, Part::Concrete(concrete_key(1, 2)));
        assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&old.vertices), bytemuck::cast_slice::<Vertex, u8>(&new.vertices), "the vertices are the same");
    }

    /// Stage A9: the threshold zone (CIE 88 via reference sheet 3.5): its
    /// level the threshold's over its first half, 0.4 of it at its end, the
    /// transition curve continuous from there down to the interior at
    /// `threshold_reach`, where a bore's triangles stop taking their portal;
    /// the portal encoding reaches that far from a tile; the shader gets it.
    #[test]
    fn the_threshold_zone_lights_the_first_stretch_in_and_falls_to_the_interior() {
        let z = THRESHOLD_ZONE_M;
        assert_eq!(threshold_share(0.0), 1.0);
        assert_eq!(threshold_share(z / 2.0), 1.0);
        assert!((threshold_share(z) - 0.4).abs() < 1e-9);
        let r = threshold_over_interior();
        assert!(r > 4.0 && r < 10.0, "L_th / L_int by day {r}");
        let reach = threshold_reach();
        assert!(reach >= z && reach >= PORTAL_DAY_M && reach < 150.0, "{reach}");
        if let Some(v) = threshold_speed() {
            // CVC 22352(b)(1): 25 mph.
            assert!((v - 11.18).abs() < 0.01, "{v}");
            assert!((threshold_share(z + 1e-6) - 0.4).abs() < 0.01, "the transition starts at the threshold's 0.4");
            assert!((r * threshold_share(reach) - 1.0).abs() < 1e-6, "at its reach the zone is at the interior's level");
            let mut last = 1.0;
            for d in 0..200 {
                let s = threshold_share(d as f64);
                assert!(s <= last + 1e-12, "falls monotonically");
                last = s;
            }
        }
        // a portal `reach` past a tile's square still encodes from its box.
        let min = [0.0f32, 0.0, 0.0];
        for p in [[-(reach as f32) - 1.0, 0.0], [128.0 + reach as f32, 128.0 + reach as f32]] {
            let w = encode_portal(min, [p[0], p[1], 1.0, 0.0]);
            let d = decode_portal(min, w).expect("in reach");
            assert!((d[0] - p[0]).abs() < 0.02 && (d[1] - p[1]).abs() < 0.02, "{d:?}");
        }
        let src = wgsl();
        for k in ["SX_ERA_SHIFT", "SX_TH_OVER", "SX_TH_ZONE_M", "SX_TH_SPEED", "SX_WX_SOIL", "fn sx_weather", "fn sx_tile_weather", "fn sx_threshold"] {
            assert!(src.contains(k), "{k}");
        }
    }
}
