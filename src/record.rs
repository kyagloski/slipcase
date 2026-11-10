//! Frame recorder for the README clips: with `SLIPCASE_RECORD=<folder>` set, the clock runs at a
//! steady 30 frames a second of game time and every frame is saved there as a numbered PNG (so the
//! clip is smooth however slowly the frames are written). `SLIPCASE_SCRIPT=<file>` also presses
//! keys on given frames: one `<frame> <key> [frames held]` per line (keys as in the config).
//! Off unless the variables are set.

use bevy::prelude::*;
use bevy::render::view::screenshot::ScreenshotManager;
use bevy::time::TimeUpdateStrategy;
use bevy::window::PrimaryWindow;
use std::path::PathBuf;
use std::time::Duration;

pub struct RecordPlugin;

#[derive(Resource)]
struct Recorder {
    dir: PathBuf,
    frame: u32,
    /// Key presses to come: (frame, key, frames held).
    script: Vec<(u32, KeyCode, u32)>,
    /// Keys down now, and the frame they are let go.
    held: Vec<(KeyCode, u32)>,
}

impl Plugin for RecordPlugin {
    fn build(&self, app: &mut App) {
        let Some(dir) = std::env::var_os("SLIPCASE_RECORD").map(PathBuf::from) else { return };
        let _ = std::fs::create_dir_all(&dir);
        let script = std::env::var_os("SLIPCASE_SCRIPT").and_then(|p| std::fs::read_to_string(p).ok()).map_or_else(Vec::new, |s| parse_script(&s));
        // Keeps drawing when the window is not focused, too.
        let always = bevy::winit::UpdateMode::Continuous;
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 30.0)))
            .insert_resource(bevy::winit::WinitSettings { focused_mode: always, unfocused_mode: always })
            .insert_resource(Recorder { dir, frame: 0, script, held: Vec::new() })
            .add_systems(PreUpdate, press_keys.after(bevy::input::InputSystem))
            .add_systems(Last, capture);
    }
}

fn parse_script(s: &str) -> Vec<(u32, KeyCode, u32)> {
    s.lines()
        .filter_map(|l| {
            let mut w = l.split('#').next()?.split_whitespace();
            let frame = w.next()?.parse().ok()?;
            let key = crate::config::parse_key(w.next()?)?;
            Some((frame, key, w.next().and_then(|h| h.parse().ok()).unwrap_or(2)))
        })
        .collect()
}

fn press_keys(mut rec: ResMut<Recorder>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    let f = rec.frame;
    rec.held.retain(|&(k, until)| {
        let keep = f < until;
        if !keep {
            keys.release(k);
        }
        keep
    });
    let due: Vec<(KeyCode, u32)> = rec.script.iter().filter(|(at, ..)| *at == f).map(|&(_, k, h)| (k, f + h.max(1))).collect();
    for (k, until) in due {
        keys.press(k);
        rec.held.push((k, until));
    }
}

fn capture(mut rec: ResMut<Recorder>, mut shots: ResMut<ScreenshotManager>, window: Query<Entity, With<PrimaryWindow>>) {
    let Ok(w) = window.get_single() else { return };
    let path = rec.dir.join(format!("{:05}.png", rec.frame));
    if shots
        .take_screenshot(w, move |img| {
            if let Ok(d) = img.try_into_dynamic() {
                let _ = d.to_rgb8().save(&path);
            }
        })
        .is_ok()
    {
        rec.frame += 1;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn script_lines() {
        let s = parse_script("# intro\n30 Right\n90 Space 10 # open\nbad line\n");
        assert_eq!(s, vec![(30, bevy::prelude::KeyCode::ArrowRight, 2), (90, bevy::prelude::KeyCode::Space, 10)]);
    }
    use super::parse_script;
}
