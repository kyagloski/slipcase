//! FPS counter (Settings > Screen > Show FPS): frames per second and frame time in the top left
//! corner, updated a few times a second. With SLIPCASE_FPS_LOG set, the same numbers also go to
//! the terminal once a second (for measuring without looking).

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::config::Config;

pub struct FpsPlugin;

impl Plugin for FpsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin).add_systems(Startup, spawn).add_systems(Update, update);
    }
}

#[derive(Component)]
struct FpsText;

fn spawn(mut commands: Commands) {
    commands.spawn((
        TextBundle {
            // Above the rest of the interface.
            z_index: ZIndex::Global(1000),
            ..TextBundle::from_section("", TextStyle { font_size: 18.0, color: Color::srgb(0.6, 1.0, 0.6), ..default() }).with_style(Style {
                position_type: PositionType::Absolute,
                left: Val::Px(10.0),
                top: Val::Px(8.0),
                ..default()
            })
        },
        FpsText,
    ));
}

fn update(
    cfg: Res<Config>,
    diag: Res<DiagnosticsStore>,
    time: Res<Time>,
    mut since: Local<f32>,
    mut log_since: Local<f32>,
    mut text: Query<(&mut Text, &mut Visibility), With<FpsText>>,
) {
    let fps = diag.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed());
    let ms = diag.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME).and_then(|d| d.smoothed());
    if std::env::var_os("SLIPCASE_FPS_LOG").is_some() {
        *log_since += time.delta_seconds();
        if *log_since >= 1.0 {
            *log_since = 0.0;
            eprintln!("[fps] {:.1} fps {:.2} ms", fps.unwrap_or(0.0), ms.unwrap_or(0.0));
        }
    }
    let Ok((mut t, mut vis)) = text.get_single_mut() else { return };
    let want = if cfg.general.show_fps { Visibility::Inherited } else { Visibility::Hidden };
    if *vis != want {
        *vis = want;
    }
    if !cfg.general.show_fps {
        return;
    }
    *since += time.delta_seconds();
    if *since < 0.25 {
        return;
    }
    *since = 0.0;
    if let (Some(fps), Some(ms)) = (fps, ms) {
        t.sections[0].value = format!("{fps:.0} FPS  {ms:.1} ms");
    }
}
