// The first seven rows are shared with the city's Look uniform. The opaque
// output is scene-linear; the existing water pass transports it to the eye.
struct Light { dir:vec4f, key:vec4f, rim_dir:vec4f, rim:vec4f, sky:vec4f, ground:vec4f, ambient:vec4f };
struct View { vp:mat4x4f, eye_time:vec4f };
@group(0) @binding(0) var<uniform> view:View;
@group(1) @binding(0) var<uniform> light:Light;
struct Out {
    @builtin(position) clip:vec4f,
    @location(0) world:vec3f,
    @location(1) normal:vec3f,
    @location(2) color:vec3f,
    @location(3) local:vec3f,
    @location(4) @interpolate(flat) kind:u32,
};
fn rotate_xz(p:vec3f,a:f32)->vec3f {
    let c=cos(a);let s=sin(a);
    return vec3f(c*p.x-s*p.z,p.y,s*p.x+c*p.z);
}
@vertex fn vs(@location(0) pos:vec3f,@location(1) normal:vec3f,@location(2) color:vec4f,
              @location(3) at:vec4f,@location(4) size:vec4f,@location(5) traits:vec4f)->Out{
    let kind=u32(traits.x+.5);
    var loc=pos;let t=view.eye_time.w;
    var root=at.xyz;
    var yaw=at.w;
    var norm=normal;
    if kind<=2u || kind==12u {
        let tail=pow(clamp((.34-loc.x)/.84,0.,1.),2.);
        let beat=sin(t*select(10.1,3.3,kind>=2u)+size.w-5.7*loc.x);
        loc.z+=beat*tail*select(.083,.029,kind>=2u);
        if kind<2u {
            // Neighbours follow one orbit, with individual body beats and spacing.
            let lap=.37*t+traits.y;
            root+=rotate_xz(vec3f(cos(lap)*traits.z,.04*sin(lap*2.+size.w),.62*sin(lap)*traits.z),at.w);
            yaw+=atan2(.62*cos(lap),-sin(lap));
        }
    } else if (kind>=3u && kind<=6u) || kind==11u || kind==13u {
        let height=clamp(pos.y,0.,1.2);
        let p=at.xz;
        let surge=sin(t*1.16+.34*p.x+.19*p.y)+.32*sin(t*1.83-.15*p.x+.22*p.y+pos.y*1.7);
        let bend=height*height;
        loc.x+=bend*(.09+.095*surge);loc.z+=bend*.057*sin(t*.96+.20*p.y+size.w*.13);
        norm=normalize(normal+vec3f(-.08*height*surge,0.,0.));
    }
    let world=rotate_xz(loc*size.xyz,yaw)+root;
    var out:Out;out.clip=view.vp*vec4f(world,1.);out.world=world;
    out.normal=rotate_xz(norm/max(size.xyz,vec3f(.001)),yaw);out.color=color.rgb*traits.w;out.kind=kind;out.local=pos;
    return out;
}
fn grain(p:vec3f)->f32{
    var a=fract(p*vec3f(.1081,.1037,.0953));a+=dot(a,a.yzx+19.31);
    return fract((a.x+a.y)*a.z);
}
@fragment fn fs(i:Out,@builtin(front_facing) front:bool)->@location(0) vec4f {
    let eye=view.eye_time.xyz;let d=distance(i.world,eye);
    let up=normalize(cross(dpdy(i.world),dpdx(i.world))+vec3f(0.,.00000001,0.));
    let footprint=max(length(dpdx(i.world)),length(dpdy(i.world)));
    var n=normalize(i.normal+vec3f(0.,.000001,0.));var alb=i.color;
    let v=normalize(eye-i.world);let sun=light.dir.xyz;
    if i.kind==14u {
        // Fine sediment and asymmetric crests use world metres. An irregular
        // phase and patches of quiet mud break up the wave trains.
        let p=i.world.xz;
        let mud=.5+.5*sin(.44*p.x+.25*p.y+.89*sin(.25*p.y));
        let phase=dot(p,vec2f(13.,7.5))+1.13*sin(.86*p.y)+.56*sin(1.38*p.x-.52*p.y);
        let crest=sin(phase)+.22*sin(2.*phase+.8);
        let relief=(.038+.12*mud)*mix(.6,1.,i.color.g)*exp(-18.*footprint);
        n=normalize(select(-up,up,up.y>=0.)+vec3f(-.89,0.,-.49)*crest*relief);
        let sand=mix(vec3f(.16,.145,.098),vec3f(.34,.291,.183),clamp(i.color.g*.60+i.color.b*.29,0.,1.));
        let grain_band=exp(-78.*footprint);
        alb=sand*(.76+.22*mud+.10*crest*relief*4.+.36*grain_band*(grain(floor(i.world*290.))-.5));
        if i.color.r>.5 && d>7. && grain(floor(i.world*21.))<smoothstep(7.,15.,d){discard;}
    } else {
        if d>10. && grain(floor(i.world*74.))<smoothstep(10.,15.,d){discard;}
        if i.kind>=7u && i.kind<=10u {
            let p=i.local;
            alb*=.86+.10*sin(20.*p.x+6.*p.y+sin(12.*p.z))+.10*grain(floor(p*60.))*exp(-25.*footprint);
        }
    }
    if !front && i.kind!=14u {n=-n;}
    let lambert=max(dot(n,sun),0.);
    let ambient=mix(light.ground.rgb,light.sky.rgb,clamp(.52+.46*n.y,0.,1.))+light.ambient.rgb;
    var radiance=alb*(ambient+light.key.rgb*lambert)/3.14159;
    if i.kind<=1u {
        let h=normalize(sun+v);radiance+=light.key.rgb*.11*pow(max(dot(n,h),0.),24.)*(.32+.68*alb);
    }
    if (i.kind>=3u && i.kind<=6u) || i.kind==11u {
        radiance+=alb*light.key.rgb*.095*max(-dot(n,sun),0.);
    }
    return vec4f(max(radiance,vec3f(0.)),1.);
}
