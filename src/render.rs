//! Builds the 3D objects: game cases, discs, cartridges and flat covers,
//! plus the procedurally generated spines and placeholder covers.

use std::collections::{HashMap, HashSet};

use bevy::asset::load_internal_asset;
use bevy::math::Affine2;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::mesh::Indices;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{AsBindGroup, Extent3d, PrimitiveTopology, ShaderRef, ShaderType, TextureDimension, TextureFormat};
use bevy::render::texture::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use image::RgbaImage;

use crate::config::{parse_hex, Config, ItemStyle};
use crate::library::Game;
use crate::platform::{CartShape, CaseKind, CaseLook, Geometry, Media, Platform, UMD_SIZE};

// ------------------------------------------------------------------------------------------------
// Geometry helpers
// ------------------------------------------------------------------------------------------------

pub fn geometry(cfg: &Config, p: Platform) -> Geometry {
    let mut g = p.geometry();
    if let Some(o) = cfg.platform_override(p) {
        if let Some(s) = o.case_size {
            g.case = (s[0], s[1], s[2]);
        }
        if let Some(d) = o.disc_diameter {
            g.media = Media::Disc { diameter: d };
        }
    }
    if cfg.display.uniform_sizes {
        g.case = (13.5, 19.0, 1.4);
    }
    g
}

pub fn accent(cfg: &Config, p: Platform) -> [f32; 3] {
    cfg.platform_override(p).and_then(|o| o.accent.as_deref()).map(parse_hex).unwrap_or_else(|| p.accent())
}

/// Bounding dimensions used by the layouts.
pub fn item_dims(cfg: &Config, p: Platform, style: ItemStyle) -> Vec3 {
    let g = geometry(cfg, p);
    match style {
        ItemStyle::Case | ItemStyle::CaseDisc => Vec3::new(g.case.0, g.case.1, g.case.2),
        ItemStyle::Flat => Vec3::new(g.case.0, g.case.1, 0.05),
        ItemStyle::Disc => media_size(g.media),
    }
}

// ------------------------------------------------------------------------------------------------
// Components & caches
// ------------------------------------------------------------------------------------------------

pub struct CoverSlot {
    pub mat: Handle<StandardMaterial>,
    /// width / height of the area on the object this slot covers.
    pub aspect: f32,
    /// Which art goes here: the cover, or (when there is some) the disc / cartridge label's or
    /// the insert's own art.
    pub kind: ArtKind,
}

/// The printed surfaces of a game that can each have their own art.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ArtKind {
    Cover,
    /// Disc, UMD or cartridge label.
    Media,
    /// The manual / insert in the lid.
    Insert,
    /// The back cover.
    Back,
    /// The spine label.
    Spine,
}

#[derive(Component)]
pub struct Item {
    pub game_id: String,
    pub view_index: usize,
    pub dims: Vec3,
    pub covers: Vec<CoverSlot>,
    /// The insert's printed pages: inside of the cover, then front and back of each sheet.
    pub pages: Vec<CoverSlot>,
    /// Back cover (cases and flat covers only), filled in when the item is turned around.
    pub back: Option<CoverSlot>,
    /// Spine label (cases only), replaced by a stylised spine when one is available.
    pub spine: Option<CoverSlot>,
    /// Hinges and media of a case (case styles only).
    pub rig: Option<CaseRig>,
    /// Soft outline round the item (a group of shells), shown while it is under the mouse.
    pub outline: Entity,
    /// Disc-shaped (mouse picking ignores the corners of its bounding box).
    pub round: bool,
}

/// Edge fade of an item. While `value` < 1 the item's parts use their own see-through copies
/// of their materials (`copies`: part, original, copy); at 1 the shared originals come back.
#[derive(Component, Default)]
pub struct Fade {
    pub value: f32,
    pub applied: f32,
    pub std: Vec<(Entity, Handle<StandardMaterial>, Handle<StandardMaterial>)>,
    pub disc: Vec<(Entity, Handle<DiscMaterial>, Handle<DiscMaterial>)>,
}

/// Marks an item's outline group.
#[derive(Component)]
pub struct Outline;

/// Shells per outline. Each is a little larger and faint, so together they fade outwards.
const OUTLINE_LAYERS: usize = 4;
/// Opacity of one outline shell.
pub const OUTLINE_ALPHA: f32 = 0.3;

#[derive(Resource)]
pub struct SpineFont(pub Option<fontdue::Font>);

#[derive(Resource, Default)]
pub struct Gfx {
    meshes: HashMap<String, Handle<Mesh>>,
    plastic: HashMap<String, Handle<StandardMaterial>>,
    clear: HashMap<String, Handle<StandardMaterial>>,
    disc: HashMap<String, Handle<DiscMaterial>>,
    /// Outline material, shared by every shell. Kept across `clear`.
    pub outline: Option<Handle<StandardMaterial>>,
    /// Console banner materials (front, spine) per platform and case size.
    banners: HashMap<String, (Handle<StandardMaterial>, Handle<StandardMaterial>)>,
    /// Random seed for "rainbow cases" (0 = pick one on first use); new every start and
    /// every time the setting is turned on.
    pub rainbow_seed: u64,
    /// "Not installed" badge (kept across `clear`).
    badge: Option<Handle<StandardMaterial>>,
}

impl Gfx {
    fn mesh(&mut self, key: String, meshes: &mut Assets<Mesh>, make: impl FnOnce() -> Mesh) -> Handle<Mesh> {
        self.meshes.entry(key).or_insert_with(|| meshes.add(make())).clone()
    }
    /// Slightly transparent polycarbonate, like the clamping area around a CD's hole.
    fn clear_plastic(&mut self, mats: &mut Assets<StandardMaterial>, alpha: f32, roughness: f32) -> Handle<StandardMaterial> {
        self.clear
            .entry(format!("{alpha:.2}/{roughness:.2}"))
            .or_insert_with(|| {
                mats.add(StandardMaterial {
                    base_color: Color::srgba(0.62, 0.66, 0.72, alpha),
                    alpha_mode: AlphaMode::Blend,
                    perceptual_roughness: roughness,
                    reflectance: 0.35,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })
            })
            .clone()
    }
    fn disc(&mut self, mats: &mut Assets<DiscMaterial>, key: &str, rainbow: f32, mirror: f32) -> Handle<DiscMaterial> {
        self.disc
            .entry(format!("{key}/{rainbow:.3}"))
            .or_insert_with(|| mats.add(disc_material(rainbow, mirror, Color::srgb(0.8, 0.82, 0.86), 0.16)))
            .clone()
    }
    /// See-through plastic with the disc's rainbow sheen (the stacking ring round the hole).
    fn disc_clear(&mut self, mats: &mut Assets<DiscMaterial>, rainbow: f32) -> Handle<DiscMaterial> {
        self.disc
            .entry(format!("clear/{rainbow:.3}"))
            .or_insert_with(|| {
                let mut m = disc_material(rainbow, 0.5, Color::srgba(0.82, 0.85, 0.9, 0.55), 0.12);
                m.base.alpha_mode = AlphaMode::Blend;
                m.base.double_sided = true;
                m.base.cull_mode = None;
                mats.add(m)
            })
            .clone()
    }
    /// Drop cached meshes/materials (used after a config reload that changes sizes).
    pub fn clear(&mut self) {
        self.meshes.clear();
        self.plastic.clear();
        self.clear.clear();
        self.disc.clear();
        self.banners.clear();
    }
}

// ------------------------------------------------------------------------------------------------
// Images
// ------------------------------------------------------------------------------------------------

pub fn make_image(w: u32, h: u32, rgba: Vec<u8>, mips: bool) -> Image {
    let mut data = rgba.clone();
    let mut levels = 1;
    if mips {
        if let Some(mut cur) = RgbaImage::from_raw(w, h, rgba.clone()) {
            let (mut cw, mut ch) = (w, h);
            while cw > 1 && ch > 1 && levels < 9 {
                cw = (cw / 2).max(1);
                ch = (ch / 2).max(1);
                cur = image::imageops::resize(&cur, cw, ch, image::imageops::FilterType::Triangle);
                data.extend_from_slice(cur.as_raw());
                levels += 1;
            }
        }
    }
    let mut img = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.data = data;
    img.texture_descriptor.mip_level_count = levels;
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    img
}

pub fn load_font() -> Option<fontdue::Font> {
    font_bytes().and_then(|b| fontdue::Font::from_bytes(b, fontdue::FontSettings::default()).ok())
}

/// The bold font used for spines and menu headings: `SLIPCASE_FONT`, `font.ttf` next to the
/// config, or a bold system font.
pub fn font_bytes() -> Option<Vec<u8>> {
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("SLIPCASE_FONT") {
        paths.push(p.into());
    }
    paths.push(crate::config::config_dir().join("font.ttf"));
    for p in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/google-noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/truetype/ubuntu/Ubuntu-B.ttf",
        r"C:\Windows\Fonts\segoeuib.ttf",
        r"C:\Windows\Fonts\arialbd.ttf",
        r"C:\Windows\Fonts\verdanab.ttf",
    ] {
        paths.push(p.into());
    }
    for p in paths {
        if let Ok(bytes) = std::fs::read(&p) {
            if fontdue::Font::from_bytes(bytes.as_slice(), fontdue::FontSettings::default()).is_ok() {
                return Some(bytes);
            }
        }
    }
    None
}

pub fn text_width(font: &fontdue::Font, s: &str, px: f32) -> f32 {
    s.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

pub fn draw_text(img: &mut RgbaImage, font: &fontdue::Font, s: &str, px: f32, x: f32, baseline: f32, color: [u8; 3]) {
    let mut pen = x;
    for c in s.chars() {
        let (m, bitmap) = font.rasterize(c, px);
        let gx = (pen + m.xmin as f32).round() as i32;
        let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
        for row in 0..m.height {
            for col in 0..m.width {
                let cov = bitmap[row * m.width + col] as f32 / 255.0;
                if cov <= 0.0 {
                    continue;
                }
                let (tx, ty) = (gx + col as i32, gy + row as i32);
                if tx < 0 || ty < 0 || tx >= img.width() as i32 || ty >= img.height() as i32 {
                    continue;
                }
                let px_ = img.get_pixel_mut(tx as u32, ty as u32);
                for k in 0..3 {
                    px_.0[k] = (px_.0[k] as f32 * (1.0 - cov) + color[k] as f32 * cov) as u8;
                }
            }
        }
        pen += m.advance_width;
    }
}

pub fn fit_text(font: &fontdue::Font, s: &str, px: f32, max_w: f32) -> String {
    if text_width(font, s, px) <= max_w {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if text_width(font, &format!("{out}{c}.."), px) > max_w {
            break;
        }
        out.push(c);
    }
    format!("{out}..")
}

/// Shortens a string to `n` characters with an ellipsis.
pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(n.saturating_sub(3)).collect::<String>())
    }
}

/// Greedy word wrap to `max_w` pixels.
pub fn wrap_text(font: &fontdue::Font, s: &str, px: f32, max_w: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if text_width(font, &trial, px) > max_w && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        } else {
            cur = trial;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn shade(c: [f32; 3], k: f32) -> [u8; 3] {
    [(c[0] * k * 255.0).min(255.0) as u8, (c[1] * k * 255.0).min(255.0) as u8, (c[2] * k * 255.0).min(255.0) as u8]
}

/// Spine texture: accent colour, platform tag at the top, title reading downwards.
/// With `banner`, the top of the spine is left for the console banner instead of the darker
/// platform tag.
pub fn make_spine(title: &str, platform: &str, accent: [f32; 3], font: Option<&fontdue::Font>, len_cm: f32, thick_cm: f32, banner: bool) -> Image {
    let l: u32 = 1024;
    let t = ((thick_cm / len_cm) * l as f32).clamp(28.0, 320.0) as u32;
    let mut canvas = RgbaImage::new(l, t);
    for y in 0..t {
        let g = 0.65 + 0.35 * (1.0 - ((y as f32 / t as f32) - 0.5).abs() * 2.0);
        let c = shade(accent, 0.9 * g);
        for x in 0..l {
            canvas.put_pixel(x, y, image::Rgba([c[0], c[1], c[2], 255]));
        }
    }
    // darker tag area at the top of the spine
    let tag = (l as f32 * if banner { 0.12 } else { 0.17 }) as u32;
    if !banner {
        for y in 0..t {
            for x in 0..tag {
                let p = canvas.get_pixel_mut(x, y);
                for k in 0..3 {
                    p.0[k] = (p.0[k] as f32 * 0.55) as u8;
                }
            }
        }
    }
    if let Some(f) = font {
        let px = (t as f32 * 0.52).max(12.0);
        let base = t as f32 * 0.5 + px * 0.34;
        if !banner {
            let small = px * 0.55;
            let tag_text = fit_text(f, platform, small, tag as f32 - 24.0);
            draw_text(&mut canvas, f, &tag_text, small, 14.0, t as f32 * 0.5 + small * 0.34, [235, 235, 235]);
        }
        let max_w = (l - tag) as f32 - 28.0;
        let title = fit_text(f, title, px, max_w);
        draw_text(&mut canvas, f, &title, px, tag as f32 + 18.0, base, [255, 255, 255]);
    }
    let rotated = image::imageops::rotate90(&canvas);
    let (w, h) = rotated.dimensions();
    make_image(w, h, rotated.into_raw(), true)
}

/// Used when no online cover exists: gradient with wrapped title and platform.
pub fn make_placeholder(title: &str, platform: &str, accent: [f32; 3], font: Option<&fontdue::Font>) -> Image {
    let (w, h) = (360u32, 540u32);
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        let t = y as f32 / h as f32;
        let k = 1.05 - t * 0.7;
        let c = shade(accent, k.max(0.2) * 1.4);
        for x in 0..w {
            let v = 1.0 - ((x as f32 / w as f32) - 0.5).abs() * 0.35;
            img.put_pixel(x, y, image::Rgba([(c[0] as f32 * v) as u8, (c[1] as f32 * v) as u8, (c[2] as f32 * v) as u8, 255]));
        }
    }
    for y in 0..22 {
        for x in 0..w {
            img.put_pixel(x, y, image::Rgba([255, 255, 255, 40]));
        }
    }
    if let Some(f) = font {
        let px = 38.0;
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for word in title.split_whitespace() {
            let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if text_width(f, &trial, px) > w as f32 - 48.0 && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            } else {
                cur = trial;
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines.truncate(6);
        let total = lines.len() as f32 * px * 1.2;
        let mut y = (h as f32 - total) * 0.45 + px;
        for l in lines {
            let l = fit_text(f, &l, px, w as f32 - 48.0);
            let x = (w as f32 - text_width(f, &l, px)) * 0.5;
            draw_text(&mut img, f, &l, px, x, y, [255, 255, 255]);
            y += px * 1.2;
        }
        let small = 22.0;
        let x = (w as f32 - text_width(f, platform, small)) * 0.5;
        draw_text(&mut img, f, platform, small, x, h as f32 - 28.0, [235, 235, 235]);
    }
    let (w, h) = img.dimensions();
    make_image(w, h, img.into_raw(), true)
}

/// UV transform that crops an image to cover a target area without stretching.
pub fn crop_transform(img_aspect: f32, target_aspect: f32) -> Affine2 {
    let (mut su, mut sv) = (1.0, 1.0);
    if img_aspect > target_aspect {
        su = target_aspect / img_aspect;
    } else {
        sv = img_aspect / target_aspect;
    }
    Affine2::from_scale_angle_translation(Vec2::new(su, sv), 0.0, Vec2::new((1.0 - su) * 0.5, (1.0 - sv) * 0.5))
}

pub fn apply_texture(mat: &mut StandardMaterial, tex: Handle<Image>, img_w: u32, img_h: u32, target_aspect: f32) {
    mat.base_color = Color::WHITE;
    mat.base_color_texture = Some(tex);
    mat.uv_transform = crop_transform(img_w as f32 / img_h.max(1) as f32, target_aspect);
}

// ------------------------------------------------------------------------------------------------
// Meshes
// ------------------------------------------------------------------------------------------------

fn build_mesh(pos: Vec<[f32; 3]>, nor: Vec<[f32; 3]>, uv: Vec<[f32; 2]>, tangents: Option<Vec<[f32; 4]>>, idx: Vec<u32>) -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    if let Some(t) = tangents {
        m.insert_attribute(Mesh::ATTRIBUTE_TANGENT, t);
    }
    m.insert_indices(Indices::U32(idx));
    m
}

/// Flat ring in the XY plane at height `z`, facing +Z (`up`) or -Z.
/// UVs map the full texture onto a circle of radius `uv_r`, so a cover image shows
/// through the ring like a printed disc label. Tangents point radially outwards
/// (the grating direction used by the disc shader).
fn ring(inner: f32, outer: f32, z: f32, up: bool, uv_r: f32, segments: usize) -> Mesh {
    let (mut pos, mut nor, mut uv, mut tan, mut idx) = (vec![], vec![], vec![], vec![], vec![]);
    let nz = if up { 1.0 } else { -1.0 };
    for i in 0..=segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        for r in [inner, outer] {
            pos.push([c * r, s * r, z]);
            nor.push([0.0, 0.0, nz]);
            // Seen from below the label would be mirrored; flip u so it reads correctly.
            let u = 0.5 + 0.5 * c * r / uv_r * nz;
            uv.push([u, 0.5 - 0.5 * s * r / uv_r]);
            tan.push([c, s, 0.0, 1.0]);
        }
    }
    for i in 0..segments as u32 {
        let b = i * 2;
        if up {
            idx.extend_from_slice(&[b, b + 1, b + 3, b, b + 3, b + 2]);
        } else {
            idx.extend_from_slice(&[b, b + 3, b + 1, b, b + 2, b + 3]);
        }
    }
    build_mesh(pos, nor, uv, Some(tan), idx)
}

/// Open cylinder wall of radius `r` between `z0` and `z1`, normals facing out or in.
fn tube(r: f32, z0: f32, z1: f32, outward: bool, segments: usize) -> Mesh {
    let (mut pos, mut nor, mut uv, mut idx) = (vec![], vec![], vec![], vec![]);
    let k = if outward { 1.0 } else { -1.0 };
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let (s, c) = (t * std::f32::consts::TAU).sin_cos();
        for z in [z0, z1] {
            pos.push([c * r, s * r, z]);
            nor.push([c * k, s * k, 0.0]);
            uv.push([t, if z == z0 { 0.0 } else { 1.0 }]);
        }
    }
    for i in 0..segments as u32 {
        let (a, d, b, c) = (i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 3);
        if outward {
            idx.extend_from_slice(&[a, b, c, a, c, d]);
        } else {
            idx.extend_from_slice(&[a, c, b, a, d, c]);
        }
    }
    build_mesh(pos, nor, uv, None, idx)
}

/// Outward vertex normals of a closed CCW outline.
fn outline_normals(pts: &[Vec2]) -> Vec<Vec2> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let prev = pts[(i + n - 1) % n];
            let next = pts[(i + 1) % n];
            let e0 = (pts[i] - prev).normalize_or_zero();
            let e1 = (next - pts[i]).normalize_or_zero();
            let n0 = Vec2::new(e0.y, -e0.x);
            let n1 = Vec2::new(e1.y, -e1.x);
            (n0 + n1).normalize_or(n1)
        })
        .collect()
}

/// Flat polygon (fan from the origin, so the outline must be star-shaped around it)
/// with UVs spanning its `w` x `h` bounding box. `front` faces +Z, otherwise -Z with the
/// texture mirrored so it reads correctly when seen from behind.
fn flat_poly(pts: &[Vec2], z: f32, front: bool, w: f32, h: f32) -> Mesh {
    let (mut pos, mut nor, mut uv, mut idx) = (vec![], vec![], vec![], vec![]);
    let nz = if front { 1.0 } else { -1.0 };
    let to_uv = |p: Vec2| [0.5 + p.x / w * nz, 0.5 - p.y / h];
    pos.push([0.0, 0.0, z]);
    nor.push([0.0, 0.0, nz]);
    uv.push(to_uv(Vec2::ZERO));
    for p in pts {
        pos.push([p.x, p.y, z]);
        nor.push([0.0, 0.0, nz]);
        uv.push(to_uv(*p));
    }
    let n = pts.len() as u32;
    for j in 0..n {
        let (a, b) = (1 + j, 1 + (j + 1) % n);
        if front {
            idx.extend_from_slice(&[0, a, b]);
        } else {
            idx.extend_from_slice(&[0, b, a]);
        }
    }
    build_mesh(pos, nor, uv, None, idx)
}

/// Thumb groove pressed into the right (+X) side wall, centred on the seam between the
/// two halves of the case: (length along the edge, depth into the wall), in cm.
#[derive(Clone, Copy)]
struct Groove {
    len: f32,
    depth: f32,
}

/// Extrudes an outline into a solid slab `d` thick whose front and back edges are
/// rounded over with radius `bevel` (smooth normals, so it catches light like moulded plastic).
fn rounded_slab(pts: &[Vec2], d: f32, bevel: f32, groove: Option<Groove>) -> Mesh {
    let n = pts.len();
    let normals = outline_normals(pts);
    let br_max = bevel.clamp(0.0, d * 0.45);
    // Bevel radius at each outline vertex.
    let brs: Vec<f32> = vec![br_max; n];
    let k = 5;
    // Profile rings from the back face round to the front face, as functions of the local
    // bevel radius: (inset, z, radial normal scale, z normal).
    type Ring = Box<dyn Fn(f32) -> (f32, f32, f32, f32)>;
    let mut prof: Vec<Ring> = Vec::new();
    for i in 0..=k {
        let a = -std::f32::consts::FRAC_PI_2 * (1.0 - i as f32 / k as f32);
        prof.push(Box::new(move |br: f32| (br * (1.0 - a.cos()), -(d * 0.5 - br) + br * a.sin(), a.cos(), a.sin())));
    }
    // Straight middle of the side wall, subdivided so the groove can be shaped.
    let mid = if groove.is_some() { 12 } else { 0 };
    for i in 1..mid {
        let t = i as f32 / mid as f32;
        prof.push(Box::new(move |br: f32| (0.0, -(d * 0.5 - br) + (d - 2.0 * br) * t, 1.0, 0.0)));
    }
    for i in 0..=k {
        let a = std::f32::consts::FRAC_PI_2 * i as f32 / k as f32;
        prof.push(Box::new(move |br: f32| (br * (1.0 - a.cos()), (d * 0.5 - br) + br * a.sin(), a.cos(), a.sin())));
    }
    // How far the groove pushes a side-wall point in (0 outside the groove).
    let half_w = d * 0.4;
    let groove_at = |p: Vec2, nrm: Vec2, z: f32| -> f32 {
        let Some(g) = groove else { return 0.0 };
        if nrm.x < 0.99 || p.x <= 0.0 {
            return 0.0;
        }
        let u = (2.0 * p.y / g.len).abs();
        let v = (z / half_w.max(0.01)).abs();
        if u >= 1.0 || v >= 1.0 {
            return 0.0;
        }
        // Rounded ends along the edge, rounded bottom across it.
        let along = (1.0 - u.powi(6)).sqrt();
        let across = (std::f32::consts::FRAC_PI_2 * v).cos().powi(2);
        g.depth * along * across
    };
    let (mut pos, mut nor, mut uv, mut idx) = (vec![], vec![], vec![], vec![]);
    let rings = prof.len();
    let mut grooved: Vec<usize> = Vec::new();
    for (ri, ring) in prof.iter().enumerate() {
        for i in 0..=n {
            let j = i % n;
            let (ins, z, c, s) = ring(brs[j]);
            let g = groove_at(pts[j], normals[j], z);
            if g > 0.0 {
                grooved.push(pos.len());
            }
            let p = pts[j] - normals[j] * (ins + g);
            pos.push([p.x, p.y, z]);
            nor.push([normals[j].x * c, normals[j].y * c, s]);
            uv.push([i as f32 / n as f32, ri as f32 / (rings - 1) as f32]);
        }
    }
    // Recompute normals inside the groove (and around it) from the shaped surface.
    let stride_u = n + 1;
    let mut fix: HashSet<usize> = HashSet::new();
    for &v in &grooved {
        for dr in [-1i32, 0, 1] {
            for di in [-1i32, 0, 1] {
                let r = (v / stride_u) as i32 + dr;
                let i = (v % stride_u) as i32 + di;
                if r > 0 && (r as usize) < rings - 1 && i > 0 && (i as usize) < n {
                    fix.insert(r as usize * stride_u + i as usize);
                }
            }
        }
    }
    for v in fix {
        let at = |k: usize| Vec3::from(pos[k]);
        let along = at(v + 1) - at(v - 1);
        let across = at(v + stride_u) - at(v - stride_u);
        let mut nn = along.cross(across).normalize_or_zero();
        if nn.dot(Vec3::from(nor[v])) < 0.0 {
            nn = -nn;
        }
        if nn != Vec3::ZERO {
            nor[v] = nn.into();
        }
    }
    let stride = (n + 1) as u32;
    for r in 0..(rings - 1) as u32 {
        for i in 0..n as u32 {
            let a = r * stride + i;
            let (b, c, d) = (a + 1, a + 1 + stride, a + stride);
            idx.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    let cap: Vec<Vec2> = pts.iter().zip(&normals).zip(&brs).map(|((p, nn), b)| *p - *nn * *b).collect();
    let (w, h) = (2.0 * pts.iter().map(|p| p.x.abs()).fold(0.0, f32::max), 2.0 * pts.iter().map(|p| p.y.abs()).fold(0.0, f32::max));
    for front in [true, false] {
        let z = if front { d * 0.5 } else { -d * 0.5 };
        let m = flat_poly(&cap, z, front, w, h);
        let base = pos.len() as u32;
        if let (Some(p), Some(nn), Some(u), Some(Indices::U32(ii))) = (
            m.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()),
            m.attribute(Mesh::ATTRIBUTE_NORMAL).and_then(|a| a.as_float3()),
            m.attribute(Mesh::ATTRIBUTE_UV_0),
            m.indices(),
        ) {
            pos.extend_from_slice(p);
            nor.extend_from_slice(nn);
            if let bevy::render::mesh::VertexAttributeValues::Float32x2(u) = u {
                uv.extend_from_slice(u);
            }
            idx.extend(ii.iter().map(|i| i + base));
        }
    }
    build_mesh(pos, nor, uv, None, idx)
}

/// Mesh builder that orients each triangle by the normal it is meant to have, so the
/// face-by-face case builder below cannot get its winding wrong.
#[derive(Default)]
struct MeshBuf {
    pos: Vec<[f32; 3]>,
    nor: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

impl MeshBuf {
    fn vert(&mut self, p: Vec3, n: Vec3, uv: Vec2) -> u32 {
        self.pos.push(p.into());
        self.nor.push(n.into());
        self.uv.push(uv.into());
        (self.pos.len() - 1) as u32
    }
    /// Triangles over a `rows` x `cols` grid of vertices starting at index `base`.
    fn grid(&mut self, base: u32, rows: usize, cols: usize) {
        for r in 0..rows - 1 {
            for c in 0..cols - 1 {
                let a = base + (r * cols + c) as u32;
                let (b, d) = (a + 1, a + cols as u32);
                self.tri(a, b, d + 1);
                self.tri(a, d + 1, d);
            }
        }
    }
    fn tri(&mut self, a: u32, b: u32, c: u32) {
        let p = |i: u32| Vec3::from(self.pos[i as usize]);
        let geo = (p(b) - p(a)).cross(p(c) - p(a));
        let want = Vec3::from(self.nor[a as usize]) + Vec3::from(self.nor[b as usize]) + Vec3::from(self.nor[c as usize]);
        if geo.dot(want) >= 0.0 {
            self.idx.extend_from_slice(&[a, b, c]);
        } else {
            self.idx.extend_from_slice(&[a, c, b]);
        }
    }
    fn build(self) -> Mesh {
        build_mesh(self.pos, self.nor, self.uv, None, self.idx)
    }
}

impl MeshBuf {
    fn quad(&mut self, corners: [Vec3; 4], n: Vec3) {
        let i = corners.map(|p| self.vert(p, n, Vec2::ZERO));
        self.tri(i[0], i[1], i[2]);
        self.tri(i[0], i[2], i[3]);
    }
    /// Box of half extents `half`, rotated by `rot` and centred on `c`.
    fn cuboid(&mut self, c: Vec3, half: Vec3, rot: Quat) {
        for axis in 0..3 {
            for s in [-1.0f32, 1.0] {
                let mut n = Vec3::ZERO;
                n[axis] = s;
                let (mut u, mut v) = (Vec3::ZERO, Vec3::ZERO);
                u[(axis + 1) % 3] = half[(axis + 1) % 3];
                v[(axis + 2) % 3] = half[(axis + 2) % 3];
                let f = n * half[axis];
                let pts = [f - u - v, f + u - v, f + u + v, f - u + v].map(|p| c + rot * p);
                self.quad(pts, rot * n);
            }
        }
    }
    /// Cylinder wall around the Z axis through `c`, from `z0` to `z1` above it.
    fn cylinder(&mut self, c: Vec3, r: f32, z0: f32, z1: f32, outward: bool, seg: usize) {
        let base = self.pos.len() as u32;
        let k = if outward { 1.0 } else { -1.0 };
        for i in 0..=seg {
            let (sn, cs) = (i as f32 / seg as f32 * std::f32::consts::TAU).sin_cos();
            for z in [z0, z1] {
                self.vert(c + Vec3::new(cs * r, sn * r, z), Vec3::new(cs * k, sn * k, 0.0), Vec2::ZERO);
            }
        }
        self.grid(base, seg + 1, 2);
    }
    /// Flat ring (or disc, with `r0` = 0) facing +Z (`up`) or -Z.
    fn annulus(&mut self, c: Vec3, r0: f32, r1: f32, z: f32, up: bool, seg: usize) {
        let base = self.pos.len() as u32;
        let n = if up { Vec3::Z } else { Vec3::NEG_Z };
        for i in 0..=seg {
            let (sn, cs) = (i as f32 / seg as f32 * std::f32::consts::TAU).sin_cos();
            for r in [r0, r1] {
                self.vert(c + Vec3::new(cs * r, sn * r, z), n, Vec2::ZERO);
            }
        }
        self.grid(base, seg + 1, 2);
    }
    /// Fan from `center` over a closed outline, all at depth `z`.
    fn fan(&mut self, center: Vec2, pts: &[Vec2], z: f32, n: Vec3) {
        let c = self.vert(center.extend(z), n, Vec2::ZERO);
        let base = self.pos.len() as u32;
        for p in pts {
            self.vert(p.extend(z), n, Vec2::ZERO);
        }
        let k = pts.len() as u32;
        for j in 0..k {
            self.tri(c, base + j, base + (j + 1) % k);
        }
    }
    /// Recomputes normals over a `rows` x `cols` vertex grid from the surface itself,
    /// keeping them on the same side as the normals it was built with.
    fn smooth_grid(&mut self, base: usize, rows: usize, cols: usize) {
        let at = |m: &MeshBuf, r: usize, c: usize| Vec3::from(m.pos[base + r * cols + c]);
        let mut out = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                let along = at(self, r, (c + 1).min(cols - 1)) - at(self, r, c.saturating_sub(1));
                let across = at(self, (r + 1).min(rows - 1), c) - at(self, r.saturating_sub(1), c);
                let old = Vec3::from(self.nor[base + r * cols + c]);
                let mut n = along.cross(across).normalize_or_zero();
                if n.dot(old) < 0.0 {
                    n = -n;
                }
                out.push(if n == Vec3::ZERO || n.dot(old) < 0.2 { old } else { n });
            }
        }
        for (i, n) in out.into_iter().enumerate() {
            self.nor[base + i] = n.into();
        }
    }
}

/// Closed CCW polygon through `corners` (x, y, radius), each corner rounded off.
fn rounded_poly(corners: &[(f32, f32, f32)]) -> Vec<Vec2> {
    let n = corners.len();
    let p = |i: usize| Vec2::new(corners[i % n].0, corners[i % n].1);
    let mut out = Vec::new();
    for i in 0..n {
        let (a, c, b) = (p(i + n - 1), p(i), p(i + 1));
        let r = corners[i].2;
        let (d0, d1) = ((a - c).normalize_or_zero(), (b - c).normalize_or_zero());
        let half = d0.angle_between(d1).abs() * 0.5;
        if r < 1e-3 || half < 1e-3 || half > std::f32::consts::FRAC_PI_2 - 1e-3 {
            out.push(c);
            continue;
        }
        let t = (r / half.tan()).min(0.45 * (a - c).length().min((b - c).length()));
        let r = t * half.tan();
        let center = c + (d0 + d1).normalize() * (r / half.sin());
        let (t0, t1) = (c + d0 * t, c + d1 * t);
        let a0 = (t0 - center).to_angle();
        let mut sweep = (t1 - center).to_angle() - a0;
        while sweep > std::f32::consts::PI {
            sweep -= std::f32::consts::TAU;
        }
        while sweep < -std::f32::consts::PI {
            sweep += std::f32::consts::TAU;
        }
        let seg = 6;
        for k in 0..=seg {
            let ang = a0 + sweep * k as f32 / seg as f32;
            out.push(center + Vec2::from_angle(ang) * r);
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------
// Cases
//
// A closed case has its spine on -X, the opening edge on +X and the front cover facing +Z.
// It is built in three pieces that hinge like a real keep case: the back half (a tray),
// the spine, and the front half (the lid). The halves meet at z = 0.
// ------------------------------------------------------------------------------------------------

/// Half of the gap between the two halves of a closed case.
const SEAM: f32 = 0.014;
/// How far the lip of each half is chamfered back, so the seam reads as a line.
const SEAM_CHAMFER: f32 = 0.04;

/// How far the spine's rounded edges reach onto the front and back.
pub fn case_bevel(kind: CaseKind, cd: f32) -> f32 {
    match kind {
        CaseKind::Keep => (cd * 0.16).clamp(0.06, 0.25),
        CaseKind::Jewel => 0.07,
        CaseKind::Box => 0.04,
    }
}

/// Plastic margin round the printed cover (on the three sides away from the spine).
pub fn case_border(kind: CaseKind) -> f32 {
    match kind {
        CaseKind::Keep => BORDER,
        CaseKind::Jewel => 0.1,
        CaseKind::Box => 0.015,
    }
}

fn case_wall(kind: CaseKind) -> f32 {
    match kind {
        CaseKind::Box => 0.06,
        _ => 0.12,
    }
}

#[derive(Clone, Copy)]
struct CaseDims {
    hw: f32,
    hh: f32,
    hd: f32,
    /// Spine bevel radius; the halves start at `xs` = -hw + br.
    br: f32,
    xs: f32,
    wall: f32,
    /// Corner radius of the two corners on the opening edge.
    rc: f32,
    border: f32,
    groove: Option<Groove>,
}

impl CaseDims {
    fn new(case: (f32, f32, f32), kind: CaseKind, rc: f32, notch: bool) -> Self {
        let (cw, ch, cd) = case;
        let br = case_bevel(kind, cd);
        let groove = (notch && kind == CaseKind::Keep)
            .then(|| Groove { len: (ch * 0.3).clamp(2.0, 6.0).min(ch - 0.6), depth: (cd * 0.18).clamp(0.08, 0.24) });
        Self {
            hw: cw * 0.5,
            hh: ch * 0.5,
            hd: cd * 0.5,
            br,
            xs: -cw * 0.5 + br,
            wall: case_wall(kind),
            rc: rc.clamp(0.0, cw.min(ch) * 0.3),
            border: case_border(kind),
            groove,
        }
    }
    fn key(&self) -> String {
        let g = self.groove.map(|g| (g.len, g.depth)).unwrap_or_default();
        format!("{:.3}/{:.3}/{:.3}/{:.3}/{:.3}/{:.3}/{:.3}/{:.3}", self.hw, self.hh, self.hd, self.br, self.wall, self.rc, g.0, g.1)
    }
    /// Centre of the inside of a half, in x.
    fn inner_cx(&self) -> f32 {
        (self.xs + self.hw - self.wall) * 0.5
    }
}

/// How deep the thumb groove presses into the opening edge at (y, z).
fn groove_depth(g: Option<Groove>, y: f32, z: f32, d: f32) -> f32 {
    let Some(g) = g else { return 0.0 };
    let wall = 0.18f32;
    let u = 1.0 - (2.0 * y / g.len).abs();
    let v = 1.0 - (z / (d * 0.34)).abs();
    let s = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    g.depth * s(u * g.len * 0.5 / (wall * 2.0).min(g.len * 0.25)) * s(v / wall)
}

/// Outline of one half seen from the front, as (point, outward normal): along the top from
/// the spine, round the opening edge, back along the bottom. Corners always get the same
/// number of samples (a sharp corner just repeats its point), so the inside and outside
/// outlines of a wall line up point for point.
fn half_outline(xs: f32, hw: f32, hh: f32, r: f32, ys: &[f32]) -> Vec<(Vec2, Vec2)> {
    let r = r.max(0.0);
    let seg = 8;
    let mut out = vec![(Vec2::new(xs, hh), Vec2::Y)];
    for k in 0..=seg {
        let n = Vec2::from_angle(std::f32::consts::FRAC_PI_2 * (1.0 - k as f32 / seg as f32));
        out.push((Vec2::new(hw - r, hh - r) + n * r, n));
    }
    for &y in ys {
        out.push((Vec2::new(hw, y), Vec2::X));
    }
    for k in 0..=seg {
        let n = Vec2::from_angle(-std::f32::consts::FRAC_PI_2 * k as f32 / seg as f32);
        out.push((Vec2::new(hw - r, -hh + r) + n * r, n));
    }
    out.push((Vec2::new(xs, -hh), Vec2::NEG_Y));
    out
}

/// One half of a case: a panel with a rim round three sides, open towards the seam.
/// Built as the back half (z < 0); `front` mirrors it into the lid.
fn case_half(c: &CaseDims, front: bool) -> Mesh {
    let sz = if front { -1.0 } else { 1.0 };
    let d = c.hd * 2.0;
    // Extra points down the opening edge so the walls can take the groove's shape.
    let ys: Vec<f32> = match c.groove {
        Some(g) => {
            let n = 120;
            let lim = c.hh - c.rc.max(c.wall) - 0.01;
            (0..=n).map(|i| (g.len * 0.5 + 0.1) - (g.len + 0.2) * i as f32 / n as f32).filter(|y| y.abs() < lim).collect()
        }
        None => Vec::new(),
    };
    let outer = half_outline(c.xs, c.hw, c.hh, c.rc, &ys);
    let inner = half_outline(c.xs, c.hw - c.wall, c.hh - c.wall, c.rc - c.wall, &ys);
    let cols = outer.len();
    let mut m = MeshBuf::default();
    let vert = |m: &mut MeshBuf, p: Vec2, z: f32, n: Vec3| m.vert(Vec3::new(p.x, p.y, z * sz), Vec3::new(n.x, n.y, n.z * sz), Vec2::ZERO);
    let gd = |p: Vec2, n: Vec2, z: f32| if n.x > 0.99 { groove_depth(c.groove, p.y, z, d) } else { 0.0 };
    let z_floor = -c.hd + c.wall;

    // Outer wall: from the panel up to a chamfered lip, pressed in by the groove.
    let rows = if c.groove.is_some() { 16 } else { 2 };
    let mut zs: Vec<(f32, f32)> = (0..=rows).map(|i| (-c.hd + (c.hd - SEAM - SEAM_CHAMFER) * i as f32 / rows as f32, 0.0)).collect();
    zs.push((-SEAM, SEAM_CHAMFER * 0.8));
    let base = m.pos.len();
    for &(z, inset) in &zs {
        for &(p, n) in &outer {
            vert(&mut m, p - n * (inset + gd(p, n, z)), z, n.extend(0.0));
        }
    }
    m.smooth_grid(base, zs.len(), cols);
    m.grid(base as u32, zs.len(), cols);

    // The groove bends the inside of the wall too where it goes deeper than the plastic.
    let gi = |p: Vec2, n: Vec2, z: f32| (gd(p + n * c.wall, n, z) - c.wall * 0.6).max(0.0);
    // Lip, facing the other half.
    let base = m.pos.len() as u32;
    for &(p, n) in &outer {
        vert(&mut m, p - n * (SEAM_CHAMFER * 0.8 + gd(p, n, -SEAM)), -SEAM, Vec3::Z);
    }
    for &(p, n) in &inner {
        vert(&mut m, p - n * gi(p, n, -SEAM), -SEAM, Vec3::Z);
    }
    m.grid(base, 2, cols);

    // Inside of the wall, down to the floor.
    let rows_in = if c.groove.is_some() { 10 } else { 1 };
    let base = m.pos.len();
    for i in 0..=rows_in {
        let z = -SEAM + (z_floor + SEAM) * i as f32 / rows_in as f32;
        for &(p, n) in &inner {
            vert(&mut m, p - n * gi(p, n, z), z, (-n).extend(0.0));
        }
    }
    m.smooth_grid(base, rows_in + 1, cols);
    m.grid(base as u32, rows_in + 1, cols);

    // Floor inside, and the outer panel the cover art sits on.
    let flip = |z: f32| z * sz;
    let inner_pts: Vec<Vec2> = inner.iter().map(|(p, _)| *p).collect();
    let outer_pts: Vec<Vec2> = outer.iter().map(|(p, _)| *p).collect();
    m.fan(Vec2::new(c.inner_cx(), 0.0), &inner_pts, flip(z_floor), Vec3::new(0.0, 0.0, sz));
    m.fan(Vec2::new((c.xs + c.hw) * 0.5, 0.0), &outer_pts, flip(-c.hd), Vec3::new(0.0, 0.0, -sz));

    // Cut ends against the spine (seen at the hinge when the case is open).
    let x = c.xs;
    let q = |y0: f32, y1: f32, z0: f32, z1: f32| {
        [Vec3::new(x, y0, flip(z0)), Vec3::new(x, y1, flip(z0)), Vec3::new(x, y1, flip(z1)), Vec3::new(x, y0, flip(z1))]
    };
    m.quad(q(c.hh - c.wall, c.hh, -c.hd, -SEAM), Vec3::NEG_X);
    m.quad(q(-c.hh, -c.hh + c.wall, -c.hd, -SEAM), Vec3::NEG_X);
    m.quad(q(-c.hh + c.wall, c.hh - c.wall, -c.hd, z_floor), Vec3::NEG_X);
    m.build()
}

/// The spine's cross-section in the XZ plane, from the back edge round to the front edge:
/// (point, outward normal, arc length so far). `grow` pushes it outwards.
fn spine_profile(c: &CaseDims, grow: f32) -> Vec<(Vec2, Vec2, f32)> {
    let k = 10;
    let mut pts: Vec<(Vec2, Vec2)> = Vec::new();
    let r = c.br + grow;
    for i in 0..=k {
        let a = -std::f32::consts::FRAC_PI_2 - std::f32::consts::FRAC_PI_2 * i as f32 / k as f32;
        let n = Vec2::from_angle(a);
        pts.push((Vec2::new(c.xs, -c.hd + c.br) + n * r, n));
    }
    for i in 0..=k {
        let a = std::f32::consts::PI - std::f32::consts::FRAC_PI_2 * i as f32 / k as f32;
        let n = Vec2::from_angle(a);
        pts.push((Vec2::new(c.xs, c.hd - c.br) + n * r, n));
    }
    let mut s = 0.0;
    let mut out = Vec::with_capacity(pts.len());
    for i in 0..pts.len() {
        if i > 0 {
            s += pts[i].0.distance(pts[i - 1].0);
        }
        out.push((pts[i].0, pts[i].1, s));
    }
    out
}

/// The spine: rounded outside, flat inside, closed at the ends.
fn case_spine(c: &CaseDims) -> Mesh {
    let prof = spine_profile(c, 0.0);
    let mut m = MeshBuf::default();
    let base = m.pos.len() as u32;
    for &(p, n, _) in &prof {
        for y in [c.hh, -c.hh] {
            m.vert(Vec3::new(p.x, y, p.y), Vec3::new(n.x, 0.0, n.y), Vec2::ZERO);
        }
    }
    m.grid(base, prof.len(), 2);
    let x = c.xs;
    m.quad([Vec3::new(x, -c.hh, -c.hd), Vec3::new(x, c.hh, -c.hd), Vec3::new(x, c.hh, c.hd), Vec3::new(x, -c.hh, c.hd)], Vec3::X);
    for (y, n) in [(c.hh, Vec3::Y), (-c.hh, Vec3::NEG_Y)] {
        let ctr = m.vert(Vec3::new(c.xs - c.br * 0.4, y, 0.0), n, Vec2::ZERO);
        let b = m.pos.len() as u32;
        for &(p, _, _) in &prof {
            m.vert(Vec3::new(p.x, y, p.y), n, Vec2::ZERO);
        }
        let k = prof.len() as u32;
        for j in 0..k {
            m.tri(ctr, b + j, b + (j + 1) % k);
        }
    }
    m.build()
}

/// Spine art wrapped round the whole spine, bevels included, so it meets the front and
/// back art with no gap. u runs from the back edge to the front edge.
fn spine_art(c: &CaseDims) -> Mesh {
    let prof = spine_profile(c, EPS);
    let len = prof.last().map(|p| p.2).unwrap_or(1.0);
    let hh = c.hh - c.border;
    let mut m = MeshBuf::default();
    let base = m.pos.len() as u32;
    for &(p, n, s) in &prof {
        for y in [hh, -hh] {
            m.vert(Vec3::new(p.x, y, p.y), Vec3::new(n.x, 0.0, n.y), Vec2::new(s / len, 0.5 - y / (hh * 2.0)));
        }
    }
    m.grid(base, prof.len(), 2);
    m.build()
}

/// Band across the top of the spine (`top` .. `top - h`), wrapped round the bevels like the
/// spine art and just above it. u runs from the back edge to the front edge.
fn spine_band(c: &CaseDims, top: f32, h: f32) -> Mesh {
    let prof = spine_profile(c, EPS * 2.0);
    let len = prof.last().map(|p| p.2).unwrap_or(1.0);
    let mut m = MeshBuf::default();
    let base = m.pos.len() as u32;
    for &(p, n, s) in &prof {
        for (y, v) in [(top, 0.0), (top - h, 1.0)] {
            m.vert(Vec3::new(p.x, y, p.y), Vec3::new(n.x, 0.0, n.y), Vec2::new(s / len, v));
        }
    }
    m.grid(base, prof.len(), 2);
    m.build()
}

/// A console banner as an image `w` x `h` pixels: the band colour, the lettering centred and
/// as large as fits, and the accent line along the bottom.
fn banner_image(b: &crate::platform::Banner, font: Option<&fontdue::Font>, w: u32, h: u32) -> Image {
    let rgb = |c: u32| [(c >> 16) as u8, (c >> 8) as u8, c as u8];
    let (bg, fg) = (rgb(b.bg), rgb(b.fg));
    let mut img = RgbaImage::from_pixel(w, h, image::Rgba([bg[0], bg[1], bg[2], 255]));
    let line_h = if b.line.is_some() { (h as f32 * 0.08).max(2.0) as u32 } else { 0 };
    if let Some(l) = b.line.map(rgb) {
        for y in h - line_h..h {
            for x in 0..w {
                img.put_pixel(x, y, image::Rgba([l[0], l[1], l[2], 255]));
            }
        }
    }
    if let Some(f) = font {
        let avail = (h - line_h) as f32;
        let mut px = avail * 0.58;
        let max_w = w as f32 * 0.86;
        let tw = text_width(f, b.text, px);
        if tw > max_w {
            px *= max_w / tw;
        }
        let x = (w as f32 - text_width(f, b.text, px)) * 0.5;
        draw_text(&mut img, f, b.text, px, x, avail * 0.5 + px * 0.36, fg);
    }
    make_image(w, h, img.into_raw(), true)
}

/// Cover art outline (centred on its own middle): square on the spine side, following the
/// case's corner radius on the opening side. Returns (outline, width, height, centre x).
fn cover_outline(c: &CaseDims) -> (Vec<Vec2>, f32, f32, f32) {
    let (x0, x1) = (c.xs, c.hw - c.border);
    let (w, h) = (x1 - x0, (c.hh - c.border) * 2.0);
    let r = (c.rc - c.border).max(0.0);
    let pts = rounded_poly(&[(-w * 0.5, -h * 0.5, 0.0), (w * 0.5, -h * 0.5, r), (w * 0.5, h * 0.5, r), (-w * 0.5, h * 0.5, 0.0)]);
    (pts, w, h, (x0 + x1) * 0.5)
}

/// Rosette hub a disc clips onto, standing on the floor of the case (z = 0 at the floor).
fn disc_hub(disc_r: f32) -> Mesh {
    let mut m = MeshBuf::default();
    let o = Vec3::ZERO;
    let seg = 48;
    // Low ring moulded round where the disc's edge sits.
    let rw = disc_r + 0.15;
    m.cylinder(o, rw, 0.0, 0.06, false, 128);
    m.cylinder(o, rw + 0.08, 0.0, 0.06, true, 128);
    m.annulus(o, rw, rw + 0.08, 0.06, true, 128);
    // Base, then the ring the disc rests on.
    m.cylinder(o, 2.15, 0.0, 0.08, true, seg);
    m.annulus(o, 0.0, 2.15, 0.08, true, seg);
    m.cylinder(o, 1.95, 0.08, 0.2, true, seg);
    m.cylinder(o, 1.5, 0.08, 0.2, false, seg);
    m.annulus(o, 1.5, 1.95, 0.2, true, seg);
    // Petals that grip the edge of the centre hole, round a push button.
    let n = 12;
    for i in 0..n {
        let rot = Quat::from_rotation_z(i as f32 / n as f32 * std::f32::consts::TAU);
        m.cuboid(rot * Vec3::new(0.56, 0.0, 0.29), Vec3::new(0.16, 0.08, 0.21), rot);
    }
    m.cylinder(o, 0.34, 0.08, 0.46, true, 32);
    m.annulus(o, 0.0, 0.34, 0.46, true, 32);
    m.build()
}

/// Frame that holds a cartridge (or UMD) flat in the case, with finger gaps at the sides
/// and two clips over its top and bottom edges. z = 0 at the floor.
fn media_holder(w: f32, h: f32, d: f32, frame: f32) -> Mesh {
    let mut m = MeshBuf::default();
    let (hw, hh) = (w * 0.5 + 0.03, h * 0.5 + 0.03);
    let ht = d * 1.05;
    let z = ht * 0.5;
    for sy in [-1.0f32, 1.0] {
        m.cuboid(Vec3::new(0.0, sy * (hh + frame * 0.5), z), Vec3::new(hw + frame, frame * 0.5, z), Quat::IDENTITY);
        // Clip lip over the edge.
        m.cuboid(Vec3::new(0.0, sy * (hh + frame * 0.5 - 0.08), ht + 0.03), Vec3::new((w * 0.18).min(1.2), frame * 0.5 + 0.08, 0.03), Quat::IDENTITY);
    }
    let gap = (h * 0.22).min(1.6);
    let side = (hh - gap * 0.5).max(0.05);
    for sx in [-1.0f32, 1.0] {
        for sy in [-1.0f32, 1.0] {
            m.cuboid(Vec3::new(sx * (hw + frame * 0.5), sy * (gap * 0.5 + side * 0.5), z), Vec3::new(frame * 0.5, side * 0.5, z), Quat::IDENTITY);
        }
    }
    m.build()
}

// ------------------------------------------------------------------------------------------------
// Cartridges
// ------------------------------------------------------------------------------------------------

/// Front outline of a cartridge (label up, contacts along the bottom edge).
fn cart_outline(shape: CartShape, w: f32, h: f32) -> Vec<Vec2> {
    let (hw, hh) = (w * 0.5, h * 0.5);
    let c: Vec<(f32, f32, f32)> = match shape {
        // Game Boy: the top right corner is cut off.
        CartShape::GameBoy => vec![(-hw, -hh, 0.12), (hw, -hh, 0.12), (hw, hh - 0.55, 0.06), (hw - 0.55, hh, 0.06), (-hw, hh, 0.14)],
        // GBA: the top is rounded over.
        CartShape::Gba => vec![(-hw, -hh, 0.1), (hw, -hh, 0.1), (hw, hh, 0.65), (-hw, hh, 0.65)],
        CartShape::Ds => vec![(-hw, -hh, 0.08), (hw, -hh, 0.08), (hw, hh, 0.1), (-hw + 0.32, hh, 0.04), (-hw, hh - 0.32, 0.04)],
        // 3DS: like the DS card, with the little tab on the right.
        CartShape::N3ds => vec![
            (-hw, -hh, 0.08),
            (hw, -hh, 0.08),
            (hw, hh - 1.05, 0.02),
            (hw + 0.1, hh - 0.95, 0.03),
            (hw + 0.1, hh - 0.55, 0.03),
            (hw, hh - 0.45, 0.02),
            (hw, hh, 0.1),
            (-hw + 0.32, hh, 0.04),
            (-hw, hh - 0.32, 0.04),
        ],
        CartShape::Switch => vec![(-hw, -hh, 0.12), (hw, -hh, 0.12), (hw, hh, 0.12), (-hw + 0.22, hh, 0.05), (-hw, hh - 0.22, 0.05)],
        CartShape::Vita => vec![(-hw, -hh, 0.08), (hw, -hh, 0.08), (hw, hh, 0.3), (-hw, hh, 0.3)],
        CartShape::Nes => vec![(-hw, -hh, 0.15), (hw, -hh, 0.15), (hw, hh, 0.3), (-hw, hh, 0.3)],
        CartShape::Snes => vec![(-hw, -hh, 0.2), (hw, -hh, 0.2), (hw, hh, 1.1), (-hw, hh, 1.1)],
        // N64: wide rounded shoulders over a narrower connector end.
        CartShape::N64 => {
            let nb = hw * 0.88;
            vec![
                (-nb, -hh, 0.15),
                (nb, -hh, 0.15),
                (nb, -hh + h * 0.3, 0.12),
                (hw, -hh + h * 0.38, 0.2),
                (hw, hh, 1.1),
                (-hw, hh, 1.1),
                (-hw, -hh + h * 0.38, 0.2),
                (-nb, -hh + h * 0.3, 0.12),
            ]
        }
        CartShape::Genesis => {
            let nb = hw * 0.93;
            vec![(-nb, -hh, 0.15), (nb, -hh, 0.15), (hw, -hh + h * 0.25, 0.2), (hw, hh, 0.45), (-hw, hh, 0.45), (-hw, -hh + h * 0.25, 0.2)]
        }
        CartShape::Generic => {
            let r = w.min(h) * 0.06;
            vec![(-hw, -hh, r), (hw, -hh, r), (hw, hh, r), (-hw, hh, r)]
        }
    };
    rounded_poly(&c)
}

/// Label rectangle (centre x, centre y, w, h) as fractions of the cartridge.
fn cart_label(shape: CartShape) -> (f32, f32, f32, f32) {
    match shape {
        CartShape::GameBoy => (0.0, -0.08, 0.80, 0.56),
        CartShape::Gba => (0.0, -0.06, 0.82, 0.62),
        CartShape::Ds | CartShape::N3ds => (0.0, -0.1, 0.82, 0.6),
        CartShape::Switch => (0.0, -0.08, 0.80, 0.6),
        CartShape::Vita => (0.0, -0.05, 0.84, 0.68),
        CartShape::Nes => (0.0, 0.2, 0.80, 0.45),
        CartShape::Snes => (0.0, 0.06, 0.70, 0.55),
        CartShape::N64 => (0.0, 0.16, 0.6, 0.52),
        CartShape::Genesis => (0.0, 0.18, 0.78, 0.48),
        CartShape::Generic => (0.0, 0.12, 0.86, 0.62),
    }
}

/// Moulded grip ridges on the front face (z = 0 at the face).
fn cart_ridges(shape: CartShape, w: f32, h: f32) -> Option<Mesh> {
    let mut m = MeshBuf::default();
    let (hw, hh) = (w * 0.5, h * 0.5);
    let ridge = |m: &mut MeshBuf, x: f32, y: f32, len: f32, horizontal: bool, t: f32| {
        let half = if horizontal { Vec3::new(len * 0.5, t, 0.025) } else { Vec3::new(t, len * 0.5, 0.025) };
        m.cuboid(Vec3::new(x, y, 0.02), half, Quat::IDENTITY);
    };
    match shape {
        CartShape::GameBoy => {
            // Grip lines across the top, and the embossed arrow pointing into the console.
            for i in 0..3 {
                ridge(&mut m, -0.3, hh - 0.35 - i as f32 * 0.16, w * 0.55, true, 0.035);
            }
            let (ax, ay, s) = (w * 0.3, hh - 0.6, 0.22);
            let tip = m.vert(Vec3::new(ax, ay - s, 0.045), Vec3::Z, Vec2::ZERO);
            let l = m.vert(Vec3::new(ax - s, ay + s * 0.4, 0.045), Vec3::Z, Vec2::ZERO);
            let r = m.vert(Vec3::new(ax + s, ay + s * 0.4, 0.045), Vec3::Z, Vec2::ZERO);
            m.tri(tip, l, r);
        }
        CartShape::Gba => {
            // The grip band along the rounded top.
            ridge(&mut m, 0.0, hh - 0.32, w * 0.62, true, 0.12);
        }
        CartShape::Ds | CartShape::N3ds => {
            for i in 0..3 {
                ridge(&mut m, 0.15, hh - 0.18 - i as f32 * 0.09, w * 0.5, true, 0.018);
            }
        }
        CartShape::Nes | CartShape::Genesis => {
            for i in 0..6 {
                ridge(&mut m, 0.0, -hh + 0.6 + i as f32 * (h * 0.06), w * 0.72, true, 0.07);
            }
        }
        CartShape::Snes => {
            for sx in [-1.0, 1.0] {
                for i in 0..3 {
                    ridge(&mut m, sx * (hw - 0.45 - i as f32 * 0.22), 0.0, h * 0.5, false, 0.05);
                }
            }
        }
        CartShape::N64 => {
            // Grip ridges on the shoulders and the ridge along the top edge.
            for sx in [-1.0, 1.0] {
                for i in 0..5 {
                    ridge(&mut m, sx * (hw - 0.45 - i as f32 * 0.24), h * 0.12, h * 0.42, false, 0.055);
                }
            }
            ridge(&mut m, 0.0, hh - 0.35, w * 0.55, true, 0.1);
        }
        _ => return None,
    }
    Some(m.build())
}

/// Gold connector pins showing at the bottom of the back (handheld cards only).
fn cart_pins(shape: CartShape, w: f32, h: f32) -> Option<Mesh> {
    let (n, span, len) = match shape {
        CartShape::GameBoy => (32, 0.82, 0.42),
        CartShape::Gba => (32, 0.85, 0.32),
        CartShape::Ds | CartShape::N3ds => (17, 0.86, 0.3),
        CartShape::Switch => (16, 0.85, 0.22),
        CartShape::Vita => (12, 0.8, 0.18),
        _ => return None,
    };
    let mut m = MeshBuf::default();
    let pitch = w * span / n as f32;
    for i in 0..n {
        let x = -w * span * 0.5 + pitch * (i as f32 + 0.5);
        m.cuboid(Vec3::new(x, -h * 0.5 + 0.05 + len * 0.5, 0.0), Vec3::new(pitch * 0.32, len * 0.5, 0.006), Quat::IDENTITY);
    }
    Some(m.build())
}

// ------------------------------------------------------------------------------------------------
// Disc material (PBR + mirror + diffraction rainbow, see disc.wgsl)
// ------------------------------------------------------------------------------------------------

pub const DISC_SHADER: Handle<Shader> = Handle::weak_from_u128(0x5eed_c0de_b4c6_41d0_9a53_1c0ffee00002);

/// Direction towards the key and rim lights (must match the lights spawned in app.rs).
pub const KEY_LIGHT_POS: Vec3 = Vec3::new(-6.0, 10.0, 14.0);
pub const RIM_LIGHT_POS: Vec3 = Vec3::new(9.0, 6.0, -12.0);

#[derive(Clone, Copy, Debug, ShaderType, Reflect)]
pub struct DiscParams {
    pub light_a: Vec4,
    pub light_b: Vec4,
    pub rainbow: f32,
    pub mirror: f32,
    pub pad0: f32,
    pub pad1: f32,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct DiscSurface {
    #[uniform(100)]
    pub params: DiscParams,
}

impl MaterialExtension for DiscSurface {
    fn fragment_shader() -> ShaderRef {
        DISC_SHADER.into()
    }
}

pub type DiscMaterial = ExtendedMaterial<StandardMaterial, DiscSurface>;

pub struct DiscMaterialPlugin;

impl Plugin for DiscMaterialPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, DISC_SHADER, "disc.wgsl", Shader::from_wgsl);
        app.add_plugins(MaterialPlugin::<DiscMaterial>::default());
    }
}

fn disc_material(rainbow: f32, mirror: f32, color: Color, roughness: f32) -> DiscMaterial {
    ExtendedMaterial {
        base: StandardMaterial { base_color: color, metallic: 0.85, perceptual_roughness: roughness, reflectance: 0.8, ..default() },
        extension: DiscSurface {
            params: DiscParams {
                light_a: KEY_LIGHT_POS.normalize().extend(1.0),
                light_b: RIM_LIGHT_POS.normalize().extend(0.6),
                rainbow,
                mirror,
                pad0: 0.0,
                pad1: 0.0,
            },
        },
    }
}


// ------------------------------------------------------------------------------------------------
// Spawning
// ------------------------------------------------------------------------------------------------

pub struct SpawnCtx<'a> {
    pub gfx: &'a mut Gfx,
    pub meshes: &'a mut Assets<Mesh>,
    pub materials: &'a mut Assets<StandardMaterial>,
    pub disc_materials: &'a mut Assets<DiscMaterial>,
    pub images: &'a mut Assets<Image>,
    pub font: Option<&'a fontdue::Font>,
    pub cfg: &'a Config,
}

enum Mat {
    Std(Handle<StandardMaterial>),
    Disc(Handle<DiscMaterial>),
}

/// Which moving piece of an item a part belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Root,
    Spine,
    Lid,
    Media,
    /// The manual in the lid, hinged on its spine edge.
    Booklet,
    /// Inside the tray (hub, holder): only seen when the case is open.
    Inside,
    /// Inside the lid (clips).
    LidInside,
    /// A blank page of the manual under its cover, hinged like it (0 = just under the cover).
    Page(u8),
    /// The rest of the manual's pages (one block).
    PageRest,
}

/// Loose blank pages under the manual's cover, which fan up after it when it lifts.
pub const MANUAL_PAGES: usize = 4;

/// Part that is only seen with the case open; it fades out faster than the rest of a closed
/// case, so it does not show through while the case fades at the edge of the screen.
#[derive(Component)]
pub struct Interior;

/// Part of the disc or cartridge of a case. While the item is see-through (fading) these are
/// drawn before the case, so the case covers them as it does when solid instead of the disc
/// showing in front of it and then jumping behind it once the fade finishes.
#[derive(Component)]
pub struct MediaPart;

struct Part {
    mesh: Handle<Mesh>,
    material: Mat,
    transform: Transform,
    slot: Slot,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaKind {
    Disc,
    Cart,
}

/// The hinged pieces of a case, so the launch animation can open it and take the game out.
pub struct CaseRig {
    /// Pivot on the back hinge; the spine swings round it.
    pub spine: Entity,
    /// Pivot on the front hinge (a child of `spine`); the lid swings round it.
    pub lid: Entity,
    pub media: Option<Entity>,
    pub media_rest: Transform,
    /// Where the disc / cartridge sits in its holder in the tray (the same as `media_rest`
    /// unless it is shown sticking out of the case).
    pub media_inside: Transform,
    pub media_kind: MediaKind,
    /// Largest side of the disc or cartridge.
    pub media_size: f32,
    /// Sideways move (local x) that centres the case once it lies open.
    pub open_shift: f32,
    /// Width and height of the open case.
    pub open_size: Vec2,
    /// Pivot of the manual in the lid (on its spine edge), when the case has one.
    pub booklet: Option<Entity>,
    /// Centre of the manual's printed face once the case lies open (item space), and its
    /// half size, for pointing at it.
    pub booklet_open: Vec3,
    pub booklet_half: Vec2,
    /// The manual's hinge in the lid, and how far the whole manual comes out while in use.
    pub booklet_base: Vec3,
    pub booklet_flip: Vec3,
    /// Pivots of the blank pages under the manual's cover (top first).
    pub pages: Vec<Entity>,
    /// Pivot of the rest of the pages (never turned; comes out with the others).
    pub booklet_rest: Option<Entity>,
    /// Half thickness of the case.
    pub half_depth: f32,
}

/// Margin between the case edge and the printed cover.
pub const BORDER: f32 = 0.14;
const EPS: f32 = 0.012;

/// Corner radius of a platform's case: the console's own, a per-platform override, or the
/// custom radius for every case.
pub fn corner_radius(cfg: &Config, p: Platform) -> f32 {
    if cfg.display.case_corners == "custom" {
        return cfg.display.case_corner_radius.max(0.0);
    }
    cfg.platform_override(p).and_then(|o| o.corner_radius).unwrap_or_else(|| p.corner_radius()).max(0.0)
}

/// Case material and kind for a platform, with the `case_color` override applied.
pub fn case_look(cfg: &Config, p: Platform) -> CaseLook {
    let mut l = p.case_look();
    if let Some(s) = cfg.platform_override(p).and_then(|o| o.case_color.as_deref()) {
        let h = s.trim().trim_start_matches('#');
        if let Ok(v) = u32::from_str_radix(h, 16) {
            let (rgb, a) = if h.len() == 8 { (v >> 8, (v & 255) as f32 / 255.0) } else { (v, 1.0) };
            l.shell = [((rgb >> 16) & 255) as f32 / 255.0, ((rgb >> 8) & 255) as f32 / 255.0, (rgb & 255) as f32 / 255.0, a];
        }
    }
    l
}

/// width / height of the printed front cover of a platform's case.
pub fn front_aspect(cfg: &Config, p: Platform) -> f32 {
    let (cw, ch, cd) = geometry(cfg, p).case;
    let kind = case_look(cfg, p).kind;
    let b = case_border(kind);
    (cw - case_bevel(kind, cd) - b) / (ch - b * 2.0)
}

/// Unlit, back faces only (see the outline shell in `spawn_item`).
pub fn outline_material(materials: &mut Assets<StandardMaterial>, rgb: [f32; 3], strength: f32) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: Color::srgb(rgb[0], rgb[1], rgb[2]).with_alpha(strength),
        unlit: true,
        cull_mode: Some(bevy::render::render_resource::Face::Front),
        alpha_mode: if strength < 1.0 { AlphaMode::Blend } else { AlphaMode::Opaque },
        ..default()
    })
}

fn cover_material(materials: &mut Assets<StandardMaterial>, accent: [f32; 3]) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: Color::srgb(accent[0] * 0.5 + 0.03, accent[1] * 0.5 + 0.03, accent[2] * 0.5 + 0.03),
        // Satin print: a soft sheen, no glare.
        perceptual_roughness: 0.6,
        reflectance: 0.25,
        depth_bias: PRINT_BIAS,
        ..default()
    })
}

/// The "not installed" badge: a dark round sticker with a white rim and a download arrow
/// over a tray, smoothed by sampling each pixel 4 x 4 times.
fn badge_image() -> Image {
    const N: usize = 128;
    let mut px = vec![0u8; N * N * 4];
    let inside_arrow = |x: f32, y: f32| -> bool {
        // Shaft, head and tray in 0..1 coordinates (y down).
        let shaft = (x - 0.5).abs() < 0.055 && y > 0.2 && y < 0.56;
        let head = y >= 0.5 && y < 0.7 && (x - 0.5).abs() < (0.7 - y) * 1.05;
        let tray = (y > 0.74 && y < 0.8 && (x - 0.5).abs() < 0.24) || ((x - 0.5).abs() > 0.185 && (x - 0.5).abs() < 0.24 && y > 0.62 && y < 0.8);
        shaft || head || tray
    };
    for py in 0..N {
        for px_ in 0..N {
            let (mut a, mut white) = (0.0f32, 0.0f32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = (px_ as f32 + (sx as f32 + 0.5) / 4.0) / N as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / 4.0) / N as f32;
                    let d = ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt();
                    if d < 0.48 {
                        a += 1.0;
                        if d > 0.42 || inside_arrow(x, y) {
                            white += 1.0;
                        }
                    }
                }
            }
            let (a, w) = (a / 16.0, if a > 0.0 { white / a } else { 0.0 });
            let mix = |dark: f32| ((dark + (1.0 - dark) * w) * 255.0) as u8;
            let o = (py * N + px_) * 4;
            px[o..o + 4].copy_from_slice(&[mix(0.07), mix(0.08), mix(0.1), (a * 0.94 * 255.0) as u8]);
        }
    }
    make_image(N as u32, N as u32, px, true)
}

impl Gfx {
    fn badge(&mut self, materials: &mut Assets<StandardMaterial>, images: &mut Assets<Image>) -> Handle<StandardMaterial> {
        self.badge
            .get_or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color_texture: Some(images.add(badge_image())),
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    depth_bias: PRINT_BIAS * 2.0,
                    ..default()
                })
            })
            .clone()
    }
}

/// Printed parts (covers, spines, labels) lie a hair above the plastic. When an item is drawn
/// see-through (fading), this makes them sort after their own case so they never vanish behind it.
const PRINT_BIAS: f32 = 4.0;

fn media_size(media: Media) -> Vec3 {
    match media {
        Media::Disc { diameter } => Vec3::new(diameter, diameter, 0.12),
        Media::Cart { w, h, d } => Vec3::new(w, h, d),
        Media::Umd => Vec3::new(UMD_SIZE.0, UMD_SIZE.1, UMD_SIZE.2),
    }
}

impl Gfx {
    fn plastic_rgba(&mut self, c: [f32; 4], roughness: f32, reflectance: f32, mats: &mut Assets<StandardMaterial>) -> Handle<StandardMaterial> {
        let key = format!("{:.3},{:.3},{:.3},{:.3}/{roughness:.2}/{reflectance:.2}", c[0], c[1], c[2], c[3]);
        self.plastic
            .entry(key)
            .or_insert_with(|| {
                mats.add(StandardMaterial {
                    base_color: Color::srgba(c[0], c[1], c[2], c[3]),
                    alpha_mode: if c[3] < 0.99 { AlphaMode::Blend } else { AlphaMode::Opaque },
                    perceptual_roughness: roughness,
                    reflectance,
                    ..default()
                })
            })
            .clone()
    }
    fn gold(&mut self, mats: &mut Assets<StandardMaterial>) -> Handle<StandardMaterial> {
        self.plastic
            .entry("gold".into())
            .or_insert_with(|| {
                mats.add(StandardMaterial { base_color: Color::srgb(0.86, 0.68, 0.32), metallic: 1.0, perceptual_roughness: 0.28, ..default() })
            })
            .clone()
    }
}

/// Adds a disc, cartridge or UMD, centred on the origin of `slot`, label towards +Z.
fn add_media(cx: &mut SpawnCtx, game: &Game, media: Media, ac: [f32; 3], parts: &mut Vec<Part>, slots: &mut Vec<CoverSlot>, slot: Slot) {
    let platform = game.platform;
    let at = Transform::IDENTITY;
    match media {
        Media::Disc { diameter } => {
            let r = diameter * 0.5;
            let t = 0.12;
            let (zt, zb) = (t * 0.5, -t * 0.5);
            // Real CD proportions: 15 mm hole, clear clamping area, a see-through stacking ring
            // with a rainbow sheen, then the printed label / data area.
            let r_hole = 0.75f32.min(r * 0.16);
            let r_stack = 1.15f32.min(r * 0.25);
            let r_hub = 1.65f32.min(r * 0.33);
            let seg = 96;
            let key = format!("{r:.3}");
            let g = &mut *cx.gfx;
            let m = &mut *cx.meshes;
            let hub_top = g.mesh(format!("hub_t{key}"), m, || ring(r_hole, r_stack, zt, true, r, seg));
            let hub_bot = g.mesh(format!("hub_b{key}"), m, || ring(r_hole, r_stack, zb, false, r, seg));
            let stack_top = g.mesh(format!("stk_t{key}"), m, || ring(r_stack, r_hub, zt, true, r, seg));
            let stack_bot = g.mesh(format!("stk_b{key}"), m, || ring(r_stack, r_hub, zb, false, r, seg));
            let hole_wall = g.mesh(format!("hole{key}"), m, || tube(r_hole, zb, zt, false, seg));
            let label_mesh = g.mesh(format!("label2{key}"), m, || ring(r_hub, r, zt, true, r, seg));
            let data_mesh = g.mesh(format!("data2{key}"), m, || ring(r_hub, r, zb, false, r, seg));
            let rim = g.mesh(format!("rim{key}"), m, || tube(r, zb, zt, true, seg));

            let rainbow = cx.cfg.disc_rainbow();
            let clear = cx.gfx.clear_plastic(cx.materials, 0.08, 0.05);
            let sheen = cx.gfx.disc_clear(cx.disc_materials, if cx.cfg.potato() { 0.0 } else { rainbow.max(0.6) });
            let mirror = cx.gfx.disc(cx.disc_materials, "mirror", 0.0, 1.0);
            let data = cx.gfx.disc(cx.disc_materials, "data", rainbow, 0.85);
            let label = cover_material(cx.materials, ac);
            for (mesh, mat) in [
                (hub_top, Mat::Std(clear.clone())),
                (hub_bot, Mat::Std(clear.clone())),
                (hole_wall, Mat::Std(clear)),
                (stack_top, Mat::Disc(sheen.clone())),
                (stack_bot, Mat::Disc(sheen)),
                (rim, Mat::Disc(mirror)),
                (data_mesh, Mat::Disc(data)),
                (label_mesh, Mat::Std(label.clone())),
            ] {
                parts.push(Part { mesh, material: mat, transform: at, slot });
            }
            slots.push(CoverSlot { mat: label, aspect: 1.0, kind: ArtKind::Media });
        }
        Media::Umd => {
            let (w, h, d) = UMD_SIZE;
            let shell = cx.gfx.plastic_rgba([0.1, 0.1, 0.11, 1.0], 0.35, 0.4, cx.materials);
            let body = cx.gfx.mesh("umd".into(), cx.meshes, || {
                let (hw, hh) = (w * 0.5, h * 0.5);
                rounded_slab(&rounded_poly(&[(-hw, -hh, 0.25), (hw, -hh, 0.25), (hw, hh, 0.9), (-hw, hh, 0.9)]), d, d * 0.2, None)
            });
            parts.push(Part { mesh: body, material: Mat::Std(shell), transform: at, slot });
            // Printed label over most of the front.
            let (lw, lh) = (w * 0.84, h * 0.66);
            let lm = cx.gfx.mesh(format!("q{lw:.3}x{lh:.3}"), cx.meshes, || Rectangle::new(lw, lh).into());
            let label = cover_material(cx.materials, ac);
            parts.push(Part { mesh: lm, material: Mat::Std(label.clone()), transform: Transform::from_xyz(0.0, h * 0.06, d * 0.5 + EPS), slot });
            slots.push(CoverSlot { mat: label, aspect: lw / lh, kind: ArtKind::Media });
            // The disc shows through the window in the back.
            let rainbow = cx.cfg.disc_rainbow();
            let data = cx.gfx.disc(cx.disc_materials, "data", rainbow, 0.85);
            let window = cx.gfx.mesh("umd_win".into(), cx.meshes, || ring(0.55, 2.85, 0.0, false, 3.0, 96));
            let hubm = cx.gfx.mesh("umd_hub".into(), cx.meshes, || ring(0.0, 0.55, 0.0, false, 3.0, 48));
            let clear = cx.gfx.clear_plastic(cx.materials, 0.35, 0.2);
            let back = Transform::from_xyz(0.0, -h * 0.04, -d * 0.5 - EPS);
            parts.push(Part { mesh: window, material: Mat::Disc(data), transform: back, slot });
            parts.push(Part { mesh: hubm, material: Mat::Std(clear), transform: back, slot });
        }
        Media::Cart { w, h, d } => {
            // This game's own cartridge colour (measured from a photo, or a famous one by title),
            // else the console's.
            let (shape, default_color) = platform.cart_look();
            let color = game.cart_color.or_else(|| platform.known_cart_color(&game.title)).unwrap_or(default_color);
            let shell = cx.gfx.plastic_rgba(color, 0.5, 0.35, cx.materials);
            // The label sits in a recessed well: a darker frame just round it.
            let well = cx.gfx.plastic_rgba([color[0] * 0.62, color[1] * 0.62, color[2] * 0.62, color[3].max(0.9)], 0.6, 0.3, cx.materials);
            let key = format!("{shape:?}{w:.3}x{h:.3}x{d:.3}");
            let body = cx.gfx.mesh(format!("cart{key}"), cx.meshes, || rounded_slab(&cart_outline(shape, w, h), d, (d * 0.15).min(0.12), None));
            parts.push(Part { mesh: body, material: Mat::Std(shell.clone()), transform: at, slot });
            let (lx, ly, fw, fh) = cart_label(shape);
            let (lw, lh) = (w * fw, h * fh);
            let lm = cx.gfx.mesh(format!("q{lw:.3}x{lh:.3}"), cx.meshes, || Rectangle::new(lw, lh).into());
            let label = cover_material(cx.materials, ac);
            parts.push(Part { mesh: lm, material: Mat::Std(label.clone()), transform: Transform::from_xyz(w * lx, h * ly, d * 0.5 + EPS), slot });
            slots.push(CoverSlot { mat: label, aspect: lw / lh, kind: ArtKind::Media });
            let rim = (w.min(h) * 0.03).clamp(0.05, 0.25);
            let (ww, wh) = (lw + rim * 2.0, lh + rim * 2.0);
            let wm = cx.gfx.mesh(format!("q{ww:.3}x{wh:.3}"), cx.meshes, || Rectangle::new(ww, wh).into());
            parts.push(Part { mesh: wm, material: Mat::Std(well), transform: Transform::from_xyz(w * lx, h * ly, d * 0.5 + EPS * 0.5), slot });
            if let Some(mesh) = cart_ridges(shape, w, h) {
                let mesh = cx.gfx.mesh(format!("ridges{key}"), cx.meshes, || mesh);
                parts.push(Part { mesh, material: Mat::Std(shell.clone()), transform: Transform::from_xyz(0.0, 0.0, d * 0.5), slot });
            }
            if let Some(mesh) = cart_pins(shape, w, h) {
                let mesh = cx.gfx.mesh(format!("pins{key}"), cx.meshes, || mesh);
                let gold = cx.gfx.gold(cx.materials);
                parts.push(Part { mesh, material: Mat::Std(gold), transform: Transform::from_xyz(0.0, 0.0, -d * 0.5 - 0.006), slot });
            }
        }
    }
}

/// Where the disc or cartridge sits inside the closed case, and the holder for it.
fn media_in_case(c: &CaseDims, kind: CaseKind, media: Media) -> Vec3 {
    let zf = -c.hd + c.wall;
    let x = c.inner_cx();
    match media {
        Media::Disc { .. } => Vec3::new(x, 0.0, zf + 0.26),
        Media::Umd => Vec3::new(x, 0.0, zf + UMD_SIZE.2 * 0.5),
        Media::Cart { h, d, .. } => {
            // Small cards sit in a holder in the upper half, like DS and Switch cases.
            let y = if kind == CaseKind::Keep && h < 5.0 { (c.hh - c.wall - h * 0.5 - 1.6).clamp(0.0, c.hh * 0.35) } else { 0.0 };
            Vec3::new(x, y, zf + d * 0.5)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_case(
    cx: &mut SpawnCtx,
    game: &Game,
    c: &CaseDims,
    look: CaseLook,
    media: Media,
    media_at: Vec3,
    ac: [f32; 3],
    parts: &mut Vec<Part>,
    slots: &mut Vec<CoverSlot>,
    back_slot: &mut Option<CoverSlot>,
    spine_slot: &mut Option<CoverSlot>,
    booklet_out: &mut Option<(Vec3, Vec3, Vec2, Vec3)>,
    page_slots: &mut Vec<CoverSlot>,
) {
    let key = format!("{}{:?}", c.key(), look.kind);
    let cardboard = look.kind == CaseKind::Box;
    let reflect = if cardboard { 0.15 } else { 0.5 };
    let shell = if cx.cfg.display.rainbow_cases {
        // Its own random colour (fixed for this game this session), keeping the console's
        // translucency and finish.
        if cx.gfx.rainbow_seed == 0 {
            cx.gfx.rainbow_seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
        }
        let mut h: u64 = 0xcbf29ce484222325 ^ cx.gfx.rainbow_seed;
        for b in game.id.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        let hue = (h % 3600) as f32 / 10.0;
        cx.materials.add(StandardMaterial {
            base_color: Color::hsla(hue, 0.8, 0.5, look.shell[3]),
            alpha_mode: if look.shell[3] < 0.99 { AlphaMode::Blend } else { AlphaMode::Opaque },
            perceptual_roughness: look.roughness,
            reflectance: reflect,
            ..default()
        })
    } else {
        cx.gfx.plastic_rgba(look.shell, look.roughness, reflect, cx.materials)
    };
    // Jewel cases: clear lid and spine over a coloured tray. Boxes: a plastic insert inside.
    let tray = match look.kind {
        CaseKind::Keep | CaseKind::Box => shell.clone(),
        CaseKind::Jewel => cx.gfx.plastic_rgba(look.tray, 0.35, 0.4, cx.materials),
    };
    let inner = match look.kind {
        CaseKind::Keep => shell.clone(),
        CaseKind::Jewel | CaseKind::Box => cx.gfx.plastic_rgba(look.tray, 0.4, 0.4, cx.materials),
    };
    let back_half = cx.gfx.mesh(format!("half_b{key}"), cx.meshes, || case_half(c, false));
    let front_half = cx.gfx.mesh(format!("half_f{key}"), cx.meshes, || case_half(c, true));
    let spine = cx.gfx.mesh(format!("spine{key}"), cx.meshes, || case_spine(c));
    parts.push(Part { mesh: back_half, material: Mat::Std(tray.clone()), transform: Transform::IDENTITY, slot: Slot::Root });
    parts.push(Part { mesh: front_half, material: Mat::Std(shell.clone()), transform: Transform::IDENTITY, slot: Slot::Lid });
    parts.push(Part { mesh: spine, material: Mat::Std(shell.clone()), transform: Transform::IDENTITY, slot: Slot::Spine });

    // Printed covers: front on the lid, back on the tray, the spine wrapped round its bevels.
    let (art, fw, fh, acx) = cover_outline(c);
    let front_mesh = cx.gfx.mesh(format!("front{key}"), cx.meshes, || flat_poly(&art, 0.0, true, fw, fh));
    let back_mesh = cx.gfx.mesh(format!("back{key}"), cx.meshes, || flat_poly(&art, 0.0, false, fw, fh));
    let front = cover_material(cx.materials, ac);
    let back = cx.materials.add(StandardMaterial {
        base_color: Color::srgb(ac[0] * 0.22 + 0.02, ac[1] * 0.22 + 0.02, ac[2] * 0.22 + 0.02),
        perceptual_roughness: 0.6,
        reflectance: 0.25,
        depth_bias: PRINT_BIAS,
        ..default()
    });
    parts.push(Part { mesh: front_mesh, material: Mat::Std(front.clone()), transform: Transform::from_xyz(acx, 0.0, c.hd + EPS), slot: Slot::Lid });
    parts.push(Part { mesh: back_mesh, material: Mat::Std(back.clone()), transform: Transform::from_xyz(acx, 0.0, -c.hd - EPS), slot: Slot::Root });
    let (sw, sh) = {
        let prof = spine_profile(c, EPS);
        (prof.last().map(|p| p.2).unwrap_or(1.0), (c.hh - c.border) * 2.0)
    };
    let spine_mesh = cx.gfx.mesh(format!("spineart{key}"), cx.meshes, || spine_art(c));
    let banner_on = cx.cfg.display.platform_banners && game.platform.banner().is_some();
    let spine_img = make_spine(&game.title, game.platform.display_name(), ac, cx.font, sh, sw, banner_on);
    let spine_mat = cx.materials.add(StandardMaterial {
        base_color_texture: Some(cx.images.add(spine_img)),
        perceptual_roughness: 0.6,
        reflectance: 0.25,
        depth_bias: PRINT_BIAS,
        ..default()
    });
    parts.push(Part { mesh: spine_mesh, material: Mat::Std(spine_mat.clone()), transform: Transform::IDENTITY, slot: Slot::Spine });

    // The console's banner across the top of the cover and of the spine, over the art (and
    // later on the manual: material and height).
    let mut manual_banner: Option<(Handle<StandardMaterial>, f32)> = None;
    if let Some(banner) = cx.cfg.display.platform_banners.then(|| game.platform.banner()).flatten() {
        let bh = fh * match look.kind {
            CaseKind::Keep => 0.1,
            CaseKind::Jewel => 0.09,
            CaseKind::Box => 0.08,
        };
        let top = c.hh - c.border;
        // Front: follows the art's rounded top corner on the opening side.
        let r = (c.rc - c.border).max(0.0);
        let poly = rounded_poly(&[(-fw * 0.5, -bh * 0.5, 0.0), (fw * 0.5, -bh * 0.5, 0.0), (fw * 0.5, bh * 0.5, r), (-fw * 0.5, bh * 0.5, 0.0)]);
        let front_band = cx.gfx.mesh(format!("band_f{key}/{bh:.3}"), cx.meshes, || flat_poly(&poly, 0.0, true, fw, bh));
        let spine_band_mesh = cx.gfx.mesh(format!("band_s{key}/{bh:.3}"), cx.meshes, || spine_band(c, top, bh));
        let bkey = format!("{:?}{key}", game.platform);
        let (front_mat, spine_band_mat) = match cx.gfx.banners.get(&bkey) {
            Some(m) => m.clone(),
            None => {
                // About 64 pixels per cm.
                let ppc = 64.0;
                let front_img = banner_image(&banner, cx.font, (fw * ppc) as u32, (bh * ppc).max(16.0) as u32);
                let spine_img = banner_image(&banner, cx.font, (sw * ppc).max(32.0) as u32, (bh * ppc).max(16.0) as u32);
                let mut mat = |img: Image| {
                    let tex = cx.images.add(img);
                    cx.materials.add(StandardMaterial {
                        base_color_texture: Some(tex),
                        perceptual_roughness: 0.6,
                        reflectance: 0.25,
                        // Above the cover art it overlaps.
                        depth_bias: PRINT_BIAS + 1.0,
                        ..default()
                    })
                };
                let m = (mat(front_img), mat(spine_img));
                cx.gfx.banners.insert(bkey, m.clone());
                m
            }
        };
        parts.push(Part {
            mesh: front_band,
            material: Mat::Std(front_mat.clone()),
            transform: Transform::from_xyz(acx, top - bh * 0.5, c.hd + EPS * 2.0),
            slot: Slot::Lid,
        });
        parts.push(Part { mesh: spine_band_mesh, material: Mat::Std(spine_band_mat), transform: Transform::IDENTITY, slot: Slot::Spine });
        manual_banner = Some((front_mat.clone(), bh));
    }
    slots.push(CoverSlot { mat: front, aspect: fw / fh, kind: ArtKind::Cover });
    *back_slot = Some(CoverSlot { mat: back, aspect: fw / fh, kind: ArtKind::Back });
    *spine_slot = Some(CoverSlot { mat: spine_mat, aspect: sw / sh, kind: ArtKind::Spine });

    // Inside the tray: the hub or holder for the game.
    let floor = -c.hd + c.wall;
    let holder_at = Vec3::new(media_at.x, media_at.y, floor);
    match media {
        Media::Disc { diameter } => {
            let hub = cx.gfx.mesh(format!("hub{diameter:.2}"), cx.meshes, || disc_hub(diameter * 0.5));
            parts.push(Part { mesh: hub, material: Mat::Std(inner.clone()), transform: Transform::from_translation(holder_at), slot: Slot::Inside });
        }
        Media::Cart { .. } | Media::Umd => {
            let Vec3 { x: w, y: h, z: d } = media_size(media);
            let frame = if cardboard { (w.min(h) * 0.12).clamp(0.3, 1.0) } else { 0.18 };
            let holder = cx.gfx.mesh(format!("holder{w:.2}x{h:.2}x{d:.2}/{frame:.2}"), cx.meshes, || media_holder(w, h, d, frame));
            parts.push(Part { mesh: holder, material: Mat::Std(inner.clone()), transform: Transform::from_translation(holder_at), slot: Slot::Inside });
        }
    }

    // Inside the lid: the manual, held by two clips along the lid's outer edge (the left side
    // when the case lies open); in a cardboard box it just lies loose in the lid. No art
    // database has manual or insert scans, so it is printed like most real manuals: the box art
    // with the console's banner across the top.
    {
        let lid_floor = c.hd - c.wall;
        let iw = c.hw - c.wall - c.xs;
        let ih = (c.hh - c.wall) * 2.0;
        let (bw, bh, bt) = (iw - 0.5, ih - 0.5, 0.06);
        let paper = cx.gfx.plastic_rgba([0.92, 0.92, 0.9, 1.0], 0.8, 0.2, cx.materials);
        let bx = c.inner_cx();
        // The printed cover is a thin sheet; under it a few loose blank pages (hinged, so they
        // fan up after the cover) and then the rest of the pages as one block against the lid.
        let (cover_t, page_t, gap) = (0.012, 0.008, 0.01);
        let cover = cx.gfx.mesh(format!("booklet_c{bw:.2}x{bh:.2}"), cx.meshes, || Cuboid::new(bw, bh, cover_t).into());
        parts.push(Part { mesh: cover, material: Mat::Std(paper.clone()), transform: Transform::from_xyz(bx, 0.0, lid_floor - bt + cover_t * 0.5), slot: Slot::Booklet });
        let (pw, ph) = (bw - 0.08, bh - 0.12);
        let page = cx.gfx.mesh(format!("page{pw:.2}x{ph:.2}"), cx.meshes, || Cuboid::new(pw, ph, page_t).into());
        let shades = [cx.gfx.plastic_rgba([0.88, 0.87, 0.84, 1.0], 0.85, 0.15, cx.materials), cx.gfx.plastic_rgba([0.82, 0.81, 0.78, 1.0], 0.85, 0.15, cx.materials)];
        for j in 0..MANUAL_PAGES {
            let z = lid_floor - bt + cover_t + gap * j as f32 + page_t * 0.5;
            // Pages hang from the same edge as the cover, a hair in from it.
            parts.push(Part { mesh: page.clone(), material: Mat::Std(shades[j % 2].clone()), transform: Transform::from_xyz(bx + 0.04, 0.0, z), slot: Slot::Page(j as u8) });
        }
        // Printed faces (blank paper until the pages are made): the inside of the cover, then
        // the front (facing out of the lid) and back of each loose sheet.
        let blank = |m: &mut Assets<StandardMaterial>| -> Handle<StandardMaterial> {
            m.add(StandardMaterial { base_color: Color::srgb(0.86, 0.85, 0.82), perceptual_roughness: 0.85, reflectance: 0.15, depth_bias: PRINT_BIAS, ..default() })
        };
        let face = |w: f32, h: f32| format!("q{:.3}x{:.3}", w - 0.04, h - 0.04);
        let flip = Quat::from_rotation_y(std::f32::consts::PI);
        let cover_face = cx.gfx.mesh(face(bw, bh), cx.meshes, || Rectangle::new(bw - 0.04, bh - 0.04).into());
        let page_face = cx.gfx.mesh(face(pw, ph), cx.meshes, || Rectangle::new(pw - 0.04, ph - 0.04).into());
        let lift = 0.0015;
        let inside = blank(cx.materials);
        parts.push(Part { mesh: cover_face, material: Mat::Std(inside.clone()), transform: Transform::from_xyz(bx, 0.0, lid_floor - bt + cover_t + lift), slot: Slot::Booklet });
        page_slots.push(CoverSlot { mat: inside, aspect: (bw - 0.04) / (bh - 0.04), kind: ArtKind::Insert });
        for j in 0..MANUAL_PAGES {
            let z = lid_floor - bt + cover_t + gap * j as f32;
            let (front, back) = (blank(cx.materials), blank(cx.materials));
            parts.push(Part { mesh: page_face.clone(), material: Mat::Std(front.clone()), transform: Transform::from_xyz(bx + 0.04, 0.0, z - lift).with_rotation(flip), slot: Slot::Page(j as u8) });
            parts.push(Part { mesh: page_face.clone(), material: Mat::Std(back.clone()), transform: Transform::from_xyz(bx + 0.04, 0.0, z + page_t + lift), slot: Slot::Page(j as u8) });
            page_slots.push(CoverSlot { mat: front, aspect: (pw - 0.04) / (ph - 0.04), kind: ArtKind::Insert });
            page_slots.push(CoverSlot { mat: back, aspect: (pw - 0.04) / (ph - 0.04), kind: ArtKind::Insert });
        }
        let rest_t = (bt - cover_t - gap * MANUAL_PAGES as f32).max(0.004);
        let block = cx.gfx.mesh(format!("pages{pw:.2}x{ph:.2}x{rest_t:.3}"), cx.meshes, || Cuboid::new(pw, ph, rest_t).into());
        parts.push(Part { mesh: block, material: Mat::Std(paper), transform: Transform::from_xyz(bx + 0.04, 0.0, lid_floor - rest_t * 0.5), slot: Slot::PageRest });
        // Hinged on its clipped edge (the lid's outer edge). Opening the case turns the lid over
        // the spine, so the face ends up at x' = 2 xs - d - x, z' = -z.
        *booklet_out = Some((
            Vec3::new(bx + bw * 0.5, 0.0, lid_floor),
            Vec3::new(2.0 * c.xs - c.hd * 2.0 - bx, 0.0, -(lid_floor - bt)),
            Vec2::new(bw * 0.5, bh * 0.5),
            // How far the whole insert comes out of the lid while it is being used: its hinge just
            // above the rim, so turning pages clear the case's walls and clips.
            Vec3::new(0.0, 0.0, -lid_floor - 0.16),
        ));
        let bm = cx.gfx.mesh(format!("q{:.3}x{:.3}", bw - 0.04, bh - 0.04), cx.meshes, || Rectangle::new(bw - 0.04, bh - 0.04).into());
        let manual = cover_material(cx.materials, ac);
        parts.push(Part {
            mesh: bm,
            material: Mat::Std(manual.clone()),
            transform: Transform::from_xyz(bx, 0.0, lid_floor - bt - EPS * 0.5).with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
            slot: Slot::Booklet,
        });
        slots.push(CoverSlot { mat: manual, aspect: (bw - 0.04) / (bh - 0.04), kind: ArtKind::Insert });
        if let Some((band, band_h)) = manual_banner {
            let (w, h) = (bw - 0.04, band_h.min(bh * 0.2));
            let m = cx.gfx.mesh(format!("q{w:.3}x{h:.3}"), cx.meshes, || Rectangle::new(w, h).into());
            parts.push(Part {
                mesh: m,
                material: Mat::Std(band),
                transform: Transform::from_xyz(bx, (bh - 0.04 - h) * 0.5, lid_floor - bt - EPS * 1.5).with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                slot: Slot::Booklet,
            });
        }
        // Clips along the outer edge: a post beside the manual's edge with a lip over it
        // (plastic cases only).
        let edge = bx + bw * 0.5;
        let clips = cx.gfx.mesh(format!("clips{key}"), cx.meshes, || {
            let mut m = MeshBuf::default();
            for sy in [-1.0f32, 1.0] {
                let y = sy * bh * 0.3;
                m.cuboid(Vec3::new(edge + 0.12, y, lid_floor - 0.13), Vec3::new(0.1, 0.55, 0.13), Quat::IDENTITY);
                m.cuboid(Vec3::new(edge - 0.12, y, lid_floor - bt - 0.05), Vec3::new(0.22, 0.42, 0.03), Quat::IDENTITY);
            }
            m.build()
        });
        if !cardboard {
            parts.push(Part { mesh: clips, material: Mat::Std(inner.clone()), transform: Transform::IDENTITY, slot: Slot::LidInside });
        }
    }
}

pub fn spawn_item(commands: &mut Commands, cx: &mut SpawnCtx, game: &Game, view_index: usize, style: ItemStyle) -> Entity {
    let geo = geometry(cx.cfg, game.platform);
    let look = case_look(cx.cfg, game.platform);
    let (cw, ch, _) = geo.case;
    let ac = accent(cx.cfg, game.platform);
    let dims = item_dims(cx.cfg, game.platform, style);
    let rc = corner_radius(cx.cfg, game.platform);
    let notch = cx.cfg.display.case_finger_notch;
    let mut slots: Vec<CoverSlot> = Vec::new();
    let mut back_slot: Option<CoverSlot> = None;
    let mut spine_slot: Option<CoverSlot> = None;
    let mut parts: Vec<Part> = Vec::new();
    // (spine pivot, lid pivot, media rest, open shift, open size) for the case styles.
    let mut hinge: Option<(Vec3, Vec3, Vec3, f32, Vec2)> = None;
    let mut media_inside = Vec3::ZERO;
    let mut booklet_geo: Option<(Vec3, Vec3, Vec2, Vec3)> = None;
    let mut page_slots: Vec<CoverSlot> = Vec::new();
    let mut half_depth = 0.0;
    let msize = media_size(geo.media);

    match style {
        ItemStyle::Case | ItemStyle::CaseDisc => {
            let c = CaseDims::new(geo.case, look.kind, rc, notch);
            let inside = media_in_case(&c, look.kind, geo.media);
            media_inside = inside;
            let rest = if style == ItemStyle::CaseDisc {
                // Sticks out of the top edge.
                let peek = cx.cfg.display.disc_peek.clamp(0.0, 0.95);
                Vec3::new(0.0, ch * 0.5 + msize.y * peek - msize.y * 0.5, 0.0)
            } else {
                inside
            };
            add_case(cx, game, &c, look, geo.media, inside, ac, &mut parts, &mut slots, &mut back_slot, &mut spine_slot, &mut booklet_geo, &mut page_slots);
            half_depth = c.hd;
            add_media(cx, game, geo.media, ac, &mut parts, &mut slots, Slot::Media);
            let d = c.hd * 2.0;
            hinge = Some((Vec3::new(c.xs, 0.0, -c.hd), Vec3::new(c.xs, 0.0, c.hd), rest, d * 0.5 - c.xs, Vec2::new(2.0 * (c.hw - c.xs) + d, ch)));
        }
        ItemStyle::Flat => {
            let m = cx.gfx.mesh(format!("q{cw:.3}x{ch:.3}"), cx.meshes, || Rectangle::new(cw, ch).into());
            let mat = cover_material(cx.materials, ac);
            let back = cover_material(cx.materials, [ac[0] * 0.4, ac[1] * 0.4, ac[2] * 0.4]);
            parts.push(Part { mesh: m.clone(), material: Mat::Std(mat.clone()), transform: Transform::IDENTITY, slot: Slot::Root });
            parts.push(Part {
                mesh: m,
                material: Mat::Std(back.clone()),
                transform: Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                slot: Slot::Root,
            });
            slots.push(CoverSlot { mat, aspect: cw / ch, kind: ArtKind::Cover });
            back_slot = Some(CoverSlot { mat: back, aspect: cw / ch, kind: ArtKind::Back });
        }
        ItemStyle::Disc => add_media(cx, game, geo.media, ac, &mut parts, &mut slots, Slot::Root),
    }
    // Not installed: a badge in the top corner of the front (on the lid of a case, so it opens
    // with it; round discs get it inside the rim).
    if !game.installed {
        let case = matches!(style, ItemStyle::Case | ItemStyle::CaseDisc);
        let (bw, bh) = if case { (cw, ch) } else { (dims.x, dims.y) };
        let size = bw.min(bh) * 0.22;
        let round = style == ItemStyle::Disc && matches!(geo.media, Media::Disc { .. });
        let (x, y) = if round {
            let r = bw * 0.5 * 0.66;
            (r * std::f32::consts::FRAC_1_SQRT_2, r * std::f32::consts::FRAC_1_SQRT_2)
        } else {
            (bw * 0.5 - size * 0.72, bh * 0.5 - size * 0.72)
        };
        let z = if style == ItemStyle::Flat { 0.02 } else { dims.z * 0.5 + 0.05 };
        let mesh = cx.gfx.mesh(format!("badge{size:.3}"), cx.meshes, || Rectangle::new(size, size).into());
        let material = cx.gfx.badge(cx.materials, cx.images);
        parts.push(Part { mesh, material: Mat::Std(material), transform: Transform::from_xyz(x, y, z), slot: if case { Slot::Lid } else { Slot::Root } });
    }

    let root = commands
        .spawn(SpatialBundle { transform: Transform::from_scale(Vec3::splat(0.001)), visibility: Visibility::Hidden, ..default() })
        .id();
    let pivot = |commands: &mut Commands, parent: Entity, t: Transform| {
        let e = commands.spawn(SpatialBundle::from_transform(t)).id();
        commands.entity(parent).add_child(e);
        e
    };
    let mut rig = None;
    let (mut spine_e, mut lid_e, mut media_e, mut booklet_e) = (root, root, root, root);
    let mut page_e: Vec<Entity> = Vec::new();
    let mut rest_e: Option<Entity> = None;
    let (mut spine_at, mut lid_at, mut media_rest, mut booklet_at) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    if let Some((s_at, l_at, rest, shift, open_size)) = hinge {
        spine_at = s_at;
        lid_at = l_at;
        media_rest = rest;
        spine_e = pivot(commands, root, Transform::from_translation(s_at));
        lid_e = pivot(commands, spine_e, Transform::from_translation(l_at - s_at));
        media_e = pivot(commands, root, Transform::from_translation(rest));
        if let Some((at, ..)) = booklet_geo {
            booklet_at = at;
            booklet_e = pivot(commands, lid_e, Transform::from_translation(at - l_at));
            for _ in 0..MANUAL_PAGES {
                page_e.push(pivot(commands, lid_e, Transform::from_translation(at - l_at)));
            }
            rest_e = Some(pivot(commands, lid_e, Transform::from_translation(at - l_at)));
        }
        rig = Some(CaseRig {
            spine: spine_e,
            lid: lid_e,
            media: Some(media_e),
            media_rest: Transform::from_translation(rest),
            media_inside: Transform::from_translation(media_inside),
            media_kind: if matches!(geo.media, Media::Disc { .. } | Media::Umd) { MediaKind::Disc } else { MediaKind::Cart },
            media_size: msize.x.max(msize.y),
            open_shift: shift,
            open_size,
            booklet: booklet_geo.map(|_| booklet_e),
            booklet_open: booklet_geo.map_or(Vec3::ZERO, |b| b.1),
            booklet_half: booklet_geo.map_or(Vec2::ZERO, |b| b.2),
            booklet_base: booklet_geo.map_or(Vec3::ZERO, |b| b.0 - l_at),
            booklet_flip: booklet_geo.map_or(Vec3::ZERO, |b| b.3),
            pages: page_e.clone(),
            booklet_rest: rest_e,
            half_depth,
        });
    }
    for part in parts {
        let (parent, origin) = match part.slot {
            Slot::Root => (root, Vec3::ZERO),
            Slot::Spine => (spine_e, spine_at),
            Slot::Lid => (lid_e, lid_at),
            Slot::Media => (media_e, media_rest),
            Slot::Booklet => (booklet_e, booklet_at),
            Slot::Page(j) => (page_e.get(j as usize).copied().unwrap_or(booklet_e), booklet_at),
            Slot::PageRest => (rest_e.unwrap_or(booklet_e), booklet_at),
            Slot::Inside => (root, Vec3::ZERO),
            Slot::LidInside => (lid_e, lid_at),
        };
        let interior = matches!(part.slot, Slot::Inside | Slot::LidInside | Slot::Booklet | Slot::Page(_) | Slot::PageRest) || (part.slot == Slot::Media && style == ItemStyle::Case);
        // Case parts are modelled in closed-case coordinates; media parts around their own centre.
        let transform = if part.slot == Slot::Media { part.transform } else { Transform::from_translation(-origin) * part.transform };
        let e = match part.material {
            Mat::Std(material) => commands.spawn(PbrBundle { mesh: part.mesh, material, transform, ..default() }).id(),
            Mat::Disc(material) => commands.spawn(MaterialMeshBundle { mesh: part.mesh, material, transform, ..default() }).id(),
        };
        if interior {
            commands.entity(e).insert(Interior);
        }
        if part.slot == Slot::Media {
            commands.entity(e).insert(MediaPart);
        }
        commands.entity(parent).add_child(e);
    }
    // Outline: shells a little larger than the item, drawn inside out so only their rims round
    // the item show. Several faint ones of growing size make the edge fade out softly. In
    // case + disc mode the disc sticking out gets its own shells.
    let round = style == ItemStyle::Disc && matches!(geo.media, Media::Disc { .. });
    enum Shape {
        Box(Vec3),
        Disc(f32, f32),
    }
    let media_shape = || match geo.media {
        Media::Disc { diameter } => Shape::Disc(diameter * 0.5, 0.12),
        _ => Shape::Box(msize),
    };
    let mut shapes: Vec<(Shape, Vec3)> = Vec::new();
    match style {
        ItemStyle::Disc => shapes.push((media_shape(), Vec3::ZERO)),
        ItemStyle::CaseDisc => {
            shapes.push((Shape::Box(dims), Vec3::ZERO));
            shapes.push((media_shape(), media_rest));
        }
        _ => shapes.push((Shape::Box(dims), Vec3::ZERO)),
    }
    let mat = cx.gfx.outline.get_or_insert_with(|| outline_material(cx.materials, [0.18, 0.55, 1.0], OUTLINE_ALPHA)).clone();
    let width = cx.cfg.display.highlight_width.clamp(0.02, 2.0);
    let outline = commands.spawn((SpatialBundle { visibility: Visibility::Hidden, ..default() }, Outline)).id();
    commands.entity(root).add_child(outline);
    for layer in 1..=OUTLINE_LAYERS {
        let w = width * layer as f32 / OUTLINE_LAYERS as f32;
        for (shape, at) in &shapes {
            let (mesh, rot) = match shape {
                Shape::Disc(r, t) => {
                    let (r, t) = (r + w, t + w * 2.0);
                    let m = cx.gfx.mesh(format!("outline_c{r:.3}x{t:.3}"), cx.meshes, || Cylinder::new(r, t).mesh().resolution(96).into());
                    (m, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                }
                Shape::Box(d) => {
                    let s = *d + Vec3::splat(w * 2.0);
                    (cx.gfx.mesh(format!("outline_b{:.3}x{:.3}x{:.3}", s.x, s.y, s.z), cx.meshes, || Cuboid::new(s.x, s.y, s.z).into()), Quat::IDENTITY)
                }
            };
            let shell = commands
                .spawn((
                    PbrBundle { mesh, material: mat.clone(), transform: Transform::from_translation(*at).with_rotation(rot), ..default() },
                    bevy::pbr::NotShadowCaster,
                    bevy::pbr::NotShadowReceiver,
                ))
                .id();
            commands.entity(outline).add_child(shell);
        }
    }
    commands.entity(root).insert((
        Item { game_id: game.id.clone(), view_index, dims, covers: slots, pages: page_slots, back: back_slot, spine: spine_slot, rig, outline, round },
        // New items fade in.
        Fade { applied: 1.0, ..default() },
    ));
    root
}
