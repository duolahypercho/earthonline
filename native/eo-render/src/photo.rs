//! Photo mode on the GPU: a physically based depth of field and the photo
//! finish, run between the temporal resolve and post when a photo is set
//! (`Renderer::photo.set`). With no photo set nothing here runs and the
//! frame is the one before photo mode existed.
//!
//! **Lens.** The camera is a thin lens in front of a 36 x 24 mm (ISO 135,
//! "full frame") sensor, cropped to the image's aspect at the full 36 mm
//! width, so a focal length is its 35 mm equivalent: f = 18 mm / tan(h / 2)
//! for the horizontal field of view h (`focal_mm`). A point at distance z
//! images to a blur circle of diameter (Hecht, *Optics*, 5th ed., sec. 5.2;
//! Ray, *Applied Photographic Optics*, 3rd ed., ch. 15)
//!
//! ```text
//! c(z) = A f (z - s) / (z (s - f)),   A = f / N
//!      = c_inf (1 - s / z),           c_inf = f^2 / (N (s - f))
//! ```
//!
//! for focus distance s and f-number N, measured along the optical axis
//! (the view depth: a flat field of focus). Behind the focus plane c > 0,
//! in front c < 0; in pixels c / sensor height x image height.
//!
//! **Aperture.** The blur disc is the image of the iris: `blades` straight
//! blades make a regular polygon of the circle's area; rounded blades (the
//! default, 9) are circular wide open and turn polygonal over the first two
//! stops down. Toward the frame's corners the barrel clips the pupil
//! (mechanical vignetting, Ray ch. 10): the disc is its intersection with a
//! second circle offset toward the frame centre, the cat's eye, which goes
//! as the lens is stopped down and the iris shrinks inside the barrel. A
//! slight over-correction of spherical aberration brightens the rim of the
//! discs behind the focus and softens those in front (H. H. Nasse, "Depth
//! of Field and Bokeh", Carl Zeiss Camera Lens Division, 2010; the
//! distribution flips either side of focus).
//!
//! **Passes** (compute, `photo.wgsl`): a depth-aware half-resolution prep
//! (a far and a near downsample; a pixel beside a depth edge, whose
//! anti-aliased colour mixes both sides, counts for little); 8 x 8 tiles of
//! the largest near and far blur, the near one dilated by its reach; a
//! scatter-as-gather over rings of equal-area taps, point-sampled (no blur
//! is interpolated across an edge), into a far layer (each tap counts where
//! the smaller of its own and the centre's disc reaches: no halo of a sharp
//! object onto the background, no background over a sharper object in
//! front) and a near layer with coverage as alpha. While the view is still
//! the gather's pattern turns every frame and both layers are accumulated
//! (a running mean), and so is each pixel's blurred share from the jittered
//! depth: the anti-aliased coverage of the focus plane's silhouettes. The
//! composite takes the sharp (temporally resolved) frame where in focus,
//! the far layer by that coverage, and at a silhouette the in-focus side's
//! colour from its fully in-focus neighbours, so the sharp background mixed
//! into an anti-aliased edge pixel never shows as a fringe; the near layer
//! goes over it. Bokeh is gathered in linear light before the tone curve.
//!
//! **Finish** (linear, before post's tone curve): exposure compensation in
//! stops; a white balance shift by a light-balancing filter's mired value
//! (Bradford adaptation between the CIE daylight-locus whites); monochrome
//! through a contrast filter's spectral response; contrast about mid-grey,
//! saturation, a faded print's lift; natural vignetting by the cos^4 law of
//! the lens; film grain in the film plane (clumped octaves calibrated to the
//! film's RMS granularity, most visible in the mid-tones); letterbox. The
//! frame's bloom is added here (and none in post) so it scales with the
//! exposure and never lights the letterbox bars.

use crate::camera::Camera;
use bytemuck::{Pod, Zeroable};
use std::sync::Mutex;

/// The sensor's width, millimetres: the ISO 135 frame (36 x 24 mm).
pub const SENSOR_WIDTH_MM: f64 = 36.0;
/// The largest blur radius drawn, as a share of the image height (full
/// resolution radius; 6 %: an 85 mm portrait at f/1.8 focused at 2.5 m
/// blurs the distance to a 5.7 % radius).
pub const MAX_RADIUS_SHARE: f64 = 0.06;
/// Frames accumulated while the view is still, by default (the weight's
/// floor is 1 / this: water and clouds still move in a held world).
pub const STILL_FRAMES: u32 = 64;
/// The same while the world moves under a still view (`--connect`): short, so nothing trails.
pub const LIVE_FRAMES: u32 = 4;
/// An offscreen still (everything held): a plain running mean.
pub const SHOT_FRAMES: u32 = 4096;
/// White point of the frame: CIE D65 (CIE 15:2004 table T.3), the sRGB / Rec. 709 white the renderer is neutral at.
pub const D65_XY: [f64; 2] = [0.31271, 0.32902];
/// The photo lens's widest stop (the first of photo mode's third stops):
/// its rounded blades make a circle there.
pub const WIDEST_F: f64 = 1.4;
/// The cat's eye at the frame's corner wide open: the barrel's circle (the
/// iris's radius at the widest stop) sits this share of the disc's radius
/// toward the frame centre, so the corner disc is about a quarter narrower
/// radially than across (a fast short telephoto's wide-open corner).
pub const CAT_EYE_CORNER: f64 = 0.55;
/// Rim brightening of the discs behind the focus (and the softening of
/// those in front): a slight spherical over-correction (see the module notes).
pub const BOKEH_RIM: f64 = 0.25;
/// The warm film's saturation (see `Grade::look`).
pub const WARM_SATURATION: f64 = 1.08;
/// The circle of confusion the "in focus" limits use: 0.03 mm, the classic
/// value for the 135 frame (Ray, *Applied Photographic Optics*, ch. 15).
pub const COC_LIMIT_MM: f64 = 0.03;

/// A film's grain, as its data sheet gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Film {
    pub name: &'static str,
    /// Diffuse RMS granularity x 1000, at net diffuse density 1.0 through a 48 um aperture.
    pub rms_granularity: f64,
    /// Development contrast (density per log10 exposure) the grain is seen through.
    pub gamma: f64,
}

/// Kodak Tri-X 400: diffuse RMS granularity 17 (Kodak publication F-4017,
/// "Kodak Professional Tri-X 320 and 400 Films", granularity table),
/// developed to a contrast index of 0.6 (the F-4017 recommendation for
/// diffusion enlargers is 0.56-0.62).
pub const TRI_X_400: Film = Film { name: "Tri-X 400", rms_granularity: 17.0, gamma: 0.6 };
/// A modern fine-grain 400 film: Kodak T-MAX 400, diffuse RMS granularity 10
/// (Kodak publication F-4043), the stand-in for a 400-speed colour negative.
pub const T_MAX_400: Film = Film { name: "T-MAX 400 class", rms_granularity: 10.0, gamma: 0.6 };

/// Per-pixel grain, stops (sigma of log2 exposure), of uncorrelated grain:
/// Selwyn's law (sigma x sqrt(aperture area) is constant; Dainty and Shaw,
/// *Image Science*, 1974, ch. 6) takes the published sigma at 48 um to the
/// pixel's aperture; the film's gamma takes density to log exposure. The
/// grain field (`grain_octaves`) follows it while its clumps are smaller than a pixel.
pub fn grain_sigma_stops(film: &Film, pixel_pitch_um: f64) -> f64 {
    let sigma_d = film.rms_granularity / 1000.0 * A48_SIDE_UM / pixel_pitch_um.max(1.0);
    sigma_d / film.gamma * std::f64::consts::LOG2_10
}

/// The side of the square of the 48 um circular aperture's area (24 sqrt(pi) um).
pub const A48_SIDE_UM: f64 = 42.539;
/// The grain field's octaves on the film, um: silver (or dye) clumps from a
/// few microns to the 50 um aggregates that make graininess visible.
pub const GRAIN_CELLS_UM: [f64; 4] = [6.0, 12.0, 24.0, 48.0];
/// Their relative amplitudes: a film's noise (Wiener) spectrum, flat to the
/// clump frequency and falling past it (Dainty and Shaw ch. 8).
pub const GRAIN_SPECTRUM: [f64; 4] = [1.0, 0.65, 0.4, 0.22];

/// The grain field at a pixel pitch (um on the 36 mm frame): each octave's
/// sigma (stops) as a pixel sees it (an octave finer than the pixel is
/// averaged over it by Selwyn's law) and its cell in pixels. Calibrated so
/// that through the 48 um aperture the field's sigma is the film's RMS
/// granularity: a 2x capture shows clumps twice as many pixels across, the
/// same grain at the same print size.
pub fn grain_octaves(film: &Film, pixel_pitch_um: f64) -> ([f64; 4], [f64; 4]) {
    let p = pixel_pitch_um.max(0.5);
    let seen = |l: f64, a: f64| (l / a).min(1.0);
    let at48: f64 = (0..4).map(|o| (GRAIN_SPECTRUM[o] * seen(GRAIN_CELLS_UM[o], A48_SIDE_UM)).powi(2)).sum::<f64>().sqrt();
    let s = film.rms_granularity / 1000.0 / at48 / film.gamma * std::f64::consts::LOG2_10;
    let sig = std::array::from_fn(|o| s * GRAIN_SPECTRUM[o] * seen(GRAIN_CELLS_UM[o], p));
    let cell = std::array::from_fn(|o| GRAIN_CELLS_UM[o] / p);
    (sig, cell)
}

/// The grade presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    Neutral,
    WarmFilm,
    Cool,
    Monochrome,
}

pub const GRADES: [Grade; 4] = [Grade::Neutral, Grade::WarmFilm, Grade::Cool, Grade::Monochrome];

/// What a grade does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradeLook {
    /// White balance shift, mired (positive warms): a Kodak Wratten
    /// light-balancing filter's value (Kodak Photographic Filters Handbook,
    /// publication B-3: 81A +18, 81B +27, 81C +35, 82A -21, 82B -32).
    pub mired: f64,
    /// The filter the shift is (shown in the panel).
    pub filter: &'static str,
    pub saturation: f64,
    /// Contrast: a log-exposure slope about mid-grey (1: none).
    pub gamma: f64,
    pub mono: bool,
    /// The film whose grain "grain 100 %" is.
    pub film: Film,
    /// Grain share a preset starts with.
    pub grain: f64,
}

impl Grade {
    pub fn name(self) -> &'static str {
        match self {
            Grade::Neutral => "neutral",
            Grade::WarmFilm => "warm-film",
            Grade::Cool => "cool",
            Grade::Monochrome => "monochrome",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Grade::Neutral => "Neutral",
            Grade::WarmFilm => "Warm film",
            Grade::Cool => "Cool",
            Grade::Monochrome => "Monochrome",
        }
    }
    pub fn parse(s: &str) -> Option<Grade> {
        GRADES.iter().copied().find(|g| g.name() == s || g.label().eq_ignore_ascii_case(s))
    }
    pub fn look(self) -> GradeLook {
        match self {
            Grade::Neutral => GradeLook { mired: 0.0, filter: "", saturation: 1.0, gamma: 1.0, mono: false, film: T_MAX_400, grain: 0.0 },
            // an 81A warming filter: a white-balance shift in the scene's
            // linear light (no lift, no fade, the neutral tone curve), so blue
            // sky and water stay blue (hue within a few degrees). A colour
            // film's slightly richer colour than the eye's (+8 %, a look, not
            // a measurement) keeps the bay's blue as saturated as before the shift.
            Grade::WarmFilm => GradeLook { mired: 18.0, filter: "81A", saturation: WARM_SATURATION, gamma: 1.0, mono: false, film: T_MAX_400, grain: 0.5 },
            // an 82B cooling filter.
            Grade::Cool => GradeLook { mired: -32.0, filter: "82B", saturation: 1.0, gamma: 1.0, mono: false, film: T_MAX_400, grain: 0.0 },
            // panchromatic film through a contrast filter (`MonoFilter`), a print's contrast, Tri-X grain.
            Grade::Monochrome => GradeLook { mired: 0.0, filter: "", saturation: 0.0, gamma: 1.15, mono: true, film: TRI_X_400, grain: 0.7 },
        }
    }
}

/// Monochrome's contrast filter: a Kodak Wratten filter in front of a
/// panchromatic film. Each is the RGB combination closest (least squares
/// over 380-720 nm, the sRGB channels' colour-matching functions from the
/// CIE 1931 observer in the Wyman-Sloan-Shirley fit, JCGT 2(2) 2013) to
/// the film's spectral response through the filter: an energy-flat
/// panchromatic film to about 655 nm (Kodak F-4017's spectral-sensitivity
/// curve), long-pass filters cutting on at their published 50 % points
/// (8 at 465 nm, 15 at 510, 21 at 530, 25 at 580; 58 a 495-590 nm band),
/// normalised so a neutral grey keeps its value (the filter factor given
/// back as exposure). Check: the fit's filter factors in daylight are 1.5
/// (8), 2.0 (15), 2.4 (21) and 4.2 (25), against Kodak's 1.5-2, 2-2.5, 3-4
/// and 8 (the fit's red lets through more of the deep red than film sees).
/// The fit of the eye's own luminance this way is (0.222, 0.704, 0.075),
/// within 0.012 of Rec. 709's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonoFilter {
    None,
    Yellow,
    DeepYellow,
    Orange,
    Red,
    Green,
}

pub const MONO_FILTERS: [MonoFilter; 6] = [MonoFilter::None, MonoFilter::Yellow, MonoFilter::DeepYellow, MonoFilter::Orange, MonoFilter::Red, MonoFilter::Green];

impl MonoFilter {
    /// The channel weights (sum 1).
    pub fn weights(self) -> [f64; 3] {
        match self {
            MonoFilter::None => [0.195, 0.437, 0.368],
            MonoFilter::Yellow => [0.248, 0.580, 0.172],
            MonoFilter::DeepYellow => [0.357, 0.609, 0.034],
            MonoFilter::Orange => [0.445, 0.536, 0.019],
            MonoFilter::Red => [0.790, 0.206, 0.004],
            MonoFilter::Green => [0.078, 0.828, 0.094],
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            MonoFilter::None => "none",
            MonoFilter::Yellow => "yellow",
            MonoFilter::DeepYellow => "deep-yellow",
            MonoFilter::Orange => "orange",
            MonoFilter::Red => "red",
            MonoFilter::Green => "green",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            MonoFilter::None => "No filter",
            MonoFilter::Yellow => "Yellow 8",
            MonoFilter::DeepYellow => "Deep yellow 15",
            MonoFilter::Orange => "Orange 21",
            MonoFilter::Red => "Red 25",
            MonoFilter::Green => "Green 58",
        }
    }
    pub fn parse(s: &str) -> Option<MonoFilter> {
        MONO_FILTERS.iter().copied().find(|f| f.name() == s || f.label().eq_ignore_ascii_case(s))
    }
}

/// The lens's iris.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aperture {
    /// Straight blades (a polygon of their number) or 0: an ideal circle.
    pub blades: u32,
    /// The blades are rounded: circular at `WIDEST_F`, polygonal two stops down.
    pub rounded: bool,
    /// The barrel's cat's eye toward the corners.
    pub cat_eye: bool,
}

impl Default for Aperture {
    fn default() -> Self {
        Self { blades: 9, rounded: true, cat_eye: true }
    }
}

/// The lens's focus: where and how wide open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dof {
    pub f_number: f64,
    /// Focus distance along the view axis, metres.
    pub focus_m: f64,
    pub aperture: Aperture,
}

/// One photo-mode frame's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhotoParams {
    /// None: no depth of field (everything sharp).
    pub dof: Option<Dof>,
    /// Exposure compensation, stops.
    pub exposure_ev: f64,
    pub grade: Grade,
    /// 0..2 of the grade's departure from neutral (1: the preset).
    pub strength: f64,
    /// Monochrome's contrast filter.
    pub filter: MonoFilter,
    /// 0..1: a faded print (lifted blacks, a softer top); 0: none.
    pub fade: f64,
    /// 0..1 of the lens's natural (cos^4) vignetting.
    pub vignette: f64,
    /// 0..1 of the grade's film grain.
    pub grain: f64,
    /// Letterbox image aspect (2.39, 1.85, ...), None: the full frame.
    pub letterbox: Option<f64>,
    /// -1..1: where the letterbox's opening sits (0 centred, -1 against the top, 1 against the bottom).
    pub frame_offset: f64,
    /// Frames the depth of field accumulates under a still view at most
    /// (`STILL_FRAMES`, `LIVE_FRAMES` while the world moves, `SHOT_FRAMES` for a held still; 1: none).
    pub accumulate: u32,
    /// A new grain every frame (in the window); a still holds its grain.
    pub animate_grain: bool,
    /// Focus peaking: the in-focus edges marked (the window only; never in a saved photo).
    pub peaking: bool,
}

impl Default for PhotoParams {
    fn default() -> Self {
        Self {
            dof: None,
            exposure_ev: 0.0,
            grade: Grade::Neutral,
            strength: 1.0,
            filter: MonoFilter::DeepYellow,
            fade: 0.0,
            vignette: 0.0,
            grain: 0.0,
            letterbox: None,
            frame_offset: 0.0,
            accumulate: STILL_FRAMES,
            animate_grain: false,
            peaking: false,
        }
    }
}

/// The focal length (mm, 35 mm equivalent) a vertical field of view gives at an aspect.
pub fn focal_mm(fov_y_deg: f64, aspect: f64) -> f64 {
    0.5 * SENSOR_WIDTH_MM / aspect.max(1e-3) / (0.5 * fov_y_deg.to_radians()).tan()
}

/// The vertical field of view (degrees) of a focal length at an aspect.
pub fn fov_y_deg(focal_mm: f64, aspect: f64) -> f64 {
    2.0 * (0.5 * SENSOR_WIDTH_MM / aspect.max(1e-3) / focal_mm.max(1.0)).atan().to_degrees()
}

/// The horizontal field of view (degrees) of a focal length: across the 36 mm width at any aspect.
pub fn fov_x_deg(focal_mm: f64) -> f64 {
    2.0 * (0.5 * SENSOR_WIDTH_MM / focal_mm.max(1.0)).atan().to_degrees()
}

/// The blur-circle diameter at infinity on the sensor, mm (thin lens).
pub fn coc_inf_mm(focal_mm: f64, f_number: f64, focus_m: f64) -> f64 {
    let f = focal_mm;
    let s = (focus_m * 1000.0).max(f * 1.001);
    f * f / (f_number.max(0.5) * (s - f))
}

/// The signed blur-circle diameter (mm on the sensor) of a point at view depth `z_m`.
pub fn coc_mm(focal_mm: f64, f_number: f64, focus_m: f64, z_m: f64) -> f64 {
    coc_inf_mm(focal_mm, f_number, focus_m) * (1.0 - focus_m / z_m.max(1e-3))
}

/// The hyperfocal distance, metres, for a circle of confusion `c_mm`
/// (the classic 0.03 mm for the 135 frame): H = f^2 / (N c) + f.
pub fn hyperfocal_m(focal_mm: f64, f_number: f64, c_mm: f64) -> f64 {
    (focal_mm * focal_mm / (f_number * c_mm) + focal_mm) / 1000.0
}

/// The near and far limits of the depth of field (metres; far None: to
/// infinity) at a circle of confusion `c_mm`: from c(z) = +-c (the thin lens above).
pub fn dof_limits_m(focal_mm: f64, f_number: f64, focus_m: f64, c_mm: f64) -> (f64, Option<f64>) {
    let k = coc_inf_mm(focal_mm, f_number, focus_m);
    let near = focus_m / (1.0 + c_mm / k);
    let far = (k > c_mm).then(|| focus_m / (1.0 - c_mm / k));
    (near, far)
}

/// The circumradius of a regular polygon of `n` sides with the unit circle's area.
pub fn polygon_area_radius(n: u32) -> f64 {
    let n = n.max(3) as f64;
    (2.0 * std::f64::consts::PI / (n * (2.0 * std::f64::consts::PI / n).sin())).sqrt()
}

/// How round a rounded-blade iris is at a stop: 1 wide open (`WIDEST_F`), 0 from two stops down.
pub fn blade_roundness(f_number: f64) -> f64 {
    let stops = 2.0 * (f_number / WIDEST_F).max(1.0).log2();
    (1.0 - stops / 2.0).clamp(0.0, 1.0)
}

/// The cat's eye at a stop: the barrel circle's offset toward the frame centre at the corner and its radius, both in disc radii.
pub fn cat_eye(f_number: f64) -> (f64, f64) {
    let k = (f_number / WIDEST_F).max(1.0);
    (CAT_EYE_CORNER * k, k)
}

/// CIE daylight-locus chromaticity of a correlated colour temperature
/// (CIE 15:2004, eq. 3.3-3.4; 4000-25000 K).
pub fn daylight_xy(cct: f64) -> [f64; 2] {
    let t = cct.clamp(4000.0, 25000.0);
    let x = if t <= 7000.0 {
        -4.6070e9 / t.powi(3) + 2.9678e6 / t.powi(2) + 0.09911e3 / t + 0.244063
    } else {
        -2.0064e9 / t.powi(3) + 1.9018e6 / t.powi(2) + 0.24748e3 / t + 0.237040
    };
    let y = -3.000 * x * x + 2.870 * x - 0.275;
    [x, y]
}

/// The correlated colour temperature a mired shift takes the frame's D65 white to.
pub fn shifted_cct(mired: f64) -> f64 {
    1.0e6 / (1.0e6 / 6504.0 + mired)
}

type M3 = [[f64; 3]; 3];

fn mul(a: &M3, b: &M3) -> M3 {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

fn apply(a: &M3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i][0] * v[0] + a[i][1] * v[1] + a[i][2] * v[2])
}

fn inverse(m: &M3) -> M3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let k = 1.0 / det;
    [[(e * i - f * h) * k, (c * h - b * i) * k, (b * f - c * e) * k], [(f * g - d * i) * k, (a * i - c * g) * k, (c * d - a * f) * k], [(d * h - e * g) * k, (b * g - a * h) * k, (a * e - b * d) * k]]
}

/// Linear sRGB / Rec. 709 to CIE XYZ (IEC 61966-2-1, D65).
pub const RGB_TO_XYZ: M3 = [[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.0721750], [0.0193339, 0.1191920, 0.9503041]];
/// The Bradford cone response (Lam 1985, as in CIE 159:2004's CMCCAT97 lineage).
pub const BRADFORD: M3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];

fn xyz_of(xy: [f64; 2]) -> [f64; 3] {
    [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

/// The linear-RGB matrix of a light-balancing filter of `mired` (positive
/// warms): the frame's white (D65) is adapted to the daylight white `mired`
/// lower in colour temperature (Bradford), so what was neutral takes the
/// filter's tint; white's luminance is kept.
pub fn white_balance(mired: f64) -> [[f64; 3]; 3] {
    if mired == 0.0 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let target = shifted_cct(mired);
    let src = apply(&BRADFORD, xyz_of(D65_XY));
    let dst = apply(&BRADFORD, xyz_of(daylight_xy(target)));
    let scale: M3 = [[dst[0] / src[0], 0.0, 0.0], [0.0, dst[1] / src[1], 0.0], [0.0, 0.0, dst[2] / src[2]]];
    let cat = mul(&inverse(&BRADFORD), &mul(&scale, &BRADFORD));
    mul(&inverse(&RGB_TO_XYZ), &mul(&cat, &RGB_TO_XYZ))
}

/// A grade at a strength: (mired, saturation, contrast, mono weights or None).
pub fn graded(grade: Grade, strength: f64, filter: MonoFilter) -> (f64, f64, f64, Option<[f64; 3]>) {
    let l = grade.look();
    let s = strength.clamp(0.0, 2.0);
    let mono = l.mono.then(|| {
        let a = MonoFilter::None.weights();
        let b = filter.weights();
        let w: [f64; 3] = std::array::from_fn(|i| (a[i] + (b[i] - a[i]) * s).max(0.0));
        let t = w.iter().sum::<f64>().max(1e-6);
        w.map(|x| x / t)
    });
    (l.mired * s, 1.0 + (l.saturation - 1.0) * if l.mono { 1.0 } else { s }, 1.0 + (l.gamma - 1.0) * s, mono)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct PhotoUniform {
    size: [f32; 4],
    half: [f32; 4],
    lens: [f32; 4],
    lens2: [f32; 4],
    grade: [f32; 4],
    wb0: [f32; 4],
    wb1: [f32; 4],
    wb2: [f32; 4],
    mono: [f32; 4],
    vig: [f32; 4],
    frame: [f32; 4],
    bokeh: [f32; 4],
    cat: [f32; 4],
    grain_a: [f32; 4],
    grain_c: [f32; 4],
    extra: [f32; 4],
}

/// Switches (`--no-dof` leaves photo mode's finish without its depth of field).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhotoOptions {
    pub dof: bool,
}

impl Default for PhotoOptions {
    fn default() -> Self {
        Self { dof: true }
    }
}

struct Targets {
    size: (u32, u32),
    prep_far: wgpu::TextureView,
    prep_near: wgpu::TextureView,
    _tiles: [wgpu::TextureView; 2],
    /// the accumulated far and near layers (half resolution), by parity.
    far: [wgpu::TextureView; 2],
    near: [wgpu::TextureView; 2],
    /// each pixel's accumulated blurred share (full resolution), by parity.
    cov: [wgpu::TextureView; 2],
    out: wgpu::TextureView,
    tiles_bg: wgpu::BindGroup,
    dilate_bg: wgpu::BindGroup,
    /// the gather writing parity p.
    gather_bg: [wgpu::BindGroup; 2],
    /// (source texture, depth texture, bloom texture, history parity, prep group, final group, post group)
    binds: Vec<(wgpu::Texture, wgpu::Texture, wgpu::Texture, usize, wgpu::BindGroup, wgpu::BindGroup, wgpu::BindGroup)>,
    bytes: u64,
}

struct State {
    params: Option<PhotoParams>,
    /// The view the accumulation holds (eye, yaw, pitch, roll, fov, aspect) and its params.
    held: Option<([f32; 8], PhotoParams, (u32, u32))>,
    /// Frames accumulated since the view last moved.
    frames: u32,
    parity: usize,
    /// The uniform's output size (an encode at another size skips the pass).
    written: Option<(u32, u32)>,
    grain_frame: u32,
}

/// The photo passes' own GPU timer (`enable_timer`): two timestamps round them.
struct Timer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    written: bool,
}

pub struct PhotoGpu {
    pub options: PhotoOptions,
    timer: Mutex<Option<Timer>>,
    device: wgpu::Device,
    uniform: wgpu::Buffer,
    bgl: [wgpu::BindGroupLayout; 5],
    pipes: [wgpu::ComputePipeline; 5],
    lin: wgpu::Sampler,
    black: wgpu::TextureView,
    targets: Mutex<Option<Targets>>,
    state: Mutex<State>,
}

fn entry_tv(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) }
}

const HALF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
/// The accumulated layers in full float: a running mean over thousands of
/// frames loses its small steps in half float.
const LAYER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const COV_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

impl PhotoGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let d = device;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let t2 = |filterable| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let depth2 = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let store = |format| wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 };
        let samp = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
        let bgl = [
            layout("photo-prep-bgl", &[entry(0, ubuf), entry(1, t2(false)), entry(2, depth2), entry(3, store(HALF_FORMAT)), entry(4, store(HALF_FORMAT))]),
            layout("photo-tiles-bgl", &[entry(0, ubuf), entry(5, t2(true)), entry(6, t2(true)), entry(7, store(TILE_FORMAT))]),
            layout("photo-dilate-bgl", &[entry(0, ubuf), entry(8, t2(false)), entry(7, store(TILE_FORMAT))]),
            layout(
                "photo-gather-bgl",
                &[entry(0, ubuf), entry(5, t2(true)), entry(6, t2(true)), entry(8, t2(false)), entry(10, store(LAYER_FORMAT)), entry(11, store(LAYER_FORMAT)), entry(18, t2(false)), entry(19, t2(false))],
            ),
            layout(
                "photo-final-bgl",
                &[entry(0, ubuf), entry(1, t2(false)), entry(2, depth2), entry(5, t2(true)), entry(9, samp), entry(12, t2(false)), entry(13, t2(false)), entry(14, t2(false)), entry(15, store(COV_FORMAT)), entry(16, store(HALF_FORMAT)), entry(17, t2(true))],
            ),
        ];
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("photo.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("photo.wgsl").into()) });
        let names = ["prep", "tiles", "dilate", "gather", "final_pass"];
        let pipes = std::array::from_fn(|i| {
            let l = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("photo-layout"), bind_group_layouts: &[Some(&bgl[i])], immediate_size: 0 });
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(names[i]), layout: Some(&l), module: &module, entry_point: Some(names[i]), compilation_options: Default::default(), cache: None })
        });
        let uniform = d.create_buffer(&wgpu::BufferDescriptor { label: Some("photo-uniform"), size: std::mem::size_of::<PhotoUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let lin = d.create_sampler(&wgpu::SamplerDescriptor { label: Some("photo-linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        // post adds this (black) in place of the bloom: the photo pass adds the bloom itself.
        let black_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("photo-black"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HALF_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &black_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[0u8; 8],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8), rows_per_image: None },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let black = black_tex.create_view(&Default::default());
        Self {
            options: PhotoOptions::default(),
            timer: Mutex::new(None),
            device: d.clone(),
            uniform,
            bgl,
            pipes,
            lin,
            black,
            targets: Mutex::new(None),
            state: Mutex::new(State { params: None, held: None, frames: 0, parity: 0, written: None, grain_frame: 0 }),
        }
    }

    /// Photo mode's settings from the next frame on (None: photo mode off, the frame as before).
    pub fn set(&self, params: Option<PhotoParams>) {
        let mut s = self.state.lock().unwrap();
        if params.is_none() {
            s.held = None;
            s.frames = 0;
            s.written = None;
        }
        s.params = params;
    }

    pub fn params(&self) -> Option<PhotoParams> {
        self.state.lock().unwrap().params
    }

    /// Whether a photo is set (the scene's depth must then be kept for the pass).
    pub fn active(&self) -> bool {
        self.state.lock().unwrap().params.is_some()
    }

    /// Frames accumulated under the still view (0 while it moves).
    pub fn still_frames(&self) -> u32 {
        self.state.lock().unwrap().frames
    }

    /// Time the photo passes on the GPU from the next frame on (needs timestamp queries).
    pub fn enable_timer(&self, device: &wgpu::Device) {
        let set = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("photo-ts"), ty: wgpu::QueryType::Timestamp, count: 2 });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor { label: Some("photo-ts-resolve"), size: 16, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = device.create_buffer(&wgpu::BufferDescriptor { label: Some("photo-ts-read"), size: 16, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        *self.timer.lock().unwrap() = Some(Timer { set, resolve, read, written: false });
    }

    /// The last timed frame's photo passes, milliseconds (blocks on the GPU; None before a timed frame).
    pub fn timer_ms(&self, device: &wgpu::Device, period_ns: f64) -> Option<f64> {
        let mut t = self.timer.lock().unwrap();
        let t = t.as_mut()?;
        if !t.written {
            return None;
        }
        t.written = false;
        let slice = t.read.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let v: [u64; 2] = {
            let d = slice.get_mapped_range().ok()?;
            [u64::from_le_bytes(d[0..8].try_into().unwrap()), u64::from_le_bytes(d[8..16].try_into().unwrap())]
        };
        t.read.unmap();
        Some(v[1].saturating_sub(v[0]) as f64 * period_ns / 1e6)
    }

    /// Bytes of the size-dependent targets.
    pub fn bytes(&self) -> u64 {
        self.targets.lock().unwrap().as_ref().map(|t| t.bytes).unwrap_or(0)
    }

    /// Write this frame's uniform (from `Renderer::update`): the lens from
    /// the camera's field of view, the accumulation's weight. `exposure` is
    /// the one post tone-maps this frame with (the peaking marks and the
    /// grade's mid-grey follow it).
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, out: (u32, u32), bloom: bool, exposure: f64) {
        let mut s = self.state.lock().unwrap();
        let Some(p) = s.params else { return };
        let (w, h) = (out.0.max(2), out.1.max(2));
        let aspect = w as f64 / h as f64;
        let view = [cam.eye.x, cam.eye.y, cam.eye.z, cam.yaw, cam.pitch, cam.roll, cam.fov_y_deg, cam.aspect];
        // the peaking marks are drawn over the frame: they never restart the accumulation.
        let key = PhotoParams { peaking: false, ..p };
        let same = s.held.as_ref().is_some_and(|(v, q, sz)| *q == key && *sz == (w, h) && v.iter().zip(view.iter()).all(|(a, b)| (a - b).abs() <= 1e-5 * a.abs().max(1.0)));
        s.frames = if same { s.frames.saturating_add(1) } else { 0 };
        s.held = Some((view, key, (w, h)));
        s.written = Some((w, h));
        s.grain_frame = if p.animate_grain { s.grain_frame.wrapping_add(1) } else { 0 };
        let n = s.frames;
        let cap = p.accumulate.max(1);
        let weight = 1.0 / (n.min(cap - 1) + 1) as f32;
        let f_mm = focal_mm(cam.fov_y_deg as f64, aspect);
        let sensor_h = SENSOR_WIDTH_MM / aspect;
        let half_h = (h / 2).max(1) as f64;
        let hw = (w / 2).max(1);
        let hh = (h / 2).max(1);
        let r_max = MAX_RADIUS_SHARE * h as f64 * 0.5;
        // radius in half px: diameter / sensor height x half height / 2
        let to_half = |mm: f64| mm / sensor_h * half_h * 0.5;
        let dof_on = p.dof.is_some() && self.options.dof;
        let (c_inf_half, focus, ap, fnum) = match (p.dof, self.options.dof) {
            (Some(d), true) => (to_half(coc_inf_mm(f_mm, d.f_number, d.focus_m)), d.focus_m, d.aperture, d.f_number),
            _ => (0.0, 1.0, Aperture::default(), WIDEST_F),
        };
        let (mired, saturation, contrast, mono) = graded(p.grade, p.strength, p.filter);
        let look = p.grade.look();
        let wb = white_balance(mired);
        let pitch_um = SENSOR_WIDTH_MM * 1000.0 / w as f64;
        let g = p.grain.clamp(0.0, 1.0);
        let (gs, gc) = grain_octaves(&look.film, pitch_um);
        // the gather's pattern: still, a golden-angle turn and an R2 radial step per frame.
        let (turn, radial) = if dof_on && n > 0 { ((n as f64 * 2.399963).rem_euclid(std::f64::consts::TAU), (0.5 + n as f64 * 0.7548776662).fract()) } else { (0.0, 0.5) };
        let flags = (dof_on as u32) | ((bloom as u32) << 1) | ((p.peaking as u32) << 2);
        let row = |r: [f64; 3]| [r[0] as f32, r[1] as f32, r[2] as f32, 0.0];
        let blades = if ap.blades >= 3 { ap.blades as f32 } else { 0.0 };
        let round = if ap.rounded { blade_roundness(fnum) } else { 0.0 };
        let (cat_off, cat_r) = if ap.cat_eye { cat_eye(fnum) } else { (0.0, 1.0) };
        let mono_w = mono.unwrap_or([0.0; 3]);
        let u = PhotoUniform {
            size: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
            half: [hw as f32, hh as f32, hw.div_ceil(8) as f32, hh.div_ceil(8) as f32],
            lens: [c_inf_half as f32, focus as f32, cam.near, cam.far],
            lens2: [r_max as f32, turn as f32, weight, flags as f32],
            grade: [2f64.powf(p.exposure_ev) as f32, saturation as f32, contrast as f32, p.fade.clamp(0.0, 1.0) as f32],
            wb0: row(wb[0]),
            wb1: row(wb[1]),
            wb2: row(wb[2]),
            mono: [mono_w[0] as f32, mono_w[1] as f32, mono_w[2] as f32, if mono.is_some() { 1.0 } else { 0.0 }],
            vig: [p.vignette.clamp(0.0, 1.0) as f32, (SENSOR_WIDTH_MM / f_mm) as f32, (sensor_h / f_mm) as f32, (exposure.max(1e-6) / 0.6) as f32],
            frame: [p.letterbox.unwrap_or(0.0) as f32, (s.grain_frame % 9973) as f32, 0.3, radial as f32],
            // a straight-bladed iris's polygon turned so a flat side is at the bottom.
            bokeh: [blades, round as f32, if blades > 0.0 { (std::f64::consts::FRAC_PI_2 + std::f64::consts::PI / blades as f64) as f32 } else { 0.0 }, polygon_area_radius(ap.blades.max(3)) as f32],
            cat: [cat_off as f32, cat_r as f32, BOKEH_RIM as f32, p.frame_offset.clamp(-1.0, 1.0) as f32],
            grain_a: gs.map(|x| (x * g) as f32),
            grain_c: gc.map(|x| x as f32),
            extra: [(0.5 * ((w as f64).powi(2) + (h as f64).powi(2)).sqrt()) as f32, to_half(COC_LIMIT_MM * 0.5).max(0.25) as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
    }

    fn ensure(&self, size: (u32, u32)) {
        let mut t = self.targets.lock().unwrap();
        if t.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let d = &self.device;
        let half = ((size.0 / 2).max(1), (size.1 / 2).max(1));
        let tiles = (half.0.div_ceil(8), half.1.div_ceil(8));
        let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let tex = |label: &str, s: (u32, u32), format| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: s.0, height: s.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
            .create_view(&Default::default())
        };
        let prep_far = tex("photo-prep-far", half, HALF_FORMAT);
        let prep_near = tex("photo-prep-near", half, HALF_FORMAT);
        let tiles_t = [tex("photo-tiles", tiles, TILE_FORMAT), tex("photo-tiles-dilated", tiles, TILE_FORMAT)];
        let far = [tex("photo-far-0", half, LAYER_FORMAT), tex("photo-far-1", half, LAYER_FORMAT)];
        let near = [tex("photo-near-0", half, LAYER_FORMAT), tex("photo-near-1", half, LAYER_FORMAT)];
        let cov = [tex("photo-cov-0", size, COV_FORMAT), tex("photo-cov-1", size, COV_FORMAT)];
        let out = tex("photo-out", size, HALF_FORMAT);
        let bg = |i: usize, label: &str, entries: &[wgpu::BindGroupEntry]| d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &self.bgl[i], entries });
        let tv = entry_tv;
        let u0 = wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() };
        let tiles_bg = bg(1, "photo-tiles-bg", &[u0.clone(), tv(5, &prep_far), tv(6, &prep_near), tv(7, &tiles_t[0])]);
        let dilate_bg = bg(2, "photo-dilate-bg", &[u0.clone(), tv(8, &tiles_t[0]), tv(7, &tiles_t[1])]);
        let gather = |p: usize| {
            bg(3, "photo-gather-bg", &[u0.clone(), tv(5, &prep_far), tv(6, &prep_near), tv(8, &tiles_t[1]), tv(10, &far[p]), tv(11, &near[p]), tv(18, &far[1 - p]), tv(19, &near[1 - p])])
        };
        let gather_bg = [gather(0), gather(1)];
        let px = |s: (u32, u32)| s.0 as u64 * s.1 as u64;
        let bytes = px(half) * (8 * 2 + 16 * 4) + px(tiles) * 8 * 2 + px(size) * (4 * 2 + 8);
        *t = Some(Targets { size, prep_far, prep_near, _tiles: tiles_t, far, near, cov, out, tiles_bg, dilate_bg, gather_bg, binds: vec![], bytes });
        let mut s = self.state.lock().unwrap();
        s.held = None; // a new size starts the accumulation again
        s.frames = 0;
    }

    /// Record the photo passes after the temporal resolve; returns the bind
    /// group post reads (the photo output, no bloom), or None when no photo
    /// is set (post reads what it read before).
    pub(crate) fn encode(&self, enc: &mut wgpu::CommandEncoder, look: &crate::look::LookGpu, depth: &wgpu::TextureView, hdr: &wgpu::TextureView, size: (u32, u32)) -> Option<wgpu::BindGroup> {
        let (written, parity, dof_on) = {
            let s = self.state.lock().unwrap();
            let p = s.params?;
            (s.written, s.parity, p.dof.is_some() && self.options.dof)
        };
        let dt = depth.texture();
        if written != Some(size) || dt.sample_count() != 1 || !dt.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) {
            return None; // the uniform was written for another size this frame, or no readable depth
        }
        self.ensure(size);
        let src = look.post_source_view(hdr);
        let bloom = look.bloom_view();
        let mut tg = self.targets.lock().unwrap();
        let t = tg.as_mut().unwrap();
        let (st, dtt) = (src.texture().clone(), dt.clone());
        let bt = bloom.as_ref().map(|b| b.texture().clone()).unwrap_or_else(|| self.black.texture().clone());
        let found = t.binds.iter().position(|b| b.0 == st && b.1 == dtt && b.2 == bt && b.3 == parity);
        let k = match found {
            Some(k) => k,
            None => {
                let d = &self.device;
                let tv = entry_tv;
                let u0 = wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() };
                let prep = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("photo-prep-bg"), layout: &self.bgl[0], entries: &[u0.clone(), tv(1, &src), tv(2, depth), tv(3, &t.prep_far), tv(4, &t.prep_near)] });
                let bloom_v = bloom.clone().unwrap_or_else(|| self.black.clone());
                let fin = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("photo-final-bg"),
                    layout: &self.bgl[4],
                    entries: &[
                        u0,
                        tv(1, &src),
                        tv(2, depth),
                        tv(5, &t.prep_far),
                        wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&self.lin) },
                        tv(12, &t.far[parity]),
                        tv(13, &t.near[parity]),
                        tv(14, &t.cov[1 - parity]),
                        tv(15, &t.cov[parity]),
                        tv(16, &t.out),
                        tv(17, &bloom_v),
                    ],
                });
                let post = look.post_bind_group(&t.out, &self.black);
                if t.binds.len() >= 8 {
                    t.binds.remove(0);
                }
                t.binds.push((st, dtt, bt, parity, prep, fin, post));
                t.binds.len() - 1
            }
        };
        let half = ((size.0 / 2).max(1), (size.1 / 2).max(1));
        let tiles = (half.0.div_ceil(8), half.1.div_ceil(8));
        let timer = self.timer.lock().unwrap();
        {
            let tw = timer.as_ref().map(|t| wgpu::ComputePassTimestampWrites { query_set: &t.set, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("photo"), timestamp_writes: tw });
            if dof_on {
                pass.set_pipeline(&self.pipes[0]);
                pass.set_bind_group(0, &t.binds[k].4, &[]);
                pass.dispatch_workgroups(half.0.div_ceil(8), half.1.div_ceil(8), 1);
                pass.set_pipeline(&self.pipes[1]);
                pass.set_bind_group(0, &t.tiles_bg, &[]);
                pass.dispatch_workgroups(tiles.0.div_ceil(8), tiles.1.div_ceil(8), 1);
                pass.set_pipeline(&self.pipes[2]);
                pass.set_bind_group(0, &t.dilate_bg, &[]);
                pass.dispatch_workgroups(tiles.0.div_ceil(8), tiles.1.div_ceil(8), 1);
                pass.set_pipeline(&self.pipes[3]);
                pass.set_bind_group(0, &t.gather_bg[parity], &[]);
                pass.dispatch_workgroups(half.0.div_ceil(8), half.1.div_ceil(8), 1);
            }
            pass.set_pipeline(&self.pipes[4]);
            pass.set_bind_group(0, &t.binds[k].5, &[]);
            pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
        }
        if let Some(tm) = timer.as_ref() {
            enc.resolve_query_set(&tm.set, 0..2, &tm.resolve, 0);
            enc.copy_buffer_to_buffer(&tm.resolve, 0, &tm.read, 0, 16);
        }
        drop(timer);
        if let Some(tm) = self.timer.lock().unwrap().as_mut() {
            tm.written = true;
        }
        let post = t.binds[k].6.clone();
        drop(tg);
        self.state.lock().unwrap().parity = 1 - parity;
        Some(post)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_50mm_lens_is_the_normal_lens_of_the_frame() {
        // 36 mm wide at 16:9: a 50 mm lens sees 39.6 degrees across and 22.9 up.
        let fov = fov_y_deg(50.0, 16.0 / 9.0);
        assert!((fov - 22.9).abs() < 0.1, "{fov}");
        assert!((fov_x_deg(50.0) - 39.6).abs() < 0.05, "{}", fov_x_deg(50.0));
        assert!((focal_mm(fov, 16.0 / 9.0) - 50.0).abs() < 1e-9);
        // the game's 52 degree lens is a wide angle (about 20.8 mm).
        let f = focal_mm(52.0, 16.0 / 9.0);
        assert!((f - 20.76).abs() < 0.05, "{f}");
    }

    #[test]
    fn the_thin_lens_blur_matches_the_depth_of_field_tables() {
        // 50 mm at f/8 with c = 0.03 mm: hyperfocal 10.47 m (f^2 / (N c) + f).
        let h = hyperfocal_m(50.0, 8.0, 0.03);
        assert!((h - 10.467).abs() < 0.01, "{h}");
        // focused at the hyperfocal distance, infinity is exactly the limit circle.
        let c = coc_inf_mm(50.0, 8.0, h);
        assert!((c - 0.03).abs() < 1e-4, "{c}");
        // and half the hyperfocal distance is the near limit (c = -0.03 mm there).
        let near = coc_mm(50.0, 8.0, h, h / 2.0);
        assert!((near + 0.03).abs() < 2e-3, "{near}");
        let (n, f) = dof_limits_m(50.0, 8.0, h, 0.03);
        assert!((n - h / 2.0).abs() < 0.02 && f.is_none(), "{n} {f:?}");
        // 85 mm f/1.8 at 4.9 m, c 0.03 mm: 4.73 to 5.08 m in focus (H = 133.9 m: near Hs / (H + s), far Hs / (H - s)).
        let (n, f) = dof_limits_m(85.0, 1.8, 4.9, 0.03);
        assert!((n - 4.730).abs() < 0.01 && (f.unwrap() - 5.083).abs() < 0.01, "{n} {f:?}");
        // in focus at the focus distance, behind positive, in front negative.
        assert_eq!(coc_mm(85.0, 1.8, 3.0, 3.0), 0.0);
        assert!(coc_mm(85.0, 1.8, 3.0, 10.0) > 0.0 && coc_mm(85.0, 1.8, 3.0, 1.5) < 0.0);
        // stopping down by a factor of two halves the blur.
        let a = coc_mm(85.0, 2.0, 3.0, 50.0);
        let b = coc_mm(85.0, 4.0, 3.0, 50.0);
        assert!((a / b - 2.0).abs() < 1e-9);
    }

    #[test]
    fn the_iris_rounds_open_and_the_cat_eye_goes_stopped_down() {
        assert_eq!(blade_roundness(WIDEST_F), 1.0);
        assert!((blade_roundness(2.0) - 0.485).abs() < 0.01, "{}", blade_roundness(2.0));
        assert_eq!(blade_roundness(2.8), 0.0);
        // a hexagon of the circle's area: circumradius sqrt(2 pi / (6 sin 60)).
        assert!((polygon_area_radius(6) - 1.0996).abs() < 1e-3);
        assert!(polygon_area_radius(9) < polygon_area_radius(6) && polygon_area_radius(9) > 1.0);
        // wide open the corner disc is clipped; from f/4 the iris sits inside the barrel's circle.
        let (o, r) = cat_eye(WIDEST_F);
        assert!(o + 1.0 > r);
        let (o, r) = cat_eye(4.0);
        assert!(o + 1.0 < r, "{o} {r}");
    }

    #[test]
    fn daylight_locus_and_white_balance() {
        // CIE 15:2004 table T.3: D65 (6504 K) at (0.3127, 0.3291), D50 (5003 K) at (0.3457, 0.3585).
        let d65 = daylight_xy(6504.0);
        assert!((d65[0] - 0.3127).abs() < 5e-4 && (d65[1] - 0.3291).abs() < 5e-4, "{d65:?}");
        let d50 = daylight_xy(5003.0);
        assert!((d50[0] - 0.3457).abs() < 5e-4 && (d50[1] - 0.3585).abs() < 5e-4, "{d50:?}");
        // no shift is the identity; a warming filter keeps white's luminance and turns it amber.
        assert_eq!(white_balance(0.0), [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        let m = white_balance(27.0);
        let w = apply(&m, [1.0, 1.0, 1.0]);
        let y = 0.2126729 * w[0] + 0.7151522 * w[1] + 0.0721750 * w[2];
        assert!((y - 1.0).abs() < 2e-3, "{w:?} {y}");
        assert!(w[0] > 1.0 && w[2] < 1.0, "{w:?}");
        let c = apply(&white_balance(-32.0), [1.0, 1.0, 1.0]);
        assert!(c[0] < 1.0 && c[2] > 1.0, "{c:?}");
        // 81B (+27 mired): 6504 K to 5532 K (1e6 / (1e6 / 6504 + 27)).
        assert!((shifted_cct(27.0) - 5532.5).abs() < 1.0, "{}", shifted_cct(27.0));
    }

    fn hue_deg(c: [f64; 3]) -> f64 {
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        let d = mx - mn;
        let h = if mx == c[0] {
            ((c[1] - c[2]) / d).rem_euclid(6.0)
        } else if mx == c[1] {
            (c[2] - c[0]) / d + 2.0
        } else {
            (c[0] - c[1]) / d + 4.0
        };
        h * 60.0
    }

    #[test]
    fn warm_film_is_a_white_balance_that_keeps_the_sky_blue() {
        // the warm grade is the 81A's shift and a touch of saturation: no contrast, no lift.
        let (mired, sat, gamma, mono) = graded(Grade::WarmFilm, 1.0, MonoFilter::None);
        assert_eq!((mired, sat, gamma, mono), (18.0, WARM_SATURATION, 1.0, None));
        let m = white_balance(mired);
        // a clear sky and the bay (linear Rec. 709) keep their hue within 10 degrees.
        for c in [[0.20, 0.38, 0.80], [0.45, 0.60, 0.85], [0.05, 0.12, 0.20]] {
            let o = apply(&m, c);
            let dh = (hue_deg(o) - hue_deg(c)).abs();
            assert!(dh < 10.0, "{c:?} -> {o:?}: {dh}");
            assert!(o[2] > o[0], "still blue: {o:?}");
        }
        // a grey card keeps its luminance (within 1 %) and warms.
        let g = apply(&m, [0.18; 3]);
        let y = 0.2126729 * g[0] + 0.7151522 * g[1] + 0.0721750 * g[2];
        assert!((y / 0.18 - 1.0).abs() < 0.01 && g[0] > g[2], "{g:?}");
        // the bay's colour (linear) keeps its hue within 6 degrees and its saturation within 15 %.
        let bay = [0.0953, 0.168, 0.231];
        let o = apply(&m, bay);
        let y = 0.2126 * o[0] + 0.7152 * o[1] + 0.0722 * o[2];
        let o = o.map(|v| y + (v - y) * sat);
        let sat_of = |c: [f64; 3]| (c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])) / c[0].max(c[1]).max(c[2]);
        assert!((hue_deg(o) - hue_deg(bay)).abs() < 6.0 && sat_of(o) > 0.85 * sat_of(bay), "{o:?}");
        // twice the strength is the 81A's shift doubled; none is neutral.
        assert_eq!(graded(Grade::WarmFilm, 2.0, MonoFilter::None).0, 36.0);
        assert_eq!(graded(Grade::Cool, 0.0, MonoFilter::None).0, 0.0);
    }

    #[test]
    fn contrast_filters_darken_a_blue_sky_and_keep_grey() {
        let sky = [0.20, 0.38, 0.80];
        let y = |w: [f64; 3], c: [f64; 3]| w[0] * c[0] + w[1] * c[1] + w[2] * c[2];
        let mut last = f64::INFINITY;
        for f in [MonoFilter::None, MonoFilter::Yellow, MonoFilter::DeepYellow, MonoFilter::Orange, MonoFilter::Red] {
            let w = f.weights();
            assert!((w.iter().sum::<f64>() - 1.0).abs() < 2e-3, "{f:?} {w:?}");
            let v = y(w, sky);
            assert!(v < last, "{f:?}: the sky darkens filter by filter");
            last = v;
        }
        // deep yellow (the default) renders the sky darker than the eye's luminance does.
        assert!(y(MonoFilter::DeepYellow.weights(), sky) < y([0.2126, 0.7152, 0.0722], sky));
        // green lightens foliage against red brick.
        let (leaf, brick) = ([0.08, 0.16, 0.04], [0.30, 0.09, 0.06]);
        assert!(y(MonoFilter::Green.weights(), leaf) > y(MonoFilter::Red.weights(), leaf));
        assert!(y(MonoFilter::Green.weights(), brick) < y(MonoFilter::Red.weights(), brick));
        for f in MONO_FILTERS {
            assert_eq!(MonoFilter::parse(f.name()), Some(f));
        }
    }

    #[test]
    fn grain_follows_selwyns_law() {
        // Tri-X at its own 48 um aperture: 0.017 density / 0.6 gamma in stops of log2 exposure.
        let s = grain_sigma_stops(&TRI_X_400, A48_SIDE_UM);
        assert!((s - 0.017 / 0.6 * std::f64::consts::LOG2_10).abs() < 1e-12);
        // half the pixel pitch, a quarter of the area: twice the sigma.
        assert!((grain_sigma_stops(&TRI_X_400, 24.0) / grain_sigma_stops(&TRI_X_400, 48.0) - 2.0).abs() < 1e-9);
        assert!(grain_sigma_stops(&T_MAX_400, 24.0) < grain_sigma_stops(&TRI_X_400, 24.0));
        // the clumped field: at the 48 um aperture it is the data sheet's sigma.
        let total = |p: f64| grain_octaves(&TRI_X_400, p).0.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!((total(A48_SIDE_UM) - s).abs() < 1e-9, "{} {s}", total(A48_SIDE_UM));
        // coarser than every clump it follows Selwyn's law exactly (sigma x pitch constant)...
        assert!((total(200.0) * 200.0 - total(100.0) * 100.0).abs() < 1e-9);
        // ...and finer pixels resolve the clumps: less than Selwyn's rise, and the cells grow in pixels.
        assert!(total(10.0) < grain_sigma_stops(&TRI_X_400, 10.0) && total(10.0) > total(20.0));
        let (_, c1) = grain_octaves(&TRI_X_400, 22.5);
        let (_, c2) = grain_octaves(&TRI_X_400, 11.25);
        assert!((c2[3] / c1[3] - 2.0).abs() < 1e-9, "a 2x capture: clumps twice as many pixels across");
    }

    #[test]
    fn grades_parse_by_name_and_label() {
        for g in GRADES {
            assert_eq!(Grade::parse(g.name()), Some(g));
            assert_eq!(Grade::parse(g.label()), Some(g));
        }
        assert_eq!(Grade::parse("sepia"), None);
    }
}
