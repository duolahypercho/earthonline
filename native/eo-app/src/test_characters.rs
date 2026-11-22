//! Local test player selection and animation from the normal play state.

use anyhow::{Result, ensure};
use eo_agents::GpuAgent;
use eo_agents::test_characters::{PlayerBody, Pose, Selection};
use eo_player::tuning::speed;
use eo_player::{Mode, Player, world::CollisionWorld};
use glam::Quat;

#[derive(Clone, Debug, Default)]
pub struct Options {
    avatar: Option<String>,
    preview: Option<String>,
}

impl Options {
    pub fn parse(&mut self, flag: &str, mut value: impl FnMut() -> Result<String>) -> Result<bool> {
        match flag {
            "--test-character" => self.avatar = Some(value()?),
            "--test-clip" => self.preview = Some(value()?),
            "--no-test-character" => {
                self.avatar = None;
                self.preview = None;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub fn selection(&self, agents: bool) -> Result<Option<Selection>> {
        let Some(avatar) = &self.avatar else {
            ensure!(
                self.preview.is_none(),
                "--test-clip requires --test-character"
            );
            return Ok(None);
        };
        ensure!(agents, "--test-character requires the agent renderer");
        Ok(Some(Selection::from_env(
            avatar.clone(),
            self.preview.clone(),
        )?))
    }
}

pub fn list() -> Result<()> {
    for (id, clips) in eo_agents::test_characters::catalogue()? {
        println!("{id} ({} clips)", clips.len());
        for clip in clips {
            println!("  {clip}");
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct Playback {
    pose: Pose,
    active: Option<usize>,
    time: f32,
    blend_time: f32,
    was_driving: bool,
    was_grounded: Option<bool>,
    landing: bool,
    crouched: Option<bool>,
    crouch_transition: bool,
    entry_blend_key: Option<u64>,
}

impl Playback {
    pub fn car_frame(
        &mut self,
        body: &PlayerBody,
        frame: &crate::car_access::Frame,
    ) -> (&[[f32; 4]], GpuAgent) {
        if frame.entry_blend_key.is_some() && self.entry_blend_key != frame.entry_blend_key {
            self.pose.start_blend();
        }
        self.entry_blend_key = frame.entry_blend_key;
        self.active = None;
        self.was_driving = false;
        self.was_grounded = None;
        self.crouched = None;
        self.landing = false;
        self.crouch_transition = false;
        let blend = if frame.entry_blend_key.is_some() {
            (frame.time / 0.12).min(1.0)
        } else {
            1.0
        };
        self.pose
            .sample(&body.rig, frame.clip, frame.time, false, blend);
        let mut instance = body.instance(
            frame.position.as_vec3().to_array(),
            frame.orientation.as_quat(),
        );
        instance.scale = eo_data::groundscale::vehicles() as f32;
        (&self.pose.rows, instance)
    }

    pub fn frame(
        &mut self,
        body: &PlayerBody,
        player: &Player,
        world: &CollisionWorld,
        visible: bool,
        dt: f32,
    ) -> Option<(&[[f32; 4]], GpuAgent)> {
        if !visible {
            return None;
        }
        let rig = &body.rig;
        let feet = player.character.feet();
        let yaw = Quat::from_rotation_y(player.facing_yaw as f32);
        if let Some(clip) = body.preview {
            self.time += dt;
            self.pose
                .sample(rig, clip, rig.preview_time(clip, self.time), true, 1.0);
            return Some((
                &self.pose.rows,
                body.instance(feet.as_vec3().to_array(), yaw),
            ));
        }

        let driving = matches!(player.mode, Mode::Driving(_));
        let grounded = player.character.grounded;
        let on_foot = !driving && !self.was_driving;
        let landed = on_foot && self.was_grounded == Some(false) && grounded;
        let crouched = player.character.crouched;
        let changed_stance = on_foot && self.crouched.is_some_and(|previous| previous != crouched);
        self.was_driving = driving;
        self.was_grounded = Some(grounded);
        self.crouched = Some(crouched);

        let (clip, position, orientation, keep_root, rate, once) = match player.mode {
            Mode::OnFoot => {
                let moving = player.ground_speed >= 0.15;
                if !grounded || moving {
                    self.landing = false;
                } else if landed {
                    self.landing = true;
                }
                if changed_stance {
                    self.crouch_transition = true;
                }
                if !grounded || moving || self.landing {
                    self.crouch_transition = false;
                }
                let name =
                    on_foot_clip(rig.motorcycle, player, self.landing, self.crouch_transition);
                let clip = rig.clip(name).unwrap_or_else(|| rig.idle());
                let duration = rig.seconds(clip);
                let one_shot = self.landing && name == "land-hard"
                    || self.crouch_transition
                        && matches!(name, "rifle-crouch-down" | "rifle-crouch-up");
                if one_shot && self.active == Some(clip) && self.time >= duration {
                    self.landing = false;
                    self.crouch_transition = false;
                }
                let authored_speed = rig.speed(clip);
                let rate = if authored_speed > 0.0 {
                    (player.ground_speed as f32 / authored_speed).clamp(0.45, 2.0)
                } else {
                    1.0
                };
                (clip, feet, yaw, false, rate, one_shot)
            }
            Mode::Driving(i) => {
                self.landing = false;
                self.crouch_transition = false;
                let car = player.vehicles.get(i)?;
                let steer = car.controls.steer;
                let name = if player.drive_cam.look_behind {
                    "car-look-back"
                } else if steer > car.spec.max_steer_rad * 0.25 {
                    "car-steer-l"
                } else if steer < -car.spec.max_steer_rad * 0.25 {
                    "car-steer-r"
                } else {
                    "car-drive"
                };
                let clip = rig.clip(name)?;
                let (position, orientation) = crate::car_access::sedan_frame(car, world);
                (
                    clip,
                    position,
                    orientation.as_quat(),
                    true,
                    1.0,
                    name != "car-drive",
                )
            }
        };
        if self.active != Some(clip) {
            if on_foot
                || driving
                    && self.active.is_some_and(|active| {
                        ["car-drive", "car-steer-l", "car-steer-r", "car-look-back"]
                            .into_iter()
                            .any(|name| rig.clip(name) == Some(active))
                    })
            {
                self.pose.start_blend();
                self.blend_time = 0.0;
            } else {
                self.blend_time = 0.12;
            }
            self.active = Some(clip);
            self.time = 0.0;
        }
        let duration = rig.seconds(clip);
        self.time += dt * rate;
        if !once && duration > 0.0 {
            self.time = self.time.rem_euclid(duration);
        }
        self.blend_time += dt;
        self.pose.sample(
            rig,
            clip,
            self.time.min(duration),
            keep_root,
            (self.blend_time / 0.12).min(1.0),
        );
        let mut instance = body.instance(position.as_vec3().to_array(), orientation);
        if driving {
            instance.scale = eo_data::groundscale::vehicles() as f32;
        }
        Some((&self.pose.rows, instance))
    }
}

fn on_foot_clip(
    motorcycle: bool,
    player: &Player,
    landing: bool,
    crouch_transition: bool,
) -> &'static str {
    const WALK: [&str; 8] = [
        "rifle-walk-f",
        "rifle-walk-fl",
        "rifle-walk-l",
        "rifle-walk-bl",
        "rifle-walk-b",
        "rifle-walk-br",
        "rifle-walk-r",
        "rifle-walk-fr",
    ];
    const JOG: [&str; 8] = [
        "rifle-jog-f",
        "rifle-jog-fl",
        "rifle-jog-l",
        "rifle-jog-bl",
        "rifle-jog-b",
        "rifle-jog-br",
        "rifle-jog-r",
        "rifle-jog-fr",
    ];
    const CROUCH: [&str; 8] = [
        "rifle-crouch-walk-f",
        "rifle-crouch-walk-fl",
        "rifle-crouch-walk-l",
        "rifle-crouch-walk-bl",
        "rifle-crouch-walk-b",
        "rifle-crouch-walk-br",
        "rifle-crouch-walk-r",
        "rifle-crouch-walk-fr",
    ];
    let moving = player.ground_speed >= 0.15;
    let running = player.ground_speed > (speed::WALK_MPS + speed::RUN_MPS) * 0.5;
    if motorcycle {
        return match (moving, running) {
            (false, _) => "idle",
            (true, false) => "walk",
            (true, true) => "run",
        };
    }
    if !player.character.grounded {
        return "fall";
    }
    if landing && !moving {
        return "land-hard";
    }
    let crouched = player.character.crouched;
    if crouch_transition {
        return if crouched {
            "rifle-crouch-down"
        } else {
            "rifle-crouch-up"
        };
    }
    if !moving {
        return if crouched {
            "rifle-crouch-idle"
        } else {
            "rifle-idle"
        };
    }
    let angle = player.momentum.vx.atan2(player.momentum.vz) - player.facing_yaw;
    let direction = ((angle / std::f64::consts::FRAC_PI_4).round() as i32).rem_euclid(8) as usize;
    if crouched {
        CROUCH[direction]
    } else if player.sprint > 0.5 && direction == 0 {
        "rifle-sprint"
    } else if running {
        JOG[direction]
    } else {
        WALK[direction]
    }
}
