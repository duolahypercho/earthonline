@group(2) @binding(0) var reef_colour:texture_2d_array<f32>;
@group(2) @binding(1) var reef_normals:texture_2d_array<f32>;
@group(2) @binding(2) var reef_arm:texture_2d_array<f32>;
@group(2) @binding(3) var reef_filter:sampler;

struct ReefTap { colour:vec3f, slope:vec2f, ao:f32, rough:f32 };

fn reef_tap(p:vec2f, dx:vec2f, dy:vec2f, layer:i32)->ReefTap {
    let size=SCAN_METRES[u32(layer)];
    let uv=p/size;
    let a=textureSampleGrad(reef_arm,reef_filter,uv,layer,dx/size,dy/size).rg;
    let n=textureSampleGrad(reef_normals,reef_filter,uv,layer,dx/size,dy/size).xyz*2.-1.;
    return ReefTap(textureSampleGrad(reef_colour,reef_filter,uv,layer,dx/size,dy/size).rgb,
                   n.xy/max(n.z,0.30),a.r,a.g);
}

fn reef_material(kind:u32, p:vec3f, local:vec3f, dx:vec3f, dy:vec3f, base:ReefSurface, beach:f32)->ReefSurface {
    var m=base;
    if kind==255u {
        // Slowly varying depositional patches expose coarse shell grit through
        // the finer sand. Both scans stay at their physical sizes as the eye moves.
        let sediment=stone_noise(vec3f(p.xz*0.43,1.8));
        let deposit=smoothstep(0.23,0.72,sediment + 0.10*(1.-beach));
        let coarse=reef_tap(p.xz,dx.xz,dy.xz,1);
        let fine=reef_tap(vec2f(-p.z,p.x),vec2f(-dx.z,dx.x),vec2f(-dy.z,dy.x),2);
        m.colour=mix(fine.colour,coarse.colour,deposit);
        let slope=mix(vec2f(fine.slope.y,-fine.slope.x),coarse.slope,deposit);
        m.n=normalize(m.n + vec3f(slope.x,0.,slope.y));
        m.ao=mix(fine.ao,coarse.ao,deposit);
        m.rough=mix(fine.rough,coarse.rough,deposit);
    } else {
        // Triplanar projection at a fixed scale keeps fracture faces and their
        // inclusions continuous around boulders without stretching steep sides.
        let n=m.n;
        let power=pow(abs(n),vec3f(5.)); let weights=power/max(dot(power,vec3f(1.)),0.001);
        let x=reef_tap(p.zy,dx.zy,dy.zy,0);
        let y=reef_tap(p.xz,dx.xz,dy.xz,0);
        let z=reef_tap(p.xy,dx.xy,dy.xy,0);
        let colour=x.colour*weights.x+y.colour*weights.y+z.colour*weights.z;
        let bump=vec3f(0.,x.slope.y,x.slope.x)*weights.x
               + vec3f(y.slope.x,0.,y.slope.y)*weights.y + vec3f(z.slope.x,z.slope.y,0.)*weights.z;
        m.n=normalize(n + bump - n*dot(bump,n));
        // Stone darkens where its rough mineral surface holds biofilm. Broad
        // encrusting patches survive on the sides; the upper faces collect fines.
        let colonies=stone_noise(p*1.3+vec3f(2.,8.,3.));
        let crust=smoothstep(0.48,0.72,colonies-0.15*n.y);
        m.colour=colour*mix(vec3f(1.34,1.29,1.24),vec3f(0.93,0.62,0.67),crust);
        // Two intersecting, finite joints in the rock's frame. Their chipped
        // lips displace the shading normal over centimetres, by the same amount
        // on a small stone as on a large ledge.
        let chip=stone_noise(p*8.)-0.5;
        let a=dot(local,vec3f(0.76,0.24,-0.60))-0.19 + 0.035*chip;
        let b=dot(local,vec3f(-0.32,0.84,0.44))+0.09 + 0.045*chip;
        let pixel=max(length(dx),length(dy));
        let opening=0.012+pixel;
        let seam_a=(1.-smoothstep(0.,opening,abs(a))) * (1.-smoothstep(0.28,0.95,abs(local.y+0.36*local.x)));
        let seam_b=(1.-smoothstep(0.,opening*0.8,abs(b))) * smoothstep(-0.18,0.32,a);
        let seam=max(seam_a,seam_b)*0.012/opening;
        m.colour*=1.-0.42*seam;
        let relief=-0.005*seam;
        let tx=cross(dy,n); let ty=cross(n,dx); let det=dot(dx,tx);
        m.n=normalize(m.n-sign(det)*(dpdx(relief)*tx+dpdy(relief)*ty)/max(abs(det),1e-8));
        m.ao=dot(weights,vec3f(x.ao,y.ao,z.ao));
        m.rough=dot(weights,vec3f(x.rough,y.rough,z.rough));
    }
    return m;
}
