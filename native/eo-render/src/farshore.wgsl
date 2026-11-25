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
    // the cinematic look (stage L5; 0 otherwise): x water everywhere outside the drawn sea,
    // y the skirt to the horizon, z the outer ring under this layer's rim, w the skirt's length (game m)
    ring: vec4<f32>,
    // the outer ring's texture: origin x, z (surveyed m), 1 / (texel x width), 1 / (texel x height)
    otex: vec4<f32>,
    // this layer's extent on the plan (game m): min x, min z, max x, max z
    ext: vec4<f32>,
    // x the cook's rim taper (surveyed m), y its sea floor (game y), z log2(outer texel / this texel),
    // w the sea sheet inside the city grid (1/0)
    rim: vec4<f32>,
    // x 1: the grass's dryness is the look's per-frame season (look.tone2.w, the date in
    // force), 0: misc.x (read at load); y, z, w unused
    season: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fu: FarU;
@group(0) @binding(1) var fs_height: texture_2d<f32>;
@group(0) @binding(2) var fs_class: texture_2d<f32>;
@group(0) @binding(3) var fs_samp: sampler;
@group(0) @binding(4) var fs_shape: texture_2d<f32>; // sky visibility, valley
@group(0) @binding(5) var fs_outer: texture_2d<f32>; // the outer ring's heights (stage L5, the cinematic look)

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
    var y = textureSampleLevel(fs_height, fs_samp, fs_uv(p), mip).r;
    if (SF_CINEMATIC && fu.ring.z > 0.5) {
        // Under the cinematic look the outer ring carries on past this cook, so the
        // cook's rim (its last rim.x m fall to the sea floor) hands over to the ring's
        // heights instead: baked = floor + (h - floor) t, so baked + (1 - t)(ring - floor)
        // is h t + ring (1 - t), the ring's height exactly at the edge.
        let q = p / fu.frame.x;
        let e0 = fu.tex.xy;
        let e1 = fu.tex.xy + 1.0 / fu.tex.zw;
        let d = min(min(q.x - e0.x, e1.x - q.x), min(q.y - e0.y, e1.y - q.y));
        if (d < fu.rim.x) {
            let t = smoothstep(0.0, fu.rim.x, d);
            let o = textureSampleLevel(fs_outer, fs_samp, (q - fu.otex.xy) * fu.otex.zw, max(mip - fu.rim.z, 0.0)).r;
            y = y + (1.0 - t) * (o - fu.rim.y);
        }
    }
    return fs_lift(y);
}

// The skirt (stage L5): the outermost layer's edge vertices moved ring.w further
// out, so the edge row of quads carries its edge heights (clamped samples) on to
// the horizon: open sea past a sea edge, land past a land edge.
fn fs_skirt(p: vec2<f32>) -> vec2<f32> {
    let e = fu.ext;
    let k = fu.ring.w;
    let tol = 0.5 * fu.lod.x;
    var q = p;
    if (p.x <= e.x + tol) {
        q.x = e.x - k;
    } else if (p.x >= e.z - tol) {
        q.x = e.z + k;
    }
    if (p.y <= e.y + tol) {
        q.y = e.y - k;
    } else if (p.y >= e.w - tol) {
        q.y = e.w + k;
    }
    return q;
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
    var own_water = !in_sea && ((pm.x >= fu.reach.x && pm.x <= fu.reach.z && pm.y < fu.grid.w + 60000.0) || in_reach);
    var wp = pm;
    if (SF_CINEMATIC && fu.ring.x > 0.5) {
        // stage L5: every point the sea does not draw this frame is this layer's water
        // (farshore.rs `sea_draws_at`): no void past the reach, nor where the sea pass is skipped.
        own_water = !in_sea;
        if (fu.ring.y > 0.5) {
            wp = fs_skirt(pm);
        }
    }
    var y = h;
    if (own_water) {
        y = max(h, fu.frame.z);
    }
    let inside = all(pm > fu.grid.xy) && all(pm < fu.grid.zw);
    if (inside && !(own_water && h <= fu.frame.z)) {
        y = y - FS_TUCK;
    }
    let margin = 2.0 * fu.lod.x;
    var deep = all(pm > fu.grid.xy + vec2<f32>(margin)) && all(pm < fu.grid.zw - vec2<f32>(margin));
    if (SF_CINEMATIC && fu.rim.w > 0.5 && inside && d < fu.cam.w * 0.99) {
        // stage L5, the sea pass stood down for distance: inside the city grid (within the
        // far plane, where the city draws its own ground) this layer is a sheet at sea level
        // in true depth, over the city's seabed and under its land (the land's vertices a
        // hand under the sea level), so the bay reads as water from the air, the waterline
        // where the city's own shore crosses the sea level.
        y = select(fu.frame.z - 0.1, fu.frame.z, h <= fu.frame.z);
        deep = false;
    }
    var o: FsOut;
    let world = vec3<f32>(wp.x, y, wp.y);
    o.clip = fu.view_proj * vec4<f32>(world, 1.0);
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
    var n = vec3<f32>(0.0, 1.0, 0.0);
    if (SF_CINEMATIC && fu.ring.x > 0.5) {
        // stage L5: the wind sea's slopes in the far water (two octaves of value noise,
        // ~70 m and ~20 m, faded out by the pixel's footprint), so the sky in the mirror
        // and the glint break up as the sea pass's do instead of one flat sheet.
        let fp = length(world - fu.cam.xyz) * fu.misc.y;
        var g = vec2<f32>(0.0);
        for (var k = 0; k < 2; k = k + 1) {
            let sc = select(70.0, 20.0, k == 1);
            let q = world.xz / sc + vec2<f32>(f32(k) * 13.0, 7.0);
            let e = 0.35;
            let gx = fs_noise(q + vec2<f32>(e, 0.0)) - fs_noise(q - vec2<f32>(e, 0.0));
            let gz = fs_noise(q + vec2<f32>(0.0, e)) - fs_noise(q - vec2<f32>(0.0, e));
            g = g + vec2<f32>(gx, gz) * (1.0 - smoothstep(0.25 * sc, 1.0 * sc, fp));
        }
        n = normalize(vec3<f32>(-g.x * 0.12, 1.0, -g.y * 0.12));
    }
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

// sf:HILLS+
// ---- the hills' vegetation cover (`--hill-cover`; this block is in the shader only with it) ----
// The Coast Ranges' hills are a mosaic, not a smooth tone: golden annual grass
// on the ridges and the south faces, dark oak and bay woodland in the folds and on
// the north faces with sharp edges against the grass, coyote brush and chaparral
// in clumps and up the gullies, rock on the ridges and the steepest faces. The
// mosaic's edges come from the drivers the layer had (the cook's wood class, the
// valleys, the north faces) broken by fractal noise from 300 m down to the pixel
// (each octave faded where a pixel spans it, so it never shimmers), and the
// gullies run down the fall line.
//
// A wood seen from afar is not a dark paint either: it is crowns over grass, and
// what the eye gets depends on where the sun and the eye stand (Li and Strahler's
// geometric-optical model, 1992): a Boolean scatter of spheroid crowns (oaks:
// radius 4.5 m, centres 7 m up, 4 m half height; scrub: 1.2 m, 1 m, 0.9 m) whose
// cover sets the gap probability exp(-L sec t') on each path; the eye sees sunlit
// crown, shaded crown, sunlit grass and shaded grass in the shares the model gives
// (the grass both lit and seen through the gaps of both paths, with their overlap
// at the hotspot: the sun at the eye's back hides every shadow). So a wood and a
// brushy face go dark and textured under a low sun and away from the hotspot, and
// the open grass between them stays bright.
struct FsCover {
    sun: vec3<f32>, // the albedo the direct light sees
    sky: vec3<f32>, // the albedo the sky light sees
};

// fractal value noise (0..1) over octaves `s0` m and finer (x0.37 each), each faded to its mean where a pixel (`fp` m) spans it.
fn fs_fbm(p: vec2<f32>, s0: f32, octaves: i32, fp: f32) -> f32 {
    var sum = 0.0;
    var norm = 0.0;
    var a = 1.0;
    var sc = s0;
    for (var i = 0; i < octaves; i = i + 1) {
        let k = 1.0 - smoothstep(0.6 * sc, 1.8 * sc, fp);
        let q = p / sc + vec2<f32>(f32(i) * 7.31, f32(i) * 3.17);
        sum = sum + a * mix(0.5, fs_noise(q), k);
        norm = norm + a;
        a = a * 0.6;
        sc = sc * 0.37;
    }
    return sum / norm;
}

// sec of the equivalent sphere's angle for a spheroid of half height b over radius r (Li-Strahler t').
fn fs_sec_eq(tan_t: f32, br: f32) -> vec2<f32> {
    let t = br * tan_t;
    return vec2<f32>(sqrt(1.0 + t * t), t); // sec t', tan t'
}

// The geometric-optical shares for one crown layer of leaf-area-like density `lam` (= -ln gap at nadir):
// x background sunlit and seen, y background seen, z crown seen, w crown seen sunlit.
fn fs_go(lam: f32, tan_i: f32, tan_v: f32, cos_phi: f32, sin_phi: f32, br: f32, hb: f32) -> vec4<f32> {
    let i = fs_sec_eq(tan_i, br);
    let v = fs_sec_eq(tan_v, br);
    let d2 = max(i.y * i.y + v.y * v.y - 2.0 * i.y * v.y * cos_phi, 0.0);
    let x = i.y * v.y * sin_phi;
    let cos_t = clamp(hb * sqrt(d2 + x * x) / (i.x + v.x), -1.0, 1.0);
    let t = acos(cos_t);
    let overlap = (t - sin(t) * cos_t) * (i.x + v.x) / FS_PI;
    let pv = exp(-lam * v.x);
    let kg = exp(-lam * (i.x + v.x - overlap));
    // the sunlit share of the crowns the eye sees: a sphere's lit part toward the eye
    // ((1 + cos of the phase) / 2), less the mutual shade of a closed canopy's lower crowns
    let ci = 1.0 / i.x;
    let cv = 1.0 / v.x;
    let cos_xi = ci * cv + i.y * ci * v.y * cv * cos_phi;
    let lit = 0.5 * (1.0 + cos_xi) * mix(1.0, 0.55, 1.0 - exp(-lam * i.x));
    return vec4<f32>(kg, pv, 1.0 - pv, (1.0 - pv) * lit);
}

fn fs_cover(p: vec2<f32>, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, fp: f32, wood_in: f32, slope: f32, valley: f32, north: f32, dry: f32, urban: f32, marsh: f32, shore: f32) -> FsCover {
    // the mosaic
    let open = (1.0 - urban) * (1.0 - marsh);
    let edge = clamp(fp / 90.0, 0.06, 0.35);
    let d = fs_fbm(p, 300.0, 5, fp) - 0.5;
    let wood = smoothstep(0.5 - edge, 0.5 + edge, wood_in + 0.55 * d) * open;
    // gullies down the fall line (streaks 90 m long, 14 m across) and brush clumps
    let fall = n.xz / max(length(n.xz), 1e-4);
    let q = vec2<f32>(dot(p, fall), dot(p, vec2<f32>(-fall.y, fall.x)));
    let gully = fs_fbm(q / vec2<f32>(6.5, 1.0), 14.0, 3, fp) * smoothstep(0.12, 0.35, slope);
    let clumps = fs_fbm(p + vec2<f32>(91.0, 17.0), 60.0, 4, fp);
    let south = clamp(-north, 0.0, 1.0);
    var brush = 0.35 * smoothstep(0.25, 0.6, slope) + 0.5 * smoothstep(0.55, 0.68, gully + 0.25 * smoothstep(0.5, 0.7, valley)) + 0.15 * south * smoothstep(0.3, 0.7, slope);
    brush = smoothstep(0.5 - edge, 0.5 + edge, brush * 0.7 + clumps * 0.55 - 0.12) * (1.0 - wood) * open;
    // rock on the ridges and the steep faces, and the trails' bare soil
    let rk = fs_fbm(p + vec2<f32>(13.0, 57.0), 80.0, 4, fp);
    let rock = smoothstep(0.62, 0.72, rk + 0.35 * smoothstep(0.3, 0.9, slope) + 0.2 * smoothstep(0.42, 0.28, valley)) * (1.0 - wood) * open;
    // the grass: drier on the south faces and the ridges, greener in the swales
    let dry_g = clamp(dry + 0.22 * south - 0.18 * smoothstep(0.5, 0.7, valley), 0.0, 1.0);
    let tone = 0.82 + 0.36 * fs_fbm(p + vec2<f32>(41.0, 5.0), 120.0, 4, fp);
    var grass = mix(vec3<f32>(0.065, 0.100, 0.034), vec3<f32>(0.230, 0.172, 0.092), dry_g) * tone;
    grass = mix(grass, vec3<f32>(0.165, 0.152, 0.128), rock * 0.85); // weathered sandstone and greywacke
    grass = mix(grass, vec3<f32>(0.065, 0.075, 0.048), marsh);
    let town = vec3<f32>(0.092, 0.094, 0.088) * (0.75 + 0.55 * fs_fbm(p + vec2<f32>(3.0, 77.0), 200.0, 4, fp));
    let bg = mix(grass, town, urban);
    // the crowns' colours: oak and bay dark olive (darker on the north faces), brush grey-olive
    let oak = mix(vec3<f32>(0.040, 0.050, 0.028), vec3<f32>(0.026, 0.036, 0.023), clamp(north, 0.0, 1.0)) * (0.85 + 0.3 * clumps);
    let scrub = vec3<f32>(0.062, 0.064, 0.042) * (0.9 + 0.2 * clumps);
    // cover fractions within each patch (a closed wood ~0.85 with gaps where it thins; brush ~0.7)
    let thin = fs_fbm(p + vec2<f32>(7.0, 29.0), 25.0, 3, fp);
    let f_oak = clamp(wood * (0.62 + 0.4 * thin), 0.0, 0.95);
    let f_brush = clamp(brush * (0.5 + 0.35 * thin), 0.0, 0.9);
    let lam_t = -log(1.0 - f_oak);
    let lam_s = -log(1.0 - f_brush);
    // the sun and the eye in the ground's frame
    let ci = clamp(dot(n, l), 0.02, 1.0);
    let cv = clamp(dot(n, v), 0.02, 1.0);
    let tan_i = sqrt(1.0 - ci * ci) / ci;
    let tan_v = sqrt(1.0 - cv * cv) / cv;
    let lt = l - n * dot(n, l);
    let vt = v - n * dot(n, v);
    let cos_phi = clamp(dot(lt, vt) / max(length(lt) * length(vt), 1e-4), -1.0, 1.0);
    let sin_phi = sqrt(max(1.0 - cos_phi * cos_phi, 0.0));
    let go_t = fs_go(lam_t, tan_i, tan_v, cos_phi, sin_phi, 4.0 / 4.5, 7.0 / 4.0);
    let go_s = fs_go(lam_s, tan_i, tan_v, cos_phi, sin_phi, 0.9 / 1.2, 1.0 / 0.9);
    // the layers together: the grass seen (and lit) through both; the crowns seen in proportion
    let kg = go_t.x * go_s.x;
    let pv = go_t.y * go_s.y;
    let seen_c = 1.0 - pv;
    let w_t = go_t.z / max(go_t.z + go_s.z, 1e-4);
    let crown = mix(scrub, oak, w_t);
    let lit_c = mix(go_s.w / max(go_s.z, 1e-4), go_t.w / max(go_t.z, 1e-4), w_t);
    // the sky over the grass among the crowns: what the crowns overhead leave open
    let gap_up = exp(-(lam_t + lam_s));
    var c: FsCover;
    c.sun = seen_c * lit_c * crown + kg * bg;
    c.sky = seen_c * crown + pv * bg * (0.5 + 0.5 * gap_up);
    // the shore itself (the last metres above the sea) as the layer drew it
    let sh = vec3<f32>(0.045, 0.046, 0.042);
    c.sun = mix(c.sun, sh, shore);
    c.sky = mix(c.sky, sh, shore);
    return c;
}

// sf:end
// sf:HILL_RELIEF+
// ---- the hills' relief under the height field's lattice (`--hill-relief`; in the shader only with it) ----
// The far land's heights are a 40 m lattice (eo-farshore's cook of the 1 arc-second
// DEM, about 30 m), and its normal is taken over two texels, so a Coast Range
// hillside shades as a smooth plane: the gullies that run down every face (a
// drainage density of 5 to 10 km per square km, so a channel every 100 to 200 m,
// rills between them every few tens of metres) and the spurs between them, which
// are what a low sun and the haze pick out on a real hill, are gone. They are put
// back as a height field below the lattice's reach: V-shaped channels down the
// fall line with rounded spurs between (value noise folded about its median,
// stretched 5:1 down the slope), three scales 160, 56 and 20 m across, 10, 4
// and 1.5 m deep (side slopes of about 0.25 to 0.3, 14 to 17 degrees: a
// ravine's walls run 20 to 35), deeper on the steeper faces and none on the
// flats, each faded to nothing where a pixel spans it.
// Its slope (central differences a pixel's footprint apart) bends the lattice's
// normal, which the light, the mosaic's north faces and the crowns' model take.
fn fs_relief_h(p: vec2<f32>, fall: vec2<f32>, fp: f32) -> f32 {
    let q = vec2<f32>(dot(p, fall), dot(p, vec2<f32>(-fall.y, fall.x)));
    var h = 0.0;
    var w = 160.0;
    var a = 10.0;
    for (var k = 0; k < 3; k = k + 1) {
        let fade = 1.0 - smoothstep(0.3 * w, 0.9 * w, fp);
        let nz = fs_noise(vec2<f32>(q.x / (5.0 * w), q.y / w) + vec2<f32>(f32(k) * 5.3 + 11.0, f32(k) * 1.7 + 3.0));
        h = h + fade * a * (abs(2.0 * nz - 1.0) - 0.5);
        w = w * 0.35;
        a = a * 0.4;
    }
    return h;
}

// The lattice's normal `n0` (slope `slope`) bent by the relief at p (`fp` m a pixel).
fn fs_relief(p: vec2<f32>, n0: vec3<f32>, fp: f32, slope: f32) -> vec3<f32> {
    let amp = smoothstep(0.04, 0.2, slope) * (0.6 + 0.8 * smoothstep(0.15, 0.6, slope));
    if (amp <= 0.0 || fp > 150.0) {
        return n0;
    }
    let fall = n0.xz / max(length(n0.xz), 1e-4);
    let d = max(fp, 3.0);
    let gx = (fs_relief_h(p + vec2<f32>(d, 0.0), fall, fp) - fs_relief_h(p - vec2<f32>(d, 0.0), fall, fp)) / (2.0 * d);
    let gz = (fs_relief_h(p + vec2<f32>(0.0, d), fall, fp) - fs_relief_h(p - vec2<f32>(0.0, d), fall, fp)) / (2.0 * d);
    let ny = max(n0.y, 0.05);
    return normalize(vec3<f32>(n0.x / ny - amp * gx, 1.0, n0.z / ny - amp * gz));
}

// sf:end
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
// sf:HILL_RELIEF-
    let n = normalize(vec3<f32>(-hx / (2.0 * e), 1.0, -hz / (2.0 * e)));
    let slope = length(vec2<f32>(hx, hz)) / (2.0 * e);
// sf:end
// sf:HILL_RELIEF+
    let slope = length(vec2<f32>(hx, hz)) / (2.0 * e);
    let n = fs_relief(p, normalize(vec3<f32>(-hx / (2.0 * e), 1.0, -hz / (2.0 * e))), fp, slope); // (`--hill-relief`)
// sf:end
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
        let dry = mix(fu.misc.x, look.tone2.w, fu.season.x);
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
// sf:HILLS-
        lit = alb * (key + rim + irr * ao) / FS_PI;
// sf:end
// sf:HILLS+
        // (`--hill-cover`) the mosaic and its crowns, lit as the geometric-optical model sees them
        let shore = smoothstep(fu.frame.z + 4.0, fu.frame.z + 0.5, h) * (1.0 - lake);
        let wood_in = 1.4 * wood + 0.6 * smoothstep(0.52, 0.72, valley) + 0.45 * smoothstep(0.1, 0.7, north) - 0.3 * smoothstep(0.47, 0.32, valley);
        let cv = fs_cover(p, n, v, l, fp, wood_in, slope, valley, north, dry, urban, marsh, shore);
        lit = (cv.sun * key + cv.sky * (rim + irr * ao)) / FS_PI;
// sf:end
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
