# Slipcase: the full guide

A fast, native 3D game launcher written in Rust on [Bevy](https://bevyengine.org). Your library is shown as
real 3D cases, discs and cartridges (sized per platform) over an animated background that adapts to
the colours of the selected cover, in the spirit of the Aurora dashboard for modded Xbox consoles.

No web view, no Electron: one native binary.

## Quick start

```sh
./build.sh            # Linux      -> dist/slipcase-linux-x86_64
.\build.ps1           # Windows    -> dist\slipcase-windows-x86_64.exe
./dist/slipcase-linux-x86_64
```

On first start Slipcase scans for Steam, Epic (Windows), Heroic and Lutris games, then downloads covers in
the background. Everything is cached, so later starts are instant.

### Adding games

| Way | How |
|---|---|
| Drag and drop | Drop a ROM, disc image, executable or a folder onto the window |
| Command line | `slipcase add "Zelda.gba"`, `slipcase add game.iso --platform ps2 --title "My Game"` |
| ROM folders | `[[scan.rom_dirs]]` in `config.toml`, then press `F5` |
| Game folders | `scan.game_dirs`: every sub-folder is one game, the biggest executable launches it |
| Stores | Steam (all libraries), Epic, Heroic, Lutris are detected automatically |

```toml
[[scan.rom_dirs]]
path = "/home/me/roms/gba"
platform = "gba"

[[scan.rom_dirs]]
path = "/home/me/roms/ps2"
platform = "ps2"
extensions = ["iso", "chd"]
```

### Controls (all rebindable in `[keys]`)

| Key | Action | Key | Action |
|---|---|---|---|
| Left / Right, A / D, wheel | Browse | Tab / Shift+Tab (or K) | Next / previous layout |
| Up / Down | Jump 8 (grid: a row) | V | Cases, discs, cases+discs, flat |
| Enter | Launch | B | Next background |
| Space, or tap Shift | Open the case to look inside (again: close it) | Backspace or Tab | Close the case |
| PageUp / PageDown, [ / ] | Platform filter | H or the heart by the title | Favourite |
| / | Search | F5 | Rescan |
| Home / End | First / last | F11 | Fullscreen |
| I | Toggle info | Q or Esc | Quit (closes the settings or an open case first) |
| F | Flip (see the back) | Z / C | Turn 90 degrees (spine, back, edge) |
| M, F1 or the gear button | Settings menu | | |

The settings menu (also `slipcase --settings`; technical options sit under each category's "Advanced" row); Reset at the bottom (click twice to confirm) puts every setting back to its default but keeps your folders, hidden games, per-console overrides and API keys has every display option plus adding games, ROM folders,
choosing a cover, renaming, changing a game's platform, its location (the file, program or URI it starts;
type it or browse) and extra launch arguments, and hiding games (Settings > Hidden games lists them, click one to bring it back). Arguments are added to the program or
emulator command; for Steam games they are passed through `steam://run/<id>//<args>/`.

The game under the mouse gets a soft coloured outline (round the disc too in case + disc mode):

```toml
[display]
highlight = "outline"        # outline, glow, both, none
highlight_color = "#2f8cff"
highlight_width = 0.16       # how far the edge fades out, cm
hover_tilt = true            # the game under the mouse leans away from the cursor
```

Zooming out (ctrl + wheel) keeps more games on screen.

With a case open you can double-click to launch, drag (left or right button) to look around it, use the wheel to spin the disc, ctrl + wheel to zoom, and point at the disc, cartridge or
manual to lift it. A flick with the right mouse button spins a game; it slows down and settles on its front
or back, as it does after any rotation.

Games at the edge of each layout fade out instead of popping in and out.

Mouse: hovering lifts the game under the cursor, outlines it and leans it away from the cursor; left-drag pans the wall (Wall layout) or scrubs through the others (let go mid-drag and it
coasts on, slowing down; any click stops it), right-drag rotates the selected
case freely (a short right click flips it), click selects, double-click launches, wheel browses (scrolls rows on the wall), ctrl + wheel zooms.

Gamepad: D-pad or left stick browse, A launch, bumpers layout, X style, Y favourite, B background,
right stick rotates the selected item, right stick click flips it, Start opens the settings.

### Games you own but have not installed

Settings > Layout > **Show games that are not installed** (`display.show_uninstalled`) adds store games you own
but have not installed; the info line says "not installed", and launching one opens the store's install dialog.
They are found by every scan (`scan.owned`, on by default) and stay hidden until the option is on:

- **Steam**: with a Steam Web API key (free, https://steamcommunity.com/dev/apikey; Settings > Library >
  Advanced, or `scan.steam_api_key`), every game on the account (the profile's game details must not be
  private). Without a key, the games you have played on this computer (from Steam's local config, named from
  its app cache). `scan.steam_id` picks the account; empty means the one last signed in.
- **Heroic**: the Epic, GOG and Amazon libraries Heroic has cached.

### Easter egg

Press Ctrl five times in a row. Every game in the list rains down onto a small grey table and piles up with real
physics; the cases slide over each other, and the ones that go over the edge fall away out of sight. Once the pile settles,
one of the games still on the table pushes its way out, spins up to the camera and becomes the selected game. Until then you can drag the cases round
the table with the left mouse button (let go mid-swing to throw one) and turn the one you hold by moving the mouse
with the right button down too; Ctrl + wheel (or the zoom keys) zooms; the pick waits until the pile has been left
alone for a moment, or press Enter / Space to pick straight away. Esc / Backspace / Q stops it early.

### Art on the disc and the insert

The disc (or cartridge) label and the insert in the lid get art of their own where there is some, so they do not
all repeat the cover: a disc gets LaunchBox's scan of the actual disc, or else the game's wide hero art with its
logo above the hole; the insert gets a gameplay screenshot under the logo.

Every case has one, including the cardboard boxes of cartridge games (N64, GBA, Game Boy, NES, SNES...), where
it lies loose in the lid. The insert is a little booklet you can read: point at it in an open case and its cover lifts; click it (or roll the
wheel down over it) to turn the page, click the turned pages (or roll up) to go back, and click on the last page to
close it again. Its pages are made from what the databases know about the game: an "About the game" page (the
LaunchBox overview or the Steam store text), a facts page under the logo (platform, release date, developer,
publisher, genres, players, rating, Metacritic score, and for Steam games how many are playing right now), screenshots, full-page art, credits and a ruled notes page.

Settings > Selected game > **Choose art (cover, disc, insert)...** opens a window with every image the sources
have for the game (LaunchBox scans, logos, fanart and screenshots; Steam's art, from its local cache or the store,
and screenshots; SteamGridDB grids with a key; libretro thumbnails; the current cover). Tabs for the cover, the back,
the spine, the disc / cartridge label and the insert each list them in the order that suits that surface; click one to use it,
**Choose a file...** for a picture of your own, or **Automatic** to go back to the default. The window stays
open so you can set several (the one in use is outlined in green). It sits over the settings panel, and the game
turns or opens to show the surface you are choosing for: its back, its spine, or the open case for the disc and
the insert. Picks are kept with
the game (copied into the cache's `chosen` folder). Each tab also offers what that surface shows now ("In use now"), including the
disc and insert art Slipcase made automatically.

Store games you own but have not installed carry a small download badge in the top corner of the cover, and the
info line under the title starts with "Not installed (Enter to install)".

### Overlay mode

`slipcase --overlay` (or `general.overlay = true`, or Settings > Background > Overlay, which offers to restart) drops the window border and the background:
the cases float over whatever you have open, and Slipcase closes as soon as a game starts. Outside overlay
mode, `general.close_on_launch` does the same quitting.

**Halfway overlay** (Settings > Background > Overlay halfway, or `background.overlay_halfway = true`): the
animated background and game art still draw, but only in the middle of the screen, fading out to your
desktop towards the edges. `background.overlay_halfway_size` (Advanced, 0.2–1.5, default 0.75) sets how far
out it reaches. It switches on and off without a restart.

The overlay covers the screen as one maximised window, so it takes the mouse and keyboard while it is
open; quit with Esc or Q, or launch something. If the overlay shows a black background instead of your desktop,
set `general.overlay_alpha` to `premultiplied`, `postmultiplied` or `inherit` (which one works depends on
the OS and GPU driver; on Linux the default is `premultiplied`). On Windows the renderer usually cannot get
a see-through window, so expect a borderless window with a black background there.

While the window is in the background (e.g. a game is running) Slipcase draws nothing and uses next to no
CPU or GPU (`general.background_fps`, 0 by default).

## Display modes

Layouts (`display.layout`): `cover_flow`, `spiral`, `stacks`, `towers`, `messy_towers`, `carousel`, `stack`, `pile`, `shelf`, `grid`, `table`, `stand_up`, `spines`, `helix`.

**Looping list** (Settings > Layout > Loop the list, or `display.loop_list = true`): past the last game comes the
first again, so you can scroll forever in either direction. Home / End still jump to the first / last game. On the
wall and the table the rows loop: below the last row comes the first again (the last row is padded so every lap
starts on a new row). Looping needs at least 3 games.

On the wall and the table, scrolling (wheel) and dragging with the left button only move the view; a game is
raised only when you click it (or move to it with the arrow keys / pad); it comes to the middle of the screen,
a little bigger. Clicking beside the games, or Backspace / Tab / Esc / Q, puts it back down (before Tab changes
the layout or Esc / Q quits); with nothing picked up, Space does nothing. The wall cannot be zoomed in past the
point where a whole row fills the screen.

`table` lays the games face up in close rows on a table turned at an angle; the selection lifts off towards you
(Up / Down move a row, like the wall). `stand_up` is a row of games leaning back; the selection stands up.
`spines` stands the games side by side seen straight on, spines only; the selection slides out showing its cover.
`towers` (Tower Stacks) and `messy_towers` (Messy Stacks) are Aurora's stacks: two piles of cases lying flat with
their spines out, left and right and angled towards you; the selection stands in the middle and scrolling (wheel,
Up / Down, vertical drag) moves games from one pile through the middle onto the other. The messy one knocks every
case a little askew, twisting down the pile.
`spiral` and `stacks` are Aurora style: the selection in front with the other games in a twisting spiral or a
straight stack receding on each side.
`shelf` is an Aurora-style shelf at an angle to the viewer: spines out, running off to the left into the
distance (fronts and discs turned towards you), with the selection standing in front of it. The board fades
out at both ends (`[layouts.shelf]` `angle` in degrees, negative puts the shelf on the right; `depth` = how
far out the selection stands; `visible` = games along the shelf).
`stack` is a tower of cases lying flat with their spines out; the selected one slides out and stands up.
`pile` is the same idea as a loose pile seen from above. Both pack items by their real thickness.

Item styles (`display.item_style`): `case`, `disc`, `case_disc`, `flat`.
Each console's banner (lettering in its colours) runs across the top of the cover and spine (`display.platform_banners`). Cases are built like the real thing for each console: two halves that meet at a seam along the edges, a
rounded spine and a thumb groove in the opening edge. The plastic matches the console (translucent blue PS4/PS5,
green Xbox 360/One, clear PS3/PSP, red Switch, white Wii/3DS, black PS2/GameCube/original Xbox). PS1, Saturn
and Dreamcast games come in CD jewel cases (clear lid, coloured tray); NES, SNES, N64 and Game Boy games in
cardboard boxes with an insert. Inside there is a rosette hub holding the disc (or a holder with the
cartridge or UMD) and the manual clipped into the lid.

The printed spine wraps round the spine's rounded edges and meets the front and back covers with no gap.
Spines are real scans from LaunchBox when it has them, otherwise generated from the game's logo and art
(`display.spine_style`); turning one around shows a generated back cover (blurb, screenshots, platform bar).
Discs have a see-through hole and clear hub, a mirror band and a rainbow data side. Cartridges have the
console's shape: the Game Boy's cut corner, the N64's shoulders, DS/3DS/Switch/Vita cards with gold contacts,
and PSP games are UMDs with the disc showing through the back.
Cartridges take the game's own colour (Pokémon Yellow is yellow, Ocarina of Time gold): measured once from
LaunchBox's photo of the cartridge when there is one, else from a built-in list of famous coloured cartridges.

```toml
[display]
case_corners = "real"       # each console's own corner radius; "custom" uses case_corner_radius
case_corner_radius = 0.0    # cm, 0 = sharp
zoom = 1.25                 # default camera distance (ctrl + wheel zooms from there)
open_case_size = 0.66       # how much of the screen an opened case fills
rainbow_cases = false       # every case its own random colour (new each start / each time it is turned on)
case_finger_notch = true
disc_rainbow = 1.0          # 0 = plain mirror
mouse_sensitivity = 1.0
```
Backgrounds (`background.style`): `game_art` (blurred fanart / hero image / screenshot of the selected game),
`aurora`, `waves`, `nebula`, `gradient`, `solid`, `synthwave` (outrun: a thick neon grid that rolls towards you and
slides sideways as you browse, in the selected game's colour near you and a second colour, a third of the way round
the colour wheel, for the horizon, the far grid and the sky).
`ps2` is the PS2 system menu: black, with seven glowing cyan orbs (in the game's colours when
`background.adaptive` is on) trailing light as they swirl in a plane that
tumbles with the clock, after the recreation at imsaud.me. `xbox` is the original Xbox dashboard's green: a very
dark green with faint smoke, glowing waves and a soft glow, under a dense, bright green, dome-warped diamond lattice, with the
pulsing green blob in the middle inside a large sphere of hexagon lattice (turned to the game's colour when
`background.adaptive` is on). Both keep their console's colours. `custom` shows a picture or animated GIF of your own over the whole screen (Settings > Background > Choose a
background picture or GIF..., or `background.custom_path`), for example a capture of a console's real menu. `galactic` is the cosmic web: undulating filaments between dark voids, crowded with stars, with bright clusters
where they meet. `aurora` is tall curtains of northern lights with drifting rays over a dark ridge. All of
them take the selected game's colours when `background.adaptive` is on. With it off they use your own three colours
(`background.color_a` dark, `color_b` main, `color_c` accent), set in Settings > Background with a colour picker
(drag in the square for saturation / brightness and along the bar for hue, or click a preset; the background
changes as you pick, Enter / OK keeps it, Esc / Cancel puts the old colour back). Picking a colour turns
"Colours follow the cover" off so it shows.

Launching plays an animation first. With the case styles the case turns to face you, opens on its hinges
and the disc or cartridge is lifted out and carried off as if into the console (`launch.open_case`,
`launch.open_duration`). Otherwise `[launch]` `style` = `zoom`, `spin`, `pulse`, `rise` or `none` is used,
plus `duration`, `flash`, `flash_color`, `recede`.

Every layout has its own tuning table:

```toml
[layouts.cover_flow]
spacing = 6.0
angle = 62.0
center_gap = 9.0
depth = 9.0
cam_distance = 38.0
cam_height = 2.0
visible = 9.0
```

Sizes are real centimetres, per platform, and can be overridden:

```toml
[platforms.gba]
command = "mgba-qt {rom}"
case_size = [7.5, 14.2, 1.8]
accent = "#4b3c8c"        # spine and placeholder colour
case_color = "#0a4bc4cc"  # case plastic; the last two digits (optional) make it translucent
```

## Where the data comes from

All lookups run on background threads and are cached on disk.

| Source (`covers.sources`) | Needs key | Used for |
|---|---|---|
| `steam` | no | Steam's local library art cache, the store API (current portrait art), the CDN; store search finds art for PC games from other launchers |
| `launchbox` | no | LaunchBox Games Database: box art, real spines and back covers, disc labels, logos, fanart (one ~110 MB download a month) |
| `libretro` | no | console box art, fuzzy matched against the libretro-thumbnails index (also screenshots for backs) |
| `wikipedia` | no | box art from the game's article, plus descriptions for non-Steam games |
| `steamgriddb` | free API key | optional, best coverage for non-Steam games |
| `thegamesdb` | API key | optional, console and PC box art |
| Steam store API | no | description, genres, release year, screenshots for backs |

Sources are tried in order. An image that does not fit the case shape (like a wide Steam banner) is only used
when nothing better turns up, and is then letterboxed over a blurred copy of itself instead of being cropped.
Badly shaped cached covers are re-checked against the sources once a week.

Order and keys: `[covers]` in `config.toml`. You can also set `cover_path` on any game in `library.json`.
Games with no cover get a generated placeholder with the title on it.

## Files

`slipcase config` prints the locations. Set `SLIPCASE_HOME=./data` for a portable install.

- `config.toml`: hot-reloaded while running, edit it and watch the launcher change
- `library.json`: your library, favourites and play counts
- `cache/covers/`: downloaded art

Spines and placeholder covers use a system font. Put a `font.ttf` next to `config.toml` to choose your own.

## Static builds

`build.sh` and `build.ps1` produce a single binary with all Rust code, TLS, image codecs and the engine linked in.
The Windows build also links the C runtime statically (no VC++ redistributable).
A desktop 3D app can never be 100% static: GPU drivers and X11/Wayland are loaded at run time by design.
On Linux, `./build.sh linux-portable` builds against an old glibc inside Docker so the binary runs on most distros.

Build requirements: Rust 1.82+. On Linux: `pkg-config`, `libudev-dev` (controllers) and `libwayland-dev libxkbcommon-dev` (Wayland).
To skip them: `cargo build --release --no-default-features` builds an X11-only, no-controller binary.

## Limitations

- Changing `general.bloom` needs a restart (HDR is chosen at startup).
- Epic detection reads the Windows launcher manifests; on Linux use Heroic.
- Default emulator commands are guesses; set `[platforms.<id>] command` for the emulators you use.

## Credits

The emoji beside the settings categories are from [Twemoji](https://github.com/jdecked/twemoji) (graphics by
Twitter, Inc. and other contributors), licensed under [CC-BY 4.0](https://creativecommons.org/licenses/by/4.0/).
