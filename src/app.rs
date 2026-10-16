//! The launcher application: Bevy systems for input, view management, covers,
//! animation, UI and persistence.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use bevy::core_pipeline::bloom::BloomSettings;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseButton, MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::render::camera::ClearColorConfig;
use bevy::render::view::RenderLayers;
use bevy::ui::IsDefaultUiCamera;
use bevy::window::{CompositeAlphaMode, FileDragAndDrop, PresentMode, PrimaryWindow, WindowMode};
use crossbeam_channel::Receiver;

use crate::background::{palette_from, palette_from_config, spawn_background, BgArt, BgMask, BgMaterial, BgState, MaskMaterial, MaskState};
use crate::config::{config_path, keys_held, keys_pressed, BackgroundStyle, Config, LayoutKind, RomDir};
use crate::covers::{BackRequest, BackdropRequest, CoverMsg, CoverRequest, CoverService, SpineRequest};
use crate::layouts::{self, grid_pos, LayoutCtx, GRID_ROW};
use crate::library::{launch_game, now_secs, Game, Library};
use crate::platform::Platform;
use crate::config::ItemStyle;
use crate::render::{self, apply_texture, make_image, make_placeholder, DiscMaterial, Gfx, Item, MediaKind, SpawnCtx, SpineFont};
use crate::scanner;

// ------------------------------------------------------------------------------------------------
// Resources & components
// ------------------------------------------------------------------------------------------------

#[derive(Resource)]
pub struct ViewState {
    /// Indices into `Library::games`, filtered and sorted.
    pub view: Vec<usize>,
    pub selected: usize,
    pub selected_id: Option<String>,
    pub focus: f32,
    pub filter: Option<Platform>,
    pub search: String,
    pub searching: bool,
    pub just_started_search: bool,
    /// Packed centre of every item by thickness (shelf, stack, pile).
    pub prefix: Vec<f32>,
    /// Packed start of every item (centre minus half its thickness), and the gap used.
    pub starts: Vec<f32>,
    pub pack_gap: f32,
    /// The list loops round (see `Display::loop_list`); `lap` is the packed length of one lap.
    pub looping: bool,
    pub lap: f32,
    /// Looping: items per lap. The wall and table pad the last row, so every lap is whole
    /// rows and the first game always starts a row.
    pub wrap_n: usize,
    /// The easter egg is running (see `egg`): the normal items are put away, and the camera
    /// looks at its table from here (position, target).
    pub egg: bool,
    pub egg_cam: Option<(Vec3, Vec3)>,
    /// Wall / table: the layout is in rows (set by `rebuild`).
    pub grid: bool,
    /// Wall / table: the selection is lifted out. Only picking a game (click, keys) raises
    /// it; scrolling and panning just move the view.
    pub raised: bool,
    pub params: crate::config::LayoutParams,
    pub seen_revision: u64,
    pub dirty: bool,
    pub respawn: bool,
    pub spawned: HashMap<usize, Entity>,
    /// Strong handles for covers of items that currently exist.
    pub covers: HashMap<String, (Handle<Image>, u32, u32)>,
    /// Back covers of items that currently exist.
    pub backs: HashMap<String, (Handle<Image>, u32, u32)>,
    /// Stylised spines (kept for the session; they are small).
    pub spines: HashMap<String, (Handle<Image>, u32, u32)>,
    /// Disc / cartridge label and insert art of their own (kept for the session).
    pub arts: HashMap<(String, render::ArtKind), (Handle<Image>, u32, u32)>,
    pub spin: f32,
    /// Wall: centre of the screen in grid units (smoothed) and where it is heading.
    pub grid_center: Vec2,
    pub grid_target: Vec2,
    /// Wall: the user dragged the wall away from the selection; stop re-centring on it.
    pub grid_free: bool,
    /// Camera zoom (ctrl + wheel), 1 = layout default.
    pub zoom: f32,
    /// Rotation of the selected item: yaw, pitch (smoothed) and target.
    pub turn: Vec2,
    pub turn_target: Vec2,
    last_selected: Option<String>,
    /// Item under the mouse cursor.
    pub hovered: Option<usize>,
    /// Lean of the hovered item away from the cursor, as a scaled rotation axis (item space).
    pub hover_tilt: Vec3,
    /// Direction (item space) away from the cursor, scaled by how far off-centre it is.
    pub hover_away: Vec3,
    /// The cursor is on the item's spine or opening edge.
    pub hover_spine: bool,
    /// Outward normal (item space) of the face under the cursor.
    pub hover_normal: Vec3,
    /// Case opened to look inside (game id). Stays set while it closes again.
    pub inspect: Option<String>,
    /// Opening (true) or closing (false).
    pub inspect_open: bool,
    /// 0 closed .. 1 open, linear in time (eased where it is used).
    pub inspect_t: f32,
    /// Looking around the open case: yaw / pitch from dragging, and zoom.
    pub inspect_rot: Vec2,
    /// Cursor position relative to the window centre (-1..1), for a little parallax.
    pub inspect_cursor: Vec2,
    /// What inside the open case is under the cursor.
    pub inspect_hover: InspectHover,
    /// The insert: sheets turned over (0 = closed), how many there are, and whether the
    /// cursor is over the turned ones (left of the hinge) rather than the rest.
    pub insert_flipped: usize,
    pub insert_total: usize,
    pub insert_left: bool,
    /// Closing the case: seconds the lid waits while the insert closes up first.
    pub insert_hold: f32,
    /// The insert's printed pages per game (kept for the session).
    pub pages: HashMap<String, Vec<(Handle<Image>, u32, u32)>>,
    /// Spin speed (rad/s) of the selected item: tracked while right-dragging, then it keeps
    /// spinning after a flick and slows down.
    pub spin_vel: f32,
    /// The selected item is being turned with the mouse right now.
    pub turning: bool,
    /// The current rotation came from free turning (so it snaps to front / back afterwards).
    pub free_turned: bool,
    /// The first selection (the middle game) has been made.
    centred: bool,
    /// Item that was just clicked (view index) and when, for a little bounce.
    pub bounce: Option<(usize, f32)>,
    /// The disc in the open case: angle on its hub and spin speed (rad/s, from the wheel).
    pub disc_spin: f32,
    pub disc_spin_vel: f32,
    /// How far the disc / cartridge in the open case is lifted (0..1, eased).
    pub media_lift: f32,
}

/// Parts of an open case that react to the mouse.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum InspectHover {
    #[default]
    None,
    Media,
    Booklet,
}

/// How far the case has come towards the camera / swung open for an `inspect_t`.
fn inspect_approach(t: f32) -> f32 {
    smooth(t / 0.45)
}
fn inspect_opened(t: f32) -> f32 {
    smooth((t - 0.2) / 0.8)
}

impl ViewState {
    pub fn new(cfg: &Config) -> Self {
        ViewState {
            view: vec![],
            selected: 0,
            selected_id: None,
            focus: 0.0,
            filter: None,
            search: String::new(),
            searching: false,
            just_started_search: false,
            prefix: vec![],
            starts: vec![],
            pack_gap: 0.0,
            looping: false,
            lap: 0.0,
            wrap_n: 0,
            egg: false,
            egg_cam: None,
            grid: false,
            raised: false,
            params: cfg.layout_params(cfg.display.layout),
            seen_revision: 0,
            dirty: true,
            respawn: true,
            spawned: HashMap::new(),
            covers: HashMap::new(),
            backs: HashMap::new(),
            spines: HashMap::new(),
            arts: HashMap::new(),
            spin: 0.0,
            grid_center: Vec2::ZERO,
            grid_target: Vec2::ZERO,
            grid_free: false,
            zoom: cfg.display.zoom.clamp(0.45, 4.0),
            turn: Vec2::ZERO,
            turn_target: Vec2::ZERO,
            last_selected: None,
            hovered: None,
            hover_tilt: Vec3::ZERO,
            hover_away: Vec3::ZERO,
            hover_spine: false,
            hover_normal: Vec3::ZERO,
            inspect: None,
            inspect_open: false,
            inspect_t: 0.0,
            inspect_rot: Vec2::ZERO,
            inspect_cursor: Vec2::ZERO,
            inspect_hover: InspectHover::None,
            insert_flipped: 0,
            insert_total: 0,
            insert_left: false,
            insert_hold: 0.0,
            pages: HashMap::new(),
            spin_vel: 0.0,
            turning: false,
            free_turned: false,
            centred: false,
            bounce: None,
            disc_spin: 0.0,
            disc_spin_vel: 0.0,
            media_lift: 0.0,
        }
    }

    pub fn selected_game<'a>(&self, lib: &'a Library) -> Option<&'a Game> {
        self.current(lib)
    }

    fn current<'a>(&self, lib: &'a Library) -> Option<&'a Game> {
        self.view.get(self.selected).and_then(|&i| lib.games.get(i))
    }

    fn move_by(&mut self, delta: i32, lib: &Library) {
        if self.view.is_empty() {
            return;
        }
        let n = self.view.len() as i32;
        let i = if self.looping { (self.selected as i32 + delta).rem_euclid(n) } else { (self.selected as i32 + delta).clamp(0, n - 1) };
        self.select(i as usize, lib);
        self.grid_free = false;
    }

    /// Where item `i` sits relative to the focus, in items: looping, the copy of it nearest the
    /// focus (the list repeats every `view.len()` items).
    pub fn rel(&self, i: usize) -> f32 {
        let d = i as f32 - self.focus;
        if self.looping {
            let n = self.wrap_n as f32;
            d - n * (d / n).round()
        } else {
            d
        }
    }

    /// Steps from item `from` to item `i`, the short way round when looping.
    pub fn steps(&self, from: usize, i: usize) -> usize {
        let d = i.abs_diff(from);
        if self.looping { d.min(self.wrap_n.saturating_sub(d)) } else { d }
    }

    /// Wall / table: put the focus on the copy of item `i` in the rows on screen (looping, the
    /// list repeats every lap), so centring on it does not scroll through whole laps.
    pub fn focus_visible(&mut self, i: usize) {
        if !self.looping {
            self.focus = i as f32;
            return;
        }
        let cols = self.params.columns.max(1);
        let lap_rows = (self.wrap_n / cols).max(1) as f32;
        let middle_row = -self.grid_center.y / GRID_ROW;
        let k = ((middle_row - (i / cols) as f32) / lap_rows).round();
        self.focus = i as f32 + k * self.wrap_n as f32;
    }

    /// The item the loaded window is centred on: the selection, or on a wall panned away from
    /// it, the item in the selection's column in the row in the middle of the screen.
    pub fn anchor(&self) -> usize {
        let n = self.view.len();
        if !self.grid || !self.grid_free || n == 0 {
            return self.selected;
        }
        let cols = self.params.columns.max(1);
        let row = (-self.grid_target.y / GRID_ROW).round() as i64;
        let rows = (self.wrap_n.max(n) / cols).max(1) as i64;
        let row = if self.looping { row.rem_euclid(rows) } else { row.clamp(0, rows - 1) } as usize;
        (row * cols + self.selected % cols).min(n - 1)
    }

    /// The case is open (or opening) for a look inside.
    pub fn inspecting(&self) -> bool {
        self.inspect.is_some() && self.inspect_open
    }

    /// After a free rotation, settle on whichever of the front or back is nearer.
    /// Only after a free rotation (drag, flick, stick): quarter turns from Z / C stay put.
    fn snap_turn(&mut self) {
        if !self.free_turned {
            return;
        }
        self.free_turned = false;
        let pi = std::f32::consts::PI;
        self.turn_target.x = (self.turn_target.x / pi).round() * pi;
        self.turn_target.y = 0.0;
    }

    pub(crate) fn select(&mut self, i: usize, lib: &Library) {
        if self.view.is_empty() {
            return;
        }
        self.selected = i.min(self.view.len() - 1);
        self.selected_id = self.current(lib).map(|g| g.id.clone());
        self.raised = true;
    }

    fn rebuild(&mut self, lib: &Library, cfg: &Config) {
        let q = self.search.to_lowercase();
        let mut v: Vec<usize> = lib
            .games
            .iter()
            .enumerate()
            .filter(|(_, g)| {
                !g.hidden
                    && (g.installed || cfg.display.show_uninstalled)
                    && self.filter.map_or(true, |p| g.platform == p)
                    && (!cfg.display.only_favorites || g.favorite)
                    && (q.is_empty() || g.title.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect();
        match cfg.display.sort.as_str() {
            "recent" => v.sort_by_key(|&i| std::cmp::Reverse(lib.games[i].last_played)),
            "most_played" => v.sort_by_key(|&i| std::cmp::Reverse(lib.games[i].play_count)),
            "platform" => v.sort_by_key(|&i| (lib.games[i].platform, lib.games[i].title.to_lowercase())),
            _ => v.sort_by_key(|&i| lib.games[i].title.to_lowercase()),
        }
        if v != self.view {
            self.respawn = true;
        }
        let pos = self.selected_id.as_ref().and_then(|id| v.iter().position(|&i| &lib.games[i].id == id));
        self.view = v;
        self.selected = pos.unwrap_or_else(|| self.selected.min(self.view.len().saturating_sub(1)));
        // Start on the game selected last time (or in the middle if it is gone), once, as soon
        // as there are games, with the view already there rather than sweeping over.
        let first = !self.centred && !self.view.is_empty();
        if first {
            self.centred = true;
            let last = (!cfg.general.last_game.is_empty()).then(|| self.view.iter().position(|&i| lib.games[i].id == cfg.general.last_game)).flatten();
            self.selected = last.unwrap_or(self.view.len() / 2);
            self.focus = self.selected as f32;
        }
        self.selected_id = self.current(lib).map(|g| g.id.clone());
        self.params = cfg.layout_params(cfg.display.layout);
        if first {
            let cols = self.params.columns.max(1);
            let at = Vec2::new(grid_centre_x(cols, self.view.len()), grid_pos(self.selected, cols).y);
            self.grid_target = at;
            self.grid_center = at;
        }

        // Items packed side by side (bookshelf) or on top of each other (stack, pile),
        // each taking up its own thickness, so mixed case sizes never overlap.
        let gap = self.params.spacing * cfg.display.spacing_scale;
        let mut x = 0.0;
        self.prefix.clear();
        self.starts.clear();
        self.pack_gap = gap;
        for &i in &self.view {
            let d = render::item_dims(cfg, lib.games[i].platform, cfg.display.item_style);
            let thick = d.z.max(0.35) * cfg.display.item_scale;
            self.prefix.push(x + thick * 0.5);
            self.starts.push(x);
            x += thick + gap;
        }
        self.lap = x;
        // Looping needs a few games to go round.
        self.looping = cfg.display.loop_list && self.view.len() >= 3;
        let n = self.view.len();
        let cols = self.params.columns.max(1);
        self.grid = cfg.display.layout.is_grid();
        self.wrap_n = if self.grid { n.div_ceil(cols) * cols } else { n };
        self.dirty = false;
    }
}

#[derive(Resource, Default)]
pub struct Toast {
    pub text: String,
    pub ttl: f32,
}

impl Toast {
    pub fn show(&mut self, s: impl Into<String>) {
        self.text = s.into();
        self.ttl = 3.5;
    }
}

#[derive(Resource, Default)]
pub struct Persist {
    pub lib: bool,
    pub cfg: bool,
    pub timer: f32,
}

impl Persist {
    pub fn mark_lib(&mut self) {
        self.lib = true;
        self.timer = 1.5;
    }
    pub fn mark_cfg(&mut self) {
        self.cfg = true;
        self.timer = 1.5;
    }
}

#[derive(Resource, Default)]
pub struct Scanner {
    pub rx: Option<Receiver<Vec<Game>>>,
}

#[derive(Resource, Default)]
pub struct ConfigWatch {
    pub mtime: Option<std::time::SystemTime>,
    pub timer: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Left,
    Right,
    Up,
    Down,
    Launch,
    NextLayout,
    PrevLayout,
    NextStyle,
    NextBackground,
    NextPlatform,
    PrevPlatform,
    Favorite,
    Search,
    Rescan,
    ToggleInfo,
    Fullscreen,
    Home,
    End,
    Quit,
    Flip,
    TurnLeft,
    TurnRight,
    ZoomIn,
    ZoomOut,
    /// Open the selected case to look inside; launch if it is already open.
    Open,
    /// Close the open case.
    CloseCase,
}

#[derive(Resource, Default)]
pub struct Actions(pub Vec<Action>);

#[derive(Component)]
pub struct MainCamera;
#[derive(Component)]
struct KeyLight;
#[derive(Component)]
struct RimLight;
#[derive(Component)]
struct ShelfDecor;

#[derive(Component)]
struct GearButton;
#[derive(Component)]
struct UiTop;
#[derive(Component)]
struct UiTitle;
#[derive(Component)]
struct UiSub;
#[derive(Component)]
struct UiDesc;
#[derive(Component)]
struct UiHints;
#[derive(Component)]
struct UiSearch;
#[derive(Component)]
struct UiToast;

// ------------------------------------------------------------------------------------------------
// Plugin
// ------------------------------------------------------------------------------------------------

/// Running as a borderless, see-through overlay (fixed at startup).
#[derive(Resource, Clone, Copy)]
pub struct Overlay(pub bool);

/// The background layer's 2D camera.
#[derive(Component)]
pub struct BgCamera;

/// Halfway overlay can be switched while running: the background camera turns on and the
/// scene camera stops clearing over it.
/// Which passes run, following the settings: the background layer (in the overlay only when
/// halfway; off in potato mode, where the 3D camera clears to the background's dark colour
/// instead), HDR with bloom, and MSAA.
#[allow(clippy::too_many_arguments)]
fn apply_cameras(
    mut commands: Commands,
    cfg: Res<Config>,
    overlay: Res<Overlay>,
    bg: Res<BgState>,
    mut msaa: ResMut<Msaa>,
    mut bg_cam: Query<&mut Camera, (With<BgCamera>, Without<MainCamera>)>,
    mut main: Query<(Entity, &mut Camera, Has<BloomSettings>), (With<MainCamera>, Without<BgCamera>)>,
) {
    let potato = cfg.potato();
    let bg_on = !potato && (!overlay.0 || cfg.background.overlay_halfway);
    // Bloom adds light without alpha, which would vanish over the desktop.
    let hdr = cfg.bloom() && !overlay.0;
    for mut c in &mut bg_cam {
        if c.is_active != bg_on {
            c.is_active = bg_on;
        }
        if c.hdr != hdr {
            c.hdr = hdr;
        }
    }
    let clear = if bg_on {
        ClearColorConfig::None
    } else if overlay.0 {
        ClearColorConfig::Custom(Color::NONE)
    } else {
        let c = bg.current[0];
        ClearColorConfig::Custom(Color::linear_rgb(c.x, c.y, c.z))
    };
    for (e, mut c, bloom) in &mut main {
        let same = match (&c.clear_color, &clear) {
            (ClearColorConfig::None, ClearColorConfig::None) => true,
            (ClearColorConfig::Custom(a), ClearColorConfig::Custom(b)) => a == b,
            _ => false,
        };
        if !same {
            c.clear_color = clear;
        }
        if c.hdr != hdr {
            c.hdr = hdr;
        }
        if hdr && !bloom {
            commands.entity(e).insert(BloomSettings { intensity: 0.10, ..BloomSettings::NATURAL });
        } else if !hdr && bloom {
            commands.entity(e).remove::<BloomSettings>();
        }
    }
    let want = msaa_from(cfg.msaa_samples());
    if *msaa != want {
        *msaa = want;
    }
}

/// Surface alpha mode for the overlay. wgpu's own "auto" always picks an opaque surface,
/// so a mode the platform's compositor blends has to be asked for by name.
pub fn overlay_alpha(name: &str) -> CompositeAlphaMode {
    match name.trim().to_ascii_lowercase().as_str() {
        "premultiplied" => CompositeAlphaMode::PreMultiplied,
        "postmultiplied" => CompositeAlphaMode::PostMultiplied,
        "inherit" => CompositeAlphaMode::Inherit,
        "opaque" => CompositeAlphaMode::Opaque,
        _ if cfg!(target_os = "macos") => CompositeAlphaMode::PostMultiplied,
        _ if cfg!(target_os = "windows") => CompositeAlphaMode::Auto,
        _ => CompositeAlphaMode::PreMultiplied,
    }
}

pub struct LauncherPlugin;

impl Plugin for LauncherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Actions>()
            .init_resource::<Toast>()
            .init_resource::<Persist>()
            .init_resource::<Scanner>()
            .init_resource::<Gfx>()
            .init_resource::<LaunchAnim>()
            .init_resource::<CoverFades>()
            .init_resource::<LaunchBoxJob>()
            .insert_resource(ConfigWatch { mtime: mtime_of(&config_path()), timer: 0.0 })
            .add_systems(Startup, setup)
            .add_systems(Last, save_on_exit)
            .add_systems(Update, apply_cameras)
            .add_systems(Update, place_window)
            .add_systems(Update, apply_covers.after(apply_spines).before(animate_items))
            .add_systems(Update, crate::egg::egg_input.after(gather_mouse).after(gather_keyboard).after(gather_gamepad).before(handle_actions))
            .add_systems(Update, crate::egg::egg_run.after(receive_covers).before(animate_items))
            .add_systems(Update, crate::egg::egg_drag.after(crate::egg::egg_run))
            .add_systems(
                Update,
                (
                    (
                        gather_keyboard,
                        gather_gamepad,
                        gather_mouse,
                        heart_button,
                        gear_button,
                        hover_glow,
                        update_outlines,
                        handle_actions,
                        run_launch,
                        request_backs,
                        search_typing,
                        handle_dropped_files,
                        receive_scan,
                        rebuild_view,
                        sync_items,
                        receive_covers,
                        apply_spines,
                        request_extras,
                        update_backdrop,
                        launchbox_update,
                    )
                        .chain(),
                    (animate_items, remember_zoom, run_cover_fades, apply_fades, animate_rigs, animate_camera, adapt_background, update_ui, config_watch, autosave).chain(),
                ),
            );
    }
}

/// A few frames after start, once the window exists on the screen it opened on (the one with
/// the mouse), go fullscreen there or maximise the overlay.
fn place_window(mut windows: Query<&mut Window, With<PrimaryWindow>>, cfg: Res<Config>, overlay: Res<Overlay>, mut frames: Local<u32>) {
    if *frames > 3 {
        return;
    }
    *frames += 1;
    if *frames == 3 {
        if let Ok(mut w) = windows.get_single_mut() {
            apply_window(&mut w, &cfg);
            if overlay.0 {
                w.set_maximized(true);
            }
        }
    }
}

fn mtime_of(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn msaa_from(n: u32) -> Msaa {
    match n {
        0 | 1 => Msaa::Off,
        2 => Msaa::Sample2,
        8 => Msaa::Sample8,
        _ => Msaa::Sample4,
    }
}

pub fn apply_window(w: &mut Window, cfg: &Config) {
    // The overlay always covers the desktop as a maximised borderless window instead.
    let mode = if cfg.general.fullscreen && !w.transparent { WindowMode::BorderlessFullscreen } else { WindowMode::Windowed };
    if w.mode != mode {
        w.mode = mode;
    }
    let pm = if cfg.general.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
    if w.present_mode != pm {
        w.present_mode = pm;
    }
}

pub fn start_scan(scanner: &mut Scanner, cfg: &Config) {
    if scanner.rx.is_some() {
        return;
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    let data = cfg.data.clone();
    std::thread::spawn(move || {
        let _ = tx.send(scanner::scan_all(&Config { data }));
    });
    scanner.rx = Some(rx);
}

// ------------------------------------------------------------------------------------------------
// Setup
// ------------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut bg_mats: ResMut<Assets<BgMaterial>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    cfg: Res<Config>,
    lib: Res<Library>,
    mut scanner: ResMut<Scanner>,
    overlay: Res<Overlay>,
    mut mask_mats: ResMut<Assets<MaskMaterial>>,
) {
    let overlay = overlay.0;
    if cfg.general.scan_on_start || lib.games.is_empty() {
        start_scan(&mut scanner, &cfg);
    }

    // Bloom adds light without alpha, which would vanish over the desktop.
    let hdr = cfg.bloom() && !overlay;
    // Background: own camera + layer, rendered first.
    let bg = spawn_background(&mut commands, &mut meshes, &mut bg_mats, &cfg, RenderLayers::layer(1));
    commands.insert_resource(bg);
    // Halfway overlay: fades the background layer out towards the screen edges (drawn last).
    let mask = mask_mats.add(MaskMaterial { params: Vec4::new(16.0 / 9.0, 0.75, 0.0, 0.0) });
    commands.spawn((
        bevy::sprite::MaterialMesh2dBundle {
            mesh: bevy::sprite::Mesh2dHandle(meshes.add(Rectangle::new(1.0, 1.0))),
            material: mask.clone(),
            transform: Transform::from_xyz(0.0, 0.0, 50.0).with_scale(Vec3::new(1600.0, 900.0, 1.0)),
            visibility: Visibility::Hidden,
            ..default()
        },
        RenderLayers::layer(1),
        BgMask,
    ));
    commands.insert_resource(MaskState(mask));
    let halfway = overlay && cfg.background.overlay_halfway;
    commands.spawn((
        Camera2dBundle {
            camera: Camera {
                order: -1,
                hdr,
                is_active: !overlay || halfway,
                clear_color: if overlay { ClearColorConfig::Custom(Color::NONE) } else { ClearColorConfig::Default },
                ..default()
            },
            tonemapping: Tonemapping::None,
            ..default()
        },
        RenderLayers::layer(1),
        BgCamera,
    ));

    // 3D camera.
    let (cam_pos, look) = layouts::camera_for(&cfg.layout_params(cfg.display.layout));
    let mut cam = commands.spawn((
        Camera3dBundle {
            camera: Camera {
                order: 0,
                hdr,
                // The overlay has no background pass (unless halfway), so this camera clears
                // to see-through.
                clear_color: if overlay && !halfway { ClearColorConfig::Custom(Color::NONE) } else { ClearColorConfig::None },
                ..default()
            },
            projection: Projection::Perspective(PerspectiveProjection { fov: cfg.display.field_of_view.to_radians(), ..default() }),
            transform: Transform::from_translation(cam_pos).looking_at(look, Vec3::Y),
            tonemapping: Tonemapping::None,
            ..default()
        },
        MainCamera,
        IsDefaultUiCamera,
    ));
    if hdr {
        cam.insert(BloomSettings { intensity: 0.10, ..BloomSettings::NATURAL });
    }

    // Lights.
    commands.insert_resource(AmbientLight { color: Color::WHITE, brightness: 450.0 });
    commands.spawn((
        DirectionalLightBundle {
            directional_light: DirectionalLight { illuminance: 6500.0, shadows_enabled: cfg.shadows(), ..default() },
            transform: Transform::from_translation(render::KEY_LIGHT_POS).looking_at(Vec3::ZERO, Vec3::Y),
            ..default()
        },
        KeyLight,
    ));
    commands.spawn((
        DirectionalLightBundle {
            directional_light: DirectionalLight { illuminance: 2500.0, shadows_enabled: false, ..default() },
            transform: Transform::from_translation(render::RIM_LIGHT_POS).looking_at(Vec3::ZERO, Vec3::Y),
            ..default()
        },
        RimLight,
    ));

    // The shelf board (only in the shelf layout): grey, no back, placed every frame, fading
    // out towards both ends (see `shelf_board`).
    let fade = images.add(shelf_fade_image());
    let grey = mats.add(StandardMaterial {
        base_color: Color::srgb(0.46, 0.47, 0.5),
        base_color_texture: Some(fade),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.45,
        metallic: 0.15,
        ..default()
    });
    commands.spawn((
        PbrBundle { mesh: meshes.add(shelf_board()), material: grey, visibility: Visibility::Hidden, ..default() },
        ShelfDecor,
    ));

    setup_ui(&mut commands, &mut images);
}

fn text(size: f32, color: Color) -> TextBundle {
    TextBundle::from_section("", TextStyle { font_size: size, color, ..default() })
}

fn setup_ui(commands: &mut Commands, images: &mut Assets<Image>) {
    commands.insert_resource(HeartImages { full: images.add(heart_image(true)), empty: images.add(heart_image(false)) });
    commands.spawn((
        ButtonBundle {
            style: Style {
                position_type: PositionType::Absolute,
                right: Val::Px(30.0),
                top: Val::Px(64.0),
                width: Val::Px(44.0),
                height: Val::Px(44.0),
                ..default()
            },
            image: UiImage::new(images.add(gear_image())),
            background_color: BackgroundColor(Color::NONE),
            z_index: ZIndex::Global(45),
            ..default()
        },
        GearButton,
    ));
    commands.spawn((
        NodeBundle {
            style: Style { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            background_color: BackgroundColor(Color::NONE),
            z_index: ZIndex::Global(50),
            focus_policy: bevy::ui::FocusPolicy::Pass,
            ..default()
        },
        FlashOverlay,
    ));
    commands
        .spawn(NodeBundle {
            style: Style {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::all(Val::Px(28.0)),
                ..default()
            },
            ..default()
        })
        .with_children(|root| {
            root.spawn(NodeBundle {
                style: Style { width: Val::Percent(100.0), justify_content: JustifyContent::SpaceBetween, ..default() },
                ..default()
            })
            .with_children(|top| {
                top.spawn((text(17.0, Color::srgba(1.0, 1.0, 1.0, 0.75)), UiTop));
                top.spawn((text(20.0, Color::srgb(1.0, 0.92, 0.5)), UiSearch));
            });
            root.spawn(NodeBundle {
                style: Style { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, row_gap: Val::Px(6.0), ..default() },
                ..default()
            })
            .with_children(|bottom| {
                // Title with the favourite heart beside it.
                bottom
                    .spawn(NodeBundle {
                        style: Style { align_items: AlignItems::Center, column_gap: Val::Px(14.0), ..default() },
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((text(46.0, Color::WHITE), UiTitle));
                        row.spawn((
                            ButtonBundle {
                                style: Style { width: Val::Px(38.0), height: Val::Px(38.0), ..default() },
                                background_color: BackgroundColor(Color::NONE),
                                ..default()
                            },
                            HeartButton,
                        ));
                    });
                bottom.spawn((text(18.0, Color::srgba(1.0, 1.0, 1.0, 0.8)), UiSub));
                let mut d = text(15.0, Color::srgba(1.0, 1.0, 1.0, 0.6));
                d.style = Style { max_width: Val::Px(760.0), ..default() };
                d.text.justify = JustifyText::Center;
                bottom.spawn((d, UiDesc));
                bottom.spawn((text(13.0, Color::srgba(1.0, 1.0, 1.0, 0.38)), UiHints));
                bottom.spawn((text(16.0, Color::srgb(0.6, 1.0, 0.8)), UiToast));
            });
        });
}

// ------------------------------------------------------------------------------------------------
// Input
// ------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Repeat {
    held: Option<Action>,
    t: f32,
}

fn gather_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<Config>,
    view: Res<ViewState>,
    menu: Res<crate::settings::Menu>,
    time: Res<Time>,
    mut rep: Local<Repeat>,
    mut out: ResMut<Actions>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut shift_tap: Local<Option<f32>>,
) {
    if view.searching || menu.open || menu.just_closed || menu.art_open() {
        *shift_tap = None;
        return;
    }
    // Shift tapped on its own (pressed and let go quickly, nothing else pressed meanwhile)
    // opens / closes the case, like Space; Shift+Tab and other combinations are left alone.
    let shifts = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
    if keys.any_just_pressed(shifts) {
        *shift_tap = Some(time.elapsed_seconds());
    }
    if keys.get_just_pressed().any(|k| !shifts.contains(k)) || mouse.get_just_pressed().next().is_some() {
        *shift_tap = None;
    }
    if keys.any_just_released(shifts) && !keys.any_pressed(shifts) {
        if shift_tap.take().is_some_and(|t| time.elapsed_seconds() - t < 0.45) {
            out.0.push(Action::Open);
        }
    }
    let k = &cfg.keys;
    let single = [
        (&k.launch, Action::Launch),
        (&k.next_layout, Action::NextLayout),
        (&k.prev_layout, Action::PrevLayout),
        (&k.next_style, Action::NextStyle),
        (&k.next_background, Action::NextBackground),
        (&k.next_platform, Action::NextPlatform),
        (&k.prev_platform, Action::PrevPlatform),
        (&k.favorite, Action::Favorite),
        (&k.search, Action::Search),
        (&k.rescan, Action::Rescan),
        (&k.toggle_info, Action::ToggleInfo),
        (&k.toggle_fullscreen, Action::Fullscreen),
        (&k.page_up, Action::Home),
        (&k.page_down, Action::End),
        (&k.quit, Action::Quit),
        (&k.flip, Action::Flip),
        (&k.turn_left, Action::TurnLeft),
        (&k.turn_right, Action::TurnRight),
        (&k.open, Action::Open),
        (&k.close, Action::CloseCase),
    ];
    // A Shift+ binding (Shift+Tab) wins over the same key's plain binding (Tab).
    let lists: Vec<&Vec<String>> = single.iter().map(|(l, _)| *l).collect();
    let taken = crate::config::shift_combos(&lists, &keys);
    for (list, act) in single {
        if crate::config::keys_pressed_except(list, &keys, &taken) {
            out.0.push(act);
        }
    }
    let dirs = [(&k.left, Action::Left), (&k.right, Action::Right), (&k.up, Action::Up), (&k.down, Action::Down)];
    let mut any = false;
    for (list, act) in dirs {
        if !keys_held(list, &keys) {
            continue;
        }
        any = true;
        if keys_pressed(list, &keys) {
            out.0.push(act);
            rep.held = Some(act);
            rep.t = -0.30;
        } else if rep.held == Some(act) {
            rep.t += time.delta_seconds();
            while rep.t >= 0.055 {
                out.0.push(act);
                rep.t -= 0.055;
            }
        }
    }
    if !any {
        rep.held = None;
    }
}

fn gather_gamepad(
    gamepads: Res<Gamepads>,
    buttons: Res<ButtonInput<GamepadButton>>,
    axes: Res<Axis<GamepadAxis>>,
    time: Res<Time>,
    mut view: ResMut<ViewState>,
    mut stick_t: Local<f32>,
    mut turning: Local<bool>,
    mut out: ResMut<Actions>,
    menu: Res<crate::settings::Menu>,
) {
    if view.searching || menu.open || menu.just_closed {
        return;
    }
    let mut stick = 0.0;
    let mut turned = false;
    for pad in gamepads.iter() {
        let b = |t| buttons.just_pressed(GamepadButton::new(pad, t));
        for (t, a) in [
            (GamepadButtonType::DPadLeft, Action::Left),
            (GamepadButtonType::DPadRight, Action::Right),
            (GamepadButtonType::DPadUp, Action::Up),
            (GamepadButtonType::DPadDown, Action::Down),
            // A opens the case, and launches once it is open (B closes it).
            (GamepadButtonType::South, if view.inspecting() { Action::Launch } else { Action::Open }),
            (GamepadButtonType::LeftTrigger, Action::PrevLayout),
            (GamepadButtonType::RightTrigger, Action::NextLayout),
            (GamepadButtonType::West, Action::NextStyle),
            (GamepadButtonType::North, Action::Favorite),
            (GamepadButtonType::East, if view.inspecting() { Action::CloseCase } else { Action::NextBackground }),
            (GamepadButtonType::Select, Action::NextPlatform),
            (GamepadButtonType::RightThumb, Action::Flip),
        ] {
            if b(t) {
                out.0.push(a);
            }
        }
        stick += axes.get(GamepadAxis::new(pad, GamepadAxisType::LeftStickX)).unwrap_or(0.0);
        // Right stick turns the selected item around.
        let rx = axes.get(GamepadAxis::new(pad, GamepadAxisType::RightStickX)).unwrap_or(0.0);
        let ry = axes.get(GamepadAxis::new(pad, GamepadAxisType::RightStickY)).unwrap_or(0.0);
        // A generous deadzone, so a slightly drifting stick neither turns nor snaps the item.
        if rx.abs() > 0.35 || ry.abs() > 0.35 {
            let dt = time.delta_seconds();
            turned = true;
            if view.inspecting() {
                // Look around the open case.
                view.inspect_rot.x = (view.inspect_rot.x + rx * dt * 1.5).clamp(-1.1, 1.1);
                view.inspect_rot.y = (view.inspect_rot.y - ry * dt * 1.2).clamp(-0.8, 0.8);
            } else {
                view.turn_target.x += rx * dt * 3.5;
                view.turn_target.y = (view.turn_target.y - ry * dt * 2.5).clamp(-1.3, 1.3);
                view.free_turned = true;
            }
        }
    }
    // Stick let go: settle on the front or the back.
    if *turning && !turned && !view.inspecting() {
        view.snap_turn();
    }
    *turning = turned;
    if stick.abs() > 0.6 {
        *stick_t += time.delta_seconds();
        if *stick_t > 0.16 {
            *stick_t = 0.0;
            out.0.push(if stick > 0.0 { Action::Right } else { Action::Left });
        }
    } else {
        *stick_t = 0.16;
    }
}

#[derive(Default)]
struct Drag {
    button: Option<MouseButton>,
    moved: f32,
    accum: Vec2,
    last_click: f32,
    last_click_index: Option<usize>,
    /// When the current button went down (for telling a short click from a hold).
    pressed_at: f32,
    /// Left-drag speed (pixels a second), and the speed it carries on at once let go.
    vel: Vec2,
    coast: Vec2,
}

/// Left-drag (and its coasting afterwards) by `d` pixels: pans the wall / table, or scrubs
/// through the games one per ~60 px elsewhere.
fn drag_scroll(view: &mut ViewState, cfg: &Config, window: &Window, d: Vec2, accum: &mut Vec2, out: &mut Actions) {
    if cfg.display.layout.is_grid() {
        // Content follows the cursor: convert pixels to world cm at the wall plane.
        let (cam, look) = layouts::camera_for(&view.params);
        let table = cfg.display.layout == LayoutKind::Table;
        let dist = if table { (cam - look).length() } else { view.params.cam_distance } * view.zoom;
        let mut world_per_px = 2.0 * dist * (cfg.display.field_of_view.to_radians() * 0.5).tan() / window.height().max(1.0);
        let cell = (view.params.spacing * cfg.display.spacing_scale).max(0.1);
        if table {
            // The table is seen at a slant, so a row covers less of the screen than on the
            // wall (rows are also 1.1 cells deep), and the far rows are up the screen:
            // dragging down pulls them towards you.
            let slant = ((cam.y - look.y) / (cam - look).length().max(0.1)).clamp(0.2, 1.0);
            world_per_px /= -slant * 1.1;
        }
        let cols = view.params.columns.max(1);
        let n = view.view.len().max(1);
        let rows = (n + cols - 1) / cols;
        // The wall only pans up and down; it stays centred across.
        let mut t = view.grid_target - Vec2::new(0.0, -d.y) * world_per_px / cell;
        t.x = grid_centre_x(cols, n);
        if !view.looping {
            t.y = t.y.clamp(-((rows - 1) as f32) * GRID_ROW, 0.0);
        }
        view.grid_target = t;
        view.grid_center = t;
        view.grid_free = true;
        // Panning only moves the view; the games around the middle of the screen get
        // loaded (see `ViewState::anchor`) without anything being selected.
    } else {
        // Scrub along the layout's main axis, one item per ~60 px.
        let vertical = matches!(cfg.display.layout, LayoutKind::Stack | LayoutKind::Pile | LayoutKind::Towers | LayoutKind::MessyTowers);
        *accum += d;
        let step = 60.0;
        let amount = if vertical { -accum.y } else { -accum.x };
        let n = (amount / step).trunc() as i32;
        if n != 0 {
            for _ in 0..n.abs() {
                out.0.push(if n > 0 { Action::Right } else { Action::Left });
            }
            if vertical {
                accum.y += n as f32 * step;
            } else {
                accum.x += n as f32 * step;
            }
        }
    }
}

/// Which item is under the cursor (ray against each item's oriented bounding box).
/// The item under the cursor, with where on it the cursor is: (index, offset across the face
/// it hit in -1..1 item units, that face's normal), all in item space.
fn pick_at(
    window: &Window,
    camera: &Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    items: &Query<(&Item, &GlobalTransform, &InheritedVisibility)>,
    hovered: Option<usize>,
) -> Option<(usize, Vec3, Vec3)> {
    let cursor = window.cursor_position()?;
    let (cam, cam_tf) = camera.get_single().ok()?;
    let ray = cam.viewport_to_world(cam_tf, cursor)?;
    let mut best: Option<(f32, usize, Vec3, Vec3)> = None;
    for (item, tf, vis) in items.iter() {
        if !vis.get() {
            continue;
        }
        let inv = tf.affine().inverse();
        let o = inv.transform_point3(ray.origin);
        let d = inv.transform_vector3(*ray.direction);
        let mut half = item.dims * 0.5;
        // A little slack on the hovered item, so leaning away does not drop the hover.
        if hovered == Some(item.view_index) {
            half = half * 1.04 + Vec3::splat(0.3);
        }
        let (mut t0, mut t1) = (f32::NEG_INFINITY, f32::INFINITY);
        for k in 0..3 {
            if d[k].abs() < 1e-8 {
                if o[k].abs() > half[k] {
                    t0 = f32::INFINITY;
                }
                continue;
            }
            let a = (-half[k] - o[k]) / d[k];
            let b = (half[k] - o[k]) / d[k];
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
        if t0 <= t1 && t1 > 0.0 {
            let local = o + d * t0.max(0.0);
            if item.round && local.truncate().length() > half.x.max(half.y) {
                continue;
            }
            // Distances in local space are scaled; compare in world space.
            let hit = tf.transform_point(local);
            let dist = hit.distance(ray.origin);
            if best.map_or(true, |(bd, ..)| dist < bd) {
                let p = local / half.max(Vec3::splat(1e-3));
                let k = (0..3).max_by(|&a, &b| p[a].abs().total_cmp(&p[b].abs())).unwrap_or(2);
                let mut n = Vec3::ZERO;
                n[k] = p[k].signum();
                let mut r = p.clamp(Vec3::NEG_ONE, Vec3::ONE);
                r[k] = 0.0;
                best = Some((dist, item.view_index, r, n));
            }
        }
    }
    best.map(|(_, i, r, n)| (i, r, n))
}

fn pick(
    window: &Window,
    camera: &Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    items: &Query<(&Item, &GlobalTransform, &InheritedVisibility)>,
) -> Option<usize> {
    pick_at(window, camera, items, None).map(|(i, ..)| i)
}

/// Mouse while a case is open: dragging with either button turns it to look around, a left
/// double click launches, and the cursor position tilts the view a little and picks the part
/// it is over.
#[allow(clippy::too_many_arguments)]
fn inspect_mouse(
    view: &mut ViewState,
    cfg: &Config,
    window: &Window,
    delta: Vec2,
    buttons: &ButtonInput<MouseButton>,
    camera: &Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    items: &Query<(&Item, &GlobalTransform, &InheritedVisibility)>,
    over_ui: bool,
    drag: &mut Drag,
    now: f32,
    out: &mut Actions,
) {
    view.hovered = None;
    let sens = cfg.display.mouse_sensitivity.max(0.05);
    for b in [MouseButton::Left, MouseButton::Right] {
        if !over_ui && buttons.just_pressed(b) && drag.button.is_none() {
            drag.button = Some(b);
            drag.moved = 0.0;
        }
    }
    if let Some(b) = drag.button {
        if buttons.pressed(b) {
            drag.moved += delta.length();
            view.inspect_rot.x = (view.inspect_rot.x + delta.x * 0.005 * sens).clamp(-1.1, 1.1);
            view.inspect_rot.y = (view.inspect_rot.y + delta.y * 0.004 * sens).clamp(-0.8, 0.8);
        }
        if buttons.just_released(b) {
            drag.button = None;
            // A click on the insert turns a page: forward on the open pages, back on the
            // turned ones (at the last page it closes up again).
            if b == MouseButton::Left && drag.moved <= 4.0 && view.inspect_hover == InspectHover::Booklet {
                if view.insert_left {
                    view.insert_flipped = view.insert_flipped.saturating_sub(1);
                } else if view.insert_flipped >= view.insert_total {
                    view.insert_flipped = 0;
                } else {
                    view.insert_flipped += 1;
                }
                drag.last_click_index = None;
            } else if b == MouseButton::Left && drag.moved <= 4.0 {
                if drag.last_click_index == Some(usize::MAX) && now - drag.last_click < 0.4 {
                    drag.last_click_index = None;
                    out.0.push(Action::Launch);
                } else {
                    drag.last_click_index = Some(usize::MAX);
                    drag.last_click = now;
                    view.bounce = Some((view.selected, now));
                }
            }
        }
    }
    let Some(cursor) = window.cursor_position() else {
        view.inspect_hover = InspectHover::None;
        return;
    };
    let size = Vec2::new(window.width(), window.height()).max(Vec2::ONE);
    view.inspect_cursor = (cursor / size * 2.0 - Vec2::ONE).clamp(Vec2::NEG_ONE, Vec2::ONE);

    // What is under the cursor: the disc / cartridge, or the manual in the lid. Both lie flat
    // in the open case, so a ray against their plane (in item space) is enough.
    let mut hover = InspectHover::None;
    let Some(id) = view.inspect.clone() else { return };
    if let (Some((item, tf, _)), Ok((cam, cam_tf))) = (items.iter().find(|(i, ..)| i.game_id == id), camera.get_single()) {
        if let (Some(rig), Some(ray)) = (item.rig.as_ref(), cam.viewport_to_world(cam_tf, cursor)) {
            let inv = tf.affine().inverse();
            let (o, d) = (inv.transform_point3(ray.origin), inv.transform_vector3(*ray.direction));
            let on_plane = |z: f32| (d.z.abs() > 1e-6).then(|| o + d * ((z - o.z) / d.z)).filter(|_| (z - o.z) / d.z > 0.0);
            let rest = rig.media_inside.translation;
            if let Some(p) = on_plane(rest.z) {
                let r = rig.media_size * 0.5;
                let off = (p - rest).truncate();
                let inside = match rig.media_kind {
                    render::MediaKind::Disc => off.length() < r,
                    render::MediaKind::Cart => off.x.abs() < r && off.y.abs() < r,
                };
                if inside {
                    hover = InspectHover::Media;
                }
            }
            if rig.booklet.is_some() {
                view.insert_total = rig.pages.len() + 1;
            }
            if hover == InspectHover::None && rig.booklet.is_some() {
                if let Some(p) = on_plane(rig.booklet_open.z) {
                    let off = (p - rig.booklet_open).truncate();
                    // The turned pages lie beyond the hinge (its left edge once open).
                    let left = (off - Vec2::new(-2.0 * rig.booklet_half.x, 0.0)).abs();
                    if off.x.abs() < rig.booklet_half.x && off.y.abs() < rig.booklet_half.y {
                        hover = InspectHover::Booklet;
                        view.insert_left = false;
                    } else if view.insert_flipped > 0 && left.x < rig.booklet_half.x && left.y < rig.booklet_half.y {
                        hover = InspectHover::Booklet;
                        view.insert_left = true;
                    }
                }
            }
        }
    }
    view.inspect_hover = hover;
}

/// Mouse: wheel browses (ctrl + wheel zooms). Left drag pans the wall / scrubs through the
/// other layouts, right drag rotates the selected item, click selects, double click launches.
#[allow(clippy::too_many_arguments)]
fn gather_mouse(
    mut wheel: EventReader<MouseWheel>,
    mut motion: EventReader<MouseMotion>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    cfg: Res<Config>,
    lib: Res<Library>,
    mut view: ResMut<ViewState>,
    mut drag: Local<Drag>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    items: Query<(&Item, &GlobalTransform, &InheritedVisibility)>,
    ui: Query<&Interaction>,
    mut out: ResMut<Actions>,
) {
    // The easter egg has the mouse to itself (dragging cases round its table).
    if view.egg {
        wheel.clear();
        motion.clear();
        drag.button = None;
        view.hovered = None;
        return;
    }
    // With the settings open the games still respond to the mouse wherever it is not over
    // the panel (`over_ui` below); the keyboard stays with the menu.
    let grid = cfg.display.layout.is_grid();
    let over_ui = ui.iter().any(|i| *i != Interaction::None);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    for e in wheel.read() {
        let y = if e.unit == MouseScrollUnit::Pixel { e.y / 40.0 } else { e.y };
        if y == 0.0 || over_ui {
            continue;
        }
        if view.inspecting() {
            if ctrl {
                // Closer to / further from the open case: the same zoom as when browsing.
                view.zoom = (view.zoom * if y > 0.0 { 1.0 / 1.1 } else { 1.1 }).clamp(0.45, 4.0);
            } else if view.inspect_hover == InspectHover::Booklet {
                // Over the insert the wheel turns its pages (down: forward).
                if y < 0.0 {
                    view.insert_flipped = (view.insert_flipped + 1).min(view.insert_total);
                } else {
                    view.insert_flipped = view.insert_flipped.saturating_sub(1);
                }
            } else {
                // The wheel spins the disc on its hub; each notch adds speed.
                view.disc_spin_vel = (view.disc_spin_vel + y * 4.0).clamp(-40.0, 40.0);
            }
            continue;
        }
        if ctrl {
            out.0.push(if y > 0.0 { Action::ZoomIn } else { Action::ZoomOut });
        } else if grid {
            // Scroll the wall a row per notch without selecting anything.
            let cols = view.params.columns.max(1);
            let n = view.view.len().max(1);
            let rows = n.div_ceil(cols);
            // On the table the far rows are up the screen, so wheel up goes to them.
            let dir = if cfg.display.layout == LayoutKind::Table { -1.0 } else { 1.0 };
            let mut t = view.grid_target + Vec2::new(0.0, dir * y.signum() * GRID_ROW);
            if !view.looping {
                t.y = t.y.clamp(-((rows.max(1) - 1) as f32) * GRID_ROW, 0.0);
            }
            view.grid_target = t;
            view.grid_free = true;
        } else {
            out.0.push(if y > 0.0 { Action::Left } else { Action::Right });
        }
    }

    let delta: Vec2 = motion.read().map(|m| m.delta).sum();
    let Ok(window) = windows.get_single() else { return };
    if view.inspecting() {
        inspect_mouse(&mut view, &cfg, window, delta, &buttons, &camera, &items, over_ui, &mut drag, time.elapsed_seconds(), &mut out);
        return;
    }
    // What is under the cursor (for the hover highlight).
    let hit = if over_ui || drag.button.is_some() { None } else { pick_at(window, &camera, &items, view.hovered) };
    let hovered = hit.map(|(i, ..)| i);
    if view.hovered != hovered {
        view.hovered = hovered;
    }
    // Pressing on a spot pushes it in, like the Steam library: each direction across the face
    // tilts on its own, barely near the middle and strongly near the edges (the offset is
    // eased per axis), so corners tilt on both axes at once.
    // On a spine (or the opening edge) the face is narrow, so the item barely moves; leaning it
    // would slide the spine out from under the cursor.
    // Narrow faces (spine, opening edge, top and bottom edge) barely move: on a game lying on
    // its side, or a spine in a row, leaning would slide it out from under the cursor.
    let narrow = |n: Vec3| if n.x.abs() > 0.5 || n.y.abs() > 0.5 { 0.04 } else { 1.0 };
    view.hover_tilt = hit.map_or(Vec3::ZERO, |(_, r, n)| {
        let eased = Vec3::new(r.x.signum() * r.x.abs().powf(1.8), r.y.signum() * r.y.abs().powf(1.8), r.z.signum() * r.z.abs().powf(1.8));
        eased.cross(-n) * 0.34 * narrow(n)
    });
    view.hover_away = hit.map_or(Vec3::ZERO, |(_, r, n)| -r.normalize_or_zero() * r.length().min(1.0) * narrow(n));
    view.hover_spine = hit.is_some_and(|(_, _, n)| n.x.abs() > 0.5 || n.y.abs() > 0.5);
    view.hover_normal = hit.map_or(Vec3::ZERO, |(_, _, n)| n);
    let sens = cfg.display.mouse_sensitivity.max(0.05);

    // Let go mid-drag: keep scrolling, slowing down (any press stops it).
    if drag.button.is_none() && drag.coast != Vec2::ZERO {
        let dt = time.delta_seconds().min(0.1);
        let c = drag.coast;
        drag_scroll(&mut view, &cfg, window, c * dt, &mut drag.accum, &mut out);
        drag.coast *= (-dt * 3.2).exp();
        if drag.coast.length() < 40.0 {
            drag.coast = Vec2::ZERO;
        }
    }
    if buttons.get_just_pressed().next().is_some() {
        drag.coast = Vec2::ZERO;
    }
    for b in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
        if buttons.just_pressed(b) && drag.button.is_none() && !over_ui {
            drag.button = Some(b);
            drag.vel = Vec2::ZERO;
            drag.moved = 0.0;
            drag.accum = Vec2::ZERO;
            drag.pressed_at = time.elapsed_seconds();
        }
    }
    let Some(button) = drag.button else { return };
    drag.moved += delta.length();

    // Turning starts with the first movement (a right click does nothing else), so a quick
    // flick is not lost to the drag threshold.
    if buttons.pressed(button) && (drag.moved > 4.0 || button == MouseButton::Right) {
        match button {
            MouseButton::Right => {
                let dx = delta.x * 0.011 * sens;
                view.turn_target.x += dx;
                view.turn_target.y = (view.turn_target.y + delta.y * 0.008 * sens).clamp(-1.3, 1.3);
                // Spin speed for the flick, smoothed over a few frames; holding still kills it.
                let dt = time.delta_seconds().max(1e-3);
                view.spin_vel = view.spin_vel * 0.5 + (dx / dt) * 0.5;
                view.turning = true;
                view.free_turned = true;
            }
            _ => {
                drag_scroll(&mut view, &cfg, window, delta * sens, &mut drag.accum, &mut out);
                // How fast it is being dragged (pixels a second, smoothed), kept when let go.
                let dt = time.delta_seconds().max(1e-3);
                drag.vel = drag.vel.lerp(delta * sens / dt, 0.35);
            }
        }
    }

    if buttons.just_released(button) {
        drag.button = None;
        // Flung: carry on at the speed it was going (a slow release just stops).
        if button == MouseButton::Left && drag.moved > 4.0 && drag.vel.length() > 250.0 {
            drag.coast = drag.vel;
        }
        // Done rotating: settle on the front or the back, whichever is nearer.
        if button == MouseButton::Right {
            view.turning = false;
            // A flick keeps spinning (see animate_items); otherwise settle on front / back now.
            if view.spin_vel.abs() < 4.0 {
                view.spin_vel = 0.0;
                if drag.moved > 4.0 {
                    view.snap_turn();
                }
            }
            // A short right click (no drag) on a game flips it, selecting it first if needed.
            let short = time.elapsed_seconds() - drag.pressed_at < 0.35;
            if drag.moved <= 4.0 && short {
                if let Some(i) = pick(window, &camera, &items) {
                    if i != view.selected {
                        let free = view.grid_free;
                        view.select(i, &lib);
                        view.grid_free = free;
                        // Start it facing front now, so the "new selection faces front" reset
                        // in animate_items does not undo the flip.
                        view.last_selected = view.selected_id.clone();
                        view.turn = Vec2::ZERO;
                        view.turn_target = Vec2::ZERO;
                        view.spin_vel = 0.0;
                        view.free_turned = false;
                    }
                    out.0.push(Action::Flip);
                }
            }
        }
        if drag.moved <= 4.0 && button == MouseButton::Left {
            let picked = pick(window, &camera, &items);
            if picked.is_none() && grid {
                // Clicked beside the games: put the picked-up one back down.
                view.raised = false;
            }
            if let Some(i) = picked {
                let now = time.elapsed_seconds();
                if drag.last_click_index == Some(i) && now - drag.last_click < 0.4 {
                    out.0.push(Action::Launch);
                    drag.last_click_index = None;
                } else {
                    view.bounce = Some((i, now));
                    view.raised = true;
                    if grid {
                        // Wall / table: the clicked game comes to the middle of the screen.
                        view.select(i, &lib);
                        view.focus_visible(i);
                        view.grid_free = false;
                    } else if i != view.selected {
                        view.select(i, &lib);
                    }
                    drag.last_click = now;
                    drag.last_click_index = Some(i);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_actions(
    mut actions: ResMut<Actions>,
    mut view: ResMut<ViewState>,
    mut cfg: ResMut<Config>,
    mut lib: ResMut<Library>,
    mut persist: ResMut<Persist>,
    mut toast: ResMut<Toast>,
    mut scanner: ResMut<Scanner>,
    mut exit: EventWriter<AppExit>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut launch: ResMut<LaunchAnim>,
) {
    let acts: Vec<Action> = std::mem::take(&mut actions.0);
    for act in acts {
        match act {
            Action::Left => view.move_by(-1, &lib),
            Action::Right => view.move_by(1, &lib),
            Action::Up | Action::Down => {
                let sign = if act == Action::Up { -1 } else { 1 };
                let step = match cfg.display.layout {
                    LayoutKind::Grid | LayoutKind::Table => view.params.columns.max(1) as i32,
                    // The piles scroll one game at a time.
                    LayoutKind::Towers | LayoutKind::MessyTowers => 1,
                    _ => 8,
                };
                if view.looping && cfg.display.layout.is_grid() && !view.view.is_empty() {
                    // Stay in the column; the padded last row comes round to the first.
                    let cols = view.params.columns.max(1);
                    let rows = (view.wrap_n / cols).max(1) as i32;
                    let row = ((view.selected / cols) as i32 + sign).rem_euclid(rows) as usize;
                    let idx = (row * cols + view.selected % cols).min(view.view.len() - 1);
                    view.select(idx, &lib);
                    view.grid_free = false;
                } else {
                    view.move_by(sign * step, &lib);
                }
            }
            Action::Home => {
                view.select(0, &lib);
                view.grid_free = false;
            }
            Action::End => {
                let n = view.view.len();
                view.select(n.saturating_sub(1), &lib);
                view.grid_free = false;
            }
            Action::Launch => start_launch(&mut launch, &mut view, &cfg, &lib),
            // Wall / table: nothing to open until a game has been picked up.
            Action::Open if view.grid && !view.raised && !view.inspecting() => {}
            Action::Open => {
                let case = matches!(cfg.display.item_style, ItemStyle::Case | ItemStyle::CaseDisc);
                let selected = view.selected_id.clone();
                if !case {
                    // Discs and flat covers have nothing to open.
                    start_launch(&mut launch, &mut view, &cfg, &lib);
                } else if view.inspecting() && view.inspect == selected {
                    // Pressed again: close it.
                    view.inspect_open = false;
                    view.inspect_hover = InspectHover::None;
                } else if launch.game.is_none() && selected.is_some() {
                    view.inspect = selected;
                    view.inspect_open = true;
                    view.inspect_rot = Vec2::ZERO;
                    view.inspect_hover = InspectHover::None;
                    view.turn_target = Vec2::ZERO;
                    view.disc_spin = 0.0;
                    view.disc_spin_vel = 0.0;
                    view.spin_vel = 0.0;
                }
            }
            Action::CloseCase => {
                if !view.inspecting() && view.grid {
                    // Wall / table: put the picked-up game back down.
                    view.raised = false;
                }
                view.inspect_open = false;
                view.inspect_hover = InspectHover::None;
            }
            Action::NextLayout | Action::PrevLayout if view.inspecting() => {
                // With a case open the layout keys (Tab) close it instead.
                view.inspect_open = false;
                view.inspect_hover = InspectHover::None;
            }
            Action::NextLayout | Action::PrevLayout if view.grid && view.raised => {
                // Wall / table: Tab puts the picked-up game back down first.
                view.raised = false;
            }
            Action::NextLayout | Action::PrevLayout => {
                let step = if act == Action::NextLayout { 1 } else { -1 };
                cfg.display.layout = cfg.display.layout.cycle(step);
                view.zoom = cfg.display.zoom.clamp(0.45, 4.0);
                view.grid_free = false;
                toast.show(cfg.display.layout.name());
                view.dirty = true;
                persist.mark_cfg();
            }
            Action::NextStyle => {
                cfg.display.item_style = cfg.display.item_style.cycle(1);
                toast.show(cfg.display.item_style.name());
                view.respawn = true;
                view.dirty = true;
                persist.mark_cfg();
            }
            Action::NextBackground => {
                cfg.background.style = cfg.background.style.cycle(1);
                persist.mark_cfg();
            }
            Action::NextPlatform | Action::PrevPlatform => {
                let mut present: Vec<Platform> = lib.games.iter().map(|g| g.platform).collect();
                present.sort();
                present.dedup();
                let mut options: Vec<Option<Platform>> = vec![None];
                options.extend(present.into_iter().map(Some));
                let cur = options.iter().position(|o| *o == view.filter).unwrap_or(0) as i32;
                let step = if act == Action::NextPlatform { 1 } else { -1 };
                view.filter = options[(cur + step).rem_euclid(options.len() as i32) as usize];
                toast.show(view.filter.map_or("All platforms", |p| p.display_name()));
                view.dirty = true;
            }
            Action::Favorite => {
                if let Some(id) = view.current(&lib).map(|g| g.id.clone()) {
                    if let Some(g) = lib.find_mut(&id) {
                        g.favorite = !g.favorite;
                        toast.show(if g.favorite { "Added to favorites" } else { "Removed from favorites" });
                    }
                    persist.mark_lib();
                    view.dirty = true;
                }
            }
            Action::Search => {
                view.searching = true;
                view.just_started_search = true;
                view.search.clear();
                view.dirty = true;
            }
            Action::Rescan => {
                start_scan(&mut scanner, &cfg);
                toast.show("Scanning for games...");
            }
            Action::ToggleInfo => {
                cfg.general.show_info = !cfg.general.show_info;
                persist.mark_cfg();
            }
            Action::Fullscreen => {
                cfg.general.fullscreen = !cfg.general.fullscreen;
                if let Ok(mut w) = windows.get_single_mut() {
                    apply_window(&mut w, &cfg);
                }
                persist.mark_cfg();
            }
            Action::Flip => {
                // Half a turn from the nearest front/back orientation, so it toggles.
                let half_turns = (view.turn_target.x / std::f32::consts::PI).round();
                view.turn_target.x = (half_turns + 1.0) * std::f32::consts::PI;
                view.turn_target.y = 0.0;
            }
            Action::TurnLeft | Action::TurnRight => {
                let q = std::f32::consts::FRAC_PI_2;
                let dir = if act == Action::TurnLeft { 1.0 } else { -1.0 };
                view.turn_target.x = ((view.turn_target.x / q).round() + dir) * q;
                view.turn_target.y = 0.0;
                // A deliberate quarter turn: nothing snaps it back.
                view.free_turned = false;
                view.spin_vel = 0.0;
            }
            Action::ZoomIn | Action::ZoomOut => {
                let k = if act == Action::ZoomIn { 1.0 / 1.12 } else { 1.12 };
                view.zoom = (view.zoom * k).clamp(0.45, 4.0);
            }
            Action::Quit => {
                if view.inspecting() {
                    // Escape / Q on an open case closes it first.
                    view.inspect_open = false;
                    view.inspect_hover = InspectHover::None;
                } else if view.grid && view.raised {
                    // Then a picked-up game on the wall / table goes back down.
                    view.raised = false;
                } else if !view.search.is_empty() {
                    view.search.clear();
                    view.dirty = true;
                } else {
                    exit.send(AppExit::Success);
                }
            }
        }
    }
}

/// Starts launching the selected game. From an open case the animation skips straight to
/// taking the game out.
fn start_launch(launch: &mut LaunchAnim, view: &mut ViewState, cfg: &Config, lib: &Library) {
    let Some(g) = view.current(lib).cloned() else { return };
    if launch.game.is_some() {
        return;
    }
    let from_open = view.inspect.as_deref() == Some(g.id.as_str()) && view.inspect_t > 0.0;
    launch.game = Some((g.id.clone(), view.selected));
    launch.t = 0.0;
    launch.fired = false;
    launch.from_open = from_open;
    launch.open = from_open || (cfg.launch.open_case && matches!(cfg.display.item_style, ItemStyle::Case | ItemStyle::CaseDisc));
    launch.dur = if launch.open { cfg.launch.open_duration } else { cfg.launch.duration }.max(0.05);
    if from_open {
        // The point where the case is fully open and the game is lifted out.
        launch.t = 0.4 * launch.dur;
        view.inspect = None;
        view.inspect_open = false;
        view.inspect_t = 0.0;
    } else if cfg.launch.style == "none" && !launch.open {
        launch.t = launch.dur;
    }
}

fn search_typing(mut events: EventReader<KeyboardInput>, keys: Res<ButtonInput<KeyCode>>, mut view: ResMut<ViewState>) {
    let evs: Vec<KeyboardInput> = events.read().cloned().collect();
    if view.just_started_search {
        view.just_started_search = false;
        return;
    }
    if !view.searching {
        return;
    }
    // Ctrl+V pastes into the search.
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight]);
    if ctrl && keys.just_pressed(KeyCode::KeyV) {
        if let Some(t) = crate::clipboard::paste() {
            view.search.push_str(&t);
            view.dirty = true;
        }
    }
    for e in evs {
        if e.state != ButtonState::Pressed {
            continue;
        }
        match &e.logical_key {
            Key::Character(_) if ctrl => {}
            Key::Character(s) => {
                view.search.push_str(s.as_str());
                view.dirty = true;
            }
            Key::Space => {
                view.search.push(' ');
                view.dirty = true;
            }
            Key::Backspace => {
                view.search.pop();
                view.dirty = true;
            }
            Key::Enter => view.searching = false,
            Key::Escape => {
                view.search.clear();
                view.searching = false;
                view.dirty = true;
            }
            _ => {}
        }
    }
}

/// Fetches the cover and info of newly added games straight away (not only once they
/// come into view).
pub fn fetch_new(svc: &mut CoverService, lib: &Library, ids: &[String], cfg: &Config) {
    for id in ids {
        if let Some(g) = lib.find(id) {
            svc.request(CoverRequest::from_game(g, cfg));
        }
    }
}

fn handle_dropped_files(
    mut events: EventReader<FileDragAndDrop>,
    mut lib: ResMut<Library>,
    cfg: Res<Config>,
    mut toast: ResMut<Toast>,
    mut persist: ResMut<Persist>,
    mut svc: ResMut<CoverService>,
) {
    for e in events.read() {
        let FileDragAndDrop::DroppedFile { path_buf, .. } = e else { continue };
        if path_buf.is_dir() {
            let mut found = scanner::scan_rom_dir(&RomDir { path: path_buf.to_string_lossy().into(), ..default() }, &cfg);
            found.extend(scanner::scan_game_dir(path_buf));
            let n = found.len();
            let ids: Vec<String> = found.iter().map(|g| g.id.clone()).collect();
            for mut g in found {
                g.manual = true;
                lib.upsert(g);
            }
            lib.revision += 1;
            fetch_new(&mut svc, &lib, &ids, &cfg);
            toast.show(format!("Added {n} games from folder"));
        } else if let Some(id) = lib.add_manual(path_buf, None, None, &cfg) {
            let (title, plat) = lib.find(&id).map(|g| (g.title.clone(), g.platform.display_name())).unwrap_or_default();
            fetch_new(&mut svc, &lib, &[id], &cfg);
            toast.show(format!("Added {title} ({plat})"));
        } else {
            toast.show("Could not add that file");
        }
        persist.mark_lib();
    }
}

fn receive_scan(mut scanner: ResMut<Scanner>, mut lib: ResMut<Library>, mut cfg: ResMut<Config>, mut toast: ResMut<Toast>, mut persist: ResMut<Persist>) {
    let Some(rx) = scanner.rx.clone() else { return };
    if let Ok(found) = rx.try_recv() {
        let (added, removed) = lib.merge_scan(found);
        if lib.apply_hidden_titles(&mut cfg.data.scan.hidden_titles) {
            persist.mark_cfg();
        }
        let installed = lib.games.iter().filter(|g| g.installed).count();
        toast.show(format!("Scan complete: {added} new, {removed} removed ({installed} installed)"));
        persist.mark_lib();
        scanner.rx = None;
    }
}

// ------------------------------------------------------------------------------------------------
// View & items
// ------------------------------------------------------------------------------------------------

fn rebuild_view(mut view: ResMut<ViewState>, lib: Res<Library>, cfg: Res<Config>) {
    if view.seen_revision != lib.revision {
        view.seen_revision = lib.revision;
        view.dirty = true;
    }
    if view.dirty {
        view.rebuild(&lib, &cfg);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_items(
    mut commands: Commands,
    mut view: ResMut<ViewState>,
    lib: Res<Library>,
    cfg: Res<Config>,
    mut gfx: ResMut<Gfx>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut disc_materials: ResMut<Assets<DiscMaterial>>,
    mut images: ResMut<Assets<Image>>,
    font: Res<SpineFont>,
    mut svc: ResMut<CoverService>,
) {
    if view.respawn {
        for (_, e) in view.spawned.drain() {
            commands.entity(e).despawn_recursive();
        }
        view.covers.clear();
        view.backs.clear();
        view.respawn = false;
    }
    if view.view.is_empty() || view.egg {
        return;
    }
    let r = layouts::window_radius(cfg.display.layout, &view.params, cfg.window_radius(), view.zoom);
    let n = view.view.len();
    let at = view.anchor();
    // Everything within `r` steps of the selection (or where the wall is panned to), the short
    // way round when looping.
    let wanted: Vec<usize> = if view.looping {
        let wn = view.wrap_n.max(n);
        let m = r.min(wn / 2) as i64;
        let mut w: Vec<usize> = (-m..=m).map(|d| (at as i64 + d).rem_euclid(wn as i64) as usize).filter(|&i| i < n).collect();
        w.sort_unstable();
        w.dedup();
        w
    } else {
        (at.saturating_sub(r)..=(at + r).min(n - 1)).collect()
    };
    let keep: HashSet<usize> = wanted.iter().copied().collect();

    let gone: Vec<usize> = view.spawned.keys().copied().filter(|i| !keep.contains(i)).collect();
    for i in gone {
        if let Some(e) = view.spawned.remove(&i) {
            commands.entity(e).despawn_recursive();
        }
        if let Some(g) = view.view.get(i).and_then(|&li| lib.games.get(li)) {
            view.covers.remove(&g.id);
            view.backs.remove(&g.id);
        }
    }

    let style = cfg.display.item_style;
    let mut pending_spines: Vec<(Entity, Handle<Image>, u32, u32)> = Vec::new();
    let mut cx = SpawnCtx {
        gfx: &mut gfx,
        meshes: &mut meshes,
        materials: &mut materials,
        disc_materials: &mut disc_materials,
        images: &mut images,
        font: font.0.as_ref(),
        cfg: &cfg,
    };
    // Spawn nearest-first so what you are looking at gets its cover first.
    let mut order = wanted;
    order.sort_by_key(|&i| view.steps(at, i));
    // Spread big batches (zooming out, the bookcase) over a few frames.
    let mut budget = 32;
    for i in order {
        if view.spawned.contains_key(&i) {
            continue;
        }
        if budget == 0 {
            break;
        }
        budget -= 1;
        let Some(game) = lib.games.get(view.view[i]) else { continue };
        let e = render::spawn_item(&mut commands, &mut cx, game, i, style);
        // Coloured cartridge variants: measure this game's cartridge from its photo once.
        if game.cart_color.is_none() && matches!(game.platform, Platform::Gb | Platform::Gbc | Platform::Gba | Platform::N64 | Platform::Nes | Platform::Snes | Platform::Genesis) {
            svc.request_cart_color(&game.id);
        }
        view.spawned.insert(i, e);
        if let Some((h, w, hh)) = view.spines.get(&game.id).cloned() {
            pending_spines.push((e, h, w, hh));
        }
        let known = ApplyCover {
            pages: view.pages.get(&game.id).cloned().unwrap_or_default(),
            cover: view.covers.get(&game.id).cloned(),
            media: view.arts.get(&(game.id.clone(), render::ArtKind::Media)).cloned(),
            insert: view.arts.get(&(game.id.clone(), render::ArtKind::Insert)).cloned(),
        };
        if known.cover.is_some() || known.media.is_some() || known.insert.is_some() || !known.pages.is_empty() {
            commands.entity(e).insert(known);
        }
        svc.request(CoverRequest::from_game(game, &cfg));
        // The disc / cartridge label and the insert get art of their own where some exists.
        if style != ItemStyle::Flat {
            svc.request_art(crate::covers::ArtRequest::from_game(game, &cfg, render::ArtKind::Media));
        }
        if matches!(style, ItemStyle::Case | ItemStyle::CaseDisc) {
            svc.request_art(crate::covers::ArtRequest::from_game(game, &cfg, render::ArtKind::Insert));
        }
    }
    for (e, h, w, hh) in pending_spines {
        commands.entity(e).insert(ApplySpine(h, w, hh));
    }
}

// ------------------------------------------------------------------------------------------------
// Launching
// ------------------------------------------------------------------------------------------------

/// The launch animation: the game starts when it finishes.
#[derive(Resource, Default)]
pub struct LaunchAnim {
    /// Game id and view index being launched.
    pub game: Option<(String, usize)>,
    pub t: f32,
    pub fired: bool,
    /// Seconds the animation lasts, fixed when it starts.
    pub dur: f32,
    /// The case opens and the game is taken out (instead of `launch.style`).
    pub open: bool,
    /// Launched from a case that was already open: the game starts from its holder.
    pub from_open: bool,
}

impl LaunchAnim {
    /// 0..1 progress of the animation (1 once the game has been started).
    pub fn progress(&self) -> f32 {
        if self.game.is_none() {
            return 0.0;
        }
        (self.t / self.dur.max(0.05)).clamp(0.0, 1.0)
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Component)]
struct FlashOverlay;

#[allow(clippy::too_many_arguments)]
fn run_launch(
    time: Res<Time>,
    cfg: Res<Config>,
    mut launch: ResMut<LaunchAnim>,
    mut lib: ResMut<Library>,
    mut toast: ResMut<Toast>,
    mut persist: ResMut<Persist>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut flash: Query<&mut BackgroundColor, With<FlashOverlay>>,
    overlay: Res<Overlay>,
    mut exit: EventWriter<AppExit>,
) {
    let dur = launch.dur.max(0.05);
    let fade = 0.35;
    let mut alpha = 0.0;
    if let Some((id, _)) = launch.game.clone() {
        launch.t += time.delta_seconds().min(0.05);
        let p = (launch.t / dur).clamp(0.0, 1.0);
        let start = if launch.open { 0.86 } else { 0.65 };
        alpha = ((p - start) / (1.0 - start)).clamp(0.0, 1.0).powi(2);
        if launch.t >= dur && !launch.fired {
            launch.fired = true;
            if let Some(g) = lib.find(&id).cloned() {
                match launch_game(&g, &cfg) {
                    Ok(()) => {
                        toast.show(format!("Launching {}", g.title));
                        if let Some(game) = lib.find_mut(&g.id) {
                            game.last_played = now_secs();
                            game.play_count += 1;
                        }
                        persist.mark_lib();
                        if overlay.0 || cfg.general.close_on_launch {
                            // Save now: the autosave timer will not get another chance.
                            lib.save();
                            exit.send(AppExit::Success);
                        } else if cfg.general.minimize_on_launch {
                            if let Ok(mut w) = windows.get_single_mut() {
                                w.set_minimized(true);
                            }
                        }
                    }
                    Err(e) => toast.show(format!("Could not launch: {e}")),
                }
            }
        }
        if launch.fired {
            alpha = 1.0 - ((launch.t - dur) / fade).clamp(0.0, 1.0);
            if launch.t >= dur + fade + 0.4 {
                launch.game = None;
            }
        }
    }
    let c = crate::config::parse_hex(&cfg.launch.flash_color);
    // No white-out flash over the user's desktop.
    let a = if overlay.0 { 0.0 } else { alpha * cfg.launch.flash.clamp(0.0, 1.0) };
    for mut bg in &mut flash {
        bg.0 = Color::srgba(c[0], c[1], c[2], a);
    }
}

// ------------------------------------------------------------------------------------------------
// Favourite button
// ------------------------------------------------------------------------------------------------

#[derive(Component)]
struct HeartButton;

#[derive(Resource)]
struct HeartImages {
    full: Handle<Image>,
    empty: Handle<Image>,
}

/// Heart icon, filled or as an outline, antialiased by supersampling.
fn heart_image(filled: bool) -> Image {
    let n = 96u32;
    let inside = |x: f32, y: f32, scale: f32| {
        // Classic heart curve (x^2 + y^2 - 1)^3 - x^2 y^3 <= 0, centred in the image.
        let (x, y) = (x / scale, y / scale);
        let a = x * x + y * y - 1.0;
        a * a * a - x * x * y * y * y <= 0.0
    };
    let mut rgba = Vec::with_capacity((n * n * 4) as usize);
    for py in 0..n {
        for px in 0..n {
            let mut cov = 0.0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = ((px as f32 + (sx as f32 + 0.5) / 4.0) / n as f32 - 0.5) * 2.6;
                    let y = -((py as f32 + (sy as f32 + 0.5) / 4.0) / n as f32 - 0.5) * 2.6 + 0.12;
                    let hit = if filled { inside(x, y, 1.0) } else { inside(x, y, 1.0) && !inside(x * 1.0, y + 0.05, 0.78) };
                    if hit {
                        cov += 1.0 / 16.0;
                    }
                }
            }
            let a = (cov * 255.0) as u8;
            rgba.extend_from_slice(&[255, 255, 255, a]);
        }
    }
    let mut img = make_image(n, n, rgba, false);
    img.texture_descriptor.format = bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb;
    img
}

fn heart_button(
    view: Res<ViewState>,
    lib: Res<Library>,
    hearts: Option<Res<HeartImages>>,
    mut q: Query<(&Interaction, &mut UiImage, &mut Style), With<HeartButton>>,
    mut last: Local<Interaction>,
    mut out: ResMut<Actions>,
    menu: Res<crate::settings::Menu>,
) {
    let Some(hearts) = hearts else { return };
    let game = view.current(&lib);
    let fav = game.is_some_and(|g| g.favorite);
    for (interaction, mut img, mut style) in &mut q {
        style.display = if game.is_some() && !menu.open { Display::Flex } else { Display::None };
        if *interaction == Interaction::Pressed && *last != Interaction::Pressed {
            out.0.push(Action::Favorite);
        }
        *last = *interaction;
        img.texture = if fav { hearts.full.clone() } else { hearts.empty.clone() };
        img.color = match (fav, *interaction) {
            (true, Interaction::Hovered) => Color::srgb(1.0, 0.45, 0.55),
            (true, _) => Color::srgb(1.0, 0.25, 0.38),
            (false, Interaction::Hovered) => Color::srgba(1.0, 1.0, 1.0, 1.0),
            (false, _) => Color::srgba(1.0, 1.0, 1.0, 0.7),
        };
    }
}

/// Gear icon: a toothed ring, antialiased by supersampling.
fn gear_image() -> Image {
    let n = 96u32;
    let teeth = 8.0;
    let inside = |x: f32, y: f32| {
        let r = (x * x + y * y).sqrt();
        let a = y.atan2(x);
        // Smoothed square wave round the rim makes the teeth.
        let wave = ((a * teeth).cos() * 3.0).clamp(-1.0, 1.0) * 0.5 + 0.5;
        r <= 0.68 + 0.22 * wave && r >= 0.3
    };
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

/// The gear in the corner opens and closes the settings menu.
fn gear_button(mut q: Query<(&Interaction, &mut UiImage, &mut Style), With<GearButton>>, mut menu: ResMut<crate::settings::Menu>, mut last: Local<Interaction>) {
    for (interaction, mut img, mut style) in &mut q {
        // Beside the open menu panel (560 px wide) rather than on top of it.
        let right = Val::Px(if menu.open { crate::settings::PANEL_W + 20.0 } else { 30.0 });
        if style.right != right {
            style.right = right;
        }
        if *interaction == Interaction::Pressed && *last != Interaction::Pressed {
            if menu.open {
                menu.hide();
            } else {
                menu.show();
            }
        }
        *last = *interaction;
        img.color = match *interaction {
            Interaction::None => Color::srgba(1.0, 1.0, 1.0, 0.7),
            _ => Color::WHITE,
        };
    }
}

/// Shows the soft outline round the game under the mouse, in the configured colour.
#[allow(clippy::too_many_arguments)]
fn update_outlines(
    view: Res<ViewState>,
    cfg: Res<Config>,
    launch: Res<LaunchAnim>,
    overlay: Res<Overlay>,
    gfx: Res<Gfx>,
    items: Query<&Item>,
    mut outlines: Query<&mut Visibility, With<render::Outline>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut applied: Local<String>,
) {
    let Some(mat) = gfx.outline.clone() else { return };
    // Recolour when the settings change. With bloom the colour is pushed past white so it glows.
    let hdr = cfg.bloom() && !overlay.0;
    let key = format!("{}{hdr}", cfg.display.highlight_color);
    if *applied != key {
        let c = crate::config::parse_hex(&cfg.display.highlight_color);
        let boost = if hdr { 2.0 } else { 1.0 };
        let lin = Color::srgb(c[0], c[1], c[2]).to_linear();
        if let Some(m) = materials.get_mut(&mat) {
            m.base_color = Color::linear_rgba(lin.red * boost, lin.green * boost, lin.blue * boost, render::OUTLINE_ALPHA);
        }
        *applied = key;
    }
    let on = matches!(cfg.highlight(), "outline" | "both") && launch.game.is_none();
    for item in &items {
        let Ok(mut vis) = outlines.get_mut(item.outline) else { continue };
        let want = if on && view.hovered == Some(item.view_index) { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
}

/// A faint glow on the item under the cursor ("glow" / "both" highlight).
fn hover_glow(
    view: Res<ViewState>,
    cfg: Res<Config>,
    items: Query<&Item>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut lit: Local<Option<usize>>,
) {
    let target = if matches!(cfg.highlight(), "glow" | "both") { view.hovered } else { None };
    if *lit == target {
        return;
    }
    for item in &items {
        let on = target == Some(item.view_index);
        let was = *lit == Some(item.view_index);
        if !on && !was {
            continue;
        }
        let glow = if on { LinearRgba::rgb(0.07, 0.075, 0.085) } else { LinearRgba::BLACK };
        for slot in item.covers.iter().chain(item.back.iter()).chain(item.spine.iter()) {
            if let Some(m) = materials.get_mut(&slot.mat) {
                m.emissive = glow;
            }
        }
    }
    *lit = target;
}

/// A known spine image to put on a freshly spawned item.
#[derive(Component)]
pub struct ApplySpine(Handle<Image>, u32, u32);

/// A cover already loaded (this session) to put on a freshly spawned item.
#[derive(Component)]
pub struct ApplyCover {
    pages: Vec<(Handle<Image>, u32, u32)>,
    cover: Option<(Handle<Image>, u32, u32)>,
    media: Option<(Handle<Image>, u32, u32)>,
    insert: Option<(Handle<Image>, u32, u32)>,
}

fn apply_covers(mut commands: Commands, items: Query<(Entity, &Item, &ApplyCover)>, mut materials: ResMut<Assets<StandardMaterial>>) {
    for (e, item, a) in &items {
        for slot in &item.covers {
            // The disc label / insert get their own art when there is some, else the cover.
            let art = match slot.kind {
                render::ArtKind::Cover => a.cover.as_ref(),
                render::ArtKind::Media => a.media.as_ref().or(a.cover.as_ref()),
                render::ArtKind::Insert => a.insert.as_ref().or(a.cover.as_ref()),
                render::ArtKind::Back | render::ArtKind::Spine => None,
            };
            if let (Some((h, w, hh)), Some(m)) = (art, materials.get_mut(&slot.mat)) {
                apply_texture(m, h.clone(), *w, *hh, slot.aspect);
            }
        }
        for (slot, (h, w, hh)) in item.pages.iter().zip(&a.pages) {
            if let Some(m) = materials.get_mut(&slot.mat) {
                apply_texture(m, h.clone(), *w, *hh, slot.aspect);
            }
        }
        commands.entity(e).remove::<ApplyCover>();
    }
}

fn apply_spines(mut commands: Commands, items: Query<(Entity, &Item, &ApplySpine)>, mut materials: ResMut<Assets<StandardMaterial>>) {
    for (e, item, a) in &items {
        if let Some(slot) = &item.spine {
            if let Some(m) = materials.get_mut(&slot.mat) {
                apply_texture(m, a.0.clone(), a.1, a.2, slot.aspect);
            }
        }
        commands.entity(e).remove::<ApplySpine>();
    }
}

/// Asks for stylised spines for the cases on screen and for the selected game's backdrop.
fn request_extras(
    view: Res<ViewState>,
    lib: Res<Library>,
    cfg: Res<Config>,
    items: Query<&Item>,
    bg_art: Res<BgArt>,
    mut svc: ResMut<CoverService>,
) {
    // The open case's insert: make its pages.
    if let Some(id) = view.inspect.as_ref().filter(|id| !view.pages.contains_key(*id)) {
        if let (Some(item), Some(g)) = (items.iter().find(|i| &i.game_id == id && !i.pages.is_empty()), lib.find(id)) {
            let aspect = item.pages.get(1).map_or(0.7, |p| p.aspect);
            svc.request_pages(crate::covers::PageRequest { game: crate::covers::BackRequest::from_game(g, &cfg, aspect), aspect, faces: item.pages.len() });
        }
    }
    {
        for item in &items {
            let Some(slot) = &item.spine else { continue };
            let id = &item.game_id;
            if view.spines.contains_key(id) || svc.spines_pending.contains(id) || svc.pending.contains(id) {
                continue;
            }
            let Some(g) = lib.find(id) else { continue };
            // Plain spines unless the user picked art for this one.
            if cfg.display.spine_style == "simple" && g.spine_art.is_none() {
                continue;
            }
            svc.request_spine(SpineRequest {
                id: id.clone(),
                title: g.title.clone(),
                platform: g.platform.display_name().to_string(),
                accent: render::accent(&cfg, g.platform),
                aspect: slot.aspect,
                steam_appid: g.steam_appid,
                cover_path: g.cover_path.clone(),
                prefer_generated: cfg.display.spine_style == "generated",
                chosen: g.spine_art.clone(),
            });
        }
    }
    // Potato mode draws no background, so the art is not needed.
    if cfg.background.style == BackgroundStyle::GameArt && !cfg.potato() {
        if let Some(g) = view.current(&lib) {
            if !bg_art.cache.contains_key(&g.id) && !svc.backdrops_pending.contains(&g.id) && !svc.pending.contains(&g.id) {
                svc.request_backdrop(BackdropRequest {
                    id: g.id.clone(),
                    blur: cfg.background.art_blur,
                    steam_appid: g.steam_appid,
                    cover_path: g.cover_path.clone(),
                });
            }
        }
    }
}

fn update_backdrop(view: Res<ViewState>, lib: Res<Library>, mut art: ResMut<BgArt>) {
    let want = view.current(&lib).and_then(|g| art.cache.get(&g.id).map(|(h, w, hh)| (g.id.clone(), h.clone(), *w, *hh)));
    if want.is_some() {
        art.want = want;
    }
}

/// Matches the library against the LaunchBox database in the background.
#[derive(Resource, Default)]
pub struct LaunchBoxJob {
    rx: Option<Receiver<usize>>,
    seen_revision: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
fn launchbox_update(
    mut job: ResMut<LaunchBoxJob>,
    lib: Res<Library>,
    cfg: Res<Config>,
    scanner: Res<Scanner>,
    mut svc: ResMut<CoverService>,
    mut view: ResMut<ViewState>,
    mut art: ResMut<BgArt>,
) {
    if let Some(rx) = &job.rx {
        if let Ok(n) = rx.try_recv() {
            job.rx = None;
            if n > 0 {
                // Fresh matches: retry missing covers, redo spines, backs and backdrops.
                svc.missing.clear();
                svc.spines_pending.clear();
                svc.backs_pending.clear();
                svc.backdrops_pending.clear();
                view.spines.clear();
                art.cache.clear();
                view.respawn = true;
                view.dirty = true;
            }
        }
        return;
    }
    if !cfg.covers.enabled || !cfg.covers.sources.iter().any(|s| s == "launchbox") || scanner.rx.is_some() {
        return;
    }
    if job.seen_revision == Some(lib.revision) {
        return;
    }
    job.seen_revision = Some(lib.revision);
    if !crate::launchbox::needs_update(&lib.games) {
        return;
    }
    let games = lib.games.clone();
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send(crate::launchbox::update(games));
    });
    job.rx = Some(rx);
}

#[allow(clippy::too_many_arguments)]
fn receive_covers(
    mut commands: Commands,
    mut svc: ResMut<CoverService>,
    mut lib: ResMut<Library>,
    mut view: ResMut<ViewState>,
    cfg: Res<Config>,
    font: Res<SpineFont>,
    items: Query<&Item>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut persist: ResMut<Persist>,
    mut bg_art: ResMut<BgArt>,
    mut fades: ResMut<CoverFades>,
) {
    let mut budget = 6; // limit GPU uploads per frame to keep scrolling smooth
    while budget > 0 {
        let Ok(msg) = svc.rx.try_recv() else { break };
        match msg {
            CoverMsg::Cover(c) => {
                budget -= 1;
                svc.pending.remove(&c.id);
                if let Some(g) = lib.find_mut(&c.id) {
                    g.dominant = Some(c.dominant);
                    persist.mark_lib();
                }
                let mut data = Some(c.rgba);
                let mut handle: Option<Handle<Image>> = None;
                for item in items.iter().filter(|i| i.game_id == c.id) {
                    if handle.is_none() {
                        if let Some(d) = data.take() {
                            handle = Some(images.add(make_image(c.width, c.height, d, true)));
                        }
                    }
                    if let Some(h) = &handle {
                        // Not over a disc label / insert that already has art of its own.
                        for slot in item.covers.iter().filter(|s| s.kind == render::ArtKind::Cover || !view.arts.contains_key(&(c.id.clone(), s.kind))) {
                            if let Some(m) = materials.get_mut(&slot.mat) {
                                apply_texture(m, h.clone(), c.width, c.height, slot.aspect);
                                begin_cover_fade(m, &slot.mat, &mut fades);
                            }
                        }
                    }
                }
                if let Some(h) = handle {
                    view.covers.insert(c.id.clone(), (h, c.width, c.height));
                }
            }
            CoverMsg::Pages(p) => {
                budget -= 1;
                let faces: Vec<(Handle<Image>, u32, u32)> = p.faces.into_iter().map(|c| (images.add(make_image(c.width, c.height, c.rgba, true)), c.width, c.height)).collect();
                for item in items.iter().filter(|i| i.game_id == p.id) {
                    for (slot, (h, w, hh)) in item.pages.iter().zip(&faces) {
                        if let Some(m) = materials.get_mut(&slot.mat) {
                            apply_texture(m, h.clone(), *w, *hh, slot.aspect);
                        }
                    }
                }
                view.pages.insert(p.id, faces);
            }
            CoverMsg::Art(kind, c) => {
                budget -= 1;
                let h = images.add(make_image(c.width, c.height, c.rgba, true));
                for item in items.iter().filter(|i| i.game_id == c.id) {
                    for slot in item.covers.iter().filter(|s| s.kind == kind) {
                        if let Some(m) = materials.get_mut(&slot.mat) {
                            apply_texture(m, h.clone(), c.width, c.height, slot.aspect);
                            begin_cover_fade(m, &slot.mat, &mut fades);
                        }
                    }
                }
                view.arts.insert((c.id.clone(), kind), (h, c.width, c.height));
            }
            CoverMsg::Missing(id) => {
                svc.pending.remove(&id);
                svc.missing.insert(id.clone());
                let Some(g) = lib.find_mut(&id) else { continue };
                let ac = render::accent(&cfg, g.platform);
                if g.dominant.is_none() {
                    g.dominant = Some([(ac[0] * 1.6).min(1.0), (ac[1] * 1.6).min(1.0), (ac[2] * 1.6).min(1.0)]);
                }
                let (title, plat) = (g.title.clone(), g.platform.display_name());
                let mut handle: Option<Handle<Image>> = None;
                for item in items.iter().filter(|i| i.game_id == id) {
                    let h = handle.get_or_insert_with(|| images.add(make_placeholder(&title, plat, ac, font.0.as_ref()))).clone();
                    for slot in item.covers.iter().filter(|s| s.kind == render::ArtKind::Cover || !view.arts.contains_key(&(id.clone(), s.kind))) {
                        if let Some(m) = materials.get_mut(&slot.mat) {
                            apply_texture(m, h.clone(), 360, 540, slot.aspect);
                            begin_cover_fade(m, &slot.mat, &mut fades);
                        }
                    }
                }
                if let Some(h) = handle {
                    view.covers.insert(id, (h, 360, 540));
                }
            }
            CoverMsg::Back(c) => {
                budget -= 1;
                svc.backs_pending.remove(&c.id);
                let h = images.add(make_image(c.width, c.height, c.rgba, true));
                for item in items.iter().filter(|i| i.game_id == c.id) {
                    if let Some(slot) = &item.back {
                        if let Some(m) = materials.get_mut(&slot.mat) {
                            apply_texture(m, h.clone(), c.width, c.height, slot.aspect);
                            begin_cover_fade(m, &slot.mat, &mut fades);
                        }
                    }
                }
                view.backs.insert(c.id.clone(), (h, c.width, c.height));
            }
            CoverMsg::Spine(c) => {
                budget -= 1;
                let h = images.add(make_image(c.width, c.height, c.rgba, true));
                for item in items.iter().filter(|i| i.game_id == c.id) {
                    if let Some(slot) = &item.spine {
                        if let Some(m) = materials.get_mut(&slot.mat) {
                            apply_texture(m, h.clone(), c.width, c.height, slot.aspect);
                            begin_cover_fade(m, &slot.mat, &mut fades);
                        }
                    }
                }
                view.spines.insert(c.id.clone(), (h, c.width, c.height));
                svc.spines_pending.remove(&c.id);
            }
            CoverMsg::Backdrop(c) => {
                budget -= 1;
                svc.backdrops_pending.remove(&c.id);
                let h = images.add(make_image(c.width, c.height, c.rgba, false));
                bg_art.insert(c.id.clone(), h, c.width, c.height);
            }
            CoverMsg::CartColor(id, Some(c)) => {
                if let Some(g) = lib.find_mut(&id) {
                    g.cart_color = Some(c);
                    persist.mark_lib();
                }
                // Rebuild that game so its cartridge takes the colour.
                if let Some((&i, &e)) = view.spawned.iter().find(|(i, _)| view.view.get(**i).and_then(|&li| lib.games.get(li)).is_some_and(|g| g.id == id)) {
                    commands.entity(e).despawn_recursive();
                    view.spawned.remove(&i);
                }
            }
            CoverMsg::CartColor(_, None) => {}
            CoverMsg::Meta(m) => {
                if let Some(g) = lib.find_mut(&m.id) {
                    g.description = m.description;
                    g.genres = m.genres;
                    g.year = m.year;
                    persist.mark_lib();
                }
            }
        }
    }
}

/// Once the selected item is being turned around, have its back cover printed.
/// Has the back cover of every game whose back faces the camera printed (the far side of the
/// carousel, a game turned or flipped round), a few per frame.
fn request_backs(
    lib: Res<Library>,
    view: Res<ViewState>,
    cfg: Res<Config>,
    items: Query<(&Item, &GlobalTransform, &InheritedVisibility)>,
    cams: Query<&GlobalTransform, With<MainCamera>>,
    mut svc: ResMut<CoverService>,
) {
    let Ok(cam) = cams.get_single().map(|c| c.translation()) else { return };
    let turning = view.turn_target != Vec2::ZERO || view.turn.length() > 0.05 || cfg.animation.spin != 0.0;
    let mut budget = 4;
    for (item, gt, vis) in &items {
        if budget == 0 {
            break;
        }
        let Some(slot) = item.back.as_ref() else { continue };
        let id = &item.game_id;
        if !vis.get() || view.backs.contains_key(id) || svc.backs_pending.contains(id) || svc.pending.contains(id) {
            continue;
        }
        // The front faces local +Z: the back shows when that points away from the camera.
        let front = gt.affine().transform_vector3(Vec3::Z).normalize_or_zero();
        let back_visible = front.dot((cam - gt.translation()).normalize_or_zero()) < -0.05;
        let being_turned = turning && item.view_index == view.selected;
        if !(back_visible || being_turned) {
            continue;
        }
        if let Some(g) = lib.find(id) {
            svc.request_back(BackRequest::from_game(g, &cfg, slot.aspect));
            budget -= 1;
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Animation
// ------------------------------------------------------------------------------------------------

fn animate_items(
    time: Res<Time>,
    mut view: ResMut<ViewState>,
    cfg: Res<Config>,
    mut items: Query<(Entity, &Item, &mut Transform, &mut Visibility, &mut render::Fade), Without<crate::egg::EggPiece>>,
    mut laps: Local<HashMap<Entity, i64>>,
    mut last_layout: Local<Option<LayoutKind>>,
    mut decor: Query<(&mut Transform, &mut Visibility), (With<ShelfDecor>, Without<Item>)>,
    launch: Res<LaunchAnim>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let dt = time.delta_seconds().min(0.1);
    let t = time.elapsed_seconds();
    let aspect = windows.get_single().map(|w| w.width() / w.height().max(1.0)).unwrap_or(16.0 / 9.0);
    let r = (layouts::window_radius(cfg.display.layout, &view.params, cfg.window_radius(), view.zoom) as f32 - 2.0).max(1.0);
    // Looping: head for the copy of the selection nearest where the view is now, so going
    // from the last game to the first carries on forwards instead of sweeping back.
    let target = view.focus + view.rel(view.selected);
    let k = 1.0 - (-dt * 9.0 * cfg.animation.scroll_speed).exp();
    view.focus += (target - view.focus) * k;
    view.focus = view.focus.clamp(target - r, target + r);
    // The wall jumps straight to the new selection instead of sweeping across every item
    // in between (the items still move smoothly).
    if cfg.display.layout.is_grid() {
        view.focus = target;
    }
    view.spin += dt * cfg.animation.spin.to_radians();

    // The open case: closes when another game is selected; swings over ~0.7 s.
    if view.inspect.is_some() && view.inspect != view.selected_id {
        view.inspect_open = false;
    }
    // Closing the case: the insert closes up first (quickly), then the lid follows.
    if !view.inspect_open && view.insert_flipped > 0 {
        view.insert_flipped = 0;
        view.insert_hold = 0.28 / cfg.animation.speed.max(0.1);
    }
    view.insert_hold = (view.insert_hold - dt).max(0.0);
    if view.inspect.is_some() {
        let target = if view.inspect_open { 1.0 } else { 0.0 };
        // Opening follows the motion speed (about half a second by default).
        let step = if view.insert_hold > 0.0 && target == 0.0 { 0.0 } else { dt * cfg.animation.speed.max(0.1) / 0.7 };
        view.inspect_t = if target > view.inspect_t { (view.inspect_t + step).min(1.0) } else { (view.inspect_t - step).max(0.0) };
        if !view.inspect_open && view.inspect_t <= 0.0 {
            view.inspect = None;
        }
    }

    // A newly selected item always starts facing front.
    if view.last_selected != view.selected_id {
        view.last_selected = view.selected_id.clone();
        view.turn = Vec2::ZERO;
        view.turn_target = Vec2::ZERO;
        view.spin_vel = 0.0;
        view.free_turned = false;
    }
    // Turning follows the input closely (the item's own smoothing adds a little more lag).
    let kt = 1.0 - (-dt * 24.0 * cfg.animation.speed).exp();
    // After a flick: keep spinning, slow down, then settle on the nearer of front / back.
    if !view.turning && view.spin_vel != 0.0 {
        let v = view.spin_vel;
        view.turn_target.x += v * dt;
        view.spin_vel *= (-dt * 2.2).exp();
        if view.spin_vel.abs() < 2.5 {
            view.spin_vel = 0.0;
            view.snap_turn();
        }
    }
    let tt = view.turn_target;
    let cur = view.turn;
    view.turn = cur + (tt - cur) * kt;
    // Land exactly on the target once close, so a settled item counts as settled.
    if (view.turn_target - view.turn).length() < 1e-3 {
        view.turn = view.turn_target;
    }
    // Being turned right now (dragging, coasting after a flick, or still swinging to its snap).
    let turning_now = view.turning || view.spin_vel != 0.0 || view.turn != view.turn_target;
    // Keep the angle bounded without a visible jump.
    let tau = std::f32::consts::TAU;
    if view.turn_target.x.abs() > tau {
        let w = tau * view.turn_target.x.signum();
        view.turn_target.x -= w;
        view.turn.x -= w;
    }

    // Wall: follow the selection unless the user dragged the wall somewhere else.
    let cols = view.params.columns.max(1);
    if !view.grid_free {
        // Follow the selection's row; stay centred across.
        let n = view.view.len().max(1);
        // The row of the focus (on the wall it is the selection, in whichever lap is in view).
        view.grid_target = Vec2::new(grid_centre_x(cols, n), layouts::grid_pos_i(view.focus.round() as i64, cols).y);
    }
    let kg = 1.0 - (-dt * 9.0 * cfg.animation.scroll_speed).exp();
    let gt = view.grid_target;
    let gc = view.grid_center;
    view.grid_center = gc + (gt - gc) * kg;

    // The shelf board: a long plank along the angled shelf, under the games' spines.
    // (Put away while the easter egg has the stage.)
    let show_shelf = cfg.display.layout == LayoutKind::Shelf && !view.egg;
    for (mut tf, mut vis) in &mut decor {
        *vis = if show_shelf { Visibility::Visible } else { Visibility::Hidden };
        if show_shelf {
            let s = cfg.display.item_scale;
            let f = layouts::shelf_frame(&view.params, s);
            // As far as the games go into the distance, and a quarter of that the near way; the
            // texture fades it out over the last stretch at both ends.
            let far = (view.params.visible * 0.5 * view.zoom.max(1.0) * 1.7 + 10.0) * s;
            let near = far * SHELF_NEAR;
            let (thick, depth) = (1.2 * s, 22.0 * s);
            let centre = f.origin + f.axis * ((far - near) * 0.5) - f.normal * (depth * 0.5 - 0.5 * s) - Vec3::Y * thick * 0.5;
            let across = f.axis.cross(Vec3::Y);
            *tf = Transform::from_matrix(Mat4::from_cols((f.axis * (far + near)).extend(0.0), (Vec3::Y * thick).extend(0.0), (across * depth).extend(0.0), centre.extend(1.0)));
        }
    }

    let kk = 1.0 - (-dt * 12.0 * cfg.animation.speed).exp();
    let k_hover = 1.0 - (-dt * 32.0 * cfg.animation.speed).exp();
    let sway = cfg.animation.idle_sway;
    let grid = cfg.display.layout.is_grid();
    // The wall cannot be zoomed in past the point where a whole row fills the screen.
    if cfg.display.layout == LayoutKind::Grid {
        let zmin = layouts::wall_min_zoom(&view.params, cfg.display.spacing_scale, cfg.display.field_of_view.to_radians(), aspect);
        if view.zoom < zmin {
            view.zoom = zmin;
        }
    }
    let wrap = view.wrap_n.max(1) as f32;
    let lap_rows = (view.wrap_n / cols) as f32 * GRID_ROW;
    if !view.looping || laps.len() > 1024 {
        laps.clear();
    }
    // Switching to the wall or table: nothing is lifted until a game is picked.
    if *last_layout != Some(cfg.display.layout) {
        *last_layout = Some(cfg.display.layout);
        view.raised = false;
    }
    let lowered = grid && !view.raised;
    for (entity, item, mut tf, mut vis, mut fade) in &mut items {
        // Looping: which lap of the list this item is shown in (the copy nearest the middle).
        // The wall and table go by the row in the middle of the screen, the rest by the focus.
        let lap = if !view.looping {
            0.0
        } else if grid {
            // Whole rows move together, so go by rows.
            let middle_row = -view.grid_center.y / GRID_ROW;
            let row = (item.view_index / cols) as f32;
            ((middle_row - row) / (wrap / cols as f32)).round()
        } else {
            ((view.focus - item.view_index as f32) / wrap).round()
        };
        let ctx = LayoutCtx {
            kind: cfg.display.layout,
            p: &view.params,
            dims: item.dims,
            index: item.view_index,
            spacing_scale: cfg.display.spacing_scale,
            item_scale: cfg.display.item_scale,
            prefix: &view.prefix,
            // Shifted by whole laps so this item's copy is the one placed.
            grid_center: view.grid_center + Vec2::new(0.0, lap * lap_rows),
            zoom: view.zoom,
            starts: &view.starts,
            gap: view.pack_gap,
            period: if view.looping { view.lap } else { 0.0 },
            // Nothing raised: put the focus far away so no item lifts out. Otherwise the
            // focus as seen from this item (its nearest copy when looping).
            focus: if lowered { -1.0e6 } else { item.view_index as f32 - view.rel(item.view_index) },
        };
        let mut pose = layouts::pose(&ctx);
        // Moved to the other end of the list: appear there (fading in) rather than flying
        // across the screen to get there.
        let lap_i = lap as i64;
        let wrapped = laps.insert(entity, lap_i).is_some_and(|old| old != lap_i);
        if wrapped {
            tf.translation = pose.pos;
            tf.rotation = pose.rot;
            fade.value = 0.0;
        }
        let a = if lowered { 0.0 } else { (1.0 - view.rel(item.view_index).abs()).max(0.0) };
        if a > 0.0 {
            pose.pos.y += (t * 1.7).sin() * 0.22 * sway * a;
            pose.rot = pose.rot * Quat::from_rotation_y((t * 0.9).sin() * 0.05 * sway * a + view.spin * a);
        }
        if item.view_index == view.selected && view.turn != Vec2::ZERO {
            // Turned by the user: around the item's own axes, so a flip shows the back
            // square on, whatever angle the layout holds the item at.
            pose.rot = pose.rot * Quat::from_rotation_y(view.turn.x) * Quat::from_rotation_x(view.turn.y);
        }
        if view.hovered == Some(item.view_index) && launch.game.is_none() {
            if view.hover_spine {
                // Pulled a little way out of the stack, straight out past its spine, so the
                // neighbours stay where they are and easy to point at.
                pose.pos += pose.rot * (view.hover_normal * 0.5 * pose.scale);
            } else {
                // Lift the item under the cursor slightly towards you.
                pose.scale *= 1.02;
                pose.pos.z += 0.3;
            }
            let turned = item.view_index == view.selected && turning_now;
            if cfg.display.hover_tilt && view.hover_tilt != Vec3::ZERO && !turned {
                // Lean away from the cursor, and shift a touch away from it.
                pose.rot = pose.rot * Quat::from_scaled_axis(view.hover_tilt);
                pose.pos += pose.rot * (view.hover_away * 0.15 * pose.scale);
            }
        }
        let lp = launch.progress();
        if lp > 0.0 {
            let e = lp * lp * (3.0 - 2.0 * lp);
            let launching = launch.game.as_ref().is_some_and(|(id, _)| *id == item.game_id);
            if let Some(rig) = item.rig.as_ref().filter(|_| launching && launch.open) {
                // Turn to face the camera and come close enough for the open case to fill the
                // screen, sliding sideways as the lid swings open so it stays centred.
                let a = smooth(lp / 0.16);
                let o = smooth((lp - 0.10) / 0.30);
                let (centre, rot) = facing_spot(&view, &cfg, rig, pose.scale, aspect);
                let target = centre + rot * Vec3::X * (rig.open_shift * pose.scale * o);
                pose.pos = pose.pos.lerp(target, a);
                pose.rot = pose.rot.slerp(rot, a);
                pose.visible = true;
            } else if launching {
                let cam_d = view.params.cam_distance * view.zoom;
                match cfg.launch.style.as_str() {
                    "spin" => {
                        pose.rot = pose.rot * Quat::from_rotation_y(e * e * std::f32::consts::TAU * 2.0);
                        pose.pos.z += e * 8.0;
                        pose.pos.y += (lp * std::f32::consts::PI).sin() * 2.5;
                    }
                    "pulse" => {
                        pose.scale *= 1.0 + 0.22 * (lp * std::f32::consts::PI * 3.0).sin().abs() * (1.0 - 0.4 * lp);
                    }
                    "rise" => {
                        pose.pos.y += e * e * 40.0;
                        pose.rot = pose.rot * Quat::from_rotation_y(e * std::f32::consts::PI);
                    }
                    _ => {
                        // "zoom": a little anticipation, then it flies into the camera.
                        let back = (lp * 4.0).min(1.0) * (1.0 - lp) * 2.0;
                        pose.pos.z += -back + e * e * cam_d * 0.9;
                        pose.pos.y = pose.pos.y * (1.0 - e) + view.params.look_height * e;
                        pose.pos.x *= 1.0 - e;
                        pose.rot = pose.rot.slerp(Quat::IDENTITY, e) * Quat::from_rotation_z(e * 0.25);
                    }
                }
            } else if launch.open {
                // Clear the stage for the open case, whatever `recede` says.
                let e = smooth(lp / 0.2);
                pose.pos.z -= e * 45.0;
                pose.scale *= 1.0 - 0.5 * e;
            } else if cfg.launch.recede {
                pose.pos.z -= e * 14.0;
                pose.scale *= 1.0 - 0.2 * e;
            }
        } else if let Some(id) = view.inspect.clone() {
            let it = view.inspect_t;
            match item.rig.as_ref().filter(|_| id == item.game_id) {
                Some(rig) => {
                    // Opened for a look inside: face the camera, then let the user look round
                    // it (drag, plus a little parallax from where the cursor is).
                    let (a, o) = (inspect_approach(it), inspect_opened(it));
                    let (centre, face) = facing_spot(&view, &cfg, rig, pose.scale, aspect);
                    let look = view.inspect_rot + Vec2::new(view.inspect_cursor.x * 0.18, view.inspect_cursor.y * 0.12);
                    let rot = face * Quat::from_rotation_y(look.x * o) * Quat::from_rotation_x(look.y * o);
                    // Turn about the middle of the open case, not the closed case's centre.
                    let target = centre + rot * Vec3::X * (rig.open_shift * pose.scale * o);
                    pose.pos = pose.pos.lerp(target, a);
                    pose.rot = pose.rot.slerp(rot, a);
                    pose.visible = true;
                }
                None => {
                    let e = smooth(it / 0.6);
                    pose.pos.z -= e * 45.0;
                    pose.scale *= 1.0 - 0.5 * e;
                }
            }
        }
        // Clicked: pressed into the screen, then springs back out past its spot and settles,
        // straight along the view direction (no sideways or vertical movement).
        if let Some((i, at)) = view.bounce {
            let age = t - at;
            if i == item.view_index && age < 0.7 && launch.game.is_none() {
                let (cam_pos, look) = layouts::camera_for(&view.params);
                let towards_camera = (cam_pos - look).normalize_or(Vec3::Z);
                let spring = -(age * 18.0).sin() * (-age * 6.5).exp();
                pose.pos += towards_camera * (spring * 1.4 * pose.scale);
            }
        }
        // Fade in and out at the edges rather than popping (also fades new items in).
        let fk = 1.0 - (-dt * 8.0).exp();
        fade.value += (pose.fade - fade.value) * fk;
        if (pose.fade - fade.value).abs() < 0.003 {
            fade.value = pose.fade;
        }
        *vis = if pose.visible || fade.value > 0.01 { Visibility::Inherited } else { Visibility::Hidden };
        if tf.scale.x < 0.01 {
            tf.translation = pose.pos;
            tf.rotation = pose.rot;
        }
        // The game under the mouse follows the cursor's lean much more tightly than the
        // layout moves, so sweeping along an edge feels immediate.
        let hands_on = (view.hovered == Some(item.view_index) && launch.game.is_none())
            || (item.view_index == view.selected && view.turn != Vec2::ZERO)
            || (view.inspecting() && view.inspect.as_deref() == Some(item.game_id.as_str()) && view.inspect_t >= 1.0)
            || view.bounce.is_some_and(|(i, at)| i == item.view_index && t - at < 0.6);
        let k = if hands_on { k_hover } else { kk };
        tf.translation = tf.translation.lerp(pose.pos, k);
        // A turned item takes its rotation straight from the (already smoothed) turn: easing
        // it again would take the short way round and stutter during a fast spin.
        let spun = item.view_index == view.selected && turning_now && launch.game.is_none() && view.inspect.is_none();
        tf.rotation = if spun { pose.rot } else { tf.rotation.slerp(pose.rot, k) };
        tf.scale = tf.scale.lerp(Vec3::splat(pose.scale), k);
    }
}

/// Keeps the zoom (ctrl + wheel, also inside an open case) in the config, so it is the same
/// next time (saved by `autosave` after a short pause).
fn remember_zoom(view: Res<ViewState>, mut cfg: ResMut<Config>, mut persist: ResMut<Persist>) {
    if (view.zoom - cfg.display.zoom).abs() > 1e-3 {
        cfg.data.display.zoom = view.zoom;
        persist.mark_cfg();
    }
    // And the selected game, to start on it next time.
    if let Some(id) = view.selected_id.as_ref().filter(|id| **id != cfg.general.last_game) {
        cfg.data.general.last_game = id.clone();
        persist.mark_cfg();
    }
}

/// Printed art that just arrived, fading in over the plain placeholder colour: (material, 0..1).
#[derive(Resource, Default)]
pub struct CoverFades(Vec<(Handle<StandardMaterial>, f32)>);

/// Starts new cover art at fully transparent, so it fades in over the case instead of popping.
fn begin_cover_fade(m: &mut StandardMaterial, h: &Handle<StandardMaterial>, fades: &mut CoverFades) {
    m.base_color = m.base_color.with_alpha(0.0);
    m.alpha_mode = AlphaMode::Blend;
    if let Some(f) = fades.0.iter_mut().find(|(x, _)| x == h) {
        f.1 = 0.0;
    } else {
        fades.0.push((h.clone(), 0.0));
    }
}

fn run_cover_fades(time: Res<Time>, mut fades: ResMut<CoverFades>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let dt = time.delta_seconds().min(0.1);
    fades.0.retain_mut(|(h, t)| {
        *t = (*t + dt / 0.4).min(1.0);
        let Some(m) = mats.get_mut(&*h) else { return false };
        m.base_color = m.base_color.with_alpha(smooth(*t));
        if *t >= 1.0 {
            m.alpha_mode = AlphaMode::Opaque;
            return false;
        }
        true
    });
}

/// Gives partly faded items see-through copies of their materials (alpha scaled by the fade)
/// and puts the shared originals back once they are fully visible again.
#[allow(clippy::too_many_arguments)]
fn apply_fades(
    mut items: Query<(Entity, &mut render::Fade)>,
    children: Query<&Children>,
    interior: Query<(), With<render::Interior>>,
    media: Query<(), With<render::MediaPart>>,
    mut changed: EventReader<AssetEvent<StandardMaterial>>,
    mut changed_disc: EventReader<AssetEvent<DiscMaterial>>,
    mut std_h: Query<&mut Handle<StandardMaterial>>,
    mut disc_h: Query<&mut Handle<DiscMaterial>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut disc_mats: ResMut<Assets<DiscMaterial>>,
) {
    // Originals that changed this frame (a cover arrived, a glow, a fade-in step): their copies
    // are refreshed right away so nothing pops.
    let mut modified: HashSet<AssetId<StandardMaterial>> = HashSet::new();
    for e in changed.read() {
        if let AssetEvent::Modified { id } = e {
            modified.insert(*id);
        }
    }
    let mut modified_disc: HashSet<AssetId<DiscMaterial>> = HashSet::new();
    for e in changed_disc.read() {
        if let AssetEvent::Modified { id } = e {
            modified_disc.insert(*id);
        }
    }
    let see_through = |mode: AlphaMode| if matches!(mode, AlphaMode::Opaque | AlphaMode::Mask(_)) { AlphaMode::Blend } else { mode };
    for (root, mut f) in &mut items {
        if f.value >= 0.995 {
            if !f.std.is_empty() || !f.disc.is_empty() {
                for (e, orig, _) in f.std.drain(..) {
                    if let Ok(mut h) = std_h.get_mut(e) {
                        *h = orig;
                    }
                }
                for (e, orig, _) in f.disc.drain(..) {
                    if let Ok(mut h) = disc_h.get_mut(e) {
                        *h = orig;
                    }
                }
            }
            f.applied = 1.0;
            continue;
        }
        if f.std.is_empty() && f.disc.is_empty() {
            let mut stack = vec![root];
            while let Some(e) = stack.pop() {
                if let Ok(ch) = children.get(e) {
                    stack.extend(ch.iter().copied());
                }
                if let Ok(mut h) = std_h.get_mut(e) {
                    if let Some(m) = mats.get(&*h).cloned() {
                        let copy = mats.add(m);
                        f.std.push((e, h.clone(), copy.clone()));
                        *h = copy;
                    }
                }
                if let Ok(mut h) = disc_h.get_mut(e) {
                    if let Some(m) = disc_mats.get(&*h).cloned() {
                        let copy = disc_mats.add(m);
                        f.disc.push((e, h.clone(), copy.clone()));
                        *h = copy;
                    }
                }
            }
            f.applied = -1.0;
        }
        let stale = f.std.iter().any(|(_, o, _)| modified.contains(&o.id())) || f.disc.iter().any(|(_, o, _)| modified_disc.contains(&o.id()));
        if (f.value - f.applied).abs() > 0.005 || stale {
            let v = f.value;
            // Parts inside the case fade out well before the shell, so they never show through it.
            let inner = v.powi(6);
            // The disc or cartridge draws before its see-through case (see `render::MediaPart`).
            let behind = |e: Entity| if media.contains(e) { -12.0 } else { 0.0 };
            for (e, orig, copy) in &f.std {
                if let Some(mut m) = mats.get(orig).cloned() {
                    let a = if interior.contains(*e) { inner } else { v };
                    m.base_color = m.base_color.with_alpha(m.base_color.alpha() * a);
                    m.depth_bias += behind(*e);
                    m.alpha_mode = see_through(m.alpha_mode);
                    if let Some(c) = mats.get_mut(copy) {
                        *c = m;
                    }
                }
            }
            for (e, orig, copy) in &f.disc {
                if let Some(mut m) = disc_mats.get(orig).cloned() {
                    let a = if interior.contains(*e) { inner } else { v };
                    m.base.base_color = m.base.base_color.with_alpha(m.base.base_color.alpha() * a);
                    m.base.depth_bias += behind(*e);
                    m.base.alpha_mode = see_through(m.base.alpha_mode);
                    if let Some(c) = disc_mats.get_mut(copy) {
                        *c = m;
                    }
                }
            }
            f.applied = v;
        }
    }
}

/// The wall's middle column (in grid units), where the view stays horizontally.
fn grid_centre_x(cols: usize, n: usize) -> f32 {
    (cols.min(n).max(1) - 1) as f32 * 0.5
}

/// How far the shelf board reaches the near way, as a share of how far it reaches into the
/// distance.
const SHELF_NEAR: f32 = 0.25;

/// The shelf board: a unit slab (x along the shelf, -0.5..0.5) with its top and both long
/// edges, u running from the near end (0) to the far end (1) on every face, so the fade
/// texture works whichever side faces the viewer.
fn shelf_board() -> Mesh {
    use bevy::render::mesh::{Indices, PrimitiveTopology};
    use bevy::render::render_asset::RenderAssetUsages;
    let (mut pos, mut nor, mut uv, mut idx): (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) = (vec![], vec![], vec![], vec![]);
    let mut quad = |c: [[f32; 3]; 4], n: [f32; 3]| {
        let b = pos.len() as u32;
        for p in c {
            pos.push(p);
            nor.push(n);
            uv.push([p[0] + 0.5, 0.5]);
        }
        // Wind each quad towards its normal.
        let (p0, p1, p2) = (Vec3::from(c[0]), Vec3::from(c[1]), Vec3::from(c[2]));
        if (p1 - p0).cross(p2 - p0).dot(Vec3::from(n)) >= 0.0 {
            idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        } else {
            idx.extend_from_slice(&[b, b + 2, b + 1, b, b + 3, b + 2]);
        }
    };
    quad([[-0.5, 0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]], [0.0, 1.0, 0.0]);
    quad([[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]], [0.0, 0.0, 1.0]);
    quad([[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]], [0.0, 0.0, -1.0]);
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_indices(Indices::U32(idx));
    m
}

/// Opacity along the shelf board (u = 0 near end .. 1 far end): faded in over most of the
/// short near stretch, solid around the selection, fading out over the last 40% into the
/// distance.
fn shelf_fade_image() -> Image {
    let n = 256u32;
    let total = 1.0 + SHELF_NEAR;
    let near_in = SHELF_NEAR * 0.8 / total;
    let far_from = (total - 0.4) / total;
    let mut rgba = Vec::with_capacity((n * 4 * 4) as usize);
    for _ in 0..4 {
        for i in 0..n {
            let u = (i as f32 + 0.5) / n as f32;
            let a = smooth(u / near_in) * (1.0 - smooth((u - far_from) / (1.0 - far_from)));
            rgba.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    make_image(n, 4, rgba, false)
}

/// The zoom at which an open case fills exactly `open_case_size` of the screen (the original
/// default zoom); zooming in or out from it makes the open case bigger or smaller.
const OPEN_CASE_ZOOM: f32 = 1.25;

/// Where an open case sits: in front of the camera, facing it, filling the screen at zoom 1
/// and further away as you zoom out. Returns (centre of the open case, rotation facing the camera).
fn facing_spot(view: &ViewState, cfg: &Config, rig: &render::CaseRig, s: f32, aspect: f32) -> (Vec3, Quat) {
    let (cam_pos, look) = layouts::camera_for(&view.params);
    let cam = look + (cam_pos - look) * view.zoom;
    let dir = (look - cam).normalize_or(Vec3::NEG_Z);
    let tan = (cfg.display.field_of_view.to_radians() * 0.5).tan();
    // At the default zoom the open case fills `open_case_size` of the screen.
    let size = cfg.display.open_case_size.clamp(0.3, 1.0);
    let dist = (rig.open_size.x * s / (2.0 * tan * aspect * size)).max(rig.open_size.y * s / (2.0 * tan * size * 0.94)) * view.zoom / OPEN_CASE_ZOOM;
    (cam + dir * dist, Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation)
}

/// Moves `t` towards `want` (all the way with `k` = 1), without touching it once there.
fn ease(t: &mut Mut<Transform>, want: Transform, k: f32) {
    let close = |a: &Transform| a.translation.distance_squared(want.translation) < 1e-8 && a.rotation.dot(want.rotation).abs() > 1.0 - 1e-7;
    if close(t) {
        return;
    }
    let mut next = Transform { translation: t.translation.lerp(want.translation, k), rotation: t.rotation.slerp(want.rotation, k), scale: t.scale };
    if close(&next) {
        next.translation = want.translation;
        next.rotation = want.rotation;
    }
    **t = next;
}

/// Opens the launching case on its hinges, lifts the disc or cartridge off its holder and
/// carries it towards the camera, then away as if into the console. Closes again afterwards.
fn animate_rigs(
    time: Res<Time>,
    cfg: Res<Config>,
    mut view: ResMut<ViewState>,
    launch: Res<LaunchAnim>,
    items: Query<(&Item, &GlobalTransform)>,
    cams: Query<&GlobalTransform, With<MainCamera>>,
    mut parts: Query<&mut Transform, Without<Item>>,
) {
    use std::f32::consts::{FRAC_PI_2, PI};
    let dt = time.delta_seconds().min(0.1);
    let k = 1.0 - (-dt * 7.0).exp();
    let lp = launch.progress();
    // The disc in an open case: wheel spin that slows down, plus a slow turn while pointed at.
    if view.inspect.is_some() {
        let base = if view.inspect_hover == InspectHover::Media { 1.2 } else { 0.0 };
        let v = view.disc_spin_vel;
        view.disc_spin = (view.disc_spin + (v + base) * dt) % std::f32::consts::TAU;
        view.disc_spin_vel *= (-dt * 1.2).exp();
        if view.disc_spin_vel.abs() < 0.05 {
            view.disc_spin_vel = 0.0;
        }
    }
    let cam = cams.get_single().map(|c| c.translation()).unwrap_or(Vec3::ZERO);
    let tan = (cfg.display.field_of_view.to_radians() * 0.5).tan();
    for (item, gt) in &items {
        let Some(rig) = &item.rig else { continue };
        let active = launch.open && launch.game.as_ref().is_some_and(|(id, _)| *id == item.game_id);
        let p = if active { lp } else { 0.0 };
        let inspected = view.inspect.as_deref() == Some(item.game_id.as_str());
        // Both hinges turn together: the spine lies flat and the lid ends up beside the tray.
        let mut o = smooth((p - 0.10) / 0.30);
        if inspected {
            o = o.max(inspect_opened(view.inspect_t));
        }
        // The manual lifts on its spine edge when pointed at, the blank pages under its cover
        // fanning up after it, each a little less.
        if let Some(b) = rig.booklet {
            // Hinged on its clipped (outer) edge, so its free edge lifts off the lid.
            let open = inspected && view.inspect_open;
            let hovered = open && view.inspect_hover == InspectHover::Booklet;
            let lift = if hovered && !view.insert_left { -0.42 } else { 0.0 };
            let flipped = if open { view.insert_flipped } else { 0 };
            // Closing: the pages snap shut quickly, before the lid comes down on them.
            let kp = if inspected && !view.inspect_open { (k * 4.0).min(1.0) } else { k };
            // In use (pointed at, or pages turned): out of the lid.
            let engaged = if hovered || flipped > 0 { 1.0 } else { 0.0 };
            if let Some(r) = rig.booklet_rest {
                if let Ok(mut t) = parts.get_mut(r) {
                    let want = Transform { translation: rig.booklet_base + rig.booklet_flip * engaged, ..*t };
                    ease(&mut t, want, kp);
                }
            }
            let fans = [1.0, 0.62, 0.4, 0.22, 0.1];
            for (j, e) in std::iter::once(b).chain(rig.pages.iter().copied()).enumerate() {
                if let Ok(mut t) = parts.get_mut(e) {
                    // Turned sheets lie over the hinge, beyond the case (the top one lifts a
                    // little when pointed at); the rest fan up after the top one.
                    let angle = if j < flipped {
                        let top = j + 1 == flipped && hovered && view.insert_left;
                        -std::f32::consts::PI + if top { 0.42 } else { 0.0 }
                    } else {
                        lift * fans[(j - flipped).min(fans.len() - 1)]
                    };
                    // While in use the whole insert comes out of the lid (hinge above the rim), so
                    // pages turn clear of the case's walls and clips; the lifting one a touch more.
                    let p = (-angle / std::f32::consts::PI).clamp(0.0, 1.0);
                    let q = (-angle / 0.42).clamp(0.0, 1.0);
                    let at = rig.booklet_base + rig.booklet_flip * engaged + Vec3::new(0.0, 0.0, -0.05) * q * (1.0 - p);
                    let want = Transform { rotation: Quat::from_rotation_y(angle), translation: at, ..*t };
                    ease(&mut t, want, kp);
                }
            }
        }
        let hinge = Quat::from_rotation_y(-o * FRAC_PI_2);
        for e in [rig.spine, rig.lid] {
            if let Ok(mut t) = parts.get_mut(e) {
                let want = Transform { rotation: hinge, ..*t };
                ease(&mut t, want, if active || inspected { 1.0 } else { k });
            }
        }
        let Some(me) = rig.media else { continue };
        let Ok(mut t) = parts.get_mut(me) else { continue };
        if !active && inspected {
            // In the open case the game sits in its holder. Pointed at, a disc rises off its
            // hub and turns slowly; a cartridge lifts and tips. The wheel spins the disc.
            let rest = if view.inspect_open { rig.media_inside } else { rig.media_rest };
            let target = if view.inspect_hover == InspectHover::Media && view.inspect_open { 1.0 } else { 0.0 };
            let lift = view.media_lift + (target - view.media_lift) * (k * 1.6).min(1.0);
            view.media_lift = lift;
            let disc = rig.media_kind == MediaKind::Disc;
            let up = Vec3::Z * (0.4 + rig.half_depth * 0.3) * lift;
            let tilt = Quat::from_rotation_x(if disc { -0.12 } else { -0.3 } * lift);
            let spin = if disc { Quat::from_rotation_z(view.disc_spin) } else { Quat::IDENTITY };
            // Moving between rest and hub eases; the spin is applied as is, so a fast spin
            // never takes the short way round.
            let next = t.translation.lerp(rest.translation + up, (k * 1.6).min(1.0));
            let want = Transform { translation: next, rotation: tilt * spin, scale: t.scale };
            if *t != want {
                *t = want;
            }
            continue;
        }
        if !active {
            ease(&mut t, rig.media_rest, 1.0);
            continue;
        }
        let s = gt.compute_transform().scale.x.max(1e-4);
        let dist = gt.translation().distance(cam);
        // Held close enough to fill most of the screen height.
        let near = rig.media_size * s / (2.0 * tan * 0.75);
        let rest = if launch.from_open { rig.media_inside } else { rig.media_rest }.translation;
        let hold_z = ((dist - near) / s).max(rest.z + 3.0);
        let drop = near * tan * 1.5 / s + rig.media_size;
        let lift = smooth((p - 0.40) / 0.12);
        let out = smooth((p - 0.50) / 0.28);
        let ins = smooth((p - 0.80) / 0.20);
        let lifted = Vec3::new(rest.x, rest.y, rest.z.max(0.0) + 2.0);
        let held = Vec3::new(-rig.open_shift, 0.0, hold_z);
        let arc = (out * PI).sin();
        let mut pos = rest.lerp(lifted, lift).lerp(held, out) + Vec3::Y * arc * rig.media_size * 0.15;
        let mut rot = Quat::from_rotation_x(-0.45 * arc) * Quat::from_rotation_y(0.3 * arc);
        match rig.media_kind {
            MediaKind::Disc => {
                // Tips flat, label up, and slides down out of view into the drive.
                rot = Quat::from_rotation_x(-FRAC_PI_2 * ins) * rot * Quat::from_rotation_z(out * 0.8);
                pos += Vec3::new(0.0, -drop, rig.media_size * 0.3) * ins * ins;
            }
            MediaKind::Cart => {
                // Connector first, down into the slot.
                pos += Vec3::new(0.0, -drop, 0.0) * ins * ins;
            }
        }
        ease(&mut t, Transform::from_translation(pos).with_rotation(rot), 1.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn animate_camera(
    time: Res<Time>,
    view: Res<ViewState>,
    cfg: Res<Config>,
    bg: Res<BgState>,
    mut cam: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
    mut key: Query<&mut DirectionalLight, (With<KeyLight>, Without<RimLight>)>,
    mut rim: Query<&mut DirectionalLight, (With<RimLight>, Without<KeyLight>)>,
    mut ambient: ResMut<AmbientLight>,
) {
    let dt = time.delta_seconds().min(0.1);
    let (pos, look) = layouts::camera_for(&view.params);
    let pos = look + (pos - look) * view.zoom;
    // The easter egg's table has its own view.
    let (pos, look) = view.egg_cam.unwrap_or((pos, look));
    let k = 1.0 - (-dt * 4.0 * cfg.animation.speed).exp();
    if let Ok((mut tf, mut proj)) = cam.get_single_mut() {
        let want = Transform::from_translation(pos).looking_at(look, Vec3::Y);
        tf.translation = tf.translation.lerp(want.translation, k);
        tf.rotation = tf.rotation.slerp(want.rotation, k);
        if let Projection::Perspective(p) = proj.as_mut() {
            p.fov = cfg.display.field_of_view.to_radians();
        }
    }
    let b = cfg.general.brightness;
    ambient.brightness = 450.0 * b;
    if let Ok(mut l) = key.get_single_mut() {
        l.illuminance = 6500.0 * b;
        l.shadows_enabled = cfg.shadows();
    }
    if let Ok(mut l) = rim.get_single_mut() {
        l.illuminance = 2500.0 * b;
        let c = bg.current[1];
        // Lift towards white so the rim light stays visible on dark palettes.
        l.color = Color::linear_rgb((c.x * 0.7 + 0.3).min(1.0), (c.y * 0.7 + 0.3).min(1.0), (c.z * 0.7 + 0.3).min(1.0));
    }
}

fn adapt_background(view: Res<ViewState>, lib: Res<Library>, cfg: Res<Config>, mut bg: ResMut<BgState>) {
    bg.scroll = view.focus;
    bg.target = if cfg.background.adaptive {
        match view.current(&lib) {
            Some(g) => palette_from(g.dominant.unwrap_or_else(|| {
                let a = render::accent(&cfg, g.platform);
                [(a[0] * 1.6).min(1.0), (a[1] * 1.6).min(1.0), (a[2] * 1.6).min(1.0)]
            })),
            None => palette_from_config(&cfg),
        }
    } else {
        palette_from_config(&cfg)
    };
}

// ------------------------------------------------------------------------------------------------
// UI
// ------------------------------------------------------------------------------------------------

fn set(t: &mut Text, s: &str) {
    if t.sections[0].value != s {
        t.sections[0].value = s.to_string();
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_ui(
    time: Res<Time>,
    view: Res<ViewState>,
    lib: Res<Library>,
    cfg: Res<Config>,
    scanner: Res<Scanner>,
    mut toast: ResMut<Toast>,
    mut q: ParamSet<(
        Query<&mut Text, With<UiTop>>,
        Query<&mut Text, With<UiTitle>>,
        Query<&mut Text, With<UiSub>>,
        Query<&mut Text, With<UiDesc>>,
        Query<&mut Text, With<UiHints>>,
        Query<&mut Text, With<UiSearch>>,
        Query<&mut Text, With<UiToast>>,
    )>,
) {
    toast.ttl = (toast.ttl - time.delta_seconds()).max(0.0);
    let game = view.current(&lib);

    if let Ok(mut t) = q.p0().get_single_mut() {
        let scanning = if scanner.rx.is_some() { "   |   scanning..." } else { "" };
        let s = format!(
            "{}  |  {}  |  {}  |  {}/{}{}",
            cfg.display.layout.name(),
            cfg.display.item_style.name(),
            view.filter.map_or("All platforms", |p| p.display_name()),
            if view.view.is_empty() { 0 } else { view.selected + 1 },
            view.view.len(),
            scanning
        );
        set(&mut t, &s);
    }
    if let Ok(mut t) = q.p1().get_single_mut() {
        let s = match game {
            Some(g) => g.title.clone(),
            None if lib.games.is_empty() => "No games yet".to_string(),
            None => "No matches".to_string(),
        };
        set(&mut t, &s);
    }
    if let Ok(mut t) = q.p2().get_single_mut() {
        let s = match game {
            Some(g) => {
                let mut parts = Vec::new();
                if !g.installed {
                    parts.push("Not installed (Enter to install)".to_string());
                }
                parts.push(g.platform.display_name().to_string());
                if !g.year.is_empty() {
                    parts.push(g.year.clone());
                }
                if !g.genres.is_empty() {
                    parts.push(g.genres.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
                }
                if g.play_count > 0 {
                    parts.push(format!("played {}x", g.play_count));
                }
                if g.favorite {
                    parts.push("favorite".into());
                }
                parts.join("  |  ")
            }
            None => String::new(),
        };
        set(&mut t, &s);
    }
    if let Ok(mut t) = q.p3().get_single_mut() {
        let s = match game {
            Some(g) if cfg.general.show_info => g.description.chars().take(320).collect::<String>(),
            _ => String::new(),
        };
        set(&mut t, &s);
    }
    if let Ok(mut t) = q.p4().get_single_mut() {
        let k = &cfg.keys;
        let first = |v: &Vec<String>| v.first().cloned().unwrap_or_default();
        let s = if !cfg.general.show_hints {
            String::new()
        } else if view.inspecting() {
            format!(
                "[{} / double-click] launch   [{} / {} / {}] close   drag (left or right) to look around, wheel to spin the disc, ctrl+wheel to zoom",
                first(&k.launch),
                first(&k.open),
                first(&k.close),
                first(&k.next_layout)
            )
        } else if lib.games.is_empty() {
            "Drop a game, ROM or folder onto this window, or add [[scan.rom_dirs]] to config.toml and press F5".to_string()
        } else {
            format!(
                "[{}/{}] browse   [{}] open case   [{}] launch   [{}] flip   [{}/{}] turn   [{}] layout   [{}] style   [{}] background   [{}/{}] platform   [{}] favorite   [{}] search   [{}] settings   [{}] quit\nmouse: drag to move, right-drag to rotate, right-click to flip, click to select, double-click to launch, ctrl+wheel to zoom",
                first(&k.left),
                first(&k.right),
                first(&k.open),
                first(&k.launch),
                first(&k.flip),
                first(&k.turn_left),
                first(&k.turn_right),
                first(&k.next_layout),
                first(&k.next_style),
                first(&k.next_background),
                first(&k.prev_platform),
                first(&k.next_platform),
                first(&k.favorite),
                first(&k.search),
                first(&k.settings),
                first(&k.quit),
            )
        };
        set(&mut t, &s);
    }
    if let Ok(mut t) = q.p5().get_single_mut() {
        let s = if view.searching {
            format!("Search: {}_", view.search)
        } else if !view.search.is_empty() {
            format!("Filter: {}", view.search)
        } else {
            String::new()
        };
        set(&mut t, &s);
    }
    if let Ok(mut t) = q.p6().get_single_mut() {
        let s = if toast.ttl > 0.0 { toast.text.clone() } else { String::new() };
        set(&mut t, &s);
        t.sections[0].style.color = Color::srgba(0.6, 1.0, 0.8, (toast.ttl / 0.6).min(1.0));
    }
}

// ------------------------------------------------------------------------------------------------
// Config hot reload & persistence
// ------------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn config_watch(
    time: Res<Time>,
    mut watch: ResMut<ConfigWatch>,
    mut cfg: ResMut<Config>,
    mut view: ResMut<ViewState>,
    mut gfx: ResMut<Gfx>,
    mut winit: ResMut<bevy::winit::WinitSettings>,
    svc: ResMut<CoverService>,
    mut toast: ResMut<Toast>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    watch.timer += time.delta_seconds();
    if watch.timer < 1.0 {
        return;
    }
    watch.timer = 0.0;
    let m = mtime_of(&config_path());
    if m == watch.mtime {
        return;
    }
    watch.mtime = m;
    let was_potato = cfg.potato();
    *cfg = Config::load();
    if cfg.potato() != was_potato {
        // Load the covers again at the new texture size.
        view.covers.clear();
        view.backs.clear();
    }
    *winit = winit_settings(&cfg, windows.get_single().is_ok_and(|w| w.transparent));
    svc.set_settings(&cfg.cover_settings());
    if let Ok(mut w) = windows.get_single_mut() {
        apply_window(&mut w, &cfg);
    }
    gfx.clear();
    view.respawn = true;
    view.dirty = true;
    toast.show("Config reloaded");
}

fn autosave(
    time: Res<Time>,
    mut persist: ResMut<Persist>,
    lib: Res<Library>,
    cfg: Res<Config>,
    mut watch: ResMut<ConfigWatch>,
) {
    if !(persist.lib || persist.cfg) {
        return;
    }
    persist.timer -= time.delta_seconds();
    if persist.timer > 0.0 {
        return;
    }
    if persist.lib {
        lib.save();
        persist.lib = false;
    }
    if persist.cfg {
        cfg.save();
        watch.mtime = mtime_of(&config_path());
        persist.cfg = false;
    }
}

/// When Slipcase quits (Q / Esc, the window closed, closing after a launch), saves whatever
/// `autosave` has not got round to yet, so the last layout, style, zoom and so on stick.
fn save_on_exit(mut exits: EventReader<AppExit>, mut persist: ResMut<Persist>, lib: Res<Library>, cfg: Res<Config>) {
    if exits.read().next().is_none() {
        return;
    }
    if persist.lib {
        lib.save();
        persist.lib = false;
    }
    if persist.cfg {
        cfg.save();
        persist.cfg = false;
    }
}

pub fn winit_settings(cfg: &Config, overlay: bool) -> bevy::winit::WinitSettings {
    // The overlay sits over other apps, which usually have the focus, so it has to keep
    // drawing (it quits as soon as a game starts).
    if overlay {
        return bevy::winit::WinitSettings { focused_mode: bevy::winit::UpdateMode::Continuous, unfocused_mode: bevy::winit::UpdateMode::Continuous };
    }
    let fps = cfg.general.background_fps;
    let wait = if fps > 0.0 { std::time::Duration::from_secs_f32(1.0 / fps) } else { std::time::Duration::from_secs(3600) };
    bevy::winit::WinitSettings {
        focused_mode: bevy::winit::UpdateMode::Continuous,
        unfocused_mode: bevy::winit::UpdateMode::reactive_low_power(wait),
    }
}

// Re-exported so main.rs does not need to know about internals.
pub fn initial_msaa(cfg: &Config) -> Msaa {
    msaa_from(cfg.msaa_samples())
}

