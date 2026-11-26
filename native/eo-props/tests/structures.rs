//! The props and the grade-separated roads (`--structures`, stage A6).
//!
//! * Without structures (none, or an empty world) every placed list of a
//!   sample of real tiles is byte for byte what it was before the stage.
//! * Under a synthetic deck: a street tree is made smaller or dropped, a lamp
//!   and a signal mast are not placed through the deck (the mast is cut down
//!   to a pedestal or dropped), a sign post is not seated on a column, no
//!   parked car stands on a column; every placed item of the tiles under the
//!   deck is clear of it; the deck luminaires stand on its barriers at the
//!   landmark deck rule's pitch and height.
//! * On the real city: nothing placed meets a deck, girder, barrier, cap or
//!   column, and what keeping clear of them did is counted (`--nocapture`).

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use eo_props::clearance::{self, Extent};
use eo_props::overhead::{self, OverheadTile};
use eo_props::{Live, Model, Prop, PropsWorld, TilePlacement};
use eo_structures::{ChainKind, ColumnSite, StructureOptions, StructureWorld};

// ---------------------------------------------------------------- the real city

struct Real {
    city: Arc<eo_data::City>,
    street: Arc<eo_street::StreetWorld>,
    structures: Arc<StructureWorld>,
}

fn real() -> &'static Real {
    static R: OnceLock<Real> = OnceLock::new();
    R.get_or_init(|| {
        let paths = eo_data::DataPaths::discover().unwrap();
        let city = Arc::new(eo_data::City::load(&paths).unwrap());
        let street = eo_props::load_street_world(&paths).unwrap();
        let ca = eo_data::CaStandards::load(&paths).unwrap();
        let structures = Arc::new(StructureWorld::build(&street, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions::default()).unwrap());
        Real { city, street, structures }
    })
}

fn standards() -> eo_props::Standards {
    let paths = eo_data::DataPaths::discover().unwrap();
    eo_props::Standards::load(&eo_data::CaStandards::load(&paths).unwrap()).unwrap()
}

fn props_world(structures: Option<Arc<StructureWorld>>) -> PropsWorld {
    // The poster hoardings drawn, as they were when `BEFORE` was recorded (eo-app
    // `--poster-hoardings`; set before any world plans, the same in every test here).
    eo_props::furniture::set_poster_hoardings(true);
    let r = real();
    let w = PropsWorld::new(r.street.clone(), Arc::new(eo_props::JsProps::load().unwrap()), standards()).with_structures(structures);
    w.install_furniture(&r.city.buildings).unwrap();
    overhead::install(&w, overhead::DropBuilding::from_city(&r.city.buildings)).unwrap();
    w
}

/// Every sixth tile with a structure in it and every 97th street tile, as
/// they were when the stage began (fixed, so the sample does not move with
/// the structures).
const SAMPLE: [(i32, i32); 180] = [
    (-81, -2), (-78, 8), (-76, 3), (-75, 52), (-73, 37), (-71, 4), (-70, 35), (-68, -7), (-67, 26), (-65, 0),
    (-64, 30), (-64, 47), (-62, -8), (-61, 30), (-59, -3), (-58, 32), (-57, 61), (-56, 75), (-54, -3), (-53, 8),
    (-52, -56), (-52, -50), (-52, 23), (-51, -48), (-51, -42), (-51, -36), (-51, 33), (-50, -33), (-50, 31),
    (-49, 29), (-48, 27), (-47, 25), (-46, 22), (-46, 74), (-45, 20), (-44, -28), (-44, 13), (-43, -26), (-43, 9),
    (-42, 9), (-41, 10), (-40, -26), (-40, 13), (-39, 17), (-38, 17), (-37, 15), (-36, 9), (-35, 4), (-35, 72),
    (-34, 2), (-33, 7), (-32, 6), (-31, 7), (-31, 72), (-30, 7), (-29, 3), (-29, 5), (-28, -1), (-27, -10),
    (-26, -16), (-25, -29), (-25, -6), (-25, 70), (-24, 62), (-23, 5), (-23, 55), (-22, 48), (-21, 40), (-20, 30),
    (-19, 24), (-18, 15), (-17, 5), (-16, -4), (-15, -15), (-14, -26), (-14, 49), (-14, 73), (-13, 63), (-12, -8),
    (-12, 55), (-11, 44), (-10, 36), (-9, -9), (-9, 31), (-8, 24), (-7, 15), (-6, 10), (-6, 43), (-5, -1), (-4, -8),
    (-3, -15), (-3, 7), (-2, -22), (-1, -30), (-1, 75), (0, 63), (1, 8), (1, 53), (2, 45), (3, 8), (3, 34), (4, 19),
    (5, 7), (6, -19), (6, -2), (7, -14), (7, 9), (8, -26), (8, 9), (8, 74), (9, 7), (9, 44), (9, 61), (10, 3),
    (10, 10), (10, 47), (10, 49), (11, 7), (11, 14), (11, 38), (11, 47), (12, 9), (12, 30), (12, 47), (13, 20),
    (13, 30), (13, 50), (14, 10), (14, 43), (15, -1), (15, 0), (16, -9), (16, -2), (17, -20), (17, -13), (17, 6),
    (17, 65), (18, -17), (18, 4), (18, 10), (19, -17), (19, -4), (19, 36), (20, -15), (20, -10), (20, 12), (20, 44),
    (21, -13), (21, -7), (21, 34), (22, -8), (22, -4), (22, 21), (22, 34), (23, 5), (23, 15), (23, 24), (23, 30),
    (24, 17), (24, 28), (25, 31), (26, 46), (27, 32), (27, 64), (29, -16), (29, 35), (31, -17), (31, 28), (33, 47),
    (34, -21), (36, -23), (36, 45), (39, -26), (40, -57), (42, -31), (42, 52), (45, 61), (46, -36), (48, -37),
    (49, -33)
];
/// The sample's hash and item count before the stage (its first run, the
/// code as it was).
const BEFORE: (u64, usize) = (0x0935487a81bb41be, 26011);

fn hash_placement(t: &TilePlacement, h: &mut impl Hasher) {
    for p in &t.props {
        (p.model as u8, p.variant).hash(h);
        p.pos.map(f64::to_bits).hash(h);
        p.yaw.to_bits().hash(h);
        p.scale.map(f32::to_bits).hash(h);
        match p.live {
            Live::None => 0u8.hash(h),
            Live::Vehicle { node, bearing } => (1u8, node.map(f64::to_bits), bearing.to_bits()).hash(h),
            Live::Pedestrian { node, bearing, length_m } => (2u8, node.map(f64::to_bits), bearing.to_bits(), length_m.to_bits()).hash(h),
        }
    }
    for c in &t.parked {
        (&c.spec, c.paint, c.entry, c.side).hash(h);
        c.pos.map(f64::to_bits).hash(h);
        (c.yaw.to_bits(), c.station.to_bits()).hash(h);
    }
    for l in &t.lights {
        l.pos.map(f64::to_bits).hash(h);
        l.base.map(f64::to_bits).hash(h);
        l.colour_linear.map(f32::to_bits).hash(h);
        [l.intensity, l.range_m, l.decay, l.angle, l.penumbra, l.source_lift_m].map(f32::to_bits).hash(h);
        (l.kind as u8).hash(h);
    }
}

fn hash_overhead(o: &OverheadTile, h: &mut impl Hasher) {
    for p in &o.poles {
        (format!("{:?}{:?}", p.kind, p.material), &p.id).hash(h);
        p.pos.map(f64::to_bits).hash(h);
        [p.yaw, p.height, p.radius_base, p.radius_top].map(f64::to_bits).hash(h);
        p.arm.map(|a| (a.pos.map(f64::to_bits), a.yaw.to_bits())).hash(h);
    }
    for w in &o.wires {
        (w.kind as u8, format!("{:?}", w.system), w.radius_m.to_bits()).hash(h);
        for q in &w.points {
            q.map(f32::to_bits).hash(h);
        }
    }
    for c in &o.cabinets {
        c.pos.map(f64::to_bits).hash(h);
        c.size.map(f64::to_bits).hash(h);
        (c.yaw.to_bits(), c.node).hash(h);
    }
    for s in &o.signs {
        s.pos.map(f64::to_bits).hash(h);
        [s.yaw, s.scale, s.post_height].map(f64::to_bits).hash(h);
        (s.variant, s.on_pole, s.plates.len()).hash(h);
        for pl in &s.plates {
            pl.centre.map(f64::to_bits).hash(h);
            (pl.w.to_bits(), pl.h.to_bits(), &pl.band, &pl.fill, &pl.face, pl.rules).hash(h);
        }
    }
    for c in &o.corner_signs {
        c.pos.map(f64::to_bits).hash(h);
        (c.yaw.to_bits(), c.on_signal_pole).hash(h);
    }
    for l in &o.lanterns {
        l.pos.map(f64::to_bits).hash(h);
        l.yaw.to_bits().hash(h);
    }
}

/// Every placed list of some tiles (props, parked cars, luminaires, the
/// overhead), hashed in tile order, and the number of items.
fn tiles_hash(w: &PropsWorld, keys: &[(i32, i32)]) -> (u64, usize) {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut n = 0;
    for &k in keys {
        k.hash(&mut h);
        let t = w.plan_tile(k);
        n += t.props.len() + t.parked.len() + t.lights.len();
        hash_placement(&t, &mut h);
        let o = overhead::plan_tile(w, k);
        n += o.poles.len() + o.wires.len() + o.cabinets.len() + o.signs.len() + o.corner_signs.len() + o.lanterns.len();
        hash_overhead(&o, &mut h);
    }
    (h.finish(), n)
}

#[test]
fn without_structures_every_placed_list_is_as_before() {
    let none = props_world(None);
    let (hash, n) = tiles_hash(&none, &SAMPLE);
    println!("{} tiles, {n} placed items, hash {hash:#018x} (before the stage: {:#018x}, {})", SAMPLE.len(), BEFORE.0, BEFORE.1);
    assert_eq!((hash, n), BEFORE, "the placed lists changed without structures");
    // an empty world places the same.
    let empty = props_world(Some(Arc::new(StructureWorld::empty())));
    assert!(empty.clear_structures().is_none() && empty.deck_lamps().is_empty());
    assert_eq!(tiles_hash(&empty, &SAMPLE), BEFORE, "an empty structures world changed the placed lists");
    // with the city's structures, the sample tiles nowhere near one place the same.
    let r = real();
    let near: std::collections::HashSet<(i32, i32)> = r.structures.tile_keys().into_iter().flat_map(|k| (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (k.0 + dx, k.1 + dz)))).collect();
    let far: Vec<(i32, i32)> = SAMPLE.iter().copied().filter(|k| !near.contains(k)).collect();
    let with = props_world(Some(r.structures.clone()));
    assert_eq!(tiles_hash(&with, &far), tiles_hash(&none, &far), "tiles away from every structure changed");
    println!("{} sample tiles away from every structure: the same with the city's structures", far.len());
}

/// Whether a placed item meets a structure (`except`: the deck a deck luminaire stands on).
fn meets(s: &StructureWorld, d: &clearance::Dims, p: &Prop, except: Option<u32>) -> bool {
    let mut ex = vec![];
    clearance::extents(d, p, &mut ex);
    !clearance::all_clear(s, &ex, except)
}

/// The deck a deck luminaire with its foot at `foot` stands on (None for any other item).
fn deck_lamp_chain(w: &PropsWorld, foot: [f64; 3]) -> Option<u32> {
    w.deck_lamps().get(&eo_street::tile_key(foot[0], foot[2]))?.iter().find(|l| l.prop.pos == foot).map(|l| l.chain)
}

#[test]
fn on_the_city_nothing_placed_meets_a_structure() {
    let r = real();
    let s = &*r.structures;
    let none = props_world(None);
    let with = props_world(Some(r.structures.clone()));
    let d = with.clear_dims();
    let mut keys: Vec<(i32, i32)> = s.tile_keys();
    keys.sort_unstable();
    let (mut before, mut after): (BTreeMap<String, usize>, BTreeMap<String, usize>) = (BTreeMap::new(), BTreeMap::new());
    let mut stats = clearance::ClearStats::default();
    let (mut parked_before, mut parked_after, mut lights_before, mut lights_after) = (0usize, 0usize, 0usize, 0usize);
    let (mut meeting, mut deck_lamps, mut checked) = (vec![], 0usize, 0usize);
    // (each world warmed on the tiles first, so the times compare the planning alone.)
    let (mut ms_none, mut ms_with) = (0.0f64, 0.0f64);
    let t_cold = std::time::Instant::now();
    for &k in &keys {
        with.plan_tile(k);
        overhead::plan_tile(&with, k);
    }
    let ms_cold = t_cold.elapsed().as_secs_f64() * 1e3;
    for &k in &keys {
        none.plan_tile(k);
        overhead::plan_tile(&none, k);
    }
    for &k in &keys {
        let t0 = std::time::Instant::now();
        let a = none.plan_tile(k);
        overhead::plan_tile(&none, k);
        ms_none += t0.elapsed().as_secs_f64() * 1e3;
        let t1 = std::time::Instant::now();
        let (b, st) = with.plan_tile_stats(k);
        overhead::plan_tile(&with, k);
        ms_with += t1.elapsed().as_secs_f64() * 1e3;
        stats.add(&st);
        for p in &a.props {
            *before.entry(format!("{:?}", p.model)).or_default() += 1;
        }
        parked_before += a.parked.len();
        parked_after += b.parked.len();
        lights_before += a.lights.len();
        lights_after += b.lights.len();
        for p in &b.props {
            checked += 1;
            let lamp = if p.model == Model::LampCobra { deck_lamp_chain(&with, p.pos) } else { None };
            if lamp.is_some() {
                deck_lamps += 1;
            } else {
                *after.entry(format!("{:?}", p.model)).or_default() += 1;
            }
            if meets(s, d, p, lamp) {
                meeting.push((p.model, p.pos));
            }
        }
        // parked cars: off every column by the margin.
        for c in &b.parked {
            for f in s.column_footprints(k).iter().chain(s.column_footprints((k.0 + 1, k.1)).iter()).chain(s.column_footprints((k.0 - 1, k.1)).iter()).chain(s.column_footprints((k.0, k.1 + 1)).iter()).chain(s.column_footprints((k.0, k.1 - 1)).iter()) {
                assert!((c.pos[0] - f.x).hypot(c.pos[2] - f.z) > f.radius + 0.5, "parked car {} at {:?} on a column at ({:.1}, {:.1})", c.spec, c.pos, f.x, f.z);
            }
        }
        // the overhead: poles, cabinets, signs and every wire sample clear.
        let (o, ost) = overhead::plan_tile_stats(&with, k);
        stats.add(&ost);
        for p in &o.poles {
            checked += 1;
            if clearance::blocked(s, &Extent { x: p.pos[0], z: p.pos[2], r: p.radius_base, y0: p.pos[1], y1: p.pos[1] + p.height }, None) {
                meeting.push((Model::SignalPole, p.pos));
            }
        }
        for wr in &o.wires {
            checked += 1;
            for q in &wr.points {
                let q = q.map(|v| v as f64);
                if s.blocked(q[0], q[2], wr.radius_m as f64, q[1], q[1]) {
                    meeting.push((Model::SignPost, q));
                    break;
                }
            }
        }
    }
    let total_lamps: usize = with.deck_lamps().values().map(|v| v.len()).sum();
    println!("{} structure tiles: {checked} placed items checked, {} meeting a structure", keys.len(), meeting.len());
    println!(
        "planning them with structures from cold: {:.1} ms a tile (the deck luminaires and the cleared junctions built on the way); again, caches warm: {:.2} ms a tile without structures (signals planned every time), {:.2} ms with (cleared junctions kept)",
        ms_cold / keys.len() as f64,
        ms_none / keys.len() as f64,
        ms_with / keys.len() as f64
    );
    println!("dropped or made smaller, by kind (before -> after): ");
    for (k, v) in &before {
        let a = after.get(k).copied().unwrap_or(0);
        if a != *v {
            println!("  {k:<15} {v} -> {a}");
        }
    }
    println!("clearance: {stats:?}");
    println!("parked cars {parked_before} -> {parked_after}; district luminaires {lights_before} -> {} (+ {deck_lamps} deck luminaires in these tiles, {total_lamps} in the city)", lights_after - deck_lamps);
    assert!(meeting.is_empty(), "{} placed items meet a structure: {:?}", meeting.len(), &meeting[..meeting.len().min(10)]);
    assert!(total_lamps > 100, "deck luminaires: {total_lamps}");
    assert!(stats.dropped_total() + stats.trees_capped + stats.pedestals > 0, "nothing under any deck was touched");
    assert!(parked_after <= parked_before);
}

// ---------------------------------------------------------------- a synthetic deck

const CELL: f64 = 8.0;
const ORIGIN: f64 = -600.0;
const CELLS: usize = 150;

fn flat_grid() -> eo_data::ElevationGrid {
    let nodata = -32768;
    eo_data::ElevationGrid {
        meta: eo_data::ElevationMeta { version: "synthetic".into(), origin_x: ORIGIN, origin_z: ORIGIN, cell_size: CELL, width: CELLS, height: CELLS, nodata },
        lidar: vec![0.0; CELLS * CELLS],
        topobathy: vec![nodata as i16; CELLS * CELLS],
        height_scale: 1.0,
    }
}

fn seg(way: &str, k: usize, highway: &str, width: f64, walk: f64, a: [f64; 2], b: [f64; 2], separated: bool) -> eo_street::city::Segment {
    eo_street::city::Segment {
        key: Some(format!("sf-street-{way}:{k}")),
        id: format!("sf-seg-{way}-{k}"),
        street_name: format!("Way {way}"),
        street_id: Some(format!("sf-street-{way}")),
        highway: highway.into(),
        width,
        lanes: Some(2.0),
        oneway: "both".into(),
        sidewalk_w: Some(walk),
        sidewalk_left: Some(walk),
        sidewalk_right: Some(walk),
        points: vec![a, b],
        separated,
        lane_layout: None,
        parking: [0.0; 2],
        cycle: [0.0; 2],
        cycle_kerbside: [false; 2],
        cycle_buffer: [0.0; 2],
        shoulder: [0.0; 2],
        median: 0.0,
        street_type: None,
        maxspeed_kmh: None,
        maxspeed: String::new(),
        right_of_way: None,
        pedestrian_eligible: true,
    }
}

/// A straight way cut into pieces of about 40 m.
fn way(out: &mut Vec<eo_street::city::Segment>, id: &str, highway: &str, width: f64, walk: f64, a: [f64; 2], b: [f64; 2], separated: bool) {
    let len = (b[0] - a[0]).hypot(b[1] - a[1]);
    let n = ((len / 40.0).ceil() as usize).max(1);
    let p = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    for k in 0..n {
        out.push(seg(id, k, highway, width, walk, p(k as f64 / n as f64), p((k + 1) as f64 / n as f64), separated));
    }
}

struct Synthetic {
    w: PropsWorld,
    s: Arc<StructureWorld>,
    plain: PropsWorld,
}

/// A 14 m motorway deck (way 1301, x = 0, z -400..400, at grade beyond) over
/// a four-lane street running under it along x = 1 (ways 1400, 1401, z
/// -150..150, 7 m footways, `street` sets its section) and a four-lane street
/// crossing it at z = 0 (ways 1500, 1501), meeting at a signalised junction
/// under the deck: every approach's signals stand on masts.
fn synthetic(street: &dyn Fn(&mut eo_street::city::Segment)) -> Synthetic {
    let mut s = vec![];
    way(&mut s, "1300", "motorway", 14.0, 0.0, [0.0, -600.0], [0.0, -400.0], false);
    way(&mut s, "1301", "motorway", 14.0, 0.0, [0.0, -400.0], [0.0, 400.0], true);
    way(&mut s, "1302", "motorway", 14.0, 0.0, [0.0, 400.0], [0.0, 600.0], false);
    let mut st = vec![];
    way(&mut st, "1400", "residential", 14.0, 7.0, [1.0, -150.0], [1.0, 0.0], false);
    way(&mut st, "1401", "residential", 14.0, 7.0, [1.0, 0.0], [1.0, 150.0], false);
    for g in st.iter_mut() {
        g.lanes = Some(4.0);
        street(g);
    }
    s.extend(st);
    // (four lanes: two arriving on each approach, so its signals stand on masts.)
    let mut cross = vec![];
    way(&mut cross, "1500", "residential", 14.0, 3.0, [-150.0, 0.0], [1.0, 0.0], false);
    way(&mut cross, "1501", "residential", 14.0, 3.0, [1.0, 0.0], [150.0, 0.0], false);
    for g in cross.iter_mut() {
        g.lanes = Some(4.0);
    }
    s.extend(cross);
    let o = eo_street::StreetOptions::load().unwrap();
    let intersections = vec![eo_street::city::Intersection { id: "synthetic-signal".into(), k: None, x: 1.0, z: 0.0, sig: Some("traffic_signals".into()) }];
    let city = eo_street::StreetCity { segments: s, intersections, crossings: vec![], crossing_nodes: vec![], continental: vec![] };
    let lift = o.open_ground_lift;
    let street = Arc::new(eo_street::StreetWorld::build(o, city, eo_street::Terrain { grid: Arc::new(flat_grid()), carpet_lift: lift }, eo_street::BuildingIndex::new(vec![])));
    // the roads' tags (the real city file, its roads swapped for these).
    let paths = eo_data::DataPaths::discover().unwrap();
    let mut roads_city = eo_data::City::load(&paths).unwrap();
    let road = |id: &str, highway: &str, layer: i32, bridge: bool| eo_data::Road { id: id.into(), name: format!("Way {id}"), highway: highway.into(), lanes: 2, maxspeed: String::new(), bridge, tunnel: false, layer, points: vec![], points64: vec![] };
    roads_city.roads = vec![road("1300", "motorway", 0, false), road("1301", "motorway", 1, true), road("1302", "motorway", 0, false), road("1400", "residential", 0, false), road("1401", "residential", 0, false), road("1500", "residential", 0, false), road("1501", "residential", 0, false)];
    roads_city.buildings.clear();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let sw = Arc::new(StructureWorld::build(&street, &roads_city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions::default()).unwrap());
    let make = |s: Option<Arc<StructureWorld>>| {
        let w = PropsWorld::new(street.clone(), Arc::new(eo_props::JsProps::load().unwrap()), standards()).with_structures(s);
        w.install_furniture(&[]).unwrap();
        overhead::install(&w, vec![]).unwrap();
        w
    };
    Synthetic { w: make(Some(sw.clone())), s: sw, plain: make(None) }
}

fn deck_of(s: &StructureWorld) -> &eo_structures::Chain {
    s.chains().iter().find(|c| c.ways().contains(&"1301")).expect("the deck chain")
}

fn wide_footways() -> &'static Synthetic {
    static S: OnceLock<Synthetic> = OnceLock::new();
    S.get_or_init(|| synthetic(&|_| {}))
}

fn tree(variant: u16, x: f64, z: f64, y: f64) -> Prop {
    Prop { model: Model::Tree, pos: [x, y, z], yaw: 0.3, scale: [1.0, 1.0, 1.0], variant, live: Live::None }
}

#[test]
fn under_a_synthetic_deck_a_tree_is_made_smaller_or_dropped() {
    let syn = wide_footways();
    let d = deck_of(&syn.s);
    assert!(d.is_built(), "{:?}", d.status);
    let dims = syn.w.clear_dims();
    let species = eo_props::eztree::street_tree_species().unwrap();
    let height = |v: u16| eo_props::eztree::ez_species(&species[v as usize]).unwrap().height_m;
    let tall = (0..species.len() as u16).max_by(|&a, &b| height(a).total_cmp(&height(b))).unwrap();
    // a tall tree on the -n footway (x -6), under the deck's edge, between bents.
    let s_mid = d.bents.windows(2).map(|b| (b[0] + b[1]) / 2.0).find(|&m| (d.frame(m).z - 70.0).abs() < 25.0).expect("a span over the street");
    let z = d.frame(s_mid).z;
    let (x, y) = (-6.0, syn.w.ground(-6.0, z));
    let t = tree(tall, x, z, y);
    assert!(meets(&syn.s, dims, &t, None), "a full-size tree under the deck meets it");
    let soffit = d.y_at(s_mid) - d.depth_at(s_mid);
    let mut stats = clearance::ClearStats::default();
    match clearance::clear_prop(&syn.s, dims, &t, &mut stats) {
        Some(q) => {
            assert!(height(q.variant) < height(tall) && !meets(&syn.s, dims, &q, None));
            assert!(y + height(q.variant) < soffit - clearance::PROP_CLEAR_M, "its top under the soffit {soffit:.2}");
            assert_eq!(stats.trees_capped, 1);
        }
        None => assert_eq!(stats.dropped.get("Tree"), Some(&1)),
    }
    println!("a {:.1} m tree under a soffit {:.2} m up: {}", height(tall), soffit - y, if stats.trees_capped > 0 { "a smaller species" } else { "dropped" });
    // beside the deck, where its crown reaches over the deck's edge: a smaller species with a narrower crown.
    let mut swapped = None;
    for k in 0..24 {
        let x = -8.0 - 0.5 * k as f64;
        let t = tree(tall, x, z, syn.w.ground(x, z));
        if !meets(&syn.s, dims, &t, None) {
            break;
        }
        if let Some(q) = syn.w.clear_prop(t) {
            assert!(!meets(&syn.s, dims, &q, None) && q.variant != tall && height(q.variant) < height(tall) && q.scale == [1.0; 3]);
            swapped = Some((x, q.variant));
            break;
        }
    }
    let (sx, sv) = swapped.expect("a tree beside the deck kept as a smaller species");
    println!("a {:.1} m tree {:.1} m off the deck's centreline: a {:.1} m species ({}) instead", height(tall), -sx, height(sv), species[sv as usize]);
    // the same tree clear of the deck stands as it is.
    let beside = tree(tall, 24.0, z, syn.w.ground(24.0, z));
    assert_eq!(syn.w.clear_prop(beside).map(|p| (p.variant, p.scale)), Some((beside.variant, beside.scale)));
    // and on a column it goes, whatever its kind.
    let col = syn.s.all_column_footprints().into_iter().find(|f| f.site == ColumnSite::FurnitureZone).expect("a furniture-zone column");
    assert!(syn.w.clear_prop(tree(tall, col.x, col.z, syn.w.ground(col.x, col.z))).is_none(), "a tree on a column");
    // a palm has no smaller kind.
    let palm = Prop { model: Model::Palm, ..t };
    assert!(syn.w.clear_prop(palm).is_none());
    // without structures nothing changes.
    assert_eq!(syn.plain.clear_prop(t).map(|p| (p.variant, p.scale)), Some((t.variant, t.scale)));
}

#[test]
fn under_a_synthetic_deck_no_lamp_or_signal_mast_goes_through_it() {
    let syn = wide_footways();
    let d = deck_of(&syn.s);
    let dims = syn.w.clear_dims();
    let z = 70.0;
    // a district cobra head on the -n footway under the deck (its arm over the street, toward +x).
    let yaw = (-1.0f64).atan2(0.0);
    let lamp = Prop { model: Model::LampCobra, pos: [-6.5, syn.w.ground(-6.5, z), z], yaw, scale: [1.0; 3], variant: 0, live: Live::None };
    assert!(meets(&syn.s, dims, &lamp, None));
    assert!(syn.w.clear_prop(lamp).is_none(), "a lamp under the deck is not placed");
    let away = Prop { pos: [16.0, syn.w.ground(16.0, z), z], ..lamp };
    assert!(syn.w.clear_prop(away).is_some(), "a lamp clear of the deck stands");
    // the signalised junction under the deck.
    let ni = syn.w.street.plan.nodes.iter().position(|n| n.signalised()).expect("the signalised junction");
    let parts = eo_props::signals::plan_node_parts(&syn.w, ni);
    let mut plain = vec![];
    eo_props::signals::plan_node(&syn.plain, ni, &mut plain);
    let through = plain.iter().filter(|p| meets(&syn.s, dims, p, None)).count();
    let masts = parts.poles.iter().filter(|p| p.kind_mast).count();
    assert!(masts > 0 && through > 0, "the junction's masts ({masts}) reach the deck: {through} parts would meet it");
    let cleared = eo_props::signals::plan_node_clear(&syn.w, &syn.s, ni);
    for p in &cleared.props {
        assert!(!meets(&syn.s, dims, p, None), "{:?} at {:?} meets the deck", p.model, p.pos);
    }
    // every mast was cut down or dropped, nothing else lost that did not need to go.
    assert!(cleared.stats.pedestals + cleared.stats.dropped.get("SignalPole").copied().unwrap_or(0) >= 1);
    assert!(!cleared.changes.is_empty());
    // a pole the deck stands over is no taller than its soffit.
    for p in cleared.props.iter().filter(|p| p.model == Model::SignalPole) {
        let (s, dist) = d.project(p.pos[0], p.pos[2]);
        if dist < d.half_at(s) {
            assert!(p.pos[1] + (p.scale[1] as f64) < d.y_at(s) - d.depth_at(s), "a pole under the deck reaches its soffit");
        }
    }
    println!(
        "junction under the deck: {} parts planned ({masts} masts, {through} meeting the deck) -> {} kept, {} masts cut to pedestals, dropped {:?}",
        plain.len(),
        cleared.props.len(),
        cleared.stats.pedestals,
        cleared.stats.dropped
    );
    // the junction's heads keep their lamp channels (live) where kept.
    assert!(cleared.props.iter().any(|p| matches!(p.live, Live::Vehicle { .. })), "no vehicle head left at the junction");
}

#[test]
fn under_a_synthetic_deck_no_sign_is_seated_on_a_column() {
    let syn = wide_footways();
    let col = syn.s.all_column_footprints().into_iter().find(|f| f.site == ColumnSite::FurnitureZone && f.z.abs() > 20.0).expect("a furniture-zone column off the junction");
    let (edge, post_r) = (0.3, 0.03);
    // listed right on the column: as before, seated there; with the deck, moved off it.
    let plain = eo_props::signs::snap(&syn.plain, col.x, col.z, edge, post_r, true);
    assert_eq!(plain.map(|p| (p.0, p.1)), Some((col.x, col.z)), "without structures the post stands where it is listed");
    let s = &syn.s;
    let top = 2.5;
    let free = |x: f64, z: f64| {
        let g = syn.w.ground(x, z);
        !clearance::blocked(s, &Extent { x, z, r: edge, y0: g, y1: g + top }, None)
    };
    let seat = eo_props::signs::snap_with(&syn.w, col.x, col.z, edge, post_r, true, &free).expect("a seat along the kerb");
    let off = (seat.0 - col.x).hypot(seat.1 - col.z);
    assert!(off > col.radius + edge + clearance::PROP_CLEAR_M, "the post {off:.2} m from the column centre (radius {:.2})", col.radius);
    println!("a post listed on a column (r {:.2} m) moved {:.2} m along the kerb", col.radius, seat.2);
    // the posts placed by the tiles under the deck: none on a column.
    for key in syn.s.tile_keys() {
        for p in eo_props::signs::plan_tile_clear(&syn.w, s, key).iter().filter(|p| p.model == Model::SignPost) {
            for f in syn.s.all_column_footprints() {
                assert!((p.pos[0] - f.x).hypot(p.pos[2] - f.z) > f.radius + clearance::PROP_CLEAR_M, "a sign post on a column");
            }
        }
    }
}

#[test]
fn under_a_synthetic_deck_every_placed_item_is_clear_and_luminaires_stand_on_its_barriers() {
    let syn = wide_footways();
    let d = deck_of(&syn.s);
    let dims = syn.w.clear_dims();
    let mut checked = 0;
    for key in syn.s.tile_keys() {
        let (t, _) = syn.w.plan_tile_stats(key);
        for p in &t.props {
            let lamp = deck_lamp_chain(&syn.w, p.pos);
            assert!(!meets(&syn.s, dims, p, lamp), "{:?} at {:?} meets the deck", p.model, p.pos);
            checked += 1;
        }
        let o = overhead::plan_tile(&syn.w, key);
        for wr in &o.wires {
            for q in &wr.points {
                assert!(!syn.s.blocked(q[0] as f64, q[2] as f64, 0.0, q[1] as f64, q[1] as f64), "a wire through the deck");
            }
        }
    }
    assert!(checked > 0);
    // the deck luminaires: both edges, every DECK_PITCH_M from half a pitch in, the head DECK_HEAD_M over the deck.
    let (pitch, head) = (eo_landmarks::lights::DECK_PITCH_M, eo_landmarks::lights::DECK_HEAD_M);
    let mut lamps: Vec<(Prop, eo_props::furniture::LampLight)> = syn.w.deck_lamps().values().flatten().map(|l| (l.prop, l.light)).collect();
    lamps.sort_by(|a, b| d.project(a.0.pos[0], a.0.pos[2]).0.total_cmp(&d.project(b.0.pos[0], b.0.pos[2]).0));
    let expect = ((d.length() - pitch / 2.0) / pitch).floor() as usize + 1;
    assert_eq!(lamps.len(), 2 * expect, "two luminaires every {pitch} m along a {:.0} m deck", d.length());
    let mut per_side: BTreeMap<bool, Vec<f64>> = BTreeMap::new();
    for (p, l) in &lamps {
        let (s, dist) = d.project(p.pos[0], p.pos[2]);
        assert!(dist > d.half_at(s) && dist < d.half_at(s) + 0.7, "on the barrier: {dist:.2} m from the centreline");
        let deck = syn.s.deck_at(l.pos[0], l.pos[2], d.level).expect("over its deck").y;
        assert!((l.pos[1] - (deck + head)).abs() < 0.05, "head {:.2} m over the deck, not {head}", l.pos[1] - deck);
        assert!((l.base[1] - p.pos[1]).abs() < 1e-9 && p.pos[1] > deck + 1.0, "the pole stands on the barrier top");
        assert_eq!(l.kind, Model::LampCobra);
        assert!(l.intensity > 0.0 && l.range_m > head as f32);
        let side = (p.pos[0] - d.frame(s).x) * d.frame(s).nx + (p.pos[2] - d.frame(s).z) * d.frame(s).nz > 0.0;
        per_side.entry(side).or_default().push(s);
    }
    for (side, ss) in &per_side {
        assert_eq!(ss.len(), expect, "side {side}");
        assert!((ss[0] - pitch / 2.0).abs() < 0.05, "the first at half a pitch: {}", ss[0]);
        for w in ss.windows(2) {
            assert!((w[1] - w[0] - pitch).abs() < 0.05, "pitch {:.2}", w[1] - w[0]);
        }
    }
    println!("{} deck luminaires along the {:.0} m deck ({} a side, {pitch} m apart, heads {head} m over the deck)", lamps.len(), d.length(), expect);
    // none without structures.
    assert!(syn.plain.deck_lamps().is_empty());
}

#[test]
fn under_a_synthetic_deck_no_parked_car_stands_on_a_column() {
    // parking lanes either side: the columns stand in them.
    let syn = synthetic(&|g| {
        g.width = 14.0;
        g.parking = [3.0, 3.0];
        g.sidewalk_left = Some(3.0);
        g.sidewalk_right = Some(3.0);
        g.sidewalk_w = Some(3.0);
    });
    let cols: Vec<_> = syn.s.all_column_footprints().into_iter().filter(|f| f.site == ColumnSite::ParkingLane).collect();
    assert!(!cols.is_empty(), "columns in the parking lanes");
    let (mut before, mut after, mut on_columns_before) = (0, 0, 0);
    for (ei, e) in syn.w.street.plan.entries.iter().enumerate() {
        if !e.id.contains("-1400-") && !e.id.contains("-1401-") {
            continue;
        }
        let plain = eo_props::parking::plan_entry(&syn.plain, ei);
        let kept = eo_props::parking::plan_entry(&syn.w, ei);
        before += plain.len();
        after += kept.len();
        let on = |c: &eo_props::ParkedCar| cols.iter().any(|f| (c.pos[0] - f.x).hypot(c.pos[2] - f.z) < f.radius + 3.0);
        on_columns_before += plain.iter().filter(|c| on(c)).count();
        for c in &kept {
            let spec = syn.w.js.vehicle_specs.iter().find(|s| s.id == c.spec).unwrap();
            let fp = eo_props::parking_rules::vehicle_footprint(spec.length, spec.width, c.pos[0], c.pos[2], c.yaw, 0.0);
            for f in &cols {
                assert!(eo_props::parking_rules::quad_distance(&fp, [f.x, f.z]) > f.radius + clearance::PROP_CLEAR_M, "a parked car on a column");
            }
        }
    }
    println!("parked cars along the street under the deck: {before} -> {after} ({on_columns_before} within reach of a column before), {} parking-lane columns", cols.len());
    assert!(after <= before);
}
