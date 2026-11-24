// The first seven rows are shared with the city's Look uniform. The opaque
// output is scene-linear; the existing water pass transports it to the eye.
struct Light { dir:vec4f, key:vec4f, rim_dir:vec4f, rim:vec4f, sky:vec4f, ground:vec4f, ambient:vec4f, environment:vec4f, sh:array<vec4f,9> };
// The same cosine-convolved sky coefficients used by the water lighting.
fn reef_sky(n:vec3f)->vec3f {
    let x=n.x; let y=n.y; let z=n.z;
    return max(vec3f(0.), light.sh[0].xyz*0.282095
        + 0.488603*(light.sh[1].xyz*y + light.sh[2].xyz*z + light.sh[3].xyz*x)
        + 1.092548*(light.sh[4].xyz*x*y + light.sh[5].xyz*y*z + light.sh[7].xyz*x*z)
        + light.sh[6].xyz*(0.315392*(3.*z*z-1.)) + light.sh[8].xyz*(0.546274*(x*x-y*y)));
}
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
    var surface_coord=pos;
    if kind<=2u || kind==12u || kind==20u {
        let tail=pow(clamp((.34-loc.x)/.84,0.,1.),2.);
        let beat=sin(t*select(10.1,3.3,(kind==2u || kind==12u))+size.w-5.7*loc.x);
        loc.z+=beat*tail*select(.083,.029,(kind==2u || kind==12u));
        if kind<2u || kind==20u {
            // Neighbours follow one orbit, with individual body beats and spacing.
            var lap=.37*t+traits.y;
            if DIVE_DETAIL {
                // Neighbours keep the same school orbit with individual lag and heave.
                let phase = 0.24 * sin(size.w) + 0.10 * sin(0.71*t + size.w);
                lap += phase;
                let heave = 0.075*sin(lap*1.8 + 0.55*size.w) + 0.027*sin(1.43*t + size.w);
                root.y += heave;
                let pitch = 0.16*cos(lap*1.8 + 0.55*size.w);
                let c = cos(pitch); let s = sin(pitch);
                loc = vec3f(c*loc.x - s*loc.y, s*loc.x + c*loc.y, loc.z);
                norm = vec3f(c*norm.x - s*norm.y, s*norm.x + c*norm.y, norm.z);
            }
            root+=rotate_xz(vec3f(cos(lap)*traits.z,.04*sin(lap*2.+size.w),.62*sin(lap)*traits.z),at.w);
            yaw+=atan2(.62*cos(lap),-sin(lap));
        }
    } else if (kind>=3u && kind<=6u) || kind==11u || kind==13u || kind==19u {
        let height=clamp(pos.y,0.,1.2);
        let p=at.xz;
        let surge=sin(t*1.16+.34*p.x+.19*p.y)+.32*sin(t*1.83-.15*p.x+.22*p.y+pos.y*1.7);
        let bend=height*height;
        loc.x+=bend*(.09+.095*surge);loc.z+=bend*.057*sin(t*.96+.20*p.y+size.w*.13);
        norm=normalize(normal+vec3f(-.08*height*surge,0.,0.));
        if (DIVE_DETAIL && kind==19u) || (COASTAL_HABITAT && (kind==3u || kind==11u)) {
            // Alpha stores distance along an individual blade. Stipes, bladders
            // and the attached end have zero weight. Short waves lag toward the tip.
            let coord = u32(round(color.a*255.));
            let along = f32(coord >> 4u) / 15.;
            if COASTAL_HABITAT && kind==11u {
                let h = clamp(pos.y,0.,1.04);
                let bow = 0.088*sin(4.3*h + 0.26*t + size.w) + 0.054*cos(2.8*h + size.w);
                loc += h*vec3f(bow, -0.23*abs(bow)*h, 0.056*sin(4.7*h + size.w));
            }
            let across = (f32(coord & 15u) - 8.) / 7.;
            surface_coord = vec3f(along, across, select(0., 1., coord != 0u));
            let phase = t*1.48 + size.w + 4.6*pos.y + 2.9*pos.z - 2.8*along;
            let bend_tip = sin(phase)*along*along;
            var flutter = vec3f(0.014*bend_tip, 0.022*bend_tip, 0.017*cos(phase)*along*along);
            if COASTAL_HABITAT {
                // A wave runs from the attachment along each free ribbon.
                // Let the clump trail the same world-space current at any yaw.
                let travel = dot(p,vec2f(cos(SWELL_DIR),sin(SWELL_DIR)));
                let wave = sin(t*SWELL_OMEGA - 0.029*travel - 1.2*along);
                flutter += rotate_xz(vec3f(cos(SWELL_DIR),0.,sin(SWELL_DIR)),-yaw)
                    * (0.083*wave*height*height + 0.037*wave*along*along);
            }
            loc += flutter;
            norm = normalize(norm + vec3f(0., -0.12*bend_tip, 0.07*bend_tip));
        }
    }
    if COASTAL_HABITAT && kind==255u { surface_coord.z = color.a; }
    let world=rotate_xz(loc*size.xyz,yaw)+root;
    var out:Out;out.clip=view.vp*vec4f(world,1.);out.world=world;
    if REEF_ROCKS || CANOPY_LIGHT { out.world=world-view.eye_time.xyz; }
    out.normal=rotate_xz(norm/max(size.xyz,vec3f(.001)),yaw);out.color=color.rgb*traits.w;out.kind=kind;out.local=surface_coord;
    return out;
}
fn grain(p:vec3f)->f32{
    var a=fract(p*vec3f(.1081,.1037,.0953));a+=dot(a,a.yzx+19.31);
    return fract((a.x+a.y)*a.z);
}
// Smooth mineral variation in object space avoids flickering where the stone
// recedes below a pixel. The fracture faces keep their own geometric normals.
fn stone_noise(p:vec3f)->f32 {
    let cell=floor(p); let v=fract(p); let u=v*v*(3.-2.*v);
    let a=mix(grain(cell),grain(cell+vec3f(1,0,0)),u.x);
    let b=mix(grain(cell+vec3f(0,1,0)),grain(cell+vec3f(1,1,0)),u.x);
    let c=mix(grain(cell+vec3f(0,0,1)),grain(cell+vec3f(1,0,1)),u.x);
    let e=mix(grain(cell+vec3f(0,1,1)),grain(cell+vec3f(1,1,1)),u.x);
    return mix(mix(a,b,u.y),mix(c,e,u.y),u.z);
}
@fragment fn fs(i:Out,@builtin(front_facing) front:bool)->@location(0) vec4f {
    let eye=view.eye_time.xyz;
    // Interpolating positions relative to the swimmer preserves derivatives
    // on small rock faces several kilometres from the city origin.
    let world=select(i.world, i.world+eye, REEF_ROCKS || CANOPY_LIGHT);
    let to_eye=select(eye-i.world, -i.world, REEF_ROCKS || CANOPY_LIGHT);
    let d=length(to_eye);
    let up=normalize(cross(dpdy(i.world),dpdx(i.world))+vec3f(0.,.00000001,0.));
    let footprint=max(length(dpdx(i.world)),length(dpdy(i.world)));
    var n=normalize(i.normal+vec3f(0.,.000001,0.));var alb=i.color;
    let v=normalize(to_eye);var sun=light.dir.xyz;
    if CANOPY_LIGHT { sun=-refract(-sun,vec3f(0.,1.,0.),1./1.333); }
    if i.kind==255u {
        // Fine sediment and asymmetric crests use world metres. An irregular
        // phase and patches of quiet mud break up the wave trains.
        let p=world.xz;
        let mud=.5+.5*sin(.44*p.x+.25*p.y+.89*sin(.25*p.y));
        let phase=dot(p,vec2f(13.,7.5))+1.13*sin(.86*p.y)+.56*sin(1.38*p.x-.52*p.y);
        let crest=sin(phase)+.22*sin(2.*phase+.8);
        let relief=(.038+.12*mud)*mix(.6,1.,i.color.g)*exp(-18.*footprint);
        n=normalize(select(-up,up,up.y>=0.)+vec3f(-.89,0.,-.49)*crest*relief);
        let sand=mix(vec3f(.16,.145,.098),vec3f(.34,.291,.183),clamp(i.color.g*.60+i.color.b*.29,0.,1.));
        let grain_band=exp(-78.*footprint);
        alb=sand*(.76+.22*mud+.10*crest*relief*4.+.36*grain_band*(grain(floor(world*290.))-.5));
        if i.color.r>.5 && d>7. && grain(floor(world*21.))<smoothstep(7.,15.,d){discard;}
    } else {
        if d>10. && grain(floor(world*74.))<smoothstep(10.,15.,d){discard;}
        if (i.kind>=7u && i.kind<=10u) || i.kind==14u || i.kind==15u {
            let p=i.local;
            alb*=.86+.10*sin(20.*p.x+6.*p.y+sin(12.*p.z))+.10*grain(floor(p*60.))*exp(-25.*footprint);
        }
    }
    if REEF_ROCKS && ((i.kind>=7u && i.kind<=9u) || i.kind==14u || i.kind==15u || i.kind==24u || i.kind==25u) {
        let p=i.local;
        let inclusion=stone_noise(p*5.6);
        let cleft=stone_noise(p*vec3f(5.,17.,6.)+inclusion*2.8);
        let pit=stone_noise(p*27.);
        let growth=smoothstep(0.47,0.80,inclusion+0.17*n.y-0.08*cleft);
        let weathered=mix(alb,vec3f(0.13,0.075,0.087),0.65*growth);
        alb=weathered*(0.62+0.51*cleft+0.31*pit);
        let relief=0.013*cleft + 0.004*pit;
        let px=dpdx(i.world); let py=dpdy(i.world);
        let tx=cross(py,n); let ty=cross(n,px); let det=dot(px,tx);
        n=normalize(n-sign(det)*(dpdx(relief)*tx+dpdy(relief)*ty)/max(abs(det),1e-8));
    }
    if (DIVE_DETAIL && i.kind==19u) || (COASTAL_HABITAT && i.kind==11u) {
        // Midrib and lengthwise corrugations track the blade as it flexes.
        let t = i.local.x;
        let side = i.local.y;
        let midrib = exp(-18.*abs(side))*(1.-t);
        let phase = 25.*side + 3.8*sin(5.*t+2.7*side);
        let resolved = 1.-smoothstep(0.3,2.,fwidth(phase));
        let groove = 0.5 + 0.5*cos(phase);
        let tissue = alb * select(vec3f(1.19,0.84,0.72),vec3f(1.05,0.99,0.93),COASTAL_HABITAT) * (0.94 + 0.12*groove*resolved)
            + vec3f(0.092,0.036,0.010)*midrib;
        alb = mix(alb,tissue,i.local.z);
    }
    if DIVE_DETAIL && i.kind==20u {
        let belly = 1.-smoothstep(-0.10,0.08,i.local.y);
        alb *= mix(vec3f(0.29,0.41,0.43),vec3f(1.0),belly);
        let eye_dist = length(i.local.xy - vec2f(0.29,0.034));
        alb *= mix(0.11,1.,smoothstep(0.021,0.041,eye_dist));
    }
    if COASTAL_HABITAT && i.kind==11u {
        let t=i.local.x;
        let side=i.local.y;
        let fold=13.*t+2.7*side+1.5*sin(4.8*t+4.*side);
        let tissue=0.5+0.5*sin(21.*t+3.3*side+1.85*sin(fold));
        let resolved=1.-smoothstep(0.37,2.1,fwidth(fold));
        // Smooth millimetre-scale corrugations perturb the lit normal, while
        // irregular amber and olive pigment varies independently across it.
        let relief=0.0018*sin(fold)*resolved*i.local.z;
        let px=dpdx(i.world); let py=dpdy(i.world);
        let tx=cross(py,n); let ty=cross(n,px);
        let det=dot(px,tx);
        n=normalize(n-sign(det)*(dpdx(relief)*tx+dpdy(relief)*ty)/max(abs(det),1.e-9));
        let pigment=mix(vec3f(0.80,0.89,0.75),vec3f(1.07,1.0,0.95),tissue);
        alb*=mix(vec3f(1.),pigment,resolved*i.local.z);
    }
    if COASTAL_HABITAT && i.kind==3u {
        let along=i.local.x; let across=abs(i.local.y);
        let rib=exp(-8.*across);
        let fibre=0.5+0.5*sin(23.*across+2.3*sin(5.*along));
        let resolved=1.-smoothstep(0.05,0.23,fwidth(i.local.y));
        alb *= 0.91+0.12*rib + 0.07*fibre*resolved;
    }
    if i.kind==19u && !DIVE_DETAIL {
        // Midrib and ruffled margins catch light differently as a frond turns.
        alb*=0.87+0.13*sin(i.local.y*70.+i.local.x*26.);
    }
    if i.kind>=16u && i.kind<=18u {
        alb*=0.84+0.25*grain(floor(i.local*64.))*exp(-8.*footprint);
    }
    if !front && i.kind!=255u {n=-n;}
    let lambert=max(dot(n,sun),0.);
    var ambient=mix(light.ground.rgb,light.sky.rgb,clamp(.52+.46*n.y,0.,1.))+light.ambient.rgb;
    if CANOPY_LIGHT {
        ambient += reef_sky(n)*light.ambient.w*light.environment.w;
    }
    if COASTAL_HABITAT && i.kind==255u { ambient *= i.local.z; }
    var radiance=alb*(ambient+light.key.rgb*lambert)/3.14159;
    if i.kind<=1u || i.kind==20u {
        let h=normalize(sun+v);radiance+=light.key.rgb*.11*pow(max(dot(n,h),0.),24.)*(.32+.68*alb);
    }
    if (i.kind>=3u && i.kind<=6u) || i.kind==11u || i.kind==19u {
        radiance+=alb*light.key.rgb*.095*max(-dot(n,sun),0.);
    }
    if (DIVE_DETAIL && i.kind==19u) || (COASTAL_HABITAT && i.kind==11u) {
        // A thin lamina receives diffuse sky on both faces. Its backlit warm
        // pigment goes through the same water transport as its reflected light.
        let backside = max(-dot(n,sun), 0.);
        radiance += alb * vec3f(1.0, 0.80, 0.43)
            * (0.11*light.sky.rgb + 0.18*backside*light.key.rgb);
    }
    return vec4f(max(radiance,vec3f(0.)),1.);
}
