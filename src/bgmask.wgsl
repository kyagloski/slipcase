#import bevy_sprite::mesh2d_vertex_output::VertexOutput

// x = aspect, y = reach (fraction of the screen), z = unused.
@group(2) @binding(0) var<uniform> p: vec4<f32>;

// Output alpha multiplies what the background layer has drawn (see MaskMaterial::specialize):
// 1 in the middle, easing to 0 towards the edges.
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // An ellipse with the screen's shape: 1 at the middle of each edge.
    let q = (in.uv - vec2<f32>(0.5)) * 2.0;
    let d = length(q);
    let r = p.y;
    let a = 1.0 - smoothstep(r * 0.3, r * 1.3, d);
    return vec4<f32>(0.0, 0.0, 0.0, a * a * (3.0 - 2.0 * a));
}
