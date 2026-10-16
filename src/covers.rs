//! Cover art, back covers and metadata.
//!
//! A pool of worker threads receives jobs, looks for the image in this order:
//! user override -> disk cache -> sources in the configured order, decodes it, fits it to
//! the shape of the case it goes on, computes the dominant colour and sends RGBA pixels back.
//! The render thread never blocks on I/O.
//!
//! Free sources (no key needed): local Steam library cache, Steam store API + CDN, Steam
//! store search (non-Steam PC games), libretro-thumbnails (fuzzy matched against the
//! thumbnail index), Wikipedia. Optional with a free key: SteamGridDB, TheGamesDB.
//!
//! An image whose shape does not match the case (e.g. a wide Steam banner for a DVD case)
//! is only used when no source has a better one, and is then letterboxed over a blurred
//! copy of itself instead of being cropped to a sliver.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use bevy::prelude::Resource;
use crossbeam_channel::{unbounded, Receiver, Sender};
use image::{imageops, DynamicImage, Rgba, RgbaImage};

use crate::config::{cache_dir, Covers};
use crate::library::Game;
use crate::platform::Platform;
use crate::render::{draw_text, fit_text, wrap_text, ArtKind};

#[derive(Clone)]
pub struct CoverRequest {
    pub id: String,
    pub title: String,
    pub steam_appid: Option<u32>,
    pub path: Option<String>,
    pub cover_path: Option<String>,
    pub libretro_system: Option<String>,
    /// PC games may be looked up in the Steam store by name.
    pub is_pc: bool,
    /// width / height of the printed cover the image will be shown on.
    pub aspect: f32,
    pub want_metadata: bool,
}

impl CoverRequest {
    pub fn from_game(g: &Game, cfg: &crate::config::Config) -> Self {
        CoverRequest {
            id: g.id.clone(),
            title: g.title.clone(),
            steam_appid: g.steam_appid,
            path: g.path.clone(),
            cover_path: g.cover_path.clone(),
            libretro_system: libretro_system(g, cfg),
            is_pc: matches!(g.platform, Platform::Pc | Platform::Other),
            aspect: crate::render::front_aspect(cfg, g.platform),
            want_metadata: cfg.covers.fetch_metadata && g.description.is_empty(),
        }
    }
}

fn libretro_system(g: &Game, cfg: &crate::config::Config) -> Option<String> {
    cfg.platform_override(g.platform)
        .and_then(|o| o.libretro_system.clone())
        .or_else(|| g.platform.libretro_system().map(str::to_owned))
}

/// Everything needed to print the back of a game case.
#[derive(Clone)]
pub struct BackRequest {
    pub id: String,
    pub title: String,
    pub platform: String,
    /// "2004  |  Action, RPG"
    pub info: String,
    pub description: String,
    pub accent: [f32; 3],
    pub aspect: f32,
    pub steam_appid: Option<u32>,
    pub libretro_system: Option<String>,
    pub path: Option<String>,
    pub cover_path: Option<String>,
    /// Picked by the user, used as is.
    pub chosen: Option<String>,
}

impl BackRequest {
    pub fn from_game(g: &Game, cfg: &crate::config::Config, aspect: f32) -> Self {
        let mut info: Vec<String> = Vec::new();
        if !g.year.is_empty() {
            info.push(g.year.clone());
        }
        if !g.genres.is_empty() {
            info.push(g.genres.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
        }
        BackRequest {
            id: g.id.clone(),
            title: g.title.clone(),
            platform: g.platform.display_name().to_string(),
            info: info.join("   |   "),
            description: g.description.clone(),
            accent: crate::render::accent(cfg, g.platform),
            aspect,
            steam_appid: g.steam_appid,
            libretro_system: libretro_system(g, cfg),
            path: g.path.clone(),
            cover_path: g.cover_path.clone(),
            chosen: g.back_art.clone(),
        }
    }
}

pub struct CoverReady {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub dominant: [f32; 3],
}

pub struct MetaReady {
    pub id: String,
    pub description: String,
    pub genres: Vec<String>,
    pub year: String,
}

pub enum CoverMsg {
    Cover(CoverReady),
    /// Nothing found (so we do not ask again this session).
    Missing(String),
    Meta(MetaReady),
    /// A back cover (same layout as `Cover`, dominant unused).
    Back(CoverReady),
    Spine(CoverReady),
    Backdrop(CoverReady),
    /// The cartridge's plastic colour (sRGB + alpha), if a photo of it was found.
    CartColor(String, Option<[f32; 4]>),
    /// Art of its own for a game's disc / cartridge label or insert (none found: the cover
    /// stays there).
    Art(ArtKind, CoverReady),
    /// The insert's pages (inside of the cover, then both sides of each sheet).
    Pages(PagesReady),
}

/// Art for the disc / cartridge label or the insert, so they need not repeat the cover.
#[derive(Clone)]
pub struct ArtRequest {
    /// The game (ids, Steam app, screenshots lookups).
    pub game: BackRequest,
    pub kind: ArtKind,
    /// The media is an optical disc (LaunchBox has scans of those).
    pub disc: bool,
    /// width / height of the printed area.
    pub aspect: f32,
    /// Picked by the user (a local file), used as is.
    pub chosen: Option<String>,
}

impl ArtRequest {
    pub fn from_game(g: &Game, cfg: &crate::config::Config, kind: ArtKind) -> Self {
        let disc = matches!(crate::render::geometry(cfg, g.platform).media, crate::platform::Media::Disc { .. });
        let aspect = if kind == ArtKind::Media && disc { 1.0 } else { crate::render::front_aspect(cfg, g.platform) };
        let chosen = match kind {
            ArtKind::Media => g.media_art.clone(),
            ArtKind::Insert => g.insert_art.clone(),
            ArtKind::Cover | ArtKind::Back | ArtKind::Spine => None,
        };
        ArtRequest { game: BackRequest::from_game(g, cfg, aspect), kind, disc, aspect, chosen }
    }
}

/// A spine label for a case.
#[derive(Clone)]
pub struct SpineRequest {
    pub id: String,
    pub title: String,
    pub platform: String,
    pub accent: [f32; 3],
    /// width / height of the spine label.
    pub aspect: f32,
    pub steam_appid: Option<u32>,
    pub cover_path: Option<String>,
    /// Use the generated logo spine even when a scanned spine exists.
    pub prefer_generated: bool,
    /// Picked by the user, used as is.
    pub chosen: Option<String>,
}

/// A blurred full-screen backdrop for the selected game.
#[derive(Clone)]
pub struct BackdropRequest {
    pub id: String,
    /// Blur radius multiplier (1 = default).
    pub blur: f32,
    pub steam_appid: Option<u32>,
    pub cover_path: Option<String>,
}

enum Job {
    Cover(CoverRequest),
    Back(BackRequest),
    Spine(SpineRequest),
    Backdrop(BackdropRequest),
    /// Measure a game's cartridge colour from its LaunchBox cartridge photo.
    CartColor(String),
    Art(ArtRequest),
    Pages(PageRequest),
}

#[derive(Resource)]
pub struct CoverService {
    tx: Sender<Job>,
    pub rx: Receiver<CoverMsg>,
    pub pending: HashSet<String>,
    pub missing: HashSet<String>,
    pub backs_pending: HashSet<String>,
    pub spines_pending: HashSet<String>,
    pub backdrops_pending: HashSet<String>,
    /// Games whose cartridge colour was asked for this session.
    pub carts_asked: HashSet<String>,
    /// Disc / insert art asked for this session.
    pub arts_asked: HashSet<(String, ArtKind)>,
    /// Insert pages asked for this session.
    pub pages_asked: HashSet<String>,
    settings: Arc<std::sync::RwLock<Covers>>,
}

struct Worker {
    agent: ureq::Agent,
    cache: PathBuf,
    shared: Arc<std::sync::RwLock<Covers>>,
    settings: Covers,
    font: Arc<Option<fontdue::Font>>,
    steam_caches: Vec<PathBuf>,
}

impl CoverService {
    pub fn start(settings: Covers) -> Self {
        let (tx, job_rx) = unbounded::<Job>();
        let (res_tx, rx) = unbounded::<CoverMsg>();
        let cache = cache_dir().join("covers");
        let _ = std::fs::create_dir_all(&cache);
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(6))
            .timeout(Duration::from_secs(20))
            .user_agent("slipcase/0.1 (https://github.com/; game launcher cover lookup)")
            .build();
        let font = Arc::new(crate::render::load_font());
        let steam_caches: Vec<PathBuf> = crate::scanner::steam_roots()
            .into_iter()
            .map(|r| r.join("appcache").join("librarycache"))
            .filter(|p| p.is_dir())
            .collect();
        let shared = Arc::new(std::sync::RwLock::new(settings.clone()));
        for _ in 0..4 {
            let mut w = Worker {
                agent: agent.clone(),
                cache: cache.clone(),
                shared: shared.clone(),
                settings: settings.clone(),
                font: font.clone(),
                steam_caches: steam_caches.clone(),
            };
            let (job_rx, res_tx) = (job_rx.clone(), res_tx.clone());
            std::thread::spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    if let Ok(s) = w.shared.read() {
                        w.settings = s.clone();
                    }
                    match job {
                        Job::Cover(req) => {
                            let msg = match w.load_cover(&req) {
                                Some(c) => CoverMsg::Cover(c),
                                None => CoverMsg::Missing(req.id.clone()),
                            };
                            let _ = res_tx.send(msg);
                            if w.settings.enabled && req.want_metadata {
                                if let Some(m) = w.fetch_meta(&req) {
                                    let _ = res_tx.send(CoverMsg::Meta(m));
                                }
                            }
                        }
                        Job::Back(req) => {
                            let b = w.make_back(&req);
                            keep_made(&b, ArtKind::Back);
                            let _ = res_tx.send(CoverMsg::Back(b));
                        }
                        Job::Spine(req) => {
                            let s = w.make_spine(&req);
                            keep_made(&s, ArtKind::Spine);
                            let _ = res_tx.send(CoverMsg::Spine(s));
                        }
                        Job::Backdrop(req) => {
                            if let Some(b) = w.make_backdrop(&req) {
                                let _ = res_tx.send(CoverMsg::Backdrop(b));
                            }
                        }
                        Job::CartColor(id) => {
                            let c = w.cart_color(&id);
                            let _ = res_tx.send(CoverMsg::CartColor(id, c));
                        }
                        Job::Pages(req) => {
                            let _ = res_tx.send(CoverMsg::Pages(w.make_pages(&req)));
                        }
                        Job::Art(req) => {
                            if let Some(a) = w.make_art(&req) {
                                keep_made(&a, req.kind);
                                let _ = res_tx.send(CoverMsg::Art(req.kind, a));
                            }
                        }
                    }
                }
            });
        }
        CoverService {
            tx,
            rx,
            pending: HashSet::new(),
            missing: HashSet::new(),
            backs_pending: HashSet::new(),
            spines_pending: HashSet::new(),
            backdrops_pending: HashSet::new(),
            carts_asked: HashSet::new(),
            arts_asked: HashSet::new(),
            pages_asked: HashSet::new(),
            settings: shared,
        }
    }

    /// New cover settings (sources, keys) take effect for the next job.
    pub fn set_settings(&self, c: &Covers) {
        if let Ok(mut s) = self.settings.write() {
            *s = c.clone();
        }
    }

    pub fn request_spine(&mut self, req: SpineRequest) {
        if self.spines_pending.insert(req.id.clone()) {
            let _ = self.tx.send(Job::Spine(req));
        }
    }

    /// Once per session per game: look up the cartridge colour.
    pub fn request_cart_color(&mut self, id: &str) {
        if self.carts_asked.insert(id.to_string()) {
            let _ = self.tx.send(Job::CartColor(id.to_string()));
        }
    }

    /// Once per session per game and kind (until `forget_art`).
    pub fn request_art(&mut self, req: ArtRequest) {
        if self.arts_asked.insert((req.game.id.clone(), req.kind)) {
            let _ = self.tx.send(Job::Art(req));
        }
    }

    /// Once per session per game: the insert's pages.
    pub fn request_pages(&mut self, req: PageRequest) {
        if self.pages_asked.insert(req.game.id.clone()) {
            let _ = self.tx.send(Job::Pages(req));
        }
    }

    /// Ask again for a game's disc and insert art (after the user picked new art).
    pub fn forget_art(&mut self, id: &str) {
        self.arts_asked.retain(|(i, _)| i != id);
    }

    pub fn request_backdrop(&mut self, req: BackdropRequest) {
        if self.backdrops_pending.insert(req.id.clone()) {
            let _ = self.tx.send(Job::Backdrop(req));
        }
    }

    pub fn request(&mut self, req: CoverRequest) {
        if self.pending.contains(&req.id) || self.missing.contains(&req.id) {
            return;
        }
        self.pending.insert(req.id.clone());
        let _ = self.tx.send(Job::Cover(req));
    }

    pub fn request_back(&mut self, req: BackRequest) {
        if self.backs_pending.insert(req.id.clone()) {
            let _ = self.tx.send(Job::Back(req));
        }
    }
}

fn safe_name(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn cache_file(cache: &Path, id: &str) -> PathBuf {
    cache.join(format!("{}.img", safe_name(id)))
}

/// Deletes the cached cover (and its re-check marker) so the next request downloads it again.
pub fn forget_cover(id: &str) {
    let f = cache_file(&cache_dir().join("covers"), id);
    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_file(f.with_extension("checked"));
    let _ = std::fs::remove_file(cache_dir().join("spines").join(format!("{}.img", safe_name(id))));
}

fn age(p: &Path) -> Option<Duration> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok())
}

/// A cover "fits" when its shape is close to the case's printed area.
fn fits(img: &DynamicImage, aspect: f32) -> bool {
    let r = img.width() as f32 / img.height().max(1) as f32 / aspect.max(0.01);
    (0.78..=1.28).contains(&r)
}

enum Candidate {
    Url(String),
    File(PathBuf),
}

impl Worker {
    fn load_cover(&self, req: &CoverRequest) -> Option<CoverReady> {
        let s = &self.settings;
        // 1. user supplied image
        if let Some(p) = &req.cover_path {
            if let Some(img) = std::fs::read(p).ok().and_then(|b| image::load_from_memory(&b).ok()) {
                return Some(finish(&req.id, img, req.aspect, s.max_texture_size));
            }
        }
        // 2. disk cache
        let file = cache_file(&self.cache, &req.id);
        let cached = std::fs::read(&file).ok().and_then(|b| image::load_from_memory(&b).ok());
        if cached.is_none() {
            let _ = std::fs::remove_file(&file);
        }
        if let Some(img) = &cached {
            if fits(img, req.aspect) || !s.enabled {
                return Some(finish(&req.id, img.clone(), req.aspect, s.max_texture_size));
            }
        }
        if !s.enabled {
            return None;
        }
        // A cached cover with the wrong shape: look for a better one at most once a week.
        let marker = file.with_extension("checked");
        if cached.is_some() && age(&marker).is_some_and(|a| a < Duration::from_secs(7 * 86400)) {
            return cached.map(|img| finish(&req.id, img, req.aspect, s.max_texture_size));
        }

        // 3. sources, in the configured order
        let mut fallback: Option<(Vec<u8>, DynamicImage)> = None;
        for source in &s.sources {
            for cand in self.candidates(source, req) {
                let bytes = match &cand {
                    Candidate::Url(u) => http_bytes(&self.agent, u, 12 * 1024 * 1024),
                    Candidate::File(p) => std::fs::read(p).ok(),
                };
                let Some(bytes) = bytes else { continue };
                let Ok(img) = image::load_from_memory(&bytes) else { continue };
                if img.width() < 64 || img.height() < 64 {
                    continue;
                }
                if fits(&img, req.aspect) {
                    let _ = std::fs::write(&file, &bytes);
                    let _ = std::fs::remove_file(&marker);
                    return Some(finish(&req.id, img, req.aspect, s.max_texture_size));
                }
                if fallback.is_none() {
                    fallback = Some((bytes, img));
                }
            }
        }
        let _ = std::fs::write(&marker, b"");
        if let Some(img) = cached {
            return Some(finish(&req.id, img, req.aspect, s.max_texture_size));
        }
        let (bytes, img) = fallback?;
        let _ = std::fs::write(&file, &bytes);
        Some(finish(&req.id, img, req.aspect, s.max_texture_size))
    }

    fn candidates(&self, source: &str, req: &CoverRequest) -> Vec<Candidate> {
        let urls = |v: Vec<String>| v.into_iter().map(Candidate::Url).collect::<Vec<_>>();
        match source {
            "steamgriddb" => urls(steamgriddb_urls(&self.agent, &self.settings.steamgriddb_api_key, req)),
            "steam" => {
                let appid = req.steam_appid.or_else(|| if req.is_pc { steam_search(&self.agent, &req.title) } else { None });
                match appid {
                    Some(id) => self.steam_candidates(id),
                    None => vec![],
                }
            }
            "libretro" => urls(libretro_urls(&self.agent, &self.cache, req, "Named_Boxarts")),
            "thegamesdb" => urls(thegamesdb_urls(&self.agent, &self.settings.thegamesdb_api_key, &req.title)),
            "launchbox" => urls(crate::launchbox::assets(&req.id).and_then(|a| a.front).map(|f| crate::launchbox::url(&f)).into_iter().collect()),
            "wikipedia" => urls(wikipedia_lookup(&self.agent, &req.title).map(|w| w.image).into_iter().flatten().collect()),
            _ => vec![],
        }
    }

    /// Steam: the client's own library art cache first (has art for every game you own,
    /// even ones whose CDN paths are hashed), then the store API, then the classic CDN paths.
    fn steam_candidates(&self, appid: u32) -> Vec<Candidate> {
        let mut out = Vec::new();
        for root in &self.steam_caches {
            let dir = root.join(appid.to_string());
            let mut files = Vec::new();
            collect_files(&dir, 2, &mut files);
            files.sort_by_key(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                match n.as_str() {
                    "library_600x900_2x.jpg" => 0,
                    "library_600x900.jpg" => 1,
                    "library_capsule_2x.jpg" => 2,
                    "library_capsule.jpg" => 3,
                    _ => 9,
                }
            });
            out.extend(
                files
                    .into_iter()
                    .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("library_600x900") || n.to_string_lossy().starts_with("library_capsule")))
                    .map(Candidate::File),
            );
        }
        let mut header: Option<String> = None;
        if let Some(assets) = steam_assets(&self.agent, appid) {
            for k in ["library_capsule_2x", "library_capsule"] {
                if let Some(u) = assets.get(k) {
                    out.push(Candidate::Url(u.clone()));
                }
            }
            header = assets.get("header").cloned();
        }
        let base = format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{appid}");
        out.push(Candidate::Url(format!("{base}/library_600x900_2x.jpg")));
        out.push(Candidate::Url(format!("{base}/library_600x900.jpg")));
        // Wide banner: only used if nothing else in any source fits.
        out.push(Candidate::Url(header.unwrap_or(format!("{base}/header.jpg"))));
        out
    }

    fn fetch_meta(&self, req: &CoverRequest) -> Option<MetaReady> {
        if let Some(app) = req.steam_appid {
            return fetch_steam_meta(&self.agent, app, &req.id);
        }
        if !self.settings.sources.iter().any(|s| s == "wikipedia") {
            return None;
        }
        let page = wikipedia_lookup(&self.agent, &req.title)?;
        let v: serde_json::Value = self
            .agent
            .get(&format!("https://en.wikipedia.org/api/rest_v1/page/summary/{}", url_escape(&page.title.replace(' ', "_"))))
            .call()
            .ok()?
            .into_json()
            .ok()?;
        let description = v.get("extract").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let short = v.get("description").and_then(|s| s.as_str()).unwrap_or("");
        let year = short.split_whitespace().find(|w| w.len() == 4 && w.chars().all(|c| c.is_ascii_digit())).unwrap_or("").to_string();
        Some(MetaReady { id: req.id.clone(), description, genres: vec![], year })
    }

    // --------------------------------------------------------------------------------------------
    // Back covers
    // --------------------------------------------------------------------------------------------

    fn screenshots(&self, req: &BackRequest) -> Vec<RgbaImage> {
        let dir = cache_dir().join("snaps");
        let _ = std::fs::create_dir_all(&dir);
        let mut out = Vec::new();
        for n in 0..2 {
            let f = dir.join(format!("{}_{n}.img", safe_name(&req.id)));
            if let Some(img) = std::fs::read(&f).ok().and_then(|b| image::load_from_memory(&b).ok()) {
                out.push(img.to_rgba8());
            }
        }
        if !out.is_empty() || !self.settings.enabled {
            return out;
        }
        let mut urls: Vec<String> = Vec::new();
        if let Some(a) = self.lb(&req.id) {
            urls.extend(a.screenshots.iter().map(|f| crate::launchbox::url(f)));
        }
        if !urls.is_empty() {
        } else if let Some(app) = req.steam_appid {
            if let Some(v) = self
                .agent
                .get(&format!("https://store.steampowered.com/api/appdetails?appids={app}&filters=screenshots"))
                .call()
                .ok()
                .and_then(|r| r.into_json::<serde_json::Value>().ok())
            {
                if let Some(a) = v.pointer(&format!("/{app}/data/screenshots")).and_then(|a| a.as_array()) {
                    urls.extend(a.iter().take(2).filter_map(|s| s.get("path_thumbnail").and_then(|u| u.as_str()).map(str::to_owned)));
                }
            }
        } else if req.libretro_system.is_some() {
            let cr = CoverRequest {
                id: req.id.clone(),
                title: req.title.clone(),
                steam_appid: None,
                path: req.path.clone(),
                cover_path: None,
                libretro_system: req.libretro_system.clone(),
                is_pc: false,
                aspect: 1.0,
                want_metadata: false,
            };
            for kind in ["Named_Snaps", "Named_Titles"] {
                urls.extend(libretro_urls(&self.agent, &self.cache, &cr, kind).into_iter().take(1));
            }
        }
        for (n, u) in urls.iter().take(2).enumerate() {
            if let Some(bytes) = http_bytes(&self.agent, u, 8 * 1024 * 1024) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    let _ = std::fs::write(dir.join(format!("{}_{n}.img", safe_name(&req.id))), &bytes);
                    out.push(img.to_rgba8());
                }
            }
        }
        out
    }

    fn lb(&self, id: &str) -> Option<crate::launchbox::Assets> {
        if self.settings.sources.iter().any(|s| s == "launchbox") {
            crate::launchbox::assets(id)
        } else {
            None
        }
    }

    /// An image cached on disk under `kind/`, downloaded from `url` the first time.
    fn cached_image(&self, kind: &str, id: &str, url: &str) -> Option<DynamicImage> {
        let dir = cache_dir().join(kind);
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join(format!("{}.img", safe_name(id)));
        if let Some(img) = std::fs::read(&f).ok().and_then(|b| image::load_from_memory(&b).ok()) {
            return Some(img);
        }
        if !self.settings.enabled {
            return None;
        }
        let bytes = http_bytes(&self.agent, url, 16 * 1024 * 1024)?;
        let img = image::load_from_memory(&bytes).ok()?;
        let _ = std::fs::write(&f, &bytes);
        Some(img)
    }

    fn cover_image(&self, id: &str, cover_path: &Option<String>) -> Option<RgbaImage> {
        cover_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .or_else(|| std::fs::read(cache_file(&self.cache, id)).ok())
            .and_then(|b| image::load_from_memory(&b).ok())
            .map(|i| i.to_rgba8())
    }

    /// Files in the Steam client's art cache for an app whose name starts with `prefix`.
    fn steam_local(&self, appid: u32, prefix: &str) -> Option<DynamicImage> {
        for root in &self.steam_caches {
            let mut files = Vec::new();
            collect_files(&root.join(appid.to_string()), 2, &mut files);
            files.sort();
            for f in files {
                if f.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix)) {
                    if let Some(img) = std::fs::read(&f).ok().and_then(|b| image::load_from_memory(&b).ok()) {
                        return Some(img);
                    }
                }
            }
        }
        None
    }

    fn logo(&self, id: &str, steam_appid: Option<u32>) -> Option<RgbaImage> {
        if let Some(file) = self.lb(id).and_then(|a| a.logo) {
            if let Some(img) = self.cached_image("logos", id, &crate::launchbox::url(&file)) {
                return Some(img.to_rgba8());
            }
        }
        let app = steam_appid?;
        if let Some(img) = self.steam_local(app, "logo") {
            return Some(img.to_rgba8());
        }
        self.cached_image("logos", id, &format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app}/logo.png")).map(|i| i.to_rgba8())
    }

    fn make_spine(&self, req: &SpineRequest) -> CoverReady {
        let (l, t) = (1024u32, ((1024.0 * req.aspect.min(1.0 / req.aspect.max(0.01))) as u32).clamp(40, 320));
        let lb = self.lb(&req.id);
        let done = |img: RgbaImage| {
            let (w, h) = img.dimensions();
            CoverReady { id: req.id.clone(), width: w, height: h, rgba: img.into_raw(), dominant: [0.0; 3] }
        };
        // 0. The user's pick (stood upright if it lies down).
        if let Some(img) = req.chosen.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| image::load_from_memory(&b).ok()) {
            let mut img = img.to_rgba8();
            if img.width() > img.height() {
                img = imageops::rotate90(&img);
            }
            return done(cover_fill(&img, t, l));
        }
        // 1. A scanned spine.
        if !req.prefer_generated {
            if let Some(file) = lb.as_ref().and_then(|a| a.spine.clone()) {
                if let Some(img) = self.cached_image("spines", &req.id, &crate::launchbox::url(&file)) {
                    let mut img = img.to_rgba8();
                    // Our label stands upright; scans are sometimes stored lying down.
                    if img.width() > img.height() {
                        img = imageops::rotate90(&img);
                    }
                    return done(cover_fill(&img, t, l));
                }
            }
        }
        // 2. Generated: the game's logo over its art, with a platform band at the top.
        let art = lb
            .as_ref()
            .and_then(|a| a.fanart.clone())
            .and_then(|f| self.cached_image("fanart", &req.id, &crate::launchbox::url(&f)).map(|i| i.to_rgba8()))
            .or_else(|| self.cover_image(&req.id, &req.cover_path));
        let logo = self.logo(&req.id, req.steam_appid);
        let canvas = compose_spine(req, l, t, art.as_ref(), logo.as_ref(), (*self.font).as_ref());
        done(imageops::rotate90(&canvas))
    }

    fn make_backdrop(&self, req: &BackdropRequest) -> Option<CoverReady> {
        let lb = self.lb(&req.id);
        let mut img: Option<DynamicImage> = None;
        if let Some(f) = lb.as_ref().and_then(|a| a.fanart.clone()) {
            img = self.cached_image("fanart", &req.id, &crate::launchbox::url(&f));
        }
        if img.is_none() {
            if let Some(app) = req.steam_appid {
                img = self.steam_local(app, "library_hero");
                if img.is_none() && self.settings.enabled {
                    let url = steam_assets(&self.agent, app)
                        .and_then(|a| a.get("library_hero").cloned())
                        .unwrap_or(format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app}/library_hero.jpg"));
                    img = self.cached_image("heroes", &req.id, &url);
                }
            }
        }
        if img.is_none() {
            if let Some(f) = lb.as_ref().and_then(|a| a.screenshots.first().cloned()) {
                img = self.cached_image("snaps", &format!("{}_0", req.id), &crate::launchbox::url(&f));
            }
        }
        let img = match img {
            Some(i) => i.to_rgba8(),
            None => self.cover_image(&req.id, &req.cover_path)?,
        };
        // Small and pre-blurred: the GPU's bilinear upscale adds the rest of the softness.
        let w = 480u32;
        let h = ((img.height() as f32 / img.width().max(1) as f32) * w as f32).max(8.0) as u32;
        let small = imageops::resize(&img, w, h, imageops::FilterType::Triangle);
        let mut out = if req.blur > 0.0 { imageops::blur(&small, 2.2 * req.blur) } else { small };
        for p in out.pixels_mut() {
            p.0[3] = 255;
        }
        let (w, h) = out.dimensions();
        Some(CoverReady { id: req.id.clone(), width: w, height: h, rgba: out.into_raw(), dominant: [0.0; 3] })
    }

    fn make_back(&self, req: &BackRequest) -> CoverReady {
        // The user's pick first.
        if let Some(img) = req.chosen.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| image::load_from_memory(&b).ok()) {
            let fitted = fit_to_aspect(&img.to_rgba8(), req.aspect, self.settings.max_texture_size);
            let (w, h) = fitted.dimensions();
            return CoverReady { id: req.id.clone(), width: w, height: h, rgba: fitted.into_raw(), dominant: [0.0; 3] };
        }
        // A scanned back cover from LaunchBox beats anything generated.
        if let Some(file) = self.lb(&req.id).and_then(|a| a.back) {
            if let Some(img) = self.cached_image("backs", &req.id, &crate::launchbox::url(&file)) {
                let fitted = fit_to_aspect(&img.to_rgba8(), req.aspect, self.settings.max_texture_size);
                let (w, h) = fitted.dimensions();
                return CoverReady { id: req.id.clone(), width: w, height: h, rgba: fitted.into_raw(), dominant: [0.0; 3] };
            }
        }
        self.compose_back(req)
    }

    fn compose_back(&self, req: &BackRequest) -> CoverReady {
        let cover = self.cover_image(&req.id, &req.cover_path);
        let shots = self.screenshots(req);
        let img = compose_back(req, cover.as_ref(), &shots, (*self.font).as_ref());
        let (w, h) = img.dimensions();
        CoverReady { id: req.id.clone(), width: w, height: h, rgba: img.into_raw(), dominant: [0.0; 3] }
    }
}

impl Worker {
    /// Steam's wide hero art for a game (local Steam cache first).
    fn hero(&self, id: &str, steam_appid: Option<u32>) -> Option<RgbaImage> {
        let app = steam_appid?;
        if let Some(img) = self.steam_local(app, "library_hero") {
            return Some(img.to_rgba8());
        }
        if !self.settings.enabled {
            return None;
        }
        let url = steam_assets(&self.agent, app)
            .and_then(|a| a.get("library_hero").cloned())
            .unwrap_or(format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app}/library_hero.jpg"));
        self.cached_image("heroes", id, &url).map(|i| i.to_rgba8())
    }

    fn fanart(&self, id: &str) -> Option<RgbaImage> {
        let f = self.lb(id)?.fanart?;
        self.cached_image("fanart", id, &crate::launchbox::url(&f)).map(|i| i.to_rgba8())
    }

    /// The disc / cartridge label's or the insert's own art: the user's pick, else a scan of the
    /// disc, else art composed from the game's hero art / screenshots with its logo. None keeps
    /// the cover there.
    fn make_art(&self, req: &ArtRequest) -> Option<CoverReady> {
        let id = &req.game.id;
        let max = self.settings.max_texture_size.clamp(256, 1024);
        let done = |img: RgbaImage| {
            let (w, h) = img.dimensions();
            CoverReady { id: id.clone(), width: w, height: h, rgba: img.into_raw(), dominant: [0.0; 3] }
        };
        if let Some(p) = &req.chosen {
            let img = std::fs::read(p).ok().and_then(|b| image::load_from_memory(&b).ok())?.to_rgba8();
            return Some(done(cover_fill(&img, (max as f32 * req.aspect.min(1.0)) as u32, (max as f32 / req.aspect.max(1.0)) as u32)));
        }
        match req.kind {
            ArtKind::Cover | ArtKind::Back | ArtKind::Spine => None,
            ArtKind::Media => {
                if !req.disc {
                    // Cartridge labels keep the box art unless the user picks something.
                    return None;
                }
                // A scan of the actual disc.
                if let Some(f) = self.lb(id).and_then(|a| a.disc) {
                    if let Some(img) = self.cached_image("discs", id, &crate::launchbox::url(&f)) {
                        let img = trim_alpha(&img.to_rgba8());
                        return Some(done(imageops::resize(&img, max, max, imageops::FilterType::Triangle)));
                    }
                }
                let art = self.hero(id, req.game.steam_appid).or_else(|| self.fanart(id))?;
                Some(done(compose_disc(&art, self.logo(id, req.game.steam_appid).as_ref(), max)))
            }
            ArtKind::Insert => {
                let art = self.screenshots(&req.game).into_iter().next().or_else(|| self.fanart(id)).or_else(|| self.hero(id, req.game.steam_appid))?;
                Some(done(compose_insert(&art, self.logo(id, req.game.steam_appid).as_ref(), req.aspect, max)))
            }
        }
    }
}

/// A disc label: the art filling the disc, darkened a touch, with the logo above the hole.
fn compose_disc(art: &RgbaImage, logo: Option<&RgbaImage>, size: u32) -> RgbaImage {
    let mut base = cover_fill(art, size, size);
    for p in base.pixels_mut() {
        for c in 0..3 {
            p.0[c] = (p.0[c] as f32 * 0.82) as u8;
        }
        p.0[3] = 255;
    }
    if let Some(logo) = logo {
        let l = trim_alpha(logo);
        let k = (size as f32 * 0.56 / l.width().max(1) as f32).min(size as f32 * 0.22 / l.height().max(1) as f32);
        let (w, h) = (((l.width() as f32 * k) as u32).max(1), ((l.height() as f32 * k) as u32).max(1));
        let l = imageops::resize(&l, w, h, imageops::FilterType::Triangle);
        let y = (size as f32 * 0.27 - h as f32 * 0.5).max(0.0) as i64;
        imageops::overlay(&mut base, &l, ((size - w) / 2) as i64, y);
    }
    base
}

/// An insert: the art (a screenshot) across the middle over a dark blurred copy of itself, with
/// the logo at the top.
fn compose_insert(art: &RgbaImage, logo: Option<&RgbaImage>, aspect: f32, max: u32) -> RgbaImage {
    let h = max;
    let w = ((max as f32 * aspect) as u32).max(16);
    let mut base = imageops::blur(&cover_fill(art, w / 4, h / 4), 6.0);
    base = imageops::resize(&base, w, h, imageops::FilterType::Triangle);
    for p in base.pixels_mut() {
        for c in 0..3 {
            p.0[c] = (p.0[c] as f32 * 0.45) as u8;
        }
        p.0[3] = 255;
    }
    let aw = (w as f32 * 0.9) as u32;
    let ah = ((art.height() as f32 / art.width().max(1) as f32) * aw as f32).min(h as f32 * 0.6) as u32;
    let shot = cover_fill(art, aw, ah.max(8));
    imageops::overlay(&mut base, &shot, ((w - aw) / 2) as i64, (h as f32 * 0.36) as i64);
    if let Some(logo) = logo {
        let l = trim_alpha(logo);
        let k = (w as f32 * 0.8 / l.width().max(1) as f32).min(h as f32 * 0.2 / l.height().max(1) as f32);
        let (lw, lh) = (((l.width() as f32 * k) as u32).max(1), ((l.height() as f32 * k) as u32).max(1));
        let l = imageops::resize(&l, lw, lh, imageops::FilterType::Triangle);
        imageops::overlay(&mut base, &l, ((w - lw) / 2) as i64, (h as f32 * 0.07) as i64);
    }
    base
}

fn collect_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                collect_files(&p, depth - 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

impl Worker {
    /// The plastic colour of a game's cartridge, measured from LaunchBox's photo of it
    /// (downloaded once into the cache).
    fn cart_color(&self, id: &str) -> Option<[f32; 4]> {
        if !self.settings.enabled || !self.settings.sources.iter().any(|s| s == "launchbox") {
            return None;
        }
        let file = crate::launchbox::assets(id)?.cart?;
        let dir = self.cache.join("carts");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(file.replace(['/', '\\'], "_"));
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                let b = http_bytes(&self.agent, &crate::launchbox::url(&file), 12 << 20)?;
                let _ = std::fs::write(&path, &b);
                b
            }
        };
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        shell_color(&img)
    }
}

/// The plastic colour of a cartridge photo: the median of the bands down its left and right
/// sides, where the shell shows beside the label. The background (transparent, or the colour
/// in the corner) is left out.
pub fn shell_color(img: &RgbaImage) -> Option<[f32; 4]> {
    let (w, h) = img.dimensions();
    if w < 16 || h < 16 {
        return None;
    }
    let corner = *img.get_pixel(1, 1);
    let dist = |a: &Rgba<u8>, b: &Rgba<u8>| (0..3).map(|k| (a[k] as i32 - b[k] as i32).abs()).sum::<i32>();
    let bg = |p: &Rgba<u8>| p[3] < 128 || (corner[3] >= 128 && dist(p, &corner) < 36);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            if !bg(img.get_pixel(x, y)) {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    if x1 <= x0 + 8 || y1 <= y0 + 8 {
        return None;
    }
    let (cw, ch) = ((x1 - x0) as f32, (y1 - y0) as f32);
    let mut px: Vec<[u8; 3]> = Vec::new();
    for y in (y0 + (ch * 0.35) as u32)..(y0 + (ch * 0.85) as u32) {
        for band in [(0.03f32, 0.09f32), (0.91, 0.97)] {
            for x in (x0 + (cw * band.0) as u32)..(x0 + (cw * band.1) as u32) {
                let p = img.get_pixel(x, y);
                if !bg(p) {
                    px.push([p[0], p[1], p[2]]);
                }
            }
        }
    }
    if px.len() < 20 {
        return None;
    }
    let median = |k: usize| {
        let mut v: Vec<u8> = px.iter().map(|p| p[k]).collect();
        v.sort_unstable();
        v[v.len() / 2] as f32 / 255.0
    };
    Some([median(0), median(1), median(2), 1.0])
}

fn http_bytes(agent: &ureq::Agent, url: &str, limit: u64) -> Option<Vec<u8>> {
    let resp = agent.get(url).call().ok()?;
    let mut buf = Vec::new();
    resp.into_reader().take(limit).read_to_end(&mut buf).ok()?;
    if buf.len() < 512 {
        None
    } else {
        Some(buf)
    }
}

// ------------------------------------------------------------------------------------------------
// Image processing
// ------------------------------------------------------------------------------------------------

fn finish(id: &str, img: DynamicImage, aspect: f32, max: u32) -> CoverReady {
    let rgba = img.to_rgba8();
    let dominant = dominant_color(&rgba);
    let fitted = fit_to_aspect(&rgba, aspect, max);
    let (w, h) = fitted.dimensions();
    CoverReady { id: id.to_string(), width: w, height: h, rgba: fitted.into_raw(), dominant }
}

fn has_transparency(img: &RgbaImage) -> bool {
    img.pixels().step_by(7).any(|p| p.0[3] < 250)
}

/// Scale to cover `w` x `h` (cropping the overflow), then blur and darken: a backdrop that
/// carries the colours of the art.
fn backdrop(img: &RgbaImage, w: u32, h: u32, dim: f32) -> RgbaImage {
    let (sw, sh) = ((w / 14).max(4), (h / 14).max(4));
    let small = cover_fill(img, sw, sh);
    let mut small = imageops::blur(&small, 1.6);
    for p in small.pixels_mut() {
        for k in 0..3 {
            p.0[k] = (p.0[k] as f32 * dim) as u8;
        }
        p.0[3] = 255;
    }
    imageops::resize(&small, w, h, imageops::FilterType::Triangle)
}

/// Scale and centre-crop to exactly `w` x `h`.
fn cover_fill(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let k = (w as f32 / iw).max(h as f32 / ih);
    let (rw, rh) = (((iw * k).ceil() as u32).max(w), ((ih * k).ceil() as u32).max(h));
    let r = imageops::resize(img, rw, rh, imageops::FilterType::Triangle);
    imageops::crop_imm(&r, (rw - w) / 2, (rh - h) / 2, w, h).to_image()
}

/// Returns an image with the case's aspect ratio. Close matches are passed through (the
/// renderer crops the small difference); anything else is letterboxed over a blurred
/// copy of itself. Transparent areas are flattened onto that backdrop as well.
fn fit_to_aspect(img: &RgbaImage, aspect: f32, max: u32) -> RgbaImage {
    let max = max.max(64);
    let (iw, ih) = img.dimensions();
    let ratio = iw as f32 / ih.max(1) as f32 / aspect.max(0.01);
    let transparent = has_transparency(img);
    if (0.82..=1.22).contains(&ratio) && !transparent {
        if iw.max(ih) > max {
            return imageops::resize(img, (iw * max / iw.max(ih)).max(1), (ih * max / iw.max(ih)).max(1), imageops::FilterType::Triangle);
        }
        return img.clone();
    }
    let (w, h) = if aspect <= 1.0 { (((max as f32) * aspect) as u32, max) } else { (max, ((max as f32) / aspect) as u32) };
    let (w, h) = (w.max(8), h.max(8));
    let mut out = backdrop(img, w, h, 0.55);
    let k = (w as f32 / iw as f32).min(h as f32 / ih as f32);
    let (fw, fh) = (((iw as f32 * k) as u32).max(1), ((ih as f32 * k) as u32).max(1));
    let fg = imageops::resize(img, fw, fh, imageops::FilterType::CatmullRom);
    imageops::overlay(&mut out, &fg, ((w - fw) / 2) as i64, ((h - fh) / 2) as i64);
    out
}

/// Picks a vivid representative colour: average of pixels weighted by saturation.
fn dominant_color(img: &RgbaImage) -> [f32; 3] {
    let (w, h) = img.dimensions();
    let step_x = (w / 32).max(1);
    let step_y = (h / 32).max(1);
    let (mut r, mut g, mut b, mut wsum) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let p = img.get_pixel(x, y).0;
            if p[3] > 128 {
                let (pr, pg, pb) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
                let mx = pr.max(pg).max(pb);
                let mn = pr.min(pg).min(pb);
                let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
                let weight = 0.05 + sat * sat * mx;
                r += pr * weight;
                g += pg * weight;
                b += pb * weight;
                wsum += weight;
            }
            x += step_x;
        }
        y += step_y;
    }
    if wsum <= 0.0 {
        return [0.2, 0.3, 0.5];
    }
    let mut c = [r / wsum, g / wsum, b / wsum];
    // Keep the result bright enough to glow on a dark background.
    let mx = c[0].max(c[1]).max(c[2]).max(0.001);
    if mx < 0.45 {
        let k = 0.45 / mx;
        c = [c[0] * k, c[1] * k, c[2] * k];
    }
    [c[0].min(1.0), c[1].min(1.0), c[2].min(1.0)]
}

fn fill_rect(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, c: [u8; 4]) {
    let a = c[3] as f32 / 255.0;
    for yy in y.max(0)..(y + h).min(img.height() as i32) {
        for xx in x.max(0)..(x + w).min(img.width() as i32) {
            let p = img.get_pixel_mut(xx as u32, yy as u32);
            for k in 0..3 {
                p.0[k] = (p.0[k] as f32 * (1.0 - a) + c[k] as f32 * a) as u8;
            }
            p.0[3] = 255;
        }
    }
}

/// Lays out a back cover: blurred cover art backdrop, title, info line, screenshots,
/// blurb, and a platform bar with a barcode, like the back of a retail box.
fn compose_back(req: &BackRequest, cover: Option<&RgbaImage>, shots: &[RgbaImage], font: Option<&fontdue::Font>) -> RgbaImage {
    let (w, h) = if req.aspect <= 1.0 { ((720.0 * req.aspect) as u32, 720u32) } else { (720u32, (720.0 / req.aspect) as u32) };
    let (w, h) = (w.max(64), h.max(64));
    let ac = req.accent;
    let mut img = match cover {
        Some(c) => backdrop(c, w, h, 0.32),
        None => {
            let mut i = RgbaImage::new(w, h);
            for (_, y, p) in i.enumerate_pixels_mut() {
                let k = 0.55 - 0.35 * y as f32 / h as f32;
                *p = Rgba([(ac[0] * 255.0 * k) as u8, (ac[1] * 255.0 * k) as u8, (ac[2] * 255.0 * k) as u8, 255]);
            }
            i
        }
    };
    // Darken towards the bottom so the text stays readable.
    for (_, y, p) in img.enumerate_pixels_mut() {
        let k = 1.0 - 0.35 * (y as f32 / h as f32);
        for c in 0..3 {
            p.0[c] = (p.0[c] as f32 * k) as u8;
        }
    }
    let wf = w as f32;
    let m = (wf * 0.07).round();
    let bar_h = (h as f32 * 0.085).max(24.0);
    let bottom = h as f32 - bar_h - m * 0.6;
    let mut y = m;

    if let Some(f) = font {
        // Title
        let tpx = (wf * 0.072).clamp(14.0, 60.0);
        for line in wrap_text(f, &req.title, tpx, wf - 2.0 * m).into_iter().take(2) {
            let line = fit_text(f, &line, tpx, wf - 2.0 * m);
            y += tpx;
            draw_text(&mut img, f, &line, tpx, m, y, [255, 255, 255]);
            y += tpx * 0.18;
        }
        if !req.info.is_empty() {
            let ipx = tpx * 0.48;
            y += ipx * 1.3;
            let s = fit_text(f, &req.info, ipx, wf - 2.0 * m);
            draw_text(&mut img, f, &s, ipx, m, y, [215, 215, 215]);
        }
        y += m * 0.6;
    }

    // Screenshots
    if !shots.is_empty() {
        let n = shots.len().min(2) as f32;
        let gap = m * 0.4;
        let sw = ((wf - 2.0 * m - gap * (n - 1.0)) / n).max(8.0);
        let sh = sw * 9.0 / 16.0;
        if y + sh < bottom - 40.0 {
            for (i, s) in shots.iter().take(2).enumerate() {
                let x = m + i as f32 * (sw + gap);
                let shot = cover_fill(s, sw as u32, sh as u32);
                fill_rect(&mut img, x as i32 - 2, y as i32 - 2, sw as i32 + 4, sh as i32 + 4, [255, 255, 255, 230]);
                imageops::overlay(&mut img, &shot, x as i64, y as i64);
            }
            y += sh + m * 0.6;
        }
    }

    if let Some(f) = font {
        // Blurb
        let dpx = (wf * 0.036).clamp(10.0, 26.0);
        let text = if req.description.is_empty() { format!("{} for {}.", req.title, req.platform) } else { req.description.clone() };
        let lines = wrap_text(f, &text, dpx, wf - 2.0 * m);
        let max_lines = ((bottom - y) / (dpx * 1.32)).floor().max(0.0) as usize;
        for (i, line) in lines.iter().take(max_lines).enumerate() {
            let last = i + 1 == max_lines && lines.len() > max_lines;
            let line = if last { fit_text(f, &format!("{line} ..."), dpx, wf - 2.0 * m) } else { line.clone() };
            y += dpx * 1.32;
            draw_text(&mut img, f, &line, dpx, m, y, [236, 236, 236]);
        }
    }

    // Platform bar + barcode
    let by = h as f32 - bar_h;
    fill_rect(&mut img, 0, by as i32, w as i32, bar_h as i32 + 1, [(ac[0] * 120.0) as u8 + 10, (ac[1] * 120.0) as u8 + 10, (ac[2] * 120.0) as u8 + 10, 245]);
    let bw = (wf * 0.24).max(30.0);
    let bx = wf - m - bw;
    let (bt, bh) = (by + bar_h * 0.16, bar_h * 0.68);
    fill_rect(&mut img, bx as i32, bt as i32, bw as i32, bh as i32, [250, 250, 250, 255]);
    let mut seed: u32 = req.id.bytes().fold(2166136261, |a, b| (a ^ b as u32).wrapping_mul(16777619));
    let mut x = bx + 4.0;
    while x < bx + bw - 5.0 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let lw = 1.0 + (seed % 3) as f32;
        if seed % 5 > 1 {
            fill_rect(&mut img, x as i32, (bt + 3.0) as i32, lw as i32, (bh - 6.0) as i32, [20, 20, 20, 255]);
        }
        x += lw + 1.0;
    }
    if let Some(f) = font {
        let ppx = (bar_h * 0.42).max(10.0);
        let s = fit_text(f, &req.platform.to_uppercase(), ppx, bx - m * 1.5);
        draw_text(&mut img, f, &s, ppx, m, by + bar_h * 0.5 + ppx * 0.36, [255, 255, 255]);
    }
    img
}

/// A stylised spine, laid out lying down (`l` along the spine, `t` across it); the caller
/// rotates it upright.
fn compose_spine(req: &SpineRequest, l: u32, t: u32, art: Option<&RgbaImage>, logo: Option<&RgbaImage>, font: Option<&fontdue::Font>) -> RgbaImage {
    let ac = req.accent;
    let mut img = match art {
        Some(a) => {
            let band = cover_fill(a, l / 4, (t / 4).max(4));
            let band = imageops::blur(&band, 1.2);
            let mut b = imageops::resize(&band, l, t, imageops::FilterType::Triangle);
            for p in b.pixels_mut() {
                for c in 0..3 {
                    p.0[c] = (p.0[c] as f32 * 0.5) as u8;
                }
                p.0[3] = 255;
            }
            b
        }
        None => {
            let mut b = RgbaImage::new(l, t);
            for (_, y, p) in b.enumerate_pixels_mut() {
                let k = 0.55 + 0.25 * (1.0 - ((y as f32 / t as f32) - 0.5).abs() * 2.0);
                *p = Rgba([(ac[0] * 255.0 * k) as u8, (ac[1] * 255.0 * k) as u8, (ac[2] * 255.0 * k) as u8, 255]);
            }
            b
        }
    };
    // Subtle bevel highlight along both long edges.
    let (tf, lf) = (t as f32, l as f32);
    fill_rect(&mut img, 0, 0, l as i32, (tf * 0.05).max(1.0) as i32, [255, 255, 255, 40]);
    fill_rect(&mut img, 0, (tf * 0.95) as i32, l as i32, (tf * 0.05).max(1.0) as i32 + 1, [0, 0, 0, 70]);
    // Platform band at the top of the spine.
    let band = (lf * 0.15) as i32;
    fill_rect(&mut img, 0, 0, band, t as i32, [12, 12, 14, 235]);
    fill_rect(&mut img, band, 0, (lf * 0.006).max(2.0) as i32, t as i32, [(ac[0] * 255.0) as u8, (ac[1] * 255.0) as u8, (ac[2] * 255.0) as u8, 255]);
    if let Some(f) = font {
        let px = (tf * 0.36).max(10.0);
        let s = fit_text(f, &req.platform.to_uppercase(), px, band as f32 - 16.0);
        draw_text(&mut img, f, &s, px, 10.0, tf * 0.5 + px * 0.36, [235, 235, 235]);
    }
    let x0 = band as f32 + lf * 0.03;
    let avail_w = lf - x0 - lf * 0.03;
    match logo {
        Some(lg) => {
            // Trim transparent margins so the logo uses the full height of the spine.
            let lg = trim_alpha(lg);
            let k = (avail_w / lg.width().max(1) as f32).min(tf * 0.8 / lg.height().max(1) as f32);
            let (w, h) = (((lg.width() as f32 * k) as u32).max(1), ((lg.height() as f32 * k) as u32).max(1));
            let r = imageops::resize(&lg, w, h, imageops::FilterType::CatmullRom);
            // Soft shadow so light logos stay readable on light art.
            let mut sh = r.clone();
            for p in sh.pixels_mut() {
                p.0 = [0, 0, 0, (p.0[3] as f32 * 0.6) as u8];
            }
            let sh = imageops::blur(&sh, 2.0);
            let (x, y) = ((x0 + (avail_w - w as f32) * 0.5) as i64, ((tf - h as f32) * 0.5) as i64);
            imageops::overlay(&mut img, &sh, x + 2, y + 2);
            imageops::overlay(&mut img, &r, x, y);
        }
        None => {
            if let Some(f) = font {
                let px = (tf * 0.5).max(12.0);
                let s = fit_text(f, &req.title, px, avail_w);
                let base = tf * 0.5 + px * 0.34;
                draw_text(&mut img, f, &s, px, x0 + 2.0, base + 2.0, [0, 0, 0]);
                draw_text(&mut img, f, &s, px, x0, base, [255, 255, 255]);
            }
        }
    }
    img
}

fn trim_alpha(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > 16 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 < x0 || y1 < y0 {
        return img.clone();
    }
    imageops::crop_imm(img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image()
}

// ------------------------------------------------------------------------------------------------
// Title matching
// ------------------------------------------------------------------------------------------------

fn fold_char(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        c => c,
    }
}

/// Lowercase words without punctuation, bracketed tags ("(USA)", "[!]"), "the",
/// trademark signs, and with roman numerals turned into digits.
pub fn normalize_title(s: &str) -> Vec<String> {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            '&' => out.push_str(" and "),
            '\'' | '’' => {}
            c => {
                let c = fold_char(c.to_lowercase().next().unwrap_or(c));
                out.push(if c.is_alphanumeric() { c } else { ' ' });
            }
        }
    }
    out.split_whitespace()
        .filter(|w| *w != "the")
        .map(|w| {
            match w {
                "ii" => "2",
                "iii" => "3",
                "iv" => "4",
                "vi" => "6",
                "vii" => "7",
                "viii" => "8",
                "ix" => "9",
                w => w,
            }
            .to_string()
        })
        .collect()
}

/// 0..1 similarity of two titles (token overlap).
pub fn similarity(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let sa: HashSet<&String> = a.iter().collect();
    let sb: HashSet<&String> = b.iter().collect();
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    (inter / union) * 0.95
}

/// Prefer common retail regions and clean dumps when several files match.
fn region_rank(name: &str) -> i32 {
    let mut r = if name.contains("(USA") {
        0
    } else if name.contains("(World") {
        1
    } else if name.contains("(Europe") {
        2
    } else if name.contains("(Japan") {
        4
    } else {
        3
    };
    for bad in ["(Beta", "(Proto", "(Demo", "(Sample", "(Kiosk", "(Pirate", "(Unl"] {
        if name.contains(bad) {
            r += 10;
        }
    }
    r
}

// ------------------------------------------------------------------------------------------------
// Sources
// ------------------------------------------------------------------------------------------------

fn url_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn url_unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).replace("&amp;", "&")
}

/// Steam's store API knows the (hashed) URLs of the current portrait art.
fn steam_assets(agent: &ureq::Agent, appid: u32) -> Option<HashMap<String, String>> {
    let input = format!(
        r#"{{"ids":[{{"appid":{appid}}}],"context":{{"language":"english","country_code":"US"}},"data_request":{{"include_assets":true}}}}"#
    );
    let v: serde_json::Value = agent
        .get(&format!("https://api.steampowered.com/IStoreBrowseService/GetItems/v1?input_json={}", url_escape(&input)))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let assets = v.pointer("/response/store_items/0/assets")?;
    let fmt = assets.get("asset_url_format")?.as_str()?;
    let mut out = HashMap::new();
    for k in ["library_capsule_2x", "library_capsule", "header", "library_hero"] {
        if let Some(f) = assets.get(k).and_then(|s| s.as_str()) {
            out.insert(k.to_string(), format!("https://shared.akamai.steamstatic.com/store_item_assets/{}", fmt.replace("${FILENAME}", f)));
        }
    }
    Some(out)
}

/// Finds a Steam app id for a PC game from another store (Epic, GOG, ...).
fn steam_search(agent: &ureq::Agent, title: &str) -> Option<u32> {
    let v: serde_json::Value = agent
        .get(&format!("https://store.steampowered.com/api/storesearch/?term={}&l=english&cc=US", url_escape(title)))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let want = normalize_title(title);
    v.get("items")?
        .as_array()?
        .iter()
        .filter_map(|it| Some((it.get("id")?.as_u64()? as u32, similarity(&want, &normalize_title(it.get("name")?.as_str()?)))))
        .filter(|(_, s)| *s >= 0.75)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// The libretro thumbnail index for a system (file names without ".png"), cached on disk
/// for two weeks and in memory for the session.
fn libretro_index(agent: &ureq::Agent, cache: &Path, system: &str, kind: &str) -> Arc<Vec<String>> {
    static MEM: OnceLock<Mutex<HashMap<String, Arc<Vec<String>>>>> = OnceLock::new();
    let mem = MEM.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{system}/{kind}");
    if let Some(v) = mem.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return v;
    }
    let dir = cache.parent().unwrap_or(cache).join("libretro");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join(format!("{}.txt", safe_name(&key)));
    let fresh = age(&file).is_some_and(|a| a < Duration::from_secs(14 * 86400));
    let mut names: Vec<String> = if fresh { std::fs::read_to_string(&file).unwrap_or_default().lines().map(str::to_owned).collect() } else { vec![] };
    if names.is_empty() {
        let url = format!("https://thumbnails.libretro.com/{}/{}/", url_escape(system), kind);
        if let Some(html) = agent.get(&url).call().ok().and_then(|r| r.into_string().ok()) {
            names = html
                .split("href=\"")
                .skip(1)
                .filter_map(|s| s.split('"').next())
                .filter(|s| s.ends_with(".png"))
                .map(|s| url_unescape(s.trim_end_matches(".png")))
                .collect();
            if !names.is_empty() {
                let _ = std::fs::write(&file, names.join("\n"));
            }
        }
        if names.is_empty() {
            // Offline: fall back to a stale copy.
            names = std::fs::read_to_string(&file).unwrap_or_default().lines().map(str::to_owned).collect();
        }
    }
    let v = Arc::new(names);
    if let Ok(mut m) = mem.lock() {
        m.insert(key, v.clone());
    }
    v
}

/// libretro replaces these characters with '_' in thumbnail file names.
fn libretro_name(s: &str) -> String {
    s.chars().map(|c| if "&*/:`<>?\\|\"".contains(c) { '_' } else { c }).collect()
}

fn libretro_urls(agent: &ureq::Agent, cache: &Path, req: &CoverRequest, kind: &str) -> Vec<String> {
    let Some(sys) = &req.libretro_system else { return vec![] };
    let index = libretro_index(agent, cache, sys, kind);
    let mut names: Vec<String> = Vec::new();
    let stem = req.path.as_ref().and_then(|p| Path::new(p).file_stem().map(|s| libretro_name(&s.to_string_lossy())));
    if let Some(stem) = &stem {
        if index.is_empty() || index.iter().any(|n| n == stem) {
            names.push(stem.clone());
        }
    }
    if !index.is_empty() {
        // Fuzzy match the title (and the file name) against every thumbnail.
        let mut wants = vec![normalize_title(&req.title)];
        if let Some(stem) = &stem {
            wants.push(normalize_title(stem));
        }
        let mut scored: Vec<(f32, i32, &String)> = index
            .iter()
            .filter_map(|n| {
                let t = normalize_title(n);
                let s = wants.iter().map(|w| similarity(w, &t)).fold(0.0, f32::max);
                (s >= 0.7).then(|| (s, region_rank(n), n))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.len().cmp(&b.2.len())));
        names.extend(scored.into_iter().take(3).map(|(_, _, n)| n.clone()));
    } else {
        // No index (offline): guess No-Intro style names.
        let t = libretro_name(&req.title);
        for suffix in [" (USA)", " (World)", " (Europe)", " (USA, Europe)", " (Japan)", ""] {
            names.push(format!("{t}{suffix}"));
        }
    }
    let mut seen = HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    names.into_iter().map(|n| format!("https://thumbnails.libretro.com/{}/{}/{}.png", url_escape(sys), kind, url_escape(&n))).collect()
}

fn steamgriddb_urls(agent: &ureq::Agent, key: &str, req: &CoverRequest) -> Vec<String> {
    if key.trim().is_empty() {
        return vec![];
    }
    let auth = format!("Bearer {}", key.trim());
    let get = |url: &str| agent.get(url).set("Authorization", &auth).call().ok().and_then(|r| r.into_json::<serde_json::Value>().ok());
    let game_id: Option<u64> = if let Some(app) = req.steam_appid {
        get(&format!("https://www.steamgriddb.com/api/v2/games/steam/{app}")).and_then(|v| v.pointer("/data/id").and_then(|i| i.as_u64()))
    } else {
        get(&format!("https://www.steamgriddb.com/api/v2/search/autocomplete/{}", url_escape(&req.title)))
            .and_then(|v| v.pointer("/data/0/id").and_then(|i| i.as_u64()))
    };
    let Some(gid) = game_id else { return vec![] };
    let grids = |q: &str| -> Vec<String> {
        get(&format!("https://www.steamgriddb.com/api/v2/grids/game/{gid}?{q}"))
            .and_then(|v| v.get("data").and_then(|d| d.as_array()).cloned())
            .map(|a| a.iter().take(2).filter_map(|g| g.get("url").and_then(|u| u.as_str()).map(str::to_owned)).collect())
            .unwrap_or_default()
    };
    let mut out = grids("dimensions=600x900,342x482,660x930&types=static");
    if out.is_empty() {
        out = grids("types=static");
    }
    out
}

fn thegamesdb_urls(agent: &ureq::Agent, key: &str, title: &str) -> Vec<String> {
    if key.trim().is_empty() {
        return vec![];
    }
    let Some(v) = agent
        .get(&format!("https://api.thegamesdb.net/v1/Games/ByGameName?apikey={}&name={}&include=boxart", url_escape(key.trim()), url_escape(title)))
        .call()
        .ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
    else {
        return vec![];
    };
    let want = normalize_title(title);
    let Some(games) = v.pointer("/data/games").and_then(|g| g.as_array()) else { return vec![] };
    let Some(best) = games
        .iter()
        .filter_map(|g| Some((g.get("id")?.as_u64()?, similarity(&want, &normalize_title(g.get("game_title")?.as_str()?)))))
        .filter(|(_, s)| *s >= 0.7)
        .max_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return vec![];
    };
    let base = v.pointer("/include/boxart/base_url/large").and_then(|s| s.as_str()).unwrap_or("https://cdn.thegamesdb.net/images/large/");
    v.pointer(&format!("/include/boxart/data/{}", best.0))
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter(|b| b.get("side").and_then(|s| s.as_str()) == Some("front"))
                .filter_map(|b| b.get("filename").and_then(|f| f.as_str()).map(|f| format!("{base}{f}")))
                .collect()
        })
        .unwrap_or_default()
}

struct WikiPage {
    title: String,
    image: Option<String>,
}

/// Finds the Wikipedia article about a video game; its infobox image is usually the box art.
fn wikipedia_lookup(agent: &ureq::Agent, title: &str) -> Option<WikiPage> {
    static MEM: OnceLock<Mutex<HashMap<String, Option<(String, Option<String>)>>>> = OnceLock::new();
    let mem = MEM.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = mem.lock().ok().and_then(|m| m.get(title).cloned()) {
        return hit.map(|(title, image)| WikiPage { title, image });
    }
    let want = normalize_title(title);
    let q = format!("{title} video game");
    let found = agent
        .get(&format!(
            "https://en.wikipedia.org/w/api.php?action=query&format=json&generator=search&gsrsearch={}&gsrlimit=6&prop=pageimages|description&piprop=thumbnail&pithumbsize=720&pilicense=any",
            url_escape(&q)
        ))
        .call()
        .ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
        .and_then(|v| {
            let pages = v.pointer("/query/pages")?.as_object()?.clone();
            pages
                .values()
                .filter_map(|p| {
                    let t = p.get("title")?.as_str()?.to_string();
                    let desc = p.get("description").and_then(|d| d.as_str()).unwrap_or("").to_lowercase();
                    if !desc.contains("game") {
                        return None;
                    }
                    let sim = similarity(&want, &normalize_title(&t));
                    let idx = p.get("index").and_then(|i| i.as_i64()).unwrap_or(99);
                    let img = p.pointer("/thumbnail/source").and_then(|s| s.as_str()).map(str::to_owned);
                    (sim >= 0.6).then_some((sim, -idx, t, img))
                })
                .max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, _, t, img)| (t, img))
        });
    if let Ok(mut m) = mem.lock() {
        m.insert(title.to_string(), found.clone());
    }
    found.map(|(title, image)| WikiPage { title, image })
}

fn fetch_steam_meta(agent: &ureq::Agent, app: u32, id: &str) -> Option<MetaReady> {
    let v: serde_json::Value = agent
        .get(&format!("https://store.steampowered.com/api/appdetails?appids={app}&l=english&filters=basic,genres,release_date"))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let d = v.get(app.to_string())?.get("data")?;
    let description = d.get("short_description").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let genres = d
        .get("genres")
        .and_then(|g| g.as_array())
        .map(|a| a.iter().filter_map(|x| x.get("description").and_then(|s| s.as_str()).map(str::to_owned)).collect())
        .unwrap_or_default();
    let date = d.pointer("/release_date/date").and_then(|s| s.as_str()).unwrap_or("");
    let year = date.rsplit(' ').next().unwrap_or("").to_string();
    Some(MetaReady { id: id.to_string(), description, genres, year })
}

#[cfg(test)]
mod cart_tests {
    use super::*;

    /// A yellow cartridge with a blue label in the middle, on transparent or white.
    fn cart(bg: Rgba<u8>) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(200, 240, bg);
        for y in 20..220 {
            for x in 30..170 {
                img.put_pixel(x, y, Rgba([242, 196, 24, 255]));
            }
        }
        for y in 60..180 {
            for x in 50..150 {
                img.put_pixel(x, y, Rgba([30, 60, 200, 255]));
            }
        }
        img
    }

    #[test]
    fn measures_the_shell_not_the_label() {
        for bg in [Rgba([0, 0, 0, 0]), Rgba([255, 255, 255, 255])] {
            let c = shell_color(&cart(bg)).expect("colour");
            assert!((c[0] - 242.0 / 255.0).abs() < 0.02 && (c[1] - 196.0 / 255.0).abs() < 0.02 && (c[2] - 24.0 / 255.0).abs() < 0.02, "{c:?}");
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Art chooser: every image the sources have for a game
// ------------------------------------------------------------------------------------------------

/// What the art chooser needs to know about a game.
#[derive(Clone)]
pub struct ArtSource {
    pub id: String,
    pub title: String,
    pub steam_appid: Option<u32>,
    pub cover_path: Option<String>,
    pub libretro_system: Option<String>,
    pub path: Option<String>,
    pub sgdb_key: String,
}

impl ArtSource {
    pub fn from_game(g: &Game, cfg: &crate::config::Config) -> Self {
        ArtSource {
            id: g.id.clone(),
            title: g.title.clone(),
            steam_appid: g.steam_appid,
            cover_path: g.cover_path.clone(),
            libretro_system: libretro_system(g, cfg),
            path: g.path.clone(),
            sgdb_key: cfg.covers.steamgriddb_api_key.clone(),
        }
    }
}

/// One image on offer: what kind of art it is (for ordering), a label, and where it is (a URL
/// or a local file).
#[derive(Clone, Debug)]
pub struct ArtCandidate {
    pub key: &'static str,
    pub label: String,
    pub src: String,
}

/// Everything the art sources have for a game: LaunchBox scans, Steam's art (the local Steam
/// cache and the store), SteamGridDB grids (with a key), libretro thumbnails, the current cover.
pub fn art_candidates(src: &ArtSource) -> Vec<ArtCandidate> {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(6)).timeout(Duration::from_secs(20)).user_agent("slipcase/0.1").build();
    let mut out: Vec<ArtCandidate> = Vec::new();
    let mut add = |key: &'static str, label: &str, src: String| {
        if !out.iter().any(|c| c.src == src) {
            out.push(ArtCandidate { key, label: label.to_string(), src });
        }
    };
    // The cover in use now.
    let current = src.cover_path.clone().or_else(|| {
        let f = cache_file(&cache_dir().join("covers"), &src.id);
        f.is_file().then(|| f.to_string_lossy().into_owned())
    });
    if let Some(c) = current {
        add("current", "Current cover", c);
    }
    // What each other surface shows now (made automatically or picked before).
    for (kind, key) in [(ArtKind::Media, "made_media"), (ArtKind::Insert, "made_insert"), (ArtKind::Back, "made_back"), (ArtKind::Spine, "made_spine")] {
        let f = made_file(&src.id, kind);
        if f.is_file() {
            add(key, "In use now", f.to_string_lossy().into_owned());
        }
    }
    if let Some(a) = crate::launchbox::assets(&src.id) {
        let lb = |f: &Option<String>| f.as_ref().map(|f| crate::launchbox::url(f));
        if let Some(u) = lb(&a.front) {
            add("front", "LaunchBox: box front", u);
        }
        if let Some(u) = lb(&a.disc) {
            add("disc", "LaunchBox: disc", u);
        }
        if let Some(u) = lb(&a.cart) {
            add("cart", "LaunchBox: cartridge", u);
        }
        if let Some(u) = lb(&a.back) {
            add("back", "LaunchBox: box back", u);
        }
        if let Some(u) = lb(&a.spine) {
            add("spine", "LaunchBox: spine", u);
        }
        if let Some(u) = lb(&a.fanart) {
            add("fanart", "LaunchBox: fanart", u);
        }
        for (i, s) in a.screenshots.iter().enumerate() {
            add("shot", &format!("LaunchBox: screenshot {}", i + 1), crate::launchbox::url(s));
        }
        if let Some(u) = lb(&a.logo) {
            add("logo", "LaunchBox: logo", u);
        }
    }
    if let Some(app) = src.steam_appid {
        // What the Steam client has cached locally (works offline).
        let mut local: HashSet<&'static str> = HashSet::new();
        for root in crate::scanner::steam_roots() {
            let mut files = Vec::new();
            collect_files(&root.join("appcache/librarycache").join(app.to_string()), 2, &mut files);
            files.sort();
            for f in files {
                let name = f.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                if name.contains("blur") {
                    continue;
                }
                let key = if name.starts_with("library_600x900") || name.starts_with("library_capsule") {
                    "grid"
                } else if name.starts_with("library_hero") {
                    "hero"
                } else if name.starts_with("logo") {
                    "logo"
                } else if name.starts_with("header") {
                    "header"
                } else {
                    continue;
                };
                local.insert(key);
                add(key, &format!("Steam: {}", name.split('.').next().unwrap_or("art").replace('_', " ")), f.to_string_lossy().into_owned());
            }
        }
        // From the store, where the local cache does not have it.
        let cdn = |f: &str| format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app}/{f}");
        for (key, label, file) in [("grid", "Steam: library cover", "library_600x900_2x.jpg"), ("hero", "Steam: hero", "library_hero.jpg"), ("header", "Steam: header", "header.jpg"), ("logo", "Steam: logo", "logo.png")] {
            if !local.contains(key) {
                add(key, label, cdn(file));
            }
        }
        if let Some(v) = agent
            .get(&format!("https://store.steampowered.com/api/appdetails?appids={app}&filters=screenshots"))
            .call()
            .ok()
            .and_then(|r| r.into_json::<serde_json::Value>().ok())
        {
            if let Some(a) = v.pointer(&format!("/{app}/data/screenshots")).and_then(|a| a.as_array()) {
                for (i, s) in a.iter().take(8).enumerate() {
                    if let Some(u) = s.get("path_full").and_then(|u| u.as_str()) {
                        add("shot", &format!("Steam: screenshot {}", i + 1), u.to_string());
                    }
                }
            }
        }
    }
    let cr = CoverRequest {
        id: src.id.clone(),
        title: src.title.clone(),
        steam_appid: src.steam_appid,
        path: src.path.clone(),
        cover_path: None,
        libretro_system: src.libretro_system.clone(),
        is_pc: src.libretro_system.is_none(),
        aspect: 0.7,
        want_metadata: false,
    };
    for (i, u) in steamgriddb_urls(&agent, &src.sgdb_key, &cr).into_iter().enumerate() {
        add("grid", &format!("SteamGridDB: grid {}", i + 1), u);
    }
    if src.libretro_system.is_some() {
        let cache = cache_dir().join("covers");
        for (kind, key, label) in [("Named_Boxarts", "front", "libretro: box art"), ("Named_Snaps", "shot", "libretro: screenshot"), ("Named_Titles", "shot", "libretro: title screen")] {
            for u in libretro_urls(&agent, &cache, &cr, kind).into_iter().take(1) {
                add(key, label, u);
            }
        }
    }
    out
}

/// Where the art made for a game's surface (disc, insert, back, spine) is kept, so the art
/// chooser can offer it.
fn made_file(id: &str, kind: ArtKind) -> PathBuf {
    cache_dir().join("made").join(format!("{}_{kind:?}.png", safe_name(id)).to_lowercase())
}

fn keep_made(c: &CoverReady, kind: ArtKind) {
    let f = made_file(&c.id, kind);
    let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
    let _ = image::save_buffer(&f, &c.rgba, c.width, c.height, image::ColorType::Rgba8);
}

/// Fetches a candidate (cached on disk by its address, so choosing it later needs no second
/// download) and returns the file and a small thumbnail (width, height, RGBA).
pub fn art_thumbnail(src: &str) -> Option<(PathBuf, u32, u32, Vec<u8>)> {
    let file = if src.starts_with("http") {
        let dir = cache_dir().join("artpick");
        let _ = std::fs::create_dir_all(&dir);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&src, &mut h);
        let f = dir.join(format!("{:016x}.img", std::hash::Hasher::finish(&h)));
        if !f.is_file() {
            let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(6)).timeout(Duration::from_secs(20)).user_agent("slipcase/0.1").build();
            let bytes = http_bytes(&agent, src, 16 * 1024 * 1024)?;
            image::load_from_memory(&bytes).ok()?;
            std::fs::write(&f, &bytes).ok()?;
        }
        f
    } else {
        PathBuf::from(src)
    };
    let img = image::load_from_memory(&std::fs::read(&file).ok()?).ok()?.to_rgba8();
    let k = 200.0 / img.width().max(img.height()).max(1) as f32;
    let t = imageops::resize(&img, ((img.width() as f32 * k) as u32).max(1), ((img.height() as f32 * k) as u32).max(1), imageops::FilterType::Triangle);
    let (w, h) = t.dimensions();
    Some((file, w, h, t.into_raw()))
}

// ------------------------------------------------------------------------------------------------
// Insert pages
// ------------------------------------------------------------------------------------------------

/// The printed pages of a game's insert (both sides of every sheet after the cover).
#[derive(Clone)]
pub struct PageRequest {
    pub game: BackRequest,
    /// width / height of a page.
    pub aspect: f32,
    /// How many faces to make: the inside of the cover, then front and back of each sheet.
    pub faces: usize,
}

pub struct PagesReady {
    pub id: String,
    pub faces: Vec<CoverReady>,
}

/// Text found for the pages.
struct PageText {
    overview: String,
    rows: Vec<(&'static str, String)>,
}

/// 1234567 -> "1,234,567".
fn group_digits(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Strips HTML tags and entities from Steam's store text.
fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for c in s.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let t = tag.to_lowercase();
                if t.starts_with("br") || t.starts_with("/p") || t.starts_with("/h") || t.starts_with("li") {
                    out.push('\n');
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    let out = out.replace("&quot;", "\"").replace("&amp;", "&").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

impl Worker {
    /// Steam's full store entry for an app (cached on disk).
    fn steam_details(&self, app: u32) -> Option<serde_json::Value> {
        let dir = cache_dir().join("steamdetails");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join(format!("{app}.json"));
        if let Some(v) = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
            return Some(v);
        }
        if !self.settings.enabled {
            return None;
        }
        let v: serde_json::Value = self.agent.get(&format!("https://store.steampowered.com/api/appdetails?appids={app}")).call().ok()?.into_json().ok()?;
        let data = v.pointer(&format!("/{app}/data"))?.clone();
        let _ = std::fs::write(&f, serde_json::to_vec(&data).unwrap_or_default());
        Some(data)
    }

    /// Players in the game on Steam at this moment.
    fn players_now(&self, app: u32) -> Option<u64> {
        let v: serde_json::Value = self
            .agent
            .get(&format!("https://api.steampowered.com/ISteamUserStats/GetNumberOfCurrentPlayers/v1/?appid={app}"))
            .call()
            .ok()?
            .into_json()
            .ok()?;
        (v.pointer("/response/result").and_then(|r| r.as_u64()) == Some(1)).then(|| v.pointer("/response/player_count").and_then(|c| c.as_u64())).flatten()
    }

    fn page_text(&self, req: &BackRequest) -> PageText {
        let lb = self.lb(&req.id).map(|a| a.info).unwrap_or_default();
        let steam = req.steam_appid.and_then(|a| self.steam_details(a));
        let sv = |p: &str| steam.as_ref().and_then(|v| v.pointer(p)).and_then(|x| x.as_str()).map(str::to_owned).unwrap_or_default();
        let list = |k: &str| {
            steam
                .as_ref()
                .and_then(|v| v.get(k))
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().or_else(|| x.get("description").and_then(|d| d.as_str()))).collect::<Vec<_>>().join(", "))
                .unwrap_or_default()
        };
        let pick = |a: String, b: String| if a.trim().is_empty() { b } else { a };
        let overview = pick(lb.overview.clone(), pick(strip_html(&sv("/about_the_game")), pick(req.description.clone(), strip_html(&sv("/short_description")))));
        let mut rows: Vec<(&'static str, String)> = vec![
            ("Platform", req.platform.clone()),
            ("Released", pick(lb.released.clone(), sv("/release_date/date"))),
            ("Developer", pick(lb.developer.clone(), list("developers"))),
            ("Publisher", pick(lb.publisher.clone(), list("publishers"))),
            ("Genres", pick(lb.genres.clone(), list("genres"))),
            ("Players", lb.players.clone()),
        ];
        if let Ok(r) = lb.rating.parse::<f32>() {
            let stars = (r.round() as usize).min(5);
            rows.push(("Rating", format!("{}{}  {r:.1} / 5", "\u{2605}".repeat(stars), "\u{2606}".repeat(5 - stars))));
        }
        if let Some(m) = steam.as_ref().and_then(|v| v.pointer("/metacritic/score")).and_then(|s| s.as_u64()) {
            rows.push(("Metacritic", m.to_string()));
        }
        // How many are playing right now (Steam games; asked each time the pages are made).
        if let Some(n) = req.steam_appid.filter(|_| self.settings.enabled).and_then(|app| self.players_now(app)) {
            rows.push(("Playing now", format!("{} on Steam", group_digits(n))));
        }
        rows.retain(|(_, v)| !v.trim().is_empty());
        PageText { overview, rows }
    }

    /// Screenshots for the pages: LaunchBox's, then Steam's (cached on disk).
    fn page_shots(&self, req: &BackRequest, want: usize) -> Vec<RgbaImage> {
        let mut out = Vec::new();
        if let Some(a) = self.lb(&req.id) {
            for (n, f) in a.screenshots.iter().enumerate() {
                if out.len() >= want {
                    return out;
                }
                if let Some(img) = self.cached_image("pageshots", &format!("{}_lb{n}", req.id), &crate::launchbox::url(f)) {
                    out.push(img.to_rgba8());
                }
            }
        }
        if let Some(v) = req.steam_appid.and_then(|a| self.steam_details(a)) {
            if let Some(list) = v.get("screenshots").and_then(|a| a.as_array()) {
                for (n, s) in list.iter().enumerate() {
                    if out.len() >= want {
                        break;
                    }
                    if let Some(u) = s.get("path_full").and_then(|u| u.as_str()) {
                        if let Some(img) = self.cached_image("pageshots", &format!("{}_st{n}", req.id), u) {
                            out.push(img.to_rgba8());
                        }
                    }
                }
            }
        }
        if out.is_empty() {
            out = self.screenshots(req);
        }
        out
    }

    fn make_pages(&self, req: &PageRequest) -> PagesReady {
        let g = &req.game;
        let text = self.page_text(g);
        let shots = self.page_shots(g, 6);
        let wide = self.fanart(&g.id).or_else(|| self.hero(&g.id, g.steam_appid));
        let logo = self.logo(&g.id, g.steam_appid);
        let font = (*self.font).as_ref();
        let h = 900u32;
        let w = ((h as f32 * req.aspect) as u32).max(200);
        let ac = g.accent;
        let accent = [(ac[0] * 200.0 + 30.0).min(255.0) as u8, (ac[1] * 200.0 + 30.0).min(255.0) as u8, (ac[2] * 200.0 + 30.0).min(255.0) as u8];
        let ink = [34u8, 34, 38];
        let m = w as f32 * 0.08;
        let paper = || {
            // Slightly grey, like paper rather than a lit screen.
            let mut i = RgbaImage::from_pixel(w, h, Rgba([224, 221, 213, 255]));
            // A faint grain so it reads as paper.
            for (x, y, p) in i.enumerate_pixels_mut() {
                let n = ((x.wrapping_mul(73) ^ y.wrapping_mul(151)) % 7) as u8;
                p.0[0] -= n;
                p.0[1] -= n;
                p.0[2] -= n;
            }
            i
        };
        let folio = |img: &mut RgbaImage, n: usize| {
            if let Some(f) = font {
                let s = format!("- {n} -");
                let px = 18.0;
                let tw = crate::render::text_width(f, &s, px);
                draw_text(img, f, &s, px, (w as f32 - tw) * 0.5, h as f32 - m * 0.5, [120, 120, 120]);
            }
        };
        let heading = |img: &mut RgbaImage, s: &str, y: f32| -> f32 {
            let Some(f) = font else { return y };
            let px = (w as f32 * 0.075).clamp(20.0, 48.0);
            let s = fit_text(f, s, px, w as f32 - 2.0 * m);
            draw_text(img, f, &s, px, m, y + px, accent);
            fill_rect(img, m as i32, (y + px * 1.35) as i32, (w as f32 - 2.0 * m) as i32, 3, [accent[0], accent[1], accent[2], 255]);
            y + px * 1.9
        };
        // Lines of the overview at body size, shared between the pages that show it.
        let body_px = (w as f32 * 0.04).clamp(13.0, 24.0);
        let mut lines: Vec<String> = Vec::new();
        if let Some(f) = font {
            for para in text.overview.split('\n') {
                lines.extend(wrap_text(f, para, body_px, w as f32 - 2.0 * m));
                lines.push(String::new());
            }
        }
        let next_line = std::cell::Cell::new(0usize);
        let body = |img: &mut RgbaImage, mut y: f32| {
            let Some(f) = font else { return };
            let bottom = h as f32 - m * 1.1;
            while next_line.get() < lines.len() && y + body_px * 1.4 < bottom {
                y += body_px * 1.4;
                draw_text(img, f, &lines[next_line.get()], body_px, m, y, ink);
                next_line.set(next_line.get() + 1);
            }
        };
        let image_block = |img: &mut RgbaImage, art: &RgbaImage, y: f32, bh: f32| {
            let bw = w as f32 - 2.0 * m;
            let pic = cover_fill(art, bw as u32, bh.max(8.0) as u32);
            fill_rect(img, m as i32 - 3, y as i32 - 3, bw as i32 + 6, bh as i32 + 6, [30, 30, 30, 255]);
            imageops::overlay(img, &pic, m as i64, y as i64);
        };
        let mut shot = {
            let mut i = 0usize;
            let shots = &shots;
            let fallback = wide.clone();
            // The screenshots in turn, then the wide art, then round again.
            move || -> Option<RgbaImage> {
                let n = shots.len() + usize::from(fallback.is_some());
                if n == 0 {
                    return None;
                }
                let k = i % n;
                i += 1;
                shots.get(k).cloned().or_else(|| fallback.clone())
            }
        };

        let mut faces: Vec<RgbaImage> = Vec::new();
        // 1. Inside the cover: about the game.
        let mut p = paper();
        let y = heading(&mut p, "About the game", m);
        body(&mut p, y);
        folio(&mut p, 1);
        faces.push(p);
        // 2. The facts, under the logo.
        let mut p = paper();
        let mut y = m;
        if let Some(l) = &logo {
            let l = trim_alpha(l);
            let k = ((w as f32 - 2.0 * m) / l.width().max(1) as f32).min(h as f32 * 0.16 / l.height().max(1) as f32);
            let (lw, lh) = (((l.width() as f32 * k) as u32).max(1), ((l.height() as f32 * k) as u32).max(1));
            // Light logos would vanish on paper: put them on a dark band.
            fill_rect(&mut p, 0, (y - m * 0.4) as i32, w as i32, (lh as f32 + m * 0.8) as i32, [28, 28, 34, 255]);
            imageops::overlay(&mut p, &imageops::resize(&l, lw, lh, imageops::FilterType::Triangle), ((w - lw) / 2) as i64, y as i64);
            y += lh as f32 + m;
        } else {
            y = heading(&mut p, &g.title, y);
        }
        if let Some(f) = font {
            let px = (w as f32 * 0.042).clamp(13.0, 26.0);
            for (k, v) in &text.rows {
                y += px * 1.7;
                draw_text(&mut p, f, k, px * 0.8, m, y, [130, 130, 130]);
                let col_w = w as f32 - 2.0 * m - w as f32 * 0.3;
                let lines = wrap_text(f, v, px, col_w);
                for (i, line) in lines.iter().take(2).enumerate() {
                    if i > 0 {
                        y += px * 1.25;
                    }
                    // More than two lines: end the second with an ellipsis.
                    let line = if i == 1 && lines.len() > 2 { fit_text(f, &format!("{} ...", line.trim_end_matches(',')), px, col_w) } else { line.clone() };
                    draw_text(&mut p, f, &line, px, m + w as f32 * 0.3, y, ink);
                }
            }
        }
        folio(&mut p, 2);
        faces.push(p);
        // 3. A screenshot, full page height.
        let mut p = paper();
        let y = heading(&mut p, "Screenshots", m);
        if let Some(s) = shot() {
            image_block(&mut p, &s, y, (h as f32 - y - m * 1.4) * 0.5 - m * 0.2);
        }
        if let Some(s) = shot() {
            let top = y + (h as f32 - y - m * 1.4) * 0.5 + m * 0.2;
            image_block(&mut p, &s, top, (h as f32 - y - m * 1.4) * 0.5 - m * 0.2);
        }
        folio(&mut p, 3);
        faces.push(p);
        // 4. Art across the whole page, with the logo.
        let mut p = match &wide {
            Some(a) => cover_fill(a, w, h),
            None => paper(),
        };
        if let Some(l) = &logo {
            let l = trim_alpha(l);
            let k = ((w as f32 * 0.8) / l.width().max(1) as f32).min(h as f32 * 0.14 / l.height().max(1) as f32);
            let (lw, lh) = (((l.width() as f32 * k) as u32).max(1), ((l.height() as f32 * k) as u32).max(1));
            imageops::overlay(&mut p, &imageops::resize(&l, lw, lh, imageops::FilterType::Triangle), ((w - lw) / 2) as i64, (h as f32 * 0.78) as i64);
        }
        faces.push(p);
        // 5. The story continues (or more screenshots when it is short).
        let mut p = paper();
        if next_line.get() < lines.len() {
            body(&mut p, m * 0.4);
        } else {
            let y = heading(&mut p, "In the game", m);
            if let Some(s) = shot() {
                image_block(&mut p, &s, y, (h as f32 - y - m * 1.4) * 0.6);
            }
        }
        folio(&mut p, 5);
        faces.push(p);
        // 6 - 7. More screenshots.
        for n in [6, 7] {
            let mut p = paper();
            let top = m;
            let each = (h as f32 - top - m * 1.6) / 2.0;
            for k in 0..2 {
                if let Some(s) = shot() {
                    image_block(&mut p, &s, top + k as f32 * (each + m * 0.3), each - m * 0.15);
                }
            }
            folio(&mut p, n);
            faces.push(p);
        }
        // 8. Credits.
        let mut p = paper();
        let mut y = heading(&mut p, "Credits", m);
        if let Some(f) = font {
            let px = (w as f32 * 0.045).clamp(13.0, 26.0);
            for (k, v) in text.rows.iter().filter(|(k, _)| matches!(*k, "Developer" | "Publisher" | "Released")) {
                y += px * 2.0;
                draw_text(&mut p, f, k, px * 0.8, m, y, [130, 130, 130]);
                y += px * 1.3;
                let v = fit_text(f, v, px, w as f32 - 2.0 * m);
                draw_text(&mut p, f, &v, px, m, y, ink);
            }
            y += px * 3.0;
            let t = fit_text(f, &g.title, px, w as f32 - 2.0 * m);
            draw_text(&mut p, f, &t, px, m, y, accent);
        }
        folio(&mut p, 8);
        faces.push(p);
        // 9. Notes: ruled lines.
        let mut p = paper();
        let y = heading(&mut p, "Notes", m);
        let mut ly = y + m * 0.6;
        while ly < h as f32 - m * 1.2 {
            fill_rect(&mut p, m as i32, ly as i32, (w as f32 - 2.0 * m) as i32, 1, [170, 180, 200, 255]);
            ly += body_px * 1.8;
        }
        folio(&mut p, 9);
        faces.push(p);

        faces.truncate(req.faces);
        PagesReady {
            id: g.id.clone(),
            faces: faces
                .into_iter()
                .map(|img| {
                    let (w, h) = img.dimensions();
                    CoverReady { id: g.id.clone(), width: w, height: h, rgba: img.into_raw(), dominant: [0.0; 3] }
                })
                .collect(),
        }
    }
}
