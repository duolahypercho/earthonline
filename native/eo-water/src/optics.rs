//! The water body's optics: src/render/materials/ocean-water-optics.js
//! (tidewater's WaterMaterial / UnderwaterLighting, as the JS ports them):
//! the clear constants, the bay's and the open Pacific's effective
//! coefficients, the regional blend across the Golden Gate, the exact
//! dielectric Fresnel, the Cox-Munk slope variance and the single-scattering
//! water volume. The WGSL in eo-render evaluates the same formulas; the
//! numbers it reads are generated from here (`wgsl_constants`).

/// `WATER_IOR`.
pub const IOR: f64 = 1.333;
/// `WATER_SIGMA_A` / `WATER_SIGMA_S` (clear water, per metre).
pub const SIGMA_A: [f64; 3] = [0.42, 0.075, 0.035];
pub const SIGMA_S: [f64; 3] = [0.012, 0.018, 0.024];
/// `WATER_BACKSCATTER`.
pub const BACKSCATTER: f64 = 0.035;
/// `WATER_ROUGHNESS`, `WATER_SSS`, `SUN_SPEC_CEILING`, `WATER_PHASE_G`.
pub const ROUGHNESS: f64 = 0.035;
pub const SSS: f64 = 1.0;
pub const SUN_SPEC_CEILING: f64 = 400.0;
pub const PHASE_G: f64 = 0.86;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub sigma_a: [f64; 3],
    pub sigma_s: [f64; 3],
    pub backscatter: f64,
}

/// `BAY_WATER` (v2): San Francisco Bay's turbid grey-green.
pub const BAY: Body = Body { sigma_a: [2.58, 2.32, 2.7], sigma_s: [0.42, 0.48, 0.36], backscatter: 0.2 };
/// `COASTAL_WATER` (v3): the open Pacific off Ocean Beach.
pub const COASTAL: Body = Body { sigma_a: [1.14, 0.5, 0.72], sigma_s: [0.08, 0.095, 0.105], backscatter: 0.025 };
/// `GOLDEN_GATE_X`: world x over which the bay's water becomes the coast's.
pub const GOLDEN_GATE_X: [f64; 2] = [-6000.0, -6800.0];

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `regionalWaterOptics(x)`: the bay east of the Golden Gate, the coast west.
pub fn regional(x: f64) -> Body {
    let w = smoothstep(GOLDEN_GATE_X[0], GOLDEN_GATE_X[1], x);
    let mix3 = |a: [f64; 3], b: [f64; 3]| [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w, a[2] + (b[2] - a[2]) * w];
    Body { sigma_a: mix3(BAY.sigma_a, COASTAL.sigma_a), sigma_s: mix3(BAY.sigma_s, COASTAL.sigma_s), backscatter: BAY.backscatter + (COASTAL.backscatter - BAY.backscatter) * w }
}

/// `fresnelDielectric(cosI, eta)`: exact unpolarised dielectric Fresnel.
pub fn fresnel_dielectric(cos_i: f64, eta: f64) -> f64 {
    let c = cos_i.clamp(0.0, 1.0);
    let g2 = eta * eta - 1.0 + c * c;
    if g2 < 0.0 {
        return 1.0;
    }
    let g = g2.sqrt();
    let a = (g - c) / (g + c);
    let b = (c * (g + c) - 1.0) / (c * (g - c) + 1.0);
    a * a * 0.5 * (b * b + 1.0)
}

/// `coxMunkMss(windSpeed)`: 0.003 + 0.00512 U.
pub fn cox_munk_mss(wind: f64) -> f64 {
    0.003 + wind * 0.00512
}

/// The deep body's diffuse reflectance of sky light by `waterVolume`'s own
/// formula, (0.25 s/c + 1.32 bb/(a + bb)) / (muV/0.75 + 1) at view mu
/// (the JS's design check on BAY_WATER: 0.040 / 0.049 / 0.033 at muV 0.7).
pub fn deep_reflectance(b: &Body, mu_v: f64) -> [f64; 3] {
    let mut out = [0.0; 3];
    for i in 0..3 {
        let (a, s) = (b.sigma_a[i], b.sigma_s[i]);
        let c = a + s;
        let bb = s * b.backscatter;
        out[i] = (0.25 * s / c + 1.32 * bb / (a + bb)) / (mu_v / 0.75 + 1.0);
    }
    out
}

// ---- the water-leaving radiance from inherent optical properties (water r1) ----
//
// The body's colour as ocean optics measures it: the remote-sensing
// reflectance Rrs (sr^-1, water-leaving radiance over downwelling
// irradiance) of an optically deep body from its absorption a and
// backscattering bb, per render channel. The water pass multiplies it by the
// downwelling irradiance the city's diffuse surfaces are lit with (sun and
// sky), so water and land keep the ratio pi Rrs : albedo they have in reality.
//
// - Gordon et al. 1988 (J. Geophys. Res. 93(D9), 10909): below the surface
//   rrs = 0.0949 u + 0.0794 u^2, u = bb / (a + bb).
// - Lee, Carder and Arnone 2002 (Appl. Opt. 41(27), 5755, the QAA): above it
//   Rrs = 0.52 rrs / (1 - 1.7 rrs); non-water absorption as one exponential
//   a_dg(440) exp(-S (l - 440)), S = 0.015 nm^-1 (CDOM, detritus and pigment
//   lumped); particle backscatter bbp(550) (550 / l)^eta.
// - Pure water absorption, Pope and Fry 1997 (Appl. Opt. 36(33), 8710):
//   0.00981 / m at 460 nm, 0.0565 at 550, 0.2224 at 600 and 0.2755 at 620
//   (610 nm: 0.249, their mean).
// - Seawater backscatter, half of Morel's (1974) scattering, 0.00288 / m at
//   500 nm times (l / 500)^-4.32.

/// The wavelengths (nm) that stand for the render's R, G and B (near the sRGB
/// primaries' dominant wavelengths, 611, 549 and 464 nm).
pub const RGB_NM: [f64; 3] = [610.0, 550.0, 460.0];
/// Pure water absorption (1/m) at `RGB_NM` (Pope and Fry 1997, above).
pub const PURE_WATER_A: [f64; 3] = [0.249, 0.0565, 0.00981];
/// The QAA's spectral slope of the lumped non-water absorption (1/nm).
pub const A_DG_SLOPE: f64 = 0.015;

/// What a body of water holds, as its inherent optical properties.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constituents {
    /// non-water absorption at 440 nm (1/m): CDOM, detritus and pigment
    pub a_dg_440: f64,
    /// particle backscatter at 550 nm (1/m)
    pub bbp_550: f64,
    /// its spectral exponent (0 for coarse mineral grains, about 2 for fine particles)
    pub eta: f64,
}

/// San Francisco Bay's central basin at the Golden Gate: an estuary's
/// coloured dissolved matter and a moderate silt load. bbp(550) 0.03 / m is
/// about 3 mg/L of suspended mineral matter at a mass-specific backscatter of
/// 0.01 m^2/g (Babin et al. 2003, J. Geophys. Res. 108(C7), 3211: coastal
/// mineral particles 0.005-0.015 m^2/g); the central bay's surface water
/// carries a few to tens of mg/L (USGS continuous monitoring), its low end
/// in the tidal exchange at the Gate. a_dg(440) 0.45 / m sits in the range of
/// estuarine coloured matter and detritus (0.1-1 / m). The result is the bay's
/// green-peaked water (Rrs about 0.005 / 0.010 / 0.005 sr^-1).
pub const BAY_IOP: Constituents = Constituents { a_dg_440: 0.45, bbp_550: 0.03, eta: 0.8 };
/// The open Pacific off Ocean Beach (California Current coastal water, little
/// silt past the surf zone): a_dg(440) 0.08 / m, bbp(550) 0.006 / m, fine
/// particles (eta 1.2). Bluer and clearer than the bay (Rrs peaks in the blue).
pub const COAST_IOP: Constituents = Constituents { a_dg_440: 0.08, bbp_550: 0.006, eta: 1.2 };
/// A beach's surf zone: sand stirred into the water column (bbp(550) 0.15 / m,
/// about 15 mg/L of coarse grains, eta 0.3) over the coast's dissolved matter
/// raised to 0.8 / m: an opaque tan-green.
pub const SILT_IOP: Constituents = Constituents { a_dg_440: 0.8, bbp_550: 0.15, eta: 0.3 };

/// Total absorption a and backscatter bb (1/m) per render channel.
pub fn iop(c: &Constituents) -> ([f64; 3], [f64; 3]) {
    let mut a = [0.0; 3];
    let mut bb = [0.0; 3];
    for i in 0..3 {
        let l = RGB_NM[i];
        a[i] = PURE_WATER_A[i] + c.a_dg_440 * (-A_DG_SLOPE * (l - 440.0)).exp();
        let bbw = 0.5 * 0.00288 * (l / 500.0).powf(-4.32);
        bb[i] = bbw + c.bbp_550 * (550.0 / l).powf(c.eta);
    }
    (a, bb)
}

/// Gordon et al. 1988 below the surface, Lee et al. 2002 above it.
pub fn rrs_of(a: f64, bb: f64) -> f64 {
    let u = bb / (a + bb).max(1e-12);
    let below = 0.0949 * u + 0.0794 * u * u;
    0.52 * below / (1.0 - 1.7 * below)
}

/// The optically deep body's remote-sensing reflectance (sr^-1) per channel.
pub fn deep_rrs(c: &Constituents) -> [f64; 3] {
    let (a, bb) = iop(c);
    [rrs_of(a[0], bb[0]), rrs_of(a[1], bb[1]), rrs_of(a[2], bb[2])]
}

/// a + bb per channel (1/m): how fast a water column fills to the deep
/// body's colour (the shallow-water term, Lee et al. 1998, Appl. Opt. 37(27),
/// 6329, attenuates as roughly 2 (a + bb) per metre of vertical column).
pub fn fill_rate(c: &Constituents) -> [f64; 3] {
    let (a, bb) = iop(c);
    [a[0] + bb[0], a[1] + bb[1], a[2] + bb[2]]
}

/// Cox and Munk 1954 (J. Opt. Soc. Am. 44(11), 838): the mean square slope of
/// a slick-covered sea, 0.008 + 0.00156 U (capillaries damped).
pub fn cox_munk_mss_slick(wind: f64) -> f64 {
    0.008 + wind * 0.00156
}

/// Bruneton, Neyret and Holzschuch 2010 (Comput. Graph. Forum 29(2), 487,
/// "Real-time realistic ocean lighting using seamless transitions from
/// geometry to BRDF"): the Fresnel reflectance averaged over the visible
/// facets of a surface whose unresolved slopes have standard deviation
/// `sigma` in the view's plane (their fit), here over water's normal
/// reflectance so it is 0.02 at normal incidence on a flat surface.
pub fn mean_fresnel(cos_v: f64, sigma: f64) -> f64 {
    let c = cos_v.clamp(0.0, 1.0);
    let f0 = fresnel_dielectric(1.0, IOR);
    let m = (1.0 - c).powf(5.0 * (-2.69 * sigma).exp()) / (1.0 + 22.7 * sigma.max(0.0).powf(1.5));
    f0 + (1.0 - f0) * m
}

/// WGSL constants for eo-render's water shader.
pub fn wgsl_constants() -> String {
    let v3 = |v: [f64; 3]| format!("vec3<f32>({:.6}, {:.6}, {:.6})", v[0], v[1], v[2]);
    let mut s = String::from("// generated by eo-water optics::wgsl_constants (src/render/materials/ocean-water-optics.js)\n");
    s.push_str(&format!("const WO_IOR: f32 = {IOR:.6};\n"));
    s.push_str(&format!("const WO_ROUGHNESS: f32 = {ROUGHNESS:.6};\n"));
    s.push_str(&format!("const WO_SSS: f32 = {SSS:.6};\n"));
    s.push_str(&format!("const WO_SPEC_CEILING: f32 = {SUN_SPEC_CEILING:.6};\n"));
    s.push_str(&format!("const WO_PHASE_G: f32 = {PHASE_G:.6};\n"));
    s.push_str(&format!("const WO_BAY_A: vec3<f32> = {};\n", v3(BAY.sigma_a)));
    s.push_str(&format!("const WO_BAY_S: vec3<f32> = {};\n", v3(BAY.sigma_s)));
    s.push_str(&format!("const WO_BAY_BB: f32 = {:.6};\n", BAY.backscatter));
    s.push_str(&format!("const WO_COAST_A: vec3<f32> = {};\n", v3(COASTAL.sigma_a)));
    s.push_str(&format!("const WO_COAST_S: vec3<f32> = {};\n", v3(COASTAL.sigma_s)));
    s.push_str(&format!("const WO_COAST_BB: f32 = {:.6};\n", COASTAL.backscatter));
    s.push_str(&format!("const WO_GATE_X: vec2<f32> = vec2<f32>({:.1}, {:.1});\n", GOLDEN_GATE_X[0], GOLDEN_GATE_X[1]));
    // the water-leaving reflectance (water r1, `deep_rrs`, `fill_rate`)
    let v3e = |v: [f64; 3]| format!("vec3<f32>({:.7}, {:.7}, {:.7})", v[0], v[1], v[2]);
    s.push_str(&format!("const WO_BAY_RRS: vec3<f32> = {};\n", v3e(deep_rrs(&BAY_IOP))));
    s.push_str(&format!("const WO_COAST_RRS: vec3<f32> = {};\n", v3e(deep_rrs(&COAST_IOP))));
    s.push_str(&format!("const WO_SILT_RRS: vec3<f32> = {};\n", v3e(deep_rrs(&SILT_IOP))));
    s.push_str(&format!("const WO_BAY_FILL: vec3<f32> = {};\n", v3e(fill_rate(&BAY_IOP))));
    s.push_str(&format!("const WO_COAST_FILL: vec3<f32> = {};\n", v3e(fill_rate(&COAST_IOP))));
    s.push_str(&format!("const WO_SILT_FILL: vec3<f32> = {};\n", v3e(fill_rate(&SILT_IOP))));
    s.push_str(&format!("const WO_F0: f32 = {:.6};\n", fresnel_dielectric(1.0, IOR)));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gordon_and_lee_by_hand() {
        // u = 0.1: rrs = 0.00949 + 0.000794 = 0.010284; Rrs = 0.52 rrs / (1 - 1.7 rrs)
        let r = rrs_of(0.9, 0.1);
        let below = 0.010284;
        assert!((r - 0.52 * below / (1.0 - 1.7 * below)).abs() < 1e-9, "{r}");
    }

    #[test]
    fn the_bay_is_green_and_the_coast_blue() {
        let bay = deep_rrs(&BAY_IOP);
        let coast = deep_rrs(&COAST_IOP);
        eprintln!("Rrs bay {bay:?}, coast {coast:?}, silt {:?}", deep_rrs(&SILT_IOP));
        // the bay peaks in the green, about twice its red and blue
        assert!(bay[1] > 1.6 * bay[0] && bay[1] > 1.6 * bay[2], "{bay:?}");
        // the coast peaks in the blue and is darker in the red than the bay
        assert!(coast[2] > coast[1] && coast[1] > coast[0] && coast[0] < 0.5 * bay[0], "{coast:?}");
        // both inside the range coastal and estuarine waters are measured in (0.001-0.03 sr^-1)
        for v in bay.iter().chain(coast.iter()) {
            assert!((0.0008..0.03).contains(v), "{v}");
        }
        // the surf zone's silt is brighter than the bay in every channel
        let silt = deep_rrs(&SILT_IOP);
        assert!((0..3).all(|i| silt[i] > bay[i]), "{silt:?}");
    }

    #[test]
    fn mean_fresnel_limits() {
        // a flat surface at normal incidence: water's normal reflectance
        assert!((mean_fresnel(1.0, 0.0) - fresnel_dielectric(1.0, IOR)).abs() < 1e-9);
        // grazing on a flat surface: a mirror
        assert!(mean_fresnel(0.0, 0.0) > 0.999);
        // a rough sea reflects less at grazing views than a flat one (masked facets)
        assert!(mean_fresnel(0.15, 0.2) < 0.7 * mean_fresnel(0.15, 0.0));
        // and it never falls under the normal reflectance
        for c in [0.0, 0.3, 0.6, 1.0] {
            for s in [0.0, 0.1, 0.3] {
                assert!(mean_fresnel(c, s) >= fresnel_dielectric(1.0, IOR) - 1e-12);
            }
        }
    }
}
