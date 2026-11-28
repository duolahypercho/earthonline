// the wire members (main cables, suspender ropes), appended to the
// city shader (draw.wgsl + look + landmarks_surface). Group 0 is the city's
// draw group, group 1 the look's, group 2 the LOD groups (the near/far
// switch the cull pass wrote this frame).
//
// landmark-surface.js `applyWireCoverage`: the member is a prism at the
// envelope radius; a pixel keeps the rope where its view ray passes within
// max(true radius, LM_WIRE_MIN_PIXELS / 2 of a pixel) of the axis, with the
// share of that width the rope truly covers as its opacity, lit with the
// member's own normal (perpendicular to the axis, toward the eye).

struct LmGroup {
    center: vec3<f32>,
    radius: f32,
    ready: u32,
    detailed: u32,
    kind: u32,
    level: u32,
};

@group(2) @binding(0) var<storage, read> lm_groups: array<LmGroup>;
// the atmosphere's sky-view table (sky_atmo.wgsl, the volume's `vol_at_view`)
// and a clamped linear sampler, for the members' aerial perspective (`lm_aerial`).
@group(2) @binding(1) var lm_at_view: texture_2d<f32>;
@group(2) @binding(2) var lm_at_lin: sampler;

struct LmWireOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) axis: vec3<f32>,
    @location(2) dir: vec3<f32>,
    @location(3) @interpolate(flat) radius: f32,
    @location(4) color: vec3<f32>,
    @location(5) @interpolate(flat) material: u32,
};

@vertex
fn lm_wire_vs(
    @location(0) pos: vec3<f32>,
    @location(1) radius: f32,
    @location(2) axis: vec3<f32>,
    @location(3) group: u32,
    @location(4) dir: vec3<f32>,
    @location(5) color: vec4<f32>,
    @location(6) material: u32,
) -> LmWireOut {
    var out: LmWireOut;
    out.clip = frame.view_proj * vec4<f32>(pos, 1.0);
    if (lm_groups[group].detailed == 0u) {
        // The landmark is far: its far item draws the cables.
        out.clip = vec4<f32>(0.0);
    }
    out.world = pos;
    out.axis = axis;
    out.dir = dir;
    out.radius = radius;
    out.color = srgb_decode(color.rgb);
    out.material = material;
    return out;
}

struct LmWireFrag {
    @location(0) color: vec4<f32>,
    // LM_WIRE_ROUND (landmarks.rs `ROUND_WIRES`): the depth of the point on
    // the view ray nearest the member's axis, not the envelope prism's, so a
    // railing in front hides a rope behind it; else the prism's own depth.
    @builtin(frag_depth) depth: f32,
};

@fragment
fn lm_wire_fs(in: LmWireOut) -> LmWireFrag {
    let cam = frame.cam_pos.xyz;
    // Metres per pixel at the fragment's depth: depth x |d ndc_y / d pixel| / P11
    // (the NDC derivative is 2 / the target height; P11 is the length of the
    // clip matrix's second row). Derivatives first, in uniform control flow.
    let clip = frame.view_proj * vec4<f32>(in.world, 1.0);
    let ndc_y = clip.y / clip.w;
    let dndc = abs(dpdy(ndc_y)) + abs(dpdx(ndc_y));
    let vp = frame.view_proj;
    let p11 = length(vec3<f32>(vp[0][1], vp[1][1], vp[2][1]));
    let depth = max(clip.w, 0.05);
    let mpp = depth * dndc / max(p11, 1e-6);
    let dir = normalize(in.dir);
    let to_eye = cam - in.axis;
    let ray = normalize(in.world - cam);
    let across = cross(ray, dir);
    let miss = abs(dot(to_eye, across)) / max(length(across), 1e-6);
    let drawn = min(max(in.radius, mpp * LM_WIRE_MIN_PIXELS * 0.5), LM_WIRE_ENVELOPE_M);
    if (!(in.radius > 0.0 && miss <= drawn)) {
        discard;
    }
    let opacity = clamp(in.radius / max(drawn, 1e-5), 0.0, 1.0);
    let facing = normalize(to_eye - dir * dot(to_eye, dir));
    var out: LmWireFrag;
    out.depth = in.clip.z;
    var nrm = facing;
    if (LM_WIRE_ROUND) {
        // the nearest points of the view ray and the axis (two lines).
        let b = dot(ray, dir);
        let w0 = cam - in.axis;
        let den = max(1.0 - b * b, 1e-6);
        let t = (b * dot(dir, w0) - dot(ray, w0)) / den;
        let p = cam + ray * max(t, 0.0);
        let q = in.axis + dir * dot(p - in.axis, dir);
        let pc = frame.view_proj * vec4<f32>(p, 1.0);
        out.depth = clamp(pc.z / max(pc.w, 1e-6), 0.0, 1.0);
        // a cylinder's normal where the ray crosses it, `u` of the drawn radius off the axis.
        let u = clamp(miss / max(drawn, 1e-5), 0.0, 1.0);
        let lat = p - q;
        let ll = length(lat);
        if (ll > 1e-6) {
            nrm = normalize(facing * sqrt(max(1.0 - u * u, 0.0)) + (lat / ll) * u);
        }
    }
    let m = materials[in.material];
    var s: Surface;
    s.albedo = in.color;
    s.rough = clamp(m.params.x, 0.03, 1.0);
    s.metal = m.params.y;
    s.ao = 1.0;
    s.n = nrm;
    s.alpha = opacity;
    s.env = ENV_PAINTED_METAL;
    let v = normalize(cam - in.world);
    let c = look_light(s, m, v, in.world, nrm, 0u);
    out.color = vec4<f32>(c.rgb, opacity);
    return out;
}

// ---- the wire members after the atmosphere (`BridgeFeatures::wires`) ----
//
// The members write no depth, so the sky resolve (sky_resolve.wgsl), which
// composites every pixel that holds no depth as the sky (the scene there
// times the volume's transmittance to infinity, about 0, plus its in-scatter),
// wiped every rope drawn over the sky: the cables and suspenders vanished
// against the sky at every distance. This pass draws them over the resolved
// frame instead (the frame the post reads), at every distance (the far item
// carries no cable beams then), with the unjittered camera, depth-tested
// against the scene:
//
// * the vertex stage widens each member's prism to its true radius plus two
//   pixels at the vertex's distance (a rope never falls between pixels);
// * the fragment's coverage is the share of a one-pixel tent filter centred
//   on the pixel's ray that the rope's true cross-section covers (the
//   filter's CDF at miss + r less at miss - r): a thick cable is solid with a
//   one-pixel soft edge, a thin rope a hairline whose summed coverage is its
//   true width (2 r / pixel), so it neither shimmers nor vanishes;
// * a rope wider than a pixel is shaded with its round section's normal.

fn lm_tent_cdf(u: f32, f: f32) -> f32 {
    let x = clamp(u / f, -1.0, 1.0);
    if (x < 0.0) {
        return 0.5 * (x + 1.0) * (x + 1.0);
    }
    return 1.0 - 0.5 * (1.0 - x) * (1.0 - x);
}

/// Metres per pixel at a clip w (the unjittered camera's).
fn lm_wire_mpp(w: f32) -> f32 {
    let vp = skyu.vp;
    let p11 = length(vec3<f32>(vp[0][1], vp[1][1], vp[2][1]));
    return max(w, 0.05) * 2.0 / (max(skyu.res.y, 1.0) * max(p11, 1e-6));
}

/// The coverage filter's half width for a suspender rope (`BridgeFeatures::curtain`),
/// metres across the pixel's ray at the rope: one pixel (`mpp`) where its
/// neighbours stand clear of it on screen, widening to the spacing between
/// neighbouring stations (`LM_*_SUSP_PITCH` along the span, measured along
/// `q`, the direction perpendicular to both the rope and the ray) where they
/// close to under two pixels. Tents as wide as their spacing sum to one, so a
/// receding curtain draws its true mean cover (2 r / spacing) as an even veil
/// instead of beating with the pixel grid. Between four and two pixels apart
/// the filter eases from the one to the other. A main cable, a band or a
/// socket keeps the one-pixel filter.
fn lm_curtain_filter(axis_pt: vec3<f32>, q: vec3<f32>, r: f32, mpp: f32) -> f32 {
    if (!LM_CURTAIN_ON || r > 0.06) {
        return mpp;
    }
    let gg = distance(axis_pt.xz, LM_GG_ORIGIN) < distance(axis_pt.xz, LM_BAY_ORIGIN);
    let ax = select(LM_BAY_AXIS, LM_GG_AXIS, gg);
    let pitch = select(LM_BAY_SUSP_PITCH, LM_GG_SUSP_PITCH, gg);
    let sep = pitch * abs(dot(vec3<f32>(ax.x, 0.0, ax.y), q));
    let k = sep / max(mpp, 1e-6);
    return max(mpp, sep * lm_ss(4.0, 2.0, k));
}

struct LmWire2Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) axis: vec3<f32>,
    @location(2) dir: vec3<f32>,
    @location(3) @interpolate(flat) radius: f32,
    @location(4) color: vec4<f32>,
    @location(5) @interpolate(flat) material: u32,
    // the atmosphere tables' gain toward the look's zenith (`lm_atmo_gain`:
    // the same for every vertex of a frame), read once a vertex, not twice a pixel.
    @location(6) @interpolate(flat) atmo_gain: f32,
    // (`LM_WIRE_AERIAL_VERTEX`) the aerial perspective's sky colours toward
    // the vertex's axis point (`lm_aerial_sky`), interpolated along the segment.
    @location(7) ap_colc: vec3<f32>,
    @location(8) ap_colw: vec3<f32>,
};

@vertex
fn lm_wire2_vs(
    @location(0) pos: vec3<f32>,
    @location(1) radius: f32,
    @location(2) axis: vec3<f32>,
    @location(3) group: u32,
    @location(4) dir: vec3<f32>,
    @location(5) color: vec4<f32>,
    @location(6) material: u32,
) -> LmWire2Out {
    var out: LmWire2Out;
    let vp = skyu.vp;
    let drawn0 = max(radius, LM_WIRE_ENVELOPE_M);
    let mpp = lm_wire_mpp((vp * vec4<f32>(pos, 1.0)).w);
    var fw = mpp;
    if (LM_CURTAIN_ON) {
        let q0 = cross(normalize(dir), normalize(pos - skyu.eye.xyz));
        fw = lm_curtain_filter(axis, q0 / max(length(q0), 1e-6), radius, mpp);
    }
    // The side faces have open ends. At a clamp an envelope much wider than
    // the actual wire projects its end past the adjoining loop when seen
    // obliquely. Only the filtered silhouette needs raster coverage.
    let filtered = radius + 2.0 * fw;
    var need = select(max(drawn0, filtered), filtered, LM_WIRE_JOINTS);
    // a rope whose peak coverage (2 r / pixel) is under 1/64 changes no pixel
    // by more than a level of the display: its prism closes to the axis there
    // (no fragments; per vertex, so a segment tapers where it crosses the
    // threshold). A lit LED rope stays.
    let lit = color.a < 0.5 && LM_LED_ON && skyu.night.x > 0.0;
    if (2.0 * radius < mpp / 64.0 && !lit) {
        need = 0.0;
    }
    let p = axis + (pos - axis) * (need / drawn0);
    out.clip = vp * vec4<f32>(p, 1.0);
    if (LM_WIRE_ROUND) {
        // (landmark access, landmarks.rs `ROUND_WIRES`) tested at the depth of
        // the member's axis, not of its widened prism's near face (a metre and
        // more in front of it), so a deck's railing in front of a rope hides
        // it; set per vertex (no fragment depth: the test stays early).
        let ac = vp * vec4<f32>(axis, 1.0);
        if (ac.w > 0.05 && out.clip.w > 0.05) {
            out.clip.z = min(out.clip.z, clamp(ac.z / ac.w, 0.0, 1.0) * out.clip.w);
        }
    }
    if (!(radius > 0.0)) {
        out.clip = vec4<f32>(0.0);
    }
    out.world = p;
    out.axis = axis;
    out.dir = dir;
    out.radius = radius;
    out.color = vec4<f32>(srgb_decode(color.rgb), color.a);
    out.material = material;
    out.atmo_gain = 1.0;
    out.ap_colc = vec3<f32>(0.0);
    out.ap_colw = vec3<f32>(0.0);
    if (LM_AIR_ON && skyu.flags.w == 1u && skyu.atmo2.x > 0.5) {
        out.atmo_gain = lm_atmo_gain();
        if (LM_WIRE_AERIAL_VERTEX) {
            let rel = axis - skyu.eye.xyz;
            let sky = lm_aerial_sky(rel / max(length(rel), 1e-3), skyu.eye.y, out.atmo_gain);
            out.ap_colc = sky.colc;
            out.ap_colw = sky.colw;
        }
    }
    return out;
}

/// The west span's suspender LEDs at night (`BridgeFeatures::night`, wire
/// vertices of alpha 0): luminance of the rope's strip, cd/m2, before the
/// lamps' scale. LM_LED_CD candela every LM_LED_PITCH_M along the rope is
/// LM_LED_CD / pitch candela a metre, over the rope's width 2 r; resolved
/// into dots where a pixel is under a third of the pitch. The sequence (the
/// installation plays changing patterns) is drawn as two slow waves along the
/// span and one down the ropes.
fn lm_led(p: vec3<f32>, r: f32, mpp: f32) -> f32 {
    let along = dot(p.xz - LM_BAY_ORIGIN, LM_BAY_AXIS);
    let t = skyu.eye.w;
    let w1 = 0.5 + 0.5 * sin(along * 0.021 - t * 0.35 + 1.3 * sin(along * 0.0047 + t * 0.07));
    let w2 = 0.5 + 0.5 * sin(along * 0.083 + p.y * 0.11 + t * 0.9);
    let fall = 0.5 + 0.5 * sin(p.y * 0.19 + t * 1.6 + along * 0.013);
    var pattern = 0.04 + 0.96 * pow(clamp(w1 * (0.45 + 0.55 * w2) * (0.5 + 0.5 * fall), 0.0, 1.0), 1.6);
    if (LM_FLOODS2_ON) {
        pattern = lm_led_bands(p, along, t);
    }
    let ph = fract(p.y / LM_LED_PITCH_M) - 0.5;
    let dot_w = 0.02;
    let g = exp(-(ph * LM_LED_PITCH_M) * (ph * LM_LED_PITCH_M) / (2.0 * dot_w * dot_w)) / (dot_w * 2.5066 / LM_LED_PITCH_M);
    let resolve = lm_ss(LM_LED_PITCH_M * 0.33, LM_LED_PITCH_M * 0.1, mpp);
    return LM_LED_CD / (LM_LED_PITCH_M * 2.0 * max(r, 1e-3)) * pattern * mix(1.0, g, resolve);
}

/// The installation's sequence as the eye sees it (`BridgeFeatures::floods`):
/// bands of lit nodes travelling along the span with dark ropes between
/// them (each LED is addressed: the pattern switches nodes off, it does not
/// dim a lit stick), a slower second train crossing the first, and the
/// lit share of a rope broken into runs down its length. Every rope is
/// one per suspender station; the band edges fall between stations
/// (`along` is quantised to the rope it is on). INFERRED: the band widths
/// and speeds (the artist's sequences are generated and never repeat).
fn lm_led_bands(p: vec3<f32>, along: f32, t: f32) -> f32 {
    let station = floor(along / 15.2 + 0.5);
    let a = station * 15.2;
    let b1 = fract(a / 420.0 - t * 0.021);
    let b2 = fract(a / 260.0 + t * 0.013 + 0.37);
    let band1 = lm_ss(0.0, 0.08, b1) * (1.0 - lm_ss(0.22, 0.34, b1));
    let band2 = lm_ss(0.0, 0.05, b2) * (1.0 - lm_ss(0.10, 0.18, b2));
    // down the rope: runs of lit nodes 6-10 m long with dark gaps, drifting down.
    let run = fract(p.y / 8.5 + t * 0.12 + fract(station * 0.618034) * 3.0);
    let lit_run = lm_ss(0.0, 0.06, run) * (1.0 - lm_ss(0.62, 0.7, run));
    let on = max(band1, band2 * 0.8) * mix(0.35, 1.0, lit_run);
    return clamp(on, 0.0, 1.0);
}

/// The gain that holds the atmosphere tables to the look's zenith
/// (`lm_atmo_sky`): the tables' zenith luminance against the geometric mean
/// of the look's dome and its own zenith, clamped. It depends on the frame
/// alone, so the vertex stage reads it (`LmWire2Out::atmo_gain`).
fn lm_atmo_gain() -> f32 {
    let z = textureSampleLevel(lm_at_view, lm_at_lin, at_view_uv(vec3<f32>(0.0, 1.0, 0.0), skyu.atmo.xyz), 0.0).rgb;
    let lz = dot(z, vec3<f32>(0.2126, 0.7152, 0.0722));
    let tgt = sqrt(lk_luma(lk_sky_dome(vec3<f32>(0.0, 1.0, 0.0))) * dot(look.dome_zenith.xyz, vec3<f32>(0.2126, 0.7152, 0.0722)));
    return clamp(tgt / max(lz, 1e-9), 0.2 * skyu.atmo.w, 400.0 * skyu.atmo.w);
}

/// The sky toward a direction from the atmosphere tables, as the volume
/// draws it (sky_volume.wgsl `vol_atmo_sky`: the tables held to the look's
/// zenith, the lowest 4 deg capped, green-led hues neutralised; the
/// near-sun and twilight corrections are left out: the members' in-scatter
/// looks toward the horizon, not at the sun).
fn lm_atmo_sky(d: vec3<f32>, g_look: f32) -> vec3<f32> {
    var c = textureSampleLevel(lm_at_view, lm_at_lin, at_view_uv(d, skyu.atmo.xyz), 0.0).rgb * g_look;
    if (d.y > -0.02 && d.y < 0.0698) {
        let hd4 = sqrt(max(1.0 - d.y * d.y, 1e-6));
        let d4 = vec3<f32>(d.x / hd4 * 0.99756, 0.0698, d.z / hd4 * 0.99756);
        let l4 = dot(textureSampleLevel(lm_at_view, lm_at_lin, at_view_uv(d4, skyu.atmo.xyz), 0.0).rgb * g_look, vec3<f32>(0.2126, 0.7152, 0.0722));
        let lh = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        let cap = 1.25 * l4;
        if (lh > cap) {
            c = c * mix(cap / lh, 1.0, smoothstep(0.035, 0.0698, d.y));
        }
    }
    let lc = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let gex = clamp((c.g - max(c.r, c.b)) / max(c.g, 1e-6) * 12.0, 0.0, 0.9);
    return mix(c, lc * vec3<f32>(1.08, 0.99, 0.86), gex);
}

/// The aerial perspective over a view distance, as the volume lays it on
/// every surface the resolve composites (sky_volume.wgsl `vol_aerial`, its
/// clear-air and weather-veil terms): (in-scattered radiance, transmittance).
/// The members are drawn after that composite, so without it a rope 2 km
/// off took no haze while the tower it hangs from took the volume's: the
/// rope drew darker and more saturated than its own tower.
fn lm_aerial(dir: vec3<f32>, dist: f32, eye_y: f32, g_look: f32) -> vec4<f32> {
    return lm_aerial_over(dir, dist, eye_y, lm_aerial_sky(dir, eye_y, g_look));
}

/// `lm_aerial`'s sky colours toward a view direction: the clear air's
/// in-scatter colour (`colc`) and the weather veil's (`colw`). They depend on
/// the direction and the eye's height alone; the distance terms are
/// `lm_aerial_over`'s.
struct LmAerialSky {
    colc: vec3<f32>,
    colw: vec3<f32>,
};

fn lm_aerial_sky(dir: vec3<f32>, eye_y: f32, g_look: f32) -> LmAerialSky {
    let above_top = smoothstep(skyu.cloud0.y, skyu.cloud0.y + 100.0, eye_y);
    let dh_dir = normalize(vec3<f32>(dir.x, max(abs(dir.y), 0.03), dir.z));
    // the dome and the tables toward `dh_dir`, each read once (the clear-air
    // colour and the veil's both take them).
    let dome = lk_sky_dome(dh_dir);
    var cs = vec3<f32>(0.0);
    if (skyu.atmo2.x > 0.5) {
        cs = lm_atmo_sky(dh_dir, g_look);
    }
    var col = dome;
    if (skyu.atmo2.x > 0.5) {
        col = mix(col, cs, skyu.atmo2.y);
    }
    let cw0 = mix(skyu.amb_base.xyz, skyu.amb_top.xyz, 0.5) * 0.9;
    let cl = dot(dome, vec3<f32>(0.2126, 0.7152, 0.0722));
    var colw = mix(cw0, vec3<f32>(cl) * vec3<f32>(0.98, 1.0, 1.02), 0.7) * 0.6;
    if (skyu.atmo2.x > 0.5) {
        colw = mix(colw, mix(vec3<f32>(cl), cs, skyu.atmo2.y), above_top);
    } else {
        colw = mix(colw, vec3<f32>(cl) * vec3<f32>(0.99, 1.0, 1.01), above_top);
    }
    return LmAerialSky(mix(col, colw, skyu.atmo3.x), colw);
}

/// `lm_aerial` over a view distance with its sky colours: the clear air's
/// and the veil's optical depth (the height-thinned haze, the cloud deck's
/// share of the path) and the in-scatter they lay over the member.
fn lm_aerial_over(dir: vec3<f32>, dist: f32, eye_y: f32, sky: LmAerialSky) -> vec4<f32> {
    let dh = dir.y * dist;
    let dens = sky_path_density(dh);
    let ray = AT_RAY * 0.001 * exp(-max(eye_y + 0.5 * dh, 0.0) / 8000.0);
    let tau = (ray + vec3<f32>(skyu.atmo3.y * dens)) * dist;
    let ptop = skyu.cloud0.y;
    let y1 = eye_y + dh;
    let lo_y = min(eye_y, y1);
    let hi_y = max(eye_y, y1);
    let inside = select(clamp((ptop - lo_y) / max(hi_y - lo_y, 1e-3), 0.0, 1.0), 1.0, hi_y <= ptop);
    let wv = skyu.atmo3.x * (1.0 - exp(-3.0 * dist * inside / max(skyu.atmo3.w, 1.0)));
    let t = exp(-tau);
    let tl = dot(t, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(sky.colc * (1.0 - tl) * (1.0 - wv) + sky.colw * wv, tl * (1.0 - wv));
}

/// Where a member looks up its sun shadow (`BridgeFeatures::air`): its
/// point moved toward the key light past its own footprint in the cascade
/// it falls in (twice its radius and 2.5 texels: the 3 x 3 tent of
/// `lk_shadow` over a rope sloping up toward the sun found the rope's own
/// depth in a neighbouring texel). A real occluder (a tower, the deck) is
/// metres to tens of metres off and still shadows it.
fn lm_shadow_point(p: vec3<f32>, r: f32) -> vec3<f32> {
    let vd = dot(p - look.cam_pos.xyz, look.cam_fwd.xyz);
    let sp = look.cascade_split;
    var ci = 0;
    if (vd > sp.x) {
        ci = 1;
    }
    if (vd > sp.y) {
        ci = 2;
    }
    let texel = look.cascade_bias[ci].z;
    return p + look.key_dir.xyz * (2.0 * r + 2.5 * texel);
}

/// The visible half section's mean key term, as the one normal a hairline
/// is shaded with (`BridgeFeatures::air`): a rope narrower than the pixel
/// shows the mean radiance of the half cylinder toward the eye, its normals
/// n(a) = facing cos a + q sin a for a in [-pi/2, pi/2], each weighted by the
/// width it projects (cos a): I = 1/2 x the integral of max(0, n(a).l) cos a.
/// With n.l = A cos(a - phi) (A, phi from l's components on facing and q) the
/// lit arc's integral is closed. Returned: sin a of the normal whose n.l is I
/// (the one nearer the eye). The fixed normal (0.3 of the radius toward the
/// light) drew a rope seen nearly along its length with the sun behind its
/// far flank, as a cable is from Crissy Field, unlit (its n.l about 0 where
/// the half section's mean is about 0.1).
fn lm_hairline_t(facing: vec3<f32>, q: vec3<f32>, l: vec3<f32>) -> f32 {
    let f = dot(facing, l);
    let g = dot(q, l);
    let amp = sqrt(f * f + g * g);
    if (amp < 1e-4) {
        return 0.0;
    }
    let hp = 1.5707963;
    let phi = atan2(g, f);
    let a = max(-hp, phi - hp);
    let b = min(hp, phi + hp);
    var mean = 0.0;
    if (b > a) {
        mean = 0.25 * amp * (0.5 * (sin(2.0 * b - phi) - sin(2.0 * a - phi)) + (b - a) * cos(phi));
    }
    let d = acos(clamp(mean / amp, 0.0, 1.0));
    let alpha = clamp(select(phi + d, phi - d, phi >= 0.0), -hp, hp);
    return sin(alpha);
}

@fragment
fn lm_wire2_fs(in: LmWire2Out) -> @location(0) vec4<f32> {
    let cam = skyu.eye.xyz;
    let dir = normalize(in.dir);
    let a = cam - in.axis;
    // Only the prism's half toward the eye (it covers the whole silhouette
    // once; the far half drew every pixel a second time). A rejected pixel
    // returns a transparent colour rather than discarding, so the scene depth
    // test runs before the fragment (a discarding shader takes it late, and
    // every rope behind a building was shaded in full).
    let off = in.world - in.axis;
    if (dot(off - dir * dot(off, dir), a - dir * dot(a, dir)) < 0.0) {
        return vec4<f32>(0.0); // (no discard: the depth test stays early)
    }
    let ray = normalize(in.world - cam);
    let q0 = cross(dir, ray);
    let q = q0 / max(length(q0), 1e-6);
    let miss = dot(a, q);
    let mpp = lm_wire_mpp((skyu.vp * vec4<f32>(in.world, 1.0)).w);
    let r = in.radius;
    let fw = lm_curtain_filter(in.axis, q, r, mpp);
    let cov = lm_tent_cdf(miss + r, fw) - lm_tent_cdf(miss - r, fw);
    if (!(r > 0.0) || cov < 1.0 / 1024.0) {
        return vec4<f32>(0.0); // (no discard: the depth test stays early)
    }
    let facing = normalize(a - dir * dot(a, dir));
    let on_axis = in.axis + dir * dot(in.world - in.axis, dir);
    var m = materials[in.material];
    // A wire member's material is never glass (landmarks.rs `append` asserts
    // it): with the bits known clear, look_light's glazing and clearcoat
    // branches compile out of this pipeline (the output is the same).
    m.flags = m.flags & ~(F_GLASS | F_THIN_GLASS);
    var s: Surface;
    s.albedo = in.color.rgb;
    s.rough = clamp(m.params.x, 0.03, 1.0);
    s.metal = m.params.y;
    s.ao = 1.0;
    s.alpha = 1.0;
    s.env = ENV_PAINTED_METAL;
    // The section the pixel sees: the round normal at its offset where the
    // rope is wider than the pixel's tent, else one normal standing for the
    // visible half section: 0.3 of the radius toward the key light's side
    // (the mean of the half section's Lambert term over its two flanks, within
    // a few per cent; the sunlit flank of a hairline shows).
    var t = clamp(miss / r, -1.0, 1.0);
    if (r <= mpp) {
        t = select(-0.3, 0.3, dot(q, look.key_dir.xyz) > 0.0);
        if (LM_AIR_ON) {
            t = lm_hairline_t(facing, q, look.key_dir.xyz);
        }
    }
    let n = normalize(q * t + facing * sqrt(max(1.0 - t * t, 0.0)));
    let p = on_axis + n * r;
    s.n = n;
    var lp = p;
    if (LM_AIR_ON) {
        lp = lm_shadow_point(p, r);
    }
    if (LM_FINISH_ON) {
        lm_rim_vis = lm_bridge_rim(); // (landmarks_surface.wgsl: no rig rim on a bridge at night)
    }
    var c = look_light(s, m, normalize(cam - p), lp, n, 0u).rgb;
    if (LM_FINISH_ON) {
        // the sea's light on the rope's faces turned down (landmarks_surface.wgsl `lm_bridge_bounce_e`).
        c = max(c + lm_bridge_bounce(p, n, s.albedo, 1.0), vec3<f32>(0.0));
    }
    if (in.color.a < 0.5 && LM_LED_ON) {
        c = c + vec3<f32>(lm_led(p, r, mpp) * skyu.bloom_w2.z * skyu.night.x);
    }
    if (LM_AIR_ON && skyu.flags.w == 1u && skyu.atmo2.x > 0.5) {
        // the volume's aerial perspective at the member's distance, as the resolve lays it on the scene.
        let rel = p - cam;
        let dist = length(rel);
        var ap: vec4<f32>;
        if (LM_WIRE_AERIAL_VERTEX) {
            // (eo-landmarks `BridgeFeatures::wire_sky`) the sky colours from the
            // segment's ends, interpolated; the distance terms at this pixel.
            ap = lm_aerial_over(rel / max(dist, 1e-3), dist, cam.y, LmAerialSky(in.ap_colc, in.ap_colw));
        } else {
            ap = lm_aerial(rel / max(dist, 1e-3), dist, cam.y, in.atmo_gain);
        }
        c = c * ap.a + ap.rgb;
    }
    return vec4<f32>(c, clamp(cov, 0.0, 1.0));
}
