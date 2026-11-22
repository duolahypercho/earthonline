//! Playable bodies and skeletal clips from the local character demos.

use crate::motion::{read_glb_clip, topo_nodes};
use crate::packed_glb::Glb;
use crate::people::{Atlas, Clip, CookedTier, RestNode};
use crate::{AGENT_PLAYER, AgentVertex, GpuAgent};
use anyhow::{Context, Result, anyhow, bail, ensure};
use eo_assets::manifest::AssetRef;
use glam::{Mat4, Quat, Vec3};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("../../assets/test-characters-manifest.json");
pub const ROOT_ENV: &str = "EO_TEST_CHARACTERS_ROOT";

#[derive(Deserialize)]
struct Manifest {
    roster: Vec<Avatar>,
    packs: BTreeMap<String, Vec<String>>,
    files: BTreeMap<String, File>,
}

#[derive(Deserialize)]
struct Avatar {
    id: String,
    body: String,
    packs: Vec<String>,
}

#[derive(Deserialize)]
struct File {
    bytes: u64,
    sha256: String,
    #[serde(default)]
    clips: Vec<String>,
}

impl Manifest {
    fn avatar(&self, id: &str) -> Result<&Avatar> {
        self.roster
            .iter()
            .find(|avatar| avatar.id == id)
            .ok_or_else(|| {
                let ids: Vec<_> = self
                    .roster
                    .iter()
                    .map(|avatar| avatar.id.as_str())
                    .collect();
                anyhow!(
                    "unknown test character {id}; choose from {}",
                    ids.join(", ")
                )
            })
    }

    fn motions<'a>(&'a self, avatar: &'a Avatar) -> impl Iterator<Item = &'a str> {
        avatar
            .packs
            .iter()
            .flat_map(|pack| self.packs[pack].iter().map(String::as_str))
    }

    fn read(&self, root: &Path, path: &str) -> Result<Vec<u8>> {
        let file = &self.files[path];
        let asset = AssetRef {
            url: path.into(),
            bytes: file.bytes,
            sha256: file.sha256.clone(),
            storey: None,
        };
        eo_assets::glb::read_verified(&root.join(path), &asset)
    }
}

/// List the pinned roster and clip names without loading the optional assets.
pub fn catalogue() -> Result<Vec<(String, Vec<String>)>> {
    let manifest: Manifest = serde_json::from_str(MANIFEST)?;
    Ok(manifest
        .roster
        .iter()
        .map(|avatar| {
            let clips = manifest
                .motions(avatar)
                .flat_map(|path| manifest.files[path].clips.iter().cloned())
                .collect();
            (avatar.id.clone(), clips)
        })
        .collect())
}

#[derive(Clone, Debug)]
pub struct Selection {
    pub id: String,
    pub preview: Option<String>,
    root: PathBuf,
}

impl Selection {
    pub fn from_env(id: String, preview: Option<String>) -> Result<Self> {
        let root = std::env::var_os(ROOT_ENV)
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("set {ROOT_ENV} to the demo's public directory"))?;
        Self::new(root, id, preview)
    }

    pub fn new(root: PathBuf, id: String, preview: Option<String>) -> Result<Self> {
        let manifest: Manifest = serde_json::from_str(MANIFEST)?;
        let avatar = manifest.avatar(&id)?;
        if let Some(clip) = &preview {
            ensure!(
                manifest
                    .motions(avatar)
                    .any(|path| manifest.files[path].clips.contains(clip)),
                "{id} has no clip {clip}; see --list-test-clips"
            );
        }
        for path in std::iter::once(avatar.body.as_str()).chain(manifest.motions(avatar)) {
            ensure!(
                root.join(path).is_file(),
                "test character {id}: missing {}",
                root.join(path).display()
            );
            if let Some(notes) = notes_path(&manifest, path) {
                ensure!(
                    root.join(&notes).is_file(),
                    "test character {id}: missing {}",
                    root.join(notes).display()
                );
            }
        }
        Ok(Self { id, preview, root })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct ClipNotes {
    speed: f32,
    #[serde(rename = "loop")]
    loops: bool,
    root: Vec<[f32; 3]>,
    door: Vec<f32>,
    events: CarEvents,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct CarEvents {
    door_open_start: f32,
    door_open_end: f32,
    seated: f32,
    unseated: f32,
    free: f32,
}

#[derive(Clone, Copy)]
pub struct CarSample {
    pub root: [f32; 3],
    pub door: f32,
    pub seat_weight: f32,
}
fn notes_path(manifest: &Manifest, motion: &str) -> Option<String> {
    let notes = motion.replace(".glb", ".json");
    manifest.files.contains_key(&notes).then_some(notes)
}

#[derive(Clone, Debug)]
struct Motion {
    clip: Clip,
    channels: Vec<(usize, usize)>,
    notes: ClipNotes,
}

#[derive(Clone, Copy, Debug)]
struct Local {
    t: Vec3,
    r: Quat,
    s: Vec3,
}

impl From<&RestNode> for Local {
    fn from(node: &RestNode) -> Self {
        Self {
            t: node.t,
            r: node.r,
            s: node.s,
        }
    }
}

/// Scratch poses for the one local player. Blend local transforms before FK.
#[derive(Default)]
pub struct Pose {
    current: Vec<Local>,
    previous: Vec<Local>,
    world: Vec<Mat4>,
    pub rows: Vec<[f32; 4]>,
}

#[derive(Clone, Debug)]
pub struct Rig {
    nodes: Vec<RestNode>,
    order: Vec<usize>,
    skin: Vec<(usize, Mat4)>,
    root: usize,
    clips: Vec<Motion>,
    names: HashMap<String, usize>,
    pub motorcycle: bool,
}

impl Pose {
    pub fn start_blend(&mut self) {
        self.previous.clone_from(&self.current);
    }

    pub fn sample(&mut self, rig: &Rig, clip: usize, time: f32, keep_root: bool, blend: f32) {
        let motion = &rig.clips[clip];
        self.current.clear();
        self.current.extend(rig.nodes.iter().map(Local::from));
        for &(node, channel) in &motion.channels {
            if !keep_root && node == rig.root {
                continue;
            }
            let c = &motion.clip.channels[channel];
            let v = motion.clip.sample(c, time);
            let local = &mut self.current[node];
            match c.path {
                0 => local.t = Vec3::new(v[0], v[1], v[2]),
                1 => local.r = Quat::from_array(v),
                _ => local.s = Vec3::new(v[0], v[1], v[2]),
            }
        }
        if blend < 1.0 && self.previous.len() == self.current.len() {
            for (current, previous) in self.current.iter_mut().zip(&self.previous) {
                current.t = previous.t.lerp(current.t, blend);
                current.r = previous.r.slerp(current.r, blend);
                current.s = previous.s.lerp(current.s, blend);
            }
        }
        self.world.resize(rig.nodes.len(), Mat4::IDENTITY);
        for &i in &rig.order {
            let local = self.current[i];
            let matrix = Mat4::from_scale_rotation_translation(local.s, local.r, local.t);
            self.world[i] = rig.nodes[i]
                .parent
                .map_or(matrix, |parent| self.world[parent] * matrix);
        }
        self.rows.clear();
        for &(i, inverse_bind) in &rig.skin {
            let rows = (self.world[i] * inverse_bind)
                .transpose()
                .to_cols_array_2d();
            self.rows.extend_from_slice(&rows[..3]);
        }
    }
}

impl Rig {
    pub fn car_access_clip(&self, entering: bool) -> Option<usize> {
        let clip = self.clip(if entering { "car-enter" } else { "car-exit" })?;
        (!self.clips[clip].notes.door.is_empty()).then_some(clip)
    }

    pub fn door_open_start(&self, clip: usize) -> f32 {
        self.clips[clip].notes.events.door_open_start
    }

    pub fn car_sample(&self, clip: usize, time: f32, entering: bool) -> CarSample {
        let notes = &self.clips[clip].notes;
        let frame = (time / self.seconds(clip)).clamp(0.0, 1.0) * (notes.door.len() - 1) as f32;
        let lo = frame as usize;
        let hi = (lo + 1).min(notes.door.len() - 1);
        let mix = |a: f32, b: f32| a + (b - a) * frame.fract();
        let root = std::array::from_fn(|i| mix(notes.root[lo][i], notes.root[hi][i]));
        let door = mix(notes.door[lo], notes.door[hi]);
        let ramp = |start: f32, end: f32| {
            let x = ((time - start) / (end - start)).clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        let events = &notes.events;
        let seat_weight = if entering {
            ramp(events.door_open_end, events.seated)
        } else {
            1.0 - ramp(events.unseated, events.free)
        };
        CarSample {
            root,
            door,
            seat_weight,
        }
    }

    pub fn clip(&self, name: &str) -> Option<usize> {
        self.names.get(name).copied()
    }

    pub fn seconds(&self, clip: usize) -> f32 {
        self.clips[clip].clip.duration
    }

    pub fn speed(&self, clip: usize) -> f32 {
        self.clips[clip].notes.speed
    }

    pub fn preview_time(&self, clip: usize, time: f32) -> f32 {
        let duration = self.seconds(clip);
        if duration <= 0.0 {
            0.0
        } else if self.clips[clip].notes.loops {
            time.rem_euclid(duration)
        } else {
            time.rem_euclid(duration + 0.5).min(duration)
        }
    }

    pub fn idle(&self) -> usize {
        self.names[if self.motorcycle {
            "idle"
        } else {
            "rifle-idle"
        }]
    }

    pub fn clip_count(&self) -> usize {
        self.clips.len()
    }
}

#[derive(Clone, Debug)]
pub struct PlayerBody {
    pub rig: Rig,
    pub model: u32,
    pub palette: u32,
    pub preview: Option<usize>,
    pub height: f32,
}

impl PlayerBody {
    pub fn instance(&self, position: [f32; 3], orientation: Quat) -> GpuAgent {
        GpuAgent {
            pos: position,
            rot: orientation.to_array(),
            model: self.model,
            flags: AGENT_PLAYER,
            scale: eo_data::groundscale::people() as f32,
            ..Default::default()
        }
    }
}

pub(crate) struct CookedBody {
    pub rig: Rig,
    pub tier: CookedTier,
    pub atlas: Atlas,
    pub normal_strength: f32,
}

pub(crate) fn cook(selection: &Selection) -> Result<CookedBody> {
    let manifest: Manifest = serde_json::from_str(MANIFEST)?;
    let avatar = manifest.avatar(&selection.id)?;
    let body = Glb::parse(&manifest.read(&selection.root, &avatar.body)?)?;
    let nodes = crate::rocketbox::rest_nodes(&body);
    let order = topo_nodes(&nodes);
    ensure!(order.len() == nodes.len(), "cyclic character skeleton");
    let root = nodes
        .iter()
        .position(|node| node.name.eq_ignore_ascii_case("root"))
        .ok_or_else(|| anyhow!("character skeleton has no root"))?;
    let skin = skin(&body, nodes.len())?;
    let node_ids: HashMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.name.as_str(), i))
        .collect();
    let mut clips = Vec::new();
    let mut names = HashMap::new();
    let motorcycle = avatar.packs.iter().any(|pack| pack == "motorcycle");
    for path in manifest.motions(avatar) {
        let glb = Glb::parse(&manifest.read(&selection.root, path)?)?;
        let mut notes: BTreeMap<String, ClipNotes> = match notes_path(&manifest, path) {
            Some(file) => {
                #[derive(Deserialize)]
                struct Notes {
                    clips: BTreeMap<String, ClipNotes>,
                }
                serde_json::from_slice::<Notes>(&manifest.read(&selection.root, &file)?)?.clips
            }
            None => BTreeMap::new(),
        };
        for (i, name) in manifest.files[path].clips.iter().enumerate() {
            ensure!(
                glb.json["animations"][i]["name"].as_str() == Some(name),
                "{path}: clip order mismatch"
            );
            let clip = read_glb_clip(&glb, i, name)?;
            // Extra tracks in the source packs animate props or IK targets
            // absent from a particular body. Bind only tracks on the body.
            let channels: Vec<_> = clip
                .channels
                .iter()
                .enumerate()
                .filter_map(|(channel, track)| {
                    node_ids
                        .get(track.node.as_str())
                        .map(|&node| (node, channel))
                })
                .collect();
            ensure!(
                !channels.is_empty(),
                "{name}: no channels bind to {}",
                selection.id
            );
            ensure!(
                names.insert(name.clone(), clips.len()).is_none(),
                "{name}: duplicate clip"
            );
            let notes = notes.remove(name).unwrap_or_else(|| motorcycle_notes(name));
            if matches!(name.as_str(), "car-enter" | "car-exit") {
                let e = &notes.events;
                ensure!(
                    notes.door.len() >= 2
                        && notes.root.len() == notes.door.len()
                        && notes
                            .door
                            .iter()
                            .all(|fraction| (0.0..=1.0).contains(fraction))
                        && notes.door.first() == Some(&0.0)
                        && notes.door.last() == Some(&0.0)
                        && notes
                            .root
                            .iter()
                            .flatten()
                            .all(|component| component.is_finite())
                        && if name == "car-enter" {
                            0.0 < e.door_open_start
                                && e.door_open_start < e.door_open_end
                                && e.door_open_end < e.seated
                                && e.seated <= clip.duration
                        } else {
                            0.0 < e.unseated && e.unseated < e.free && e.free <= clip.duration
                        },
                    "{name}: invalid driver-door timeline"
                );
            }
            clips.push(Motion {
                clip,
                channels,
                notes,
            });
        }
    }
    ensure!(
        names.contains_key(if motorcycle { "idle" } else { "rifle-idle" }),
        "character pack has no idle"
    );
    let rig = Rig {
        nodes,
        order,
        skin,
        root,
        clips,
        names,
        motorcycle,
    };
    let (tier, material) = mesh(&body, rig.skin.len())?;
    let (atlas, normal_strength) = atlas(&body, material)?;
    Ok(CookedBody {
        rig,
        tier,
        atlas,
        normal_strength,
    })
}

fn motorcycle_notes(name: &str) -> ClipNotes {
    let speed = match name {
        "walk" => 1.6,
        "run" => 3.5,
        _ => 0.0,
    };
    ClipNotes {
        speed,
        loops: matches!(
            name,
            "idle" | "walk" | "run" | "ride_rider" | "ride_passenger"
        ),
        ..Default::default()
    }
}

fn index(value: &Value, name: &str) -> Result<usize> {
    value[name]
        .as_u64()
        .map(|n| n as usize)
        .ok_or_else(|| anyhow!("character GLB missing {name}"))
}

fn skin(body: &Glb, node_count: usize) -> Result<Vec<(usize, Mat4)>> {
    let [skin] = body.skins() else {
        bail!("character GLB must have one skin")
    };
    let joints = skin["joints"]
        .as_array()
        .ok_or_else(|| anyhow!("character skin has no joints"))?;
    let matrices = body.mat4s(index(skin, "inverseBindMatrices")?)?;
    ensure!(
        !joints.is_empty() && joints.len() <= 256 && joints.len() == matrices.len(),
        "invalid character skin size"
    );
    joints
        .iter()
        .zip(matrices)
        .map(|(joint, matrix)| {
            let node = joint.as_u64().map_or(node_count, |n| n as usize);
            ensure!(
                node < node_count && matrix.iter().all(|v| v.is_finite()),
                "invalid character inverse bind"
            );
            Ok((node, Mat4::from_cols_array(&matrix)))
        })
        .collect()
}

fn mesh(body: &Glb, joint_count: usize) -> Result<(CookedTier, usize)> {
    let node = body
        .nodes()
        .iter()
        .find(|node| node["skin"].as_u64() == Some(0))
        .ok_or_else(|| anyhow!("character skin has no mesh"))?;
    let mesh = body
        .meshes()
        .get(index(node, "mesh")?)
        .ok_or_else(|| anyhow!("invalid character mesh index"))?;
    let Some([p]) = mesh["primitives"].as_array().map(Vec::as_slice) else {
        bail!("character must have one atlas primitive")
    };
    ensure!(
        p["mode"].as_u64().unwrap_or(4) == 4,
        "character mesh must be triangles"
    );
    let attr = |name| index(&p["attributes"], name);
    let pos = body.floats(attr("POSITION")?)?;
    let normal = body.floats(attr("NORMAL")?)?;
    let uv = body.floats(attr("TEXCOORD_0")?)?;
    let joints = body.uints(attr("JOINTS_0")?)?;
    let weights = body.floats(attr("WEIGHTS_0")?)?;
    let count = pos.len();
    ensure!(
        count > 0 && [normal.len(), uv.len(), joints.len(), weights.len()] == [count; 4],
        "character vertex counts differ"
    );
    let mut tier = CookedTier {
        min: [f32::MAX; 3],
        max: [f32::MIN; 3],
        ..Default::default()
    };
    for i in 0..count {
        let [x, y, z, _] = pos[i];
        let position = Vec3::new(x, y, z);
        ensure!(
            position.is_finite()
                && joints[i]
                    .iter()
                    .all(|&joint| (joint as usize) < joint_count),
            "invalid character vertex"
        );
        tier.min = position.min(Vec3::from(tier.min)).to_array();
        tier.max = position.max(Vec3::from(tier.max)).to_array();
        let sum: f32 = weights[i].iter().sum();
        ensure!(
            sum.is_finite() && sum > 0.0 && weights[i].iter().all(|&w| w >= 0.0),
            "invalid character skin weights"
        );
        let mut packed = weights[i].map(|weight| (255.0 * weight / sum).round() as i32);
        let largest = (0..4).max_by_key(|&j| packed[j]).unwrap();
        packed[largest] += 255 - packed.iter().sum::<i32>();
        let [nx, ny, nz, _] = normal[i];
        tier.vertices.push(AgentVertex {
            pos: position.to_array(),
            normal: crate::vehicles::pack_normal(Vec3::new(nx, ny, nz)),
            uv: std::array::from_fn(|j| (uv[i][j].clamp(0.0, 1.0) * 65535.0).round() as u16),
            joints: joints[i].map(|joint| joint as u8),
            weights: packed.map(|w| w as u8),
            tag: 0,
        });
    }
    tier.indices = body
        .uints(index(p, "indices")?)?
        .into_iter()
        .map(|v| v[0])
        .collect();
    ensure!(
        !tier.indices.is_empty()
            && tier.indices.len() % 3 == 0
            && tier.indices.iter().all(|&i| (i as usize) < count),
        "invalid character triangle indices"
    );
    Ok((tier, index(p, "material")?))
}

fn atlas(body: &Glb, material: usize) -> Result<(Atlas, f32)> {
    let material = body
        .materials()
        .get(material)
        .ok_or_else(|| anyhow!("invalid character material"))?;
    let pbr = &material["pbrMetallicRoughness"];
    let normal = &material["normalTexture"];
    let albedo = decode_texture(body, index(&pbr["baseColorTexture"], "index")?)?;
    let normal_map = decode_texture(body, index(normal, "index")?)?;
    let orm = decode_texture(body, index(&pbr["metallicRoughnessTexture"], "index")?)?;
    let factors = [
        material["occlusionTexture"]["strength"].as_f64().unwrap_or(
            if material["occlusionTexture"].is_object() {
                1.0
            } else {
                0.0
            },
        ) as f32,
        pbr["roughnessFactor"].as_f64().unwrap_or(1.0) as f32,
        pbr["metallicFactor"].as_f64().unwrap_or(1.0) as f32,
    ];
    let mask = material["alphaMode"].as_str() == Some("MASK");
    Ok((
        crate::rocketbox::material_atlas([albedo, normal_map, orm], mask, factors),
        normal["scale"].as_f64().unwrap_or(1.0) as f32,
    ))
}

fn decode_texture(body: &Glb, texture: usize) -> Result<(u32, u32, Vec<u8>)> {
    let (mime, bytes) = body.texture_bytes(texture)?;
    if mime == "image/jpeg" {
        let (w, h, rgb) = eo_assets::jpeg::decode_rgb(&bytes)?;
        return Ok((
            w as u32,
            h as u32,
            rgb.chunks_exact(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
        ));
    }
    ensure!(mime == "image/png", "unsupported character texture {mime}");
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .context("character texture too large")?
    ];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => bail!(
            "unsupported character PNG colour type {:?}",
            info.color_type
        ),
    };
    Ok((info.width, info.height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_demo_roster_contains_every_current_clip() {
        let roster = catalogue().unwrap();
        assert_eq!(roster.len(), 7);
        let main: Vec<_> = roster
            .iter()
            .filter(|(name, _)| !name.starts_with("motorcycle-"))
            .collect();
        let names = |clips: &[String]| {
            clips
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(names(&main[0].1).len(), 110);
        for (_, clips) in main {
            assert_eq!(names(clips), names(&roster[0].1));
        }
        for (_, clips) in roster
            .iter()
            .filter(|(name, _)| name.starts_with("motorcycle-"))
        {
            assert_eq!(names(clips).len(), 15);
        }
    }

    #[test]
    fn demo_glbs_cook_and_every_clip_poses_its_skin() -> Result<()> {
        let Some(root) = std::env::var_os(ROOT_ENV).map(PathBuf::from) else {
            eprintln!("set {ROOT_ENV} to exercise the local demo GLBs");
            return Ok(());
        };
        for (id, clips) in catalogue()? {
            let selection = Selection::new(root.clone(), id.clone(), None)?;
            let body = cook(&selection).with_context(|| id.clone())?;
            let mut pose = Pose::default();
            for name in &clips {
                let clip = body
                    .rig
                    .clip(name)
                    .with_context(|| format!("{id} missing {name}"))?;
                let duration = body.rig.seconds(clip);
                for t in [0.0, duration * 0.5, duration] {
                    pose.sample(&body.rig, clip, t, true, 1.0);
                    assert_eq!(pose.rows.len(), body.rig.skin.len() * 3);
                    assert!(
                        pose.rows.iter().flatten().all(|value| value.is_finite()),
                        "{id}: {name} at {t}"
                    );
                }
            }
            if let Some(enter) = body.rig.clip("car-enter") {
                pose.sample(&body.rig, enter, 0.0, true, 1.0);
                let with_root = pose.rows.clone();
                pose.sample(&body.rig, enter, 0.0, false, 1.0);
                assert_ne!(
                    pose.rows, with_root,
                    "{id}: preview retains the authored entry path"
                );
                for entering in [true, false] {
                    let clip = body.rig.car_access_clip(entering).unwrap();
                    let duration = body.rig.seconds(clip);
                    let first = body.rig.car_sample(clip, 0.0, entering);
                    let last = body.rig.car_sample(clip, duration, entering);
                    assert_eq!((first.door, last.door), (0.0, 0.0));
                    assert_eq!(
                        (first.seat_weight, last.seat_weight),
                        if entering { (0.0, 1.0) } else { (1.0, 0.0) }
                    );
                    assert!((1..60).any(|i| {
                        body.rig
                            .car_sample(clip, duration * i as f32 / 60.0, entering)
                            .door
                            > 0.99
                    }));
                }
            }
        }
        Ok(())
    }
}
