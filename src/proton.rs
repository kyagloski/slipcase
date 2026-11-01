//! Running Windows programs on Linux through Proton: finds the Proton builds Steam knows
//! about and builds the command that starts a game in its own prefix.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `Game::proton` value that runs the program directly instead.
pub const OFF: &str = "off";

#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// Folder name, which is what the settings store.
    pub name: String,
    pub dir: PathBuf,
}

/// Files that only Windows runs.
pub fn is_windows_program(program: &str) -> bool {
    let p = program.to_ascii_lowercase();
    p.ends_with(".exe") || p.ends_with(".bat") || p.ends_with(".msi")
}

/// Compares names with numbers in numeric order ("Proton 10.0" > "Proton 9.0").
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let chunks = |s: &str| -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.chars() {
            let d = c.is_ascii_digit();
            match out.last_mut() {
                Some((ld, cur)) if *ld == d => cur.push(c),
                _ => out.push((d, c.to_string())),
            }
        }
        out
    };
    for (x, y) in chunks(a).iter().zip(chunks(b).iter()) {
        let o = match (x, y) {
            ((true, x), (true, y)) => x.parse::<u64>().unwrap_or(0).cmp(&y.parse::<u64>().unwrap_or(0)),
            ((_, x), (_, y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        };
        if o.is_ne() {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

fn scan_tools() -> Vec<Tool> {
    let mut dirs_: Vec<PathBuf> = Vec::new();
    for root in crate::scanner::steam_roots() {
        dirs_.push(root.join("compatibilitytools.d"));
        dirs_.push(root.join("steamapps/common"));
    }
    dirs_.push(PathBuf::from("/usr/share/steam/compatibilitytools.d"));
    let mut custom = Vec::new();
    let mut official = Vec::new();
    for d in dirs_ {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let dir = e.path();
            // Steam's runtimes (EasyAntiCheat, Sniper...) sit next to Proton but have no `proton` script.
            if !dir.join("proton").is_file() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let t = Tool { name, dir };
            if d.ends_with("common") { official.push(t) } else { custom.push(t) }
        }
    }
    // Builds the user installed (GE, CachyOS...) first, then numbered official ones, newest first;
    // Experimental / Hotfix last.
    custom.sort_by(|a, b| natural_cmp(&b.name, &a.name));
    official.sort_by(|a, b| {
        let num = |t: &Tool| t.name.chars().any(|c| c.is_ascii_digit());
        num(b).cmp(&num(a)).then_with(|| natural_cmp(&b.name, &a.name))
    });
    let mut out = custom;
    out.extend(official);
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.name.clone()));
    out
}

/// Installed Proton builds, preferred first. Cached for a few seconds: the settings menu asks every frame.
pub fn tools() -> Vec<Tool> {
    static CACHE: Mutex<Option<(Instant, Vec<Tool>)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = c.as_ref() {
        if at.elapsed() < Duration::from_secs(5) {
            return list.clone();
        }
    }
    let list = scan_tools();
    *c = Some((Instant::now(), list.clone()));
    list
}

/// The build to use: the game's choice, else the default from the settings, else the preferred one.
/// None when the game is set to run directly or nothing is installed.
pub fn resolve(game_choice: &str, default: &str) -> Option<Tool> {
    if game_choice == OFF {
        return None;
    }
    let all = tools();
    let want = if game_choice.is_empty() { default } else { game_choice };
    all.iter().find(|t| t.name == want).or_else(|| all.first()).cloned()
}

/// Where a game's Wine prefix lives (one per game).
fn prefix_dir(game_id: &str) -> PathBuf {
    let base = std::env::var_os("SLIPCASE_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("slipcase")))
        .unwrap_or_else(|| PathBuf::from("."));
    let safe: String = game_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    base.join("prefixes").join(safe)
}

fn on_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(program)).find(|p| p.is_file())
}

/// `KEY=value` pairs from the game's Proton options; anything else is ignored.
pub fn parse_env(options: &str) -> Vec<(String, String)> {
    crate::library::expand_command(options, "")
        .into_iter()
        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// The command that runs `program` with Proton: through umu-run when it is installed (it adds
/// the Steam runtime and protonfixes), else Proton's own script.
pub fn command(tool: &Tool, game_id: &str, program: &str, args: &[String], options: &str) -> Result<Command, String> {
    let prefix = prefix_dir(game_id);
    std::fs::create_dir_all(&prefix).map_err(|e| format!("{}: {e}", prefix.display()))?;
    let mut c = match on_path("umu-run") {
        Some(umu) => {
            let mut c = Command::new(umu);
            c.arg(program).env("WINEPREFIX", prefix.join("pfx")).env("PROTONPATH", &tool.dir).env("GAMEID", "umu-default");
            c
        }
        None => {
            let mut c = Command::new(tool.dir.join("proton"));
            let client = crate::scanner::steam_roots().into_iter().next().unwrap_or_else(|| prefix.clone());
            c.arg("run").arg(program).env("STEAM_COMPAT_DATA_PATH", &prefix).env("STEAM_COMPAT_CLIENT_INSTALL_PATH", client);
            c
        }
    };
    c.args(args);
    c.envs(parse_env(options));
    if let Some(dir) = Path::new(program).parent().filter(|d| !d.as_os_str().is_empty()) {
        c.current_dir(dir);
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["Proton 9.0", "Proton 10.0", "Proton 8.0"];
        v.sort_by(|a, b| natural_cmp(b, a));
        assert_eq!(v, ["Proton 10.0", "Proton 9.0", "Proton 8.0"]);
        assert!(natural_cmp("GE-Proton10-3", "GE-Proton9-27").is_gt());
    }

    #[test]
    fn env_pairs() {
        assert_eq!(
            parse_env(r#"PROTON_ENABLE_NVAPI=1 junk DXVK_HUD="fps,frametimes""#),
            [("PROTON_ENABLE_NVAPI".to_string(), "1".to_string()), ("DXVK_HUD".to_string(), "fps,frametimes".to_string())]
        );
    }

    #[test]
    fn windows_programs() {
        assert!(is_windows_program("/games/X/Game.EXE"));
        assert!(!is_windows_program("/usr/bin/flatpak"));
    }

    /// Lists this computer's Proton builds; run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn local_tools() {
        for t in tools() {
            println!("{} {}", t.name, t.dir.display());
        }
    }
}
