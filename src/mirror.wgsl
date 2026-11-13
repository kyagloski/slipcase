// Mirror floor: shows the mirror camera's picture (the scene seen from below the floor) at
// this pixel, upside down, fading out away from the game.
#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings::view}

// xyz: the game's base (where the fade starts), w: strength
@group(2) @binding(0) var<uniform> params: vec4<f32>;
@group(2) @binding(1) var mirror_tex: texture_2d<f32>;
@group(2) @binding(2) var mirror_samp: sampler;
// xyz: towards the camera along the floor, w: fade length
@group(2) @binding(3) var<uniform> fade_dir: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = (in.position.xy - view.viewport.xy) / view.viewport.zw;
    let c = textureSample(mirror_tex, mirror_samp, vec2<f32>(s.x, 1.0 - s.y));
    let f = 1.0 - clamp(dot(in.world_position.xyz - params.xyz, fade_dir.xyz) / fade_dir.w, 0.0, 1.0);
    return vec4<f32>(c.rgb, c.a * f * f * params.w);
}
