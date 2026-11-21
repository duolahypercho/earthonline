// In the reflected direction, intersect the surfaces measured in Blender.
// Tree nodes and triangle records are in preorder so rays need no local stack.
// (group 1 binding 21, look.rs `B_WINDOW_OPTICS`: after the lighting additions' 12-19 and the atmosphere view's 20)
@group(1) @binding(21) var<storage, read> window_optics: array<vec4<u32>>;

fn window_hdr(p: u32) -> vec3<f32> {
    return vec3<f32>(vec3<u32>(p&511u,(p>>9u)&511u,(p>>18u)&511u))*exp2(f32(p>>27u)-24.);
}

fn window_vertex(index:u32) -> vec3<f32> {
    return bitcast<vec3<f32>>(window_optics[window_optics[0].y+2u*index].xyz);
}

fn window_weather(clear:vec3<f32>, uniform_sky:vec3<f32>) -> vec3<f32> {
    return mix(clear,uniform_sky*look.wall_overcast.rgb,look.wall_sample.z*(1.-skyu.night.z));
}

fn window_vertex_light(index:u32) -> vec3<f32> {
    let addr=window_optics[0].y+2u*index;
    let b=window_optics[addr+1u];
    let timed=vec4<u32>(window_optics[addr].w,b.xyz);
    let a=window_hdr(timed[u32(look.window_sample.y)]);
    let c=window_hdr(timed[u32(look.window_sample.z)]);
    return window_weather(mix(a,c,look.window_sample.w),window_hdr(b.w));
}

fn window_sky_pixel(p:vec2<u32>, layer:u32) -> vec3<f32> {
    let id=160u*(80u*layer+p.y)+p.x;
    return window_hdr(window_optics[window_optics[0].z+(id>>2u)][id&3u]);
}

fn window_clear_sky(uv:vec2<f32>, layer:u32) -> vec3<f32> {
    let grid=vec2<f32>(160.,80.)*uv-0.5;
    let ij=vec2<i32>(floor(grid));
    let x=u32((ij.x+160)%160);let y=u32(clamp(ij.y,0,79));
    let xx=(x+1u)%160u;let yy=u32(clamp(ij.y+1,0,79));
    let f=fract(grid);
    return mix(mix(window_sky_pixel(vec2<u32>(x,y),layer),window_sky_pixel(vec2<u32>(xx,y),layer),f.x),
               mix(window_sky_pixel(vec2<u32>(x,yy),layer),window_sky_pixel(vec2<u32>(xx,yy),layer),f.x),f.y);
}

fn window_incident_sky(d:vec3<f32>) -> vec3<f32> {
    let b=normalize(vec3<f32>(dot(bitcast<vec3<f32>>(window_optics[3].xyz),d),
        dot(bitcast<vec3<f32>>(window_optics[4].xyz),d),dot(bitcast<vec3<f32>>(window_optics[5].xyz),d)));
    let uv=vec2<f32>(.5+atan2(b.x,b.y)/(2.*PI),.5+asin(clamp(b.z,-1.,1.))/PI);
    return window_weather(mix(window_clear_sky(uv,u32(look.window_sample.y)),
        window_clear_sky(uv,u32(look.window_sample.z)),look.window_sample.w),vec3<f32>(1.));
}

fn window_trace(start:vec3<f32>, ray:vec3<f32>, limit:f32, finish:u32) -> vec4<f32> {
    let o=(look.enclosure_frame*vec4<f32>(start,1.)).xyz;
    let d=(look.enclosure_frame*vec4<f32>(ray,0.)).xyz;
    let step=bitcast<vec3<f32>>(window_optics[2].xyz);
    let q=(o-bitcast<vec3<f32>>(window_optics[1].xyz))/step;
    let inv=step/select(d,select(vec3<f32>(1e-12),vec3<f32>(-1e-12),d<vec3<f32>(0.)),abs(d)<vec3<f32>(1e-12));
    let end=window_optics[0].y;
    var at=7u;
    var best=limit;
    var hit=vec3<u32>(0u);
    var weights=vec3<f32>(0.);
    while at<end {
        let node=window_optics[at];
        let escape=node.w&0x7fffffffu;
        let first=(vec3<f32>(node.xyz&vec3<u32>(65535u))-q)*inv;
        let last=(vec3<f32>(node.xyz>>vec3<u32>(16u))-q)*inv;
        let lo=min(first,last);let hi=max(first,last);
        if max(max(lo.x,lo.y),max(lo.z,0.))>min(min(hi.x,hi.y),min(hi.z,best)) {
            at=escape;continue;
        }
        at++;
        if (node.w&0x80000000u)==0u {continue;}
        for (;at<escape;at++) {
            let record=window_optics[at];
            if (finish != 0u && (record.w >> 28u) != finish) ||
                (look.wall_sample.w > .5 && (record.w & 0x08000000u) != 0u) { continue; }
            let tri=record.xyz;
            let a=window_vertex(tri.x);let e=window_vertex(tri.y)-a;let f=window_vertex(tri.z)-a;
            let h=cross(d,f);let det=dot(e,h);
            if abs(det)<1e-10 {continue;}
            let delta=o-a;let u=dot(delta,h)/det;
            if u<0. || u>1. {continue;}
            let k=cross(delta,e);let v=dot(d,k)/det;let t=dot(f,k)/det;
            if v>=0. && u+v<=1. && t>.0002 && t<best {
                best=t;hit=tri;weights=vec3<f32>(1.-u-v,u,v);
            }
        }
    }
    if best==limit {
        if finish!=0u {return vec4<f32>(0.);}
        return vec4<f32>(window_incident_sky(d),0.);
    }
    return vec4<f32>(weights.x*window_vertex_light(hit.x)+weights.y*window_vertex_light(hit.y)+weights.z*window_vertex_light(hit.z),1.);

}
