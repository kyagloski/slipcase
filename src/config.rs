//! All user-facing settings. Everything has a default, so an empty or missing
//! `config.toml` works, and unknown/missing keys never break loading.
//! The file is hot-reloaded while the launcher is running.

use std::collections::HashMap;
use std::path::PathBuf;

use bevy::prelude::{KeyCode, Resource};
use serde::{Deserialize, Serialize};

use crate::platform::Platform;

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("SLIPCASE_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|d| d.join("slipcase")))
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&base);
    base
}

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("SLIPCASE_HOME")
        .map(|p| PathBuf::from(p).join("cache"))
        .or_else(|| dirs::cache_dir().map(|d| d.join("slipcase")))
        .unwrap_or_else(|| PathBuf::from("cache"));
    let _ = std::fs::create_dir_all(&base);
    base
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

// ------------------------------------------------------------------------------------------------
// Enums
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutKind {
    CoverFlow,
    Carousel,
    Stack,
    Pile,
    Shelf,
    Grid,
    Helix,
    /// Aurora-style: the selection in front, the rest twisting away in a spiral on each side.
    Spiral,
    /// Aurora-style: the selection in front, the rest in straight stacks receding on each side.
    Stacks,
    /// Aurora-style: two vertical piles of cases lying flat, spines out, left and right; the
    /// selection stands in the middle.
    Towers,
    /// Like `Towers`, with every case a little twisted and offset, spiralling down each pile.
    MessyTowers,
    /// Aurora-style: games lying face up on a table at an angle, close together in rows; the
    /// selection lifts off the table towards you.
    Table,
    /// Aurora-style: a row of games leaning back; the selection stands up straight.
    StandUp,
    /// Aurora-style: games standing side by side seen straight on, only their spines showing;
    /// the selection slides out and shows its cover.
    Spines,
}

impl LayoutKind {
    pub const ALL: [LayoutKind; 14] = [
        LayoutKind::CoverFlow,
        LayoutKind::Spiral,
        LayoutKind::Stacks,
        LayoutKind::Towers,
        LayoutKind::MessyTowers,
        LayoutKind::Carousel,
        LayoutKind::Stack,
        LayoutKind::Pile,
        LayoutKind::Shelf,
        LayoutKind::Grid,
        LayoutKind::Table,
        LayoutKind::StandUp,
        LayoutKind::Spines,
        LayoutKind::Helix,
    ];
    pub fn name(self) -> &'static str {
        match self {
            LayoutKind::CoverFlow => "Cover Flow",
            LayoutKind::Carousel => "Carousel",
            LayoutKind::Stack => "Stack",
            LayoutKind::Pile => "Pile",
            LayoutKind::Shelf => "Bookshelf",
            LayoutKind::Grid => "Wall",
            LayoutKind::Helix => "Helix",
            LayoutKind::Spiral => "Spiral",
            LayoutKind::Stacks => "Twin Stacks",
            LayoutKind::Towers => "Tower Stacks",
            LayoutKind::MessyTowers => "Messy Stacks",
            LayoutKind::Table => "Table",
            LayoutKind::StandUp => "Stand Up",
            LayoutKind::Spines => "Spines",
        }
    }
    /// Laid out in rows and columns (the wall and the table): Up / Down move a row, the wheel
    /// scrolls rows, dragging pans.
    pub fn is_grid(self) -> bool {
        matches!(self, LayoutKind::Grid | LayoutKind::Table)
    }
    pub fn cycle(self, step: i32) -> Self {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0) as i32;
        Self::ALL[(i + step).rem_euclid(Self::ALL.len() as i32) as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStyle {
    /// 3D game case with cover, spine and back.
    Case,
    /// Just the disc (or cartridge for cart-based platforms).
    Disc,
    /// Case with the disc peeking out of the top.
    CaseDisc,
    /// Flat 2D cover art.
    Flat,
}

impl ItemStyle {
    pub const ALL: [ItemStyle; 4] = [ItemStyle::Case, ItemStyle::Disc, ItemStyle::CaseDisc, ItemStyle::Flat];
    pub fn name(self) -> &'static str {
        match self {
            ItemStyle::Case => "Cases",
            ItemStyle::Disc => "Discs",
            ItemStyle::CaseDisc => "Cases + Discs",
            ItemStyle::Flat => "Flat covers",
        }
    }
    pub fn cycle(self, step: i32) -> Self {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0) as i32;
        Self::ALL[(i + step).rem_euclid(Self::ALL.len() as i32) as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundStyle {
    /// Blurred art (fanart / hero image / screenshot) of the selected game.
    GameArt,
    Aurora,
    Waves,
    Nebula,
    Gradient,
    Solid,
    /// Outrun: a neon grid rolling towards you.
    Synthwave,
    /// The cosmic web: filaments of galaxies between dark voids.
    Galactic,
    /// Like the PS2's main menu: glowing towers of blocks in a dark blue void.
    Ps2,
    /// Like the original Xbox dashboard: green smoke, a glowing orb and tubes.
    Xbox,
    /// A picture or animated GIF of your own (`background.custom_path`).
    Custom,
}

impl BackgroundStyle {
    pub const ALL: [BackgroundStyle; 11] = [
        BackgroundStyle::GameArt,
        BackgroundStyle::Aurora,
        BackgroundStyle::Waves,
        BackgroundStyle::Nebula,
        BackgroundStyle::Gradient,
        BackgroundStyle::Solid,
        BackgroundStyle::Synthwave,
        BackgroundStyle::Galactic,
        BackgroundStyle::Ps2,
        BackgroundStyle::Xbox,
        BackgroundStyle::Custom,
    ];
    /// Style number used by the background shader.
    pub fn index(self) -> f32 {
        match self {
            // Game art is drawn over a soft gradient.
            BackgroundStyle::GameArt => 3.0,
            BackgroundStyle::Aurora => 0.0,
            BackgroundStyle::Waves => 1.0,
            BackgroundStyle::Nebula => 2.0,
            BackgroundStyle::Gradient => 3.0,
            BackgroundStyle::Solid => 4.0,
            BackgroundStyle::Synthwave => 5.0,
            BackgroundStyle::Galactic => 6.0,
            BackgroundStyle::Ps2 => 7.0,
            BackgroundStyle::Xbox => 8.0,
            // Behind the picture (while it loads, or if it cannot be read): the solid colour.
            BackgroundStyle::Custom => 4.0,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            BackgroundStyle::GameArt => "Game art",
            BackgroundStyle::Aurora => "Aurora",
            BackgroundStyle::Waves => "Waves",
            BackgroundStyle::Nebula => "Nebula",
            BackgroundStyle::Gradient => "Gradient",
            BackgroundStyle::Solid => "Solid",
            BackgroundStyle::Synthwave => "Synthwave",
            BackgroundStyle::Galactic => "Galactic",
            BackgroundStyle::Ps2 => "PS2",
            BackgroundStyle::Xbox => "Xbox",
            BackgroundStyle::Custom => "Custom image",
        }
    }
    pub fn cycle(self, step: i32) -> Self {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0) as i32;
        Self::ALL[(i + step).rem_euclid(Self::ALL.len() as i32) as usize]
    }
}

// ------------------------------------------------------------------------------------------------
// Sections
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub fullscreen: bool,
    pub vsync: bool,
    /// 1, 2, 4 or 8.
    pub msaa: u32,
    pub window_width: f32,
    pub window_height: f32,
    pub minimize_on_launch: bool,
    /// Quit Slipcase once a game has started.
    pub close_on_launch: bool,
    /// No window border and no background: the cases float over your other windows.
    /// Closes when a game starts. Needs a restart (or start with `slipcase --overlay`).
    pub overlay: bool,
    /// The game selected when Slipcase last closed (its id); it starts there again.
    pub last_game: String,
    /// How the compositor blends the overlay: "auto" (picked per OS), "premultiplied",
    /// "postmultiplied" or "inherit". Try another if the overlay shows a black background.
    pub overlay_alpha: String,
    pub show_hints: bool,
    pub show_info: bool,
    pub bloom: bool,
    /// Multiplies all scene lighting.
    pub brightness: f32,
    pub shadows: bool,
    /// Rescan installed stores when the launcher starts.
    pub scan_on_start: bool,
    /// How many items to keep alive on each side of the selection.
    pub window_radius: usize,
    /// Frames per second while the window is in the background (0 = draw nothing until
    /// it is focused again, so a running game gets the whole GPU).
    pub background_fps: f32,
    /// Potato mode: much faster at the cost of looks everywhere. No background effects (a plain
    /// colour instead), no bloom, no anti-aliasing, no shadows, no outline, no disc rainbow,
    /// smaller cover textures and fewer games kept around the selection. The settings it
    /// overrides keep their own values for when it is turned off again.
    pub potato: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            fullscreen: false,
            vsync: true,
            msaa: 4,
            window_width: 1600.0,
            window_height: 900.0,
            minimize_on_launch: false,
            close_on_launch: false,
            overlay: false,
            overlay_alpha: "auto".into(),
            last_game: String::new(),
            show_hints: true,
            show_info: true,
            bloom: true,
            brightness: 1.0,
            shadows: false,
            scan_on_start: true,
            window_radius: 14,
            background_fps: 0.0,
            potato: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub layout: LayoutKind,
    pub item_style: ItemStyle,
    /// Fraction of the disc that sticks out of the case in `case_disc` style (0..1).
    pub disc_peek: f32,
    /// Scales every item.
    pub item_scale: f32,
    /// Gap between spines/cases is multiplied by this in every layout.
    pub spacing_scale: f32,
    pub field_of_view: f32,
    /// Make every case the same size (ignores per-platform geometry).
    pub uniform_sizes: bool,
    pub only_favorites: bool,
    /// The list loops: past the last game comes the first again, so you can scroll forever
    /// (not in the wall or table, which are laid out in rows).
    pub loop_list: bool,
    /// Show store games you own that are not installed (see `Scan::owned`).
    pub show_uninstalled: bool,
    /// all | alphabetical | recent | most_played
    pub sort: String,
    /// Case corners: "real" (each console's own case, as sold) or "custom" (`case_corner_radius`
    /// for every case).
    pub case_corners: String,
    /// Corner radius of game cases in cm when `case_corners = "custom"` (0 = sharp corners).
    pub case_corner_radius: f32,
    /// Camera distance (1 = the layout's own; higher is further out). Ctrl + wheel changes it
    /// and it is remembered.
    pub zoom: f32,
    /// Small thumb groove in the opening edge of cases.
    pub case_finger_notch: bool,
    /// Strength of the rainbow on the data side of discs (0 = plain mirror).
    pub disc_rainbow: f32,
    /// Mouse drag sensitivity (panning the wall, scrubbing, rotating).
    pub mouse_sensitivity: f32,
    /// Case spines: "auto" (scanned spine if LaunchBox has one, else logo spine),
    /// "generated" (always the logo spine), "simple" (plain title text).
    pub spine_style: String,
    /// Print each console's banner across the top of the cover and spine.
    pub platform_banners: bool,
    /// How much of the screen an opened case fills at the default zoom (0.3 .. 1).
    pub open_case_size: f32,
    /// Case plastic slowly cycles through the colours of the rainbow.
    pub rainbow_cases: bool,
    /// How the game under the mouse is marked: "outline" (soft coloured edge round it),
    /// "glow" (the cover lights up), "both" or "none".
    pub highlight: String,
    /// Hex colour of the outline.
    pub highlight_color: String,
    /// How far the outline fades out from the edge, in cm.
    pub highlight_width: f32,
    /// The game under the mouse leans away from the cursor.
    pub hover_tilt: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            layout: LayoutKind::CoverFlow,
            item_style: ItemStyle::Case,
            disc_peek: 0.38,
            item_scale: 1.0,
            spacing_scale: 1.0,
            field_of_view: 38.0,
            uniform_sizes: false,
            only_favorites: false,
            loop_list: false,
            show_uninstalled: false,
            sort: "alphabetical".into(),
            case_corners: "real".into(),
            case_corner_radius: 0.0,
            zoom: 1.25,
            case_finger_notch: true,
            disc_rainbow: 1.0,
            mouse_sensitivity: 1.0,
            spine_style: "auto".into(),
            platform_banners: true,
            open_case_size: 0.66,
            rainbow_cases: false,
            highlight: "outline".into(),
            highlight_color: "#2f8cff".into(),
            highlight_width: 0.16,
            hover_tilt: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Background {
    pub style: BackgroundStyle,
    /// Follow the dominant colour of the selected cover.
    pub adaptive: bool,
    /// Colours used when `adaptive = false` or nothing is selected (hex strings).
    pub color_a: String,
    pub color_b: String,
    pub color_c: String,
    pub speed: f32,
    pub intensity: f32,
    /// How fast the background colours blend to a new game's palette.
    pub blend_speed: f32,
    /// Game art style: opacity of the art over the animated background (0..1).
    pub art_opacity: f32,
    /// Game art style: brightness of the art (0..1), lower keeps the cases readable.
    pub art_brightness: f32,
    /// Game art style: blur amount (0 = sharp).
    pub art_blur: f32,
    /// Custom style: a picture or animated GIF (any image file) shown over the whole screen.
    pub custom_path: String,
    /// Overlay mode: still draw the background, but only in the middle of the screen,
    /// fading out to the desktop towards the edges.
    pub overlay_halfway: bool,
    /// Halfway overlay: how far out the background reaches (fraction of the screen, 0.2..1.5).
    pub overlay_halfway_size: f32,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            style: BackgroundStyle::GameArt,
            adaptive: true,
            color_a: "#0b1d3a".into(),
            color_b: "#1fd6a1".into(),
            color_c: "#7a3cff".into(),
            speed: 1.0,
            intensity: 1.0,
            blend_speed: 2.5,
            art_opacity: 0.9,
            art_brightness: 0.62,
            art_blur: 1.0,
            custom_path: String::new(),
            overlay_halfway: false,
            overlay_halfway_size: 0.75,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Launch {
    /// "zoom" (flies into the screen), "spin", "pulse", "rise", or "none".
    pub style: String,
    /// Seconds; the game starts when the animation ends.
    pub duration: f32,
    /// Strength of the flash at the end (0 = no flash).
    pub flash: f32,
    pub flash_color: String,
    /// Other items fall back while the selected one launches.
    pub recede: bool,
    /// Cases open, and the disc or cartridge is taken out before the game starts
    /// (replaces `style` for the case styles).
    pub open_case: bool,
    /// Seconds for the open-the-case animation.
    pub open_duration: f32,
}

impl Default for Launch {
    fn default() -> Self {
        Self {
            style: "zoom".into(),
            duration: 0.75,
            flash: 0.85,
            flash_color: "#ffffff".into(),
            recede: true,
            open_case: true,
            open_duration: 2.2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    /// Higher = snappier. 1.0 feels like a console dashboard.
    pub speed: f32,
    /// How fast the focus slides when you hold a direction key.
    pub scroll_speed: f32,
    /// Idle floating/sway of the selected item (0 disables).
    pub idle_sway: f32,
    /// Selected item slowly spins (degrees per second, 0 disables).
    pub spin: f32,
}

impl Default for Animation {
    fn default() -> Self {
        Self { speed: 1.25, scroll_speed: 1.2, idle_sway: 1.0, spin: 0.0 }
    }
}

/// Per-layout tuning. One struct shared by every layout; each layout uses what it needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutParams {
    pub spacing: f32,
    pub angle: f32,
    pub radius: f32,
    pub center_gap: f32,
    pub depth: f32,
    pub rows: usize,
    pub columns: usize,
    pub cam_distance: f32,
    pub cam_height: f32,
    pub look_height: f32,
    pub visible: f32,
}

impl LayoutParams {
    fn base() -> Self {
        Self {
            spacing: 1.0,
            angle: 62.0,
            radius: 14.0,
            center_gap: 1.0,
            depth: 1.0,
            rows: 3,
            columns: 6,
            cam_distance: 34.0,
            cam_height: 3.0,
            look_height: 0.0,
            visible: 9.0,
        }
    }
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self::base()
    }
}

pub fn default_layout_params(kind: LayoutKind) -> LayoutParams {
    let mut p = LayoutParams::base();
    match kind {
        LayoutKind::CoverFlow => {
            p.spacing = 6.0;
            p.angle = 62.0;
            p.center_gap = 9.0;
            p.depth = 9.0;
            p.cam_distance = 38.0;
            p.cam_height = 2.0;
            p.visible = 9.0;
        }
        LayoutKind::Carousel => {
            p.radius = 30.0;
            p.angle = 24.0;
            p.cam_distance = 26.0;
            p.cam_height = 5.0;
            p.look_height = 0.0;
            p.visible = 14.0;
        }
        LayoutKind::Stack => {
            // A neat tower of cases lying flat, spines out; the selected one slides out
            // to the left. spacing = gap between cases, center_gap = tower <-> selection
            // distance, depth = how far the selection comes forward, angle = its yaw.
            p.spacing = 0.12;
            p.center_gap = 17.0;
            p.depth = 7.0;
            p.angle = 14.0;
            p.cam_distance = 46.0;
            p.cam_height = 4.0;
            p.visible = 18.0;
        }
        LayoutKind::Pile => {
            p.spacing = 0.35;
            p.depth = 1.0;
            p.cam_distance = 44.0;
            p.cam_height = 24.0;
            p.look_height = -4.0;
            p.visible = 24.0;
        }
        LayoutKind::Shelf => {
            // One shelf at `angle` degrees to the viewer (positive: off to the left, negative: to
            // the right); the selection stands `depth` cm out in front of it; `visible` / 2
            // games along it into the distance.
            p.spacing = 0.25;
            p.angle = 35.0;
            p.depth = 12.0;
            p.center_gap = 15.0;
            p.cam_distance = 52.0;
            p.cam_height = 9.0;
            p.look_height = -1.0;
            p.visible = 80.0;
        }
        LayoutKind::Table => {
            // spacing = column pitch (cm, rows are a little deeper), angle = how far the table
            // is turned from square on, look_height = table height.
            p.spacing = 13.9;
            p.columns = 8;
            p.rows = 4;
            p.angle = 14.0;
            p.cam_distance = 50.0;
            p.cam_height = 44.0;
            p.look_height = -8.0;
            p.visible = 60.0;
        }
        LayoutKind::StandUp => {
            // spacing = distance between games, angle = how far they lean back, depth = how
            // far forward the selection comes as it stands up.
            p.spacing = 13.9;
            p.angle = 62.0;
            p.depth = 6.0;
            p.cam_distance = 52.0;
            p.cam_height = 12.0;
            p.look_height = -2.0;
            p.visible = 16.0;
        }
        LayoutKind::Spines => {
            // spacing = gap between spines, center_gap = room made either side of the
            // selection, depth = how far it slides out.
            p.spacing = 0.3;
            p.center_gap = 9.0;
            p.depth = 10.0;
            p.cam_distance = 50.0;
            p.cam_height = 0.0;
            p.look_height = 0.0;
            p.visible = 60.0;
        }
        LayoutKind::Grid => {
            p.spacing = 17.0;
            p.columns = 7;
            p.rows = 3;
            p.cam_distance = 62.0;
            p.cam_height = 0.0;
            p.visible = 40.0;
        }
        LayoutKind::Spiral => {
            // center_gap = distance of each spiral from the middle, radius = spiral radius,
            // angle = twist per item (degrees), spacing = depth step per item,
            // depth = how far the selection stands in front.
            p.center_gap = 17.5;
            p.radius = 5.0;
            p.angle = 32.0;
            p.spacing = 4.2;
            p.depth = 6.0;
            p.cam_distance = 44.0;
            p.cam_height = 2.0;
            p.visible = 22.0;
        }
        LayoutKind::Stacks => {
            // center_gap = distance of each stack from the middle, spacing = depth step,
            // radius = sideways drift per item, angle = how much the stacks face inwards.
            p.center_gap = 17.0;
            p.spacing = 3.6;
            p.radius = 2.4;
            p.angle = 28.0;
            p.depth = 6.0;
            p.cam_distance = 44.0;
            p.cam_height = 2.0;
            p.visible = 22.0;
        }
        LayoutKind::Towers | LayoutKind::MessyTowers => {
            // center_gap = how far each pile is from the middle, radius = height of the piles'
            // tops, angle = how far they turn towards the middle, depth = how far forward the
            // selection stands, spacing = gap between cases in a pile (cm).
            p.center_gap = 24.0;
            // Pile tops up near the top of the screen, well above the selection.
            p.radius = 20.0;
            p.angle = if kind == LayoutKind::Towers { 22.0 } else { 18.0 };
            p.depth = 14.0;
            p.spacing = if kind == LayoutKind::Towers { 0.08 } else { 0.18 };
            p.cam_distance = 62.0;
            p.cam_height = 4.0;
            p.look_height = 0.0;
            p.visible = 34.0;
        }
        LayoutKind::Helix => {
            p.radius = 24.0;
            p.angle = 30.0;
            p.spacing = 4.5;
            p.cam_distance = 30.0;
            p.cam_height = 0.0;
            p.visible = 10.0;
        }
    }
    p
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PlatformOverride {
    /// Overrides the launch command. `{rom}` expands to the quoted game path.
    pub command: Option<String>,
    /// width, height, depth in cm.
    pub case_size: Option<[f32; 3]>,
    pub disc_diameter: Option<f32>,
    /// Case corner radius in cm (overrides the console's own).
    pub corner_radius: Option<f32>,
    /// Hex colour for the spine and placeholder art.
    pub accent: Option<String>,
    /// Hex colour of the case plastic (`#rrggbb`, or `#rrggbbaa` for a translucent case).
    pub case_color: Option<String>,
    pub extensions: Option<Vec<String>>,
    pub libretro_system: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RomDir {
    pub path: String,
    /// Platform id (see `slipcase platforms`). Leave empty to guess from extensions.
    pub platform: String,
    pub recursive: bool,
    /// Extra extensions to accept in this folder (e.g. ["iso", "chd"]).
    pub extensions: Vec<String>,
}

impl Default for RomDir {
    fn default() -> Self {
        Self { path: String::new(), platform: String::new(), recursive: true, extensions: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Scan {
    pub steam: bool,
    pub epic: bool,
    pub lutris: bool,
    pub heroic: bool,
    /// GOG Galaxy, EA app / Origin, Ubisoft Connect (Windows).
    pub gog: bool,
    pub ea: bool,
    pub ubisoft: bool,
    /// Flatpak apps in the Game category (Linux), e.g. the Hytale launcher.
    pub flatpak: bool,
    /// Additional Steam library roots (folders containing `steamapps`).
    pub steam_libraries: Vec<String>,
    pub rom_dirs: Vec<RomDir>,
    /// Folders whose sub-folders each contain a game (looks for the biggest .exe / .sh / AppImage).
    pub game_dirs: Vec<String>,
    /// Title substrings to ignore (redistributables, tools).
    pub ignore: Vec<String>,
    /// Exact titles to hide that are not in the library (yet). A game with one of these titles
    /// is hidden as soon as a scan finds it. Hidden games are listed in the settings menu.
    pub hidden_titles: Vec<String>,
    /// Also list store games you own that are not installed (Steam, Heroic).
    pub owned: bool,
    /// Steam Web API key (https://steamcommunity.com/dev/apikey): lists every game on the
    /// account. Without one, only games played on this computer are found.
    pub steam_api_key: String,
    /// 64-bit Steam id to list; empty = the account last signed in here.
    pub steam_id: String,
}

impl Default for Scan {
    fn default() -> Self {
        Self {
            steam: true,
            epic: true,
            lutris: true,
            heroic: true,
            gog: true,
            ea: true,
            ubisoft: true,
            flatpak: true,
            steam_libraries: vec![],
            rom_dirs: vec![],
            game_dirs: vec![],
            ignore: vec![
                "Steamworks Common Redistributables".into(),
                "Steam Linux Runtime".into(),
                "Proton".into(),
                "Steamworks Shared".into(),
            ],
            hidden_titles: vec![],
            owned: true,
            steam_api_key: String::new(),
            steam_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Covers {
    pub enabled: bool,
    /// Tried in order. Covers that do not fit the case shape are kept only as a last resort.
    /// "steamgriddb" (key), "steam" (local Steam cache, Steam store API and CDN, store search for
    /// non-Steam PC games), "launchbox" (LaunchBox Games Database: box art, spines, backs, discs,
    /// logos, fanart), "libretro" (console box art), "thegamesdb" (key), "wikipedia".
    pub sources: Vec<String>,
    /// Free key from https://www.steamgriddb.com/profile/preferences/api (optional).
    pub steamgriddb_api_key: String,
    /// Key from https://api.thegamesdb.net (optional).
    pub thegamesdb_api_key: String,
    /// Fetch descriptions/genres from the Steam store API.
    pub fetch_metadata: bool,
    /// Longest side of cover textures in pixels.
    pub max_texture_size: u32,
}

impl Default for Covers {
    fn default() -> Self {
        Self {
            enabled: true,
            sources: default_sources(),
            steamgriddb_api_key: String::new(),
            thegamesdb_api_key: String::new(),
            fetch_metadata: true,
            max_texture_size: 768,
        }
    }
}

fn default_sources() -> Vec<String> {
    v(&["steamgriddb", "steam", "launchbox", "libretro", "thegamesdb", "wikipedia"])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub up: Vec<String>,
    pub down: Vec<String>,
    pub launch: Vec<String>,
    pub next_layout: Vec<String>,
    pub prev_layout: Vec<String>,
    pub next_style: Vec<String>,
    pub next_background: Vec<String>,
    pub next_platform: Vec<String>,
    pub prev_platform: Vec<String>,
    pub favorite: Vec<String>,
    pub search: Vec<String>,
    pub rescan: Vec<String>,
    pub toggle_info: Vec<String>,
    pub toggle_fullscreen: Vec<String>,
    pub page_up: Vec<String>,
    pub page_down: Vec<String>,
    pub quit: Vec<String>,
    /// Turn the selected item around to see its back.
    pub flip: Vec<String>,
    /// Rotate the selected item 90 degrees (spine, back, other edge).
    pub turn_left: Vec<String>,
    pub turn_right: Vec<String>,
    pub settings: Vec<String>,
    /// Open the selected case to look inside (pressed again: launch).
    pub open: Vec<String>,
    /// Close the open case.
    pub close: Vec<String>,
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            left: v(&["Left", "A"]),
            right: v(&["Right", "D"]),
            up: v(&["Up", "W"]),
            down: v(&["Down", "S"]),
            launch: v(&["Enter"]),
            next_layout: v(&["Tab", "L"]),
            prev_layout: v(&["K", "Shift+Tab"]),
            next_style: v(&["V"]),
            next_background: v(&["B"]),
            next_platform: v(&["PageDown", "]"]),
            prev_platform: v(&["PageUp", "["]),
            favorite: v(&["H"]),
            search: v(&["Slash"]),
            rescan: v(&["F5"]),
            toggle_info: v(&["I"]),
            toggle_fullscreen: v(&["F11"]),
            page_up: v(&["Home"]),
            page_down: v(&["End"]),
            quit: v(&["Q", "Escape"]),
            flip: v(&["F"]),
            turn_left: v(&["Z"]),
            turn_right: v(&["C"]),
            settings: v(&["M", "F1"]),
            open: v(&["Space"]),
            close: v(&["Backspace"]),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ConfigData {
    pub general: General,
    pub display: Display,
    pub background: Background,
    pub animation: Animation,
    pub launch: Launch,
    pub scan: Scan,
    pub covers: Covers,
    pub keys: Keys,
    /// `[layouts.coverflow]`, `[layouts.shelf]` ... Missing entries use built-in tuning.
    pub layouts: HashMap<String, LayoutParams>,
    /// `[platforms.gba]`, ...
    pub platforms: HashMap<String, PlatformOverride>,
}

#[derive(Resource, Debug, Clone, Default)]
pub struct Config {
    pub data: ConfigData,
}

impl std::ops::Deref for Config {
    type Target = ConfigData;
    fn deref(&self) -> &ConfigData {
        &self.data
    }
}
impl std::ops::DerefMut for Config {
    fn deref_mut(&mut self) -> &mut ConfigData {
        &mut self.data
    }
}

impl Config {
    pub fn load() -> Config {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<ConfigData>(&text) {
                Ok(mut data) => {
                    // Configs written by older versions list only the original three sources;
                    // upgrade them so the new sources are used.
                    if data.covers.sources == ["steamgriddb", "steam", "libretro"]
                        || data.covers.sources == ["steamgriddb", "steam", "libretro", "thegamesdb", "wikipedia"]
                    {
                        data.covers.sources = default_sources();
                    }
                    // F used to be favourite; it is now flip (favourite is the heart / H).
                    if data.keys.favorite == ["F"] {
                        data.keys.favorite = v(&["H"]);
                    }
                    if data.keys.flip == ["R"] {
                        data.keys.flip = v(&["F"]);
                    }
                    // "Hide this game" used to add the title to `ignore`; those are hidden titles now,
                    // so they show up in the hidden games list and only hide that exact game.
                    let tools = Scan::default().ignore;
                    let (keep, moved): (Vec<String>, Vec<String>) = data.scan.ignore.drain(..).partition(|t| tools.contains(t));
                    data.scan.ignore = keep;
                    for t in moved {
                        if !data.scan.hidden_titles.iter().any(|h| h.eq_ignore_ascii_case(&t)) {
                            data.scan.hidden_titles.push(t);
                        }
                    }
                    // Animations got quicker by default; configs still on the old defaults follow.
                    let old = |v: f32, d: f32| (v - d).abs() < 1e-4;
                    if old(data.launch.open_duration, 2.8) {
                        data.launch.open_duration = 2.2;
                    }
                    if old(data.launch.duration, 0.9) {
                        data.launch.duration = 0.75;
                    }
                    if old(data.animation.speed, 1.0) {
                        data.animation.speed = 1.25;
                    }
                    if old(data.animation.scroll_speed, 1.0) {
                        data.animation.scroll_speed = 1.2;
                    }
                    // The outline used to be a solid 3 mm band; it is now a thinner soft edge.
                    if (data.display.highlight_width - 0.3).abs() < 1e-4 {
                        data.display.highlight_width = 0.16;
                    }
                    // Q now quits and Escape opens the settings; the platform filter moved off Q/E.
                    // Shift+Tab goes back a layout.
                    if data.keys.prev_layout == ["K"] {
                        data.keys.prev_layout = v(&["K", "Shift+Tab"]);
                    }
                    // Space now opens the case; Enter launches.
                    if data.keys.launch == ["Enter", "Space"] {
                        data.keys.launch = v(&["Enter"]);
                    }
                    if data.keys.quit == ["Escape"] {
                        data.keys.quit = v(&["Q"]);
                    }
                    // Escape now quits (closing the settings or an open case first).
                    if data.keys.quit == ["Q"] {
                        data.keys.quit = v(&["Q", "Escape"]);
                    }
                    if data.keys.settings == ["Escape", "M", "F1"] {
                        data.keys.settings = v(&["M", "F1"]);
                    }
                    if data.keys.prev_platform == ["Q", "PageUp"] {
                        data.keys.prev_platform = v(&["PageUp", "["]);
                    }
                    if data.keys.next_platform == ["E", "PageDown"] {
                        data.keys.next_platform = v(&["PageDown", "]"]);
                    }
                    Config { data }
                }
                Err(e) => {
                    eprintln!("[slipcase] config error in {}: {e}", path.display());
                    Config::default()
                }
            },
            Err(_) => {
                let cfg = Config::default();
                cfg.save();
                cfg
            }
        }
    }

    /// Writes the full, commented-free config so every option is discoverable.
    pub fn save(&self) {
        if let Ok(text) = toml::to_string_pretty(&self.data) {
            let _ = std::fs::write(config_path(), text);
        }
    }

    // What is actually used for the settings potato mode overrides (see `General::potato`).

    pub fn potato(&self) -> bool {
        self.data.general.potato
    }

    pub fn msaa_samples(&self) -> u32 {
        if self.potato() { 1 } else { self.data.general.msaa }
    }

    pub fn bloom(&self) -> bool {
        self.data.general.bloom && !self.potato()
    }

    pub fn shadows(&self) -> bool {
        self.data.general.shadows && !self.potato()
    }

    /// Minimum items kept on each side of the selection (layouts still keep what they show).
    pub fn window_radius(&self) -> usize {
        if self.potato() { 0 } else { self.data.general.window_radius }
    }

    pub fn disc_rainbow(&self) -> f32 {
        if self.potato() { 0.0 } else { self.data.display.disc_rainbow.max(0.0) }
    }

    /// The hover highlight: potato mode swaps the outline (an extra see-through shell) for the glow.
    pub fn highlight(&self) -> &str {
        match self.data.display.highlight.as_str() {
            "outline" | "both" if self.potato() => "glow",
            h => h,
        }
    }

    /// Settings for the cover service: potato mode caps textures at 256 px.
    pub fn cover_settings(&self) -> Covers {
        let mut c = self.data.covers.clone();
        if self.potato() {
            c.max_texture_size = c.max_texture_size.min(256);
        }
        c
    }

    pub fn layout_params(&self, kind: LayoutKind) -> LayoutParams {
        let key = serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
        self.data.layouts.get(&key).cloned().unwrap_or_else(|| default_layout_params(kind))
    }

    pub fn platform_override(&self, p: Platform) -> Option<&PlatformOverride> {
        self.data.platforms.get(p.id())
    }

    pub fn command_for(&self, p: Platform) -> String {
        self.platform_override(p).and_then(|o| o.command.clone()).unwrap_or_else(|| p.default_command().to_string())
    }
}

pub fn parse_hex(s: &str) -> [f32; 3] {
    let s = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).unwrap_or(0x202020);
    [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0]
}

/// Parses a key name from the config into a bevy `KeyCode`.
pub fn parse_key(name: &str) -> Option<KeyCode> {
    let n = name.trim();
    let up = n.to_ascii_uppercase();
    if up.len() == 1 {
        let c = up.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(match c {
                'A' => KeyCode::KeyA, 'B' => KeyCode::KeyB, 'C' => KeyCode::KeyC, 'D' => KeyCode::KeyD,
                'E' => KeyCode::KeyE, 'F' => KeyCode::KeyF, 'G' => KeyCode::KeyG, 'H' => KeyCode::KeyH,
                'I' => KeyCode::KeyI, 'J' => KeyCode::KeyJ, 'K' => KeyCode::KeyK, 'L' => KeyCode::KeyL,
                'M' => KeyCode::KeyM, 'N' => KeyCode::KeyN, 'O' => KeyCode::KeyO, 'P' => KeyCode::KeyP,
                'Q' => KeyCode::KeyQ, 'R' => KeyCode::KeyR, 'S' => KeyCode::KeyS, 'T' => KeyCode::KeyT,
                'U' => KeyCode::KeyU, 'V' => KeyCode::KeyV, 'W' => KeyCode::KeyW, 'X' => KeyCode::KeyX,
                'Y' => KeyCode::KeyY, _ => KeyCode::KeyZ,
            });
        }
        if c.is_ascii_digit() {
            return Some(match c {
                '0' => KeyCode::Digit0, '1' => KeyCode::Digit1, '2' => KeyCode::Digit2, '3' => KeyCode::Digit3,
                '4' => KeyCode::Digit4, '5' => KeyCode::Digit5, '6' => KeyCode::Digit6, '7' => KeyCode::Digit7,
                '8' => KeyCode::Digit8, _ => KeyCode::Digit9,
            });
        }
    }
    Some(match up.as_str() {
        "LEFT" => KeyCode::ArrowLeft,
        "RIGHT" => KeyCode::ArrowRight,
        "UP" => KeyCode::ArrowUp,
        "DOWN" => KeyCode::ArrowDown,
        "ENTER" | "RETURN" => KeyCode::Enter,
        "SPACE" => KeyCode::Space,
        "TAB" => KeyCode::Tab,
        "ESCAPE" | "ESC" => KeyCode::Escape,
        "HOME" => KeyCode::Home,
        "END" => KeyCode::End,
        "PAGEUP" => KeyCode::PageUp,
        "PAGEDOWN" => KeyCode::PageDown,
        "SLASH" => KeyCode::Slash,
        "[" | "BRACKETLEFT" => KeyCode::BracketLeft,
        "]" | "BRACKETRIGHT" => KeyCode::BracketRight,
        "BACKSPACE" => KeyCode::Backspace,
        "F1" => KeyCode::F1, "F2" => KeyCode::F2, "F3" => KeyCode::F3, "F4" => KeyCode::F4,
        "F5" => KeyCode::F5, "F6" => KeyCode::F6, "F7" => KeyCode::F7, "F8" => KeyCode::F8,
        "F9" => KeyCode::F9, "F10" => KeyCode::F10, "F11" => KeyCode::F11, "F12" => KeyCode::F12,
        _ => return None,
    })
}

/// A binding: "Tab", or with Shift held: "Shift+Tab".
pub fn parse_binding(name: &str) -> Option<(bool, KeyCode)> {
    let n = name.trim();
    match n.split_once('+') {
        Some((m, k)) if m.trim().eq_ignore_ascii_case("shift") && !k.trim().is_empty() => parse_key(k).map(|k| (true, k)),
        _ => parse_key(n).map(|k| (false, k)),
    }
}

fn shift_held(input: &bevy::input::ButtonInput<KeyCode>) -> bool {
    input.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])
}

/// Keys pressed this frame as part of a Shift+ binding in `lists`: their plain bindings
/// should not fire as well (Shift+Tab is not also Tab).
pub fn shift_combos(lists: &[&Vec<String>], input: &bevy::input::ButtonInput<KeyCode>) -> Vec<KeyCode> {
    if !shift_held(input) {
        return Vec::new();
    }
    lists
        .iter()
        .flat_map(|l| l.iter())
        .filter_map(|k| parse_binding(k))
        .filter(|(shift, k)| *shift && input.just_pressed(*k))
        .map(|(_, k)| k)
        .collect()
}

/// Was any binding in `list` pressed this frame (Shift+ bindings need Shift held)?
pub fn keys_pressed(list: &[String], input: &bevy::input::ButtonInput<KeyCode>) -> bool {
    keys_pressed_except(list, input, &[])
}

/// Like `keys_pressed`, but plain bindings of keys in `taken` (used by a Shift+ binding this
/// frame) do not count.
pub fn keys_pressed_except(list: &[String], input: &bevy::input::ButtonInput<KeyCode>, taken: &[KeyCode]) -> bool {
    list.iter().filter_map(|k| parse_binding(k)).any(|(shift, k)| {
        input.just_pressed(k) && if shift { shift_held(input) } else { !taken.contains(&k) }
    })
}

pub fn keys_held(list: &[String], input: &bevy::input::ButtonInput<KeyCode>) -> bool {
    list.iter().filter_map(|k| parse_binding(k)).any(|(shift, k)| input.pressed(k) && (!shift || shift_held(input)))
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use bevy::input::ButtonInput;

    #[test]
    fn shift_tab_is_not_tab() {
        let next = vec!["Tab".to_string(), "L".to_string()];
        let prev = vec!["K".to_string(), "Shift+Tab".to_string()];
        let mut input = ButtonInput::<KeyCode>::default();
        input.press(KeyCode::Tab);
        let taken = shift_combos(&[&next, &prev], &input);
        assert!(keys_pressed_except(&next, &input, &taken) && !keys_pressed_except(&prev, &input, &taken));
        let mut input = ButtonInput::<KeyCode>::default();
        input.press(KeyCode::ShiftLeft);
        input.press(KeyCode::Tab);
        let taken = shift_combos(&[&next, &prev], &input);
        assert!(!keys_pressed_except(&next, &input, &taken) && keys_pressed_except(&prev, &input, &taken));
        assert_eq!(parse_binding("shift + tab"), Some((true, KeyCode::Tab)));
    }
}
