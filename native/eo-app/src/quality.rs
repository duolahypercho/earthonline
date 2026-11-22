//! `--quality low|medium|high|hyper` and the lighting switches.
//!
//! A tier expands to settings once every argument has been read
//! (`Switches::apply`); a per-feature switch given anywhere on the line wins
//! over the tier. `medium` is the default, so `--quality medium` is the same
//! as no flag; it expands to the street look (`--grime 1`, `--stand-ins`,
//! `--surveyed-furniture`, `--tree-basins`), which the owner approved as the default on
//! 2026-11-20 after the citywide street audit, with `--tree-basins`;
//! `--no-grime`, `--no-stand-ins`, `--no-surveyed-furniture` and
//! `--no-tree-basins` give the frame before it.
//!
//! | Tier | Sets |
//! | --- | --- |
//! | low | `--shadow-map 2048`, `--no-ssr`, `--no-contact-shadows`, `--sky-quality low`, `--no-ao`, `--no-shafts`, `--no-bloom`, `--surveyed-furniture`, `--tree-basins` |
//! | medium | `--grime 1`, `--stand-ins`, `--surveyed-furniture`, `--tree-basins` (the street look, made the default with the owner's approval of 2026-11-20) |
//! | high | `--cascade-blend`, `--far-shadows 1`, `--ambient-volume 1` (with `--ground-bounce`), `--light-shadows 16`, `--headlight-shadows 2`, `--lit-fog 1`, `--smooth-time`, `--shadow-slope-bias`, `--grime 1`, `--street-life 1`, `--stand-ins`, `--photo-finish 1`, `--facade-clutter 1`, `--shop-light 1`, `--car-detail 1`, `--surveyed-furniture`, `--tree-basins`, `--look cinematic`, `--water-wind-texture`, `--water-lean` |
//! | hyper | high, then `--shadow-map 4096`, `--sky-quality high`, `--far-shadows 2`, `--ambient-volume 2`, `--probe-grid 2`, `--light-shadows 48`, `--light-shadow-moving`, `--headlight-shadows 8`, `--lit-fog 2`, `--grime 2`, `--street-life 2`, `--photo-finish 2`, `--facade-clutter 2`, `--shop-light 2`, `--car-detail 2`, `--surveyed-furniture`, `--tree-basins`, `--look cinematic` |
//!
//! Each tier has its own frame budget for the bench's checks (`budget_ms`).
//! `--water-wind-texture` (the sea's cat's-paw gust patches, the tidal jet's
//! rough water through the Golden Gate and the rotated re-read of the chop
//! cascade; eo-render water.rs `FEAT_WIND`) and `--water-lean` (the
//! anisotropic cascade fade and the sub-pixel slope variance; eo-water
//! `switches::LEAN`) are sea detail the high and hyper tiers turn on; each
//! `--no-` form turns it off wherever it stands. Together they cost the
//! medium water pass about 0.5 ms on `bay` (3.1 -> 2.6 ms without them,
//! with the features compiled in as constants), so medium leaves them out.
//! The tiers are tuned to those budgets on the bench paths (stage T2): the
//! probe grid is hyper's alone, because every lit pixel reading two probes
//! cost high 4.9 ms on `city_overview` (20.3 -> 15.4 ms median without it);
//! low also drops the surface occlusion pass, the sun shafts and the bloom
//! (0.3-0.6 ms on the street paths). `--no-ao`, `--no-shafts` and `--no-bloom`
//! have no positive form, so the low tier always drops them.
//! The lighting switches take an optional level; with none they take the
//! `high` tier's (`--probe-grid`, which high leaves off, takes 1):
//! `--far-shadows [1|2]`, `--ambient-volume [1|2]`, `--probe-grid [1|2]`,
//! `--light-shadows [N]` (lamps, rounded up to a multiple of 16, at most 48), `--headlight-shadows [N]` (vehicles),
//! `--lit-fog [1|2]`; and the plain `--cascade-blend`,
//! `--light-shadow-moving`, `--smooth-time`. Each has a `--no-` form.
//! `--ground-bounce` (light bounced up from the sunlit ground) lives in the
//! ambient volume: it is on whenever the volume is, unless
//! `--no-ground-bounce`, and never without the volume.
//! `--shadow-slope-bias` (the sun cascades' casters drawn with a
//! slope-scaled depth bias, so surfaces the sun grazes stop shadowing
//! themselves; `LookOptions::shadow_slope_bias`) is a shadow setting the
//! high and hyper tiers turn on, with its `--no-` form.
//! `--grime [1|2]` (surface weathering that reads at normal viewing
//! distance, stage W1; `LookOptions::grime`, eo-render weathering.rs) is a
//! surface setting the high (1) and hyper (2) tiers turn on; with no value it
//! takes 1; `--no-grime` turns it off wherever it stands.
//! `--street-life [1|2]` (litter and leaves, graffiti, posters and stickers,
//! stage W2b; `LookOptions::street_life`, eo-render street_life.rs) is a
//! surface setting the high (1) and hyper (2) tiers turn on, as `--grime`:
//! no value takes 1, `--no-street-life` turns it off wherever it stands.
//! `--stand-ins` (procedural models for street-prop kinds whose GLB is absent
//! from the checkout, stage W2a; `Props2Args::stand_ins`) is on in the high and
//! hyper tiers, with its `--no-` form; off, the grey proxies as before.
//! `--photo-finish [1|2]` (a camera's response, sensor noise and lens in the
//! post pass, San Francisco's measured air in the sky, a lens's bloom, stage
//! W4; `LookOptions::photo_finish`, eo-render photo.rs) is a finish the high
//! (1) and hyper (2) tiers turn on, as `--grime`: no value takes 1,
//! `--no-photo-finish` turns it off wherever it stands.
//! `--facade-clutter [1|2]` (fire escapes, awnings, signs, shutters,
//! downspouts, meters and roof clutter on the kit buildings, stage W3;
//! `Args::facade_clutter`, eo-kit clutter.rs; level 2 adds window air
//! conditioners, front dishes, scaffolding and boarded shops) is a world
//! detail the high (1) and hyper (2) tiers turn on, as `--grime`: no value
//! takes 1, `--no-facade-clutter` turns it off wherever it stands.
//! `--shop-light [1|2]` (the shop windows lit at night by their trade's
//! hours, colour and luminance, spilling onto the sidewalk, stage N1;
//! `SkyOptions::shop_light`, eo-render shop_lights.rs; 1 within 160 m of the
//! eye, 2 out to the light clusters' 300 m with two lights a window) is a
//! night setting the high (1) and hyper (2) tiers turn on, as `--grime`: no
//! value takes 1, `--no-shop-light` turns it off wherever it stands.
//! `--surveyed-furniture` (eo-props `furniture::surveyed_first`: the hashed
//! furniture walk leaves empty a slot it gives to a kind rare on SF footways,
//! and one it gives to a street-commerce kind away from a trading frontage;
//! `Props2Args::surveyed_furniture`) is on in every tier;
//! `--no-surveyed-furniture` gives the JS walk as ported.
//! `--tree-basins` (eo-props `basins`: a soil and mulch basin under each street
//! tree on a footway, sized by the footway from Director's Order 187,246;
//! `Props2Args::tree_basins`) is on in every tier; `--no-tree-basins` gives
//! the trees standing on bare paving as before.
//! `--car-detail [1|2]` (the cars' detail: the far slot a simplified body
//! instead of three boxes, eo-agents `far_body`, and the nearest cars kept at
//! full detail further out, eo-render `Tiers::with_car_detail`; `Args::car_detail`)
//! is a detail setting the high (1) and hyper (2) tiers turn on, as `--grime`:
//! no value takes 1, `--no-car-detail` turns it off wherever it stands.
//! `--look default|cinematic` (the look profile, the cinematic look plan's
//! stage L0; `LookOptions::look` and `SkyOptions::cinematic`, eo-look
//! `cinematic`: a graded open-world tone, grade and light palette whose
//! departures from the default's physical references are listed in
//! `CINEMATIC_DEVIATIONS`) is a look the high and hyper tiers turn on;
//! `--no-look` (the same as `--look default`) turns it off wherever it stands.
//!
//! The world switches `--structures` and `--tunnels` (and their `--no-`
//! forms) are read here too, so every parser takes them alike; no tier sets
//! them (they change collision and traffic, so client and server must agree).
//! `--tunnels` matters only with `--structures`; the last of a switch and its
//! `--no-` form wins.

use crate::Args;
use anyhow::{Result, anyhow, bail};
use eo_render::look::{LightingOptions, LookProfile};

/// The quality tier (`--quality`); `Medium` is the default (the street look).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    Low,
    #[default]
    Medium,
    High,
    Hyper,
}

/// What a tier sets; `None` leaves the value as it was.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tier {
    pub shadow_map: Option<u32>,
    pub ssr: Option<bool>,
    pub contact_shadows: Option<bool>,
    /// `--sky-quality` (0 low, 1 the default, 2 high).
    pub sky_quality: Option<u32>,
    /// `--shadow-slope-bias` (`LookOptions::shadow_slope_bias`).
    pub shadow_slope_bias: Option<bool>,
    /// `--no-ao` (`LookOptions::ao`: the surfaces' occlusion + bounce pass), `--no-shafts`, `--no-bloom` (sky).
    pub ao: Option<bool>,
    pub shafts: Option<bool>,
    pub bloom: Option<bool>,
    /// `--grime` (`LookOptions::grime`, surface weathering: 1 high, 2 hyper).
    pub grime: Option<u8>,
    /// `--street-life` (`LookOptions::street_life`, litter, graffiti, posters: 1 high, 2 hyper).
    pub street_life: Option<u8>,
    /// `--stand-ins` (`Props2Args::stand_ins`, procedural models for absent prop GLBs).
    pub stand_ins: Option<bool>,
    /// `--surveyed-furniture` (`Props2Args::surveyed_furniture`, eo-props `furniture::surveyed_first`).
    pub surveyed_furniture: Option<bool>,
    /// `--tree-basins` (`Props2Args::tree_basins`, eo-props `basins`).
    pub tree_basins: Option<bool>,
    /// `--photo-finish` (`LookOptions::photo_finish`, the camera, the air, the lens: 1 high, 2 hyper).
    pub photo_finish: Option<u8>,
    /// `--facade-clutter` (`Args::facade_clutter`, fire escapes, awnings, signs, services: 1 high, 2 hyper).
    pub facade_clutter: Option<u8>,
    /// `--shop-light` (`SkyOptions::shop_light`, lit shop windows at night: 1 high, 2 hyper).
    pub shop_light: Option<u8>,
    /// `--car-detail` (`Args::car_detail`, far bodies and nearer full-detail cars: 1 high, 2 hyper).
    pub car_detail: Option<u8>,
    /// `--look` (`LookOptions::look`, the look profile: cinematic in high and hyper).
    pub look: Option<LookProfile>,
    /// `--water-wind-texture` (eo-render water.rs `FEAT_WIND` in `LookOptions::water_features`: high and hyper).
    pub water_wind: Option<bool>,
    /// `--water-lean` (eo-water `switches::LEAN`, left out through `LookOptions::water_parts_off` below high).
    pub water_lean: Option<bool>,
    pub lighting: LightingOptions,
}

/// Street lamps per `LightingOptions::light_shadows` step, and the most there are.
const LAMPS_PER_STEP: u32 = 16;
const MAX_LIGHT_SHADOW_STEPS: u32 = 3;
/// `--probe-grid` with no level: 1 (four probes of 128 px). The high tier
/// leaves the grid off (its cost, stage T2), so it cannot be the default.
const PROBE_GRID_BARE: u8 = 1;

impl Quality {
    pub const ALL: [Quality; 4] = [Quality::Low, Quality::Medium, Quality::High, Quality::Hyper];

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "low" => Quality::Low,
            "medium" => Quality::Medium,
            "high" => Quality::High,
            "hyper" => Quality::Hyper,
            other => bail!("--quality low|medium|high|hyper, not {other}"),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Quality::Low => "low",
            Quality::Medium => "medium",
            Quality::High => "high",
            Quality::Hyper => "hyper",
        }
    }

    /// The whole-frame budget (CPU + GPU, ms at 1920 x 1080) the bench checks
    /// this tier against; `--budget-ms` overrides it.
    pub fn budget_ms(self) -> f64 {
        match self {
            Quality::Low => 8.0,
            Quality::Medium => crate::bench::FRAME_BUDGET_MS,
            Quality::High => 16.6,
            Quality::Hyper => 33.3,
        }
    }

    /// What the tier sets. The sky quality and the shadow map only where they
    /// differ from the defaults (the default sky quality is 1, the map 3072).
    pub fn tier(self) -> Tier {
        let none = Tier { shadow_map: None, ssr: None, contact_shadows: None, sky_quality: None, shadow_slope_bias: None, ao: None, shafts: None, bloom: None, grime: None, street_life: None, stand_ins: None, surveyed_furniture: None, tree_basins: None, photo_finish: None, facade_clutter: None, shop_light: None, car_detail: None, look: None, water_wind: None, water_lean: None, lighting: LightingOptions::OFF };
        let high = LightingOptions { cascade_blend: true, far_shadows: 1, ambient_volume: 1, probe_grid: 0, light_shadows: 1, light_shadow_moving: false, headlight_shadows: 2, lit_fog: 1, smooth_time: true, ground_bounce: true, rt_gi: false, rt_reflect: false };
        match self {
            Quality::Low => Tier { shadow_map: Some(2048), ssr: Some(false), contact_shadows: Some(false), sky_quality: Some(0), ao: Some(false), shafts: Some(false), bloom: Some(false), surveyed_furniture: Some(true), tree_basins: Some(true), ..none },
            // the street look, the default since the owner's approval of 2026-11-20 (citywide street audit).
            Quality::Medium => Tier { grime: Some(1), stand_ins: Some(true), surveyed_furniture: Some(true), tree_basins: Some(true), ..none },
            Quality::High => Tier { shadow_slope_bias: Some(true), grime: Some(1), street_life: Some(1), stand_ins: Some(true), surveyed_furniture: Some(true), tree_basins: Some(true), photo_finish: Some(1), facade_clutter: Some(1), shop_light: Some(1), car_detail: Some(1), look: Some(LookProfile::Cinematic), water_wind: Some(true), water_lean: Some(true), lighting: high, ..none },
            Quality::Hyper => Tier {
                shadow_map: Some(4096),
                sky_quality: Some(2),
                shadow_slope_bias: Some(true),
                grime: Some(2),
                street_life: Some(2),
                stand_ins: Some(true),
                surveyed_furniture: Some(true),
                tree_basins: Some(true),
                photo_finish: Some(2),
                facade_clutter: Some(2),
                shop_light: Some(2),
                car_detail: Some(2),
                look: Some(LookProfile::Cinematic),
                water_wind: Some(true),
                water_lean: Some(true),
                lighting: LightingOptions { far_shadows: 2, ambient_volume: 2, probe_grid: 2, light_shadows: 3, light_shadow_moving: true, headlight_shadows: 8, lit_fog: 2, ..high },
                ..none
            },
        }
    }
}

/// The quality and lighting switches as one command line gives them,
/// applied once every argument is read (`apply`).
#[derive(Clone, Debug, Default)]
pub struct Switches {
    quality: Option<Quality>,
    // the lighting switches given (each wins over the tier).
    cascade_blend: Option<bool>,
    far_shadows: Option<u8>,
    ambient_volume: Option<u8>,
    probe_grid: Option<u8>,
    light_shadows: Option<u8>,
    light_shadow_moving: Option<bool>,
    headlight_shadows: Option<u8>,
    lit_fog: Option<u8>,
    smooth_time: Option<bool>,
    ground_bounce: Option<bool>,
    // the ray-traced diffuse light (`--rt-gi`, `--no-rt-gi`; no tier sets it).
    rt_gi: Option<bool>,
    // the ray-traced reflections (`--rt-reflect`, `--no-rt-reflect`; no tier sets it).
    rt_reflect: Option<bool>,
    // a shadow switch a tier sets (each wins over the tier).
    shadow_slope_bias: Option<bool>,
    // surface weathering (`--grime [1|2]`, `--no-grime`; wins over the tier).
    grime: Option<u8>,
    // street life (`--street-life [1|2]`, `--no-street-life`; wins over the tier).
    street_life: Option<u8>,
    // procedural stand-ins for absent street-prop GLBs (`--stand-ins`, `--no-stand-ins`).
    stand_ins: Option<bool>,
    // the surveyed-first furniture walk (`--surveyed-furniture`, `--no-surveyed-furniture`).
    surveyed_furniture: Option<bool>,
    // the street trees' basins (`--tree-basins`, `--no-tree-basins`).
    tree_basins: Option<bool>,
    // the photographic finish (`--photo-finish [1|2]`, `--no-photo-finish`; wins over the tier).
    photo_finish: Option<u8>,
    // the facade clutter (`--facade-clutter [1|2]`, `--no-facade-clutter`; wins over the tier).
    facade_clutter: Option<u8>,
    // the lit shop windows (`--shop-light [1|2]`, `--no-shop-light`; wins over the tier).
    shop_light: Option<u8>,
    // the cars' detail (`--car-detail [1|2]`, `--no-car-detail`; wins over the tier).
    car_detail: Option<u8>,
    // the look profile (`--look default|cinematic`, `--no-look`; wins over the tier).
    look: Option<LookProfile>,
    // the sea's wind texture and lean (`--water-wind-texture`, `--water-lean` and their `--no-` forms; win over the tier).
    water_wind: Option<bool>,
    water_lean: Option<bool>,
    // the world switches (no tier sets them).
    structures: Option<bool>,
    tunnels: Option<bool>,
    // existing switches a tier also sets, seen on the line (the tier then leaves them).
    shadow_map_given: bool,
    ssr_given: bool,
    contact_given: bool,
    sky_quality_given: bool,
}

/// The argument after a switch as a number, if it is one (an optional value).
fn number(next: Option<&str>) -> Option<u32> {
    next.and_then(|s| s.parse::<u32>().ok())
}

/// `--flag [1|2]`: the level given, or `default`; 0 is off. Returns (level, arguments consumed).
fn level(flag: &str, next: Option<&str>, default: u8) -> Result<(u8, usize)> {
    match number(next) {
        Some(v @ 0..=2) => Ok((v as u8, 1)),
        Some(v) => bail!("{flag} takes 1 or 2 (0 is off), not {v}"),
        None => Ok((default, 0)),
    }
}

impl Switches {
    /// Takes `flag` if it is `--quality`, a lighting switch or a world switch
    /// (`--structures`, `--tunnels` and their `--no-` forms); `next` is the
    /// argument after it, not yet consumed. Returns None when the flag is not
    /// one of these (the caller parses it), else how many of the following
    /// arguments it consumed (0 or 1). `--shadow-map`, `--no-ssr`,
    /// `--no-contact-shadows`, `--sky-quality` and `--no-sky` are noted (the
    /// tier then leaves what they set) and left to the caller.
    pub fn take(&mut self, flag: &str, next: Option<&str>) -> Result<Option<usize>> {
        let high = Quality::High.tier().lighting;
        let taken = match flag {
            "--quality" => {
                self.quality = Some(Quality::parse(next.ok_or_else(|| anyhow!("--quality needs low|medium|high|hyper"))?)?);
                1
            }
            "--cascade-blend" | "--no-cascade-blend" => {
                self.cascade_blend = Some(flag == "--cascade-blend");
                0
            }
            "--far-shadows" => {
                let (v, n) = level(flag, next, high.far_shadows)?;
                self.far_shadows = Some(v);
                n
            }
            "--ambient-volume" => {
                let (v, n) = level(flag, next, high.ambient_volume)?;
                self.ambient_volume = Some(v);
                n
            }
            "--probe-grid" => {
                let (v, n) = level(flag, next, PROBE_GRID_BARE)?;
                self.probe_grid = Some(v);
                n
            }
            "--lit-fog" => {
                let (v, n) = level(flag, next, high.lit_fog)?;
                self.lit_fog = Some(v);
                n
            }
            "--light-shadows" => {
                // lamps, rounded up to a multiple of 16, at most 48.
                let (steps, n) = match number(next) {
                    Some(lamps) => (lamps.div_ceil(LAMPS_PER_STEP).min(MAX_LIGHT_SHADOW_STEPS) as u8, 1),
                    None => (high.light_shadows, 0),
                };
                self.light_shadows = Some(steps);
                n
            }
            "--headlight-shadows" => {
                let (v, n) = match number(next) {
                    Some(v) => (u8::try_from(v).map_err(|_| anyhow!("--headlight-shadows takes at most 255 vehicles, not {v}"))?, 1),
                    None => (high.headlight_shadows, 0),
                };
                self.headlight_shadows = Some(v);
                n
            }
            "--light-shadow-moving" | "--no-light-shadow-moving" => {
                self.light_shadow_moving = Some(flag == "--light-shadow-moving");
                0
            }
            "--smooth-time" | "--no-smooth-time" => {
                self.smooth_time = Some(flag == "--smooth-time");
                0
            }
            "--ground-bounce" | "--no-ground-bounce" => {
                self.ground_bounce = Some(flag == "--ground-bounce");
                0
            }
            "--rt-gi" | "--no-rt-gi" => {
                self.rt_gi = Some(flag == "--rt-gi");
                0
            }
            "--rt-reflect" | "--no-rt-reflect" => {
                self.rt_reflect = Some(flag == "--rt-reflect");
                0
            }
            "--shadow-slope-bias" | "--no-shadow-slope-bias" => {
                self.shadow_slope_bias = Some(flag == "--shadow-slope-bias");
                0
            }
            "--grime" => {
                let (v, n) = level(flag, next, Quality::High.tier().grime.unwrap_or(1))?;
                self.grime = Some(v);
                n
            }
            "--no-grime" => {
                self.grime = Some(0);
                0
            }
            "--street-life" => {
                let (v, n) = level(flag, next, Quality::High.tier().street_life.unwrap_or(1))?;
                self.street_life = Some(v);
                n
            }
            "--no-street-life" => {
                self.street_life = Some(0);
                0
            }
            "--stand-ins" | "--no-stand-ins" => {
                self.stand_ins = Some(flag == "--stand-ins");
                0
            }
            "--surveyed-furniture" | "--no-surveyed-furniture" => {
                self.surveyed_furniture = Some(flag == "--surveyed-furniture");
                0
            }
            "--tree-basins" | "--no-tree-basins" => {
                self.tree_basins = Some(flag == "--tree-basins");
                0
            }
            "--photo-finish" => {
                let (v, n) = level(flag, next, Quality::High.tier().photo_finish.unwrap_or(1))?;
                self.photo_finish = Some(v);
                n
            }
            "--no-photo-finish" => {
                self.photo_finish = Some(0);
                0
            }
            "--facade-clutter" => {
                let (v, n) = level(flag, next, Quality::High.tier().facade_clutter.unwrap_or(1))?;
                self.facade_clutter = Some(v);
                n
            }
            "--no-facade-clutter" => {
                self.facade_clutter = Some(0);
                0
            }
            "--shop-light" => {
                let (v, n) = level(flag, next, Quality::High.tier().shop_light.unwrap_or(1))?;
                self.shop_light = Some(v);
                n
            }
            "--no-shop-light" => {
                self.shop_light = Some(0);
                0
            }
            "--car-detail" => {
                let (v, n) = level(flag, next, Quality::High.tier().car_detail.unwrap_or(1))?;
                self.car_detail = Some(v);
                n
            }
            "--no-car-detail" => {
                self.car_detail = Some(0);
                0
            }
            "--look" => {
                let v = next.ok_or_else(|| anyhow!("--look needs default|cinematic"))?;
                self.look = Some(LookProfile::parse(v).ok_or_else(|| anyhow!("--look default|cinematic, not {v}"))?);
                1
            }
            "--no-look" => {
                self.look = Some(LookProfile::Default);
                0
            }
            "--no-far-shadows" => {
                self.far_shadows = Some(0);
                0
            }
            "--no-ambient-volume" => {
                self.ambient_volume = Some(0);
                0
            }
            "--no-probe-grid" => {
                self.probe_grid = Some(0);
                0
            }
            "--no-light-shadows" => {
                self.light_shadows = Some(0);
                0
            }
            "--no-headlight-shadows" => {
                self.headlight_shadows = Some(0);
                0
            }
            "--no-lit-fog" => {
                self.lit_fog = Some(0);
                0
            }
            // the sea's wind texture and lean: a `--no-` form wins wherever it stands
            // (`--no-water-lean` is also read by eo-water itself, `switches`)
            "--water-wind-texture" => {
                self.water_wind = Some(self.water_wind != Some(false));
                0
            }
            "--no-water-wind-texture" => {
                self.water_wind = Some(false);
                0
            }
            "--water-lean" => {
                self.water_lean = Some(self.water_lean != Some(false));
                0
            }
            "--no-water-lean" => {
                self.water_lean = Some(false);
                0
            }
            "--structures" | "--no-structures" => {
                self.structures = Some(flag == "--structures");
                0
            }
            "--tunnels" | "--no-tunnels" => {
                self.tunnels = Some(flag == "--tunnels");
                0
            }
            // what a tier also sets: noted, parsed by the caller.
            "--shadow-map" => {
                self.shadow_map_given = true;
                return Ok(None);
            }
            "--no-ssr" => {
                self.ssr_given = true;
                return Ok(None);
            }
            "--no-contact-shadows" => {
                self.contact_given = true;
                return Ok(None);
            }
            "--sky-quality" => {
                self.sky_quality_given = true;
                return Ok(None);
            }
            "--no-sky" => {
                (self.ssr_given, self.contact_given, self.sky_quality_given) = (true, true, true);
                return Ok(None);
            }
            _ => return Ok(None),
        };
        Ok(Some(taken))
    }

    /// The tier, then the lighting switches given over it, into `a`.
    pub fn apply(&self, a: &mut Args) {
        let q = self.quality.unwrap_or_default();
        let t = q.tier();
        a.quality = q;
        if let Some(v) = t.shadow_map.filter(|_| !self.shadow_map_given) {
            a.look.shadow_map = v;
        }
        if let Some(v) = t.ssr.filter(|_| !self.ssr_given) {
            a.look.sky.ssr = v;
        }
        if let Some(v) = t.contact_shadows.filter(|_| !self.contact_given) {
            a.look.sky.contact = v;
        }
        if let Some(v) = t.sky_quality.filter(|_| !self.sky_quality_given) {
            a.look.sky.quality = v;
        }
        if let Some(v) = t.ao {
            a.look.ao = v;
        }
        if let Some(v) = t.shafts {
            a.look.sky.shafts = v;
        }
        if let Some(v) = t.bloom {
            a.look.sky.bloom = v;
        }
        if let Some(v) = self.shadow_slope_bias.or(t.shadow_slope_bias) {
            a.look.shadow_slope_bias = v;
        }
        if let Some(v) = self.grime.or(t.grime) {
            a.look.grime = v;
        }
        if let Some(v) = self.street_life.or(t.street_life) {
            a.look.street_life = v;
        }
        if let Some(v) = self.stand_ins.or(t.stand_ins) {
            a.props2.stand_ins = v;
        }
        if let Some(v) = self.surveyed_furniture.or(t.surveyed_furniture) {
            a.props2.surveyed_furniture = v;
        }
        if let Some(v) = self.tree_basins.or(t.tree_basins) {
            a.props2.tree_basins = v;
        }
        if let Some(v) = self.photo_finish.or(t.photo_finish) {
            a.look.photo_finish = v;
        }
        if let Some(v) = self.facade_clutter.or(t.facade_clutter) {
            a.facade_clutter = v;
        }
        if let Some(v) = self.shop_light.or(t.shop_light) {
            a.look.sky.shop_light = v;
        }
        if let Some(v) = self.car_detail.or(t.car_detail) {
            a.car_detail = v;
        }
        if let Some(v) = self.look.or(t.look) {
            // the profile and the sky's copy of it (the sky passes read `SkyOptions` alone).
            a.look.look = v;
            a.look.sky.cinematic = v.is_cinematic();
        }
        // the sea's wind texture and lean: off below high unless given (each compiled out of the water's shaders when off).
        if self.water_wind.or(t.water_wind).unwrap_or(false) {
            a.look.water_features |= eo_render::water::FEAT_WIND;
        } else {
            a.look.water_features &= !eo_render::water::FEAT_WIND;
        }
        a.look.water_parts_off = if self.water_lean.or(t.water_lean).unwrap_or(false) { 0 } else { eo_water::switches::LEAN };
        let mut l = t.lighting;
        let given = |v: &mut u8, s: Option<u8>| *v = s.unwrap_or(*v);
        let given_b = |v: &mut bool, s: Option<bool>| *v = s.unwrap_or(*v);
        given_b(&mut l.cascade_blend, self.cascade_blend);
        given(&mut l.far_shadows, self.far_shadows);
        given(&mut l.ambient_volume, self.ambient_volume);
        given(&mut l.probe_grid, self.probe_grid);
        given(&mut l.light_shadows, self.light_shadows);
        given_b(&mut l.light_shadow_moving, self.light_shadow_moving);
        given(&mut l.headlight_shadows, self.headlight_shadows);
        given(&mut l.lit_fog, self.lit_fog);
        given_b(&mut l.smooth_time, self.smooth_time);
        given_b(&mut l.rt_gi, self.rt_gi);
        given_b(&mut l.rt_reflect, self.rt_reflect);
        // the ground bounce lives in the ambient volume: on with it unless `--no-ground-bounce`, never without it.
        l.ground_bounce = l.ambient_volume > 0 && self.ground_bounce.unwrap_or(true);
        a.look.lighting = l;
        a.structures = self.structures.unwrap_or(false).then(|| eo_structures::StructureOptions { tunnels: self.tunnels.unwrap_or(false) });
    }
}

/// The bench JSON's top-level `lighting` object: the tier and every `LightingOptions` field.
pub fn lighting_json(args: &Args) -> serde_json::Value {
    let l = args.look.lighting;
    serde_json::json!({
        "on": l.any(),
        "quality": args.quality.name(),
        "cascade_blend": l.cascade_blend,
        "far_shadows": l.far_shadows,
        "ambient_volume": l.ambient_volume,
        "probe_grid": l.probe_grid,
        "light_shadows": l.light_shadows,
        "light_shadows_lamps": l.shadowed_lamps(),
        "light_shadow_moving": l.light_shadow_moving,
        "headlight_shadows": l.headlight_shadows,
        "lit_fog": l.lit_fog,
        "smooth_time": l.smooth_time,
        "ground_bounce": l.ground_bounce_on(),
        "rt_gi": l.rt_gi,
        "rt_reflect": l.rt_reflect,
    })
}

/// The bench JSON's `rt` object (`--rt-gi`, `--rt-reflect`): for each pass
/// whether it was asked for and runs (a device with ray queries), its
/// settings and what it did; the traced scene and the G-buffer.
pub fn rt_json(args: &Args, renderer: &eo_render::Renderer) -> serde_json::Value {
    serde_json::json!({
        "gi": {
            "asked": args.look.lighting.rt_gi,
            "live": renderer.rt_gi().is_some(),
            "config": renderer.rt_gi().map(|r| r.config.json()),
            "stats": renderer.rt_gi().map(|r| r.stats().json()),
            "target_bytes": renderer.rt_gi().map(|r| r.target_bytes()),
        },
        "reflect": {
            "asked": args.look.lighting.rt_reflect,
            "live": renderer.rt_reflect().is_some(),
            "config": renderer.rt_reflect().map(|r| r.config.json()),
            "stats": renderer.rt_reflect().map(|r| r.stats().json()),
            "target_bytes": renderer.rt_reflect().map(|r| r.target_bytes()),
        },
        "gbuffer_bytes": renderer.rt_composite().map(|c| c.bytes()),
        "scene_config": renderer.rt_scene().map(|s| s.config.json()),
        "scene_stats": renderer.rt_scene().map(|s| s.stats().json()),
    })
}

/// The bench JSON's top-level `grime` object (surface weathering, `--grime`).
pub fn grime_json(args: &Args) -> serde_json::Value {
    serde_json::json!({
        "on": args.look.grime > 0,
        "level": args.look.grime,
        "weathering_decals": eo_render::cc0_decals::weathering(),
    })
}

/// The bench JSON's top-level `street_life` object (`--street-life`): the level and
/// the survey the placement read (eo-render street_life.rs).
pub fn street_life_json(args: &Args) -> serde_json::Value {
    let s = eo_render::street_life::stats();
    serde_json::json!({
        "on": args.look.street_life > 0,
        "level": args.look.street_life,
        "placing": eo_render::street_life::level(),
        "buildings_classed": s.buildings_classed,
        "trees": s.trees,
        "structures": s.structures,
    })
}

/// The bench JSON's top-level `car_detail` object (`--car-detail`): the level and
/// the vehicle tiers it sets (eo-render `Tiers::with_car_detail`), whether the
/// agents are drawn or not.
pub fn car_detail_json(args: &Args) -> serde_json::Value {
    let tiers = eo_agents::catalog::AgentsJs::load().ok().map(|js| eo_render::agents::Tiers::from_js(&js).with_car_detail(args.car_detail));
    serde_json::json!({
        "on": args.car_detail > 0,
        "level": args.car_detail,
        "far_body": args.car_detail > 0,
        "vehicle_radius_m": tiers.map(|t| t.veh_radius),
        "vehicle_cap_lod0_lod1": tiers.map(|t| [t.veh_cap[0], t.veh_cap[1]]),
    })
}

/// The bench JSON's top-level `surveyed_furniture` object (`--surveyed-furniture`).
pub fn surveyed_furniture_json(args: &Args) -> serde_json::Value {
    serde_json::json!({ "on": args.props2.surveyed_furniture, "rare_kinds": eo_props::furniture::RARE_KINDS, "commerce_kinds": eo_props::furniture::COMMERCE_KINDS })
}

/// The bench JSON's top-level `tree_basins` object (`--tree-basins`).
pub fn tree_basins_json(args: &Args) -> serde_json::Value {
    serde_json::json!({ "on": args.props2.tree_basins })
}

/// The bench JSON's top-level `stand_ins` object (`--stand-ins`).
pub fn stand_ins_json(args: &Args) -> serde_json::Value {
    serde_json::json!({ "on": args.props2.stand_ins })
}

/// The bench JSON's top-level `facade_clutter` object (`--facade-clutter`): the level,
/// the one the planner draws and its kit tile cache tag (eo-kit clutter.rs).
pub fn facade_clutter_json(args: &Args) -> serde_json::Value {
    serde_json::json!({
        "on": args.facade_clutter > 0,
        "level": args.facade_clutter,
        "planning": eo_kit::clutter::level(),
        "cache_tag": eo_kit::clutter::cache_tag(),
    })
}

/// The bench JSON's top-level `shop_light` object (`--shop-light`): the level,
/// the reach the lights are gathered within and the placement's census
/// (`shop_census`, once eo-app's placement has run; null before or without it).
pub fn shop_light_json(args: &Args, census: Option<&eo_render::shop_lights::ShopStats>) -> serde_json::Value {
    serde_json::json!({
        "on": args.look.sky.shop_light > 0,
        "level": args.look.sky.shop_light,
        "reach_m": eo_render::shop_lights::reach_m(args.look.sky.shop_light),
        "census": census.map(|c| serde_json::json!({
            "planned": c.planned, "buildings": c.with_bays, "bays": c.bays, "shops": c.shops, "lights": c.lights,
            "by_kind": c.by_kind.iter().map(|(k, n)| (k.to_string(), serde_json::json!(n))).collect::<serde_json::Map<_, _>>(),
            "vacant": c.vacant, "boarded": c.boarded, "work_lit": c.work_lit, // (stage N2) the one vacancy rule
            "ms": c.ms,
        })),
    })
}

/// The bench JSON's top-level `look_profile` object (`--look`): the profile and
/// the cinematic look's departures from the default's physical references.
pub fn look_profile_json(args: &Args) -> serde_json::Value {
    serde_json::json!({
        "on": args.look.look.is_cinematic(),
        "profile": args.look.look.name(),
        "deviations": eo_render::look::CINEMATIC_DEVIATIONS.iter().map(|d| serde_json::json!({ "name": d.name, "cinematic": d.cinematic, "default": d.default, "reason": d.reason })).collect::<Vec<_>>(),
    })
}

/// The bench JSON's top-level `photo_finish` object (`--photo-finish`): the level and
/// the clear air's visual range it draws (eo-sky `photo`).
pub fn photo_finish_json(args: &Args) -> serde_json::Value {
    serde_json::json!({
        "on": args.look.photo_finish > 0,
        "level": args.look.photo_finish,
        "clear_visual_range_km": eo_sky::photo::visual_range_km(eo_sky::weather::CLEAR.haze),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &[&str]) -> Args {
        let raw: Vec<String> = line.iter().map(|s| s.to_string()).collect();
        crate::parse_args_from(&raw).unwrap_or_else(|e| panic!("{line:?}: {e:#}"))
    }

    /// `Args` has no PartialEq; its Debug form names every field and value.
    fn same(a: &Args, b: &Args) -> bool {
        format!("{a:?}") == format!("{b:?}")
    }

    #[test]
    fn medium_is_no_flag() {
        let none = parse(&[]);
        assert!(same(&none, &parse(&["--quality", "medium"])), "--quality medium must give exactly the Args of no flag");
        assert_eq!(none.quality, Quality::Medium);
        assert_eq!(none.look.lighting, LightingOptions::OFF);
        // and among other switches, wherever it stands.
        assert!(same(&parse(&["--bench", "--frames", "30"]), &parse(&["--quality", "medium", "--bench", "--frames", "30"])));
        assert!(same(&parse(&["--bench", "--no-ssr"]), &parse(&["--bench", "--no-ssr", "--quality", "medium"])));
        let t = Quality::Medium.tier();
        // medium is the street look (the owner's approval of 2026-11-20) and nothing else.
        assert_eq!(t, Tier { shadow_map: None, ssr: None, contact_shadows: None, sky_quality: None, shadow_slope_bias: None, ao: None, shafts: None, bloom: None, grime: Some(1), street_life: None, stand_ins: Some(true), surveyed_furniture: Some(true), tree_basins: Some(true), photo_finish: None, facade_clutter: None, shop_light: None, car_detail: None, look: None, water_wind: None, water_lean: None, lighting: LightingOptions::OFF });
        assert_eq!((none.look.grime, none.props2.stand_ins, none.props2.surveyed_furniture), (1, true, true), "no flag is the street look");
    }

    #[test]
    fn tiers_expand_as_their_table() {
        let d = parse(&[]);
        let low = parse(&["--quality", "low"]);
        assert_eq!((low.look.shadow_map, low.look.sky.ssr, low.look.sky.contact, low.look.sky.quality), (2048, false, false, 0));
        assert_eq!((low.look.ao, low.look.sky.shafts, low.look.sky.bloom), (false, false, false));
        assert_eq!((d.look.ao, d.look.sky.shafts, d.look.sky.bloom), (true, true, true), "the default draws them");
        assert_eq!(low.look.lighting, LightingOptions::OFF);
        let high = parse(&["--quality", "high"]);
        assert_eq!((high.look.shadow_map, high.look.sky.ssr, high.look.sky.contact, high.look.sky.quality), (d.look.shadow_map, d.look.sky.ssr, d.look.sky.contact, d.look.sky.quality));
        assert_eq!(
            high.look.lighting,
            LightingOptions { cascade_blend: true, far_shadows: 1, ambient_volume: 1, probe_grid: 0, light_shadows: 1, light_shadow_moving: false, headlight_shadows: 2, lit_fog: 1, smooth_time: true, ground_bounce: true, rt_gi: false, rt_reflect: false }
        );
        let hyper = parse(&["--quality", "hyper"]);
        assert_eq!((hyper.look.shadow_map, hyper.look.sky.quality), (4096, 2));
        assert_eq!(
            hyper.look.lighting,
            LightingOptions { cascade_blend: true, far_shadows: 2, ambient_volume: 2, probe_grid: 2, light_shadows: 3, light_shadow_moving: true, headlight_shadows: 8, lit_fog: 2, smooth_time: true, ground_bounce: true, rt_gi: false, rt_reflect: false }
        );
        assert_eq!(hyper.quality, Quality::Hyper);
        // no tier entry equals a default (it would be a no-op that hides a mistake); the street look's
        // entries against the untiered values (grime 0, the grey proxies, the walk as ported), since
        // the default tier is medium and medium sets them.
        for q in Quality::ALL {
            let t = q.tier();
            assert!(t.shadow_map != Some(d.look.shadow_map) && t.ssr != Some(d.look.sky.ssr) && t.contact_shadows != Some(d.look.sky.contact) && t.sky_quality != Some(d.look.sky.quality), "{q:?}");
            assert!(t.shadow_slope_bias != Some(d.look.shadow_slope_bias), "{q:?}");
            assert!(t.ao != Some(d.look.ao) && t.shafts != Some(d.look.sky.shafts) && t.bloom != Some(d.look.sky.bloom), "{q:?}");
            assert!(t.grime != Some(0), "{q:?}");
            assert!(t.street_life != Some(d.look.street_life), "{q:?}");
            assert!(t.stand_ins != Some(false) && t.surveyed_furniture != Some(false) && t.tree_basins != Some(false), "{q:?}");
            assert!(t.photo_finish != Some(d.look.photo_finish), "{q:?}");
            assert!(t.facade_clutter != Some(d.facade_clutter), "{q:?}");
            assert!(t.shop_light != Some(d.look.sky.shop_light), "{q:?}");
            assert!(t.car_detail != Some(d.car_detail), "{q:?}");
            assert!(t.water_wind != Some(d.look.water_features & eo_render::water::FEAT_WIND != 0), "{q:?}");
            assert!(t.water_lean != Some(d.look.water_parts_off & eo_water::switches::LEAN == 0), "{q:?}");
        }
        let budgets: Vec<f64> = Quality::ALL.iter().map(|q| q.budget_ms()).collect();
        assert_eq!(budgets, vec![8.0, 10.0, 16.6, 33.3]);
    }

    #[test]
    fn hyper_contains_high() {
        let (h, y) = (Quality::High.tier(), Quality::Hyper.tier());
        let (h, y) = (h.lighting, y.lighting);
        assert!(!h.cascade_blend || y.cascade_blend);
        assert!(!h.light_shadow_moving || y.light_shadow_moving);
        assert!(!h.smooth_time || y.smooth_time);
        assert!(!h.ground_bounce || y.ground_bounce);
        for (name, a, b) in [
            ("far_shadows", h.far_shadows, y.far_shadows),
            ("ambient_volume", h.ambient_volume, y.ambient_volume),
            ("probe_grid", h.probe_grid, y.probe_grid),
            ("light_shadows", h.light_shadows, y.light_shadows),
            ("headlight_shadows", h.headlight_shadows, y.headlight_shadows),
            ("lit_fog", h.lit_fog, y.lit_fog),
        ] {
            assert!(b >= a, "{name}: hyper {b} below high {a}");
        }
        assert_eq!(h.mask() & y.mask(), h.mask(), "every feature on at high is on at hyper");
        assert!(Quality::Hyper.tier().shadow_map.unwrap_or(0) >= parse(&[]).look.shadow_map);
    }

    #[test]
    fn an_explicit_switch_wins_over_the_tier_wherever_it_stands() {
        for line in [&["--far-shadows", "2", "--quality", "high"][..], &["--quality", "high", "--far-shadows", "2"]] {
            assert_eq!(parse(line).look.lighting.far_shadows, 2, "{line:?}");
        }
        for line in [&["--no-cascade-blend", "--quality", "hyper"][..], &["--quality", "hyper", "--no-cascade-blend"]] {
            let a = parse(line);
            assert!(!a.look.lighting.cascade_blend, "{line:?}");
            assert_eq!(a.look.lighting.lit_fog, 2, "{line:?}: the rest of the tier stays");
        }
        for line in [&["--shadow-map", "3072", "--quality", "low"][..], &["--quality", "low", "--shadow-map", "3072"]] {
            let a = parse(line);
            assert_eq!(a.look.shadow_map, 3072, "{line:?}");
            assert!(!a.look.sky.ssr, "{line:?}: the rest of the tier stays");
        }
        assert_eq!(parse(&["--sky-quality", "medium", "--quality", "hyper"]).look.sky.quality, 1);
        assert_eq!(parse(&["--quality", "high", "--no-light-shadows", "--headlight-shadows", "4"]).look.lighting.light_shadows, 0);
        assert_eq!(parse(&["--quality", "high", "--headlight-shadows", "4"]).look.lighting.headlight_shadows, 4);
        // the last of two `--quality` wins; a feature alone needs no tier.
        assert_eq!(parse(&["--quality", "hyper", "--quality", "low"]).quality, Quality::Low);
        let alone = parse(&["--lit-fog", "2"]);
        assert_eq!((alone.quality, alone.look.lighting), (Quality::Medium, LightingOptions { lit_fog: 2, ..LightingOptions::OFF }));
    }

    #[test]
    fn optional_levels_take_a_number_only() {
        let high = Quality::High.tier().lighting;
        // no value: the high level, and the next switch is parsed as a switch.
        let a = parse(&["--far-shadows", "--bench", "--probe-grid"]);
        assert!(a.bench);
        assert_eq!((a.look.lighting.far_shadows, a.look.lighting.probe_grid), (high.far_shadows, PROBE_GRID_BARE));
        assert_eq!(parse(&["--probe-grid"]).look.lighting.probe_grid, 1, "bare --probe-grid draws the grid though high leaves it off");
        assert_eq!(parse(&["--light-shadows"]).look.lighting.light_shadows, high.light_shadows);
        assert_eq!(parse(&["--headlight-shadows"]).look.lighting.headlight_shadows, high.headlight_shadows);
        assert_eq!(parse(&["--ambient-volume", "2"]).look.lighting.ambient_volume, 2);
        assert_eq!(parse(&["--lit-fog", "0"]).look.lighting.lit_fog, 0);
        // lamps round up to sixteens, at most 48.
        for (lamps, steps) in [("1", 1), ("16", 1), ("17", 2), ("32", 2), ("48", 3), ("100", 3), ("0", 0)] {
            let a = parse(&["--light-shadows", lamps]);
            assert_eq!(a.look.lighting.light_shadows, steps, "--light-shadows {lamps}");
            assert_eq!(a.look.lighting.shadowed_lamps(), steps as u32 * 16);
        }
        let raw = |line: &[&str]| crate::parse_args_from(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(raw(&["--far-shadows", "3"]).is_err(), "a level past 2 is refused");
        assert!(raw(&["--quality", "ultra"]).is_err());
        assert!(raw(&["--quality"]).is_err());
        assert!(raw(&["--headlight-shadows", "300"]).is_err());
    }

    #[test]
    fn the_ground_bounce_comes_with_the_ambient_volume() {
        let lit = |line: &[&str]| parse(line).look.lighting;
        // on with the volume, wherever it comes from; off with `--no-ground-bounce` wherever it stands.
        for line in [&["--ambient-volume"][..], &["--ambient-volume", "2"], &["--quality", "high"], &["--quality", "hyper"], &["--ground-bounce", "--ambient-volume", "1"]] {
            let l = lit(line);
            assert!(l.ground_bounce && l.ground_bounce_on(), "{line:?}");
        }
        for (line, without) in [
            (&["--ambient-volume", "--no-ground-bounce"][..], &["--ambient-volume"][..]),
            (&["--no-ground-bounce", "--quality", "high"], &["--quality", "high"]),
            (&["--quality", "hyper", "--no-ground-bounce"], &["--quality", "hyper"]),
        ] {
            let l = lit(line);
            assert!(!l.ground_bounce && l.ambient_volume > 0, "{line:?}");
            assert_eq!(l, LightingOptions { ground_bounce: false, ..lit(without) }, "{line:?}: the rest stays");
        }
        // never without the volume: alone it is nothing, so the default frame is untouched.
        for line in [&["--ground-bounce"][..], &["--no-ground-bounce"], &["--quality", "high", "--no-ambient-volume"], &["--ground-bounce", "--ambient-volume", "0"]] {
            assert!(!lit(line).ground_bounce, "{line:?}");
        }
        assert_eq!(lit(&["--ground-bounce"]), LightingOptions::OFF);
        assert!(same(&parse(&["--no-ground-bounce", "--bench"]), &parse(&["--bench"])));
        // it has no bit of its own (the shader reads its row): the volume's bit is the mask.
        assert_eq!(lit(&["--ambient-volume"]).mask(), lit(&["--ambient-volume", "--no-ground-bounce"]).mask());
    }

    #[test]
    fn the_shadow_slope_bias_comes_with_high_and_hyper_and_its_switch_wins() {
        assert!(!parse(&[]).look.shadow_slope_bias, "off by default");
        for (line, want) in [
            (&["--quality", "low"][..], false),
            (&["--quality", "medium"], false),
            (&["--quality", "high"], true),
            (&["--quality", "hyper"], true),
            (&["--shadow-slope-bias"], true),
            (&["--quality", "high", "--no-shadow-slope-bias"], false),
            (&["--no-shadow-slope-bias", "--quality", "hyper"], false), // wherever it stands
            (&["--shadow-slope-bias", "--quality", "low"], true),
        ] {
            assert_eq!(parse(line).look.shadow_slope_bias, want, "{line:?}");
        }
        // it is a shadow setting, not a lighting addition: no lighting bit, and nothing else moves.
        let mut a = parse(&["--bench", "--shadow-slope-bias"]);
        assert_eq!(a.look.lighting, LightingOptions::OFF);
        a.look.shadow_slope_bias = false;
        assert!(same(&a, &parse(&["--bench"])));
        assert!(same(&parse(&["--no-shadow-slope-bias", "--bench"]), &parse(&["--bench"])));
    }

    #[test]
    fn grime_comes_with_medium_high_and_hyper_and_its_switch_wins() {
        assert_eq!(parse(&[]).look.grime, 1, "on by default (medium, the street look)");
        for (line, want) in [
            (&["--quality", "low"][..], 0),
            (&["--quality", "medium"], 1),
            (&["--no-grime"], 0),
            (&["--quality", "high"], 1),
            (&["--quality", "hyper"], 2),
            (&["--grime"], 1),
            (&["--grime", "2"], 2),
            (&["--grime", "0"], 0),
            (&["--quality", "hyper", "--grime", "1"], 1),
            (&["--grime", "1", "--quality", "hyper"], 1), // wherever it stands
            (&["--quality", "high", "--no-grime"], 0),
            (&["--no-grime", "--quality", "hyper"], 0),
            (&["--grime", "--bench"], 1), // no value: the high level, the next switch parsed as a switch
        ] {
            assert_eq!(parse(line).look.grime, want, "{line:?}");
        }
        assert!(parse(&["--grime", "--bench"]).bench);
        let raw = |line: &[&str]| crate::parse_args_from(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(raw(&["--grime", "3"]).is_err(), "a level past 2 is refused");
        // a surface setting, not a lighting addition: nothing else moves, and `--no-grime` is the default less the grime.
        let mut a = parse(&["--bench", "--grime", "2"]);
        assert_eq!(a.look.lighting, LightingOptions::OFF);
        a.look.grime = 1;
        assert!(same(&a, &parse(&["--bench"])));
        let mut n = parse(&["--no-grime", "--bench"]);
        n.look.grime = 1;
        assert!(same(&n, &parse(&["--bench"])));
        // the frame switches carry it in their own bits: none at 0.
        assert_eq!(eo_render::facade::grime_bits(0), 0);
        assert_eq!(eo_render::facade::grime_bits(1) & !eo_render::facade::RUN_GRIME_MASK, 0);
        assert_eq!(eo_render::facade::grime_bits(2) >> eo_render::facade::RUN_GRIME_SHIFT, 2);
        assert_eq!(eo_render::facade::grime_bits(1) & eo_render::look::LookOptions::default().facade, 0, "no default switch shares its bits");
    }

    #[test]
    fn street_life_comes_with_high_and_hyper_and_its_switch_wins() {
        assert_eq!(parse(&[]).look.street_life, 0, "off by default");
        for (line, want) in [
            (&["--quality", "low"][..], 0),
            (&["--quality", "medium"], 0),
            (&["--quality", "high"], 1),
            (&["--quality", "hyper"], 2),
            (&["--street-life"], 1),
            (&["--street-life", "2"], 2),
            (&["--street-life", "0"], 0),
            (&["--quality", "hyper", "--street-life", "1"], 1),
            (&["--street-life", "1", "--quality", "hyper"], 1), // wherever it stands
            (&["--quality", "high", "--no-street-life"], 0),
            (&["--no-street-life", "--quality", "hyper"], 0),
            (&["--street-life", "--bench"], 1), // no value: the high level, the next switch parsed as a switch
        ] {
            assert_eq!(parse(line).look.street_life, want, "{line:?}");
        }
        assert!(parse(&["--street-life", "--bench"]).bench);
        let raw = |line: &[&str]| crate::parse_args_from(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(raw(&["--street-life", "3"]).is_err(), "a level past 2 is refused");
        // a surface setting of its own: nothing else moves (not the grime), and `--no-street-life` alone is no flag.
        let mut a = parse(&["--bench", "--street-life", "2"]);
        assert_eq!((a.look.lighting, a.look.grime), (LightingOptions::OFF, 1));
        a.look.street_life = 0;
        assert!(same(&a, &parse(&["--bench"])));
        assert!(same(&parse(&["--no-street-life", "--bench"]), &parse(&["--bench"])));
        // the frame switches carry it in their own bits: none at 0, none shared with the grime or a default switch.
        use eo_render::facade::{RUN_GRIME_MASK, RUN_STREET_LIFE_MASK, RUN_STREET_LIFE_SHIFT, street_life_bits};
        assert_eq!(street_life_bits(0), 0);
        assert_eq!(street_life_bits(2) >> RUN_STREET_LIFE_SHIFT, 2);
        assert_eq!(street_life_bits(1) & !RUN_STREET_LIFE_MASK, 0);
        assert_eq!(RUN_STREET_LIFE_MASK & RUN_GRIME_MASK, 0);
        assert_eq!(RUN_STREET_LIFE_MASK & eo_render::look::LookOptions::default().facade, 0);
    }

    #[test]
    fn stand_ins_come_with_medium_high_and_hyper_and_their_switch_wins() {
        assert!(parse(&[]).props2.stand_ins, "on by default (medium, the street look)");
        for (line, want) in [
            (&["--quality", "low"][..], false),
            (&["--quality", "medium"], true),
            (&["--no-stand-ins"], false),
            (&["--quality", "high"], true),
            (&["--quality", "hyper"], true),
            (&["--stand-ins"], true),
            (&["--quality", "high", "--no-stand-ins"], false),
            (&["--no-stand-ins", "--quality", "hyper"], false), // wherever it stands
            (&["--stand-ins", "--quality", "low"], true),
            (&["--stand-ins", "--no-props2"], true), // the props2 switches leave it
        ] {
            assert_eq!(parse(line).props2.stand_ins, want, "{line:?}");
        }
        let mut n = parse(&["--no-stand-ins", "--bench"]);
        n.props2.stand_ins = true;
        assert!(same(&n, &parse(&["--bench"])), "--no-stand-ins moves nothing else");
    }

    #[test]
    fn tree_basins_come_with_every_tier_and_their_switch_wins() {
        assert!(parse(&[]).props2.tree_basins, "on by default");
        for (line, want) in [
            (&["--quality", "low"][..], true),
            (&["--quality", "hyper"], true),
            (&["--no-tree-basins"], false),
            (&["--no-tree-basins", "--quality", "high"], false), // wherever it stands
            (&["--no-tree-basins", "--tree-basins"], true),
            (&["--no-tree-basins", "--no-props2"], false),
        ] {
            assert_eq!(parse(line).props2.tree_basins, want, "{line:?}");
        }
        let mut n = parse(&["--no-tree-basins", "--bench"]);
        n.props2.tree_basins = true;
        assert!(same(&n, &parse(&["--bench"])), "--no-tree-basins moves nothing else");
        assert_eq!(tree_basins_json(&parse(&[]))["on"].as_bool(), Some(true));
    }

    #[test]
    fn surveyed_furniture_comes_with_every_tier_and_its_switch_wins() {
        assert!(parse(&[]).props2.surveyed_furniture, "on by default");
        for (line, want) in [
            (&["--quality", "low"][..], true),
            (&["--quality", "medium"], true),
            (&["--quality", "high"], true),
            (&["--quality", "hyper"], true),
            (&["--no-surveyed-furniture"], false),
            (&["--quality", "hyper", "--no-surveyed-furniture"], false),
            (&["--no-surveyed-furniture", "--quality", "high"], false), // wherever it stands
            (&["--no-surveyed-furniture", "--surveyed-furniture"], true), // the last wins
            (&["--no-surveyed-furniture", "--no-props2"], false), // the props2 switches leave it
        ] {
            assert_eq!(parse(line).props2.surveyed_furniture, want, "{line:?}");
        }
        let mut n = parse(&["--no-surveyed-furniture", "--bench"]);
        n.props2.surveyed_furniture = true;
        assert!(same(&n, &parse(&["--bench"])), "--no-surveyed-furniture moves nothing else");
        let j = surveyed_furniture_json(&parse(&[]));
        assert_eq!(j["on"].as_bool(), Some(true));
        assert!(j["rare_kinds"].as_array().unwrap().iter().any(|k| k == "bollard"));
    }

    #[test]
    fn the_cinematic_look_comes_with_high_and_hyper_and_its_switch_wins() {
        let d = parse(&[]);
        assert_eq!((d.look.look, d.look.sky.cinematic), (LookProfile::Default, false), "off by default");
        for (line, want) in [
            (&["--quality", "low"][..], LookProfile::Default),
            (&["--quality", "medium"], LookProfile::Default),
            (&["--quality", "high"], LookProfile::Cinematic),
            (&["--quality", "hyper"], LookProfile::Cinematic),
            (&["--look", "cinematic"], LookProfile::Cinematic),
            (&["--look", "default"], LookProfile::Default),
            (&["--quality", "high", "--no-look"], LookProfile::Default),
            (&["--no-look", "--quality", "hyper"], LookProfile::Default), // wherever it stands
            (&["--quality", "hyper", "--look", "default"], LookProfile::Default),
            (&["--quality", "low", "--look", "cinematic"], LookProfile::Cinematic),
            (&["--look", "cinematic", "--no-sky"], LookProfile::Cinematic),
        ] {
            let a = parse(line);
            assert_eq!(a.look.look, want, "{line:?}");
            assert_eq!(a.look.sky.cinematic, want.is_cinematic(), "{line:?}: the sky's copy follows");
        }
        let raw = |line: &[&str]| crate::parse_args_from(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(raw(&["--look", "filmic"]).is_err(), "an unknown profile is refused");
        assert!(raw(&["--look"]).is_err(), "--look needs a value");
        // no tier entry equals the default, and the profile moves nothing else.
        for q in Quality::ALL {
            assert!(q.tier().look != Some(d.look.look), "{q:?}");
        }
        let mut a = parse(&["--bench", "--look", "cinematic"]);
        (a.look.look, a.look.sky.cinematic) = (LookProfile::Default, false);
        assert!(same(&a, &parse(&["--bench"])));
        assert!(same(&parse(&["--no-look", "--bench"]), &parse(&["--bench"])), "--no-look alone is no flag");
        assert!(same(&parse(&["--look", "default", "--bench"]), &parse(&["--bench"])));
        let j = look_profile_json(&parse(&["--quality", "hyper"]));
        assert_eq!((j["on"].as_bool(), j["profile"].as_str()), (Some(true), Some("cinematic")));
        assert_eq!(look_profile_json(&d)["on"].as_bool(), Some(false));
    }

    #[test]
    fn the_photo_finish_comes_with_high_and_hyper_and_its_switch_wins() {
        assert_eq!(parse(&[]).look.photo_finish, 0, "off by default");
        for (line, want) in [
            (&["--quality", "low"][..], 0),
            (&["--quality", "medium"], 0),
            (&["--quality", "high"], 1),
            (&["--quality", "hyper"], 2),
            (&["--photo-finish"], 1),
            (&["--photo-finish", "2"], 2),
            (&["--photo-finish", "0"], 0),
            (&["--quality", "hyper", "--photo-finish", "1"], 1),
            (&["--photo-finish", "1", "--quality", "hyper"], 1), // wherever it stands
            (&["--quality", "high", "--no-photo-finish"], 0),
            (&["--no-photo-finish", "--quality", "hyper"], 0),
            (&["--photo-finish", "--bench"], 1), // no value: the high level, the next switch parsed as a switch
        ] {
            assert_eq!(parse(line).look.photo_finish, want, "{line:?}");
        }
        assert!(parse(&["--photo-finish", "--bench"]).bench);
        let raw = |line: &[&str]| crate::parse_args_from(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(raw(&["--photo-finish", "3"]).is_err(), "a level past 2 is refused");
        // a finish of its own: nothing else moves, and `--no-photo-finish` alone is no flag.
        let mut a = parse(&["--bench", "--photo-finish", "2"]);
        assert_eq!((a.look.lighting, a.look.grime, a.look.street_life), (LightingOptions::OFF, 1, 0));
        a.look.photo_finish = 0;
        assert!(same(&a, &parse(&["--bench"])));
        assert!(same(&parse(&["--no-photo-finish", "--bench"]), &parse(&["--bench"])));
        // the bench JSON's record.
        let j = photo_finish_json(&parse(&["--photo-finish", "2"]));
        assert_eq!((j["on"].as_bool(), j["level"].as_u64()), (Some(true), Some(2)));
        assert_eq!(photo_finish_json(&parse(&[]))["on"].as_bool(), Some(false));
    }

    #[test]
    fn facade_clutter_comes_with_high_and_hyper_and_its_switch_wins() {
        assert_eq!(parse(&[]).facade_clutter, 0, "off by default");
        for (line, want) in [
            (&["--quality", "low"][..], 0),
            (&["--quality", "medium"], 0),
            (&["--quality", "high"], 1),
            (&["--quality", "hyper"], 2),
            (&["--facade-clutter"], 1),
            (&["--facade-clutter", "2"], 2),
            (&["--facade-clutter", "0"], 0),
            (&["--quality", "hyper", "--facade-clutter", "1"], 1),
            (&["--facade-clutter", "1", "--quality", "hyper"], 1), // wherever it stands
            (&["--quality", "high", "--no-facade-clutter"], 0),
            (&["--no-facade-clutter", "--quality", "hyper"], 0),
            (&["--facade-clutter", "--bench"], 1), // no value: the high level, the next switch parsed as a switch
        ] {
            assert_eq!(parse(line).facade_clutter, want, "{line:?}");
        }
        assert!(parse(&["--facade-clutter", "--bench"]).bench);
        let raw = |line: &[&str]| crate::parse_args_from(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(raw(&["--facade-clutter", "3"]).is_err(), "a level past 2 is refused");
        // a world detail of its own: nothing else moves, and `--no-facade-clutter` alone is no flag.
        let mut a = parse(&["--bench", "--facade-clutter", "2"]);
        assert_eq!((a.look.lighting, a.look.grime, a.look.street_life), (LightingOptions::OFF, 1, 0));
        a.facade_clutter = 0;
        assert!(same(&a, &parse(&["--bench"])));
        assert!(same(&parse(&["--no-facade-clutter", "--bench"]), &parse(&["--bench"])));
        let j = facade_clutter_json(&parse(&["--facade-clutter", "2"]));
        assert_eq!((j["on"].as_bool(), j["level"].as_u64()), (Some(true), Some(2)));
        assert_eq!(facade_clutter_json(&parse(&[]))["on"].as_bool(), Some(false));
    }

    #[test]
    fn car_detail_comes_with_high_and_hyper_and_its_switch_wins() {
        assert_eq!(parse(&[]).car_detail, 0, "off by default");
        for (line, want) in [
            (&["--quality", "low"][..], 0),
            (&["--quality", "medium"], 0),
            (&["--quality", "high"], 1),
            (&["--quality", "hyper"], 2),
            (&["--car-detail"], 1),
            (&["--car-detail", "2"], 2),
            (&["--car-detail", "0"], 0),
            (&["--quality", "hyper", "--car-detail", "1"], 1),
            (&["--car-detail", "1", "--quality", "hyper"], 1), // wherever it stands
            (&["--quality", "high", "--no-car-detail"], 0),
            (&["--no-car-detail", "--quality", "hyper"], 0),
            (&["--car-detail", "--bench"], 1), // no value: the high level, the next switch parsed as a switch
        ] {
            assert_eq!(parse(line).car_detail, want, "{line:?}");
        }
        assert!(parse(&["--car-detail", "--bench"]).bench);
        let raw = |line: &[&str]| crate::parse_args_from(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(raw(&["--car-detail", "3"]).is_err(), "a level past 2 is refused");
        // a detail of its own: nothing else moves, and `--no-car-detail` alone is no flag.
        let mut a = parse(&["--bench", "--car-detail", "2"]);
        assert_eq!((a.look.lighting, a.look.grime, a.facade_clutter), (LightingOptions::OFF, 1, 0));
        a.car_detail = 0;
        assert!(same(&a, &parse(&["--bench"])));
        assert!(same(&parse(&["--no-car-detail", "--bench"]), &parse(&["--bench"])));
        // the bench JSON's record: level 0 keeps the browser build's tiers, each level widens them.
        let (off, one, two) = (car_detail_json(&parse(&[])), car_detail_json(&parse(&["--car-detail"])), car_detail_json(&parse(&["--car-detail", "2"])));
        assert_eq!((off["on"].as_bool(), off["far_body"].as_bool()), (Some(false), Some(false)));
        assert_eq!((two["on"].as_bool(), two["level"].as_u64()), (Some(true), Some(2)));
        let caps = |j: &serde_json::Value| j["vehicle_cap_lod0_lod1"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect::<Vec<_>>();
        let radii = |j: &serde_json::Value| j["vehicle_radius_m"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
        assert_eq!(caps(&off), vec![2, 10], "level 0: the JS overlay's caps");
        assert!(caps(&off).iter().zip(caps(&one)).all(|(a, b)| a < &b) && caps(&one).iter().zip(caps(&two)).all(|(a, b)| a < &b));
        assert!(radii(&off)[..3].iter().zip(&radii(&one)[..3]).all(|(a, b)| a < b) && radii(&one)[..3].iter().zip(&radii(&two)[..3]).all(|(a, b)| a < b));
        assert_eq!(radii(&off)[3], radii(&two)[3], "the far slot ends where it did");
    }

    #[test]
    fn structures_and_tunnels_are_world_switches() {
        use eo_structures::StructureOptions;
        // off by default and under every tier.
        assert_eq!(parse(&[]).structures, None);
        for q in ["low", "medium", "high", "hyper"] {
            assert_eq!(parse(&["--quality", q]).structures, None, "--quality {q}");
        }
        let on = |tunnels| Some(StructureOptions { tunnels });
        for (line, want) in [
            (&["--structures"][..], on(false)),
            (&["--structures", "--tunnels"], on(true)),
            (&["--tunnels", "--structures"], on(true)), // wherever it stands
            (&["--tunnels"], None),                     // tunnels only with structures
            (&["--structures", "--no-structures"], None),
            (&["--no-structures", "--structures"], on(false)), // the last one wins
            (&["--structures", "--tunnels", "--no-tunnels"], on(false)),
            (&["--quality", "hyper", "--structures", "--bench"], on(false)),
        ] {
            let a = parse(line);
            assert_eq!(a.structures, want, "{line:?}");
        }
        // nothing else moves: the same Args as without them, but for the field.
        let mut a = parse(&["--bench", "--structures", "--tunnels"]);
        a.structures = None;
        assert!(same(&a, &parse(&["--bench"])));
    }

    #[test]
    fn the_shop_light_comes_with_high_and_hyper_and_its_switch_wins() {
        assert_eq!(parse(&[]).look.sky.shop_light, 0, "off by default");
        for (line, want) in [
            (&["--quality", "low"][..], 0),
            (&["--quality", "medium"], 0),
            (&["--quality", "high"], 1),
            (&["--quality", "hyper"], 2),
            (&["--shop-light"], 1),
            (&["--shop-light", "2"], 2),
            (&["--shop-light", "0"], 0),
            (&["--quality", "hyper", "--shop-light", "1"], 1),
            (&["--shop-light", "1", "--quality", "hyper"], 1), // wherever it stands
            (&["--quality", "high", "--no-shop-light"], 0),
            (&["--no-shop-light", "--quality", "hyper"], 0),
            (&["--shop-light", "--bench"], 1), // no value: the high level, the next switch parsed as a switch
        ] {
            assert_eq!(parse(line).look.sky.shop_light, want, "{line:?}");
        }
        assert!(parse(&["--shop-light", "--bench"]).bench);
        let raw = |line: &[&str]| crate::parse_args_from(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(raw(&["--shop-light", "3"]).is_err(), "a level past 2 is refused");
        // a night setting of its own: nothing else moves, and `--no-shop-light` alone is no flag.
        let mut a = parse(&["--bench", "--shop-light", "2"]);
        assert_eq!((a.look.lighting, a.look.grime, a.look.street_life, a.look.photo_finish), (LightingOptions::OFF, 1, 0, 0));
        a.look.sky.shop_light = 0;
        assert!(same(&a, &parse(&["--bench"])));
        assert!(same(&parse(&["--no-shop-light", "--bench"]), &parse(&["--bench"])));
        // the bench JSON's record.
        let j = shop_light_json(&parse(&["--shop-light", "2"]), None);
        assert_eq!((j["on"].as_bool(), j["level"].as_u64(), j["reach_m"].as_f64()), (Some(true), Some(2), Some(300.0)));
        assert_eq!(shop_light_json(&parse(&[]), None)["on"].as_bool(), Some(false));
    }

    #[test]
    fn the_sea_wind_texture_and_lean_come_with_high_and_hyper_and_their_switches_win() {
        use eo_render::water::FEAT_WIND;
        use eo_water::switches::LEAN;
        let parts = |a: &Args| (a.look.water_features & FEAT_WIND != 0, a.look.water_parts_off & LEAN == 0);
        let d = parse(&[]);
        assert_eq!(parts(&d), (false, false), "medium leaves both out");
        assert_eq!(d.look.water_features | FEAT_WIND, eo_render::water::FEAT_ALL, "and only the wind texture of the features");
        assert_eq!(d.look.water_parts_off, LEAN, "and only the lean of the parts");
        for (line, want) in [
            (&["--quality", "low"][..], (false, false)),
            (&["--quality", "medium"], (false, false)),
            (&["--quality", "high"], (true, true)),
            (&["--quality", "hyper"], (true, true)),
            (&["--water-wind-texture"], (true, false)),
            (&["--water-lean"], (false, true)),
            (&["--quality", "high", "--no-water-wind-texture"], (false, true)),
            (&["--no-water-lean", "--quality", "hyper"], (true, false)), // wherever it stands
            (&["--quality", "low", "--water-wind-texture", "--water-lean"], (true, true)),
            (&["--water-lean", "--no-water-lean"], (false, false)), // a `--no-` form wins wherever it stands
            (&["--no-water-wind-texture", "--water-wind-texture", "--quality", "hyper"], (false, true)),
        ] {
            assert_eq!(parts(&parse(line)), want, "{line:?}");
        }
        // the other features and switches are untouched by the tier.
        let a = parse(&["--quality", "high", "--no-water-glint"]);
        assert_eq!(a.look.water_features, eo_render::water::FEAT_ALL & !eo_render::water::FEAT_GLINT);
    }
}
