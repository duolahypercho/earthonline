// The interface's one pipeline: rounded rectangles (SDF), gradients
// (vertex colours), glyphs (R8 atlas), images (with an optional projective
// coordinate), the radar plate (ground under a pinhole camera) and ellipses.
// Colours arrive premultiplied, in sRGB as CSS authors them.

struct Globals {
    // w, h, 1 when the target encodes sRGB itself (then we output linear), 0
    screen: vec4<f32>,
    // player x, z, forward x, forward z (world metres, x east, z south)
    radar0: vec4<f32>,
    // focal px, camera height m, camera setback m, image opacity
    radar1: vec4<f32>,
    // sin pitch, cos pitch, image origin x, image origin z (metres)
    radar2: vec4<f32>,
    // 1 / image span x, 1 / image span z (metres), 0, 0
    radar3: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var image: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct VIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) local: vec2<f32>,
    @location(3) half_size: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) radius: f32,
    @location(6) q: f32,
    @location(7) mode: u32,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) half_size: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) @interpolate(flat) radius: f32,
    @location(5) q: f32,
    @location(6) @interpolate(flat) mode: u32,
};

@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    let ndc = vec2<f32>(v.pos.x / g.screen.x * 2.0 - 1.0, 1.0 - v.pos.y / g.screen.y * 2.0);
    o.clip = vec4<f32>(ndc, 0.0, 1.0);
    o.uv = v.uv;
    o.local = v.local;
    o.half_size = v.half_size;
    o.color = v.color;
    o.radius = v.radius;
    o.q = v.q;
    o.mode = v.mode;
    return o;
}

fn rrect(p: vec2<f32>, h: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - h + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn coverage(i: VOut) -> f32 {
    if (i.half_size.x <= 0.0) {
        return 1.0;
    }
    let d = rrect(i.local, i.half_size, i.radius);
    return clamp(0.5 - d, 0.0, 1.0);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    var c = vec4<f32>(0.0);
    switch i.mode {
        case 1u: {
            // glyph: uv in atlas texels
            let a = textureLoad(atlas, vec2<i32>(i.uv), 0).r;
            c = i.color * a;
        }
        case 2u: {
            // image, uv already premultiplied by q for a projective quad
            let t = textureSampleLevel(image, samp, i.uv / i.q, 0.0);
            c = t * i.color.a * coverage(i);
        }
        case 3u: {
            // radar: the plate pixel (local, y down, from the plate centre)
            // back-projected onto the ground under the map camera.
            let a = i.local.x / g.radar1.x;
            let b = i.local.y / g.radar1.x;
            let sp = g.radar2.x;
            let cp = g.radar2.y;
            let denom = max(sp + b * cp, 1e-3);
            let t = g.radar1.y / denom;
            let u = t * a;
            let d = -g.radar1.z + t * (cp - b * sp);
            let fwd = g.radar0.zw;
            let right = vec2<f32>(-fwd.y, fwd.x);
            let w = g.radar0.xy + right * u + fwd * d;
            let uv = (w - g.radar2.zw) * g.radar3.xy;
            let tex = textureSampleLevel(image, samp, uv, 0.0);
            c = tex * g.radar1.w * i.color.a * coverage(i);
        }
        case 4u: {
            // ellipse (radius > 0: a ring of that width)
            let n = i.local / i.half_size;
            let k = length(n);
            let grad = length(n / i.half_size) / max(k, 1e-4);
            let d = (k - 1.0) / max(grad, 1e-4);
            var cov = clamp(0.5 - d, 0.0, 1.0);
            if (i.radius > 0.0) {
                cov = cov * clamp(0.5 + d + i.radius, 0.0, 1.0);
            }
            c = i.color * cov;
        }
        default: {
            c = i.color * coverage(i);
        }
    }
    if (g.screen.z > 0.5 && c.a > 0.0) {
        c = vec4<f32>(to_linear(c.rgb / c.a) * c.a, c.a);
    }
    return c;
}
