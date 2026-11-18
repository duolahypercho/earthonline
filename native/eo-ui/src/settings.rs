//! Player settings, mirroring the JS settings objects field for field and
//! persisted to one JSON file under the user's config directory.
//!
//! The file holds each JS `localStorage` entry under its JS key, with the
//! same JSON the browser stores:
//!
//! ```json
//! {
//!   "earth-online.performance.v1": { "resolutionScale": 1, "shadows": "on", ..., "crowd": "medium", ... },
//!   "earth-online:input-settings:v1": { "lookX": 1, "lookY": 1, "aimMultiplier": 0.55, "invertY": false },
//!   "earthonline.audio.volume.v1": 0.6,
//!   "earthonline.audio.mix.v1": { "sfx": 1, "music": 1, "dialogue": 1 }
//! }
//! ```
//!
//! Sources: `src/render/performance-settings.js` (graphics: presets,
//! options, labels, `DEFAULT_PEOPLE_LEVEL = 'medium'`, normalisation),
//! `src/input/action-map.js` (look sensitivity), `src/audio/volume-settings.js`
//! (master volume and mix; the master and the sound-effects mix drive
//! eo-audio, the radio and dialogue levels are kept and persisted with nothing
//! to play yet). The defaults, ranges, labels and presets were copied from
//! those modules.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PERFORMANCE_STORAGE_KEY: &str = "earth-online.performance.v1";
pub const INPUT_SETTINGS_STORAGE_KEY: &str = "earth-online:input-settings:v1";
pub const AUDIO_VOLUME_STORAGE_KEY: &str = "earthonline.audio.volume.v1";
pub const AUDIO_MIX_STORAGE_KEY: &str = "earthonline.audio.mix.v1";

pub const GRAPHICS_LEVELS: &[&str] = &["high", "medium", "low"];
pub const POST_EFFECT_LEVELS: &[&str] = &["high", "medium", "low", "off"];
pub const TOGGLE_LEVELS: &[&str] = &["on", "off"];
pub const ANTIALIAS_LEVELS: &[&str] = &["smaa", "fxaa", "off"];
pub const INTERNAL_RESOLUTION_LEVELS: &[&str] = &["native", "1440p", "1080p"];
/// `DEFAULT_PEOPLE_LEVEL`: the People row a fresh install starts with.
pub const DEFAULT_PEOPLE_LEVEL: &str = "medium";

/// `GRAPHICS_OPTIONS` with `GRAPHICS_OPTION_LABELS`, in the JS order.
pub const GRAPHICS_OPTIONS: &[(&str, &str, &[&str])] = &[
    ("shadows", "Sun shadows", TOGGLE_LEVELS),
    ("reflections", "Live reflections", TOGGLE_LEVELS),
    ("postEffects", "Post effects", POST_EFFECT_LEVELS),
    ("crowd", "People", GRAPHICS_LEVELS),
    ("vehicles", "Nearby cars", GRAPHICS_LEVELS),
    ("drawDistance", "View distance", GRAPHICS_LEVELS),
    ("antialiasing", "Anti-aliasing", ANTIALIAS_LEVELS),
    ("internalResolution", "Resolution cap", INTERNAL_RESOLUTION_LEVELS),
];

/// One graphics preset (`GRAPHICS_PRESETS`).
pub struct Preset {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub fields: PerformanceSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceSettings {
    pub resolution_scale: f64,
    pub shadows: String,
    pub reflections: String,
    pub post_effects: String,
    pub crowd: String,
    pub vehicles: String,
    pub draw_distance: String,
    pub antialiasing: String,
    pub internal_resolution: String,
    pub show_fps: bool,
}

fn preset_fields(scale: f64, v: [&str; 8]) -> PerformanceSettings {
    PerformanceSettings {
        resolution_scale: scale,
        shadows: v[0].into(),
        reflections: v[1].into(),
        post_effects: v[2].into(),
        crowd: v[3].into(),
        vehicles: v[4].into(),
        draw_distance: v[5].into(),
        antialiasing: v[6].into(),
        internal_resolution: v[7].into(),
        show_fps: false,
    }
}

pub fn presets() -> [Preset; 3] {
    [
        Preset {
            key: "high",
            label: "High",
            description: "Full city detail: sun shadows, unique nearby cars, a busy near-field street of people, long view.",
            fields: preset_fields(1.0, ["on", "on", "high", "high", "high", "high", "smaa", "native"]),
        },
        Preset {
            key: "medium",
            label: "Medium",
            description: "Fewer unique cars and people, shorter view. Shadows stay on.",
            fields: preset_fields(0.85, ["on", "on", "medium", "medium", "medium", "medium", "smaa", "1440p"]),
        },
        Preset {
            key: "low",
            label: "Low",
            description: "Playable on weaker machines: no sun shadows, fewer cars and people, short view.",
            fields: preset_fields(0.65, ["off", "off", "low", "low", "low", "low", "smaa", "1080p"]),
        },
    ]
}

pub const CUSTOM_DESCRIPTION: &str = "Custom mix. Presets fill every row; changing one row leaves the others as you set them.";

impl Default for PerformanceSettings {
    /// `DEFAULT_PERFORMANCE_SETTINGS`: High with People at `medium`.
    fn default() -> Self {
        let mut s = presets()[0].fields.clone();
        s.crowd = DEFAULT_PEOPLE_LEVEL.into();
        s.show_fps = false;
        s
    }
}

impl PerformanceSettings {
    pub fn get(&self, key: &str) -> &str {
        match key {
            "shadows" => &self.shadows,
            "reflections" => &self.reflections,
            "postEffects" => &self.post_effects,
            "crowd" => &self.crowd,
            "vehicles" => &self.vehicles,
            "drawDistance" => &self.draw_distance,
            "antialiasing" => &self.antialiasing,
            "internalResolution" => &self.internal_resolution,
            _ => "",
        }
    }
    fn slot(&mut self, key: &str) -> Option<&mut String> {
        Some(match key {
            "shadows" => &mut self.shadows,
            "reflections" => &mut self.reflections,
            "postEffects" => &mut self.post_effects,
            "crowd" => &mut self.crowd,
            "vehicles" => &mut self.vehicles,
            "drawDistance" => &mut self.draw_distance,
            "antialiasing" => &mut self.antialiasing,
            "internalResolution" => &mut self.internal_resolution,
            _ => return None,
        })
    }
    /// `performancePresetName`: the preset every gameplay field matches, or "custom".
    pub fn preset_name(&self) -> &'static str {
        for p in presets() {
            let f = &p.fields;
            if f.resolution_scale == self.resolution_scale && GRAPHICS_OPTIONS.iter().all(|(k, _, _)| f.get(k) == self.get(k)) {
                return p.key;
            }
        }
        "custom"
    }
    /// `graphicsPresetFields` applied (keeps `showFps`).
    pub fn apply_preset(&mut self, key: &str) {
        if let Some(p) = presets().into_iter().find(|p| p.key == key) {
            let fps = self.show_fps;
            *self = p.fields;
            self.show_fps = fps;
        }
    }
    /// `cycleGraphicsOption`: the next value of one row.
    pub fn cycle(&mut self, key: &str) {
        let Some((_, _, list)) = GRAPHICS_OPTIONS.iter().find(|(k, _, _)| *k == key) else { return };
        let Some(slot) = self.slot(key) else { return };
        let i = list.iter().position(|v| v == slot).unwrap_or(0);
        *slot = list[(i + 1) % list.len()].to_string();
    }
    /// `normalizePerformanceSettings` for a saved record (fields only).
    pub fn normalize(mut self) -> Self {
        let base = presets()[0].fields.clone();
        let pick = |v: &mut String, allowed: &[&str], fallback: &str| {
            if !allowed.contains(&v.as_str()) {
                *v = fallback.to_string();
            }
        };
        pick(&mut self.shadows, TOGGLE_LEVELS, &base.shadows);
        pick(&mut self.reflections, TOGGLE_LEVELS, &base.reflections);
        pick(&mut self.post_effects, POST_EFFECT_LEVELS, &base.post_effects);
        pick(&mut self.crowd, GRAPHICS_LEVELS, &base.crowd);
        pick(&mut self.vehicles, GRAPHICS_LEVELS, &base.vehicles);
        pick(&mut self.draw_distance, GRAPHICS_LEVELS, &base.draw_distance);
        pick(&mut self.antialiasing, ANTIALIAS_LEVELS, &base.antialiasing);
        pick(&mut self.internal_resolution, INTERNAL_RESOLUTION_LEVELS, &base.internal_resolution);
        self.resolution_scale = if self.resolution_scale.is_finite() {
            (self.resolution_scale.clamp(0.5, 1.0) * 100.0).round() / 100.0
        } else {
            base.resolution_scale
        };
        self
    }
}

/// `optionLabel` in src/ui/performance-settings.js.
pub fn option_label(value: &str) -> &str {
    match value {
        "on" => "On",
        "off" => "Off",
        "high" => "High",
        "medium" => "Medium",
        "low" => "Low",
        "smaa" => "SMAA",
        "fxaa" => "FXAA",
        "native" => "Native",
        other => other,
    }
}

/// `{ min, max, step }` of `INPUT_SETTINGS_RANGES`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

pub const LOOK_RANGE: Range = Range { min: 0.1, max: 3.0, step: 0.05 };
pub const AIM_RANGE: Range = Range { min: 0.2, max: 1.0, step: 0.05 };
/// `INPUT_SLIDERS` in the menus: key, label.
pub const INPUT_SLIDERS: &[(&str, &str)] = &[("lookX", "Look sensitivity X"), ("lookY", "Look sensitivity Y"), ("aimMultiplier", "Aim sensitivity")];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputSettings {
    pub look_x: f64,
    pub look_y: f64,
    pub aim_multiplier: f64,
    pub invert_y: bool,
}

impl Default for InputSettings {
    fn default() -> Self {
        Self { look_x: 1.0, look_y: 1.0, aim_multiplier: 0.55, invert_y: false }
    }
}

impl InputSettings {
    pub fn normalize(mut self) -> Self {
        let d = Self::default();
        let c = |v: f64, r: Range, f: f64| if v.is_finite() { v.clamp(r.min, r.max) } else { f };
        self.look_x = c(self.look_x, LOOK_RANGE, d.look_x);
        self.look_y = c(self.look_y, LOOK_RANGE, d.look_y);
        self.aim_multiplier = c(self.aim_multiplier, AIM_RANGE, d.aim_multiplier);
        self
    }
    pub fn value(&self, key: &str) -> f64 {
        match key {
            "lookX" => self.look_x,
            "lookY" => self.look_y,
            _ => self.aim_multiplier,
        }
    }
    pub fn range(key: &str) -> Range {
        if key == "aimMultiplier" { AIM_RANGE } else { LOOK_RANGE }
    }
    /// One slider step up or down, snapped to the range's step like an `<input type=range>`.
    pub fn step(&mut self, key: &str, dir: f64) {
        let r = Self::range(key);
        let v = self.value(key) + dir * r.step;
        let snapped = (((v - r.min) / r.step).round() * r.step + r.min).clamp(r.min, r.max);
        let snapped = (snapped * 1e6).round() / 1e6;
        match key {
            "lookX" => self.look_x = snapped,
            "lookY" => self.look_y = snapped,
            _ => self.aim_multiplier = snapped,
        }
    }
}

pub const AUDIO_VOLUME_DEFAULT: f64 = 0.6;
pub const AUDIO_VOLUME_MAX: f64 = 1.0;
pub const AUDIO_VOLUME_STEP: f64 = 0.05;
/// `AUDIO_MIX_CATEGORIES`: key, label, note.
pub const AUDIO_MIX_CATEGORIES: &[(&str, &str, &str)] = &[
    ("sfx", "Sound effects", "Engines, gunfire, impacts, footsteps and the street."),
    ("music", "Radio", "Station music in vehicles."),
    ("dialogue", "Dialogue", "Voices on the street and the phone."),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioMix {
    pub sfx: f64,
    pub music: f64,
    pub dialogue: f64,
}

impl Default for AudioMix {
    fn default() -> Self {
        Self { sfx: 1.0, music: 1.0, dialogue: 1.0 }
    }
}

impl AudioMix {
    pub fn get(&self, key: &str) -> f64 {
        match key {
            "sfx" => self.sfx,
            "music" => self.music,
            _ => self.dialogue,
        }
    }
}

/// `formatAudioVolume`.
pub fn format_audio_volume(v: f64) -> String {
    let v = v.clamp(0.0, AUDIO_VOLUME_MAX);
    if v <= 0.0 { "Muted".into() } else { format!("{}%", (v * 100.0).round() as i64) }
}

/// Everything the menus edit.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub performance: PerformanceSettings,
    pub input: InputSettings,
    pub audio_volume: f64,
    pub audio_mix: AudioMix,
    /// The title's "Background motion" row. Session-only, as in the JS.
    pub background_motion: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            performance: PerformanceSettings::default(),
            input: InputSettings::default(),
            audio_volume: AUDIO_VOLUME_DEFAULT,
            audio_mix: AudioMix::default(),
            background_motion: true,
        }
    }
}

/// `%APPDATA%\Earth Online\settings.json` on Windows, else
/// `$XDG_CONFIG_HOME/earth-online/settings.json` (or `~/.config/...`).
/// `EO_UI_SETTINGS` overrides the path.
pub fn default_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EO_UI_SETTINGS") {
        return Some(PathBuf::from(p));
    }
    if cfg!(windows) {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return Some(PathBuf::from(appdata).join("Earth Online").join("settings.json"));
        }
    }
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(x).join("earth-online").join("settings.json"));
    }
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config").join("earth-online").join("settings.json"))
}

impl Settings {
    /// Read the file; a missing or unreadable file (or entry) gives the
    /// defaults, as a fresh browser profile does.
    pub fn load(path: &Path) -> Settings {
        let mut s = Settings::default();
        let Ok(text) = std::fs::read_to_string(path) else { return s };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return s };
        if let Some(p) = v.get(PERFORMANCE_STORAGE_KEY) {
            // Missing fields take the High preset's, as `normalizePerformanceSettings` does.
            let mut merged = serde_json::to_value(&presets()[0].fields).unwrap_or_default();
            if let (Some(m), Some(src)) = (merged.as_object_mut(), p.as_object()) {
                for (k, val) in src {
                    m.insert(k.clone(), val.clone());
                }
            }
            if let Ok(perf) = serde_json::from_value::<PerformanceSettings>(merged) {
                s.performance = perf.normalize();
            }
        }
        if let Some(i) = v.get(INPUT_SETTINGS_STORAGE_KEY) {
            let mut merged = serde_json::to_value(InputSettings::default()).unwrap_or_default();
            if let (Some(m), Some(src)) = (merged.as_object_mut(), i.as_object()) {
                for (k, val) in src {
                    m.insert(k.clone(), val.clone());
                }
            }
            if let Ok(inp) = serde_json::from_value::<InputSettings>(merged) {
                s.input = inp.normalize();
            }
        }
        if let Some(vol) = v.get(AUDIO_VOLUME_STORAGE_KEY).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|t| t.parse().ok()))) {
            s.audio_volume = vol.clamp(0.0, AUDIO_VOLUME_MAX);
        }
        if let Some(m) = v.get(AUDIO_MIX_STORAGE_KEY) {
            for (key, _, _) in AUDIO_MIX_CATEGORIES {
                if let Some(level) = m.get(*key).and_then(|x| x.as_f64()) {
                    let level = level.clamp(0.0, AUDIO_VOLUME_MAX);
                    match *key {
                        "sfx" => s.audio_mix.sfx = level,
                        "music" => s.audio_mix.music = level,
                        _ => s.audio_mix.dialogue = level,
                    }
                }
            }
        }
        s
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            PERFORMANCE_STORAGE_KEY: self.performance,
            INPUT_SETTINGS_STORAGE_KEY: self.input,
            AUDIO_VOLUME_STORAGE_KEY: self.audio_volume,
            AUDIO_MIX_STORAGE_KEY: self.audio_mix,
        })
    }

    /// Write the file (through a temporary file and a rename).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.to_json()).unwrap_or_default())?;
        std::fs::rename(&tmp, path)
    }
}
