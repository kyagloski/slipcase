//! The interface font (Settings > Screen > Font). The bundled fonts are all SIL Open Font
//! License (their licences are next to them in src/fonts). Every bit of interface text gets the
//! chosen font: new or changed text as it appears, and all of it when the setting changes.
//! Only the chosen font is ever loaded; "default" is Bevy's built-in one.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::ui::UiSystem;

use crate::config::Config;

/// Settings name, file. Shown in this order in the settings.
pub const FONTS: &[(&str, &[u8])] = &[
    ("default", &[]),
    ("minecraft", include_bytes!("fonts/Monocraft.ttf")),
    ("arcade", include_bytes!("fonts/PressStart2P-Regular.ttf")),
    ("terminal", include_bytes!("fonts/VT323-Regular.ttf")),
    ("tiny pixel", include_bytes!("fonts/Silkscreen-Regular.ttf")),
    ("sci-fi", include_bytes!("fonts/Orbitron.ttf")),
    ("retro future", include_bytes!("fonts/Audiowide-Regular.ttf")),
    ("signage", include_bytes!("fonts/Bungee-Regular.ttf")),
];

pub struct FontsPlugin;

impl Plugin for FontsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, apply_font.before(UiSystem::Layout).before(bevy::ui::widget::measure_text_system));
    }
}

/// The file of a bundled font by name, or of a .ttf / .otf path; None for "default" (or a
/// file that cannot be read).
pub fn font_bytes(name: &str) -> Option<Vec<u8>> {
    match FONTS.iter().find(|(n, _)| *n == name) {
        Some((_, b)) if !b.is_empty() => Some(b.to_vec()),
        Some(_) => None,
        None => std::fs::read(name.trim()).ok(),
    }
}

/// The fonts a text was given by the code that made it (one per section), kept while it wears
/// the picked font so "default" can put them back (the menus use a regular and a bold face).
#[derive(Component)]
struct BaseFonts(Vec<Handle<Font>>);

#[allow(clippy::type_complexity)]
fn apply_font(
    mut commands: Commands,
    cfg: Res<Config>,
    mut fonts: ResMut<Assets<Font>>,
    mut loaded: Local<HashMap<String, Option<Handle<Font>>>>,
    mut applied: Local<Option<String>>,
    // Every font this has loaded: never mistaken for one the code gave a text.
    mut ours: Local<std::collections::HashSet<AssetId<Font>>>,
    mut texts: Query<(Entity, &mut Text, Option<&mut BaseFonts>)>,
) {
    let name = &cfg.general.font;
    let custom = loaded
        .entry(name.clone())
        .or_insert_with(|| font_bytes(name).and_then(|b| Font::try_from_bytes(b).ok()).map(|f| fonts.add(f)))
        .clone();
    if let Some(c) = &custom {
        ours.insert(c.id());
    }
    let fresh = applied.as_ref() != Some(name);
    if fresh {
        *applied = Some(name.clone());
        // Fonts no longer picked are let go (and their glyph caches with them).
        loaded.retain(|n, _| n == name);
    }
    for (e, mut t, base) in &mut texts {
        // With the default font, text that never wore another costs nothing.
        if !(fresh || t.is_changed()) || (custom.is_none() && base.is_none()) {
            continue;
        }
        // What the code asked for: every section not wearing the picked font.
        let mut b = base.as_ref().map(|b| b.0.clone()).unwrap_or_default();
        b.truncate(t.sections.len());
        for (i, sec) in t.sections.iter().enumerate() {
            let ours = ours.contains(&sec.style.font.id());
            if i >= b.len() {
                let f = if ours { b.first().cloned().unwrap_or_default() } else { sec.style.font.clone() };
                b.push(f);
            } else if !ours {
                b[i] = sec.style.font.clone();
            }
        }
        if t.sections.iter().zip(&b).any(|(sec, bf)| sec.style.font != *custom.as_ref().unwrap_or(bf)) {
            for (sec, bf) in t.sections.iter_mut().zip(&b) {
                sec.style.font = custom.clone().unwrap_or_else(|| bf.clone());
            }
        }
        match (custom.is_some(), base) {
            (false, Some(_)) => {
                commands.entity(e).remove::<BaseFonts>();
            }
            (true, Some(mut base)) => {
                if base.0 != b {
                    base.0 = b;
                }
            }
            (true, None) => {
                commands.entity(e).insert(BaseFonts(b));
            }
            (false, None) => {}
        }
    }
}
