// the surf strip (eo-render/src/surf.rs; native/README.md "Surf strip").
// Composed after water_common, the look, water.wgsl and water_surf.wgsl: the
// sea's bindings (group 0), the look (group 1), its helpers and its `w_shade`
// are used as they are; group 2 is the strip's own.
//
// The grid: columns along the waterline polyline (s), rows that move with the
// waves (each wave's phase u in [-0.5, 0.5) warped so the rows crowd at the
// crest), so a breaking face and its lip are drawn with 0.2-0.4 m rows while
// the long backs are coarse. A vertex at (s, phase) stands at the n where the
// wave train's travel time puts that phase, displaced by the wave's
// cross-section for its breaking state (the strip's tables), plus the sea's
// own surface (FFT, swell, the faded analytic surf), which is all that is
// left at the strip's outer ring.

struct SurfU {
    // x: rows' crest warp, y: the storm share, z: weather height factor, w: debug view (EO_SURF_DEBUG)
    misc: vec4<f32>,
};

// a: samples offset, count, along offset, count; b: waves offset, n-of-tau offset, count, beach weight offset;
// c: n0, period, h0, hmax; d: n_max, pacific, rows per wave, waves; e: tau at n = 0, tau max, -, -;
// f: axis offset, columns, rows, -
struct StripInfo {
    a: vec4<u32>,
    b: vec4<u32>,
    c: vec4<f32>,
    d: vec4<f32>,
    e: vec4<f32>,
    f: vec4<u32>,
};

@group(2) @binding(0) var<uniform> su: SurfU;
@group(2) @binding(1) var<storage, read> s_info: array<StripInfo>;
@group(2) @binding(2) var<storage, read> s_vec: array<vec4<f32>>;
@group(2) @binding(3) var<storage, read> s_f: array<f32>;
@group(2) @binding(4) var<storage, read> s_axis: array<f32>;

const SP_SAMPLE: f32 = 4.0;
const SP_LEVELS: u32 = 16u;
const SP_TAU_STEP: f32 = 0.05;
const SP_TAU2: f32 = 6.2831853;

// The polyline at s: (x, z, normal x, normal z).
fn sp_frame(k: u32, s: f32) -> vec4<f32> {
    let a = s_info[k].a;
    let f = clamp(s / SP_SAMPLE, 0.0, f32(a.y - 1u));
    let i = min(u32(f), a.y - 2u);
    let t = f - f32(i);
    let p = mix(s_vec[a.x + i], s_vec[a.x + i + 1u], t);
    return vec4<f32>(p.xy, normalize(p.zw + vec2<f32>(1e-6, 0.0)));
}

fn sp_beach_w(k: u32, s: f32) -> f32 {
    let a = s_info[k].a;
    let f = clamp(s / SP_SAMPLE, 0.0, f32(a.y - 1u));
    let i = min(u32(f), a.y - 2u);
    let b = s_info[k].b.w;
    return mix(s_f[b + i], s_f[b + i + 1u], f - f32(i));
}

// The tables along n: (depth, travel time, phase speed, bed slope).
fn sp_along(k: u32, n: f32) -> vec4<f32> {
    let info = s_info[k];
    let f = clamp(n - info.c.x, 0.0, f32(info.a.w - 1u));
    let i = min(u32(f), info.a.w - 2u);
    return mix(s_vec[info.a.z + i], s_vec[info.a.z + i + 1u], f - f32(i));
}

fn sp_n_of_tau(k: u32, tau: f32) -> f32 {
    let b = s_info[k].b;
    let f = clamp(tau / SP_TAU_STEP, 0.0, f32(b.z - 1u));
    let i = min(u32(f), b.z - 2u);
    return mix(s_f[b.y + i], s_f[b.y + i + 1u], f - f32(i));
}

// The break table at n for an incoming height. The breaking state is not a
// switch: `brk` is the share of the four neighbouring entries that are
// breaking (so a section's end fades over the table's step along the shore),
// and `age` is the mean of the breaking entries' ages.
struct Wave {
    h: f32,
    age: f32,
    inten: f32,
    plunge: f32,
    brk: f32,
};

fn sp_wave(k: u32, n: f32, hin: f32) -> Wave {
    let info = s_info[k];
    let f = clamp(n - info.c.x, 0.0, f32(info.a.w - 1u));
    let i = min(u32(f), info.a.w - 2u);
    let fl = clamp(hin / info.c.w, 0.0, 1.0) * f32(SP_LEVELS - 1u);
    let l = min(u32(fl), SP_LEVELS - 2u);
    let o = info.b.x;
    let ws = array<vec4<f32>, 4>(s_vec[o + i * SP_LEVELS + l], s_vec[o + i * SP_LEVELS + l + 1u], s_vec[o + (i + 1u) * SP_LEVELS + l], s_vec[o + (i + 1u) * SP_LEVELS + l + 1u]);
    let tl = fl - f32(l);
    let tn = f - f32(i);
    let wt = array<f32, 4>((1.0 - tl) * (1.0 - tn), tl * (1.0 - tn), (1.0 - tl) * tn, tl * tn);
    var r: Wave;
    var age = 0.0;
    for (var j = 0; j < 4; j = j + 1) {
        let e = ws[j];
        r.h = r.h + e.x * wt[j];
        r.inten = r.inten + e.z * wt[j];
        r.plunge = r.plunge + e.w * wt[j];
        let b = select(0.0, 1.0, e.y >= 0.0);
        r.brk = r.brk + b * wt[j];
        age = age + max(e.y, 0.0) * b * wt[j];
    }
    r.age = select(-1.0, age / max(r.brk, 1e-4), r.brk > 0.0);
    return r;
}

fn sp_hash(x: f32) -> f32 {
    return fract(sin(x * 127.1 + 311.7) * 43758.5453);
}

// The weather's share of a storm.
fn sp_storm() -> f32 {
    return w_storm();
}

// The incoming height of wave m at s: sets of about seven, peaks and
// shoulders along the shore, lower in the rip gaps, faded past the beach.
// (scaled by the swell's significant height, su.misc.z = Hs / 1.8 m, the clear
// day's, from the sea's plan: a storm's 3 m swell is 1.67 x the clear day's; it was
// a fixed 2.1 x in a storm)
fn sp_hin(k: u32, m: f32, s: f32) -> f32 {
    let info = s_info[k];
    let sets = 0.62 + 0.6 * pow(0.5 + 0.5 * sin(m * 0.8976 + 1.3), 1.5) + 0.2 * (sp_hash(m * 1.37 + 0.2) - 0.5);
    let along = 1.0 + 0.22 * sin(s * 0.0150 + m * 1.7) + 0.12 * sin(s * 0.0419 + m * 4.1) + 0.06 * sin(s * 0.11 + m * 2.9);
    let rip = 1.0 - 0.35 * wg_smooth(0.72, 0.95, sin(s * 0.0203 + 0.7 * sin(s * 0.0103)));
    return info.c.z * su.misc.z * sets * max(along, 0.4) * rip * sp_beach_w(k, s);
}

// Where along the shore the bars stand nearer or further (m, Pacific).
fn sp_bar_shift(k: u32, s: f32) -> f32 {
    return (16.0 * sin(s * 0.0089 + 0.6) + 8.0 * sin(s * 0.023 + 2.0)) * s_info[k].d.y;
}

// the tables' n for a point at n: the bars moved along the shore, the beach
// face where it is (the shift ran out the whole table, so where the bars stood
// nearer the inner surf read as dry land and nothing broke there)
fn sp_table_n(k: u32, s: f32, n: f32) -> f32 {
    return n + sp_bar_shift(k, s) * wg_smooth(30.0, 90.0, n);
}

// The phase offset along the shore (waves); on the Pacific the strip's
// waves continue the sea's lead swell mode where they enter the strip (the
// waves worker's 315 m, 14.2 s mode from (0.9701, 0.2425), h = a sin(k.x - w t),
// no meander; it was `w_swell`'s 210 m train), with a third of its obliquity,
// plus sections that break early or late.
fn sp_delta(k: u32, s: f32) -> f32 {
    let info = s_info[k];
    var d = (0.09 * sin(s * 0.0273 + 1.1) + 0.05 * sin(s * 0.0698 + 2.3) + 0.03 * sin(s * 0.17 + 0.4)) * (1.0 + 1.5 * sp_storm());
    if (info.d.y > 0.5) {
        let fr = sp_frame(k, s);
        let pb = fr.xy + fr.zw * info.d.x;
        let f0 = sp_frame(k, 0.0);
        let p0 = f0.xy + f0.zw * info.d.x;
        let dir = vec2<f32>(0.9701, 0.2425);
        let a = dot(pb, dir) * (SP_TAU2 / 315.0);
        let a0 = dot(p0, dir) * (SP_TAU2 / 315.0);
        d = d + 0.25 - (a0 + 0.3 * (a - a0)) / SP_TAU2;
    }
    return d;
}

struct Crest {
    h_wave: f32,
    depth: f32,
    lam: f32,
    age: f32,
    inten: f32,
    plunge: f32,
    brk: f32,
    e: f32,
    n: f32,
};

fn sp_crest(k: u32, m: f32, s: f32, t: f32, dl: f32) -> Crest {
    let info = s_info[k];
    let tau_c = t - (m - dl) * info.c.y;
    let nc = sp_n_of_tau(k, tau_c);
    let al = sp_along(k, nc);
    let tn = sp_table_n(k, s, nc);
    let wv = sp_wave(k, tn, sp_hin(k, m, s));
    var c: Crest;
    c.h_wave = wv.h;
    c.depth = max(al.x, 0.05);
    c.lam = max(al.z * info.c.y, 4.0);
    // the energy a breaking wave loses (the table's decay toward the bore) leaves the
    // crest only once the lip has landed: through the throw the crest stands at the
    // height it broke at (0.78 of the depth where it began to break)
    if (wv.brk > 0.0) {
        let a = wv.age / c.lam;
        let h_on = sp_along(k, tn + wv.age).x;
        let hold = wv.brk * (1.0 - wg_smooth(0.22, 0.5, a));
        c.h_wave = max(wv.h, mix(wv.h, 0.78 * h_on, hold));
    }
    c.age = wv.age;
    c.inten = wv.inten;
    // (sections along the crest plunge where the bar's face is steeper: about a quarter of the
    // crest length of the larger waves throws a lip; the rest spills)
    let sect = 0.5 + 0.5 * sin(s * 0.017 + m * 2.3 + 0.8 * sin(s * 0.0061 + m));
    c.plunge = max(wv.plunge, 0.85 * wg_smooth(0.62, 0.9, sect) * wg_smooth(0.9, 1.8, wv.h));
    c.brk = wv.brk;
    c.e = wv.h / (0.78 * c.depth);
    c.n = nc;
    return c;
}

// The cross-section at phase u (u < 0 in front of the crest, shoreward):
// (shoreward displacement m, height over the still sea m). A shoaling wave in
// shallow water is a sharp crest over a long flat trough (most of its height
// above the still level, as a cnoidal wave), and the crest rises and leans as
// it nears breaking; then the lip is thrown and the wave becomes a bore.
fn sp_profile(u: f32, c: Crest) -> vec2<f32> {
    let hw = c.h_wave;
    if (hw < 0.005) {
        return vec2<f32>(0.0);
    }
    let e = clamp(c.e, 0.0, 1.0);
    let p = mix(1.3, 4.8, e * e);
    let skew = 0.55 * wg_smooth(0.45, 1.0, e);
    let us = u - skew * (1.0 - cos(SP_TAU2 * u)) / SP_TAU2;
    let cc = max((cos(SP_TAU2 * us) + 1.0) * 0.5, 0.0);
    let mean = 1.0 / sqrt((p + 0.25) * 3.14159265);
    let peak = 1.0 + 0.3 * wg_smooth(0.7, 1.0, e);
    var y = hw * (pow(cc, p) * peak - mean);
    var dx = 0.32 * hw * wg_smooth(0.7, 1.0, e) * exp(-pow(u / 0.06, 2.0));
    if (c.brk > 0.0) {
        let a = c.age / c.lam;
        let pl = c.plunge;
        // the lip: the crest rows thrown forward over the face, then falling;
        // a spilling crest only bulges and tumbles
        let life = wg_smooth(0.0, 0.03, a) * (1.0 - wg_smooth(0.16, 0.3, a));
        let reach = hw * (0.2 + mix(0.25, 2.4, pl) * min(a, 0.16));
        let k_lip = exp(-pow((u - 0.004) / 0.011, 2.0));
        let tip = exp(-pow((u - 0.002) / 0.006, 2.0));
        let fall = hw * pl * wg_smooth(0.05, 0.2, a) * 0.95;
        var yb = y - fall * tip * life + hw * 0.12 * k_lip * life * (1.0 - pl);
        var dxb = dx + reach * k_lip * life;
        // then a bore: a steep turbulent front, a long falling back
        let bore = wg_smooth(0.14, 0.42, a);
        let front = wg_smooth(-0.035, 0.0, u);
        let back = exp(-max(u, 0.0) * 5.5);
        yb = mix(yb, hw * (select(back, front, u < 0.0) - 0.22), bore);
        dxb = dxb * (1.0 - bore);
        y = mix(y, yb, c.brk);
        dx = mix(dx, dxb, c.brk);
    }
    // a trough never sinks to the sand under it (the drawn bed is at most 2.05 m down)
    return vec2<f32>(dx, max(y, -0.8 * min(c.depth, 2.05)));
}

// The run-up of the swash from wave m at s: the height (m over the still
// sea) its sheet reaches on the sand, cusped along the shore.
fn sp_runup(k: u32, m: f32, s: f32) -> f32 {
    let hin = sp_hin(k, m, s);
    let cusp = 1.0 + 0.15 * sin(s * 0.19 + m * 2.3) + 0.08 * sin(s * 0.53 + m * 1.1);
    return min(0.2 * pow(max(hin, 0.0), 0.8), 0.45) * cusp;
}

// The swash: (the level the last bore's sheet has reached, the phase since
// that bore reached the waterline, how far it has drained 0..1, its run-up).
// It rises fast and drains over the rest of the period.
fn sp_swash(k: u32, s: f32, t: f32, dl: f32) -> vec4<f32> {
    let info = s_info[k];
    let phi0 = (t - info.e.x) / info.c.y + dl;
    let m0 = floor(phi0);
    let us = phi0 - m0;
    let r = sp_runup(k, m0, s);
    let rise = wg_smooth(0.0, 0.17, us);
    let drain = wg_smooth(0.17, 0.97, us);
    return vec4<f32>(r * rise * pow(1.0 - drain, 2.0), us, drain, r);
}

// How recently the sand at signed shore distance x (m, < 0 on the sand) was
// under a swash sheet: 1 while covered, falling with an 18 s e-fold after
// the sheet drained off it (glossy, then damp), over the last four swashes.
fn sp_wet(k: u32, s: f32, x: f32, t: f32, dl: f32) -> f32 {
    if (x >= 0.0) {
        return 1.0;
    }
    let info = s_info[k];
    let phi0 = (t - info.e.x) / info.c.y + dl;
    let m0 = floor(phi0);
    let us = phi0 - m0;
    let xh = -x / 55.0;
    var age = 1e6;
    for (var j = 0; j < 4; j = j + 1) {
        let r = sp_runup(k, m0 - f32(j), s);
        if (r > xh) {
            // the drain phase at which the sheet's edge passed back over x
            let q = pow(xh / r, 0.5);
            let ux = 0.17 + 0.8 * (1.0 - q);
            let a = (us + f32(j) - ux) * info.c.y;
            if (j == 0 && us < ux) {
                age = 0.0;
            } else {
                age = min(age, max(a, 0.0));
            }
        }
    }
    return exp(-age / 18.0);
}

struct SurfOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) base: vec2<f32>,
    // s, n of the undisplaced point (z the column's phase offset, sp_delta, once per vertex)
    @location(2) sn: vec3<f32>,
    @location(3) phi: f32,
    @location(4) @interpolate(flat) strip: u32,
    // the strip surface's slope (d height / d world x, z)
    @location(5) slope: vec2<f32>,
    // x: height of the surf over the still sea, y: the swash sheet's thickness over the sand (0: the wet glaze)
    @location(6) lift: vec2<f32>,
    // the atmosphere table's gain (water.wgsl `w_atmo_gain`)
    @location(7) @interpolate(flat) atmo_g: f32,
};

// The sea's displacement at an undisplaced point (water.wgsl `vs`, the same
// formulas): the FFT ramped in off the shore, the analytic surf (faded out
// inside the strip) (no groundswell sines: the lead swell mode is in the cascades).
fn sp_sea_disp(base: vec2<f32>, f: vec2<f32>, cov: f32) -> vec3<f32> {
    let r = wu.fwd.w;
    let local = base - wu.grid.xy;
    let spacing = max(wg_spacing(local.x, r), wg_spacing(local.y, r));
    var disp = vec3<f32>(0.0);
    if (wu.flags.w > 0.5) {
        let ramp = wg_smooth(50.0, 220.0, f.x) * (1.0 - 0.6 * f.y) + 0.25 * f.y * wg_smooth(0.0, 60.0, f.x);
        if (ramp > 0.0) {
            let w = wg_band(spacing) * wg_cascade_amp(base.x);
            for (var c = 0u; c < 4u; c = c + 1u) {
                if (w[c] > 0.0) {
                    disp = disp + textureSampleLevel(w_disp, w_repeat, w_casc_uv(base, c), c, 0.0).xyz * w[c];
                }
            }
            // (the cascades carry the wind and the swell themselves: no storm factor, as the sea's vertex)
            disp = disp * ramp * (1.0 - 0.7 * cov);
        }
    }
    let surf_vw = 1.0 - wg_smooth(1.0, 2.5, spacing);
    if (cov < 0.999 && f.y > 0.01 && f.x > -2.0 && f.x < 420.0 && surf_vw > 0.0) {
        let fr = w_shore_frame(base);
        let s = surf_at(wu.cam.w, f.x, fr.z, f.y, sf_amp_scale(base.x));
        disp = disp + vec3<f32>(fr.x * s.shoreward * 0.5, s.height, fr.y * s.shoreward * 0.5) * surf_vw;
    }
    // (the groundswell's sines are retired (`w_swell` is 0): the lead mode is in the cascades)
    return disp;
}

// The bore's roller: the tumbling whitewater riding its front, (height m,
// d height / d phase). (a lumpy mass 0.45-0.8 of the bore's height, steep
// on its shoreward front and longer behind, so from the sand it stands as a
// thick roller with a shadowed front, its top torn along the crest.)
fn sp_roll(s: f32, n: f32, u: f32, cr: Crest, t: f32) -> vec2<f32> {
    if (cr.brk <= 0.0) {
        return vec2<f32>(0.0);
    }
    let a = cr.age / cr.lam;
    let g = cr.brk * wg_smooth(0.08, 0.3, a);
    let du = u + 0.008;
    let w = select(0.03, 0.011, du < 0.0);
    let e = exp(-(du / w) * (du / w));
    let q = vec3<f32>(s * 1.1, n * 1.1, t * 0.9);
    let tumble = mx_noise(q) * 0.6 + mx_noise(q * 2.4 + vec3<f32>(3.1, 1.7, 0.0)) * 0.45;
    let amp = g * cr.h_wave * clamp(0.5 + 0.6 * tumble, 0.12, 1.0);
    return vec2<f32>(amp * e, amp * e * (-2.0 * du / (w * w)));
}

struct SpPoint {
    base: vec2<f32>,
    nrm: vec2<f32>,
    n: f32,
    // the shore field there (signed distance, beach)
    f: vec2<f32>,
    dx: f32,
    y: f32,
    // the swash sheet's thickness over the drawn sand (m)
    film: f32,
};

// The strip surface at (s, phase): undisplaced point, the surf's own displacement.
// Near the shore the bores give way to the swash: a thin sheet over the drawn
// sand (the foreshore 1:55 through the still waterline, eo-water
// `beach_foreshore`, read on the field's signed distance as the ground is),
// thickest behind its edge and thinning as it drains; past its edge, where
// the sand is still wet, a glaze 4 mm over the sand; dry sand hides it.
// `roll`: the roller's height there (sp_roll, r4: evaluated once per vertex by the caller)
fn sp_point(k: u32, s: f32, phi: f32, u: f32, cr: Crest, sw: vec4<f32>, wet_on: f32, dl: f32, roll: f32) -> SpPoint {
    let info = s_info[k];
    let t = wu.cam.w;
    let n = sp_n_of_tau(k, t - (phi - dl) * info.c.y);
    let fr = sp_frame(k, s);
    var o: SpPoint;
    o.base = fr.xy + fr.zw * n;
    o.nrm = fr.zw;
    o.n = n;
    o.f = w_field_at(o.base);
    let x = o.f.x;
    let pr = sp_profile(u, cr);
    let sand = -x / 55.0;
    // the bores die out at the waterline; their troughs never sink to the drawn sand (the
    // waves feel a steeper beach face than the drawn 1:55, a shorebreak)
    var yw = max(pr.y, -0.85 * max(x, 0.0) / 55.0) * wg_smooth(-2.0, 10.0, x);
    // the bore's roller riding its front (sp_roll)
    yw = yw + roll * wg_smooth(-2.0, 10.0, x);
    // the sheet: its edge where the sand stands at the swash's level
    // (lobed and cusped along the shore, not a ruled line)
    let lobes = 2.2 * sin(s * 0.23 + sw.y * 2.0) + 1.3 * sin(s * 0.61 + 1.7) + 0.8 * sin(s * 1.37 + 0.4);
    let edge = -max(sw.x, 0.0) * 55.0 + lobes * wg_smooth(0.0, 0.05, sw.x);
    let thick = clamp((x - edge) * 0.012, 0.0, 0.09) * (1.0 - 0.5 * sw.z);
    var sheet = sand + thick;
    o.film = thick;
    if (thick <= 0.0005) {
        // past the sheet's edge: the wet glaze, or under the sand where it is dry
        let wet = select(0.0, sp_wet(k, s, x, t, dl), wet_on > 0.5);
        sheet = select(sand - 0.05, sand + 0.004, wet > 0.03);
        o.film = 0.0;
    }
    let shore = wg_smooth(14.0, 3.0, x);
    o.dx = pr.x * (1.0 - shore);
    o.y = mix(yw, max(yw, sheet), shore);
    return o;
}

@vertex
fn vs_surf(@builtin(vertex_index) vi: u32, @builtin(instance_index) k: u32) -> SurfOut {
    let info = s_info[k];
    let cols = info.f.y;
    let i = vi % (cols + 1u);
    let r = vi / (cols + 1u);
    let s = s_axis[info.f.x + min(i, cols)];
    let rows_per = info.d.z;
    let period = info.c.y;
    let t = wu.cam.w;
    let dl = sp_delta(k, s);
    // the rows' phase: continuous along the shore (z0 is not rounded), crowded
    // at whole phases (the crests) by the warp, from past the outer edge (the
    // newest wave) to past the land edge (the oldest)
    let z = t / period + dl + 1.5 - info.d.w + f32(r) / rows_per;
    let phi = z - su.misc.x / SP_TAU2 * sin(SP_TAU2 * z);
    let m = floor(phi + 0.5);
    let u = phi - m;
    // (the rows on the sand past the land fade (45 m up the beach: the strip is faded out
    // there, and the highest run-up's glaze ends near 25 m) lie under the sand: their work is skipped)
    let n_here = sp_n_of_tau(k, t - (phi - dl) * period);
    if (n_here < -45.0) {
        let fh = sp_frame(k, s);
        let bh = fh.xy + fh.zw * n_here;
        var oh: SurfOut;
        let wh = vec3<f32>(bh.x, wu.grid.z - 0.5, bh.y);
        oh.clip = wu.view_proj * vec4<f32>(wh, 1.0);
        oh.world = wh;
        oh.base = bh;
        oh.sn = vec3<f32>(s, n_here, dl);
        oh.phi = phi;
        oh.strip = k;
        oh.slope = vec2<f32>(0.0);
        oh.lift = vec2<f32>(0.0);
        oh.atmo_g = w_atmo_gain();
        return oh;
    }
    let cr = sp_crest(k, m, s, t, dl);
    let sw = sp_swash(k, s, t, dl);
    let rl = sp_roll(s, n_here, u, cr, t);
    let p = sp_point(k, s, phi, u, cr, sw, 1.0, dl, rl.x);
    // the cross-section's slope from its neighbours in phase, the profile alone (the lip's underside faces down: held)
    let e = 0.0012;
    let pra = sp_profile(u - e, cr);
    let prb = sp_profile(u + e, cr);
    let na = sp_n_of_tau(k, t - (phi - e - dl) * period);
    let nb = sp_n_of_tau(k, t - (phi + e - dl) * period);
    let dn = (nb - prb.x) - (na - pra.x);
    // (+ the roller's own slope, so its front faces the shore and shades)
    let dy = (prb.y - pra.y + rl.y * 2.0 * e) * wg_smooth(-2.0, 10.0, p.f.x);
    let slope_n = clamp(dy / select(max(dn, 1e-3), min(dn, -1e-3), dn < 0.0), -4.0, 4.0);
    let cov = surf_cover(p.base, p.f);
    let sea = sp_sea_disp(p.base, p.f, cov);
    let horiz = p.base + sea.xz - p.nrm * (p.dx * cov);
    let world = vec3<f32>(horiz.x, wu.grid.z + sea.y + p.y * cov, horiz.y);
    var o: SurfOut;
    o.clip = wu.view_proj * vec4<f32>(world, 1.0);
    o.world = world;
    o.base = p.base;
    o.sn = vec3<f32>(s, p.n, dl);
    o.phi = phi;
    o.strip = k;
    // d height / d world: along the offshore normal (the slope along the shore is left to the sea's normals)
    o.slope = p.nrm * slope_n * cov * (1.0 - wg_smooth(6.0, 1.0, p.f.x));
    o.lift = vec2<f32>(p.y * cov, select(p.film, 1.0, p.f.x > 14.0));
    o.atmo_g = w_atmo_gain();
    return o;
}

// A signed noise from the sea detail texture (water.wgsl `w_detail`, a tiling
// fbm, four channels): far cheaper per pixel than the 3D Perlin; fragment only.
fn sp_tex(q: vec2<f32>, c: u32) -> f32 {
    return (textureSampleLevel(w_detail, w_repeat, q, 0.0)[c] - 0.5) * 2.4;
}

/// the surf zone's churned foam: patches of 10-25 m broken into streaks
// along the shore and torn flecks of a metre or two, domain-warped; no ridged
// octaves (the old web of filaments read as a cell lattice). Remapped to
// about uniform 0..1, so a coverage c whitens about a share c of the water.
// `q` in metres (along the shore, offshore).
const SP_CHURN_SPREAD: f32 = 0.42;

fn sp_churn(q: vec2<f32>, footprint: f32) -> f32 {
    // the streaks and the flecks fade where a pixel holds them (no shimmer far out)
    let mid_res = 1.0 - wg_smooth(1.0, 4.0, footprint);
    let fine_res = 1.0 - wg_smooth(0.2, 0.7, footprint);
    let w = vec2<f32>(sp_tex(q * vec2<f32>(0.03, 0.06) + vec2<f32>(0.11, 0.53), 2u), sp_tex(q * vec2<f32>(0.03, 0.06) + vec2<f32>(0.67, 0.19), 3u)) * vec2<f32>(1.1, 0.5) * mid_res;
    let p = q + w;
    let big = sp_tex(p * vec2<f32>(0.012, 0.02) + vec2<f32>(0.21, 0.63), 2u);
    let streak = sp_tex(p * vec2<f32>(0.01, 0.06) + vec2<f32>(0.13, 0.29), 3u) * mid_res;
    let r = vec2<f32>(p.x * 0.92 - p.y * 0.38, p.x * 0.38 + p.y * 0.92);
    let mid = sp_tex(r * 0.05 + vec2<f32>(0.5, 0.1), 0u) * mid_res;
    let fine = sp_tex(r * 0.16 + vec2<f32>(0.29, 0.83), 1u) * fine_res;
    let v = big * 0.3 + streak * 0.3 + mid * 0.45 + fine * 0.35;
    let spread = SP_CHURN_SPREAD * sqrt((0.09 + (0.09 + 0.2025) * mid_res * mid_res + 0.1225 * fine_res * fine_res) / 0.5);
    return wg_smooth(-spread, spread, v);
}

struct SpFoam {
    // visible foam (0..1) and how fresh it is
    cover: f32,
    fresh: f32,
    face: f32,
    bored: f32,
    // the churned, aerated water under and between the foam (0..1)
    aer: f32,
    // the roller front's own shade (1: open) and the whitewater's lumps (-1..1)
    ao: f32,
    lump: f32,
    // how thick the foam is (0 its thin edges .. 1 a patch's heart)
    thick: f32,
    // the fizz (-1..1)
    speck: f32,
    // the churn texture (EO_SURF_DEBUG=3)
    pat: f32,
    // the water just outside a foam edge (shaded)
    rim: f32,
    // EO_SURF_DEBUG=3 blue (the backwash share)
    dbg: f32,
    crest: Crest,
};

// the whitewater of a dissipative surf zone. Every bore carries a solid
// roller on its front (torn front and back edges, moving with it); behind it a
// wake of whitewater thins over about 6 s, and what each of the last four
// bores left lasts 20-40 s as residual foam in patches and streaks, so between
// the bores 40-60 % of the water stays white. Under and between the foam the
// water is churned and aerated (bubbles and stirred sand) for as long as the
// bores keep breaking there: no glassy flat inside the break. A storm whitens
// most of the zone.
fn sp_foam(k: u32, s: f32, n: f32, x: f32, phi: f32, lift: f32, footprint: f32, dl: f32) -> SpFoam {
    let info = s_info[k];
    let t = wu.cam.w;
    let period = info.c.y;
    let storm = sp_storm();
    let m = floor(phi + 0.5);
    let u = phi - m;
    let cr = sp_crest(k, m, s, t, dl);
    var o: SpFoam;
    o.crest = cr;
    o.ao = 1.0;
    // metres behind the crest along the wave (< 0: in front of it, shoreward)
    let ul = u * cr.lam;
    let fine_res = 1.0 - wg_smooth(0.25, 0.9, footprint);
    // the roller's lumps, carried with the crest
    let lumps = sp_tex(vec2<f32>(s * 0.06, ul * 0.09 + m * 0.31), 1u) * 0.55 + sp_tex(vec2<f32>(s * 0.21, ul * 0.27 + m * 0.17), 0u) * 0.3 * fine_res
        + sp_tex(vec2<f32>(s * 0.7, ul * 0.8 + m * 0.43), 3u) * 0.25 * fine_res;
    o.lump = clamp(lumps, -1.0, 1.0);
    var roll = 0.0;
    var gap_face = 0.0;
    if (cr.brk > 0.0 && cr.h_wave > 0.03) {
        let a = cr.age / cr.lam;
        let land = mix(0.0, 0.13, cr.plunge);
        let act = wg_smooth(land * 0.7, land + 0.02, a);
        let wander = sp_tex(vec2<f32>(s * 0.0014, m * 0.137), 2u);
        // the roller's reach in metres: ahead of the crest line its front (the thrown lip's
        // splash first), behind it 1-3.5 times its height as the bore matures
        let splash = (0.9 * cr.h_wave + 0.4) * (0.4 + 1.6 * cr.plunge * wg_smooth(land, land + 0.12, a)) * (0.7 + 0.6 * wander);
        // (the whitewater spills down the whole front face, as wide as the face is, so a
        // broken wave's front is a wall of white from the sand, not a clear green face)
        let face_len = mix(0.08, 0.035, wg_smooth(0.14, 0.42, a)) * cr.lam;
        let ahead = max(splash, mix(0.3, face_len + 0.4, wg_smooth(land, land + 0.15, a)) * (0.85 + 0.3 * wander));
        let behind = (1.0 + 2.5 * wg_smooth(0.1, 0.5, a)) * (0.6 * cr.h_wave + 0.8) * (0.75 + 0.5 * wander);
        // (streaks running down the face, metres long and decimetres across: the front's
        // toe reaches down in fingers and its foam is streaked light and dark)
        let streak_v = sp_tex(vec2<f32>(s * 0.35, ul * 0.05 + m * 0.7), 0u);
        let ur = ul + (lumps * 0.5 + select(0.0, streak_v * 0.9, ul < 0.0)) * (0.5 + 0.6 * cr.h_wave);
        let on = wg_smooth(-ahead - 0.3, -ahead + 0.3, ur) * (1.0 - wg_smooth(behind * 0.6, behind, ur));
        // thin sections of the bore carry less (patchy along the crest, never a ruled band)
        let patchy = wg_smooth(-0.45, 0.25, sp_tex(vec2<f32>(s * 0.0011, m * 0.193 + 0.5), 3u) + 0.3 * cr.inten);
        // (and gaps between the breaking sections, 10-30 m, where the face is still green)
        let gap = wg_smooth(-0.5, -0.25, sp_tex(vec2<f32>(s * 0.004, m * 0.29 + 0.13), 1u) + 0.25 * cr.inten - 0.3 * wg_smooth(0.3, 0.6, a));
        roll = on * act * cr.brk * mix(0.75, 1.0, patchy) * mix(0.8, 1.0, cr.inten) * gap;
        gap_face = (1.0 - gap) * cr.brk;
        o.lump = mix(o.lump, clamp(streak_v * 1.4, -1.0, 1.0), select(0.0, 0.7, ul < 0.0));
        // the lip's own white edge before it lands
        let tip = exp(-pow((u - 0.002) / 0.008, 2.0)) * wg_smooth(0.02, 0.08, a) * (1.0 - act) * cr.plunge * cr.brk;
        roll = max(roll, tip * 0.8);
        // the roller's front shades itself: darker toward its toe, its top open to the sky and the sun
        // (on the face only: the water ahead of the toe is open again)
        let front = select(0.0, 1.0, ul < 0.0) * wg_smooth(-ahead - 0.8, -ahead - 0.1, ul) * wg_smooth(0.1, 0.3, a) * cr.brk;
        o.ao = mix(1.0, mix(0.38, 1.0, pow(wg_smooth(-ahead, -0.1 * ahead, ul), 0.7)), front);
    }
    // what the bores left (the last three): the wake of the latest, the residual of each, the churn
    var wake = 0.0;
    var res = 0.0;
    var aer = 0.0;
    let mp = select(m - 1.0, m, u >= 0.0);
    let tn = sp_table_n(k, s, n);
    for (var j = 0; j < 3; j = j + 1) {
        let mj = mp - f32(j);
        let age = (phi - mj) * period;
        let wv = sp_wave(k, tn, sp_hin(k, mj, s));
        if (wv.brk > 0.0 && wv.h > 0.02) {
            let d = (0.6 + 0.4 * wv.inten) * wv.brk * wg_smooth(0.02, 0.25, wv.h);
            if (j == 0) {
                wake = d * exp(-age / mix(6.0, 10.0, storm));
            }
            let r = 0.55 * d * exp(-age / mix(22.0, 40.0, storm));
            res = 1.0 - (1.0 - res) * (1.0 - r);
            aer = max(aer, d * (0.7 * exp(-age / 50.0) + 0.3 * exp(-age / 6.0)));
        }
    }
    // a storm's surf zone is white over most of it, streaked along the shore
    let zone = wg_smooth(-2.0, 8.0, n) * wg_smooth(520.0, 260.0, n) * info.d.y;
    let streak = sp_tex(vec2<f32>(s * 0.004, n * 0.03 + t * 0.0005), 2u) * 0.7 + sp_tex(vec2<f32>(s * 0.011, n * 0.08 - t * 0.001), 3u) * 0.3;
    let churn = wg_smooth(0.1, 0.55, storm) * zone * mix(0.7, 0.97, wg_smooth(-0.35, 0.3, streak));
    // (the swash: after the bore's foam is carried up, the draining backwash near the
    // waterline is clear, glossy water with a few streaks; the foam stands back from it)
    let sw = sp_swash(k, s, t, dl);
    let backwash = wg_smooth(12.0, 3.0, x) * wg_smooth(0.12, 0.4, sw.y) * (1.0 - storm);
    let cover_a = clamp(max(1.0 - (1.0 - wake) * (1.0 - res), churn), 0.0, 1.0) * (1.0 - 0.8 * backwash);
    // the residual drifts shoreward and along the shore
    let qa = vec2<f32>(s + t * 0.25, n + t * 0.4);
    // (the foam's bubbly fizz: a decimetre-scale noise that frays every edge and
    // speckles the inside, faded where a pixel holds it)
    let fizz = sp_tex(qa * 0.6 + vec2<f32>(0.71, 0.37), 2u) * fine_res;
    o.speck = fizz;
    let pa = sp_churn(qa, footprint) + 0.07 * fizz;
    o.pat = pa;
    o.dbg = backwash;
    // (crisp near, soft only where a pixel holds metres)
    let ea = 0.008 + 0.02 * (1.0 - wake) + 0.07 * wg_smooth(0.05, 2.0, footprint);
    var vis = wg_smooth(1.0 - cover_a - ea, 1.0 - cover_a + ea, pa);
    o.thick = wg_smooth(1.0 - cover_a, 1.0 - cover_a + 0.3, pa);
    // (the thinning residue breaks into lace: bubbly holes of decimetres to a metre open in
    // it, more toward its thin edges and as it ages, leaving filaments; fresh whitewater stays whole)
    let hole_n = sp_tex(qa * 0.15 + vec2<f32>(0.19, 0.61), 3u) + 0.35 * fizz;
    let hole_thr = -0.75 + 0.75 * (1.0 - wake) * (1.0 - 0.5 * o.thick) * (1.0 - 0.85 * wg_smooth(0.1, 0.55, storm));
    vis = vis * mix(1.0, wg_smooth(hole_thr - 0.04, hole_thr + 0.04, hole_n), fine_res);
    // (the water just outside a patch's edge, in its shade and wetted darker)
    o.rim = wg_smooth(1.0 - cover_a - ea - 0.05, 1.0 - cover_a - ea, pa) * (1.0 - vis) * fine_res;
    // the roller is solid, its edges torn by its lumps
    let pf = wg_smooth(-0.5, 0.5, lumps) * 0.8 + 0.2 * pa;
    let vr = wg_smooth(1.0 - roll - 0.1, 1.0 - roll + 0.1, pf);
    vis = max(vis, vr) * (1.0 - 0.85 * backwash);
    o.thick = max(o.thick, vr);
    var fresh = max(roll, wake * 0.8);
    // the swash: a lobed bubble line at the sheet's edge, the bore's foam carried up and left draining
    if (x < 15.0) {
        let front = -max(sw.x, 0.0) * 55.0;
        let lobe = 0.7 + 0.6 * sp_tex(vec2<f32>(s * 0.008, x * 0.003 + sw.y * 0.1), 1u);
        let line = wg_smooth(lobe * 1.2, 0.0, abs(x - front - 0.4)) * wg_smooth(0.0, 0.05, sw.x);
        let carried = (1.0 - wg_smooth(0.03, 0.35, sw.y)) * wg_smooth(15.0, 3.0, x) * 0.5;
        let sfoam = max(wg_smooth(0.4, 0.7, pa) * line, wg_smooth(1.0 - carried - 0.06, 1.06 - carried, pa) * carried);
        vis = max(vis, sfoam);
        fresh = max(fresh, line * 0.6);
    }
    o.cover = vis;
    o.fresh = clamp(max(fresh, churn * 0.6), 0.0, 1.0);
    // a breaking wave's rising face is clear water drawn up from outside the break
    // (and so is the face in a gap between breaking sections)
    let clear_face = wg_smooth(-0.07, -0.01, u) * (1.0 - wg_smooth(-0.006, 0.01, u)) * max(1.0 - cr.brk, gap_face);
    o.cover = o.cover * (1.0 - clear_face);
    o.aer = clamp(max(aer, churn), 0.0, 1.0) * (1.0 - clear_face) * wg_smooth(0.0, 4.0, x) * (1.0 - 0.85 * backwash);
    // the steep, thin front face near an unbroken or just-breaking crest
    o.face = wg_smooth(-0.07, -0.006, u) * (1.0 - wg_smooth(-0.002, 0.02, u)) * wg_smooth(0.45, 0.95, cr.e) * (1.0 - fresh) * wg_smooth(0.1, 0.6, lift + cr.h_wave * 0.5);
    o.bored = cr.brk * select(0.0, 1.0, cr.age >= 0.0) * wg_smooth(0.1, 0.4, cr.age / cr.lam);
    return o;
}

// The foam's own light: a diffuser of albedo 0.42 fresh and 0.28 aged (an
// aged whitecap's effective reflectance is about 0.22) under the sun where
// it reaches it (the cascades and the cloud decks) and the sky's irradiance,
// as the city's own diffuse surfaces are lit: never brighter than the light
// it gets, and darker than sunlit cloud tops.
// (split into the light a rough diffuser there receives, shared with the
// aerated water, and the foam's albedo; a face tilted off the sky sees less of it)
// `sky`: the sky's mean radiance as drawn (sp_sky_mean); `vis`: the sun's share reaching it (sp_sun_vis)
fn sp_irradiance(n: vec3<f32>, sky: vec3<f32>, vis: f32) -> vec3<f32> {
    let l = look.key_dir.xyz;
    // (under an overcast the sun's part is mostly gone, as the sea's foam takes it (1 - 0.85 x the
    // overcast weight): the storm's foam had the clear key's warm light on it and read pinkish)
    let sun = look.key.xyz * max(dot(n, l), 0.0) * vis * (1.0 - 0.85 * clamp(look.wx.x, 0.0, 1.0));
    let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5);
    // (without the SH term: a tenth of the hue only, and a cost per pixel)
    let amb_raw = (hemi + look.ambient.xyz) / WG_PI;
    var amb = mix(amb_raw * (lk_luma(sky) / max(lk_luma(amb_raw), 1e-6)), sky, 0.9) * (0.55 + 0.45 * clamp(n.y, 0.0, 1.0));
    // (under a storm's deck the light is the cloud's grey: white water under it is grey-white, not tan)
    // (and a white diffuser under the whole dome averages its hues: the warm belt round a low sun is a
    // part of it, so the foam at golden hour is warm white, not the colour of the sand)
    amb = mix(amb, vec3<f32>(lk_luma(amb)) * vec3<f32>(0.97, 0.99, 1.02), max(0.9 * clamp(sp_storm(), 0.0, 1.0), 0.35));
    return sun / WG_PI + amb;
}

// the sky's part at the brightness the sky is drawn (a diffuser under the whole dome
// returns its albedo times the sky's mean radiance: the zenith and the horizon)
// (the zenith as the frame drew it: under a storm deck the grey cloud, not the clear model)
// the sun reaching a point (the cascades and the cloud decks)
fn sp_sun_vis(world: vec3<f32>) -> f32 {
    return w_shadow(world, vec3<f32>(0.0, 1.0, 0.0)) * sky_cloud_transmittance(world);
}

fn sp_sky_mean() -> vec3<f32> {
    let up = vec3<f32>(0.0, 1.0, 0.0);
    // (at a low sun the horizon's warm belt is one side of the sky, not all of it)
    let low = wg_smooth(0.3, 0.05, look.key_dir.y);
    let hz = mix(look.horizon.xyz, vec3<f32>(lk_luma(look.horizon.xyz)), 0.5 * low);
    return 0.5 * (w_drawn_sky(up, w_atmo_dome(up)) + hz);
}

fn sp_foam_albedo(fresh: f32) -> vec3<f32> {
    // (thick fresh foam reflects 0.6-0.8; under the sun it is held to 0.42 so it stays
    // under sunlit cloud tops as the sky draws them; under a deck it returns most of the sky's light)
    let deck = clamp(max(look.wx.x, sp_storm()), 0.0, 1.0);
    // (thinning residue greyer, 0.24; under a low sun fresh foam returns more, 0.55: little of
    // the sun reaches its top, and white water in the evening light reads warm white, not peach)
    let low = wg_smooth(0.3, 0.05, look.key_dir.y);
    // (under a deck the residue too returns most of the sky's light: with the sun gone it is the brightest thing on the sea)
    return mix(mix(0.24, 0.6, deck), mix(mix(0.42, 0.55, low), 0.85, deck), fresh) * vec3<f32>(0.97, 0.98, 1.0);
}

// The wet glaze: a film a few millimetres thick over the sand the swash
// just left, a mirror of the sky that dries out; the sand under it darker.
// Premultiplied (colour, alpha over the sand).
fn sp_glaze(world: vec3<f32>, wet: f32) -> vec4<f32> {
    let v = normalize(wu.cam.xyz - world);
    let ripple = vec2<f32>(sp_tex(world.xz * 0.05, 0u), sp_tex(world.xz * 0.05 + vec2<f32>(0.41, 0.23), 1u)) * 0.03;
    let n = normalize(vec3<f32>(ripple.x, 1.0, ripple.y));
    let r = reflect(-v, n);
    let rup = normalize(vec3<f32>(r.x, max(r.y, 0.004), r.z));
    let sky = w_drawn_sky(rup, w_atmo_dome(rup));
    let f = wo_fresnel(max(dot(n, v), 1e-3), WO_IOR);
    // the sun's own glint on it (only the sun: a mirror shows nothing in the opposite sky)
    let l = look.key_dir.xyz;
    let h = normalize(l + v);
    let a2 = 0.004;
    let ndh = max(dot(n, h), 0.0);
    let dd = ndh * ndh * (a2 - 1.0) + 1.0;
    let spec = min(a2 / (WG_PI * dd * dd) * 0.25 * wo_fresnel(max(dot(v, h), 0.0), WO_IOR) * max(dot(n, l), 0.0), 40.0);
    let vis = w_shadow(world, vec3<f32>(0.0, 1.0, 0.0)) * sky_cloud_transmittance(world);
    // glossy for a few seconds behind the drained edge (a band of metres; e-fold 3 s), in
    // streaks along the shore, then damp and darker for longer (18 s), and matte
    let glossy = pow(wet, 6.0) * (0.35 + 0.65 * wg_smooth(-0.25, 0.35, sp_tex(vec2<f32>(world.x * 0.006, world.z * 0.02), 3u)));
    let col = (sky * f + look.key.xyz * spec * vis) * glossy;
    let fog = lk_smooth(look.fog.x, look.fog.y, sky_fog_depth(max(dot(world - wu.cam.xyz, wu.fwd.xyz), 1e-3), world.y - look.cam_pos.y));
    let a = clamp(0.3 * wet + 0.7 * f * glossy, 0.0, 0.9) * (1.0 - fog);
    return vec4<f32>(col * (1.0 - fog), a);
}

@fragment
fn fs_surf(i: SurfOut) -> @location(0) vec4<f32> {
    w_atmo_g = i.atmo_g;
    // the opaque scene in front of the strip (reverse Z: larger is nearer)
    let px = vec2<i32>(i.clip.xy);
    if (textureLoad(w_depth, px, 0) > i.clip.z) {
        discard;
    }
    let f = w_field_at(i.base);
    let cov = surf_cover(i.base, f);
    if (cov <= 0.002) {
        discard;
    }
    let k = i.strip;
    let t = wu.cam.w;
    // the wet glaze past the swash's edge
    // (and the sheet's thinnest edge, under 1.5 cm, a wet film over the sand: glossy, the sand through it)
    if (i.lift.y < 0.015 && f.x < 2.0) {
        let wet = select(sp_wet(k, i.sn.x, f.x, t, i.sn.z), 1.0, i.lift.y >= 0.002);
        if (wet <= 0.03) {
            discard;
        }
        if (su.misc.w > 0.5) {
            return vec4<f32>(0.0, 0.0, wet, 1.0);
        }
        let g = sp_glaze(i.world, wet);
        return g * cov;
    }
    let footprint = clamp(sqrt(length(dpdx(i.world.xz)) * length(dpdy(i.world.xz))), 0.0, 64.0);
    let fo = sp_foam(k, i.sn.x, i.sn.y, f.x, i.phi, i.lift.x, footprint, i.sn.z);
    // EO_SURF_DEBUG=1: r the surf's height over the still sea (2.5 m full), g how far it is breaking, b the foam
    if (su.misc.w > 0.5 && su.misc.w < 1.5) {
        return vec4<f32>(clamp(i.lift.x / 2.5, 0.0, 1.0), fo.crest.brk, fo.cover, 1.0);
    }
    // EO_SURF_DEBUG=3: r the aerated water, g the churn texture (it should fill 0..1 evenly), b the backwash
    if (su.misc.w > 2.5) {
        return vec4<f32>(fo.aer, fo.pat, fo.dbg, 1.0);
    }
    // the foam over the water: its own light, the look's fog
    // (the foam stands a few centimetres proud of the water: the slope of its thickness, from
    // the pixel's own derivatives, tilts its normal, so its edges and bubbles facing the light are lit
    // and the far ones shaded)
    let jx = dpdx(i.world.xz);
    let jy = dpdy(i.world.xz);
    let gs = vec2<f32>(dpdx(fo.thick), dpdy(fo.thick));
    let det = jx.x * jy.y - jx.y * jy.x;
    let gw = select(vec2<f32>(0.0), vec2<f32>(jy.y * gs.x - jx.y * gs.y, jx.x * gs.y - jy.x * gs.x) / det, abs(det) > 1e-10);
    let relief = clamp(gw * 0.25, vec2<f32>(-0.8), vec2<f32>(0.8)) * (1.0 - wg_smooth(0.2, 0.8, footprint));
    let nf = normalize(vec3<f32>(-i.slope.x - relief.x, 1.0, -i.slope.y - relief.y));
    let view_dist = max(dot(i.world - wu.cam.xyz, wu.fwd.xyz), 1e-3);
    let fog = lk_smooth(look.fog.x, look.fog.y, sky_fog_depth(view_dist, i.world.y - look.cam_pos.y));
    let sky_mean = sp_sky_mean();
    let sun_vis = sp_sun_vis(i.world);
    let irr = sp_irradiance(nf, sky_mean, sun_vis);
    // (fresh whitewater has volume: its lumps lighter and darker, the roller's front in its own shade)
    let tone = (0.86 + 0.3 * fo.lump * fo.fresh) * fo.ao * mix(0.7, 1.0, fo.thick) * (0.93 + 0.14 * fo.speck);
    // (thin foam and its bubbles' edges pass the sun forward: toward a low sun the foam
    // glows warm at its thin parts and edges, held under 1.5 x the horizon's brightness)
    let vdir = normalize(i.world - wu.cam.xyz);
    let lsun = look.key_dir.xyz;
    let cos_t = dot(vdir, lsun);
    let hg = 0.75 / (4.0 * WG_PI * pow(max(1.25 - cos_t, 1e-3), 1.5));
    let glow = min(look.key.xyz * sun_vis * wg_smooth(-0.02, 0.08, lsun.y) * hg * 0.2 * (1.0 - 0.6 * fo.thick) * (1.0 - 0.85 * clamp(look.wx.x, 0.0, 1.0)), vec3<f32>(lk_luma(look.horizon.xyz) * 1.5));
    // (at a low sun the foam's colour is held toward a warm white: the light's hue at half its
    // saturation; the churned zone reads as foam, not as sand of the same hue)
    let low = wg_smooth(0.3, 0.05, lsun.y);
    var foam_lit = sp_foam_albedo(fo.fresh) * irr * tone + glow;
    let fl = lk_luma(foam_lit);
    foam_lit = mix(foam_lit, vec3<f32>(fl) * vec3<f32>(1.04, 1.0, 0.9), 0.55 * low);
    let foam_col = mix(foam_lit, look.fog_color.xyz, fog);
    // aged foam is a thin translucent film; fresh whitewater is opaque
    // (and so is the heart of an aged patch, where the foam is thick)
    // (thinning out over the sheet's last few centimetres, frayed by the fizz, so it does not stop
    // on the straight line where the thin film takes over)
    let fray = select(1.0, wg_smooth(0.015, 0.05, i.lift.y + 0.02 * fo.speck), f.x < 2.0);
    let fa = clamp(fo.cover * fray, 0.0, 1.0) * mix(mix(0.6, 1.0, wg_smooth(0.5, 0.9, fo.thick)), 1.0, max(fo.fresh, sp_storm()));
    // (opaque whitewater hides the water: its shading is skipped; from 0.95, drawn opaque)
    if (fa >= 0.95) {
        return vec4<f32>(foam_col * cov, cov);
    }
    // the churned water between the foam is chopped (its own short, steep ripples)
    let cq = i.world.xz;
    let chop_res = 1.0 - wg_smooth(1.0, 5.0, footprint);
    let chop = vec2<f32>(sp_tex(cq * 0.21 + vec2<f32>(t * 0.05, 0.0), 0u), sp_tex(cq * 0.21 + vec2<f32>(0.31, 0.77 - t * 0.04), 1u)) * 0.15 * fo.aer * chop_res;
    sf_inj.on = 1.0;
    sf_inj.slope = i.slope + chop;
    // the foam is drawn over the water below with its own light, not the sea's
    sf_inj.foam = 0.0;
    sf_inj.face = fo.face * cov;
    sf_inj.ww = fo.fresh * cov;
    sf_inj.bored = fo.bored * cov;
    sf_inj.swash = 0.0;
    // foam is a rough scatterer: none of the water's sun glint shows through it
    sf_inj.spec_cut = clamp(fo.cover, 0.0, 1.0);
    // (churned water mirrors no scene: the sea's screen-space march is skipped there)
    sf_inj.rough = select(0.0, 1.0, fo.aer > 0.6);
    let copy = wu.flags.x > 0.5;
    let s = w_shade(i.world, i.base, i.clip, copy);
    // the churned water: bubbles and stirred sand make it a pale grey-green scatterer
    // (diffuse albedo about 0.1) that hides the bottom; its chopped surface shows the sky
    // blurred to its mean and dimmer, and half the sun's glints
    // (at a low sun the churned water keeps more of its mirror, darker and cooler: the warm
    // diffuse body under the evening light read as wet sand)
    let ak = 0.85 * fo.aer * (1.0 - 0.45 * low);
    let ac = irr * 0.1 * vec3<f32>(0.86, 1.0, 0.82);
    let aer_col = mix(mix(ac, vec3<f32>(lk_luma(ac)) * vec3<f32>(0.9, 1.0, 1.05), 0.6 * low), look.fog_color.xyz, fog);
    let own = (s.own * (1.0 - 0.5 * ak) + aer_col * ak) * (1.0 - 0.22 * fo.rim);
    let rk = s.refl_k * (1.0 - 0.45 * ak);
    let through_k = 1.0 - ak;
    if (!copy) {
        let through = clamp(dot(s.through, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0) * through_k;
        let wc = own + mix(s.sky, sky_mean, 0.7 * ak) * rk * (1.0 - 0.22 * fo.rim);
        let a = 1.0 - through;
        return vec4<f32>(mix(wc, foam_col, fa) * cov, mix(a, 1.0, fa) * cov);
    }
    // as water.wgsl `fs_copy` combines its terms
    let bottom = textureSampleLevel(w_scene, w_clamp, s.uv, 0.0).rgb;
    var refl = s.sky;
    if (s.mirror_w > 0.0) {
        let u = s.mirror_uv;
        let sm = s.smear;
        var tap = textureSampleLevel(w_scene, w_clamp, u, 0.0).rgb * 0.4
            + textureSampleLevel(w_scene, w_clamp, u - vec2<f32>(0.0, sm * 0.5), 0.0).rgb * 0.25
            + textureSampleLevel(w_scene, w_clamp, u - vec2<f32>(0.0, sm), 0.0).rgb * 0.2
            + textureSampleLevel(w_scene, w_clamp, u - vec2<f32>(0.0, sm * 1.5), 0.0).rgb * 0.1
            + textureSampleLevel(w_scene, w_clamp, u - vec2<f32>(0.0, sm * 2.0), 0.0).rgb * 0.05;
        if (skyu.flags.x != 0u && skyu.taa.y > 0.5) {
            let pu = w_prev_uv(u);
            if (all(pu > vec2<f32>(0.0)) && all(pu < vec2<f32>(1.0))) {
                tap = textureSampleLevel(sky_prev, sky_rep, pu, 0.0).rgb * 0.4
                    + textureSampleLevel(sky_prev, sky_rep, pu - vec2<f32>(0.0, sm * 0.5), 0.0).rgb * 0.25
                    + textureSampleLevel(sky_prev, sky_rep, pu - vec2<f32>(0.0, sm), 0.0).rgb * 0.2
                    + textureSampleLevel(sky_prev, sky_rep, pu - vec2<f32>(0.0, sm * 1.5), 0.0).rgb * 0.1
                    + textureSampleLevel(sky_prev, sky_rep, pu - vec2<f32>(0.0, sm * 2.0), 0.0).rgb * 0.05;
            }
        }
        let scene = min(max(tap, vec3<f32>(0.0)), max(s.sky * WS_SCENE_CEIL, vec3<f32>(WS_SCENE_CEIL_FLOOR)));
        let live = wg_smooth(0.0, WS_SCENE_FLOOR, lk_luma(scene));
        refl = mix(s.sky, scene, s.mirror_w * live * WS_SCENE_TAKE);
    }
    let col = own + mix(refl, sky_mean, 0.7 * ak) * rk + bottom * s.through * through_k;
    return vec4<f32>(mix(col, foam_col, fa) * cov, cov);
}

// ---- spray: plumes off the breaking crests, and the salt haze over the break ----
// One quad per column per wave slot: a plume standing on each breaking crest
// from its throw until the bore runs on, 1.2-3 of the wave's height (more
// where it plunges, more in a storm), thrown up and back over the wave (the
// crest runs out from under it, an offshore wind blows it back). Two more
// slots per column are the salt haze hanging over the outer and inner bars'
// break lines, 5-11 m tall, thicker in a storm: what lifts the break off the
// horizon. Lit by the sky and the sun scattered forward through it (bright
// and translucent against a low sun), never brighter than the sky's light.

struct SprayOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    // x along the shore (m), y up the quad 0..1, z amount, w the column's wave is stable (0 at a rounding flip)
    @location(1) q: vec4<f32>,
    // 1 for the haze band, 0 for a plume
    @location(2) @interpolate(flat) haze: f32,
    // the atmosphere table's gain (water.wgsl `w_atmo_gain`)
    @location(3) @interpolate(flat) atmo_g: f32,
};

@vertex
fn vs_spray(@builtin(vertex_index) vi: u32, @builtin(instance_index) k: u32) -> SprayOut {
    let info = s_info[k];
    let cols = info.f.y;
    let quad = vi / 6u;
    let corner = vi % 6u;
    let w = f32(quad / cols);
    var i = quad % cols;
    var top = 0.0;
    // corners: (0, b) (0, t) (1, b) / (1, b) (0, t) (1, t)
    if (corner == 2u || corner == 3u || corner == 5u) {
        i = i + 1u;
    }
    if (corner == 1u || corner == 4u || corner == 5u) {
        top = 1.0;
    }
    let s = s_axis[info.f.x + min(i, cols)];
    let period = info.c.y;
    let t = wu.cam.w;
    let storm = sp_storm();
    let wind = select(vec2<f32>(0.0), skyu.wind.xy, skyu.flags.x != 0u);
    var o: SprayOut;
    if (w >= info.d.w) {
        // the salt haze over a bar's break line (the outer bar, then the inner)
        let band = w - info.d.w;
        // (the bar's table n less the shift, as sp_table_n maps them)
        let nb = select(105.0, 235.0, band < 0.5) - sp_bar_shift(k, s) - 10.0;
        let fr = sp_frame(k, s);
        let base = fr.xy + fr.zw * nb;
        let f = w_field_at(base);
        let cov = surf_cover(base, f);
        let h0 = info.c.z * su.misc.z;
        let height = (2.0 + 2.2 * h0) * select(0.7, 1.0, band < 0.5);
        let offshore = clamp(dot(wind, fr.zw) / 8.0, 0.0, 1.0);
        let horiz = base + fr.zw * (top * height * (0.8 + 1.5 * offshore));
        let world = vec3<f32>(horiz.x, wu.grid.z - 0.3 + top * height, horiz.y);
        o.clip = wu.view_proj * vec4<f32>(world, 1.0);
        o.world = world;
        o.q = vec4<f32>(s, top, cov * sp_beach_w(k, s) * (0.3 + 1.4 * storm + 0.12 * h0), 1.0);
        o.haze = 1.0;
        o.atmo_g = w_atmo_gain();
        return o;
    }
    let dl = sp_delta(k, s);
    let z = t / period + dl + 1.5 - info.d.w + w;
    let m = floor(z + 0.5);
    let stable = wg_smooth(0.0, 0.08, 0.5 - abs(z - m));
    let cr = sp_crest(k, m, s, t, dl);
    var amount = 0.0;
    var height = 0.0;
    if (cr.brk > 0.0 && cr.h_wave > 0.4) {
        let a = cr.age / cr.lam;
        // from the throw while the wave still breaks hard (its height over the stable bore's: the
        // table's intensity), fading as it settles into a bore; a spilling crest throws less
        let life = wg_smooth(0.0, 0.03, a) * wg_smooth(0.25, 0.8, cr.inten);
        // (and thin wisps blown off a bore's roller as it runs in)
        let wisp = 0.3 * wg_smooth(0.15, 0.4, a) * wg_smooth(0.7, 1.2, cr.h_wave);
        // in puffs along the crest (sections of 20-60 m), not a curtain
        let puff = 0.5 + 0.5 * sin(s * 0.071 + m * 2.1 + 0.6 * sin(s * 0.023 + m));
        amount = max(life, wisp) * mix(0.5, 1.0, cr.plunge) * mix(0.6, 1.0, cr.inten) * (1.0 + 1.5 * storm) * cr.brk * wg_smooth(0.25, 0.75, puff + 0.3 * storm);
        height = cr.h_wave * (1.6 + 2.2 * cr.plunge) * (0.6 + 0.4 * life) * (1.0 + 0.6 * storm);
    }
    let sw = sp_swash(k, s, t, dl);
    let crest = sp_point(k, s, m + 0.004, 0.004, cr, sw, 0.0, dl, 0.0);
    let f = crest.f;
    let cov = surf_cover(crest.base, f);
    let sea = sp_sea_disp(crest.base, f, cov);
    let y0 = wu.grid.z + sea.y + crest.y * cov - 0.25 * cr.h_wave;
    // thrown up and back: the crest runs out from under it, an offshore wind blows it seaward
    let offshore = clamp(dot(wind, crest.nrm) / 8.0, 0.0, 1.0);
    let lean = (0.6 + 1.0 * offshore) * height;
    let horiz = crest.base + sea.xz - crest.nrm * (crest.dx * cov) + crest.nrm * (lean * top * top);
    let world = vec3<f32>(horiz.x, y0 + (height + 0.25 * cr.h_wave) * top, horiz.y);
    o.clip = wu.view_proj * vec4<f32>(world, 1.0);
    o.world = world;
    o.q = vec4<f32>(s, top, amount * cov, stable);
    o.haze = 0.0;
    o.atmo_g = w_atmo_gain();
    return o;
}

@fragment
fn fs_spray(i: SprayOut) -> @location(0) vec4<f32> {
    w_atmo_g = i.atmo_g;
    // EO_SURF_DEBUG=2: the spray quads (magenta: amount, cyan: the haze bands)
    if (su.misc.w > 1.5 && su.misc.w < 2.5) {
        return select(vec4<f32>(i.q.z * i.q.w, 0.0, i.q.z * i.q.w, 1.0), vec4<f32>(0.0, i.q.z, i.q.z, 1.0), i.haze > 0.5);
    }
    if (i.q.z * i.q.w <= 0.002) {
        discard;
    }
    let px = vec2<i32>(i.clip.xy);
    if (textureLoad(w_depth, px, 0) > i.clip.z) {
        discard;
    }
    let t = wu.cam.w;
    let v = i.q.y;
    var billow = 0.0;
    var profile = wg_smooth(0.0, 0.12, v) * pow(1.0 - v, 1.4);
    var scale = 2.2;
    if (i.haze < 0.5) {
        // billows rising off the crest: three octaves drifting up and along
        // (read from the detail texture, scrolled: three 3D noises a pixel were most of the spray's cost)
        let n1 = sp_tex(vec2<f32>(i.q.x * 0.07, v * 0.32 - t * 0.1), 0u);
        let n2 = sp_tex(vec2<f32>(i.q.x * 0.26 + 0.41, v * 1.0 - t * 0.28), 1u);
        let n3 = sp_tex(vec2<f32>(i.q.x * 0.74 + 0.13, v * 2.2 - t * 0.46), 2u);
        // wisps, not a sheet: thresholded billows, ragged at every scale
        billow = wg_smooth(0.15, 0.75, 0.5 + 0.45 * n1 + 0.3 * n2 + 0.2 * n3);
    } else {
        // the haze: a soft band, densest low, its top ragged
        billow = 0.65 + 0.35 * sp_tex(vec2<f32>(i.q.x * 0.004 + t * 0.0006, v * 0.3), 2u);
        profile = wg_smooth(0.0, 0.25, v) * (1.0 - wg_smooth(0.35 + 0.35 * billow, 1.0, v));
        scale = 0.28;
    }
    let dens = billow * profile * i.q.z * i.q.w;
    let to_cam = wu.cam.xyz - i.world;
    let vdir = normalize(to_cam);
    let l = look.key_dir.xyz;
    let cloud_t = sky_cloud_transmittance(i.world);
    // forward scattering of the sun through the mist (Henyey-Greenstein, g 0.7), and the sky's light
    let g = 0.7;
    let cos_t = dot(-vdir, l);
    let hg = (1.0 - g * g) / (4.0 * 3.14159265 * pow(max(1.0 + g * g - 2.0 * g * cos_t, 1e-4), 1.5));
    // droplets scatter nearly all they get: the sky's light as the frame draws it, and
    // the sun as a white scatterer (a plume side-lit reads as white as sunlit foam, against
    // a low sun it glows), held under 2.5 x the horizon's brightness
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let sky = 0.5 * (w_drawn_sky(up, w_atmo_dome(up)) + look.horizon.xyz) * 0.9;
    let sun = look.key.xyz * cloud_t * (0.12 + 1.6 * hg) * wg_smooth(-0.02, 0.1, l.y) / 3.14159265;
    var col = min(sky + sun, vec3<f32>(max(lk_luma(look.horizon.xyz) * 2.5, 1e-4)));
    let view_dist = max(dot(i.world - wu.cam.xyz, wu.fwd.xyz), 1e-3);
    let fog = lk_smooth(look.fog.x, look.fog.y, sky_fog_depth(view_dist, i.world.y - look.cam_pos.y));
    col = mix(col, look.fog_color.xyz, fog);
    let a = clamp(dens * scale, 0.0, select(0.7, 0.4, i.haze > 0.5));
    return vec4<f32>(col * a, a);
}
