//! Full-screen animated background drawn by its own 2D camera behind the 3D scene.
//! Colours adapt (smoothly) to the dominant colour of the selected cover.

use bevy::asset::load_internal_asset;
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor, ShaderRef, ShaderType,
    SpecializedMeshPipelineError,
};
use bevy::sprite::Material2dKey;
use bevy::sprite::{Material2d, Material2dPlugin, MaterialMesh2dBundle, Mesh2dHandle};
use bevy::window::PrimaryWindow;

use crate::config::{parse_hex, BackgroundStyle, Config};

pub const BG_SHADER: Handle<Shader> = Handle::weak_from_u128(0x5eed_c0de_b4c6_41d0_9a53_1c0ffee00001);
pub const MASK_SHADER: Handle<Shader> = Handle::weak_from_u128(0x5eed_c0de_b4c6_41d0_9a53_1c0ffee00003);

/// Halfway overlay: drawn last on the background layer, it scales everything already drawn
/// there (colour and alpha) by a soft radial fade, so the desktop shows through at the edges.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct MaskMaterial {
    /// x = aspect, y = reach (fraction of the screen), z = softness.
    #[uniform(0)]
    pub params: Vec4,
}

impl Material2d for MaskMaterial {
    fn fragment_shader() -> ShaderRef {
        MASK_SHADER.into()
    }
    fn specialize(descriptor: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, _: Material2dKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        // dst = dst * src.a: a multiply, so the result stays premultiplied for the compositor.
        let mul = BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::SrcAlpha, operation: BlendOperation::Add };
        if let Some(f) = descriptor.fragment.as_mut() {
            for t in f.targets.iter_mut().flatten() {
                t.blend = Some(BlendState { color: mul, alpha: mul });
            }
        }
        Ok(())
    }
}

#[derive(Component)]
pub struct BgMask;

#[derive(Resource)]
pub struct MaskState(pub Handle<MaskMaterial>);

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct BgParams {
    pub time: f32,
    pub style: f32,
    pub intensity: f32,
    pub speed: f32,
    pub aspect: f32,
    pub scroll: f32,
    pub pad0: f32,
    pub pad1: f32,
    pub color_a: Vec4,
    pub color_b: Vec4,
    pub color_c: Vec4,
    /// PS2 style: each orb's trail points (x, y in the 2560 px wide layout, trail width,
    /// orb size), the orb itself first; worked out once a frame instead of per pixel.
    pub orbs: [Vec4; ORB_COUNT * ORB_POINTS],
    /// Each orb's bounding box (min x, min y, max x, max y) including its glow.
    pub orb_box: [Vec4; ORB_COUNT],
}

pub const ORB_COUNT: usize = 7;
pub const ORB_POINTS: usize = 16;

/// The PS2 menu's orbs (after imsaud.me): 7 orbs on a 90 px circle whose plane tumbles with the
/// clock (`day` = seconds of the day), seen through an 800 px perspective, the orbit scaled 3.6x
/// and centred; with the path each took over the last 0.6 s for its trail.
fn ps2_orbs(day: f32, aspect: f32) -> ([Vec4; ORB_COUNT * ORB_POINTS], [Vec4; ORB_COUNT]) {
    use std::f32::consts::PI;
    let design_w = 2560.0;
    let design_h = design_w / aspect.max(0.1);
    let centre = Vec2::new(design_w * 0.5, design_h * 0.5);
    let scale = 3.6;
    let r = day.rem_euclid(60.0);
    let minute = (day / 60.0).floor();
    let hours = (day / 3600.0).rem_euclid(12.0);
    let a = (90.0 - hours * 30.0).to_radians();
    let lpts = [PI / 2.0, PI + PI / 6.0, 0.0];
    let m0 = (minute.rem_euclid(3.0)) as usize;
    let l = lpts[m0] + r / 60.0 * (lpts[(m0 + 1) % 3] - lpts[m0]);
    let span = 0.022 * 28.0;
    let mut pts = [Vec4::ZERO; ORB_COUNT * ORB_POINTS];
    let mut boxes = [Vec4::ZERO; ORB_COUNT];
    for i in 0..ORB_COUNT {
        let fi = i as f32;
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for j in 0..ORB_POINTS {
            let tt = r - j as f32 * span / (ORB_POINTS - 1) as f32;
            let c = Mat3::from_rotation_z(-PI * tt) * Mat3::from_rotation_x(PI / 2.0 * tt + l) * Mat3::from_rotation_z(a);
            let n = tt * fi * 6.0_f32.to_radians();
            let v = c * Vec3::new(90.0 * n.cos(), 90.0 * n.sin(), 0.2 * fi);
            let w = v * scale;
            let persp = 800.0 / (800.0 - w.z).max(40.0);
            let at = centre + Vec2::new(w.x, w.y) * persp;
            let fade = 1.0 - j as f32 / ORB_POINTS as f32;
            let k_trail = scale * persp * 0.42 * (0.35 + 0.65 * fade);
            let k_orb = scale * persp * 0.7;
            pts[i * ORB_POINTS + j] = Vec4::new(at.x, at.y, k_trail, k_orb);
            let reach = if j == 0 { 39.0 * k_orb } else { 16.0 * k_trail };
            lo = lo.min(at - Vec2::splat(reach));
            hi = hi.max(at + Vec2::splat(reach));
        }
        boxes[i] = Vec4::new(lo.x, lo.y, hi.x, hi.y);
    }
    (pts, boxes)
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct BgMaterial {
    #[uniform(0)]
    pub params: BgParams,
}

impl Material2d for BgMaterial {
    fn fragment_shader() -> ShaderRef {
        BG_SHADER.into()
    }
}

#[derive(Component)]
pub struct BgQuad;

/// Two sprites that cross-fade between blurred game art images.
#[derive(Component)]
pub struct ArtSprite(pub usize);

#[derive(Resource, Default)]
pub struct BgArt {
    /// Image wanted on screen now (game id, image, width, height).
    pub want: Option<(String, Handle<Image>, u32, u32)>,
    /// Which sprite shows `want`, and each sprite's game id.
    front: usize,
    shown: [Option<String>; 2],
    alpha: [f32; 2],
    sizes: [Vec2; 2],
    /// Loaded backdrops, so revisiting a game is instant.
    pub cache: std::collections::HashMap<String, (Handle<Image>, u32, u32)>,
}

impl BgArt {
    pub fn insert(&mut self, id: String, h: Handle<Image>, w: u32, hh: u32) {
        if self.cache.len() > 48 {
            let drop: Vec<String> = self.cache.keys().filter(|k| self.want.as_ref().map_or(true, |w| &w.0 != *k)).take(16).cloned().collect();
            for k in drop {
                self.cache.remove(&k);
            }
        }
        self.cache.insert(id, (h, w, hh));
    }
}

#[derive(Resource)]
pub struct BgState {
    pub handle: Handle<BgMaterial>,
    /// Current colours (linear), blended towards `target`.
    pub current: [Vec3; 3],
    pub target: [Vec3; 3],
    pub scroll: f32,
}

/// The custom background: the picture (or a GIF's frames, each with how long it shows).
#[derive(Resource, Default)]
pub struct CustomBg {
    path: String,
    frames: Vec<(Handle<Image>, f32)>,
    size: Vec2,
    frame: usize,
    t: f32,
    rx: Option<crossbeam_channel::Receiver<Vec<(u32, u32, Vec<u8>, f32)>>>,
}

#[derive(Component)]
pub struct CustomSprite;

/// Reads a picture or every frame of an animated GIF (frames scaled to at most 1920 wide).
fn read_custom(path: &str) -> Vec<(u32, u32, Vec<u8>, f32)> {
    use image::AnimationDecoder;
    let fit = |img: image::RgbaImage| -> image::RgbaImage {
        if img.width() > 1920 {
            let h = (img.height() as f32 * 1920.0 / img.width() as f32) as u32;
            image::imageops::resize(&img, 1920, h.max(1), image::imageops::FilterType::Triangle)
        } else {
            img
        }
    };
    let Ok(bytes) = std::fs::read(path) else { return vec![] };
    if path.to_lowercase().ends_with(".gif") {
        if let Ok(dec) = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&bytes)) {
            let frames: Vec<_> = dec
                .into_frames()
                .take(900)
                .filter_map(|f| f.ok())
                .map(|f| {
                    let (n, d) = f.delay().numer_denom_ms();
                    let secs = (n as f32 / d.max(1) as f32 / 1000.0).max(0.02);
                    let img = fit(f.into_buffer());
                    (img.width(), img.height(), img.into_raw(), secs)
                })
                .collect();
            if !frames.is_empty() {
                return frames;
            }
        }
    }
    match image::load_from_memory(&bytes) {
        Ok(img) => {
            let img = fit(img.to_rgba8());
            vec![(img.width(), img.height(), img.into_raw(), 1.0)]
        }
        Err(_) => vec![],
    }
}

/// Loads the custom picture when it changes (on a thread), plays GIF frames and keeps the
/// picture covering the window.
fn update_custom(
    time: Res<Time>,
    cfg: Res<Config>,
    mut custom: ResMut<CustomBg>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut sprites: Query<(&mut Sprite, &mut Handle<Image>, &mut Transform, &mut Visibility), With<CustomSprite>>,
) {
    let on = cfg.background.style == BackgroundStyle::Custom && !cfg.background.custom_path.is_empty();
    if on && custom.path != cfg.background.custom_path {
        custom.path = cfg.background.custom_path.clone();
        custom.frames.clear();
        let path = custom.path.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send(read_custom(&path));
        });
        custom.rx = Some(rx);
    }
    if let Some(frames) = custom.rx.as_ref().and_then(|r| r.try_recv().ok()) {
        custom.rx = None;
        custom.size = frames.first().map_or(Vec2::ONE, |f| Vec2::new(f.0 as f32, f.1 as f32));
        custom.frames = frames.into_iter().map(|(w, h, px, d)| (images.add(crate::render::make_image(w, h, px, false)), d)).collect();
        custom.frame = 0;
        custom.t = 0.0;
        if custom.frames.is_empty() {
            eprintln!("[slipcase] could not read the background picture {}", custom.path);
        }
    }
    let shown = on && !custom.frames.is_empty();
    if shown && custom.frames.len() > 1 {
        custom.t += time.delta_seconds();
        while custom.t >= custom.frames[custom.frame].1 {
            custom.t -= custom.frames[custom.frame].1;
            custom.frame = (custom.frame + 1) % custom.frames.len();
        }
    }
    let (ww, wh) = windows.get_single().map(|w| (w.width().max(1.0), w.height().max(1.0))).unwrap_or((1600.0, 900.0));
    for (mut sp, mut img, mut tf, mut vis) in &mut sprites {
        *vis = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if !shown {
            continue;
        }
        let h = custom.frames[custom.frame].0.clone();
        if *img != h {
            *img = h;
        }
        // Cover the window, cropping the overflow.
        let size = custom.size.max(Vec2::ONE);
        let k = (ww / size.x).max(wh / size.y);
        sp.custom_size = Some(size * k);
        tf.translation.z = 3.0;
    }
}

pub struct BackgroundPlugin;

impl Plugin for BackgroundPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, BG_SHADER, "background.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, MASK_SHADER, "bgmask.wgsl", Shader::from_wgsl);
        app.add_plugins(Material2dPlugin::<BgMaterial>::default())
            .add_plugins(Material2dPlugin::<MaskMaterial>::default())
            .init_resource::<BgArt>()
            .init_resource::<CustomBg>()
            .add_systems(Update, update_custom)
            .add_systems(Update, (update_background, update_art));
    }
}

pub fn srgb_to_linear(c: [f32; 3]) -> Vec3 {
    let f = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    Vec3::new(f(c[0]), f(c[1]), f(c[2]))
}

fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let (r, g, b) = (c[0], c[1], c[2]);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    let h = if d == 0.0 {
        0.0
    } else if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    [h, if mx == 0.0 { 0.0 } else { d / mx }, mx]
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// Builds the three background colours (linear) from a cover colour.
pub fn palette_from(dominant: [f32; 3]) -> [Vec3; 3] {
    let [h, s, v] = rgb_to_hsv(dominant);
    let s = s.clamp(0.35, 1.0);
    let base = hsv_to_rgb(h, (s * 0.9).min(1.0), 0.16 + v * 0.06);
    let mid = hsv_to_rgb(h, s, 0.75 + v * 0.25);
    let accent = hsv_to_rgb(h + 0.12, s * 0.9, 0.85);
    [srgb_to_linear(base), srgb_to_linear(mid), srgb_to_linear(accent)]
}

pub fn palette_from_config(cfg: &Config) -> [Vec3; 3] {
    [
        srgb_to_linear(parse_hex(&cfg.background.color_a)),
        srgb_to_linear(parse_hex(&cfg.background.color_b)),
        srgb_to_linear(parse_hex(&cfg.background.color_c)),
    ]
}

pub fn spawn_background(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<BgMaterial>,
    cfg: &Config,
    layer: bevy::render::view::RenderLayers,
) -> BgState {
    let pal = palette_from_config(cfg);
    let handle = materials.add(BgMaterial {
        params: BgParams {
            time: 0.0,
            style: cfg.background.style.index(),
            intensity: cfg.background.intensity,
            speed: cfg.background.speed,
            aspect: 16.0 / 9.0,
            scroll: 0.0,
            pad0: 0.0,
            pad1: 0.0,
            orbs: [Vec4::ZERO; ORB_COUNT * ORB_POINTS],
            orb_box: [Vec4::ZERO; ORB_COUNT],
            color_a: pal[0].extend(1.0),
            color_b: pal[1].extend(1.0),
            color_c: pal[2].extend(1.0),
        },
    });
    commands.spawn((
        MaterialMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Rectangle::new(1.0, 1.0))),
            material: handle.clone(),
            transform: Transform::from_scale(Vec3::new(1600.0, 900.0, 1.0)),
            ..default()
        },
        layer.clone(),
        BgQuad,
    ));
    for i in 0..2 {
        commands.spawn((
            SpriteBundle {
                sprite: Sprite { color: Color::srgba(1.0, 1.0, 1.0, 0.0), custom_size: Some(Vec2::ONE), ..default() },
                transform: Transform::from_xyz(0.0, 0.0, 1.0 + i as f32),
                ..default()
            },
            layer.clone(),
            ArtSprite(i),
        ));
    }
    // The custom picture, over everything else on the background layer.
    commands.spawn((
        SpriteBundle { sprite: Sprite { custom_size: Some(Vec2::ONE), ..default() }, visibility: Visibility::Hidden, ..default() },
        layer.clone(),
        CustomSprite,
    ));
    BgState { handle, current: pal, target: pal, scroll: 0.0 }
}

/// Cross-fades the game art sprites and keeps them covering the window.
fn update_art(
    time: Res<Time>,
    cfg: Res<Config>,
    mut art: ResMut<BgArt>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut sprites: Query<(&ArtSprite, &mut Sprite, &mut Handle<Image>, &mut Transform)>,
) {
    let dt = time.delta_seconds().min(0.1);
    let on = cfg.background.style == BackgroundStyle::GameArt;
    // A new image goes onto the back sprite, which then fades in over the front one.
    if let Some((id, h, w, hh)) = art.want.clone() {
        let front = art.front;
        if art.shown[front].as_ref() != Some(&id) {
            let back = 1 - front;
            art.front = back;
            art.shown[back] = Some(id);
            art.alpha[back] = 0.0;
            art.sizes[back] = Vec2::new(w as f32, hh as f32);
            for (s, _, mut img, _) in &mut sprites {
                if s.0 == back {
                    *img = h.clone();
                }
            }
        }
    }
    let k = 1.0 - (-dt * 5.0).exp();
    let front = art.front;
    for i in 0..2 {
        let target = if on && i == front && art.want.is_some() { 1.0 } else if i == front { 0.0 } else { art.alpha[i] };
        art.alpha[i] += (target - art.alpha[i]) * k;
        // The old image fades out only once the new one has mostly covered it.
        if i != front && art.alpha[front] > 0.9 {
            art.alpha[i] *= 1.0 - k;
        }
    }
    let (ww, wh) = windows.get_single().map(|w| (w.width().max(1.0), w.height().max(1.0))).unwrap_or((1600.0, 900.0));
    let b = cfg.background.art_brightness.clamp(0.0, 1.0);
    let o = cfg.background.art_opacity.clamp(0.0, 1.0);
    for (s, mut sp, _, mut tf) in &mut sprites {
        let i = s.0;
        let a = art.alpha[i] * o;
        sp.color = Color::srgba(b, b, b, a);
        // Cover the window, cropping the overflow.
        let size = art.sizes[i].max(Vec2::ONE);
        let k = (ww / size.x).max(wh / size.y) * 1.04;
        sp.custom_size = Some(size * k);
        tf.translation.z = if i == front { 2.0 } else { 1.0 };
    }
}

fn update_background(
    time: Res<Time>,
    cfg: Res<Config>,
    mut bg: ResMut<BgState>,
    mut mats: ResMut<Assets<BgMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut quads: Query<&mut Transform, (With<BgQuad>, Without<BgMask>)>,
    mut masks: Query<(&mut Transform, &mut Visibility), (With<BgMask>, Without<BgQuad>)>,
    mask: Option<Res<MaskState>>,
    mut mask_mats: ResMut<Assets<MaskMaterial>>,
    overlay: Res<crate::app::Overlay>,
) {
    let dt = time.delta_seconds();
    let k = 1.0 - (-dt * cfg.background.blend_speed).exp();
    for i in 0..3 {
        let t = bg.target[i];
        bg.current[i] = bg.current[i].lerp(t, k);
    }
    let (w, h) = windows.get_single().map(|w| (w.width().max(1.0), w.height().max(1.0))).unwrap_or((1600.0, 900.0));
    for mut t in &mut quads {
        t.scale = Vec3::new(w * 1.01, h * 1.01, 1.0);
    }
    let halfway = overlay.0 && cfg.background.overlay_halfway;
    for (mut t, mut vis) in &mut masks {
        t.scale = Vec3::new(w * 1.01, h * 1.01, 1.0);
        let v = if halfway { Visibility::Visible } else { Visibility::Hidden };
        if *vis != v {
            *vis = v;
        }
    }
    if let Some(m) = mask.and_then(|m| mask_mats.get_mut(&m.0)) {
        let p = Vec4::new(w / h, cfg.background.overlay_halfway_size.clamp(0.2, 1.5), 0.0, 0.0);
        if m.params != p {
            m.params = p;
        }
    }
    // Potato mode draws no background (see `app::apply_cameras`).
    if cfg.potato() {
        return;
    }
    if let Some(m) = mats.get_mut(&bg.handle) {
        m.params.time = time.elapsed_seconds();
        // Seconds of the day (UTC), counting on from start: the PS2 style's orbs follow the clock.
        static DAY_START: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
        let day = *DAY_START.get_or_init(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| (d.as_secs() % 86400) as f32 + d.subsec_millis() as f32 / 1000.0).unwrap_or(0.0)
        });
        m.params.pad0 = (day + time.elapsed_seconds()) % 86400.0;
        if cfg.background.style == BackgroundStyle::Ps2 {
            let (pts, boxes) = ps2_orbs(m.params.pad0, w / h);
            m.params.orbs = pts;
            m.params.orb_box = boxes;
        }
        // Whether the colours follow the selected game (styles with colours of their own, like
        // the PS2 one, use them only then).
        m.params.pad1 = if cfg.background.adaptive { 1.0 } else { 0.0 };
        m.params.style = cfg.background.style.index();
        m.params.intensity = cfg.background.intensity;
        m.params.speed = cfg.background.speed;
        m.params.aspect = w / h;
        m.params.scroll = bg.scroll;
        m.params.color_a = bg.current[0].extend(1.0);
        m.params.color_b = bg.current[1].extend(1.0);
        m.params.color_c = bg.current[2].extend(1.0);
    }
}
