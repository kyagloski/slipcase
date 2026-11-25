//! Controller rumble (Settings > Controller): a tiny tick as the selection moves, a soft bump
//! when a case opens and a stronger one when a game launches. Only while the controller is the
//! thing being used: touching the keyboard or mouse stops it until the controller is used again.

use std::time::Duration;

use bevy::input::gamepad::{GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy::prelude::*;

use crate::app::{LaunchAnim, ViewState};
use crate::config::Config;

pub struct RumblePlugin;

impl Plugin for RumblePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, rumble);
    }
}

#[derive(Default)]
struct Seen {
    /// The controller was used more recently than the keyboard or mouse.
    pad: bool,
    selected: Option<String>,
    open: bool,
    launching: bool,
}

#[allow(clippy::too_many_arguments)]
fn rumble(
    cfg: Res<Config>,
    gamepads: Res<Gamepads>,
    buttons: Res<ButtonInput<GamepadButton>>,
    axes: Res<Axis<GamepadAxis>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    view: Res<ViewState>,
    launch: Res<LaunchAnim>,
    mut seen: Local<Seen>,
    mut out: EventWriter<GamepadRumbleRequest>,
) {
    let stick = gamepads.iter().any(|g| {
        [GamepadAxisType::LeftStickX, GamepadAxisType::LeftStickY, GamepadAxisType::RightStickX, GamepadAxisType::RightStickY]
            .into_iter()
            .any(|a| axes.get(GamepadAxis::new(g, a)).unwrap_or(0.0).abs() > 0.4)
    });
    if buttons.get_just_pressed().next().is_some() || stick {
        seen.pad = true;
    } else if keys.get_just_pressed().next().is_some() || mouse.get_just_pressed().next().is_some() {
        seen.pad = false;
    }

    let moved = view.selected_id != seen.selected && seen.selected.is_some();
    let opened = view.inspect_open && !seen.open;
    let launched = launch.game.is_some() && !seen.launching;
    seen.selected.clone_from(&view.selected_id);
    seen.open = view.inspect_open;
    seen.launching = launch.game.is_some();

    if !cfg.general.rumble || !seen.pad {
        return;
    }
    let pulse = if launched {
        Some((GamepadRumbleIntensity { strong_motor: 0.55, weak_motor: 0.45 }, 240))
    } else if opened {
        Some((GamepadRumbleIntensity { strong_motor: 0.0, weak_motor: 0.4 }, 90))
    } else if moved {
        Some((GamepadRumbleIntensity::weak_motor(0.18), 35))
    } else {
        None
    };
    let Some((intensity, ms)) = pulse else { return };
    for gamepad in gamepads.iter() {
        out.send(GamepadRumbleRequest::Add { duration: Duration::from_millis(ms), intensity, gamepad });
    }
}
