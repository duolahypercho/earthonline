//! Lit shop windows that carry the night street (`--shop-light 1|2`, stage
//! N1; eo-sky `shops`): a light just inside the glass of every shop window
//! the kit draws, in the trade's colour, at its window's luminance, lit by its
//! opening hours; through the frame's light list (sky.rs `set_shop_lights`,
//! sky_lights.wgsl `lights_shops`), so it is clustered, occluded with the
//! lamp shadows on (light_shadow.rs; no map of its own), lights the fog
//! (lit_fog.wgsl) and the room behind the glass reads it (rooms.wgsl
//! `rm_shop_light`: a lit shop's interior lit in its light, a dark one dark).
//!
//! Where: the kit planner's shop bays (eo-kit shop_ground.rs, the plan's
//! `shopBays`: each storefront window's span along its front edge, its sill
//! and head, the glass's setback and the shop's identity), planned again here
//! for every kit building with a retail ground floor, so a light stands
//! exactly behind the glass the kit draws. The building's floor is the one
//! the kit cooks it on (`kit::kit_floor`). One light a window bay (level 1) or
//! two across it at its quarter points (level 2), each a point source
//! `SETBACK_M` behind the glass at the window's middle height, aimed out
//! along the front's normal and `DOWN_TILT` down, with the window's
//! Lambertian lobe (sky_light.wgsl's spot shape for kinds 3 and up: cone 90
//! degrees out, full along the normal). Its intensity is the lit window's
//! luminance (eo-sky `shops::luminance`) times the bay's glazed area; its
//! range where it falls to `FLOOR_LUX`.

use crate::kit::KitSet;
use crate::scene::NO_GROUP;
use eo_kit::city::KitCity;
use eo_sky::shops::{self, Fit, ShopKind};

/// The lights stand this far behind the glass (m): inside the shop, where
/// the light the window lets out leaves from (a window's spill is brightest a
/// metre or two out, dim at the stall riser's foot). INFERRED.
pub const SETBACK_M: f64 = 0.3;
/// Aimed down from the front's normal by this much (rad): the lit floor and
/// displays behind the glass send more down than up. INFERRED.
pub const DOWN_TILT: f64 = 0.18;
/// A light's range ends where its axis falls to this illuminance (lux):
/// past it a shop adds less than a tenth of a local street's average (SF
/// local streets 0.4-0.9 fc, 4-10 lux; reference sheet 3.3).
pub const FLOOR_LUX: f64 = 0.6;
/// The ranges the lights are kept within (m).
pub const RANGE_M: [f64; 2] = [5.0, 18.0];
/// The lights are gathered within this far of the eye (m), by level: 1 the
/// near street (a block and a half each way), 2 the clusters' whole reach.
pub const REACH_M: [f64; 2] = [160.0, 300.0];
/// The bays a light spans at most (m): a wider window takes more lights at level 2.
pub const BAY_SPLIT_M: f64 = 2.0;

/// One shop light as the GPU reads it (sky_lights.wgsl `ShopLamp`), 64 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuShopLight {
    pub pos: [f32; 3],
    /// Where its light ends (m).
    pub range: f32,
    /// Linear sRGB at unit luminance.
    pub colour: [f32; 3],
    /// Candela along its axis while the shop is open.
    pub candela: f32,
    /// Its axis: out of the window, a little down.
    pub dir: [f32; 3],
    /// The glazed area it stands for (m2): its window's luminance is `candela / area`.
    pub area: f32,
    /// The hour it opens, the hours it is lit from then (24: always; 0: never), its level while closed.
    pub open: f32,
    pub span: f32,
    pub security: f32,
    /// `ShopKind` (diagnostics).
    pub kind: u32,
}
const _: () = assert!(std::mem::size_of::<GpuShopLight>() == 64);

/// One storefront window the kit draws (its plan's `shopBays` entry), in world metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bay {
    pub building: u32,
    pub tenancy: u32,
    /// The glass's middle (x, y, z) and its plan's outward normal (x, z).
    pub centre: [f64; 3],
    pub normal: [f64; 2],
    /// Along the front, up.
    pub width: f64,
    pub height: f64,
    pub kind: ShopKind,
    /// (stage N2) The plan's edge the window stands on and its span along
    /// that edge's basis (the facade clutter's boards are laid in the same).
    pub edge: u32,
    pub span: [f64; 2],
    /// (stage N2) The shop's floor (world y), what fills its room and its goods (eo-sky `shops::fit_of`, `goods_of`).
    pub floor: f64,
    pub fit: Fit,
    pub goods: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShopStats {
    /// Kit buildings with a retail ground floor planned, the ones that drew shop bays.
    pub planned: usize,
    pub with_bays: usize,
    pub bays: usize,
    pub shops: usize,
    pub lights: usize,
    /// Shops by kind (`ShopKind::name`).
    pub by_kind: Vec<(&'static str, usize)>,
    /// (stage N2) Shops vacant (eo-sky `shops::vacant`, or boarded), of them
    /// boarded up by the facade clutter, and vacant ones with a work light.
    pub vacant: usize,
    pub boarded: usize,
    pub work_lit: usize,
    pub ms: f64,
}

/// The shop bays the kit planner gives building `b` (none without a shop
/// ground floor or a plan), on its floor as the kit cooks it: on the street
/// front's walk (`kit::kit_floor`'s, without its foundation, from the plan's
/// own front) when `streets` is given, else at `fallback_base` (the
/// footprint's lowest ground, the kit's own fallback). The planner lifts every
/// part by the base it is given and decides nothing else by it, so the plan is
/// made at 0 and lifted here.
pub fn bays_of(kc: &KitCity, b: u32, streets: Option<&eo_street::StreetWorld>, fallback_base: f64) -> Vec<Bay> {
    bays_and_boards(kc, b, streets, fallback_base, 0).0
}

/// `bays_of`, and (stage N2) the shop fronts the facade clutter boards up at
/// `clutter_level` (eo-kit `clutter::plan`, level 2 only): (edge, span along
/// its basis), exactly as the kit draws them (the same plan).
pub fn bays_and_boards(kc: &KitCity, b: u32, streets: Option<&eo_street::StreetWorld>, fallback_base: f64, clutter_level: u8) -> (Vec<Bay>, Vec<(u32, [f64; 2])>) {
    let rec = &kc.records[b as usize];
    let Some(c) = eo_kit::pipeline::classify_loaded(rec) else { return (vec![], vec![]) };
    if !c.has_retail_ground {
        return (vec![], vec![]);
    }
    let Some(planned) = eo_kit::pipeline::plan_building(rec, &kc.world, b, 0.0) else { return (vec![], vec![]) };
    let boards: Vec<(u32, [f64; 2])> = if clutter_level >= 2 {
        eo_kit::clutter::plan(&planned, clutter_level).boxes.iter().filter(|(k, ..)| *k == "boarding").map(|(_, e, _, bx)| (*e as u32, [bx.s0, bx.s1])).collect()
    } else {
        vec![]
    };
    let base_y = match streets {
        Some(sw) => {
            let ring: Vec<[f64; 2]> = rec.polygon.iter().map(|p| [p.x, p.z]).collect();
            let front = (planned.front.source == "street" || planned.front.source == "service")
                .then(|| planned.frame.edges.iter().find(|e| e.index == planned.front.index))
                .flatten()
                .map(|e| eo_street::grounding::FrontEdge { start: [e.start.x, e.start.z], u: [e.u.x, e.u.z], n: [e.n.x, e.n.z], length: e.length });
            sw.street_front_floor(&ring, front.as_ref()).y
        }
        None => fallback_base,
    };
    let Some(list) = planned.plan.extra.get("shopBays").and_then(|v| v.as_array()) else { return (vec![], boards) };
    let f = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_f64());
    let mut out = vec![];
    for bay in list {
        let (Some(s0), Some(s1), Some(y0), Some(y1)) = (f(bay, "s0"), f(bay, "s1"), f(bay, "y0"), f(bay, "y1")) else { continue };
        let basis = &bay["basis"];
        let v2 = |k: &str| -> Option<[f64; 2]> { Some([basis[k][0].as_f64()?, basis[k][1].as_f64()?]) };
        let (Some(o), Some(u), Some(n)) = (v2("origin"), v2("u"), v2("n")) else { continue };
        if s1 - s0 < 0.3 || y1 - y0 < 0.3 {
            continue;
        }
        let w = f(bay, "glassW").unwrap_or(-0.18);
        let s = 0.5 * (s0 + s1);
        let centre = [o[0] + u[0] * s + n[0] * w, base_y + 0.5 * (y0 + y1), o[1] + u[1] * s + n[1] * w];
        let (shop_type, text) = (bay["shopType"].as_str().unwrap_or(""), bay["text"].as_str().unwrap_or(""));
        let kind = shops::kind_of(shop_type, text);
        let floor = base_y + f(bay, "floorY").unwrap_or(0.0);
        out.push(Bay {
            building: b,
            tenancy: bay["tenancy"].as_u64().unwrap_or(0) as u32,
            centre,
            normal: n,
            width: s1 - s0,
            height: y1 - y0,
            kind,
            edge: f(bay, "edge").unwrap_or(0.0) as u32,
            span: [s0, s1],
            floor,
            fit: shops::fit_of(kind, shop_type, text),
            goods: shops::goods_of(kind, shop_type, text),
        });
    }
    (out, boards)
}

/// (stage N2) Whether a bay stands behind one of the facade clutter's boards (its edge, overlapping its span).
pub fn boarded(bay: &Bay, boards: &[(u32, [f64; 2])]) -> bool {
    boards.iter().any(|(e, s)| *e == bay.edge && s[0] < bay.span[1] - 0.05 && s[1] > bay.span[0] + 0.05)
}

/// The lights of one shop window (`level` 1: one at its middle; 2: one a
/// `BAY_SPLIT_M` of its width, at least two), for the shop of `key`.
pub fn lights_of(bay: &Bay, key: u32, level: u8) -> Vec<GpuShopLight> {
    lights_of_front(bay, key, level, false)
}

/// `lights_of`, (stage N2) for a front the facade clutter boards up when
/// `boarded`: placed (so every bay keeps its lights) but never lit.
pub fn lights_of_front(bay: &Bay, key: u32, level: u8, boarded: bool) -> Vec<GpuShopLight> {
    let h = if boarded { shops::Hours { open: 0.0, span: 0.0, security: 0.0 } } else { shops::hours(bay.kind, key) };
    let col = shops::colour(bay.kind, key);
    let lum = shops::luminance(bay.kind, key);
    let n = if level >= 2 { ((bay.width / BAY_SPLIT_M).ceil() as usize).max(2) } else { 1 };
    let area = bay.width * bay.height / n as f64;
    let candela = lum * area;
    let range = shops::reach_m(candela, FLOOR_LUX).clamp(RANGE_M[0], RANGE_M[1]);
    let [nx, nz] = bay.normal;
    let len = nx.hypot(nz).max(1e-9);
    let (nx, nz) = (nx / len, nz / len);
    let (tx, tz) = (-nz, nx);
    let (c, s) = (DOWN_TILT.cos(), DOWN_TILT.sin());
    (0..n)
        .map(|k| {
            let along = (k as f64 + 0.5) / n as f64 - 0.5;
            let p = [bay.centre[0] + tx * along * bay.width - nx * SETBACK_M, bay.centre[1], bay.centre[2] + tz * along * bay.width - nz * SETBACK_M];
            GpuShopLight {
                pos: p.map(|v| v as f32),
                range: range as f32,
                colour: col.map(|v| v as f32),
                candela: candela as f32,
                dir: [(nx * c) as f32, (-s) as f32, (nz * c) as f32],
                area: area as f32,
                open: h.open as f32,
                span: h.span as f32,
                security: h.security as f32,
                kind: bay.kind as u32,
            }
        })
        .collect()
}

/// Every kit-drawn shop window's lights (`level` 1 or 2), planned on
/// `threads` worker threads. The kit's floors stand on `set.streets` (else
/// the footprint's lowest ground, as the kit's own fallback).
pub fn build(kc: &KitCity, set: &KitSet, level: u8, threads: usize) -> (Vec<GpuShopLight>, ShopStats) {
    let (lights, _, st) = build_with_fronts(kc, set, level, threads);
    (lights, st)
}

/// `build`, and (stage N2) every shop's front for the rooms shader
/// (`fronts_table`): one per shop, its vacancy the lights' own (eo-sky
/// `shops::vacant`), a front the facade clutter boards up (`--facade-clutter
/// 2`, eo-kit `clutter::level`) counted vacant and its lights never lit.
pub fn build_with_fronts(kc: &KitCity, set: &KitSet, level: u8, threads: usize) -> (Vec<GpuShopLight>, Vec<Front>, ShopStats) {
    let t = std::time::Instant::now();
    let clutter_level = eo_kit::clutter::level();
    let candidates: Vec<u32> = (0..kc.records.len() as u32).filter(|&b| set.building_group.get(b as usize).is_some_and(|g| *g != NO_GROUP)).collect();
    let chunk = candidates.len().div_ceil(threads.max(1)).max(1);
    let parts: Vec<(Vec<(Bay, bool)>, usize)> = std::thread::scope(|s| {
        let hs: Vec<_> = candidates
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    crate::hitch::worker_started();
                    let mut bays = vec![];
                    let mut planned = 0;
                    for &b in c {
                        if !eo_kit::pipeline::classify_loaded(&kc.records[b as usize]).is_some_and(|c| c.has_retail_ground) {
                            continue;
                        }
                        planned += 1;
                        let (list, boards) = bays_and_boards(kc, b, set.streets.as_deref(), set.base_y[b as usize] as f64, clutter_level);
                        // a board covers its whole tenancy: every bay of a shop with one boarded bay is boarded.
                        let shut: std::collections::BTreeSet<u32> = list.iter().filter(|bay| boarded(bay, &boards)).map(|bay| bay.tenancy).collect();
                        bays.extend(list.into_iter().map(|bay| (bay, shut.contains(&bay.tenancy))));
                    }
                    (bays, planned)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap_or_default()).collect()
    });
    let mut st = ShopStats::default();
    let mut lights = vec![];
    let mut kinds: std::collections::BTreeMap<ShopKind, usize> = Default::default();
    let mut shops_seen: std::collections::BTreeSet<(u32, u32)> = Default::default();
    let mut buildings: std::collections::BTreeSet<u32> = Default::default();
    let mut all: Vec<(Bay, bool, u32)> = vec![];
    for (bays, planned) in parts {
        st.planned += planned;
        for (bay, shut) in bays {
            st.bays += 1;
            buildings.insert(bay.building);
            let key = shops::shop_key(&kc.records[bay.building as usize].id, bay.tenancy);
            if shops_seen.insert((bay.building, bay.tenancy)) {
                *kinds.entry(bay.kind).or_default() += 1;
            }
            lights.extend(lights_of_front(&bay, key, level, shut));
            all.push((bay, shut, key));
        }
    }
    let fronts = fronts_of(&all, |b| set.building_group[b as usize]);
    st.with_bays = buildings.len();
    st.shops = shops_seen.len();
    st.lights = lights.len();
    st.by_kind = kinds.into_iter().map(|(k, n)| (k.name(), n)).collect();
    st.vacant = fronts.iter().filter(|f| f.vacant).count();
    st.boarded = fronts.iter().filter(|f| f.boarded).count();
    st.work_lit = fronts.iter().filter(|f| f.work).count();
    st.ms = t.elapsed().as_secs_f64() * 1e3;
    *CENSUS.lock().unwrap() = Some(st.clone());
    (lights, fronts, st)
}

/// (stage N2) A shop's room reaches this far past its end windows along the
/// front (m): a door beside them, the reveal. INFERRED (a shop door and its
/// jambs, about 1.2 m).
pub const FRONT_END_M: f64 = 1.6;
/// A pane belongs to a front within this of its glass plane (m): a door's
/// recess stands behind the windows. INFERRED.
pub const FRONT_PLANE_M: f64 = 1.2;
/// ... and facing within this of its heading (cosine).
pub const FRONT_FACING: f64 = 0.9;
/// The heading's steps in its 16 bits.
const HEADING_STEPS: f64 = 65536.0;

/// (stage N2) One shop's front as the rooms shader reads it (shop_rooms.wgsl
/// `sr_front`): the room behind every pane of its tenancy (its door and its
/// windows look into one room), and the one vacancy truth. Its frame is its
/// own: an anchor on its glass and its outward normal, so a pane is placed
/// along the front in centimetres wherever the city puts it (measured along
/// a world axis, a heading off by a thousandth of a radian moves a pane
/// kilometres from the origin by metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Front {
    pub building: u32,
    pub tenancy: u32,
    /// The kit building's LOD group (draw.wgsl's `objects[..].lod_group`).
    pub group: u32,
    /// The middle of its windows on their glass plane (world x, z).
    pub anchor: [f64; 2],
    /// Its room's span along the front from the anchor, along the tangent
    /// (-n.z, n.x) of the outward normal n (m): its windows, out to halfway to
    /// the next shop on that face and at most `FRONT_END_M` past its end windows.
    pub s: [f32; 2],
    /// Its windows' span the same way (the neighbours' rooms stop halfway to it).
    pub glass: [f32; 2],
    /// Its floor (world y) and its windows' head over it (m).
    pub floor: f32,
    pub head: f32,
    /// The outward normal's heading atan2(n.z, n.x) (rad).
    pub heading: f32,
    pub kind: ShopKind,
    pub fit: Fit,
    pub goods: u32,
    /// Vacant (eo-sky `shops::vacant`, or boarded), boarded up (the facade
    /// clutter's plywood), its work light on (a vacant front's).
    pub vacant: bool,
    pub boarded: bool,
    pub work: bool,
    /// Its shop key (eo-sky `shops::shop_key`): its room's seed.
    pub key: u32,
    /// Its lit window's luminance (cd/m2, eo-sky `shops::luminance`): the rooms' adaptation (`room_adapt`).
    pub lum: f32,
}

impl Front {
    fn axes(&self) -> ([f64; 2], [f64; 2]) {
        let h = self.heading as f64;
        let n = [h.cos(), h.sin()];
        (n, [-n[1], n[0]])
    }

    /// The two table rows (`fronts_table`): (anchor x, anchor z, floor as f32
    /// bits, heading in 1/65536 turn | head in cm << 16); (room start, room end
    /// along the front from the anchor as f32 bits, fit | goods << 4 | kind << 8
    /// | vacant << 12 | boarded << 13 | work << 14 | its luminance in whole cd/m2
    /// (1-255) << 16, key).
    pub fn rows(&self) -> [[u32; 4]; 2] {
        let turn = (self.heading as f64).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU;
        let heading = ((turn * HEADING_STEPS).round() as u32) & 0xffff;
        let head = ((self.head as f64 * 100.0).round().clamp(0.0, 65535.0)) as u32;
        let lum = (self.lum.round() as u32).clamp(1, 255);
        let info = self.fit as u32 | (self.goods & 15) << 4 | (self.kind as u32) << 8 | (self.vacant as u32) << 12 | (self.boarded as u32) << 13 | (self.work as u32) << 14 | lum << 16;
        [
            [(self.anchor[0] as f32).to_bits(), (self.anchor[1] as f32).to_bits(), self.floor.to_bits(), heading | head << 16],
            [self.s[0].to_bits(), self.s[1].to_bits(), info, self.key],
        ]
    }
}

/// The fronts of these bays (each with its boarded flag and shop key), one a
/// shop; `group` the building's LOD group.
pub fn fronts_of(bays: &[(Bay, bool, u32)], group: impl Fn(u32) -> u32) -> Vec<Front> {
    let mut by_shop: std::collections::BTreeMap<(u32, u32), Vec<usize>> = Default::default();
    for (i, (bay, ..)) in bays.iter().enumerate() {
        by_shop.entry((bay.building, bay.tenancy)).or_default().push(i);
    }
    let mut fronts: Vec<Front> = vec![];
    for ((building, tenancy), list) in by_shop {
        let (first, shut, key) = bays[list[0]];
        let [nx, nz] = first.normal;
        let len = nx.hypot(nz).max(1e-9);
        let (nx, nz) = (nx / len, nz / len);
        let (tx, tz) = (-nz, nx);
        let c0 = first.centre;
        let (mut s0, mut s1, mut off, mut head) = (f64::INFINITY, f64::NEG_INFINITY, 0.0, f64::NEG_INFINITY);
        for &i in &list {
            let b = &bays[i].0;
            let d = [b.centre[0] - c0[0], b.centre[2] - c0[2]];
            let s = d[0] * tx + d[1] * tz;
            s0 = s0.min(s - 0.5 * b.width);
            s1 = s1.max(s + 0.5 * b.width);
            off += d[0] * nx + d[1] * nz;
            head = head.max(b.centre[1] + 0.5 * b.height - first.floor);
        }
        let (mid, off) = (0.5 * (s0 + s1), off / list.len() as f64);
        let anchor = [c0[0] + tx * mid + nx * off, c0[2] + tz * mid + nz * off];
        let half = (0.5 * (s1 - s0)) as f32;
        let vacant = shops::vacant(first.kind, key) || shut;
        fronts.push(Front {
            building,
            tenancy,
            group: group(building),
            anchor,
            s: [-half - FRONT_END_M as f32, half + FRONT_END_M as f32],
            glass: [-half, half],
            floor: first.floor as f32,
            head: head as f32,
            heading: nz.atan2(nx) as f32,
            kind: first.kind,
            fit: first.fit,
            goods: first.goods,
            vacant,
            boarded: shut,
            work: !shut && shops::work_light(first.kind, key),
            key,
            lum: shops::luminance(first.kind, key) as f32,
        });
    }
    // a front's room reaches halfway to the next shop's windows on the same face of its building
    // (whichever plan edge they stand on: a face may be split into collinear edges).
    let mut by_building: std::collections::BTreeMap<u32, Vec<usize>> = Default::default();
    for (i, f) in fronts.iter().enumerate() {
        by_building.entry(f.building).or_default().push(i);
    }
    for list in by_building.values() {
        for &a in list {
            let (na, ta) = fronts[a].axes();
            let [g0, g1] = fronts[a].glass;
            let (mut lo, mut hi) = (fronts[a].s[0], fronts[a].s[1]);
            for &b in list {
                let (nb, _) = fronts[b].axes();
                if a == b || na[0] * nb[0] + na[1] * nb[1] < FRONT_FACING {
                    continue;
                }
                let d = [fronts[b].anchor[0] - fronts[a].anchor[0], fronts[b].anchor[1] - fronts[a].anchor[1]];
                if (d[0] * na[0] + d[1] * na[1]).abs() > FRONT_PLANE_M {
                    continue;
                }
                let at = (d[0] * ta[0] + d[1] * ta[1]) as f32;
                let (b0, b1) = (at + fronts[b].glass[0], at + fronts[b].glass[1]);
                if b1 <= g0 {
                    lo = lo.max(0.5 * (b1 + g0));
                } else if b0 >= g1 {
                    hi = hi.min(0.5 * (g1 + b0));
                }
            }
            fronts[a].s = [lo, hi];
        }
    }
    fronts
}

/// (stage N2) The fronts as the rooms shader's table (look.rs group 1
/// binding 19, `array<vec4<u32>>`; shop_rooms.wgsl `sr_front`): row 0 (first
/// group, groups, first record row, records); then one u32 a group from the
/// first, (first record << 8 | records), 0 for none; then two rows a record
/// (`Front::rows`), a building's together. Empty: no fronts.
pub fn fronts_table(fronts: &[Front]) -> Vec<[u32; 4]> {
    if fronts.is_empty() {
        return vec![];
    }
    let mut fs = fronts.to_vec();
    fs.sort_by_key(|f| (f.group, f.tenancy));
    let g0 = fs[0].group;
    let groups = fs[fs.len() - 1].group - g0 + 1;
    let first_record = 1 + groups.div_ceil(4);
    let mut out = vec![[0u32; 4]; first_record as usize + 2 * fs.len()];
    out[0] = [g0, groups, first_record, fs.len() as u32];
    let mut i = 0;
    while i < fs.len() {
        let g = fs[i].group;
        let mut j = i;
        while j < fs.len() && fs[j].group == g {
            j += 1;
        }
        let n = (j - i).min(255) as u32;
        let k = (g - g0) as usize;
        out[1 + k / 4][k % 4] = (i as u32) << 8 | n;
        i = j;
    }
    for (k, f) in fs.iter().enumerate() {
        let [a, b] = f.rows();
        out[first_record as usize + 2 * k] = a;
        out[first_record as usize + 2 * k + 1] = b;
    }
    out
}

/// What the rooms shader finds in the table for a pane at `pos` (world) of
/// outward normal `n` in LOD group `group` (shop_rooms.wgsl `sr_front`, the
/// same steps): the record's two rows.
pub fn front_at(table: &[[u32; 4]], group: u32, pos: [f32; 3], n: [f32; 3]) -> Option<[[u32; 4]; 2]> {
    let h = *table.first()?;
    if h[1] == 0 || group < h[0] || group >= h[0] + h[1] {
        return None;
    }
    let k = (group - h[0]) as usize;
    let e = table[1 + k / 4][k % 4];
    let (first, count) = ((e >> 8) as usize, (e & 255) as usize);
    let nl = (n[0] * n[0] + n[2] * n[2]).sqrt().max(1e-6);
    let (nx, nz) = (n[0] / nl, n[2] / nl);
    for r in first..first + count {
        let a = table[h[2] as usize + 2 * r];
        let b = table[h[2] as usize + 2 * r + 1];
        let turn = (a[3] & 0xffff) as f32 / HEADING_STEPS as f32 * std::f32::consts::TAU;
        let (rx, rz) = (turn.cos(), turn.sin());
        if rx * nx + rz * nz < FRONT_FACING as f32 {
            continue;
        }
        let q = [pos[0] - f32::from_bits(a[0]), pos[2] - f32::from_bits(a[1])];
        let s = q[0] * -rz + q[1] * rx;
        let off = q[0] * rx + q[1] * rz;
        if off.abs() > FRONT_PLANE_M as f32 || s < f32::from_bits(b[0]) || s > f32::from_bits(b[1]) {
            continue;
        }
        return Some([a, b]);
    }
    None
}

/// The fronts table on the GPU (look.rs `set_shop_fronts`); None when empty.
pub fn fronts_buffer(device: &wgpu::Device, queue: &wgpu::Queue, table: &[[u32; 4]]) -> Option<wgpu::Buffer> {
    if table.is_empty() {
        return None;
    }
    let b = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shop-fronts"),
        size: (table.len() * 16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&b, 0, bytemuck::cast_slice(table));
    Some(b)
}

static CENSUS: std::sync::Mutex<Option<ShopStats>> = std::sync::Mutex::new(None);

/// The last placement's census (`build`; None before any): the bench JSON's `shop_light.census`.
pub fn census() -> Option<ShopStats> {
    CENSUS.lock().unwrap().clone()
}

/// The eye distance the lights are gathered within at `level` (0: none).
pub fn reach_m(level: u8) -> f64 {
    match level {
        0 => 0.0,
        1 => REACH_M[0],
        _ => REACH_M[1],
    }
}

/// The sky uniform's flag bits (`Sky.flags.x`, sky.rs) while the shop
/// windows' lights are placed and on: any level, and level 2 (the reach).
/// Clear of `SkyOptions::bits`' (1..16384).
pub const SKY_SHOPS: u32 = 1 << 15;
pub const SKY_SHOPS_FAR: u32 = 1 << 16;
/// The room behind a shop's glass reads its light within this share of the
/// lights' reach, and gives way to the rooms' own rule over the rest (rooms.wgsl).
pub const ROOM_FADE_FROM: f64 = 0.8;

/// The WGSL constants of the shop lights (sky_lights.wgsl, rooms.wgsl).
pub fn wgsl() -> String {
    format!(
        "\n// generated by shop_lights.rs wgsl\nconst SHOP_KIND: u32 = {SHOP_LIGHT_KIND}u;\nconst SKY_SHOPS: u32 = {SKY_SHOPS}u;\nconst SKY_SHOPS_FAR: u32 = {SKY_SHOPS_FAR}u;\nconst SHOP_REACH_NEAR: f32 = {:.1};\nconst SHOP_REACH_FAR: f32 = {:.1};\nconst SHOP_ROOM_FADE: f32 = {ROOM_FADE_FROM:.3};\n",
        REACH_M[0], REACH_M[1]
    )
}

/// (stage N2) The shop rooms' brightness (shop_rooms.wgsl), per unit of the
/// window's luminance (N1's `shops::luminance`, in the lamps' units: the
/// light the window lets out, its fixtures' light on the walk). What the eye
/// sees through the glass is that light redistributed: the fixtures (an LED
/// strip, a troffer, a pendant's mouth: some thousands of cd/m2 against a
/// window's 16-180) blown out and small, the lit surfaces (goods, shelving,
/// floor and walls at a mixed reflectance near 0.3, lit to about 0.7 of the
/// fixtures' plane and falling to the back and the corners) where the
/// night street's exposure keeps them: `ROOM_GAIN` puts a mid-grey shelf at
/// about a tenth of the window's luminance, which the night exposure shows
/// at 150-200 of 255 for a 75 cd/m2 shop (a pane at the full luminance
/// shows white, the flat box N1 left). That is the eye's adaptation to the
/// lit window, and a phone's merged exposures (the reference photo, sheet
/// 5.10 Castro and 18th, shows the goods inside the bright shops); a single
/// exposure for the street would clip them. The glows over the lit surfaces:
/// a cooler's doors lit from inside, the light caught on bottles, a lit
/// kitchen pass, a back bar's LED strip, a display case; the light at the
/// room's back against its front; the eye distance the fittings are drawn
/// within. INFERRED (sheet 3.7 has no interior values; IES retail practice
/// 500-1000 lux on a sales floor, the back and the corners a half to a third
/// of it; tests/shop_rooms.rs checks the display at the night exposure).
pub const ROOM_GAIN: f64 = 0.36;
pub const ROOM_FIXTURE: f64 = 14.0;
pub const ROOM_COOLER: f64 = 0.22;
pub const ROOM_GLINT: f64 = 0.5;
pub const ROOM_KITCHEN: f64 = 0.7;
pub const ROOM_STRIP: f64 = 5.0;
pub const ROOM_CASE_GLOW: f64 = 0.45;
pub const ROOM_PENDANT: f64 = 12.0;
pub const ROOM_BACK_FALL: f64 = 0.5;
pub const ROOM_DETAIL_M: f64 = 110.0;
/// (stage N2) The eye adapts to each lit window it looks into: shop to shop,
/// a room is seen at its window's luminance compressed toward this one
/// (cd/m2) by this power (a 180 cd/m2 market reads about 1.7 times a 60
/// cd/m2 shop, not 3; a 16 cd/m2 bar half of it, not a quarter: brightness
/// grows as a power of luminance well under 1, Stevens' 0.33-0.5 for an
/// extended field). Only what is seen through the glass: the light the
/// window lets out on the walk is N1's. INFERRED.
pub const ROOM_ADAPT_REF: f64 = 60.0;
pub const ROOM_ADAPT_POWER: f64 = 0.5;

/// The factor a shop's room is seen at, for its window's luminance `lum` (cd/m2; shop_rooms.wgsl `sr_adapt`).
pub fn room_adapt(lum: f64) -> f64 {
    (ROOM_ADAPT_REF / lum.clamp(1.0, 255.0)).powf(1.0 - ROOM_ADAPT_POWER)
}

/// (stage N2) The shop rooms' WGSL (rooms.rs `wgsl_constants`, the city
/// shader only): their constants and shop_rooms.wgsl.
pub fn rooms_wgsl() -> String {
    format!(
        "\n// generated by shop_lights.rs rooms_wgsl\nconst SR_HEADING_STEPS: f32 = {HEADING_STEPS:.1};\nconst SR_FACING: f32 = {FRONT_FACING:.4};\nconst SR_PLANE: f32 = {FRONT_PLANE_M:.4};\nconst SR_GAIN: f32 = {ROOM_GAIN:.4};\nconst SR_FIXTURE: f32 = {ROOM_FIXTURE:.4};\nconst SR_COOLER: f32 = {ROOM_COOLER:.4};\nconst SR_GLINT: f32 = {ROOM_GLINT:.4};\nconst SR_KITCHEN: f32 = {ROOM_KITCHEN:.4};\nconst SR_STRIP: f32 = {ROOM_STRIP:.4};\nconst SR_CASE_GLOW: f32 = {ROOM_CASE_GLOW:.4};\nconst SR_PENDANT: f32 = {ROOM_PENDANT:.4};\nconst SR_BACK_FALL: f32 = {ROOM_BACK_FALL:.4};\nconst SR_DETAIL_M: f32 = {ROOM_DETAIL_M:.1};\nconst SR_ADAPT_REF: f32 = {ROOM_ADAPT_REF:.1};\nconst SR_ADAPT_POWER: f32 = {ROOM_ADAPT_POWER:.4};\n{}",
        include_str!("shop_rooms.wgsl")
    )
}

/// The frame list's kind of a shop light (sky_light.wgsl: a spot of the
/// window's lobe, as kind 3; sky_fx draws no lens or pool for it, the lamp
/// shadows give it no map, lit_fog.wgsl lights the fog round it every way).
pub const SHOP_LIGHT_KIND: u32 = 4;

#[cfg(test)]
mod tests {
    use super::*;

    fn bay(kind: ShopKind, width: f64) -> Bay {
        Bay { building: 1, tenancy: 0, centre: [10.0, 1.8, 5.0], normal: [0.0, 1.0], width, height: 2.4, kind, edge: 0, span: [8.5, 8.5 + width], floor: 0.2, fit: Fit::Racks, goods: 0 }
    }

    /// A window's lights stand behind its glass across it, aimed out and
    /// down, at its luminance times its area, ranging where they fall to the
    /// floor illuminance; level 2 splits the window.
    #[test]
    fn a_windows_lights_stand_behind_its_glass_and_carry_its_luminance() {
        let b = bay(ShopKind::Retail, 3.0);
        let key = shops::shop_key("b1", 0);
        let one = lights_of(&b, key, 1);
        assert_eq!(one.len(), 1);
        let l = one[0];
        assert!((l.pos[2] as f64 - (5.0 - SETBACK_M)).abs() < 1e-5 && (l.pos[0] - 10.0).abs() < 1e-5, "{:?}", l.pos);
        assert!(l.dir[2] > 0.9 && l.dir[1] < 0.0, "out and down: {:?}", l.dir);
        let lum = shops::luminance(ShopKind::Retail, key);
        assert!((l.candela as f64 - lum * 3.0 * 2.4).abs() < 1e-3);
        assert!((l.area - 7.2).abs() < 1e-5);
        assert!(l.range >= RANGE_M[0] as f32 && l.range <= RANGE_M[1] as f32);
        let two = lights_of(&b, key, 2);
        assert_eq!(two.len(), 2);
        let mut xs: Vec<f32> = two.iter().map(|l| l.pos[0]).collect();
        xs.sort_by(f32::total_cmp);
        assert!((xs[0] - 9.25).abs() < 1e-5 && (xs[1] - 10.75).abs() < 1e-5, "{xs:?}");
        assert!(((two[0].candela + two[1].candela) - l.candela).abs() < 1e-3, "the window's light split, not doubled");
        assert_eq!(lights_of(&bay(ShopKind::Retail, 5.0), key, 2).len(), 3);
        // the same shop, the same lights.
        assert_eq!(lights_of(&b, key, 1), one);
        let h = shops::hours(ShopKind::Retail, key);
        assert_eq!((l.open, l.span, l.security), (h.open as f32, h.span as f32, h.security as f32));
        assert!(wgsl().contains(&format!("const SHOP_KIND: u32 = {SHOP_LIGHT_KIND}u;")));
        // its flag bits are clear of the sky options' own.
        let common = include_str!("sky_common.wgsl");
        for bit in [SKY_SHOPS, SKY_SHOPS_FAR] {
            assert!(bit > 16384 && !common.contains(&format!("= {bit}u;")), "bit {bit} is taken");
        }
    }
    /// (stage N2) A building's shops: one front a shop over all its bays,
    /// its room reaching halfway to the next shop and past its end windows,
    /// its vacancy the lights' own (a boarded front vacant and never lit);
    /// the table finds each front from a pane on its glass, nothing behind it,
    /// off its ends or in another building.
    #[test]
    fn a_buildings_shops_become_fronts_the_rooms_find() {
        let at = |tenancy: u32, x: f64, width: f64, kind: ShopKind, edge: u32| Bay {
            building: 3,
            tenancy,
            centre: [3000.0 + x, 1.9, 5.0],
            normal: [0.0, 1.0],
            width,
            height: 2.4,
            kind,
            edge,
            span: [x - width * 0.5, x + width * 0.5],
            floor: 0.3,
            fit: shops::fit_of(kind, "", ""),
            goods: 0,
        };
        // one face, its last shop on a collinear plan edge of its own (a face split at a vertex)
        let bays = vec![at(0, 2.0, 2.0, ShopKind::Cafe, 1), at(0, 4.4, 2.0, ShopKind::Cafe, 1), at(1, 8.0, 3.0, ShopKind::Retail, 1), at(2, 13.0, 2.5, ShopKind::Bar, 2)];
        let kt = |t: u32| shops::shop_key("b3", t);
        let list: Vec<(Bay, bool, u32)> = bays.iter().map(|b| (*b, b.tenancy == 2, kt(b.tenancy))).collect();
        let fronts = fronts_of(&list, |_| 77);
        assert_eq!(fronts.len(), 3, "one front a shop");
        let f0 = fronts.iter().find(|f| f.tenancy == 0).unwrap();
        let f1 = fronts.iter().find(|f| f.tenancy == 1).unwrap();
        let f2 = fronts.iter().find(|f| f.tenancy == 2).unwrap();
        // anchored on the middle of each shop's windows; the normal +z: along the front is -x
        assert!((f0.anchor[0] - 3003.2).abs() < 1e-9 && (f0.anchor[1] - 5.0).abs() < 1e-9 && (f0.glass[1] - 2.2).abs() < 1e-6, "{f0:?}");
        // rooms meet halfway between neighbours' windows, across the edge split too; free ends reach FRONT_END_M
        let room = |f: &Front| (f.anchor[0] - f.s[1] as f64, f.anchor[0] - f.s[0] as f64);
        let (r0, r1, r2) = (room(f0), room(f1), room(f2));
        assert!((r0.0 - (3001.0 - FRONT_END_M)).abs() < 1e-4 && (r0.1 - 3005.95).abs() < 1e-4, "{r0:?}");
        assert!((r1.0 - 3005.95).abs() < 1e-4 && (r1.1 - 3010.625).abs() < 1e-4, "{r1:?}");
        assert!((r2.0 - 3010.625).abs() < 1e-4 && (r2.1 - 3014.25 - FRONT_END_M).abs() < 1e-4, "{r2:?}");
        assert!((f0.head - (1.9 + 1.2 - 0.3)).abs() < 1e-5 && f0.group == 77);
        // the vacancy: the rule's, a boarded shop vacant and not work-lit; its lights never lit
        assert_eq!(f0.vacant, shops::vacant(ShopKind::Cafe, kt(0)));
        assert!(f2.vacant && f2.boarded && !f2.work);
        let shut = lights_of_front(&bays[3], kt(2), 2, true);
        assert!(shut.iter().all(|l| l.span == 0.0 && l.security == 0.0), "a boarded front is never lit");
        let open = lights_of_front(&bays[2], kt(1), 1, false);
        assert_eq!(open[0].span == 0.0, f1.vacant, "the lights' vacancy is the front's");
        // the table: a pane on each shop's glass finds it, its door beside the end window too
        let table = fronts_table(&fronts);
        let n = [0.0f32, 0.0, 1.0];
        let find = |x: f32, z: f32, g: u32| front_at(&table, g, [3000.0 + x, 1.5, z], n).map(|[_, b]| b[3]);
        assert_eq!(find(2.0, 5.0, 77), Some(kt(0)));
        assert_eq!(find(4.9, 4.9, 77), Some(kt(0)), "the pane's own plane");
        assert_eq!(find(6.4, 4.2, 77), Some(kt(1)), "a recessed door between two shops");
        assert_eq!(find(10.3, 5.0, 77), Some(kt(1)), "across the edge split, nearer its windows");
        assert_eq!(find(10.9, 5.0, 77), Some(kt(2)));
        assert_eq!(find(15.0, 5.0, 77), Some(kt(2)), "a door past the end window");
        assert_eq!(find(17.0, 5.0, 77), None, "off the front's end");
        assert_eq!(find(8.0, 7.0, 77), None, "off the glass plane");
        assert_eq!(front_at(&table, 77, [3008.0, 1.5, 5.0], [0.0, 0.0, -1.0]), None, "facing the other way");
        // a pane whose normal is a little off the plan's (the kit's normals are bytes) finds the same shop
        assert_eq!(front_at(&table, 77, [3008.0, 1.5, 5.0], [0.02, 0.0, 1.0]).map(|[_, b]| b[3]), Some(kt(1)));
        assert_eq!(find(8.0, 5.0, 76), None, "another building");
        // the rows decode to the front
        let [a, b] = f1.rows();
        assert_eq!((f32::from_bits(b[0]), f32::from_bits(b[1]), b[3]), (f1.s[0], f1.s[1], f1.key));
        assert_eq!((f32::from_bits(a[0]) as f64, f32::from_bits(a[2])), (f1.anchor[0] as f32 as f64, f1.floor));
        assert_eq!(b[2] & 15, f1.fit as u32);
        assert_eq!((b[2] >> 12) & 1, f1.vacant as u32);
        // the rooms shader reads its vacancy from the front for W5's paper and dirt
        let rooms = include_str!("rooms.wgsl");
        assert!(rooms.contains("wx_vacant = select(0.0, 1.0, front.found && front.vacant);") && rooms.contains("rm_w5.z = wx_vacant;"));
        assert!(rooms_wgsl().contains("fn sr_front(") && rooms_wgsl().contains(&format!("const SR_PLANE: f32 = {FRONT_PLANE_M:.4};")));
    }
}
