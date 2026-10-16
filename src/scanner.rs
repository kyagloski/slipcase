//! Finds installed games on Windows and Linux.
//!
//! Sources: Steam (all library folders), Epic Games Launcher (Windows), Lutris,
//! Heroic (Epic via legendary), ROM folders and plain "one folder per game" directories.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::library::{clean_title, game_from_path, stable_id, Game, Launch};
use crate::platform::Platform;

/// Extensions that identify a disc image but not the console; they need `platform` set on the folder.
const DISC_EXTS: &[&str] = &["iso", "chd", "cue", "cso", "pbp", "rvz", "gcz", "wbfs", "xiso", "gdi", "cdi", "m3u", "elf"];

pub fn platform_for_extension(ext: &str, cfg: &Config) -> Option<Platform> {
    let ext = ext.to_ascii_lowercase();
    for p in Platform::ALL {
        let custom = cfg.platform_override(*p).and_then(|o| o.extensions.as_ref());
        let hit = match custom {
            Some(list) => list.iter().any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext)),
            None => p.extensions().contains(&ext.as_str()),
        };
        if hit {
            return Some(*p);
        }
    }
    None
}

pub fn scan_all(cfg: &Config) -> Vec<Game> {
    let mut out = Vec::new();
    if cfg.scan.steam {
        out.extend(scan_steam(cfg));
    }
    if cfg.scan.epic {
        out.extend(scan_epic());
    }
    if cfg.scan.lutris {
        out.extend(scan_lutris());
    }
    if cfg.scan.heroic {
        out.extend(scan_heroic());
    }
    for d in &cfg.scan.rom_dirs {
        out.extend(scan_rom_dir(d, cfg));
    }
    for d in &cfg.scan.game_dirs {
        out.extend(scan_game_dir(Path::new(d)));
    }
    // Owned but not installed (shown only with Display > Show games that are not installed).
    if cfg.scan.owned {
        let installed: HashSet<String> = out.iter().map(|g| g.id.clone()).collect();
        if cfg.scan.steam {
            out.extend(scan_steam_owned(cfg, &installed));
        }
        if cfg.scan.heroic {
            out.extend(scan_heroic_owned(&installed));
        }
    }
    let ignore: Vec<String> = cfg.scan.ignore.iter().map(|s| s.to_lowercase()).collect();
    out.retain(|g| {
        let t = g.title.to_lowercase();
        !ignore.iter().any(|i| t.contains(i))
    });
    let mut seen = HashSet::new();
    out.retain(|g| seen.insert(g.id.clone()));
    out
}

// ------------------------------------------------------------------------------------------------
// Steam
// ------------------------------------------------------------------------------------------------

/// Extracts every `"key"  "value"` pair that sits on one line of a Valve KeyValues file.
fn vdf_pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = Vec::new();
        let mut cur = String::new();
        let mut inq = false;
        let mut esc = false;
        for c in line.chars() {
            if inq {
                if esc {
                    cur.push(c);
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    parts.push(std::mem::take(&mut cur));
                    inq = false;
                } else {
                    cur.push(c);
                }
            } else if c == '"' {
                inq = true;
            }
        }
        if parts.len() == 2 {
            out.push((parts[0].clone(), parts[1].clone()));
        }
    }
    out
}

pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        for (hive, sub) in [
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam"),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam"),
            (HKEY_CURRENT_USER, r"Software\Valve\Steam"),
        ] {
            if let Ok(key) = RegKey::predef(hive).open_subkey(sub) {
                for name in ["InstallPath", "SteamPath"] {
                    if let Ok(p) = key.get_value::<String, _>(name) {
                        roots.push(PathBuf::from(p));
                    }
                }
            }
        }
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        roots.push(PathBuf::from(r"C:\Program Files\Steam"));
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".steam/steam"));
            roots.push(home.join(".local/share/Steam"));
            roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
            roots.push(home.join("snap/steam/common/.local/share/Steam"));
        }
    }
    roots.retain(|p| p.join("steamapps").is_dir());
    // De-duplicate symlinked roots (e.g. ~/.steam/steam -> ~/.local/share/Steam).
    let mut seen = HashSet::new();
    roots.retain(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())));
    roots
}

pub fn scan_steam(cfg: &Config) -> Vec<Game> {
    let mut libs: Vec<PathBuf> = Vec::new();
    for root in steam_roots() {
        libs.push(root.clone());
        if let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            for (k, v) in vdf_pairs(&text) {
                if k == "path" {
                    libs.push(PathBuf::from(v));
                }
            }
        }
    }
    for extra in &cfg.scan.steam_libraries {
        libs.push(PathBuf::from(extra));
    }
    let mut seen = HashSet::new();
    libs.retain(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())));

    let mut games = Vec::new();
    for lib in libs {
        let apps = lib.join("steamapps");
        let Ok(rd) = std::fs::read_dir(&apps) else { continue };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            let pairs = vdf_pairs(&text);
            let get = |key: &str| pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone());
            let (Some(appid), Some(title)) = (get("appid"), get("name")) else { continue };
            let Ok(appid_n) = appid.parse::<u32>() else { continue };
            let mut g = Game::new(
                format!("steam:{appid}"),
                title,
                Platform::Pc,
                Launch::Uri { uri: format!("steam://rungameid/{appid}") },
                "steam",
            );
            g.steam_appid = Some(appid_n);
            games.push(g);
        }
    }
    games
}

// ------------------------------------------------------------------------------------------------
// Epic / Heroic / Lutris
// ------------------------------------------------------------------------------------------------

pub fn scan_epic() -> Vec<Game> {
    #[allow(unused_mut)]
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        let pd = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
        let dir = PathBuf::from(pd).join(r"Epic\EpicGamesLauncher\Data\Manifests");
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.path().extension().map(|x| x != "item").unwrap_or(true) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                let cats = v.get("AppCategories").map(|c| c.to_string()).unwrap_or_default();
                if cats.contains("addons") || (!cats.is_empty() && !cats.contains("games")) {
                    continue;
                }
                let (name, app, ns, item) = (s("DisplayName"), s("AppName"), s("CatalogNamespace"), s("CatalogItemId"));
                if name.is_empty() || app.is_empty() {
                    continue;
                }
                let uri = format!("com.epicgames.launcher://apps/{ns}%3A{item}%3A{app}?action=launch&silent=true");
                out.push(Game::new(format!("epic:{app}"), name, Platform::Pc, Launch::Uri { uri }, "epic"));
            }
        }
    }
    out
}

pub fn scan_heroic() -> Vec<Game> {
    let mut out = Vec::new();
    let Some(cfg_dir) = dirs::config_dir() else { return out };
    let candidates = [
        cfg_dir.join("heroic/legendaryConfig/legendary/installed.json"),
        dirs::home_dir().unwrap_or_default().join(".var/app/com.heroicgameslauncher.hgl/config/heroic/legendaryConfig/legendary/installed.json"),
    ];
    for p in candidates {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let Some(map) = v.as_object() else { continue };
        for (app, info) in map {
            let title = info.get("title").and_then(|t| t.as_str()).unwrap_or(app).to_string();
            out.push(Game::new(
                format!("heroic:{app}"),
                title,
                Platform::Pc,
                Launch::Uri { uri: format!("heroic://launch/legendary/{app}") },
                "heroic",
            ));
        }
    }
    out
}

pub fn scan_lutris() -> Vec<Game> {
    let mut out = Vec::new();
    let Some(cfg_dir) = dirs::config_dir() else { return out };
    let dir = cfg_dir.join("lutris/games");
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().map(|x| x != "yml").unwrap_or(true) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let top = |key: &str| -> Option<String> {
            text.lines()
                .find_map(|l| l.strip_prefix(&format!("{key}:")))
                .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        };
        let (Some(name), Some(slug)) = (top("name"), top("slug")) else { continue };
        let plat = match top("runner").unwrap_or_default().as_str() {
            "dolphin" => Platform::Gamecube,
            "pcsx2" => Platform::Ps2,
            "rpcs3" => Platform::Ps3,
            "ppsspp" => Platform::Psp,
            "mame" | "linux" | "wine" | "steam" | "" => Platform::Pc,
            _ => Platform::Pc,
        };
        out.push(Game::new(
            format!("lutris:{slug}"),
            name,
            plat,
            Launch::Uri { uri: format!("lutris:rungame/{slug}") },
            "lutris",
        ));
    }
    out
}

// ------------------------------------------------------------------------------------------------
// Folders
// ------------------------------------------------------------------------------------------------

fn walk(dir: &Path, recursive: bool, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if recursive && depth < 8 {
                walk(&p, recursive, depth + 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

pub fn scan_rom_dir(d: &crate::config::RomDir, cfg: &Config) -> Vec<Game> {
    let forced = Platform::from_id(&d.platform);
    let mut files = Vec::new();
    walk(Path::new(&d.path), d.recursive, 0, &mut files);
    let extra: Vec<String> = d.extensions.iter().map(|e| e.trim_start_matches('.').to_ascii_lowercase()).collect();
    let mut out = Vec::new();
    for f in files {
        let Some(ext) = f.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()) else { continue };
        let mut by_ext = platform_for_extension(&ext, cfg);
        // Disc images, raw .bin dumps and zips do not say which console they are for: look
        // inside (and at the folder names) when the folder has no platform set.
        if by_ext.is_none() && forced.is_none() && (DISC_EXTS.contains(&ext.as_str()) || matches!(ext.as_str(), "bin" | "img" | "zip")) {
            by_ext = crate::detect::detect_platform(&f, cfg);
        }
        let accept = match forced {
            Some(p) => {
                p.extensions().contains(&ext.as_str())
                    || DISC_EXTS.contains(&ext.as_str())
                    || extra.contains(&ext)
                    || by_ext == Some(p)
            }
            None => by_ext.is_some() || extra.contains(&ext),
        };
        if !accept {
            continue;
        }
        let platform = forced.or(by_ext);
        if let Some(mut g) = game_from_path(&f, None, platform, cfg) {
            g.source = "rom".into();
            out.push(g);
        }
    }
    out
}

fn is_executable_candidate(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ["unins", "setup", "crash", "redist", "vcredist", "dxsetup", "installer", "updater", "launcher_helper"].iter().any(|b| name.contains(b)) {
        return false;
    }
    #[cfg(windows)]
    {
        name.ends_with(".exe")
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let ext_ok = name.ends_with(".sh") || name.ends_with(".x86_64") || name.ends_with(".appimage") || name.ends_with(".exe");
        let exec = std::fs::metadata(p).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false);
        (exec && !name.contains('.')) || (exec && ext_ok) || name.ends_with(".appimage")
    }
}

/// Each sub-folder is a game; the biggest plausible executable inside is its launcher.
pub fn scan_game_dir(dir: &Path) -> Vec<Game> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let folder = e.path();
        if !folder.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        walk(&folder, true, 6, &mut files);
        files.retain(|f| is_executable_candidate(f));
        files.sort_by_key(|f| std::cmp::Reverse(std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)));
        let Some(exe) = files.first() else { continue };
        let title = clean_title(&folder.file_name().unwrap_or_default().to_string_lossy());
        let exe_s = exe.to_string_lossy().to_string();
        let cwd = exe.parent().map(|p| p.to_string_lossy().to_string());
        let mut g = Game::new(
            stable_id("path", &exe_s),
            title,
            Platform::Pc,
            Launch::Command { program: exe_s.clone(), args: vec![], cwd },
            "exe",
        );
        g.path = Some(exe_s);
        out.push(g);
    }
    out
}

// ------------------------------------------------------------------------------------------------
// Owned but not installed
// ------------------------------------------------------------------------------------------------

const STEAM_ID_BASE: u64 = 76561197960265728;

/// Every Steam account that has signed in here (64-bit ids), most recent first.
fn steam_accounts() -> Vec<u64> {
    let mut out: Vec<(u64, bool)> = Vec::new();
    for root in steam_roots() {
        let Ok(text) = std::fs::read_to_string(root.join("config/loginusers.vdf")) else { continue };
        let mut current: Option<u64> = None;
        for line in text.lines() {
            let t = line.trim().trim_matches('"');
            if t.len() == 17 && t.chars().all(|c| c.is_ascii_digit()) {
                current = t.parse().ok();
                if let Some(id) = current {
                    out.push((id, false));
                }
            } else if let (Some(id), true) = (current, line.contains("\"MostRecent\"") && line.contains("\"1\"")) {
                if let Some(e) = out.iter_mut().find(|e| e.0 == id) {
                    e.1 = true;
                }
            }
        }
    }
    out.sort_by_key(|e| !e.1);
    let mut seen = HashSet::new();
    out.into_iter().map(|e| e.0).filter(|id| seen.insert(*id)).collect()
}

/// Steam games you own (or have played) that are not installed: everything on the account
/// with a Steam Web API key, otherwise the games played on this computer.
pub fn scan_steam_owned(cfg: &Config, installed: &HashSet<String>) -> Vec<Game> {
    let key = cfg.scan.steam_api_key.trim();
    let mut found: Vec<(u32, String)> = Vec::new();
    let ids: Vec<u64> = match cfg.scan.steam_id.trim().parse::<u64>() {
        Ok(id) => vec![id],
        Err(_) => steam_accounts(),
    };
    if !key.is_empty() {
        for id in ids.iter().take(1) {
            match steam_owned_api(key, *id) {
                Ok(list) => found.extend(list),
                Err(e) => eprintln!("[slipcase] Steam owned games: {e}"),
            }
        }
    }
    if found.is_empty() {
        found = steam_played_local(&ids);
    }
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|(appid, name)| !name.is_empty() && seen.insert(*appid) && !installed.contains(&format!("steam:{appid}")))
        .map(|(appid, name)| {
            // Launching it opens Steam's install dialog.
            let mut g = Game::new(format!("steam:{appid}"), name, Platform::Pc, Launch::Uri { uri: format!("steam://install/{appid}") }, "steam");
            g.steam_appid = Some(appid);
            g.installed = false;
            g
        })
        .collect()
}

fn steam_owned_api(key: &str, id: u64) -> Result<Vec<(u32, String)>, String> {
    let url = format!(
        "https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?key={key}&steamid={id}&include_appinfo=1&include_played_free_games=1&format=json"
    );
    let v: serde_json::Value = ureq::get(&url).timeout(std::time::Duration::from_secs(20)).call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
    let games = v.pointer("/response/games").and_then(|g| g.as_array()).ok_or("no games in the reply (is the profile's game list private?)")?;
    Ok(games
        .iter()
        .filter_map(|g| Some((g.get("appid")?.as_u64()? as u32, g.get("name")?.as_str()?.to_string())))
        .collect())
}

/// Games each account has played here (from Steam's local config), named from Steam's app
/// cache. Only real games: tools, DLC and soundtracks are skipped.
fn steam_played_local(ids: &[u64]) -> Vec<(u32, String)> {
    let mut appids: Vec<u32> = Vec::new();
    for root in steam_roots() {
        for id in ids {
            let id3 = id.saturating_sub(STEAM_ID_BASE);
            let p = root.join(format!("userdata/{id3}/config/localconfig.vdf"));
            let Ok(text) = std::fs::read_to_string(p) else { continue };
            appids.extend(localconfig_apps(&text));
        }
    }
    if appids.is_empty() {
        return vec![];
    }
    let info = steam_roots().into_iter().find_map(|r| std::fs::read(r.join("appcache/appinfo.vdf")).ok()).map(|b| appinfo_names(&b)).unwrap_or_default();
    appids
        .into_iter()
        .filter_map(|a| {
            let (name, kind) = info.get(&a)?;
            kind.eq_ignore_ascii_case("game").then(|| (a, name.clone()))
        })
        .collect()
}

/// App ids under `UserLocalConfigStore/Software/Valve/Steam/apps` (text KeyValues).
fn localconfig_apps(text: &str) -> Vec<u32> {
    let mut path: Vec<String> = Vec::new();
    let mut last: Option<String> = None;
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t == "{" {
            path.push(last.take().unwrap_or_default());
        } else if t == "}" {
            path.pop();
        } else if t.starts_with('"') {
            let parts: Vec<&str> = t.split('"').filter(|s| !s.trim().is_empty()).collect();
            if parts.len() == 1 {
                last = Some(parts[0].to_string());
                // A section header: an app id right under .../Steam/apps.
                let n = path.len();
                if n >= 2 && path[n - 1].eq_ignore_ascii_case("apps") && path[n - 2].eq_ignore_ascii_case("steam") {
                    if let Ok(a) = parts[0].parse::<u32>() {
                        out.push(a);
                    }
                }
            }
        }
    }
    out
}

/// Names and types of the apps in Steam's binary `appinfo.vdf` (versions 27 to 29).
pub fn appinfo_names(b: &[u8]) -> std::collections::HashMap<u32, (String, String)> {
    let mut out = std::collections::HashMap::new();
    let u32_at = |i: usize| b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let Some(magic) = u32_at(0) else { return out };
    let version = match magic {
        0x07564427 => 27,
        0x07564428 => 28,
        0x07564429 => 29,
        _ => return out,
    };
    let mut pos = 8;
    // Version 29 stores key names once, in a table at the end.
    let mut strings: Vec<String> = Vec::new();
    if version >= 29 {
        let Some(off) = b.get(8..16).map(|s| u64::from_le_bytes(s.try_into().unwrap()) as usize) else { return out };
        pos = 16;
        if let Some(count) = u32_at(off) {
            let mut p = off + 4;
            for _ in 0..count {
                let Some(end) = b[p.min(b.len())..].iter().position(|&c| c == 0) else { break };
                strings.push(String::from_utf8_lossy(&b[p..p + end]).into_owned());
                p += end + 1;
            }
        }
    }
    // appid, size, state, updated, token(8), sha1(20), change(4), [binary sha1(20)]
    let head = 4 + 4 + 8 + 20 + 4 + if version >= 28 { 20 } else { 0 };
    while let (Some(appid), Some(size)) = (u32_at(pos), u32_at(pos + 4)) {
        if appid == 0 {
            break;
        }
        let start = pos + 8;
        let end = (start + size as usize).min(b.len());
        if let Some(kv) = b.get(start + head..end) {
            let mut name = String::new();
            let mut kind = String::new();
            let mut path: Vec<String> = Vec::new();
            kv_walk(kv, &strings, version >= 29, &mut path, &mut |p, k, v| {
                if p.len() == 2 && p[1] == "common" {
                    if k == "name" {
                        name = v.to_string();
                    } else if k == "type" {
                        kind = v.to_string();
                    }
                }
            });
            if !name.is_empty() {
                out.insert(appid, (name, kind));
            }
        }
        pos = end;
    }
    out
}

/// Walks binary KeyValues, calling `f(path, key, value)` for every string value.
fn kv_walk(b: &[u8], strings: &[String], indexed: bool, path: &mut Vec<String>, f: &mut dyn FnMut(&[String], &str, &str)) {
    let mut i = 0;
    let cstr = |i: &mut usize| -> String {
        let s = &b[(*i).min(b.len())..];
        let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        *i += n + 1;
        String::from_utf8_lossy(&s[..n]).into_owned()
    };
    let mut depth = 0usize;
    while i < b.len() {
        let t = b[i];
        i += 1;
        if t == 0x08 {
            if depth == 0 {
                return;
            }
            depth -= 1;
            path.pop();
            continue;
        }
        let key = if indexed {
            let Some(s) = b.get(i..i + 4) else { return };
            i += 4;
            strings.get(u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize).cloned().unwrap_or_default()
        } else {
            cstr(&mut i)
        };
        match t {
            0x00 => {
                path.push(key);
                depth += 1;
            }
            0x01 => {
                let v = cstr(&mut i);
                f(path, &key, &v);
            }
            0x02 | 0x03 | 0x04 | 0x06 => i += 4,
            0x07 | 0x0A => i += 8,
            0x05 => {
                // UTF-16 string, ends with two zero bytes.
                while i + 1 < b.len() && !(b[i] == 0 && b[i + 1] == 0) {
                    i += 2;
                }
                i += 2;
            }
            _ => return,
        }
    }
}

/// Games in Heroic's Epic / GOG / Amazon libraries that are not installed.
pub fn scan_heroic_owned(installed: &HashSet<String>) -> Vec<Game> {
    let mut out = Vec::new();
    let mut bases = Vec::new();
    if let Some(c) = dirs::config_dir() {
        bases.push(c.join("heroic"));
    }
    if let Some(h) = dirs::home_dir() {
        bases.push(h.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"));
    }
    for base in bases {
        for (file, runner) in [("store_cache/legendary_library.json", "legendary"), ("store_cache/gog_library.json", "gog"), ("store_cache/nile_library.json", "nile")] {
            let Ok(text) = std::fs::read_to_string(base.join(file)) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let list = v.get("library").or_else(|| v.get("games")).and_then(|l| l.as_array()).cloned().unwrap_or_default();
            for g in list {
                let s = |k: &str| g.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                let (app, title) = (s("app_name"), s("title"));
                if app.is_empty() || title.is_empty() || g.get("is_installed").and_then(|x| x.as_bool()) == Some(true) {
                    continue;
                }
                let id = format!("heroic:{app}");
                if installed.contains(&id) || g.get("is_dlc").and_then(|x| x.as_bool()) == Some(true) {
                    continue;
                }
                let mut game = Game::new(id, title, Platform::Pc, Launch::Uri { uri: format!("heroic://launch/{runner}/{app}") }, "heroic");
                game.installed = false;
                out.push(game);
            }
        }
    }
    out
}

#[cfg(test)]
mod owned_tests {
    use super::*;

    /// Reads this computer's Steam files (read only); run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn local_steam_owned() {
        let ids = steam_accounts();
        println!("accounts {ids:?}");
        let played = steam_played_local(&ids);
        println!("played games {}: {:?}", played.len(), played.iter().take(8).collect::<Vec<_>>());
        let cfg = Config::default();
        let installed: HashSet<String> = scan_steam(&cfg).into_iter().map(|g| g.id).collect();
        let owned = scan_steam_owned(&cfg, &installed);
        println!("installed {} / not installed {}: {:?}", installed.len(), owned.len(), owned.iter().take(8).map(|g| &g.title).collect::<Vec<_>>());
    }
}
