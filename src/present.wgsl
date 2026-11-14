#import bevy_sprite::mesh2d_vertex_output::VertexOutput

// The scene drawn at the render resolution, shown over the whole window (see resolution.rs).
// Its colours are already multiplied by alpha (it was drawn over a clear, see-through picture),
// so it is blended as premultiplied: the see-through overlay keeps its edges and fades right.
@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var samp: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
