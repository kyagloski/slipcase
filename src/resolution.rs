//! Render resolution (Settings > Screen): the background and the games drawn at a percentage of
//! the window's resolution, then stretched over the window. Lower is faster, higher is sharper
//! (drawn bigger and scaled down). At 100% everything is drawn straight to the window as usual.
//!
//! While scaled, the main and background cameras draw into a picture of that size, and a camera
//! of its own shows it over the whole window. The on-screen text moves to that camera, so it
//! stays sharp (and clickable: Bevy only clicks interface drawn to a window), but is then not
//! covered by the screen filters. In the overlay the picture keeps its see-through parts.

use bevy::asset::load_internal_asset;
use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{
    AsBindGroup, BlendState, Extent3d, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
    TextureUsages,
};
use bevy::render::texture::ImageSampler;
use bevy::render::view::RenderLayers;
use bevy::sprite::{Material2d, Material2dKey, Material2dPlugin, MaterialMesh2dBundle, Mesh2dHandle};
use bevy::ui::IsDefaultUiCamera;
use bevy::window::{PrimaryWindow, WindowRef};

use crate::app::{BgCamera, MainCamera};
use crate::config::Config;

/// Seen only by the camera that shows the scaled picture.
const PRESENT_LAYER: usize = 6;

const PRESENT_SHADER: Handle<Shader> = Handle::weak_from_u128(0x5eed_c0de_b4c6_41d0_9a53_1c0ffee00009);

/// The scaled picture, blended as premultiplied alpha (it was drawn over a see-through clear).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct PresentMaterial {
    #[texture(0)]
    #[sampler(1)]
    tex: Handle<Image>,
}

impl Material2d for PresentMaterial {
    fn fragment_shader() -> ShaderRef {
        PRESENT_SHADER.into()
    }
    fn specialize(descriptor: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, _: Material2dKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(f) = descriptor.fragment.as_mut() {
            for t in f.targets.iter_mut().flatten() {
                t.blend = Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING);
            }
        }
        Ok(())
    }
}

pub struct ResolutionPlugin;

impl Plugin for ResolutionPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, PRESENT_SHADER, "present.wgsl", Shader::from_wgsl);
        app.add_plugins(Material2dPlugin::<PresentMaterial>::default())
            .add_systems(Startup, spawn).add_systems(PostUpdate, apply.before(bevy::render::camera::CameraUpdateSystem));
    }
}

#[derive(Component)]
struct PresentCamera;
#[derive(Component)]
struct PresentSprite;

fn spawn(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<PresentMaterial>>) {
    commands.spawn((
        Camera2dBundle {
            // After the games (0), before the settings windows (2).
            camera: Camera { order: 1, is_active: false, ..default() },
            tonemapping: bevy::core_pipeline::tonemapping::Tonemapping::None,
            deband_dither: bevy::core_pipeline::tonemapping::DebandDither::Disabled,
            ..default()
        },
        RenderLayers::layer(PRESENT_LAYER),
        PresentCamera,
    ));
    commands.spawn((
        MaterialMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Rectangle::new(1.0, 1.0))),
            material: mats.add(PresentMaterial { tex: Handle::default() }),
            visibility: Visibility::Hidden,
            ..default()
        },
        RenderLayers::layer(PRESENT_LAYER),
        PresentSprite,
    ));
}

fn picture(images: &mut Assets<Image>, size: UVec2) -> Handle<Image> {
    let mut img = Image::new_fill(
        Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    img.sampler = ImageSampler::linear();
    images.add(img)
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn apply(
    mut commands: Commands,
    cfg: Res<Config>,
    mut mats: ResMut<Assets<PresentMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut state: Local<Option<(Handle<Image>, UVec2)>>,
    mut main: Query<(Entity, &mut Camera, &mut Projection, Has<IsDefaultUiCamera>), (With<MainCamera>, Without<BgCamera>, Without<PresentCamera>)>,
    mut bg: Query<(&mut Camera, &mut OrthographicProjection), (With<BgCamera>, Without<MainCamera>, Without<PresentCamera>)>,
    mut present: Query<(Entity, &mut Camera, Has<IsDefaultUiCamera>), (With<PresentCamera>, Without<MainCamera>, Without<BgCamera>)>,
    mut sprite: Query<(&mut Transform, &Handle<PresentMaterial>, &mut Visibility), With<PresentSprite>>,
) {
    let (Ok(window), Ok((main_e, mut main_cam, mut main_proj, main_ui)), Ok((present_e, mut present_cam, present_ui)), Ok((mut tf, mat, mut vis))) =
        (windows.get_single(), main.get_single_mut(), present.get_single_mut(), sprite.get_single_mut())
    else {
        return;
    };
    let pct = cfg.general.render_resolution.clamp(25.0, 200.0);
    let scaled = (pct - 100.0).abs() > 0.5;
    if !scaled {
        if state.take().is_some() {
            let to_window = RenderTarget::Window(WindowRef::Primary);
            main_cam.target = to_window.clone();
            main_proj.set_changed();
            for (mut c, mut proj) in &mut bg {
                c.target = to_window.clone();
                proj.set_changed();
            }
            present_cam.is_active = false;
            *vis = Visibility::Hidden;
        }
        for (_, mut proj) in &mut bg {
            if proj.scale != 1.0 {
                proj.scale = 1.0;
            }
        }
        if !main_ui {
            commands.entity(main_e).insert(IsDefaultUiCamera);
        }
        if present_ui {
            commands.entity(present_e).remove::<IsDefaultUiCamera>();
        }
        return;
    }
    let want = (Vec2::new(window.physical_width() as f32, window.physical_height() as f32) * pct / 100.0).round().max(Vec2::ONE).min(Vec2::splat(8192.0)).as_uvec2();
    if state.as_ref().map(|(_, s)| *s) != Some(want) {
        let h = picture(&mut images, want);
        // Bevy works out a camera's size again only when its projection (or its window, or a
        // changed picture) says so, not for a new target: without the nudge the background
        // camera keeps the old size and the drawing passes disagree (a wgpu crash).
        main_cam.target = RenderTarget::Image(h.clone());
        main_proj.set_changed();
        for (mut c, mut proj) in &mut bg {
            c.target = RenderTarget::Image(h.clone());
            proj.set_changed();
        }
        if let Some(m) = mats.get_mut(mat) {
            m.tex = h.clone();
        }
        *state = Some((h, want));
    }
    // The background is laid out in the window's points: its camera shows that many over the
    // smaller (or bigger) picture, so it fills it the same as at 100%.
    let fit = window.width() / want.x.max(1) as f32;
    for (_, mut proj) in &mut bg {
        if (proj.scale - fit).abs() > 1e-6 {
            proj.scale = fit;
        }
    }
    if !present_cam.is_active {
        present_cam.is_active = true;
    }
    // Same HDR setting as the settings camera drawing over it (see `settings::menu_camera`).
    if present_cam.hdr != main_cam.hdr {
        present_cam.hdr = main_cam.hdr;
    }
    let size = Vec2::new(window.width(), window.height()).extend(1.0);
    if tf.scale != size {
        tf.scale = size;
    }
    if *vis != Visibility::Inherited {
        *vis = Visibility::Inherited;
    }
    if main_ui {
        commands.entity(main_e).remove::<IsDefaultUiCamera>();
    }
    if !present_ui {
        commands.entity(present_e).insert(IsDefaultUiCamera);
    }
}

/// The mouse in the main camera's picture: the window's cursor, scaled to the picture's size
/// when the scene is drawn at another resolution.
pub fn view_cursor(window: &Window, cam: &Camera) -> Option<Vec2> {
    let c = window.cursor_position()?;
    let Some(size) = cam.logical_viewport_size() else { return Some(c) };
    Some(c * size / Vec2::new(window.width().max(1.0), window.height().max(1.0)))
}
