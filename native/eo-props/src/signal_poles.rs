//! Signal poles v1 (eo-app `--no-signal-poles` keeps the poles and mast arms built in code):
//! `scripts/assets/build-signal-poles-v1.py` models the San Francisco signal poles, mast arms and
//! the hardware that joins the heads to them in Blender (Caltrans RSP ES-7C's base plate, anchor
//! bolts and arm connection; the game's signal-hardware constants), baked and weathered into
//! three atlases, in `public/assets/signal-poles/`. Every file's digest is in the provenance
//! beside them and checked when the renderer reads it.
//!
//! Every part comes in two finishes ([`FINISHES`]): the signal paint, chalked and chipped, and
//! bare galvanising gone to its patina; a pole and everything on it take the finish of the spot
//! it stands on ([`finish_at`]; `variants` off, every pole painted).
//!
//! The props stay as [`crate::signals`] plans them; [`hardware`] reads a tile's signal props and
//! says what draws in their place: each [`Model::SignalPole`] the mast pole, pedestal or
//! pedestrian post model (its handhole away from its first arm), each [`Model::MastArm`] the arm
//! tube (scaled to the arm's length as before), and the pieces that are not stretched added: the
//! arm's root (pole plate, flange, cap screws) and its end cap, a hanger for each head on an arm,
//! a pair of pipe brackets for each head on a pole, a clamshell for each pedestrian head (and two
//! pipe arms from it when the head stands off the pole).

use crate::data::StreetProp;
use crate::{Model, Prop};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::path::Path;

/// The provenance (the digests every file is checked against).
pub const PROVENANCE: &str = "public/assets/signal-poles/signal-poles-v1.provenance.json";

/// The parts, in the renderer's model order (eo-render `props_lib2::SIGNAL_POLE_BASE +
/// finish * PARTS.len() + part`, [`Piece::model`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Part {
    MastPole,
    PedestalPole,
    PedPost,
    ArmTube,
    ArmRoot,
    ArmTip,
    MastMount0,
    MastMount1,
    MastMount2,
    SideMountPipe,
    SideMountMast,
    FrontMountPipe,
    FrontMountMast,
    PedMountPipe,
    PedMountMast,
    PedReach,
    // round 2: the rising arms (one per `signals::ArmClass` row, `--no-arm-rise`), the Street
    // Name sign's brackets (`--no-arm-name-signs`), the push buttons with their signs
    // (`--no-push-buttons`), the arrow along the part's +Z (P) or -Z (N): built round a pedestal
    // pipe, set out to a mast pole's face (`Dims::button_mast_offset_m`).
    ArmRise0,
    ArmRise1,
    ArmRise2,
    ArmRise3,
    ArmRise4,
    ArmRise5,
    ArmRise6,
    SignBracket,
    ButtonPipeP,
    ButtonPipeN,
    // round 3: the mast pole cut at its shaft (`--no-pole-cut`): its foot (the base plate,
    // anchor bolts and handhole) and its top (the cap) at their true size, the shaft between
    // drawn at the length the pole needs ([`Dims::mast_cut_m`]).
    MastFoot,
    MastMid,
    MastTop,
    // round 4: the level arms at ES-7E's diameter for their length (`--no-arm-diameter`), one
    // per `signals::ArmClass` row: the rising arm's tube with no rise ([`ARM_LEVEL_PARTS`]).
    ArmLevel0,
    ArmLevel1,
    ArmLevel2,
    ArmLevel3,
    ArmLevel4,
    ArmLevel5,
    ArmLevel6,
    // round 5: the corner hardware. The controller cabinet on its pad (the city's MSF60,
    // `--no-cabinet-model`: the code-built box), the No. 5 traffic pull box beside a signal pole
    // (`--no-pull-boxes`), the stiffener channel across an arm sign's back, unit length along +X
    // (`--no-sign-channels`); one atlas each family, the same files in both finishes.
    Cabinet,
    PullBox,
    SignChannel,
}

/// The parts drawn the same in every finish (zinc and cast hardware): the provenance lists the
/// same files for each finish.
pub const ONE_FINISH: [Part; 6] = [Part::SignBracket, Part::ButtonPipeP, Part::ButtonPipeN, Part::Cabinet, Part::PullBox, Part::SignChannel];

/// The rising arms' parts, one per `signals::ArmClass` row.
pub const ARM_RISE_PARTS: [Part; 7] = [Part::ArmRise0, Part::ArmRise1, Part::ArmRise2, Part::ArmRise3, Part::ArmRise4, Part::ArmRise5, Part::ArmRise6];
/// The level arms' parts at their class's diameter, one per `signals::ArmClass` row.
pub const ARM_LEVEL_PARTS: [Part; 7] = [Part::ArmLevel0, Part::ArmLevel1, Part::ArmLevel2, Part::ArmLevel3, Part::ArmLevel4, Part::ArmLevel5, Part::ArmLevel6];
/// The hangers by the arm radius they are built round (`Dims::mount_arm_radii_m`, same order;
/// on a rising arm thicker than the first, the first: its strap hidden in the arm).
pub const MAST_MOUNTS: [Part; 3] = [Part::MastMount0, Part::MastMount1, Part::MastMount2];

/// Every part and its name in the provenance, in [`Part`] order.
pub const PARTS: [(Part, &str); 39] = [
    (Part::MastPole, "mastPole"),
    (Part::PedestalPole, "pedestalPole"),
    (Part::PedPost, "pedPost"),
    (Part::ArmTube, "armTube"),
    (Part::ArmRoot, "armRoot"),
    (Part::ArmTip, "armTip"),
    (Part::MastMount0, "mastMount0"),
    (Part::MastMount1, "mastMount1"),
    (Part::MastMount2, "mastMount2"),
    (Part::SideMountPipe, "sideMountPipe"),
    (Part::SideMountMast, "sideMountMast"),
    (Part::FrontMountPipe, "frontMountPipe"),
    (Part::FrontMountMast, "frontMountMast"),
    (Part::PedMountPipe, "pedMountPipe"),
    (Part::PedMountMast, "pedMountMast"),
    (Part::PedReach, "pedReach"),
    (Part::ArmRise0, "armRise0"),
    (Part::ArmRise1, "armRise1"),
    (Part::ArmRise2, "armRise2"),
    (Part::ArmRise3, "armRise3"),
    (Part::ArmRise4, "armRise4"),
    (Part::ArmRise5, "armRise5"),
    (Part::ArmRise6, "armRise6"),
    (Part::SignBracket, "signBracket"),
    (Part::ButtonPipeP, "pushButtonPipeP"),
    (Part::ButtonPipeN, "pushButtonPipeN"),
    (Part::MastFoot, "mastFoot"),
    (Part::MastMid, "mastMid"),
    (Part::MastTop, "mastTop"),
    (Part::ArmLevel0, "armLevel0"),
    (Part::ArmLevel1, "armLevel1"),
    (Part::ArmLevel2, "armLevel2"),
    (Part::ArmLevel3, "armLevel3"),
    (Part::ArmLevel4, "armLevel4"),
    (Part::ArmLevel5, "armLevel5"),
    (Part::ArmLevel6, "armLevel6"),
    (Part::Cabinet, "cabinet"),
    (Part::PullBox, "pullBox"),
    (Part::SignChannel, "signChannel"),
];

/// The arm Street Name signs' sheeting roughness (eo-app `--no-name-sign-matte` off; the other
/// plates' is the renderer's own): aged sheeting under a mast arm reads matte at street distance,
/// not glossy (a modelling value, the owner's weathered look; no measurement was read). Carried
/// to props_draw.wgsl in the faces' colour alpha (1: the plates' own).
pub const NAME_SIGN_ROUGHNESS: f32 = 0.65;

/// The finishes, in the renderer's model order: the provenance's `finishes` keys.
pub const FINISHES: [&str; 2] = ["painted", "galvanised"];

/// The share of spots whose poles are galvanised (a modelling value: the reference photographs
/// show both finishes about equally; no survey of the city's poles was read).
pub const GALVANISED_SHARE: f64 = 0.5;
/// The size of the cells a finish is drawn by, so the poles of one junction mostly agree (m).
pub const FINISH_CELL_M: f64 = 48.0;

/// The finish (index into [`FINISHES`]) of a pole standing at (x, z).
pub fn finish_at(x: f64, z: f64) -> u8 {
    let (cx, cz) = ((x / FINISH_CELL_M).floor() as i64, (z / FINISH_CELL_M).floor() as i64);
    let mut h = (cx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (cz as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    u8::from(((h >> 11) as f64 / (1u64 << 53) as f64) < GALVANISED_SHARE)
}

/// A rising arm's part (`ArmRise{k}`): built at its class's length (instances scale X to the
/// arm's), its axis at `class.height_at(t)` over the root, its radius from `root_radius_m` at the
/// pole to `tip_radius_m` at its end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmDims {
    pub class: crate::signals::ArmClass,
    pub root_radius_m: f64,
    pub tip_radius_m: f64,
}

impl ArmDims {
    /// The arm's radius at the share `t` of its length.
    pub fn radius_at(&self, t: f64) -> f64 {
        self.root_radius_m + (self.tip_radius_m - self.root_radius_m) * t.clamp(0.0, 1.0)
    }
}

/// The dimensions the parts are built at (the provenance's `dims`).
#[derive(Clone, Debug, PartialEq)]
pub struct Dims {
    pub mast_pole_height_m: f64,
    pub pedestal_height_m: f64,
    pub post_height_m: f64,
    /// The arm tube's radius at the pole and at its end (the tube is drawn scaled to the arm's length).
    pub arm_root_radius_m: f64,
    pub arm_tip_radius_m: f64,
    /// The arm radius each [`MAST_MOUNTS`] strap is built round.
    pub mount_arm_radii_m: [f64; 3],
    /// How far from the pole axis the side and front brackets reach (the head's centre).
    pub side_reach_m: f64,
    pub front_reach_m: f64,
    /// How far the pedestrian head's housing reaches behind its face (its origin).
    pub ped_back_m: f64,
    /// The rising arms, one per `signals::ArmClass` row ([`ARM_RISE_PARTS`]).
    pub arms: Vec<ArmDims>,
    /// How far a push button on a mast pole stands out past one on a pedestal pipe (the mast's
    /// radius at the button's height over the pipe's).
    pub button_mast_offset_m: f64,
    /// How far from the pole's axis the arm's flange plate stands (`ArmRoot`): a rising arm's
    /// root plate is centred on the arm there.
    pub arm_flange_x_m: f64,
    /// Where the mast pole is cut (m over its foot): [`Part::MastFoot`] below the first,
    /// [`Part::MastTop`] above the second, [`Part::MastMid`] the shaft between.
    pub mast_cut_m: [f64; 2],
    /// The controller cabinet ([`Part::Cabinet`], McCain MSF60): width (local X), depth (local Z,
    /// its doors on +Z), height over its pad; and the pad's height over the sidewalk and how far
    /// it stands out round the cabinet (m).
    pub cabinet_m: [f64; 3],
    pub cabinet_pad_m: [f64; 2],
    /// The pull box's outside ([`Part::PullBox`], No. 5): length (local X), width (local Z), m.
    pub pull_box_m: [f64; 2],
    /// The sign channel ([`Part::SignChannel`]): its web's height and how far it stands out of
    /// the sign's back, m.
    pub sign_channel_m: [f64; 2],
}

pub struct SignalPoles {
    /// Per finish ([`FINISHES`]) and part ([`PARTS`]), its three levels: index
    /// `finish * PARTS.len() + part`.
    pub parts: Vec<[StreetProp; 3]>,
    pub dims: Dims,
}

fn file_of(what: &str, a: &Value) -> Result<StreetProp> {
    let s = |k: &str| a[k].as_str().map(str::to_string).ok_or_else(|| anyhow!("{PROVENANCE}: {what} has no {k}"));
    Ok(StreetProp {
        path: s("path")?,
        sha256: s("sha256")?,
        bytes: a["bytes"].as_u64().ok_or_else(|| anyhow!("{PROVENANCE}: {what} has no bytes"))?,
        triangles: a["triangles"].as_u64(),
        dimensions_m: a["dimensionsM"].as_array().map(|d| d.iter().filter_map(Value::as_f64).collect()),
    })
}

fn pair(d: &Value, k: &str) -> Result<[f64; 2]> {
    let c = d[k].as_array().filter(|a| a.len() == 2).ok_or_else(|| anyhow!("{PROVENANCE}: dims has no {k}[2]"))?;
    let g = |i: usize| c[i].as_f64().ok_or_else(|| anyhow!("{PROVENANCE}: {k}[{i}]"));
    Ok([g(0)?, g(1)?])
}

/// Read the provenance (the files are not read here).
pub fn load(repo: &Path) -> Result<SignalPoles> {
    let path = repo.join(PROVENANCE);
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let v: Value = serde_json::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let mut parts = vec![];
    for finish in FINISHES {
        for (_, name) in PARTS {
            let lods = v["finishes"][finish][name]["lods"].as_array().ok_or_else(|| anyhow!("{PROVENANCE}: no {finish} {name}"))?;
            if lods.len() != 3 {
                bail!("{PROVENANCE}: {finish} {name} has {} levels, not 3", lods.len());
            }
            parts.push([file_of(name, &lods[0])?, file_of(name, &lods[1])?, file_of(name, &lods[2])?]);
        }
    }
    let d = &v["dims"];
    let n = |k: &str| d[k].as_f64().ok_or_else(|| anyhow!("{PROVENANCE}: dims has no {k}"));
    let radii = d["mountArmRadiiM"].as_array().filter(|a| a.len() == MAST_MOUNTS.len()).ok_or_else(|| anyhow!("{PROVENANCE}: dims has no mountArmRadiiM[{}]", MAST_MOUNTS.len()))?;
    let r = |i: usize| radii[i].as_f64().ok_or_else(|| anyhow!("{PROVENANCE}: mountArmRadiiM[{i}]"));
    let dims = Dims {
        mast_pole_height_m: n("mastPoleHeightM")?,
        pedestal_height_m: n("pedestalHeightM")?,
        post_height_m: n("postHeightM")?,
        arm_root_radius_m: n("armRootRadiusM")?,
        arm_tip_radius_m: n("armTipRadiusM")?,
        mount_arm_radii_m: [r(0)?, r(1)?, r(2)?],
        side_reach_m: n("sideReachM")?,
        front_reach_m: n("frontReachM")?,
        ped_back_m: n("pedBackM")?,
        arms: {
            let rows = d["armClasses"].as_array().filter(|a| a.len() == ARM_RISE_PARTS.len()).ok_or_else(|| anyhow!("{PROVENANCE}: dims has no armClasses[{}]", ARM_RISE_PARTS.len()))?;
            let mut out = vec![];
            for (i, r) in rows.iter().enumerate() {
                let g = |k: &str| r[k].as_f64().ok_or_else(|| anyhow!("{PROVENANCE}: armClasses[{i}] has no {k}"));
                let class = crate::signals::ArmClass { length: g("lengthM")?, rise: g("riseM")?, angle: g("angleDeg")?.to_radians(), od_at_pole: g("odAtPoleM")? };
                out.push(ArmDims { class, root_radius_m: g("rootRadiusM")?, tip_radius_m: g("tipRadiusM")? });
            }
            out
        },
        button_mast_offset_m: n("pushButtonMastOffsetM")?,
        arm_flange_x_m: n("armFlangeXM")?,
        mast_cut_m: {
            let c = d["mastCutM"].as_array().filter(|a| a.len() == 2).ok_or_else(|| anyhow!("{PROVENANCE}: dims has no mastCutM[2]"))?;
            let g = |i: usize| c[i].as_f64().ok_or_else(|| anyhow!("{PROVENANCE}: mastCutM[{i}]"));
            [g(0)?, g(1)?]
        },
        cabinet_m: {
            let c = d["cabinetM"].as_array().filter(|a| a.len() == 3).ok_or_else(|| anyhow!("{PROVENANCE}: dims has no cabinetM[3]"))?;
            let g = |i: usize| c[i].as_f64().ok_or_else(|| anyhow!("{PROVENANCE}: cabinetM[{i}]"));
            [g(0)?, g(1)?, g(2)?]
        },
        cabinet_pad_m: pair(d, "cabinetPadM")?,
        pull_box_m: pair(d, "pullBoxM")?,
        sign_channel_m: pair(d, "signChannelM")?,
    };
    Ok(SignalPoles { parts, dims })
}

/// One drawn part at a place (the props' frame: three.js `rotation.y`, local +X to (cos yaw, -sin yaw)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub part: Part,
    /// Index into [`FINISHES`].
    pub finish: u8,
    pub pos: [f64; 3],
    pub yaw: f64,
    pub scale: [f32; 3],
}

impl Piece {
    /// The render model past `SIGNAL_POLE_BASE` (and the index into [`SignalPoles::parts`]).
    pub fn model(&self) -> u32 {
        self.finish as u32 * PARTS.len() as u32 + self.part as u32
    }
}

/// A sign plate drawn with the plates' own model (`Model::SignPlate`, its face by index):
/// a Street Name sign under an arm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plate {
    pub pos: [f64; 3],
    pub yaw: f64,
    /// (width, height, 1)
    pub scale: [f32; 3],
    /// The face index (the plates' face list, past the inventory's faces).
    pub face: u32,
}

/// What a tile's signal props draw as ([`hardware`]).
#[derive(Clone, Debug, Default)]
pub struct Hardware {
    /// Per prop (the slice given), the piece drawn in its place: every signal pole and mast arm.
    pub replace: Vec<Option<Piece>>,
    /// The pieces added: arm roots and end caps, the heads' mounts, the name signs' brackets,
    /// the push buttons.
    pub added: Vec<Piece>,
    /// The Street Name signs' plates.
    pub plates: Vec<Plate>,
    /// The street lights standing on a mast pole (`Fitting::Luminaire`), by index into the
    /// props: drawn without their post's base, the post inside the pole.
    pub pole_lamps: Vec<usize>,
}

/// The Street Name signs the renderer can draw: per legend, its face index and real size
/// (width, height, m; eo-render builds the faces from the official D3-1 layout).
#[derive(Clone, Debug, Default)]
pub struct NameSigns {
    pub faces: std::collections::HashMap<String, (u32, [f64; 2])>,
}

/// What [`hardware`] draws beyond round 1's poles, arms and mounts.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options<'a> {
    /// Each pole in the finish of its spot ([`finish_at`]); off, every pole painted.
    pub variants: bool,
    /// The fittings' Street Name signs (None: none, `--no-arm-name-signs`).
    pub name_signs: Option<&'a NameSigns>,
    /// The fittings' push buttons (`--no-push-buttons`).
    pub push_buttons: bool,
    /// A mast pole of another height than its model's drawn cut at its shaft (its foot and top
    /// at their size); off (`--no-pole-cut`), the whole model scaled.
    pub pole_cut: bool,
    /// A level arm at least as long as ES-7E's shortest drawn at its length class's diameter and
    /// taper ([`ARM_LEVEL_PARTS`]); off (`--no-arm-diameter`), the round 1 tube.
    pub level_arm_diameter: bool,
    /// The fittings' pull boxes beside the poles (`Fitting::PullBox`, `--no-pull-boxes`).
    pub pull_boxes: bool,
    /// Two stiffener channels across each Street Name sign's back (`--no-sign-channels`).
    pub sign_channels: bool,
    /// The controller cabinets standing round (the overhead's): a pull box keeps off their pads.
    pub cabinets: &'a [crate::overhead::Cabinet],
}

/// The yaw that turns local +X to the direction (dx, dz).
fn yaw_to(dx: f64, dz: f64) -> f64 {
    (-dz).atan2(dx)
}

/// A pole with no arm: a yaw from where it stands (its handhole faces somewhere, not always north).
fn yaw_of_spot(x: f64, z: f64) -> f64 {
    let h = ((x * 12.9898 + z * 78.233).sin() * 43758.5453).fract().abs();
    h * std::f64::consts::TAU
}

/// A pedestrian head whose housing's back is further than this from its pole's axis is carried
/// on pipe arms (the clamshell's plate reaches 0.09 m on a pedestal pipe); they end this far
/// inside the housing. Modelling values.
const PED_REACH_FROM_M: f64 = 0.09;
const PED_REACH_INTO_M: f64 = 0.05;

struct PoleAt {
    i: usize,
    x: f64,
    y: f64,
    z: f64,
    top: f64,
    part: Part,
    finish: u8,
}

/// The pieces a tile's signal props draw as (see the module comment). Pure: the same props give
/// the same pieces. Props other than the signal kinds are left alone (`replace` None).
/// `variants`: each pole in the finish of its spot ([`finish_at`]); off, every pole painted.
pub fn hardware(props: &[Prop], fittings: &[crate::signals::Fitting], d: &Dims, o: &Options) -> Hardware {
    let finish = |x: f64, z: f64| if o.variants { finish_at(x, z) } else { 0 };
    let mut out = Hardware { replace: vec![None; props.len()], added: vec![], plates: vec![], pole_lamps: vec![] };
    // an arm's shape: its rising class (variant 1 + row) or level
    let arm_of = |a: &Prop| (a.variant as usize).checked_sub(1).and_then(|c| d.arms.get(c).map(|x| (c, *x)));
    // (`--no-arm-diameter` off) a level arm's length class: its diameter and taper
    let classes: Vec<crate::signals::ArmClass> = d.arms.iter().map(|x| x.class).collect();
    let level_of = |a: &Prop| if o.level_arm_diameter && a.variant == 0 { crate::signals::ArmClass::of(&classes, a.scale[0] as f64).map(|c| (c, d.arms[c])) } else { None };
    // the arm's axis over its root and its radius at `x` metres along it
    let arm_at = |a: &Prop, x: f64| -> (f64, f64) {
        let t = x / (a.scale[0] as f64).max(1e-6);
        match (arm_of(a), level_of(a)) {
            (Some((_, ad)), _) => (ad.class.height_at(t), ad.radius_at(t)),
            (None, Some((_, ad))) => (0.0, ad.radius_at(t)),
            (None, None) => (0.0, d.arm_root_radius_m + (d.arm_tip_radius_m - d.arm_root_radius_m) * t.clamp(0.0, 1.0)),
        }
    };
    let poles: Vec<PoleAt> = props
        .iter()
        .enumerate()
        .filter(|(_, p)| p.model == Model::SignalPole)
        .map(|(i, p)| {
            let h = p.scale[1] as f64;
            let part = if p.variant == 1 {
                Part::MastPole
            } else if (h - d.post_height_m).abs() < (h - d.pedestal_height_m).abs() {
                Part::PedPost
            } else {
                Part::PedestalPole
            };
            PoleAt { i, x: p.pos[0], y: p.pos[1], z: p.pos[2], top: p.pos[1] + h, part, finish: finish(p.pos[0], p.pos[2]) }
        })
        .collect();
    let arms: Vec<(usize, &Prop)> = props.iter().enumerate().filter(|(_, p)| p.model == Model::MastArm).collect();
    for pole in &poles {
        let p = &props[pole.i];
        let model_h = match pole.part {
            Part::MastPole => d.mast_pole_height_m,
            Part::PedPost => d.post_height_m,
            _ => d.pedestal_height_m,
        };
        let h = p.scale[1] as f64;
        // the handhole (local -X) away from the pole's first arm
        let yaw = arms.iter().find(|(_, a)| (a.pos[0] - pole.x).hypot(a.pos[2] - pole.z) < 0.05).map(|(_, a)| a.yaw).unwrap_or_else(|| yaw_of_spot(pole.x, pole.z));
        let sy = if (h - model_h).abs() < 1e-3 { 1.0 } else { (h / model_h) as f32 };
        // a mast pole of another height: cut at its shaft, the shaft between its foot and its
        // top as long as the pole needs (too short to cut: scaled as before)
        let mid = d.mast_cut_m[1] + (h - model_h) - d.mast_cut_m[0];
        if o.pole_cut && pole.part == Part::MastPole && sy != 1.0 && mid > MAST_MID_MIN_M {
            let [c0, c1] = d.mast_cut_m;
            let k = mid / (c1 - c0);
            let at = |dy: f64| [p.pos[0], p.pos[1] + dy, p.pos[2]];
            out.replace[pole.i] = Some(Piece { part: Part::MastFoot, finish: pole.finish, pos: p.pos, yaw, scale: [1.0; 3] });
            out.added.push(Piece { part: Part::MastMid, finish: pole.finish, pos: at(c0 * (1.0 - k)), yaw, scale: [1.0, k as f32, 1.0] });
            out.added.push(Piece { part: Part::MastTop, finish: pole.finish, pos: at(h - model_h), yaw, scale: [1.0; 3] });
            continue;
        }
        out.replace[pole.i] = Some(Piece { part: pole.part, finish: pole.finish, pos: p.pos, yaw, scale: [1.0, sy, 1.0] });
    }
    for &(i, a) in &arms {
        let len = a.scale[0] as f64;
        // an arm stands on its pole's axis: the pole's finish
        let f = finish(a.pos[0], a.pos[2]);
        let tube = match (arm_of(a), level_of(a)) {
            // a rising arm: its class's part, built at the class's length
            (Some((c, ad)), _) => Piece { part: ARM_RISE_PARTS[c], finish: f, pos: a.pos, yaw: a.yaw, scale: [(len / ad.class.length) as f32, 1.0, 1.0] },
            // a level arm at its class's diameter
            (None, Some((c, ad))) => Piece { part: ARM_LEVEL_PARTS[c], finish: f, pos: a.pos, yaw: a.yaw, scale: [(len / ad.class.length) as f32, 1.0, 1.0] },
            (None, None) => Piece { part: Part::ArmTube, finish: f, pos: a.pos, yaw: a.yaw, scale: a.scale },
        };
        out.replace[i] = Some(tube);
        // the root plate centred on the arm where it meets the flange (a level arm: on its root)
        let at_flange = arm_of(a).map_or(0.0, |_| arm_at(a, d.arm_flange_x_m).0);
        out.added.push(Piece { part: Part::ArmRoot, finish: f, pos: [a.pos[0], a.pos[1] + at_flange, a.pos[2]], yaw: a.yaw, scale: [1.0; 3] });
        let (s, c) = a.yaw.sin_cos();
        let up = arm_at(a, len).0;
        out.added.push(Piece { part: Part::ArmTip, finish: f, pos: [a.pos[0] + c * len, a.pos[1] + up, a.pos[2] - s * len], yaw: a.yaw, scale: [1.0; 3] });
    }
    let pole_for = |x: f64, y: f64, z: f64, reach: f64| -> Option<&PoleAt> {
        poles
            .iter()
            .filter(|q| y > q.y && y < q.top + 0.3)
            .map(|q| (q, (q.x - x).hypot(q.z - z)))
            .filter(|(_, dist)| *dist <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(q, _)| q)
    };
    for p in props {
        match p.model {
            Model::SignalHead if p.variant == 1 => {
                // the arm over it: the one whose axis passes closest above the head
                let mut best: Option<(f64, [f64; 3], f64, f64, u8)> = None; // (off-axis, point, yaw, radius, finish)
                for &(_, a) in &arms {
                    let len = a.scale[0] as f64;
                    let (s, c) = a.yaw.sin_cos();
                    let (dx, dz) = (c, -s);
                    let (vx, vz) = (p.pos[0] - a.pos[0], p.pos[2] - a.pos[2]);
                    let t = vx * dx + vz * dz;
                    let off = (vx * dz - vz * dx).abs();
                    let (up, r) = arm_at(a, t);
                    let drop = a.pos[1] + up - p.pos[1];
                    if !(-0.05..=len + 0.05).contains(&t) || off > 0.3 || !(0.2..=1.5).contains(&drop) {
                        continue;
                    }
                    if best.is_none_or(|b| off < b.0) {
                        best = Some((off, [a.pos[0] + dx * t, a.pos[1] + up, a.pos[2] + dz * t], a.yaw, r, finish(a.pos[0], a.pos[2])));
                    }
                }
                if let Some((_, at, yaw, r, f)) = best {
                    let k = (0..MAST_MOUNTS.len()).min_by(|&i, &j| (d.mount_arm_radii_m[i] - r).abs().total_cmp(&(d.mount_arm_radii_m[j] - r).abs())).unwrap_or(0);
                    out.added.push(Piece { part: MAST_MOUNTS[k], finish: f, pos: at, yaw, scale: [1.0; 3] });
                }
            }
            Model::SignalHead => {
                // a head on a pole: beside it (mount 0) or in front of it (mount 2)
                let front = p.variant == 2;
                let reach = if front { d.front_reach_m } else { d.side_reach_m };
                if let Some(q) = pole_for(p.pos[0], p.pos[1], p.pos[2], reach + 0.25) {
                    let (dx, dz) = (p.pos[0] - q.x, p.pos[2] - q.z);
                    let dist = dx.hypot(dz);
                    if dist < 0.02 {
                        continue;
                    }
                    let mast = q.part == Part::MastPole;
                    let part = match (front, mast) {
                        (false, false) => Part::SideMountPipe,
                        (false, true) => Part::SideMountMast,
                        (true, false) => Part::FrontMountPipe,
                        (true, true) => Part::FrontMountMast,
                    };
                    let sx = if (dist - reach).abs() < 0.01 { 1.0 } else { (dist / reach) as f32 };
                    out.added.push(Piece { part, finish: q.finish, pos: [q.x, p.pos[1], q.z], yaw: yaw_to(dx, dz), scale: [sx, 1.0, 1.0] });
                }
            }
            Model::PedHead => {
                if let Some(q) = pole_for(p.pos[0], p.pos[1], p.pos[2], 0.6) {
                    let (dx, dz) = (p.pos[0] - q.x, p.pos[2] - q.z);
                    // toward the head (or, on the pole's axis, the way it faces)
                    let (dx, dz) = if dx.hypot(dz) < 0.02 { (p.yaw.sin(), p.yaw.cos()) } else { (dx, dz) };
                    let part = if q.part == Part::MastPole { Part::PedMountMast } else { Part::PedMountPipe };
                    let yaw = yaw_to(dx, dz);
                    out.added.push(Piece { part, finish: q.finish, pos: [q.x, p.pos[1], q.z], yaw, scale: [1.0; 3] });
                    // a head standing off the pole (its housing's back past the clamshell): two
                    // pipe arms from the clamshell into the housing
                    let back = dx.hypot(dz) - d.ped_back_m;
                    if back > PED_REACH_FROM_M {
                        out.added.push(Piece { part: Part::PedReach, finish: q.finish, pos: [q.x, p.pos[1], q.z], yaw, scale: [(back + PED_REACH_INTO_M) as f32, 1.0, 1.0] });
                    }
                }
            }
            _ => {}
        }
    }
    for ft in fittings {
        match ft {
            crate::signals::Fitting::NameSign { arm, yaw, gap, facing, legend } => {
                let Some(ns) = o.name_signs else { continue };
                let Some(&(face, [w, h])) = ns.faces.get(legend) else { continue };
                let Some(&(_, a)) = arms.iter().find(|(_, a)| (a.pos[0] - arm[0]).hypot(a.pos[2] - arm[2]) < 0.05 && (a.yaw - yaw).abs() < 1e-6) else { continue };
                if gap[1] - gap[0] < w + 2.0 * SIGN_CLEAR_M {
                    continue;
                }
                let xc = (gap[0] + gap[1]) / 2.0;
                let (s, c) = a.yaw.sin_cos();
                let at = |x: f64| [a.pos[0] + c * x, a.pos[2] - s * x];
                // hung level under the arm's lower end over its width
                let ends = [xc - w / 2.0, xc + w / 2.0];
                let low = ends.iter().map(|&x| arm_at(a, x)).map(|(up, r)| up - r).fold(f64::INFINITY, f64::min);
                let top = a.pos[1] + low - SIGN_HANG_M;
                let p = at(xc);
                out.plates.push(Plate { pos: [p[0], top - h / 2.0, p[1]], yaw: *facing, scale: [w as f32, h as f32, 1.0], face });
                // (`--no-sign-channels` off) two stiffener channels across its back, a quarter of
                // its height in from its top and its bottom, ending short of its edges
                if o.sign_channels {
                    let (fs, fc) = facing.sin_cos();
                    let back = [p[0] - fs * SIGN_PLATE_T_M, p[1] - fc * SIGN_PLATE_T_M];
                    for dy in [-h / 4.0, h / 4.0] {
                        let len = (w - 2.0 * SIGN_CHANNEL_IN_M).max(0.1);
                        out.added.push(Piece { part: Part::SignChannel, finish: 0, pos: [back[0], top - h / 2.0 + dy, back[1]], yaw: *facing, scale: [len as f32, 1.0, 1.0] });
                    }
                }
                // two brackets from the arm's axis down to the sign's top, behind its face
                let (fs, fc) = facing.sin_cos();
                let f = finish(a.pos[0], a.pos[2]);
                for x in [xc - w / 2.0 + SIGN_BRACKET_IN_M, xc + w / 2.0 - SIGN_BRACKET_IN_M] {
                    let q = at(x);
                    let y = a.pos[1] + arm_at(a, x).0;
                    let len = y - top + SIGN_BRACKET_INTO_M;
                    out.added.push(Piece { part: Part::SignBracket, finish: f, pos: [q[0] - fs * SIGN_BRACKET_BEHIND_M, y, q[1] - fc * SIGN_BRACKET_BEHIND_M], yaw: a.yaw, scale: [1.0, len as f32, 1.0] });
                }
            }
            crate::signals::Fitting::PullBox { pos, yaw } => {
                // (a box another junction's pole or box stands on, the node planning it not seeing
                // them, is left out)
                let half = d.pull_box_m[0] / 2.0;
                let on_pole = poles.iter().any(|q| (q.x - pos[0]).hypot(q.z - pos[2]) < half + if q.part == Part::MastPole { crate::signals::MAST_FOUNDATION_R_M } else { crate::signals::POST_FOUNDATION_R_M });
                let on_box = out.added.iter().any(|q| q.part == Part::PullBox && (q.pos[0] - pos[0]).hypot(q.pos[2] - pos[2]) < 2.0 * half);
                // nor under a cabinet's pad (its larger half extent round it)
                let pad = d.cabinet_m[0].max(d.cabinet_m[1]) / 2.0 + d.cabinet_pad_m[1];
                let on_cabinet = o.cabinets.iter().any(|c| (c.pos[0] - pos[0]).hypot(c.pos[2] - pos[2]) < pad + half);
                if o.pull_boxes && !on_pole && !on_box && !on_cabinet {
                    out.added.push(Piece { part: Part::PullBox, finish: 0, pos: *pos, yaw: *yaw, scale: [1.0; 3] });
                }
            }
            crate::signals::Fitting::Luminaire { pole } => {
                let on_pole = |q: &Prop| (q.pos[0] - pole[0]).hypot(q.pos[2] - pole[2]) < 0.05;
                if poles.iter().any(|q| q.part == Part::MastPole && on_pole(&props[q.i])) {
                    out.pole_lamps.extend(props.iter().position(|q| matches!(q.model, Model::LampCobra | Model::LampBanner) && on_pole(q)));
                }
            }
            crate::signals::Fitting::PushButton { pole, height, yaw, arrow_plus_z } => {
                if !o.push_buttons {
                    continue;
                }
                let Some(q) = poles.iter().find(|q| (q.x - pole[0]).hypot(q.z - pole[2]) < 0.05) else { continue };
                let part = if *arrow_plus_z { Part::ButtonPipeP } else { Part::ButtonPipeN };
                // on a mast pole, set out to its face (local +X: (cos yaw, -sin yaw))
                let out_m = if q.part == Part::MastPole { d.button_mast_offset_m } else { 0.0 };
                let (s, c) = yaw.sin_cos();
                out.added.push(Piece { part, finish: q.finish, pos: [q.x + c * out_m, q.y + height, q.z - s * out_m], yaw: *yaw, scale: [1.0; 3] });
            }
        }
    }
    out
}

/// A Street Name sign keeps this clear of the heads and the arm's plate on each side, hangs
/// this far under the arm, on brackets this far in from its ends that reach this far down
/// behind its face (modelling values).
pub const SIGN_CLEAR_M: f64 = 0.05;
/// The Street Name sign plate's thickness (eo-render props_lib's code-built plate: its face at
/// local z 0, its back 3 mm behind): the channels stand on its back.
pub const SIGN_PLATE_T_M: f64 = 0.003;
/// The channels end this far in from the sign's edges (modelling value).
pub const SIGN_CHANNEL_IN_M: f64 = 0.03;
pub const SIGN_HANG_M: f64 = 0.06;
const SIGN_BRACKET_IN_M: f64 = 0.15;
const SIGN_BRACKET_INTO_M: f64 = 0.05;
const SIGN_BRACKET_BEHIND_M: f64 = 0.012;
/// The shortest shaft a cut mast pole draws between its foot and its top (shorter: the whole
/// pole scaled). A modelling value.
const MAST_MID_MIN_M: f64 = 0.05;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Live;

    fn dims() -> Dims {
        Dims {
            mast_pole_height_m: 6.9,
            pedestal_height_m: 3.95,
            post_height_m: 3.2,
            arm_root_radius_m: 0.09,
            arm_tip_radius_m: 0.06,
            mount_arm_radii_m: [0.0875, 0.0775, 0.0675],
            side_reach_m: 0.34,
            front_reach_m: 0.2,
            ped_back_m: 0.209,
            arms: [(4.572, 1.27, 23.0, 0.1683), (6.096, 1.27, 23.0, 0.1778), (7.62, 2.032, 23.0, 0.2), (9.144, 2.134, 23.0, 0.2032), (10.668, 2.134, 21.0, 0.2223), (12.192, 2.134, 15.0, 0.2381), (13.716, 2.337, 15.0, 0.2572)]
                .iter()
                .map(|&(length, rise, deg, od)| ArmDims { class: crate::signals::ArmClass { length, rise, angle: f64::to_radians(deg), od_at_pole: od }, root_radius_m: od / 2.0, tip_radius_m: od / 2.0 - 0.04 })
                .collect(),
            button_mast_offset_m: 0.08,
            arm_flange_x_m: 0.245,
            mast_cut_m: [2.0, 4.45],
            cabinet_m: [0.9144, 0.4699, 1.5494],
            cabinet_pad_m: [0.076, 0.06],
            pull_box_m: [0.6985, 0.4572],
            sign_channel_m: [0.0381, 0.0191],
        }
    }

    fn on() -> Options<'static> {
        Options { variants: true, ..Default::default() }
    }

    fn prop(model: Model, pos: [f64; 3], yaw: f64, scale: [f32; 3], variant: u16) -> Prop {
        Prop { model, pos, yaw, scale, variant, live: Live::None }
    }

    /// A mast pole with an arm and two heads on it, a head beside the pole, a pedestal with a
    /// head in front of it, a pedestrian post with its head.
    fn junction() -> Vec<Prop> {
        let yaw = 0.7f64;
        let (s, c) = yaw.sin_cos();
        let along = |t: f64| [10.0 + c * t, 6.45, 20.0 - s * t];
        let at = |t: f64, y: f64| [10.0 + c * t, y, 20.0 - s * t];
        vec![
            prop(Model::SignalPole, [10.0, 0.0, 20.0], 0.0, [1.0, 6.9, 1.0], 1),
            prop(Model::MastArm, along(0.0), yaw, [8.0, 1.0, 1.0], 0),
            prop(Model::SignalHead, at(0.9, 5.75), yaw + 1.57, [1.0; 3], 1),
            prop(Model::SignalHead, at(7.6, 5.75), yaw + 1.57, [1.0; 3], 1),
            prop(Model::SignalHead, at(0.34, 3.25), yaw + 1.57, [1.0; 3], 0),
            prop(Model::SignalPole, [30.0, 0.1, 20.0], 0.0, [1.0, 3.95, 1.0], 0),
            prop(Model::SignalHead, [30.0, 3.2, 20.2], 0.0, [1.0; 3], 2),
            prop(Model::SignalPole, [40.0, 0.0, 25.0], 0.0, [1.0, 3.2, 1.0], 0),
            prop(Model::PedHead, [40.2, 2.8, 25.0], 1.57, [1.0; 3], 0),
            prop(Model::Hydrant, [41.0, 0.0, 25.0], 0.0, [1.0; 3], 0),
        ]
    }

    #[test]
    fn poles_and_arms_take_their_models() {
        let props = junction();
        let hw = hardware(&props, &[], &dims(), &on());
        let parts: Vec<Option<Part>> = hw.replace.iter().map(|r| r.map(|p| p.part)).collect();
        assert_eq!(parts[0], Some(Part::MastPole));
        assert_eq!(parts[1], Some(Part::ArmTube));
        assert_eq!(parts[5], Some(Part::PedestalPole));
        assert_eq!(parts[7], Some(Part::PedPost));
        assert!(parts[2].is_none() && parts[8].is_none() && parts[9].is_none(), "heads and other props stay");
        // the mast pole turned to its arm (handhole away), the arm as it was
        assert!((hw.replace[0].unwrap().yaw - 0.7).abs() < 1e-9);
        assert_eq!(hw.replace[1].unwrap().scale, [8.0, 1.0, 1.0]);
        assert_eq!(hw.replace[0].unwrap().scale, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn arm_root_and_end_cap() {
        let props = junction();
        let hw = hardware(&props, &[], &dims(), &on());
        let root = hw.added.iter().find(|p| p.part == Part::ArmRoot).unwrap();
        let tip = hw.added.iter().find(|p| p.part == Part::ArmTip).unwrap();
        assert_eq!(root.pos, props[1].pos);
        let (s, c) = 0.7f64.sin_cos();
        assert!((tip.pos[0] - (10.0 + 8.0 * c)).abs() < 1e-9 && (tip.pos[2] - (20.0 - 8.0 * s)).abs() < 1e-9 && tip.pos[1] == 6.45);
    }

    #[test]
    fn heads_get_their_mounts() {
        let props = junction();
        let hw = hardware(&props, &[], &dims(), &on());
        let count = |part: Part| hw.added.iter().filter(|p| p.part == part).count();
        // near the root the thickest strap, near the end the thinnest
        assert_eq!(count(Part::MastMount0), 1);
        assert_eq!(count(Part::MastMount2), 1);
        let near = hw.added.iter().find(|p| p.part == Part::MastMount0).unwrap();
        assert!((near.pos[1] - 6.45).abs() < 1e-9, "on the arm's axis");
        assert_eq!(count(Part::SideMountMast), 1);
        let side = hw.added.iter().find(|p| p.part == Part::SideMountMast).unwrap();
        assert_eq!(side.scale, [1.0, 1.0, 1.0]);
        assert!((side.pos[1] - 3.25).abs() < 1e-9 && side.pos[0] == 10.0 && side.pos[2] == 20.0);
        // local +X toward the head
        let (s, c) = side.yaw.sin_cos();
        let (hx, hz) = (props[4].pos[0] - 10.0, props[4].pos[2] - 20.0);
        assert!((c * hx - s * hz - 0.34).abs() < 1e-6);
        assert_eq!(count(Part::FrontMountPipe), 1);
        assert_eq!(count(Part::PedMountPipe), 1);
        let ped = hw.added.iter().find(|p| p.part == Part::PedMountPipe).unwrap();
        assert!((ped.yaw - 0.0).abs() < 1e-9, "+X toward the head at +x");
        assert_eq!(hw.added.len(), 2 + 2 + 1 + 1 + 1);
        assert!(hw.added.iter().all(|p| p.part != Part::PedReach), "a head on its clamshell needs no arms");
    }

    #[test]
    fn a_pedestrian_head_off_its_pole_on_pipe_arms() {
        let props = vec![prop(Model::SignalPole, [0.0, 0.0, 0.0], 0.0, [1.0, 3.95, 1.0], 0), prop(Model::PedHead, [0.2, 2.8, -0.36], 0.0, [1.0; 3], 0)];
        let hw = hardware(&props, &[], &dims(), &on());
        assert_eq!(hw.added.len(), 2);
        let reach = hw.added.iter().find(|p| p.part == Part::PedReach).unwrap();
        let dist = 0.2f64.hypot(0.36);
        assert!((reach.scale[0] as f64 - (dist - 0.209 + 0.05)).abs() < 1e-6);
        // +X toward the head
        let (s, c) = reach.yaw.sin_cos();
        assert!((c * 0.2 - s * -0.36 - dist).abs() < 1e-9);
    }

    #[test]
    fn a_pole_and_its_hardware_share_its_finish() {
        let props = junction();
        let on = hardware(&props, &[], &dims(), &on());
        let mast_finish = finish_at(10.0, 20.0);
        for p in on.added.iter().filter(|p| matches!(p.part, Part::ArmRoot | Part::ArmTip | Part::MastMount0 | Part::MastMount2 | Part::SideMountMast)) {
            assert_eq!(p.finish, mast_finish, "{:?}", p.part);
        }
        assert_eq!(on.replace[1].unwrap().finish, mast_finish, "the arm");
        let off = hardware(&props, &[], &dims(), &Options::default());
        assert!(off.replace.iter().flatten().chain(off.added.iter()).all(|p| p.finish == 0), "off: every pole painted");
        assert_eq!(off.added.len(), on.added.len());
        // both finishes drawn across the city, about half each
        let galv = (0..400).filter(|k| finish_at((k % 20) as f64 * 50.0, (k / 20) as f64 * 50.0) == 1).count();
        assert!((140..260).contains(&galv), "{galv} of 400 cells galvanised");
        assert_eq!(Piece { part: Part::ArmTip, finish: 1, pos: [0.0; 3], yaw: 0.0, scale: [1.0; 3] }.model(), PARTS.len() as u32 + Part::ArmTip as u32);
    }

    /// A rising arm (variant 1 + class row): its class's part scaled to its length, its end cap
    /// at its rise, the hanger on its axis over the head, a name sign under it in its gap
    /// with two brackets, a push button on a pole.
    #[test]
    fn rising_arm_name_sign_and_push_button() {
        use crate::signals::Fitting;
        let d = dims();
        let yaw = 0.0f64; // +X along +x
        let mut props = vec![
            prop(Model::SignalPole, [0.0, 0.0, 0.0], 0.0, [1.0, 6.9, 1.0], 1),
            prop(Model::MastArm, [0.0, 5.5, 0.0], yaw, [10.0, 1.0, 1.0], 5), // class 4 (10.668 m)
        ];
        let ad = d.arms[4];
        let head_y = 5.5 + ad.class.height_at(0.9) - 0.7;
        props.push(prop(Model::SignalHead, [9.0, head_y, 0.0], 1.57, [1.0; 3], 1));
        let mut ns = NameSigns::default();
        ns.faces.insert("ORTEGA ST".into(), (30, [2.0, 0.45]));
        let fittings = vec![
            Fitting::NameSign { arm: [0.0, 5.5, 0.0], yaw, gap: [0.45, 8.5], facing: 1.57, legend: "ORTEGA ST".into() },
            Fitting::NameSign { arm: [0.0, 5.5, 0.0], yaw, gap: [0.45, 8.5], facing: 1.57, legend: "UNKNOWN".into() },
            Fitting::PushButton { pole: [0.0, 0.0, 0.0], height: 1.067, yaw: 0.3, arrow_plus_z: false },
        ];
        let o = Options { variants: false, name_signs: Some(&ns), push_buttons: true, pole_cut: false, level_arm_diameter: false, ..Default::default() };
        let hw = hardware(&props, &fittings, &d, &o);
        let tube = hw.replace[1].unwrap();
        assert_eq!(tube.part, Part::ArmRise4);
        assert!((tube.scale[0] as f64 - 10.0 / 10.668).abs() < 1e-6);
        let tip = hw.added.iter().find(|p| p.part == Part::ArmTip).unwrap();
        assert!((tip.pos[1] - (5.5 + ad.class.rise)).abs() < 1e-9 && (tip.pos[0] - 10.0).abs() < 1e-9, "the end cap at the rise: {:?}", tip.pos);
        let mount = hw.added.iter().find(|p| MAST_MOUNTS.contains(&p.part)).unwrap();
        assert!((mount.pos[1] - (5.5 + ad.class.height_at(0.9))).abs() < 1e-9, "the hanger on the arm's axis");
        // the strap built round the radius nearest the arm's there
        let r = ad.radius_at(0.9);
        let k = MAST_MOUNTS.iter().position(|&m| m == mount.part).unwrap();
        assert!(d.mount_arm_radii_m.iter().all(|x| (x - r).abs() >= (d.mount_arm_radii_m[k] - r).abs()));
        // one sign (the unknown legend has no face), level under the arm's lower end, two brackets
        assert_eq!(hw.plates.len(), 1);
        let pl = hw.plates[0];
        assert_eq!(pl.face, 30);
        let xc = (0.45 + 8.5) / 2.0;
        let low = ad.class.height_at((xc - 1.0) / 10.0) - ad.radius_at((xc - 1.0) / 10.0);
        assert!((pl.pos[1] + 0.225 - (5.5 + low - SIGN_HANG_M)).abs() < 1e-9, "the sign's top just under the arm");
        let brackets: Vec<&Piece> = hw.added.iter().filter(|p| p.part == Part::SignBracket).collect();
        assert_eq!(brackets.len(), 2);
        assert!(brackets[1].scale[1] > brackets[0].scale[1], "the bracket further out is longer (the arm rises)");
        let b = hw.added.iter().find(|p| p.part == Part::ButtonPipeN).unwrap();
        assert!((b.pos[1] - 1.067).abs() < 1e-9 && b.yaw == 0.3);
        assert!((b.pos[0] - 0.3f64.cos() * 0.08).abs() < 1e-9 && (b.pos[2] + 0.3f64.sin() * 0.08).abs() < 1e-9, "set out to the mast's face");
        // off: no sign, no button
        let off = hardware(&props, &fittings, &d, &Options::default());
        assert!(off.plates.is_empty() && off.added.iter().all(|p| !matches!(p.part, Part::SignBracket | Part::ButtonPipeN)));
    }

    /// Round 3: a mast pole shorter than its model cut at its shaft (its foot and top at their
    /// size, the shaft between at the length left), one at its model's height drawn whole; a
    /// street light on a mast pole's foot listed with the pole's lamps.
    #[test]
    fn cut_pole_and_pole_lamp() {
        use crate::signals::Fitting;
        let d = dims();
        let o = Options { pole_cut: true, ..Default::default() };
        let props = vec![
            prop(Model::SignalPole, [1.0, 2.0, 3.0], 0.0, [1.0, 5.5, 1.0], 1),
            prop(Model::SignalPole, [9.0, 0.0, 3.0], 0.0, [1.0, 6.9, 1.0], 1),
            prop(Model::LampCobra, [1.0, 2.0, 3.0], 0.4, [1.0; 3], 0),
            prop(Model::LampBanner, [9.0, 0.0, 3.0], 0.4, [1.0; 3], 0),
        ];
        let fittings = vec![Fitting::Luminaire { pole: [1.0, 2.0, 3.0] }];
        let hw = hardware(&props, &fittings, &d, &o);
        let foot = hw.replace[0].unwrap();
        assert_eq!((foot.part, foot.scale, foot.pos), (Part::MastFoot, [1.0; 3], [1.0, 2.0, 3.0]), "the foot at its size on the pole's foot");
        let mid = hw.added.iter().find(|p| p.part == Part::MastMid).unwrap();
        let top = hw.added.iter().find(|p| p.part == Part::MastTop).unwrap();
        let [c0, c1] = d.mast_cut_m;
        let k = mid.scale[1] as f64;
        assert!((mid.pos[1] + c0 * k - (2.0 + c0)).abs() < 1e-6, "the shaft starts at the foot's cut");
        assert!((mid.pos[1] + c1 * k - (top.pos[1] + c1)).abs() < 1e-6, "and ends at the top's cut");
        assert!((top.pos[1] + d.mast_pole_height_m - (2.0 + 5.5)).abs() < 1e-6 && top.scale == [1.0; 3], "the top at its size, the pole 5.5 m");
        assert_eq!(hw.replace[1].unwrap().part, Part::MastPole, "a pole of its model's height drawn whole");
        assert_eq!(hw.pole_lamps, vec![2], "the light on the fitting's pole, not the one beside the other pole");
        // off: the whole model scaled, as before
        let off = hardware(&props, &fittings, &d, &Options::default());
        let whole = off.replace[0].unwrap();
        assert_eq!(whole.part, Part::MastPole);
        assert!((whole.scale[1] as f64 - 5.5 / 6.9).abs() < 1e-6);
        assert!(off.added.iter().all(|p| !matches!(p.part, Part::MastMid | Part::MastTop)));
        // too short to cut: scaled
        let short = vec![prop(Model::SignalPole, [0.0; 3], 0.0, [1.0, 4.4, 1.0], 1)];
        assert_eq!(hardware(&short, &[], &d, &o).replace[0].unwrap().part, Part::MastPole);
    }

    /// Round 4: a level arm at least as long as ES-7E's shortest drawn at its length class's
    /// diameter (its hanger built round the class's radius there); a shorter one, or with the
    /// switch off, the round 1 tube.
    #[test]
    fn level_arm_at_its_class_diameter() {
        let d = dims();
        let yaw = 0.0f64;
        let props = vec![
            prop(Model::SignalPole, [0.0; 3], 0.0, [1.0, 6.9, 1.0], 1),
            prop(Model::MastArm, [0.0, 5.9, 0.0], yaw, [13.0, 1.0, 1.0], 0),
            prop(Model::SignalHead, [12.0, 5.2, 0.0], 1.57, [1.0; 3], 1),
            prop(Model::SignalPole, [30.0, 0.0, 0.0], 0.0, [1.0, 6.9, 1.0], 1),
            prop(Model::MastArm, [30.0, 5.9, 0.0], yaw, [3.0, 1.0, 1.0], 0),
        ];
        let on = hardware(&props, &[], &d, &Options { level_arm_diameter: true, ..Default::default() });
        let arm = on.replace[1].unwrap();
        assert_eq!(arm.part, ARM_LEVEL_PARTS[6], "13 m: the 45 ft row");
        assert!((arm.scale[0] as f64 - 13.0 / d.arms[6].class.length).abs() < 1e-6 && arm.scale[1] == 1.0);
        let mount = on.added.iter().find(|p| MAST_MOUNTS.contains(&p.part)).unwrap();
        assert!((mount.pos[1] - 5.9).abs() < 1e-9, "the hanger on the level arm's axis");
        assert_eq!(on.replace[4].unwrap().part, Part::ArmTube, "under 15 ft: the tube");
        let off = hardware(&props, &[], &d, &Options::default());
        assert_eq!(off.replace[1].unwrap().part, Part::ArmTube, "off: the round 1 tube");
        assert_eq!(off.replace[1].unwrap().scale, [13.0, 1.0, 1.0]);
    }

    #[test]
    fn nothing_without_signal_props() {
        let props = vec![prop(Model::Hydrant, [0.0; 3], 0.0, [1.0; 3], 0)];
        let hw = hardware(&props, &[], &dims(), &on());
        assert!(hw.replace.iter().all(Option::is_none) && hw.added.is_empty());
    }
}
