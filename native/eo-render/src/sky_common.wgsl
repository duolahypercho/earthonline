// the sky/weather/night/finish uniform (eo-render/src/sky.rs SkyUniform)
// and helpers shared by the city shader, the sky pass, the volume pass, the
// resolve, the bloom and post. Appended to look_common.wgsl (look.rs
// look_common_source); bindings are declared by each consumer.

struct Sky {
    // unjittered view-projection (absolute world) of this frame and the last
    vp: mat4x4<f32>,
    prev_vp: mat4x4<f32>,
    // xyz eye, w world seconds (wrapped to a day)
    eye: vec4<f32>,
    prev_eye: vec4<f32>,
    // xy this frame's jitter (NDC), zw the last frame's
    jitter: vec4<f32>,
    // x TAA on, y history valid (the frames of history since the last cut, 0 = none), z blend (new frame weight), w frame dt (s)
    taa: vec4<f32>,
    // full-resolution width, height, 1/width, 1/height
    res: vec4<f32>,
    // low deck: base m, top m, coverage, opacity
    cloud0: vec4<f32>,
    // extinction 1/m, high deck altitude m, high opacity, beam floor (covered sun)
    cloud1: vec4<f32>,
    // drift offsets (m): low u, low v, high u, high v
    cloud2: vec4<f32>,
    // marine layer: sigma 1/m (inland), top m, swing m, offshore sigma 1/m
    cloud3: vec4<f32>,
    // marine drift offset u, v (m); haze sigma at the ground 1/m; haze scale height m
    cloud4: vec4<f32>,
    // key (sun or moon) radiance for the volume, w 1 when the moon keys
    sun_rad: vec4<f32>,
    // sky ambient at the cloud tops (irradiance / pi), w city glow on the bases
    amb_top: vec4<f32>,
    // ambient under the decks (ground bounce, city glow colour), w cloud steps
    amb_base: vec4<f32>,
    // wetness, puddle fill, rain mm/h, seconds (animation clock, wrapped)
    wet: vec4<f32>,
    // rain: near live fraction, far live fraction, fall speed m/s, streak strength
    rain: vec4<f32>,
    // wind x, z m/s, gust, lightning flash 0..1
    wind: vec4<f32>,
    // x frame lights cap, y/z/w cluster grid
    lights: vec4<u32>,
    // cluster near m, far m, tan(fov_y / 2), aspect
    cluster: vec4<f32>,
    // camera basis for the clusters (forward, right, up)
    cam_fwd: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    // lamps on, dusk, nightness, vehicle lamps
    night: vec4<f32>,
    // toward the moon xyz, w disc radiance
    moon: vec4<f32>,
    // star level, sky rotation (rad), moon illuminated fraction, moon waxing (1/0)
    stars: vec4<f32>,
    // bloom: threshold, knee width, strength gain, on
    bloom: vec4<f32>,
    // mip weights 0..3
    bloom_w: vec4<f32>,
    // mip 4 weight, source ceiling (x threshold), units per lux (lamps), lamp haze gain
    bloom_w2: vec4<f32>,
    // x stage bits (SKY_*), y frame index, z ssr usable (1/0), w volume usable (1/0)
    flags: vec4<u32>,
    // x the weather's mean direct-beam transmission (the sun every pass
    // takes under a continuous deck: `sky_cloud_transmittance`), y, z, w unused
    beam: vec4<f32>,
    // the twilight light on the clouds from the sky toward the set sun (rgb)
    twi: vec4<f32>,
    // toward the sun xyz (even under the horizon), w the sky's scale (look units per unit solar illuminance)
    atmo: vec4<f32>,
    // x the atmosphere tables on (1/0), y their share of the sky (day and twilight 1, deep night 0), z the aerosol (haze) density x, w the clouds' sun scale
    atmo2: vec4<f32>,
    // aerial perspective: x the weather veils (1/0), y clear air's aerosol extinction (1/m), z, w the weather's range-fog near / far (m)
    atmo3: vec4<f32>,
    // the sun's key light as it reaches the ground through the atmosphere,
    // past the rig's horizon envelope (sea glint): rgb in key units (as `look.key`), w the
    // refracted elevation's sine. Zero once the disc's upper limb has set.
    sun_glint: vec4<f32>,
};

const SKY_CLOUDS: u32 = 1u;
const SKY_CLOUD_SHADOWS: u32 = 2u;
const SKY_FOG: u32 = 4u;
const SKY_SHAFTS: u32 = 8u;
const SKY_MOON_STARS: u32 = 16u;
const SKY_LIGHTS: u32 = 32u;
const SKY_WET: u32 = 64u;
const SKY_RAIN: u32 = 128u;
const SKY_SSR: u32 = 256u;
const SKY_TAA: u32 = 512u;
const SKY_BLOOM: u32 = 1024u;
const SKY_SPRITES: u32 = 4096u;
const SKY_CONTACT: u32 = 8192u;
// the fair-weather cumulus field (off by default: `EO_CUMULUS=1`, sky.rs `SkyOptions::cumulus`)
const SKY_CUMULUS: u32 = 16384u;
// a low-deck cloud over the ground hazed by its own depth (`--cloud-depth-haze`, off by
// default; off, the ground's whole aerial perspective is in front of it, as before; sky.rs
// `SkyOptions::cloud_depth_haze`)
const SKY_CLOUD_DEPTH_HAZE: u32 = 2048u;
// the cinematic look (`--look cinematic`, sky.rs `SkyOptions::cinematic`; the cinematic look
// plan's stage L4): the golden haze and the shade in it (sky_volume.wgsl `vol_aerial`, the
// shafts); its code is compiled out with SF_CINEMATIC.
const SKY_CINEMATIC: u32 = 131072u;
// the damp street (`--weather damp`, stage L3; sky.rs `SKY_DAMP_BIT`): the film's drying
// patches (sky_light.wgsl `sky_damp_left`); never set by another weather.
const SKY_DAMP: u32 = 262144u;

/// World metres one cloud-sheet tile spans (eo-sky clouds.rs).
const SKY_LOW_TILE: f32 = 3200.0;
const SKY_HIGH_TILE: f32 = 10000.0;
const SKY_MARINE_TILE: f32 = 9600.0;
/// the cloud-shadow map (clouds.rs): texels a side, metres a texel (a 32 km
/// window about the eye, wrapped: a texel is anchored to the world point it stands for).
const SKY_CSHADOW_RES: f32 = 384.0;
const SKY_CSHADOW_TEXEL: f32 = 83.333336;

/// the cloud-shadow map's transmittance toward the key `l` at a world point
/// (read with a repeating sampler): the map is made for the sea-level point on the
/// point's own key ray; faded out toward the window's edge and under a sun within a
/// few degrees of the horizon (the shadows then land tens of km away).
fn sky_cshadow_at(m: texture_2d<f32>, smp: sampler, world: vec3<f32>, l: vec3<f32>, eye: vec3<f32>, sea_y: f32) -> f32 {
    let g = world.xz - l.xz * (world.y - sea_y) / max(l.y, 0.02);
    let s = textureSampleLevel(m, smp, g / (SKY_CSHADOW_RES * SKY_CSHADOW_TEXEL), 0.0).r;
    let fade = (1.0 - smoothstep(0.42, 0.48, length(g - eye.xz) / (SKY_CSHADOW_RES * SKY_CSHADOW_TEXEL))) * smoothstep(0.03, 0.09, l.y);
    // the forward-scattered light under a cumulus core keeps it from going to nothing
    return mix(1.0, max(s, 0.1), fade);
}

/// Henyey-Greenstein phase.
fn sky_hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (12.566371 * pow(max(1.0 + g2 - 2.0 * g * c, 1e-4), 1.5));
}

/// The cloud phase (sky-atmosphere.js CLOUD_MARCH): 0.85 HG(0.76) + 0.15 HG(-0.25).
fn sky_cloud_phase(c: f32, k: f32) -> f32 {
    return 0.85 * sky_hg(c, 0.76 * k) + 0.15 * sky_hg(c, -0.25 * k);
}

/// Interleaved gradient noise (Jimenez 2014), 0..1.
fn sky_ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

fn sky_hash1(n: u32) -> f32 {
    var h = n;
    h = h ^ (h >> 16u);
    h = h * 0x7feb352du;
    h = h ^ (h >> 15u);
    h = h * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) / 4294967296.0;
}

fn sky_on(bits: u32, flag: u32) -> bool {
    return (bits & flag) != 0u;
}

/// The JS post chain's aerial path density (post-chain.js aerial stage): the
/// haze thins with height (scale height 400 m, a 20 m dead band), so a ray
/// climbing or falling |dh| metres crosses (1 - e^-u) / u of the range fog's
/// view depth, u = (|dh| - 20) / 400.
fn sky_path_density(dh: f32) -> f32 {
    let u = max(abs(dh) - 20.0, 0.0) / 400.0;
    return select(1.0, (1.0 - exp(-u)) / u, u > 1e-2);
}

// the city's skyglow: the street and window light of a lit city
// scattered back by the air over it (light pollution). Over San Francisco the
// night sky is never black: a dull orange-grey dome brightest at the horizon
// over the city and fading upward (Falchi et al. 2016, the World Atlas of
// artificial sky brightness: the Bay Area at 10-40 x the natural zenith),
// darker over the open Pacific. Radiance in the look's units, x the weather's
// urban glow over clear's (`g`: the uniform's `amb_top.w`, 0 by day, on through nautical
// twilight); the dome's own JS glow term stays (a narrow band at the horizon).
// Drawn by the volume pass (sky pixels, and as a veil over distant ground).
// the centre of the lit city moved toward downtown and SoMa (the
// densest light), from (-2,000, -300).
const SKY_CITY_CENTRE: vec2<f32> = vec2<f32>(0.0, -1200.0);
// the city's light on a cloud base or the fog over the city, at a
// glow of 1 (clear): ~2 x the skyglow at the horizon, lamp colour.
const SKY_CITY_BASE_RGB: vec3<f32> = vec3<f32>(0.105, 0.080, 0.054);
const SKY_GLOW_RGB: vec3<f32> = vec3<f32>(0.0310, 0.0242, 0.0180);

fn sky_city_glow(d: vec3<f32>, eye: vec3<f32>, g: f32) -> vec3<f32> {
    if (g <= 0.0) {
        return vec3<f32>(0.0);
    }
    let el = clamp(d.y, 0.0, 1.0);
    // Brightest at the horizon (the long path through the lit low air), ~1/5 of it overhead.
    let shape = 0.28 + 0.72 * exp(-el / 0.14); // zenith floor 0.2 -> 0.28
    let to = SKY_CITY_CENTRE - eye.xz;
    let r = length(to);
    let h = length(d.xz);
    var along = 0.0;
    if (h > 1e-4 && r > 1.0) {
        along = dot(d.xz / h, to / r);
    }
    // Inside the city the glow stands all round; from its edge (Ocean Beach)
    // it is a dome over the city and the sky over the sea is darker.
    let inside = 1.0 - smoothstep(3000.0, 9000.0, r);
    // a dome over the city (brightest low toward it, x1.6 there),
    // dark low over the open Pacific (0.16 looking straight away from it).
    // 0.05 looking straight away from the city (0.16: the empty Pacific horizon glowed)
    let toward = mix(0.05 + 1.55 * pow(0.5 + 0.5 * along, 3.0), 1.0, 0.75 * inside);
    let w = mix(toward, 0.55 + 0.45 * inside, smoothstep(0.15, 0.85, el));
    return SKY_GLOW_RGB * (g * shape * w);
}

// fair-weather cumulus come in fields and clear gaps, not one even
// scatter (an even field repeated every 3.2 km tile read as a pattern toward
// the horizon). The low deck's cover is modulated by the sheet's marine
// channel read over a 14 km tile: x 0.25 .. 1.75 (mean 1: the deck's mean
// cover is unchanged); a stratus deck (cover over 0.75) is not modulated.
// `big` is that channel at `(xz + drift) / SKY_CLUSTER_TILE`.
const SKY_CLUSTER_TILE: f32 = 13984.0;

fn sky_cluster_cover(cov: f32, big: f32) -> f32 {
    let stratus = clamp((cov - 0.75) / 0.2, 0.0, 1.0);
    // x 3 big^2 (mean 1, as before): most of a clear sky is empty, with a
    // few fields of cumulus (r3 x 0.25 .. 1.75 spread them evenly everywhere).
    // x (0.15 + 2.55 big^2), mean 1: a thin scatter between the fields instead of
    // none (x 3 big^2 left most clear-day cards with no cloud in view at all).
    // x 1.45 for a cumulus field: the domes stand on 0.4-0.6 of their footprint, so the
    // sheet's cover over-states what is seen (clear cards showed a cloud or two).
    // x 1.25 (1.45 made the cells big enough to tower)
    return mix(cov * 1.25 * (0.15 + 2.55 * big * big), cov, stratus);
}

// the cumulus weather map, for the march, its light taps and the cloud
// shadows alike. Cells of two sizes: the sheet's coverage at its 3.2 km tile and
// again at 2.6 x that, the larger where a ~14 km size field (the high deck's
// channel, offset) is high, so a field holds big cumulus beside small ones
// instead of one stamped size. The fields of cloud and clear sky are drawn out
// 2.5 : 1 along a NW-SE line (cloud streets on the onshore wind). Returns the
// sheet's rank (x) and the field's cover (y, `sky_cluster_cover`). `xz` carries
// the drift; `lod` is the 3.2 km sheet's mip.
fn sky_cumulus(sheet: texture_2d<f32>, smp: sampler, xz: vec2<f32>, cov: f32, lod: f32) -> vec2<f32> {
    let stratus = clamp((cov - 0.75) / 0.2, 0.0, 1.0);
    let q = vec2<f32>(dot(xz, vec2<f32>(0.8, -0.6)) / 2.5, dot(xz, vec2<f32>(0.6, 0.8)));
    // fields 0.55 x the tile (~8 km, streets ~19 km long): several in every view
    let big = textureSampleLevel(sheet, smp, q / (SKY_CLUSTER_TILE * 0.55), max(lod - 1.3, 0.0)).b;
    let sz = textureSampleLevel(sheet, smp, xz / SKY_CLUSTER_TILE + vec2<f32>(0.37, 0.11), max(lod - 2.1, 0.0)).g;
    let w = smoothstep(0.3, 0.7, sz) * (1.0 - stratus);
    // only the cell size(s) this place uses: one read where the size field is settled
    // (the same mix(a, b, w); the march reads this at every step)
    var r = 0.0;
    if (w < 1.0) {
        r = textureSampleLevel(sheet, smp, xz / SKY_LOW_TILE, lod).r * (1.0 - w);
    }
    if (w > 0.0) {
        r = r + textureSampleLevel(sheet, smp, xz / (SKY_LOW_TILE * 2.6) + vec2<f32>(0.5, 0.21), max(lod - 1.4, 0.0)).r * w;
    }
    return vec2<f32>(r, sky_cluster_cover(cov, big));
}
