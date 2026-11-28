// what is drawn over the lit city before the resolve (appended to
// look_common.wgsl): the street lamps' luminaires (a core at the lamp's own
// luminance and the air light around it, lamp-haze.js), and rain
// (precipitation-field.js: a near set of 6,000 and a far set of 5,000
// world-anchored drops wrapped round the camera, streaks as long as the
// drop falls in the exposure (0.022 s), fall speed 3.5 + 1.9 ln(1 + rate)
// m/s, the live share and strength from the rate, tinted by the fog colour,
// premultiplied blend c a + dst (1 - 0.35 a)).

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var<uniform> skyu: Sky;
@group(0) @binding(2) var<storage, read> fx_lights: array<FxLight>;

struct FxLight {
    pos: vec3<f32>,
    intensity: f32,
    color: vec3<f32>,
    range: f32,
    dir: vec3<f32>,
    cos_outer: f32,
    cos_inner: f32,
    kind: u32,
    size: f32,
    pad: f32,
};

struct FxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) extra: vec4<f32>,
};

fn fx_clip(p: vec3<f32>) -> vec4<f32> {
    let c = skyu.vp * vec4<f32>(p, 1.0);
    return vec4<f32>(c.xy + skyu.jitter.xy * c.w, c.z, c.w);
}

fn fx_corner(vi: u32) -> vec2<f32> {
    var k = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    return k[vi % 6u];
}

/// Metres one pixel spans at view distance `z`.
fn fx_pixel_m(z: f32) -> f32 {
    return 2.0 * z * skyu.cluster.z * skyu.res.w;
}

// ---- luminaires ----

const FX_LENS_M: f32 = 0.16;
/// (lighting, `--lit-fog`) From this share of the clusters' far depth the
/// lamps' air light comes back as the lit fog's volume ends.
const FX_LIT_FOG_FADE: f32 = 0.8;
/// ... and with the eye this far (the sine of the angle) below a street
/// luminaire's plane the volume draws all of its glow toward the eye.
const FX_LIT_FOG_BELOW_SIN: f32 = 0.0872;
/// The JS lamp practical's virtual source lift over the head, in head heights
/// (`lampPracticals.sourceLiftMH`): its intensity carries the pool gain
/// (1 + lift)^2 that keeps the nadir as an unlifted source's.
const FX_JS_SOURCE_LIFT_MH: f32 = 0.57;

/// A street lamp's luminaire and the intensity its lens shows: a fallback
/// lamp (kind 0) is its own luminaire; a props lamp's (kind 3) source stands
/// `size` m over the head, with the pool gain taken back off.
fn fx_luminaire(l: FxLight) -> vec4<f32> {
    if (l.kind == 3u && l.size > 0.0) {
        let g = 1.0 + FX_JS_SOURCE_LIFT_MH;
        return vec4<f32>(l.pos - vec3<f32>(0.0, l.size, 0.0), l.intensity / (g * g));
    }
    return vec4<f32>(l.pos, l.intensity);
}

/// (`--night-photometry`; sky_lights.wgsl `lt_emit`) a street luminaire's
/// intensity toward the eye over its nadir's, carried in its record's fraction
/// (1: as before, a globe or a landmark's lamp, or the switch off).
fn fx_eye_share(l: FxLight) -> f32 {
    if (!SF_NIGHT_PHOTOMETRY) {
        return 1.0;
    }
    return 1.0 - fract(l.pad);
}

@vertex
fn sprite_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> FxOut {
    var out: FxOut;
    let l = fx_lights[ii];
    out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    // a point light (kind 2: the bridges' aviation and necklace
    // lights) shows its source too, with no pool decal.
    if ((l.kind != 0u && l.kind != 2u && l.kind != 3u) || !sky_on(skyu.flags.x, SKY_SPRITES)) {
        return out;
    }
    let lum = fx_luminaire(l);
    let head = lum.xyz;
    let rel = head - skyu.eye.xyz;
    let z = dot(rel, skyu.cam_fwd.xyz);
    if (z < 0.5) {
        return out;
    }
    if (vi >= 6u) {
        // The pool decal under a far lamp (not shaded by the clusters there):
        // a horizontal quad on the street, lit in the fragment as the lamp
        // lights an albedo-0.2 ground (`size` is the head's height; for a
        // props lamp the source's lift, the head 1 / sourceLiftMH of it).
        if ((u32(l.pad) & 1u) == 1u || l.kind == 2u) { // no pool under a point light
            return out;
        }
        let k3 = l.kind == 3u;
        let h = select(max(l.size, 3.0), max(l.size / FX_JS_SOURCE_LIFT_MH, 3.0), k3);
        let hs = select(h, h + l.size, k3);
        let reach = h * 2.4;
        let c = fx_corner(vi);
        let pool_y = l.pos.y - hs + 0.25;
        // A downward-facing horizontal road pool decal cannot be seen from below its plane:
        // culling pools above the eye eliminates overhanging 48 m-wide Bay Bridge deck discs
        // floating in mid-air on both sides of the tower pier (`06a_night_skyline`).
        if (SF_AAA_AERIAL && pool_y > skyu.eye.y + 0.5) {
            return out;
        }
        let p = vec3<f32>(head.x + c.x * reach, pool_y, head.z + c.y * reach);
        out.clip = fx_clip(p);
        out.uv = c * reach;
        out.color = vec4<f32>(l.color * l.intensity, hs);
        out.extra = vec4<f32>(-1.0, l.cos_outer, l.cos_inner, select(0.0, 1.0, k3));
        return out;
    }
    // The core never under 1.5 px (its energy kept), the halo a few metres.
    let px = fx_pixel_m(z);
    var core = max(FX_LENS_M, 1.5 * px);
    if (SF_LUMINAIRES && (u32(l.pad) & 4u) != 0u) {
        // (`--luminaires`) a lantern drawn with its panes lit: the core is its lamp seen
        // through the clear glass (`LUM_LANTERN_LAMP_M`), its energy kept, the frame round it seen
        core = max(LUM_LANTERN_LAMP_M, 1.5 * px);
    }
    let vc_rain_h = select(0.0, smoothstep(0.5, 18.0, skyu.wet.z), sky_vol_clouds_on(skyu.cirrus));
    let halo = (0.72 + 1.35 * (skyu.bloom_w2.w - 1.0)) * (1.0 + 0.22 * vc_rain_h) * max(1.0, core / FX_LENS_M);
    let size = max(core, halo);
    let c = fx_corner(vi);
    let p = head + (skyu.cam_right.xyz * c.x + skyu.cam_up.xyz * c.y) * size;
    out.clip = fx_clip(p);
    // Pull the quad toward the camera so the pole-less luminaire is not cut
    // by the facade it hangs in front of (depth only).
    let pull = fx_clip(p - normalize(rel) * min(size, 2.0));
    out.clip.z = pull.z / pull.w * out.clip.w;
    out.uv = c * size;
    // Luminance of the lens: the intensity over its projected area (`--night-photometry`: toward the eye).
    let eye = fx_eye_share(l);
    // (`--luminaires`: over a fixture drawn with its glass lit, the core is the lamp seen
    // through its clear panes or the glare of its lens, props_draw.wgsl `pr_lamp_glass`)
    let lens = lum.w * eye / (FX_LENS_M * FX_LENS_M * 3.14159) * (FX_LENS_M / core) * (FX_LENS_M / core);
    out.color = vec4<f32>(l.color * lens, core);
    // z: (lighting, `--lit-fog`) the share of the lamp's air light the lit fog's
    // volume already draws, not added twice: within the clusters' far depth, and
    // for a street luminaire (kinds 0, 3: nothing above its horizontal plane) only
    // with the eye below it (from above, the volume holds no glow toward the eye);
    // a point light's glow every way.
    var fogged = 0.0;
    if (lk_lx_on(LX_BIT_LIT_FOG)) {
        let below = select(smoothstep(0.0, FX_LIT_FOG_BELOW_SIN, (head.y - skyu.eye.y) / max(length(rel), 1e-3)), 1.0, l.kind == 2u);
        fogged = below * (1.0 - smoothstep(FX_LIT_FOG_FADE * skyu.cluster.y, skyu.cluster.y, z));
    }
    // (`--night-photometry`; eo-sky `night_photometry::far_air_light_share`) past the distance
    // where the halo stops shrinking on screen, its air light keeps single scattering's
    // 1 / distance at a fixed angle (the far lamps down a long street no longer sum to a wedge).
    var air = lum.w;
    if (SF_NIGHT_PHOTOMETRY && look.fog.z > 0.0) {
        air = air * eye * FX_LENS_M / core;
    }
    out.extra = vec4<f32>(halo, air, fogged, 0.0);
    return out;
}

@fragment
fn sprite_fs(in: FxOut) -> @location(0) vec4<f32> {
    let r = length(in.uv);
    if (in.extra.x < 0.0) {
        // Pool decal: E = I shape cos / d^2 within the cutoff, x 0.2 / pi; the
        // shape the batwing (1 + 1.5 sin^2) of a fallback luminaire, flat in
        // the JS practical's cone (extra.w 1).
        let h = in.color.w;
        let d2 = h * h + r * r;
        let ct = h / sqrt(d2);
        let shape = select(1.0 + 1.5 * (1.0 - ct * ct), 1.0, in.extra.w > 0.5) * smoothstep(in.extra.y, in.extra.z, ct);
        return vec4<f32>(in.color.rgb * shape * ct / d2 * (0.2 / 3.14159), 0.0);
    }
    let core = in.color.w;
    let c = 1.0 - smoothstep(core * 0.6, core, r);
    // Air light round the lamp: forward scattering in the haze, stronger in
    // fog and rain (lamp-haze.js weather gains), falling off as 1/r^2.
    let halo = in.extra.x;
    let vc_rain_h = select(0.0, smoothstep(0.5, 18.0, skyu.wet.z), sky_vol_clouds_on(skyu.cirrus));
    let a = skyu.bloom_w2.w * (1.0 + 0.30 * vc_rain_h) * 0.0010 * in.extra.y / (1.0 + (r * r) / (0.025 * halo * halo)) * (1.0 - smoothstep(0.45 * halo, halo, r)) * (1.0 - in.extra.z);
    let lum = in.color.rgb * c + in.color.rgb / max(lk_luma(in.color.rgb), 1e-6) * a;
    return vec4<f32>(lum, 0.0);
}

// ---- (`--look cinematic`, stage L1) the lamp glare ----
//
// A lens's starburst and a faint halo round the brightest point lights: the
// street lamps (kinds 0, 3), the bridges' point lights (2) and the headlamps
// (1) whose beam faces the eye. Its own pipeline (sky.rs `glare_pipe`, made
// only with SF_CINEMATIC compiled in, drawn only under the look with the
// bloom on); the luminaire sprites above are untouched. A light's glare
// grows with how many threshold-bright pixels its light is worth on the
// sensor: its intensity toward the eye over a pixel's area at its distance
// and the bloom's threshold (which carries the exposure): none under 2^ev0,
// the longest rays from 2^(ev0 + range) (eo-look `cinematic::grade::CINE_GLARE_*`,
// sky.rs `glare_params` in `bloom_w`, the rays' angle in `bloom.z`). The rays
// and the halo carry a fixed share of that light (held at the full-length
// light, so a near lamp's rays do not burn to white): a deterministic
// pattern fixed to the frame, as a lens's blades are; no ghosts.

/// The halo's share of the glare's light and its width (a share of the rays' length).
const FX_GLARE_HALO_SHARE: f32 = 0.5;
const FX_GLARE_HALO_SIGMA: f32 = 0.12;
/// The rays' half-width across (px).
const FX_GLARE_RAY_PX: f32 = 1.2;

@vertex
fn glare_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> FxOut {
    var out: FxOut;
    out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    let l = fx_lights[ii];
    if (!SF_CINEMATIC || skyu.bloom_w.w >= 0.0 || vi >= 6u || l.kind > 3u || !sky_on(skyu.flags.x, SKY_SPRITES)) {
        return out;
    }
    var head = l.pos;
    var inten = l.intensity;
    if (l.kind != 1u) {
        let lum = fx_luminaire(l);
        head = lum.xyz;
        inten = lum.w * fx_eye_share(l); // (`--night-photometry`: toward the eye)
    }
    let rel = head - skyu.eye.xyz;
    let z = dot(rel, skyu.cam_fwd.xyz);
    if (z < 0.5) {
        return out;
    }
    if (l.kind == 1u) {
        // a headlamp only from within its beam.
        inten = inten * smoothstep(l.cos_outer, l.cos_inner, dot(l.dir, -normalize(rel)));
    }
    let px = fx_pixel_m(z);
    let th = max(skyu.bloom.x, 1e-20);
    // the light on the sensor in threshold-bright pixels (log2), and the glare's size.
    let ev = log2(max(inten / (px * px * th), 1e-30));
    let range = -skyu.bloom_w.w;
    let g = clamp((ev - skyu.bloom_w.z) / range, 0.0, 1.0);
    if (g <= 0.0) {
        return out;
    }
    let len_px = max(skyu.bloom_w.y * g, 2.0);
    let size = len_px * px;
    let c = fx_corner(vi);
    let p = head + (skyu.cam_right.xyz * c.x + skyu.cam_up.xyz * c.y) * size;
    out.clip = fx_clip(p);
    let pull = fx_clip(p - normalize(rel) * min(size, 2.0));
    out.clip.z = pull.z / pull.w * out.clip.w;
    out.uv = c * len_px;
    // the light's sum over the sensor's pixels (radiance x pixels), held at the full length's.
    let e = min(inten / (px * px), exp2(skyu.bloom_w.z + range) * th) * skyu.bloom_w.x;
    out.color = vec4<f32>(l.color * e, len_px);
    // the rays' norm (6 half rays of (1 - r/L)^2 along, a triangle of 2 x FX_GLARE_RAY_PX across:
    // 2 L FX_GLARE_RAY_PX in all) and the halo's (a Gaussian of sigma FX_GLARE_HALO_SIGMA L).
    let sg = FX_GLARE_HALO_SIGMA * len_px;
    out.extra = vec4<f32>(cos(skyu.bloom.z), sin(skyu.bloom.z), (1.0 - FX_GLARE_HALO_SHARE) / (2.0 * len_px * FX_GLARE_RAY_PX), FX_GLARE_HALO_SHARE / (6.2831853 * sg * sg));
    return out;
}

@fragment
fn glare_fs(in: FxOut) -> @location(0) vec4<f32> {
    let len = in.color.w;
    let p = in.uv;
    let r = length(p);
    if (r >= len) {
        return vec4<f32>(0.0);
    }
    var rays = 0.0;
    for (var k = 0; k < 3; k = k + 1) {
        // three lines, 60 deg apart, from the blades' angle.
        let a = f32(k) * 1.0471976;
        let ca = cos(a);
        let sa = sin(a);
        let dir = vec2<f32>(in.extra.x * ca - in.extra.y * sa, in.extra.y * ca + in.extra.x * sa);
        let along = abs(dot(p, dir));
        let across = abs(dot(p, vec2<f32>(-dir.y, dir.x)));
        let fall = 1.0 - along / len;
        rays = rays + max(1.0 - across / FX_GLARE_RAY_PX, 0.0) * fall * fall;
    }
    let sg = FX_GLARE_HALO_SIGMA * len;
    let halo = exp(-r * r / (2.0 * sg * sg));
    return vec4<f32>(in.color.rgb * (rays * in.extra.z + halo * in.extra.w), 0.0);
}

// ---- the waterside luminaires in the sea ----
//
// Each lamp by the water (its record's pad >= 2, marked from the shore field
// when the lamps are placed) seen in the sea as wind-roughened water shows
// it: the "glitter path" (Cox & Munk 1954, the sun's glitter by the same
// facet statistics), here as the Cook-Torrance specular of a point source on
// the mean sea plane with Beckmann facets of the water slice's mean square
// slope (eo-water's reference wind), the Fresnel of water (Schlick, F0 0.02)
// and Smith masking (Walter et al. 2007). The strip drawn is where a facet
// within FX_GLITTER_K sigma of level reflects the lamp, along the eye-lamp
// line; the sea's depth test keeps it off land, piers and hulls. Lamps off
// screen are not seen in it (the gather keeps the ones in the frame).

const FX_GLITTER_K: f32 = 3.0;
// the slope variance the lamps' columns are drawn with: the sea's
// reference wind plus the metre-scale chop it leaves out (4 x), so a
// waterfront lamp a kilometre off throws its column most of the way to the eye.
const FX_GLITTER_MSS: f32 = SKY_SEA_MSS * 4.0;

/// The in-plane facet tilt (rad) the reflection of a lamp `hl` m over the sea
/// and `dist` m away needs at `x` m from under an eye `he` m over the sea.
fn fx_tilt(x: f32, dist: f32, he: f32, hl: f32) -> f32 {
    return 0.5 * (atan2(hl, max(dist - x, 1e-3)) - atan2(he, max(x, 1e-3)));
}

/// Where along the eye-lamp line the tilt reaches `k` (it grows with x).
fn fx_tilt_at(k: f32, dist: f32, he: f32, hl: f32) -> f32 {
    var lo = 0.0;
    var hi = dist;
    for (var i = 0; i < 16; i = i + 1) {
        let m = 0.5 * (lo + hi);
        if (fx_tilt(m, dist, he, hl) < k) {
            lo = m;
        } else {
            hi = m;
        }
    }
    return 0.5 * (lo + hi);
}

/// The specular radiance of the lamp (`ci` = colour x intensity, at `lp`) at
/// a point `s` of the mean sea seen from the eye.
fn fx_glitter(s: vec3<f32>, lp: vec3<f32>, ci: vec3<f32>) -> vec3<f32> {
    let ve = skyu.eye.xyz - s;
    let v = normalize(ve);
    let lv = lp - s;
    let d2 = max(dot(lv, lv), 1.0);
    let l = lv * inverseSqrt(d2);
    let h = normalize(v + l);
    let c2 = max(h.y * h.y, 1e-4);
    let m2 = FX_GLITTER_MSS;
    let dist_d = exp(-(1.0 - c2) / (c2 * m2)) / (3.14159265 * m2 * c2 * c2);
    let f = 0.02 + 0.98 * pow(1.0 - clamp(dot(v, h), 0.0, 1.0), 5.0);
    let nv = max(v.y, 1e-3);
    let nl = max(l.y, 1e-3);
    let g = fx_smith_beckmann(nv, m2) * fx_smith_beckmann(nl, m2);
    return ci * (f * dist_d * g / (4.0 * nv * d2));
}

/// Smith masking of Beckmann facets of mean square slope `m2` at cos `c`
/// from the normal (Walter et al. 2007's rational fit; it holds at the
/// grazing angles a sea is seen at, where the Schlick form falls short).
fn fx_smith_beckmann(c: f32, m2: f32) -> f32 {
    let a = c / (sqrt(m2) * sqrt(max(1.0 - c * c, 1e-8)));
    if (a >= 1.6) {
        return 1.0;
    }
    return (3.535 * a + 2.181 * a * a) / (1.0 + 2.276 * a + 2.577 * a * a);
}

@vertex
fn glitter_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> FxOut {
    var out: FxOut;
    out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    let l = fx_lights[ii];
    if (vi >= 6u || (l.kind != 0u && l.kind != 3u) || (u32(l.pad) & 2u) == 0u || !sky_on(skyu.flags.x, SKY_SPRITES)) { // (bit 1: by the water; bit 2 `--luminaires`)
        return out;
    }
    // The lens, as the luminaire sprite draws it.
    let lum = fx_luminaire(l);
    let lp = lum.xyz;
    let eye = skyu.eye.xyz;
    let he = eye.y - SKY_SEA_Y;
    let hl = lp.y - SKY_SEA_Y;
    let dxz = lp.xz - eye.xz;
    let dist = length(dxz);
    if (he < 0.3 || hl < 0.5 || dist < 2.0) {
        return out;
    }
    let sigma = sqrt(0.5 * FX_GLITTER_MSS);
    let k = atan(FX_GLITTER_K * sigma);
    let x0 = fx_tilt_at(-k, dist, he, hl);
    let x1 = fx_tilt_at(k, dist, he, hl);
    let dir = dxz / dist;
    // (sea r3: a lamp high over the water near the eye lights a broad patch, not
    // a column: its glitter fades as its strip widens past 12 m)
    let wide = FX_GLITTER_K * sqrt(0.5 * FX_GLITTER_MSS) * max(he, hl) + 0.5;
    // (and 3 x for the column: a lamp is the brightest thing on a night sea and its
    // broken column reaches the eye in every photograph of a waterfront)
    let ci = l.color * lum.w * min(1.0, 144.0 / (wide * wide)) * 3.0;
    // (sea r6: and a lamp across the water keeps its column: the eye adapts to the
    // night bay and sees the far columns where the per-point radiance has fallen
    // with the square of the range; up to 30 x from 150 m out)
    // (only for street lamps by the water: a bridge light 60 m up lights a broad patch)
    let ci_far = ci * mix(1.0, clamp(dist * dist / 22500.0, 1.0, 30.0), 1.0 - smoothstep(12.0, 20.0, hl));
    // Too faint to see at the frame's exposure: not drawn.
    let xs = fx_tilt_at(0.0, dist, he, hl);
    let peak = fx_glitter(vec3<f32>(eye.x + dir.x * xs, SKY_SEA_Y, eye.z + dir.y * xs), lp, ci_far);
    if (lk_luma(peak) * look.key.w < 1e-3) {
        return out;
    }
    // Across: the cross slope reaches k sigma within k sigma x max(he, hl) of the line.
    let w = FX_GLITTER_K * sigma * max(he, hl) + 0.5;
    let side = vec2<f32>(-dir.y, dir.x);
    let c = fx_corner(vi);
    let x = mix(x0, x1, c.y * 0.5 + 0.5);
    let q = eye.xz + dir * x + side * (c.x * w);
    // 5 cm over the sea, so the flat sea quad (`--no-water`) does not hide it.
    out.clip = fx_clip(vec3<f32>(q.x, SKY_SEA_Y + 0.05, q.y));
    out.uv = q;
    out.color = vec4<f32>(ci_far, 0.0);
    out.extra = vec4<f32>(lp, 0.0);
    return out;
}

@fragment
fn glitter_fs(in: FxOut) -> @location(0) vec4<f32> {
    let fp_g = max(length(dpdx(in.uv)), length(dpdy(in.uv)));
    let s = vec3<f32>(in.uv.x, SKY_SEA_Y, in.uv.y);
    var lum = fx_glitter(s, in.extra.xyz, in.color.rgb);
    // the column is broken by the waves into dashes across the line
    // of sight (the crests that face the lamp, 1-3 m apart, wandering), with
    // the same mean, and fades out toward the strip's sides instead of ending
    // on its edge. No per-pixel sparkle: it read as confetti.
    let lp = in.extra.xyz;
    let ex = skyu.eye.xz;
    let dir = normalize(lp.xz - ex + vec2<f32>(1e-4, 0.0));
    let rel = in.uv - ex;
    let along = dot(rel, dir);
    let across = abs(dot(rel, vec2<f32>(-dir.y, dir.x)));
    let w = FX_GLITTER_K * sqrt(0.5 * FX_GLITTER_MSS) * max(skyu.eye.y - SKY_SEA_Y, lp.y - SKY_SEA_Y) + 0.5;
    var dash: f32;
    if (SF_AAA_AERIAL) {
        // Nyquist-faded domain-warped world-space wave phases (no `abs(across)` bilateral mirror,
        // no `along * sin(...)` phase chirp, and bounded 1.4 m low-freq warp + 3rd angled crest
        // to eliminate straight diamond-grid regularity on night Bay glitter).
        let k_hi = 1.0 - smoothstep(0.45, 1.8, fp_g);
        let k_lo = 1.0 - smoothstep(1.8, 6.5, fp_g);
        let wp = in.uv + vec2<f32>(
            sin(in.uv.y * 0.13 + in.uv.x * 0.07),
            cos(in.uv.x * 0.11 - in.uv.y * 0.09),
        ) * 1.4;
        let w_rip = (0.34 * sin(dot(wp, vec2<f32>(1.15, 0.42)) + skyu.wet.w * 1.4)
            + 0.27 * sin(dot(wp, vec2<f32>(-0.78, 1.05)) - skyu.wet.w * 1.1)
            + 0.22 * sin(dot(wp, vec2<f32>(0.37, -1.21)) + skyu.wet.w * 0.92)) * k_hi
            + (0.22 * sin(dot(wp, vec2<f32>(0.24, -0.31)) + skyu.wet.w * 0.65)
            + 0.16 * sin(dot(wp, vec2<f32>(-0.29, -0.17)) - skyu.wet.w * 0.48)) * k_lo;
        dash = clamp(1.0 + 0.58 * w_rip, 0.35, 1.65);
    } else {
        let wob = sin(across * 0.9 + along * 0.13) * 1.3 + sin(across * 2.3 - along * 0.07) * 0.6;
        dash = mix(0.3, 1.7, smoothstep(-0.2, 0.6, sin(along * (2.6 + 0.8 * sin(along * 0.05 + across * 0.4)) + wob + skyu.wet.w * 1.7)));
    }
    lum = lum * dash * (1.0 - smoothstep(0.35, 1.0, across / w));
    // The range fog on the way to the eye, as the water draws its own.
    let vd = dot(s - skyu.eye.xyz, skyu.cam_fwd.xyz) * sky_path_density(s.y - skyu.eye.y);
    lum = lum * (1.0 - lk_smooth(look.fog.x, look.fog.y, vd));
    return vec4<f32>(lum, 0.0);
}

// ---- rain ----

const FX_NEAR_N: u32 = 6000u;
const FX_FAR_N: u32 = 5000u;

@vertex
fn rain_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> FxOut {
    var out: FxOut;
    out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    let rate = skyu.wet.z;
    if (rate <= 0.0 || !sky_on(skyu.flags.x, SKY_RAIN)) {
        return out;
    }
    let near = ii < FX_NEAR_N;
    let live = select(skyu.rain.y, skyu.rain.x, near);
    let h0 = sky_hash1(ii * 3u + 1u);
    if (h0 > live) {
        return out;
    }
    let h1 = sky_hash1(ii * 3u + 2u);
    let h2 = sky_hash1(ii * 3u + 3u);
    let h3 = sky_hash1(ii * 7u + 11u);
    let box = select(vec3<f32>(48.0, 26.0, 48.0), vec3<f32>(10.0, 14.0, 10.0), near);
    let ahead = select(6.0, 3.0, near);
    let f = normalize(vec3<f32>(skyu.cam_fwd.x, 0.0, skyu.cam_fwd.z) + vec3<f32>(1e-4, 0.0, 0.0));
    let center = skyu.eye.xyz + f * ahead + vec3<f32>(0.0, select(6.0, 3.0, near), 0.0);
    // Speed classes x0.8 / 1 / 1.22 and the drift with the wind.
    let cls = select(select(1.22, 1.0, h3 < 0.66), 0.8, h3 < 0.33);
    let speed = skyu.rain.z * cls;
    let t = skyu.wet.w;
    var drift = vec2<f32>(skyu.wind.x, skyu.wind.y) * 0.7 * (0.8 + 0.4 * skyu.wind.z);
    let vc_on = sky_vol_clouds_on(skyu.cirrus);
    let weather_fx = sky_weather_fx_on(skyu.cirrus) || vc_on;
    if (weather_fx) {
        let g = 1.0 + 0.24 * sin(t * 0.95 + h0 * 3.1) + 0.14 * sin(t * 2.25 + h1 * 5.7);
        drift = drift * g;
    }
    let p0 = vec3<f32>(h1, h2, h0 / max(live, 1e-4)) * box + vec3<f32>(drift.x * t, -speed * t, drift.y * t);
    let lo = center - box * 0.5;
    let world = lo + (fract((p0 - lo) / box)) * box;
    let vel = vec3<f32>(drift.x, -speed, drift.y);
    let s = h3;
    let heavy_r = smoothstep(5.0, 20.0, rate);
    let len_k = select(1.0, mix(1.08, 1.48, heavy_r), vc_on);
    let len = speed * 0.022 * len_k * (0.55 + 0.9 * s) * (0.7 + 0.6 * h1);
    let a = world;
    let b = world - normalize(vel) * len;
    let rel = a - skyu.eye.xyz;
    let z = dot(rel, skyu.cam_fwd.xyz);
    if (z < 0.3) {
        return out;
    }
    let d = length(rel);
    // Fades: the near set 1.1-2 m in, 8-11 m out; the far set 9-12 in, 34-44 out.
    var fade = smoothstep(9.0, 12.0, d) * (1.0 - smoothstep(34.0, 44.0, d));
    if (near) {
        let near_in0 = select(1.1, 1.7, vc_on);
        let near_in1 = select(2.0, 3.4, vc_on);
        fade = smoothstep(near_in0, near_in1, d) * (1.0 - smoothstep(8.0, 11.0, d));
    }
    let drizzle = 1.0 - smoothstep(0.0, 1.0, (rate - 0.5) / 2.5);
    // (`--weather-fx` / `--volumetric-clouds`) passing spatial shower-band modulation and
    // intensity-varied drop gauge: fine silver threads in light rain, dense sweeping sheets in storms.
    let shower_var = select(1.0, 0.74 + 0.38 * sin(dot(world.xz, vec2<f32>(0.024, 0.019)) - t * 0.52) * cos(dot(world.xz, vec2<f32>(-0.017, 0.029)) - t * 0.36), weather_fx);
    let gauge = select(1.0, mix(0.76, select(1.12, select(1.14, 0.92, near), vc_on), heavy_r), weather_fx);
    // Keep raindrop streaks physically gossamer-thin so they read as blurred water wisps,
    // never thick solid needles floating in front of shaded walls.
    let close_k = select(0.006, 0.0022, vc_on);
    let diam = ((0.0006 + 0.0024 * s * s) * (1.0 - 0.55 * drizzle) + close_k * (1.0 - smoothstep(1.2, 4.2, d)) * (s + 0.45)) * gauge;
    let px = fx_pixel_m(z);
    let wpx = max(diam / px, 0.85);
    let cover = min(diam / px, 1.0);
    let ca = fx_clip(a);
    let cb = fx_clip(b);
    let sa = ca.xy / ca.w;
    let sb = cb.xy / cb.w;
    var dir = sb - sa;
    let aspect = skyu.res.x * skyu.res.w;
    dir = vec2<f32>(dir.x * aspect, dir.y);
    var nrm = normalize(vec2<f32>(-dir.y, dir.x) + vec2<f32>(1e-6, 0.0));
    nrm = vec2<f32>(nrm.x / aspect, nrm.y) * wpx * skyu.res.w;
    let c = fx_corner(vi);
    let along = select(cb, ca, c.y < 0.0);
    out.clip = vec4<f32>(along.xy + nrm * c.x * along.w, along.z, along.w);
    out.uv = vec2<f32>(c.x, c.y);
    let vc_str = select(1.0, select(mix(1.00, 1.45, heavy_r), mix(0.95, 1.25, heavy_r), near), vc_on);
    let strength = skyu.rain.w * clamp(0.50 + 0.50 * sqrt(rate / 25.0), 0.0, 1.0) * (1.0 - 0.35 * drizzle) * shower_var * select(1.0, 1.0 + 0.20 * heavy_r, weather_fx) * vc_str;
    // Radiance: the scene's fog colour (the drop refracts the sky and street
    // round it), lifted by the drop gain and the lightning.
    // At night the drops are lit by the street lamps they fall past (a lit
    // road's radiance, ~10 lux over pi at the lamps' scale, x lamps on).
    let lamp_lit = vec3<f32>(1.0, 0.72, 0.42) * skyu.night.x * skyu.bloom_w2.z * (select(10.0, 12.0, vc_on) / 3.14159);
    // (`--weather-fx` / `--volumetric-clouds`) Sunshower sparkle kept subtle so drops inside
    // building shadows do not glow like solid white/peach sticks.
    let sun_shower = select(0.0, smoothstep(0.35, 0.72, skyu.beam.x) * smoothstep(0.03, 0.14, skyu.atmo.y), weather_fx);
    let sun_lit = look.key.xyz * sun_shower * (0.09 + select(0.0, 0.26, h2 < 0.14));
    let glint_drop = select(0.0, select(0.0, 0.24, h2 < 0.06 && near), weather_fx);
    let flash_tint = vec3<f32>(1.0) + vec3<f32>(2.4, 3.0, 4.4) * skyu.wind.w;
    let base_tint = (look.fog_color.xyz * select(1.1, mix(0.98, 1.14, heavy_r), vc_on) + lamp_lit + sun_lit);
    let tint = base_tint * select(vec3<f32>(1.0 + 3.0 * skyu.wind.w), flash_tint, vc_on) * (1.0 + 0.35 * (h1 - 0.5) + select(0.0, 0.28, h2 < 0.12) + glint_drop);
    let cov_floor = select(0.18, select(mix(0.11, 0.20, heavy_r), mix(0.09, 0.16, heavy_r), near), vc_on);
    let alpha_cap = select(1.0, select(0.28, 0.20, near), vc_on);
    out.color = vec4<f32>(tint, min(strength * fade * max(cover, cov_floor), alpha_cap));
    return out;
}

@fragment
fn rain_fs(in: FxOut) -> @location(0) vec4<f32> {
    let e = max(1.0 - abs(in.uv.x), 0.0);
    let vc_on = sky_vol_clouds_on(skyu.cirrus);
    let edge = select(e, e * e, vc_on);
    // Smoothly taper to zero at both tips so motion-blurred drops have silky soft ends,
    // slightly weighted toward the falling lower droplet head.
    let tip = max(1.0 - in.uv.y * in.uv.y, 0.0);
    let along = select(0.6 + 0.4 * (1.0 - abs(in.uv.y)), tip * (0.65 + 0.35 * clamp(-in.uv.y, 0.0, 1.0)), vc_on);
    let a = in.color.w * edge * along;
    return vec4<f32>(in.color.rgb * a, 0.25 * a);
}
