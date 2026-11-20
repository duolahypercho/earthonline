// One shader for all static geometry, drawn by two pipelines (opaque, and
// alpha-blended transparent glass). The object index arrives as the
// instance index (the cull pass writes it as first_instance); the object
// gives the material and the stress-copy offset. Materials live in a storage
// buffer; the authored kit's maps in three texture arrays (albedo, normal,
// ORM), sampled triplanar in world space. Lighting is look_light.wgsl
// (appended to this source with look_common.wgsl by look.rs).

struct Frame {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient_sky: vec4<f32>,
    // xyz where the front probe cube was captured, w 1 when one is in use
    probe_info: vec4<f32>,
    // x = metres between stress copies (0 when not multiplied); y =
    // the facade switches (facade::RUN_*), z = 1 inside the probe face pass
    copy_step: vec4<f32>,
};

struct Object {
    min: vec3<f32>,
    index_count: u32,
    max: vec3<f32>,
    first_index: u32,
    base_vertex: i32,
    kind: u32,
    copy: u32,
    material: u32,
    lod_group: u32,
    lod_role: u32,
    far_first_index: u32,
    far_index_count: u32,
};

struct Material {
    base: vec4<f32>,
    emissive: vec4<f32>,
    // roughness, metalness, normal strength, 1 / metres per repeat
    params: vec4<f32>,
    layer: i32,
    flags: u32,
    rough_flatten: f32,
    thin_absorption: f32,
};

const F_VERTEX_COLOUR: u32 = 1u;
const F_TEXTURED: u32 = 2u;
const F_BLEND: u32 = 4u;
const F_GLASS: u32 = 8u;
const F_THIN_GLASS: u32 = 16u;
// the street surface material (street classes in the vertex alpha).
const F_STREET: u32 = 32u;
const FLOOR_SCAN_BIT: u32 = 512u;
const GRAVEL_BIT: u32 = 1024u;
const DOOR_SCAN_BIT: u32 = 2048u;
const FRONT_SASH_BIT: u32 = 4096u;
const LIMEWASH_BIT: u32 = 8192u;
const PI: f32 = 3.14159265;

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> objects: array<Object>;
@group(0) @binding(2) var<storage, read> materials: array<Material>;
@group(0) @binding(3) var kit_albedo: texture_2d_array<f32>;
@group(0) @binding(4) var kit_normal: texture_2d_array<f32>;
@group(0) @binding(5) var kit_orm: texture_2d_array<f32>;
@group(0) @binding(6) var kit_sampler: sampler;
// the street's scanned sets (layer 0 asphalt, 1 concrete).
@group(0) @binding(7) var street_albedo: texture_2d_array<f32>;
@group(0) @binding(8) var street_nra: texture_2d_array<f32>;
// The JS street scans' sampler: repeat, trilinear, anisotropy 8.
@group(0) @binding(9) var street_sampler: sampler;

/// Offset of stress copy k: a 4-wide grid of small shifts (see scene::copy_offset).
fn copy_offset(k: u32) -> vec3<f32> {
    return vec3<f32>(f32(k % 4u), 0.0, f32(k / 4u)) * frame.copy_step.x;
}

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) @interpolate(flat) material: u32,
    @location(4) @interpolate(flat) object: u32,
    // x the vertex alpha byte (street class), y the normal's w
    // (street: distance to the kerb line / 6.35 m, negative when none).
    @location(5) aux: vec2<f32>,
    // the position relative to the eye (small numbers: its screen
    // derivatives are smooth where the world position's are not).
    @location(6) rel: vec3<f32>,
    // (bays along the wall, storeys up it) on an envelope wall, else 0.
    @location(7) ft: vec2<f32>,
};

fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

@vertex
fn vs(
    @location(0) pos_in: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) color: vec4<f32>,
    @builtin(instance_index) inst: u32,
) -> VOut {
    let o = objects[inst];
    var out: VOut;
    let pos = pos_in + copy_offset(o.copy);
    out.clip = frame.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    // Vertex colours are authored as sRGB bytes; light in linear.
    out.color = pow(color.rgb, vec3<f32>(2.2));
    out.normal = normal.xyz;
    out.material = o.material;
    out.aux = vec2<f32>(color.a * 255.0, normal.w);
    out.rel = pos - frame.cam_pos.xyz; // cc0
    out.ft = vec2<f32>(0.0);
    let kind = o.kind & 255u;
    if (kind == 6u || kind == 7u || kind == 9u || kind == 10u) { // 9, 10
        // Street tints are the JS palette's sRGB hex: decode exactly.
        out.color = srgb_decode(color.rgb);
    }
    if (kind == 4u && normal.w > 0.49) {
        // The house builder stores window and fixture transfer in R and G.
        out.color = color.rgb;
        if ((o.kind & DOOR_SCAN_BIT) != 0u) {
            out.ft = vec2<f32>(color.a, color.b * 3.1);
        } else if ((o.kind & FLOOR_SCAN_BIT) != 0u) {
            let strip = floor(color.b * 8.0);
            let grain = select(1.0, -1.0, (u32(strip) & 1u) != 0u);
            out.ft = vec2<f32>(grain * color.a * 6.5 + fract(strip * 0.618034), color.b);
        } else if (color.b > 0.5) {
            let angle = color.a * 6.2831853;
            let axis = vec2<f32>(cos(angle), sin(angle));
            out.ft = vec2<f32>(dot(pos.xz, axis), dot(pos.xz, vec2<f32>(-axis.y, axis.x)));
        }
    }
    if (kind == 5u) {
        // Kit building: the vertex alpha byte is the kit palette index (the
        // object's material is the palette's first entry); rgb is the paint.
        out.material = o.material + u32(round(color.a * 255.0));
        // the raw bytes (a kit glass vertex packs its window frame in
        // them, rooms.rs); the paint is decoded per fragment in `shade` (it is
        // one colour per material run, so decoding after interpolation is the same).
        out.color = color.rgb;
        // an envelope wall's normal carries the storey count in y
        // and the bay count in w (kit.rs `facade_test_mark`); its own y is 0.
        if ((u32(frame.copy_step.y) & 256u) != 0u && o.lod_role == 4u && abs(normal.y) < 0.95) {
            out.ft = vec2<f32>(normal.w, normal.y) * 127.0;
            out.normal = vec3<f32>(normal.x, 0.0, normal.z);
        }
    }
    out.object = inst;
    return out;
}

struct Surface {
    albedo: vec3<f32>,
    rough: f32,
    metal: f32,
    ao: f32,
    n: vec3<f32>,
    alpha: f32,
    // envMapIntensity class override (look_light's ENV_*), <= 0 none.
    env: i32,
};

// ---- surfaces: the JS street ground materials -----------------------------
// `installCarriagewayShading` / `installConcreteShading` / `surfaceSetNodes`
// (src/world/streets/street-surface-v2-render.js): palette tint x base fold x
// the scanned set's normalised variance (albedo, roughness, occlusion,
// relief), the asphalt's coarse aggregate tap, the carriageway's macro tone,
// cross fall and gutter silt. Constants (ST_*) are generated from
// eo-street/data/street-material.json (look.rs `street_constants_wgsl`).
const SC_ASPHALT: u32 = 0u;
const SC_GUTTER: u32 = 1u;
const SC_PAD: u32 = 2u;
const SC_KERB_FACE: u32 = 3u;
const SC_KERB_TOP: u32 = 4u;
const SC_FOOTWAY: u32 = 5u;
const SC_RAMP: u32 = 6u;
const SC_PAINT: u32 = 7u;
const SC_VERGE: u32 = 8u;

fn st_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

// Value noise, 0..1.
fn st_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = st_hash(i);
    let b = st_hash(i + vec2<f32>(1.0, 0.0));
    let c = st_hash(i + vec2<f32>(0.0, 1.0));
    let d = st_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn st_fade(near: f32, far: f32, dist: f32) -> f32 {
    return 1.0 - smoothstep(near, far, dist);
}

struct StTap {
    albedo: vec3<f32>,
    rough: f32,
    ao: f32,
    tn: vec2<f32>,
};

fn st_tap(uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>, layer: i32) -> StTap {
    var t: StTap;
    let d = textureSampleGrad(street_albedo, street_sampler, uv, layer, gx, gy);
    let m = textureSampleGrad(street_nra, street_sampler, uv, layer, gx, gy);
    t.albedo = d.rgb;
    t.rough = m.b;
    t.ao = clamp((m.a * 255.0 - 128.0) / 127.0, 0.0, 1.0);
    t.tn = m.rg * 2.0 - 1.0;
    // The JS READY GATE: until the scans are bound (1 x 1 transparent
    // placeholder) every term is identity and the surface is its flat tint.
    if (d.a < 0.5) {
        t.albedo = select(ST_ASPHALT_MEAN, ST_CONCRETE_MEAN, layer == 1);
        t.rough = select(ST_ASPHALT_MEAN_ROUGH, ST_CONCRETE_MEAN_ROUGH, layer == 1);
        t.ao = 1.0;
        t.tn = vec2<f32>(0.0);
    }
    return t;
}

fn street_surface(in: VOut, n_geom: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) -> Surface {
    var s: Surface;
    let cls = u32(round(in.aux.x));
    let dist = length(frame.cam_pos.xyz - in.world);
    let concrete = cls >= SC_KERB_FACE;
    let layer = select(0, 1, concrete);
    // Planar projection on the face's dominant axis (the JS uv is world metres
    // in the street frame; the scans are isotropic, so the street's bearing
    // does not matter).
    var p = in.world.xz;
    var gx = dpx.xz;
    var gy = dpy.xz;
    var t_axis = vec3<f32>(1.0, 0.0, 0.0);
    var b_axis = vec3<f32>(0.0, 0.0, 1.0);
    if (abs(n_geom.y) < 0.6) {
        if (abs(n_geom.x) > abs(n_geom.z)) {
            p = in.world.zy;
            gx = dpx.zy;
            gy = dpy.zy;
            t_axis = vec3<f32>(0.0, 0.0, 1.0);
        } else {
            p = in.world.xy;
            gx = dpx.xy;
            gy = dpy.xy;
        }
        b_axis = vec3<f32>(0.0, 1.0, 0.0);
    }
    var tile = ST_ASPHALT_TILE;
    var mean = ST_ASPHALT_MEAN;
    var amp = ST_ASPHALT_CONTRAST * mix(ST_ASPHALT_FAR_CONTRAST, 1.0, st_fade(ST_ASPHALT_NEAR, ST_ASPHALT_FAR, dist));
    var aclamp = ST_ASPHALT_ALBEDO_CLAMP;
    var rgain = ST_ASPHALT_ROUGH_GAIN;
    var rclamp = ST_ASPHALT_ROUGH_CLAMP;
    var mean_rough = ST_ASPHALT_MEAN_ROUGH;
    var mean_ao = ST_ASPHALT_MEAN_AO;
    var ao_str = ST_ASPHALT_AO_STRENGTH;
    var n_str = ST_ASPHALT_NORMAL;
    var base_rough = ST_ASPHALT_ROUGHNESS;
    if (concrete) {
        tile = ST_CONCRETE_TILE;
        mean = ST_CONCRETE_MEAN;
        amp = ST_CONCRETE_CONTRAST * mix(ST_CONCRETE_FAR_CONTRAST, 1.0, st_fade(ST_CONCRETE_NEAR, ST_CONCRETE_FAR, dist));
        aclamp = ST_CONCRETE_ALBEDO_CLAMP;
        rgain = ST_CONCRETE_ROUGH_GAIN;
        rclamp = ST_CONCRETE_ROUGH_CLAMP;
        mean_rough = ST_CONCRETE_MEAN_ROUGH;
        mean_ao = ST_CONCRETE_MEAN_AO;
        ao_str = ST_CONCRETE_AO_STRENGTH;
        n_str = ST_CONCRETE_NORMAL;
        base_rough = ST_CONCRETE_ROUGHNESS;
    }
    let k = 1.0 / tile;
    var f: StTap;
    // Past ST_SCAN_CUTOFF_M the scans are averaged to their mean by the mip
    // chain (a 3 m tile is under a pixel there): the taps are skipped and the
    // terms are their identity, as the JS ready gate's identity.
    let near_scan = dist < ST_SCAN_CUTOFF_M;
    // the footway, kerb and ramp from the CC0 concrete in place of the JS
    // scan (its every term written: the JS taps are skipped).
    let cc0_concrete = concrete && near_scan && cc0_on(CC0_RUN_STREET);
    if (cc0_concrete) {
        cc0_street_concrete(&f, p, gx, gy, in.world, cls, mean, mean_rough, mean_ao, in.color, in.aux.y >= 0.0);
    } else if (near_scan) {
        f = st_tap(p * k, gx * k, gy * k, layer);
    } else {
        f.albedo = mean;
        f.rough = mean_rough;
        f.ao = mean_ao;
        f.tn = vec2<f32>(0.0);
    }
    if (!concrete && near_scan) {
        // Fresh patch asphalt: the same set re-sampled rotated and rescaled
        // under a ~30 m world mask, darker binder, tighter finish.
        let pm = smoothstep(ST_PATCH_START, ST_PATCH_FULL, st_noise(in.world.xz * ST_PATCH_FREQ + ST_PATCH_PHASE));
        if (pm > 0.0) {
            let uv = p * k;
            let puv = vec2<f32>(-uv.y, uv.x) * ST_PATCH_SCALE + ST_PATCH_OFFSET;
            let pgx = vec2<f32>(-gx.y, gx.x) * (k * ST_PATCH_SCALE);
            let pgy = vec2<f32>(-gy.y, gy.x) * (k * ST_PATCH_SCALE);
            let q = st_tap(puv, pgx, pgy, 0);
            f.albedo = mix(f.albedo, q.albedo * ST_PATCH_TONE, pm);
            f.tn = mix(f.tn, vec2<f32>(q.tn.y, -q.tn.x), pm);
            f.ao = mix(f.ao, q.ao, pm);
            f.rough = mix(f.rough, q.rough * ST_PATCH_ROUGH, pm);
        }
    }
    // cracked asphalt under a slow field, and the tile's street decals (scan stage).
    if (!concrete && near_scan && cc0_on(CC0_RUN_STREET)) {
        cc0_street_asphalt(&f, p, gx, gy, in.world, select(-1.0, in.aux.y * 6.35, cls == SC_ASPHALT), mean, mean_rough, mean_ao);
    }
    if (near_scan) {
        cc0_street_decals(&f, in.world, cls, dpx, dpy, mean, mean_rough, mean_ao);
    } else {
        cc0_reset_post();
    }
    let cc0_street = cc0_on(CC0_RUN_STREET); // cc0
    if (!concrete && cc0_street) {
        // more of the scan's own fine variation inside the carriageway (the aggregate and its binder).
        amp = min(amp * CC0_ASPHALT_DETAIL_GAIN, 1.0);
    }
    var albedo_term = mix(vec3<f32>(1.0), clamp(f.albedo / mean, vec3<f32>(aclamp.x), vec3<f32>(aclamp.y)), amp);
    var rough_term = clamp(mix(1.0, f.rough / mean_rough, rgain), rclamp.x, rclamp.y);
    let ao_term = mix(1.0, min(1.0, f.ao / mean_ao), ao_str);
    var tn = f.tn * n_str;
    var base = in.color;
    if (!concrete) {
      if (near_scan) {
        // The aggregate tap: the same maps at the coarse tile, a multiplier
        // round one (base fold 1), relief faded harder than the tone.
        let ka = 1.0 / ST_AGG_TILE;
        let g = st_tap(p * ka, gx * ka, gy * ka, 0);
        let gamp = ST_AGG_CONTRAST * mix(ST_AGG_FAR_CONTRAST, 1.0, st_fade(ST_AGG_NEAR, ST_AGG_FAR, dist));
        albedo_term = albedo_term * mix(vec3<f32>(1.0), clamp(g.albedo / mean, vec3<f32>(ST_AGG_ALBEDO_CLAMP.x), vec3<f32>(ST_AGG_ALBEDO_CLAMP.y)), gamp);
        rough_term = rough_term * clamp(mix(1.0, g.rough / mean_rough, ST_AGG_ROUGH_GAIN), ST_AGG_ROUGH_CLAMP.x, ST_AGG_ROUGH_CLAMP.y);
        tn = tn + g.tn * ST_AGG_NORMAL * st_fade(ST_AGG_RELIEF_NEAR, ST_AGG_RELIEF_FAR, dist);
      }
        if (cls == SC_GUTTER) {
            // The pan is delivered raw (no base fold), with its own pour silt.
            base = base * (1.0 - ST_GUTTER_POUR_SILT);
        } else {
            base = base * ST_ASPHALT_BASE;
            if (cls == SC_ASPHALT && cc0_street) {
                // aged asphalt's albedo, neutral grey; the class and age variation kept as luminance.
                base = cc0_asphalt_base(base);
            } else if (cls == SC_PAD && cc0_street) {
                // the junction pad is the same asphalt (no lighter patch at each intersection).
                base = CC0_ASPHALT_TINT * CC0_ASPHALT_ALBEDO;
            }
        }
        if (cls == SC_ASPHALT) {
            // Macro tone: two octaves of world-keyed noise.
            let m = st_noise(in.world.xz * ST_MACRO_COARSE) * 0.6 + st_noise(in.world.xz * ST_MACRO_FINE + 17.3) * 0.4;
            base = base * (1.0 + ST_MACRO_AMPLITUDE * (m - 0.5) * 2.0);
            // Cross fall toward the gutter lip, and the silt line inboard of the kerb.
            let kd = in.aux.y * 6.35;
            if (kd >= 0.0) {
                let lip = kd - ST_GUTTER_WIDTH;
                base = base * (1.0 - ST_CROSS_FALL_DARKEN * clamp(1.0 - lip / ST_CROSS_FALL_REACH, 0.0, 1.0));
                base = base * (1.0 - ST_SILT_DARKEN * (1.0 - smoothstep(0.0, ST_SILT_REACH, lip)));
            }
        }
    } else {
        base = base * ST_CONCRETE_BASE;
    }
    s.albedo = base * albedo_term;
    s.rough = base_rough * rough_term;
    s.metal = 0.0;
    s.ao = ao_term;
    s.alpha = 1.0;
    // Tangent-space relief on the face's projection axes.
    let tz = sqrt(max(0.0, 1.0 - dot(tn, tn)));
    let ta = normalize(t_axis - n_geom * dot(t_axis, n_geom));
    let ba = normalize(b_axis - n_geom * dot(b_axis, n_geom));
    s.n = normalize(n_geom * tz + ta * tn.x + ba * tn.y);
    s.env = select(ENV_ASPHALT, ENV_SIDEWALK, concrete);
    // the decal effects past the composition; the SF scoring joints on the footway.
    cc0_street_post(&s, n_geom);
    if (in.aux.y >= 0.0 && near_scan && cc0_street) {
        // The kerb-line coordinate's world-xz gradient (the walk's or the gutter's across direction), from its screen derivatives.
        let rx = cc0_drx;
        let ry = cc0_dry;
        let det = rx.x * ry.z - rx.z * ry.x;
        var g = vec2<f32>(0.0);
        if (abs(det) > 1e-14) {
            g = vec2<f32>(cc0_daux.x * ry.z - cc0_daux.y * rx.z, cc0_daux.y * rx.x - cc0_daux.x * ry.x) / det * 6.35;
        }
        if (cls == SC_FOOTWAY) {
            cc0_scoring(&s, in.rel, n_geom, in.aux.y * 6.35, g, rx, ry);
        } else if (cls == SC_GUTTER || cls == SC_ASPHALT) {
            cc0_gutter_grime(&s, in.rel, in.aux.y * 6.35, g, rx, ry);
        } else if (cls == SC_KERB_FACE) {
            cc0_kerb_grime(&s, p, gx, gy, in.aux.y * 6.35);
        }
    }
    sky_wet_street(&s, in.world, n_geom, in.aux.y * 6.35, !concrete); // wet ground, puddles, rain rings
    return s;
}

// ---- facade-test: a facade drawn by the shader alone on the envelope box ----
// No geometry is added: the wall stays one flat quad. Per bay and storey the
// shader ray-casts a rectangular recess (jambs, head, sill face, a pane set
// back in it with the kit's room behind the glass), and paints the casing,
// sill, hood, belt course, cornice and plinth with fake relief (tilted
// normals, contact shade, a sun shadow inside the recess).
fn ft_cov(d: f32, w: f32) -> f32 {
    return clamp(0.5 - d / max(w, 1e-4), 0.0, 1.0);
}

fn ft_rect(p: vec2<f32>, c: vec2<f32>, h: vec2<f32>) -> f32 {
    let q = abs(p - c) - h;
    return max(q.x, q.y);
}

fn ft_h(x: f32) -> f32 {
    return fract(sin(x * 12.9898 + 4.1) * 43758.5453);
}

fn ft_facade(s: ptr<function, Surface>, m: ptr<function, Material>, world: vec3<f32>, n: vec3<f32>, uv: vec2<f32>, du: vec2<f32>, object: u32, drx: vec3<f32>, dry: vec3<f32>) {
    let ob = objects[object];
    let d = fa_drivers(fa_ages[ob.lod_group]);
    let local_y = world.y - d.base_y;
    if (local_y < 0.0) {
        return;
    }
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let t = normalize(vec3<f32>(-n.z, 0.0, n.x));
    let top = ob.max.y;
    var sh = 3.2;
    if (uv.y > 0.04) {
        sh = clamp(local_y / uv.y, 2.2, 7.0);
    }
    // Metres a pixel covers along the wall and up it.
    let fw = vec2<f32>(abs(dot(drx, t)) + abs(dot(dry, t)), abs(drx.y) + abs(dry.y));
    let fwm = max(fw.x, fw.y);
    let along = dot(world, t);
    let wall = (*s).albedo;
    let luma = dot(wall, vec3<f32>(0.2126, 0.7152, 0.0722));
    let h1 = ft_h(d.seed);
    let h2 = ft_h(d.seed + 17.0);
    var trim = mix(vec3<f32>(0.70, 0.68, 0.62), vec3<f32>(0.58, 0.56, 0.52), h1);
    if (luma > 0.4) {
        trim = mix(vec3<f32>(0.84, 0.83, 0.78), wall * 0.42, step(0.62, h2));
    }
    var col = wall;
    var rough = (*s).rough;
    var metal = (*s).metal;
    var nrm = (*s).n;
    var occ = 1.0;
    var sun_cut = 0.0;

    // The key light in the wall's frame (along, up, out of the wall).
    let l = look.key_dir.xyz;
    let sl = vec3<f32>(dot(l, t), l.y, dot(l, n));
    let vdir = normalize(world - frame.cam_pos.xyz);
    let dir = vec3<f32>(dot(vdir, t), vdir.y, -dot(vdir, n));
    let dz = max(dir.z, 0.06);

    // Plinth.
    col = col * mix(0.7, 1.0, smoothstep(0.32, 0.4, local_y));

    let storeys = max(1.0, floor((top - d.base_y) / sh + 0.5));
    let k = clamp(floor(uv.y), 0.0, storeys - 1.0);
    let y = local_y - k * sh;
    let ground = k < 0.5;

    // Belt course over the ground floor, and its shade below.
    if (storeys > 1.5) {
        if (k > 0.5 && k < 1.5) {
            let c = ft_cov(y - 0.16, fw.y);
            col = mix(col, trim, c);
            rough = mix(rough, 0.55, c);
            nrm = normalize(mix(nrm, n + up * (0.9 * smoothstep(0.11, 0.16, y)), c));
        } else if (ground) {
            occ = occ * mix(0.55, 1.0, smoothstep(0.0, 0.16, sh - y));
        }
    }

    let has_bays = (abs(du.x) + abs(du.y)) * 14.0 > fw.x && uv.x > 1e-3;
    if (has_bays) {
        let bay = floor(uv.x);
        let x = (uv.x - bay - 0.5) * 2.8;
        let bh = ft_h(bay * 3.7 + d.seed * 1.3);
        var hw = 0.5 + 0.1 * floor(h2 * 3.0);
        var y0 = 0.85;
        var y1 = min(sh - 0.55, 2.65);
        var depth = 0.17;
        var cw = 0.11;
        var kind = 0;
        if (ground) {
            if (bh < 0.3) {
                kind = 2;
                hw = 0.5;
                y0 = 0.03;
                y1 = 2.15;
                depth = 0.28;
            } else {
                kind = 1;
                hw = 0.98;
                y0 = 0.55;
                y1 = min(sh - 0.6, 2.7);
                depth = 0.13;
                cw = 0.07;
            }
        }
        let p = vec2<f32>(x, y);
        let c = vec2<f32>(0.0, (y0 + y1) * 0.5);
        let half = vec2<f32>(hw, (y1 - y0) * 0.5);
        let sd = ft_rect(p, c, half);
        if (sd < 0.0) {
            // Inside the opening: the ray runs into the recess.
            let hit = p + dir.xy / dz * depth;
            var tau = depth;
            var at = hit;
            if (ft_rect(hit, c, half) < 0.0) {
                let fr = select(0.055, 0.09, kind == 2);
                let sdg = ft_rect(hit, c, half - vec2<f32>(fr));
                var pane = ft_cov(sdg, fwm);
                if (kind == 0) {
                    pane = pane * (1.0 - ft_cov(abs(hit.y - c.y) - 0.022, fw.y));
                } else if (kind == 1) {
                    pane = pane * (1.0 - ft_cov(abs(hit.x) - 0.025, fw.x));
                }
                col = trim * 0.96;
                rough = 0.5;
                metal = 0.0;
                nrm = n;
                if (kind == 2) {
                    // A panelled door: its paint, two sunk panels.
                    let hue = ft_h(bay + d.seed * 0.7);
                    var paint = vec3<f32>(0.10, 0.16, 0.13);
                    if (hue > 0.66) {
                        paint = vec3<f32>(0.24, 0.06, 0.05);
                    } else if (hue > 0.33) {
                        paint = vec3<f32>(0.07, 0.10, 0.18);
                    }
                    let pa = ft_rect(hit, vec2<f32>(0.0, y0 + 0.55), vec2<f32>(hw - 0.2, 0.36));
                    let pb = ft_rect(hit, vec2<f32>(0.0, y0 + 1.5), vec2<f32>(hw - 0.2, 0.42));
                    let sunk = max(ft_cov(pa, fwm), ft_cov(pb, fwm));
                    let rim = smoothstep(0.03, 0.0, abs(min(pa, pb)));
                    col = mix(trim * 0.96, paint * mix(1.0, 0.8, sunk) * mix(1.0, 0.6, rim), pane);
                    rough = mix(0.5, 0.35, pane);
                } else if (pane > 0.0) {
                    let size = half * 2.0;
                    let seed = ft_h(bay * 7.3 + k * 13.1 + d.seed);
                    let shop = select(0.0, 1.0, kind == 1);
                    let r = rm_room(hit - (c - half), size, dir, vec4<f32>(seed, shop, 0.0, length(world - frame.cam_pos.xyz)), vec2<f32>(ft_h(d.seed + 3.0), d.base_y + k * sh + y0), look.rooms_env, look.rooms_work, true, true);
                    var glass = vec3<f32>(0.5, 0.56, 0.58);
                    var gm = 0.9;
                    glass = mix(glass, r.day.xyz, r.day.w);
                    gm = mix(gm, gm * 0.3, r.day.w);
                    col = mix(col, glass, pane);
                    rough = mix(rough, 0.06, pane);
                    metal = mix(metal, gm, pane);
                    rm_emit = r.night * pane;
                    if (pane > 0.5) {
                        (*m).flags = ((*m).flags | F_GLASS) & ~FA_KIT_WEATHER;
                    }
                }
            } else {
                // The reveal: the jamb, the head or the sill's top.
                var tx = 1e9;
                var ty = 1e9;
                if (abs(dir.x) > 1e-4) {
                    tx = (sign(dir.x) * hw - x) * dz / dir.x;
                }
                if (abs(dir.y) > 1e-4) {
                    ty = (select(y0, y1, dir.y > 0.0) - y) * dz / dir.y;
                }
                tau = clamp(min(tx, ty), 0.0, depth);
                at = p + dir.xy / dz * tau;
                if (tx < ty) {
                    nrm = -sign(dir.x) * t;
                } else {
                    nrm = -sign(dir.y) * up;
                }
                col = select(trim * 0.92, wall, kind == 1);
                rough = 0.6;
                metal = 0.0;
                occ = occ * mix(0.95, 0.55, tau / depth);
            }
            // The key light inside the recess: lit only where the ray to it clears the opening.
            if (sl.z > 0.02) {
                let q = at + sl.xy / sl.z * tau;
                sun_cut = 1.0 - ft_cov(ft_rect(q, c, half), 0.025);
            }
        } else {
            // The wall round the opening: casing, sill, hood.
            let cas = ft_cov(sd - cw, fwm);
            if (cas > 0.0) {
                let q = abs(p - c) - half;
                var g = vec2<f32>(sign(p.x - c.x), 0.0);
                if (q.y > q.x) {
                    g = vec2<f32>(0.0, sign(p.y - c.y));
                }
                let bevel = smoothstep(cw - 0.03, cw, sd);
                col = mix(col, trim, cas);
                rough = mix(rough, 0.52, cas);
                metal = mix(metal, 0.0, cas);
                nrm = normalize(mix(nrm, n + (t * g.x + up * g.y) * (0.95 * bevel), cas));
                occ = mix(occ, 1.0, cas);
            } else {
                occ = occ * mix(0.72, 1.0, smoothstep(cw, cw + 0.07, sd));
            }
            if (kind != 2) {
                let sw = hw + cw + 0.06;
                let sill = ft_cov(ft_rect(p, vec2<f32>(0.0, y0 - 0.05), vec2<f32>(sw, 0.05)), fwm);
                col = mix(col, trim * 1.04, sill);
                rough = mix(rough, 0.5, sill);
                nrm = normalize(mix(nrm, n + up * (1.1 * smoothstep(y0 - 0.035, y0, y)), sill));
                // Under the sill: its shade, and the run-off at its ends.
                let below = y0 - 0.1 - y;
                if (below > 0.0 && abs(x) < sw) {
                    occ = occ * mix(0.5, 1.0, smoothstep(0.0, 0.17, below));
                }
                if (below > 0.0) {
                    let e = abs(abs(x) - (hw + cw));
                    let streak = exp(-e / 0.045) * exp(-below / 0.9) * (0.5 + 0.5 * fa_noise(vec2<f32>(along * 9.0, world.y * 1.3), 5.0));
                    col = col * (1.0 - 0.3 * streak);
                }
            }
            if (kind == 0) {
                let hood = ft_cov(ft_rect(p, vec2<f32>(0.0, y1 + cw + 0.045), vec2<f32>(hw + cw + 0.07, 0.045)), fwm);
                col = mix(col, trim, hood);
                nrm = normalize(mix(nrm, n + up * (0.9 * smoothstep(y1 + cw + 0.06, y1 + cw + 0.09, y)), hood));
                let under = y1 + cw - y;
                if (under > 0.0 && under < 0.06 && abs(x) < hw + cw) {
                    occ = occ * mix(0.6, 1.0, under / 0.06);
                }
            }
        }
    }

    // Cornice: fascia, cove, bed mould, dentils, and its shade on the wall.
    let dt = top - world.y;
    let ch = 0.62;
    if (dt < ch + 0.5) {
        let c = ft_cov(dt - ch, fw.y);
        var cc = trim;
        var cn = n;
        var co = 1.0;
        if (dt > 0.14 && dt < 0.34) {
            let q = (dt - 0.14) / 0.2;
            cn = normalize(n * (0.25 + q) - up * (1.0 - q));
            co = mix(0.5, 1.0, q);
        } else if (dt >= 0.34 && dt < 0.46) {
            cc = trim * 0.93;
            cn = normalize(n + up * (0.8 * smoothstep(0.37, 0.34, dt)));
        } else if (dt >= 0.46) {
            let gcell = abs(fract(along / 0.26) - 0.5) * 0.26;
            let block = ft_cov(gcell - 0.07, fw.x);
            cc = mix(trim * 0.3, trim, block);
            co = mix(0.8, 1.0, block);
        }
        col = mix(col, cc, c);
        rough = mix(rough, 0.55, c);
        metal = mix(metal, 0.0, c);
        nrm = normalize(mix(nrm, cn, c));
        occ = mix(occ * mix(0.5, 1.0, smoothstep(ch, ch + 0.45, dt)), co, c);
        sun_cut = mix(sun_cut, 0.0, c);
    }

    // The recess's sun shadow as a darkening where the wall faces the key light.
    let facing = smoothstep(0.0, 0.2, sl.z);
    col = col * mix(1.0, 0.4, sun_cut * facing);
    (*s).albedo = col * mix(1.0, occ, 0.55);
    (*s).ao = (*s).ao * occ;
    (*s).rough = clamp(rough, 0.03, 1.0);
    (*s).metal = metal;
    (*s).n = nrm;
}

// Four octahedral tiles in the final albedo layer. Each row samples the
// furnished room from one appliance. Left is window transfer and open-sky
// coverage, right is light from the actual dining pendant.
fn kitchen_reflection(r: vec3<f32>, angle: f32, row: f32, rough: f32) -> vec3<f32> {
    let across = vec2<f32>(cos(angle), sin(angle));
    let local = vec3<f32>(dot(r.xz, across), r.y, dot(r.xz, vec2<f32>(-across.y, across.x)));
    var uv = local.xz / dot(abs(local), vec3<f32>(1.0));
    if (local.y < 0.0) {
        uv = (1.0 - abs(uv.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), uv >= vec2<f32>(0.0));
    }
    let lod = clamp(log2(1.0 + 64.0 * rough * rough), 0.0, 6.0);
    // Keep the bilinear/trilinear footprint inside its own tile at both mip levels.
    let border = 0.5 * exp2(ceil(lod)) / (0.5 * f32(textureDimensions(kit_albedo, 0).x));
    uv = clamp(uv * 0.5 + 0.5, vec2<f32>(border), vec2<f32>(1.0 - border));
    let st = 0.5 * (uv + vec2<f32>(0.0, row));
    let layer = i32(textureNumLayers(kit_albedo)) - 1;
    let day = textureSampleLevel(kit_albedo, kit_sampler, st, layer, lod);
    let lamp = textureSampleLevel(kit_albedo, kit_sampler, st + vec2<f32>(0.5, 0.0), layer, lod);
    let sky = look.hemi_sky.rgb + look.key.rgb * max(look.key_dir.y, 0.0);
    let albedo = textureSampleLevel(kit_normal, kit_sampler, st, layer, lod).rgb;
    let encoded = textureSampleLevel(kit_normal, kit_sampler, st + vec2<f32>(0.5, 0.0), layer, lod).rgb * 2.0 - 1.0;
    let back = vec2<f32>(-across.y, across.x);
    let plane = across * encoded.x + back * encoded.z;
    let hit_n = vec3<f32>(plane.x, encoded.y, plane.y) / max(length(encoded), 0.001);
    let hemi = mix(look.hemi_ground.rgb, look.hemi_sky.rgb, 0.5 + 0.5 * hit_n.y);
    let ambient = hemi + look.ambient.rgb + lk_sh(hit_n) * look.ambient.w * look.env_class.x;
    return albedo * ambient / PI + day.rgb * sky + lamp.rgb * 4.0 + day.a * lk_sky_sample(r);
}

fn shade(in: VOut) -> vec4<f32> {
    var m = materials[in.material];
    let ft_du = vec2<f32>(dpdx(in.ft.x), dpdy(in.ft.x));
    // Derivatives first, in uniform control flow.
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let scan_dx = dpdx(in.ft);
    let scan_dy = dpdy(in.ft);
    let face = normalize(cross(dpx, dpy));
    cc0_daux = vec2<f32>(dpdx(in.aux.y), dpdy(in.aux.y)); // the footway's across coordinate (scoring joints)
    cc0_drx = dpdx(in.rel); // cc0
    cc0_dry = dpdy(in.rel); // cc0
    cc0_wall_hit = false; // cc0
    cc0_reset_post(); // cc0
    let v = normalize(frame.cam_pos.xyz - in.world);
    var n = face;
    if (dot(in.normal, in.normal) > 0.25) {
        n = normalize(in.normal);
    }
    // Two-sided: shade the side the camera sees.
    if (dot(n, v) < 0.0) {
        n = -n;
    }

    if ((m.flags & F_STREET) != 0u) {
        let ss = street_surface(in, n, dpx, dpy);
        return look_light(ss, m, v, in.world, n, in.object);
    }
    var s: Surface;
    s.albedo = m.base.rgb;
    s.rough = m.params.x;
    s.metal = m.params.y;
    s.ao = 1.0;
    s.n = n;
    s.alpha = m.base.a;
    s.env = -1;
    // a kit vertex carries raw bytes (see vs): the paint decodes here.
    let kit_object = (objects[in.object].kind & 255u) == 5u;
    let house_finish = (objects[in.object].kind & 255u) == 4u && in.aux.y > 0.49;
    let interior_transfer = house_finish && in.aux.y > 0.99;
    var vcol = select(in.color, vec3<f32>(1.0), house_finish);
    if (kit_object) {
        vcol = pow(in.color, vec3<f32>(2.2));
    }
    rm_emit = vec3<f32>(0.0);
    if ((m.flags & F_VERTEX_COLOUR) != 0u) {
        s.albedo = vcol;
        if ((objects[in.object].kind & 255u) == 7u) {
            // street paint ('painted-metal', roughness 0.58) and the
            // verge bank (the concrete material's 'sidewalk' class).
            let cls = u32(round(in.aux.x));
            if (cls == SC_PAINT) {
                s.rough = ST_PAINT_ROUGHNESS;
                s.env = ENV_PAINTED_METAL;
                // worn paint inside the geometric marking (an ambientCG road-line scan as the wear mask only).
                cc0_paint_wear(&s, in.world, in.rel, in.aux.y * 6.35, cc0_drx, cc0_dry);
            } else {
                s.rough = ST_CONCRETE_ROUGHNESS;
                s.env = ENV_SIDEWALK;
            }
        }
    }
    // the terrain's ground scans around its tone (not the beach sand: below), before the wet margin.
    let cc0_terrain = (m.flags & F_VERTEX_COLOUR) != 0u && (objects[in.object].kind & 255u) == 1u;
    let cc0_sand = cc0_terrain && (m.flags & LM_F_WET_GROUND) != 0u && in.aux.x < 253.5;
    if (cc0_terrain && !cc0_sand) {
        cc0_ground(&s, in.world, n, dpx, dpy);
    }
    if (((m.flags >> LM_SURFACE_SHIFT) & 7u) != 0u) {
        lm_surface(&s, m, in.world, n, dpx, dpy); // landmark-surface.js, the seawall's concrete
    } else if ((m.flags & LM_F_WET_GROUND) != 0u && (objects[in.object].kind & 255u) == 1u) {
        if (in.aux.x < 253.5) {
            // beach sand (its vertex alpha carries the shore distance): beach-surface.js's sand.
            lm_beach(&s, in.world, in.aux.x / 1.5 - 64.0);
        } else {
            lm_wet_ground(&s, in.world); // the wet margin at the waterline (beach-surface.js)
        }
    }
    if (cc0_sand) {
        cc0_beach(&s, in.world, n, dpx, dpy, in.aux.x / 1.5 - 64.0); // the dry and damp sand scans under lm_beach's bands
    }
    // an open space's paving, sand or concrete: the scan's detail around its vertex colour.
    if ((m.flags & F_VERTEX_COLOUR) != 0u && (objects[in.object].kind & 255u) != 1u) {
        let vc_hit = cc0_surface(&s, m, in.material, in.world, n, dpx, dpy, s.albedo, in.object, true);
    }
    // a finish drawn from the scanned facade atlas (`atlasId`): the JS
    // blends two translations of the scan under a world-anchored noise, on the
    // directional face frame (kit/materials.js build(), the atlas branch).
    let atlas = (m.flags & FA_ATLAS) != 0u && (m.flags & F_TEXTURED) != 0u && (fa_run() & FA_RUN_ATLAS2) != 0u;
    // a kit finish with a CC0 set draws the scan in place of its bake (the per-building paint, tint and ageing as before).
    let cc0_done = (m.flags & F_TEXTURED) != 0u && (m.flags & F_GLASS) == 0u && cc0_surface(&s, m, in.material, in.world, n, dpx, dpy, vcol, in.object, false);
    if (cc0_done) {
    } else if (atlas) {
        let fr = fa_frame(n);
        let k = m.params.w;
        let uvm = vec2<f32>(dot(in.world, fr.t), dot(in.world, fr.b));
        let gxm = vec2<f32>(dot(dpx, fr.t), dot(dpx, fr.b));
        let gym = vec2<f32>(dot(dpy, fr.t), dot(dpy, fr.b));
        // The kit layer is stored top row first; the JS atlas is uploaded with flipY.
        let fl = vec2<f32>(1.0, -1.0);
        let uva = uvm * k * fl;
        let uvb = (uvm + vec2<f32>(0.73, 1.19)) * k * fl;
        let ga = gxm * k * fl;
        let gb = gym * k * fl;
        let blend = fa_ss(0.2, 0.8, fa_noise(uvm / 3.7, 19.0));
        let alb = mix(textureSampleGrad(kit_albedo, kit_sampler, uva, m.layer, ga, gb), textureSampleGrad(kit_albedo, kit_sampler, uvb, m.layer, ga, gb), blend);
        let nrm = mix(textureSampleGrad(kit_normal, kit_sampler, uva, m.layer, ga, gb), textureSampleGrad(kit_normal, kit_sampler, uvb, m.layer, ga, gb), blend);
        let orm = mix(textureSampleGrad(kit_orm, kit_sampler, uva, m.layer, ga, gb), textureSampleGrad(kit_orm, kit_sampler, uvb, m.layer, ga, gb), blend);
        s.albedo = alb.rgb * m.base.rgb * vcol; // vcol
        s.rough = orm.g;
        s.metal = 0.0;
        s.ao = 1.0;
        let xy = nrm.xy * 2.0 - 1.0;
        let z = sqrt(max(0.0, 1.0 - dot(xy, xy)));
        let str = m.params.z;
        s.n = normalize(fr.t * (xy.x * str) + fr.b * (xy.y * str) + fr.n * z);
    } else if ((m.flags & F_TEXTURED) != 0u) {
        // Triplanar in world space, `params.w` repeats per metre.
        let k = m.params.w;
        let p = in.world * k;
        let gx = dpx * k;
        let gy = dpy * k;
        var w = pow(abs(n), vec3<f32>(4.0));
        // surfaces (a saving in the city pass): a plane carrying under 2 % of
        // the blend is dropped and the rest renormalised, so an axis-aligned
        // wall or roof reads one plane's three maps instead of nine taps.
        w = select(vec3<f32>(0.0), w, w > vec3<f32>(0.02 * max(w.x, max(w.y, w.z))));
        w = w / max(w.x + w.y + w.z, 1e-5);
        let layer = m.layer;
        let uvx = p.zy;
        let uvy = p.xz;
        let uvz = p.xy;
        var orm = vec4<f32>(0.0);
        if (w.x > 0.0) {
            orm = orm + textureSampleGrad(kit_orm, kit_sampler, uvx, layer, gx.zy, gy.zy) * w.x;
        }
        if (w.y > 0.0) {
            orm = orm + textureSampleGrad(kit_orm, kit_sampler, uvy, layer, gx.xz, gy.xz) * w.y;
        }
        if (w.z > 0.0) {
            orm = orm + textureSampleGrad(kit_orm, kit_sampler, uvz, layer, gx.xy, gy.xy) * w.z;
        }
        s.rough = mix(orm.g, m.params.x, m.rough_flatten);
        if ((m.flags & F_GLASS) == 0u) {
            var alb = vec3<f32>(0.0);
            var tx = vec3<f32>(0.0, 0.0, 1.0);
            var ty = vec3<f32>(0.0, 0.0, 1.0);
            var tz = vec3<f32>(0.0, 0.0, 1.0);
            if (w.x > 0.0) {
                alb = alb + textureSampleGrad(kit_albedo, kit_sampler, uvx, layer, gx.zy, gy.zy).rgb * w.x;
                tx = textureSampleGrad(kit_normal, kit_sampler, uvx, layer, gx.zy, gy.zy).xyz * 2.0 - 1.0;
            }
            if (w.y > 0.0) {
                alb = alb + textureSampleGrad(kit_albedo, kit_sampler, uvy, layer, gx.xz, gy.xz).rgb * w.y;
                ty = textureSampleGrad(kit_normal, kit_sampler, uvy, layer, gx.xz, gy.xz).xyz * 2.0 - 1.0;
            }
            if (w.z > 0.0) {
                alb = alb + textureSampleGrad(kit_albedo, kit_sampler, uvz, layer, gx.xy, gy.xy).rgb * w.z;
                tz = textureSampleGrad(kit_normal, kit_sampler, uvz, layer, gx.xy, gy.xy).xyz * 2.0 - 1.0;
            }
            // The vertex colour is the paint multiply (white on models).
            s.albedo = alb * m.base.rgb * vcol; // vcol
            s.metal = orm.b;
            s.ao = orm.r * 0.85 + 0.15;
            // Whiteout blend of the three tangent-space normals.
            let str = m.params.z;
            tx = vec3<f32>(tx.xy * str + n.zy, abs(tx.z) * n.x);
            ty = vec3<f32>(ty.xy * str + n.xz, abs(ty.z) * n.y);
            tz = vec3<f32>(tz.xy * str + n.xy, abs(tz.z) * n.z);
            s.n = normalize(tx.zyx * w.x + ty.xzy * w.y + tz.xyz * w.z);
        }
    }
    s.rough = clamp(s.rough, 0.03, 1.0);
    // the room behind kit glazing (rooms.wgsl; kit/materials.js: by day
    // the pane's colour mixes to the room's albedo by its share and gives up
    // 70 % of its metalness there, by night the lit room is its emission).
    let room = (m.flags >> 8u) & 7u;
    if ((m.flags & F_GLASS) != 0u && room != 0u && kit_object && frame.copy_step.z < 0.5 && (fa_run() & 32u) != 0u) {
        let ng_out = select(face, normalize(in.normal), dot(in.normal, in.normal) > 0.25);
        let r = rm_kit_pane(vec4<f32>(in.color, in.aux.y), in.world, ng_out, frame.cam_pos.xyz, room, look.rooms_env, look.rooms_work);
        s.albedo = mix(s.albedo, r.day.xyz, r.day.w);
        s.metal = mix(s.metal, s.metal * 0.3, r.day.w);
        // A blended pane's room reads at full strength after the blend (1 / opacity, floored at 0.25).
        let transmission = select(1.0, 1.0 / max(0.25, m.base.a), (m.flags & F_BLEND) != 0u);
        rm_emit = r.night * transmission;
    }
    if ((objects[in.object].kind & GRAVEL_BIT) != 0u) {
        // Granitic fines and larger chips share one world-space surface across
        // the authored bed and the site mesh. Fade subpixel grain at distance.
        let uv = in.world.xz;
        let footprint = max(length(dpx),length(dpy));
        let chips = st_noise(uv*28.0);
        let grain = st_noise(uv*129.0);
        let chip_detail = 1.0-smoothstep(0.012,0.066,footprint);
        let grain_detail = 1.0-smoothstep(0.002,0.014,footprint);
        s.albedo *= 0.93 + 0.23*(st_noise(uv*1.8)-0.5)
                   + 0.47*(chips-0.5)*chip_detail + 0.53*(grain-0.5)*grain_detail;
        let gradient = vec2<f32>(st_noise(uv*28.0+vec2<f32>(0.26,0.0))-chips,
                                 st_noise(uv*28.0+vec2<f32>(0.0,0.26))-chips);
        s.n = normalize(n-vec3<f32>(gradient.x,0.0,gradient.y)*chip_detail*0.58);
        s.rough = 0.95;
    }
    if ((objects[in.object].kind & LIMEWASH_BIT) != 0u) {
        let exposure = clamp((in.color.b * 255.0 - 3.0) / 55.0, 0.0, 1.0);
        s.albedo *= mix(vec3<f32>(1.0), vec3<f32>(0.79, 0.75, 0.70), exposure);
    }
    let door_scan = (objects[in.object].kind & DOOR_SCAN_BIT) != 0u;
    let floor_scan = (objects[in.object].kind & (DOOR_SCAN_BIT | FLOOR_SCAN_BIT)) != 0u;
    if (floor_scan && (door_scan || abs(n.y) > 0.7)) {
        let layer = select(i32(textureNumLayers(kit_albedo)) - 2, m.layer, door_scan);
        s.albedo = textureSampleGrad(kit_albedo, kit_sampler, in.ft, layer, scan_dx, scan_dy).rgb * select(vec3<f32>(1.0), m.base.rgb, door_scan);
        s.rough = textureSampleGrad(kit_orm, kit_sampler, in.ft, layer, scan_dx, scan_dy).g;
        let bump = textureSampleGrad(kit_normal, kit_sampler, in.ft, layer, scan_dx, scan_dy).xyz * 2.0 - 1.0;
        let det = scan_dx.x * scan_dy.y - scan_dy.x * scan_dx.y;
        if (abs(det) > 1e-10) {
            let along = normalize((dpx * scan_dy.y - dpy * scan_dx.y) * sign(det));
            let across = normalize((dpy * scan_dx.x - dpx * scan_dy.x) * sign(det));
            s.n = normalize(n * bump.z + select(1.0, m.params.z, door_scan) * (along * bump.x - across * bump.y));
        }
    } else if (house_finish && !floor_scan && in.color.b > 0.5) {
        // Longitudinal oak fibres curve around growth rings. Filter the finer
        // pores out as their spacing approaches a screen pixel.
        let uv = select(vec2<f32>(in.ft.x + in.ft.y, in.world.y), in.ft, abs(n.y) > 0.62);
        let flow = st_noise(uv * vec2<f32>(6.0, 1.7));
        let fibres = st_noise(vec2<f32>(uv.x * 310.0 + flow * 9.0, uv.y * 2.8));
        let rings = sin(uv.x * 198.0 + 10.0 * flow + 2.0 * sin(uv.y * 1.5));
        let detail = 1.0 - smoothstep(0.002, 0.019, max(length(dpx), length(dpy)));
        s.albedo *= 0.95 + 0.29 * (flow - 0.5) + detail * (0.052 * rings + 0.18 * (fibres - 0.5));
        s.rough = clamp(s.rough + 0.12 * (fibres - 0.5) * detail, 0.18, 0.9);
    }
    if (house_finish && !floor_scan && in.color.b > 0.30 && in.color.b < 0.49) {
        let plane = select(select(in.world.zy, in.world.xy, abs(n.z) > abs(n.x)), in.world.xz, abs(n.y) > 0.7);
        s.albedo *= 0.97 + 0.057 * st_noise(plane * 2.8) + 0.025 * st_noise(plane * 62.0);
    }
    if (interior_transfer && (m.flags & F_GLASS) == 0u) {
        // Transfer is sampled at build time against the actual windows, furniture
        // and luminaire. Window light follows the current sky; the pendant stays warm.
        let transfer = in.color.rg * in.color.rg;
        let window = 0.76 * transfer.x * (look.hemi_sky.rgb + look.key.rgb * max(look.key_dir.y, 0.0));
        let pendant = transfer.y * vec3<f32>(1.55, 1.05, 0.56);
        rm_emit += s.albedo * (1.0 - s.metal) * (window + pendant);
    }
    // weathering (Blender buildings) and archetype ageing (kit buildings).
    fa_apply(&s, m, in.world, n, in.object, dpx, dpy);
    if (house_finish && !floor_scan && in.color.b > 0.5 / 255.0 && in.color.b < 2.5 / 255.0) {
        s.env = ENV_KITCHEN;
        let row = round(in.color.b * 255.0) - 1.0;
        room_radiance = kitchen_reflection(reflect(-v, s.n), in.aux.x * (6.2831853 / 255.0), row, s.rough);
    }
    // the shader-only facade on a near envelope wall.
    if ((fa_run() & 256u) != 0u && kit_object && objects[in.object].lod_role == 4u && dot(in.normal, in.normal) > 0.25 && abs(in.normal.y) < 0.3 && frame.copy_step.z < 0.5) {
        ft_facade(&s, &m, in.world, normalize(in.normal), in.ft, ft_du, in.object, cc0_drx, cc0_dry);
    }
    if ((objects[in.object].kind & 255u) == 0u) {
        rm_emit = rm_emit + sky_far_windows(in.world, n, in.object, dpx, dpy); // lit windows on the far boxes
    }

    let street_pane = (objects[in.object].kind & FRONT_SASH_BIT) != 0u && in.color.b > 3.5/255.0;
    if (street_pane && frame.copy_step.z < 0.5) {
        let angle = in.aux.x * (2.0 * PI / 255.0);
        let outside = vec2<f32>(sin(angle),-cos(angle));
        if (dot(v.xz,outside) <= 0.0) {
            return look_light(s,m,v,in.world,n,in.object);
        }
        let pane_layer = i32(textureNumLayers(kit_albedo))-3;
        // The two real glass faces blend separately. Divide by opacity because
        // the blended pass multiplies the returned radiance by alpha once.
        // A low-opacity pane otherwise suppresses its own reflection twice.
        let ray = reflect(-v,n);
        let uv = vec2<f32>(atan2(ray.x,-ray.z) / (2.0*PI) + 0.5,
                           clamp(0.5 - asin(clamp(ray.y,-1.0,1.0)) / PI,0.002,0.998));
        let day = textureSampleLevel(kit_albedo,kit_sampler,uv,pane_layer,0.7).rgb;
        let night = srgb_decode(textureSampleLevel(kit_normal,kit_sampler,uv,pane_layer,0.7).rgb);
        let captured = mix(day,night,skyu.night.z);
        // Low-iron clear glazing transmits the room while keeping a slight
        // neutral veil from the outer surface over bright sky reflections.
        let plate = mix(vec3<f32>(dot(captured, vec3<f32>(0.2126,0.7152,0.0722))), captured, 0.83);
        let reflected = plate / max(vec3<f32>(0.15),vec3<f32>(1.0)-0.79*plate);
        let fresnel = 0.042 + 0.958*pow(1.0-clamp(dot(n,v),0.0,1.0),5.0);
        let alpha = fresnel + (1.0-fresnel)*0.047;
        return vec4<f32>(reflected*fresnel/alpha,alpha);
    }
    // Lighting, shadows and fog: the JS game's light rig (look_light.wgsl).
    return look_light(s, m, v, in.world, n, in.object);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return shade(in);
}

struct IdsOut {
    @location(0) color: vec4<f32>,
    @location(1) ids: vec2<u32>,
};

/// Debug/test variant: also writes (object index, material index).
@fragment
fn fs_ids(in: VOut) -> IdsOut {
    var out: IdsOut;
    out.color = shade(in);
    out.ids = vec2<u32>(in.object, in.material);
    return out;
}
