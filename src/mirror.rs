//! "Reflection under selected": a planar mirror at the selected game's base. A second camera,
//! the main one mirrored under that floor, renders the games into a texture; a floor quad
//! (seen only by the main camera) shows it, fading out away from the game.

use bevy::asset::load_internal_asset;
use bevy::pbr::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{AsBindGroup, Extent3d, ShaderRef, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::texture::ImageSampler;
use bevy::render::view::RenderLayers;
use bevy::transform::TransformSystem;
use bevy::window::PrimaryWindow;

use crate::app::{MainCamera, ViewState};
use crate::config::Config;
use crate::render::Item;

/// Layer of the floor quad, so the mirror camera does not see it.
pub const FLOOR_LAYER: usize = 2;
/// The selected game's parts are also on this layer, the only one the mirror camera sees, so
/// only it is reflected (not the games stacked round it).
pub const GAME_LAYER: usize = 3;
const SHADER: Handle<Shader> = Handle::weak_from_u128(0x5eed_c0de_b4c6_41d0_9a53_1c0ffee00004);
const STRENGTH: f32 = 0.45;

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct FloorMaterial {
    #[uniform(0)]
    params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    tex: Handle<Image>,
    #[uniform(3)]
    fade: Vec4,
}

impl Material for FloorMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

#[derive(Component)]
struct MirrorCam;
#[derive(Component)]
struct MirrorFloor(Handle<FloorMaterial>);

pub struct MirrorPlugin;

impl Plugin for MirrorPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "mirror.wgsl", Shader::from_wgsl);
        app.add_plugins(MaterialPlugin::<FloorMaterial>::default())
            .add_systems(Startup, spawn)
            // After all the moving in Update, before transforms are propagated.
            .add_systems(PostUpdate, follow.before(TransformSystem::TransformPropagate).after(bevy::pbr::SimulationLightSystems::AddClusters));
    }
}

fn target(images: &mut Assets<Image>, w: u32, h: u32) -> Handle<Image> {
    let mut img = Image::new_fill(
        Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    img.sampler = ImageSampler::linear();
    images.add(img)
}

fn spawn(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<FloorMaterial>>) {
    let tex = target(&mut images, 1, 1);
    commands.spawn((
        Camera3dBundle {
            // Spawned active: Bevy 0.14 only sets up light clusters for a camera active when added
            // (without them drawing through it panics). `follow` turns it off when not needed.
            camera: Camera { order: -2, target: RenderTarget::Image(tex.clone()), clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
            tonemapping: bevy::core_pipeline::tonemapping::Tonemapping::None,
            ..default()
        },
        MirrorCam,
        RenderLayers::layer(GAME_LAYER),
    ));
    let mat = mats.add(FloorMaterial { params: Vec4::new(0.0, 0.0, 0.0, STRENGTH), tex, fade: Vec4::new(0.0, 0.0, 1.0, 1.0) });
    commands.spawn((
        MaterialMeshBundle { mesh: meshes.add(Rectangle::new(1.0, 1.0)), material: mat.clone(), visibility: Visibility::Hidden, ..default() },
        RenderLayers::layer(FLOOR_LAYER),
        NotShadowCaster,
        NotShadowReceiver,
        MirrorFloor(mat),
    ));
}

/// Puts every part of `root` on the game layer as well (`on`), or back to the default only.
fn tag(commands: &mut Commands, children: &Query<&Children>, layered: &Query<(), With<RenderLayers>>, root: Entity, on: bool) {
    for e in std::iter::once(root).chain(children.iter_descendants(root)) {
        let Some(mut ec) = commands.get_entity(e) else { continue };
        match (on, layered.contains(e)) {
            (true, false) => {
                ec.insert(RenderLayers::from_layers(&[0, GAME_LAYER]));
            }
            (false, true) => {
                ec.remove::<RenderLayers>();
            }
            _ => {}
        }
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn follow(
    mut commands: Commands,
    children: Query<&Children>,
    layered: Query<(), With<RenderLayers>>,
    mut tagged: Local<Option<Entity>>,
    cfg: Res<Config>,
    time: Res<Time>,
    view: Res<ViewState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    items: Query<(Entity, &Item, &Transform), (Without<MainCamera>, Without<MirrorCam>, Without<MirrorFloor>)>,
    main: Query<(&Transform, &Projection), (With<MainCamera>, Without<MirrorCam>, Without<MirrorFloor>)>,
    mut cam: Query<(&mut Camera, &mut Transform, &mut Projection), (With<MirrorCam>, Without<MainCamera>, Without<MirrorFloor>)>,
    mut floor: Query<(&MirrorFloor, &mut Transform, &mut Visibility), (Without<MainCamera>, Without<MirrorCam>)>,
    mut images: ResMut<Assets<Image>>,
    mut mats: ResMut<Assets<FloorMaterial>>,
    // The render target's size, and the selected game's floor height (see below).
    mut memo: Local<(UVec2, Option<(Entity, f32)>)>,
    toss: Res<crate::toss::Toss>,
) {
    let (Ok((mut c, mut ctf, mut cproj)), Ok((f, mut ftf, mut fvis)), Ok((mtf, mproj))) = (cam.get_single_mut(), floor.get_single_mut(), main.get_single()) else { return };
    // Wall / table: only once a game is lifted out.
    // The game in the middle of the screen: scrolling fast, the selection runs on ahead of it.
    // (The wall and table do not scroll that way: there it is the selection.)
    let sel = (cfg.display.selected_reflection && !cfg.potato() && (!view.grid || view.raised) && !view.egg && view.inspect.is_none())
        .then(|| {
            if view.grid {
                items.iter().find(|(_, i, _)| i.view_index == view.selected)
            } else {
                items.iter().filter(|(_, i, _)| view.rel(i.view_index).abs() < 0.5).min_by(|a, b| view.rel(a.1.view_index).abs().total_cmp(&view.rel(b.1.view_index).abs()))
            }
        })
        .flatten();
    // Keep the selected game's parts (new ones too, as covers and spines arrive) on the game
    // layer, and take the last one's off.
    let root = sel.map(|(e, ..)| e);
    if let Some(old) = tagged.filter(|old| Some(*old) != root) {
        tag(&mut commands, &children, &layered, old, false);
    }
    if let Some(r) = root {
        tag(&mut commands, &children, &layered, r, true);
    }
    *tagged = root;
    let Some((_, item, tf)) = sel else {
        if c.is_active {
            c.is_active = false;
        }
        if *fvis != Visibility::Hidden {
            *fvis = Visibility::Hidden;
        }
        return;
    };
    if !c.is_active {
        c.is_active = true;
    }
    if *fvis != Visibility::Inherited {
        *fvis = Visibility::Inherited;
    }

    // Half the window's resolution is plenty for a faded reflection.
    if let Ok(w) = windows.get_single() {
        let want = UVec2::new(w.physical_width() / 2, w.physical_height() / 2).max(UVec2::ONE);
        if memo.0 != want {
            memo.0 = want;
            let tex = target(&mut images, want.x, want.y);
            c.target = RenderTarget::Image(tex.clone());
            if let Some(m) = mats.get_mut(&f.0) {
                m.tex = tex;
            }
        }
    }

    // The floor: level with the game's base (a hair under), from below the game towards the camera.
    // Under the game's lowest corner (tilted or leaning, a corner dips below its middle), at
    // the bottom of its idle float: the game never goes through the reflection, and with the
    // float taken out the floor stays still, so the reflection bobs the opposite way.
    let bob = crate::app::idle_bob(time.elapsed_seconds(), cfg.animation.idle_sway);
    let lowest = (0..8)
        .map(|i| tf.transform_point(item.dims * 0.5 * Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 })).y)
        .fold(f32::MAX, f32::min);
    let h = lowest - bob - crate::app::idle_bob(std::f32::consts::FRAC_PI_2 / 1.7, cfg.animation.idle_sway).abs() - 0.02;
    // Moved by physics (dragged, thrown, knocked): the floor stays where it was, only following
    // the game down if it goes below it.
    let h = match (memo.1, root) {
        (Some((e, was)), Some(r)) if e == r && toss.physical(&item.game_id) => h.min(was),
        _ => h,
    };
    memo.1 = root.map(|r| (r, h));
    let base = Vec3::new(tf.translation.x, h, tf.translation.z);
    let mut dir = Vec3::new(mtf.translation.x - base.x, 0.0, mtf.translation.z - base.z);
    let len = dir.length().max(1.0);
    dir /= len;
    let right = (-dir).cross(Vec3::Y);
    // The game's mirror image only reaches about two thirds of the way to the camera.
    let depth = len * 0.8;
    // Also reaching back behind the game: turned, one side of it is further away than its
    // middle, and its reflection would be cut off at the edge (at full strength there; the fade
    // starts at the middle).
    let back = item.dims.length() * 0.5 * tf.scale.x;
    *ftf = Transform {
        translation: base + dir * (depth - back) * 0.5,
        rotation: Quat::from_mat3(&Mat3::from_cols(right, -dir, Vec3::Y)),
        scale: Vec3::new(len * 4.0, depth + back, 1.0),
    };
    let (params, fade) = (base.extend(STRENGTH), dir.extend(depth));
    if let Some(m) = mats.get(&f.0).filter(|m| m.params != params || m.fade != fade).map(|_| ()).and_then(|_| mats.get_mut(&f.0)) {
        (m.params, m.fade) = (params, fade);
    }

    // The mirror camera: the main one reflected in the floor, kept upright (so its picture is
    // the reflection upside down, which the floor's shader flips back).
    // A true mirror on purpose, on every layout: a game tipped back (the table) reflects tipped
    // forwards, as it would over a real one.
    let p = mtf.translation;
    let fwd = mtf.forward();
    let mp = Vec3::new(p.x, 2.0 * h - p.y, p.z);
    *ctf = Transform::from_translation(mp).looking_to(Vec3::new(fwd.x, -fwd.y, fwd.z), Vec3::Y);
    *cproj = mproj.clone();
}
