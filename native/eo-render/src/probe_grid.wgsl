// The probe grid (probe_grid.rs, `--probe-grid 1|2`): reflection probes on
// the streets round the camera, their cubes at group 1 binding 15
// (`lx_probe_grid`: 6 layers a slot, every level) and their table at
// binding 16 (`lx_probe_table`, look_ext.wgsl):
//
//   [0]           x entries, y log2 of the face size, z the last level, w the
//                 row of the first slot's irradiance (16 rows a slot: the
//                 fill's lobe row, then SH 0-8; probe_fill.wgsl `lobe`)
//   [1]           x the distance weight's softening squared (m^2), y z the
//                 reach (full within y metres of a probe, none past z), w spare
//   [2 + 2e]      entry e: the probe's point, its presence (0..1, faded in and out)
//   [2 + 2e + 1]  its slot, the slot it cross-fades from (-1: none), the
//                 weight of its slot in that cross-fade (1: done), spare
//
// Everything is read from the table, not from `Look` rows (the props'
// cut-out pipeline takes no more uniform reads, stage B2). `PG_SHADER`
// (probe_grid.rs `wgsl`, per shader variant) compiles the lookups in or
// out; with no entry (the stand-in's zeros, the grid off) nothing else is
// read. `pg_select` is mirrored on the CPU by probe_grid.rs `selection`.
// The lookups loop over the chosen slots (one sample in the body, a count
// the compiler cannot unroll): Apple's shader compiler stands at its limit
// on the lit pipelines, and inlined copies of the cube lookup broke them.

const PG_ENTRY: u32 = 2u;

// What a point reads: up to four slots (two probes, each with the slot it
// cross-fades from) and their weights (summing to 1), how much of the grid
// stands in for the analytic sky there, and the blended probes' height (the
// glass's canyon lift).
struct PgSel {
    slots: vec4<f32>,
    w: vec4<f32>,
    n: u32,
    cover: f32,
    y: f32,
}

// Keeps the probe table's header reads (`lx_probe_table[0..1]`) in the fragment
// body rather than Apple AGX's uniform preamble: hoisting device-buffer header
// loads alongside a large fragment shader's uniforms and `discard` aborts
// `MTLCompilerService` in `MCObjectStreamer::emitInstruction`
// (`XPC_ERROR_CONNECTION_INTERRUPTED`, e.g. `prop-opaque` / `prop-cutout`).
// Always 0u for any finite shader value (and NaN); even if an input were
// infinite, 0u + 1u and 1u + 1u stay inside the 4-row stand-in buffer.
fn pg_nu(x: f32) -> u32 {
    return u32(abs(x) > 1e30);
}

fn pg_live() -> bool {
    return PG_SHADER && lk_lx_on(LX_BIT_PROBE_GRID) && lx_probe_table[0].x > 0.5;
}

// The two entries of highest presence / (distance^2 + soft^2), each less the
// third's weight: when the second and third trade places the second's share
// is zero, so the pair changes with no step. `cover`: the largest presence x
// reach over the entries (the sky where no probe stands near or none is in).
fn pg_select(world: vec3<f32>) -> PgSel {
    let nu = pg_nu(world.y);
    let n = u32(lx_probe_table[nu].x + 0.5);
    let k = lx_probe_table[1u + nu];
    var w1 = 0.0;
    var w2 = 0.0;
    var w3 = 0.0;
    var i1 = 0u;
    var i2 = 0u;
    var cover = 0.0;
    for (var e = 0u; e < n; e = e + 1u) {
        let p = lx_probe_table[PG_ENTRY + 2u * e];
        let v = world - p.xyz;
        let d2 = dot(v, v);
        let w = p.w / (d2 + k.x);
        cover = max(cover, p.w * (1.0 - lk_smooth(k.y, k.z, sqrt(d2))));
        if (w > w1) {
            w3 = w2;
            w2 = w1;
            i2 = i1;
            w1 = w;
            i1 = e;
        } else if (w > w2) {
            w3 = w2;
            w2 = w;
            i2 = e;
        } else if (w > w3) {
            w3 = w;
        }
    }
    var a = w1 - w3;
    var b = w2 - w3;
    if (a + b <= 0.0) {
        a = 1.0;
        b = 0.0;
    }
    let wa = a / (a + b);
    let wb = b / (a + b);
    // each entry's slot and the one it cross-fades from, compacted.
    var s: PgSel;
    s.n = 0u;
    s.slots = vec4<f32>(0.0);
    s.w = vec4<f32>(0.0);
    for (var j = 0u; j < 2u; j = j + 1u) {
        let wj = select(wb, wa, j == 0u);
        let q = lx_probe_table[PG_ENTRY + 2u * select(i2, i1, j == 0u) + 1u];
        if (wj > 0.0) {
            s.slots[s.n] = q.x;
            s.w[s.n] = wj * q.z;
            s.n = s.n + 1u;
            if (q.z < 1.0) {
                s.slots[s.n] = q.y;
                s.w[s.n] = wj * (1.0 - q.z);
                s.n = s.n + 1u;
            }
        }
    }
    s.cover = select(0.0, cover, w1 > 0.0);
    s.y = wa * lx_probe_table[PG_ENTRY + 2u * i1].y + wb * lx_probe_table[PG_ENTRY + 2u * i2].y;
    return s;
}

// three's cubeUV roughness -> level at the grid's face size (as fa_probe_lod).
fn pg_lod(rough: f32) -> f32 {
    let h = lx_probe_table[pg_nu(rough)];
    return clamp(h.y + 2.0 * log2(max(1.16 * rough, 1e-4)), 0.0, h.z);
}

// The chosen slots along a direction (the faces and their basis as probe.rs
// renders them, facade_age.wgsl `fa_probe_sample`). The uv is held half a
// texel of the coarser level inside the face, so the repeat sampler
// (binding 11, linear between levels) never reaches across the face's edge.
fn pg_sample(s: PgSel, d: vec3<f32>, lod: f32) -> vec3<f32> {
    let a = abs(d);
    var f = 0u;
    var fw = vec3<f32>(1.0, 0.0, 0.0);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (a.x >= a.y && a.x >= a.z) {
        f = select(1u, 0u, d.x > 0.0);
        fw = vec3<f32>(sign(d.x), 0.0, 0.0);
    } else if (a.y >= a.z) {
        f = select(3u, 2u, d.y > 0.0);
        fw = vec3<f32>(0.0, sign(d.y), 0.0);
        up = vec3<f32>(0.0, 0.0, -sign(d.y));
    } else {
        f = select(5u, 4u, d.z > 0.0);
        fw = vec3<f32>(0.0, 0.0, sign(d.z));
    }
    let right = cross(fw, up);
    let k = 1.0 / max(dot(d, fw), 1e-6);
    let h = 0.5 * exp2(ceil(lod) - lx_probe_table[pg_nu(d.x)].y);
    let uv = clamp(vec2<f32>(dot(d, right) * k * 0.5 + 0.5, 0.5 - dot(d, up) * k * 0.5), vec2<f32>(h), vec2<f32>(1.0 - h));
    var c = vec3<f32>(0.0);
    for (var i = 0u; i < s.n; i = i + 1u) {
        let layer = i32(s.slots[i] + 0.5) * 6 + i32(f);
        c = c + s.w[i] * textureSampleLevel(lx_probe_grid, sky_rep, uv, layer, lod).rgb;
    }
    return c;
}

// The specular environment along a reflection `r`: the slots at the
// roughness's level; for glass (facade_age.wgsl `fa_glazing`) never sharper
// than the 128 px probe's 4-texel footprint, the lookup lifted as if pane
// and probes stood in one street canyon (from the blended probes' height),
// and the dome's own-ray term where the lifted sample is open sky.
fn pg_reflect(s: PgSel, r: vec3<f32>, world: vec3<f32>, rough: f32, glass: bool) -> vec3<f32> {
    var d = r;
    var lod = pg_lod(rough);
    if (glass) {
        let h = max(world.y - s.y, 0.0);
        let horizontal = max(length(r.xz), 1e-4);
        d = normalize(vec3<f32>(r.x, r.y + horizontal * (h / PG_CANYON_M), r.z));
        lod = pg_lod(max(rough, PG_GLAZING_FLOOR));
    }
    var c = pg_sample(s, d, lod);
    if (glass) {
        let dome_lifted = lk_js_dome(d);
        let dome_own = lk_js_dome(normalize(r));
        let lp = lk_luma(c);
        let ld = lk_luma(dome_lifted);
        let sky_like = lk_smooth(0.55, 0.85, min(lp, ld) / max(max(lp, ld), 1e-6));
        c = max(c + (dome_own - dome_lifted) * sky_like, vec3<f32>(0.0));
    }
    return c;
}

// The chosen slots' irradiance (the fill's cosine-convolved SH9, as
// `fa_probe_irradiance`; `--probe-yield`'s diffuse).
fn pg_irradiance(s: PgSel, n: vec3<f32>) -> vec3<f32> {
    let x = n.x;
    let y = n.y;
    let z = n.z;
    let nu = pg_nu(x);
    var e = vec3<f32>(0.0);
    for (var i = 0u; i < s.n; i = i + 1u) {
        let b = u32(lx_probe_table[nu].w + 0.5) + 16u * u32(s.slots[i] + 0.5) + 1u;
        var c = lx_probe_table[b].xyz * 0.282095;
        c = c + lx_probe_table[b + 1u].xyz * (0.488603 * y) + lx_probe_table[b + 2u].xyz * (0.488603 * z) + lx_probe_table[b + 3u].xyz * (0.488603 * x);
        c = c + lx_probe_table[b + 4u].xyz * (1.092548 * x * y) + lx_probe_table[b + 5u].xyz * (1.092548 * y * z) + lx_probe_table[b + 6u].xyz * (0.315392 * (3.0 * z * z - 1.0));
        c = c + lx_probe_table[b + 7u].xyz * (1.092548 * x * z) + lx_probe_table[b + 8u].xyz * (0.546274 * (x * x - y * y));
        e = e + s.w[i] * c;
    }
    return max(e, vec3<f32>(0.0));
}
