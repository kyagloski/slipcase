//! Works out which console a game file is for when the extension does not say: by looking
//! inside disc images (and a few ROM headers), and failing that by the folder names in its path.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::platform::Platform;
use crate::scanner::platform_for_extension;

/// Best guess at the console for `path`, or None (then it is treated as a PC game / other).
pub fn detect_platform(path: &Path, cfg: &Config) -> Option<Platform> {
    if path.is_dir() {
        return folder_game(path).or_else(|| from_path_names(path));
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if let Some(p) = platform_for_extension(&ext, cfg) {
        return Some(p);
    }
    let sniffed = match ext.as_str() {
        "iso" | "img" | "bin" | "mdf" | "gcm" => sniff_image(path),
        "cue" => cue_track(path).and_then(|bin| sniff_image(&bin)),
        "zip" => zip_inner(path, cfg),
        "xbe" => Some(Platform::Xbox),
        "xex" => Some(Platform::Xbox360),
        "elf" | "dol" => None,
        _ => None,
    };
    // Executables are PC games; anything else falls back to the folder it is in.
    let exe = matches!(ext.as_str(), "exe" | "sh" | "appimage" | "bat" | "lnk" | "x86_64");
    sniffed.or_else(|| if exe { None } else { from_path_names(path) })
}

/// Unpacked console games kept as folders.
fn folder_game(dir: &Path) -> Option<Platform> {
    let has = |name: &str| dir.join(name).exists();
    if has("PS3_GAME") || has("PS3_DISC.SFB") {
        Some(Platform::Ps3)
    } else if has("PSP_GAME") {
        Some(Platform::Psp)
    } else if has("default.xex") {
        Some(Platform::Xbox360)
    } else if has("default.xbe") {
        Some(Platform::Xbox)
    } else {
        None
    }
}

fn read_at(f: &mut File, offset: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// Looks inside a disc image (or cartridge dump with a disc-like extension).
fn sniff_image(path: &Path) -> Option<Platform> {
    let mut f = File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let head = read_at(&mut f, 0, 0x200.min(size as usize))?;
    let at = |o: usize, magic: &[u8]| head.len() >= o + magic.len() && &head[o..o + magic.len()] == magic;
    // Nintendo optical discs carry a magic word in their header.
    if at(0x1C, &[0xC2, 0x33, 0x9F, 0x3D]) {
        return Some(Platform::Gamecube);
    }
    if at(0x18, &[0x5D, 0x1C, 0x9E, 0xA3]) {
        return Some(Platform::Wii);
    }
    // Sega discs start with their system ID (after a 16-byte sync header in raw .bin dumps).
    for o in [0usize, 0x10] {
        if at(o, b"SEGA SEGASATURN") {
            return Some(Platform::Saturn);
        }
        if at(o, b"SEGA SEGAKATANA") {
            return Some(Platform::Dreamcast);
        }
    }
    // A Mega Drive / Genesis cartridge dump saved as .bin.
    if size <= 16 << 20 && (at(0x100, b"SEGA MEGA DRIVE") || at(0x100, b"SEGA GENESIS") || at(0x100, b" SEGA")) {
        return Some(Platform::Genesis);
    }
    xbox_image(&mut f, size).or_else(|| iso9660(&mut f, size))
}

/// Xbox / Xbox 360 game images (XDVDFS): the volume header sits 64 KiB into the game
/// partition, which starts at different places in full disc dumps.
fn xbox_image(f: &mut File, size: u64) -> Option<Platform> {
    const MAGIC: &[u8] = b"MICROSOFT*XBOX*MEDIA";
    for base in [0u64, 0x1830_0000, 0x0FD9_0000, 0x0208_0000] {
        if base + 0x10000 + 32 > size {
            continue;
        }
        let Some(hdr) = read_at(f, base + 0x10000, 28) else { continue };
        if &hdr[..20] != MAGIC {
            continue;
        }
        let root_sector = u32::from_le_bytes(hdr[20..24].try_into().ok()?) as u64;
        let root_size = (u32::from_le_bytes(hdr[24..28].try_into().ok()?) as usize).min(1 << 20);
        let root = read_at(f, base + root_sector * 2048, root_size).unwrap_or_default();
        // The boot file names the console; the 360 boots default.xex, the original default.xbe.
        if contains(&root, b"default.xex") {
            return Some(Platform::Xbox360);
        }
        if contains(&root, b"default.xbe") {
            return Some(Platform::Xbox);
        }
        // Full disc dumps with the game partition far in are Xbox 360 (XGD2/3) or Xbox (XGD1).
        return Some(if base == 0x1830_0000 { Platform::Xbox } else { Platform::Xbox360 });
    }
    None
}

/// CD / DVD images with an ISO 9660 file system: PlayStation discs, told apart by their files.
fn iso9660(f: &mut File, size: u64) -> Option<Platform> {
    // (bytes per sector, offset of the data in a sector): plain .iso, raw Mode 1, raw Mode 2.
    for (sector, data) in [(2048u64, 0u64), (2352, 16), (2352, 24)] {
        let pvd_at = 16 * sector + data;
        if pvd_at + 2048 > size {
            continue;
        }
        let Some(pvd) = read_at(f, pvd_at, 2048) else { continue };
        if &pvd[1..6] != b"CD001" {
            continue;
        }
        let system = String::from_utf8_lossy(&pvd[8..40]).trim().to_string();
        let lba = |rec: &[u8]| u32::from_le_bytes(rec[2..6].try_into().unwrap_or_default()) as u64;
        let len = |rec: &[u8]| u32::from_le_bytes(rec[10..14].try_into().unwrap_or_default()) as usize;
        // Root directory record, then its entries.
        let root = &pvd[156..190];
        let (root_lba, root_len) = (lba(root), len(root).min(64 * 2048));
        let mut names: Vec<(String, u64, usize)> = Vec::new();
        let mut done = 0;
        while done < root_len {
            let Some(sec) = read_at(f, (root_lba + (done / 2048) as u64) * sector + data, 2048) else { break };
            let mut i = 0;
            while i < 2048 {
                let rlen = sec[i] as usize;
                if rlen == 0 || i + rlen > 2048 {
                    break;
                }
                let nlen = sec[i + 32] as usize;
                if 33 + nlen <= rlen {
                    let name = String::from_utf8_lossy(&sec[i + 33..i + 33 + nlen]).to_ascii_uppercase();
                    names.push((name, lba(&sec[i..]), len(&sec[i..])));
                }
                i += rlen;
            }
            done += 2048;
        }
        let find = |n: &str| names.iter().find(|(name, ..)| name.trim_end_matches(";1") == n);
        if find("PSP_GAME").is_some() || system.contains("PSP") {
            return Some(Platform::Psp);
        }
        if find("PS3_GAME").is_some() || find("PS3_DISC.SFB").is_some() {
            return Some(Platform::Ps3);
        }
        if let Some((_, l, n)) = find("SYSTEM.CNF") {
            let cnf = read_at(f, l * sector + data, (*n).min(2048)).unwrap_or_default();
            // PS2 discs boot with BOOT2, PS1 discs with BOOT.
            return Some(if contains(&cnf, b"BOOT2") { Platform::Ps2 } else { Platform::Ps1 });
        }
        if system.contains("PLAYSTATION") {
            return Some(if sector == 2352 { Platform::Ps1 } else { Platform::Ps2 });
        }
        return None;
    }
    None
}

/// The first data track a .cue sheet points at.
fn cue_track(cue: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(cue).ok()?;
    let line = text.lines().find(|l| l.trim_start().to_ascii_uppercase().starts_with("FILE"))?;
    let name = line.split('"').nth(1).map(str::to_owned).or_else(|| line.split_whitespace().nth(1).map(str::to_owned))?;
    Some(cue.parent()?.join(name))
}

/// A zipped ROM: the console of the file inside.
fn zip_inner(path: &Path, cfg: &Config) -> Option<Platform> {
    let mut z = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
    for i in 0..z.len().min(16) {
        let name = z.by_index(i).ok()?.name().to_string();
        let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if let Some(p) = platform_for_extension(&ext, cfg) {
            return Some(p);
        }
    }
    None
}

/// Console names in the folders above the file ("ROMs/PS2/...", "Nintendo 64", "xbox360").
pub fn from_path_names(path: &Path) -> Option<Platform> {
    use Platform::*;
    for dir in path.ancestors().skip(1).take(6) {
        let Some(name) = dir.file_name() else { continue };
        let name = name.to_string_lossy().to_lowercase();
        let compact: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let tokens: Vec<&str> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).collect();
        let has = |t: &str| tokens.contains(&t);
        // Longest names first so "gameboyadvance" wins over "gameboy".
        let by_compact: &[(&str, Platform)] = &[
            ("playstationportable", Psp),
            ("playstationvita", Vita),
            ("playstation5", Ps5),
            ("playstation4", Ps4),
            ("playstation3", Ps3),
            ("playstation2", Ps2),
            ("playstation1", Ps1),
            ("gameboyadvance", Gba),
            ("gameboycolor", Gbc),
            ("gameboy", Gb),
            ("supernintendo", Snes),
            ("superfamicom", Snes),
            ("nintendo64", N64),
            ("nintendo3ds", N3ds),
            ("nintendods", Nds),
            ("nintendoswitch", Switch),
            ("gamecube", Gamecube),
            ("xbox360", Xbox360),
            ("xboxone", XboxOne),
            ("megadrive", Genesis),
            ("genesis", Genesis),
            ("dreamcast", Dreamcast),
            ("saturn", Saturn),
            ("famicom", Nes),
        ];
        if let Some((_, p)) = by_compact.iter().find(|(k, _)| compact.contains(k)) {
            return Some(*p);
        }
        let by_token: &[(&str, Platform)] = &[
            ("ps1", Ps1),
            ("psx", Ps1),
            ("psone", Ps1),
            ("ps2", Ps2),
            ("ps3", Ps3),
            ("ps4", Ps4),
            ("ps5", Ps5),
            ("psp", Psp),
            ("vita", Vita),
            ("psvita", Vita),
            ("ngc", Gamecube),
            ("gc", Gamecube),
            ("wii", Wii),
            ("switch", Switch),
            ("nsw", Switch),
            ("n64", N64),
            ("snes", Snes),
            ("sfc", Snes),
            ("nes", Nes),
            ("gba", Gba),
            ("gbc", Gbc),
            ("gb", Gb),
            ("nds", Nds),
            ("ds", Nds),
            ("3ds", N3ds),
            ("md", Genesis),
            ("dc", Dreamcast),
            ("x360", Xbox360),
            ("playstation", Ps1),
            ("xbox", Xbox),
        ];
        if let Some((_, p)) = by_token.iter().find(|(k, _)| has(k)) {
            return Some(*p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("slipcase-detect-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    /// A minimal ISO 9660 image: one root entry (a file or a directory) with `content`.
    /// `sector` 2048 = plain .iso; 2352 with `data` 24 = raw Mode 2 .bin.
    fn iso(entry: &str, dir: bool, content: &[u8], system: &str, sector: usize, data: usize) -> Vec<u8> {
        let mut img = vec![0u8; sector * 24];
        let put = |img: &mut Vec<u8>, lba: usize, bytes: &[u8]| {
            let o = lba * sector + data;
            img[o..o + bytes.len()].copy_from_slice(bytes);
        };
        let record = |name: &str, lba: u32, len: u32, dir: bool| {
            let mut r = vec![0u8; 33 + name.len() + (name.len() + 1) % 2];
            r[0] = r.len() as u8;
            r[2..6].copy_from_slice(&lba.to_le_bytes());
            r[10..14].copy_from_slice(&len.to_le_bytes());
            r[25] = if dir { 2 } else { 0 };
            r[32] = name.len() as u8;
            r[33..33 + name.len()].copy_from_slice(name.as_bytes());
            r
        };
        let mut pvd = vec![0u8; 2048];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        let sys = format!("{system:<32}");
        pvd[8..40].copy_from_slice(sys.as_bytes());
        let root = record("\0", 18, 2048, true);
        pvd[156..156 + 34].copy_from_slice(&root[..34]);
        put(&mut img, 16, &pvd);
        put(&mut img, 18, &record(entry, 19, content.len() as u32, dir));
        put(&mut img, 19, content);
        img
    }

    fn write(name: &str, bytes: &[u8]) -> PathBuf {
        let p = tmp(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn playstation_discs() {
        let cfg = Config::default();
        let ps2 = write("a.iso", &iso("SYSTEM.CNF;1", false, b"BOOT2 = cdrom0:\\SLUS_200.62;1\n", "PLAYSTATION", 2048, 0));
        assert_eq!(detect_platform(&ps2, &cfg), Some(Platform::Ps2));
        let ps1 = write("b.bin", &iso("SYSTEM.CNF;1", false, b"BOOT = cdrom:\\SCUS_941.63;1\n", "PLAYSTATION", 2352, 24));
        assert_eq!(detect_platform(&ps1, &cfg), Some(Platform::Ps1));
        let cue = write("b.cue", b"FILE \"b.bin\" BINARY\n  TRACK 01 MODE2/2352\n");
        assert_eq!(detect_platform(&cue, &cfg), Some(Platform::Ps1));
        let psp = write("c.iso", &iso("PSP_GAME", true, &[0u8; 64], "PSP GAME", 2048, 0));
        assert_eq!(detect_platform(&psp, &cfg), Some(Platform::Psp));
    }

    #[test]
    fn headers() {
        let cfg = Config::default();
        let mut gc = vec![0u8; 0x8000];
        gc[0x1C..0x20].copy_from_slice(&[0xC2, 0x33, 0x9F, 0x3D]);
        assert_eq!(detect_platform(&write("gc.iso", &gc), &cfg), Some(Platform::Gamecube));
        let mut wii = vec![0u8; 0x8000];
        wii[0x18..0x1C].copy_from_slice(&[0x5D, 0x1C, 0x9E, 0xA3]);
        assert_eq!(detect_platform(&write("wii.iso", &wii), &cfg), Some(Platform::Wii));
        let mut sat = vec![0u8; 0x8000];
        sat[0x10..0x1F].copy_from_slice(b"SEGA SEGASATURN");
        assert_eq!(detect_platform(&write("sat.bin", &sat), &cfg), Some(Platform::Saturn));
        let mut md = vec![0u8; 0x8000];
        md[0x100..0x10F].copy_from_slice(b"SEGA MEGA DRIVE");
        assert_eq!(detect_platform(&write("sonic.bin", &md), &cfg), Some(Platform::Genesis));
        // Xbox 360 image: XDVDFS header at 64 KiB, root directory naming default.xex.
        let mut x = vec![0u8; 0x30000];
        x[0x10000..0x10014].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
        x[0x10014..0x10018].copy_from_slice(&33u32.to_le_bytes());
        x[0x10018..0x1001C].copy_from_slice(&2048u32.to_le_bytes());
        x[33 * 2048 + 14..33 * 2048 + 25].copy_from_slice(b"default.xex");
        assert_eq!(detect_platform(&write("halo.iso", &x), &cfg), Some(Platform::Xbox360));
    }

    #[test]
    fn zipped_rom_and_folder_names() {
        let cfg = Config::default();
        let p = tmp("pokemon.zip");
        {
            let mut z = zip::ZipWriter::new(File::create(&p).unwrap());
            z.start_file("Pokemon Emerald.gba", zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut z, b"rom").unwrap();
            z.finish().unwrap();
        }
        assert_eq!(detect_platform(&p, &cfg), Some(Platform::Gba));
        assert_eq!(from_path_names(Path::new("/roms/Sony - PlayStation 2/Okami.chd")), Some(Platform::Ps2));
        assert_eq!(from_path_names(Path::new("/games/xbox 360/halo3.iso")), Some(Platform::Xbox360));
        assert_eq!(from_path_names(Path::new("/games/GBA/x.bin")), Some(Platform::Gba));
        assert_eq!(from_path_names(Path::new("/games/Nintendo GameCube/x.rvz")), Some(Platform::Gamecube));
        assert_eq!(from_path_names(Path::new("/home/me/Downloads/thing.iso")), None);
    }
}
