// A pair of horizontally filtered slices stores the shade's visible projected
// solid angle and the previous point's unoccluded inverse-square weight.
fn porch_slice(xz: vec2<f32>, row: i32) -> vec2<f32> {
    let origin = 64. * vec2<f32>(f32(row % 8), f32(row / 8));
    let uv = (origin + xz * 47. + .5) / 512.;
    let texel = textureSampleLevel(kit_orm, kit_sampler, uv, i32(look.porch_light[1].w), 0.);
    let bytes = vec2<f32>(256., 1.) / 257.;
    return vec2<f32>(dot(texel.rg, bytes), dot(texel.ba, bytes));
}

fn porch_irradiance(world: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let lamp = look.porch_light[0];
    if lamp.w <= 0. || dot(world-lamp.xyz, world-lamp.xyz) > lamp.w { return vec3<f32>(0.); }
    let local = (look.enclosure_frame * vec4<f32>(world + .019*normal, 1.)).xyz;
    let pos = (local-look.porch_light[1].xyz) * look.porch_light[2].xyz;
    if any(pos < vec3<f32>(0.)) || any(pos > vec3<f32>(1.)) { return vec3<f32>(0.); }
    let boundary = min(pos, 1.-pos) / look.porch_light[2].xyz;
    let fade = smoothstep(0., .065, min(min(boundary.x,boundary.y),boundary.z));
    let y = 63.*pos.y;
    let row = i32(floor(y));
    let fields = mix(porch_slice(pos.xz,row), porch_slice(pos.xz,min(63,row+1)), fract(y));
    return vec3<f32>(fields * vec2<f32>(look.porch_light[2].w,look.porch_light[3].w) * fade, fade);
}

// The bake excludes the moving leaf. Shadow its current rectangle using the
// same hinge, angle, height and width as the player's collision body.
fn porch_door_visibility(world: vec3<f32>) -> f32 {
    let light = look.porch_light[0].xyz;
    let hinge = look.porch_light[5];
    let leaf = look.porch_light[6];
    let rotation = vec2<f32>(cos(leaf.w), sin(leaf.w));
    let edge = vec2<f32>(dot(leaf.xy,rotation),dot(leaf.yx,rotation*vec2<f32>(1.,-1.)));
    let face = vec2<f32>(-edge.y,edge.x);
    let ray = world-light;
    let denom = dot(ray.xz,face);
    if abs(denom) < .0001 { return 1.; }
    let t = dot(hinge.xz-light.xz,face)/denom;
    if t <= 0. || t >= 1. || (1.-t)*abs(denom) < .039 { return 1.; }
    let p = light+t*ray-hinge.xyz;
    let along = dot(p.xz, edge);
    let inset = min(min(along,leaf.z-along), min(p.y,hinge.w-p.y));
    let penumbra = max(.001,(1.-t)*look.porch_light[4].w);
    return 1.-smoothstep(-penumbra,penumbra,inset);
}

fn porch_lit(base: vec3<f32>, world: vec3<f32>, n: vec3<f32>, v: vec3<f32>,
             diffuse: vec3<f32>, f0: vec3<f32>, rough: f32) -> vec3<f32> {
    let to_light = look.porch_light[0].xyz-world;
    let distance_to_light = max(length(to_light),.001);
    let l = to_light/distance_to_light;
    let previous = look.porch_light[3].xyz-world;
    let old = previous/max(length(previous),.001);
    let tint = look.porch_light[4].rgb;
    let diffuse_brdf = diffuse/LK_PI;
    let measured = max(measured_wall,joinery_diffuse.a);
    let live_diffuse = diffuse_brdf*(1.-porch_measured);
    let remove = tint * live_diffuse * porch_field.y * max(dot(old,n),0.) * measured;
    let beam = porch_field.x * max(dot(n,l),0.) * porch_door_visibility(world);
    let remaining = max(vec3<f32>(0.),base-remove);
    if beam <= 0. { return remaining; }
    let area_rough = min(1.,sqrt(rough*rough + .5*look.porch_light[4].w/distance_to_light));
    let specular = lk_ggx(n,v,l,f0,area_rough);
    return remaining + tint*beam*(live_diffuse+specular);
}

// Unit-albedo receivers follow both side walls and the elliptical soffit.
fn porch_chart(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if n.z < -.7 && abs(p.z-.288) < .035 && p.x>=5.15 && p.x<=6.76 && p.y<=2.72 {
        return vec3<f32>((p.x-5.16)/1.59, p.y/2.70, 0.);
    }
    let depth = (p.z+.85)/1.26;
    if depth>=-.02 && depth<=1.03 && p.y>=0. && p.y<=2.94 {
        let ell = vec2<f32>((p.x-5.955)/.795,(p.y-1.92)/.72);
        if ell.y>=-.03 && abs(length(ell)-1.)<.055 && dot(-ell,n.xy)>.65 {
            return vec3<f32>(atan2(max(0.,ell.y),-ell.x)/LK_PI,depth,3.);
        }
        if p.y<=1.93 && abs(n.y)<.15 {
            if n.x>.8 && abs(p.x-5.16)<.035 { return vec3<f32>(depth,p.y/2.92,1.); }
            if n.x<-.8 && abs(p.x-6.75)<.035 { return vec3<f32>(depth,p.y/2.92,2.); }
        }
    }
    if n.z<-.7 && abs(p.z+.85)<.04 && p.x>=4.93 && p.x<=6.98 && p.y<=2.94 {
        return vec3<f32>((p.x-4.95)/2.01,p.y/2.92,4.);
    }
    return vec3<f32>(0.,0.,-1.);
}

fn porch_radiance(chart: vec3<f32>, row: i32) -> vec3<f32> {
    let index = row*5+i32(chart.z);
    let tile = index%16;
    let cell = 128.*vec2<f32>(f32(tile%4), f32(tile/4));
    let uv = (cell+clamp(chart.xy*128.,vec2<f32>(.5),vec2<f32>(127.5)))/512.;
    let layer = i32(look.porch_light[1].w);
    var value: vec3<f32>;
    if index<16 { value=textureSampleLevel(kit_normal,kit_sampler,uv,layer,0.).rgb; }
    else { value=textureSampleLevel(kit_albedo,kit_sampler,uv,layer,0.).rgb; }
    let encoded=value*value;
    return encoded/max(vec3<f32>(.00001),1.-encoded);
}

fn porch_pigment(chart: vec3<f32>) -> vec4<f32> {
    let id=i32(chart.z);
    let corner=vec2<f32>(256.+64.*f32(id%4),384.+64.*f32(id/4));
    let uv=(corner+clamp(64.*chart.xy,vec2<f32>(.5),vec2<f32>(63.5)))/512.;
    return textureSampleLevel(kit_albedo,kit_sampler,uv,i32(look.porch_light[1].w),0.);
}

fn porch_surface_light(chart: vec3<f32>) -> vec3<f32> {
    let clear = mix(porch_radiance(chart,i32(look.window_sample.y)),
                    porch_radiance(chart,i32(look.window_sample.z)),look.window_sample.w);
    let diffuse_sky=window_weather(clear,porch_radiance(chart,4));
    return diffuse_sky+porch_radiance(chart,5);
}
