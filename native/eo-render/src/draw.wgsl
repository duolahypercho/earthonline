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
const F_FRONT_DAYLIGHT: u32 = 64u;
const F_STOOP_LIGHT: u32 = 262144u;
const F_JOINERY_LIGHT: u32 = 128u;
const WALL_DAYLIGHT_BIT: u32 = 0x80000000u;
const FLOOR_SCAN_BIT: u32 = 512u;
const THIN_FIBRE_BIT: u32 = 262144u;
const QUILT_FIBRE_BIT: u32 = 524288u;
const TABLE_BOARD_BIT: u32 = 1048576u;
const ENTRY_BRASS_BIT: u32 = 2097152u;
const CUT_COTTON_BIT: u32 = 4194304u;
const BEDROOM_LIGHT_BIT: u32 = 8388608u;
const DINING_FLOOR_LIGHT_BIT: u32 = 16777216u;
const DOWNSTAIRS_LIGHT_BIT: u32 = 33554432u;
const TABLE_LIGHT_BIT: u32 = 67108864u;
const FLOOR_REFLECTION_BIT: u32 = 268435456u;
const FITTED_ROOM_BIT: u32 = 536870912u;
const FITTED_FLOOR_BIT: u32 = 1073741824u;
const OAK_FINISH_BIT: u32 = 134217728u;
const ENCLOSURE_TRANSFER_BIT: u32 = 131072u;
const WAXED_BOARD_BIT: u32 = 32768u;
const COURTYARD_BOUNCE_BIT: u32 = 65536u;
const GRAVEL_BIT: u32 = 1024u;
const DOOR_SCAN_BIT: u32 = 2048u;
const FRONT_SASH_BIT: u32 = 4096u;
const LIMEWASH_BIT: u32 = 8192u;
const LOCAL_UV_SCAN_BIT: u32 = 16384u;
// An object kind (or material flag) bit of the playable house's measured
// materials, lights and optics (scene.rs, authored.rs): compiled out with the
// house (look.rs `ShaderFeatures::house`, SF_HOUSE).
fn house_bit(word: u32, bit: u32) -> bool {
    return SF_HOUSE && (word & bit) != 0u;
}
const PI: f32 = 3.14159265;
// The city's opaque classes (look.rs `CITY_CLASS`, lib.rs `CityClasses`): one
// pipeline for every object (CITY_ALL: the glass, the probe, the fallback
// lists), or, with the classes split, one a class, each compiling out the
// others' surfaces: the street surface (F_STREET), a landmark's surface kinds
// (`lm_kind`), and the rest. cull.wgsl `city_class` sorts the objects.
const CITY_ALL: u32 = 0u;
const CITY_GENERAL: u32 = 1u;
const CITY_STREET: u32 = 2u;
const CITY_LANDMARK: u32 = 3u;

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
// (stage S1, `--facade-clutter`) the shop signs' lettering: one layer a word
// cell, r the letters' fill, g a neon tube (sign_letters.rs; a 1 x 1 stand-in
// without the clutter or a font).
@group(0) @binding(28) var sgn_atlas: texture_2d_array<f32>;

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
    // HOUSE_VARYINGS (look.rs `draw_wgsl`: the playable house's enclosure and
    // table light, interpolated only with SF_HOUSE; read through `vo_house_z`,
    // `vo_table_fixture`, `vo_table_reverse`, written through `vo_set_house`)
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
    var pos = pos_in + copy_offset(o.copy);
    if (SF_SEABED && (o.kind & 255u) == 13u && color.a > 0.0) {
        // Alpha is flex from root to tip. Both current components wrap with the sky clock.
        let phase = dot(pos.xz, vec2<f32>(0.85, 0.54));
        let clock = skyu.wet.w * (6.2831853 / 6.0);
        pos.x += color.a * (0.045 * sin(clock + phase) + 0.020 * sin(clock * 2.0 - phase));
        pos.z += color.a * 0.032 * cos(clock + phase * 0.7);
    }
    out.clip = frame.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    // Vertex colours are authored as sRGB bytes; light in linear.
    out.color = pow(color.rgb, vec3<f32>(2.2));
    out.normal = normal.xyz;
    out.material = o.material;
    let enclosed = house_bit(o.kind, ENCLOSURE_TRANSFER_BIT) && normal.w < -0.50;
    out.aux = vec2<f32>(color.a * 255.0, select(normal.w, -normal.w, enclosed));
    // (the playable house's varyings, SF_HOUSE: `vo_set_house` below)
    var house_z = select(0.0, 1.0, enclosed);
    var table_fixture = vec4<f32>(0.);
    var table_reverse = vec4<f32>(0.);
    if (house_bit(o.kind, TABLE_LIGHT_BIT) && normal.w < 0. && normal.w > -.5) {
        // Each split mesh normal was metered in Blender. Interpolate physical
        // radiance across the triangle after decoding the four stored channels.
        let cell = vec2<i32>(256,0) + vec2<i32>(round(color.rg*255.));
        let encoded = textureLoad(kit_orm,cell,i32(textureNumLayers(kit_orm))-3,0);
        table_fixture = encoded*encoded/max(1.-encoded*encoded,vec4<f32>(.0001));
        let reverse = textureLoad(kit_orm,cell+vec2<i32>(0,96),i32(textureNumLayers(kit_orm))-3,0);
        table_reverse = reverse*reverse/max(1.-reverse*reverse,vec4<f32>(.0001));
        out.aux.y = 64./127.-normal.w;
        house_z = 2.;
    }
    out.rel = pos - frame.cam_pos.xyz; // cc0
    out.ft = vec2<f32>(0.0);
    let kind = o.kind & 255u;
    if (kind == 6u || kind == 7u || kind == 9u || kind == 10u || kind == 11u) { // 9, 10; 11 a structure's deck and paint
        // Street tints are the JS palette's sRGB hex: decode exactly.
        out.color = srgb_decode(color.rgb);
    }
    if (kind == 12u) {
        // a structure's concrete: its formwork coordinates (metres) in ft,
        // its wear in aux.x (structures.rs `encode_concrete`); the tint is the material's.
        out.ft = sx_decode(color, normal.w);
        out.color = vec3<f32>(1.0);
        out.aux = vec2<f32>(color.a * 255.0, -1.0);
        house_z = 0.0;
    }
    if (SF_HOUSE && kind == 4u && out.aux.y > 0.49) {
        // The house builder stores window and fixture transfer in R and G.
        out.color = color.rgb;
        if (house_bit(o.kind, CUT_COTTON_BIT)) {
            out.ft = vec2<f32>(color.b, color.a);
        } else if ((o.kind & LOCAL_UV_SCAN_BIT) != 0u || house_bit(o.kind, TABLE_BOARD_BIT)) {
            out.ft = clamp(vec2<f32>(color.b, color.a), vec2<f32>(0.001), vec2<f32>(0.999));
        } else if ((o.kind & DOOR_SCAN_BIT) != 0u) {
            out.ft = vec2<f32>(color.a, color.b * 3.1);
        } else if (house_bit(o.kind, FLOOR_REFLECTION_BIT) && abs((look.enclosure_frame*vec4<f32>(pos,1.)).y)<.04) {
            let bytes = round(color.ba * 255.);
            let strip_v = dot(bytes,vec2<f32>(1.,256.))/65535.;
            let row = floor(strip_v*8.);
            let depth = (look.enclosure_frame*vec4<f32>(pos,1.)).z;
            let grain = select(1.,-1.,(u32(row)&1u)!=0u);
            out.ft = vec2<f32>(grain*depth*.5+fract(row*.618034),strip_v);
        } else if (house_bit(o.kind, WAXED_BOARD_BIT)) {
            out.ft = vec2<f32>(color.b * 7.0, color.a * 13.0);
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
    if (house_bit(o.kind, FITTED_ROOM_BIT)) {
        out.ft = unpack2x16float(pack4x8unorm(color));
    }
    if (kind == 5u) {
        // Kit building: the vertex alpha byte is the kit palette index (the
        // object's material is the palette's first entry); rgb is the paint.
        out.material = o.material + u32(round(color.a * 255.0));
        // the raw bytes (a kit glass vertex packs its window frame in
        // them, rooms.rs); the paint is decoded per fragment in `shade` (it is
        // one colour per material run, so decoding after interpolation is the same).
        out.color = color.rgb;
        // (stage S1) a lettered sign's (u, v) in its text box ride in r and g:
        // through ft their screen derivatives come with the others in `shade`.
        out.ft = color.rg;
        // an envelope wall's normal carries the storey count in y
        // and the bay count in w (kit.rs `facade_test_mark`); its own y is 0.
        if ((u32(frame.copy_step.y) & 256u) != 0u && o.lod_role == 4u && abs(normal.y) < 0.95) {
            out.ft = vec2<f32>(normal.w, normal.y) * 127.0;
            out.normal = vec3<f32>(normal.x, 0.0, normal.z);
        }
    }
    out.object = inst;
    vo_set_house(&out, house_z, table_fixture, table_reverse);
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
    // a structure's deck (kind 11) takes none of the street tile's decals below it.
    if (near_scan && (objects[in.object].kind & 255u) != 11u) {
        if (sl_level() != 0u) {
            sl_kd = in.aux.y * 6.35; // street life (`--street-life`): the kerb distance its decals read
        }
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
    if (wx_level() != 0u && (objects[in.object].kind & 255u) != 11u) {
        // weathering (`--grime`, weathering.wgsl): aged walks, chipped kerbs, a patched and sealed carriageway.
        wx_street(&s, in.world, in.rel, cls, in.aux.y * 6.35, dpx, dpy);
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

// facade clutter: a sign's face lit from inside (64..95) or a neon tube (96..127);
// the low five bits are its switch roll against the shops' lit share
// (`look.rooms_env.z`, the share the shop windows light by), by dusk
// (`rooms_env.x`). Its colour is its own paint. Storefront signs measure
// 40-450 cd/m2 against lit windows' ~6-50 (Lighting Research Center 2006; the
// reference sheet 3.7). Stage S1 lowered the face's gain from W3's 1.1 (about
// 4x a lit pane, rooms.wgsl caps one near 0.24, which the tone curve clipped to
// a flat white panel) to about 1.5x a pane, the colour kept under the clip;
// the letters of a back-lit sign glow brighter than its face (channel letters
// over a dim face, LRC's "letters 40-230" against an unlit wall), and a neon
// tube about 9x a pane over a faint glow on its board.
const FC_BOX_GAIN: f32 = 0.5;
const FC_LETTER_GAIN: f32 = 1.05;
const FC_BACK_GAIN: f32 = 0.035;
const FC_NEON_GAIN: f32 = 2.2;
const FC_HALO_GAIN: f32 = 0.3;
// INFERRED: the dimmest open shop window's luminance (cd/m2; N1's bars, 16 / 1.5),
// the level at which a shop light reads as the shop open (sky.rs `shop_k`).
const FC_SHOP_OPEN_L: f32 = 10.0;
// (`--look cinematic`, stage L2) the signs raised into the reference's range:
// a lit face ~40, letters ~80, a neon tube ~165 cd/m2 against lit windows'
// ~6 and the lamp pools' ~1 (sheet 3.3, 3.7: storefront signs 40-450), the
// default's gains (a face ~12, a tube ~55 cd/m2 at a pane's ~6) x 3.
const FC_CINE_SIGN_GAIN: f32 = 3.0;

// How lit a sign is, 0..1 by dusk. With the shop windows' lights (`--shop-light`,
// sky flag SKY_SHOPS) within their reach: where the light of the shop at `p`
// (its front facing `n`) is on (rooms.wgsl `rm_shop_light`), dimmed to the
// security level a closed shop keeps; dark over a dark shop. Past their reach
// (a per-sign hashed hand-over across the rooms' fade, as the panes do) and
// without them: `roll` against the shops' lit share (W3's rule).
fn fc_lit(p: vec3<f32>, n: vec3<f32>, roll: f32) -> f32 {
    let dusk = look.rooms_env.x;
    var lit = step(roll, look.rooms_env.z);
    if (SF_SHOP_LIGHT && (skyu.flags.x & SKY_SHOPS) != 0u && dusk > 0.0) {
        let reach = select(SHOP_REACH_NEAR, SHOP_REACH_FAR, (skyu.flags.x & SKY_SHOPS_FAR) != 0u);
        let fade = rm_ss(SHOP_ROOM_FADE * reach, reach, length(p - skyu.eye.xyz));
        if (fract(roll * 7.31 + 0.37) >= fade) {
            let sl = rm_shop_light(p, n);
            let open = dot(sl.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) / max(FC_SHOP_OPEN_L * skyu.bloom_w2.z * dusk, 1e-6);
            lit = sl.w * clamp(open, 0.0, 1.0);
        }
    }
    return lit * dusk;
}

// A sign's light this fragment: its emission when lit, and what `fc_lit` asks.
struct FcGlow {
    emit: vec3<f32>,
    q: vec3<f32>,
    n: vec3<f32>,
    roll: f32,
};

// W3's glow code (a sign drawn without words): its own paint lit, its shop
// looked up a little under the fragment on the face's own front.
fn fc_glow(albedo: vec3<f32>, code: f32, world: vec3<f32>, n_raw: vec3<f32>) -> FcGlow {
    let k = round(code * 127.0);
    let neon = k >= 95.5;
    var g: FcGlow;
    g.roll = (k - select(64.0, 96.0, neon)) / 31.0;
    g.emit = albedo * select(FC_BOX_GAIN, FC_NEON_GAIN, neon);
    g.q = world - vec3<f32>(0.0, 1.2, 0.0);
    g.n = vec3<f32>(n_raw.x, 0.0, n_raw.z);
    return g;
}

// ---- shop sign lettering (stage S1, `--facade-clutter`; eo-kit clutter.rs) ----
// A lettered quad's vertex: rgb = (u, v of its text box, the atlas cell), the
// normal's y and w = the rest (`LetterCode::bytes`); w is negative, which no
// other non-glass kit vertex is. Its normal's own y is 0 (a vertical face).
// The fields' paint (eo-kit `sign_field_tint`: SIGN_FIELDS at three fades, then
// the neon board) as bytes, and the inks (`SIGN_INKS`, sRGB bytes): an eo-render
// test keeps both equal to eo-kit's.
const SGN_FIELD_COUNT: u32 = 37u;
const SGN_FIELDS = array<vec3<f32>, 37>(
    vec3<f32>(195.0, 36.0, 44.0), vec3<f32>(246.0, 240.0, 227.0), vec3<f32>(27.0, 45.0, 88.0), vec3<f32>(33.0, 34.0, 35.0),
    vec3<f32>(237.0, 189.0, 46.0), vec3<f32>(33.0, 108.0, 70.0), vec3<f32>(217.0, 88.0, 35.0), vec3<f32>(133.0, 45.0, 128.0),
    vec3<f32>(40.0, 124.0, 185.0), vec3<f32>(233.0, 225.0, 197.0), vec3<f32>(166.0, 22.0, 81.0), vec3<f32>(67.0, 67.0, 74.0),
    vec3<f32>(204.0, 79.0, 83.0), vec3<f32>(244.0, 238.0, 226.0), vec3<f32>(73.0, 86.0, 118.0), vec3<f32>(78.0, 78.0, 77.0),
    vec3<f32>(237.0, 198.0, 85.0), vec3<f32>(78.0, 135.0, 103.0), vec3<f32>(222.0, 119.0, 77.0), vec3<f32>(156.0, 86.0, 148.0),
    vec3<f32>(83.0, 147.0, 194.0), vec3<f32>(234.0, 226.0, 203.0), vec3<f32>(182.0, 69.0, 112.0), vec3<f32>(104.0, 103.0, 107.0),
    vec3<f32>(213.0, 118.0, 119.0), vec3<f32>(243.0, 237.0, 225.0), vec3<f32>(115.0, 124.0, 145.0), vec3<f32>(119.0, 117.0, 114.0),
    vec3<f32>(237.0, 207.0, 121.0), vec3<f32>(119.0, 160.0, 133.0), vec3<f32>(226.0, 148.0, 114.0), vec3<f32>(176.0, 124.0, 167.0),
    vec3<f32>(123.0, 169.0, 201.0), vec3<f32>(235.0, 228.0, 207.0), vec3<f32>(196.0, 110.0, 140.0), vec3<f32>(139.0, 136.0, 137.0),
    vec3<f32>(31.0, 34.0, 39.0),
);
const SGN_INKS = array<vec3<f32>, 6>(
    vec3<f32>(240.0, 232.0, 214.0), vec3<f32>(24.0, 24.0, 26.0), vec3<f32>(222.0, 172.0, 52.0),
    vec3<f32>(196.0, 34.0, 38.0), vec3<f32>(236.0, 92.0, 160.0), vec3<f32>(44.0, 92.0, 196.0),
);
// eo-kit `SIGN_BLEACH` (sRGB bytes) and `SIGN_FADES`: an ink fades with its field.
const SGN_BLEACH: vec3<f32> = vec3<f32>(214.0, 206.0, 192.0);
// (u, v) of the text box the r and g bytes span (`LETTER_UV_MIN`, `LETTER_UV_SPAN`).
const SGN_UV_MIN: f32 = -7.5;
const SGN_UV_SPAN: f32 = 16.0;
const SGN_LIT_LETTERS: u32 = 1u;
const SGN_LIT_FACE: u32 = 2u;
const SGN_NEON: u32 = 3u;

struct SgnCode {
    cell: u32,
    mode: u32,
    query: u32,
    ink: u32,
    field: u32,
    emblem: u32,
};

fn sgn_decode(ny: f32, nw: f32, cell: f32) -> SgnCode {
    var c = u32((i32(round(ny * 127.0)) + 127) * 127 + (-i32(round(nw * 127.0)) - 1));
    var o: SgnCode;
    o.field = c % SGN_FIELD_COUNT;
    c = c / SGN_FIELD_COUNT;
    o.ink = c % 6u;
    c = c / 6u;
    o.query = c % 3u;
    c = c / 3u;
    o.mode = c % 4u;
    o.emblem = c / 4u;
    o.cell = u32(round(cell * 255.0));
    return o;
}

// Coverage of a signed distance `d` (text heights) over a pixel of `fw`.
fn sgn_cov(d: f32, fw: f32) -> f32 {
    return clamp(0.5 - d / fw, 0.0, 1.0);
}

fn sgn_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// A lettered sign's face: the word (atlas fill, or its neon tubes and their
// glow), the emblem beside it or the border round it (as eo-kit's planner
// leaves room for them: centred 0.6 text heights past the box's end, 0.42
// across; a border 0.25 and 0.18 out), painted in its ink over the field,
// faded with it. `t` the text box (u, v), `du`, `dv` their screen
// derivatives. Returns its light when lit (the shop looked up under the box's
// centre, on the front `query` names) for `fc_lit`.
fn sgn_sign(s: ptr<function, Surface>, g: SgnCode, t: vec2<f32>, du: vec2<f32>, dv: vec2<f32>, world: vec3<f32>, n_raw: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, group: u32) -> FcGlow {
    // the sign's plane per unit u and v from the screen derivatives (u, v run
    // affinely over the quad), so the box's centre and size are known here.
    let det = du.x * dv.y - du.y * dv.x;
    var pu = vec3<f32>(0.0);
    var pv = vec3<f32>(0.0);
    if (abs(det) > 1e-12) {
        pu = (dpx * dv.y - dpy * dv.x) / det;
        pv = (dpy * du.x - dpx * du.y) / det;
    }
    let aspect = clamp(length(pu) / max(length(pv), 1e-4), 0.25, 40.0);
    // the letters: fill and tube; with no font installed (a 1 x 1 stand-in) a band, W3's design.
    var cov = vec2<f32>(0.0);
    var halo = 0.0;
    let inside = all(t > vec2<f32>(0.0)) && all(t < vec2<f32>(1.0));
    let x = (t.x - 0.5) * aspect;
    let y = t.y - 0.5;
    let fw = max(length(dv), 1e-4);
    if (inside && textureDimensions(sgn_atlas).x > 1u) {
        let at = vec2<f32>(t.x, 1.0 - t.y);
        let gx = vec2<f32>(du.x, -dv.x);
        let gy = vec2<f32>(du.y, -dv.y);
        cov = textureSampleGrad(sgn_atlas, kit_sampler, at, i32(g.cell), gx, gy).rg;
        if (g.mode == SGN_NEON) {
            // the tubes' glow on their board: the tube mask a few levels down.
            halo = textureSampleGrad(sgn_atlas, kit_sampler, at, i32(g.cell), gx * 10.0, gy * 10.0).g;
        }
    } else if (inside) {
        cov = vec2<f32>(sgn_cov(sgn_box(vec2<f32>(x, y), vec2<f32>(0.46 * aspect, 0.09), 0.0), fw));
    }
    // the emblem (1..6: disc, ring, diamond; odd on the left) or the border (7).
    var d = 1e3;
    if (g.emblem == 7u) {
        d = abs(sgn_box(vec2<f32>(x, y), vec2<f32>(0.5 * aspect + 0.25, 0.68), 0.12)) - 0.035;
    } else if (g.emblem > 0u) {
        let e = vec2<f32>(x - select(1.0, -1.0, (g.emblem & 1u) == 1u) * (0.5 * aspect + 0.6), y);
        let kind = (g.emblem - 1u) / 2u;
        if (kind == 0u) {
            d = length(e) - 0.42;
        } else if (kind == 1u) {
            d = abs(length(e) - 0.33) - 0.09;
        } else {
            d = (abs(e.x) + abs(e.y)) * 0.7071 - 0.3;
        }
    }
    let shape_tube = select(abs(d + 0.035) - 0.035, d, g.emblem == 7u);
    let letters = max(cov.x, sgn_cov(d, fw));
    let tube = max(cov.y, sgn_cov(shape_tube, fw));
    halo = max(halo, 0.4 * clamp(1.0 - max(shape_tube, 0.0) / 0.3, 0.0, 1.0));
    // the ink, faded with its field.
    let fade = select(f32(g.field / 12u) * 0.16, 0.0, g.field >= 36u);
    let ink_s = mix(SGN_INKS[min(g.ink, 5u)], SGN_BLEACH, fade) / 255.0;
    let ink = pow(ink_s, vec3<f32>(2.2));
    var o: FcGlow;
    if (g.mode == SGN_NEON) {
        // glass tubes, pale by day; by night their colour at full strength
        // (the tone curve whitens the core) and a glow on the dark board.
        let neon = pow(ink / max(max(ink.x, ink.y), max(ink.z, 1e-4)), vec3<f32>(1.3));
        (*s).albedo = mix((*s).albedo, mix(vec3<f32>(0.5), neon * 0.6, 0.4), tube);
        (*s).rough = mix((*s).rough, 0.15, tube);
        o.emit = neon * (tube * FC_NEON_GAIN + halo * FC_HALO_GAIN);
    } else {
        let field = (*s).albedo;
        (*s).albedo = mix(field, ink * 0.9, letters);
        if (g.mode == SGN_LIT_LETTERS) {
            o.emit = ink * letters * FC_LETTER_GAIN + field * FC_BACK_GAIN;
        } else {
            // lit through: brightest between the lamps, falling off toward the edges.
            o.emit = (*s).albedo * FC_BOX_GAIN * (1.0 - 0.25 * smoothstep(0.3, 0.85, abs(y)));
        }
    }
    if (g.mode == 0u) {
        o.emit = vec3<f32>(0.0);
    }
    // its shop: under the box's centre, on the front the query names (a blade's
    // root, toward the wall and down to the window).
    let nf = normalize(vec3<f32>(n_raw.x, 0.0, n_raw.z) + vec3<f32>(1e-6, 0.0, 0.0));
    let right = vec3<f32>(nf.z, 0.0, -nf.x);
    let centre = world + (0.5 - t.x) * pu + (0.5 - t.y) * pv;
    o.n = select(select(-right, right, g.query == 1u), nf, g.query == 0u);
    o.q = centre - select(o.n * 0.45 + vec3<f32>(0.0, 2.2, 0.0), vec3<f32>(0.0, 1.2, 0.0), g.query == 0u);
    // its switch roll: the building's group and the sign's design, hashed (stable across loads and slots).
    var h = group * 747796405u + g.cell * 2891336453u + (g.field * 7u + g.ink) * 277803737u + (g.mode * 8u + g.emblem) * 2654435761u;
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = (h ^ (h >> 13u)) * 3266489917u;
    h = h ^ (h >> 16u);
    o.roll = f32(h >> 8u) / 16777216.0;
    return o;
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

// The shading normal of a two-sided surface (`GG3_FACET_FIX`; terrain-surface
// item 1). Round 2 flipped the interpolated normal whenever it faced away from
// the eye; on a smooth-shaded steep facet seen at a grazing angle the
// interpolated normal can face away while the triangle itself faces the eye,
// and the flip lit that part of the triangle as its back: the dark wedges on
// the Baker cliff. Here the flip follows the triangle (the test
// @builtin(front_facing) makes, taken from the triangle's own plane, `face`
// from the eye-relative position's screen derivatives: it holds for either
// winding, and footprint rings come in both), and a normal that still faces
// away is bent toward the eye's horizon, to just above it (GG3_GRAZE: the
// least n.v kept, 0.05, INFERRED: about 3 degrees, enough that the light's
// N.V terms stay finite).
const GG3_GRAZE: f32 = 0.05;

fn gg3_facing_normal(n_in: vec3<f32>, v: vec3<f32>, face: vec3<f32>, rel: vec3<f32>) -> vec3<f32> {
    var n = n_in;
    // the triangle's normal on the eye's side (rel = point - eye)
    let f = select(face, -face, dot(face, rel) > 0.0);
    if (dot(n, f) < 0.0) {
        n = -n; // the triangle is seen from behind its vertex normals
    }
    let nv = dot(n, v);
    if (nv < GG3_GRAZE) {
        n = normalize(n + v * (GG3_GRAZE - nv));
    }
    return n;
}

// The quarter-board probes are prefiltered separately for the scanned oak and
// its wax. The atlas has a duplicate texel at the azimuth seam in each tile.
fn oak_ray_uv(ray: vec3<f32>) -> vec2<f32> {
    let depth = normalize((vec4<f32>(0.,0.,1.,0.) * look.enclosure_frame).xyz);
    let across = vec3<f32>(depth.z,0.,-depth.x);
    let azimuth = atan2(dot(ray,across),-dot(ray,depth));
    let elevation = asin(clamp(ray.y,0.,1.));
    return vec2<f32>(1.) + vec2<f32>(62.,30.) *
        vec2<f32>(.5 + azimuth/(2.*PI),elevation/(.5*PI));
}

// The ground-floor survey preserves the actual window, furniture and enclosure
// seen in each view direction, separately for each lighting condition.
fn floor_probe(angular: vec2<f32>, cell: vec2<u32>, source: u32) -> vec3<f32> {
    let index = cell.y*4u + cell.x;
    let tile = vec2<u32>(index % 8u, source*4u + index/8u);
    let uv = (angular + vec2<f32>(tile)*vec2<f32>(64.,16.)) / 512.;
    let layer = i32(textureNumLayers(kit_orm)) - 5;
    let encoded = textureSampleLevel(kit_orm,kit_sampler,uv,layer,0.).rgb;
    let square = encoded*encoded;
    return square / max(1.-square,vec3<f32>(.0001));
}

fn floor_source(angular: mat4x2<f32>, near: vec2<u32>, far: vec2<u32>, f: vec2<f32>, source: u32) -> vec3<f32> {
    let a = mix(floor_probe(angular[0],near,source),floor_probe(angular[1],vec2<u32>(far.x,near.y),source),f.x);
    let b = mix(floor_probe(angular[2],vec2<u32>(near.x,far.y),source),floor_probe(angular[3],far,source),f.x);
    return mix(a,b,f.y);
}

// Reproject the reflected ray onto the physical enclosure. Each probe then
// looks at the same point on the sill, wall or ceiling as the visible board.
fn floor_angular(hit: vec3<f32>, cell: vec2<u32>, parcel_to_world: mat2x2<f32>) -> vec2<f32> {
    let offset = hit.xz - (vec2<f32>(cell)+.5)*vec2<f32>(7./4.,13./8.);
    let horizontal = parcel_to_world*offset;
    let ray = normalize(vec3<f32>(horizontal.x,(hit.y-.015)/look.enclosure_frame[1].y,horizontal.y));
    let uv = oak_ray_uv(ray)-1.;
    return vec2<f32>(1.+uv.x,1.+uv.y*(14./30.));
}

fn floor_reflected_room(plan: vec2<f32>, world: vec3<f32>, ray: vec3<f32>) -> vec3<f32> {
    let grid = clamp(plan*vec2<f32>(4./7.,8./13.)-.5,vec2<f32>(0.),vec2<f32>(3.,7.));
    let near = vec2<u32>(grid);
    let far = min(near+1u,vec2<u32>(3u,7u));
    let f = fract(grid);
    let direction = (look.enclosure_frame*vec4<f32>(ray,0.)).xyz;
    let origin = vec3<f32>(plan.x,0.,plan.y);
    let wall = select(vec3<f32>(0.),vec3<f32>(7.,10./3.,13.),direction>vec3<f32>(0.));
    let distances = (wall-origin)/select(vec3<f32>(1.),direction,abs(direction)>vec3<f32>(.00001));
    let valid = select(vec3<f32>(100.),distances,(distances>vec3<f32>(.03)) & (abs(direction)>vec3<f32>(.00001)));
    let hit = origin + direction*min(valid.x,min(valid.y,valid.z));
    let ex = look.enclosure_frame[0].xz;
    let ez = look.enclosure_frame[2].xz;
    let inverse_basis = mat2x2<f32>(vec2<f32>(ez.y,-ex.y),vec2<f32>(-ez.x,ex.x))*(1./(ex.x*ez.y-ez.x*ex.y));
    let angular = mat4x2<f32>(floor_angular(hit,near,inverse_basis),
        floor_angular(hit,vec2<u32>(far.x,near.y),inverse_basis),
        floor_angular(hit,vec2<u32>(near.x,far.y),inverse_basis),floor_angular(hit,far,inverse_basis));
    var light = floor_source(angular,near,far,f,0u);
    let sun = look.key_dir.xyz;
    let cloud = sky_cloud_transmittance(world);
    let clear = cloud*smoothstep(0.,.35,sun.y)*(1.-skyu.night.z);
    if (clear < 1.) {
        let tint = max(lk_sh(vec3<f32>(0.,1.,0.)),vec3<f32>(.00001));
        light += floor_source(angular,near,far,f,1u)*tint*(look.house_sky.x/lk_luma(tint))*(1.-clear);
    }
    if (clear > 0.) {
        light += floor_source(angular,near,far,f,2u)*look.key.rgb*clear;
    }
    if (sun.y > 0. && cloud > 0.) {
        let back = normalize((vec4<f32>(0.,0.,1.,0.)*look.enclosure_frame).xyz);
        let along = dot(sun,back);
        let row = select(3u,5u,along>0.);
        let elevation = clamp(asin(clamp(sun.y,0.,1.)),radians(25.),radians(57.));
        let t = (elevation-radians(25.))/radians(32.);
        light += mix(floor_source(angular,near,far,f,row),floor_source(angular,near,far,f,row+1u),t)
               * look.key.rgb*(cloud*abs(along)/cos(elevation));
    }
    return light;
}

fn oak_probe(uv: vec2<f32>, tile: vec2<f32>) -> vec3<f32> {
    let layer = i32(textureNumLayers(kit_orm))-2;
    let sample = textureSampleLevel(kit_orm,kit_sampler,(uv+tile*vec2<f32>(64.,32.))/512.,layer,0.).rba;
    let square = sample*sample;
    return square/max(vec3<f32>(.0001),1.-square);
}

fn oak_field(uv: vec2<f32>, spatial: vec2<f32>, row: f32, lobe: f32) -> vec3<f32> {
    let col = 4.*lobe;
    let front = mix(oak_probe(uv,vec2<f32>(col,row)),oak_probe(uv,vec2<f32>(col+1.,row)),spatial.x);
    let rear = mix(oak_probe(uv,vec2<f32>(col+2.,row)),oak_probe(uv,vec2<f32>(col+3.,row)),spatial.x);
    return mix(front,rear,spatial.y);
}

fn oak_environment(uv: vec2<f32>, spatial: vec2<f32>, lobe: f32,
                   sky: vec3<f32>, clear: f32, sun_row: f32, elevation: f32,
                   sun_beam: vec3<f32>) -> vec3<f32> {
    var light = oak_field(uv,spatial,0.,lobe);
    if (clear < 1.) { light += oak_field(uv,spatial,1.,lobe)*sky*(1.-clear); }
    if (clear > 0.) { light += oak_field(uv,spatial,2.,lobe)*look.key.rgb*clear; }
    if (any(sun_beam > vec3<f32>(0.))) {
        light += mix(oak_field(uv,spatial,sun_row,lobe),oak_field(uv,spatial,sun_row+1.,lobe),elevation)*sun_beam;
    }
    return light;
}

fn oak_day_map(plan: vec2<f32>, page: f32) -> vec3<f32> {
    let tile = vec2<f32>(page-2.*floor(page*.5),floor(page*.5))*vec2<f32>(256.,64.);
    let offset = vec2<f32>(0.,224.) + tile + clamp(plan,vec2<f32>(.5),vec2<f32>(255.5,63.5));
    let layer = i32(textureNumLayers(kit_orm))-2;
    let encoded = textureSampleLevel(kit_orm,kit_sampler,offset/512.,layer,0.).rba;
    let sq = encoded*encoded;
    return sq/max(vec3<f32>(.0001),1.-sq);
}

fn oak_reflections(point: vec3<f32>, world: vec3<f32>, v: vec3<f32>, base_n: vec3<f32>, coat_n: vec3<f32>, surface_light: bool) {
    let bounds = look.oak_finish[0];
    let spatial = clamp((point.xz-bounds.xy)/(bounds.zw-bounds.xy),vec2<f32>(0.),vec2<f32>(1.));
    let sky_tint = max(lk_sh(vec3<f32>(0.,1.,0.)),vec3<f32>(.00001));
    let sky = sky_tint*(look.house_sky.x/lk_luma(sky_tint));
    let cloud = sky_cloud_transmittance(world);
    let sun = look.key_dir.xyz;
    let clear = cloud*smoothstep(0.,.35,sun.y)*(1.-skyu.night.z);
    let elevation = clamp(asin(clamp(sun.y,0.,1.)),radians(25.),radians(57.));
    let t = (elevation-radians(25.))/radians(32.);
    let depth = normalize((vec4<f32>(0.,0.,1.,0.) * look.enclosure_frame).xyz);
    let along = dot(sun,depth);
    let side = select(3.,5.,along>0.);
    let beam = look.key.rgb*select(0.,cloud*abs(along)/cos(elevation),sun.y>0.);
    room_radiance = oak_environment(oak_ray_uv(reflect(-v,base_n)),spatial,0.,sky,clear,side,t,beam);
    oak_coat_radiance = oak_environment(oak_ray_uv(reflect(-v,coat_n)),spatial,1.,sky,clear,side,t,beam);
    oak_coated = true;
    if (surface_light) {
        let extent = look.oak_finish[2];
        let plan = (point.xz-extent.xy)/(extent.zw-extent.xy)*vec2<f32>(256.,64.);
        if (clear < 1.) { oak_daylight += oak_day_map(plan,0.)*sky*(1.-clear); }
        if (clear > 0.) { oak_daylight += oak_day_map(plan,1.)*look.key.rgb*clear; }
        if (any(beam>vec3<f32>(0.))) {
            oak_daylight += mix(oak_day_map(plan,side-1.),oak_day_map(plan,side),t)*beam;
        }
    }
}

fn board_ambient(transfer: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(.56,.41,.29) *
        (look.ambient.rgb + transfer.x*look.hemi_sky.rgb + transfer.y*vec3<f32>(1.2,.78,.46));
}

// Project the reflected ray onto the finite set of three sash bays. The
// interpolated board coordinates are in the Blender plan's across/depth axes.
// Their screen derivatives convert the world ray to that same surveyed frame.
fn board_reflection(plan: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>,
                    wx: vec3<f32>, wy: vec3<f32>, ray: vec3<f32>,
                    rough: f32, ambient: vec3<f32>, height: f32) -> vec3<f32> {
    let up = vec3<f32>(0., 1., 0.);
    let det = dot(cross(wx,wy),up);
    if (abs(det) < 1e-12 || ray.y <= 0.) { return ambient; }
    let rd = (dx * dot(ray,cross(wy,up)) + dy * dot(ray,cross(up,wx))) / det;
    if (abs(rd.y) < 0.0001) { return ambient; }
    let rear = rd.y > 0.;
    let t = (select(0.04,12.92,rear)-plan.y) / rd.y;
    if (t <= 0.) { return ambient; }
    let hit = vec2<f32>(plan.x+t*rd.x,height+t*ray.y);
    let width = max(0.035, t * rough * rough * .60);
    let low = smoothstep(vec2<f32>(.72,.86)-width,vec2<f32>(.72,.86)+width,hit);
    let high = 1.-smoothstep(vec2<f32>(4.56,2.54)-width,vec2<f32>(4.56,2.54)+width,hit);
    let aperture = low.x*low.y*high.x*high.y;
    let uv = vec2<f32>(atan2(ray.x,-ray.z)/(2.*PI)+.5, clamp(.5-asin(clamp(ray.y,-1.,1.))/PI,.002,.998));
    let layer = i32(textureNumLayers(kit_albedo)) - select(3,4,rear);
    let level = 2.+rough*4.;
    let sunlit = textureSampleLevel(kit_albedo,kit_sampler,uv,layer,level).rgb;
    let night = srgb_decode(textureSampleLevel(kit_normal,kit_sampler,uv,layer,level).rgb);
    let outside = mix(sunlit,night,skyu.night.z);
    // Recover intensity from luminance. Inverting each channel independently
    // makes blue sky disproportionately saturated in the relatively dim room.
    let lum = dot(outside, vec3<f32>(.2126,.7152,.0722));
    let sky_radiance = outside / max(.19,1.-.79*lum);
    // The broad coat lobe integrates the reveal and the outward-facing linen
    // shade with the exterior. A lit blind becomes a diffuse emitter in its own right.
    let window_irradiance = look.hemi_sky.rgb*.22 + look.key.rgb*max(0.,look.key_dir.y)*.66;
    let cloth = vec3<f32>(.81,.70,.52)*window_irradiance/PI;
    let shade = .35+.50*smoothstep(1.70,2.48,hit.y);
    let radiance = mix(sky_radiance,cloth,shade);
    return mix(ambient,radiance,aperture);
}

// Trace the first sun bounce from a waxed floor back through its sash. The real
// shadow map at the board rejects blocked rays from the furniture and adjacent lots.
fn floor_daylight(world: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let sun = look.key_dir.xyz;
    let bounced = vec3<f32>(-sun.x, sun.y, -sun.z);
    let cos_receiver = max(0., dot(normal, -bounced));
    if (sun.y < 0.04 || cos_receiver < 0.02) { return vec3<f32>(0.); }
    let point = (look.enclosure_frame * vec4<f32>(world, 1.)).xyz;
    let level = floor((point.y + .005) / (10./3.)) * (10./3.);
    let r = (look.enclosure_frame * vec4<f32>(bounced, 0.)).xyz;
    let t = (point.y - level) / max(.001, r.y);
    let board = point - t*r;
    if (t < .15 || any(board.xz < vec2<f32>(.15,.16)) || any(board.xz > vec2<f32>(6.84,12.84))) {
        return vec3<f32>(0.);
    }
    let incident = (look.enclosure_frame * vec4<f32>(sun, 0.)).xyz;
    if (abs(incident.z) < .015) { return vec3<f32>(0.); }
    let k = (select(.02,12.96,incident.z>0.) - board.z) / incident.z;
    let glass = vec2<f32>(board.x + k*incident.x, k*incident.y);
    let blur = .035 + .032*(k+t);
    let low = smoothstep(vec2<f32>(.72,.91)-blur,vec2<f32>(.72,.91)+blur,glass);
    let high = 1.-smoothstep(vec2<f32>(4.55,2.53)-blur,vec2<f32>(4.55,2.53)+blur,glass);
    let aperture = low.x*low.y*high.x*high.y;
    if (aperture < .005) { return vec3<f32>(0.); }
    let floor_world = world - bounced*t;
    let sunlit = lk_shadow(floor_world+vec3<f32>(0.,.014,0.),vec3<f32>(0.,1.,0.));
    // Unpolarised Fresnel through the clear wax coat plus the warm diffuse
    // return from the timber beneath. The varnish broadens the reflected beam.
    let fresnel = .042 + .958*pow(1.-sun.y,5.);
    let grain = vec3<f32>(.14,.079,.039) / (1.+.24*t*t);
    return look.key.rgb * sky_cloud_transmittance(floor_world) * sunlit * aperture
        * cos_receiver * (vec3<f32>(fresnel) + grain);
}

// Each row of the atlas is a hemisphere integral under one incident source.
// X/Y interpolate in hardware. The two Z slices remain separate to avoid
// filtering between adjacent tiles or between two lighting conditions.
fn room_axis(grid: vec3<f32>, axis: u32, source: u32, layer: i32, slices: vec2<f32>) -> vec3<f32> {
    let z = floor(grid.z);
    let row = 6u * (6u * source + axis);
    let texel = vec2<f32>(grid.x + slices.x * z, grid.y + f32(row)) + .5;
    let a = textureSampleLevel(kit_orm, kit_sampler, texel / 512., layer, 0.).rgb;
    let b = textureSampleLevel(kit_orm, kit_sampler,
        (texel + vec2<f32>(slices.x * min(1.,slices.y-z),0.)) / 512., layer, 0.).rgb;
    let aa = a*a;
    let bb = b*b;
    if (grid.y >= 256.) {
        // The downstairs probes include the near field of real emitters.
        return mix(aa/max(1.-aa,vec3<f32>(.0001)),bb/max(1.-bb,vec3<f32>(.0001)),fract(grid.z));
    }
    return mix(aa, bb, fract(grid.z));
}

fn room_basis(grid: vec3<f32>, n: vec3<f32>, source: u32, layer: i32, slices: vec2<f32>) -> vec3<f32> {
    let direction = vec3<u32>(select(1u,0u,n.x>=0.), select(3u,2u,n.y>=0.), select(5u,4u,n.z>=0.));
    let w = n*n;
    if (grid.y >= 256.) {
        if (w.y > .99999) { return room_axis(grid,direction.y,source,layer,slices); }
        if (w.x > .99999) { return room_axis(grid,direction.x,source,layer,slices); }
        if (w.z > .99999) { return room_axis(grid,direction.z,source,layer,slices); }
    }
    return w.x * room_axis(grid,direction.x,source,layer,slices)
         + w.y * room_axis(grid,direction.y,source,layer,slices)
         + w.z * room_axis(grid,direction.z,source,layer,slices);
}

fn room_irradiance(point: vec3<f32>, world_normal: vec3<f32>, world: vec3<f32>, downstairs: bool,
                   use_table: bool) -> vec3<f32> {
    // Each floor has six vertical samples per hemisphere. The denser downstairs
    // slices occupy the lower half of the same linear atlas.
    let minimum = vec3<f32>(.22, select(10./3.+.15,.09,downstairs), .23);
    let maximum = vec3<f32>(6.78, select(20./3.-.17,10./3.-.16-.055,downstairs), 12.77);
    let slices = select(vec2<f32>(10.,18.),vec2<f32>(15.,32.),downstairs);
    var grid = clamp((point-minimum)/(maximum-minimum), vec3<f32>(0.), vec3<f32>(1.))
        * vec3<f32>(slices.x-1.,5.,slices.y);
    grid.y += select(0.,256.,downstairs);
    let x = normalize((vec4<f32>(1.,0.,0.,0.) * look.enclosure_frame).xyz);
    let z = vec3<f32>(-x.z,0.,x.x);
    let n = vec3<f32>(dot(world_normal,x),world_normal.y,dot(world_normal,z));
    let layer = i32(textureNumLayers(kit_orm))-3;
    // A unit sky gives unit outgoing radiance on an unobstructed white receiver.
    // The city's SH already includes the streets' occlusion. Recover the open
    // sky measured by eo-look before applying the room's own window transfer.
    let sky_tint = max(lk_sh(vec3<f32>(0.,1.,0.)),vec3<f32>(.00001));
    let sky = sky_tint * (look.house_sky.x / lk_luma(sky_tint));
    let cloud = sky_cloud_transmittance(world);
    let clear = cloud * smoothstep(0.,.35,look.key_dir.y) * (1.-skyu.night.z);
    let diffuse_sky = mix(room_basis(grid,n,0u,layer,slices) * sky,
                          room_basis(grid,n,5u,layer,slices) * look.key.rgb, clear);
    var fixture = vec3<f32>(0.);
    if (!use_table) { fixture = 2. * room_basis(grid,n,6u,layer,slices); }
    var light = diffuse_sky + fixture;
    let sun = look.key_dir.xyz;
    let along = dot(sun,z);
    if (sun.y > 0. && abs(along) > .01) {
        let elevation = asin(clamp(sun.y,0.,1.));
        let t = clamp((elevation-radians(25.))/radians(32.),0.,1.);
        let low = select(1u,3u,along>0.);
        let solar = mix(room_basis(grid,n,low,layer,slices),room_basis(grid,n,low+1u,layer,slices),t);
        let projected = abs(along)/max(.01,cos(clamp(elevation,radians(25.),radians(57.))));
        light += solar * look.key.rgb * projected * cloud;
    }
    return light;
}

// The full-resolution RGB pages keep the lamp and daylight independent. The
// clear-sky page's alpha also holds the overcast sky and indirect solar return.
struct DiningLight { diffuse: vec3<f32>, direct: vec3<f32> };
// The continuous receiver sits 4 mm above the oak, resolving shadows from
// the bowl, cloth and mug handles independently of the room's vertical spacing.
fn oak_incident(point: vec3<f32>) -> vec4<f32> {
    let fraction = (point.xz-look.table_light[0].xz)/(look.table_light[1].xz-look.table_light[0].xz);
    let uv = clamp(fraction*vec2<f32>(256.,64.),vec2<f32>(.5),vec2<f32>(255.5,63.5));
    let layer = i32(textureNumLayers(kit_orm))-3;
    let encoded = textureSampleLevel(kit_orm,kit_sampler,(vec2<f32>(256.,192.)+uv)/512.,layer,0.);
    return encoded*encoded/max(1.-encoded*encoded,vec4<f32>(.0001));
}

fn dining_floor_light(point: vec2<f32>, world: vec3<f32>) -> DiningLight {
    let uv = clamp(point / vec2<f32>(7.,13.), vec2<f32>(.5/512.), vec2<f32>(1.-.5/512.));
    let layers = i32(textureNumLayers(kit_orm));
    let fixture = textureSampleLevel(kit_orm, kit_sampler, uv, layers-4, 0.);
    let day = textureSampleLevel(kit_orm, kit_sampler, uv, layers-1, 0.).rgb;
    let sky_tint = max(lk_sh(vec3<f32>(0.,1.,0.)),vec3<f32>(.00001));
    let sky = sky_tint * (look.house_sky.x / lk_luma(sky_tint));
    let sun = look.key_dir.xyz;
    let cloud = sky_cloud_transmittance(world);
    let clear = cloud * smoothstep(0.,.35,sun.y) * (1.-skyu.night.z);
    let sky_v = clamp(uv.y,.5/256.,1.-.5/256.);
    let overcast = textureSampleLevel(kit_orm,kit_sampler,vec2<f32>(uv.x,.5+.5*sky_v),layers-1,0.).a;
    let lamp = 2.*fixture.rgb*fixture.rgb;
    var return_light = lamp + mix(overcast*overcast*sky, day*day*look.key.rgb, clear);
    if (sun.y > 0.) {
        let across = normalize((vec4<f32>(1.,0.,0.,0.) * look.enclosure_frame).xyz);
        let along = dot(sun, vec3<f32>(-across.z,0.,across.x));
        let elevation = asin(clamp(sun.y,0.,1.));
        let angle = clamp(elevation,radians(25.),radians(57.));
        let t = (angle-radians(25.))/radians(32.);
        let half_texel = vec2<f32>(.5/256.,.5/128.);
        let base = vec2<f32>(.5,.25) * (clamp(uv,half_texel,1.-half_texel) + vec2<f32>(0.,select(0.,1.,along>0.)));
        let low = textureSampleLevel(kit_orm, kit_sampler, base, layers-1, 0.).a;
        let high = textureSampleLevel(kit_orm, kit_sampler, base+vec2<f32>(.5,0.), layers-1, 0.).a;
        return_light += mix(low*low,high*high,t)*look.key.rgb*cloud*abs(along)/cos(angle);
    }
    return DiningLight(return_light, 2.*fixture.a*fixture.a*lamp/max(lamp.r,.00001));
}

// Integrate the finite opal globe around its closest direction to the mirror
// ray. Use the direct-only bake so chair and table shadows also mask the glint.
fn pendant_gloss(direct: vec3<f32>, world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32) -> vec3<f32> {
    let to_lamp = look.dining_lamp.xyz-world;
    let dist = max(length(to_lamp),.01);
    let centre = to_lamp/dist;
    let mirror_ray = reflect(-v,n);
    let offset = mirror_ray*max(dot(to_lamp,mirror_ray),0.)-to_lamp;
    let radius = look.dining_lamp.w;
    let l = normalize(to_lamp + offset*min(1.,radius/max(length(offset),.00001)));
    let alpha = rough*rough;
    let spread = min(1.,alpha+radius/(2.*dist));
    let energy = alpha/max(spread,.0001);
    return PI*direct*(max(dot(n,centre),0.)/max(.01,centre.y))*energy*energy
        *lk_ggx(n,v,l,vec3<f32>(.04),rough);
}

// Irradiance divided by pi. The six points lie on two rows along the sloping bed.
fn front_sky_radiance(world: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let local = (look.enclosure_frame * vec4<f32>(world, 1.)).xyz;
    let u = clamp((local.x - .68) / 1.94, 0., 2.);
    let col = min(u32(u), 1u);
    let du = u - f32(col);
    let dv = clamp((local.z + 1.78) / 1.26, 0., 1.);
    let back = normalize((vec4<f32>(0., 0., 1., 0.) * look.enclosure_frame).xyz);
    let x = dot(normal, vec3<f32>(back.z, 0., -back.x));
    let y = -dot(normal, back);
    let z = normal.y;
    let weights = array<f32, 9>(1.,x,y,z,x*y,y*z,3.*z*z-1.,x*z,x*x-y*y);
    var result = vec3<f32>(0.);
    for (var j = 0u; j < 9u; j++) {
        let front = mix(look.front_daylight[col*9u+j].xyz, look.front_daylight[(col+1u)*9u+j].xyz, du);
        let rear = mix(look.front_daylight[(col+3u)*9u+j].xyz, look.front_daylight[(col+4u)*9u+j].xyz, du);
        result += weights[j] * mix(front, rear, dv);
    }
    return max(result, vec3<f32>(0.));
}

// Match the receiving face, including its orientation. Interior plaster often
// shares both its material and its draw call with the street side of the wall.
fn wall_chart(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let height = (p.y + .16) / 11.16;
    if (p.x>=.52 && p.x<=4.76 && p.y>=7.23 && p.y<=9.65 && p.z>=-.46 && p.z<=.48) {
        let u = (p.x-.52)/4.24;
        let lane = clamp(round((p.x-1.315)/1.31),0.,2.);
        let spring = 20./3.+.72+(1.77-.86)*2.06/1.69;
        let ell = vec2<f32>((p.x-(1.315+1.31*lane))/.595,(p.y-spring)/(.72*2.06/1.69));
        if (p.z>=-.248 && p.z<=.068 && ell.y>=-.02 && abs(length(ell)-1.)<.09
            && dot(n.xy,-ell)>.55) {
            return vec3<f32>(atan2(max(0.,ell.y),-ell.x)/PI,(p.z+.235)/.291,11.+lane);
        }
        if (n.y<-.8 && abs(p.y-7.26)<.04) { return vec3<f32>((p.x-.55)/4.15,(p.z+.445)/.545,14.); }
        if (n.z<-.8 && abs(p.z-.408)<.055) { return vec3<f32>(u,(p.y-7.32)/2.25,9.); }
        if (n.z<-.55 && p.z<-.16) { return vec3<f32>(u,(p.y-7.24)/2.40,10.); }
    }
    if (p.y>=1.90 && p.y<=2.68 && p.z>=-.87 && p.z<=.44) {
        let ell=vec2<f32>((p.x-5.955)/.795,(p.y-1.92)/.72);
        if (ell.y>=-.027 && abs(length(ell)-1.)<.04 && dot(n.xy,-ell)>.7) {
            return vec3<f32>(atan2(max(0.,ell.y),-ell.x)/PI,(p.z+.85)/1.27,15.);
        }
    }
    if (abs(n.y) > .32 || height < 0. || height > 1.) {
        return vec3<f32>(0.,0.,-1.);
    }
    if (n.z<-.70 && p.y>=9.95 && abs(p.z+.43)<.045) {
        return vec3<f32>((p.x+.15)/7.30,(p.y-9.96)/1.04,8.);
    }
    if (p.z >= -.02 && p.z <= 13.02) {
        if (abs(p.x) < .035 && n.x < -.92) { return vec3<f32>(p.z/13.,height,0.); }
        if (abs(p.x-7.) < .035 && n.x > .92) { return vec3<f32>(p.z/13.,height,1.); }
    }
    if (abs(p.z+.05)<.047 && n.z<-.92) { return vec3<f32>(p.x/7.,height,2.); }
    if (abs(p.z-13.04)<.05 && n.z>.92) { return vec3<f32>(p.x/7.,height,3.); }
    let radial = vec2<f32>(p.x-5.955,(.04-p.z)/.90);
    if (p.y>=2.97 && p.y<=10.19 && radial.y>=-.008 && abs(length(radial)-1.)<.026
        && dot(n.xz,vec2<f32>(radial.x,-radial.y))>.89) {
        return vec3<f32>(.5+atan2(radial.x,max(radial.y,0.))/PI,(p.y-2.98)/7.2,4.);
    }
    if (p.y>=-.015 && p.y<=2.93 && p.x>=4.93 && p.x<=6.98) {
        if (abs(p.z+.85)<.03 && n.z<-.92) { return vec3<f32>((p.x-4.95)/2.01,p.y/2.92,5.); }
        let depth = (p.z+.85)/1.27;
        if (depth>=-.01 && depth<=1.01) {
            if (abs(p.x-5.16)<.03 && n.x>.92) { return vec3<f32>(depth,p.y/2.92,6.); }
            if (abs(p.x-6.75)<.03 && n.x<-.92) { return vec3<f32>(depth,p.y/2.92,7.); }
        }
    }
    return vec3<f32>(0.,0.,-1.);
}

fn wall_light_row(chart: vec3<f32>, row: f32) -> vec3<f32> {
    let xy = 64.*vec2<f32>(chart.z%8.,row) + clamp(64.*chart.xy,vec2<f32>(.5),vec2<f32>(63.5));
    var a: vec3<f32>;
    if (chart.z<8.) {
        a = textureSampleLevel(kit_orm,kit_sampler,xy/512.,i32(look.wall_sample.x),0.).rgb;
    } else {
        a = textureSampleLevel(kit_normal,kit_sampler,xy/512.,i32(look.wall_sample.x),0.).rgb;
    }
    let sq = a*a;
    return sq / max(vec3<f32>(.0001),1.-sq);
}

fn measured_wall_light(chart: vec3<f32>) -> vec3<f32> {
    let hour = look.wall_sample.y;
    let lo = min(4.,floor(hour));
    let clear = mix(wall_light_row(chart,lo),wall_light_row(chart,lo+1.),hour-lo);
    let overcast = wall_light_row(chart,6.)*look.wall_overcast.rgb;
    return mix(clear,overcast,look.wall_sample.z)+wall_light_row(chart,7.);
}

fn shade(in: VOut) -> vec4<f32> {
    var m = materials[in.material];
    let ft_du = vec2<f32>(dpdx(in.ft.x), dpdy(in.ft.x));
    let ft_dv = vec2<f32>(dpdx(in.ft.y), dpdy(in.ft.y)); // a structure's formwork v (structures.wgsl)
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
    gg3_sand_slope = select(0.0, in.aux.y * GG3_SAND_SLOPE_FULL, in.aux.y > 0.004);
    var n = face;
    if (dot(in.normal, in.normal) > 0.25) {
        n = normalize(in.normal);
    }
    // Solid sand and shells keep their outward normal across a grazing crest.
    // A ribbon of eelgrass, like the other thin geometry, shades both sides.
    // (the sea bed's habitats, compiled in only with seabed objects in the world: SF_SEABED)
    let seabed = SF_SEABED && (objects[in.object].kind & 255u) == 13u;
    let tissue_tag = in.aux.y * 127.0;
    let rockweed = seabed && tissue_tag >= 31.5 && tissue_tag <= 63.5;
    let bed_surface = seabed && in.aux.x < 0.01 && !rockweed;
    let mineral = bed_surface && in.aux.y < -0.25;
    if (house_bit(objects[in.object].kind, FITTED_ROOM_BIT) && m.thin_absorption == -1.) {
        // (the playable house) the fitted window linen, lit through both faces
        return vec4<f32>(window_linen(in.ft,in.world,m,dot(n,v)>=0.,scan_dx,scan_dy),1.);
    }
    if (GG3_FACET_FIX && !bed_surface) {
        n = gg3_facing_normal(n, v, cross(cc0_drx, cc0_dry), in.rel);
    } else if (!bed_surface && dot(n, v) < 0.0) {
        // Two-sided: shade the side the camera sees.
        n = -n;
    }
    // (stage S1, `--facade-clutter`) a lettered sign face: its normal's y byte
    // carries its code (a vertical face's own y is 0); none without the clutter.
    let sgn_on = SF_FACADE_CLUTTER && (objects[in.object].kind & 255u) == 5u && (m.flags & F_GLASS) == 0u && in.aux.y < -0.004 && (fa_run() & 256u) == 0u;
    if (sgn_on) {
        n = normalize(vec3<f32>(in.normal.x, 0.0, in.normal.z) + vec3<f32>(1e-6, 0.0, 0.0));
        if (dot(n, v) < 0.0) {
            n = -n;
        }
    }

    if SF_HOUSE && look.porch_light[0].w > 0. && frame.copy_step.z < .5 {
        porch_field = porch_irradiance(in.world, n);
    }
    // (look.rs `CITY_CLASS`) with the city's opaque classes split, the street
    // surface has a pipeline of its own and the others compile it out.
    let street_class = select(CITY_CLASS == CITY_STREET, (m.flags & F_STREET) != 0u, CITY_CLASS == CITY_ALL);
    if (street_class) {
        let ss = street_surface(in, n, dpx, dpy);
        if (sx_in_bore(in.object)) {
            return sx_bore_light(ss, m, v, in.world, in.object); // a tunnel bore's roadway (structures.wgsl)
        }
        // (`--far-street-light`) past the frame's lamp lights, the street lit from its lamps' pools.
        rm_emit = ss.albedo * lk_far_street_light(in.world, frame.cam_pos.xyz);
        return look_light(ss, m, v, in.world, n, in.object);
    }
    if (CITY_CLASS == CITY_STREET) {
        return vec4<f32>(0.0); // (never reached: the street class draws only the street material)
    }
    if (SF_STRUCTURES && (objects[in.object].kind & 255u) == 12u) {
        // a grade-separated road's concrete (structures.wgsl; none without `--structures`: SF_STRUCTURES).
        var sx = sx_concrete(in, m, n, dpx, dpy, ft_du, ft_dv);
        if (sl_level() != 0u) {
            sl_structure(&sx, in.world, n, dpx, dpy); // street life (`--street-life`): graffiti round the columns' bases
        }
        if (sx_in_bore(in.object)) {
            return sx_bore_light(sx, m, v, in.world, in.object); // a tunnel bore's lining and walkways
        }
        return look_light(sx, m, v, in.world, n, in.object);
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
    // (the house builder's finishes: a model vertex's normal w over 0.49 comes from it alone, SF_HOUSE)
    let house_finish = SF_HOUSE && (objects[in.object].kind & 255u) == 4u && in.aux.y > 0.49;
    let interior_transfer = house_finish && in.aux.y > 64.5 / 127.0;
    var vcol = select(in.color, vec3<f32>(1.0), house_finish);
    if (kit_object) {
        vcol = pow(in.color, vec3<f32>(2.2));
    }
    // (stage S1) a lettered face's paint is its field's (its colour bytes are its text box and cell).
    var sgn = SgnCode();
    if (sgn_on) {
        sgn = sgn_decode(in.normal.y, in.aux.y, in.color.b);
        vcol = pow(SGN_FIELDS[min(sgn.field, SGN_FIELD_COUNT - 1u)] / 255.0, vec3<f32>(2.2));
    }
    rm_emit = vec3<f32>(0.0);
    if ((m.flags & F_VERTEX_COLOUR) != 0u) {
        s.albedo = vcol;
        let paint_kind = objects[in.object].kind & 255u;
        if (bed_surface && in.aux.y >= -0.25) {
            // Mineral grains and small dark inclusions on the bed. Filter both
            // scales by their pixel footprint so the bottom stays still in motion.
            let footprint = max(length(dpx.xz), length(dpy.xz));
            let local = in.world.xz - objects[in.object].min.xz;
            let fleck = st_noise(local * 135.0);
            let fine = st_noise(local * 640.0);
            let fleck_weight = 1.0 - smoothstep(0.002, 0.009, footprint);
            let fine_weight = 1.0 - smoothstep(0.0006, 0.0025, footprint);
            s.albedo *= (1.0 + (fine - 0.5) * 0.78 * fine_weight)
                * (1.0 + (fleck - 0.5) * 0.9 * fleck_weight);
            s.rough = 0.91;
        }
        if (paint_kind == 7u || paint_kind == 11u) {
            // street paint ('painted-metal', roughness 0.58) and the
            // verge bank (the concrete material's 'sidewalk' class); a
            // structure's paint, and its joints and downspouts in the bank's class.
            let cls = u32(round(in.aux.x));
            if (cls == SC_PAINT) {
                s.rough = ST_PAINT_ROUGHNESS;
                s.env = ENV_PAINTED_METAL;
                // worn paint inside the geometric marking (an ambientCG road-line scan as the wear mask only).
                // (`--grime`) the scan's holes toned down: the markings' own wear (`wx_paint`) does the wearing.
                let scan_bare = select(1.0, WX_PAINT_SCAN_BARE, wx_level() != 0u && paint_kind == 7u && !WX_STREET_V1);
                // (`--area-paint`) a marking as wide as a lane is an area of colour, worn as one.
                let gp = wx_kerb_grad();
                let gg = dot(gp, gp);
                let area = WX_AREA_PAINT && paint_kind == 7u && gg > 0.0 && gg < 1.0 / (WX_AREA_MIN_W * WX_AREA_MIN_W);
                if (!area) {
                    cc0_paint_wear(&s, in.world, in.rel, in.aux.y * 6.35, cc0_drx, cc0_dry, scan_bare);
                }
                if (wx_level() != 0u && paint_kind == 7u) {
                    if (area) {
                        wx_area_paint(&s, in.world, in.aux.y * 6.35, gp, dpx, dpy);
                    } else {
                        wx_paint(&s, in.world, dpx, dpy); // weathering (`--grime`): faded, chipped markings
                    }
                }
            } else {
                s.rough = ST_CONCRETE_ROUGHNESS;
                s.env = ENV_SIDEWALK;
            }
        }
    }
    // the terrain's ground scans around its tone (not the beach sand: below), before the wet margin.
    let cc0_terrain = (m.flags & F_VERTEX_COLOUR) != 0u && (objects[in.object].kind & 255u) == 1u;
    let cc0_sand = cc0_terrain && (m.flags & LM_F_WET_GROUND) != 0u && in.aux.x < 253.5;
    // the terrain surface pass's ground (its vertices' alpha GG3_MATERIAL_ALPHA): cc0.wgsl gg3_terrain
    let gg3_ground = GG3_MATERIALS && cc0_terrain && abs(in.aux.x - GG3_MATERIAL_ALPHA) < 0.5;
    if (gg3_ground) {
        gg3_terrain(&s, in.world, n, dpx, dpy, in.aux.y);
    } else if (cc0_terrain && !cc0_sand) {
        cc0_ground(&s, in.world, n, dpx, dpy);
    }
    if (CITY_CLASS != CITY_GENERAL && lm_kind(m.flags) != 0u) { // (the landmark class's own pipeline, or the one pipeline)
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
    } else if ((m.flags & F_TEXTURED) != 0u && (!seabed || mineral)) {
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
            s.albedo = alb * m.base.rgb * select(vcol, vec3<f32>(0.74) + 0.85 * vcol, mineral);
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
        if (sl_level() != 0u) {
            wx_window_life_pre(vec4<f32>(in.color, in.aux.y), in.world, ng_out, in.object, room); // stage W5 (`--street-life`, weathering.wgsl): the wall beside, sill plants, a vacant shop
        }
        let r = rm_kit_pane(vec4<f32>(in.color, in.aux.y), in.world, ng_out, frame.cam_pos.xyz, room, look.rooms_env, look.rooms_work, objects[in.object].lod_group); // + the building's group: its shop fronts (stage N2, `--shop-light`)
        // By day the room is lit through the pane and seen back through it:
        // its share goes as the glass's Tv squared against the clear unit's
        // (rooms.rs `pane_transmittance`; no Tv bits: as before).
        var through = 1.0;
        if (RM_PANE_TV) { // (`--no-pane-transmittance` compiles it out)
            let tv = f32((m.flags >> RM_PANE_TV_SHIFT) & 15u) / 15.0;
            through = select(1.0, min(tv * tv / (RM_PANE_TV_REFERENCE * RM_PANE_TV_REFERENCE), 1.0), tv > 0.0);
        }
        s.albedo = mix(s.albedo, r.day.xyz * through, r.day.w);
        // (`--room-lit-runs`) a dark pane keeps its glass's reflection as the light fades.
        let keep_reflection = select(0.0, look.rooms_env.x, RM_LIT_RUNS);
        s.metal = mix(s.metal, s.metal * 0.3, r.day.w * (1.0 - keep_reflection));
        // A blended pane's room reads at full strength after the blend (1 / opacity, floored at 0.25).
        let transmission = select(1.0, 1.0 / max(0.25, m.base.a), (m.flags & F_BLEND) != 0u);
        rm_emit = r.night * transmission;
        if (wx_level() != 0u || sl_level() != 0u) {
            wx_window(&s, &m, vec4<f32>(in.color, in.aux.y), in.world, ng_out, in.object, room, dpx, dpy); // stage W5: dirty glass (`--grime`); stickers, scratched tags (`--street-life`)
        }
    }
    // facade clutter (`--facade-clutter`): a lit sign face carries a glow code in the normal's w
    // byte (eo-kit clutter.rs, kit.rs), a lettered one (stage S1) its code; none on any other kit
    // surface, so nothing runs without it. One shop lookup a fragment (`fc_lit`).
    var fc = FcGlow();
    var fc_on = false;
    if (SF_FACADE_CLUTTER && kit_object && (m.flags & F_GLASS) == 0u && in.aux.y > 0.49 && (fa_run() & 256u) == 0u) {
        fc = fc_glow(s.albedo, in.aux.y, in.world, in.normal);
        fc_on = true;
    }
    if (sgn_on) {
        fc = sgn_sign(&s, sgn, in.ft * SGN_UV_SPAN + SGN_UV_MIN, ft_du * SGN_UV_SPAN, ft_dv * SGN_UV_SPAN, in.world, in.normal, dpx, dpy, objects[in.object].lod_group);
        fc_on = sgn.mode != 0u;
    }
    if (fc_on) {
        rm_emit = rm_emit + fc.emit * fc_lit(fc.q, fc.n, fc.roll) * select(1.0, FC_CINE_SIGN_GAIN, SF_CINEMATIC && sky_on(skyu.flags.x, SKY_CINEMATIC));
    }
    if (house_bit(objects[in.object].kind, GRAVEL_BIT)) {
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
    if (house_bit(objects[in.object].kind, LIMEWASH_BIT)) {
        let exposure = clamp((in.color.b * 255.0 - 3.0) / 55.0, 0.0, 1.0);
        if (house_bit(objects[in.object].kind, COURTYARD_BOUNCE_BIT)) {
            let stain = exposure * st_noise(in.world.xy*5.5);
            s.albedo *= mix(vec3<f32>(1.),vec3<f32>(.45,.35,.22),stain);
        } else {
            s.albedo *= mix(vec3<f32>(1.0), vec3<f32>(0.79, 0.75, 0.70), exposure);
        }
    }
    let cut_cotton = house_bit(objects[in.object].kind, CUT_COTTON_BIT);
    let local_scan = cut_cotton || house_bit(objects[in.object].kind, DOOR_SCAN_BIT | LOCAL_UV_SCAN_BIT);
    var bedroom_point = vec3<f32>(0.);
    if (house_bit(objects[in.object].kind, BEDROOM_LIGHT_BIT) && vo_house_z(in) > .5) {
        bedroom_point = (look.enclosure_frame * vec4<f32>(in.world + .025*n,1.)).xyz;
        measured_room = bedroom_point.y > 10./3. && bedroom_point.y < 20./3.;
    }
    var measured_floor = false;
    var floor_light: DiningLight;
    if (house_bit(objects[in.object].kind, DINING_FLOOR_LIGHT_BIT) && n.y > .7) {
        let parcel = (look.enclosure_frame * vec4<f32>(in.world,1.)).xyz;
        measured_floor = abs(parcel.y) < .04;
        if (measured_floor) { floor_light = dining_floor_light(parcel.xz,in.world); }
    }
    let table_board = house_bit(objects[in.object].kind, TABLE_BOARD_BIT);
    let oak_finish = house_bit(objects[in.object].kind, OAK_FINISH_BIT);
    let joinery = table_board && look.table_light[1].w > 0.;
    let floor_scan = table_board || local_scan || house_bit(objects[in.object].kind, FLOOR_SCAN_BIT);
    if (floor_scan && (local_scan || table_board || abs(n.y) > 0.7)) {
        var layer = select(i32(textureNumLayers(kit_albedo)) - 2, m.layer, local_scan);
        var uv = in.ft;
        var uv_dx = scan_dx;
        var uv_dy = scan_dy;
        let waxed = house_bit(objects[in.object].kind, WAXED_BOARD_BIT);
        let floor_survey = house_bit(objects[in.object].kind, FLOOR_REFLECTION_BIT) && abs((look.enclosure_frame*vec4<f32>(in.world,1.)).y)<.04;
        if (floor_survey && house_bit(objects[in.object].kind, FITTED_FLOOR_BIT)) {
            // The final fitted layer precedes floor, yard, street and kitchen probes and the common boards.
            layer = i32(textureNumLayers(kit_albedo)) - 6;
        }
        if (waxed && !floor_survey) {
            let board = in.ft.x / .18;
            let row = (u32(max(0.,floor(board))) * 5u + in.material) % 8u;
            let grain = select(1.,-1.,(row & 1u) != 0u);
            uv = vec2<f32>(grain*in.ft.y*.5 + fract(f32(row)*.618034),
                           (f32(row)+.07+.86*fract(board))/8.);
            // Derivatives of the continuous wood grain within a single plank.
            uv_dx = vec2<f32>(grain*.5*scan_dx.y, .86/(.18*8.)*scan_dx.x);
            uv_dy = vec2<f32>(grain*.5*scan_dy.y, .86/(.18*8.)*scan_dy.x);
        }
        s.albedo = textureSampleGrad(kit_albedo, kit_sampler, uv, layer, uv_dx, uv_dy).rgb * select(vec3<f32>(1.0), m.base.rgb, local_scan);
        s.rough = textureSampleGrad(kit_orm, kit_sampler, uv, layer, uv_dx, uv_dy).g;
        let bump = textureSampleGrad(kit_normal, kit_sampler, uv, layer, uv_dx, uv_dy).xyz * 2.0 - 1.0;
        let det = uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y;
        if (abs(det) > 1e-10) {
            let along = normalize((dpx * uv_dy.y - dpy * uv_dx.y) * sign(det));
            let across = normalize((dpy * uv_dx.x - dpx * uv_dy.x) * sign(det));
            let relief = select(select(1.0, m.params.z, local_scan), select(0.28,look.oak_finish[1].x,oak_finish), table_board);
            s.n = normalize(n * bump.z + relief * (along * bump.x - across * bump.y));
        }
        if (cut_cotton) {
            // Specular IOR Level .18 in the source cotton material.
            dielectric_scale = .36;
        }
        if (waxed) {
            if (measured_room) {
                // The oiled upper boards have their own stain and roughness in Blender.
                s.albedo *= vec3<f32>(.70,.48,.32);
                s.rough = clamp(.75 * s.rough + .27, .03, 1.);
                dielectric_scale = .48;
            }
            s.env = ENV_INTERIOR_FLOOR;
            if (floor_survey) {
                let plan = (look.enclosure_frame*vec4<f32>(in.world,1.)).xz;
                room_radiance = floor_reflected_room(plan,in.world,reflect(-v,s.n));
            } else {
                room_radiance = board_reflection(in.ft,scan_dx,scan_dy,dpx,dpy,
                    reflect(-v,s.n),s.rough,select(board_ambient(in.color.rg*in.color.rg),floor_light.diffuse,measured_floor),0.);
            }
        } else if (table_board) {
            let frame_pos = (look.enclosure_frame * vec4<f32>(in.world,1.)).xyz;
            let wood_axis = normalize((vec4<f32>(1.,0.,0.,0.) * look.enclosure_frame).xyz);
            let cut_face = abs(dot(select(n,face,joinery),wood_axis)) > .86;
            if (joinery && cut_face) {
                let lo = look.table_light[0];
                let hi = look.table_light[1];
                let side = select(0.,48.,frame_pos.x > (lo.x+hi.x)*.5);
                let size = vec2<f32>(504./(hi.z-lo.z),40./hi.w);
                let cut = (frame_pos.zy-vec2<f32>(lo.z,lo.w-hi.w))*size;
                let uv = (vec2<f32>(4.,420.+side)+cut)/512.;
                let dx = (look.enclosure_frame*vec4<f32>(dpx,0.)).zy*size;
                let dy = (look.enclosure_frame*vec4<f32>(dpy,0.)).zy*size;
                let lod = clamp(log2(max(1.,max(length(dx),length(dy)))),0.,2.);
                let encoded = textureSampleLevel(kit_orm,kit_sampler,uv,layer,lod).rba;
                s.albedo = encoded*encoded;
                s.rough = .55;
                s.n = n;
            } else if (cut_face) {
                // The saw cut runs across the growth rings, perpendicular to the long grain.
                let cut = frame_pos.zy;
                let rings = sin(length(cut - floor(cut/.32)*.32) * 850. + 2.35*st_noise(cut*45.));
                let resolved_rings = 1.-smoothstep(.002,.011,max(length(dpx),length(dpy)));
                s.albedo = m.base.rgb * (.745 + .175*rings*resolved_rings);
                s.rough = .55;
                s.n = n;
            } else {
                s.albedo *= mix(vec3<f32>(1.),vec3<f32>(.54,.37,.24),.64);
                // Wax fills the open pores of the planed table. Keep the roughness
                // scan, with a broader reflection than the floor varnish.
                if (!oak_finish) { s.rough = clamp(.31 + .43*s.rough, .36, .67); }
                if (n.y > .64) {
                    let local_dx = (look.enclosure_frame * vec4<f32>(dpx,0.)).xz;
                    let local_dy = (look.enclosure_frame * vec4<f32>(dpy,0.)).xz;
                    s.env = ENV_INTERIOR_FLOOR;
                    if (oak_finish) {
                        oak_reflections(frame_pos,in.world,v,s.n,n,house_bit(objects[in.object].kind, TABLE_LIGHT_BIT));
                    } else {
                        room_radiance = board_reflection(frame_pos.xz,local_dx,local_dy,dpx,dpy,
                            reflect(-v,s.n),s.rough,select(board_ambient(in.color.rg*in.color.rg),vo_table_fixture(in).rgb,joinery),frame_pos.y);
                    }
                }
            }
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
    if (house_bit(objects[in.object].kind, COURTYARD_BOUNCE_BIT)) {
        let upward = max(0.,n.y);
        let fill = (.033 + .039 * upward) * s.ao;
        rm_emit += s.albedo * fill * (look.hemi_sky.rgb + look.key.rgb*max(look.key_dir.y,0.));
    }
    if (SF_HOUSE && measured_floor) {
        // Furniture occlusion is already resolved on the neutral floor receiver.
        room_enclosure = 1.;
        measured_room = true;
        room_diffuse = floor_light.diffuse;
        rm_emit += pendant_gloss(floor_light.direct,in.world,s.n,v,s.rough);
    } else if (interior_transfer && (m.flags & F_GLASS) == 0u) {
        // Transfer is sampled at build time against the actual windows, furniture
        // and luminaire. Window light follows the current sky; the pendant stays warm.
        // Near-field visibility is baked against each seat, post and rug in the room.
        s.ao *= clamp((in.aux.y * 127.0 - 65.0) / 62.0, 0.0, 1.0);
        let transfer = in.color.rg * in.color.rg;
        // Window and fixture transfer already include a diffuse bounce. Weight
        // the outdoor field by the room's sky visibility instead of adding it twice.
        room_enclosure = min(vo_house_z(in),1.);
        room_daylight = transfer.x;
        var downstairs = false;
        let room_kind = objects[in.object].kind;
        if (house_bit(room_kind, DOWNSTAIRS_LIGHT_BIT | TABLE_LIGHT_BIT) && vo_house_z(in) > .5) {
            let room_point = (look.enclosure_frame * vec4<f32>(in.world + .025*n,1.)).xyz;
            var top = false;
            var table = false;
            var lamps: DiningLight;
            if (house_bit(room_kind, TABLE_LIGHT_BIT)) {
                let local = (look.enclosure_frame * vec4<f32>(in.world,1.)).xyz;
                top = table_board && n.y>.85 && abs(local.y-look.table_light[0].w)<.004;
                table = top || vo_house_z(in) > 1.5;
                if (table) {
                    var measured = select(vo_table_reverse(in),vo_table_fixture(in),dot(in.normal,v)>=0.);
                    var beam = measured.a * max(0.,normalize(look.dining_lamp.xyz-in.world).y);
                    if (top) {
                        measured = oak_incident(local);
                        beam = measured.a;
                    }
                    lamps.diffuse = measured.rgb;
                    lamps.direct = look.table_light[2].rgb * (beam/look.table_light[2].r);
                    rm_emit += pendant_gloss(lamps.direct,in.world,s.n,v,s.rough);
                    if (oak_coated) { oak_pendant = lamps.direct; }
                }
            }
            downstairs = (table || house_bit(room_kind, DOWNSTAIRS_LIGHT_BIT))
                       && room_point.y >= 0. && room_point.y < 10./3.;
            if (downstairs) {
                measured_room = true;
                // The surface bake already includes the contact shadow. Apply
                // near-field occlusion only to the coarse daylight component.
                surface_fixture = lamps.diffuse;
                if (table && oak_coated && (!joinery || top)) {
                    // Both surveys contain the real contact shadows from the bowl and book.
                    surface_fixture += oak_daylight;
                } else {
                    room_diffuse = room_irradiance(room_point,s.n,in.world,true,table);
                }
            }
        }
        if (SF_HOUSE && vo_house_z(in) > .5 && !measured_room) {
            rm_emit += s.albedo * (1.-s.metal) * floor_daylight(in.world, s.n) / PI;
        }
        let window = 0.76 * transfer.x * (look.hemi_sky.rgb + look.key.rgb * max(look.key_dir.y, 0.0));
        let pendant = transfer.y * (1.-porch_field.z) * vec3<f32>(1.55, 1.05, 0.56);
        if (SF_HOUSE && measured_room) {
            // Direct sunlight still comes from the current cascaded shadow map.
            if (!downstairs) {
                room_diffuse = room_irradiance(bedroom_point,s.n,in.world,false,false);
                rm_emit += s.albedo * (1.0 - s.metal) * pendant;
            }
        } else {
            rm_emit += s.albedo * (1.0 - s.metal) * (window + pendant);
        }
    }
    var wall_pigment = vec4<f32>(0.);
    if (house_bit(objects[in.object].kind, WALL_DAYLIGHT_BIT)) {
        let parcel = (look.enclosure_frame*vec4<f32>(in.world,1.)).xyz;
        let local_n = normalize((look.enclosure_frame*vec4<f32>(n,0.)).xyz);
        let chart = wall_chart(parcel,local_n);
        if (chart.z >= 0. && look.wall_sample.x > 0.) {
            measured_wall = smoothstep(0.,.20,look.key_dir.y)*(1.-skyu.night.z);
            measured_front = measured_wall;
            front_ambient = LK_PI*measured_wall_light(chart);
            let origin = 128.*vec2<f32>(chart.z%4.,floor(chart.z/4.));
            let uv = (origin+clamp(128.*chart.xy,vec2<f32>(.5),vec2<f32>(127.5)))/512.;
            wall_pigment = textureSampleLevel(kit_albedo,kit_sampler,uv,i32(look.wall_sample.x),0.);
            // The wall field already includes diffuse sky, practicals and sun bounce.
            rm_emit *= 1.-measured_wall;
        }
        if porch_field.z > 0. {
            let porch = porch_chart(parcel, local_n);
            if porch.z >= 0. {
                // The closed-leaf bake includes its indirect bounce and shadow.
                // As it opens the independent area-light field handles its live shadow.
                porch_measured = porch_field.z*(1.-smoothstep(0.,.18,abs(look.porch_light[6].w)));
                let weight = mix(measured_front,1.,porch_measured);
                if weight > 0. {
                    front_ambient = mix(front_ambient*measured_front,LK_PI*porch_surface_light(porch),porch_measured)/weight;
                    measured_front = weight;
                    measured_wall = mix(measured_wall,1.,porch_measured);
                    wall_pigment = mix(wall_pigment,porch_pigment(porch),porch_measured);
                    rm_emit *= 1.-porch_measured;
                }
            }
        }
    }
    if (house_bit(objects[in.object].kind, THIN_FIBRE_BIT)) {
        // A woven shade transmits diffuse skylight as well as direct light.
        // A single shadow query on the source-facing side keeps its folds and the
        // tree's canopy from emitting in the shadow of neighbouring buildings.
        let to_light = look.key_dir.xyz;
        let back = max(0., dot(-n, to_light));
        var beam = vec3<f32>(0.);
        if (back > 0.001) {
            beam = look.key.rgb * back * lk_shadow(in.world, -n) * sky_cloud_transmittance(in.world);
        }
        let sky = look.hemi_sky.rgb + lk_sh(-n) * look.ambient.w;
        rm_emit += s.albedo * (0.19 * beam + 0.21 * sky) / PI;
        s.rough = max(s.rough, 0.87);
    }
    if (house_bit(objects[in.object].kind, QUILT_FIBRE_BIT)) {
        // The cut cotton already carries the roughness of the source material.
        if (!cut_cotton) { s.rough = max(0.90, s.rough); }
        s.metal = 0.;
    }
    if (house_bit(objects[in.object].kind, FITTED_ROOM_BIT)) {
        // Negative normal.w stores the fraction of nearby diffuse sky blocked by the planted bed.
        s.ao *= clamp(1. + in.aux.y, 0., 1.);
        if (m.layer >= 0) {
            let st = textureSampleGrad(kit_albedo,kit_sampler,in.ft,m.layer,scan_dx,scan_dy);
            let properties = textureSampleGrad(kit_orm,kit_sampler,in.ft,m.layer,scan_dx,scan_dy);
            s.albedo = m.base.rgb * st.rgb;
            s.rough = clamp(m.params.x * properties.g,.03,1.);
            s.metal = m.params.y * properties.b;
            let bump = textureSampleGrad(kit_normal,kit_sampler,in.ft,m.layer,scan_dx,scan_dy).xyz*2.-1.;
            let det = scan_dx.x*scan_dy.y-scan_dx.y*scan_dy.x;
            if (abs(det)>1e-12) {
                let t = normalize((dpx*scan_dy.y-dpy*scan_dx.y)*sign(det));
                let b = normalize((dpy*scan_dx.x-dpx*scan_dy.x)*sign(det));
                s.n = normalize(n*bump.z+m.params.z*(t*bump.x-b*bump.y));
            }
        }
        if (house_bit(m.flags, F_STOOP_LIGHT) && look.porch_light[0].w > 0.) {
            let p = (look.enclosure_frame*vec4<f32>(in.world,1.)).xyz;
            let local_n = normalize((look.enclosure_frame*vec4<f32>(n,0.)).xyz);
            let chart = stoop_chart(p,local_n);
            if chart.z >= 0. {
                let atlas = m.layer+i32(m.params.w);
                let weight = 1.-smoothstep(.05,.32,abs(look.porch_light[6].w));
                measured_front = weight;
                measured_wall = weight;
                porch_measured = weight;
                front_ambient = LK_PI*stoop_lighting(chart,atlas);
            }
        }
        if (house_bit(m.flags, F_FRONT_DAYLIGHT)) {
            measured_front = smoothstep(0., .20, look.key_dir.y) * (1. - skyu.night.z);
            front_ambient = LK_PI * front_sky_radiance(in.world, s.n);
        }
        if (house_bit(objects[in.object].kind, DOWNSTAIRS_LIGHT_BIT)) {
            let local = (look.enclosure_frame*vec4<f32>(in.world+.022*n,1.)).xyz;
            room_enclosure = 1.;
            measured_room = true;
            let point = (look.enclosure_frame*vec4<f32>(in.world,1.)).xyz;
            let local_finish = u32(m.thin_absorption) != KITCHEN_SHARED ||
                               (all(point>=look.kitchen_frame[3].xyz) && all(point<=look.kitchen_frame[4].xyz));
            if (m.thin_absorption > 0. && local_finish) {
                s.env=ENV_LOCAL_PROBE;
                fitted_light(point,in.ft,in.world,v,n,s,m);
            } else {
                room_diffuse = room_irradiance(local,s.n,in.world,true,false);
            }
        }
    }
    // weathering (Blender buildings) and archetype ageing (kit buildings).
    fa_apply(&s, m, in.world, n, in.object, dpx, dpy);
    if (sl_level() != 0u && kit_object && (m.flags & F_GLASS) == 0u) {
        sl_kit_wall(&s, in.world, n, in.object, dpx, dpy); // street life (`--street-life`): graffiti, buffs, posters at the building line
    }
    if (SF_HOUSE && wall_pigment.a>.01) {
        // The source pigment already includes its scan, plaster tint and rain streaks.
        // Alpha is the coverage of the projection at material boundaries.
        s.albedo = mix(s.albedo,wall_pigment.rgb/wall_pigment.a,smoothstep(.15,.9,wall_pigment.a));
    }
    if (house_finish && !floor_scan && in.color.b > 0.5 / 255.0 && in.color.b < 2.5 / 255.0) {
        s.env = ENV_LOCAL_PROBE;
        let row = round(in.color.b * 255.0) - 1.0;
        room_radiance = kitchen_reflection(reflect(-v, s.n), in.aux.x * (6.2831853 / 255.0), row, s.rough);
    }
    if (house_bit(objects[in.object].kind, ENTRY_BRASS_BIT)) {
        // The lever reflects the street through a finite opening in the plaster porch.
        let ray = reflect(-v,s.n);
        let local_ray = (look.enclosure_frame * vec4<f32>(ray,0.)).xyz;
        let uv = vec2<f32>(atan2(ray.x,-ray.z)/(2.*PI)+.5,
                           clamp(.5-asin(clamp(ray.y,-1.,1.))/PI,.002,.998));
        let layer = i32(textureNumLayers(kit_albedo)) - 3;
        let mip = 1.0 + 5.0*s.rough;
        let day = textureSampleLevel(kit_albedo,kit_sampler,uv,layer,mip).rgb;
        let night = srgb_decode(textureSampleLevel(kit_normal,kit_sampler,uv,layer,mip).rgb);
        let plate = mix(day,night,skyu.night.z);
        let incident = plate / max(.15,1.-.83*dot(plate,vec3<f32>(.2126,.7152,.0722)));
        let opening = smoothstep(.05,.65,-normalize(local_ray).z);
        let wall = vec3<f32>(.48,.40,.31)*(look.ambient.rgb+.34*look.hemi_sky.rgb)/PI;
        s.env = ENV_LOCAL_PROBE;
        room_radiance = mix(wall,incident,opening);
    }
    // the shader-only facade on a near envelope wall.
    if ((fa_run() & 256u) != 0u && kit_object && objects[in.object].lod_role == 4u && dot(in.normal, in.normal) > 0.25 && abs(in.normal.y) < 0.3 && frame.copy_step.z < 0.5) {
        ft_facade(&s, &m, in.world, normalize(in.normal), in.ft, ft_du, in.object, cc0_drx, cc0_dry);
    }
    if ((objects[in.object].kind & 255u) == 0u) {
        if (SF_FAR_FACADE && fb_on(in)) {
            rm_emit = rm_emit + fb_apply(&s, in, n, v, dpx, dpy, cc0_daux * 127.0); // the far box's facade (far_box.wgsl; `--no-far-facade` compiles it out)
        } else {
            rm_emit = rm_emit + sky_far_windows(in.world, n, in.object, dpx, dpy); // lit windows on the far boxes
        }
    }

    if (SF_COVE_SEDIMENT && (cc0_terrain || (bed_surface && in.aux.y > 0.5)) && look.cove.x > 0.0 && in.world.y < LM_SEA_LEVEL_Y + 0.035) { // (`--cove-sediment`)
        lm_cove_sediment(&s, in.world, n, dpx, dpy);
    }
    let street_pane = house_bit(objects[in.object].kind, FRONT_SASH_BIT) && in.color.b > 3.5/255.0;
    if (street_pane && frame.copy_step.z < 0.5) {
        let angle = in.aux.x * (2.0 * PI / 255.0);
        let rear_sash = in.color.b > 4.5 / 255.0;
        let outside = select(1.0, -1.0, rear_sash) * vec2<f32>(sin(angle),-cos(angle));
        let outward=dot(v.xz,outside)>0.;
        let ray=reflect(-v,n);
        if SF_HOUSE && look.window_sample.x>.5 && (outward || (window_optics[6].w&0x80000000u)!=0u) {
            // (the playable house, `--house-window-optics`) both faces of the pane traced through the measured scene
            // Resolve both interfaces of a thin uncoated sheet at its outer face.
            if dot(in.normal,v)<0. { return vec4<f32>(0.); }
            let cos_i = clamp(dot(n,v),0.,1.);
            let ior = 1.49;
            let cos_t = sqrt(max(0.,1.-(1.-cos_i*cos_i)/(ior*ior)));
            let rs=(cos_i-ior*cos_t)/max(.0001,cos_i+ior*cos_t);
            let rp=(ior*cos_i-cos_t)/max(.0001,ior*cos_i+cos_t);
            let reflection=.5*(rs*rs+rp*rp);
            return vec4<f32>(window_trace(in.world+.002*ray,ray,1e8,0u).rgb, 2.*reflection/(1.+reflection));
        }
        if !outward { return look_light(s,m,v,in.world,n,in.object); }
        let pane_layer = i32(textureNumLayers(kit_albedo)) - select(3, 4, rear_sash);
        // Each face blends separately when using the captured panorama.
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
    if (rockweed) {
        // Thin margins transmit scattered light from the other face. The rib
        // and swollen tips absorb more. The water pass attenuates this radiance
        // along with the rock, using the same path to the light and camera.
        s.albedo = vcol;
        s.env = ENV_SUBMERGED;
        s.rough = 0.54;
        s.metal = 0.0;
        let mm = max((tissue_tag - 32.0) * 0.25, 0.25);
        let back = max(dot(-n, look.key_dir.xyz), 0.0);
        var incoming = look.ambient.xyz + mix(look.hemi_ground.xyz, look.hemi_sky.xyz, 0.5 - 0.5 * n.y);
        if (back > 0.0) {
            incoming += look.key.xyz * back * lk_shadow(in.world, -n) * sky_cloud_transmittance(in.world);
        }
        let transmission = vec3<f32>(0.14, 0.19, 0.07) * exp(-mm * vec3<f32>(0.65, 0.45, 1.15));
        rm_emit = transmission * incoming / LK_PI;
    }
    // a landmark deck's interior (eo-landmarks access_sky.rs): the sky it
    // sees through its openings, baked per vertex into the alpha byte (255:
    // none), on the indirect light only (the sun's cascades cast the patches);
    // and the player's body on a deck: its contact occlusion (landmarks_deck.wgsl).
    // (`SF_LANDMARK_ACCESS`: compiled out with `--no-landmark-access`.)
    let lm_kind = objects[in.object].kind & 255u;
    if (SF_LANDMARK_ACCESS && (lm_kind == 9u || lm_kind == 10u)) {
        if (in.aux.x < 254.5) {
            s.ao = s.ao * (in.aux.x / 254.0);
        }
        s.ao = s.ao * deck_player_occlusion(in.world, n);
    }

    // Lighting, shadows and fog: the JS game's light rig (look_light.wgsl).
    if (sx_in_bore(in.object)) {
        return sx_bore_light(s, m, v, in.world, in.object); // a tunnel bore's paint and luminaire (structures.wgsl)
    }
    if SF_HOUSE && (m.flags & F_JOINERY_LIGHT) != 0u && frame.copy_step.z < .5 {
        // (the playable house) source-lit joinery: its baked finish traced through the windows
        let finish = (m.flags >> 8u) & 7u;
        joinery_diffuse = window_trace(in.world + .012*n, -n, .024, finish);
    }
    // (`--far-street-light`) past the frame's lamp lights, a street lit from its lamps' pools.
    let street_kind = objects[in.object].kind & 255u;
    if (street_kind == 6u || street_kind == 7u || street_kind == 2u) {
        rm_emit += s.albedo * lk_far_street_light(in.world, frame.cam_pos.xyz);
    }
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
