// Hide the console window on Windows release builds.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod app;
mod background;
mod clipboard;
mod config;
mod covers;
mod cursor;
mod detect;
mod egg;
mod launchbox;
mod layouts;
mod library;
mod platform;
mod render;
mod scanner;
mod settings;
mod textedit;

use bevy::prelude::*;
use bevy::window::{CompositeAlphaMode, PresentMode, WindowMode, WindowPosition};

use config::Config;
use library::Library;
use platform::Platform;

const HELP: &str = "\
slipcase - a 3D game launcher

USAGE:
    slipcase                          start the launcher
    slipcase scan                     scan for games and update the library (no window)
    slipcase add <path> [--title T] [--platform ID]
                                      add a ROM, disc image or executable
    slipcase list                     print the library
    slipcase platforms                list platform ids for --platform / [platforms.*]
    slipcase config                   print the config and library locations
    slipcase --settings               start with the settings menu open
    slipcase --overlay                no window or background: cases float over your desktop,
                                      and Slipcase closes when a game starts
    slipcase --help

Environment:
    SLIPCASE_HOME   store config, library and cache in this folder (portable mode)
    SLIPCASE_FONT   path to a .ttf used for spines and placeholder covers
";

fn cli(args: &[String]) -> Option<i32> {
    let cmd = args.first()?.as_str();
    match cmd {
        "--help" | "-h" | "help" => {
            print!("{HELP}");
            Some(0)
        }
        "config" => {
            println!("config:  {}", config::config_path().display());
            println!("library: {}", library::library_path().display());
            println!("cache:   {}", config::cache_dir().display());
            Some(0)
        }
        "platforms" => {
            for p in Platform::ALL {
                println!("{:<10} {}", p.id(), p.display_name());
            }
            Some(0)
        }
        "list" => {
            let lib = Library::load();
            for g in &lib.games {
                println!("{:<22} {:<8} {}", g.platform.display_name(), g.source, g.title);
            }
            println!("{} games", lib.games.len());
            Some(0)
        }
        "scan" => {
            let cfg = Config::load();
            let mut lib = Library::load();
            let found = scanner::scan_all(&cfg);
            let (added, removed) = lib.merge_scan(found);
            lib.save();
            println!("{added} added, {removed} removed, {} total", lib.games.len());
            Some(0)
        }
        "add" => {
            let mut path: Option<String> = None;
            let mut title = None;
            let mut plat = None;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--title" => title = it.next().cloned(),
                    "--platform" => {
                        let id = it.next().cloned().unwrap_or_default();
                        plat = Platform::from_id(&id);
                        if plat.is_none() {
                            eprintln!("unknown platform '{id}' (see `slipcase platforms`)");
                            return Some(2);
                        }
                    }
                    other => path = Some(other.to_string()),
                }
            }
            let Some(path) = path else {
                eprintln!("usage: slipcase add <path> [--title T] [--platform ID]");
                return Some(2);
            };
            let cfg = Config::load();
            let mut lib = Library::load();
            match lib.add_manual(std::path::Path::new(&path), title, plat, &cfg) {
                Some(id) => {
                    let t = lib.find(&id).map(|g| g.title.clone()).unwrap_or_default();
                    println!("added: {t}");
                    Some(0)
                }
                None => {
                    eprintln!("could not add {path}");
                    Some(1)
                }
            }
        }
        _ => None,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli(&args) {
        std::process::exit(code);
    }

    let mut cfg = Config::load();
    let mut lib = Library::load();
    if lib.apply_hidden_titles(&mut cfg.data.scan.hidden_titles) {
        lib.save();
        cfg.save();
    }
    // Games whose file has been deleted since the last run.
    let mut toast = app::Toast::default();
    let removed = lib.prune_missing();
    if !removed.is_empty() {
        lib.save();
        toast.show(match removed.as_slice() {
            [one] => format!("Removed {one} (its file is no longer there)"),
            many => format!("Removed {} games whose files are no longer there", many.len()),
        });
    }
    let covers = covers::CoverService::start(cfg.covers.clone());
    let overlay = cfg.general.overlay || args.iter().any(|a| a == "--overlay");

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Slipcase".into(),
                resolution: (cfg.general.window_width, cfg.general.window_height).into(),
                present_mode: if cfg.general.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync },
                // Always start as a normal window on the screen with the mouse: Bevy would put a
                // window that starts fullscreen on the primary screen. `app::place_window` goes
                // fullscreen (or maximises the overlay) a moment later, on whichever screen it is on.
                mode: WindowMode::Windowed,
                position: match cursor::global_cursor() {
                    Some((x, y)) => WindowPosition::At(IVec2::new(x - cfg.general.window_width as i32 / 2, y - cfg.general.window_height as i32 / 2)),
                    None => WindowPosition::Automatic,
                },
                transparent: overlay,
                decorations: !overlay,
                composite_alpha_mode: if overlay { app::overlay_alpha(&cfg.general.overlay_alpha) } else { CompositeAlphaMode::Auto },
                ..default()
            }),
            ..default()
        }))
        // Draw continuously while focused; in the background draw nothing (or a few fps)
        // so a running game gets the GPU and CPU.
        .insert_resource(app::winit_settings(&cfg, overlay))
        .add_plugins(background::BackgroundPlugin)
        .add_plugins(render::DiscMaterialPlugin)
        .insert_resource(app::initial_msaa(&cfg))
        .insert_resource(ClearColor(if overlay { Color::NONE } else { Color::BLACK }))
        .insert_resource(app::Overlay(overlay))
        .insert_resource(toast)
        .insert_resource(app::ViewState::new(&cfg))
        .insert_resource(render::SpineFont(render::load_font()))
        .insert_resource(cfg)
        .insert_resource(lib)
        .insert_resource(covers)
        .add_plugins((app::LauncherPlugin, egg::EggPlugin))
        .add_plugins(settings::SettingsPlugin)
        .insert_resource(if args.iter().any(|a| a == "--settings") { settings::Menu::opened() } else { settings::Menu::default() })
        .run();
}

