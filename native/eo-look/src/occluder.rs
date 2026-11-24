//! The light of what hides the sky (eo-render `--occluder-light`, ambient.wgsl
//! `amb_irradiance_lit`): the ambient volume knows how much of the sky a point
//! sees (a large-scale, baked visibility), and the rig fills the rest. Without
//! this the blocked sky is lit by the rig's ground lobe (`AMB_OCCLUDER` x the
//! sunlit street's bounce), as if every wall and soffit that hides the sky
//! were as bright as a sunlit street: a canyon's lower floors, which see more
//! wall and less sky, came out brighter than its upper ones, and the ground
//! under a deck as bright as open ground.
//!
//! Here what hides the sky is lit as it is:
//!
//! * **the shaded walls** by their own sky, the street below them and each
//!   other: the walls round a point whose street sees `v_g` of the sky are a
//!   canyon's, and see the sky a canyon's wall sees (`wall_sky_view`: the
//!   two-dimensional canyon's view factors, street `sqrt(1 + (H/W)^2) - H/W`,
//!   wall `(1 + W/H - sqrt(1 + (W/H)^2)) / 2`; the whole wall's, not the
//!   point's own, since the sky a street point loses is mostly the walls'
//!   upper floors), the street lobe below them (its sky part only as far as
//!   the street sees the sky) and, over the rest of their upper half, walls
//!   like themselves (a geometric series: `M = rho E / (1 - rho (1/2 - v_w))`).
//!   Their exitance `M` is the blocked sky's light.
//! * **the sunlit walls** by the sun bounce (`sun_bounce`, `--sun-bounce`),
//!   which with this on adds a sunlit wall's sun part whole: its sky part is
//!   `M` already.
//! * **the street below** the point keeps the rig's lobe, but the lobe's sky
//!   part (`bounce::LOBE_SKY_ALBEDO` x the sky's fill: the street's
//!   reflection of the sky) only as far as the street there sees the sky
//!   (`v_g`, the volume facing up at the street under the point): the ground
//!   in a narrow street or under a deck sees a fraction of the sky and sends
//!   that fraction up. The lobe's sun part is the ground bounce's (`bounce`).
//! * **the ambient term and the environment's diffuse** take the visible sky
//!   and the ground's share at `v_g` (their blocked share is in `M`), and
//!   **the rig's rim** (a directional stand-in for the sky opposite the sun,
//!   never shadowed) is hidden as the sky along it is (`rim_hidden`), taken
//!   from the fill it stands for.
//!
//! * **the decks' undersides** (a lid over the point rather than walls round
//!   it) by the ground they face: what hides the sky is a mix of the two,
//!   by where the sky the point still sees lies (`lid_share`: under a lid it
//!   is low, at the sides; between walls it is high, overhead). A deck's
//!   underside sees, beyond its edges, as much open ground as the street under
//!   it sees of the sky beyond them (the same edges from the other side:
//!   exact for a flat deck over flat ground), that ground open to the sky and
//!   in the sun; under itself, covered ground lit by the street's own sky
//!   (`soffit_exitance`). Without this a viaduct's soffit stood in as a
//!   canyon's walls, lit by the sky they do not see: the ground under it went
//!   sky blue.
//! * **the ground's sun part** (`bounce::added_irradiance_whole`): where the
//!   ambient volume knows the ground, the rig's flat lobe's sun part leaves
//!   whole and the sunlit ground's exitance comes back over its open share.
//!   The ground bounce alone takes out at most the sunlit ground's exitance a
//!   channel, which at a low sun (an orange exitance under a neutral lobe)
//!   left the lobe's green and blue behind: a navy fill on every underside.
//!
//! With the open sky and open ground (`v_s = t`, `v_g = 1`) this is the rig's
//! hemisphere, ambient and environment exactly: an open plaza is unchanged.
//!
//! Units are the rig's (irradiance; the shader's `irr x albedo / pi`).

use crate::math::Rgb;

/// The walls' diffuse albedo (weathered concrete, stucco and painted brick,
/// warm), the sun bounce's (`sun_bounce::WALL_ALBEDO`).
pub const WALL_ALBEDO: Rgb = crate::sun_bounce::WALL_ALBEDO;
/// The height over the ground at which the street's own sky visibility is read
/// (the volume's lowest layer's centre, metres).
pub const GROUND_SKY_H_M: f64 = 1.0;
/// The most of a wall's irradiance a vertical surface's sky can be (the open sky's share on a vertical surface).
pub const SIDE_OPEN: f64 = 0.5;
/// Shares under this are left out (the shader's skip): a surface with no lower
/// hemisphere and at most this much of its sky blocked does not read the
/// street's sky (it counts as open).
pub const SKIP_SHARE: f64 = 0.01;
/// The street's sky visibility is read between these for the canyon relation
/// (a street seeing 2 % of the sky lies between walls 25 x its width high; one
/// seeing 99.9 % is open ground).
pub const MIN_STREET_SKY: f64 = 0.02;
pub const MAX_STREET_SKY: f64 = 0.999;

/// The sky a canyon's wall sees (its view factor to the opening, `SIDE_OPEN`
/// in the open) for a street that sees `vg` of the sky: the canyon is
/// `H/W = (1 - vg^2) / (2 vg)` (from `vg = sqrt(1 + (H/W)^2) - H/W`) and its
/// wall sees `(1 + W/H - sqrt(1 + (W/H)^2)) / 2` (Hottel's crossed strings on
/// the canyon's section).
pub fn wall_sky_view(vg: f64) -> f64 {
    let s = vg.clamp(MIN_STREET_SKY, MAX_STREET_SKY);
    let x = 2.0 * s / (1.0 - s * s);
    SIDE_OPEN * (1.0 + x - (1.0 + x * x).sqrt())
}

/// The rig's fill as the shader holds it (irradiance, the hemisphere's two
/// lobes, the ambient term, and the environment's diffuse on a horizontal
/// normal averaged over azimuth).
#[derive(Clone, Copy, Debug)]
pub struct Fill {
    pub hemi_sky: Rgb,
    pub hemi_ground: Rgb,
    pub ambient: Rgb,
    pub env_side: Rgb,
}

/// The ground lobe's sky part: the street's reflection of the sky's fill
/// (`bounce::LOBE_SKY_ALBEDO` x (sky lobe + ambient)), never more than the lobe.
pub fn lobe_sky(f: &Fill) -> Rgb {
    [0, 1, 2].map(|i| (crate::bounce::LOBE_SKY_ALBEDO * (f.hemi_sky[i] + f.ambient[i])).min(f.hemi_ground[i]))
}

/// The ground lobe with its sky part at the street's own visibility `vg`.
pub fn ground_lobe(f: &Fill, vg: f64) -> Rgb {
    let ls = lobe_sky(f);
    [0, 1, 2].map(|i| f.hemi_ground[i] - ls[i] * (1.0 - vg.clamp(0.0, 1.0)))
}

/// The exitance of the shaded walls that hide the sky from a point whose
/// street sees `vg` of the sky.
pub fn shaded_exitance(f: &Fill, vg: f64) -> Rgb {
    let vw = wall_sky_view(vg);
    let vg = vg.clamp(0.0, 1.0);
    let g = ground_lobe(f, vg);
    [0, 1, 2].map(|i| {
        let e = f.hemi_sky[i] * vw + g[i] * 0.5 + (f.ambient[i] + f.env_side[i]) * (vw + 0.5 * vg);
        WALL_ALBEDO[i] * e / (1.0 - WALL_ALBEDO[i] * (SIDE_OPEN - vw))
    })
}

/// The indirect diffuse irradiance on a normal whose upper hemisphere's
/// share is `t` (= (1 + n.y) / 2), of which the volume says the sky fills
/// `vs` (<= t); `vg` the street's own sky, `env` the environment's diffuse
/// on the normal. Without the ground bounce.
pub fn irradiance(f: &Fill, t: f64, vs: f64, vg: f64, env: Rgb) -> Rgb {
    let vs = vs.min(t);
    let lower = 1.0 - t;
    let vg = vg.clamp(0.0, 1.0);
    let g = ground_lobe(f, vg);
    let m = shaded_exitance(f, vg);
    [0, 1, 2].map(|i| f.hemi_sky[i] * vs + g[i] * lower + m[i] * (t - vs) + (f.ambient[i] + env[i]) * (vs + lower * vg))
}

/// The share of the rig's rim light (a directional stand-in for the sky
/// opposite the sun, never shadowed) that what hides the sky hides: one less
/// the volume's visible share along its direction `vis_d` over the open sky's
/// (`(1 + d.y) / 2`).
pub fn rim_hidden(vis_d: f64, d_y: f64) -> f64 {
    1.0 - (vis_d / (0.5 + 0.5 * d_y).max(1e-3)).min(1.0)
}

/// The decks' underside albedo: weathered grey concrete. INFERRED from the
/// reference sheet (2.1) as `bounce::GROUND_ALBEDO`'s concrete: new grey
/// concrete about 0.40 (US EPA 2012), weathered and soiled lower; 0.30, grey.
pub const DECK_ALBEDO: f64 = 0.30;

/// The lid's share of what hides the sky is read from where the sky a point
/// still sees lies: `r = a0 / (a0 + ay)`, the visible sky's share on a
/// vertical surface (its mean over azimuth, `a0`) over its share facing up.
/// A patch of sky at elevation `e` adds `sin e` to the second and
/// `cos e / pi` to the first, so `r` is 1/2 under the open sky, lower where
/// walls close the sky's sides and leave the top (a canyon: 0.2 to 0.35 for
/// H/W 1 to 3), higher where a lid closes the top and leaves the sides (a
/// deck: 0.6 at its centre at H = half its width, more under a wider one).
/// The lid's share eases from 0 at `LID_R0` to 1 at `LID_R1`.
pub const LID_R0: f64 = 0.5;
pub const LID_R1: f64 = 0.8;

/// The lid's share of what hides the sky over a point whose volume visibility
/// has azimuth mean `a0` and upward share `up` (`a0 + ay`).
pub fn lid_share(a0: f64, up: f64) -> f64 {
    let r = a0 / up.max(1e-3);
    let t = ((r - LID_R0) / (LID_R1 - LID_R0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The exitance of a deck's underside over a street that sees `vg` of the
/// sky, `sun_ground` the sunlit open ground's exitance from the sun
/// (`bounce::sun_exitance` with the ground bounce, the rig's lobe's sun part
/// without it). Beyond its edges the underside sees `vg` of its lower
/// hemisphere in open ground (lit by the whole sky and the sun), under itself
/// `1 - vg` of covered ground lit by the street's own sky (`vg`).
pub fn soffit_exitance(f: &Fill, vg: f64, sun_ground: Rgb) -> Rgb {
    let vg = vg.clamp(0.0, 1.0);
    let ls = lobe_sky(f);
    [0, 1, 2].map(|i| DECK_ALBEDO * (vg * (ls[i] + sun_ground[i]) + (1.0 - vg) * vg * ls[i]))
}

/// What hides the sky's exitance: the shaded walls' and the decks' undersides'
/// by the lid's share `lid`.
pub fn occluder_exitance(f: &Fill, vg: f64, lid: f64, sun_ground: Rgb) -> Rgb {
    let w = shaded_exitance(f, vg);
    let s = soffit_exitance(f, vg, sun_ground);
    let lid = lid.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| w[i] + (s[i] - w[i]) * lid)
}

/// `irradiance` with what hides the sky at the exitance `m` (`occluder_exitance`).
pub fn irradiance_with(f: &Fill, t: f64, vs: f64, vg: f64, m: Rgb, env: Rgb) -> Rgb {
    let vs = vs.min(t);
    let lower = 1.0 - t;
    let vg = vg.clamp(0.0, 1.0);
    let g = ground_lobe(f, vg);
    [0, 1, 2].map(|i| f.hemi_sky[i] * vs + g[i] * lower + m[i] * (t - vs) + (f.ambient[i] + env[i]) * (vs + lower * vg))
}

/// The same with the rig's stand-in (`AMB_OCCLUDER` = 1: the blocked sky at the ground lobe), as before.
pub fn irradiance_stand_in(f: &Fill, t: f64, vs: f64, env: Rgb) -> Rgb {
    let vs = vs.min(t);
    [0, 1, 2].map(|i| f.hemi_sky[i] * vs + f.hemi_ground[i] * (1.0 - t + (t - vs)) + f.ambient[i] + env[i] * (1.0 - t + vs))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rig at an hour (the JS fallback yield).
    fn fill(hour: f64) -> Fill {
        let lf = crate::LookFrame::new(hour);
        let sh = lf.env_sh;
        let env_i = lf.rig.environment_intensity * lf.rig.env_painted;
        let side = [0, 1, 2].map(|c| (sh[0][c] * 0.282095 + sh[6][c] * 0.157696 + sh[8][c] * 0.273137).max(0.0) * env_i);
        Fill { hemi_sky: lf.hemi_sky, hemi_ground: lf.hemi_ground, ambient: lf.ambient, env_side: side }
    }

    #[test]
    fn open_sky_and_ground_are_the_rig_as_before() {
        for h in [9.0, 12.0, 18.5, 21.0] {
            let f = fill(h);
            for ny in [-1.0, -0.4, 0.0, 0.5, 1.0] {
                let t = 0.5 * ny + 0.5;
                let env = [0.01, 0.02, 0.03];
                let a = irradiance(&f, t, t, 1.0, env);
                let b = irradiance_stand_in(&f, t, t, env);
                for c in 0..3 {
                    assert!((a[c] - b[c]).abs() < 1e-12, "{h} {ny}: {a:?} {b:?}");
                }
            }
        }
    }

    #[test]
    fn the_canyon_view_factors() {
        // H/W = 2: the street sees sqrt(5) - 2, the wall (1 + 1/2 - sqrt(5/4)) / 2.
        let vg = 5f64.sqrt() - 2.0;
        assert!((wall_sky_view(vg) - 0.5 * (1.5 - 1.25f64.sqrt())).abs() < 1e-12);
        // H/W = 6 (a lane between towers), and the open.
        let vg6 = 37f64.sqrt() - 6.0;
        assert!((wall_sky_view(vg6) - 0.5 * (1.0 + 1.0 / 6.0 - (1.0 + 1.0 / 36.0f64).sqrt())).abs() < 1e-12);
        assert!((wall_sky_view(1.0) - SIDE_OPEN).abs() < 0.01);
        // a wall sees less sky than its street, and more as the street opens.
        let mut last = 0.0;
        for i in 1..=20 {
            let v = i as f64 / 20.0;
            let w = wall_sky_view(v);
            assert!(w > last && w <= SIDE_OPEN && w < v, "{v}: {w}");
            last = w;
        }
    }

    #[test]
    fn a_canyon_is_darker_below_than_above_and_than_the_stand_in() {
        for h in [12.0, 18.5] {
            let f = fill(h);
            // a wall facing a street between blocks twice its width high (the street sees 0.24 of
            // the sky): little sky low on the wall, more near the top.
            let vg = 5f64.sqrt() - 2.0;
            let (low, high) = (0.05, 0.35); // vs on the vertical normal (t = 1/2)
            let e_low = irradiance(&f, 0.5, low, vg, [0.0; 3]);
            let e_high = irradiance(&f, 0.5, high, vg, [0.0; 3]);
            let s_low = irradiance_stand_in(&f, 0.5, low, [0.0; 3]);
            let s_high = irradiance_stand_in(&f, 0.5, high, [0.0; 3]);
            assert!(e_low[1] < e_high[1], "{h}: the lower floor sees less sky and more shaded wall");
            assert!(e_low[1] < s_low[1] && e_high[1] < s_high[1]);
            // the stand-in made the lower floor barely darker than the upper one (the inverted canyon).
            assert!(s_high[1] / s_low[1] < e_high[1] / e_low[1]);
            // and a shaded wall is well under the sunlit street's lobe, yet never black.
            let m = shaded_exitance(&f, vg);
            assert!(m[1] > 0.0 && m[1] < 0.5 * f.hemi_ground[1], "{h}: {m:?} vs {:?}", f.hemi_ground);
        }
    }

    #[test]
    fn under_a_deck_the_ground_lobe_keeps_the_ground_s_sky() {
        let f = fill(12.0);
        // a soffit (n down: t = 0) over a street that sees a fifth of the sky.
        let e = irradiance(&f, 0.0, 0.0, 0.2, [0.0; 3]);
        let s = irradiance_stand_in(&f, 0.0, 0.0, [0.0; 3]);
        let ls = lobe_sky(&f);
        for c in 0..3 {
            assert!((s[c] - e[c] - (ls[c] + f.ambient[c]) * 0.8).abs() < 1e-12);
        }
        // more sky at the street, more light; the whole sky, the rig's lobe.
        assert!(irradiance(&f, 0.0, 0.0, 0.6, [0.0; 3])[1] > e[1]);
        assert!((irradiance(&f, 0.0, 0.0, 1.0, [0.0; 3])[1] - s[1]).abs() < 1e-12);
    }

    #[test]
    fn the_rim_is_hidden_as_the_sky_is() {
        // the open sky along the rim's direction: nothing hidden; under a deck: all of it.
        for dy in [0.1, 0.47, 0.9] {
            assert!(rim_hidden(0.5 + 0.5 * dy, dy).abs() < 1e-12);
            assert!((rim_hidden(0.0, dy) - 1.0).abs() < 1e-12);
        }
        // half the open sky's share along it: half hidden.
        assert!((rim_hidden(0.5 * 0.735, 0.47) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn the_lid_is_read_from_where_the_visible_sky_lies() {
        // the open sky: r = 1/2, no lid.
        assert!(lid_share(0.5, 1.0).abs() < 1e-12);
        // a two-dimensional canyon H/W = 2 at its street's centre: the slot overhead
        // (half angle asin(0.29) ~ 17 deg) seen facing up, little of it facing the walls.
        assert!(lid_share(0.06, 0.29).abs() < 1e-12);
        // a deck as high as half its width over the point: the sky at the sides
        // below 45 deg; facing up 1 - sin 45 deg, facing across (sin 45 deg) / 2
        // on the open sides, about nothing along the deck.
        let up = 1.0 - 0.5f64.sqrt();
        let a0 = 0.5 * (0.5 * 0.5f64.sqrt());
        assert!(a0 / up > 0.6 && lid_share(a0, up) > 0.25, "{} {}", a0 / up, lid_share(a0, up));
        // a deck three times wider than high: the sky below 18 deg, all lid.
        let e = 18f64.to_radians();
        assert!(lid_share(0.5 * 0.5 * e.sin(), 1.0 - e.cos()) > 0.99);
        // nothing seen at all (a bore): a lid.
        assert!((lid_share(0.0, 0.0) - 0.0).abs() < 1e-12 || lid_share(0.0, 0.0) <= 1.0);
    }

    #[test]
    fn under_a_deck_the_ground_lights_what_hides_the_sky() {
        for h in [12.0, 18.5] {
            let lf = crate::LookFrame::new(h);
            let f = fill(h);
            let sun = crate::bounce::sun_exitance(&lf);
            // the road under a viaduct: a fifth of the sky at the sides, the street a quarter.
            let (vs, vg) = (0.15, 0.25);
            let walls = irradiance(&f, 1.0, vs, vg, [0.0; 3]);
            let deck = irradiance_with(&f, 1.0, vs, vg, occluder_exitance(&f, vg, 1.0, sun), [0.0; 3]);
            let br = |e: Rgb| e[2] / e[0];
            let (m_w, m_s) = (shaded_exitance(&f, vg), soffit_exitance(&f, vg, sun));
            println!("{h} h: walls {m_w:?} deck {m_s:?}; road under walls {walls:?} under the deck {deck:?}");
            // the deck's underside is never bluer than the ground it sees reflects the sky.
            let ls = lobe_sky(&f);
            assert!(br(m_s) <= br(ls) + 1e-9, "{h}: {m_s:?} vs the ground's sky {ls:?}");
            if h < 15.0 {
                // under a high sun the sunlit ground beyond its edges makes it warmer and
                // brighter than walls lit by the sky they see, and the road under it too.
                assert!(br(m_s) < br(m_w) && m_s[1] > m_w[1], "{h}: {m_s:?} vs {m_w:?}");
                assert!(br(deck) < br(walls), "{h}: {deck:?} vs {walls:?}");
            } else {
                // under a low sun the ground is dim (the sun grazes it): the deck sees no
                // sky and is darker than walls that do.
                assert!(m_s[1] < m_w[1], "{h}: {m_s:?} vs {m_w:?}");
            }
            // never more than the sunlit open ground itself bounces.
            for c in 0..3 {
                assert!(m_s[c] <= DECK_ALBEDO * (lobe_sky(&f)[c] + sun[c]) + 1e-12);
            }
        }
        // no lid: the walls as before; the open sky: nothing hidden, the rig as before.
        let f = fill(12.0);
        let sun = crate::bounce::sun_exitance(&crate::LookFrame::new(12.0));
        let (a, b) = (occluder_exitance(&f, 0.3, 0.0, sun), shaded_exitance(&f, 0.3));
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 1e-12);
        }
        let open = irradiance_with(&f, 1.0, 1.0, 1.0, occluder_exitance(&f, 1.0, 1.0, sun), [0.0; 3]);
        let rig = irradiance_stand_in(&f, 1.0, 1.0, [0.0; 3]);
        for c in 0..3 {
            assert!((open[c] - rig[c]).abs() < 1e-12);
        }
    }

    #[test]
    fn the_walls_light_each_other_and_stay_bounded() {
        let f = fill(12.0);
        let open = shaded_exitance(&f, 1.0);
        for vg in [0.02, 0.1, 0.24, 0.5, 1.0] {
            let m = shaded_exitance(&f, vg);
            assert!(m[1] <= open[1] + 1e-12, "{vg}");
            let vw = wall_sky_view(vg);
            let e = f.hemi_sky[1] * vw + ground_lobe(&f, vg)[1] * 0.5 + (f.ambient[1] + f.env_side[1]) * (vw + 0.5 * vg);
            assert!((m[1] - WALL_ALBEDO[1] * (e + m[1] * (SIDE_OPEN - vw))).abs() < 1e-12, "the wall's balance");
        }
    }
}
