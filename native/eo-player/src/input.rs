//! Input mapping as plain data: a port of the JS action map
//! (src/input/action-map.js). Keys are named by the browser's
//! `KeyboardEvent.code` strings (`KeyW`, `ShiftLeft`, `Space`, `ArrowUp`...),
//! which are also winit's `KeyCode` debug names, so a window front end feeds
//! `format!("{code:?}")` straight in. Mouse buttons are `Mouse0`/`Mouse2`.
//! Gamepads use the standard-mapping button indices; the stick curve,
//! deadzone, trigger threshold and look rates are the JS values.
//!
//! The actions carried are the ones this crate acts on (walk, sprint, crouch,
//! jump, aim, interact, vehicle toggle, throttle, brake, handbrake, look
//! behind, pause). Weapons, phone, aircraft and the other JS actions are not
//! ported yet.

use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    WalkForward,
    WalkBackward,
    WalkLeft,
    WalkRight,
    Sprint,
    Crouch,
    Jump,
    Aim,
    Interact,
    VehicleToggle,
    Throttle,
    Brake,
    Handbrake,
    LookBehind,
    Pause,
}

pub const ALL_ACTIONS: [Action; 15] = [
    Action::WalkForward,
    Action::WalkBackward,
    Action::WalkLeft,
    Action::WalkRight,
    Action::Sprint,
    Action::Crouch,
    Action::Jump,
    Action::Aim,
    Action::Interact,
    Action::VehicleToggle,
    Action::Throttle,
    Action::Brake,
    Action::Handbrake,
    Action::LookBehind,
    Action::Pause,
];

impl Action {
    /// The JS action name.
    pub fn js_name(self) -> &'static str {
        match self {
            Action::WalkForward => "walkForward",
            Action::WalkBackward => "walkBackward",
            Action::WalkLeft => "walkLeft",
            Action::WalkRight => "walkRight",
            Action::Sprint => "sprint",
            Action::Crouch => "crouch",
            Action::Jump => "jump",
            Action::Aim => "aim",
            Action::Interact => "interact",
            Action::VehicleToggle => "vehicleToggle",
            Action::Throttle => "throttle",
            Action::Brake => "brake",
            Action::Handbrake => "handbrake",
            Action::LookBehind => "lookBehind",
            Action::Pause => "pause",
        }
    }

    /// `DEFAULT_KEYBOARD_BINDINGS`.
    pub fn default_keys(self) -> &'static [&'static str] {
        match self {
            Action::WalkForward => &["KeyW", "ArrowUp"],
            Action::WalkBackward => &["KeyS", "ArrowDown"],
            Action::WalkLeft => &["KeyA", "ArrowLeft"],
            Action::WalkRight => &["KeyD", "ArrowRight"],
            Action::Sprint => &["ShiftLeft", "ShiftRight"],
            Action::Crouch => &["KeyC", "ControlLeft", "ControlRight"],
            Action::Jump => &["Space"],
            Action::Aim => &["Mouse2"],
            Action::Interact => &["KeyE"],
            Action::VehicleToggle => &["KeyF"],
            Action::Throttle => &["KeyW", "ArrowUp"],
            Action::Brake => &["KeyS", "ArrowDown"],
            Action::Handbrake => &["Space"],
            Action::LookBehind => &["KeyC"],
            Action::Pause => &["Escape"],
        }
    }

    /// `DEFAULT_GAMEPAD_BINDINGS` (standard mapping indices).
    pub fn default_buttons(self) -> &'static [u8] {
        use gamepad::*;
        match self {
            Action::Sprint => &[LS],
            Action::Crouch => &[RS],
            Action::Jump => &[A],
            Action::Aim => &[LT],
            Action::Interact => &[Y],
            Action::VehicleToggle => &[B],
            Action::Throttle => &[RT],
            Action::Brake => &[LT],
            Action::Handbrake => &[A],
            Action::LookBehind => &[RS],
            Action::Pause => &[START],
            _ => &[],
        }
    }
}

/// `GAMEPAD_BUTTONS`.
pub mod gamepad {
    pub const A: u8 = 0;
    pub const B: u8 = 1;
    pub const X: u8 = 2;
    pub const Y: u8 = 3;
    pub const LB: u8 = 4;
    pub const RB: u8 = 5;
    pub const LT: u8 = 6;
    pub const RT: u8 = 7;
    pub const BACK: u8 = 8;
    pub const START: u8 = 9;
    pub const LS: u8 = 10;
    pub const RS: u8 = 11;
}

/// `GAMEPAD_STICK`.
pub const STICK_DEADZONE: f64 = 0.18;
pub const STICK_CURVE: f64 = 1.8;
pub const STICK_LOOK_RATE_RAD_PER_S: f64 = 2.6;
pub const STICK_LOOK_PITCH_RATE_RAD_PER_S: f64 = 1.7;
pub const TRIGGER_THRESHOLD: f64 = 0.12;
/// `MOUSE_LOOK_BASE`.
pub const MOUSE_YAW_RAD_PER_PX: f64 = 0.00235;
pub const MOUSE_PITCH_RAD_PER_PX: f64 = 0.002;

/// `INPUT_SETTINGS_DEFAULTS`.
#[derive(Clone, Copy, Debug)]
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

/// `applyStickCurve`: radial deadzone and power response. (x, y, magnitude).
pub fn apply_stick_curve(x: f64, y: f64) -> (f64, f64, f64) {
    let m = x.hypot(y);
    if m <= STICK_DEADZONE {
        return (0.0, 0.0, 0.0);
    }
    let normalized = ((m - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).clamp(0.0, 1.0);
    let shaped = normalized.powf(STICK_CURVE);
    let scale = shaped / m;
    (x * scale, y * scale, shaped)
}

/// `applyTriggerThreshold`.
pub fn apply_trigger(v: f64) -> f64 {
    let raw = v.clamp(0.0, 1.0);
    if raw <= TRIGGER_THRESHOLD { 0.0 } else { (raw - TRIGGER_THRESHOLD) / (1.0 - TRIGGER_THRESHOLD) }
}

/// `mouseLookDelta`: positive yaw turns left (the world's yaw convention),
/// positive pitch looks down unless inverted.
pub fn mouse_look_delta(mx: f64, my: f64, s: &InputSettings, aiming: bool) -> (f64, f64) {
    let aim = if aiming { s.aim_multiplier } else { 1.0 };
    let yaw = -mx * MOUSE_YAW_RAD_PER_PX * s.look_x * aim;
    let pitch = my * MOUSE_PITCH_RAD_PER_PX * s.look_y * aim * if s.invert_y { -1.0 } else { 1.0 };
    (yaw + 0.0, pitch + 0.0)
}

/// Raw device state a front end accumulates between frames.
#[derive(Clone, Debug, Default)]
pub struct RawInput {
    /// Held keys and mouse buttons by code name.
    pub held: HashSet<String>,
    /// Mouse motion since the last frame, pixels.
    pub mouse_dx: f64,
    pub mouse_dy: f64,
    /// First gamepad, if any: axes [lx, ly, rx, ry] and analog button values.
    pub gamepad: Option<GamepadState>,
}

#[derive(Clone, Debug, Default)]
pub struct GamepadState {
    pub axes: [f64; 4],
    pub buttons: Vec<f64>,
}

/// One resolved frame (`getState()`): what the player reads.
#[derive(Clone, Debug, Default)]
pub struct InputFrame {
    pub pressed: HashSet<Action>,
    pub just_pressed: HashSet<Action>,
    /// On-foot move: x right, y forward, magnitude in [0, 1].
    pub move_x: f64,
    pub move_y: f64,
    pub move_magnitude: f64,
    /// Look delta this frame, radians (yaw positive left, pitch positive down).
    pub look_yaw: f64,
    pub look_pitch: f64,
    /// Driving: throttle and brake in [0, 1], steer positive left.
    pub throttle: f64,
    pub brake: f64,
    pub steer: f64,
}

impl InputFrame {
    pub fn held(&self, a: Action) -> bool {
        self.pressed.contains(&a)
    }
    pub fn tapped(&self, a: Action) -> bool {
        self.just_pressed.contains(&a)
    }
}

/// The action map: bindings and the edge state between frames.
#[derive(Clone, Debug, Default)]
pub struct ActionMap {
    pub settings: InputSettings,
    previous: HashSet<Action>,
}

impl ActionMap {
    /// `getState(delta, { aiming })`. Call once per drawn frame; consumes the
    /// mouse motion in `raw`.
    pub fn resolve(&mut self, raw: &mut RawInput, delta: f64, aiming: bool) -> InputFrame {
        let mut f = InputFrame::default();
        let pad = raw.gamepad.clone();
        let mut value = std::collections::HashMap::new();
        for a in ALL_ACTIONS {
            let keyboard = a.default_keys().iter().any(|k| raw.held.contains(*k));
            let mut pad_value: f64 = 0.0;
            if let Some(p) = &pad {
                for &b in a.default_buttons() {
                    let v = p.buttons.get(b as usize).copied().unwrap_or(0.0);
                    let v = if b == gamepad::LT || b == gamepad::RT { apply_trigger(v) } else if v > 0.5 { 1.0 } else { 0.0 };
                    pad_value = pad_value.max(v);
                }
            }
            if keyboard || pad_value > 0.0 {
                f.pressed.insert(a);
                if !self.previous.contains(&a) {
                    f.just_pressed.insert(a);
                }
            }
            value.insert(a, if keyboard { 1.0 } else { pad_value });
        }
        let p = |a| f.pressed.contains(&a) as i32 as f64;
        f.move_x = p(Action::WalkRight) - p(Action::WalkLeft);
        f.move_y = p(Action::WalkForward) - p(Action::WalkBackward);
        f.move_magnitude = if f.move_x != 0.0 || f.move_y != 0.0 { 1.0 } else { 0.0 };
        f.steer = p(Action::WalkLeft) - p(Action::WalkRight);
        f.throttle = if f.pressed.contains(&Action::Throttle) { value[&Action::Throttle] } else { 0.0 };
        f.brake = if f.pressed.contains(&Action::Brake) { value[&Action::Brake] } else { 0.0 };
        if let Some(pad) = &pad {
            let (lx, ly, lm) = apply_stick_curve(pad.axes[0], pad.axes[1]);
            if lm > 0.0 {
                f.move_x = lx;
                f.move_y = -ly;
                f.move_magnitude = lm;
                f.steer = -lx;
            }
            let rt = apply_trigger(pad.buttons.get(gamepad::RT as usize).copied().unwrap_or(0.0));
            let lt = apply_trigger(pad.buttons.get(gamepad::LT as usize).copied().unwrap_or(0.0));
            f.throttle = f.throttle.max(rt);
            f.brake = f.brake.max(lt);
        }
        let (my, mp) = mouse_look_delta(raw.mouse_dx, raw.mouse_dy, &self.settings, aiming);
        f.look_yaw = my;
        f.look_pitch = mp;
        if let Some(pad) = &pad {
            let (rx, ry, rm) = apply_stick_curve(pad.axes[2], pad.axes[3]);
            if rm > 0.0 {
                let aim = if aiming { self.settings.aim_multiplier } else { 1.0 };
                let step = delta.max(0.0);
                f.look_yaw += -rx * STICK_LOOK_RATE_RAD_PER_S * self.settings.look_x * aim * step;
                f.look_pitch += ry * STICK_LOOK_PITCH_RATE_RAD_PER_S * self.settings.look_y * aim * step * if self.settings.invert_y { -1.0 } else { 1.0 };
            }
        }
        raw.mouse_dx = 0.0;
        raw.mouse_dy = 0.0;
        self.previous = f.pressed.clone();
        f
    }
}
