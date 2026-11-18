
// The landmark decks (eo-landmarks access.rs; eo-render interior_meter.rs
// fills `look.deck_*`, all off with `--no-landmark-access`). Appended to
// look_common.wgsl.

// The sky visibility a body standing in the player's room has (the room's
// mean, access_sky.rs), on its indirect light; 1 outside it.
fn deck_room_vis(world: vec3<f32>) -> f32 {
    if (look.deck_room_min.w < 0.0) {
        return 1.0;
    }
    if (all(world >= look.deck_room_min.xyz) && all(world <= look.deck_room_max.xyz)) {
        return look.deck_room_min.w;
    }
    return 1.0;
}

// The player's body as an occluder of the indirect light on what it stands
// on: a stack of spheres up the body (an adult's proportions for a 1.7 m
// body: ankles, shins, knees, thighs, hips, chest, shoulders, head; INFERRED
// from the ANSUR II medians, scaled to the drawn height), each hiding the
// solid angle it subtends, weighted by its cosine: a sphere of radius r at
// distance d whose centre lies at angle theta off the normal has the form
// factor cos(theta) (r / d)^2 (Howell's catalogue, B-7), 1 inside it.
fn deck_player_occlusion(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let h = look.deck_player.w;
    if (h <= 0.0) {
        return 1.0;
    }
    let feet = look.deck_player.xyz;
    let rel = world - feet;
    if (dot(rel, rel) > 6.25 * h * h) {
        return 1.0;
    }
    let k = h / 1.7;
    // (height over the feet, radius) for a 1.7 m body.
    var spheres = array<vec2<f32>, 8>(
        vec2<f32>(0.08, 0.10),
        vec2<f32>(0.32, 0.12),
        vec2<f32>(0.55, 0.13),
        vec2<f32>(0.80, 0.15),
        vec2<f32>(1.02, 0.17),
        vec2<f32>(1.25, 0.18),
        vec2<f32>(1.42, 0.17),
        vec2<f32>(1.60, 0.11),
    );
    var occ = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        let c = feet + vec3<f32>(0.0, spheres[i].x * k, 0.0);
        let r = spheres[i].y * k;
        let to = c - world;
        let d2 = max(dot(to, to), 1e-6);
        if (d2 <= r * r) {
            return 0.0;
        }
        let cos_t = dot(n, to) * inverseSqrt(d2);
        occ = occ + max(cos_t, 0.0) * r * r / d2;
    }
    return clamp(1.0 - occ, 0.0, 1.0);
}

// Whether a point lies in the player's body (a column 0.35 m round its
// axis, scaled with its height), for the screen-space bounce
// (surfaces_ao.wgsl): the body still hides what is behind it, but its few,
// sunlit and saturated pixels hand round none of their light, which the
// 2-slice estimate spreads as coloured speckle on the deck beside it.
fn deck_in_player(world: vec3<f32>) -> bool {
    let h = look.deck_player.w;
    if (h <= 0.0) {
        return false;
    }
    let rel = world - look.deck_player.xyz;
    let r = 0.35 * h / 1.7;
    return rel.y > -0.05 && rel.y < h + 0.1 && dot(rel.xz, rel.xz) < r * r;
}
