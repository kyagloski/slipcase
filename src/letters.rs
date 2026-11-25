//! Jump by letter on a controller: hold a bumper (LB / RB, L1 / R1, L / R) and a strip of the
//! first letters in the list shows; left / right (d-pad or stick) picks a letter and the
//! selection jumps to the first game starting with it, live; letting go closes the strip.
//! A quick tap of a bumper still changes the layout, as before.

use bevy::prelude::*;

use crate::app::{Action, Actions, ViewState};
use crate::library::Library;

/// Held this long without a direction, the strip shows anyway.
const HOLD: f32 = 0.3;

#[derive(Resource, Default)]
pub struct LetterJump {
    /// The strip is showing.
    pub open: bool,
    /// The bumper held (true: right), and for how long.
    held: Option<(bool, f32)>,
    letters: Vec<char>,
    idx: usize,
    /// Direction held, how long, and the time to its next step.
    dir: (i32, f32, f32),
    /// Seconds since the strip showed (for its fade in).
    age: f32,
    /// The strip's letters changed and its cells need building again.
    dirty: bool,
}

impl LetterJump {
    /// The controller belongs to the strip this frame (the app leaves it alone).
    pub fn busy(&self) -> bool {
        self.open || self.held.is_some()
    }
}

pub struct LettersPlugin;

impl Plugin for LettersPlugin {
    fn build(&self, app: &mut App) {
        // `letters_input` is ordered before the app's own controller input in `app::LauncherPlugin`.
        app.init_resource::<LetterJump>().add_systems(Startup, spawn).add_systems(Update, draw.after(letters_input));
    }
}

/// The letter a title is filed under: its first letter or digit (digits all under '#').
fn letter(title: &str) -> char {
    match title.chars().find(|c| c.is_alphanumeric()) {
        Some(c) if c.is_alphabetic() => c.to_uppercase().next().unwrap_or(c),
        Some(_) => '#',
        None => '#',
    }
}

#[allow(clippy::too_many_arguments)]
pub fn letters_input(
    mut lj: ResMut<LetterJump>,
    gamepads: Res<Gamepads>,
    buttons: Res<ButtonInput<GamepadButton>>,
    axes: Res<Axis<GamepadAxis>>,
    time: Res<Time>,
    mut view: ResMut<ViewState>,
    lib: Res<Library>,
    menu: Res<crate::settings::Menu>,
    chart: Res<crate::controls::ControlsChart>,
    mut out: ResMut<Actions>,
) {
    let dt = time.delta_seconds();
    if view.searching || menu.open || chart.busy() || view.view.is_empty() {
        lj.open = false;
        lj.held = None;
        return;
    }
    let pressed = |t| gamepads.iter().any(|g| buttons.pressed(GamepadButton::new(g, t)));
    let just = |t| gamepads.iter().any(|g| buttons.just_pressed(GamepadButton::new(g, t)));
    let (lb, rb) = (GamepadButtonType::LeftTrigger, GamepadButtonType::RightTrigger);

    if lj.held.is_none() {
        if just(lb) || just(rb) {
            lj.held = Some((just(rb), 0.0));
            lj.dir = (0, 0.0, 0.0);
        } else {
            return;
        }
    }
    let Some((right, t)) = lj.held else { return };
    let bumper = if right { rb } else { lb };

    // Let go: a quick tap changes the layout; after the strip showed, it just closes.
    if !pressed(bumper) {
        if !lj.open {
            out.0.push(if right { Action::NextLayout } else { Action::PrevLayout });
        }
        lj.open = false;
        lj.held = None;
        return;
    }
    lj.held = Some((right, t + dt));

    // Left / right (d-pad or stick); up / down work too, as the letter before / after.
    let mut d = 0;
    for g in gamepads.iter() {
        let b = |t| buttons.pressed(GamepadButton::new(g, t));
        if b(GamepadButtonType::DPadLeft) || b(GamepadButtonType::DPadUp) {
            d = -1;
        }
        if b(GamepadButtonType::DPadRight) || b(GamepadButtonType::DPadDown) {
            d = 1;
        }
        let x = axes.get(GamepadAxis::new(g, GamepadAxisType::LeftStickX)).unwrap_or(0.0);
        let y = axes.get(GamepadAxis::new(g, GamepadAxisType::LeftStickY)).unwrap_or(0.0);
        if d == 0 && x.abs() > 0.6 && x.abs() >= y.abs() {
            d = x.signum() as i32;
        } else if d == 0 && y.abs() > 0.6 {
            d = -(y.signum() as i32);
        }
    }

    if !lj.open && (t + dt >= HOLD || d != 0) {
        // The letters in the order the list has them, each once.
        let mut letters: Vec<char> = Vec::new();
        for &i in &view.view {
            let c = letter(&lib.games[i].title);
            if !letters.contains(&c) {
                letters.push(c);
            }
        }
        let cur = view.selected_game(&lib).map(|g| letter(&g.title));
        lj.idx = cur.and_then(|c| letters.iter().position(|&l| l == c)).unwrap_or(0);
        if lj.letters != letters {
            lj.letters = letters;
            lj.dirty = true;
        }
        lj.open = true;
        lj.age = 0.0;
        // The direction that opened it is not a step as well.
        lj.dir = (d, 0.0, 0.35);
        return;
    }
    if !lj.open {
        return;
    }
    lj.age += dt;

    // One step at once, then quicker steps the longer it is held.
    let mut steps = 0;
    if d != lj.dir.0 {
        lj.dir = (d, 0.0, 0.35);
        steps = d;
    } else if d != 0 {
        lj.dir.1 += dt;
        lj.dir.2 -= dt;
        while lj.dir.2 <= 0.0 {
            steps += d;
            lj.dir.2 += (0.12 / (1.0 + lj.dir.1)).max(0.04);
        }
    }
    if steps == 0 || lj.letters.is_empty() {
        return;
    }
    let n = lj.letters.len() as i32;
    lj.idx = (lj.idx as i32 + steps).rem_euclid(n) as usize;
    let want = lj.letters[lj.idx];
    if let Some(i) = view.view.iter().position(|&g| letter(&lib.games[g].title) == want) {
        view.select(i, &lib);
        view.grid_free = false;
    }
}

#[derive(Component)]
struct LetterStrip;
#[derive(Component)]
struct LetterRow;
#[derive(Component)]
struct LetterCell(usize);

fn spawn(mut commands: Commands) {
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    display: Display::None,
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Percent(14.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                z_index: ZIndex::Global(50),
                focus_policy: bevy::ui::FocusPolicy::Pass,
                ..default()
            },
            LetterStrip,
        ))
        .with_children(|s| {
            s.spawn((
                NodeBundle {
                    style: Style {
                        align_items: AlignItems::Center,
                        flex_wrap: FlexWrap::Wrap,
                        justify_content: JustifyContent::Center,
                        max_width: Val::Percent(90.0),
                        padding: UiRect::axes(Val::Px(18.0), Val::Px(8.0)),
                        column_gap: Val::Px(4.0),
                        ..default()
                    },
                    background_color: BackgroundColor(Color::srgba(0.03, 0.035, 0.05, 0.0)),
                    border_radius: BorderRadius::all(Val::Px(16.0)),
                    ..default()
                },
                LetterRow,
            ));
        });
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[allow(clippy::type_complexity)]
fn draw(
    mut commands: Commands,
    mut lj: ResMut<LetterJump>,
    mut strip: Query<&mut Style, (With<LetterStrip>, Without<LetterCell>)>,
    mut row: Query<(Entity, &mut BackgroundColor), (With<LetterRow>, Without<LetterCell>)>,
    mut cells: Query<(&LetterCell, &mut Text, &mut BackgroundColor), Without<LetterRow>>,
) {
    let d = if lj.open { Display::Flex } else { Display::None };
    let a = ease(lj.age / 0.15);
    for mut s in &mut strip {
        if s.display != d {
            s.display = d;
        }
        // Drops in from a little higher up.
        let top = Val::Percent(14.0 - (1.0 - a) * 2.0);
        if s.top != top {
            s.top = top;
        }
    }
    if !lj.open {
        return;
    }
    let Ok((row_e, mut row_bg)) = row.get_single_mut() else { return };
    row_bg.0 = Color::srgba(0.03, 0.035, 0.05, 0.82 * a);
    if lj.dirty {
        lj.dirty = false;
        commands.entity(row_e).despawn_descendants();
        commands.entity(row_e).with_children(|r| {
            for (i, c) in lj.letters.iter().enumerate() {
                r.spawn((
                    TextBundle::from_section(c.to_string(), TextStyle { font_size: 30.0, color: Color::NONE, ..default() }).with_style(Style {
                        min_width: Val::Px(36.0),
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                        ..default()
                    }),
                    BorderRadius::all(Val::Px(8.0)),
                    LetterCell(i),
                ));
            }
        });
        return;
    }
    for (cell, mut text, mut bg) in &mut cells {
        let on = cell.0 == lj.idx;
        let (c, size, back) = if on {
            (Color::srgba(1.0, 1.0, 1.0, a), 40.0, Color::srgba(1.0, 1.0, 1.0, 0.16 * a))
        } else {
            (Color::srgba(1.0, 1.0, 1.0, 0.45 * a), 28.0, Color::NONE)
        };
        let s = &mut text.sections[0].style;
        if s.color != c {
            s.color = c;
        }
        if s.font_size != size {
            s.font_size = size;
        }
        if bg.0 != back {
            bg.0 = back;
        }
    }
}
