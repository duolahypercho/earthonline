// (`--car-lamps`; appended to the light gather's module by agents.rs
// `car_lamps_wgsl`, its constants there: `LT_CAR_LAMPS` false compiles it out)
// A lit car's rear lamps as the city's lights, and the wet street's streak
// pick by what a vehicle lamp sends toward the eye.
//
// Each moving or queued car (a parked one is dark: `lights_agents` returns
// before) whose lamps are on (the vehicle lamps' state, `lt.scale.y` > 0:
// dusk, night, rain, fog) adds its two rear lamps beside its two low beams:
// at the body's own tail lamps (eo-agents `vehicles::rear_lamp`, in the
// model's spare words `wheel[2].w`, `wheel[3].w`), aimed back along the body,
// each a red LED lamp of the regulations' intensity (the tail lamp's, or the
// stop lamp's while the car brakes) shaped as their photometric field, its
// lens the size of a lamp's. They light the street and the car behind in red
// as every other light does (the clusters), and the wet street mirrors them
// in its streaks where they are among the frame's brightest toward the eye.

/// A body's left rear lamp (x from the centre line, y up, z along: the right
/// one mirrors it), or a stand-in at the body's tail for a body without one.
fn lt_rear_lamp_at(m: LtModel) -> vec3<f32> {
    let w = m.wheel[3].w;
    if (w > 0.0) {
        let xi = floor(w / LT_REAR_PACK);
        return vec3<f32>(xi / 100.0, (w - xi * LT_REAR_PACK) / 100.0, m.wheel[2].w);
    }
    return vec3<f32>(abs(m.wheel[2].x) * 0.85, m.wheel[2].y + LT_REAR_STAND_IN.y, m.wheel[2].z - LT_REAR_STAND_IN.z);
}

/// `lights_agents`' rear lamps for agent `a` of model `m` (lamps on, not parked).
fn lt_rear_lamps(a: LtAgent, m: LtModel) {
    if (!LT_CAR_LAMPS) {
        return;
    }
    let p = lt_rear_lamp_at(m);
    let braking = (a.flags & LT_FLAG_BRAKING) != 0u;
    let cd = select(LT_TAIL_LAMP_CD, LT_STOP_LAMP_CD, braking);
    let axis = normalize(lt_rotate(a.rot, vec3<f32>(0.0, 0.0, -1.0)));
    for (var s = 0; s < 2; s = s + 1) {
        let side = select(-1.0, 1.0, s == 0);
        var l: SkyLight;
        l.pos = a.pos + lt_rotate(a.rot, vec3<f32>(side * p.x, p.y, p.z) * a.scale);
        // the low beams' units per candela (`lt.scale.y` is that x LOW_BEAM_PEAK_CD)
        l.intensity = lt.scale.y / LT_LOW_BEAM_PEAK_CD * cd;
        l.color = LT_REAR_RED;
        l.range = sqrt(cd / LT_REAR_REACH_LUX);
        l.dir = axis;
        l.cos_outer = LT_REAR_COS_OUTER;
        l.cos_inner = LT_REAR_COS_INNER;
        l.kind = LT_REAR_KIND;
        l.size = LT_REAR_LENS_M2;
        l.pad = 0.0;
        if (lt_visible(l.pos, l.range)) {
            lt_push(l);
        }
    }
}

/// (`--car-lamps`, the streak pick) The share of a vehicle lamp's intensity
/// it sends toward the eye's side (1 for every other light): a low beam aimed
/// away from the eye leaves the wet street no streak toward it (its lens's
/// floor, as `sky_cine_streaks` bounds it), a rear lamp facing the eye its
/// whole field; `to_eye` from the light to the eye.
fn lt_car_toward_eye(l: SkyLight, to_eye: vec3<f32>) -> f32 {
    if (!LT_CAR_LAMPS) {
        return 1.0;
    }
    let h = normalize(vec2<f32>(to_eye.x, to_eye.z) + vec2<f32>(1e-6, 0.0));
    let ha = normalize(vec2<f32>(l.dir.x, l.dir.z) + vec2<f32>(1e-6, 0.0));
    let c = dot(h, ha);
    if (l.kind == 1u) {
        return max(smoothstep(l.cos_outer, l.cos_inner, c), LT_HEAD_LENS_SHARE * smoothstep(-0.2, 0.2, c));
    }
    if (l.kind == LT_REAR_KIND) {
        return smoothstep(l.cos_outer, l.cos_inner, c);
    }
    return 1.0;
}
