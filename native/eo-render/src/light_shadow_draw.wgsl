// lamp and headlight shadows (lighting; light_shadow.rs): the casters into
// the atlas, after light_shadow.rs's constants (and, for the props and the
// agents, their own types and posing). One non-indexed draw a caster list:
// vertex v is corner v % 3 of the list's triangle v / 3, found in the record
// whose first triangle (`start`, ls_scan) is the last at or before it; its
// index and vertex are read from the geometry's own buffers (storage), the
// point projected through its tile's frustum into the tile's cell of the
// atlas. Fragments outside the cell are discarded; the depth written is the
// point's depth along the light's axis, linear from near (0) to far (1).

struct LsRec {
    tile: u32,
    first: u32,
    tris: u32,
    base: i32,
    payload: u32,
    flags: u32,
    start: u32,
    pad: u32,
};

// light_shadow.wgsl LsFrame's head and the tiles drawn this frame (the clears).
struct LsDrawFrame {
    head: vec4<u32>,
    totals: vec4<u32>,
    redraw: array<vec4<u32>, LS_REDRAWS>,
};

@group(0) @binding(0) var<storage, read> ls_table: array<vec4<f32>>;
// the list's block: [0] its records, [1] its triangles
@group(0) @binding(1) var<storage, read> ls_list: array<u32>;
@group(0) @binding(2) var<storage, read> ls_recs: array<LsRec>;
// the geometry: its vertex and index buffers as words
@group(0) @binding(3) var<storage, read> ls_vtx: array<u32>;
@group(0) @binding(4) var<storage, read> ls_idx: array<u32>;
@group(0) @binding(5) var<storage, read> ls_frame: LsDrawFrame;

// LsOut (the position, the depth along the light's axis 0 near .. 1 far, the
// tile's cell in the atlas in texels, props' uv, material and alpha-tested
// flag, and with the device's clip distances the tile's four sides) and
// `ls_clip_to_tile` are generated (light_shadow.rs `draw_source`).

fn ls_none() -> LsOut {
    var o: LsOut;
    o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    return o;
}

/// The record holding triangle `tri` of the list (the last whose first triangle is at or before it).
fn ls_find(tri: u32) -> u32 {
    let n = min(ls_list[0], LS_REC_CAP);
    var lo = 0u;
    var hi = max(n, 1u) - 1u;
    loop {
        if (lo >= hi) {
            break;
        }
        let mid = (lo + hi + 1u) / 2u;
        if (ls_recs[mid].start <= tri) {
            lo = mid;
        } else {
            hi = mid - 1u;
        }
    }
    return lo;
}

/// The index of corner `v % 3` of triangle `v / 3`, and its record.
fn ls_vertex(v: u32) -> vec2<u32> {
    let tri = v / 3u;
    let r = ls_find(tri);
    let rec = ls_recs[r];
    let local = tri - rec.start;
    return vec2<u32>(u32(i32(ls_idx[rec.first + local * 3u + v % 3u]) + rec.base), r);
}

/// A world point into tile `tile`'s cell of the atlas.
fn ls_project(tile: u32, p: vec3<f32>) -> LsOut {
    var o: LsOut;
    let hdr = ls_table[0];
    let b = 1u + tile * 5u;
    let r0 = ls_table[b];
    let r1 = ls_table[b + 1u];
    let r2 = ls_table[b + 2u];
    let pp = ls_table[b + 3u];
    let dd = ls_table[b + 4u];
    let v = vec3<f32>(dot(r0.xyz, p) + r0.w, dot(r1.xyz, p) + r1.w, dot(r2.xyz, p) + r2.w);
    let cols = hdr.x;
    let rows = hdr.y;
    let cell = vec2<f32>(f32(tile % u32(cols)), f32(tile / u32(cols)));
    let w = v.z;
    // the tile's clip position (its projection; z the usual perspective depth for the clipping)...
    let cx = v.x * pp.x + pp.z * w;
    let cy = v.y * pp.y + pp.w * w;
    let cz = (v.z - dd.x) * dd.y / (dd.y - dd.x);
    // ... into its cell (columns left to right, rows top down).
    o.clip = vec4<f32>(cx / cols + ((2.0 * cell.x + 1.0) / cols - 1.0) * w, cy / rows + (1.0 - (2.0 * cell.y + 1.0) / rows) * w, cz, w);
    // the tile's four sides (a triangle reaching past them is clipped, not drawn over the atlas).
    ls_clip_to_tile(&o, vec4<f32>(w + cx, w - cx, w + cy, w - cy));
    o.lin = (v.z - dd.x) / (dd.y - dd.x);
    o.cell = vec4<f32>(cell * hdr.z, (cell + vec2<f32>(1.0)) * hdr.z);
    return o;
}

fn ls_outside(in: LsIn) -> bool {
    let p = in.clip.xy;
    return p.x < in.cell.x || p.y < in.cell.y || p.x >= in.cell.z || p.y >= in.cell.w;
}

@fragment
fn ls_cast_fs(in: LsIn) -> @builtin(frag_depth) f32 {
    if (ls_outside(in)) {
        discard;
    }
    return clamp(in.lin, 0.0, 1.0);
}

/// The city's objects (scene.rs Vertex: position, normal, colour; 5 words).
@vertex
fn ls_city_vs(@builtin(vertex_index) v: u32) -> LsOut {
    if (v / 3u >= ls_list[1]) {
        return ls_none();
    }
    let iv = ls_vertex(v);
    let w = iv.x * 5u;
    let p = vec3<f32>(bitcast<f32>(ls_vtx[w]), bitcast<f32>(ls_vtx[w + 1u]), bitcast<f32>(ls_vtx[w + 2u]));
    return ls_project(ls_recs[iv.y].tile, p);
}

/// The tiles drawn this frame, each cleared to the far plane (instance: the redraw slot).
@vertex
fn ls_clear_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) r: u32) -> LsOut {
    let lamp = r < LS_LAMP_REDRAWS;
    let used = select(r - LS_LAMP_REDRAWS < ls_frame.head.y, r < ls_frame.head.x, lamp);
    if (!used) {
        return ls_none();
    }
    let hdr = ls_table[0];
    let tile = ls_frame.redraw[r].x;
    let cell = vec2<f32>(f32(tile % u32(hdr.x)), f32(tile / u32(hdr.x)));
    var k = array<vec2<f32>, 6>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
    let c = (cell + k[vi % 6u]) / hdr.xy;
    var o: LsOut;
    o.clip = vec4<f32>(c.x * 2.0 - 1.0, 1.0 - c.y * 2.0, 0.5, 1.0);
    o.lin = 1.0;
    return o;
}

@fragment
fn ls_clear_fs(in: LsIn) -> @builtin(frag_depth) f32 {
    return 1.0;
}
