//! Site and collision integration for the Blender-built terrace at 21 Beaver Street.
mod dimensions;
mod fitted;
mod fitted_board;
mod window_linen;
mod table_light;
mod oak;
use table_light::TableLight;
mod door;
mod lod;
mod planting;
mod porch;
mod stoop;
mod street_garden;
mod roof;
mod surfaces;
pub use surfaces::Scan;
use anyhow::{Result, ensure};
pub use door::Door;
use eo_assets::{AssetRef, CookedModel, KitLibrary};
use eo_player::world::{GroundProvider, SurfaceMesh, TileKey};
use eo_render::authored::MaterialTable;
use eo_render::scene::{BLEND_BIT, GpuObject, ObjectKind, Scene, Vertex};
use glam::{DVec2, DVec3, Mat3, Vec3};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

const FINISH_DIR: &str = "public/assets/buildings/beaver-terrace/finishes";
const FLOOR_MATERIALS: [&str; 4] = [
    "oiled-oak@ba9a79",
    "oiled-oak@b49575",
    "oiled-oak@ba9e82",
    "oiled-oak@b6987a",
];
pub const ID: &str = "266146886";
pub fn owns(id: &str) -> bool {
    id.trim_start_matches("sf-building-") == ID
}

type Point = [f64; 3];
#[derive(Deserialize)]
struct Manifest {
    #[serde(flatten)]
    asset: AssetRef,
    door: DoorSpec,
    boxes: Vec<[Point; 2]>,
    ramps: Vec<[Point; 4]>,
    #[serde(default)]
    metric_assemblies: Vec<dimensions::MetricAssembly>,
    #[serde(default)]
    ground_components: Vec<GroundComponent>,
    #[serde(default)]
    rear_parcel: Option<RearParcel>,
}

#[derive(Deserialize)]
struct RearParcel {
    survey_xz: [[f64; 2]; 4],
}

#[derive(Deserialize)]
struct GroundComponent {
    name: String,
    anchor: Point,
    #[serde(default)]
    box_index: Option<usize>,
    #[serde(default)]
    survey_xz: Option<[f64; 2]>,
}
#[derive(Deserialize)]
struct DoorSpec {
    hinge: Point,
    bounds: [Point; 2],
    materials: Vec<String>,
}

#[derive(Clone, Deserialize)]
struct ProbeVolume {
    min: [f32; 3],
    max: [f32; 3],
}

#[derive(Deserialize)]
struct ProbeManifest {
    #[serde(flatten)]
    asset: AssetRef,
    house_sha256: String,
    probes: [ProbeVolume; 2],
}

#[derive(Deserialize)]
struct RoomLight {
    #[serde(flatten)]
    asset: AssetRef,
    mean_sky_return: f32,
    exposure_range_ev: f32,
    exterior_exposed_beam: f32,
}

pub struct Finishes {
    fitted: Option<fitted::Recipe>,
    fitted_maps: Vec<eo_assets::kit::BakedMaps>,
    garden: Option<street_garden::Garden>,
    stoop: Option<stoop::Recipe>,
    stoop_maps: Vec<eo_assets::kit::BakedMaps>,
    garden_maps: Vec<eo_assets::kit::BakedMaps>,
    front_daylight: Option<eo_render::front_daylight::FrontDaylight>,
    wall_daylight: Option<eo_assets::kit::BakedMaps>,
    window_optics: Option<Arc<eo_render::window_optics::WindowOptics>>,
    joinery_light: bool,
    porch_light: Option<(porch::Lamp, eo_assets::kit::BakedMaps)>,
    site_camera_exposure: Option<f32>,
    downstairs_light: bool,
    dining_meter: Option<[f32; 3]>,
    dining_camera_exposure: Option<f32>,
    table_light: Option<TableLight>,
    oak_finish: Option<oak::Finish>,
    dining_lamp: Option<[f32; 4]>,
    mission_roof: bool,
    room_light: Option<RoomLight>,
    glazing: bool,
    furniture: bool,
    pub scans: Vec<Scan>,
    pub floor_reflections: Option<eo_assets::kit::BakedMaps>,
    pub yard_reflections: Option<eo_assets::kit::BakedMaps>,
    pub street_reflections: Option<eo_assets::kit::BakedMaps>,
    directory: std::path::PathBuf,
    manifest: ProbeManifest,
    pub atlas: eo_assets::kit::BakedMaps,
    pub floorboards: eo_assets::kit::BakedMaps,
}

impl Finishes {
    pub fn load(root: &Path, args: &crate::Args) -> Result<Self> {
        let (patina, furniture, photo) = (args.house_patina, args.house_furniture, args.house_photo);
        let glazing = args.house_glazing;
        let directory = if patina {
            root.join(FINISH_DIR).join("patina")
        } else {
            root.join(FINISH_DIR)
        };
        let photo_dir = directory.join("furniture/photo");
        let blender_bedroom = args.house_blender_bedroom && photo && furniture && patina;
        let fitted_enabled = args.house_fitted_dining && photo && furniture && patina;
        let dining_dir = if fitted_enabled { photo_dir.join("fitted-dining") } else { photo_dir.clone() };
        let dining_light = (args.house_dining_light || fitted_enabled) && args.interior_lighting && photo && furniture && patina;
        let downstairs_light = (args.house_downstairs_light || fitted_enabled) && args.interior_lighting && photo && furniture && patina;
        let table_enabled = !fitted_enabled && (args.house_table_light || args.house_oak_joinery) && args.interior_lighting && photo && furniture && patina;
        let mission_roof = args.house_mission_roof && photo && furniture && patina;
        let mut scans = if patina {
            surfaces::NAMES
                .iter()
                .map(|name| {
                    let source = if furniture && photo && matches!(*name, "limewash" | "fired-earth") {
                        &photo_dir
                    } else {
                        &directory
                    };
                    Scan::load(root, source, name)
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![]
        };
        let room_light = if blender_bedroom {
            let light: RoomLight = serde_json::from_slice(&std::fs::read(
                photo_dir.join("bedroom-daylight.provenance.json"),
            )?)?;
            ensure!((0.0..1.0).contains(&light.mean_sky_return)
                && (0.0..8.0).contains(&light.exposure_range_ev) && light.exterior_exposed_beam > 0., "invalid bedroom daylight calibration");
            Some(light)
        } else {
            None
        };
        let mut street_reflections = if patina {
            let panorama: AssetRef = serde_json::from_slice(&std::fs::read(
                if photo && glazing { photo_dir.join("street-reflections.provenance.json") }
                else { directory.join("street-reflections.provenance.json") },
            )?)?;
            let mut maps = Self::read_maps(root, &panorama, 2)?;
            if let Some(light) = &room_light {
                // The two panorama images occupy the color and normal arrays.
                // Their third array slot can hold the linear light-transfer grid.
                maps.orm = Self::read_pixels(root, &light.asset, 1)?;
            }
            Some(maps)
        } else {
            None
        };
        let directory = if furniture {
            let variant = directory.join("furniture");
            for name in surfaces::FURNITURE_NAMES {
                scans.push(Scan::load(root, &variant, name)?);
            }
            if photo {
                for name in surfaces::SITE_NAMES {
                    scans.push(Scan::load(root, &photo_dir, name)?);
                }
                if mission_roof {
                    scans.push(Scan::load(root, &photo_dir, "mission-clay")?);
                }
                if blender_bedroom {
                    scans.insert(0, Scan::load(root, &photo_dir, "washed-cotton")?);
                }
                photo_dir.clone()
            } else {
                variant
            }
        } else {
            directory
        };
        let manifest: ProbeManifest = serde_json::from_slice(&std::fs::read(
            directory.join("kitchen-reflections.provenance.json"),
        )?)?;
        let meter_enabled = args.house_dining_meter && args.interior_lighting && photo && furniture && patina;
        let mut dining_meter = None;
        if downstairs_light || table_enabled || meter_enabled {
            #[derive(Deserialize)]
            struct DownstairsLight {
                #[serde(flatten)]
                asset: AssetRef,
                house_sha256: String,
                grid: [usize; 3],
                atlas_row_offset: usize,
                transfer_encoding: String,
                mean_sky_return: f32,
                exposure_range_ev: f32,
                exterior_exposed_beam: f32,
            }
            let light: DownstairsLight = serde_json::from_slice(&std::fs::read(
                dining_dir.join("downstairs-daylight.provenance.json"),
            )?)?;
            let res = eo_assets::kit::HIGH_RES;
            ensure!(light.house_sha256 == manifest.house_sha256 && light.grid == [15, 6, 33]
                && light.atlas_row_offset == res / 2 && light.transfer_encoding == "sqrt(L/(1+L))",
                "unexpected downstairs lighting layout");
            if meter_enabled {
                ensure!((0.0..1.0).contains(&light.mean_sky_return) && (0.0..8.0).contains(&light.exposure_range_ev)
                    && light.exterior_exposed_beam > 0., "invalid ground-floor exposure calibration");
                dining_meter = Some([light.mean_sky_return, light.exposure_range_ev, light.exterior_exposed_beam]);
            }
            if downstairs_light || table_enabled {
                let pixels = Self::read_pixels(root, &light.asset, 1)?;
                let split = light.atlas_row_offset * res * 4;
                // The bedroom occupies rows 0..252. The downstairs survey sits in the lower half.
                let street = street_reflections.as_mut().ok_or_else(|| anyhow::anyhow!("missing house reflection atlas"))?;
                street.orm[split..].copy_from_slice(&pixels[split..]);
            }
        }
        let mut table_light = if table_enabled {
            let street = street_reflections.as_mut().ok_or_else(|| anyhow::anyhow!("missing house reflection atlas"))?;
            Some(TableLight::install(root, &directory, &manifest.house_sha256, &mut street.orm)?)
        } else { None };
        let mut yard_reflections = if furniture && photo {
            let panorama: AssetRef = serde_json::from_slice(&std::fs::read(
                if glazing { directory.join("yard-reflections.provenance.json") }
                else { root.join(FINISH_DIR).join("patina/street-reflections.provenance.json") },
            )?)?;
            Some(Self::read_maps(root, &panorama, 2)?)
        } else {
            None
        };
        let floor: AssetRef = serde_json::from_slice(&std::fs::read(
            if furniture && photo { photo_dir.join("floorboards.provenance.json") }
            else { root.join(FINISH_DIR).join("floorboards.provenance.json") },
        )?)?;
        let mut atlas = Self::read_maps(root, &manifest.asset, 2)?;
        let dining_lamp = if dining_light {
            #[derive(Deserialize)]
            struct FloorLight {
                #[serde(flatten)]
                asset: AssetRef,
                house_sha256: String,
                lamp_parcel_xyz: [f32; 3],
                lamp_radius_m: f32,
            }
            let light: FloorLight = serde_json::from_slice(&std::fs::read(
                dining_dir.join("dining-floor-light.provenance.json"),
            )?)?;
            ensure!(light.house_sha256 == manifest.house_sha256, "dining light was baked for a different house");
            let mut pages = Self::read_pixels(root, &light.asset, 2)?;
            // The panoramas use color and normal only. Bind the two linear
            // light pages in their otherwise empty ORM slots.
            atlas.orm = pages.split_off(pages.len() / 2);
            yard_reflections.as_mut().ok_or_else(|| anyhow::anyhow!("missing courtyard layer for dining light"))?.orm = pages;
            let [x,y,z] = light.lamp_parcel_xyz;
            ensure!((0.0..7.0).contains(&x) && (0.0..3.3).contains(&y) && (0.0..13.0).contains(&z)
                && (0.0..1.0).contains(&light.lamp_radius_m), "invalid dining pendant position or radius");
            Some([x,y,z,light.lamp_radius_m])
        } else {
            None
        };
        let floor_reflections = if args.house_floor_reflections && args.interior_lighting && photo && furniture && patina {
            Some(oak::floor_reflections(root, &dining_dir, &manifest.house_sha256)?)
        } else { None };
        let dining_camera_exposure = if meter_enabled && args.look.blender_camera {
            Some(oak::dining_exposure(&dining_dir, &manifest.house_sha256)?)
        } else { None };
        let mut floorboards = Self::read_maps(root, &floor, 3)?;
        let oak_finish = if args.house_oak_finish && args.interior_lighting && photo && furniture && patina {
            Some(oak::Finish::load(root, &directory, &manifest.house_sha256, &mut floorboards.orm)?)
        } else { None };
        if args.house_oak_joinery && let Some(table) = table_light.as_mut() {
            let street = street_reflections.as_mut().ok_or_else(|| anyhow::anyhow!("missing dining light atlas"))?;
            table.install_joinery(root, &directory, &manifest.house_sha256, args.house_table_light,
                &mut street.orm, &mut floorboards.orm)?;
        }
        let (fitted, fitted_maps) = if fitted_enabled {
            let (recipe, maps) = fitted::Recipe::read(root, &dining_dir, &manifest.house_sha256, args.house_fitted_finishes && args.interior_lighting, args.house_kitchen_light && args.interior_lighting, args.house_window_linen, args.interior_lighting)?;
            (Some(recipe), maps)
        } else { (None, Vec::new()) };
        let (garden, garden_maps) = if args.house_street_garden && photo && furniture && patina {
            let (recipe, maps) = street_garden::Garden::read(root, &photo_dir, &manifest.house_sha256)?;
            (Some(recipe), maps)
        } else { (None, vec![]) };
        let front_daylight = garden.as_ref().filter(|_| args.house_front_daylight)
            .map(|g| g.read_daylight(root, &photo_dir)).transpose()?;
        let window_optics = if (args.house_window_optics && glazing || args.house_joinery_light) && photo && patina && furniture {
            Some(Arc::new(Self::read_window_optics(root, &photo_dir, &manifest.house_sha256)?))
        } else { None };
        let (stoop, stoop_maps) = if args.house_fired_stoop && photo && patina && furniture {
            let (recipe,maps)=stoop::Recipe::read(root,&photo_dir,&manifest.house_sha256)?;
            (Some(recipe),maps)
        } else { (None,Vec::new()) };
        let porch_light = if args.house_porch_lamp && photo && patina && furniture {
            Some(porch::Lamp::read(root, &photo_dir, &manifest.house_sha256)?)
        } else { None };
        let camera = args.house_site_camera && args.look.blender_camera;
        let (wall_daylight, site_camera_exposure) = if (args.house_wall_daylight || camera) && photo && furniture && patina {
            Self::read_wall_daylight(root, &photo_dir, &manifest.house_sha256, args.house_wall_daylight, camera)?
        } else { (None, None) };
        Ok(Self {
            stoop, stoop_maps, porch_light, joinery_light: args.house_joinery_light, window_optics, site_camera_exposure, wall_daylight, front_daylight, garden, garden_maps, fitted, fitted_maps, downstairs_light,
            dining_meter,
            dining_camera_exposure,
            oak_finish,
            table_light,
            dining_lamp,
            mission_roof,
            room_light,
            glazing,
            furniture,
            directory,
            scans,
            floor_reflections,
            yard_reflections,
            street_reflections,
            manifest,
            atlas,
            floorboards,
        })
    }

    pub fn into_maps(self) -> Vec<eo_assets::kit::BakedMaps> {
        let mut maps: Vec<_> = self.scans.into_iter().map(|scan| scan.maps).collect();
        maps.extend(self.wall_daylight);
        maps.extend(self.porch_light.map(|(_, maps)| maps));
        maps.extend(self.garden_maps);
        maps.extend(self.stoop_maps);
        maps.extend(self.fitted_maps);
        maps.extend(self.floor_reflections);
        maps.extend(self.yard_reflections);
        maps.extend(self.street_reflections);
        maps.push(self.floorboards);
        maps.push(self.atlas);
        maps
    }

    fn read_window_optics(root: &Path, directory: &Path, shell: &str) -> Result<eo_render::window_optics::WindowOptics> {
        #[derive(Deserialize)]
        struct Recipe {
            #[serde(flatten)]
            asset: AssetRef,
            house_sha256: String,
            payload_bytes: usize,
            encoding: String,
            width: u32,
            height: u32,
        }
        let recipe: Recipe = serde_json::from_slice(&std::fs::read(directory.join("window-optics.provenance.json"))?)?;
        ensure!(recipe.house_sha256 == shell && recipe.encoding == "BWO1" && recipe.width == 1024
            && recipe.height > 0 && recipe.height <= 32768, "incompatible window scene");
        let size = (recipe.width * recipe.height * 4) as usize;
        ensure!(recipe.payload_bytes <= size && size - recipe.payload_bytes < 4096, "invalid optical scene padding");
        let bytes = eo_assets::glb::read_verified(&recipe.asset.path(root), &recipe.asset)?;
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
        ensure!(reader.info().width == recipe.width && reader.info().height == recipe.height
            && reader.info().color_type == png::ColorType::Rgba && reader.info().bit_depth == png::BitDepth::Eight,
            "invalid window scene image");
        let mut data = vec![0; size];
        reader.next_frame(&mut data)?;
        data.truncate(recipe.payload_bytes);
        eo_render::window_optics::WindowOptics::from_bytes(data)
    }

    fn read_wall_daylight(root: &Path, directory: &Path, shell: &str, walls: bool, camera: bool) -> Result<(Option<eo_assets::kit::BakedMaps>, Option<f32>)> {
        #[derive(Deserialize)]
        struct Survey {
            #[serde(flatten)]
            asset: AssetRef,
            house_sha256: String,
            hours: [u8; 6],
            charts: [String; 16],
            source_exposure_ev: f32,
            linear_game_units_per_source_unit: f32,
            display_meter_gain: f32,
            cell_size: usize,
            pigment_cell_size: usize,
            encoding: String,
        }
        let survey: Survey = serde_json::from_slice(&std::fs::read(directory.join("wall-daylight.provenance.json"))?)?;
        ensure!(survey.house_sha256 == shell && survey.hours == [8,10,12,14,16,18]
            && survey.cell_size == 64 && survey.pigment_cell_size == 128 && survey.encoding == "sqrt(L/(1+L))"
            && survey.charts == ["Narrow side passage", "Opposite side passage", "Street wall", "Rear wall",
                "Round stair bay", "Entry arch face", "Entry east reveal", "Entry west reveal", "Street gable",
                "Upper casement backing", "Arcade moulding face", "First arch intrados", "Middle arch intrados",
                "Third arch intrados", "Arcade sill underside", "Porch arch intrados"], "incompatible plaster survey");
        let exposure = survey.source_exposure_ev.exp2() / (survey.linear_game_units_per_source_unit * survey.display_meter_gain);
        ensure!(exposure.is_finite() && exposure > 0., "invalid exterior camera calibration");
        let exposure = camera.then_some(exposure);
        if !walls { return Ok((None, exposure)); }
        let mut orm = Self::read_pixels(root, &survey.asset, 3)?;
        let page = orm.len()/3;
        let albedo = orm.split_off(2*page);
        let normal = orm.split_off(page);
        let maps = eo_assets::kit::BakedMaps { res: eo_assets::kit::HIGH_RES, metres_per_tile: 1.,
            albedo, normal, orm };
        Ok((Some(maps), exposure))
    }

    fn read_pixels(root: &Path, asset: &AssetRef, pages: usize) -> Result<Vec<u8>> {
        let bytes = eo_assets::glb::read_verified(&asset.path(root), asset)?;
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
        let res = eo_assets::kit::HIGH_RES;
        let info = reader.info();
        ensure!(
            info.width as usize == res
                && info.height as usize == pages * res
                && info.color_type == png::ColorType::Rgba
                && info.bit_depth == png::BitDepth::Eight,
            "unexpected image format for {}",
            asset.url
        );
        let size = res * res * 4;
        let mut albedo = vec![0; pages * size];
        reader.next_frame(&mut albedo)?;
        Ok(albedo)
    }

    fn read_maps(root: &Path, asset: &AssetRef, pages: usize) -> Result<eo_assets::kit::BakedMaps> {
        let res = eo_assets::kit::HIGH_RES;
        let size = res * res * 4;
        let mut albedo = Self::read_pixels(root, asset, pages)?;
        let orm = if pages == 3 {
            albedo.split_off(size * 2)
        } else {
            vec![0; size]
        };
        let normal = albedo.split_off(size);
        Ok(eo_assets::kit::BakedMaps {
            res,
            metres_per_tile: 2.,
            normal,
            orm,
            albedo,
        })
    }
}

const FACES: [[usize; 4]; 6] = [
    [0, 3, 2, 1],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [3, 7, 6, 2],
    [0, 4, 7, 3],
    [1, 2, 6, 5],
];
fn corners([x, y, z]: Point, [u, v, w]: Point) -> [Point; 8] {
    [
        [x, y, z],
        [u, y, z],
        [u, v, z],
        [x, v, z],
        [x, y, w],
        [u, y, w],
        [u, v, w],
        [x, v, w],
    ]
}

pub struct House {
    fitted: Option<fitted::Room>,
    garden: Option<fitted::Geometry>,
    stoop: Option<stoop::Stoop>,
    pub front_daylight: Option<eo_render::front_daylight::FrontDaylight>,
    pub wall_daylight_layer: Option<usize>,
    porch_light: Option<(porch::Lamp, usize)>,
    pub window_optics: Option<Arc<eo_render::window_optics::WindowOptics>>,
    site_camera_exposure: Option<f32>,
    origin: DVec2,
    across: DVec2,
    back: DVec2,
    floor: f64,
    bedside: Point,
    height_scale: f64,
    model: CookedModel,
    door: DoorSpec,
    materials: Vec<u32>,
    light_transfer: Vec<[f32; 2]>,
    visibility: Vec<f32>,
    cut_pattern: eo_assets::glb::CutPattern,
    table_vertices: eo_assets::glb::CutPattern,
    roof_uv: Option<roof::RoofUv>,
    room_meter: Option<[f32; 3]>,
    downstairs_light: bool,
    dining_meter: Option<[f32; 3]>,
    dining_camera_exposure: Option<f32>,
    table_light: Option<TableLight>,
    oak_finish: Option<oak::Finish>,
    dining_lamp: Option<[f32; 4]>,
    floor_reflections: bool,
    probes: Option<[ProbeVolume; 2]>,
    collision: Vec<[Point; 3]>,
    steps: Vec<[Point; 2]>,
    pavers: Vec<([Point; 4], u32)>,
    mortar: Vec<([Point; 4], u32)>,
    aggregate: Vec<([Point; 4], u32)>,
    aggregate_material: u32,
    yard: Vec<([Point; 4], u32)>,
    yard_material: u32,
    rear_stucco: Vec<([Point; 4], u32)>,
    footing: Vec<([Point; 4], u32)>,
    rear_stucco_material: u32,
    planting: planting::Planting,
    concrete_material: u32,
    paver_material: u32,
    patina: bool,
    photo: bool,
    glazing: bool,
    entry: Option<DVec3>,
}

impl House {
    pub fn build(
        interior_lighting: bool,
        finishes: Option<&Finishes>,
        b: &eo_data::Building,
        ground: &dyn Fn(f64, f64) -> f64,
        street: Option<&eo_street::StreetWorld>,
        root: &Path,
        kit: &KitLibrary,
        materials: &mut MaterialTable,
    ) -> Result<Self> {
        ensure!(b.ring.len() == 4, "unexpected 21 Beaver footprint");
        let ring: Vec<_> = b
            .ring
            .iter()
            .map(|p| DVec2::new(p[0] as f64, p[1] as f64))
            .collect();
        let dir = finishes.map_or_else(
            || root.join("public/assets/buildings/beaver-terrace"),
            |f| f.directory.clone(),
        );
        let patina = finishes.is_some_and(|f| !f.scans.is_empty());
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(dir.join("terrace.provenance.json"))?)?;
        if let Some(r) = finishes {
            ensure!(
                r.manifest.house_sha256 == manifest.asset.sha256,
                "kitchen reflections were baked for a different house"
            );
        }
        let raw = eo_assets::glb::read_verified(&manifest.asset.path(root), &manifest.asset)?;
        let across = (ring[2] - ring[1]) / 7.;
        let back = (ring[0] - ring[1]) / 13.;
        let height_scale = b.height as f64 / 10.;
        let floor = ring
            .iter()
            .map(|p| ground(p.x, p.y))
            .fold(f64::NEG_INFINITY, f64::max)
            + 0.22;
        let mut layout = dimensions::Layout::new(&manifest.metric_assemblies, manifest.boxes.len(), across, back, height_scale)?;
        // The shell sits on the high corner of the footprint. Each freestanding
        // planter instead stands on its own sampled yard grade, mesh and collider together.
        for component in &manifest.ground_components {
            let [x, y, z] = component.anchor;
            let anchor = DVec3::new(x, y, z);
            let planned = ring[1] + across * x + back * z;
            let transform = if let Some(survey) = component.survey_xz {
                // Use the same decimetre rounding as the DataSF street-tree loader.
                let scale = eo_data::groundscale::scale();
                let site = (DVec2::from(survey) * (10. * scale)).round() * 0.1;
                let det = across.perp_dot(back);
                let local = site - ring[1];
                let rooted = DVec3::new(local.perp_dot(back) / det,
                    (ground(site.x, site.y) - floor) / height_scale, across.perp_dot(local) / det);
                // The inventory's crown and height are physical metres. Only the
                // root follows the mapped ground plan, including at alternate scales.
                let axis = |v: DVec2| DVec3::new(v.x, 0., v.y);
                let site_axes = glam::DMat3::from_cols(axis(across), DVec3::Y*height_scale, axis(back));
                let along = across.normalize();
                let metres = glam::DMat3::from_cols(axis(along), DVec3::Y,
                    axis(along.perp() * det.signum()));
                glam::DMat4::from_translation(rooted) * glam::DMat4::from_mat3(site_axes.inverse()*metres)
                    * glam::DMat4::from_translation(-anchor)
            } else {
                let dy = (ground(planned.x, planned.y) - floor) / height_scale - y;
                glam::DMat4::from_translation(DVec3::Y * dy)
            };
            ensure!(layout.nodes.insert(component.name.clone(), transform.as_mat4()).is_none(),
                    "duplicate grounded component {}", component.name);
            if let Some(index) = component.box_index {
                let slot = layout.boxes.get_mut(index)
                    .ok_or_else(|| anyhow::anyhow!("missing planter collision box"))?;
                ensure!(slot.is_none(), "grounded collision box already placed");
                *slot = Some(transform);
            }
        }
        let fitted_recipe = finishes.and_then(|f| f.fitted.as_ref());
        let furnished = finishes.is_some_and(|f| f.furniture);
        let room_meter = finishes.and_then(|f| f.room_light.as_ref())
            .map(|light| [light.mean_sky_return, light.exposure_range_ev, light.exterior_exposed_beam]);
        let mut cut_pattern = eo_assets::glb::CutPattern::new();
        let mission_roof = finishes.is_some_and(|f| f.mission_roof);
        let (mut model, light_transfer, visibility) = if furnished {
            ensure!(!layout.nodes.is_empty(), "furnished house is missing its metric placement");
            let garden = finishes.and_then(|f| f.garden.as_ref());
            let skip = |name: &str| (mission_roof && roof::old_clay(name))
                || (finishes.is_some_and(|f| f.stoop.is_some()) && stoop::Stoop::replaces(name))
                || (fitted_recipe.is_some() && fitted::replaced(name))
                || garden.is_some_and(|g| g.replaces(name));
            let (model, uv, contact, pattern) = eo_assets::glb::cook_furnished_glb_filtered(
                &raw, &layout.nodes, room_meter.is_some(), Some(&skip),
            )?;
            cut_pattern = pattern;
            if interior_lighting { (model, uv, contact) } else { (model, vec![], vec![]) }
        } else if interior_lighting {
            let (model, uv) = eo_assets::glb::cook_glb_with_uv0(&raw)?;
            (model, uv, Vec::new())
        } else {
            (eo_assets::glb::cook_glb(&raw)?, Vec::new(), Vec::new())
        };
        if let Some(recipe) = finishes.and_then(|f| f.stoop.as_ref()) {
            recipe.extend_piers(&mut model, |x,z| {
                let site=ring[1]+across*x+back*z;
                ((ground(site.x,site.y)-floor-0.06)/height_scale).min(-0.005)
            });
        }
        let roof_uv = mission_roof.then(|| roof::install(root, &dir, &mut model)).transpose()?;
        let resolved: Vec<u32> = model
            .materials
            .iter()
            .map(|m| {
                let original = finishes
                    .and_then(|f| Scan::material(&f.scans, m, materials, !cut_pattern.is_empty()))
                    .unwrap_or_else(|| materials.resolve(kit, m));
                if m.name.starts_with("porch-opal@") {
                    if let Some((lamp, _)) = finishes.and_then(|f| f.porch_light.as_ref()) {
                        let mut material = materials.gpu[original as usize];
                        material.emissive[..3].copy_from_slice(&lamp.emission());
                        return materials.named("21-beaver/opal-porch-lamp", material);
                    }
                }
                if finishes.is_some_and(|f| f.joinery_light && f.window_optics.is_some()) {
                    let family = match m.name.split('@').next().unwrap_or("") {
                        "oil-rubbed-wood" => 1, "sash-putty" => 2, "wrought-iron" => 3,
                        "front-door-paint" => 4, "front-door-moulding" => 5, _ => 0,
                    };
                    if family != 0 {
                        let flags = eo_render::authored::F_JOINERY_LIGHT | (family << 8);
                        // House scan layers receive their final atlas offset after all
                        // city materials are loaded. Keep that house-owned slot.
                        if materials.names[original as usize].starts_with("21-beaver-scan/") {
                            materials.gpu[original as usize].flags |= flags;
                            return original;
                        }
                        let mut lit = materials.gpu[original as usize];
                        lit.flags |= flags;
                        return materials.named(&format!("beaver/joinery-light/{}", m.name), lit);
                    }
                }
                original
            })
            .collect();
        let aggregate_material = model
            .materials
            .iter()
            .position(|m| m.name == "decomposed-granite@b4a087")
            .map_or(0, |i| resolved[i]);
        let photo = finishes.is_some_and(|f| f.yard_reflections.is_some());
        let paver_family = if photo { "terracotta-paver@" } else { "fired-clay@" };
        let paver_material = model
            .materials
            .iter()
            .position(|m| m.name.starts_with(paver_family))
            .filter(|_| patina)
            .map_or(0, |i| resolved[i]);
        let site_material = |family: &str| model.materials.iter()
            .position(|m| m.name.starts_with(family)).map(|i| resolved[i]);
        let (yard_material, concrete_material) = if photo {
            (site_material("garden-topsoil@").ok_or_else(|| anyhow::anyhow!("yard soil missing"))?,
             site_material("cast-stone@").ok_or_else(|| anyhow::anyhow!("private apron concrete missing"))?)
        } else { (0,0) };
        let rear_stucco_material = if photo {
            site_material("painted-stucco@").ok_or_else(|| anyhow::anyhow!("rear stucco missing"))?
        } else { 0 };
        let table_light = finishes.and_then(|f| f.table_light.clone());
        let table_vertices = table_light.as_ref().map(|s| s.vertices(&model)).transpose()?.unwrap_or_default();
        let scans = finishes.map_or(0, |f| f.scans.len());
        let front_daylight = finishes.and_then(|f| f.front_daylight.clone());
        let wall_daylight_layer = finishes.filter(|f| f.wall_daylight.is_some()).map(|_| scans);
        let after_wall = scans + usize::from(wall_daylight_layer.is_some());
        let porch_light = finishes.and_then(|f| f.porch_light.as_ref()).map(|(lamp, _)| (lamp.clone(), after_wall));
        let garden_base = after_wall + usize::from(porch_light.is_some());
        let garden = finishes.and_then(|f| f.garden.as_ref())
            .map(|g| g.load(root, materials, garden_base, front_daylight.is_some())).transpose()?;
        let first_stoop = garden_base + finishes.map_or(0, |f| f.garden_maps.len());
        let stoop = finishes.and_then(|f| f.stoop.as_ref()).map(|r|r.load(root,materials,first_stoop)).transpose()?;
        let first_fitted = first_stoop + finishes.map_or(0, |f| f.stoop_maps.len());
        let fitted = fitted_recipe.map(|r| r.load(root,materials,first_fitted)).transpose()?;
        let mut h = Self {
            window_optics: finishes.and_then(|f| f.window_optics.clone()),
            site_camera_exposure: finishes.and_then(|f| f.site_camera_exposure),
            stoop, porch_light, wall_daylight_layer, front_daylight, garden, fitted,
            origin: ring[1],
            across,
            back,
            floor,
            height_scale,
            bedside: {
                let p = Vec3::new(3.35, 10. / 3. + 0.03, 10.8);
                layout.nodes.get("double-bed").map_or(p, |m| m.transform_point3(p)).as_dvec3().to_array()
            },
            model,
            door: manifest.door,
            materials: resolved,
            light_transfer,
            visibility,
            cut_pattern,
            roof_uv,
            room_meter,
            dining_meter: finishes.and_then(|f| f.dining_meter),
            dining_camera_exposure: finishes.and_then(|f| f.dining_camera_exposure),
            table_light,
            table_vertices,
            oak_finish: finishes.and_then(|f| f.oak_finish),
            downstairs_light: finishes.is_some_and(|f| f.downstairs_light),
            dining_lamp: finishes.and_then(|f| f.dining_lamp),
            floor_reflections: finishes.is_some_and(|f| f.floor_reflections.is_some()),
            probes: finishes.map(|r| [
                layout.probe("enamel-refrigerator", &r.manifest.probes[0]),
                layout.probe("oven-range", &r.manifest.probes[1]),
            ]),
            collision: vec![],
            steps: vec![],
            pavers: vec![],
            mortar: vec![],
            aggregate: vec![],
            yard: vec![],
            rear_stucco: vec![],
            footing: vec![],
            rear_stucco_material,
            planting: planting::Planting::default(),
            yard_material,
            concrete_material,
            aggregate_material,
            paver_material,
            patina,
            photo,
            glazing: finishes.is_some_and(|f| f.glazing),
            entry: None,
        };
        if let Some(fit) = fitted_recipe {
            for pts in &fit.boxes {
                for face in FACES { h.solid_quad(face.map(|i| pts[i])); }
            }
        }
        for (index, ([a, b], matrix)) in manifest.boxes.into_iter().zip(layout.boxes).enumerate() {
            if fitted_recipe.is_some_and(|r|r.replaced_boxes.contains(&index)) { continue; }
            if let Some(m) = matrix {
                let pts = corners(a, b).map(|p| m.transform_point3(DVec3::from(p)).to_array());
                for face in FACES {
                    h.solid_quad(face.map(|i| pts[i]));
                }
            } else {
                h.solid_box(a, b);
            }
        }
        for (i,ramp) in manifest.ramps.into_iter().enumerate() {
            if !fitted_recipe.is_some_and(|r|r.replaced_ramps.contains(&i)) { h.solid_quad(ramp); }
        }
        if let Some(fit) = fitted_recipe {
            for ramp in &fit.ramps { h.solid_quad(*ramp); }
        }
        // The front stoop follows the surveyed ground at this scale.
        for i in 0..48 {
            let z = -(i as f64 + 1.) * 0.33;
            let y = -(i as f64) * 0.145;
            let a = [5.15, y - 0.50, z];
            let b = [6.75, y, z + 0.33];
            // The street falls across the lot. A horizontal tread that clears
            // the centre can still penetrate the uphill corner.
            if finishes.is_some()
                && i > 0
                && [a[0], b[0]].iter().any(|&x| {
                    let p = h.point(x, y, z);
                    p.y < ground(p.x, p.z) + 0.036
                })
            {
                break;
            }
            // The first tread and the recessed, tiled landing share one walking
            // surface up to the sill. The oak floor substrate sits 25 mm lower.
            let mut landing = b;
            if furnished && i == 0 {
                landing[2] = h.door.bounds[1][2];
            }
            h.solid_box(a, landing);
            h.steps.push([a, b]);
            let p = h.point(5.95, y, z);
            if p.y <= ground(p.x, p.z) + 0.12 {
                break;
            }
        }
        if finishes.is_some()
            && let Some(street) = street
        {
            h.approach(street, ground);
        }
        if let Some(stoop)=&h.stoop {
            for plan in stoop.pier_plans() {
                let top = plan.map(|[x,z]| [x,0.,z]);
                let bottom = plan.map(|[x,z]| {
                    let p = h.point(x,0.,z);
                    [x, ((ground(p.x,p.z)-p.y-0.06)/h.height_scale).min(-0.005), z]
                });
                let min = bottom.iter().map(|v| v[1]).fold(f64::INFINITY, f64::min);
                h.solid_box([plan[0][0], min, plan[0][1]], top[2]);
            }
        } else if finishes.is_some() {
            h.tile_treads();
        }
        if let Some(parcel) = manifest.rear_parcel {
            h.planting = planting::Planting::load(&dir, root, &h, ground, street, kit, materials)?;
            // Continue the exterior finish down the stem wall, leaving the same
            // 19 cm reveal above local grade as the source Blender elevation.
            let installed_width = across.length() * 7.;
            let blocks = (installed_width / 0.40).ceil() as usize;
            let grade = |x: f64| {
                let site = ring[1] + across*x + back*13.052;
                ground(site.x, site.y) - floor
            };
            for i in 0..blocks {
                let left = 7. * i as f64 / blocks as f64;
                let right = 7. * (i + 1) as f64 / blocks as f64;
                let top_m = ((grade(left).max(grade(right)) + 0.19) / 0.20).ceil() * 0.20;
                let top = top_m / height_scale;
                if top >= 0.015 { continue; }
                let z = 13.052;
                let quad = [[left, top, z], [right, top, z], [right, 0.015, z], [left, 0.015, z]];
                h.rear_stucco.push((quad, 0xffffffff));
                let joint = 0.003 / across.length();
                let face = z + 0.007 / back.length();
                let back_edge = z - 0.007 / back.length();
                for row in [0., 1.] {
                    let cap = (top_m - 0.20*row - 0.004) / height_scale;
                    let pts = corners([left + joint, cap - 0.192/height_scale, back_edge],
                                      [right - joint, cap, face]);
                    for side in [FACES[1], FACES[3], FACES[4], FACES[5]] {
                        h.footing.push((side.map(|v| pts[v]), 0xffe2e6e8));
                    }
                }
            }
            let det = across.perp_dot(back);
            let mut plan = parcel.survey_xz.map(|p| {
                let local = DVec2::from(p) * eo_data::groundscale::scale() - ring[1];
                DVec2::new(local.perp_dot(back)/det, across.perp_dot(local)/det)
            });
            plan.sort_by(|a,b| a.y.total_cmp(&b.y));
            let (front,rear) = plan.split_at_mut(2);
            front.sort_by(|a,b| a.x.total_cmp(&b.x));
            rear.sort_by(|a,b| a.x.total_cmp(&b.x));
            let mut start = [DVec2::ZERO; 2];
            for i in 0..2 {
                ensure!(front[i].y < 13. && rear[i].y > 13., "parcel does not contain the rear facade");
                start[i] = front[i].lerp(rear[i], (13.-front[i].y)/(rear[i].y-front[i].y));
            }
            // Follow both parcel lines and triangulate the ground at its sampled grade.
            let sample = |i: i32,j: i32| {
                let t = f64::from(j)/42.;
                let q = start[0].lerp(rear[0],t).lerp(start[1].lerp(rear[1],t),f64::from(i)/18.);
                let site = ring[1] + across*q.x + back*q.y;
                [q.x,(ground(site.x, site.y) - floor + 0.012)/height_scale,q.y]
            };
            for i in 0..18 {
                for j in 0..42 {
                    let quad = [sample(i,j),sample(i,j+1),sample(i+1,j+1),sample(i+1,j)];
                    let middle = (h.point(quad[0][0],0.,quad[0][2]) + h.point(quad[2][0],0.,quad[2][2]))*0.5;
                    if street.is_some_and(|s| s.buildings.contains(middle.x,middle.z)
                        || s.paved_layer_at(middle.x,middle.z).is_some()) { continue }
                    h.yard.push((quad,0xffffffff));
                }
            }
        }
        Ok(h)
    }

    /// Individually jointed clay tiles wrap around the noses of the
    /// ground-fitted private steps. The structural tread owns collision.
    fn tile_treads(&mut self) {
        let fired_clay = [0xff6382a8, 0xff6686b0, 0xff6b8bb2, 0xff6583a7, 0xff6e8bb4];
        for (step, &[a, b]) in self.steps.iter().enumerate() {
            let top = b[1] + 0.007;
            let front = a[2];
            let width = (b[0] - a[0]) / 5.;
            for tile in 0..5 {
                let x = a[0] + width * tile as f64 + 0.003;
                let u = x + width - 0.006;
                let clay = fired_clay[(step * 3 + tile * 2) % fired_clay.len()];
                // The mortar joint continues over the curved edge and down its lip.
                let old_profile = [
                    (b[2] - 0.005, top),
                    (front + 0.009, top),
                    (front - 0.005, top - 0.004),
                    (front - 0.011, top - 0.013),
                    (front - 0.011, top - 0.034),
                ];
                let bullnose = [
                    (b[2] - 0.005, top),
                    (front - 0.010, top),
                    (front - 0.020, top - 0.003),
                    (front - 0.028, top - 0.010),
                    (front - 0.030, top - 0.019),
                    (front - 0.027, top - 0.027),
                    (front - 0.018, top - 0.035),
                    (front - 0.003, top - 0.037),
                ];
                let profile = if self.patina {
                    // The rounded nosing projects beyond the upright riser.
                    // Its return catches grazing light and shades the joint.
                    self.pavers.push((
                        [
                            [x, top - 0.038, front - 0.004],
                            [u, top - 0.038, front - 0.004],
                            [u, top - 0.144, front - 0.004],
                            [x, top - 0.144, front - 0.004],
                        ],
                        clay,
                    ));
                    &bullnose[..]
                } else {
                    &old_profile[..]
                };
                for ends in profile.windows(2) {
                    let (z, y) = ends[0];
                    let (w, h) = ends[1];
                    self.pavers
                        .push(([[x, y, z], [u, y, z], [u, h, w], [x, h, w]], clay));
                }
            }
        }
    }

    /// Fit the private entry path between the last tread and the first mapped
    /// paving. Both edges are clipped independently on the sloping lot.
    fn approach(&mut self, street: &eo_street::StreetWorld, ground: &dyn Fn(f64, f64) -> f64) {
        let Some(&[a, b]) = self.steps.last() else {
            return;
        };
        let start = a[2];
        let edge = |x, start| -> Option<f64> {
            let paved = |z| {
                let p = self.point(x, 0., z);
                street.paved_layer_at(p.x, p.z)
            };
            if paved(start).is_some() {
                return None;
            }
            let mut inside = start;
            for i in 1..=64 {
                let mut outside = start - f64::from(i) * 0.25;
                if let Some(layer) = paved(outside) {
                    if !matches!(
                        layer,
                        eo_street::Layer::Sidewalk
                            | eo_street::Layer::Verge
                            | eo_street::Layer::Ramp
                    ) {
                        return None;
                    }
                    for _ in 0..12 {
                        let mid = (inside + outside) * 0.5;
                        if paved(mid).is_some() {
                            outside = mid;
                        } else {
                            inside = mid;
                        }
                    }
                    return Some(inside);
                }
                inside = outside;
            }
            None
        };
        let Some(left) = edge(a[0], start) else {
            return;
        };
        let Some(right) = edge(b[0], start) else {
            return;
        };
        let end = (left + right) * 0.5;
        if start - end < 0.03 {
            return;
        }
        let span = b[0] - a[0];
        let terrain = |x, z| {
            let p = self.point(x, 0., z);
            (ground(p.x, p.z) - self.floor) / self.height_scale
        };
        // Start the landing just under the last level tread. Ease the raised
        // downhill corner back to grade before the public paving.
        let landing = (b[1] - 0.13)
            .max(terrain(a[0], start) + 0.012)
            .max(terrain(b[0], start) + 0.012);
        let pos = |x: f64, t: f64| {
            let z = start + ((left + (right - left) * ((x - a[0]) / span)) - start) * t;
            let surface = terrain(x, z) + 0.012;
            let lift = (landing - terrain(x, start) - 0.012).max(0.) * (1. - t) * (1. - t);
            [x, surface + lift, z]
        };
        let courses = ((start - end) / 0.37).ceil() as usize;
        let mut tiles = Vec::new();
        let mut bedding = Vec::new();
        let mut walk = Vec::new();
        for j in 0..courses {
            let t0 = j as f64 / courses as f64;
            let t1 = (j + 1) as f64 / courses as f64;
            let course = [pos(a[0], t0), pos(b[0], t0), pos(b[0], t1), pos(a[0], t1)];
            walk.push(course);
            // Bed the pavers in mortar and close the exposed downhill edge
            // of the fill. The terrain can be well below the raised landing.
            let mortar = course.map(|[x, y, z]| [x, y - 0.008, z]);
            let mortar_faces = if self.patina {
                &mut bedding
            } else {
                &mut tiles
            };
            mortar_faces.push((mortar, 0xff829198));
            for (from, to) in [(mortar[0], mortar[3]), (mortar[2], mortar[1])] {
                let toe = |[x, _, z]: Point| [x, terrain(x, z) - 0.028, z];
                mortar_faces.push(([from, to, toe(to), toe(from)], 0xff80909c));
            }
            for k in 0..5 {
                let x = a[0] + (k as f64 - 0.5 * (j % 2) as f64) * span / 4.;
                let u = (x + span / 4.).min(b[0]) - 0.004;
                let x = x.max(a[0]) + 0.004;
                if u <= x {
                    continue;
                }
                let inset = 0.003 / (start - end);
                let quad = [
                    pos(x, t0 + inset),
                    pos(u, t0 + inset),
                    pos(u, t1 - inset),
                    pos(x, t1 - inset),
                ];
                let color = [0xff698cae, 0xff7c9eb7, 0xff7195b5, 0xff6f90b1][(j * 7 + k * 3) % 4];
                tiles.push((quad, color));
            }
        }
        // The retaining wall enters a graded gravel bed on the private side
        // of the sidewalk. Clip every strip independently to the surveyed edge.
        let mut aggregate = Vec::new();
        for (lo, hi) in [(0.10, a[0]), (b[0], 6.91)] {
            let columns = ((hi - lo) / 0.26).ceil() as usize;
            let mut last: Option<(f64, f64)> = None;
            for i in 0..=columns {
                let x = lo + (hi - lo) * i as f64 / columns as f64;
                let rear = -0.04;
                let Some(outer) = edge(x, rear) else {
                    last = None;
                    continue;
                };
                if let Some((prev_x, prev_outer)) = last {
                    let grade = |xx, zz| [xx, terrain(xx, zz) + 0.006, zz];
                    for row in 0..12 {
                        let mut t = row as f64 / 12.;
                        let mut u = (row + 1) as f64 / 12.;
                        let (mut left, mut right) = (prev_x,x);
                        if self.photo {
                            // Saw cuts in the private slab stop at the surveyed sidewalk.
                            let joint = 0.004;
                            if (i-1) % 5 == 0 { left += joint; }
                            if i % 5 == 0 { right -= joint; }
                            let depth = (prev_outer - rear).abs().max(0.1);
                            if row % 4 == 0 { t += joint/depth; }
                            if (row+1) % 4 == 0 { u -= joint/depth; }
                        }
                        aggregate.push((
                            [
                                grade(left, rear + (prev_outer - rear) * t),
                                grade(right, rear + (outer - rear) * t),
                                grade(right, rear + (outer - rear) * u),
                                grade(left, rear + (prev_outer - rear) * u),
                            ],
                            0xffffffff,
                        ));
                    }
                }
                last = Some((x, outer));
            }
        }
        self.aggregate = aggregate;
        for q in walk {
            self.solid_quad(q);
        }
        self.pavers = tiles;
        self.mortar = bedding;
        self.entry = Some(self.point((a[0] + b[0]) * 0.5, 0., end));
    }

    pub fn entry_yaw(&self) -> f64 {
        self.back.x.atan2(self.back.y)
    }

    pub fn entry_position(&self) -> DVec3 {
        self.entry.unwrap_or_else(|| self.point(5.95, 0., -3.3))
    }

    #[cfg(test)]
    pub fn lowest_tread_corners(&self) -> [DVec3; 2] {
        let [a, b] = self.steps.last().expect("the house has a private stair");
        [a[0], b[0]].map(|x| self.point(x, b[1], a[2]))
    }

    pub fn kitchen_frame(&self) -> [[f32; 4]; 5] {
        self.fitted.as_ref().map(|room| room.kitchen_frame).unwrap_or_default()
    }

    pub fn fitted_finish(&self) -> [[f32; 4]; 2] {
        self.fitted.as_ref().map(|room| room.finish_parameters).unwrap_or_default()
    }

    pub fn oak_finish(&self) -> [[f32; 4]; 3] {
        self.oak_finish.map(|finish| finish.parameters).unwrap_or_default()
    }

    pub fn table_light(&self) -> [[f32; 4]; 3] {
        self.table_light.as_ref().map(TableLight::uniforms).unwrap_or_default()
    }

    pub fn porch_light(&self, city_layers: usize) -> Option<[[f32; 4]; 7]> {
        let (lamp, slot) = self.porch_light.as_ref()?;
        Some(lamp.uniforms(self, city_layers + slot))
    }

    pub fn dining_lamp(&self) -> Option<[f32; 4]> {
        let [x,y,z,radius] = self.dining_lamp.or_else(|| self.table_light.as_ref().map(TableLight::lamp))?;
        Some(self.point(x.into(),y.into(),z.into()).as_vec3().extend(radius).to_array())
    }

    pub fn point(&self, x: f64, y: f64, z: f64) -> DVec3 {
        let p = self.origin + self.across * x + self.back * z;
        DVec3::new(p.x, self.floor + y * self.height_scale, p.y)
    }
    pub fn parcel_to_world(&self) -> glam::Mat4 {
        glam::Mat4::from_cols(
            Vec3::new(self.across.x as f32, 0., self.across.y as f32).extend(0.),
            (Vec3::Y * self.height_scale as f32).extend(0.),
            Vec3::new(self.back.x as f32, 0., self.back.y as f32).extend(0.),
            self.point(0., 0., 0.).as_vec3().extend(1.),
        )
    }
    const ROOM_MIN: Vec3 = Vec3::new(0.14, -0.08, 0.50);
    const ROOM_MAX: Vec3 = Vec3::new(6.86, 9.88, 12.86);

    pub fn site_camera(&self) -> Option<eo_render::interior_meter::ExteriorCamera> {
        let mut camera = eo_render::interior_meter::ExteriorCamera::new(
            self.parcel_to_world(), Self::ROOM_MIN, Self::ROOM_MAX, self.site_camera_exposure?);
        if let Some((lamp, _)) = &self.porch_light {
            let (centre, radius) = lamp.camera(self);
            camera.set_porch(centre, radius);
        }
        Some(camera)
    }

    pub fn interior_meter(&self) -> Option<eo_render::interior_meter::InteriorMeter> {
        if !self.photo || self.light_transfer.is_empty() { return None; }
        let basis = self.parcel_to_world();
        let glass = self.model.parts.iter()
            .filter(|part| self.model.materials[part.material as usize].name.starts_with("glass-door@"))
            .flat_map(|part| {
                let first = part.first_index as usize;
                self.model.indices[first..first + part.index_count as usize].chunks_exact(3)
            })
            .map(|indices| [0, 1, 2].map(|i| Vec3::from_array(self.model.positions[indices[i] as usize])))
            // Each pane is a thin closed solid. Integrate light through a single face.
            .filter(|[a,b,c]| (*b-*a).cross(*c-*a).normalize_or_zero().z > 0.8);
        let mut meter = eo_render::interior_meter::InteriorMeter::new(basis,
            Self::ROOM_MIN, Self::ROOM_MAX, 10. / 3., glass);
        if let Some([sky_return, exposure_range, exposed_beam]) = self.room_meter {
            meter.set_room_response(1, sky_return, exposure_range, exposed_beam);
        }
        if let Some([sky_return, exposure_range, exposed_beam]) = self.dining_meter {
            meter.set_room_camera_response(0, sky_return, exposure_range, exposed_beam, self.dining_camera_exposure);
        }
        Some(meter)
    }

    pub fn contains_feet(&self, feet: DVec3) -> bool {
        let p = DVec2::new(feet.x, feet.z) - self.origin;
        let det = self.across.perp_dot(self.back);
        let x = p.perp_dot(self.back) / det;
        let z = self.across.perp_dot(p) / det;
        (0.14..6.86).contains(&x) && (0.12..12.86).contains(&z)
            && (-0.08..9.85).contains(&((feet.y - self.floor) / self.height_scale))
    }

    fn solid_quad(&mut self, q: [Point; 4]) {
        let pts = q.map(|[x, y, z]| self.point(x, y, z).to_array());
        for tri in [[0, 1, 2], [0, 2, 3]] {
            self.collision.push(tri.map(|i| pts[i]));
        }
    }
    fn solid_box(&mut self, a: Point, b: Point) {
        let pts = corners(a, b);
        for face in FACES {
            self.solid_quad(face.map(|i| pts[i]));
        }
    }
    pub fn wake_position(&self) -> DVec3 {
        let [x, y, z] = self.bedside;
        self.point(x, y, z)
    }
    pub fn by_bed(&self, p: DVec3) -> bool {
        p.distance(self.wake_position()) < 1.45
    }
    fn bounds(&self, min: [f32; 3], max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = -lo;
        for [x, y, z] in corners(min.map(f64::from), max.map(f64::from)) {
            let p = self.point(x, y, z).as_vec3();
            lo = lo.min(p);
            hi = hi.max(p);
        }
        (lo.to_array(), hi.to_array())
    }
    pub fn append(&self, scene: &mut Scene, materials: &MaterialTable, detail_lod: bool) {
        let base = scene.vertices.len();
        let first = scene.indices.len();
        let objects = scene.objects.len();
        let normals = Mat3::from_cols(
            Vec3::new(self.across.x as f32, 0., self.across.y as f32),
            Vec3::Y * self.height_scale as f32,
            Vec3::new(self.back.x as f32, 0., self.back.y as f32),
        )
        .inverse()
        .transpose();
        // The grain runs down the long axis of the boards. Give the shader the
        // surveyed angle so grain stays parallel to each seam on rotated lots.
        let angle = self
            .across
            .y
            .atan2(self.across.x)
            .rem_euclid(std::f64::consts::TAU);
        let angle_byte = (angle / std::f64::consts::TAU * 255.0).round() as u32;
        let mut floor_uv = if self.probes.is_some() {
            vec![None; self.model.positions.len()]
        } else {
            Vec::new()
        };
        let mut finish = vec![0_u8; self.model.positions.len()];
        for part in &self.model.parts {
            let name = &self.model.materials[part.material as usize].name;
            let code = if name.starts_with("oiled-oak@") {
                255
            } else if name.starts_with("lime-plaster@") {
                100
            } else {
                0
            };
            let appliance = matches!(
                name.split('@').next().unwrap_or(""),
                "appliance-steel"
                    | "brushed-brass"
                    | "porcelain"
                    | "satin-nickel"
                    | "ivory-enamel"
                    | "smoked-enamel"
                    | "enamel-primer"
                    | "exposed-steel"
                    | "cast-iron"
                    | "iron-oxide"
                    | "seasoned-iron"
            );
            if self.probes.is_some()
                && let Some(paint) = FLOOR_MATERIALS.iter().position(|m| m == name)
            {
                let start = part.first_index as usize;
                for &i in &self.model.indices[start..start + part.index_count as usize] {
                    let [x, h, z] = self.model.positions[i as usize];
                    let section = if h > 1.0 && x >= 5.0999 { 5.1 } else { 0.0 };
                    let board = ((x - section) / 0.18 + 0.0001).max(0.0);
                    let level = (h / (10.0 / 3.0)).round() as usize;
                    let row = (board as usize * 5 + paint * 3 + level) % 8;
                    let across = (row as f32 + 0.07 + 0.86 * board.fract()) / 8.0;
                    floor_uv[i as usize] = Some(if self.floor_reflections && h.abs() < 0.04 {
                        // Preserve the source strip at each vertex. Reconstruct the lengthwise
                        // coordinate from the unquantized parcel position in the vertex shader.
                        let [lo, hi] = ((across * 65535.).round() as u16).to_le_bytes();
                        [f32::from(lo) / 255., f32::from(hi) / 255.]
                    } else {
                        [if self.photo { (x / 7.).clamp(0.,1.) } else { across },
                         (z / 13.0).clamp(0.0, 1.0)]
                    });
                }
            } else if self.patina && name.starts_with("glass-door@") {
                // Blender batches the front and rear panes of a storey into
                // one primitive. Only a street-facing pane sees this probe.
                let start = part.first_index as usize;
                for &i in &self.model.indices[start..start + part.index_count as usize] {
                    if self.model.positions[i as usize][2] < 0.75 {
                        finish[i as usize] = 4;
                    } else if self.photo && self.model.positions[i as usize][2] > 12.7 {
                        finish[i as usize] = 5;
                    }
                }
            } else if self.patina && plaster_finish(name) {
                let start = part.first_index as usize;
                for &i in &self.model.indices[start..start + part.index_count as usize] {
                    let [x, y, z] = self.model.positions[i as usize];
                    // Splashback at the soil and rain paths below projecting sill ends.
                    let mut wear = ((0.62 - y) / 0.84).clamp(0., 1.) * 0.53;
                    if z < 0.09 {
                        for h in [0.82, 4.15, 7.49] {
                            let drop = h - y;
                            if (0. ..1.12).contains(&drop) {
                                let edge = (x - 0.65).abs().min((x - 4.65).abs());
                                wear += 0.58
                                    * (1. - drop / 1.12)
                                    * (1. - edge / (0.14 + drop * 0.19)).max(0.);
                            }
                        }
                    }
                    finish[i as usize] = 3 + (wear.clamp(0., 1.) * 55.).round() as u8;
                }
            } else if self.photo && let Some(plank) = dining_plank(name) {
                // The four measured boards use the same distinct scan strips as in Blender.
                let range = Vec3::from(part.max) - Vec3::from(part.min);
                let first = part.first_index as usize;
                for &i in &self.model.indices[first..first + part.index_count as usize] {
                    let index = i as usize;
                    let t = ((Vec3::from(self.model.positions[index]) - Vec3::from(part.min)) / range)
                        .clamp(Vec3::ZERO, Vec3::ONE);
                    let n = eo_assets::glb::unpack_normal(self.model.normals[index]);
                    let u = if plank % 2 == 0 { 1. - t.x } else { t.x };
                    let section = if n.y.abs() > 0.64 { 0.07 + 0.86 * (1. - t.z) }
                                  else { 0.10 + 0.12 * t.y };
                    let row = [0., 3., 5., 7.][plank];
                    floor_uv[index] = Some([0.025 + 0.95 * u, (row + section) / 8.]);
                }
            } else if self.patina && name.starts_with("woven-carpet@") {
                let first = part.first_index as usize;
                for &i in &self.model.indices[first..first + part.index_count as usize] {
                    let [x, _, z] = self.model.positions[i as usize];
                    let u = (z - part.min[2]) / (part.max[2] - part.min[2]);
                    let v = (x - part.min[0]) / (part.max[0] - part.min[0]);
                    floor_uv[i as usize] = Some([u.clamp(0., 1.), v.clamp(0., 1.)]);
                }
            } else if self.patina && door_paint(name) {
                let start = part.first_index as usize;
                for &i in &self.model.indices[start..start + part.index_count as usize] {
                    let [x, y, _] = self.model.positions[i as usize];
                    let [a, b] = self.door.bounds;
                    // Fractions belong to the leaf, so the wood grain follows its hinge.
                    floor_uv[i as usize] = Some([
                        ((x - a[0] as f32) / (b[0] - a[0]) as f32).clamp(0., 1.),
                        (y / b[1] as f32).clamp(0., 1.),
                    ]);
                }
            } else if appliance && let Some(probes) = &self.probes {
                let first = part.first_index as usize;
                for &index in &self.model.indices[first..first + part.index_count as usize] {
                    let point = self.model.positions[index as usize];
                    if let Some(row) = probes
                        .iter()
                        .position(|p| (0..3).all(|k| (p.min[k]..=p.max[k]).contains(&point[k])))
                    {
                        // Blue bytes 1 and 2 select the two rows in the reflection atlas.
                        finish[index as usize] = row as u8 + 1;
                    }
                }
            } else if code != 0 {
                let start = part.first_index as usize;
                for &i in &self.model.indices[start..start + part.index_count as usize] {
                    finish[i as usize] = code;
                }
            }
        }
        for (i, (&[x, y, z], &n)) in self
            .model
            .positions
            .iter()
            .zip(&self.model.normals)
            .enumerate()
        {
            let mut normal =
                eo_assets::glb::pack_normal(normals * eo_assets::glb::unpack_normal(n));
            let mut color = if let Some(&[day, lamp]) = self.light_transfer.get(i) {
                // Signed normal W is unused on authored models. Mark the two light channels
                // so their values survive interpolation independently of the paint colour.
                normal |= 0x7f00_0000;
                let pack = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
                (angle_byte << 24) | (u32::from(finish[i]) << 16) | pack(day) | (pack(lamp) << 8)
            } else if finish[i] != 0 {
                normal |= 0x4000_0000;
                (angle_byte << 24) | (u32::from(finish[i]) << 16)
            } else {
                0xffff_ffff
            };
            if let Some(&Some([u, v])) = floor_uv.get(i) {
                normal |= if self.light_transfer.is_empty() {
                    0x4000_0000
                } else {
                    0x7f00_0000
                };
                color = (color & 0xffff)
                    | (((u * 255.).round() as u32) << 16)
                    | (((v * 255.).round() as u32) << 24);
            }
            if let Some(&[u, v]) = self.cut_pattern.get(&(i as u32)) {
                normal |= 0x4000_0000;
                color = (color & 0xffff) | (u32::from(u) << 16) | (u32::from(v) << 24);
            }
            if let Some([u,v]) = self.roof_uv.as_ref().and_then(|roof| roof.get(i)) {
                normal |= 0x4000_0000;
                color = u32::from(u) << 16 | u32::from(v) << 24;
            }
            if let Some(visibility) = self.visibility.get(i) {
                // 64 identifies unlit authored normals. Codes 65..127 carry diffuse visibility.
                normal = (normal & 0x00ff_ffff) | ((65 + (62. * visibility).round() as u32) << 24);
            }
            if self.photo && self.light_transfer.get(i).is_some() {
                // Classify each side of a wall independently. The signed normal's
                // spare W channel keeps contact visibility on both sides of a reveal.
                let sample = Vec3::new(x, y, z) + eo_assets::glb::unpack_normal(n) * 0.025;
                let porch = sample.y < 2.75 && sample.x > 5.10 && sample.z < 0.50;
                if !porch && (0.10..6.90).contains(&sample.x) && (0.05..12.96).contains(&sample.z)
                    && (0.0..9.98).contains(&sample.y) {
                    let visibility = (normal >> 24) as i8;
                    normal = (normal & 0x00ff_ffff) | (u32::from((-visibility) as u8) << 24);
                }
            }
            if let Some(&[u,v]) = self.table_vertices.get(&(i as u32)) {
                // The low bytes address the photographed prop's surface light.
                // Negative codes -1..-63 preserve the original near-field visibility.
                let visibility = ((normal >> 24) as i8).unsigned_abs().clamp(65,127);
                normal = (normal & 0x00ff_ffff) | (u32::from((-(visibility as i8 - 64)) as u8) << 24);
                color = (color & 0xffff_0000) | u32::from(u) | (u32::from(v) << 8);
            }
            scene.vertices.push(Vertex {
                pos: self
                    .point(x.into(), y.into(), z.into())
                    .as_vec3()
                    .to_array(),
                color,
                normal,
            });
        }
        scene.indices.extend_from_slice(&self.model.indices);
        for part in &self.model.parts {
            let (lo, hi) = if self
                .door
                .materials
                .contains(&self.model.materials[part.material as usize].name)
            {
                // The cull and shadow tables cover the full swing, including
                // the handle. At rest or in motion it is one piece of joinery.
                let [x, y, z] = self.door.hinge;
                let hinge = self.point(x, y, z).as_vec3();
                let reach = ((self.door.bounds[1][0] - self.door.bounds[0][0] + 0.18)
                    * self.across.length()) as f32;
                let height = (self.door.bounds[1][1] * self.height_scale) as f32;
                (
                    (hinge - Vec3::new(reach, 0.01, reach)).to_array(),
                    (hinge + Vec3::new(reach, height + 0.01, reach)).to_array(),
                )
            } else {
                self.bounds(part.min, part.max)
            };
            let mut o = GpuObject::new(
                ObjectKind::Model,
                lo,
                hi,
                first as u32 + part.first_index,
                part.index_count,
                base as i32,
            );
            if self.probes.is_some()
                && FLOOR_MATERIALS
                    .contains(&self.model.materials[part.material as usize].name.as_str())
            {
                o.kind |= eo_render::scene::FLOOR_SCAN_BIT;
                if self.dining_lamp.is_some() && part.min[1] < 0.04 {
                    o.kind |= eo_render::scene::DINING_FLOOR_LIGHT_BIT;
                }
                if self.photo { o.kind |= eo_render::scene::WAXED_BOARD_BIT; }
                if self.floor_reflections && part.min[1] < 0.04 {
                    o.kind |= eo_render::scene::FLOOR_REFLECTION_BIT;
                    if self.fitted.is_some() { o.kind |= eo_render::scene::FITTED_FLOOR_BIT; }
                }
            }
            let family = &self.model.materials[part.material as usize].name;
            if self.patina && (family.starts_with("woven-carpet@") || family == "mission-roof-clay") {
                o.kind |= eo_render::scene::LOCAL_UV_SCAN_BIT;
            }
            if self.patina && door_paint(&self.model.materials[part.material as usize].name) {
                o.kind |= eo_render::scene::DOOR_SCAN_BIT;
            }
            if self.photo && dining_plank(&self.model.materials[part.material as usize].name).is_some() {
                o.kind |= eo_render::scene::TABLE_BOARD_BIT;
                if self.oak_finish.is_some() {
                    o.kind |= eo_render::scene::OAK_FINISH_BIT;
                }
            }
            if self.photo && !self.light_transfer.is_empty() {
                o.kind |= eo_render::scene::ENCLOSURE_TRANSFER_BIT;
                if self.table_light.as_ref().is_some_and(|table| dining_plank(family).is_some() || table.material(family)) {
                    o.kind |= eo_render::scene::TABLE_LIGHT_BIT;
                }
                if self.downstairs_light && part.min[1] < 10. / 3. + 0.03 && part.max[1] >= 0. {
                    o.kind |= eo_render::scene::DOWNSTAIRS_LIGHT_BIT;
                }
            }
            if self.photo {
                match family.split('@').next().unwrap_or("") {
                    "front-door-brass" => o.kind |= eo_render::scene::ENTRY_BRASS_BIT,
                    "curtain-linen" | "street-tree-foliage" => o.kind |= eo_render::scene::THIN_FIBRE_BIT,
                    "cotton-coverlet" | "cotton-turnback" | "cotton-binding" =>
                        o.kind |= eo_render::scene::QUILT_FIBRE_BIT,
                    _ => {}
                }
            }
            if self.room_meter.is_some() {
                if !self.cut_pattern.is_empty() && family.starts_with("cotton-coverlet@") {
                    o.kind |= eo_render::scene::CUT_COTTON_BIT;
                }
                if !self.light_transfer.is_empty() && part.max[1] >= 10./3. - 0.02
                    && part.min[1] <= 20./3. + 0.02 {
                    o.kind |= eo_render::scene::BEDROOM_LIGHT_BIT;
                }
            }
            o.material = self.materials[part.material as usize];
            if (self.wall_daylight_layer.is_some() || self.porch_light.is_some()) && (plaster_finish(family) || family.starts_with("lime-plaster@") || family.starts_with("painted-white@")) {
                o.kind |= eo_render::scene::WALL_DAYLIGHT_BIT;
            }
            if self.patina && plaster_finish(family) {
                o.kind |= eo_render::scene::LIMEWASH_BIT;
            }
            if self.patina && self.glazing && family.starts_with("glass-door@") && (part.min[2] < 0.75 || self.photo && part.max[2] > 12.7) {
                o.kind |= eo_render::scene::FRONT_SASH_BIT;
            }
            if !self.patina && self.aggregate_material != 0 && o.material == self.aggregate_material
            {
                o.kind |= eo_render::scene::GRAVEL_BIT;
            }
            if materials.gpu[o.material as usize].is_blend() {
                o.kind |= BLEND_BIT;
            }
            scene.objects.push(o);
        }
        // The fixed mesh stops at its threshold. Only the ground-fitted stoop is generated here.
        for &[a, b] in &self.steps {
            let pts = corners(a, b);
            let start = scene.indices.len();
            for face in FACES {
                let p = face.map(|i| {
                    let [x, y, z] = pts[i];
                    self.point(x, y, z).as_vec3()
                });
                let normal = eo_assets::glb::pack_normal((p[1] - p[0]).cross(p[2] - p[0]));
                let v = scene.vertices.len() as u32;
                scene.vertices.extend(p.map(|p| Vertex {
                    pos: p.to_array(),
                    normal,
                    color: if self.patina {
                        0xff78858f
                    } else if self.probes.is_some() {
                        0xffa8b6be
                    } else {
                        0xff9aa4aa
                    },
                }));
                scene.indices.extend([v, v + 1, v + 2, v, v + 2, v + 3]);
            }
            let (lo, hi) = self.bounds(a.map(|v| v as f32), b.map(|v| v as f32));
            scene.objects.push(GpuObject::new(
                ObjectKind::Landmark,
                lo,
                hi,
                start as u32,
                36,
                0,
            ));
        }
        for (quads, kind, material) in [
            (&self.mortar, ObjectKind::Landmark, 0),
            (
                &self.pavers,
                if self.patina {
                    ObjectKind::Model
                } else {
                    ObjectKind::Landmark
                },
                self.paver_material,
            ),
            (&self.aggregate, ObjectKind::Model,
                if self.photo { self.concrete_material } else { self.aggregate_material }),
            (&self.yard, ObjectKind::Model, self.yard_material),
            (&self.rear_stucco, ObjectKind::Model, self.rear_stucco_material),
            (&self.footing, ObjectKind::Model, self.concrete_material),
        ] {
            if quads.is_empty() {
                continue;
            }
            let start = scene.indices.len() as u32;
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = -lo;
            for &(quad, color) in quads {
                let color = if self.patina && material == self.paver_material {
                    // Each fired tile modulates the scan by its own kiln tone.
                    let tone = 220 + (color & 31);
                    0xff000000 | tone * 0x010101
                } else {
                    color
                };
                let p = quad.map(|[x, y, z]| self.point(x, y, z).as_vec3());
                let normal = eo_assets::glb::pack_normal((p[1] - p[0]).cross(p[2] - p[0]));
                let v = scene.vertices.len() as u32;
                for point in p {
                    let (normal, color) = if self.photo && material == self.rear_stucco_material {
                        let above_plinth = (point.y - p[0].y).max(0.);
                        let wear = (0.78 - above_plinth / 0.74).clamp(0.12, 0.94);
                        (normal | 0x4000_0000, (3 + (55. * wear).round() as u32) << 16)
                    } else { (normal, color) };
                    lo = lo.min(point);
                    hi = hi.max(point);
                    scene.vertices.push(Vertex {
                        pos: point.to_array(),
                        normal,
                        color,
                    });
                }
                scene.indices.extend([v, v + 1, v + 2, v, v + 2, v + 3]);
            }
            let mut object = GpuObject::new(
                kind,
                lo.to_array(),
                hi.to_array(),
                start,
                scene.indices.len() as u32 - start,
                0,
            );
            object.material = material;
            if self.photo && material == self.rear_stucco_material {
                object.kind |= eo_render::scene::LIMEWASH_BIT;
            }
            if self.photo && kind == ObjectKind::Model {
                object.kind |= eo_render::scene::COURTYARD_BOUNCE_BIT;
            }
            if !self.patina && material != 0 && material == self.aggregate_material {
                object.kind |= eo_render::scene::GRAVEL_BIT;
            }
            scene.objects.push(object);
        }
        if let Some(fit) = &self.fitted { fit.append(self,scene,normals); }
        if let Some(garden) = &self.garden { garden.append(self, scene, normals); }
        if let Some(stoop) = &self.stoop { stoop.append(self,scene,normals); }
        self.planting.append(scene);
        if self.photo && detail_lod { lod::install(scene, objects); }
        scene.stats.objects += scene.objects.len() - objects;
        scene.stats.vertices += scene.vertices.len() - base;
        scene.stats.indices += scene.indices.len() - first;
        scene.stats.triangles += ((scene.indices.len() - first) / 3) as u64;
    }
}

fn dining_plank(name: &str) -> Option<usize> {
    let family = name.split_once('@')?.0;
    family.strip_prefix("dining-board-")?.parse::<usize>().ok().filter(|i| *i < 4)
}

fn plaster_finish(name: &str) -> bool {
    name.starts_with("painted-stucco@") || name.starts_with("float-finish-stucco@")
}

fn door_paint(name: &str) -> bool {
    name.starts_with("front-door-paint@") || name.starts_with("front-door-moulding@")
}

pub struct HouseGround {
    pub base: Arc<dyn GroundProvider>,
    pub house: Arc<House>,
}
impl GroundProvider for HouseGround {
    fn ground_y(&self, x: f64, z: f64) -> f64 {
        self.base.ground_y(x, z)
    }
    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        let base = self.base.surface_tile(key);
        if key != TileKey::of(self.house.origin.x, self.house.origin.y) {
            return base;
        }
        let mut mesh = base.unwrap_or_default();
        mesh.barrier.extend_from_slice(&self.house.collision);
        Some(mesh)
    }
}
