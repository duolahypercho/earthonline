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
    // x: debug view (EO_WATER_DEBUG), 0 off; y the surf strip draws; z the shading features (water.rs `FEAT_*`)
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
    // per cascade, the GPU spectrum's mean square slope (eo-water Plan `mss_gpu`)
    wv_mss: vec4<f32>,
    // (iteration 2) the exposure field's frame: origin x, z, 1 / extent x, z (eo-water exposure.rs)
    wv_expo: vec4<f32>,
    // (iteration 2) x the tidal current at the Gate (m/s, + flood), y the parts on
    // (eo-water switches.rs bits), z 1 with an exposure field, w 1 with the beaches' slopes (in its A)
    wv_gg2: vec4<f32>,
    // (iteration 2) per cascade, the wind sea's amplitude on the tidal jet (wave
    // action against the current, Longuet-Higgins and Stewart 1961)
    wv_dop: vec4<f32>,
    // (iteration 2, exposure) the fetch factors and wind ratios the bay's wind
    // sea is tabulated at (eight each), and at 8 g + b its amplitude per
    // cascade at wind node g, fetch node b (eo-water waves.rs `Plan::expo_amp`)
    wv_xn: array<vec4<f32>, 2>,
    wv_gn: array<vec4<f32>, 2>,
    wv_xa: array<vec4<f32>, 64>,
    // (iteration 3) the extra field's frame (eo-water extra.rs): origin x, z, 1 / extent x, z
    wv_extra: vec4<f32>,
};

// (iteration 3) the extra field (eo-water extra.rs, RGBA16F; a 1 x 1 zero texel
// without one) and its sampler, in group 0 of the water, underwater, spray and
// surf pipelines (water.rs `draw_bgl`): the measured sea bed and the swell over
// it (eo-water bathymetry.rs), read by `wb_set`, in the texture's upper half,
// and the obstacles' layer (extra.rs `with_obstacles`), read by `wb_obstacle`, in its lower half.
@group(0) @binding(18) var w_extra: texture_2d<f32>;
@group(0) @binding(19) var w_extra_samp: sampler;

// (iteration 3, bathymetry) the swell period the arrival time is counted in
// (eo-water bathymetry.rs `PERIOD_S`; tests/water.rs checks they agree).
const WB_PERIOD_S: f32 = 14.2;
// (iteration 3, obstacle foam) the clear-day swell's significant height (m; eo-water waves.rs `SWELL_HS[0]`, tests/water.rs checks)
const WB_HS_CLEAR: f32 = 1.8;

// (iteration 3, bathymetry) the extra field at the point being drawn (`wb_set`):
// `on` 1 inside the field (fading to 0 over its last 200 m and without one or
// with --no-bathymetry); depth below mean sea level (m, negative on land); K_r
// the refraction coefficient (the swell's height over the open sea's, before
// shoaling); t the swell's arrival time in periods (its crests are where
// time / period - t is whole); land the distance to surveyed land standing
// above the sea (m, 60 and over far from it).
struct WbAt {
    on: f32,
    depth: f32,
    kr: f32,
    t: f32,
    land: f32,
};

var<private> wb: WbAt = WbAt(0.0, 0.0, 1.0, 0.0, 60.0);

// (iteration 3) an iteration-3 water part is on (eo-water switches.rs bits, carried in the same word)
fn wg_gg3(bit: u32) -> bool {
    return wg_gg2(bit);
}

// (iteration 3) the extra field's layer `layer` (0 the measured bed, 1 the
// obstacles: eo-water extra.rs `LAYERS`, stacked down the texture) at the
// layer coordinate `uv` (0..1), held half a texel inside the layer.
fn wb_layer(uv: vec2<f32>, layer: f32) -> vec4<f32> {
    let hh = 0.5 / f32(textureDimensions(w_extra).y);
    let v = clamp(uv.y * 0.5, hh, 0.5 - hh) + 0.5 * layer;
    return textureSampleLevel(w_extra, w_extra_samp, vec2<f32>(uv.x, v), 0.0);
}

// (iteration 3, obstacle foam) the obstacles' layer at `xz`: x the distance to
// the nearest registered obstacle's rim (m, 60 and over far from any), y and z
// the tidal current's wake behind them on the flood and on the ebb (0..1);
// none outside the field.
fn wb_obstacle(xz: vec2<f32>) -> vec3<f32> {
    let uv = (xz - wu.wv_extra.xy) * wu.wv_extra.zw;
    if (wu.wv_extra.z >= 0.5 || any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return vec3<f32>(60.0, 0.0, 0.0);
    }
    return wb_layer(uv, 1.0).xyz;
}

// (iteration 3, bathymetry) read the field at `xz` into `wb`.
fn wb_set(xz: vec2<f32>) {
    wb = WbAt(0.0, 0.0, 1.0, 0.0, 60.0);
    // (a field's inverse extent is under 0.5 / m; the identity frame's is 1)
    if (wu.wv_extra.z >= 0.5 || !wg_gg3(256u)) {
        return;
    }
    let uv = (xz - wu.wv_extra.xy) * wu.wv_extra.zw;
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return;
    }
    let e = wb_layer(uv, 0.0);
    let ext = 1.0 / wu.wv_extra.zw;
    let edge = min(min(uv.x, 1.0 - uv.x) * ext.x, min(uv.y, 1.0 - uv.y) * ext.y);
    wb = WbAt(wg_smooth(0.0, 200.0, edge), e.x, e.y, e.z, e.w);
}

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
// kept. Past its region the shore field reads its edge texels (open water).
fn wg_reach() -> vec4<f32> {
    return wu.extent + vec4<f32>(-60000.0, -60000.0, 60000.0, 0.0);
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

// (iteration 2) an iteration-2 water part is on (eo-water switches.rs bits)
fn wg_gg2(bit: u32) -> bool {
    return (u32(wu.wv_gg2.y + 0.5) & bit) != 0u;
}

// (iteration 2, exposure) the exposure here, set by the water pass for the
// point it shades (water.wgsl `w_expo_set`): x the open Pacific's swell share,
// y the wind over the strait's (the lee and the Gate's channelling), z the
// fetch factor, w 1 over water. x below 0: none (the earlier ramp across the Gate).
var<private> wg_ex: vec4<f32> = vec4<f32>(-1.0, 1.0, 1.0, 1.0);

// The open Pacific's share west of the Golden Gate (ocean-water-optics.js GOLDEN_GATE_X).
fn wg_ocean_x(x: f32) -> f32 {
    // widened westward to 3 km (the JS's 800 m drew a seam across the Gate from the air)
    return wg_smooth(WO_GATE_X.x, WO_GATE_X.y - 2200.0, x);
}

// (iteration 2, Baker surf: water-2) the open coast's share for the beaches'
// surf model: the ramp across the Gate, or where the exposure field finds more
// of the Pacific's swell (Baker, Marshall's and China beaches face it inside
// the ramp), that share, unless --no-baker-surf
fn wg_ocean_surf(x: f32) -> f32 {
    if (wg_ex.x >= 0.0 && wg_gg2(128u)) {
        return max(wg_ocean_x(x), wg_ex.x);
    }
    return wg_ocean_x(x);
}

// The open Pacific's share here: the exposure field's swell share where it is
// set, else the ramp across the Gate.
fn wg_ocean(x: f32) -> f32 {
    if (wg_ex.x >= 0.0) {
        return wg_ex.x;
    }
    return wg_ocean_x(x);
}

// (exposure) the wind here over the strait's (1 without the field)
fn wg_wind_k() -> f32 {
    return select(1.0, wg_ex.y, wg_ex.x >= 0.0);
}

// (exposure) where `v` falls among eight ascending nodes (two vec4s): (the lower node's index, the fraction to the next)
fn wg_node(v: f32, a: vec4<f32>, b: vec4<f32>) -> vec2<f32> {
    let x = clamp(v, a.x, b.w);
    let n = array<f32, 8>(a.x, a.y, a.z, a.w, b.x, b.y, b.z, b.w);
    var i = 0u;
    for (var k = 1u; k < 7u; k = k + 1u) {
        if (x > n[k]) {
            i = k;
        }
    }
    return vec2<f32>(f32(i), clamp((x - n[i]) / max(n[i + 1u] - n[i], 1e-6), 0.0, 1.0));
}

// (exposure) the bay's wind sea per cascade here: the plan's tabulated at this
// place's wind ratio and fetch factor (bilinear). Each cascade by its own
// growth: a short fetch takes the long waves and keeps the ripples, which
// follow the local wind (eo-water waves.rs `Plan::expo_amp`); the plan's bay without the field.
fn wg_bay_amp() -> vec4<f32> {
    if (wg_ex.x < 0.0) {
        return wu.wv_bay;
    }
    let g = wg_node(wg_ex.y, wu.wv_gn[0], wu.wv_gn[1]);
    let b = wg_node(wg_ex.z, wu.wv_xn[0], wu.wv_xn[1]);
    let i = u32(g.x) * 8u + u32(b.x);
    let lo = mix(wu.wv_xa[i], wu.wv_xa[i + 1u], b.y);
    let hi = mix(wu.wv_xa[i + 8u], wu.wv_xa[i + 9u], b.y);
    return mix(lo, hi, g.y);
}

// (exposure) the bay's whitecap foam here over the plan's: coverage grows as
// the wind to the 3.41 (Monahan and O'Muircheartaigh 1980), and a short
// fetch's young sea breaks less: at one wind the coverage goes as the breaking
// parameter u*^2 / (nu omega_p) to the 1.09 (Zhao and Toba 2001, J. Oceanogr.
// 57, 603), and the peak frequency as the fetch to the -1/3 (waves.rs
// `WindSea::tp`), so as the fetch to the 0.36: the fetch factor (its square
// root) to the 0.72
fn wg_foam_k() -> f32 {
    return select(1.0, clamp(pow(max(wg_ex.y, 0.0), 3.41) * pow(max(wg_ex.z, 0.0), 0.72), 0.0, 3.0), wg_ex.x >= 0.0);
}

// Per-cascade amplitude here: the bay keeps the short wind sea and only a
// trace of the open-ocean swell (see water.rs `BAY_SWELL`).
fn wg_cascade_amp(x: f32) -> vec4<f32> {
    return mix(wg_bay_amp(), wu.wv_pac, wg_ocean(x)); // each place's share of each cascade (the sea state's plan)
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
    return mix(d.w * wg_foam_k(), d.z, wg_ocean(x)) * mix(0.45, 1.3, smoothstep(0.2, 0.75, gust));
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
fn wv_edge(raw: f32, fine: f32, lace: f32, far: f32, x: f32, torn: bool) -> f32 {
    let f = clamp(raw * (1.0 + 2.5 * clamp(fine, -0.3, 0.3)), 0.0, 1.0);
    // (a graded rim: the sea draws lower foam greyer and thinner)
    var core = smoothstep(0.22, 0.56, f);
    if (torn) {
        // (foam cover: a breaking crest's foam is a dense patch torn at its
        // edges into streaks and holes by the turbulence, solid only where it is
        // thick and fresh; the foam texels' bilinear shapes cut paper blobs)
        core = core * smoothstep(-0.45, 0.3, lace + 2.2 * (f - 0.62));
    }
    let ridge = 1.0 - abs(lace);
    let filaments = smoothstep(1.0 - f * 1.35, 1.1 - f * 1.35, ridge) * smoothstep(0.03, 0.1, f);
    let near = max(core, filaments * 0.85);
    let cover = mix(wu.wv_sea.z * wg_foam_k(), wu.wv_sea.w, wg_ocean(x));
    return mix(near, cover * 0.6 * smoothstep(0.02, 0.05, cover), far);
}

// The wind over the water here (m/s).
fn wv_u10(x: f32) -> f32 {
    return mix(wu.wv_sea.x * wg_wind_k(), wu.wv_sea.y, wg_ocean(x));
}

// The short ripples' share for the wind (the shading's chop and unresolved
// roughness): glassy in a calm (0.15 at 1 m/s), 1 at the 6 m/s the chop was
// tuned for, 1.5 in a gale.
fn wv_ripple(x: f32) -> f32 {
    let u = wv_u10(x);
    return clamp(mix(0.15, 1.0, smoothstep(1.0, 6.0, u)) + 0.5 * smoothstep(8.0, 17.0, u), 0.15, 1.5);
}

// a shading feature is on (water.rs `FEAT_*`: 1 the slope-variance BRDF, 2 the body's optics, 4 the foam cover)
fn wv_feat(bit: u32) -> bool {
    return (u32(wu.screen.z + 0.5) & bit) != 0u;
}
