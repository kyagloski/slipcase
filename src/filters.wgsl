// Screen filters, see filters.rs. Each filter is a bit in `f.flags`; any mix can be on.
// Order: bend / move where to read (CRT curve, VHS wobble, pixel grid), read (with colour
// fringes), change the colours (in sRGB, like the hardware), then lay effects on top.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Filters {
    flags: u32,
    time: f32,
    strength: f32,
    lines: f32,
    // CRT frame (bezel) opacity: 0 shows the plain picture round the curved glass.
    frame: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> f: Filters;

const CRT: u32 = 1u;
const PIXELATE: u32 = 2u;
const POSTERIZE: u32 = 4u;
const GAME_BOY: u32 = 8u;
const VHS: u32 = 16u;
const GRAIN: u32 = 32u;
const VIGNETTE: u32 = 64u;
const BLACK_WHITE: u32 = 128u;
const SEPIA: u32 = 256u;
const CHROMATIC: u32 = 512u;

const PI: f32 = 3.14159265;

fn on(bit: u32) -> bool {
    return (f.flags & bit) != 0u;
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

// 4x4 ordered dither, -0.5..0.5.
fn bayer(p: vec2<f32>) -> f32 {
    var m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    let x = u32(p.x) % 4u;
    let y = u32(p.y) % 4u;
    return (m[y * 4u + x] + 0.5) / 16.0 - 0.5;
}

fn quantize(c: vec3<f32>, levels: vec3<f32>, d: f32) -> vec3<f32> {
    let steps = levels - vec3<f32>(1.0);
    return clamp(floor(c * steps + vec3<f32>(0.5 + d)) / steps, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Signed distance to a rounded box of half size `b` (negative inside).
fn rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// The plastic round the tube: dark, with a soft reflection of the picture near the glass.
fn bezel(c: vec2<f32>, edge: f32) -> vec4<f32> {
    // Mirror the point back into the screen to read what the bezel reflects.
    var r = c;
    r = select(r, sign(r) * (2.0 - abs(r)), abs(r) > vec2<f32>(1.0));
    let ruv = clamp(r * 0.5 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    let blur = 0.012;
    var refl = vec3<f32>(0.0);
    refl = refl + textureSample(screen, samp, ruv + vec2<f32>(blur, 0.0)).rgb;
    refl = refl + textureSample(screen, samp, ruv - vec2<f32>(blur, 0.0)).rgb;
    refl = refl + textureSample(screen, samp, ruv + vec2<f32>(0.0, blur)).rgb;
    refl = refl + textureSample(screen, samp, ruv - vec2<f32>(0.0, blur)).rgb;
    refl = to_srgb(refl * 0.25);
    let fade = exp(-edge * 30.0);
    var col = vec3<f32>(0.035, 0.034, 0.032) + refl * 0.22 * fade * f.strength;
    col = apply_tint(col);
    // A thin dark lip right at the glass.
    col = col * smoothstep(0.0, 0.012, edge);
    return vec4<f32>(to_linear(col), 1.0);
}

// The CRT phosphor colour (bits 16-17): 0 keeps the colours, else one colour like a terminal.
fn apply_tint(c: vec3<f32>) -> vec3<f32> {
    let mode = (f.flags >> 16u) & 3u;
    if mode == 0u || (f.flags & CRT) == 0u {
        return c;
    }
    var tint = vec3<f32>(1.0, 0.506, 0.0);
    if mode == 2u {
        tint = vec3<f32>(0.2, 1.0, 0.35);
    } else if mode == 3u {
        tint = vec3<f32>(0.92, 0.95, 1.0);
    }
    // Brighter parts burn towards white, like a real phosphor.
    let l = luma(c);
    return mix(tint * l * 1.25, vec3<f32>(1.0), smoothstep(0.7, 1.2, l) * 0.35);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(screen));
    let s = f.strength;
    let t = f.time;
    var uv = in.uv;

    // --- Where to read ---------------------------------------------------------------------
    if on(CRT) {
        // Curved glass, with rounded corners, sitting in a dark bezel.
        var c = in.uv * 2.0 - 1.0;
        let k = vec2<f32>(0.11, 0.15) * s;
        c = c * (vec2<f32>(1.0) + c.yx * c.yx * k);
        let edge = rounded_box(c, vec2<f32>(1.0), 0.09);
        if edge > 0.0 {
            let a = clamp(f.frame, 0.0, 1.0);
            if a <= 0.0 {
                return textureSample(screen, samp, in.uv);
            }
            return mix(textureSample(screen, samp, in.uv), bezel(c, edge), a);
        }
        uv = c * 0.5 + 0.5;
        // Each line shivers sideways a little (unsteady horizontal sync).
        let row = floor(uv.y * dims.y / 2.0);
        uv.x = uv.x + (hash(vec2<f32>(row, floor(t * 30.0))) - 0.5) * 0.0011 * s;
    }
    if on(VHS) {
        // Lines wobble sideways; a tracking band rolls down the picture and tears it more.
        let row = floor(uv.y * 240.0);
        var shift = sin(uv.y * 30.0 + t * 2.3) * 0.0012 + (hash(vec2<f32>(row, floor(t * 24.0))) - 0.5) * 0.0018;
        let band = fract(t * 0.07);
        let near = 1.0 - smoothstep(0.0, 0.035, abs(uv.y - band));
        shift = shift + near * (hash(vec2<f32>(row, t)) - 0.5) * 0.02;
        uv.x = uv.x + shift * s;
    }
    // Size of one retro pixel, in screen pixels.
    let block = max(dims.y / f.lines, 1.0);
    var cell = in.position.xy;
    if on(PIXELATE) {
        cell = floor(uv * dims / block);
        uv = (cell + 0.5) * block / dims;
    }

    // --- Read, with colour fringes -----------------------------------------------------------
    var radial = 0.0;
    var across = 0.0;
    if on(CHROMATIC) {
        radial = radial + 0.012 * s;
    }
    if on(CRT) {
        radial = radial + 0.002 * s;
    }
    if on(VHS) {
        across = across + 0.003 * s;
    }
    let off = (uv - 0.5) * radial * 2.0 * length(uv - 0.5) + vec2<f32>(across, 0.0);
    let mid = textureSample(screen, samp, uv);
    var lin = mid.rgb;
    if radial > 0.0 || across > 0.0 {
        lin = vec3<f32>(textureSample(screen, samp, uv + off).r, mid.g, textureSample(screen, samp, uv - off).b);
    }
    if on(CRT) {
        // Phosphor glow: light bleeds round everything bright. Six reads on one ring, between
        // pixels so the filtering blurs each read over four of them (cheap, and soft enough).
        let px = vec2<f32>(dims.y / dims.x, 1.0) * (9.0 / dims.y);
        var glow = textureSample(screen, samp, uv + vec2<f32>(1.0, 0.0) * px).rgb;
        glow = glow + textureSample(screen, samp, uv + vec2<f32>(0.5, 0.866) * px).rgb;
        glow = glow + textureSample(screen, samp, uv + vec2<f32>(-0.5, 0.866) * px).rgb;
        glow = glow + textureSample(screen, samp, uv + vec2<f32>(-1.0, 0.0) * px).rgb;
        glow = glow + textureSample(screen, samp, uv + vec2<f32>(-0.5, -0.866) * px).rgb;
        glow = glow + textureSample(screen, samp, uv + vec2<f32>(0.5, -0.866) * px).rgb;
        lin = lin + glow / 6.0 * 0.75 * s;
    }
    var col = to_srgb(lin);
    let d = bayer(cell);

    // --- Colours -----------------------------------------------------------------------------
    if on(VHS) {
        // Washed out, a little warm, with the chroma smeared.
        col = mix(col, vec3<f32>(luma(col)), 0.3 * s);
        col = col * vec3<f32>(1.04, 1.0, 0.94);
    }
    if on(BLACK_WHITE) {
        let g = clamp((luma(col) - 0.5) * 1.1 + 0.5, 0.0, 1.0);
        col = mix(col, vec3<f32>(g), clamp(s, 0.0, 1.0));
    }
    if on(SEPIA) {
        let sep = vec3<f32>(dot(col, vec3<f32>(0.393, 0.769, 0.189)), dot(col, vec3<f32>(0.349, 0.686, 0.168)), dot(col, vec3<f32>(0.272, 0.534, 0.131)));
        col = mix(col, clamp(sep, vec3<f32>(0.0), vec3<f32>(1.0)), clamp(s, 0.0, 1.0));
    }
    if on(GAME_BOY) {
        // The DMG's four greens, dithered.
        let l = clamp(luma(col) + d * 0.3, 0.0, 0.999);
        let i = u32(floor(l * 4.0));
        var pal = array<vec3<f32>, 4>(
            vec3<f32>(0.059, 0.220, 0.059),
            vec3<f32>(0.188, 0.384, 0.188),
            vec3<f32>(0.545, 0.675, 0.059),
            vec3<f32>(0.608, 0.737, 0.059),
        );
        col = mix(col, pal[i], clamp(s, 0.0, 1.0));
    } else if on(POSTERIZE) {
        // 256 colours: 3 bits red, 3 green, 2 blue.
        col = mix(col, quantize(col, vec3<f32>(8.0, 8.0, 4.0), d * 0.9), clamp(s, 0.0, 1.0));
    } else if on(PIXELATE) {
        // 15-bit colour, 32 levels a channel.
        col = mix(col, quantize(col, vec3<f32>(32.0), d * 0.5), clamp(s, 0.0, 1.0));
    }

    col = apply_tint(col);

    // --- On top ------------------------------------------------------------------------------
    if on(CRT) {
        // Scanlines: one per retro line when pixelated, else every third screen line.
        var period = 3.0;
        if on(PIXELATE) {
            period = block;
        }
        let sl = 0.5 + 0.5 * cos(2.0 * PI * (in.position.y / period));
        // Bright lines bloom over the gaps, so the dark lines show most in the dark parts.
        let lift = clamp(luma(col), 0.0, 1.0);
        col = col * mix(1.0, mix(0.2, 0.75, lift) + (1.0 - mix(0.2, 0.75, lift)) * sl, clamp(s, 0.0, 1.0));
        // Shadow mask: red, green and blue phosphor stripes.
        let m = u32(in.position.x) % 3u;
        var mask = vec3<f32>(0.55);
        if m == 0u {
            mask.r = 1.0;
        } else if m == 1u {
            mask.g = 1.0;
        } else {
            mask.b = 1.0;
        }
        col = col * mix(vec3<f32>(1.0), mask, clamp(0.75 * s, 0.0, 1.0));
        // Make up for the dark lines and the mask.
        col = col * (1.0 + 0.8 * s);
        // Flicker, static, and a soft bright band rolling up the screen.
        col = col * (1.0 + (sin(t * 47.0) * 0.5 + hash(vec2<f32>(floor(t * 60.0), 1.0)) - 0.5) * 0.06 * s);
        col = col + (hash(in.position.xy + vec2<f32>(t * 71.0, t * 13.0)) - 0.5) * 0.07 * s;
        let band = fract(-t * 0.12);
        col = col * (1.0 + 0.12 * s * exp(-pow((uv.y - band) * 9.0, 2.0)));
        // Dark towards the curved edges of the glass.
        let e = uv * (1.0 - uv);
        col = col * clamp(pow(e.x * e.y * 18.0, 0.35), 0.0, 1.0);
    }
    if on(VHS) {
        // Snow, and a bright noisy line in the tracking band.
        let n = hash(in.position.xy + vec2<f32>(t * 91.0, t * 37.0));
        col = col + (n - 0.5) * 0.08 * s;
        let band = fract(t * 0.07);
        let near = 1.0 - smoothstep(0.0, 0.012, abs(in.uv.y - band));
        col = col + near * n * 0.35 * s;
    }
    if on(GRAIN) {
        let n = hash(floor(in.position.xy) + fract(t * 7.0) * 113.0) - 0.5;
        col = col + n * 0.12 * s;
    }
    if on(VIGNETTE) {
        let c = in.uv - 0.5;
        let v = clamp(1.0 - dot(c, c) * 1.6 * s, 0.0, 1.0);
        col = col * v;
    }

    return vec4<f32>(to_linear(col), mid.a);
}
