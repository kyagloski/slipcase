//! Platform catalogue: physical sizes of cases / discs / cartridges (in cm),
//! file extensions, libretro thumbnail system names and default launch commands.
//!
//! Every value here can be overridden from `config.toml` under `[platforms.<id>]`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Pc,
    Gb,
    Gbc,
    Gba,
    Nds,
    N3ds,
    Nes,
    Snes,
    N64,
    Gamecube,
    Wii,
    Switch,
    Genesis,
    Saturn,
    Dreamcast,
    Ps1,
    Ps2,
    Ps3,
    Ps4,
    Ps5,
    Psp,
    Vita,
    Xbox,
    Xbox360,
    XboxOne,
    Other,
}

impl Platform {
    pub const ALL: &'static [Platform] = &[
        Platform::Pc,
        Platform::Gb,
        Platform::Gbc,
        Platform::Gba,
        Platform::Nds,
        Platform::N3ds,
        Platform::Nes,
        Platform::Snes,
        Platform::N64,
        Platform::Gamecube,
        Platform::Wii,
        Platform::Switch,
        Platform::Genesis,
        Platform::Saturn,
        Platform::Dreamcast,
        Platform::Ps1,
        Platform::Ps2,
        Platform::Ps3,
        Platform::Ps4,
        Platform::Ps5,
        Platform::Psp,
        Platform::Vita,
        Platform::Xbox,
        Platform::Xbox360,
        Platform::XboxOne,
        Platform::Other,
    ];

    /// Stable lowercase id used in config files (`[platforms.gba]`).
    pub fn id(self) -> &'static str {
        match self {
            Platform::Pc => "pc",
            Platform::Gb => "gb",
            Platform::Gbc => "gbc",
            Platform::Gba => "gba",
            Platform::Nds => "nds",
            Platform::N3ds => "n3ds",
            Platform::Nes => "nes",
            Platform::Snes => "snes",
            Platform::N64 => "n64",
            Platform::Gamecube => "gamecube",
            Platform::Wii => "wii",
            Platform::Switch => "switch",
            Platform::Genesis => "genesis",
            Platform::Saturn => "saturn",
            Platform::Dreamcast => "dreamcast",
            Platform::Ps1 => "ps1",
            Platform::Ps2 => "ps2",
            Platform::Ps3 => "ps3",
            Platform::Ps4 => "ps4",
            Platform::Ps5 => "ps5",
            Platform::Psp => "psp",
            Platform::Vita => "vita",
            Platform::Xbox => "xbox",
            Platform::Xbox360 => "xbox360",
            Platform::XboxOne => "xbox_one",
            Platform::Other => "other",
        }
    }

    pub fn from_id(s: &str) -> Option<Platform> {
        let s = s.trim().to_ascii_lowercase();
        Platform::ALL.iter().copied().find(|p| p.id() == s)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Platform::Pc => "PC",
            Platform::Gb => "Game Boy",
            Platform::Gbc => "Game Boy Color",
            Platform::Gba => "Game Boy Advance",
            Platform::Nds => "Nintendo DS",
            Platform::N3ds => "Nintendo 3DS",
            Platform::Nes => "NES",
            Platform::Snes => "Super Nintendo",
            Platform::N64 => "Nintendo 64",
            Platform::Gamecube => "GameCube",
            Platform::Wii => "Wii",
            Platform::Switch => "Switch",
            Platform::Genesis => "Genesis / Mega Drive",
            Platform::Saturn => "Saturn",
            Platform::Dreamcast => "Dreamcast",
            Platform::Ps1 => "PlayStation",
            Platform::Ps2 => "PlayStation 2",
            Platform::Ps3 => "PlayStation 3",
            Platform::Ps4 => "PlayStation 4",
            Platform::Ps5 => "PlayStation 5",
            Platform::Psp => "PSP",
            Platform::Vita => "PS Vita",
            Platform::Xbox => "Xbox",
            Platform::Xbox360 => "Xbox 360",
            Platform::XboxOne => "Xbox One",
            Platform::Other => "Other",
        }
    }

    /// Name of the system folder on thumbnails.libretro.com (None = not covered).
    pub fn libretro_system(self) -> Option<&'static str> {
        Some(match self {
            Platform::Gb => "Nintendo - Game Boy",
            Platform::Gbc => "Nintendo - Game Boy Color",
            Platform::Gba => "Nintendo - Game Boy Advance",
            Platform::Nds => "Nintendo - Nintendo DS",
            Platform::N3ds => "Nintendo - Nintendo 3DS",
            Platform::Nes => "Nintendo - Nintendo Entertainment System",
            Platform::Snes => "Nintendo - Super Nintendo Entertainment System",
            Platform::N64 => "Nintendo - Nintendo 64",
            Platform::Gamecube => "Nintendo - GameCube",
            Platform::Wii => "Nintendo - Wii",
            Platform::Switch => return None,
            Platform::Genesis => "Sega - Mega Drive - Genesis",
            Platform::Saturn => "Sega - Saturn",
            Platform::Dreamcast => "Sega - Dreamcast",
            Platform::Ps1 => "Sony - PlayStation",
            Platform::Ps2 => "Sony - PlayStation 2",
            Platform::Ps3 => "Sony - PlayStation 3",
            Platform::Psp => "Sony - PlayStation Portable",
            Platform::Vita => "Sony - PlayStation Vita",
            Platform::Xbox => "Microsoft - Xbox",
            Platform::Xbox360 => "Microsoft - Xbox 360",
            _ => return None,
        })
    }

    /// File extensions (lowercase, no dot) that identify this platform's ROMs/images.
    /// Ambiguous containers (iso, bin, cue, chd) are intentionally left to the
    /// folder-level `platform` setting in `[[scan.rom_dirs]]`.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Platform::Gb => &["gb"],
            Platform::Gbc => &["gbc"],
            Platform::Gba => &["gba"],
            Platform::Nds => &["nds", "dsi"],
            Platform::N3ds => &["3ds", "cia", "cxi"],
            Platform::Nes => &["nes", "fds", "unf"],
            Platform::Snes => &["sfc", "smc", "fig", "swc"],
            Platform::N64 => &["n64", "z64", "v64"],
            Platform::Gamecube => &["gcm", "rvz", "gcz"],
            Platform::Wii => &["wbfs", "wad"],
            Platform::Switch => &["nsp", "xci"],
            Platform::Genesis => &["md", "gen", "smd"],
            Platform::Saturn => &[],
            Platform::Dreamcast => &["gdi", "cdi"],
            Platform::Ps1 => &[],
            Platform::Ps2 => &[],
            Platform::Ps3 => &[],
            Platform::Psp => &["cso", "pbp"],
            Platform::Vita => &["vpk"],
            _ => &[],
        }
    }

    /// Default emulator / launcher command. `{rom}` is replaced with the quoted path.
    /// These are only sensible guesses: override them in `config.toml`.
    pub fn default_command(self) -> &'static str {
        match self {
            Platform::Gb | Platform::Gbc | Platform::Gba => "mgba-qt {rom}",
            Platform::Nds => "melonDS {rom}",
            Platform::N3ds => "azahar {rom}",
            Platform::Nes | Platform::Snes | Platform::Genesis => "retroarch {rom}",
            Platform::N64 => "simple64-gui {rom}",
            Platform::Gamecube | Platform::Wii => "dolphin-emu -b -e {rom}",
            Platform::Switch => "ryujinx {rom}",
            Platform::Saturn | Platform::Dreamcast => "retroarch {rom}",
            Platform::Ps1 => "duckstation-qt -batch {rom}",
            Platform::Ps2 => "pcsx2-qt -batch {rom}",
            Platform::Ps3 => "rpcs3 {rom}",
            Platform::Psp => "PPSSPPSDL {rom}",
            Platform::Vita => "Vita3K -r {rom}",
            Platform::Xbox => "xemu -dvd_path {rom}",
            _ => "{rom}",
        }
    }

    /// Physical geometry. Units are centimetres; the renderer scales everything
    /// uniformly so relative sizes stay correct between platforms.
    pub fn geometry(self) -> Geometry {
        // (case w, h, d) ; media
        let dvd = (13.5, 19.0, 1.4);
        match self {
            Platform::Pc => Geometry::case(dvd, Media::Disc { diameter: 12.0 }),
            Platform::Gb | Platform::Gbc => Geometry::case((12.5, 12.5, 2.4), Media::Cart { w: 5.7, h: 6.5, d: 0.8 }),
            Platform::Gba => Geometry::case((12.0, 12.0, 2.4), Media::Cart { w: 5.7, h: 3.5, d: 0.8 }),
            Platform::Nds | Platform::N3ds => Geometry::case((12.5, 13.7, 1.4), Media::Cart { w: 3.5, h: 3.3, d: 0.38 }),
            Platform::Nes => Geometry::case((13.0, 17.5, 4.2), Media::Cart { w: 12.0, h: 13.3, d: 2.0 }),
            Platform::Snes => Geometry::case((18.4, 13.0, 3.8), Media::Cart { w: 12.0, h: 10.4, d: 2.0 }),
            Platform::N64 => Geometry::case((19.0, 13.4, 4.8), Media::Cart { w: 11.6, h: 7.6, d: 1.9 }),
            Platform::Gamecube => Geometry::case(dvd, Media::Disc { diameter: 8.0 }),
            Platform::Wii => Geometry::case((13.5, 19.0, 1.5), Media::Disc { diameter: 12.0 }),
            Platform::Switch => Geometry::case((10.5, 17.0, 1.0), Media::Cart { w: 2.1, h: 3.1, d: 0.33 }),
            Platform::Genesis => Geometry::case((13.5, 19.0, 2.6), Media::Cart { w: 11.0, h: 13.0, d: 1.9 }),
            Platform::Saturn => Geometry::case((14.2, 12.5, 1.0), Media::Disc { diameter: 12.0 }),
            Platform::Dreamcast => Geometry::case((14.2, 12.5, 1.0), Media::Disc { diameter: 12.0 }),
            Platform::Ps1 => Geometry::case((14.2, 12.5, 1.0), Media::Disc { diameter: 12.0 }),
            Platform::Ps2 => Geometry::case(dvd, Media::Disc { diameter: 12.0 }),
            Platform::Ps3 => Geometry::case((13.5, 17.2, 1.4), Media::Disc { diameter: 12.0 }),
            Platform::Ps4 | Platform::Ps5 => Geometry::case((13.5, 17.2, 1.4), Media::Disc { diameter: 12.0 }),
            Platform::Psp => Geometry::case((10.4, 17.6, 1.4), Media::Umd),
            Platform::Vita => Geometry::case((10.5, 13.5, 1.1), Media::Cart { w: 2.2, h: 3.0, d: 0.2 }),
            Platform::Xbox | Platform::Xbox360 => Geometry::case(dvd, Media::Disc { diameter: 12.0 }),
            Platform::XboxOne => Geometry::case((13.5, 17.2, 1.4), Media::Disc { diameter: 12.0 }),
            Platform::Other => Geometry::case(dvd, Media::Disc { diameter: 12.0 }),
        }
    }

    /// What the box is made of and what colour the plastic is, as sold.
    pub fn case_look(self) -> CaseLook {
        use CaseKind::*;
        let look = |kind, shell: u32, alpha: f32, tray: u32, roughness: f32| CaseLook {
            kind,
            shell: srgba(shell, alpha),
            // Jewel cases: the tray's own colour; boxes: the cardboard insert; keep cases: unused.
            tray: srgba(tray, 1.0),
            roughness,
        };
        match self {
            // Black DVD keep cases.
            Platform::Pc | Platform::Ps2 | Platform::Gamecube | Platform::Other => look(Keep, 0x111113, 1.0, 0x111113, 0.32),
            // The original Xbox shipped in black cases (the green is on the printed banner).
            Platform::Xbox => look(Keep, 0x121412, 1.0, 0x121412, 0.32),
            Platform::Xbox360 => look(Keep, 0x3f9a2c, 0.93, 0x3f9a2c, 0.22),
            Platform::XboxOne => look(Keep, 0x13a10e, 0.82, 0x13a10e, 0.18),
            Platform::Ps3 => look(Keep, 0x9fb4d8, 0.42, 0x9fb4d8, 0.12),
            Platform::Ps4 | Platform::Ps5 => look(Keep, 0x0a4bc4, 0.80, 0x0a4bc4, 0.16),
            Platform::Psp => look(Keep, 0xd4d8de, 0.40, 0xd4d8de, 0.14),
            Platform::Vita => look(Keep, 0xaebfd8, 0.42, 0xaebfd8, 0.14),
            Platform::Wii => look(Keep, 0xeeeff1, 1.0, 0xeeeff1, 0.30),
            Platform::Switch => look(Keep, 0xd8101a, 0.97, 0xd8101a, 0.26),
            Platform::Nds => look(Keep, 0xc3c8cf, 0.50, 0xc3c8cf, 0.16),
            Platform::N3ds => look(Keep, 0xf2f2f2, 1.0, 0xf2f2f2, 0.30),
            Platform::Genesis => look(Keep, 0x141414, 1.0, 0x141414, 0.40),
            // CD jewel cases: clear lid, coloured tray.
            Platform::Ps1 | Platform::Saturn => look(Jewel, 0xdfe5ea, 0.22, 0x0d0d0f, 0.06),
            Platform::Dreamcast => look(Jewel, 0xdfe5ea, 0.22, 0xe8e8ea, 0.06),
            // Cardboard boxes with a plastic or card insert.
            Platform::Nes | Platform::Snes | Platform::N64 => look(Box, 0xaaa294, 1.0, 0x1a1a1c, 0.92),
            Platform::Gb | Platform::Gbc | Platform::Gba => look(Box, 0xaaa294, 1.0, 0x1c1c22, 0.92),
        }
    }

    /// Corner radius of the case as sold, in cm (measured off retail cases; boxes and jewel
    /// cases are nearly square).
    pub fn corner_radius(self) -> f32 {
        match self {
            // Amaray DVD keep cases.
            Platform::Pc | Platform::Ps2 | Platform::Xbox | Platform::Xbox360 | Platform::Gamecube | Platform::Wii | Platform::Other => 0.35,
            // Blu-ray style cases.
            Platform::Ps3 | Platform::Ps4 | Platform::Ps5 | Platform::XboxOne => 0.45,
            Platform::Switch => 0.6,
            Platform::Nds | Platform::N3ds => 0.5,
            Platform::Psp => 0.6,
            Platform::Vita => 0.5,
            Platform::Genesis => 0.3,
            // CD jewel cases: moulded, only just softened.
            Platform::Ps1 | Platform::Saturn | Platform::Dreamcast => 0.08,
            // Folded cardboard.
            Platform::Nes | Platform::Snes | Platform::N64 | Platform::Gb | Platform::Gbc | Platform::Gba => 0.05,
        }
    }

    /// The console's banner printed across the top of the cover and spine (None: no banner).
    /// Plain lettering in the console's colours, not the trademarked logos.
    pub fn banner(self) -> Option<Banner> {
        let b = |text, bg, fg, line| Some(Banner { text, bg, fg, line });
        match self {
            Platform::Xbox => b("XBOX", 0x0d0d0d, 0x8fd44a, Some(0x52b043)),
            Platform::Xbox360 => b("XBOX 360", 0xf4f4f4, 0x5a9e2c, Some(0x7cbf3f)),
            Platform::XboxOne => b("XBOX ONE", 0x107c10, 0xffffff, None),
            Platform::Ps1 => b("PlayStation", 0x000000, 0xffffff, None),
            Platform::Ps2 => b("PlayStation 2", 0x000000, 0xffffff, Some(0x1f5fd6)),
            Platform::Ps3 => b("PS3", 0x050505, 0xffffff, None),
            Platform::Ps4 => b("PS4", 0x0050c8, 0xffffff, None),
            Platform::Ps5 => b("PS5", 0xffffff, 0x101010, None),
            Platform::Psp => b("PSP", 0x0a0a0a, 0xffffff, None),
            Platform::Vita => b("PS VITA", 0x003a8c, 0xffffff, None),
            Platform::Gamecube => b("NINTENDO GAMECUBE", 0x0a0a0a, 0xffffff, Some(0x6a4fb3)),
            Platform::Wii => b("Wii", 0xf2f2f2, 0x8a8a8a, Some(0x3aa0d8)),
            Platform::Switch => b("NINTENDO SWITCH", 0xe60012, 0xffffff, None),
            Platform::Nds => b("NINTENDO DS", 0xe6e6e6, 0x303030, None),
            Platform::N3ds => b("NINTENDO 3DS", 0xffffff, 0xce1020, None),
            Platform::Nes => b("NINTENDO ENTERTAINMENT SYSTEM", 0x101010, 0xffffff, Some(0xd02020)),
            Platform::Snes => b("SUPER NINTENDO", 0xbdbdc4, 0x3a2f7a, None),
            Platform::N64 => b("NINTENDO 64", 0x0a0a0a, 0xffffff, Some(0x1f8f3a)),
            Platform::Gb => b("GAME BOY", 0x8b8b92, 0xffffff, None),
            Platform::Gbc => b("GAME BOY COLOR", 0x2a2a2e, 0xffffff, Some(0xe0b020)),
            Platform::Gba => b("GAME BOY ADVANCE", 0x4b3c8c, 0xffffff, None),
            Platform::Genesis => b("SEGA GENESIS", 0x000000, 0xffffff, Some(0xc81e1e)),
            Platform::Saturn => b("SEGA SATURN", 0x000000, 0xffffff, None),
            Platform::Dreamcast => b("DREAMCAST", 0xffffff, 0xe86a10, None),
            Platform::Pc | Platform::Other => None,
        }
    }

    /// Cartridges famous for their own colour, by title, for when no photo of the cartridge
    /// is available. Clear (see-through) shells have alpha below 1.
    pub fn known_cart_color(self, title: &str) -> Option<[f32; 4]> {
        let t: String = title.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        let has = |k: &str| t.contains(k);
        let pokemon = has("pokemon") || has("pocket monsters");
        let c = |h: u32, a: f32| Some(srgba(h, a));
        match self {
            Platform::Gb | Platform::Gbc | Platform::Gba if pokemon => {
                if has("yellow") {
                    c(0xf2c418, 1.0)
                } else if has("leafgreen") || has("leaf green") {
                    c(0x3fae49, 1.0)
                } else if has("firered") || has("fire red") {
                    c(0xd8321e, 1.0)
                } else if has("emerald") {
                    c(0x22b573, 0.75)
                } else if has("sapphire") {
                    c(0x2f5fd0, 0.75)
                } else if has("ruby") {
                    c(0xd0202e, 0.75)
                } else if has("crystal") {
                    c(0x8fd3f5, 0.6)
                } else if has("silver") {
                    c(0xc2c4c8, 1.0)
                } else if has("gold") {
                    c(0xd4af37, 1.0)
                } else if has("green") {
                    c(0x3c9a45, 1.0)
                } else if has("blue") {
                    c(0x2e5fc7, 1.0)
                } else if has("red") {
                    c(0xc8291e, 1.0)
                } else if has("pinball") || has("puzzle challenge") {
                    c(0x1b1b1e, 1.0)
                } else {
                    None
                }
            }
            Platform::N64 if has("zelda") && (has("ocarina") || has("majora")) => c(0xd4af37, 1.0),
            Platform::N64 if has("donkey kong 64") => c(0xf2c418, 1.0),
            Platform::N64 if pokemon && has("stadium") => c(0x6a6c72, 1.0),
            Platform::Nes if has("zelda") => c(0xd4af37, 1.0),
            _ => None,
        }
    }

    /// Outline family and shell colour of the cartridge.
    pub fn cart_look(self) -> (CartShape, [f32; 4]) {
        match self {
            Platform::Gb => (CartShape::GameBoy, srgba(0xa9a9ad, 1.0)),
            Platform::Gbc => (CartShape::GameBoy, srgba(0x1b1b1e, 1.0)),
            Platform::Gba => (CartShape::Gba, srgba(0x4a4a50, 1.0)),
            Platform::Nds => (CartShape::Ds, srgba(0x26272b, 1.0)),
            Platform::N3ds => (CartShape::N3ds, srgba(0xd5d6d8, 1.0)),
            Platform::Switch => (CartShape::Switch, srgba(0x1f2023, 1.0)),
            Platform::Vita => (CartShape::Vita, srgba(0x151517, 1.0)),
            Platform::Nes => (CartShape::Nes, srgba(0x8d8d90, 1.0)),
            Platform::Snes => (CartShape::Snes, srgba(0x8a8a8e, 1.0)),
            Platform::N64 => (CartShape::N64, srgba(0x55565a, 1.0)),
            Platform::Genesis => (CartShape::Genesis, srgba(0x121213, 1.0)),
            _ => (CartShape::Generic, srgba(0x333336, 1.0)),
        }
    }

    /// Case plastic / spine accent colour as sRGB.
    pub fn accent(self) -> [f32; 3] {
        let hex = |h: u32| [((h >> 16) & 255) as f32 / 255.0, ((h >> 8) & 255) as f32 / 255.0, (h & 255) as f32 / 255.0];
        match self {
            Platform::Pc => hex(0x1b2838),
            Platform::Gb | Platform::Gbc => hex(0x8b956d),
            Platform::Gba => hex(0x4b3c8c),
            Platform::Nds | Platform::N3ds => hex(0xc8c8c8),
            Platform::Nes | Platform::Snes | Platform::N64 | Platform::Gamecube | Platform::Wii => hex(0xb01e23),
            Platform::Switch => hex(0xe60012),
            Platform::Genesis | Platform::Saturn | Platform::Dreamcast => hex(0x0b3d91),
            Platform::Ps1 | Platform::Ps2 | Platform::Ps3 | Platform::Ps4 | Platform::Ps5 | Platform::Psp | Platform::Vita => hex(0x003791),
            Platform::Xbox | Platform::Xbox360 | Platform::XboxOne => hex(0x107c10),
            Platform::Other => hex(0x444444),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Media {
    Disc { diameter: f32 },
    Cart { w: f32, h: f32, d: f32 },
    /// PSP Universal Media Disc: a 6 cm disc in a plastic caddy.
    Umd,
}

/// Outer size of a UMD caddy (w, h, d) in cm.
pub const UMD_SIZE: (f32, f32, f32) = (6.4, 6.5, 0.42);

/// A console banner: lettering on a coloured band, with an optional thin line under it.
#[derive(Debug, Clone, Copy)]
pub struct Banner {
    pub text: &'static str,
    pub bg: u32,
    pub fg: u32,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseKind {
    /// Moulded plastic keep case (DVD, Blu-ray, Switch, DS, Genesis clamshell...).
    Keep,
    /// CD jewel case: clear lid on a coloured tray.
    Jewel,
    /// Cardboard box with an insert holding the cartridge.
    Box,
}

#[derive(Debug, Clone, Copy)]
pub struct CaseLook {
    pub kind: CaseKind,
    /// Plastic (or cardboard) colour, sRGB with alpha for translucent cases.
    pub shell: [f32; 4],
    pub tray: [f32; 4],
    pub roughness: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CartShape {
    GameBoy,
    Gba,
    Ds,
    N3ds,
    Switch,
    Vita,
    Nes,
    Snes,
    N64,
    Genesis,
    Generic,
}

fn srgba(h: u32, a: f32) -> [f32; 4] {
    [((h >> 16) & 255) as f32 / 255.0, ((h >> 8) & 255) as f32 / 255.0, (h & 255) as f32 / 255.0, a]
}

#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub case: (f32, f32, f32),
    pub media: Media,
}

impl Geometry {
    fn case(case: (f32, f32, f32), media: Media) -> Self {
        Self { case, media }
    }
}
