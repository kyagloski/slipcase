//! Settings presets: whole sets of settings, kept as `presets/<name>.toml` next to the config.
//! The settings in use belong to the preset named in `general.preset`; switching saves them
//! there first, then puts the other preset's settings in place. The library side of the config
//! (scanned stores and folders, hidden games) and the last selected game are shared by all.

use std::path::PathBuf;

use crate::config::{config_dir, Config, ConfigData};

pub fn dir() -> PathBuf {
    let d = config_dir().join("presets");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// A name that is safe as a file name (the characters files cannot have taken out).
pub fn clean(name: &str) -> String {
    let s: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    s.trim().trim_matches('.').trim().chars().take(40).collect()
}

fn path(name: &str) -> PathBuf {
    dir().join(format!("{}.toml", clean(name)))
}

/// Every saved preset, and the one in use (saved or not), by name.
pub fn list(current: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "toml")).then(|| p.file_stem().map(|s| s.to_string_lossy().into_owned())).flatten()
        })
        .collect();
    if !v.iter().any(|n| n.eq_ignore_ascii_case(current)) {
        v.push(current.to_string());
    }
    v.sort_by_key(|n| n.to_lowercase());
    v
}

pub fn exists(name: &str) -> bool {
    path(name).exists()
}

pub fn save(name: &str, data: &ConfigData) {
    if let Ok(text) = toml::to_string_pretty(data) {
        let _ = std::fs::write(path(name), text);
    }
}

pub fn delete(name: &str) {
    let _ = std::fs::remove_file(path(name));
}

pub fn rename(from: &str, to: &str) {
    let (a, b) = (path(from), path(to));
    if a.exists() {
        let _ = std::fs::rename(a, b);
    }
}

/// Saves the settings in use to their preset (unless `keep_current` is false, for a deleted
/// one), then puts `to`'s settings in place and writes the config. False if `to` has no file.
pub fn switch(cfg: &mut Config, to: &str, keep_current: bool) -> bool {
    let Some(mut next) = std::fs::read_to_string(path(to)).ok().and_then(|t| toml::from_str::<ConfigData>(&t).ok()) else { return false };
    if keep_current {
        save(&cfg.general.preset, &cfg.data);
    }
    // Shared by every preset: the library and where it comes from.
    next.scan = cfg.data.scan.clone();
    next.general.last_game = cfg.data.general.last_game.clone();
    next.general.preset = to.to_string();
    cfg.data = next;
    cfg.save();
    true
}
