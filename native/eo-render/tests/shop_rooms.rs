//! Shop rooms by trade (`--shop-light`, stage N2; shop_rooms.wgsl, needs a
//! GPU): every trade's room drawn alone through a storefront (the city
//! shader's own functions in a compute pass) is a lit room with things in it,
//! not a flat box: its mean near the window's luminance, real contrast
//! (fixtures far over the mean, corners and shelving under it), each trade
//! its own; a vacant shop an emptied room; the same twice. And the fronts
//! table the rooms read (shop_lights.rs `fronts_table`) is found on the GPU
//! exactly as on the CPU (`front_at`): one vacancy truth for the lights, the
//! papered glass and the rooms.
//!
//! `EO_SHOP_ROOMS_PNG=<dir>` also writes the rooms as a contact sheet
//! (`shop-rooms.png`: a row a trade, then vacant; night head on, night from
//! the side, by day) for looking at.

use eo_render::shop_lights::{self, Front};
use eo_render::{GpuContext, GpuOptions, read_mapped, wgpu};
use eo_sky::shops::{Fit, ShopKind};

const IW: u32 = 240;
const IH: u32 = 180;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

fn harness() -> String {
    format!(
        "@group(0) @binding(0) var<storage, read> lx_shop_fronts: array<vec4<u32>>;\n{}\n{}",
        shop_lights::rooms_wgsl(),
        r#"
struct Hp { eye: vec4<f32>, win: vec4<f32>, room: vec4<f32>, info: vec4<u32> };
@group(0) @binding(1) var<uniform> hp: Hp;
@group(0) @binding(2) var<storage, read_write> img: array<vec4<f32>>;
struct Q { pos: vec4<f32>, n: vec4<f32>, group: vec4<u32> };
@group(0) @binding(3) var<storage, read> queries: array<Q>;
@group(0) @binding(4) var<storage, read_write> answers: array<vec4<u32>>;

@compute @workgroup_size(8, 8)
fn sr_image(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = u32(hp.room.w);
    let h = hp.info.w >> 16u;
    if (id.x >= w || id.y >= h) {
        return;
    }
    let g = vec2<f32>(hp.win.x + (f32(id.x) + 0.5) / f32(w) * hp.win.z, hp.win.y + (1.0 - (f32(id.y) + 0.5) / f32(h)) * hp.win.w);
    let o = vec3<f32>(g, 0.0);
    let d = normalize(o - hp.eye.xyz);
    var r: SrRoom;
    r.size = hp.room.xyz;
    r.fit = hp.info.x;
    r.goods = hp.info.y;
    r.key = hp.info.z;
    r.vacant = (hp.info.w & 1u) != 0u;
    r.work = (hp.info.w & 2u) != 0u;
    r.kind = (hp.info.w >> 4u) & 15u;
    let out = sr_room(o, d, r, hp.eye.w, length(o - hp.eye.xyz));
    img[2u * (id.y * w + id.x)] = vec4<f32>(out.night, 1.0);
    img[2u * (id.y * w + id.x) + 1u] = vec4<f32>(out.day, 1.0);
}

@compute @workgroup_size(64)
fn sr_lookup(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= arrayLength(&queries)) {
        return;
    }
    let q = queries[id.x];
    let f = sr_front(q.group.x, q.pos.xyz, q.n.xyz);
    let info = f.fit | (f.goods << 4u) | (f.kind << 8u) | (u32(f.vacant) << 12u) | (u32(f.boarded) << 13u) | (u32(f.work) << 14u) | (u32(f.lum) << 16u);
    answers[2u * id.x] = vec4<u32>(select(0u, 1u, f.found), bitcast<u32>(f.s0), bitcast<u32>(f.s1), f.key);
    answers[2u * id.x + 1u] = vec4<u32>(info, bitcast<u32>(f.floor), bitcast<u32>(f.x + f.s0), u32(round(f.head * 100.0)));
}
"#
    )
}

struct Rig {
    img: wgpu::ComputePipeline,
    lookup: wgpu::ComputePipeline,
}

fn rig(ctx: &GpuContext) -> Rig {
    let src = harness();
    let module = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shop rooms harness"), source: wgpu::ShaderSource::Wgsl(src.into()) });
    let pipe = |e: &str| ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(e), layout: None, module: &module, entry_point: Some(e), compilation_options: Default::default(), cache: None });
    Rig { img: pipe("sr_image"), lookup: pipe("sr_lookup") }
}

fn storage(ctx: &GpuContext, bytes: &[u8], rw: bool) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    let usage = wgpu::BufferUsages::STORAGE | if rw { wgpu::BufferUsages::COPY_SRC } else { wgpu::BufferUsages::empty() };
    let mut data = bytes.to_vec();
    data.resize(data.len().max(16), 0);
    ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &data, usage })
}

fn read(ctx: &GpuContext, b: &wgpu::Buffer) -> Vec<u8> {
    let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: b.size(), usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_buffer_to_buffer(b, 0, &staging, 0, b.size());
    ctx.queue.submit([enc.finish()]);
    read_mapped(ctx, &staging).unwrap()
}

#[derive(Clone, Copy)]
struct Shop {
    fit: Fit,
    goods: u32,
    kind: ShopKind,
    key: u32,
    vacant: bool,
    work: bool,
    size: [f32; 3],
}

/// The room of `s` through a 4.4 m x 2.7 m storefront from `eye` (room
/// frame: x along the front, y up from the floor, z in; the glass at z 0):
/// per pixel (night radiance per unit window radiance, day albedo).
fn draw(ctx: &GpuContext, rig: &Rig, s: &Shop, eye: [f32; 3], win: [f32; 4]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let mut hp: Vec<u8> = vec![];
    let pix = 1.0f32 / 900.0; // a pixel's angle: a 1920 px frame over a 60-degree field
    for v in [eye[0], eye[1], eye[2], pix, win[0], win[1], win[2], win[3], s.size[0], s.size[1], s.size[2], IW as f32] {
        hp.extend_from_slice(&v.to_le_bytes());
    }
    let flags = s.vacant as u32 | (s.work as u32) << 1 | (s.kind as u32) << 4 | IH << 16;
    for v in [s.fit as u32, s.goods, s.key, flags] {
        hp.extend_from_slice(&v.to_le_bytes());
    }
    use wgpu::util::DeviceExt;
    let ub = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &hp, usage: wgpu::BufferUsages::UNIFORM });
    let img = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (IW * IH * 32) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
    let layout = rig.img.get_bind_group_layout(0);
    let bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 1, resource: ub.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: img.as_entire_binding() },
        ],
    });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&rig.img);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(IW.div_ceil(8), IH.div_ceil(8), 1);
    }
    ctx.queue.submit([enc.finish()]);
    let raw: Vec<f32> = bytemuck::pod_collect_to_vec(&read(ctx, &img));
    let (mut night, mut day) = (vec![], vec![]);
    for p in raw.chunks_exact(8) {
        night.push([p[0], p[1], p[2]]);
        day.push([p[4], p[5], p[6]]);
    }
    (night, day)
}

fn luma(c: &[f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn shops() -> Vec<(&'static str, Shop)> {
    let s = |fit, goods, kind, key: u32, w: f32| Shop { fit, goods, kind, key, vacant: false, work: false, size: [w, 3.6, 9.0] };
    vec![
        ("market", s(Fit::Shelves, 0, ShopKind::Market, 0x2b7e_1516, 6.2)),
        ("liquor", s(Fit::Shelves, 1, ShopKind::Liquor, 0x28ae_d2a6, 5.0)),
        ("pharmacy", s(Fit::Shelves, 2, ShopKind::Pharmacy, 0xabf7_1588, 6.8)),
        ("hardware", s(Fit::Shelves, 3, ShopKind::Retail, 0x09cf_4f3c, 5.4)),
        ("books", s(Fit::Shelves, 4, ShopKind::Retail, 0x7615_7ab1, 4.6)),
        ("flowers", s(Fit::Shelves, 5, ShopKind::Retail, 0x1234_5678, 4.2)),
        ("clothes", s(Fit::Racks, 0, ShopKind::Retail, 0x3c6e_f372, 6.0)),
        ("restaurant", s(Fit::Dining, 0, ShopKind::Restaurant, 0xa54f_f53a, 6.4)),
        ("cafe", s(Fit::Dining, 0, ShopKind::Cafe, 0x510e_527f, 4.8)),
        ("bar", s(Fit::Bar, 0, ShopKind::Bar, 0x9b05_688c, 5.6)),
        ("salon", s(Fit::Salon, 0, ShopKind::Salon, 0x1f83_d9ab, 5.2)),
        ("laundromat", s(Fit::Laundry, 0, ShopKind::Laundromat, 0x5be0_cd19, 6.6)),
        ("gallery", s(Fit::Gallery, 0, ShopKind::Retail, 0xcbbb_9d5d, 5.8)),
        ("clinic", s(Fit::Clinic, 0, ShopKind::Salon, 0x629a_292a, 5.0)),
        ("cleaners", s(Fit::Cleaners, 0, ShopKind::Salon, 0x9159_015a, 4.6)),
        ("gym", s(Fit::Fitness, 0, ShopKind::Fitness, 0x152f_ecd8, 6.0)),
        ("vacant", Shop { vacant: true, work: true, ..s(Fit::Racks, 0, ShopKind::Retail, 0x6734_6c79, 5.6) }),
    ]
}

/// The display (0-255) the game gives linear radiance `c` at `exposure` (eo-look `tone::display`).
fn shown(c: [f32; 3], exposure: f64) -> [u8; 3] {
    eo_look::tone::display(c.map(|v| v as f64), exposure, [0.5, 0.5]).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The same through the photo finish (`--photo-finish`, eo-look `tone::photo_display`; the frame's middle).
fn shown_photo(c: [f32; 3], exposure: f64) -> [u8; 3] {
    eo_look::tone::photo_display(c.map(|v| v as f64), exposure, [0.5, 0.5], [0.577, 0.325]).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// At the night street's exposure (eo-sky `night_exposure` at 21:00; the
/// game's display transform), each lit shop shows its room: its lit surfaces
/// at mid tones with real contrast, its fixtures blown out, not a flat
/// white box (stage N1's room at the window's full luminance shows 255
/// over the whole pane).
#[test]
fn every_trade_draws_a_lit_room_with_things_in_it() {
    let ctx = ctx();
    let rig = rig(&ctx);
    let list = shops();
    let exposure = eo_sky::night::night_exposure(&eo_look::LookFrame::new(21.0), 0.0);
    let units = eo_sky::night::lamp_units_per_lux();
    // N1's room: the window's luminance over the whole pane
    let flat = shown([1.0; 3].map(|v: f32| v * (75.0 * units) as f32), exposure);
    println!("night exposure {exposure:.3}; a pane at a 75 cd/m2 window's full luminance shows {flat:?}");
    let head_on = |s: &Shop| ([s.size[0] * 0.5, 1.6, -4.5], [0.0, 0.3, s.size[0], 2.7]);
    let side = |s: &Shop| ([-3.5, 1.6, -5.0], [0.0, 0.3, s.size[0], 2.7]);
    let mut sheet: Vec<Vec<[u8; 3]>> = vec![];
    // (the checks are gathered and asserted after the sheet is written, so a failing room can be looked at)
    let mut bad: Vec<String> = vec![];
    let mut check = |ok: bool, what: String| {
        if !ok {
            bad.push(what);
        }
    };
    let mut means = vec![];
    let mut nights = vec![];
    for (name, s) in &list {
        let (eye, win) = head_on(s);
        let (night, day) = draw(&ctx, &rig, s, eye, win);
        let (eye2, win2) = side(s);
        let (night2, _) = draw(&ctx, &rig, s, eye2, win2);
        assert!(night.iter().chain(&day).chain(&night2).all(|c| c.iter().all(|v| v.is_finite() && *v >= 0.0)), "{name}: finite, not negative");
        let mut l: Vec<f32> = night.iter().map(luma).collect();
        let mean = l.iter().sum::<f32>() / l.len() as f32;
        // the shop's own window luminance (its light: a vacant one's work light), in the lamps' units
        let open = eo_sky::shops::luminance(s.kind, s.key);
        let lum = open * shop_lights::room_adapt(open) * if s.vacant { eo_sky::shops::VACANT_LIGHT } else { 1.0 };
        let px = |c: &[f32; 3]| shown(c.map(|v| v * (lum * units) as f32), exposure);
        let mut d: Vec<f32> = night.iter().map(|c| luma(&px(c).map(|v| v as f32))).collect();
        l.sort_by(f32::total_cmp);
        d.sort_by(f32::total_cmp);
        let q = |v: &Vec<f32>, f: f32| v[((v.len() - 1) as f32 * f) as usize];
        let clipped = d.iter().filter(|v| **v >= 250.0).count() as f32 / d.len() as f32;
        let day_mean = day.iter().map(luma).sum::<f32>() / day.len() as f32;
        println!(
            "{name:>10} ({lum:5.1} cd/m2): radiance mean {mean:.3} of the window's (max {:.1}); shown p10 {:.0} p50 {:.0} p90 {:.0}, {:.1} % at 250+; day albedo mean {day_mean:.3}",
            q(&l, 1.0),
            q(&d, 0.1),
            q(&d, 0.5),
            q(&d, 0.9),
            clipped * 100.0
        );
        if !s.vacant {
            // its lit surfaces at mid tones (a bar's dim room lower, a restaurant's lower than a shop's),
            // not black, not white: the middle over shops of this trade (their own paint and goods)
            // (a bar is a dim room: its middle dark, its back bar and the pools on its counter over it)
            let mid = match s.kind {
                ShopKind::Bar => 12.0..=170.0,
                ShopKind::Restaurant | ShopKind::Cafe => 50.0..=225.0,
                _ => 70.0..=225.0,
            };
            let mut mids: Vec<f32> = (0..6u32)
                .map(|j| {
                    let o = Shop { key: s.key.wrapping_mul(2654435761).wrapping_add(j * 0x9e37_79b9), ..*s };
                    let (n, _) = draw(&ctx, &rig, &o, eye, win);
                    let lo = eo_sky::shops::luminance(o.kind, o.key);
                    let lo = lo * shop_lights::room_adapt(lo);
                    let mut v: Vec<f32> = n.iter().map(|c| luma(&shown(c.map(|x| x * (lo * units) as f32), exposure).map(|x| x as f32))).collect();
                    v.sort_by(f32::total_cmp);
                    v[v.len() / 2]
                })
                .collect();
            mids.sort_by(f32::total_cmp);
            let typical = 0.5 * (mids[2] + mids[3]);
            check(mid.contains(&typical), format!("{name}: the room's middle shows {typical} (over six shops {mids:?})"));
            // real contrast: the shelving, the goods and the corners under it, the walls and tops over it
            check(q(&d, 0.9) - q(&d, 0.1) >= 60.0 && q(&d, 0.9) >= 60.0, format!("{name}: shown p10 {} p90 {}", q(&d, 0.1), q(&d, 0.9)));
            // not a flat box: little of the pane white; its fixtures (or lit displays) blown out
            check(clipped <= 0.2, format!("{name}: {:.1} % of the pane at 250+", clipped * 100.0));
            check(q(&d, 1.0) >= 250.0, format!("{name}: no fixture blown out ({})", q(&d, 1.0)));
            // the photo finish shows it no flatter
            let mut ph: Vec<f32> = night.iter().map(|c| luma(&shown_photo(c.map(|v| v * (lum * units) as f32), exposure).map(|v| v as f32))).collect();
            ph.sort_by(f32::total_cmp);
            let ph_clip = ph.iter().filter(|v| **v >= 250.0).count() as f32 / ph.len() as f32;
            check(ph_clip <= 0.2 && q(&ph, 0.9) - q(&ph, 0.1) >= 60.0, format!("{name}: photo finish p10 {} p90 {}, {:.1} % at 250+", q(&ph, 0.1), q(&ph, 0.9), ph_clip * 100.0));
            check((0.04..=0.6).contains(&day_mean), format!("{name}: day albedo {day_mean}"));
            means.push(mean);
            nights.push((name, night.iter().map(|c| luma(c) / mean).collect::<Vec<f32>>()));
        } else {
            // emptied and dark: its work light's bare bulb, the rest dim
            check(q(&d, 1.0) >= 200.0, format!("vacant: the work light's bulb ({})", q(&d, 1.0)));
            check(q(&d, 0.5) <= 40.0, format!("vacant: the room's middle shows {}", q(&d, 0.5)));
        }
        // the sheet: night head on (the game's display at the night exposure, and 2 stops darker), from the side, by day
        let mut rows = vec![vec![]; IH as usize];
        for y in 0..IH as usize {
            for (img, k) in [(&night, 1.0f32), (&night, 0.25), (&night2, 1.0), (&day, 0.0)] {
                for x in 0..IW as usize {
                    let c = img[y * IW as usize + x];
                    rows[y].push(if k > 0.0 { px(&c.map(|v| v * k)) } else { shown(c.map(|v| v * 0.5), 1.0) });
                }
            }
        }
        sheet.extend(rows);
    }
    // each trade its own room
    for i in 0..nights.len() {
        for j in i + 1..nights.len() {
            let d = nights[i].1.iter().zip(&nights[j].1).map(|(a, b)| (a.min(4.0) - b.min(4.0)).abs()).sum::<f32>() / nights[i].1.len() as f32;
            check(d > 0.08, format!("{} and {} look alike ({d})", nights[i].0, nights[j].0));
        }
    }
    // the same twice
    let (a, _) = draw(&ctx, &rig, &list[7].1, head_on(&list[7].1).0, head_on(&list[7].1).1);
    let (b, _) = draw(&ctx, &rig, &list[7].1, head_on(&list[7].1).0, head_on(&list[7].1).1);
    assert!(a.iter().zip(&b).all(|(x, y)| x.iter().zip(y).all(|(p, q)| p.to_bits() == q.to_bits())), "deterministic");
    if let Ok(dir) = std::env::var("EO_SHOP_ROOMS_PNG") {
        let w = IW * 4;
        let h = sheet.len() as u32;
        let file = std::fs::File::create(std::path::Path::new(&dir).join("shop-rooms.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        let data: Vec<u8> = sheet.iter().flat_map(|r| r.iter().flat_map(|c| c.iter().copied())).collect();
        wr.write_image_data(&data).unwrap();
        println!("wrote {dir}/shop-rooms.png ({w} x {h}: a row a trade, then vacant; night head on at the night exposure and 2 stops under, from the side, by day)");
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The fronts table: the GPU's lookup (shop_rooms.wgsl `sr_front`) finds
/// exactly the front the CPU's does (`front_at`), its vacancy and trade with
/// it, and nothing off the shop's front, behind it or in another building.
#[test]
fn the_rooms_find_the_same_front_as_the_placement() {
    let ctx = ctx();
    let rig = rig(&ctx);
    let mut fronts = vec![];
    // three buildings (groups 40, 41, 45) kilometres from the origin, shops along fronts of different headings
    for (b, group, heading, n_shops) in [(0u32, 40u32, 0.3f32, 4u32), (1, 41, 2.1, 1), (2, 45, -1.2, 3)] {
        for t in 0..n_shops {
            let key = eo_sky::shops::shop_key(&format!("b{b}"), t);
            let kind = ShopKind::ALL[(key % 10) as usize];
            let (nx, nz) = (heading.cos() as f64, heading.sin() as f64);
            let along = 5.0 * t as f64;
            fronts.push(Front {
                building: b,
                tenancy: t,
                group,
                anchor: [2900.0 + 40.0 * b as f64 - nz * along, -1600.0 + 30.0 * b as f64 + nx * along],
                s: [-2.5, 2.5],
                glass: [-2.3, 2.3],
                floor: 12.5,
                head: 3.1,
                heading,
                kind,
                fit: eo_sky::shops::fit_of(kind, "", ""),
                goods: 0,
                vacant: eo_sky::shops::vacant(kind, key) || t == 2,
                boarded: t == 2,
                work: false,
                key,
                lum: eo_sky::shops::luminance(kind, key) as f32,
            });
        }
    }
    let table = shop_lights::fronts_table(&fronts);
    // queries: on each front's glass, off its ends, behind it, the wrong way, another group
    let mut qs: Vec<([f32; 3], [f32; 3], u32)> = vec![];
    let mut rng = 0x1234_5678u32;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng as f32 / u32::MAX as f32
    };
    for f in &fronts {
        let (nx, nz) = (f.heading.cos(), f.heading.sin());
        let (tx, tz) = (-nz, nx);
        for _ in 0..40 {
            let s = f.s[0] - 1.0 + (f.s[1] - f.s[0] + 2.0) * next();
            let off = (next() - 0.5) * 3.2;
            let p = [f.anchor[0] as f32 + tx * s + nx * off, f.floor + 3.0 * next(), f.anchor[1] as f32 + tz * s + nz * off];
            // a pane in the front's own plane (a little off), or one facing away (another face)
            let turn = if next() < 0.12 { std::f32::consts::PI } else { (next() - 0.5) * 0.02 };
            let n = [(f.heading + turn).cos(), 0.0, (f.heading + turn).sin()];
            let group = if next() < 0.1 { f.group + 2 } else { f.group };
            qs.push((p, n, group));
        }
    }
    let mut qb: Vec<u8> = vec![];
    for (p, n, g) in &qs {
        for v in [p[0], p[1], p[2], 0.0, n[0], n[1], n[2], 0.0] {
            qb.extend_from_slice(&v.to_le_bytes());
        }
        for v in [*g, 0, 0, 0] {
            qb.extend_from_slice(&v.to_le_bytes());
        }
    }
    let tb = storage(&ctx, bytemuck::cast_slice(&table), false);
    let qbuf = storage(&ctx, &qb, false);
    let ans = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (qs.len() * 32) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
    let layout = rig.lookup.get_bind_group_layout(0);
    let bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: tb.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: qbuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: ans.as_entire_binding() },
        ],
    });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&rig.lookup);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups((qs.len() as u32).div_ceil(64), 1, 1);
    }
    ctx.queue.submit([enc.finish()]);
    let got: Vec<[u32; 4]> = bytemuck::pod_collect_to_vec(&read(&ctx, &ans));
    let (mut found, mut missed, mut vacant) = (0, 0, 0);
    for (i, (p, n, g)) in qs.iter().enumerate() {
        let cpu = shop_lights::front_at(&table, *g, *p, *n);
        let (a, b) = (got[2 * i], got[2 * i + 1]);
        match cpu {
            None => {
                assert_eq!(a[0], 0, "query {i}: the GPU found a front the CPU did not");
                missed += 1;
            }
            Some([r0, r1]) => {
                assert_eq!(a[0], 1, "query {i}: the GPU missed {r0:?}");
                assert_eq!((a[1], a[2], a[3]), (r1[0], r1[1], r1[3]), "query {i}: span and key");
                assert_eq!(b[0], r1[2], "query {i}: trade and vacancy bits");
                assert_eq!((b[1], b[3]), (r0[2], r0[3] >> 16), "query {i}: floor, head");
                // the pane's place along the front, within a millimetre of the CPU's
                let f = fronts.iter().find(|f| f.key == r1[3]).unwrap();
                let (nx, nz) = (f.heading.cos() as f64, f.heading.sin() as f64);
                let cpu_s = (p[0] as f64 - f.anchor[0]) * -nz + (p[2] as f64 - f.anchor[1]) * nx;
                assert!((f32::from_bits(b[2]) as f64 - cpu_s).abs() < 2e-3, "query {i}: along {} vs {cpu_s}", f32::from_bits(b[2]));
                // the record is the front the query was made on, with its vacancy
                assert!(f.group == *g && (b[0] >> 12) & 1 == f.vacant as u32);
                vacant += f.vacant as usize;
                found += 1;
            }
        }
    }
    println!("{} queries: {found} on a shop front ({vacant} vacant), {missed} on none", qs.len());
    assert!(found > qs.len() / 3 && missed > qs.len() / 10);
}
