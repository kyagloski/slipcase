// Optical disc surface: standard PBR lighting plus
//  - a fake mirror environment (there is no environment map in the scene, so plain
//    metallic surfaces would render almost black), and
//  - diffraction-grating iridescence: the rainbow you see on the data side of a CD.
//
// The mesh supplies a radial tangent (pointing away from the disc centre). The tracks
// run around the disc, so that radial direction is the grating vector.
// Diffraction model after Jos Stam, "Simulating Diffraction" (GPU Gems, ch. 8).

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

struct DiscParams {
    // xyz: direction towards the light (world), w: strength
    light_a: vec4<f32>,
    light_b: vec4<f32>,
    rainbow: f32,
    mirror: f32,
    pad0: f32,
    pad1: f32,
};

@group(2) @binding(100) var<uniform> disc: DiscParams;

fn bump3(x: vec3<f32>) -> vec3<f32> {
    return max(vec3<f32>(1.0) - x * x, vec3<f32>(0.0));
}

fn diffraction(L: vec3<f32>, V: vec3<f32>, R: vec3<f32>, N: vec3<f32>) -> vec3<f32> {
    let nl = dot(N, L);
    if (nl <= 0.0) {
        return vec3<f32>(0.0);
    }
    let H = L + V;
    // Track pitch of a CD is ~1.6 micrometres.
    let u = abs(dot(R, H)) * 1.6;
    var col = vec3<f32>(0.0);
    for (var n = 1; n < 8; n++) {
        let y = 2.0 * u / f32(n) - 1.0;
        col += bump3(vec3<f32>(4.0 * (y - 0.75), 4.0 * (y - 0.5), 4.0 * (y - 0.25)));
    }
    // Keep it saturated instead of blowing out to white where orders overlap.
    let m = max(col.r, max(col.g, col.b));
    if (m > 1.0) {
        col /= m;
    }
    return col * smoothstep(0.0, 0.25, nl);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    let N = pbr_input.N;
    let V = pbr_input.V;

    // Fake environment: bright "sky" above, dark floor, soft side light.
    let refl = reflect(-V, N);
    let sky = smoothstep(-0.35, 0.85, refl.y);
    let side = pow(max(refl.x * 0.7 + 0.3, 0.0), 3.0) * 0.35;
    let fres = 0.75 + 0.25 * pow(1.0 - max(dot(N, V), 0.0), 3.0);
    let env = (vec3<f32>(0.05, 0.055, 0.065) + vec3<f32>(0.62, 0.65, 0.7) * sky + side) * fres;
    var extra = env * disc.mirror * pbr_input.material.base_color.rgb;

#ifdef VERTEX_TANGENTS
    if (disc.rainbow > 0.0) {
        var R = in.world_tangent.xyz;
        R = normalize(R - N * dot(N, R));
        var rb = diffraction(normalize(disc.light_a.xyz), V, R, N) * disc.light_a.w;
        rb += diffraction(normalize(disc.light_b.xyz), V, R, N) * disc.light_b.w;
        // A ring of soft virtual lights around the viewer so the rainbow is visible
        // from the usual straight-on viewing angles, not just at grazing ones.
        for (var k = 0; k < 4; k++) {
            let a = f32(k) * 1.5707963 + 0.6;
            let L = normalize(V + vec3<f32>(cos(a), sin(a), 0.0) * 0.85);
            rb += diffraction(L, V, R, N) * 0.22;
        }
        extra += rb * disc.rainbow * 0.55;
    }
#endif

    out.color = vec4<f32>(out.color.rgb + extra, out.color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
