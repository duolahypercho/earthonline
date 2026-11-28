// Map cartography rendered into a texture: a land-cover fill (water, parks,
// woods, sand, land) and the city's building footprints and roads, with
// road widths metric when close and a pixel floor when far (the JS city map
// renderer's rule).

struct View {
    // centre x, centre z (metres), scale (px per metre), 0
    view: vec4<f32>,
    // target width, height (px)
    size: vec4<f32>,
    // land-cover texel coordinate = (ax * x + bx, az * z + bz)
    lc: vec4<f32>,
    // land-cover width, height
    lcs: vec4<f32>,
};

struct Palette {
    colors: array<vec4<f32>, 8>,
    // x: half width (m), y: width floor (px, full), z: alpha
    widths: array<vec4<f32>, 8>,
    // land, water, park, wood, sand
    land: array<vec4<f32>, 6>,
};

@group(0) @binding(0) var<uniform> v: View;
@group(0) @binding(1) var landcover: texture_2d<u32>;
@group(1) @binding(0) var<uniform> pal: Palette;

struct FOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_fill(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn class_color(c: u32) -> vec4<f32> {
    switch c {
        case 80u: { return pal.land[1]; }
        case 30u, 40u, 100u: { return pal.land[2]; }
        case 10u, 20u, 90u, 95u: { return pal.land[3]; }
        case 60u: { return pal.land[4]; }
        default: { return pal.land[0]; }
    }
}

fn texel(c: vec2<i32>) -> u32 {
    let n = vec2<i32>(i32(v.lcs.x), i32(v.lcs.y));
    if (c.x < 0 || c.y < 0 || c.x >= n.x || c.y >= n.y) {
        return 0u;
    }
    return textureLoad(landcover, c, 0).r;
}

@fragment
fn fs_fill(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let world = v.view.xy + (frag.xy - v.size.xy * 0.5) / v.view.z;
    let f = vec2<f32>(v.lc.x * world.x + v.lc.y, v.lc.z * world.y + v.lc.w) - vec2<f32>(0.5, 0.5);
    let c0 = vec2<i32>(floor(f));
    let t = f - floor(f);
    // Bilinear blend of the four cells' colours, sharpened so a 10 m cell
    // edge reads as a shoreline rather than a blur.
    let s = smoothstep(vec2<f32>(0.3), vec2<f32>(0.7), t);
    let a = mix(class_color(texel(c0)), class_color(texel(c0 + vec2<i32>(1, 0))), s.x);
    let b = mix(class_color(texel(c0 + vec2<i32>(0, 1))), class_color(texel(c0 + vec2<i32>(1, 1))), s.x);
    return mix(a, b, s.y);
}

@vertex
fn vs_mesh(@location(0) pos: vec2<f32>, @location(1) dir: vec2<f32>, @location(2) ext: vec2<f32>, @location(3) kind: u32) -> FOut {
    var o: FOut;
    // Bit 3 marks an underground road. Its class still sets its width and colour.
    let road_class = kind & 7u;
    let w = pal.widths[road_class];
    let hw = max(w.x, 0.5 * w.y / v.view.z);
    let perp = vec2<f32>(-dir.y, dir.x);
    let p = pos + dir * ext.x * hw + perp * ext.y * hw;
    let d = (p - v.view.xy) * v.view.z / (v.size.xy * 0.5);
    o.clip = vec4<f32>(d.x, -d.y, 0.0, 1.0);
    let c = pal.colors[road_class];
    let a = c.a * w.z * select(1.0, 0.6, (kind & 8u) != 0u);
    o.color = vec4<f32>(c.rgb * a, a);
    return o;
}

@fragment
fn fs_mesh(i: FOut) -> @location(0) vec4<f32> {
    return i.color;
}
