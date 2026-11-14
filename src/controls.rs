//! The controls chart: a little controller in the bottom left corner opens a chart of every
//! control. With a controller plugged in it shows that controller's buttons (Xbox, PlayStation
//! or Nintendo style, from its name), otherwise the keys (as bound in config.toml) and the
//! mouse. A click outside the chart, Esc / Backspace (or the close / quit keys) or the
//! controller's B button closes it again.

use bevy::input::gamepad::{Gamepad, GamepadButton, GamepadButtonType, Gamepads};
use bevy::prelude::*;

use crate::config::{keys_pressed, Config};
use crate::render::make_image;

#[derive(Resource, Default)]
pub struct ControlsChart {
    pub open: bool,
    /// Closed this frame: the key or click that closed it does nothing else.
    pub just_closed: bool,
    /// What the chart shows now (None: keyboard and mouse), so it is rebuilt when a controller
    /// comes or goes.
    built: Option<Option<PadKind>>,
}

impl ControlsChart {
    /// The chart has the keys, buttons and mouse this frame.
    pub fn busy(&self) -> bool {
        self.open || self.just_closed
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PadKind {
    Xbox,
    PlayStation,
    Nintendo,
}

/// The controller icon in the corner.
#[derive(Component)]
pub struct ControlsButton;
/// The dimmed full-screen layer the chart sits on (clicks on it close the chart).
#[derive(Component)]
pub struct ControlsLayer;
/// The chart itself (clicks on it keep it open).
#[derive(Component)]
pub struct ControlsPanel;

#[derive(Resource)]
struct Glyphs {
    cross: Handle<Image>,
    circle: Handle<Image>,
    square: Handle<Image>,
    triangle: Handle<Image>,
    dpad: Handle<Image>,
    stick: Handle<Image>,
}

pub struct ControlsPlugin;

impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        // `controls_input` is ordered before the app's own input in `app::LauncherPlugin`.
        app.init_resource::<ControlsChart>().add_systems(Startup, spawn).add_systems(Update, draw.after(controls_input));
    }
}

/// A white shape on clear, from an inside test over -1..1 (y down), smoothed 4x4.
fn mask_image(inside: impl Fn(f32, f32) -> bool) -> Image {
    let n = 96u32;
    let mut rgba = Vec::with_capacity((n * n * 4) as usize);
    for py in 0..n {
        for px in 0..n {
            let mut cov = 0.0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = ((px as f32 + (sx as f32 + 0.5) / 4.0) / n as f32 - 0.5) * 2.0;
                    let y = ((py as f32 + (sy as f32 + 0.5) / 4.0) / n as f32 - 0.5) * 2.0;
                    if inside(x, y) {
                        cov += 1.0 / 16.0;
                    }
                }
            }
            rgba.extend_from_slice(&[255, 255, 255, (cov * 255.0) as u8]);
        }
    }
    make_image(n, n, rgba, false)
}

fn rounded_box(x: f32, y: f32, hw: f32, hh: f32, r: f32) -> bool {
    let qx = (x.abs() - hw + r).max(0.0);
    let qy = (y.abs() - hh + r).max(0.0);
    x.abs() <= hw && y.abs() <= hh && (qx * qx + qy * qy).sqrt() <= r
}

/// A controller seen from the front: body and grips, with the d-pad and four buttons cut out.
fn controller_image() -> Image {
    mask_image(|x, y| {
        let body = rounded_box(x, y + 0.08, 0.66, 0.3, 0.22);
        let grip = |cx: f32| ((x - cx).powi(2) / 0.07 + (y - 0.22).powi(2) / 0.11) <= 1.0;
        if !(body || grip(-0.46) || grip(0.46)) {
            return false;
        }
        let (dx, dy) = (x + 0.38, y + 0.06);
        let dpad = (dx.abs() < 0.14 && dy.abs() < 0.045) || (dy.abs() < 0.14 && dx.abs() < 0.045);
        let button = |bx: f32, by: f32| ((x - 0.38 - bx).powi(2) + (y + 0.06 - by).powi(2)).sqrt() < 0.05;
        let buttons = button(0.0, -0.1) || button(0.0, 0.1) || button(-0.1, 0.0) || button(0.1, 0.0);
        !(dpad || buttons)
    })
}

fn triangle(x: f32, y: f32) -> bool {
    y <= 0.45 && y >= -0.6 + x.abs() * 3f32.sqrt()
}

fn spawn(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(Glyphs {
        cross: images.add(mask_image(|x, y| x.abs().max(y.abs()) < 0.52 && ((x - y).abs() < 0.17 || (x + y).abs() < 0.17))),
        circle: images.add(mask_image(|x, y| (0.38..0.56).contains(&(x * x + y * y).sqrt()))),
        square: images.add(mask_image(|x, y| (0.32..0.5).contains(&x.abs().max(y.abs())))),
        // An outline: the triangle less a smaller one round the same middle.
        triangle: images.add(mask_image(|x, y| triangle(x, y - 0.05) && !triangle(x / 0.55, 0.1 + (y - 0.05 - 0.1) / 0.55))),
        dpad: images.add(mask_image(|x, y| rounded_box(x, y, 0.22, 0.7, 0.06) || rounded_box(x, y, 0.7, 0.22, 0.06))),
        stick: images.add(mask_image(|x, y| (0.58..0.78).contains(&(x * x + y * y).sqrt()))),
    });
    commands.spawn((
        ButtonBundle {
            style: Style {
                position_type: PositionType::Absolute,
                left: Val::Px(30.0),
                bottom: Val::Px(26.0),
                width: Val::Px(48.0),
                height: Val::Px(48.0),
                ..default()
            },
            image: UiImage::new(images.add(controller_image())),
            background_color: BackgroundColor(Color::NONE),
            z_index: ZIndex::Global(45),
            ..default()
        },
        ControlsButton,
    ));
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    display: Display::None,
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62)),
                z_index: ZIndex::Global(60),
                focus_policy: bevy::ui::FocusPolicy::Block,
                ..default()
            },
            Interaction::default(),
            ControlsLayer,
        ))
        .with_children(|layer| {
            layer.spawn((
                NodeBundle {
                    style: Style {
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::axes(Val::Px(34.0), Val::Px(26.0)),
                        max_width: Val::Percent(94.0),
                        max_height: Val::Percent(94.0),
                        border: UiRect::all(Val::Px(1.0)),
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    background_color: BackgroundColor(Color::srgba(0.06, 0.07, 0.09, 0.95)),
                    border_color: BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.12)),
                    border_radius: BorderRadius::all(Val::Px(14.0)),
                    focus_policy: bevy::ui::FocusPolicy::Block,
                    ..default()
                },
                Interaction::default(),
                ControlsPanel,
            ));
        });
}

/// Opens the chart from the corner icon and closes it again. Runs before the app's input, which
/// leaves everything alone while the chart is open (see `ControlsChart::busy`).
#[allow(clippy::too_many_arguments)]
pub fn controls_input(
    mut chart: ResMut<ControlsChart>,
    icon: Query<&Interaction, With<ControlsButton>>,
    panel: Query<&Interaction, With<ControlsPanel>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    gamepads: Res<Gamepads>,
    pad: Res<ButtonInput<GamepadButton>>,
    cfg: Res<Config>,
    menu: Res<crate::settings::Menu>,
) {
    chart.just_closed = false;
    if !chart.open {
        if mouse.just_pressed(MouseButton::Left) && !menu.open && icon.iter().any(|i| *i == Interaction::Pressed) {
            chart.open = true;
        }
        return;
    }
    let pad_pressed = |t: GamepadButtonType| gamepads.iter().any(|g| pad.just_pressed(GamepadButton::new(g, t)));
    let k = &cfg.keys;
    let by_key = keys.any_just_pressed([KeyCode::Escape, KeyCode::Backspace]) || keys_pressed(&k.close, &keys) || keys_pressed(&k.quit, &keys);
    let by_pad = pad_pressed(crate::app::face(&cfg, GamepadButtonType::East));
    let by_click = mouse.get_just_pressed().next().is_some() && !panel.iter().any(|i| *i != Interaction::None);
    if by_key || by_pad || by_click {
        chart.open = false;
        chart.just_closed = true;
    } else if menu.open {
        // The settings opened over it (settings key / Start): they take over.
        chart.open = false;
    }
}

/// The first plugged-in controller. Some keyboards and mice have a part that is reported as a
/// gamepad too (their media / power keys: "System Control", "Consumer Control"), and some
/// controllers have extra parts listed on their own (a touchpad, motion sensors): not these.
fn first_pad(gamepads: &Gamepads) -> Option<Gamepad> {
    gamepads.iter().find(|&g| {
        let name = gamepads.name(g).unwrap_or("").to_lowercase();
        !["system control", "consumer control", "keyboard", "mouse", "touchpad", "motion sensors"].iter().any(|w| name.contains(w))
    })
}

fn pad_kind(gamepads: &Gamepads, cfg: &Config) -> Option<PadKind> {
    let pad = first_pad(gamepads)?;
    let name = gamepads.name(pad).unwrap_or("").to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    Some(if has(&["playstation", "dualsense", "dualshock", "ps3", "ps4", "ps5", "sony", "wireless controller"]) {
        PadKind::PlayStation
    } else if has(&["nintendo", "switch", "pro controller", "joy-con"]) || cfg.general.nintendo_buttons {
        PadKind::Nintendo
    } else {
        PadKind::Xbox
    })
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    mut chart: ResMut<ControlsChart>,
    cfg: Res<Config>,
    gamepads: Res<Gamepads>,
    glyphs: Res<Glyphs>,
    mut layer: Query<&mut Style, With<ControlsLayer>>,
    panel: Query<Entity, With<ControlsPanel>>,
    mut icon: Query<(&Interaction, &mut UiImage), With<ControlsButton>>,
) {
    for (interaction, mut img) in &mut icon {
        let c = match *interaction {
            Interaction::None if !chart.open => Color::srgba(1.0, 1.0, 1.0, 0.7),
            _ => Color::WHITE,
        };
        if img.color != c {
            img.color = c;
        }
    }
    let d = if chart.open { Display::Flex } else { Display::None };
    for mut s in &mut layer {
        if s.display != d {
            s.display = d;
        }
    }
    if !chart.open {
        chart.built = None;
        return;
    }
    let kind = pad_kind(&gamepads, &cfg);
    if chart.built == Some(kind) {
        return;
    }
    chart.built = Some(kind);
    let Ok(panel) = panel.get_single() else { return };
    commands.entity(panel).despawn_descendants();
    let name = first_pad(&gamepads).and_then(|g| gamepads.name(g)).unwrap_or("Controller").to_string();
    commands.entity(panel).with_children(|p| {
        p.spawn(text("Controls", 30.0, Color::WHITE));
        let sub = match kind {
            Some(_) => format!("{name}   (unplug it to see the keyboard and mouse)"),
            None => "Keyboard and mouse   (plug in a controller to see its buttons)".to_string(),
        };
        p.spawn(text(&sub, 15.0, Color::srgba(1.0, 1.0, 1.0, 0.5)));
        p.spawn(NodeBundle {
            style: Style { column_gap: Val::Px(48.0), margin: UiRect::top(Val::Px(12.0)), align_items: AlignItems::FlexStart, ..default() },
            ..default()
        })
        .with_children(|cols| match kind {
            Some(kind) => pad_chart(cols, kind, &cfg, &glyphs),
            None => key_chart(cols, &cfg),
        });
        let close = match kind {
            Some(kind) => format!("Press {} or click anywhere to close", face_name(kind, crate::app::face(&cfg, GamepadButtonType::East))),
            None => "Press Esc or click anywhere to close".to_string(),
        };
        p.spawn(text(&close, 14.0, Color::srgba(1.0, 1.0, 1.0, 0.4))).insert(Style { margin: UiRect::top(Val::Px(14.0)), align_self: AlignSelf::Center, ..default() });
    });
}

fn text(s: &str, size: f32, color: Color) -> TextBundle {
    TextBundle::from_section(s, TextStyle { font_size: size, color, ..default() })
}

fn column(cols: &mut ChildBuilder, f: impl FnOnce(&mut ChildBuilder)) {
    cols.spawn(NodeBundle { style: Style { flex_direction: FlexDirection::Column, row_gap: Val::Px(7.0), ..default() }, ..default() }).with_children(f);
}

fn heading(c: &mut ChildBuilder, s: &str) {
    c.spawn(text(s, 14.0, Color::srgb(0.55, 0.8, 1.0))).insert(Style { margin: UiRect::new(Val::Px(0.0), Val::Px(0.0), Val::Px(8.0), Val::Px(2.0)), ..default() });
}

/// One line of the chart: the keys / buttons on the left (right-aligned), what they do beside.
fn row(c: &mut ChildBuilder, what: &str, chips: impl FnOnce(&mut ChildBuilder)) {
    c.spawn(NodeBundle { style: Style { align_items: AlignItems::Center, column_gap: Val::Px(14.0), ..default() }, ..default() }).with_children(|r| {
        r.spawn(NodeBundle {
            style: Style { width: Val::Px(190.0), justify_content: JustifyContent::FlexEnd, align_items: AlignItems::Center, column_gap: Val::Px(5.0), flex_wrap: FlexWrap::Wrap, row_gap: Val::Px(4.0), ..default() },
            ..default()
        })
        .with_children(chips);
        r.spawn(text(what, 16.0, Color::srgba(1.0, 1.0, 1.0, 0.88)));
    });
}

/// A key cap with its name.
fn key_chip(c: &mut ChildBuilder, s: &str) {
    c.spawn(NodeBundle {
        style: Style { padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)), border: UiRect::new(Val::Px(1.0), Val::Px(1.0), Val::Px(1.0), Val::Px(3.0)), min_width: Val::Px(26.0), justify_content: JustifyContent::Center, ..default() },
        background_color: BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.1)),
        border_color: BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.3)),
        border_radius: BorderRadius::all(Val::Px(5.0)),
        ..default()
    })
    .with_children(|k| {
        k.spawn(text(s, 14.0, Color::WHITE));
    });
}

/// Readable names for the keys as written in config.toml.
fn pretty(k: &str) -> String {
    k.split('+')
        .map(|p| match p {
            "Slash" => "/",
            "Escape" => "Esc",
            "PageUp" => "PgUp",
            "PageDown" => "PgDn",
            "Backspace" => "Backspace",
            "Backslash" => "\\",
            "Comma" => ",",
            "Period" => ".",
            "Minus" => "-",
            "Equal" => "=",
            "Semicolon" => ";",
            "Quote" => "'",
            "Backquote" => "`",
            "BracketLeft" => "[",
            "BracketRight" => "]",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Every binding of an action, a chip each.
fn keys(c: &mut ChildBuilder, list: &[String]) {
    for k in list {
        key_chip(c, &pretty(k));
    }
}

/// Two opposite actions (left / right), paired up: "Left / Right", "A / D".
fn key_pairs(c: &mut ChildBuilder, a: &[String], b: &[String]) {
    for i in 0..a.len().max(b.len()) {
        let s = match (a.get(i), b.get(i)) {
            (Some(x), Some(y)) => format!("{} / {}", pretty(x), pretty(y)),
            (Some(x), None) | (None, Some(x)) => pretty(x),
            _ => continue,
        };
        key_chip(c, &s);
    }
}

fn key_chart(cols: &mut ChildBuilder, cfg: &Config) {
    let k = &cfg.keys;
    column(cols, |c| {
        heading(c, "BROWSING");
        row(c, "Move left / right", |r| key_pairs(r, &k.left, &k.right));
        row(c, "Move up / down", |r| key_pairs(r, &k.up, &k.down));
        row(c, "First / last game", |r| key_pairs(r, &k.page_up, &k.page_down));
        row(c, "Open the case (tap Shift too)", |r| keys(r, &k.open));
        row(c, "Close the case", |r| keys(r, &k.close));
        row(c, "Launch", |r| keys(r, &k.launch));
        row(c, "Flip over", |r| keys(r, &k.flip));
        row(c, "Turn left / right", |r| key_pairs(r, &k.turn_left, &k.turn_right));
        row(c, "Favorite", |r| keys(r, &k.favorite));
        row(c, "Search", |r| keys(r, &k.search));
    });
    column(cols, |c| {
        heading(c, "VIEW");
        row(c, "Next / previous layout", |r| key_pairs(r, &k.next_layout, &k.prev_layout));
        row(c, "Case style", |r| keys(r, &k.next_style));
        row(c, "Background", |r| keys(r, &k.next_background));
        row(c, "Screen filters on / off", |r| keys(r, &k.toggle_filters));
        row(c, "Previous / next platform", |r| key_pairs(r, &k.prev_platform, &k.next_platform));
        row(c, "Game info", |r| keys(r, &k.toggle_info));
        row(c, "Fullscreen", |r| keys(r, &k.toggle_fullscreen));
        row(c, "Rescan for games", |r| keys(r, &k.rescan));
        row(c, "Settings", |r| keys(r, &k.settings));
        row(c, "Quit", |r| keys(r, &k.quit));
    });
    column(cols, |c| {
        heading(c, "MOUSE");
        for (m, what) in [
            ("Click", "Select"),
            ("Double-click", "Launch"),
            ("Drag", "Move through the games"),
            ("Wheel", "Scroll"),
            ("Ctrl+Wheel", "Zoom"),
            ("Right-drag", "Turn the game round"),
            ("Right-click", "Flip over"),
            ("Ctrl+Drag", "Pick up and throw a game"),
        ] {
            row(c, what, |r| key_chip(r, m));
        }
        heading(c, "OPEN CASE");
        row(c, "Look around", |r| key_chip(r, "Drag"));
        row(c, "Spin the disc / turn pages", |r| key_chip(r, "Wheel"));
    });
}

/// The label printed on a face button, by where it is (South: the bottom one).
fn face_name(kind: PadKind, b: GamepadButtonType) -> &'static str {
    use GamepadButtonType::*;
    match (kind, b) {
        (PadKind::Xbox, South) => "A",
        (PadKind::Xbox, East) => "B",
        (PadKind::Xbox, West) => "X",
        (PadKind::Xbox, North) => "Y",
        (PadKind::Nintendo, South) => "B",
        (PadKind::Nintendo, East) => "A",
        (PadKind::Nintendo, West) => "Y",
        (PadKind::Nintendo, North) => "X",
        (PadKind::PlayStation, South) => "Cross",
        (PadKind::PlayStation, East) => "Circle",
        (PadKind::PlayStation, West) => "Square",
        (PadKind::PlayStation, North) => "Triangle",
        _ => "",
    }
}

/// A round face button, in its colours: Xbox's coloured letters, Nintendo's grey ones,
/// PlayStation's coloured symbols.
fn face_chip(c: &mut ChildBuilder, kind: PadKind, b: GamepadButtonType, glyphs: &Glyphs) {
    use GamepadButtonType::*;
    let (bg, fg) = match (kind, b) {
        (PadKind::Xbox, South) => (Color::srgb(0.2, 0.62, 0.2), Color::WHITE),
        (PadKind::Xbox, East) => (Color::srgb(0.82, 0.18, 0.18), Color::WHITE),
        (PadKind::Xbox, West) => (Color::srgb(0.15, 0.42, 0.88), Color::WHITE),
        (PadKind::Xbox, North) => (Color::srgb(0.95, 0.72, 0.1), Color::srgb(0.1, 0.1, 0.1)),
        (PadKind::PlayStation, South) => (Color::srgb(0.12, 0.12, 0.14), Color::srgb(0.5, 0.65, 1.0)),
        (PadKind::PlayStation, East) => (Color::srgb(0.12, 0.12, 0.14), Color::srgb(1.0, 0.4, 0.42)),
        (PadKind::PlayStation, West) => (Color::srgb(0.12, 0.12, 0.14), Color::srgb(0.95, 0.55, 0.85)),
        (PadKind::PlayStation, North) => (Color::srgb(0.12, 0.12, 0.14), Color::srgb(0.3, 0.88, 0.68)),
        _ => (Color::srgb(0.28, 0.28, 0.3), Color::WHITE),
    };
    c.spawn(NodeBundle {
        style: Style { width: Val::Px(28.0), height: Val::Px(28.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border: UiRect::all(Val::Px(1.0)), ..default() },
        background_color: BackgroundColor(bg),
        border_color: BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.25)),
        border_radius: BorderRadius::MAX,
        ..default()
    })
    .with_children(|f| {
        if kind == PadKind::PlayStation {
            let img = match b {
                South => &glyphs.cross,
                East => &glyphs.circle,
                West => &glyphs.square,
                _ => &glyphs.triangle,
            };
            let mut ui = UiImage::new(img.clone());
            ui.color = fg;
            f.spawn(ImageBundle { style: Style { width: Val::Px(20.0), height: Val::Px(20.0), ..default() }, image: ui, ..default() });
        } else {
            f.spawn(text(face_name(kind, b), 16.0, fg));
        }
    });
}

/// A shoulder button, trigger or small button: a dark pill with its label.
fn pill(c: &mut ChildBuilder, s: &str) {
    c.spawn(NodeBundle {
        style: Style { padding: UiRect::axes(Val::Px(9.0), Val::Px(3.0)), min_width: Val::Px(34.0), justify_content: JustifyContent::Center, border: UiRect::all(Val::Px(1.0)), ..default() },
        background_color: BackgroundColor(Color::srgb(0.2, 0.2, 0.23)),
        border_color: BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.3)),
        border_radius: BorderRadius::all(Val::Px(9.0)),
        ..default()
    })
    .with_children(|k| {
        k.spawn(text(s, 14.0, Color::WHITE));
    });
}

/// An image glyph (d-pad, stick) with an optional letter over it.
fn image_chip(c: &mut ChildBuilder, img: &Handle<Image>, label: &str) {
    let mut ui = UiImage::new(img.clone());
    ui.color = Color::srgba(1.0, 1.0, 1.0, 0.85);
    c.spawn(ImageBundle {
        style: Style { width: Val::Px(30.0), height: Val::Px(30.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
        image: ui,
        ..default()
    })
    .with_children(|i| {
        if !label.is_empty() {
            i.spawn(text(label, 13.0, Color::WHITE));
        }
    });
}

fn pad_chart(cols: &mut ChildBuilder, kind: PadKind, cfg: &Config, glyphs: &Glyphs) {
    use GamepadButtonType::*;
    let f = |b| crate::app::face(cfg, b);
    let (lb, rb, lt, rt) = match kind {
        PadKind::Xbox => ("LB", "RB", "LT", "RT"),
        PadKind::PlayStation => ("L1", "R1", "L2", "R2"),
        PadKind::Nintendo => ("L", "R", "ZL", "ZR"),
    };
    let (select, start, guide, r3) = match kind {
        PadKind::Xbox => ("View", "Menu", "Xbox", "RS"),
        PadKind::PlayStation => ("Create", "Options", "PS", "R3"),
        PadKind::Nintendo => ("-", "+", "Home", "RS"),
    };
    column(cols, |c| {
        heading(c, "BROWSING");
        row(c, "Move", |r| {
            image_chip(r, &glyphs.dpad, "");
            image_chip(r, &glyphs.stick, "L");
        });
        row(c, "Jump 5 games", |r| {
            pill(r, lt);
            pill(r, rt);
        });
        row(c, "Open the case, again to launch", |r| face_chip(r, kind, f(South), glyphs));
        row(c, "Close the case", |r| face_chip(r, kind, f(East), glyphs));
        row(c, "Turn the game / look around", |r| image_chip(r, &glyphs.stick, "R"));
        row(c, "Flip over (press the stick)", |r| pill(r, r3));
        row(c, "Favorite", |r| face_chip(r, kind, f(North), glyphs));
    });
    column(cols, |c| {
        heading(c, "VIEW");
        row(c, "Previous / next layout", |r| {
            pill(r, lb);
            pill(r, rb);
        });
        row(c, "Background", |r| face_chip(r, kind, f(West), glyphs));
        row(c, "Next platform", |r| pill(r, select));
        row(c, "Settings", |r| pill(r, start));
        if cfg.general.home_button {
            row(c, "Bring the window back", |r| pill(r, guide));
        }
    });
}
