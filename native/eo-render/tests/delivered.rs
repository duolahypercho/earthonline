//! the delivered tone and chroma of the street's surfaces under the
//! light rig the running JS game solves, through the display transform (CPU
//! reference of look_post.wgsl), pinned to the JS's own records:
//!
//!   * shade chroma: frames of the earlier browser version (Folsom Street, not
//!     a matched camera or hour) delivered shaded asphalt sRGB (37.5, 47.6, 57.4), shaded
//!     footway (36.9, 44.2, 52.1) and (54.1, 60.6, 68.4), shaded walls
//!     (104.8, 112.3, 115.8) and (116.5, 119.3, 116.9): blue over red at most
//!     1.53 on the ground and 1.10 on a wall;
//!   * lit / shadow: the 11:00 clear card's delivered band 5.0-7.0
//!     (src/render/environment-ibl.js, the `groundAlbedo` record), scene-referred,
//!     for one class lit against itself in shade;
//!   * sunlit asphalt: the JS age rule's own record (street-surface-age.js,
//!     `bleachFresh`): the batch-3 frames delivered sunlit asphalt at luma
//!     170-180 and were darkened for it; the photographs read 55-130.
//!
//! Diffuse only (Lambert under key + hemisphere + ambient + the bound probe's
//! diffuse, as look_light.wgsl), mean albedos of the JS ground materials.

use eo_look::math::{luminance, srgb_to_linear};
// the night and weather checks read eo-sky (a dependency of eo-render).
use eo_look::{LookFrame, sh_eval, tone};

fn lin(c: [u8; 3]) -> [f64; 3] {
    c.map(|v| srgb_to_linear(v as f64 / 255.0))
}

struct Out {
    sun: [f64; 3],
    shade: [f64; 3],
    sun_l: f64,
    shade_l: f64,
}

fn shade(f: &LookFrame, albedo: [f64; 3], n: [f64; 3], env_class: f64) -> Out {
    let up = n[1] * 0.5 + 0.5;
    let hemi = [0, 1, 2].map(|i| f.hemi_ground[i] + (f.hemi_sky[i] - f.hemi_ground[i]) * up);
    let sh = sh_eval(&f.env_sh, n);
    let env = f.rig.environment_intensity * env_class;
    let fill = [0, 1, 2].map(|i| hemi[i] + f.ambient[i] + sh[i] * env);
    let nl = (n[0] * f.key_dir[0] + n[1] * f.key_dir[1] + n[2] * f.key_dir[2]).max(0.0);
    let lit = [0, 1, 2].map(|i| (f.key[i] * nl + fill[i]) * albedo[i] / std::f64::consts::PI);
    let dark = [0, 1, 2].map(|i| fill[i] * albedo[i] / std::f64::consts::PI);
    let d = |c: [f64; 3]| tone::display(c, f.exposure, [0.5, 0.5]).map(|x| x * 255.0);
    Out { sun: d(lit), shade: d(dark), sun_l: luminance(lit), shade_l: luminance(dark) }
}

#[test]
fn street_surfaces_deliver_the_js_tone_and_shade_chroma() {
    let m = eo_street::material::StreetMaterial::get();
    // Mean JS albedos: carriageway = residential tint x mid-cycle bleach on the
    // sRGB code (street-surface-age.js), linearised, x the 0.44 base fold;
    // footway = sidewalk tint x the concrete tile's 0.456; a painted wall at a
    // mid-grey stucco.
    let bleach = (m.surface_age.bleach_fresh + m.surface_age.bleach_aged) / 2.0;
    let asphalt_srgb = m.asphalt_tint("residential").map(|v| ((v as f64 * bleach).round()) as u8);
    let base = m.pbr_sets.carriageway.base_mean_linear.unwrap()[0];
    let asphalt = lin(asphalt_srgb).map(|c| c * base);
    // x the aged-concrete fold the street shader draws with.
    let footway = lin(m.tint("sidewalk")).map(|c| c * m.concrete_base_mean_linear * eo_render::look::concrete_aged_fold());
    let wall = [0.45, 0.45, 0.45];
    let env = |k: &str| m.env_base.get(k).copied().unwrap_or(1.0);
    for hour in [9.0, 11.0, 12.0, 17.0] {
        let f = LookFrame::new(hour);
        let a = shade(&f, asphalt, [0.0, 1.0, 0.0], env("asphalt"));
        let w = shade(&f, footway, [0.0, 1.0, 0.0], env("sidewalk"));
        // A wall facing away from the sun (its normal opposite the key's horizontal).
        let kh = (f.key_dir[0].hypot(f.key_dir[2])).max(1e-6);
        let wn = [-f.key_dir[0] / kh, 0.0, -f.key_dir[2] / kh];
        let v = shade(&f, wall, wn, 0.62);
        let br = |c: [f64; 3]| c[2] / c[0].max(1.0);
        let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        println!(
            "{hour:>5} h  asphalt sun {:?} shade {:?} (B/R {:.2}, lit/shadow {:.2})  footway sun {:?} shade {:?} (B/R {:.2}, lit/shadow {:.2})  shaded wall {:?} (B/R {:.2})",
            a.sun.map(|x| x.round()),
            a.shade.map(|x| x.round()),
            br(a.shade),
            a.sun_l / a.shade_l,
            w.sun.map(|x| x.round()),
            w.shade.map(|x| x.round()),
            br(w.shade),
            w.sun_l / w.shade_l,
            v.shade.map(|x| x.round()),
            br(v.shade)
        );
        assert!(br(a.shade) <= 1.6, "{hour} h shaded asphalt B/R {:.2} over the JS frames' 1.53", br(a.shade));
        assert!(br(w.shade) <= 1.6, "{hour} h shaded footway B/R {:.2}", br(w.shade));
        assert!(br(v.shade) <= 1.25, "{hour} h shaded wall B/R {:.2} over the JS frames' 1.10", br(v.shade));
        assert!(luma(a.sun) < luma(w.sun), "{hour} h: sunlit asphalt must read darker than sunlit footway");
        assert!(luma(a.sun) <= 180.0, "{hour} h sunlit asphalt luma {:.0} above the JS's rejected 170-180", luma(a.sun));
        if hour == 11.0 {
            for (what, o) in [("asphalt", &a), ("footway", &w)] {
                let r = o.sun_l / o.shade_l;
                assert!((4.5..=7.5).contains(&r), "11 h {what} lit/shadow {r:.2} outside the JS's delivered 5.0-7.0 (+-0.5)");
            }
        }
    }
}

/// a painted wall, sunlit and in shade, against the JS's own records.
///
///   * the JS's albedo: the kit stream's `painted-stucco` draws the interior
///     library's plaster bake (kit/materials.js `borrowInterior`, seed
///     `authored-slice-v1|authored-exterior`, archetype industrial, quality
///     medium), mean linear albedo (0.7520, 0.6766, 0.6249), measured from the
///     JS's own bake in node; this port draws its own bake of the kit spec
///     (#d9cfbd, about (0.674, 0.605, 0.494)): the listed deviation;
///   * sunlit: the earlier browser version's records for sf-building-159953388
///     (src/world/buildings/authored/buildings/b159953388.js, the notes on its
///     painted-stucco tint) read the sunlit wall at luma 201-208 at tint
///     #edf2f7 and 194-196 at #e4eaf0 on the 10:30 card (its weathering grime
///     0.18: x 0.91 at mid-wall);
///   * shade: the JS frames on disk (r3-gatecheck.png, default-play.png):
///     shaded walls (104.8, 112.3, 115.8) and (116.5, 119.3, 116.9).
///
/// The rig must reproduce the sunlit record at an incidence a vertical wall
/// can have at 10:30 (N.L between 0 and the sun-facing maximum), a sun-facing
/// wall must stay out of clipping, and the shaded wall's chroma must stay
/// inside the JS frames' band. Diffuse only: the frame adds the screen-space
/// bounce (the agents street measures a shaded wall at (99, 105, 108) against
/// the JS frames' (105, 112, 116)).
#[test]
fn painted_wall_delivers_the_js_sunlit_and_shade_tone() {
    const JS_PLASTER_MEAN: [f64; 3] = [0.7520, 0.6766, 0.6249];
    let weather = 1.0 - 0.18 * 0.5;
    let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let f = LookFrame::new(10.5);
    let kh = (f.key_dir[0].hypot(f.key_dir[2])).max(1e-6);
    let toward = [f.key_dir[0] / kh, 0.0, f.key_dir[2] / kh];
    let nl_max = kh;
    // The fill on a vertical wall (as `shade`), the key at an explicit N.L.
    let up = toward[1] * 0.5 + 0.5;
    let hemi = [0, 1, 2].map(|i| f.hemi_ground[i] + (f.hemi_sky[i] - f.hemi_ground[i]) * up);
    let sh = sh_eval(&f.env_sh, toward);
    let fill = [0, 1, 2].map(|i| hemi[i] + f.ambient[i] + sh[i] * f.rig.environment_intensity);
    for (tint, js) in [("#edf2f7", (201.0, 208.0)), ("#e4eaf0", (194.0, 196.0))] {
        let t = eo_assets::kit::hex_to_linear(tint).unwrap();
        let albedo = [0, 1, 2].map(|i| JS_PLASTER_MEAN[i] * t[i] * weather);
        let at = |nl: f64| {
            let lit = [0, 1, 2].map(|i| (f.key[i] * nl + fill[i]) * albedo[i] / std::f64::consts::PI);
            tone::display(lit, f.exposure, [0.5, 0.5]).map(|x| x * 255.0)
        };
        let sun_max = at(nl_max);
        let dark = at(0.0);
        let need = (0..=200).map(|k| k as f64 / 200.0 * nl_max).find(|nl| luma(at(*nl)) >= js.0);
        println!(
            "JS painted-stucco x {tint}: albedo {:?}; 10:30 sun-facing (N.L {nl_max:.2}) {:?} luma {:.0}; the JS record {js:?} is met at N.L {:?}; shade {:?} luma {:.0} (B/R {:.2})",
            albedo.map(|v| (v * 1000.0).round() / 1000.0),
            sun_max.map(|x| x.round()),
            luma(sun_max),
            need.map(|v| (v * 100.0).round() / 100.0),
            dark.map(|x| x.round()),
            luma(dark),
            dark[2] / dark[0].max(1.0)
        );
        assert!(need.is_some(), "{tint}: the rig cannot reach the JS's recorded sunlit {js:?} on a vertical wall at 10:30");
        assert!(luma(sun_max) < 245.0, "{tint}: a sun-facing wall clips ({:.0})", luma(sun_max));
        assert!(dark[2] / dark[0].max(1.0) <= 1.25, "{tint}: shaded wall B/R over the JS frames' 1.10 (+0.15)");
    }
}

/// the sunlit footway against the JS's own delivered record.
///
///   * card 01 (`01-street-day`, 11:00 clear; scripts/qa/capture-quality-cards-v1.mjs),
///     the street-surface pair of record (street-surface-v2-render.js): sunlit sidewalk RGB (184, 175, 160), Y 175.8;
///   * pedestrian-presentation.js: "the ~170-luma sunlit pavement the delivered
///     cards measure".
///
/// The rig and the display transform gave the mean JS footway albedo (sidewalk
/// tint x the concrete tile's 0.456, linear 0.389: new concrete) 204 at 11 h
/// and (226, 209, 193) at noon, its scored joints in the shoulder of the tone
/// curve. fix3 draws the walk concrete at the measured albedo of weathered
/// concrete (`CONCRETE_AGED_ALBEDO` 0.30, LBNL Heat Island Group: new 0.35-0.45,
/// weathered 0.20-0.30). Photographic check: an 18 % grey card in the 11 h sun
/// delivers luma 150 (the JS exposure, unchanged here). None of the JS post-chain
/// stages this port lacks lowers an open sunlit footway (by their code: the
/// aerial ramp is 3 % at 100 m, the sun veil and bloom add light, the contact
/// AO and contact shadow act within 0.55-0.85 m of a contact, the far sun
/// shadow is off by default, the highlight shoulder starts at display 0.85
/// and is ported); the cloud deck's key term (`cloudShadowSample`, not ported)
/// is the deck's transmittance over a 120 m footprint relative to its mean,
/// at most 1: under 1 only where more than about 30 % of that footprint is
/// under cloud (clear's coverage 0.30), so it can explain a darker card only
/// with its footprint recorded. Pinned: the 11 h sunlit footway within
/// +-12 luma of the JS card-01 record, a joint (the scan's darker 20 %:
/// albedo x 0.8) at least 15 sRGB steps under the walk beside it in the sun
/// (15.8; 14.8 at the new-concrete albedo), and the footway still lighter than
/// the asphalt and the grey card.
/// the walk at night under a street lamp, and the day under weather,
/// through the frame the renderer draws with every sky stage on (eo-sky
/// `frame::look_frame`: the weather's sky model, the moon key, the night
/// exposure solve), against physical references (sources in
/// eo-sky/src/photometry.rs and the README "Validation"):
///
///   * the unlit walk at night at display 16/255 (the JS night solve's grey
///     card, sky-atmosphere.js), the walk under an arterial lamp
///     (`LAMP_POOL_PEAK_LUX` 30 lux at 8.84 m, the JS head) inside the
///     photographic middle-grey band (an 18 % card at sRGB 84-161: exposed
///     for the lit street), the pool's peak over the walk 16 m out inside
///     ANSI/IES RP-8-18's uniformity (max:min 3-20);
///   * blue hour (sun -4 deg): the lit walk reads brighter than the unlit
///     walk by the same ratio, and the sky over the street stays brighter
///     than the unlit walk (civil twilight: the sky is the brightest thing
///     not lit by a lamp);
///   * noon under overcast and rain: horizontal illuminance (calibrated on
///     the clear noon, 90 klx) inside the IES overcast band, darker with
///     each heavier weather, the exposure rising (the JS curve over the
///     weather's illuminance) and capped at the JS 1.55.
#[test]
fn night_lamps_and_weather_against_physical_references() {
    use eo_sky::{frame, lights, night, photometry as ph, weather};
    let m = eo_street::material::StreetMaterial::get();
    let footway = lin(m.tint("sidewalk")).map(|c| c * m.concrete_base_mean_linear * eo_render::look::concrete_aged_fold());
    let card = |e: f64, exposure: f64, albedo: f64| night::display_grey(albedo * e / std::f64::consts::PI, exposure) * 255.0;
    let lamp = lights::place_street_lamps(&[lights::SegmentIn { highway: "primary", width: 12.0, sidewalk_left: 3.0, sidewalk_right: 3.0, points: &[[0.0, 0.0], [64.0, 0.0]] }], |_, _| 0.0)[0];
    let walk_l = luminance(footway);
    for (name, hour) in [("blue hour", 19.4), ("night", 21.0)] {
        let c = eo_sky::Conditions { hour, weather: weather::preset(weather::Kind::Clear), moon_day: eo_sky::moon::DEFAULT_MOON_DAY };
        let (f, t) = frame::look_frame(&c, None);
        let unlit = night::ground_fill(&f, night::SIDEWALK_ENV) + night::key_on_ground(&f);
        let k = t.lamp_units_per_lux * t.lamps_on;
        let peak = unlit + lights::street_lamp_lux(&lamp, 0.0) * k;
        let out = unlit + lights::street_lamp_lux(&lamp, 16.0) * k;
        let sky = luminance(f.model.zenith_radiance);
        println!(
            "{name} ({hour} h, sun {:+.1} deg, exposure {:.2}, lamps on {:.2}): 18 % card unlit {:.1}, under the lamp {:.1}, 16 m out {:.1}; walk (albedo {:.3}) unlit {:.1}, lit {:.1}; zenith sky radiance {:.4} vs unlit walk {:.4}",
            f.model.sun.altitude_deg,
            f.exposure,
            t.lamps_on,
            card(unlit, f.exposure, 0.18),
            card(peak, f.exposure, 0.18),
            card(out, f.exposure, 0.18),
            walk_l,
            card(unlit, f.exposure, walk_l),
            card(peak, f.exposure, walk_l),
            sky,
            walk_l * unlit / std::f64::consts::PI
        );
        assert!(t.lamps_on > 0.9, "{name}: street lamps not on at sun {:.1} deg", f.model.sun.altitude_deg);
        assert!(peak / unlit > 2.0, "{name}: the lamp pool is not 2x the unlit walk");
        if name == "night" {
            // at the night solve's target (eo-sky `night::unlit_display_255`: 14 under `--night-lift`, else the owner-approved 8; the JS 16)
            assert!((card(unlit, f.exposure, 0.18) - eo_sky::night::unlit_display_255()).abs() < 1.5, "night: unlit card off the night solve's target");
            assert!(ph::GREY_CARD_MIDDLE.holds(card(peak, f.exposure, 0.18)), "night: the card under the lamp outside middle grey +-1 EV");
            assert!(ph::STREET_UNIFORMITY_MAX_MIN.holds(peak / out), "night: pool uniformity {:.2} outside RP-8", peak / out);
        } else {
            // The twilight sky outshines the unlit walk (a Lambertian ground
            // under the sky returns albedo x its radiance): > 2 at the walk's
            // 0.30. The JS rig gave 0.58 here; eo-sky caps the blue-hour fill.
            let ratio = sky / (walk_l * unlit / std::f64::consts::PI);
            println!("  blue hour: zenith sky / unlit walk radiance {ratio:.2} (physical twilight > 2; the JS rig 0.58)");
            assert!(ratio > 2.0, "blue hour: the unlit walk outshines the twilight sky ({ratio:.2})");
        }
    }
    let mut last = (f64::INFINITY, 0.0);
    for k in [weather::Kind::Clear, weather::Kind::Overcast, weather::Kind::Rain, weather::Kind::Storm] {
        let c = eo_sky::Conditions { hour: 12.0, weather: weather::preset(k), moon_day: 0.0 };
        let (f, t) = frame::look_frame(&c, None);
        let e = luminance(f.key) * f.key_dir[1].max(0.0) * night::mean_beam(&c.weather) + night::ground_fill(&f, night::SIDEWALK_ENV);
        // An 18 % card in the weather's light: the photographic band, at most
        // 1.5 EV under middle grey (the weather compensation, eo-sky night.rs).
        let c18 = card(e, t.exposure_target, 0.18);
        println!("noon {:<9} horizontal {:>7.0} lux (calibrated), exposure {:.3}, 18 % card sRGB {:.0}", k.name(), ph::lux(e), t.exposure_target, c18);
        assert!(e < last.0 && t.exposure_target >= last.1, "{}: not darker / not exposed up", k.name());
        if k != weather::Kind::Storm {
            assert!(c18 >= 118.0 * 0.5f64.powf(1.5 / 2.2) - 1.0 && c18 <= ph::GREY_CARD_MIDDLE.hi, "{}: the card at {c18:.0} is outside middle grey -1.5..+1 EV", k.name());
        }
        if k == weather::Kind::Overcast {
            assert!(ph::OVERCAST_NOON_GLOBAL.holds(ph::lux(e)), "overcast noon {:.0} lux outside the IES band", ph::lux(e));
        }
        last = (e, t.exposure_target);
    }
}

#[test]
fn sunlit_footway_against_the_js_card_record() {
    let m = eo_street::material::StreetMaterial::get();
    let footway = lin(m.tint("sidewalk")).map(|c| c * m.concrete_base_mean_linear * eo_render::look::concrete_aged_fold());
    let env = m.env_base.get("sidewalk").copied().unwrap_or(1.0);
    let f = LookFrame::new(11.0);
    let w = shade(&f, footway, [0.0, 1.0, 0.0], env);
    let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let js = 0.2126 * 184.0 + 0.7152 * 175.0 + 0.0722 * 160.0;
    for h in [11.0, 12.0] {
        let fh = LookFrame::new(h);
        let card = shade(&fh, [0.18; 3], [0.0, 1.0, 0.0], env);
        let fw = shade(&fh, footway, [0.0, 1.0, 0.0], env);
        println!("{h} h: 18 % grey card in the sun {:?} (luma {:.1}); footway albedo {:?} -> {:?}", card.sun.map(|x| x.round()), luma(card.sun), footway.map(|v| (v * 1000.0).round() / 1000.0), fw.sun.map(|x| x.round()));
    }
    println!("11 h sunlit footway {:?} luma {:.1}; the JS card-01 record (184, 175, 160) luma {js:.1}: {:+.1}", w.sun.map(|x| x.round()), luma(w.sun), luma(w.sun) - js);
    let joint = shade(&f, footway.map(|c| c * 0.8), [0.0, 1.0, 0.0], env);
    let steps = luma(w.sun) - luma(joint.sun);
    let card = shade(&f, [0.18; 3], [0.0, 1.0, 0.0], env);
    let fold = eo_render::look::concrete_aged_fold();
    let (new_w, new_j) = (shade(&f, footway.map(|c| c / fold), [0.0, 1.0, 0.0], env), shade(&f, footway.map(|c| c * 0.8 / fold), [0.0, 1.0, 0.0], env));
    println!("11 h joint (albedo x 0.8) {:?}: {steps:.1} sRGB luma steps under the walk (at the JS's new-concrete albedo {:.1})", joint.sun.map(|x| x.round()), luma(new_w.sun) - luma(new_j.sun));
    assert!((luma(w.sun) - js).abs() <= 12.0, "11 h sunlit footway luma {:.1} more than 12 off the JS card-01 record {js:.1}", luma(w.sun));
    assert!(steps >= 15.0, "11 h sunlit joint contrast {steps:.1} steps under 15");
    assert!(luma(w.sun) > luma(card.sun), "the footway (albedo 0.30) must read over an 18 % card");
}


/// the CC0 scans keep the tones pinned above.
///
///   * a kit finish drawing a scan keeps its bake's mean colour: a natural set
///     (brick, stone, concrete, metal, roofing) scaled channel by channel to
///     it, a painted set (clapboard, stucco, block, shingle, garage door)
///     contributing its luminance only, around the bake's mean (cc0.rs
///     `material_table`); checked on every mapped finish with the sets' own
///     measured means (from the CC0 cache when the downloads are present, else
///     a spread of synthetic means: the identity holds for any mean);
///   * what the shader adds on top moves a wall's mean by at most the crevice
///     grime (-grime x 0.8 on the lowest relief) and the macro tone field
///     (+-6 %); the painted stucco wall at both extremes, under the 10:30 rig,
///     must still reach the earlier browser version's sunlit record (201-208 at
///     #edf2f7), stay out of clipping facing the sun, and keep the shaded
///     wall's chroma inside the JS frames' band (B/R <= 1.25);
///   * the street's CC0 concrete and asphalt scans enter normalised to the JS
///     sets' means and the ground's scans are mean one around their tone (a
///     normalised set's mean is exactly the JS mean: checked on the measured
///     means), so the footway checks above hold unchanged; the carriageway's
///     base tone is the one deliberate change (aged asphalt, neutral grey:
///     `cc0_carriageway_reads_dark_neutral_grey_in_sun` below).
#[test]
fn cc0_scans_keep_the_pinned_wall_and_ground_tone() {
    use eo_assets::cc0 as ac;
    use eo_render::cc0;
    let root = ac::root().unwrap_or_default();
    let real = ac::root().is_some() && ac::available(&root).is_ok();
    let infos: Vec<cc0::LayerInfo> = ac::MATERIAL_SETS
        .iter()
        .enumerate()
        .map(|(i, def)| {
            if real {
                if let Ok(s) = ac::load_cached(&root, def, Some(&eo_assets::default_cache_dir())) {
                    return cc0::LayerInfo::from_set(&s);
                }
            }
            let k = i as f32;
            cc0::LayerInfo { mean_rgb: [0.05 + 0.011 * k, 0.04 + 0.009 * k, 0.03 + 0.007 * k], mean_rough: 0.8, mean_ao: 0.9, mean_height: 0.5, size_m: [2.0, 2.0] }
        })
        .collect();
    let lib = eo_assets::kit::KitLibrary::load().unwrap();
    let mut mt = eo_render::authored::MaterialTable::new();
    mt.register_kit_palette(&lib);
    let mean_of = |source: &str| -> [f32; 3] {
        let spec = &lib.specs[source];
        let b = eo_assets::kit::bake(source, spec, eo_assets::kit::RUNTIME_SEED, 64);
        let mut s = [0f64; 3];
        for px in b.albedo.chunks(4) {
            for c in 0..3 {
                s[c] += srgb_to_linear(px[c] as f64 / 255.0);
            }
        }
        let n = (b.albedo.len() / 4) as f64;
        s.map(|v| (v / n) as f32)
    };
    let kit_means: Vec<[f32; 3]> = mt.layers.iter().map(|src| mean_of(src)).collect();
    let (table, mapped) = cc0::material_table(&mt, &kit_means, &infos);
    println!("cc0: {mapped} kit materials draw a scan ({} set means)", if real { "measured" } else { "synthetic" });
    assert!(mapped >= 15, "only {mapped} finishes mapped");
    let mut stucco_mean = None;
    for (i, g) in mt.gpu.iter().enumerate() {
        let e = &table[i * cc0::MAT_VEC4..i * cc0::MAT_VEC4 + 4];
        if e[0][0] < 0.0 {
            continue;
        }
        let name = mt.names[i].split('@').next().unwrap().to_string();
        let class = cc0::kit_class(&name).unwrap();
        let km = kit_means[g.layer as usize];
        let mode = (e[1][3] as u32) & 15;
        for (layer, mul) in [(e[0][0], [e[1][0], e[1][1], e[1][2]]), (e[0][1], [e[2][0], e[2][1], e[2][2]])] {
            if layer < 0.0 {
                continue;
            }
            let li = &infos[layer as usize];
            let got = match mode {
                cc0::MODE_PAINTED => mul.map(|m| m * li.luma()),
                _ => [0, 1, 2].map(|k| mul[k] * li.mean_rgb[k]),
            };
            for k in 0..3 {
                let unclamped = mul[k] > 0.0201 && mul[k] < 49.99;
                assert!(!unclamped || (got[k] - km[k]).abs() <= 1e-4 * km[k].max(1e-3) + 1e-6, "{name}: channel {k} delivers {} where its bake's mean is {}", got[k], km[k]);
            }
        }
        if name == "painted-stucco" {
            stucco_mean = Some((km, class.grime));
        }
    }
    // The street's normalisation: a set over its own mean times the JS mean is the JS mean.
    let m = eo_street::material::StreetMaterial::get();
    for (l, js) in [("dirty_concrete", m.pbr_sets.concrete.base_mean_linear), ("worn_asphalt", m.pbr_sets.carriageway.base_mean_linear)] {
        let li = &infos[cc0::layer_of(l).unwrap() as usize];
        if let Some(js) = js {
            let back = [0, 1, 2].map(|k| li.mean_rgb[k] / li.mean_rgb[k].max(1e-3) * js[k] as f32);
            assert!((0..3).all(|k| (back[k] - js[k] as f32).abs() < 1e-5), "{l}: normalised mean {back:?} != the JS {js:?}");
        }
    }
    // The painted stucco wall at the shader's extremes, against the JS records (as the facade test above).
    let (km, grime) = stucco_mean.expect("painted-stucco is mapped");
    let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let f = LookFrame::new(10.5);
    let kh = (f.key_dir[0].hypot(f.key_dir[2])).max(1e-6);
    let toward = [f.key_dir[0] / kh, 0.0, f.key_dir[2] / kh];
    let up = toward[1] * 0.5 + 0.5;
    let hemi = [0, 1, 2].map(|i| f.hemi_ground[i] + (f.hemi_sky[i] - f.hemi_ground[i]) * up);
    let sh = sh_eval(&f.env_sh, toward);
    let fill = [0, 1, 2].map(|i| hemi[i] + f.ambient[i] + sh[i] * f.rig.environment_intensity);
    let tint = eo_assets::kit::hex_to_linear("#edf2f7").unwrap();
    let weather = 1.0 - 0.18 * 0.5;
    for (what, k) in [("crevice grime at its deepest", 1.0 - grime as f64 * 0.8), ("macro field high", 1.06), ("mean", 1.0)] {
        // This port's own bake mean (the listed deviation: the JS borrows the brighter interior plaster bake).
        let albedo = [0, 1, 2].map(|i| km[i] as f64 * tint[i] * weather * k);
        let at = |nl: f64| {
            let lit = [0, 1, 2].map(|i| (f.key[i] * nl + fill[i]) * albedo[i] / std::f64::consts::PI);
            tone::display(lit, f.exposure, [0.5, 0.5]).map(|x| x * 255.0)
        };
        let (sun, dark) = (at(kh), at(0.0));
        println!("cc0 painted stucco ({what}): sun-facing {:?} luma {:.0}, shade {:?} B/R {:.2}", sun.map(|x| x.round()), luma(sun), dark.map(|x| x.round()), dark[2] / dark[0].max(1.0));
        assert!(luma(sun) < 245.0, "cc0 stucco ({what}) clips facing the sun");
        assert!(dark[2] / dark[0].max(1.0) <= 1.25, "cc0 stucco ({what}): shaded B/R over the JS band");
        if k >= 1.0 {
            assert!(luma(sun) >= 194.0, "cc0 stucco ({what}): sun-facing luma {:.0} cannot reach the JS's recorded 194-208", luma(sun));
        }
    }
}

/// the carriageway with the CC0 street on reads dark neutral grey in
/// the sun (the look study: the JS tone read tan and too bright, and a matched
/// capture showed the asphalt going almost white).
///
///   * albedo: `cc0::ASPHALT_ALBEDO` (aged asphalt pavement, LBNL Heat Island
///     Group: weathered 0.10-0.18, fresh 0.04-0.06), neutral grey; each
///     street's class and age vary it as luminance around the JS mid-cycle
///     residential (`cc0::asphalt_albedo`, the shader's `cc0_asphalt_base`,
///     the JS spread compressed by `ASPHALT_SPREAD`):
///     checked at the brightest class at the most bleached age and the
///     darkest class at the freshest;
///   * delivered, 9-17 h: sunlit asphalt inside the photographs' 55-130 luma
///     (the JS age rule's own record, street-surface-age.js), at least 40
///     luma under the sunlit footway, and no warmer in the sun than a neutral
///     grey of its own luminance under the same rig (the asphalt adds no tan
///     of its own); the
///     shaded chroma inside the JS frames' band and the 11 h lit/shadow ratio
///     inside the JS card's, as the JS street above.
///
/// Reported, not pinned: the rig's key is warm at every hour (a neutral grey
/// reads B/R 0.74-0.76 in the sun) and the renderer has no white balance, so
/// a neutral albedo still carries that warmth in the sun; cancelling it in
/// the albedo (`ASPHALT_SUN_NEUTRAL`, the sweep printed first) pushes the
/// shaded asphalt past the JS frames' blue band (B/R 2.0 at half), so the
/// grey stays neutral (the setting 0).
#[test]
fn cc0_carriageway_reads_dark_neutral_grey_in_sun() {
    let m = eo_street::material::StreetMaterial::get();
    let base = m.pbr_sets.carriageway.base_mean_linear.unwrap();
    let footway = lin(m.tint("sidewalk")).map(|c| c * m.concrete_base_mean_linear * eo_render::look::concrete_aged_fold());
    let env = |k: &str| m.env_base.get(k).copied().unwrap_or(1.0);
    let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let br = |c: [f64; 3]| c[2] / c[0].max(1.0);
    let a = &m.surface_age;
    // Reported: how much of the noon rig's warmth the grey cancels (`ASPHALT_SUN_NEUTRAL`) against the sunlit and shaded chroma.
    for share in [0.0, 0.5, 0.75, 1.0] {
        let t = eo_render::cc0::asphalt_tint_with(share);
        let alb = [0, 1, 2].map(|i| eo_render::cc0::ASPHALT_ALBEDO as f64 * t[i] as f64);
        let row: Vec<String> = [9.0, 11.0, 12.0, 15.0, 17.0]
            .iter()
            .map(|&h| {
                let o = shade(&LookFrame::new(h), alb, [0.0, 1.0, 0.0], env("asphalt"));
                format!("{h} h sun B/R {:.2} luma {:.0} shade B/R {:.2}", br(o.sun), luma(o.sun), br(o.shade))
            })
            .collect();
        println!("cc0 asphalt grey cancelling {share:.2} of the noon warmth (tint {t:?}): {}", row.join("; "));
    }
    for (class, bleach) in [("residential", (a.bleach_fresh + a.bleach_aged) / 2.0), ("service", a.bleach_aged), ("motorway", a.bleach_fresh)] {
        let srgb = m.asphalt_tint(class).map(|v| ((v as f64 * bleach).round()) as u8);
        let js = [0, 1, 2].map(|i| lin(srgb)[i] * base[i]);
        let l = eo_render::cc0::asphalt_albedo(luminance(js));
        let tint = eo_render::cc0::asphalt_tint();
        let albedo = [0, 1, 2].map(|i| l * tint[i] as f64);
        for hour in [9.0, 11.0, 12.0, 17.0, 18.5] {
            let f = LookFrame::new(hour);
            let o = shade(&f, albedo, [0.0, 1.0, 0.0], env("asphalt"));
            let w = shade(&f, footway, [0.0, 1.0, 0.0], env("sidewalk"));
            println!(
                "cc0 asphalt {class} (bleach {bleach:.2}, albedo {l:.3}) {hour:>4} h: sun {:?} luma {:.0} (B/R {:.2}), shade {:?} (B/R {:.2}), footway sun luma {:.0}",
                o.sun.map(|x| x.round()),
                luma(o.sun),
                br(o.sun),
                o.shade.map(|x| x.round()),
                br(o.shade),
                luma(w.sun)
            );
            if hour > 17.5 {
                continue; // golden hour: reported (the low sun is meant to warm it)
            }
            assert!((55.0..=130.0).contains(&luma(o.sun)), "{class} {hour} h: sunlit asphalt luma {:.0} outside the photographs' 55-130", luma(o.sun));
            assert!(luma(w.sun) - luma(o.sun) >= 40.0, "{class} {hour} h: sunlit asphalt {:.0} not 40 luma under the footway {:.0}", luma(o.sun), luma(w.sun));
            // A neutral grey of the asphalt's own luminance under the same light (the tone curve's hue shift with level included).
            let card = shade(&f, [luminance(albedo); 3], [0.0, 1.0, 0.0], env("asphalt"));
            assert!(br(o.sun) >= br(card.sun) - 0.01, "{class} {hour} h: sunlit asphalt B/R {:.2} warmer than a neutral grey of its luminance ({:.2})", br(o.sun), br(card.sun));
            assert!(br(o.shade) <= 1.6, "{class} {hour} h: shaded asphalt B/R {:.2} over the JS frames' 1.53", br(o.shade));
            if hour == 11.0 {
                let r = o.sun_l / o.shade_l;
                assert!((4.5..=7.5).contains(&r), "{class} 11 h lit/shadow {r:.2} outside the JS card's 5.0-7.0 (+-0.5)");
            }
        }
    }
}
