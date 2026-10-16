//! In-app settings menu: every display option, covers, launching, and adding / editing
//! games, without touching config.toml. Opens with Esc / M / F1, the gear button, or Start
//! on a gamepad.
//!
//! Keyboard: Up/Down select, Left/Right change, Enter toggles / edits / runs, Esc closes.
//! Mouse: hover to select, left click = next / run, right click = previous, wheel scrolls.

use std::path::PathBuf;

use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use crossbeam_channel::Receiver;

use crate::app::{apply_window, start_scan, Persist, Scanner, Toast, ViewState};
use crate::config::{keys_pressed, Config, ConfigData, ItemStyle, RomDir};
use crate::covers::CoverService;
use crate::library::Library;
use crate::platform::Platform;
use crate::render::Gfx;

// ------------------------------------------------------------------------------------------------
// Entries
// ------------------------------------------------------------------------------------------------

/// What has to be refreshed after a value changes.
#[derive(Clone, Copy, PartialEq)]
enum Effect {
    None,
    /// Re-filter / re-sort / re-layout.
    View,
    /// Rebuild every 3D item (sizes, styles, materials).
    Respawn,
    Window,
    Covers,
    /// Go to the default zoom.
    Zoom,
    /// Only applies after a restart: offer one.
    Restart,
    /// New random case colours: re-roll the seed and rebuild every item.
    Rainbow,
    /// Re-filter; scan first if no uninstalled games have been found yet.
    Uninstalled,
    /// Potato mode: covers at the new texture size, items rebuilt without the costly extras.
    Potato,
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
    AddFiles,
    AddFolder,
    AddRomFolder,
    Rescan,
    SetCover,
    RefetchCover,
    ChooseArt,
    BackgroundImage,
    BrowseLocation,
    HideGame,
}

enum Kind {
    Header,
    Toggle(fn(&mut ConfigData) -> &mut bool),
    Num { f: fn(&mut ConfigData) -> &mut f32, min: f32, max: f32, step: f32 },
    Choice { get: fn(&ConfigData) -> String, set: fn(&mut ConfigData, i32) },
    Text(fn(&mut ConfigData) -> &mut String),
    /// Membership of a cover source in `covers.sources`.
    Source(&'static str),
    Act(Act),
    /// Platform used for the next "add ROM folder".
    RomPlatform,
    GameTitle,
    GamePlatform,
    GameFavorite,
    /// File, program or URI the game launches (empty = the detected one).
    GameLocation,
    /// Extra launch arguments.
    GameArgs,
    /// A hidden game in the library (by id); activating it unhides it.
    HiddenGame(String),
    /// A hidden title no scan has found yet; activating it removes it from the list.
    HiddenTitle(String),
    /// A line of text that cannot be selected.
    Note(&'static str),
    /// Shows / hides the advanced settings of the category with this header.
    Advanced(&'static str),
    /// A colour (hex string), set with the colour picker.
    Color(fn(&mut ConfigData) -> &mut String),
}

struct Entry {
    label: &'static str,
    kind: Kind,
    effect: Effect,
    /// Technical setting, listed under the category's "Advanced" row.
    advanced: bool,
}

/// Marks a setting as advanced (shown when its category's "Advanced" row is opened).
fn adv(mut e: Entry) -> Entry {
    e.advanced = true;
    e
}

macro_rules! header {
    ($l:expr) => {
        Entry { label: $l, kind: Kind::Header, effect: Effect::None, advanced: false }
    };
}
macro_rules! toggle {
    ($l:expr, $($p:ident).+, $e:expr) => {
        Entry { label: $l, kind: Kind::Toggle({ fn f(c: &mut ConfigData) -> &mut bool { &mut c.$($p).+ } f }), effect: $e, advanced: false }
    };
}
macro_rules! num {
    ($l:expr, $($p:ident).+, $min:expr, $max:expr, $step:expr, $e:expr) => {
        Entry {
            label: $l,
            kind: Kind::Num { f: { fn f(c: &mut ConfigData) -> &mut f32 { &mut c.$($p).+ } f }, min: $min, max: $max, step: $step },
            effect: $e,
            advanced: false,
        }
    };
}
macro_rules! color {
    ($l:expr, $($p:ident).+) => {
        Entry { label: $l, kind: Kind::Color({ fn f(c: &mut ConfigData) -> &mut String { &mut c.$($p).+ } f }), effect: Effect::None, advanced: false }
    };
}
macro_rules! text {
    ($l:expr, $($p:ident).+, $e:expr) => {
        Entry { label: $l, kind: Kind::Text({ fn f(c: &mut ConfigData) -> &mut String { &mut c.$($p).+ } f }), effect: $e, advanced: false }
    };
}
/// A string field that cycles through fixed options.
macro_rules! options {
    ($l:expr, $($p:ident).+, [$($o:expr),+], $e:expr) => {
        Entry {
            label: $l,
            kind: Kind::Choice {
                get: { fn g(c: &ConfigData) -> String { c.$($p).+.clone() } g },
                set: {
                    fn s(c: &mut ConfigData, d: i32) {
                        let opts: &[&str] = &[$($o),+];
                        let i = opts.iter().position(|o| *o == c.$($p).+).unwrap_or(0) as i32;
                        c.$($p).+ = opts[(i + d).rem_euclid(opts.len() as i32) as usize].to_string();
                    }
                    s
                },
            },
            effect: $e,
            advanced: false,
        }
    };
}

const HIDDEN_HEADER: &str = "Hidden games";

/// Every named category header (the ones that can be folded).
fn category_headers() -> Vec<&'static str> {
    entries().iter().filter(|e| matches!(e.kind, Kind::Header) && !e.label.is_empty()).map(|e| e.label).collect()
}

/// Rows for the "Hidden games" section: one per hidden game, then titles not found yet.
fn hidden_entries(lib: &Library, cfg: &Config) -> Vec<Entry> {
    let mut games: Vec<&crate::library::Game> = lib.games.iter().filter(|g| g.hidden).collect();
    games.sort_by_key(|g| g.title.to_lowercase());
    let mut out: Vec<Entry> = games.into_iter().map(|g| Entry { label: "", kind: Kind::HiddenGame(g.id.clone()), effect: Effect::View, advanced: false }).collect();
    out.extend(cfg.scan.hidden_titles.iter().map(|t| Entry { label: "", kind: Kind::HiddenTitle(t.clone()), effect: Effect::None, advanced: false }));
    if out.is_empty() {
        out.push(Entry { label: "", kind: Kind::Note("No hidden games"), effect: Effect::None, advanced: false });
    }
    out
}

fn cycle_list<T: PartialEq + Copy>(all: &[T], cur: T, d: i32) -> T {
    let i = all.iter().position(|x| *x == cur).unwrap_or(0) as i32;
    all[(i + d).rem_euclid(all.len() as i32) as usize]
}

fn entries() -> Vec<Entry> {
    vec![
        header!("Library"),
        Entry { label: "Add game files...", kind: Kind::Act(Act::AddFiles), effect: Effect::None, advanced: false },
        Entry { label: "Add a folder of games...", kind: Kind::Act(Act::AddFolder), effect: Effect::None, advanced: false },
        Entry { label: "ROM folder platform", kind: Kind::RomPlatform, effect: Effect::None, advanced: true },
        Entry { label: "Add ROM folder (keeps scanning it)...", kind: Kind::Act(Act::AddRomFolder), effect: Effect::None, advanced: true },
        Entry { label: "Rescan all stores and folders", kind: Kind::Act(Act::Rescan), effect: Effect::None, advanced: false },
        adv(toggle!("Scan Steam", scan.steam, Effect::None)),
        adv(toggle!("Scan Epic", scan.epic, Effect::None)),
        adv(toggle!("Scan Heroic", scan.heroic, Effect::None)),
        adv(toggle!("Scan Lutris", scan.lutris, Effect::None)),
        adv(toggle!("Scan GOG Galaxy", scan.gog, Effect::None)),
        adv(toggle!("Scan EA app / Origin", scan.ea, Effect::None)),
        adv(toggle!("Scan Ubisoft Connect", scan.ubisoft, Effect::None)),
        adv(toggle!("Scan Flatpak games", scan.flatpak, Effect::None)),
        adv(toggle!("Rescan when starting", general.scan_on_start, Effect::None)),
        adv(toggle!("Find owned games that are not installed", scan.owned, Effect::None)),
        adv(text!("Steam Web API key (lists every owned game)", scan.steam_api_key, Effect::None)),
        header!("Selected game"),
        Entry { label: "Title", kind: Kind::GameTitle, effect: Effect::View, advanced: false },
        Entry { label: "Platform", kind: Kind::GamePlatform, effect: Effect::Respawn, advanced: true },
        Entry { label: "Favorite", kind: Kind::GameFavorite, effect: Effect::View, advanced: false },
        Entry { label: "Location (Enter to type, empty = detected)", kind: Kind::GameLocation, effect: Effect::None, advanced: true },
        Entry { label: "Browse for location...", kind: Kind::Act(Act::BrowseLocation), effect: Effect::None, advanced: true },
        Entry { label: "Launch arguments", kind: Kind::GameArgs, effect: Effect::None, advanced: true },
        Entry { label: "Choose art (cover, disc, insert)...", kind: Kind::Act(Act::ChooseArt), effect: Effect::None, advanced: false },
        Entry { label: "Choose cover image...", kind: Kind::Act(Act::SetCover), effect: Effect::None, advanced: true },
        Entry { label: "Download cover again", kind: Kind::Act(Act::RefetchCover), effect: Effect::None, advanced: true },
        Entry { label: "Hide this game", kind: Kind::Act(Act::HideGame), effect: Effect::None, advanced: false },
        // Filled in from the library, see `hidden_entries`.
        header!(HIDDEN_HEADER),
        header!("Layout"),
        Entry {
            label: "Layout",
            kind: Kind::Choice {
                get: |c| c.display.layout.name().to_string(),
                set: |c, d| c.display.layout = c.display.layout.cycle(d),
            },
            effect: Effect::View,
            advanced: false,
        },
        Entry {
            label: "Show as",
            kind: Kind::Choice {
                get: |c| c.display.item_style.name().to_string(),
                set: |c, d| c.display.item_style = cycle_list(&ItemStyle::ALL, c.display.item_style, d),
            },
            effect: Effect::Respawn,
            advanced: false,
        },
        options!("Sort by", display.sort, ["alphabetical", "recent", "most_played", "platform"], Effect::View),
        toggle!("Only favorites", display.only_favorites, Effect::View),
        toggle!("Loop the list (scroll forever)", display.loop_list, Effect::View),
        toggle!("Show games that are not installed", display.show_uninstalled, Effect::Uninstalled),
        num!("Item size", display.item_scale, 0.4, 2.5, 0.05, Effect::View),
        num!("Zoom", display.zoom, 0.45, 4.0, 0.05, Effect::Zoom),
        adv(num!("Spacing", display.spacing_scale, 0.4, 2.5, 0.05, Effect::View)),
        adv(num!("Field of view", display.field_of_view, 15.0, 90.0, 1.0, Effect::None)),
        adv(toggle!("Same size for every case", display.uniform_sizes, Effect::Respawn)),
        adv(num!("Mouse sensitivity", display.mouse_sensitivity, 0.1, 4.0, 0.1, Effect::None)),
        options!("Highlight", display.highlight, ["outline", "glow", "both", "none"], Effect::None),
        options!(
            "Highlight colour",
            display.highlight_color,
            ["#2f8cff", "#00d0ff", "#ffffff", "#3ddc84", "#ffc531", "#ff4f6d", "#b36bff"],
            Effect::None
        ),
        adv(num!("Highlight width (cm)", display.highlight_width, 0.04, 1.0, 0.02, Effect::Respawn)),
        adv(toggle!("Lean away from the mouse", display.hover_tilt, Effect::None)),
        header!("Cases and discs"),
        options!("Spines", display.spine_style, ["auto", "generated", "simple"], Effect::Respawn),
        toggle!("Console banners", display.platform_banners, Effect::Respawn),
        num!("Open case size", display.open_case_size, 0.3, 1.0, 0.02, Effect::None),
        adv(options!("Case corners", display.case_corners, ["real", "custom"], Effect::Respawn)),
        adv(num!("Custom corner radius (cm)", display.case_corner_radius, 0.0, 1.5, 0.05, Effect::Respawn)),
        adv(toggle!("Thumb groove", display.case_finger_notch, Effect::Respawn)),
        adv(num!("Disc rainbow", display.disc_rainbow, 0.0, 3.0, 0.1, Effect::Respawn)),
        adv(num!("Disc sticking out of case", display.disc_peek, 0.0, 0.9, 0.02, Effect::Respawn)),
        adv(toggle!("Rainbow cases", display.rainbow_cases, Effect::Rainbow)),
        header!("Background"),
        Entry {
            label: "Style",
            kind: Kind::Choice {
                get: |c| c.background.style.name().to_string(),
                set: |c, d| c.background.style = c.background.style.cycle(d),
            },
            effect: Effect::None,
            advanced: false,
        },
        adv(num!("Game art opacity", background.art_opacity, 0.0, 1.0, 0.05, Effect::None)),
        adv(num!("Game art brightness", background.art_brightness, 0.0, 1.0, 0.05, Effect::None)),
        adv(num!("Game art blur (new images)", background.art_blur, 0.0, 4.0, 0.25, Effect::None)),
        toggle!("Colours follow the cover", background.adaptive, Effect::None),
        Entry { label: "Choose a background picture or GIF...", kind: Kind::Act(Act::BackgroundImage), effect: Effect::None, advanced: false },
        adv(text!("Background picture (file)", background.custom_path, Effect::None)),
        color!("Colour: dark", background.color_a),
        color!("Colour: main", background.color_b),
        color!("Colour: accent", background.color_c),
        toggle!("Overlay: no window or background", general.overlay, Effect::Restart),
        toggle!("Overlay halfway: background fades out from the middle", background.overlay_halfway, Effect::None),
        adv(num!("Halfway background size", background.overlay_halfway_size, 0.2, 1.5, 0.05, Effect::None)),
        adv(num!("Animation speed", background.speed, 0.0, 4.0, 0.1, Effect::None)),
        adv(num!("Intensity", background.intensity, 0.0, 2.0, 0.05, Effect::None)),
        header!("Launching"),
        adv(options!("Launch animation", launch.style, ["zoom", "spin", "pulse", "rise", "none"], Effect::None)),
        adv(num!("Animation length (s)", launch.duration, 0.2, 3.0, 0.1, Effect::None)),
        adv(num!("Flash", launch.flash, 0.0, 1.0, 0.05, Effect::None)),
        options!("Flash colour", launch.flash_color, ["#ffffff", "#000000", "#9fe8ff", "#ffd27a", "#b7ffb0"], Effect::None),
        adv(toggle!("Other games fall back", launch.recede, Effect::None)),
        toggle!("Open the case and take the game out", launch.open_case, Effect::None),
        adv(num!("Open-case length (s)", launch.open_duration, 1.0, 6.0, 0.1, Effect::None)),
        adv(toggle!("Minimize after launching", general.minimize_on_launch, Effect::None)),
        toggle!("Close after launching", general.close_on_launch, Effect::None),
        header!("Motion"),
        num!("Speed", animation.speed, 0.2, 4.0, 0.1, Effect::None),
        adv(num!("Scroll speed", animation.scroll_speed, 0.2, 4.0, 0.1, Effect::None)),
        adv(num!("Idle sway", animation.idle_sway, 0.0, 3.0, 0.1, Effect::None)),
        adv(num!("Spin selected (deg/s)", animation.spin, 0.0, 180.0, 5.0, Effect::None)),
        header!("Covers"),
        toggle!("Download covers", covers.enabled, Effect::Covers),
        Entry { label: "Steam", kind: Kind::Source("steam"), effect: Effect::Covers, advanced: true },
        Entry { label: "LaunchBox (spines, backs, discs)", kind: Kind::Source("launchbox"), effect: Effect::Covers, advanced: true },
        Entry { label: "libretro thumbnails", kind: Kind::Source("libretro"), effect: Effect::Covers, advanced: true },
        Entry { label: "Wikipedia", kind: Kind::Source("wikipedia"), effect: Effect::Covers, advanced: true },
        Entry { label: "SteamGridDB", kind: Kind::Source("steamgriddb"), effect: Effect::Covers, advanced: true },
        adv(text!("SteamGridDB API key", covers.steamgriddb_api_key, Effect::Covers)),
        Entry { label: "TheGamesDB", kind: Kind::Source("thegamesdb"), effect: Effect::Covers, advanced: true },
        adv(text!("TheGamesDB API key", covers.thegamesdb_api_key, Effect::Covers)),
        adv(toggle!("Descriptions and genres", covers.fetch_metadata, Effect::Covers)),
        header!("Screen"),
        toggle!("Fullscreen", general.fullscreen, Effect::Window),
        adv(toggle!("Vsync", general.vsync, Effect::Window)),
        toggle!("Potato mode (much faster, plainer looks)", general.potato, Effect::Potato),
        num!("Brightness", general.brightness, 0.2, 3.0, 0.05, Effect::None),
        adv(toggle!("Shadows", general.shadows, Effect::None)),
        adv(num!("FPS in background (0 = paused)", general.background_fps, 0.0, 30.0, 1.0, Effect::Window)),
        toggle!("Show key hints", general.show_hints, Effect::None),
        adv(toggle!("Show description", general.show_info, Effect::None)),
    ]
}

// ------------------------------------------------------------------------------------------------
// State & UI
// ------------------------------------------------------------------------------------------------

enum Picked {
    Files(Vec<PathBuf>),
    Folder(PathBuf),
    RomFolder(PathBuf, Option<Platform>),
    Cover(String, PathBuf),
    Location(String, PathBuf),
    Background(PathBuf),
}

#[derive(Resource)]
pub struct Menu {
    pub open: bool,
    /// Set for the frame the menu closed on, so the key that closed it is not reused.
    pub just_closed: bool,
    index: usize,
    scroll: usize,
    /// Text being edited in the dialog.
    editing: Option<crate::textedit::TextEdit>,
    /// Mouse is dragging a selection in the dialog's field; last click (time, char) for double clicks.
    edit_drag: bool,
    edit_click: (f32, usize),
    rom_platform: Option<Platform>,
    picker: Option<Receiver<Option<Picked>>>,
    entries: Vec<Entry>,
    repeat: f32,
    /// Rows that fit on screen.
    rows: usize,
    /// Library revision the hidden games list was built from (None = rebuild).
    built: Option<u64>,
    /// Reset was clicked once: a second click before this time (elapsed seconds) resets.
    reset_armed_until: f32,
    /// Asking whether to restart now (a setting changed that needs one).
    restart_prompt: bool,
    /// Categories (by header) whose advanced settings are shown.
    expanded: Vec<&'static str>,
    /// Categories (by header) folded away.
    collapsed: Vec<&'static str>,
    /// The colour picker, while it is open.
    color: Option<ColorPick>,
    /// The art chooser, while it is open.
    art: Option<ArtPick>,
}

impl Menu {
    /// The art chooser is open (it can stay open after the settings close).
    pub fn art_open(&self) -> bool {
        self.art.is_some()
    }

    /// Starts with the menu open (`slipcase --settings`).
    pub fn opened() -> Self {
        Menu { open: true, ..Default::default() }
    }

    pub fn hide(&mut self) {
        close(self);
    }

    /// Opens the menu (gear button, keys).
    pub fn show(&mut self) {
        self.open = true;
        self.built = None;
        if !selectable(&self.entries[self.index]) {
            step_index(self, 1);
        }
    }
}

impl Default for Menu {
    fn default() -> Self {
        Menu {
            open: false,
            just_closed: false,
            index: 0,
            scroll: 0,
            editing: None,
            edit_drag: false,
            edit_click: (-1.0, usize::MAX),
            rom_platform: None,
            picker: None,
            entries: entries(),
            repeat: 0.0,
            rows: 20,
            built: None,
            reset_armed_until: 0.0,
            restart_prompt: false,
            color: None,
            art: None,
            expanded: Vec::new(),
            // Every category starts folded.
            collapsed: category_headers(),
        }
    }
}

/// Row widgets kept for the menu; how many are used depends on the window height.
const MAX_ROWS: usize = 80;
/// Height of one row including the gap, in logical pixels (headings add a little more).
const ROW_H: f32 = 38.0;
/// Title, help text, footer, the button bar and padding.
const MENU_CHROME: f32 = 220.0;
/// Width of the settings panel.
pub const PANEL_W: f32 = 500.0;

#[derive(Component)]
struct MenuPanel;
#[derive(Component)]
struct MenuRow(usize);
#[derive(Component)]
struct RowLabel(usize);
/// The emoji beside a category heading.
#[derive(Component)]
struct RowIcon(usize);

/// Shows the emoji beside the category headings in view.
fn draw_menu_icons(menu: Res<Menu>, category_icons: Option<Res<CategoryIcons>>, mut icons: Query<(&RowIcon, &mut UiImage, &mut Style)>) {
    for (ic, mut img, mut style) in &mut icons {
        let icon = menu
            .entries
            .get(menu.scroll + ic.0)
            .filter(|e| matches!(e.kind, Kind::Header) && ic.0 < menu.rows)
            .and_then(|e| category_icons.as_ref().and_then(|c| c.0.get(e.label)));
        let d = if icon.is_some() { Display::Flex } else { Display::None };
        if style.display != d {
            style.display = d;
        }
        if let Some(h) = icon {
            if img.texture != *h {
                img.texture = h.clone();
            }
        }
    }
}

/// Emoji for the settings categories (Twemoji images, built in).
#[derive(Resource, Default)]
struct CategoryIcons(std::collections::HashMap<&'static str, Handle<Image>>);

fn load_category_icons(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let files: [(&'static str, &[u8]); 10] = [
        ("Library", include_bytes!("icons/1f4da.png")),
        ("Selected game", include_bytes!("icons/1f3ae.png")),
        (HIDDEN_HEADER, include_bytes!("icons/1f648.png")),
        ("Layout", include_bytes!("icons/1f5c2.png")),
        ("Cases and discs", include_bytes!("icons/1f4bf.png")),
        ("Background", include_bytes!("icons/1f30c.png")),
        ("Launching", include_bytes!("icons/1f680.png")),
        ("Motion", include_bytes!("icons/1f39e.png")),
        ("Covers", include_bytes!("icons/1f5bc.png")),
        ("Screen", include_bytes!("icons/1f5a5.png")),
    ];
    let mut map = std::collections::HashMap::new();
    for (name, bytes) in files {
        if let Ok(img) = image::load_from_memory(bytes) {
            let img = img.to_rgba8();
            let (w, h) = img.dimensions();
            map.insert(name, images.add(crate::render::make_image(w, h, img.into_raw(), true)));
        }
    }
    commands.insert_resource(CategoryIcons(map));
}
#[derive(Component)]
struct RowValue(usize);
#[derive(Component)]
struct MenuFooter;
/// Line under a category heading.
#[derive(Component)]
struct RowUnderline(usize);

/// The X in the panel's top right corner.
#[derive(Component)]
struct MenuCloseBtn;
/// The < (-1) or > (+1) button on a row.
#[derive(Component)]
struct ArrowBtn(usize, i32);

/// The box holding a row's value (fixed width on rows with arrows).
#[derive(Component)]
struct RowValueBox(usize);

/// Rows with arrows get a value box of fixed width, so the arrows do not move as the value
/// changes length.
fn size_value_boxes(arrows: Query<(&ArrowBtn, &Style), Without<RowValueBox>>, mut boxes: Query<(&RowValueBox, &mut Style), Without<ArrowBtn>>) {
    for (b, mut style) in &mut boxes {
        let arrows_shown = arrows.iter().any(|(a, s)| a.0 == b.0 && s.display != Display::None);
        let w = if arrows_shown { Val::Px(190.0) } else { Val::Auto };
        if style.width != w {
            style.width = w;
        }
    }
}

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Menu>()
            .add_systems(Startup, (load_category_icons, spawn_menu, spawn_edit_dialog, spawn_restart_prompt, spawn_color_picker, spawn_art_picker).chain())
            .add_systems(Update, (art_pick_input, draw_art_picker).chain().after(menu_input))
            .add_systems(Update, draw_menu_icons.after(draw_menu))
            .add_systems(Update, size_value_boxes.after(draw_menu))
            .add_systems(Update, (menu_input, color_pick_input, menu_pickers, draw_menu, draw_edit_dialog, draw_restart_prompt, draw_color_picker).chain());
    }
}

/// Regular and semi-bold faces of one sans-serif family for the whole menu (the default font
/// if none is installed).
#[derive(Resource, Clone)]
struct MenuFonts {
    regular: Handle<Font>,
    bold: Handle<Font>,
    /// The regular face again, for measuring text (where the cursor and selection go).
    measure: Option<std::sync::Arc<fontdue::Font>>,
}

impl MenuFonts {
    /// Width of `s` in logical pixels at `px`, as the UI lays it out (advances plus kerning).
    fn width(&self, s: &str, px: f32) -> f32 {
        let Some(f) = &self.measure else { return s.chars().count() as f32 * px * 0.6 };
        // Bevy's text renderer scales a font so ascent - descent is `px` tall; fontdue scales
        // by the em square. Convert so both agree.
        let px = match f.horizontal_line_metrics(1.0) {
            Some(m) if m.ascent - m.descent > 0.0 => px / (m.ascent - m.descent),
            _ => px,
        };
        let mut w = 0.0;
        let mut prev: Option<char> = None;
        for c in s.chars() {
            if let Some(p) = prev {
                w += f.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            w += f.metrics(c, px).advance_width;
            prev = Some(c);
        }
        w
    }
}

/// (regular, bold) font pairs, in order of preference.
const MENU_FONT_PAIRS: &[(&str, &str)] = &[
    ("/usr/share/fonts/TTF/OpenSans-Regular.ttf", "/usr/share/fonts/TTF/OpenSans-SemiBold.ttf"),
    ("/usr/share/fonts/open-sans/OpenSans-Regular.ttf", "/usr/share/fonts/open-sans/OpenSans-SemiBold.ttf"),
    ("/usr/share/fonts/truetype/open-sans/OpenSans-Regular.ttf", "/usr/share/fonts/truetype/open-sans/OpenSans-SemiBold.ttf"),
    ("/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/noto/NotoSans-SemiBold.ttf"),
    ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf"),
    ("/usr/share/fonts/google-noto/NotoSans-Regular.ttf", "/usr/share/fonts/google-noto/NotoSans-SemiBold.ttf"),
    ("/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
    ("/usr/share/fonts/liberation/LiberationSans-Regular.ttf", "/usr/share/fonts/liberation/LiberationSans-Bold.ttf"),
    ("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf", "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf"),
    (r"C:\Windows\Fonts\segoeui.ttf", r"C:\Windows\Fonts\seguisb.ttf"),
    (r"C:\Windows\Fonts\arial.ttf", r"C:\Windows\Fonts\arialbd.ttf"),
];

fn load_menu_fonts(fonts: &mut Assets<Font>) -> MenuFonts {
    let load = |p: &str| std::fs::read(p).ok().and_then(|b| Font::try_from_bytes(b).ok());
    for (r, b) in MENU_FONT_PAIRS {
        if let (Some(rf), Some(bf)) = (load(r), load(b)) {
            let measure = std::fs::read(r).ok().and_then(|b| fontdue::Font::from_bytes(b, fontdue::FontSettings::default()).ok()).map(std::sync::Arc::new);
            return MenuFonts { regular: fonts.add(rf), bold: fonts.add(bf), measure };
        }
    }
    MenuFonts { regular: Handle::default(), bold: Handle::default(), measure: None }
}

// Menu colours.
const ACCENT: Color = Color::srgb(0.55, 0.78, 1.0);
const TEXT: Color = Color::srgba(1.0, 1.0, 1.0, 0.92);
const DIM: Color = Color::srgba(1.0, 1.0, 1.0, 0.5);

/// Buttons along the bottom of the panel.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum BarBtn {
    ExpandAll,
    CollapseAll,
    Reset,
    Close,
}

/// The Reset button's text (it changes to ask for confirmation).
#[derive(Component)]
struct ResetLabel;

/// Seconds the Reset button stays armed waiting for the confirming click.
const RESET_CONFIRM_SECS: f32 = 4.0;

fn spawn_menu(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let f = load_menu_fonts(&mut fonts);
    commands.insert_resource(f.clone());
    let txt = |font: &Handle<Font>, size: f32, color: Color, s: &str| {
        TextBundle::from_section(s.to_string(), TextStyle { font: font.clone(), font_size: size, color })
    };
    let radius = BorderRadius::all(Val::Px(6.0));
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Px(PANEL_W),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect { left: Val::Px(24.0), right: Val::Px(24.0), top: Val::Px(22.0), bottom: Val::Px(18.0) },
                    row_gap: Val::Px(2.0),
                    border: UiRect::left(Val::Px(1.0)),
                    display: Display::None,
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.03, 0.035, 0.05, 0.94)),
                border_color: BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                z_index: ZIndex::Global(40),
                // Tracked so the games behind know when the mouse is over the panel.
                focus_policy: bevy::ui::FocusPolicy::Block,
                ..default()
            },
            MenuPanel,
            Interaction::default(),
        ))
        .with_children(|p| {
            p.spawn(txt(&f.bold, 28.0, Color::WHITE, "Settings"));
            p.spawn((
                ButtonBundle {
                    style: Style {
                        position_type: PositionType::Absolute,
                        right: Val::Px(16.0),
                        top: Val::Px(18.0),
                        width: Val::Px(34.0),
                        height: Val::Px(34.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    border_radius: radius,
                    background_color: BackgroundColor(Color::NONE),
                    ..default()
                },
                MenuCloseBtn,
            ))
            .with_children(|b| {
                b.spawn(txt(&f.regular, 22.0, DIM, "X"));
            });
            let mut h = txt(&f.regular, 14.0, DIM, "Up / Down to move, Left / Right to change, Enter to edit, Esc to close");
            h.style.margin = UiRect { top: Val::Px(2.0), bottom: Val::Px(12.0), ..default() };
            p.spawn(h);
            // The list takes all the height between the title and the bottom bar and clips what
            // does not fit, so the bar never moves.
            p.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    min_height: Val::Px(0.0),
                    row_gap: Val::Px(2.0),
                    overflow: Overflow::clip(),
                    ..default()
                },
                ..default()
            })
            .with_children(|p| {
                for i in 0..MAX_ROWS {
                    p.spawn((
                        ButtonBundle {
                            style: Style {
                                width: Val::Percent(100.0),
                                min_height: Val::Px(34.0),
                                justify_content: JustifyContent::SpaceBetween,
                                align_items: AlignItems::Center,
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(2.0)),
                                ..default()
                            },
                            border_radius: radius,
                            background_color: BackgroundColor(Color::NONE),
                            ..default()
                        },
                        MenuRow(i),
                    ))
                    .with_children(|r| {
                        // Icon (category headings only) and label, together on the left.
                        r.spawn(NodeBundle { style: Style { align_items: AlignItems::Center, column_gap: Val::Px(8.0), ..default() }, focus_policy: bevy::ui::FocusPolicy::Pass, ..default() }).with_children(|l| {
                            l.spawn((
                                ImageBundle { style: Style { width: Val::Px(20.0), height: Val::Px(20.0), display: Display::None, ..default() }, focus_policy: bevy::ui::FocusPolicy::Pass, ..default() },
                                RowIcon(i),
                            ));
                            l.spawn((txt(&f.regular, 17.0, TEXT, ""), RowLabel(i)));
                        });
                        // Divider under a category heading, across the whole row.
                        r.spawn((
                            NodeBundle {
                                style: Style {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(10.0),
                                    right: Val::Px(10.0),
                                    bottom: Val::Px(0.0),
                                    height: Val::Px(1.0),
                                    display: Display::None,
                                    ..default()
                                },
                                background_color: BackgroundColor(Color::srgba(0.55, 0.78, 1.0, 0.35)),
                                ..default()
                            },
                            RowUnderline(i),
                        ));
                        r.spawn(NodeBundle { style: Style { align_items: AlignItems::Center, column_gap: Val::Px(4.0), ..default() }, ..default() })
                            .with_children(|v| {
                                let arrow = |v: &mut ChildBuilder, dir: i32| {
                                    v.spawn((
                                        ButtonBundle {
                                            style: Style {
                                                width: Val::Px(24.0),
                                                height: Val::Px(24.0),
                                                justify_content: JustifyContent::Center,
                                                align_items: AlignItems::Center,
                                                ..default()
                                            },
                                            border_radius: BorderRadius::all(Val::Px(4.0)),
                                            background_color: BackgroundColor(Color::NONE),
                                            ..default()
                                        },
                                        ArrowBtn(i, dir),
                                    ))
                                    .with_children(|b| {
                                        b.spawn(txt(&f.bold, 16.0, ACCENT, if dir < 0 { "<" } else { ">" }));
                                    });
                                };
                                arrow(v, -1);
                                // The value sits in a box of fixed width when the row has arrows,
                                // so the arrows stay put whatever the option's length.
                                v.spawn((NodeBundle { style: Style { justify_content: JustifyContent::Center, ..default() }, ..default() }, RowValueBox(i)))
                                    .with_children(|b| {
                                        b.spawn((txt(&f.regular, 17.0, ACCENT, ""), RowValue(i)));
                                    });
                                arrow(v, 1);
                            });
                    });
                }
            });
            // Pinned to the bottom: which game the "Selected game" settings change, then the buttons.
            let mut foot = txt(&f.regular, 14.0, DIM, "");
            foot.style.margin = UiRect { top: Val::Px(10.0), bottom: Val::Px(8.0), ..default() };
            p.spawn((foot, MenuFooter));
            p.spawn(NodeBundle { style: Style { width: Val::Percent(100.0), column_gap: Val::Px(8.0), flex_shrink: 0.0, ..default() }, ..default() })
                .with_children(|bar| {
                    for (b, label) in [(BarBtn::ExpandAll, "Expand all"), (BarBtn::CollapseAll, "Collapse all"), (BarBtn::Reset, "Reset"), (BarBtn::Close, "Close")] {
                        bar.spawn((
                            ButtonBundle {
                                style: Style {
                                    flex_grow: 1.0,
                                    flex_basis: Val::Px(0.0),
                                    height: Val::Px(40.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                border_radius: radius,
                                background_color: BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.07)),
                                ..default()
                            },
                            b,
                        ))
                        .with_children(|t| {
                            let mut e = t.spawn(txt(&f.bold, 16.0, TEXT, label));
                            if b == BarBtn::Reset {
                                e.insert(ResetLabel);
                            }
                        });
                    }
                });
        });
}

// ------------------------------------------------------------------------------------------------
// Text dialog
// ------------------------------------------------------------------------------------------------

/// Text size in the editing box's field, and the field's inner padding.
const EDIT_PX: f32 = 17.0;
const EDIT_PAD: f32 = 10.0;
/// From the top of the box to the top of its field (padding + title), so the field can sit
/// over the row being edited.
const EDIT_FIELD_TOP: f32 = 32.0;

/// What the text dialog's mouse handling reads (kept together to stay under Bevy's
/// parameter limit for `menu_input`).
#[derive(bevy::ecs::system::SystemParam)]
struct EditInput<'w, 's> {
    field: Query<'w, 's, (&'static Interaction, &'static bevy::ui::RelativeCursorPosition, &'static Node), With<EditField>>,
    buttons: Query<'w, 's, (&'static EditBtn, &'static Interaction)>,
    fonts: Res<'w, MenuFonts>,
    restart: Query<'w, 's, (&'static RestartBtn, &'static Interaction)>,
}

/// Restart (true) / Later (false) in the restart question.
#[derive(Component, Clone, Copy)]
struct RestartBtn(bool);
#[derive(Component)]
struct RestartPrompt;

/// Saves everything and starts a fresh copy of Slipcase with the same arguments (minus
/// `--overlay`, so turning the overlay off sticks); the caller then quits this one.
fn restart_app(ctx: &mut Ctx) {
    ctx.cfg.save();
    ctx.lib.save();
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--overlay").collect();
    match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(&args).spawn()) {
        Ok(_) => {}
        Err(e) => eprintln!("[slipcase] could not restart: {e}"),
    }
}

fn spawn_restart_prompt(mut commands: Commands, fonts: Res<MenuFonts>) {
    let f = fonts.clone();
    let txt = |font: &Handle<Font>, size: f32, color: Color, s: &str| {
        TextBundle::from_section(s.to_string(), TextStyle { font: font.clone(), font_size: size, color })
    };
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    display: Display::None,
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                z_index: ZIndex::Global(70),
                focus_policy: bevy::ui::FocusPolicy::Block,
                ..default()
            },
            RestartPrompt,
            Interaction::default(),
        ))
        .with_children(|o| {
            o.spawn(NodeBundle {
                style: Style {
                    width: Val::Px(400.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(20.0)),
                    row_gap: Val::Px(10.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgb(0.07, 0.08, 0.11)),
                border_color: BorderColor(Color::srgba(0.55, 0.78, 1.0, 0.5)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            })
            .with_children(|d| {
                d.spawn(txt(&f.bold, 19.0, Color::WHITE, "Restart Slipcase now?"));
                d.spawn(txt(&f.regular, 14.5, DIM, "The overlay setting takes effect after a restart."));
                d.spawn(NodeBundle { style: Style { justify_content: JustifyContent::FlexEnd, column_gap: Val::Px(8.0), margin: UiRect::top(Val::Px(6.0)), ..default() }, ..default() })
                    .with_children(|row| {
                        for (b, label) in [(false, "Later"), (true, "Restart")] {
                            row.spawn((
                                ButtonBundle {
                                    style: Style { width: Val::Px(100.0), height: Val::Px(34.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                                    border_radius: BorderRadius::all(Val::Px(6.0)),
                                    background_color: BackgroundColor(if b { Color::srgba(0.35, 0.62, 1.0, 0.45) } else { Color::srgba(1.0, 1.0, 1.0, 0.08) }),
                                    ..default()
                                },
                                RestartBtn(b),
                            ))
                            .with_children(|t| {
                                t.spawn(txt(&f.bold, 15.0, TEXT, label));
                            });
                        }
                    });
            });
        });
}

fn draw_restart_prompt(menu: Res<Menu>, mut prompt: Query<&mut Style, With<RestartPrompt>>, mut buttons: Query<(&RestartBtn, &Interaction, &mut BackgroundColor)>) {
    for mut s in &mut prompt {
        let d = if menu.restart_prompt { Display::Flex } else { Display::None };
        if s.display != d {
            s.display = d;
        }
    }
    for (b, i, mut bg) in &mut buttons {
        let base = if b.0 { 0.45 } else { 0.08 };
        let a = if *i == Interaction::None { base } else { base + 0.15 };
        bg.0 = if b.0 { Color::srgba(0.35, 0.62, 1.0, a) } else { Color::srgba(1.0, 1.0, 1.0, a) };
    }
}

#[derive(Component)]
struct EditDialog;
/// The box itself, placed over the row being edited.
#[derive(Component)]
struct EditBox;
#[derive(Component)]
struct EditTitle;
#[derive(Component)]
struct EditField;
#[derive(Component)]
struct EditText;
#[derive(Component)]
struct EditCaret;
#[derive(Component)]
struct EditSel;
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum EditBtn {
    Ok,
    Cancel,
}

fn spawn_edit_dialog(mut commands: Commands, fonts: Res<MenuFonts>) {
    let f = fonts.clone();
    let txt = |font: &Handle<Font>, size: f32, color: Color, s: &str| {
        TextBundle::from_section(s.to_string(), TextStyle { font: font.clone(), font_size: size, color })
    };
    // A light full-screen layer (catches the mouse) with a small box over the edited row.
    commands
        .spawn((
            NodeBundle {
                style: Style { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), display: Display::None, ..default() },
                background_color: BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.25)),
                z_index: ZIndex::Global(60),
                focus_policy: bevy::ui::FocusPolicy::Block,
                ..default()
            },
            EditDialog,
            Interaction::default(),
        ))
        .with_children(|o| {
            o.spawn((
                NodeBundle {
                    style: Style {
                        position_type: PositionType::Absolute,
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(8.0)),
                        row_gap: Val::Px(6.0),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    background_color: BackgroundColor(Color::srgb(0.07, 0.08, 0.11)),
                    border_color: BorderColor(Color::srgba(0.55, 0.78, 1.0, 0.5)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                EditBox,
            ))
            .with_children(|d| {
                let mut title = txt(&f.bold, 14.0, ACCENT, "");
                title.style.height = Val::Px(18.0);
                d.spawn((title, EditTitle));
                // The field: text, selection highlight and caret, scrolled sideways as needed.
                d.spawn((
                    NodeBundle {
                        style: Style { width: Val::Percent(100.0), height: Val::Px(36.0), border: UiRect::all(Val::Px(1.5)), overflow: Overflow::clip(), ..default() },
                        background_color: BackgroundColor(Color::srgb(0.02, 0.025, 0.04)),
                        border_color: BorderColor(ACCENT),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        focus_policy: bevy::ui::FocusPolicy::Block,
                        ..default()
                    },
                    EditField,
                    Interaction::default(),
                    bevy::ui::RelativeCursorPosition::default(),
                ))
                .with_children(|fld| {
                    fld.spawn((
                        NodeBundle {
                            style: Style { position_type: PositionType::Absolute, top: Val::Px(4.0), height: Val::Px(25.0), ..default() },
                            background_color: BackgroundColor(Color::srgba(0.35, 0.62, 1.0, 0.45)),
                            ..default()
                        },
                        EditSel,
                    ));
                    let mut t = txt(&f.regular, EDIT_PX, Color::WHITE, "");
                    t.text.linebreak_behavior = bevy::text::BreakLineOn::NoWrap;
                    t.style.position_type = PositionType::Absolute;
                    t.style.top = Val::Px(5.0);
                    fld.spawn((t, EditText));
                    fld.spawn((
                        NodeBundle {
                            style: Style { position_type: PositionType::Absolute, top: Val::Px(5.0), width: Val::Px(2.0), height: Val::Px(23.0), ..default() },
                            background_color: BackgroundColor(Color::WHITE),
                            ..default()
                        },
                        EditCaret,
                    ));
                });
                d.spawn(NodeBundle { style: Style { align_items: AlignItems::Center, column_gap: Val::Px(6.0), ..default() }, ..default() }).with_children(|row| {
                    let mut hint = txt(&f.regular, 12.0, DIM, "Enter saves, Esc cancels");
                    hint.style.flex_grow = 1.0;
                    row.spawn(hint);
                    for (b, label) in [(EditBtn::Cancel, "Cancel"), (EditBtn::Ok, "OK")] {
                        row.spawn((
                            ButtonBundle {
                                style: Style { width: Val::Px(70.0), height: Val::Px(26.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                                border_radius: BorderRadius::all(Val::Px(5.0)),
                                background_color: BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                                ..default()
                            },
                            b,
                        ))
                        .with_children(|t| {
                            t.spawn(txt(&f.bold, 13.5, TEXT, label));
                        });
                    }
                });
            });
        });
}

/// Shows the dialog while something is being edited, with the text scrolled so the caret is
/// in view, the selection highlighted and the caret blinking.
#[allow(clippy::type_complexity)]
fn draw_edit_dialog(
    mut menu: ResMut<Menu>,
    fonts: Res<MenuFonts>,
    time: Res<Time>,
    mut dialog: Query<&mut Style, (With<EditDialog>, Without<EditText>, Without<EditCaret>, Without<EditSel>)>,
    mut title: Query<&mut Text, (With<EditTitle>, Without<EditText>)>,
    mut text: Query<(&mut Text, &mut Style), (With<EditText>, Without<EditTitle>, Without<EditDialog>, Without<EditCaret>, Without<EditSel>)>,
    mut caret: Query<(&mut Style, &mut Visibility), (With<EditCaret>, Without<EditDialog>, Without<EditText>, Without<EditSel>)>,
    mut sel: Query<(&mut Style, &mut Visibility), (With<EditSel>, Without<EditDialog>, Without<EditText>, Without<EditCaret>)>,
    mut buttons: Query<(&Interaction, &mut BackgroundColor), With<EditBtn>>,
    mut boxes: Query<&mut Style, (With<EditBox>, Without<EditDialog>, Without<EditText>, Without<EditCaret>, Without<EditSel>)>,
    rows: Query<(&MenuRow, &GlobalTransform, &Node)>,
    field: Query<&Node, With<EditField>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let open = menu.editing.is_some();
    for mut s in &mut dialog {
        let d = if open { Display::Flex } else { Display::None };
        if s.display != d {
            s.display = d;
        }
    }
    if !open {
        return;
    }
    // Over the row being edited: same width, its field where the row's text is, kept on screen.
    let row = menu.index.saturating_sub(menu.scroll);
    if let Some((_, gt, node)) = rows.iter().find(|(r, ..)| r.0 == row) {
        let (c, size) = (gt.translation().truncate(), node.size());
        let win_h = windows.get_single().map(|w| w.height()).unwrap_or(1080.0);
        let top = (c.y - size.y * 0.5 - EDIT_FIELD_TOP).clamp(8.0, (win_h - 120.0).max(8.0));
        for mut s in &mut boxes {
            s.left = Val::Px(c.x - size.x * 0.5 - 8.0);
            s.top = Val::Px(top);
            s.width = Val::Px(size.x + 16.0);
        }
    }
    let Some(ed) = menu.editing.as_mut() else { return };
    // Keep the caret inside the field.
    let field_w = field.get_single().map(|n| n.size().x).unwrap_or(400.0);
    let inner = (field_w - EDIT_PAD * 2.0).max(10.0);
    let cx = fonts.width(&ed.prefix(ed.cursor), EDIT_PX);
    if cx - ed.scroll > inner {
        ed.scroll = cx - inner;
    } else if cx < ed.scroll {
        ed.scroll = cx;
    }
    let total = fonts.width(&ed.text(), EDIT_PX);
    ed.scroll = ed.scroll.min((total - inner).max(0.0)).max(0.0);
    let left = EDIT_PAD - ed.scroll;
    for mut t in &mut title {
        if t.sections[0].value != ed.title {
            t.sections[0].value = ed.title.clone();
        }
    }
    let value = ed.text();
    for (mut t, mut s) in &mut text {
        if t.sections[0].value != value {
            t.sections[0].value = value.clone();
        }
        s.left = Val::Px(left);
    }
    // Solid while typing, then blinking.
    let blink_on = ((time.elapsed_seconds() - ed.last_input) * 1.8).floor() as i64 % 2 == 0;
    for (mut s, mut v) in &mut caret {
        s.left = Val::Px(left + cx - 1.0);
        *v = if blink_on { Visibility::Inherited } else { Visibility::Hidden };
    }
    let range = ed.selection();
    for (mut s, mut v) in &mut sel {
        match range {
            Some((a, b)) => {
                let x0 = fonts.width(&ed.prefix(a), EDIT_PX);
                let x1 = fonts.width(&ed.prefix(b), EDIT_PX);
                s.left = Val::Px(left + x0);
                s.width = Val::Px(x1 - x0);
                *v = Visibility::Inherited;
            }
            None => *v = Visibility::Hidden,
        }
    }
    for (i, mut bg) in &mut buttons {
        bg.0 = match i {
            Interaction::None => Color::srgba(1.0, 1.0, 1.0, 0.08),
            Interaction::Hovered => Color::srgba(1.0, 1.0, 1.0, 0.16),
            Interaction::Pressed => Color::srgba(0.35, 0.62, 1.0, 0.4),
        };
    }
}

fn fmt_num(v: f32, step: f32) -> String {
    if step >= 1.0 {
        format!("{v:.0}")
    } else if step >= 0.1 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

fn value_text(e: &Entry, cfg: &mut ConfigData, menu: &Menu, game: Option<&crate::library::Game>) -> String {
    match &e.kind {
        Kind::Header => String::new(),
        Kind::Toggle(f) => if *f(cfg) { "On" } else { "Off" }.into(),
        Kind::Num { f, step, .. } => fmt_num(*f(cfg), *step),
        Kind::Choice { get, .. } => get(cfg),
        Kind::Color(f) => f(cfg).clone(),
        Kind::Text(f) => {
            let v = f(cfg);
            if v.is_empty() {
                "(not set)".into()
            } else if v.len() > 8 {
                format!("{}...", &v[..6])
            } else {
                v.clone()
            }
        }
        Kind::Source(s) => if cfg.covers.sources.iter().any(|x| x == s) { "On" } else { "Off" }.into(),
        Kind::Act(_) => ">".into(),
        Kind::RomPlatform => menu.rom_platform.map_or("guess from files", |p| p.display_name()).to_string(),
        Kind::GameTitle => game.map(|g| crate::render::truncate(&g.title, 26)).unwrap_or_default(),
        Kind::GamePlatform => game.map(|g| g.platform.display_name().to_string()).unwrap_or_default(),
        Kind::GameFavorite => game.map(|g| if g.favorite { "Yes" } else { "No" }.to_string()).unwrap_or_default(),
        Kind::GameLocation => game
            .map(|g| {
                let l = g.location();
                let n = l.chars().count();
                let short = if n > 26 { format!("...{}", l.chars().skip(n - 23).collect::<String>()) } else { l };
                if g.custom_location.is_some() { format!("{short} *") } else { short }
            })
            .unwrap_or_default(),
        Kind::GameArgs => game.map(|g| if g.launch_args.is_empty() { "(none)".to_string() } else { crate::render::truncate(&g.launch_args, 26) }).unwrap_or_default(),
        Kind::HiddenGame(_) => "Unhide".into(),
        Kind::HiddenTitle(_) => "Unhide (not found yet)".into(),
        Kind::Note(_) => String::new(),
        Kind::Advanced(_) => String::new(),
    }
}

/// Entries changed with < and >.
fn has_arrows(e: &Entry) -> bool {
    matches!(e.kind, Kind::Num { .. } | Kind::Choice { .. } | Kind::RomPlatform | Kind::GamePlatform)
}

/// Text in the label column: hidden games show their title.
fn label_text(e: &Entry, lib: &Library, menu: &Menu) -> String {
    match &e.kind {
        Kind::HiddenGame(id) => lib.find(id).map(|g| crate::render::truncate(&g.title, 30)).unwrap_or_default(),
        Kind::HiddenTitle(t) => crate::render::truncate(t, 30),
        Kind::Note(t) => t.to_string(),
        // + to open, - to fold.
        Kind::Header if e.label.is_empty() => String::new(),
        Kind::Header => format!("{}   {}", e.label, if menu.collapsed.contains(&e.label) { "+" } else { "-" }),
        Kind::Advanced(h) => format!("Advanced settings   {}", if menu.expanded.contains(h) { "-" } else { "+" }),
        _ => e.label.to_string(),
    }
}

fn selectable(e: &Entry) -> bool {
    // Category headers can be selected to fold them (the last, unnamed one cannot).
    match e.kind {
        Kind::Header => !e.label.is_empty(),
        Kind::Note(_) => false,
        _ => true,
    }
}

#[allow(clippy::type_complexity)]
fn draw_menu(
    menu: Res<Menu>,
    mut cfg: ResMut<Config>,
    lib: Res<Library>,
    view: Res<ViewState>,
    mut panel: Query<&mut Style, (With<MenuPanel>, Without<ArrowBtn>)>,
    mut rows: Query<(&MenuRow, &Interaction, &mut BackgroundColor, &mut Style), (Without<MenuPanel>, Without<ArrowBtn>, Without<MenuCloseBtn>, Without<BarBtn>)>,
    mut arrows: Query<(&ArrowBtn, &Interaction, &mut Style, &mut BackgroundColor), (Without<MenuPanel>, Without<MenuRow>, Without<MenuCloseBtn>, Without<BarBtn>)>,
    mut buttons: Query<(&Interaction, &mut BackgroundColor, Has<MenuCloseBtn>, Option<&BarBtn>), (Or<(With<MenuCloseBtn>, With<BarBtn>)>, Without<MenuRow>, Without<ArrowBtn>)>,
    mut reset_label: Query<&mut Text, (With<ResetLabel>, Without<RowLabel>, Without<RowValue>, Without<MenuFooter>)>,
    time: Res<Time>,
    mut underlines: Query<(&RowUnderline, &mut Style), (Without<MenuRow>, Without<ArrowBtn>, Without<MenuPanel>, Without<BarBtn>, Without<MenuCloseBtn>)>,
    fonts: Res<MenuFonts>,
    mut labels: Query<(&RowLabel, &mut Text), (Without<RowValue>, Without<MenuFooter>, Without<ResetLabel>)>,
    mut values: Query<(&RowValue, &mut Text), (Without<RowLabel>, Without<MenuFooter>, Without<ResetLabel>)>,
    mut footer: Query<&mut Text, (With<MenuFooter>, Without<RowLabel>, Without<RowValue>, Without<ResetLabel>)>,
) {
    for mut s in &mut panel {
        let d = if menu.open { Display::Flex } else { Display::None };
        if s.display != d {
            s.display = d;
        }
    }
    if !menu.open {
        return;
    }
    let game = view.selected_game(&lib);
    for (row, interaction, mut bg, mut style) in &mut rows {
        let i = menu.scroll + row.0;
        let d = if row.0 < menu.rows && i < menu.entries.len() { Display::Flex } else { Display::None };
        if style.display != d {
            style.display = d;
        }
        let e = menu.entries.get(i);
        let heading = e.is_some_and(|e| matches!(e.kind, Kind::Header));
        // Headings get breathing room above (not the first row on screen); advanced settings
        // are indented under their category.
        let top = Val::Px(if heading && row.0 > 0 { 10.0 } else { 0.0 });
        if style.margin.top != top {
            style.margin.top = top;
        }
        let left = Val::Px(if e.is_some_and(|e| e.advanced) { 26.0 } else { 10.0 });
        if style.padding.left != left {
            style.padding.left = left;
        }
        bg.0 = if i == menu.index {
            Color::srgba(0.35, 0.62, 1.0, 0.24)
        } else if *interaction != Interaction::None && e.is_some_and(selectable) {
            Color::srgba(1.0, 1.0, 1.0, 0.06)
        } else {
            Color::NONE
        };
    }
    // Text values need mutable config access for the getters; they do not change anything.
    let data = &mut cfg.data;
    for (l, mut t) in &mut labels {
        let i = menu.scroll + l.0;
        let Some(e) = menu.entries.get(i) else { continue };
        let header = matches!(e.kind, Kind::Header);
        let s = label_text(e, &lib, &menu);
        if t.sections[0].value != s {
            t.sections[0].value = s;
        }
        let advanced_row = matches!(e.kind, Kind::Advanced(_));
        let style = &mut t.sections[0].style;
        style.color = if header {
            ACCENT
        } else if advanced_row || matches!(e.kind, Kind::Note(_)) {
            DIM
        } else {
            TEXT
        };
        // Category headings: semi-bold and a size up, with a divider; "Advanced" rows small.
        style.font_size = if header { 18.0 } else if advanced_row { 15.0 } else { 17.0 };
        let font = if header { &fonts.bold } else { &fonts.regular };
        if style.font != *font {
            style.font = font.clone();
        }
    }
    for (v, mut t) in &mut values {
        let i = menu.scroll + v.0;
        let Some(e) = menu.entries.get(i) else { continue };
        let s = value_text(e, data, &menu, game);
        // Colour settings show their value in that colour (lifted enough to read).
        let color = match &e.kind {
            Kind::Color(_) => {
                let c = crate::config::parse_hex(&s);
                let lift = (0.55 / c[0].max(c[1]).max(c[2]).max(0.05)).max(1.0);
                Color::srgb((c[0] * lift).min(1.0), (c[1] * lift).min(1.0), (c[2] * lift).min(1.0))
            }
            _ => ACCENT,
        };
        if t.sections[0].style.color != color {
            t.sections[0].style.color = color;
        }
        if t.sections[0].value != s {
            t.sections[0].value = s;
        }
    }
    let armed = menu.reset_armed_until > time.elapsed_seconds();
    for (interaction, mut bg, is_x, bar_btn) in &mut buttons {
        // The X is bare until hovered; the bottom buttons always read as buttons, and Reset
        // turns red while it waits for the confirming click.
        let idle = if is_x { Color::NONE } else { Color::srgba(1.0, 1.0, 1.0, 0.07) };
        bg.0 = match (interaction, armed && bar_btn == Some(&BarBtn::Reset)) {
            (Interaction::None, true) => Color::srgba(0.85, 0.2, 0.2, 0.55),
            (_, true) => Color::srgba(0.95, 0.25, 0.25, 0.75),
            (Interaction::None, _) => idle,
            (Interaction::Hovered, _) => Color::srgba(1.0, 1.0, 1.0, 0.14),
            (Interaction::Pressed, _) => Color::srgba(0.35, 0.62, 1.0, 0.35),
        };
    }
    for mut t in &mut reset_label {
        let s = if armed { "Confirm?" } else { "Reset" };
        if t.sections[0].value != s {
            t.sections[0].value = s.to_string();
        }
    }
    for (u, mut style) in &mut underlines {
        let named = menu.entries.get(menu.scroll + u.0).is_some_and(|e| matches!(e.kind, Kind::Header) && !e.label.is_empty());
        let d = if named && u.0 < menu.rows { Display::Flex } else { Display::None };
        if style.display != d {
            style.display = d;
        }
    }
    for (a, interaction, mut style, mut bg) in &mut arrows {
        let show = menu.entries.get(menu.scroll + a.0).is_some_and(has_arrows);
        let d = if show { Display::Flex } else { Display::None };
        if style.display != d {
            style.display = d;
        }
        bg.0 = if *interaction == Interaction::None { Color::NONE } else { Color::srgba(1.0, 1.0, 1.0, 0.15) };
    }
    for mut t in &mut footer {
        let s = if menu.picker.is_some() {
            "Waiting for the file picker...".to_string()
        } else if let Some(g) = game {
            format!("Selected game: {}", g.title)
        } else {
            String::new()
        };
        if t.sections[0].value != s {
            t.sections[0].value = s;
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Input
// ------------------------------------------------------------------------------------------------

fn step_index(menu: &mut Menu, d: i32) {
    let n = menu.entries.len() as i32;
    let mut i = menu.index as i32;
    for _ in 0..n {
        i = (i + d).rem_euclid(n);
        if selectable(&menu.entries[i as usize]) {
            break;
        }
    }
    menu.index = i as usize;
    keep_visible(menu);
}

fn keep_visible(menu: &mut Menu) {
    if menu.index < menu.scroll + 1 {
        menu.scroll = menu.index.saturating_sub(1);
    }
    if menu.index + 2 > menu.scroll + menu.rows {
        menu.scroll = (menu.index + 2).saturating_sub(menu.rows);
    }
    menu.scroll = menu.scroll.min(menu.entries.len().saturating_sub(menu.rows));
}

/// Everything a menu change may need to touch.
#[derive(bevy::ecs::system::SystemParam)]
struct Ctx<'w, 's> {
    cfg: ResMut<'w, Config>,
    lib: ResMut<'w, Library>,
    view: ResMut<'w, ViewState>,
    gfx: ResMut<'w, Gfx>,
    persist: ResMut<'w, Persist>,
    toast: ResMut<'w, Toast>,
    svc: ResMut<'w, CoverService>,
    scanner: ResMut<'w, Scanner>,
    windows: Query<'w, 's, &'static mut Window, With<PrimaryWindow>>,
    winit: ResMut<'w, bevy::winit::WinitSettings>,
}

fn apply(ctx: &mut Ctx, effect: Effect) {
    ctx.persist.mark_cfg();
    match effect {
        Effect::None => {}
        Effect::View => ctx.view.dirty = true,
        Effect::Uninstalled => {
            ctx.view.dirty = true;
            if ctx.cfg.display.show_uninstalled && !ctx.lib.games.iter().any(|g| !g.installed) {
                ctx.cfg.data.scan.owned = true;
                start_scan(&mut ctx.scanner, &ctx.cfg);
                ctx.toast.show("Looking for games you own that are not installed...");
            }
        }
        Effect::Respawn => {
            ctx.gfx.clear();
            ctx.view.respawn = true;
            ctx.view.dirty = true;
            ctx.view.spines.clear();
            ctx.svc.spines_pending.clear();
        }
        Effect::Window => {
            if let Ok(mut w) = ctx.windows.get_single_mut() {
                apply_window(&mut w, &ctx.cfg);
            }
            let overlay = ctx.windows.get_single().is_ok_and(|w| w.transparent);
            *ctx.winit = crate::app::winit_settings(&ctx.cfg, overlay);
        }
        Effect::Zoom => ctx.view.zoom = ctx.cfg.display.zoom.clamp(0.45, 4.0),
        // The menu shows the restart question (see `change`). The overlay always closes when a
        // game starts, so its setting says so too.
        Effect::Restart => {
            if ctx.cfg.general.overlay {
                ctx.cfg.data.general.close_on_launch = true;
            }
        }
        Effect::Rainbow => {
            ctx.gfx.rainbow_seed = 0;
            apply(ctx, Effect::Respawn);
        }
        Effect::Potato => {
            ctx.svc.set_settings(&ctx.cfg.cover_settings());
            ctx.view.covers.clear();
            ctx.view.backs.clear();
            apply(ctx, Effect::Respawn);
        }
        Effect::Covers => {
            ctx.svc.set_settings(&ctx.cfg.cover_settings());
            ctx.svc.missing.clear();
            ctx.view.respawn = true;
        }
    }
}

fn spawn_picker(menu: &mut Menu, f: impl FnOnce() -> Option<Picked> + Send + 'static) {
    if menu.picker.is_some() {
        return;
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    menu.picker = Some(rx);
}

/// Changes the selected entry: `d` = +1 / -1 for next / previous, 0 for "activate".
fn change(menu: &mut Menu, ctx: &mut Ctx, d: i32) {
    let i = menu.index;
    let effect = menu.entries[i].effect;
    let game_id = ctx.view.selected_game(&ctx.lib).map(|g| g.id.clone());
    let dir = if d == 0 { 1 } else { d };
    match &menu.entries[i].kind {
        Kind::Header => {
            let h = menu.entries[i].label;
            if let Some(p) = menu.collapsed.iter().position(|x| *x == h) {
                menu.collapsed.remove(p);
            } else {
                menu.collapsed.push(h);
            }
            rebuild_entries(menu, &ctx.lib, &ctx.cfg);
            if let Some(i) = menu.entries.iter().position(|e| matches!(e.kind, Kind::Header) && e.label == h) {
                menu.index = i;
                keep_visible(menu);
            }
        }
        Kind::Toggle(f) => {
            let v = f(&mut ctx.cfg.data);
            *v = !*v;
            apply(ctx, effect);
            if effect == Effect::Restart {
                menu.restart_prompt = true;
            }
        }
        Kind::Num { f, min, max, step } => {
            let v = f(&mut ctx.cfg.data);
            *v = ((*v + step * dir as f32) / step).round() * step;
            *v = v.clamp(*min, *max);
            apply(ctx, effect);
        }
        Kind::Choice { set, .. } => {
            set(&mut ctx.cfg.data, dir);
            apply(ctx, effect);
        }
        Kind::Source(name) => {
            let list = &mut ctx.cfg.covers.sources;
            if let Some(p) = list.iter().position(|s| s == name) {
                list.remove(p);
            } else {
                list.push(name.to_string());
            }
            apply(ctx, effect);
        }
        Kind::Text(f) => {
            if d == 0 {
                let label = menu.entries[i].label;
                menu.editing = Some(crate::textedit::TextEdit::new(label, f(&mut ctx.cfg.data)));
            }
        }
        Kind::Color(f) => {
            if d == 0 {
                let (label, f) = (menu.entries[i].label, *f);
                open_color_pick(menu, ctx, label, f);
            }
        }
        Kind::GameTitle => {
            if d == 0 {
                menu.editing = ctx.view.selected_game(&ctx.lib).map(|g| crate::textedit::TextEdit::new("Title", &g.title));
            }
        }
        Kind::GameLocation => {
            if d == 0 {
                menu.editing = ctx
                    .view
                    .selected_game(&ctx.lib)
                    .map(|g| crate::textedit::TextEdit::new("Location (file, program or URI; empty = detected)", &g.custom_location.clone().unwrap_or_else(|| g.location())));
            }
        }
        Kind::GameArgs => {
            if d == 0 {
                menu.editing = ctx.view.selected_game(&ctx.lib).map(|g| crate::textedit::TextEdit::new("Launch arguments", &g.launch_args));
            }
        }
        Kind::GameFavorite => {
            if let Some(g) = game_id.as_ref().and_then(|id| ctx.lib.find_mut(id)) {
                g.favorite = !g.favorite;
                ctx.persist.mark_lib();
                ctx.view.dirty = true;
            }
        }
        Kind::GamePlatform => {
            if let Some(g) = game_id.as_ref().and_then(|id| ctx.lib.find_mut(id)) {
                g.platform = cycle_list(Platform::ALL, g.platform, dir);
                g.replatformed = true;
                ctx.lib.revision += 1;
                ctx.persist.mark_lib();
                apply(ctx, effect);
            }
        }
        Kind::HiddenGame(id) => {
            let id = id.clone();
            if let Some(g) = ctx.lib.find_mut(&id) {
                g.hidden = false;
                let title = g.title.clone();
                ctx.lib.revision += 1;
                ctx.persist.mark_lib();
                ctx.view.dirty = true;
                ctx.toast.show(format!("{title} is back"));
            }
        }
        Kind::HiddenTitle(t) => {
            let t = t.clone();
            ctx.cfg.scan.hidden_titles.retain(|h| *h != t);
            ctx.persist.mark_cfg();
            // The scan thread reads the config, so save it first.
            ctx.cfg.save();
            start_scan(&mut ctx.scanner, &ctx.cfg);
            menu.built = None;
            ctx.toast.show(format!("Unhid {t}, scanning for it..."));
        }
        Kind::Note(_) => {}
        Kind::Advanced(h) => {
            let h = *h;
            if let Some(p) = menu.expanded.iter().position(|x| *x == h) {
                menu.expanded.remove(p);
            } else {
                menu.expanded.push(h);
            }
            // Rebuild now and keep the cursor on this row.
            rebuild_entries(menu, &ctx.lib, &ctx.cfg);
            if let Some(i) = menu.entries.iter().position(|e| matches!(e.kind, Kind::Advanced(x) if x == h)) {
                menu.index = i;
                keep_visible(menu);
            }
        }
        Kind::RomPlatform => {
            let mut opts: Vec<Option<Platform>> = vec![None];
            opts.extend(Platform::ALL.iter().copied().filter(|p| *p != Platform::Pc).map(Some));
            menu.rom_platform = cycle_list(&opts, menu.rom_platform, dir);
        }
        Kind::Act(act) => match act {
            Act::Rescan => {
                start_scan(&mut ctx.scanner, &ctx.cfg);
                ctx.toast.show("Scanning for games...");
            }
            Act::AddFiles => spawn_picker(menu, || {
                rfd::FileDialog::new().set_title("Add games (ROMs, disc images, executables)").pick_files().map(Picked::Files)
            }),
            Act::AddFolder => spawn_picker(menu, || rfd::FileDialog::new().set_title("Add a folder of games").pick_folder().map(Picked::Folder)),
            Act::AddRomFolder => {
                let plat = menu.rom_platform;
                spawn_picker(menu, move || {
                    rfd::FileDialog::new().set_title("ROM folder to keep scanning").pick_folder().map(|p| Picked::RomFolder(p, plat))
                })
            }
            Act::SetCover => {
                if let Some(id) = game_id {
                    spawn_picker(menu, move || {
                        rfd::FileDialog::new()
                            .set_title("Cover image")
                            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                            .pick_file()
                            .map(|p| Picked::Cover(id, p))
                    })
                }
            }
            Act::BrowseLocation => {
                if let Some(id) = game_id {
                    spawn_picker(menu, move || rfd::FileDialog::new().set_title("Game location (program, ROM or disc image)").pick_file().map(|p| Picked::Location(id, p)))
                }
            }
            Act::BackgroundImage => spawn_picker(menu, || {
                rfd::FileDialog::new()
                    .set_title("Background picture or animated GIF")
                    .add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "gif", "bmp"])
                    .pick_file()
                    .map(Picked::Background)
            }),
            Act::ChooseArt => {
                if game_id.is_some() {
                    open_art_pick(menu, ctx);
                }
            }
            Act::RefetchCover => {
                if let Some(id) = game_id {
                    crate::covers::forget_cover(&id);
                    ctx.svc.missing.remove(&id);
                    ctx.view.covers.remove(&id);
                    ctx.view.respawn = true;
                    ctx.toast.show("Downloading the cover again...");
                }
            }
            Act::HideGame => {
                if let Some(g) = game_id.as_ref().and_then(|id| ctx.lib.find_mut(id)) {
                    g.hidden = true;
                    let title = g.title.clone();
                    ctx.lib.revision += 1;
                    ctx.persist.mark_lib();
                    ctx.toast.show(format!("Hid {title} (Settings > Hidden games brings it back)"));
                }
            }
        },
    }
}

fn commit_edit(menu: &mut Menu, ctx: &mut Ctx) {
    let Some(text) = menu.editing.take().map(|e| e.text()) else { return };
    let effect = menu.entries[menu.index].effect;
    match &menu.entries[menu.index].kind {
        Kind::Text(f) => {
            *f(&mut ctx.cfg.data) = text.trim().to_string();
            apply(ctx, effect);
        }
        Kind::GameTitle => {
            let id = ctx.view.selected_game(&ctx.lib).map(|g| g.id.clone());
            if let Some(g) = id.as_ref().and_then(|id| ctx.lib.find_mut(id)) {
                if !text.trim().is_empty() {
                    g.title = text.trim().to_string();
                    g.renamed = true;
                }
            }
            ctx.lib.revision += 1;
            ctx.persist.mark_lib();
            ctx.view.respawn = true;
            apply(ctx, effect);
        }
        Kind::GameLocation | Kind::GameArgs => {
            let is_loc = matches!(menu.entries[menu.index].kind, Kind::GameLocation);
            let id = ctx.view.selected_game(&ctx.lib).map(|g| g.id.clone());
            if let Some(g) = id.as_ref().and_then(|id| ctx.lib.find_mut(id)) {
                let t = text.trim().to_string();
                if is_loc {
                    // Clearing it, or leaving the detected one, goes back to the detected location.
                    g.custom_location = None;
                    if !t.is_empty() && t != g.location() {
                        g.custom_location = Some(t);
                    }
                } else {
                    g.launch_args = t;
                }
                ctx.persist.mark_lib();
                ctx.toast.show(if is_loc { "Location saved" } else { "Launch arguments saved" });
            }
        }
        _ => {}
    }
}

fn close(menu: &mut Menu) {
    menu.open = false;
    menu.just_closed = true;
    menu.editing = None;
}

#[allow(clippy::too_many_arguments)]
fn menu_input(
    mut menu: ResMut<Menu>,
    mut ctx: Ctx,
    keys: Res<ButtonInput<KeyCode>>,
    mut typed: EventReader<KeyboardInput>,
    mut wheel: EventReader<MouseWheel>,
    mouse: Res<ButtonInput<MouseButton>>,
    gamepads: Res<Gamepads>,
    pad: Res<ButtonInput<GamepadButton>>,
    rows: Query<(&MenuRow, &Interaction)>,
    arrows: Query<(&ArrowBtn, &Interaction)>,
    panel: Query<&Interaction, With<MenuPanel>>,
    close_btn: Query<&Interaction, With<MenuCloseBtn>>,
    bar: Query<(&BarBtn, &Interaction)>,
    edit: EditInput,
    time: Res<Time>,
    mut exit: EventWriter<AppExit>,
) {
    menu.just_closed = false;
    let pressed_pad = |t: GamepadButtonType| gamepads.iter().any(|g| pad.just_pressed(GamepadButton::new(g, t)));
    let toggle = keys_pressed(&ctx.cfg.keys.settings, &keys) || pressed_pad(GamepadButtonType::Start);
    let typed: Vec<KeyboardInput> = typed.read().cloned().collect();
    if !menu.open {
        if toggle && !ctx.view.searching {
            menu.show();
        }
        wheel.clear();
        return;
    }
    // The colour picker / art chooser has the mouse and keys while it is open. Closing the
    // settings (settings key, X, Close) closes the art chooser with them; Q / Esc close just the
    // art chooser (see `art_pick_input`).
    if menu.color.is_some() || menu.art.is_some() {
        wheel.clear();
        let clicked_close = mouse.just_pressed(MouseButton::Left)
            && (close_btn.iter().any(|i| *i == Interaction::Pressed) || bar.iter().any(|(b, i)| *b == BarBtn::Close && *i == Interaction::Pressed));
        if menu.color.is_none() && (toggle || clicked_close) {
            menu.art = None;
            art_preview(&mut ctx, None);
            close(&mut menu);
        }
        return;
    }

    // As many rows as fit the window.
    if let Ok(w) = ctx.windows.get_single() {
        let fit = (((w.height() - MENU_CHROME) / ROW_H).floor() as usize).clamp(6, MAX_ROWS);
        if fit != menu.rows {
            menu.rows = fit;
            keep_visible(&mut menu);
        }
    }
    // Rebuild the hidden games section when the library changes.
    if menu.built != Some(ctx.lib.revision) {
        rebuild_entries(&mut menu, &ctx.lib, &ctx.cfg);
    }

    // The restart question: Enter / Restart restarts, Esc / Later keeps going.
    if menu.restart_prompt {
        let clicked = if mouse.just_pressed(MouseButton::Left) { edit.restart.iter().find(|(_, i)| **i == Interaction::Pressed).map(|(b, _)| b.0) } else { None };
        let restart = clicked == Some(true) || keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
        let later = clicked == Some(false) || keys.just_pressed(KeyCode::Escape);
        if restart {
            menu.restart_prompt = false;
            restart_app(&mut ctx);
            exit.send(AppExit::Success);
        } else if later {
            menu.restart_prompt = false;
            ctx.toast.show("The change applies the next time Slipcase starts");
        }
        wheel.clear();
        return;
    }

    // The text dialog takes the keyboard and mouse while it is open.
    if menu.editing.is_some() {
        let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight]);
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        let now = time.elapsed_seconds();
        for e in &typed {
            let r = menu.editing.as_mut().map(|ed| ed.key(e, ctrl, shift, now)).unwrap_or(crate::textedit::EditResult::None);
            match r {
                crate::textedit::EditResult::Commit => {
                    commit_edit(&mut menu, &mut ctx);
                    break;
                }
                crate::textedit::EditResult::Cancel => {
                    menu.editing = None;
                    break;
                }
                crate::textedit::EditResult::None => {}
            }
        }
        // Mouse: click places the cursor (shift extends), drag selects, double click picks a word.
        let fonts = &edit.fonts;
        if let (Some(ed), Ok((interaction, rel, node))) = (menu.editing.as_ref(), edit.field.get_single()) {
            let index_at = |x_norm: f32| {
                let x = x_norm * node.size().x - EDIT_PAD + ed.scroll;
                (0..=ed.len()).min_by(|&a, &b| (fonts.width(&ed.prefix(a), EDIT_PX) - x).abs().total_cmp(&(fonts.width(&ed.prefix(b), EDIT_PX) - x).abs())).unwrap_or(0)
            };
            let pos = rel.normalized.map(|p| index_at(p.x));
            if mouse.just_pressed(MouseButton::Left) && *interaction != Interaction::None {
                if let Some(i) = pos {
                    let (t0, i0) = menu.edit_click;
                    let double = now - t0 < 0.4 && i0 == i;
                    let ed = menu.editing.as_mut().unwrap();
                    ed.last_input = now;
                    if double {
                        ed.select_word_at(i.min(ed.len().saturating_sub(1)));
                    } else {
                        ed.move_to(i, shift);
                    }
                    menu.edit_drag = !double;
                    menu.edit_click = (now, i);
                }
            } else if mouse.pressed(MouseButton::Left) && menu.edit_drag {
                if let Some(i) = pos {
                    let ed = menu.editing.as_mut().unwrap();
                    ed.move_to(i, true);
                    ed.last_input = now;
                }
            }
        }
        if mouse.just_released(MouseButton::Left) {
            menu.edit_drag = false;
        }
        if mouse.just_pressed(MouseButton::Left) {
            if let Some((b, _)) = edit.buttons.iter().find(|(_, i)| **i == Interaction::Pressed) {
                match b {
                    EditBtn::Ok => commit_edit(&mut menu, &mut ctx),
                    EditBtn::Cancel => menu.editing = None,
                }
            }
        }
        wheel.clear();
        return;
    }

    // The quit keys (Q, Esc) close the settings rather than quitting from them.
    if toggle || keys.just_pressed(KeyCode::Escape) || keys_pressed(&ctx.cfg.keys.quit, &keys) || pressed_pad(GamepadButtonType::East) {
        close(&mut menu);
        return;
    }
    // Up / down with key repeat.
    let up = keys.pressed(KeyCode::ArrowUp) || keys.pressed(KeyCode::KeyW);
    let down = keys.pressed(KeyCode::ArrowDown) || keys.pressed(KeyCode::KeyS);
    let up_now = keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) || pressed_pad(GamepadButtonType::DPadUp);
    let down_now = keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) || pressed_pad(GamepadButtonType::DPadDown);
    if up_now || down_now {
        menu.repeat = -0.35;
        step_index(&mut menu, if up_now { -1 } else { 1 });
    } else if up || down {
        menu.repeat += time.delta_seconds();
        if menu.repeat > 0.06 {
            menu.repeat = 0.0;
            step_index(&mut menu, if up { -1 } else { 1 });
        }
    }
    let left = keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::KeyA) || pressed_pad(GamepadButtonType::DPadLeft);
    let right = keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::KeyD) || pressed_pad(GamepadButtonType::DPadRight);
    let enter = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) || pressed_pad(GamepadButtonType::South);
    if left {
        change(&mut menu, &mut ctx, -1);
    } else if right {
        change(&mut menu, &mut ctx, 1);
    } else if enter {
        change(&mut menu, &mut ctx, 0);
    }

    if mouse.just_pressed(MouseButton::Left) {
        if close_btn.iter().any(|i| *i == Interaction::Pressed) {
            close(&mut menu);
            return;
        }
        if let Some((b, _)) = bar.iter().find(|(_, i)| **i == Interaction::Pressed) {
            match b {
                BarBtn::Close => {
                    close(&mut menu);
                    return;
                }
                BarBtn::ExpandAll => menu.collapsed.clear(),
                BarBtn::CollapseAll => menu.collapsed = category_headers(),
                BarBtn::Reset => {
                    let now = time.elapsed_seconds();
                    if menu.reset_armed_until > now {
                        menu.reset_armed_until = 0.0;
                        reset_settings(&mut ctx);
                    } else {
                        menu.reset_armed_until = now + RESET_CONFIRM_SECS;
                        ctx.toast.show("Click Confirm to put every setting back to its default");
                    }
                }
            }
            rebuild_entries(&mut menu, &ctx.lib, &ctx.cfg);
        }
    }
    // Mouse: the wheel scrolls the menu only over the panel (elsewhere it browses the games).
    let over_panel = panel
        .iter()
        .chain(close_btn.iter())
        .chain(bar.iter().map(|(_, i)| i))
        .chain(rows.iter().map(|(_, i)| i))
        .chain(arrows.iter().map(|(_, i)| i))
        .any(|i| *i != Interaction::None);
    for e in wheel.read() {
        if !over_panel {
            continue;
        }
        let y = if e.unit == MouseScrollUnit::Pixel { e.y / 40.0 } else { e.y };
        let max = menu.entries.len().saturating_sub(menu.rows);
        if y > 0.0 {
            menu.scroll = menu.scroll.saturating_sub(2);
        } else if y < 0.0 {
            menu.scroll = (menu.scroll + 2).min(max);
        }
    }
    // The < and > buttons step backwards / forwards (they sit on top of the row).
    let mut arrow_used = false;
    for (a, interaction) in &arrows {
        let i = menu.scroll + a.0;
        if *interaction == Interaction::None || i >= menu.entries.len() {
            continue;
        }
        menu.index = i;
        arrow_used = true;
        if mouse.just_pressed(MouseButton::Left) {
            change(&mut menu, &mut ctx, a.1);
        }
    }
    if arrow_used {
        return;
    }
    for (row, interaction) in &rows {
        let i = menu.scroll + row.0;
        if i >= menu.entries.len() || !selectable(&menu.entries[i]) {
            continue;
        }
        if *interaction != Interaction::None {
            menu.index = i;
            if mouse.just_pressed(MouseButton::Left) {
                change(&mut menu, &mut ctx, 0);
            } else if mouse.just_pressed(MouseButton::Right) {
                change(&mut menu, &mut ctx, -1);
            }
        }
    }
}

/// Every setting back to its default, except what describes the user's library rather than
/// how it looks: scanned folders, hidden games, per-console overrides (emulator commands)
/// and cover API keys.
fn reset_settings(ctx: &mut Ctx) {
    let old = std::mem::take(&mut ctx.cfg.data);
    let d = &mut ctx.cfg.data;
    d.scan = old.scan;
    d.platforms = old.platforms;
    d.covers.steamgriddb_api_key = old.covers.steamgriddb_api_key;
    d.covers.thegamesdb_api_key = old.covers.thegamesdb_api_key;
    ctx.view.zoom = ctx.cfg.display.zoom.clamp(0.45, 4.0);
    ctx.view.grid_free = false;
    for effect in [Effect::Respawn, Effect::Window, Effect::Covers] {
        apply(ctx, effect);
    }
    ctx.cfg.save();
    ctx.toast.show("Settings reset to defaults (bloom, overlay and anti-aliasing apply after a restart)");
}

/// Static entries plus one row per hidden game after the "Hidden games" header.
fn rebuild_entries(menu: &mut Menu, lib: &Library, cfg: &Config) {
    let mut all = entries();
    if let Some(h) = all.iter().position(|e| matches!(e.kind, Kind::Header) && e.label == HIDDEN_HEADER) {
        let tail = all.split_off(h + 1);
        all.extend(hidden_entries(lib, cfg));
        all.extend(tail);
    }
    // Each category: its everyday settings, then an "Advanced" row, then (when that is open)
    // the technical ones.
    let mut list: Vec<Entry> = Vec::new();
    let mut header: &'static str = "";
    let mut held: Vec<Entry> = Vec::new();
    let flush = |list: &mut Vec<Entry>, held: &mut Vec<Entry>, header: &'static str, expanded: &[&'static str]| {
        if !held.is_empty() {
            list.push(Entry { label: "", kind: Kind::Advanced(header), effect: Effect::None, advanced: false });
            if expanded.contains(&header) {
                list.append(held);
            }
            held.clear();
        }
    };
    for e in all {
        if matches!(e.kind, Kind::Header) {
            flush(&mut list, &mut held, header, &menu.expanded);
            header = e.label;
            list.push(e);
        } else if menu.collapsed.contains(&header) {
            // Folded category: only its header shows.
        } else if e.advanced {
            held.push(e);
        } else {
            list.push(e);
        }
    }
    flush(&mut list, &mut held, header, &menu.expanded);
    menu.entries = list;
    menu.built = Some(lib.revision);
    menu.index = menu.index.min(menu.entries.len() - 1);
    if !selectable(&menu.entries[menu.index]) {
        step_index(menu, 1);
    }
    keep_visible(menu);
}

fn menu_pickers(mut menu: ResMut<Menu>, mut ctx: Ctx) {
    let Some(rx) = &menu.picker else { return };
    let Ok(result) = rx.try_recv() else { return };
    menu.picker = None;
    let Some(picked) = result else { return };
    match picked {
        Picked::Files(files) => {
            let ids: Vec<String> = files.iter().filter_map(|f| ctx.lib.add_manual(f, None, None, &ctx.cfg)).collect();
            crate::app::fetch_new(&mut ctx.svc, &ctx.lib, &ids, &ctx.cfg);
            let n = ids.len();
            let msg = match ids.first().and_then(|id| ctx.lib.find(id)) {
                Some(g) if n == 1 => format!("Added {} ({})", g.title, g.platform.display_name()),
                _ => format!("Added {n} game{}", if n == 1 { "" } else { "s" }),
            };
            ctx.toast.show(msg);
        }
        Picked::Folder(dir) => {
            let mut found = crate::scanner::scan_rom_dir(&RomDir { path: dir.to_string_lossy().into(), ..default() }, &ctx.cfg);
            found.extend(crate::scanner::scan_game_dir(&dir));
            let n = found.len();
            let ids: Vec<String> = found.iter().map(|g| g.id.clone()).collect();
            for mut g in found {
                g.manual = true;
                ctx.lib.upsert(g);
            }
            ctx.lib.revision += 1;
            ctx.persist.mark_lib();
            crate::app::fetch_new(&mut ctx.svc, &ctx.lib, &ids, &ctx.cfg);
            ctx.toast.show(format!("Added {n} games from the folder"));
        }
        Picked::RomFolder(dir, plat) => {
            ctx.cfg.scan.rom_dirs.push(RomDir { path: dir.to_string_lossy().into(), platform: plat.map(|p| p.id().to_string()).unwrap_or_default(), ..default() });
            ctx.persist.mark_cfg();
            // The scan thread reads the config, so save it first.
            ctx.cfg.save();
            start_scan(&mut ctx.scanner, &ctx.cfg);
            ctx.toast.show("ROM folder added, scanning...");
        }
        Picked::Cover(id, path) => {
            if let Some(g) = ctx.lib.find_mut(&id) {
                g.cover_path = Some(path.to_string_lossy().into());
            }
            ctx.persist.mark_lib();
            ctx.svc.missing.remove(&id);
            ctx.view.covers.remove(&id);
            ctx.view.respawn = true;
            ctx.toast.show("Cover set");
        }
        Picked::Background(path) => {
            ctx.cfg.data.background.custom_path = path.to_string_lossy().into_owned();
            ctx.cfg.data.background.style = crate::config::BackgroundStyle::Custom;
            ctx.persist.mark_cfg();
            ctx.toast.show("Background picture set");
        }
        Picked::Location(id, path) => {
            if let Some(g) = ctx.lib.find_mut(&id) {
                g.custom_location = Some(path.to_string_lossy().into());
            }
            ctx.persist.mark_lib();
            ctx.toast.show("Location set");
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Colour picker (the background colours)
// ------------------------------------------------------------------------------------------------

/// A colour being picked: which setting, the colour as hue / saturation / value, and what to
/// go back to on Cancel. The setting changes live, so the background shows the colour at once.
struct ColorPick {
    target: fn(&mut ConfigData) -> &mut String,
    title: &'static str,
    hsv: Vec3,
    original: String,
    was_adaptive: bool,
    /// Dragging in the square (1) or along the hue bar (2).
    drag: u8,
}

#[derive(Resource)]
struct PickImages {
    square: Handle<Image>,
}

#[derive(Component)]
struct PickLayer;
#[derive(Component)]
struct PickTitle;
#[derive(Component)]
struct PickSquare;
#[derive(Component)]
struct PickDot;
#[derive(Component)]
struct PickHue;
#[derive(Component)]
struct PickHueMark;
#[derive(Component)]
struct PickPreview;
#[derive(Component)]
struct PickHex;
#[derive(Component)]
struct PickNote;
/// OK (true) / Cancel (false).
#[derive(Component, Clone, Copy)]
struct PickBtn(bool);
#[derive(Component, Clone, Copy)]
struct PickPreset(&'static str);

const PICK_W: f32 = 272.0;
const PICK_SQ_H: f32 = 170.0;
const PICK_PRESETS: [&str; 10] = ["#0b1d3a", "#1fd6a1", "#7a3cff", "#ff2fa0", "#ff7a1a", "#ffd23f", "#2fd2ff", "#3b6bff", "#e8e8f0", "#141418"];

fn pick_hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

fn pick_rgb_to_hsv(c: [f32; 3]) -> Vec3 {
    let (r, g, b) = (c[0], c[1], c[2]);
    let mx = r.max(g).max(b);
    let d = mx - r.min(g).min(b);
    let h = if d == 0.0 {
        0.0
    } else if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    Vec3::new(h, if mx == 0.0 { 0.0 } else { d / mx }, mx)
}

fn pick_hex(hsv: Vec3) -> String {
    let [r, g, b] = pick_hsv_to_rgb(hsv.x, hsv.y, hsv.z);
    let u = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", u(r), u(g), u(b))
}

/// Opens the picker on a colour setting (the background stops following the cover while it is
/// open, so the colour shows).
fn open_color_pick(menu: &mut Menu, ctx: &mut Ctx, title: &'static str, f: fn(&mut ConfigData) -> &mut String) {
    let original = f(&mut ctx.cfg.data).clone();
    let was_adaptive = ctx.cfg.background.adaptive;
    ctx.cfg.data.background.adaptive = false;
    menu.color = Some(ColorPick { target: f, title, hsv: pick_rgb_to_hsv(crate::config::parse_hex(&original)), original, was_adaptive, drag: 0 });
}

fn spawn_color_picker(mut commands: Commands, fonts: Res<MenuFonts>, mut images: ResMut<Assets<Image>>) {
    use bevy::render::render_asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let square = images.add(Image::new_fill(Extent3d { width: 64, height: 64, depth_or_array_layers: 1 }, TextureDimension::D2, &[0, 0, 0, 255], TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default()));
    // The hue bar never changes.
    let mut hue = Vec::with_capacity(128 * 4);
    for x in 0..128 {
        let [r, g, b] = pick_hsv_to_rgb(x as f32 / 127.0, 1.0, 1.0);
        hue.extend_from_slice(&[(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255]);
    }
    let hue = images.add(Image::new(Extent3d { width: 128, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, hue, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default()));
    commands.insert_resource(PickImages { square: square.clone() });

    let f = fonts.clone();
    let txt = |font: &Handle<Font>, size: f32, color: Color, s: &str| TextBundle::from_section(s.to_string(), TextStyle { font: font.clone(), font_size: size, color });
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    display: Display::None,
                    ..default()
                },
                background_color: BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.2)),
                z_index: ZIndex::Global(65),
                focus_policy: bevy::ui::FocusPolicy::Block,
                ..default()
            },
            PickLayer,
            Interaction::default(),
        ))
        .with_children(|o| {
            o.spawn(NodeBundle {
                style: Style {
                    width: Val::Px(PICK_W + 28.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(14.0)),
                    row_gap: Val::Px(10.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                background_color: BackgroundColor(Color::srgb(0.07, 0.08, 0.11)),
                border_color: BorderColor(Color::srgba(0.55, 0.78, 1.0, 0.5)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            })
            .with_children(|d| {
                d.spawn((txt(&f.bold, 15.0, ACCENT, ""), PickTitle));
                // Saturation (across) and brightness (up) for the current hue.
                d.spawn((
                    ImageBundle {
                        style: Style { width: Val::Px(PICK_W), height: Val::Px(PICK_SQ_H), ..default() },
                        image: UiImage::new(square),
                        focus_policy: bevy::ui::FocusPolicy::Block,
                        ..default()
                    },
                    BorderRadius::all(Val::Px(5.0)),
                    PickSquare,
                    Interaction::default(),
                    bevy::ui::RelativeCursorPosition::default(),
                ))
                .with_children(|sq| {
                    sq.spawn((
                        NodeBundle {
                            style: Style { position_type: PositionType::Absolute, width: Val::Px(14.0), height: Val::Px(14.0), border: UiRect::all(Val::Px(2.0)), ..default() },
                            border_color: BorderColor(Color::WHITE),
                            border_radius: BorderRadius::all(Val::Px(7.0)),
                            focus_policy: bevy::ui::FocusPolicy::Pass,
                            ..default()
                        },
                        PickDot,
                    ));
                });
                d.spawn((
                    ImageBundle {
                        style: Style { width: Val::Px(PICK_W), height: Val::Px(16.0), ..default() },
                        image: UiImage::new(hue),
                        focus_policy: bevy::ui::FocusPolicy::Block,
                        ..default()
                    },
                    BorderRadius::all(Val::Px(4.0)),
                    PickHue,
                    Interaction::default(),
                    bevy::ui::RelativeCursorPosition::default(),
                ))
                .with_children(|h| {
                    h.spawn((
                        NodeBundle {
                            style: Style { position_type: PositionType::Absolute, top: Val::Px(-3.0), width: Val::Px(6.0), height: Val::Px(22.0), border: UiRect::all(Val::Px(2.0)), ..default() },
                            border_color: BorderColor(Color::WHITE),
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            focus_policy: bevy::ui::FocusPolicy::Pass,
                            ..default()
                        },
                        PickHueMark,
                    ));
                });
                d.spawn(NodeBundle { style: Style { align_items: AlignItems::Center, column_gap: Val::Px(10.0), ..default() }, ..default() }).with_children(|row| {
                    row.spawn((
                        NodeBundle {
                            style: Style { width: Val::Px(46.0), height: Val::Px(26.0), border: UiRect::all(Val::Px(1.0)), ..default() },
                            border_color: BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.3)),
                            border_radius: BorderRadius::all(Val::Px(5.0)),
                            ..default()
                        },
                        PickPreview,
                    ));
                    row.spawn((txt(&f.regular, 15.0, TEXT, ""), PickHex));
                });
                d.spawn(NodeBundle { style: Style { column_gap: Val::Px(5.0), ..default() }, ..default() }).with_children(|row| {
                    for hex in PICK_PRESETS {
                        let c = crate::config::parse_hex(hex);
                        row.spawn((
                            ButtonBundle {
                                style: Style { width: Val::Px(22.5), height: Val::Px(22.0), border: UiRect::all(Val::Px(1.0)), ..default() },
                                background_color: BackgroundColor(Color::srgb(c[0], c[1], c[2])),
                                border_color: BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.25)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                ..default()
                            },
                            PickPreset(hex),
                        ));
                    }
                });
                d.spawn((txt(&f.regular, 12.0, DIM, ""), PickNote));
                d.spawn(NodeBundle { style: Style { align_items: AlignItems::Center, column_gap: Val::Px(6.0), ..default() }, ..default() }).with_children(|row| {
                    let mut hint = txt(&f.regular, 12.0, DIM, "Enter keeps, Esc cancels");
                    hint.style.flex_grow = 1.0;
                    row.spawn(hint);
                    for (b, label) in [(false, "Cancel"), (true, "OK")] {
                        row.spawn((
                            ButtonBundle {
                                style: Style { width: Val::Px(70.0), height: Val::Px(26.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                                border_radius: BorderRadius::all(Val::Px(5.0)),
                                background_color: BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                                ..default()
                            },
                            PickBtn(b),
                        ))
                        .with_children(|t| {
                            t.spawn(txt(&f.bold, 13.5, TEXT, label));
                        });
                    }
                });
            });
        });
}

/// Mouse and keys while the picker is open: drag in the square or along the hue bar, click a
/// preset, OK / Enter keeps the colour, Cancel / Esc puts the old one back.
#[allow(clippy::type_complexity)]
fn color_pick_input(
    mut menu: ResMut<Menu>,
    mut cfg: ResMut<Config>,
    mut persist: ResMut<Persist>,
    mut toast: ResMut<Toast>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    square: Query<(&Interaction, &bevy::ui::RelativeCursorPosition), (With<PickSquare>, Without<PickHue>)>,
    hue: Query<(&Interaction, &bevy::ui::RelativeCursorPosition), (With<PickHue>, Without<PickSquare>)>,
    presets: Query<(&PickPreset, &Interaction)>,
    buttons: Query<(&PickBtn, &Interaction)>,
) {
    let Some(pick) = menu.color.as_mut() else { return };
    let before = pick.hsv;
    let clicked = mouse.just_pressed(MouseButton::Left);
    if clicked {
        if square.iter().any(|(i, _)| *i == Interaction::Pressed) {
            pick.drag = 1;
        } else if hue.iter().any(|(i, _)| *i == Interaction::Pressed) {
            pick.drag = 2;
        } else if let Some((p, _)) = presets.iter().find(|(_, i)| **i == Interaction::Pressed) {
            pick.hsv = pick_rgb_to_hsv(crate::config::parse_hex(p.0));
        }
    }
    if !mouse.pressed(MouseButton::Left) {
        pick.drag = 0;
    }
    match pick.drag {
        1 => {
            if let Some(n) = square.get_single().ok().and_then(|(_, r)| r.normalized) {
                pick.hsv.y = n.x.clamp(0.0, 1.0);
                pick.hsv.z = 1.0 - n.y.clamp(0.0, 1.0);
            }
        }
        2 => {
            if let Some(n) = hue.get_single().ok().and_then(|(_, r)| r.normalized) {
                pick.hsv.x = n.x.clamp(0.0, 0.9999);
            }
        }
        _ => {}
    }
    if pick.hsv != before {
        *(pick.target)(&mut cfg.data) = pick_hex(pick.hsv);
        persist.mark_cfg();
    }
    let button = if clicked { buttons.iter().find(|(_, i)| **i == Interaction::Pressed).map(|(b, _)| b.0) } else { None };
    let keep = button == Some(true) || keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    let cancel = button == Some(false) || keys.just_pressed(KeyCode::Escape);
    if keep {
        let pick = menu.color.take().unwrap();
        if pick.was_adaptive {
            toast.show("Colours follow the cover is now off, so your colours show");
        }
        persist.mark_cfg();
    } else if cancel {
        let pick = menu.color.take().unwrap();
        *(pick.target)(&mut cfg.data) = pick.original;
        cfg.data.background.adaptive = pick.was_adaptive;
        persist.mark_cfg();
    }
}

#[allow(clippy::type_complexity)]
fn draw_color_picker(
    menu: Res<Menu>,
    imgs: Option<Res<PickImages>>,
    mut images: ResMut<Assets<Image>>,
    mut last_hue: Local<Option<f32>>,
    mut layer: Query<&mut Style, (With<PickLayer>, Without<PickDot>, Without<PickHueMark>)>,
    mut dot: Query<&mut Style, (With<PickDot>, Without<PickLayer>, Without<PickHueMark>)>,
    mut mark: Query<&mut Style, (With<PickHueMark>, Without<PickLayer>, Without<PickDot>)>,
    mut preview: Query<&mut BackgroundColor, (With<PickPreview>, Without<PickBtn>)>,
    mut texts: Query<(&mut Text, Option<&PickTitle>, Option<&PickHex>, Option<&PickNote>)>,
    mut buttons: Query<(&Interaction, &mut BackgroundColor), (With<PickBtn>, Without<PickPreview>)>,
) {
    let open = menu.color.is_some();
    for mut s in &mut layer {
        let d = if open { Display::Flex } else { Display::None };
        if s.display != d {
            s.display = d;
        }
    }
    let Some(pick) = menu.color.as_ref() else {
        *last_hue = None;
        return;
    };
    let hsv = pick.hsv;
    // The square shows every saturation and brightness of the current hue.
    if *last_hue != Some(hsv.x) {
        *last_hue = Some(hsv.x);
        if let Some(img) = imgs.as_ref().and_then(|i| images.get_mut(&i.square)) {
            for y in 0..64usize {
                for x in 0..64usize {
                    let [r, g, b] = pick_hsv_to_rgb(hsv.x, x as f32 / 63.0, 1.0 - y as f32 / 63.0);
                    let o = (y * 64 + x) * 4;
                    img.data[o..o + 4].copy_from_slice(&[(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255]);
                }
            }
        }
    }
    for mut s in &mut dot {
        s.left = Val::Px(hsv.y * PICK_W - 7.0);
        s.top = Val::Px((1.0 - hsv.z) * PICK_SQ_H - 7.0);
    }
    for mut s in &mut mark {
        s.left = Val::Px(hsv.x * PICK_W - 3.0);
    }
    let [r, g, b] = pick_hsv_to_rgb(hsv.x, hsv.y, hsv.z);
    for mut c in &mut preview {
        c.0 = Color::srgb(r, g, b);
    }
    let hex = pick_hex(hsv);
    let note = if pick.was_adaptive { "\"Colours follow the cover\" turns off so these colours show." } else { "" };
    for (mut t, title, hx, nt) in &mut texts {
        let want = if title.is_some() {
            pick.title
        } else if hx.is_some() {
            hex.as_str()
        } else if nt.is_some() {
            note
        } else {
            continue;
        };
        if t.sections[0].value != want {
            t.sections[0].value = want.to_string();
        }
    }
    for (i, mut bg) in &mut buttons {
        bg.0 = match i {
            Interaction::None => Color::srgba(1.0, 1.0, 1.0, 0.08),
            Interaction::Hovered => Color::srgba(1.0, 1.0, 1.0, 0.16),
            Interaction::Pressed => Color::srgba(0.35, 0.62, 1.0, 0.4),
        };
    }
}


// ------------------------------------------------------------------------------------------------
// Art chooser (Selected game > Choose art): the cover, the disc / cartridge label and the insert
// ------------------------------------------------------------------------------------------------

use crate::render::ArtKind;

/// Thumbnails shown at a time (3 across, scrolled by rows).
const ART_COLS: usize = 3;
const ART_ROWS: usize = 4;
const ART_SLOTS: usize = 36;
const ART_THUMB: f32 = 132.0;
const ART_CELL_H: f32 = ART_THUMB + 34.0;

enum ArtMsg {
    List(Vec<crate::covers::ArtCandidate>),
    Thumb(usize, PathBuf, u32, u32, Vec<u8>),
    Failed(usize),
}

struct ArtCand {
    cand: crate::covers::ArtCandidate,
    file: Option<PathBuf>,
    thumb: Option<(Handle<Image>, u32, u32)>,
    failed: bool,
}

struct ArtPick {
    game_id: String,
    title: String,
    tab: ArtKind,
    cands: Vec<ArtCand>,
    listed: bool,
    rx: Receiver<ArtMsg>,
    /// Rows scrolled down.
    scroll: usize,
    /// A local file being chosen (the file dialog runs on its own thread).
    file_rx: Option<Receiver<Option<PathBuf>>>,
    /// RGBA thumbnails waiting to become images (made in the draw system).
    pending: Vec<(usize, u32, u32, Vec<u8>)>,
    /// Picked this time, per surface (the candidate, or None for a file / automatic).
    in_use: std::collections::HashMap<ArtKind, Option<usize>>,
}

/// How well a kind of image suits each surface (lower first; unsuited ones go last).
fn art_rank(key: &str, tab: ArtKind) -> u8 {
    // What a surface shows now leads its own tab and is left out of the others' order.
    if let Some(k) = key.strip_prefix("made_") {
        let mine = matches!((k, tab), ("media", ArtKind::Media) | ("insert", ArtKind::Insert) | ("back", ArtKind::Back) | ("spine", ArtKind::Spine));
        return if mine { 0 } else { 250 };
    }
    let order: &[&str] = match tab {
        ArtKind::Cover => &["current", "front", "grid", "back", "header", "hero", "fanart", "shot", "disc", "cart", "logo"],
        ArtKind::Media => &["disc", "cart", "hero", "fanart", "logo", "shot", "grid", "front", "current", "header", "back"],
        ArtKind::Insert => &["shot", "fanart", "hero", "back", "front", "grid", "current", "header", "disc", "cart", "logo"],
        ArtKind::Back => &["back", "shot", "fanart", "hero", "header", "grid", "front", "current", "logo", "disc", "cart"],
        ArtKind::Spine => &["spine", "logo", "hero", "fanart", "header", "grid", "front", "current", "shot", "back", "disc", "cart"],
    };
    order.iter().position(|k| *k == key).unwrap_or(order.len()) as u8
}

impl ArtPick {
    /// Candidates in this tab's order (only those whose thumbnail loaded, or still loading).
    fn order(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.cands.len()).filter(|&i| !self.cands[i].failed && art_rank(self.cands[i].cand.key, self.tab) < 250).collect();
        v.sort_by_key(|&i| (art_rank(self.cands[i].cand.key, self.tab), i));
        v
    }
}

/// Shows the selected game so the surface being picked for is in view: turned round for the
/// back, onto its spine for the spine, opened for the disc / cartridge and the insert, and back
/// to normal (front, closed) for the cover or when the chooser closes.
fn art_preview(ctx: &mut Ctx, tab: Option<ArtKind>) {
    let case = matches!(ctx.cfg.display.item_style, ItemStyle::Case | ItemStyle::CaseDisc);
    let view = &mut *ctx.view;
    view.free_turned = false;
    view.spin_vel = 0.0;
    let turn = match tab {
        Some(ArtKind::Back) => std::f32::consts::PI,
        Some(ArtKind::Spine) => std::f32::consts::FRAC_PI_2,
        _ => 0.0,
    };
    view.turn_target = Vec2::new(turn, 0.0);
    if case && matches!(tab, Some(ArtKind::Media | ArtKind::Insert)) {
        view.inspect = view.selected_id.clone();
        view.inspect_open = true;
    } else {
        view.inspect_open = false;
    }
}

fn open_art_pick(menu: &mut Menu, ctx: &mut Ctx) {
    let Some(g) = ctx.view.selected_game(&ctx.lib) else { return };
    let src = crate::covers::ArtSource::from_game(g, &ctx.cfg);
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::spawn(move || {
        let list = crate::covers::art_candidates(&src);
        let srcs: Vec<String> = list.iter().map(|c| c.src.clone()).collect();
        if tx.send(ArtMsg::List(list)).is_err() {
            return;
        }
        // A few at a time, so a slow server does not hold up the rest.
        let tx2 = tx.clone();
        let jobs: Vec<(usize, String)> = srcs.into_iter().enumerate().collect();
        let chunks: Vec<Vec<(usize, String)>> = jobs.chunks(4).map(|c| c.to_vec()).collect();
        let mut handles = Vec::new();
        for lane in 0..4 {
            let tx = tx2.clone();
            let mine: Vec<(usize, String)> = chunks.iter().filter_map(|c| c.get(lane).cloned()).collect();
            handles.push(std::thread::spawn(move || {
                for (i, s) in mine {
                    let msg = match crate::covers::art_thumbnail(&s) {
                        Some((f, w, h, px)) => ArtMsg::Thumb(i, f, w, h, px),
                        None => ArtMsg::Failed(i),
                    };
                    if tx.send(msg).is_err() {
                        return;
                    }
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
    });
    let (gid, gtitle) = (g.id.clone(), g.title.clone());
    art_preview(ctx, Some(ArtKind::Cover));
    menu.art = Some(ArtPick { game_id: gid, title: gtitle, tab: ArtKind::Cover, cands: Vec::new(), listed: false, rx, scroll: 0, file_rx: None, pending: Vec::new(), in_use: Default::default() });
}

/// Uses `file` (None = automatic) for one of the game's surfaces: copied next to the cache so
/// it stays put, saved with the game, and the game's items rebuilt with it.
fn set_game_art(ctx: &mut Ctx, id: &str, kind: ArtKind, file: Option<PathBuf>) {
    let kept = file.and_then(|f| {
        let dir = crate::config::cache_dir().join("chosen");
        let _ = std::fs::create_dir_all(&dir);
        let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let dest = dir.join(format!("{safe}_{kind:?}_{stamp}.img").to_lowercase());
        std::fs::copy(&f, &dest).ok().map(|_| dest.to_string_lossy().into_owned())
    });
    let Some(g) = ctx.lib.find_mut(id) else { return };
    match kind {
        ArtKind::Cover => g.cover_path = kept.clone(),
        ArtKind::Media => g.media_art = kept.clone(),
        ArtKind::Insert => g.insert_art = kept.clone(),
        ArtKind::Back => g.back_art = kept.clone(),
        ArtKind::Spine => g.spine_art = kept.clone(),
    }
    ctx.persist.mark_lib();
    if kind == ArtKind::Cover {
        if kept.is_none() {
            crate::covers::forget_cover(id);
        }
        ctx.svc.missing.remove(id);
        ctx.svc.pending.remove(id);
        ctx.view.covers.remove(id);
    }
    if kind == ArtKind::Back {
        ctx.svc.backs_pending.remove(id);
        ctx.view.backs.remove(id);
    }
    if kind == ArtKind::Spine {
        ctx.svc.spines_pending.remove(id);
        ctx.view.spines.remove(id);
    }
    ctx.svc.forget_art(id);
    ctx.view.arts.retain(|(i, _), _| i != id);
    ctx.view.respawn = true;
    let what = match kind {
        ArtKind::Cover => "Cover",
        ArtKind::Media => "Disc / cartridge art",
        ArtKind::Insert => "Insert art",
        ArtKind::Back => "Back cover",
        ArtKind::Spine => "Spine art",
    };
    ctx.toast.show(if kept.is_some() { format!("{what} set") } else { format!("{what} back to automatic") });
}

#[derive(Component)]
struct ArtLayer;
#[derive(Component)]
struct ArtTitle;
#[derive(Component)]
struct ArtStatus;
#[derive(Component, Clone, Copy)]
struct ArtTab(ArtKind);
#[derive(Component, Clone, Copy)]
struct ArtThumb(usize);
#[derive(Component, Clone, Copy)]
struct ArtThumbImg(usize);
#[derive(Component, Clone, Copy)]
struct ArtThumbLabel(usize);
#[derive(Component)]
struct ArtGrid;
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ArtBtn {
    Auto,
    File,
    Close,
}

fn spawn_art_picker(mut commands: Commands, fonts: Res<MenuFonts>) {
    let f = fonts.clone();
    let txt = |font: &Handle<Font>, size: f32, color: Color, s: &str| TextBundle::from_section(s.to_string(), TextStyle { font: font.clone(), font_size: size, color });
    let width = ART_COLS as f32 * (ART_THUMB + 10.0) + 28.0;
    let button = |w: f32| ButtonBundle {
        style: Style { width: Val::Px(w), height: Val::Px(30.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
        border_radius: BorderRadius::all(Val::Px(6.0)),
        background_color: BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
        ..default()
    };
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    // Over the settings panel (not needed while picking), so the game in the
                    // middle of the screen stays in view; only the box itself takes the mouse,
                    // so the settings' Close button below it still works.
                    justify_content: JustifyContent::FlexEnd,
                    align_items: AlignItems::Center,
                    padding: UiRect::right(Val::Px(((PANEL_W - (ART_COLS as f32 * (ART_THUMB + 10.0) + 28.0)) * 0.5).max(8.0))),
                    display: Display::None,
                    ..default()
                },
                background_color: BackgroundColor(Color::NONE),
                z_index: ZIndex::Global(66),
                focus_policy: bevy::ui::FocusPolicy::Pass,
                ..default()
            },
            ArtLayer,
        ))
        .with_children(|o| {
            o.spawn((
                NodeBundle {
                    style: Style { width: Val::Px(width), flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(14.0)), row_gap: Val::Px(10.0), border: UiRect::all(Val::Px(1.0)), ..default() },
                    background_color: BackgroundColor(Color::srgb(0.07, 0.08, 0.11)),
                    border_color: BorderColor(Color::srgba(0.55, 0.78, 1.0, 0.5)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    focus_policy: bevy::ui::FocusPolicy::Block,
                    ..default()
                },
                Interaction::default(),
            ))
            .with_children(|d| {
                d.spawn((txt(&f.bold, 17.0, Color::WHITE, ""), ArtTitle));
                d.spawn(NodeBundle { style: Style { column_gap: Val::Px(4.0), ..default() }, ..default() }).with_children(|row| {
                    for (kind, label) in [(ArtKind::Cover, "Cover"), (ArtKind::Back, "Back"), (ArtKind::Spine, "Spine"), (ArtKind::Media, "Disc/Cart"), (ArtKind::Insert, "Insert")] {
                        row.spawn((button(if kind == ArtKind::Media { 92.0 } else { 78.0 }), ArtTab(kind))).with_children(|t| {
                            t.spawn(txt(&f.bold, 14.0, TEXT, label));
                        });
                    }
                });
                // The thumbnails, scrolled by rows inside a clipped area.
                d.spawn(NodeBundle { style: Style { width: Val::Percent(100.0), height: Val::Px(ART_CELL_H * ART_ROWS as f32), overflow: Overflow::clip(), ..default() }, ..default() })
                    .with_children(|clip| {
                        clip.spawn((
                            NodeBundle { style: Style { position_type: PositionType::Absolute, width: Val::Percent(100.0), flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(10.0), ..default() }, ..default() },
                            ArtGrid,
                        ))
                        .with_children(|grid| {
                            for i in 0..ART_SLOTS {
                                grid.spawn((
                                    ButtonBundle {
                                        style: Style {
                                            width: Val::Px(ART_THUMB),
                                            height: Val::Px(ART_CELL_H),
                                            flex_direction: FlexDirection::Column,
                                            align_items: AlignItems::Center,
                                            display: Display::None,
                                            ..default()
                                        },
                                        background_color: BackgroundColor(Color::NONE),
                                        ..default()
                                    },
                                    ArtThumb(i),
                                ))
                                .with_children(|cell| {
                                    cell.spawn(NodeBundle {
                                        style: Style { width: Val::Px(ART_THUMB), height: Val::Px(ART_THUMB), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border: UiRect::all(Val::Px(2.0)), ..default() },
                                        background_color: BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.05)),
                                        border_color: BorderColor(Color::NONE),
                                        border_radius: BorderRadius::all(Val::Px(6.0)),
                                        focus_policy: bevy::ui::FocusPolicy::Pass,
                                        ..default()
                                    })
                                    .with_children(|frame| {
                                        frame.spawn((
                                            ImageBundle { style: Style { width: Val::Px(10.0), height: Val::Px(10.0), ..default() }, focus_policy: bevy::ui::FocusPolicy::Pass, ..default() },
                                            ArtThumbImg(i),
                                        ));
                                    });
                                    let mut t = txt(&f.regular, 11.0, DIM, "");
                                    t.style.max_width = Val::Px(ART_THUMB);
                                    t.style.margin = UiRect::top(Val::Px(3.0));
                                    t.focus_policy = bevy::ui::FocusPolicy::Pass;
                                    cell.spawn((t, ArtThumbLabel(i)));
                                });
                            }
                        });
                    });
                d.spawn((txt(&f.regular, 12.5, DIM, ""), ArtStatus));
                d.spawn(NodeBundle { style: Style { align_items: AlignItems::Center, column_gap: Val::Px(8.0), ..default() }, ..default() }).with_children(|row| {
                    row.spawn((button(100.0), ArtBtn::Auto)).with_children(|t| {
                        t.spawn(txt(&f.bold, 13.5, TEXT, "Automatic"));
                    });
                    row.spawn((button(130.0), ArtBtn::File)).with_children(|t| {
                        t.spawn(txt(&f.bold, 13.5, TEXT, "Choose a file..."));
                    });
                    row.spawn(NodeBundle { style: Style { flex_grow: 1.0, ..default() }, ..default() });
                    row.spawn((button(80.0), ArtBtn::Close)).with_children(|t| {
                        t.spawn(txt(&f.bold, 13.5, TEXT, "Close"));
                    });
                });
            });
        });
}

/// Mouse, wheel and keys while the art chooser is open; also collects the background results.
#[allow(clippy::type_complexity)]
fn art_pick_input(
    mut menu: ResMut<Menu>,
    mut ctx: Ctx,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: EventReader<MouseWheel>,
    tabs: Query<(&ArtTab, &Interaction)>,
    thumbs: Query<(&ArtThumb, &Interaction)>,
    buttons: Query<(&ArtBtn, &Interaction)>,
) {
    let Some(pick) = menu.art.as_mut() else {
        wheel.clear();
        return;
    };
    while let Ok(m) = pick.rx.try_recv() {
        match m {
            ArtMsg::List(list) => {
                pick.cands = list.into_iter().map(|cand| ArtCand { cand, file: None, thumb: None, failed: false }).collect();
                pick.listed = true;
            }
            ArtMsg::Thumb(i, f, w, h, px) => {
                if let Some(c) = pick.cands.get_mut(i) {
                    c.file = Some(f);
                }
                pick.pending.push((i, w, h, px));
            }
            ArtMsg::Failed(i) => {
                if let Some(c) = pick.cands.get_mut(i) {
                    c.failed = true;
                }
            }
        }
    }
    // Scroll by rows.
    let rows = pick.order().len().div_ceil(ART_COLS);
    for w in wheel.read() {
        let y = if w.unit == MouseScrollUnit::Pixel { w.y / 40.0 } else { w.y };
        if y < 0.0 && pick.scroll + ART_ROWS < rows {
            pick.scroll += 1;
        } else if y > 0.0 && pick.scroll > 0 {
            pick.scroll -= 1;
        }
    }
    // A file picked in the dialog.
    if let Some(rx) = &pick.file_rx {
        if let Ok(res) = rx.try_recv() {
            pick.file_rx = None;
            if let Some(f) = res {
                let (id, kind) = (pick.game_id.clone(), pick.tab);
                set_game_art(&mut ctx, &id, kind, Some(f));
                // Stays open for the other surfaces; nothing in the list is the one in use now.
                pick.in_use.insert(kind, None);
                return;
            }
        }
    }
    // Q / Esc close the art chooser first (the settings stay open).
    if keys.just_pressed(KeyCode::Escape) || keys_pressed(&ctx.cfg.keys.quit, &keys) {
        menu.art = None;
        art_preview(&mut ctx, None);
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    if let Some((t, _)) = tabs.iter().find(|(_, i)| **i == Interaction::Pressed) {
        pick.tab = t.0;
        pick.scroll = 0;
        let tab = t.0;
        art_preview(&mut ctx, Some(tab));
        return;
    }
    if let Some((t, _)) = thumbs.iter().find(|(_, i)| **i == Interaction::Pressed) {
        let order = pick.order();
        let Some(&ci) = order.get(t.0) else { return };
        let Some(file) = pick.cands[ci].file.clone() else { return };
        let (id, kind) = (pick.game_id.clone(), pick.tab);
        set_game_art(&mut ctx, &id, kind, Some(file));
        // Stays open (for the other surfaces); the pick is outlined.
        pick.in_use.insert(kind, Some(ci));
        return;
    }
    match buttons.iter().find(|(_, i)| **i == Interaction::Pressed).map(|(b, _)| *b) {
        Some(ArtBtn::Close) => {
            menu.art = None;
            art_preview(&mut ctx, None);
        }
        Some(ArtBtn::Auto) => {
            let (id, kind) = (pick.game_id.clone(), pick.tab);
            set_game_art(&mut ctx, &id, kind, None);
            pick.in_use.insert(kind, None);
        }
        Some(ArtBtn::File) => {
            if pick.file_rx.is_none() {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(rfd::FileDialog::new().set_title("Choose an image").add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp"]).pick_file());
                });
                pick.file_rx = Some(rx);
            }
        }
        None => {}
    }
}

#[allow(clippy::type_complexity)]
fn draw_art_picker(
    mut menu: ResMut<Menu>,
    mut images: ResMut<Assets<Image>>,
    mut layer: Query<&mut Style, (With<ArtLayer>, Without<ArtThumb>, Without<ArtThumbImg>, Without<ArtGrid>)>,
    mut grid: Query<&mut Style, (With<ArtGrid>, Without<ArtLayer>, Without<ArtThumb>, Without<ArtThumbImg>)>,
    mut cells: Query<(&ArtThumb, &mut Style, &Interaction, &Children), (Without<ArtLayer>, Without<ArtThumbImg>, Without<ArtGrid>)>,
    mut frames: Query<&mut BorderColor>,
    mut imgs: Query<(&ArtThumbImg, &mut UiImage, &mut Style), (Without<ArtLayer>, Without<ArtThumb>, Without<ArtGrid>)>,
    mut texts: Query<(&mut Text, Option<&ArtTitle>, Option<&ArtStatus>, Option<&ArtThumbLabel>)>,
    mut tabs: Query<(&ArtTab, &Interaction, &mut BackgroundColor), Without<ArtBtn>>,
    mut buttons: Query<(&Interaction, &mut BackgroundColor), (With<ArtBtn>, Without<ArtTab>)>,
) {
    let open = menu.art.is_some();
    for mut s in &mut layer {
        let d = if open { Display::Flex } else { Display::None };
        if s.display != d {
            s.display = d;
        }
    }
    let Some(pick) = menu.art.as_mut() else { return };
    for (i, w, h, px) in pick.pending.drain(..) {
        if let Some(c) = pick.cands.get_mut(i) {
            c.thumb = Some((images.add(crate::render::make_image(w, h, px, false)), w, h));
        }
    }
    let order = pick.order();
    for mut s in &mut grid {
        s.top = Val::Px(-(pick.scroll as f32) * ART_CELL_H);
    }
    let in_use = pick.in_use.get(&pick.tab).copied().flatten();
    for (cell, mut style, inter, children) in &mut cells {
        let shown = cell.0 < order.len();
        let picked = in_use.is_some() && order.get(cell.0).copied() == in_use;
        let d = if shown { Display::Flex } else { Display::None };
        if style.display != d {
            style.display = d;
        }
        for c in children.iter() {
            if let Ok(mut b) = frames.get_mut(*c) {
                b.0 = if picked {
                    Color::srgb(0.4, 0.9, 0.55)
                } else if *inter == Interaction::None {
                    Color::NONE
                } else {
                    ACCENT
                };
            }
        }
    }
    for (ti, mut img, mut style) in &mut imgs {
        let thumb = order.get(ti.0).and_then(|&ci| pick.cands[ci].thumb.clone());
        match thumb {
            Some((h, w, hh)) => {
                if img.texture != h {
                    img.texture = h;
                }
                let k = (ART_THUMB - 8.0) / (w.max(hh).max(1) as f32);
                style.width = Val::Px(w as f32 * k);
                style.height = Val::Px(hh as f32 * k);
            }
            None => {
                style.width = Val::Px(0.0);
                style.height = Val::Px(0.0);
            }
        }
    }
    let loading = pick.cands.iter().filter(|c| c.thumb.is_none() && !c.failed).count();
    let status = if !pick.listed {
        "Looking for art...".to_string()
    } else if order.is_empty() {
        "No art found; choose a file instead".to_string()
    } else if loading > 0 {
        format!("{} images, loading {loading}...", order.len())
    } else {
        format!("{} images (wheel scrolls)", order.len())
    };
    let what = match pick.tab {
        ArtKind::Cover => "cover",
        ArtKind::Media => "disc / cartridge label",
        ArtKind::Insert => "insert",
        ArtKind::Back => "back cover",
        ArtKind::Spine => "spine",
    };
    let title = format!("Art for {}: pick the {what}", pick.title);
    for (mut t, ti, st, lb) in &mut texts {
        let want = if ti.is_some() {
            title.clone()
        } else if st.is_some() {
            status.clone()
        } else if let Some(l) = lb {
            order.get(l.0).map(|&ci| pick.cands[ci].cand.label.clone()).unwrap_or_default()
        } else {
            continue;
        };
        if t.sections[0].value != want {
            t.sections[0].value = want;
        }
    }
    for (tab, inter, mut bg) in &mut tabs {
        let on = tab.0 == pick.tab;
        bg.0 = if on {
            Color::srgba(0.35, 0.62, 1.0, 0.45)
        } else if *inter == Interaction::None {
            Color::srgba(1.0, 1.0, 1.0, 0.08)
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.16)
        };
    }
    for (inter, mut bg) in &mut buttons {
        bg.0 = match inter {
            Interaction::None => Color::srgba(1.0, 1.0, 1.0, 0.08),
            Interaction::Hovered => Color::srgba(1.0, 1.0, 1.0, 0.16),
            Interaction::Pressed => Color::srgba(0.35, 0.62, 1.0, 0.4),
        };
    }
}



