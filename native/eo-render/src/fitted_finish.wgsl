// Each measured position sees the complete sphere. The narrow and broad
// prefiltered GGX lobes occupy the normal and ORM arrays of one house layer.
// Group numbers are stored by the fitted room's material importer.
const FINISH_RAIL: u32 = 1u;
const FINISH_BOWL: u32 = 2u;
const FINISH_BOARDS: u32 = 3u;
const FINISH_LINEN: u32 = 5u;

fn fitted_probe_uv(r: vec3<f32>) -> vec2<f32> {
    let depth = normalize((vec4<f32>(0.,0.,1.,0.)*look.enclosure_frame).xyz);
    let across = vec3<f32>(depth.z,0.,-depth.x);
    return vec2<f32>(1.) + vec2<f32>(62.,30.) * vec2<f32>(
        .5+atan2(dot(r,across),-dot(r,depth))/(2.*PI),
        .5+asin(clamp(r.y,-1.,1.))/PI);
}
fn fitted_decode(q: vec3<f32>) -> vec3<f32> {
    let square=q*q;
    return square/max(vec3<f32>(.0001),1.-square);
}
fn fitted_probe(uv: vec2<f32>, column: u32, source: u32, layer: i32, tier: u32) -> vec3<f32> {
    let st=(uv+vec2<f32>(f32(column)*64.,f32(source+7u*(tier%2u))*32.))/512.;
    if (tier<2u) { return fitted_decode(textureSampleLevel(kit_normal,kit_sampler,st,layer,0.).rgb); }
    return fitted_decode(textureSampleLevel(kit_orm,kit_sampler,st,layer,0.).rgb);
}
struct FinishRegion {
    columns: vec4<u32>,
    blend: vec2<f32>,
}
fn fitted_region(point: vec3<f32>, group: u32) -> FinishRegion {
    if (group == FINISH_RAIL) {
        let heights=look.fitted_finish[1].xyz;
        let upper=point.y>heights.y;
        let start=select(0u,1u,upper);
        let interval=select(heights.xy,heights.yz,upper);
        let f=clamp((point.y-interval.x)/(interval.y-interval.x),0.,1.);
        return FinishRegion(vec4<u32>(start,start+1u,start,start+1u),vec2<f32>(f,0.));
    }
    if (group == FINISH_BOWL) { return FinishRegion(vec4<u32>(3u),vec2<f32>(0.)); }
    let rect=look.fitted_finish[0];
    let relative=(point.xz-rect.xy)/(rect.zw-rect.xy);
    return FinishRegion(vec4<u32>(4u,5u,6u,7u),clamp((relative-.2)/.6,vec2<f32>(0.),vec2<f32>(1.)));
}
fn fitted_spatial(uv: vec2<f32>, region: FinishRegion, source: u32, layer: i32, tier: u32) -> vec3<f32> {
    let c=region.columns;
    let a=fitted_probe(uv,c.x,source,layer,tier);
    if (c.x==c.y) { return a; }
    let near=mix(a,fitted_probe(uv,c.y,source,layer,tier),region.blend.x);
    if (c.z==c.x) { return near; }
    let far=mix(fitted_probe(uv,c.z,source,layer,tier),fitted_probe(uv,c.w,source,layer,tier),region.blend.x);
    return mix(near,far,region.blend.y);
}
fn fitted_lobe(uv: vec2<f32>, region: FinishRegion, source: u32, layer: i32, rough: f32) -> vec3<f32> {
    let tier=select(select(0u,1u,rough>.20),2u,rough>.38);
    let intervals=array<vec2<f32>,3>(vec2<f32>(.035,.20),vec2<f32>(.20,.38),vec2<f32>(.38,.70));
    let range=intervals[tier];
    let blend=clamp((rough-range.x)/(range.y-range.x),0.,1.);
    if (blend>=1.) { return fitted_spatial(uv,region,source,layer,tier+1u); }
    let low=fitted_spatial(uv,region,source,layer,tier);
    if (blend<=0.) { return low; }
    return mix(low,fitted_spatial(uv,region,source,layer,tier+1u),blend);
}
fn fitted_chart(uv: vec2<f32>, source: u32, layer: i32, linen: bool) -> vec3<f32> {
    if (linen) {
        let at=uv+vec2<f32>(f32(source)*64.,448.);
        return fitted_decode(textureSampleLevel(kit_normal,kit_sampler,at/512.,layer,0.).rgb);
    }
    let origin=vec2<f32>(f32(source%2u)*256., f32(source/2u)*128.);
    return fitted_decode(textureSampleLevel(kit_albedo,kit_sampler,(uv+origin)/512.,layer,0.).rgb);
}
struct FinishSample {
    base: vec3<f32>,
    coat: vec3<f32>,
    diffuse: vec3<f32>,
}
fn fitted_source(source: u32, region: FinishRegion, uv: vec2<f32>, coat_uv: vec2<f32>, chart: vec2<f32>, m: Material, rough: f32) -> FinishSample {
    let layer=i32(m.params.w);
    var sample: FinishSample;
    if (u32(m.thin_absorption) == FINISH_LINEN) {
        sample.diffuse=fitted_chart(chart,source,layer,true);
        sample.base=sample.diffuse;
        return sample;
    }
    sample.base=fitted_lobe(uv,region,source,layer,rough);
    if (m.emissive.w>0.) { sample.coat=fitted_lobe(coat_uv,region,source,layer,m.rough_flatten); }
    if (chart.x>=0.) { sample.diffuse=fitted_chart(chart,source,layer,false); }
    return sample;
}
fn add_finish(a: FinishSample,b: FinishSample,weight: vec3<f32>) -> FinishSample {
    return FinishSample(a.base+b.base*weight,a.coat+b.coat*weight,a.diffuse+b.diffuse*weight);
}
var<private> fitted_coat_light: vec3<f32>;
var<private> fitted_pendant_horizontal: vec3<f32>;
fn fitted_light(point: vec3<f32>, texcoord: vec2<f32>, world: vec3<f32>, v: vec3<f32>, ng: vec3<f32>, s: Surface, m: Material) {
    let group=u32(m.thin_absorption);
    let region=fitted_region(point,group);
    let uv=fitted_probe_uv(reflect(-v,s.n));
    let coat_uv=fitted_probe_uv(reflect(-v,ng));
    var chart=vec2<f32>(-1.);
    if (group == FINISH_BOWL) { chart=vec2<f32>(1.5)+125.*clamp(texcoord,vec2<f32>(0.),vec2<f32>(1.)); }
    if (group == FINISH_LINEN) { chart=vec2<f32>(.75)+62.5*clamp(texcoord,vec2<f32>(0.),vec2<f32>(1.)); }
    if (group == FINISH_BOARDS && ng.y>.001) {
        let rect=look.fitted_finish[0];
        chart=vec2<f32>(129.5,1.5)+125.*clamp((point.xz-rect.xy)/(rect.zw-rect.xy),vec2<f32>(0.),vec2<f32>(1.));
    }
    let sun=look.key_dir.xyz;
    let cloud=sky_cloud_transmittance(world);
    let clear=cloud*smoothstep(0.,.35,sun.y)*(1.-skyu.night.z);
    var light=fitted_source(0u,region,uv,coat_uv,chart,m,s.rough);
    if (clear<1.) {
        let tint=max(lk_sh(vec3<f32>(0.,1.,0.)),vec3<f32>(.00001));
        light=add_finish(light,fitted_source(1u,region,uv,coat_uv,chart,m,s.rough),tint*(look.hemi_sky.w/lk_luma(tint))*(1.-clear));
    }
    if (clear>0.) { light=add_finish(light,fitted_source(2u,region,uv,coat_uv,chart,m,s.rough),look.key.rgb*clear); }
    if (sun.y>0. && cloud>0.) {
        let depth=normalize((vec4<f32>(0.,0.,1.,0.)*look.enclosure_frame).xyz);
        let along=dot(sun,depth);
        let altitude=clamp(asin(clamp(sun.y,0.,1.)),radians(25.),radians(57.));
        let blend=(altitude-radians(25.))/radians(32.);
        let first=select(3u,5u,along>0.);
        let incident=look.key.rgb*cloud*abs(along)/cos(altitude);
        if (blend<1.) { light=add_finish(light,fitted_source(first,region,uv,coat_uv,chart,m,s.rough),incident*(1.-blend)); }
        if (blend>0.) { light=add_finish(light,fitted_source(first+1u,region,uv,coat_uv,chart,m,s.rough),incident*blend); }
    }
    room_radiance=light.base;
    fitted_coat_light=light.coat;
    if (chart.x>=0.) {
        room_diffuse=light.diffuse;
        let to_lamp=normalize(look.dining_lamp.xyz-world);
        let direct=fitted_chart(chart,7u,i32(m.params.w),group == FINISH_LINEN);
        if (group == FINISH_BOARDS) {
            // Board irradiance was measured facing upward. The bevel turns
            // towards the side hemisphere while the direct light follows N.L.
            fitted_pendant_horizontal=direct;
            if (ng.y<.999) {
                let horizontal_n=vec3<f32>(ng.x,0.,ng.z);
                let side=horizontal_n/max(length(horizontal_n),.00001);
                let horizontal=max(to_lamp.y,.01);
                let side_return=max(vec3<f32>(0.),room_irradiance(point,side,world,true,false)
                    -direct*(max(0.,dot(side,to_lamp))/horizontal));
                let vertical_return=max(vec3<f32>(0.),light.diffuse-direct);
                room_diffuse=mix(side_return,vertical_return,ng.y*ng.y)
                    +direct*(max(0.,dot(s.n,to_lamp))/horizontal);
            }
        } else {
            let cosine=dot(ng,to_lamp);
            if (cosine>.02) { fitted_pendant_horizontal=direct*(to_lamp.y/cosine); }
            if (group == FINISH_LINEN && ng.y<0.) {
                room_diffuse=mix(light.diffuse,room_irradiance(point,s.n,world,true,false),-ng.y);
            }
        }
    } else {
        room_diffuse=room_irradiance(point,s.n,world,true,false);
    }
}
