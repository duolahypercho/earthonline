//! Player settings, mirroring the JS settings objects field for field and
//! persisted to one JSON file under the user's config directory.
//!
//! The file holds each JS `localStorage` entry under its JS key, with the
//! same JSON the browser stores:
//!
//! ```json
//! {
//!   "earth-online.performance.v1": { "resolutionScale": 1, "shadows": "on", ..., "crowd": "medium", ..., "lighting": "medium" },
//!   "earth-online:input-settings:v1": { "lookX": 1, "lookY": 1, "aimMultiplier": 0.55, "invertY": false },
//!   "earthonline.audio.volume.v1": 0.6,
//!   "earthonline.audio.mix.v1": { "sfx": 1, "music": 1, "dialogue": 1 },
//!   "earth-online.world.v1": { "structures": "off" }
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
//!
//! Native additions, with no JS counterpart: the "Lighting and detail" row
//! (`lighting`: the `--quality` tiers of eo-app, `LIGHTING_LEVELS`) and its
//! "Hyper real" preset, and the world record (`earth-online.world.v1`: the
//! "Grade-separated roads" row, `--structures` / `--tunnels`), which no preset
//! touches. eo-app's `--ui` window applies them (eo-app `ui.rs`); the menus
//! here only keep them.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PERFORMANCE_STORAGE_KEY: &str = "earth-online.performance.v1";
pub const INPUT_SETTINGS_STORAGE_KEY: &str = "earth-online:input-settings:v1";
pub const AUDIO_VOLUME_STORAGE_KEY: &str = "earthonline.audio.volume.v1";
pub const AUDIO_MIX_STORAGE_KEY: &str = "earthonline.audio.mix.v1";
/// The world record (native only): what the city is built with, not how it is drawn.
pub const WORLD_STORAGE_KEY: &str = "earth-online.world.v1";

pub const GRAPHICS_LEVELS: &[&str] = &["high", "medium", "low"];
pub const POST_EFFECT_LEVELS: &[&str] = &["high", "medium", "low", "off"];
pub const TOGGLE_LEVELS: &[&str] = &["on", "off"];
pub const ANTIALIAS_LEVELS: &[&str] = &["smaa", "fxaa", "off"];
pub const INTERNAL_RESOLUTION_LEVELS: &[&str] = &["native", "1440p", "1080p"];
/// The "Lighting and detail" row: eo-app's `--quality` tiers by their own
/// names, in the order the row steps through them.
pub const LIGHTING_LEVELS: &[&str] = &["low", "medium", "high", "hyper"];
/// `DEFAULT_PEOPLE_LEVEL`: the People row a fresh install starts with.
pub const DEFAULT_PEOPLE_LEVEL: &str = "medium";
/// The tier a fresh install starts with, and a record written before the row
/// existed that matches no preset: `medium`, the game's default frame (the
/// one every record drew before), inside the 10 ms frame budget.
pub const DEFAULT_LIGHTING_LEVEL: &str = "medium";
/// The "Grade-separated roads" row: off, `--structures`, `--structures --tunnels`.
pub const STRUCTURES_LEVELS: &[&str] = &["off", "on", "tunnels"];

/// `GRAPHICS_OPTIONS` with `GRAPHICS_OPTION_LABELS`, in the JS order, after
/// the native "Lighting and detail" row.
pub const GRAPHICS_OPTIONS: &[(&str, &str, &[&str])] = &[
    ("lighting", "Lighting and detail", LIGHTING_LEVELS),
    ("shadows", "Sun shadows", TOGGLE_LEVELS),
    ("reflections", "Live reflections", TOGGLE_LEVELS),
    ("postEffects", "Post effects", POST_EFFECT_LEVELS),
    ("crowd", "People", GRAPHICS_LEVELS),
    ("vehicles", "Nearby cars", GRAPHICS_LEVELS),
    ("drawDistance", "View distance", GRAPHICS_LEVELS),
    ("antialiasing", "Anti-aliasing", ANTIALIAS_LEVELS),
    ("internalResolution", "Resolution cap", INTERNAL_RESOLUTION_LEVELS),
];

/// The world rows (native only): key, label, values. Not graphics: no preset
/// sets them and a preset's name does not depend on them.
pub const WORLD_OPTIONS: &[(&str, &str, &[&str])] = &[("structures", "Grade-separated roads", STRUCTURES_LEVELS)];

/// The note under a row, where the row needs one: what it changes and what
/// waits for a restart (eo-app `ui.rs` `live_look` keeps those at their start).
pub fn option_note(key: &str) -> Option<&'static str> {
    Some(match key {
        "lighting" => {
            "The game's quality tiers. Shadows, the sky and the camera finish change at once. City detail, weathering, street life, shop windows and any lighting beyond what was on at start take effect after restart."
        }
        "structures" => "Freeways, ramps, road bridges and, with tunnels, their bores: drawn and drivable. This changes the roads, not the graphics, so no preset sets it. Takes effect after restart.",
        _ => return None,
    })
}

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
    /// "Lighting and detail": the `--quality` tier (`LIGHTING_LEVELS`).
    pub lighting: String,
}

fn preset_fields(scale: f64, lighting: &str, v: [&str; 8]) -> PerformanceSettings {
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
        lighting: lighting.into(),
    }
}

/// The presets, most detailed first: the native "Hyper real", then the JS
/// High, Medium and Low (their rows as in the JS, each with the tier of its name).
pub fn presets() -> [Preset; 4] {
    [
        Preset {
            key: "hyper",
            label: "Hyper real",
            description: "High with the most detailed lighting and city: far shadows to the horizon, 48 shadowed lamps, finer ambient light, reflections and lit fog, more weathering and street life. For the fastest machines.",
            fields: preset_fields(1.0, "hyper", ["on", "on", "high", "high", "high", "high", "smaa", "native"]),
        },
        Preset {
            key: "high",
            label: "High",
            description: "Full city detail: sun shadows, unique nearby cars, a busy near-field street of people, long view.",
            fields: preset_fields(1.0, "high", ["on", "on", "high", "high", "high", "high", "smaa", "native"]),
        },
        Preset {
            key: "medium",
            label: "Medium",
            description: "Fewer unique cars and people, shorter view. Shadows stay on.",
            fields: preset_fields(0.85, "medium", ["on", "on", "medium", "medium", "medium", "medium", "smaa", "1440p"]),
        },
        Preset {
            key: "low",
            label: "Low",
            description: "Playable on weaker machines: no sun shadows, fewer cars and people, short view.",
            fields: preset_fields(0.65, "low", ["off", "off", "low", "low", "low", "low", "smaa", "1080p"]),
        },
    ]
}

/// The High preset's rows: what a stored record's missing or unreadable rows
/// take (`normalizePerformanceSettings`).
fn high_fields() -> PerformanceSettings {
    presets().into_iter().find(|p| p.key == "high").map(|p| p.fields).expect("the High preset")
}

pub const CUSTOM_DESCRIPTION: &str = "Custom mix. Presets fill every row; changing one row leaves the others as you set them.";

impl Default for PerformanceSettings {
    /// `DEFAULT_PERFORMANCE_SETTINGS`: High with People at `medium`, and
    /// (native) Lighting and detail at `DEFAULT_LIGHTING_LEVEL`.
    fn default() -> Self {
        let mut s = high_fields();
        s.crowd = DEFAULT_PEOPLE_LEVEL.into();
        s.show_fps = false;
        s.lighting = DEFAULT_LIGHTING_LEVEL.into();
        s
    }
}

impl PerformanceSettings {
    pub fn get(&self, key: &str) -> &str {
        match key {
            "lighting" => &self.lighting,
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
            "lighting" => &mut self.lighting,
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
    /// The Lighting and detail tier a record that has none (written before the
    /// row existed) or an unreadable one takes: the tier of the JS preset
    /// (High, Medium, Low) its other rows match, so a preset stays that preset;
    /// any other record `DEFAULT_LIGHTING_LEVEL`, the frame it drew before.
    pub fn implied_lighting(&self) -> &'static str {
        for p in presets().into_iter().filter(|p| p.key != "hyper") {
            let f = &p.fields;
            let same = f.resolution_scale == self.resolution_scale && GRAPHICS_OPTIONS.iter().filter(|(k, _, _)| *k != "lighting").all(|(k, _, _)| f.get(k) == self.get(k));
            if same {
                if let Some(l) = LIGHTING_LEVELS.iter().find(|l| **l == f.lighting) {
                    return l;
                }
            }
        }
        DEFAULT_LIGHTING_LEVEL
    }
    /// `normalizePerformanceSettings` for a saved record (fields only); an
    /// unknown Lighting and detail tier takes `implied_lighting`.
    pub fn normalize(mut self) -> Self {
        let base = high_fields();
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
        if !LIGHTING_LEVELS.contains(&self.lighting.as_str()) {
            self.lighting = self.implied_lighting().into();
        }
        self
    }
}

/// The world record (`WORLD_STORAGE_KEY`): what eo-app's `--ui` builds the
/// city with at start. No preset touches it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldSettings {
    /// "Grade-separated roads" (`STRUCTURES_LEVELS`): "off", "on"
    /// (`--structures`) or "tunnels" (`--structures --tunnels`).
    pub structures: String,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self { structures: "off".into() }
    }
}

impl WorldSettings {
    pub fn get(&self, key: &str) -> &str {
        match key {
            "structures" => &self.structures,
            _ => "",
        }
    }
    /// The next value of one world row.
    pub fn cycle(&mut self, key: &str) {
        let Some((_, _, list)) = WORLD_OPTIONS.iter().find(|(k, _, _)| *k == key) else { return };
        let slot = match key {
            "structures" => &mut self.structures,
            _ => return,
        };
        let i = list.iter().position(|v| v == slot).unwrap_or(0);
        *slot = list[(i + 1) % list.len()].to_string();
    }
    /// An unknown value is the default's (off).
    pub fn normalize(mut self) -> Self {
        if !STRUCTURES_LEVELS.contains(&self.structures.as_str()) {
            self.structures = Self::default().structures;
        }
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
        "hyper" => "Hyper real",
        "tunnels" => "On, with tunnels",
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
    /// The world rows (native only; `WORLD_STORAGE_KEY`).
    pub world: WorldSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            performance: PerformanceSettings::default(),
            input: InputSettings::default(),
            audio_volume: AUDIO_VOLUME_DEFAULT,
            audio_mix: AudioMix::default(),
            background_motion: true,
            world: WorldSettings::default(),
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

/// Where photo mode saves its pictures: `EO_PHOTOS_DIR`, else a `photos`
/// folder in the application-data folder the settings file lives in
/// (`%APPDATA%\Earth Online\photos` on Windows, else
/// `$XDG_CONFIG_HOME/earth-online/photos` or `~/.config/earth-online/photos`).
pub fn photos_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EO_PHOTOS_DIR") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if cfg!(windows) {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return Some(PathBuf::from(appdata).join("Earth Online").join("photos"));
        }
    }
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(x).join("earth-online").join("photos"));
    }
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config").join("earth-online").join("photos"))
}

impl Settings {
    /// Read the file; a missing or unreadable file (or entry) gives the
    /// defaults, as a fresh browser profile does.
    pub fn load(path: &Path) -> Settings {
        let mut s = Settings::default();
        let Ok(text) = std::fs::read_to_string(path) else { return s };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return s };
        if let Some(p) = v.get(PERFORMANCE_STORAGE_KEY) {
            // Missing fields take the High preset's, as `normalizePerformanceSettings` does;
            // a record from before the Lighting and detail row takes `implied_lighting`.
            let mut merged = serde_json::to_value(high_fields()).unwrap_or_default();
            if let (Some(m), Some(src)) = (merged.as_object_mut(), p.as_object()) {
                for (k, val) in src {
                    m.insert(k.clone(), val.clone());
                }
            }
            if let Ok(perf) = serde_json::from_value::<PerformanceSettings>(merged) {
                let mut perf = perf.normalize();
                if p.get("lighting").is_none() {
                    perf.lighting = perf.implied_lighting().into();
                }
                s.performance = perf;
            }
        }
        if let Some(w) = v.get(WORLD_STORAGE_KEY) {
            let mut merged = serde_json::to_value(WorldSettings::default()).unwrap_or_default();
            if let (Some(m), Some(src)) = (merged.as_object_mut(), w.as_object()) {
                for (k, val) in src {
                    m.insert(k.clone(), val.clone());
                }
            }
            if let Ok(world) = serde_json::from_value::<WorldSettings>(merged) {
                s.world = world.normalize();
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
            WORLD_STORAGE_KEY: self.world,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A settings file of its own under the system's temporary folder (removed first).
    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eo-ui-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("settings.json")
    }

    fn write(path: &Path, v: &serde_json::Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string_pretty(v).unwrap()).unwrap();
    }

    fn done(path: &Path) {
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    fn preset(key: &str) -> PerformanceSettings {
        presets().into_iter().find(|p| p.key == key).unwrap().fields
    }

    /// A record as the JS (and this crate before the Lighting and detail row) wrote it.
    fn without_lighting(p: &PerformanceSettings) -> serde_json::Value {
        let mut v = serde_json::to_value(p).unwrap();
        v.as_object_mut().unwrap().remove("lighting");
        v
    }

    #[test]
    fn the_js_presets_keep_their_rows_and_take_the_tier_of_their_name() {
        assert_eq!(presets().map(|p| p.key), ["hyper", "high", "medium", "low"]);
        let rows = |key: &str| {
            let f = preset(key);
            let r: Vec<String> = GRAPHICS_OPTIONS.iter().filter(|(k, _, _)| *k != "lighting").map(|(k, _, _)| f.get(k).to_string()).collect();
            (f.resolution_scale, r, f.lighting.clone(), f.show_fps)
        };
        let s = |v: [&str; 8]| v.map(String::from).to_vec();
        assert_eq!(rows("high"), (1.0, s(["on", "on", "high", "high", "high", "high", "smaa", "native"]), "high".into(), false));
        assert_eq!(rows("medium"), (0.85, s(["on", "on", "medium", "medium", "medium", "medium", "smaa", "1440p"]), "medium".into(), false));
        assert_eq!(rows("low"), (0.65, s(["off", "off", "low", "low", "low", "low", "smaa", "1080p"]), "low".into(), false));
        // the row order of the JS after the native first row.
        let keys: Vec<&str> = GRAPHICS_OPTIONS.iter().map(|(k, _, _)| *k).collect();
        assert_eq!(keys, ["lighting", "shadows", "reflections", "postEffects", "crowd", "vehicles", "drawDistance", "antialiasing", "internalResolution"]);
    }

    #[test]
    fn hyper_real_is_high_at_the_hyper_tier() {
        let hyper = preset("hyper");
        assert_eq!(hyper.lighting, "hyper");
        assert_eq!(PerformanceSettings { lighting: "high".into(), ..hyper.clone() }, preset("high"));
        // every preset is recognised as itself (the row alone tells Hyper real from High).
        for p in presets() {
            assert_eq!(p.fields.preset_name(), p.key);
        }
        let mut s = PerformanceSettings { show_fps: true, ..Default::default() };
        s.apply_preset("hyper");
        assert_eq!((s.preset_name(), s.show_fps), ("hyper", true), "the preset fills every row and keeps Show FPS");
        assert_eq!(s, PerformanceSettings { show_fps: true, ..hyper });
        s.cycle("lighting"); // hyper -> low: a custom mix
        assert_eq!((s.lighting.as_str(), s.preset_name()), ("low", "custom"));
        assert_eq!(option_label("hyper"), "Hyper real");
        assert!(presets()[0].description.len() > 40);
    }

    #[test]
    fn the_lighting_row_steps_through_the_tiers() {
        let mut s = PerformanceSettings::default();
        assert_eq!(s.lighting, "medium");
        let mut seen = vec![];
        for _ in 0..4 {
            s.cycle("lighting");
            seen.push(s.lighting.clone());
        }
        assert_eq!(seen, ["high", "hyper", "low", "medium"]);
        assert_eq!(LIGHTING_LEVELS, ["low", "medium", "high", "hyper"], "eo-app's --quality names, in order");
        assert!(option_note("lighting").unwrap().contains("after restart"));
    }

    #[test]
    fn a_fresh_install_draws_the_default_frame() {
        let s = Settings::default();
        assert_eq!(s.performance.lighting, DEFAULT_LIGHTING_LEVEL);
        assert_eq!(DEFAULT_LIGHTING_LEVEL, "medium");
        assert_eq!((s.performance.shadows.as_str(), s.world.structures.as_str()), ("on", "off"));
        // High with People at medium, as before: a custom mix.
        assert_eq!(s.performance, PerformanceSettings { crowd: "medium".into(), lighting: "medium".into(), ..preset("high") });
        assert_eq!(s.performance.preset_name(), "custom");
        let path = temp_file("missing");
        assert_eq!(Settings::load(&path), Settings::default());
        write(&path, &json!("not an object"));
        assert_eq!(Settings::load(&path), Settings::default());
        done(&path);
    }

    #[test]
    fn the_stored_file_round_trips() {
        let path = temp_file("round-trip");
        for tier in LIGHTING_LEVELS {
            for roads in STRUCTURES_LEVELS {
                let mut s = Settings::default();
                s.performance.apply_preset("medium");
                s.performance.lighting = tier.to_string();
                s.performance.show_fps = true;
                s.world.structures = roads.to_string();
                s.audio_volume = 0.35;
                s.input.look_x = 1.5;
                s.audio_mix.music = 0.25;
                s.save(&path).unwrap();
                assert_eq!(Settings::load(&path), s, "{tier} {roads}");
                let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                assert_eq!((v[PERFORMANCE_STORAGE_KEY]["lighting"].as_str(), v[WORLD_STORAGE_KEY]["structures"].as_str()), (Some(*tier), Some(*roads)));
                assert_eq!(v[PERFORMANCE_STORAGE_KEY]["crowd"].as_str(), Some("medium"), "the JS keys as before");
            }
        }
        for p in presets() {
            let mut s = Settings::default();
            s.performance.apply_preset(p.key);
            s.save(&path).unwrap();
            assert_eq!(Settings::load(&path).performance.preset_name(), p.key);
        }
        done(&path);
    }

    #[test]
    fn a_file_from_before_the_new_rows_stays_readable() {
        let path = temp_file("old");
        // a JS preset's record stays that preset, with the tier of its name.
        for key in ["high", "medium", "low"] {
            write(&path, &json!({ PERFORMANCE_STORAGE_KEY: without_lighting(&preset(key)), AUDIO_VOLUME_STORAGE_KEY: 0.4 }));
            let s = Settings::load(&path);
            assert_eq!(s.performance, preset(key), "{key}");
            assert_eq!(s.performance.preset_name(), key);
            assert_eq!((s.world.clone(), s.audio_volume), (WorldSettings::default(), 0.4), "no world record: off");
        }
        // the default mix and any other custom record: the default tier, the frame it drew before; its rows kept.
        let mut custom = Settings::default().performance;
        custom.vehicles = "low".into();
        custom.show_fps = true;
        for rec in [Settings::default().performance, custom] {
            write(&path, &json!({ PERFORMANCE_STORAGE_KEY: without_lighting(&rec) }));
            let s = Settings::load(&path);
            assert_eq!(s.performance, PerformanceSettings { lighting: DEFAULT_LIGHTING_LEVEL.into(), ..rec.clone() });
        }
        // missing rows take the High preset's (`normalizePerformanceSettings`), then the tier follows.
        write(&path, &json!({ PERFORMANCE_STORAGE_KEY: { "crowd": "high" } }));
        assert_eq!(Settings::load(&path).performance, preset("high"));
        write(&path, &json!({ PERFORMANCE_STORAGE_KEY: { "shadows": "off" } }));
        assert_eq!(Settings::load(&path).performance, PerformanceSettings { shadows: "off".into(), lighting: DEFAULT_LIGHTING_LEVEL.into(), ..preset("high") });
        // an unreadable tier or road value.
        let mut rec = serde_json::to_value(preset("low")).unwrap();
        rec["lighting"] = json!("ultra");
        write(&path, &json!({ PERFORMANCE_STORAGE_KEY: rec, WORLD_STORAGE_KEY: { "structures": "maybe" } }));
        let s = Settings::load(&path);
        assert_eq!((s.performance.lighting.as_str(), s.world.structures.as_str()), ("low", "off"));
        // a stored tier is kept even where it differs from the preset's.
        let mut rec = serde_json::to_value(preset("low")).unwrap();
        rec["lighting"] = json!("hyper");
        write(&path, &json!({ PERFORMANCE_STORAGE_KEY: rec }));
        assert_eq!(Settings::load(&path).performance.lighting, "hyper");
        done(&path);
    }

    #[test]
    fn the_world_row_is_not_graphics() {
        let mut s = Settings::default();
        let mut seen = vec![];
        for _ in 0..3 {
            s.world.cycle("structures");
            seen.push(s.world.structures.clone());
        }
        assert_eq!(seen, ["on", "tunnels", "off"]);
        s.world.structures = "tunnels".into();
        for p in presets() {
            s.performance.apply_preset(p.key);
            assert_eq!(s.world.structures, "tunnels", "no preset touches it");
            assert_eq!(s.performance.preset_name(), p.key, "and a preset's name does not depend on it");
        }
        assert!(!GRAPHICS_OPTIONS.iter().any(|(k, _, _)| *k == "structures"));
        assert_eq!(WORLD_OPTIONS.iter().map(|(k, _, l)| (*k, l.to_vec())).collect::<Vec<_>>(), [("structures", vec!["off", "on", "tunnels"])]);
        assert_eq!((option_label("off"), option_label("on"), option_label("tunnels")), ("Off", "On", "On, with tunnels"));
        assert!(option_note("structures").unwrap().contains("after restart"));
    }
}
