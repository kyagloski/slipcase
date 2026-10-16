//! The game library: data model, JSON persistence, merging scan results, launching.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::config::{config_dir, Config};
use crate::platform::Platform;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Launch {
    /// Opened with the OS URL handler, e.g. `steam://rungameid/220`.
    Uri { uri: String },
    /// A program with arguments.
    Command { program: String, args: Vec<String>, cwd: Option<String> },
    /// A ROM / disc image opened with the platform's configured emulator command.
    Rom { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub title: String,
    pub platform: Platform,
    pub launch: Launch,
    /// Where this came from: "steam", "epic", "lutris", "heroic", "rom", "exe", "manual".
    pub source: String,
    pub steam_appid: Option<u32>,
    /// ROM / executable path, used to derive libretro cover names.
    pub path: Option<String>,
    /// User-chosen cover image (overrides every online source).
    pub cover_path: Option<String>,
    #[serde(default)]
    pub manual: bool,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub last_played: u64,
    #[serde(default)]
    pub play_count: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub year: String,
    /// Dominant colour of the cover (sRGB), drives the adaptive background.
    #[serde(default)]
    pub dominant: Option<[f32; 3]>,
    /// Title / platform were set by the user; rescans keep them.
    #[serde(default)]
    pub renamed: bool,
    #[serde(default)]
    pub replatformed: bool,
    /// Set by the user: the file, program or URI to launch instead of the detected one.
    #[serde(default)]
    pub custom_location: Option<String>,
    /// Set by the user: extra arguments added to the launch command.
    #[serde(default)]
    pub launch_args: String,
    /// Hidden from the launcher (listed under Settings > Hidden games).
    #[serde(default)]
    pub hidden: bool,
    /// Cartridge plastic colour (sRGB + alpha), measured from a photo of the cartridge.
    #[serde(default)]
    pub cart_color: Option<[f32; 4]>,
    /// Art picked by the user for the disc / cartridge label and the insert (local files).
    #[serde(default)]
    pub media_art: Option<String>,
    #[serde(default)]
    pub insert_art: Option<String>,
    /// Picked by the user for the back cover and the spine (local files).
    #[serde(default)]
    pub back_art: Option<String>,
    #[serde(default)]
    pub spine_art: Option<String>,
    /// In a store library but not installed (launching it offers to install it).
    #[serde(default = "yes")]
    pub installed: bool,
}

fn yes() -> bool {
    true
}

impl Game {
    pub fn new(id: impl Into<String>, title: impl Into<String>, platform: Platform, launch: Launch, source: &str) -> Self {
        Game {
            id: id.into(),
            title: title.into(),
            platform,
            launch,
            source: source.into(),
            steam_appid: None,
            path: None,
            cover_path: None,
            manual: false,
            favorite: false,
            last_played: 0,
            play_count: 0,
            description: String::new(),
            genres: vec![],
            year: String::new(),
            dominant: None,
            renamed: false,
            replatformed: false,
            custom_location: None,
            launch_args: String::new(),
            hidden: false,
            cart_color: None,
            media_art: None,
            insert_art: None,
            back_art: None,
            spine_art: None,
            installed: true,
        }
    }

    /// The file, program or URI this game launches.
    pub fn location(&self) -> String {
        if let Some(l) = &self.custom_location {
            return l.clone();
        }
        match &self.launch {
            Launch::Uri { uri } => uri.clone(),
            Launch::Command { program, .. } => program.clone(),
            Launch::Rom { path } => path.clone(),
        }
    }
}

#[derive(Resource, Default)]
pub struct Library {
    pub games: Vec<Game>,
    /// Bumped every time the list changes so the view knows to rebuild.
    pub revision: u64,
}

pub fn library_path() -> PathBuf {
    config_dir().join("library.json")
}

impl Library {
    pub fn load() -> Library {
        let games = std::fs::read_to_string(library_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Game>>(&s).ok())
            .unwrap_or_default();
        Library { games, revision: 1 }
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(&self.games) {
            let tmp = library_path().with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(tmp, library_path());
            }
        }
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Game> {
        self.games.iter_mut().find(|g| g.id == id)
    }

    pub fn find(&self, id: &str) -> Option<&Game> {
        self.games.iter().find(|g| g.id == id)
    }

    /// Adds or replaces a game, keeping the user's favorites/play stats.
    pub fn upsert(&mut self, mut g: Game) -> bool {
        if let Some(existing) = self.games.iter_mut().find(|e| e.id == g.id) {
            g.favorite = existing.favorite;
            g.last_played = existing.last_played;
            g.play_count = existing.play_count;
            g.manual |= existing.manual;
            if g.description.is_empty() {
                g.description = existing.description.clone();
                g.genres = existing.genres.clone();
                g.year = existing.year.clone();
            }
            if g.cover_path.is_none() {
                g.cover_path = existing.cover_path.clone();
            }
            g.dominant = existing.dominant;
            if existing.renamed {
                g.title = existing.title.clone();
                g.renamed = true;
            }
            if existing.replatformed {
                g.platform = existing.platform;
                g.replatformed = true;
            }
            // Scans never know these; they are the user's.
            g.custom_location = existing.custom_location.clone();
            g.launch_args = existing.launch_args.clone();
            g.hidden = existing.hidden;
            g.cart_color = existing.cart_color;
            g.media_art = existing.media_art.clone();
            g.insert_art = existing.insert_art.clone();
            g.back_art = existing.back_art.clone();
            g.spine_art = existing.spine_art.clone();
            *existing = g;
            false
        } else {
            self.games.push(g);
            true
        }
    }

    /// Merge scan results: new games are added, vanished *scanned* games removed.
    pub fn merge_scan(&mut self, scanned: Vec<Game>) -> (usize, usize) {
        let mut added = 0;
        let ids: std::collections::HashSet<String> = scanned.iter().map(|g| g.id.clone()).collect();
        for g in scanned {
            // Owned-but-not-installed games are not news.
            let installed = g.installed;
            if self.upsert(g) && installed {
                added += 1;
            }
        }
        let before = self.games.len();
        self.games.retain(|g| g.manual || ids.contains(&g.id));
        let removed = before - self.games.len();
        self.revision += 1;
        (added, removed)
    }

    /// Removes games added by hand (ROMs, programs, game folders) whose file is gone, and
    /// returns their titles. Store games (Steam, Epic, ...) are left to the store scan. A game
    /// only counts as gone when its folder is still there, or the folder above it is there and
    /// not empty (so the folder was deleted): a disconnected drive or share keeps its games.
    pub fn prune_missing(&mut self) -> Vec<String> {
        let gone = |g: &Game| -> bool {
            let path: Option<PathBuf> = match (&g.custom_location, &g.launch) {
                (Some(l), _) if l.contains("://") => None,
                (Some(l), _) => Some(PathBuf::from(l.trim().trim_matches('"'))),
                (None, Launch::Rom { path }) => Some(PathBuf::from(path)),
                (None, Launch::Command { program, .. }) => Some(PathBuf::from(program)),
                (None, Launch::Uri { .. }) => None,
            };
            // Bare commands ("lutris ...") are found through PATH, not checked here.
            let Some(p) = path.filter(|p| p.is_absolute()) else { return false };
            if p.exists() {
                return false;
            }
            let Some(parent) = p.parent() else { return false };
            if parent.exists() {
                return true;
            }
            parent.parent().is_some_and(|gp| gp.parent().is_some() && gp.read_dir().is_ok_and(|mut r| r.next().is_some()))
        };
        let removed: Vec<String> = self.games.iter().filter(|g| gone(g)).map(|g| g.title.clone()).collect();
        if !removed.is_empty() {
            self.games.retain(|g| !gone(g));
            self.revision += 1;
        }
        removed
    }

    /// Hides games whose title is in `titles` (exact, any case) and drops those titles from
    /// the list, since the games now carry the flag themselves. Returns whether anything changed.
    pub fn apply_hidden_titles(&mut self, titles: &mut Vec<String>) -> bool {
        let before = titles.len();
        for g in &mut self.games {
            if let Some(p) = titles.iter().position(|t| t.eq_ignore_ascii_case(&g.title)) {
                g.hidden = true;
                titles.remove(p);
            }
        }
        if titles.len() != before {
            self.revision += 1;
            return true;
        }
        false
    }

    pub fn add_manual(&mut self, path: &Path, title: Option<String>, platform: Option<Platform>, cfg: &Config) -> Option<String> {
        let g = game_from_path(path, title, platform, cfg)?;
        let id = g.id.clone();
        let mut g = g;
        g.manual = true;
        self.upsert(g);
        self.revision += 1;
        self.save();
        Some(id)
    }
}

pub fn stable_id(prefix: &str, text: &str) -> String {
    // FNV-1a: stable across runs/platforms, unlike std's randomised hasher.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}:{h:016x}")
}

pub fn clean_title(stem: &str) -> String {
    // "Super Mario Land (USA, Europe) [!]" stays intact for cover lookup elsewhere;
    // the displayed title drops the bracketed tags.
    let mut out = String::new();
    let mut depth = 0;
    for c in stem.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let t = out.replace('_', " ");
    let t = t.trim();
    if t.is_empty() { stem.to_string() } else { t.to_string() }
}

/// Builds a `Game` from a dropped / passed path: ROM, executable or script.
pub fn game_from_path(path: &Path, title: Option<String>, platform: Option<Platform>, cfg: &Config) -> Option<Game> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    // The extension, else what is inside the file, else the folder names in its path.
    let plat = platform.or_else(|| crate::detect::detect_platform(&path, cfg));
    let title = title.unwrap_or_else(|| clean_title(&stem));
    let path_s = path.to_string_lossy().to_string();
    let id = stable_id("path", &path_s);
    let mut g = if let Some(p) = plat.filter(|p| *p != Platform::Pc && *p != Platform::Other) {
        Game::new(id, title, p, Launch::Rom { path: path_s.clone() }, "rom")
    } else if matches!(ext.as_str(), "exe" | "sh" | "appimage" | "bat" | "lnk" | "x86_64" | "") {
        let cwd = path.parent().map(|p| p.to_string_lossy().to_string());
        Game::new(
            id,
            title,
            Platform::Pc,
            Launch::Command { program: path_s.clone(), args: vec![], cwd },
            "exe",
        )
    } else {
        // Unknown file type with an explicit platform, e.g. an .iso for PS2.
        Game::new(id, title, platform.unwrap_or(Platform::Other), Launch::Rom { path: path_s.clone() }, "rom")
    };
    g.path = Some(path_s);
    Some(g)
}

// ------------------------------------------------------------------------------------------------
// Launching
// ------------------------------------------------------------------------------------------------

fn open_uri(uri: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd").args(["/C", "start", "", uri]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(uri).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(uri).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
    }
}

/// Splits a command template into program + args, honouring simple double quotes,
/// and substitutes `{rom}`.
pub fn expand_command(template: &str, rom: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut has = false;
    for c in template.chars() {
        match c {
            '"' => {
                in_q = !in_q;
                has = true;
            }
            c if c.is_whitespace() && !in_q => {
                if has || !cur.is_empty() {
                    parts.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        parts.push(cur);
    }
    parts.into_iter().map(|p| p.replace("{rom}", rom)).collect()
}

pub fn launch_game(game: &Game, cfg: &Config) -> Result<(), String> {
    let extra = expand_command(&game.launch_args, "");
    let launch = match &game.custom_location {
        Some(loc) => location_launch(game, loc),
        None => game.launch.clone(),
    };
    match &launch {
        // Steam takes extra arguments through its run URI.
        Launch::Uri { uri } if !extra.is_empty() => {
            match uri.strip_prefix("steam://rungameid/") {
                Some(id) => open_uri(&format!("steam://run/{id}//{}/", game.launch_args.trim())).map_err(|e| e.to_string()),
                None => open_uri(uri).map_err(|e| e.to_string()),
            }
        }
        Launch::Uri { uri } => open_uri(uri).map_err(|e| e.to_string()),
        Launch::Command { program, args, cwd } => {
            let mut c = Command::new(program);
            c.args(args).args(&extra).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            if let Some(d) = cwd {
                c.current_dir(d);
            }
            c.spawn().map(|_| ()).map_err(|e| format!("{program}: {e}"))
        }
        Launch::Rom { path } => {
            let template = cfg.command_for(game.platform);
            let parts = expand_command(&template, path);
            let (prog, rest) = parts.split_first().ok_or("empty launch command")?;
            // "{rom}" alone means "just open the file" (works for executables too).
            if rest.is_empty() && prog == path && extra.is_empty() {
                return open_uri(path).map_err(|e| e.to_string());
            }
            Command::new(prog)
                .args(rest)
                .args(&extra)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("{prog}: {e} (edit [platforms.{}] command in config.toml)", game.platform.id()))
        }
    }
}

/// How to launch a location the user typed in: a URI is opened, a console game's file goes
/// to its emulator, anything else is run as a program from its own folder.
fn location_launch(game: &Game, loc: &str) -> Launch {
    let loc = loc.trim().trim_matches('"').to_string();
    if loc.contains("://") {
        Launch::Uri { uri: loc }
    } else if matches!(game.launch, Launch::Rom { .. }) || !matches!(game.platform, Platform::Pc | Platform::Other) {
        Launch::Rom { path: loc }
    } else {
        let cwd = Path::new(&loc).parent().map(|p| p.to_string_lossy().to_string()).filter(|s| !s.is_empty());
        Launch::Command { program: loc, args: vec![], cwd }
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod prune_tests {
    use super::*;

    #[test]
    fn removes_deleted_files_but_keeps_missing_drives() {
        let root = std::env::temp_dir().join(format!("slipcase-prune-{}", std::process::id()));
        let roms = root.join("roms");
        std::fs::create_dir_all(&roms).unwrap();
        std::fs::write(roms.join("here.gba"), b"rom").unwrap();
        let rom = |id: &str, p: PathBuf| Game::new(id, id, Platform::Gba, Launch::Rom { path: p.to_string_lossy().into() }, "rom");
        let mut lib = Library {
            games: vec![
                rom("here", roms.join("here.gba")),
                // File deleted, folder still there.
                rom("deleted", roms.join("gone.gba")),
                // Whole folder deleted; the folder above still has things in it.
                rom("folder", root.join("old_folder").join("x.gba")),
                // A drive that is not connected: nothing of its path exists.
                rom("drive", PathBuf::from("/slipcase-no-such-drive/roms/y.gba")),
                Game::new("steam", "steam", Platform::Pc, Launch::Uri { uri: "steam://rungameid/1".into() }, "steam"),
            ],
            revision: 1,
        };
        let mut removed = lib.prune_missing();
        removed.sort();
        assert_eq!(removed, vec!["deleted".to_string(), "folder".to_string()]);
        let left: Vec<&str> = lib.games.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(left, vec!["here", "drive", "steam"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
