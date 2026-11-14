//! Screen filters (CRT, pixelate, Game Boy, VHS, ...): one full-screen pass on the main 3D
//! camera after tonemapping and before the menus are drawn, so the background and the games are
//! filtered and the settings stay readable. Any number of filters can be on at once (a bit
//! each); with none on, the camera has no `FilterSettings` and the pass does not run.

use bevy::asset::load_internal_asset;
use bevy::core_pipeline::core_2d::graph::{Core2d, Node2d};
use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::core_pipeline::fullscreen_vertex_shader::fullscreen_shader_vertex_state;
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin};
use bevy::render::render_graph::{NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice};
use bevy::render::texture::BevyDefault;
use bevy::render::view::ViewTarget;
use bevy::render::RenderApp;

use crate::app::{BgCamera, MainCamera};
use crate::config::Config;

const SHADER: Handle<Shader> = Handle::weak_from_u128(0x5eed_c0de_b4c6_41d0_9a53_1c0ffee00f17);

// Must match filters.wgsl.
const CRT: u32 = 1;
const PIXELATE: u32 = 2;
const POSTERIZE: u32 = 4;
const GAME_BOY: u32 = 8;
const VHS: u32 = 16;
const GRAIN: u32 = 32;
const VIGNETTE: u32 = 64;
const BLACK_WHITE: u32 = 128;
const SEPIA: u32 = 256;
const CHROMATIC: u32 = 512;
/// Not a filter: drawn after the menus instead of before them (the shader ignores it).
const OVER_UI: u32 = 1 << 20;

#[derive(Component, Default, Clone, Copy, ExtractComponent, ShaderType)]
pub struct FilterSettings {
    flags: u32,
    time: f32,
    strength: f32,
    lines: f32,
    frame: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
}

pub struct FiltersPlugin;

impl Plugin for FiltersPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "filters.wgsl", Shader::from_wgsl);
        app.add_plugins((ExtractComponentPlugin::<FilterSettings>::default(), UniformComponentPlugin::<FilterSettings>::default()))
            .add_systems(PostUpdate, sync_filters);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        // Two places in the frame: before the on-screen text is drawn, or after (it is then
        // filtered too; the settings are drawn later still, by their own camera). Only the one that `OVER_UI` picks does anything.
        render_app
            .add_render_graph_node::<ViewNodeRunner<FilterNode<false>>>(Core3d, FilterLabel)
            .add_render_graph_edges(Core3d, (Node3d::Tonemapping, FilterLabel, Node3d::EndMainPassPostProcessing))
            .add_render_graph_node::<ViewNodeRunner<FilterNode<true>>>(Core3d, FilterOverUiLabel)
            .add_render_graph_edges(Core3d, (bevy::ui::graph::NodeUi::UiPass, FilterOverUiLabel, Node3d::Upscaling))
            // "Only on the background": the background's own 2D camera, before the games are drawn over it.
            .add_render_graph_node::<ViewNodeRunner<FilterNode<false>>>(Core2d, FilterLabel)
            .add_render_graph_edges(Core2d, (Node2d::Tonemapping, FilterLabel, Node2d::EndMainPassPostProcessing));
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.init_resource::<FilterPipeline>();
    }
}

fn flags(cfg: &Config) -> u32 {
    let f = &cfg.filters;
    if !f.enabled {
        return 0;
    }
    [
        (f.crt, CRT),
        (f.pixelate, PIXELATE),
        (f.posterize, POSTERIZE),
        (f.game_boy, GAME_BOY),
        (f.vhs, VHS),
        (f.film_grain, GRAIN),
        (f.vignette, VIGNETTE),
        (f.black_and_white, BLACK_WHITE),
        (f.sepia, SEPIA),
        (f.chromatic_aberration, CHROMATIC),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |a, (_, b)| a | b)
}

/// Puts the settings on the main camera (or the background's, for "only on the background")
/// while any filter is on (and takes them off when none is), updating them in place from then on.
#[allow(clippy::type_complexity)]
fn sync_filters(
    mut commands: Commands,
    cfg: Res<Config>,
    time: Res<Time>,
    mut cams: Query<(Entity, Option<&mut FilterSettings>, Has<BgCamera>), Or<(With<MainCamera>, With<BgCamera>)>>,
) {
    let flags = flags(&cfg);
    let bg_only = cfg.filters.background_only;
    for (e, cur, is_bg) in &mut cams {
        if flags == 0 || is_bg != bg_only {
            if cur.is_some() {
                commands.entity(e).remove::<FilterSettings>();
            }
            continue;
        }
        // The CRT's phosphor colour rides in bits 16-17.
        let tint = match cfg.filters.crt_colour.as_str() {
            "amber" => 1u32,
            "green" => 2,
            "white" => 3,
            _ => 0,
        };
        let s = FilterSettings {
            flags: flags | tint << 16 | if cfg.filters.cover_menus && !bg_only { OVER_UI } else { 0 },
            time: time.elapsed_seconds_wrapped(),
            strength: cfg.filters.strength.clamp(0.0, 3.0),
            lines: cfg.filters.retro_lines.clamp(60.0, 2160.0),
            frame: cfg.filters.crt_frame.clamp(0.0, 1.0),
            ..default()
        };
        match cur {
            Some(mut cur) => *cur = s,
            None => {
                commands.entity(e).insert(s);
            }
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct FilterLabel;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct FilterOverUiLabel;

/// The filter pass; `OVER` is the one after the menus.
#[derive(Default)]
struct FilterNode<const OVER: bool>;

impl<const OVER: bool> ViewNode for FilterNode<OVER> {
    type ViewQuery = (&'static ViewTarget, &'static FilterSettings, &'static DynamicUniformIndex<FilterSettings>);

    fn run(&self, _graph: &mut RenderGraphContext, ctx: &mut RenderContext, (target, settings, index): QueryItem<Self::ViewQuery>, world: &World) -> Result<(), NodeRunError> {
        if (settings.flags & OVER_UI != 0) != OVER {
            return Ok(());
        }
        let pipes = world.resource::<FilterPipeline>();
        let id = if target.is_hdr() { pipes.hdr } else { pipes.sdr };
        let Some(pipeline) = world.resource::<PipelineCache>().get_render_pipeline(id) else { return Ok(()) };
        let Some(uniforms) = world.resource::<ComponentUniforms<FilterSettings>>().uniforms().binding() else { return Ok(()) };
        let post = target.post_process_write();
        let bind = ctx.render_device().create_bind_group("filters", &pipes.layout, &BindGroupEntries::sequential((post.source, &pipes.sampler, uniforms.clone())));
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("filters"),
            color_attachments: &[Some(RenderPassColorAttachment { view: post.destination, resolve_target: None, ops: Operations::default() })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[index.index()]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

#[derive(Resource)]
struct FilterPipeline {
    layout: BindGroupLayout,
    sampler: Sampler,
    sdr: CachedRenderPipelineId,
    hdr: CachedRenderPipelineId,
}

impl FromWorld for FilterPipeline {
    fn from_world(world: &mut World) -> Self {
        let device = world.resource::<RenderDevice>();
        let layout = device.create_bind_group_layout(
            "filters",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (texture_2d(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering), uniform_buffer::<FilterSettings>(true)),
            ),
        );
        let sampler = device.create_sampler(&SamplerDescriptor { mag_filter: FilterMode::Linear, min_filter: FilterMode::Linear, ..default() });
        let cache = world.resource::<PipelineCache>();
        let queue = |format: TextureFormat| {
            cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("filters".into()),
                layout: vec![layout.clone()],
                vertex: fullscreen_shader_vertex_state(),
                fragment: Some(FragmentState {
                    shader: SHADER,
                    shader_defs: vec![],
                    entry_point: "fragment".into(),
                    targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                push_constant_ranges: vec![],
            })
        };
        let sdr = queue(TextureFormat::bevy_default());
        let hdr = queue(ViewTarget::TEXTURE_FORMAT_HDR);
        Self { layout, sampler, sdr, hdr }
    }
}
