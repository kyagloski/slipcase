//! LaunchBox Games Database (free, no key): real box spines, back covers, disc / cart
//! labels, clear logos, fanart and screenshots for a very large number of games.
//!
//! The whole database is published as one zip (~110 MB, ~500 MB of XML). It is downloaded
//! at most once a month into the cache, streamed through once, and only the matches for
//! games in the library are kept (`matches.json`). New games trigger a re-match against
//! the cached zip in the background.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::cache_dir;
use crate::covers::normalize_title;
use crate::library::Game;
use crate::platform::Platform;

const ZIP_URL: &str = "https://gamesdb.launchbox-app.com/Metadata.zip";
pub const IMAGE_BASE: &str = "https://images.launchbox-app.com/";

/// Image file names (relative to `IMAGE_BASE`) found for one game.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Assets {
    pub front: Option<String>,
    pub back: Option<String>,
    pub spine: Option<String>,
    pub disc: Option<String>,
    pub cart: Option<String>,
    pub logo: Option<String>,
    pub fanart: Option<String>,
    pub screenshots: Vec<String>,
    /// What the database says about the game.
    #[serde(default)]
    pub info: GameInfo,
}

/// Text details of a game from the LaunchBox database.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GameInfo {
    pub overview: String,
    pub developer: String,
    pub publisher: String,
    /// Release date (YYYY-MM-DD) or year.
    pub released: String,
    pub genres: String,
    pub players: String,
    /// Community rating out of 5 (empty when unknown).
    pub rating: String,
}

/// Bumped when more is read from the database, so earlier matches are looked up again.
const MATCH_VERSION: u32 = 1;

#[derive(Default, Serialize, Deserialize)]
struct Matches {
    /// Library game ids that were looked up (found or not).
    checked: HashSet<String>,
    found: HashMap<String, Assets>,
    #[serde(default)]
    version: u32,
}

fn state() -> &'static RwLock<Matches> {
    static S: OnceLock<RwLock<Matches>> = OnceLock::new();
    S.get_or_init(|| {
        let mut m: Matches = std::fs::read(dir().join("matches.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if m.version < MATCH_VERSION {
            // Made before the game details were read: look everything up again.
            m.checked.clear();
            m.version = MATCH_VERSION;
        }
        RwLock::new(m)
    })
}

fn dir() -> PathBuf {
    let d = cache_dir().join("launchbox");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn url(file: &str) -> String {
    format!("{IMAGE_BASE}{file}")
}

/// Assets for a library game, if LaunchBox has it.
pub fn assets(game_id: &str) -> Option<Assets> {
    state().read().ok()?.found.get(game_id).cloned()
}

/// Whether some games have not been looked up yet.
pub fn needs_update(games: &[Game]) -> bool {
    let Ok(s) = state().read() else { return false };
    games.iter().any(|g| !s.checked.contains(&g.id))
}

pub fn lb_platform(p: Platform) -> Option<&'static str> {
    Some(match p {
        Platform::Pc => "Windows",
        Platform::Gb => "Nintendo Game Boy",
        Platform::Gbc => "Nintendo Game Boy Color",
        Platform::Gba => "Nintendo Game Boy Advance",
        Platform::Nds => "Nintendo DS",
        Platform::N3ds => "Nintendo 3DS",
        Platform::Nes => "Nintendo Entertainment System",
        Platform::Snes => "Super Nintendo Entertainment System",
        Platform::N64 => "Nintendo 64",
        Platform::Gamecube => "Nintendo GameCube",
        Platform::Wii => "Nintendo Wii",
        Platform::Switch => "Nintendo Switch",
        Platform::Genesis => "Sega Genesis",
        Platform::Saturn => "Sega Saturn",
        Platform::Dreamcast => "Sega Dreamcast",
        Platform::Ps1 => "Sony Playstation",
        Platform::Ps2 => "Sony Playstation 2",
        Platform::Ps3 => "Sony Playstation 3",
        Platform::Ps4 => "Sony Playstation 4",
        Platform::Ps5 => "Sony Playstation 5",
        Platform::Psp => "Sony PSP",
        Platform::Vita => "Sony Playstation Vita",
        Platform::Xbox => "Microsoft Xbox",
        Platform::Xbox360 => "Microsoft Xbox 360",
        Platform::XboxOne => "Microsoft Xbox One",
        Platform::Other => return None,
    })
}

fn region_rank(r: &str) -> i32 {
    match r {
        "" | "North America" | "United States" => 0,
        "World" => 1,
        "Canada" | "United Kingdom" | "Europe" | "Australia" => 2,
        _ => 3,
    }
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

/// `<Tag>value</Tag>` on one line.
fn tag(line: &str) -> Option<(&str, &str)> {
    let l = line.trim();
    let rest = l.strip_prefix('<')?;
    let end = rest.find('>')?;
    let name = &rest[..end];
    let value = rest[end + 1..].strip_suffix(&format!("</{name}>"))?;
    Some((name, value))
}

fn download(agent: &ureq::Agent) -> Option<PathBuf> {
    let zip = dir().join("Metadata.zip");
    let fresh = std::fs::metadata(&zip)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|a| a < Duration::from_secs(30 * 86400));
    if fresh {
        return Some(zip);
    }
    let tmp = dir().join("Metadata.zip.part");
    let ok = (|| -> Option<()> {
        let resp = agent.get(ZIP_URL).timeout(Duration::from_secs(600)).call().ok()?;
        let mut f = std::fs::File::create(&tmp).ok()?;
        std::io::copy(&mut resp.into_reader(), &mut f).ok()?;
        std::fs::rename(&tmp, &zip).ok()
    })();
    if ok.is_none() {
        let _ = std::fs::remove_file(&tmp);
    }
    zip.exists().then_some(zip)
}

/// Downloads (if needed) and matches every library game that has not been looked up yet.
/// Blocking: run on a background thread. Returns how many games were matched.
pub fn update(games: Vec<Game>) -> usize {
    let todo: Vec<Game> = {
        let Ok(s) = state().read() else { return 0 };
        games.into_iter().filter(|g| !s.checked.contains(&g.id)).collect()
    };
    if todo.is_empty() {
        return 0;
    }
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).user_agent("slipcase/0.1").build();
    let Some(zip) = download(&agent) else { return 0 };
    let Ok(file) = std::fs::File::open(&zip) else { return 0 };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        let _ = std::fs::remove_file(&zip);
        return 0;
    };
    let Ok(xml) = archive.by_name("Metadata.xml") else { return 0 };

    // What we are looking for.
    let by_app: HashMap<String, usize> = todo.iter().enumerate().filter_map(|(i, g)| g.steam_appid.map(|a| (a.to_string(), i))).collect();
    let wanted_platforms: HashSet<&str> = todo.iter().filter_map(|g| lb_platform(g.platform)).collect();
    let titles: Vec<Vec<String>> = todo.iter().map(|g| normalize_title(&g.title)).collect();

    // Pass over the XML: games come first, then all images.
    let mut best: Vec<Option<(f32, String)>> = vec![None; todo.len()];
    let mut info: Vec<GameInfo> = vec![GameInfo::default(); todo.len()];
    let mut overview: Option<String> = None;
    let mut cur: HashMap<String, String> = HashMap::new();
    let mut in_game = false;
    let mut in_image = false;
    let mut ids: HashMap<String, Vec<usize>> = HashMap::new();
    // game index -> type -> (region rank, file)
    let mut found: HashMap<usize, HashMap<String, Vec<(i32, String)>>> = HashMap::new();
    for line in BufReader::with_capacity(1 << 20, xml).lines() {
        let Ok(line) = line else { break };
        // A game's overview can run over several lines.
        if let Some(ov) = overview.as_mut() {
            match line.find("</Overview>") {
                Some(end) => {
                    ov.push('\n');
                    ov.push_str(&line[..end]);
                    cur.insert("Overview".into(), overview.take().unwrap_or_default());
                }
                None => {
                    ov.push('\n');
                    ov.push_str(&line);
                }
            }
            continue;
        }
        let t = line.trim();
        if in_game {
            if let Some(rest) = t.strip_prefix("<Overview>") {
                if !rest.contains("</Overview>") {
                    overview = Some(rest.to_string());
                    continue;
                }
            }
        }
        match t {
            "<Game>" => {
                in_game = true;
                cur.clear();
            }
            "</Game>" => {
                in_game = false;
                let id = cur.get("DatabaseID").cloned().unwrap_or_default();
                let plat = cur.get("Platform").map(|s| unescape(s)).unwrap_or_default();
                let details = || GameInfo {
                    overview: unescape(cur.get("Overview").map(String::as_str).unwrap_or("")).trim().to_string(),
                    developer: unescape(cur.get("Developer").map(String::as_str).unwrap_or("")),
                    publisher: unescape(cur.get("Publisher").map(String::as_str).unwrap_or("")),
                    released: cur.get("ReleaseDate").map(|d| d.chars().take(10).collect()).or_else(|| cur.get("ReleaseYear").cloned()).unwrap_or_default(),
                    genres: unescape(cur.get("Genres").map(String::as_str).unwrap_or("")).replace(';', ", "),
                    players: cur.get("MaxPlayers").cloned().unwrap_or_default(),
                    rating: cur.get("CommunityRating").map(|r| r.chars().take(4).collect()).unwrap_or_default(),
                };
                if let Some(&i) = cur.get("SteamAppId").and_then(|a| by_app.get(a)) {
                    best[i] = Some((2.0, id.clone()));
                    info[i] = details();
                }
                if wanted_platforms.contains(plat.as_str()) {
                    let name = normalize_title(&unescape(cur.get("Name").map(String::as_str).unwrap_or("")));
                    for (i, g) in todo.iter().enumerate() {
                        // Cheap pre-filter: same platform and same first word.
                        if titles[i].first() != name.first() || lb_platform(g.platform) != Some(plat.as_str()) {
                            continue;
                        }
                        let s = crate::covers::similarity(&titles[i], &name);
                        if s >= 0.8 && best[i].as_ref().map_or(true, |(b, _)| s > *b) {
                            best[i] = Some((s, id.clone()));
                            info[i] = details();
                        }
                    }
                }
            }
            "<GameImage>" => {
                if ids.is_empty() {
                    for (i, b) in best.iter().enumerate() {
                        if let Some((_, id)) = b {
                            ids.entry(id.clone()).or_default().push(i);
                        }
                    }
                }
                in_image = true;
                cur.clear();
            }
            "</GameImage>" => {
                in_image = false;
                if let (Some(id), Some(ty), Some(file)) = (cur.get("DatabaseID"), cur.get("Type"), cur.get("FileName")) {
                    if let Some(list) = ids.get(id) {
                        let rank = region_rank(cur.get("Region").map(String::as_str).unwrap_or(""));
                        for &i in list {
                            found.entry(i).or_default().entry(unescape(ty)).or_default().push((rank, file.clone()));
                        }
                    }
                }
            }
            _ if in_game || in_image => {
                if let Some((k, v)) = tag(t) {
                    if matches!(
                        k,
                        "DatabaseID" | "Platform" | "SteamAppId" | "Name" | "Type" | "FileName" | "Region" | "Overview" | "Developer" | "Publisher" | "ReleaseDate" | "ReleaseYear" | "Genres" | "MaxPlayers" | "CommunityRating"
                    ) {
                        cur.insert(k.to_string(), v.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    let pick = |m: &HashMap<String, Vec<(i32, String)>>, ty: &str| -> Option<String> {
        m.get(ty).and_then(|v| v.iter().min_by_key(|(r, _)| *r)).map(|(_, f)| f.clone())
    };
    let Ok(mut s) = state().write() else { return 0 };
    let mut n = 0;
    for (i, g) in todo.iter().enumerate() {
        s.checked.insert(g.id.clone());
        let empty = HashMap::new();
        let m = match found.get(&i) {
            Some(m) => m,
            None if best[i].is_some() => &empty,
            None => continue,
        };
        let mut shots: Vec<(i32, String)> = m.get("Screenshot - Gameplay").cloned().unwrap_or_default();
        shots.sort();
        let a = Assets {
            front: pick(m, "Box - Front"),
            back: pick(m, "Box - Back"),
            spine: pick(m, "Box - Spine"),
            disc: pick(m, "Disc"),
            cart: pick(m, "Cart - Front"),
            logo: pick(m, "Clear Logo"),
            fanart: pick(m, "Fanart - Background"),
            screenshots: shots.into_iter().take(6).map(|(_, f)| f).collect(),
            info: std::mem::take(&mut info[i]),
        };
        s.found.insert(g.id.clone(), a);
        n += 1;
    }
    if let Ok(text) = serde_json::to_vec(&*s) {
        let _ = std::fs::write(dir().join("matches.json"), text);
    }
    n
}

/// Forget all matches (e.g. to pick up a fresher database).
#[allow(dead_code)]
pub fn reset() {
    if let Ok(mut s) = state().write() {
        *s = Matches::default();
    }
    let _ = std::fs::remove_file(dir().join("matches.json"));
}
