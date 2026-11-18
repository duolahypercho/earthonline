// the far terrain (eo-render/src/farshore.rs). Appended to the
// look's shared code (look_common.wgsl + sky_common.wgsl via
// :look_common_source) and sky_light_core.wgsl, so the land takes the
// city's sun, sky light, cloud shadows, fog colour and the sky's aerial path
// density. A CDLOD quadtree of one patch mesh, morphed between levels; heights
// from the cooked heightfield (surveyed metres, game y), put on the ground
// plan here (x s about the map origin, heights x h above sea level).
//
// Depth: the city's reverse-Z depth reaches 0 at its far plane; this layer
// writes max(city depth, FS_DEPTH_K / view depth), so past the far plane it
// sorts behind every city surface and in true order among itself. Deep inside
// the city's grid (where the city draws its own ground) it writes only
// FS_DEPTH_K / view depth: always behind the city, seen only past the far plane.

struct FarU {
    view_proj: mat4x4<f32>,
    // xyz eye, w the camera's far plane (m)
    cam: vec4<f32>,
    // plan scale s, height scale h, sea level y, haze tail (x the range fog's far end)
    frame: vec4<f32>,
    // cooked origin x, z (surveyed m), 1 / (texel x width), 1 / (texel x height)
    tex: vec4<f32>,
    // texel (game m), top level, morph start (share of a level's range), level-0 range (game m)
    lod: vec4<f32>,
    // the city grid (game m): min x, min z, max x, max z
    grid: vec4<f32>,
    // the sea's reach (game m): min x, min z, max x, max z
    reach: vec4<f32>,
    // dryness of the grass (0 green .. 1 golden), radians per pixel, self-shadow (1/0), debug
    misc: vec4<f32>,
    // the sea's grid this frame (eo-water grid): centre x, z, half size, drawn (1/0)
    sea: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fu: FarU;
@group(0) @binding(1) var fs_height: texture_2d<f32>;
@group(0) @binding(2) var fs_class: texture_2d<f32>;
@group(0) @binding(3) var fs_samp: sampler;
@group(0) @binding(4) var fs_shape: texture_2d<f32>; // sky visibility, valley

@group(1) @binding(0) var<uniform> look: Look;

const FS_PI: f32 = 3.14159265;
// metres the land inside the city grid is drawn under the city's own ground
const FS_TUCK: f32 = 2.0;

fn fs_uv(p: vec2<f32>) -> vec2<f32> {
    return (p / fu.frame.x - fu.tex.xy) * fu.tex.zw;
}

// :lift_by: heights above mean sea level x h, the sea and below unchanged.
fn fs_lift(y: f32) -> f32 {
    let d = y - fu.frame.z;
    return select(y, fu.frame.z + d * fu.frame.y, d > 0.0);
}

fn fs_h(p: vec2<f32>, mip: f32) -> f32 {
    return fs_lift(textureSampleLevel(fs_height, fs_samp, fs_uv(p), mip).r);
}

fn fs_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

fn fs_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(fs_hash(i), fs_hash(i + vec2<f32>(1.0, 0.0)), u.x), mix(fs_hash(i + vec2<f32>(0.0, 1.0)), fs_hash(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

struct FsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    // the cooked height before the sea clamp, and 1 where the sea draws (its reach)
    @location(1) hr: vec2<f32>,
};

@vertex
fn fs_vs(@location(0) g: vec2<u32>, @location(1) inst: vec4<f32>) -> FsOut {
    let level = inst.w;
    let sp = inst.z / 32.0;
    let gf = vec2<f32>(g);
    let p = inst.xy + gf * sp;
    // CDLOD morph: past `lod.z` of this level's range the odd vertices slide onto the next level's lattice.
    let h0 = fs_h(p, level);
    let d = distance(vec3<f32>(p.x, h0, p.y), fu.cam.xyz);
    let r = fu.lod.w * exp2(level);
    var m = clamp((d - r * fu.lod.z) / (r * (1.0 - fu.lod.z)), 0.0, 1.0);
    if (level >= fu.lod.y) {
        m = 0.0;
    }
    let odd = fract(gf * 0.5) * 2.0;
    let pm = p - odd * sp * m;
    let h = mix(fs_h(pm, level), fs_h(pm, level + 1.0), m);
    let in_reach = all(pm >= fu.reach.xy) && all(pm <= fu.reach.zw);
    // Outside the sea's reach this layer draws the water itself, at sea level,
    // but only south of the grid (the South Bay, where the sea does not reach);
    // past the reach elsewhere (the cooked rim) nothing under the sea is
    // drawn (its grazing sky made a white line along the horizon).
    // and wherever the sea's grid does not reach this frame (it is 4.4 km round
    // a camera near the water, so the bay between it and a far shore showed the sky).
    let in_sea = fu.sea.w > 0.5 && in_reach && all(abs(pm - fu.sea.xy) <= vec2<f32>(fu.sea.z));
    let own_water = !in_sea && ((pm.x >= fu.reach.x && pm.x <= fu.reach.z && pm.y < fu.grid.w + 60000.0) || in_reach);
    var y = h;
    if (own_water) {
        y = max(h, fu.frame.z);
    }
    let inside = all(pm > fu.grid.xy) && all(pm < fu.grid.zw);
    if (inside && !(own_water && h <= fu.frame.z)) {
        y = y - FS_TUCK;
    }
    var o: FsOut;
    let world = vec3<f32>(pm.x, y, pm.y);
    o.clip = fu.view_proj * vec4<f32>(world, 1.0);
    let margin = 2.0 * fu.lod.x;
    let deep = all(pm > fu.grid.xy + vec2<f32>(margin)) && all(pm < fu.grid.zw - vec2<f32>(margin));
    o.clip.z = select(max(o.clip.z, FS_DEPTH_K), FS_DEPTH_K, deep);
    o.world = world;
    o.hr = vec2<f32>(h, select(1.0, 0.0, own_water));
    return o;
}

fn fs_sh(n: vec3<f32>) -> vec3<f32> {
    let x = n.x;
    let y = n.y;
    let z = n.z;
    var e = look.sh[0].xyz * 0.282095;
    e = e + look.sh[1].xyz * (0.488603 * y) + look.sh[2].xyz * (0.488603 * z) + look.sh[3].xyz * (0.488603 * x);
    e = e + look.sh[4].xyz * (1.092548 * x * y) + look.sh[5].xyz * (1.092548 * y * z) + look.sh[6].xyz * (0.315392 * (3.0 * z * z - 1.0));
    e = e + look.sh[7].xyz * (1.092548 * x * z) + look.sh[8].xyz * (0.546274 * (x * x - y * y));
    return max(e, vec3<f32>(0.0));
}

// The ridges' own shadow: the height field marched toward the key light
// (12 steps, 40 m to ~9 km), soft over ~1 degree.
fn fs_self_shadow(world: vec3<f32>, l: vec3<f32>) -> f32 {
    let lh = length(l.xz);
    if (fu.misc.z < 0.5 || lh < 1e-4 || l.y <= 0.0) {
        return 1.0;
    }
    let dir = l.xz / lh;
    let rise = l.y / lh;
    var vis = 1.0;
    var t = fu.lod.x;
    for (var i = 0; i < 12; i = i + 1) {
        t = t * 1.55;
        let p = world.xz + dir * t;
        let mip = clamp(log2(t / (fu.lod.x * 6.0)), 0.0, 6.0);
        let hg = fs_h(p, mip);
        let excess = (hg - (world.y + FS_TUCK * 0.5 + t * rise)) / t;
        vis = min(vis, clamp(1.0 - excess / 0.018, 0.0, 1.0));
    }
    return vis;
}

// The haze: the city's range fog with the sky's aerial path density, the same
// value as the city's up to the camera's far plane; past it (where the city
// has nothing) it keeps thickening toward the fog colour as 1 - e^(-x / L),
// L = the haze tail x the range fog's far end, from the city's value at the far plane.
fn fs_fog(world: vec3<f32>) -> f32 {
    let vd = max(dot(world - look.cam_pos.xyz, look.cam_fwd.xyz), 0.0);
    let dh = world.y - look.cam_pos.y;
    let vde = sky_fog_depth(vd, dh);
    let f = fu.cam.w;
    if (vd <= f) {
        return lk_smooth(look.fog.x, look.fog.y, vde);
    }
    let seam = sky_fog_depth(f, dh * f / vd);
    let f0 = lk_smooth(look.fog.x, look.fog.y, seam);
    let tail = max(fu.frame.w * look.fog.y, 1.0);
    return 1.0 - (1.0 - f0) * exp(-max(vde - seam, 0.0) / tail);
}

// Distant water this layer draws itself (lakes, and the bay past the sea's reach):
// the sky in the mirror at the water's Fresnel over a dark body, the sun's glint.
fn fs_water(world: vec3<f32>, v: vec3<f32>, irr: vec3<f32>) -> vec3<f32> {
    let n = vec3<f32>(0.0, 1.0, 0.0);
    let nv = max(dot(n, v), 0.02);
    // as the sea draws its far water: a wind-roughened surface reflects sky
    // from a few degrees higher than the mirror direction, and its grazing
    // Fresnel stays well under a mirror's (the bright horizon line otherwise).
    let fres = min(0.02 + 0.98 * pow(1.0 - nv, 5.0), 0.55);
    let r = reflect(-v, n);
    let sky = lk_sky_dome(normalize(vec3<f32>(r.x, max(r.y, 0.004) + 0.05, r.z)));
    let body = vec3<f32>(0.010, 0.020, 0.024) * irr / FS_PI;
    let l = look.key_dir.xyz;
    let hv = normalize(l + v);
    let nh = max(dot(n, hv), 0.0);
    // a broad glint (wind-roughened, slope variance ~0.02)
    let a2 = 0.02;
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let spec = look.key.xyz * max(l.y, 0.0) * a2 / (FS_PI * dd * dd) * fres * 0.25 * sky_cloud_transmittance(world);
    return sky * fres + body * (1.0 - fres) + spec;
}

@fragment
fn fs_fs(i: FsOut) -> @location(0) vec4<f32> {
    let h = i.hr.x;
    let sea = fu.frame.z;
    // Inside the sea's reach nothing of this layer stands under the sea surface (the sea
    // draws there, and a bottom just under it would read as a shore to the sea's wash).
    if (i.hr.y > 0.5 && min(h, i.world.y) < sea) {
        discard;
    }
    let world = i.world;
    let to_cam = fu.cam.xyz - world;
    let dist = length(to_cam);
    let v = to_cam / max(dist, 1e-3);
    let p = world.xz;
    // The height field's slope at the pixel's footprint.
    let fp = dist * fu.misc.y;
    let mip = clamp(log2(max(fp, 1.0) / fu.lod.x), 0.0, 6.0); // the full 40 m relief wherever a pixel resolves it
    let e = fu.lod.x * exp2(floor(mip));
    let hx = fs_h(p + vec2<f32>(e, 0.0), mip) - fs_h(p - vec2<f32>(e, 0.0), mip);
    let hz = fs_h(p + vec2<f32>(0.0, e), mip) - fs_h(p - vec2<f32>(0.0, e), mip);
    let n = normalize(vec3<f32>(-hx / (2.0 * e), 1.0, -hz / (2.0 * e)));
    let slope = length(vec2<f32>(hx, hz)) / (2.0 * e);
    let c = textureSampleLevel(fs_class, fs_samp, fs_uv(p), max(mip - 1.0, 0.0));
    let urban = c.r;
    let lake = c.g;
    let wood = c.b;
    let marsh = c.a;
    let sh = textureSampleLevel(fs_shape, fs_samp, fs_uv(p), max(mip - 1.0, 0.0)); // r2
    let sky_vis = sh.r;
    let valley = sh.g;

    let l = look.key_dir.xyz;
    let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5);
    let irr = hemi + look.ambient.xyz + fs_sh(n) * look.ambient.w * look.env_class.w;
    var lit: vec3<f32>;
    if (h < sea || lake > 0.5) {
        lit = fs_water(world, v, irr);
    } else {
        // What the land is, from heights alone (eo-farshore's class guesses), the
        // ground's shape (ravines and north faces wooded, ridges and south faces
        // open grass and scrub) and the season.
        let n1 = fs_noise(p / 700.0);
        let n2 = fs_noise(p / 190.0 + vec2<f32>(17.0, 3.0));
        let n3 = fs_noise(p / 60.0 + vec2<f32>(5.0, 41.0));
        let dry = fu.misc.x;
        let flat_n = max(length(n.xz), 1e-4);
        let north = clamp(-n.z / flat_n, -1.0, 1.0) * smoothstep(0.05, 0.3, slope);
        // ravines and north faces carry oak, bay and redwood; ridgelines stay open
        var wood_w = 1.4 * wood + 0.6 * smoothstep(0.52, 0.72, valley) + 0.45 * smoothstep(0.1, 0.7, north) - 0.3 * smoothstep(0.47, 0.32, valley);
        wood_w = clamp(wood_w * (0.75 + 0.5 * n2) + 0.25 * (n3 - 0.5), 0.0, 1.0) * (1.0 - urban);
        let grass = mix(vec3<f32>(0.065, 0.100, 0.034), vec3<f32>(0.215, 0.165, 0.092), dry) * (0.8 + 0.3 * n1 + 0.15 * n3);
        // coyote brush and chaparral on the steeper open faces: olive, darker than the grass
        let scrub_w = smoothstep(0.2, 0.55, slope) * (0.55 + 0.45 * n2) * (1.0 - wood_w);
        let tree = mix(vec3<f32>(0.036, 0.046, 0.026), vec3<f32>(0.022, 0.032, 0.021), clamp(north, 0.0, 1.0)) * (0.8 + 0.35 * n3);
        var alb = mix(grass, vec3<f32>(0.075, 0.074, 0.046), scrub_w * 0.7);
        alb = mix(alb, tree, wood_w);
        alb = mix(alb, vec3<f32>(0.065, 0.075, 0.048), marsh);
        let rock = smoothstep(0.75, 1.3, slope) * (1.0 - wood_w);
        alb = mix(alb, vec3<f32>(0.17, 0.155, 0.13), rock);
        // the flats' generic built-up tone: roofs, streets, street trees and their
        // shade seen from far off (darker; a grey with a little green)
        let town = vec3<f32>(0.092, 0.094, 0.088) * (0.75 + 0.3 * n2 + 0.25 * n3);
        alb = mix(alb, town, urban);
        // the shore itself (the last metres above the sea: riprap, wet mud, piers'
        // shade) is a dark contact line, not a strip of sunlit dry grass
        alb = mix(alb, vec3<f32>(0.045, 0.046, 0.042), smoothstep(fu.frame.z + 4.0, fu.frame.z + 0.5, h) * (1.0 - lake));
        let nl = max(dot(n, l), 0.0);
        var key = vec3<f32>(0.0);
        if (nl > 0.0) {
            key = look.key.xyz * nl * fs_self_shadow(world, l) * sky_cloud_transmittance(world);
        }
        let rim = look.rim.xyz * max(dot(n, look.rim_dir.xyz), 0.0);
        // the sky light a valley or the foot of a slope does not see (the cook's
        // horizon-based sky visibility)
        let ao = pow(mix(0.15, 1.0, sky_vis), 2.0);
        lit = alb * (key + rim + irr * ao) / FS_PI;
        // The built-up flats by night: an even warm glow standing in for their lights (no lighting data).
        let lamps = skyu.night.x;
        if (lamps > 0.0) {
            // (radiance of a white ground under ~1.5 lux, in the lamps' units: a city's lights seen 10 km off)
            lit = lit + urban * lamps * skyu.bloom_w2.z * 1.5 / FS_PI * vec3<f32>(1.0, 0.72, 0.42) * (0.6 + 0.8 * n2);
        }
    }
    if (fu.misc.w > 0.5) {
        lit = vec3<f32>(urban, wood, lake + marsh) * 0.2; // EO_FARSHORE_DEBUG=1: the class guesses
    }
    let fog = fs_fog(world);
    lit = mix(lit, look.fog_color.xyz, fog);
    return vec4<f32>(lit, 1.0);
}
