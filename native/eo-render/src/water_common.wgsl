// what the water's cull and draw share (eo-render/src/water.rs).

struct WaterU {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // xyz camera, w water clock (s)
    cam: vec4<f32>,
    // xyz camera forward, w grid radius (m)
    fwd: vec4<f32>,
    // xy grid offset (world x, z), z sea level y, w patch box half height (m)
    grid: vec4<f32>,
    // shore field: origin x, z, size x, z (world m)
    field: vec4<f32>,
    // where the water draws (the terrain grid, as the flat sea did): min x, min z, max x, max z
    extent: vec4<f32>,
    // x: debug view (EO_WATER_DEBUG), 0 off; w the reach's extra run south (m, stage L5)
    screen: vec4<f32>,
    // x scene copy taken this frame (1/0), y probe on (1/0), z land mask (m below which the field is land), w FFT live (1/0)
    flags: vec4<f32>,
    // FFT cascade tile sizes (m)
    sizes: vec4<f32>,
    // probe capture point xyz; w beach mask (m)
    probe: vec4<f32>,
    // bay amplitude per cascade (the open Pacific is 1)
    bay: vec4<f32>,
    planes: array<vec4<f32>, 6>,
    // per cascade, the bay's and the open Pacific's amplitude (eo-water waves.rs Plan)
    wv_bay: vec4<f32>,
    wv_pac: vec4<f32>,
    // x U10 bay, y U10 Pacific (m/s), z foam cover bay, w foam cover Pacific (fractions)
    wv_sea: vec4<f32>,
    // x storm 0..1, y the sine groundswell's gain (0: the spectrum carries the swell)
    wv_misc: vec4<f32>,
};

// 1024 segments and the two-zone axis of eo-water grid.rs.
const WG_SEGMENTS: f32 = 1024.0;
const WG_PATCH_QUADS: u32 = 16u;
const WG_PATCHES: u32 = 64u;
const WG_INNER_SHARE: f32 = 0.6;
const WG_INNER_RADIUS_SHARE: f32 = 0.056818182; // 250 / 4400
const WG_PI: f32 = 3.14159265;

// (inner half width, inner slope per unit t, outer quadratic coefficient)
fn wg_zones(r: f32) -> vec3<f32> {
    let rin = r * WG_INNER_RADIUS_SHARE;
    let s0 = rin / WG_INNER_SHARE;
    let o = 1.0 - WG_INNER_SHARE;
    return vec3<f32>(rin, s0, (r - rin - s0 * o) / (o * o));
}

// Grid line i (0..SEGMENTS) at radius r (grid.rs `axis`).
fn wg_axis(i: f32, r: f32) -> f32 {
    let t = (i / WG_SEGMENTS) * 2.0 - 1.0;
    let z = wg_zones(r);
    let u = abs(t);
    var c = z.y * u;
    if (u > WG_INNER_SHARE) {
        let v = u - WG_INNER_SHARE;
        c = z.x + z.y * v + z.z * v * v;
    }
    return select(c, -c, t < 0.0);
}

// where the water may draw. The terrain grid's rectangle (where the flat
// sea was drawn) widened by 60 km to the west (the Pacific), the north (the
// Golden Gate and the bay toward Marin) and the east (the bay toward the East
// Bay shore, which has no data), so from the air the sea runs on into the haze
// instead of ending on a straight edge; the south (the peninsula's land) is
// kept, except under the cinematic look (stage L5: `screen.w`, water.rs
// `CINEMATIC_SOUTH_REACH_M`; 0 otherwise). Past its region the shore field
// reads its edge texels (open water).
fn wg_reach() -> vec4<f32> {
    return wu.extent + vec4<f32>(-60000.0, -60000.0, 60000.0, wu.screen.w);
}

// Vertex spacing at local coordinate c (grid.rs `spacing_at`).
fn wg_spacing(c: f32, r: f32) -> f32 {
    let z = wg_zones(r);
    let a = abs(c);
    var slope = z.y;
    if (a > z.x) {
        slope = sqrt(z.y * z.y + 4.0 * z.z * (a - z.x));
    }
    return slope * (2.0 / WG_SEGMENTS);
}

fn wg_smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// The open Pacific's share west of the Golden Gate (ocean-water-optics.js GOLDEN_GATE_X).
fn wg_ocean(x: f32) -> f32 {
    // widened westward to 3 km (the JS's 800 m drew a seam across the Gate from the air)
    return wg_smooth(WO_GATE_X.x, WO_GATE_X.y - 2200.0, x);
}

// Per-cascade amplitude here: the bay keeps the short wind sea and only a
// trace of the open-ocean swell (see water.rs `BAY_SWELL`).
fn wg_cascade_amp(x: f32) -> vec4<f32> {
    return mix(wu.wv_bay, wu.wv_pac, wg_ocean(x)); // each place's share of each cascade (the sea state's plan)
}

// `cascadeBandWeights`: band c fades out as the footprint grows from half its
// shortest wave to half its longest.
fn wg_band(footprint: f32) -> vec4<f32> {
    let s = wu.sizes;
    let lo = s / 128.0;
    let hi = vec4<f32>(lo.x * 8.0, s.x / 128.0, s.y / 128.0, s.z / 128.0);
    return vec4<f32>(
        1.0 - wg_smooth(lo.x, hi.x, footprint),
        1.0 - wg_smooth(lo.y, hi.y, footprint),
        1.0 - wg_smooth(lo.z, hi.z, footprint),
        1.0 - wg_smooth(lo.w, hi.w, footprint),
    );
}

// what the wave simulation hands the shading (eo-water waves.rs, water_fft.wgsl).
// The derivative layers carry each place's whitecap foam in z (open Pacific) and w (bay).

// The whitecap foam of one cascade's derivative sample at world x (the place's
// foam, 1 fresh and fading with its age), gathered into the gusts (`gust` 0..1,
// the sea detail's: whitecaps cluster where the wind is strongest, and the
// cascades' tiles stop repeating).
fn wv_foam(d: vec4<f32>, x: f32, gust: f32) -> f32 {
    return mix(d.w, d.z, wg_ocean(x)) * mix(0.45, 1.3, smoothstep(0.2, 0.75, gust));
}

// How many of cascade c's texels a footprint (m) spans, as a log2 (0 at one).
fn wv_lod(footprint: f32, c: u32) -> f32 {
    return clamp(log2(max(footprint, 1e-4) * 256.0 / wu.sizes[c]), 0.0, 8.0);
}

// The whitecap as drawn from the foam `raw` (1 fresh, fading with age), the
// fine chop under it (`fine`, its slope), a lace noise (`lace`, signed, about
// -1..1, world space) and how far the pixel reads past the foam's texels
// (`far`, 0 near .. 1 where a texel is under the pixel). Fresh foam is a
// solid bright core, torn at its rim by the chop; older foam is a lace of
// filaments along the noise's zero lines that thin out as it fades; far off
// the place's foam cover, an even whitening.
fn wv_edge(raw: f32, fine: f32, lace: f32, far: f32, x: f32) -> f32 {
    let f = clamp(raw * (1.0 + 2.5 * clamp(fine, -0.3, 0.3)), 0.0, 1.0);
    // (a graded rim: the sea draws lower foam greyer and thinner)
    let core = smoothstep(0.22, 0.56, f);
    let ridge = 1.0 - abs(lace);
    let filaments = smoothstep(1.0 - f * 1.35, 1.1 - f * 1.35, ridge) * smoothstep(0.03, 0.1, f);
    let near = max(core, filaments * 0.85);
    let cover = mix(wu.wv_sea.z, wu.wv_sea.w, wg_ocean(x));
    return mix(near, cover * 0.6 * smoothstep(0.02, 0.05, cover), far);
}

// The wind over the water here (m/s).
fn wv_u10(x: f32) -> f32 {
    return mix(wu.wv_sea.x, wu.wv_sea.y, wg_ocean(x));
}

// The short ripples' share for the wind (the shading's chop and unresolved
// roughness): glassy in a calm (0.15 at 1 m/s), 1 at the 6 m/s the chop was
// tuned for, 1.5 in a gale.
fn wv_ripple(x: f32) -> f32 {
    let u = wv_u10(x);
    return clamp(mix(0.15, 1.0, smoothstep(1.0, 6.0, u)) + 0.5 * smoothstep(8.0, 17.0, u), 0.15, 1.5);
}
