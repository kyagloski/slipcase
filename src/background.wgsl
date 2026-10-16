#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct BgParams {
    time: f32,
    style: f32,
    intensity: f32,
    speed: f32,
    aspect: f32,
    scroll: f32,
    pad0: f32,
    pad1: f32,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    color_c: vec4<f32>,
    // PS2 style: orb / trail points (x, y, trail width, orb size), 16 per orb; bounding boxes.
    orbs: array<vec4<f32>, 112>,
    orb_box: array<vec4<f32>, 7>,
};

@group(2) @binding(0) var<uniform> p: BgParams;

fn p_aspect() -> f32 {
    return p.aspect;
}

fn p_scroll() -> f32 {
    return p.scroll;
}

fn hash21(q: vec2<f32>) -> f32 {
    var v = fract(q * vec2<f32>(123.34, 456.21));
    v += dot(v, v + 45.32);
    return fract(v.x * v.y);
}

fn vnoise(q: vec2<f32>) -> f32 {
    let i = floor(q);
    let f = fract(q);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(q_in: vec2<f32>) -> f32 {
    var q = q_in;
    var amp = 0.5;
    var sum = 0.0;
    for (var i = 0; i < 5; i++) {
        sum += amp * vnoise(q);
        q = q * 2.03 + vec2<f32>(17.1, 9.2);
        amp *= 0.5;
    }
    return sum;
}

fn stars(q: vec2<f32>, scale: f32, density: f32, t: f32) -> f32 {
    let sq = q * scale;
    let h = hash21(floor(sq));
    let off = vec2<f32>(hash21(floor(sq) + 3.1), hash21(floor(sq) + 7.7)) - 0.5;
    let d = length(fract(sq) - 0.5 - off * 0.6);
    return step(1.0 - density, h) * smoothstep(0.18, 0.0, d) * (0.55 + 0.45 * sin(t * (2.0 + h * 5.0) + h * 60.0));
}

// Northern lights over a dark ridge: several tall curtains with a sharp bright lower edge
// fading upwards, fine vertical rays drifting through them, brightness pulses running along the
// folds, the cover's colour at the base turning to its accent higher up.
fn aurora(uv: vec2<f32>, t: f32) -> vec3<f32> {
    let q = vec2<f32>(uv.x * p.aspect + p.scroll * 0.06, uv.y);
    var col = mix(p.color_a.rgb * 0.45, p.color_a.rgb * 0.06, smoothstep(0.1, 0.9, uv.y));
    var light = vec3<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let fi = f32(i);
        // The curtain's foot, folding as it drifts.
        let xw = q.x * (0.7 + fi * 0.15) + t * (0.12 + fi * 0.05);
        let fold = fbm(vec2<f32>(xw * 1.4, fi * 4.1 + t * 0.15));
        // Low on the sky (just above the ridge), reaching most of the way to the top.
        let base = 0.3 + fi * 0.06 + 0.1 * sin(xw * 1.7 + fi * 2.3 + t * 0.35) + (fold - 0.5) * 0.3;
        let h = uv.y - base;
        // Sharp lower edge, long fade upwards (taller for the farther curtains).
        let body = smoothstep(-0.01, 0.003, h) * exp(-max(h, 0.0) * (4.2 - fi * 0.6));
        // Rays: thin vertical streaks that follow the folds and shimmer.
        let rx = xw * 38.0 + fold * 6.0;
        let ray = pow(vnoise(vec2<f32>(rx, t * 0.8 + fi * 9.0)), 2.5) * 2.0 + 0.12;
        // Brightness pulses running along the curtain.
        let pulse = 0.55 + 0.45 * sin(xw * 6.0 - t * 2.2 + fi * 1.7) * sin(xw * 2.3 + t * 0.9);
        let tint = mix(p.color_b.rgb, p.color_c.rgb, smoothstep(0.0, 0.45, h) * 0.85 + fi * 0.05);
        let edge = exp(-abs(h) * 120.0) * 0.6;
        light += tint * (body * ray * pulse + edge * pulse) * (0.75 - fi * 0.12);
    }
    // A soft glow over the whole display, and the stars dimmed behind the bright parts.
    col = col * (1.0 - clamp(length(light), 0.0, 0.7)) + light + p.color_b.rgb * 0.025 * smoothstep(0.3, 0.6, uv.y);
    // A dark ridge along the bottom.
    let ridge = 0.16 + 0.07 * fbm(vec2<f32>(q.x * 1.6 + 3.0, 1.0)) + 0.025 * fbm(vec2<f32>(q.x * 7.0, 4.0));
    let land = smoothstep(ridge + 0.003, ridge - 0.003, uv.y);
    col = mix(col, p.color_a.rgb * 0.12 + light * 0.05, land);
    return col;
}

// Voronoi distances (nearest, second, third) to scattered points.
fn worley3(q: vec2<f32>) -> vec3<f32> {
    let i = floor(q);
    let f = fract(q);
    var d = vec3<f32>(9.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let g = vec2<f32>(f32(x), f32(y));
            let o = vec2<f32>(hash21(i + g), hash21(i + g + 19.19));
            let r = length(g + o - f);
            if (r < d.x) {
                d = vec3<f32>(r, d.x, d.y);
            } else if (r < d.y) {
                d = vec3<f32>(d.x, r, d.y);
            } else if (r < d.z) {
                d.z = r;
            }
        }
    }
    return d;
}

// The cosmic web: glowing filaments between voids, bright galaxy clusters where they meet, and
// star dust along them, slowly drifting, in the cover's colours.
fn galactic(uv: vec2<f32>, t: f32) -> vec3<f32> {
    let q0 = vec2<f32>(uv.x * p.aspect + p.scroll * 0.03, uv.y);
    var col = p.color_a.rgb * 0.06;
    var web = 0.0;
    var nodes = 0.0;
    for (var i = 0; i < 2; i++) {
        let fi = f32(i);
        let sc = 4.5 + fi * 6.0;
        // Warped so the filaments wander instead of running straight.
        let w1 = vec2<f32>(fbm(q0 * 1.3 + vec2<f32>(t * 0.16, fi * 5.0)), fbm(q0 * 1.3 + vec2<f32>(7.0 + fi, t * 0.13))) - 0.5;
        let w2 = vec2<f32>(fbm(q0 * 4.0 + w1 * 2.0 + vec2<f32>(3.3, t * 0.2)), fbm(q0 * 4.0 + w1 * 2.0 + vec2<f32>(8.8, -t * 0.17))) - 0.5;
        let q = q0 * sc + w1 * (3.2 - fi * 1.2) + w2 * 0.9 + vec2<f32>(t * 0.02, -t * 0.015) * (1.0 + fi);
        let d = worley3(q);
        let edge = d.y - d.x;
        let strength = (1.0 - fi * 0.45);
        // Large-scale density: many filaments fade away entirely, leaving dark voids.
        let clumps = smoothstep(0.38, 0.72, fbm(q0 * (1.6 + fi) + vec2<f32>(11.0 + fi * 3.0, t * 0.01)));
        let bead = 0.5 + 0.8 * vnoise(q * 6.0);
        web += (exp(-edge * 60.0) * 1.1 * bead + exp(-edge * 12.0) * 0.18) * strength * clumps;
        // Where three cells meet: a cluster.
        nodes += exp(-(d.z - d.x) * 40.0) * strength * smoothstep(0.35, 0.7, fbm(q0 * (1.6 + fi) + vec2<f32>(11.0 + fi * 3.0, t * 0.01)));
    }
    let fil = mix(p.color_b.rgb, p.color_c.rgb, smoothstep(0.3, 1.2, web));
    col += fil * web * 0.38;
    col += mix(p.color_c.rgb, vec3<f32>(1.0), 0.45) * nodes * 0.9;
    // Stars crowding along the filaments (several sizes), and a sprinkling across the voids.
    let along = clamp(web, 0.0, 1.0);
    // Thickest right on the filaments, so the web reads as rivers of stars.
    let core = clamp(web * 1.6, 0.0, 1.0);
    var dust = stars(q0, 75.0, 0.02 + core * 0.55, t) * 1.5;
    dust += stars(q0 + 1.3, 130.0, 0.03 + core * 0.75, t * 1.2) * 1.2;
    dust += stars(q0 + 4.1, 220.0, 0.04 + core * 0.9, t * 0.8) * 0.9;
    let star_col = mix(vec3<f32>(1.0), mix(p.color_b.rgb, p.color_c.rgb, 0.5) * 1.6, 0.35);
    col += star_col * dust * (0.7 + along);
    col += vec3<f32>(stars(q0 + 2.7, 70.0, 0.02, t)) * 0.8;
    // Faint nebulous glow over the densest regions.
    col += p.color_b.rgb * smoothstep(0.35, 0.9, fbm(q0 * 1.3 + t * 0.01)) * 0.12;
    return col;
}

fn waves(uv: vec2<f32>, t: f32) -> vec3<f32> {
    var col = mix(p.color_a.rgb * 0.5, p.color_a.rgb, uv.y);
    let x = uv.x * p.aspect + p.scroll * 0.08;
    for (var i = 0; i < 5; i++) {
        let fi = f32(i);
        let y = 0.25 + fi * 0.11 + 0.06 * sin(x * (1.6 + fi * 0.35) + t * (1.0 + fi * 0.2)) + 0.03 * sin(x * 4.3 - t * 1.3 + fi);
        let d = uv.y - y;
        let line = smoothstep(0.02, 0.0, abs(d));
        let fill = smoothstep(0.0, -0.5, d) * 0.06;
        let tint = mix(p.color_b.rgb, p.color_c.rgb, fi / 4.0);
        col += tint * (line * 0.65 + fill);
    }
    return col;
}

fn nebula(uv: vec2<f32>, t: f32) -> vec3<f32> {
    let q = vec2<f32>(uv.x * p.aspect + p.scroll * 0.05, uv.y) * 2.2;
    let w1 = fbm(q + vec2<f32>(t * 0.5, -t * 0.3));
    let w2 = fbm(q + vec2<f32>(w1 * 2.2, w1 * 1.7) + vec2<f32>(-t * 0.4, t * 0.2));
    var col = mix(p.color_a.rgb, p.color_b.rgb, smoothstep(0.25, 0.85, w2));
    col = mix(col, p.color_c.rgb, smoothstep(0.55, 1.0, w1) * 0.7);
    // stars
    let sq = vec2<f32>(uv.x * p.aspect, uv.y) * 90.0;
    let h = hash21(floor(sq));
    let star = step(0.985, h) * (0.5 + 0.5 * sin(t * 3.0 + h * 40.0)) * smoothstep(0.5, 0.0, length(fract(sq) - 0.5));
    return col * 0.8 + vec3<f32>(star);
}

fn gradient(uv: vec2<f32>, t: f32) -> vec3<f32> {
    let c = vec2<f32>(0.5 + 0.2 * sin(t * 0.7), 0.5 + 0.15 * cos(t * 0.5));
    let d = length((uv - c) * vec2<f32>(p.aspect, 1.0));
    var col = mix(p.color_b.rgb * 0.8, p.color_a.rgb, smoothstep(0.0, 1.1, d));
    col = mix(col, p.color_c.rgb * 0.5, smoothstep(0.7, 1.6, d + uv.y * 0.3));
    return col;
}

// Turns a colour round the colour wheel by `a` radians (a rotation about the grey axis), which
// changes smoothly as the colour blends, unlike picking out its hue.
fn hue_turn(c: vec3<f32>, a: f32) -> vec3<f32> {
    let k = vec3<f32>(0.57735);
    let ca = cos(a);
    return c * ca + cross(k, c) * sin(a) + k * dot(k, c) * (1.0 - ca);
}

// Brightest channel at 1 (keeps the hue), then `sat` of the way from grey to it.
fn vivid(c: vec3<f32>, sat: f32) -> vec3<f32> {
    let m = max(max(c.r, c.g), max(c.b, 1e-3));
    let v = c / m;
    let grey = vec3<f32>(dot(v, vec3<f32>(0.299, 0.587, 0.114)));
    return mix(grey, v, sat);
}

// Outrun: a dusk sky over a glowing grid that rolls towards you (and slides sideways as you
// browse). Two colour sets from the selected game: its own colour for the grid near you, and
// that colour turned a third of the way round the wheel (like outrun's pink and cyan) for the
// horizon, the far grid and the sky. The sky and ground are softer than the grid.
fn synthwave(uv: vec2<f32>, t: f32) -> vec3<f32> {
    let horizon = 0.5;
    let x = (uv.x - 0.5) * p.aspect;
    let base = vivid(max(p.color_b.rgb, vec3<f32>(0.0)), 1.0);
    let other = clamp(hue_turn(base, -2.1), vec3<f32>(0.0), vec3<f32>(1.0));
    let near = vivid(base, 0.85);
    let farc = vivid(other, 0.72);
    let haze = vivid(hue_turn(other, 0.25), 0.45);
    var col: vec3<f32>;
    if (uv.y >= horizon) {
        // Sky: a deep, soft shade of the game's colour overhead, the second colour glowing at
        // the horizon, with a few stars.
        let h = (uv.y - horizon) / (1.0 - horizon);
        // (Linear colour: small values are already dark on screen; the sky is near black.)
        let mid = vivid(hue_turn(other, 0.4), 0.4) * 0.035;
        let top = vivid(hue_turn(base, 0.3), 0.45) * 0.006;
        col = mix(haze * 0.22, mid, smoothstep(0.0, 0.18, h));
        col = mix(col, top, smoothstep(0.25, 0.9, h));
        let sq = vec2<f32>(x, uv.y) * 70.0;
        let s = hash21(floor(sq));
        col += vec3<f32>(step(0.992, s) * smoothstep(0.45, 0.0, length(fract(sq) - 0.5)) * smoothstep(0.3, 0.8, h) * (0.6 + 0.4 * sin(t * 4.0 + s * 50.0)));
    } else {
        // Floor: a grid in perspective, lines fading into the haze at the horizon.
        let below = horizon - uv.y;
        let z = 0.32 / max(below, 0.002);
        // About a dozen columns across the screen near you, rows rolling in from the horizon.
        let wx = x * z * 7.0 + p.scroll * 0.6;
        let wz = z * 7.0 + t * 3.0;
        let gx = abs(fract(wx) - 0.5) / max(fwidth(wx), 1e-4);
        let gz = abs(fract(wz) - 0.5) / max(fwidth(wz), 1e-4);
        let g = min(gx, gz);
        // A solid core about six pixels wide, with a wide glow round it.
        let line = 1.0 - smoothstep(2.4, 4.2, g);
        let glow = exp(-g * 0.45) * 0.18;
        let far = smoothstep(0.0, 0.2, below);
        let ground = vivid(hue_turn(base, 0.2), 0.4) * 0.004;
        col = mix(haze * 0.12, ground, smoothstep(0.0, 0.08, below));
        // The core is drawn in the colour itself (not added on), so it stays rich instead of
        // blowing out to white; only the glow adds light.
        let lc = mix(farc, near, smoothstep(0.05, 0.45, below));
        col += lc * glow * (0.4 + 0.6 * far);
        col = mix(col, lc * (0.3 + 0.32 * far), line);
    }
    // A bright line along the horizon.
    col += haze * 0.45 * exp(-abs(uv.y - horizon) * 160.0);
    return col;
}

fn srgb_lin(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// PS2 system menu, after the recreation at imsaud.me: a black screen with 7 glowing orbs (white
// core, white / cyan / blue glow, screen-blended) circling in a plane that tumbles with the clock,
// each drawing a continuous trail. Their positions are worked out once a frame on the CPU
// (`ps2_orbs` in background.rs); here each pixel only measures its distance to them.
fn ps2(uv: vec2<f32>, t: f32) -> vec3<f32> {
    let design_w = 2560.0;
    let design_h = design_w / p_aspect();
    let px = vec2<f32>(uv.x * design_w, (1.0 - uv.y) * design_h);
    // Glow colours: cyan and blue, or the selected game's colours when they are followed.
    let follow = clamp(p.pad1, 0.0, 1.0);
    let main_c = vivid(max(p.color_b.rgb, vec3<f32>(0.0)), 1.0);
    let cyan_c = mix(vec3<f32>(0.0, 0.8, 1.0), main_c, follow);
    let blue_c = mix(vec3<f32>(0.0, 0.62, 1.0), clamp(hue_turn(main_c, 0.35), vec3<f32>(0.0), vec3<f32>(1.0)), follow);
    let trail_c = mix(vec3<f32>(0.0, 0.85, 1.0), main_c, follow);
    var keep = vec3<f32>(1.0);
    for (var i = 0; i < 7; i++) {
        // Nowhere near this orb or its trail: nothing to add.
        let bb = p.orb_box[i];
        if (px.x < bb.x || px.y < bb.y || px.x > bb.z || px.y > bb.w) {
            continue;
        }
        let base = i * 16;
        // The trail: segments joining the points, narrowing and fading along it.
        var tr = 0.0;
        for (var j = 1; j < 16; j++) {
            let a0 = p.orbs[base + j - 1];
            let a1 = p.orbs[base + j];
            let seg = a1.xy - a0.xy;
            let h = clamp(dot(px - a0.xy, seg) / max(dot(seg, seg), 1e-4), 0.0, 1.0);
            let d = length(px - a0.xy - seg * h);
            let kk = mix(a0.z, a1.z, h);
            let fade = 1.0 - (f32(j - 1) + h) / 16.0;
            tr = max(tr, (1.0 - smoothstep(0.0, 16.0 * kk, d)) * fade * 0.55);
        }
        keep *= vec3<f32>(1.0) - trail_c * tr;
        // The orb: box-shadow 0 0 10px 4px #fff, 0 0 20px 10px #0cf, 0 0 40px 15px #05f on an
        // 8 px dot (the outer glow pulled towards cyan).
        let o = p.orbs[base];
        let k = o.w;
        let d = length(px - o.xy);
        let core = 1.0 - smoothstep(3.0 * k, 4.0 * k, d);
        let white = max(core, 1.0 - smoothstep(3.0 * k, 13.0 * k, d));
        let cyan = 1.0 - smoothstep(4.0 * k, 24.0 * k, d);
        let blue = 1.0 - smoothstep(0.0, 39.0 * k, d);
        keep *= vec3<f32>(1.0) - blue_c * blue;
        keep *= vec3<f32>(1.0) - cyan_c * cyan;
        keep *= vec3<f32>(1.0) - vec3<f32>(1.0) * white;
    }
    return srgb_lin(vec3<f32>(1.0) - keep);
}

// Original Xbox dashboard: very dark green, faint drifting and whirling smoke, glowing waves sweeping across, a
// soft glow in the middle, and a dense, bright green diamond lattice, warped like a dome, cutting
// through it all.
fn xbox(uv: vec2<f32>, t: f32) -> vec3<f32> {
    var q = (uv - vec2<f32>(0.5)) * 2.0;
    q.x *= p_aspect();
    let r = length(q * vec2<f32>(0.5, 0.58));
    // Screen-space colours, made linear at the end.
    // Very dark green behind the lattice.
    var col = mix(vec3<f32>(0.035, 0.1, 0.02), vec3<f32>(0.008, 0.03, 0.006), smoothstep(0.0, 0.95, r));
    // Smoke.
    let sq = q * 0.5;
    let sm = fbm(sq * 2.2 + vec2<f32>(t * 0.05, -t * 0.03) + fbm(sq * 1.5 - t * 0.02) * 1.5);
    col += vec3<f32>(0.04, 0.13, 0.02) * smoothstep(0.35, 0.95, sm) * 0.8;
    // A slight whirl of smoke turning round the middle (twisting more towards the centre).
    let wr = length(sq);
    let wa = t * 0.12 + 1.6 / (wr + 0.35);
    let wq = vec2<f32>(sq.x * cos(wa) - sq.y * sin(wa), sq.x * sin(wa) + sq.y * cos(wa));
    let whirl = fbm(wq * 2.6 + vec2<f32>(0.0, t * 0.04));
    col += vec3<f32>(0.06, 0.2, 0.035) * smoothstep(0.45, 0.95, whirl) * smoothstep(1.3, 0.15, wr) * 0.9;
    // Waves: glowing curves sweeping across the screen.
    let x = q.x * 0.5 + p_scroll() * 0.01;
    for (var i = 0; i < 4; i++) {
        let fi = f32(i);
        let yy = 0.5 + 0.28 * sin(x * (1.3 + fi * 0.35) + t * (0.25 + fi * 0.07) + fi * 1.9) + 0.08 * sin(x * 3.1 - t * 0.4 + fi);
        let d = abs(uv.y - yy);
        let pulse = 0.6 + 0.4 * sin(x * 4.0 - t * 1.6 + fi * 2.0);
        col += vec3<f32>(0.3, 0.85, 0.2) * (exp(-d * 260.0) * 0.6 + exp(-d * 38.0) * 0.08) * pulse;
    }
    // A soft glow in the middle.
    col += vec3<f32>(0.08, 0.22, 0.04) * exp(-r * 4.0) * (0.85 + 0.15 * sin(t * 1.2)) * 0.5;
    // The diamond lattice, centred and warped like a dome (bulging towards you in the middle),
    // drawn over everything.
    let rr = length(q * 0.5);
    let dome = q * 0.5 * (1.0 + 0.55 * rr * rr) / (1.0 + 0.25 * rr);
    // Diamonds stretched tall (the vertical squeezed into the lines), and denser.
    let lu = (dome.x * 19.0 + dome.y * 9.5) + t * 0.03;
    let lv = (dome.x * 19.0 - dome.y * 9.5) - t * 0.03;
    let gu = abs(fract(lu) - 0.5) / max(fwidth(lu), 1e-4);
    let gv = abs(fract(lv) - 0.5) / max(fwidth(lv), 1e-4);
    let g = min(gu, gv);
    let lattice = 1.0 - smoothstep(1.0, 2.2, g);
    // Bright green lines with a faint glow, brightest in the middle.
    let lit = vec3<f32>(0.22, 0.62, 0.12) * (1.0 - 0.35 * smoothstep(0.2, 1.1, r));
    // Bloom: a soft glow spreading from the lines.
    col += lit * (exp(-g * 0.3) * 0.18 + exp(-g * 0.08) * 0.05);
    col = mix(col, lit, lattice * 0.9);

    // A large circle of hexagon lattice round the middle, curved like the front of a sphere.
    let cq = q * 0.5;
    let cr = length(cq);
    let disc_r = 0.62;
    if (cr < disc_r * 1.08) {
        let z = sqrt(max(disc_r * disc_r - cr * cr, 0.0));
        let sph = cq / (z + disc_r * 0.6) * 1.4;
        let spin = t * 0.05;
        let sp = vec2<f32>(sph.x * cos(spin) - sph.y * sin(spin), sph.x * sin(spin) + sph.y * cos(spin));
        let hq = sp * 7.0;
        let hs = vec2<f32>(1.0, 1.7320508);
        let ha = hq - hs * floor(hq / hs + 0.5);
        let hb = (hq - hs * 0.5) - hs * floor((hq - hs * 0.5) / hs + 0.5);
        let hg = select(hb, ha, dot(ha, ha) < dot(hb, hb));
        // Distance to the cell's centre in hexagon terms (0.5 on its edge).
        let hd = max(dot(abs(hg), vec2<f32>(0.5, 0.8660254)), abs(hg.x));
        let edge_px = max(fwidth(hd), 1e-4);
        let hex_line = 1.0 - smoothstep(edge_px * 0.6, edge_px * 2.4, 0.5 - hd);
        // Brighter towards the rim (the sphere's edge), fading out at it.
        let rim = smoothstep(0.0, disc_r, cr);
        let fade = 1.0 - smoothstep(disc_r * 0.92, disc_r * 1.06, cr);
        let hex_col = vec3<f32>(0.25, 0.75, 0.15) * (0.45 + 0.55 * rim);
        // Darken what is behind it, so the hexagons read clearly over the diamonds.
        col = mix(col, vec3<f32>(0.01, 0.035, 0.008), fade * 0.82);
        col = mix(col, hex_col, hex_line * fade * 0.85);
        // Bloom round the hexagon lines.
        let hex_dist = (0.5 - hd) / edge_px;
        col += hex_col * (exp(-max(hex_dist, 0.0) * 0.3) * 0.2 + exp(-max(hex_dist, 0.0) * 0.08) * 0.05) * fade;
        // A thin bright ring round its edge.
        col += vec3<f32>(0.3, 0.85, 0.2) * exp(-abs(cr - disc_r) * 140.0) * 0.6;
    }

    // The pulsing green blob in the middle: a wobbling, organic edge, a bright yellow-green core.
    let ba = atan2(cq.y, cq.x);
    // The blob's own, quicker clock.
    let bt = t * 1.5;
    let pulse = 0.5 + 0.5 * sin(bt * 1.6);
    let wob = 0.03 * sin(ba * 3.0 + bt * 1.3) + 0.02 * sin(ba * 5.0 - bt * 1.9) + 0.025 * (fbm(vec2<f32>(ba * 1.2, bt * 0.4)) - 0.5);
    let blob_r = 0.2 + 0.015 * pulse + wob;
    let inside = 1.0 - smoothstep(blob_r - 0.012, blob_r + 0.012, cr);
    let depth = clamp(1.0 - cr / blob_r, 0.0, 1.0);
    let swirl = fbm(cq * 9.0 + vec2<f32>(bt * 0.3, -bt * 0.25));
    let blob_col = mix(vec3<f32>(0.2, 0.6, 0.08), vec3<f32>(0.85, 1.0, 0.35), depth * depth * (0.7 + 0.3 * pulse)) * (0.75 + 0.35 * swirl);
    col = mix(col, blob_col, inside * 0.92);
    // Its glow.
    col += vec3<f32>(0.3, 0.9, 0.15) * exp(-max(cr - blob_r, 0.0) * 14.0) * (0.25 + 0.2 * pulse) * (1.0 - inside);
    // Following the game's colours: turn the whole scene from green to the game's hue (by how
    // colourful the game's colour is, so grey covers keep the green).
    let game = max(p.color_b.rgb, vec3<f32>(0.0));
    let green_h = atan2(1.7320508 * (0.85 - 0.2), 2.0 * 0.3 - 0.85 - 0.2);
    let game_h = atan2(1.7320508 * (game.g - game.b), 2.0 * game.r - game.g - game.b);
    let mx = max(max(game.r, game.g), max(game.b, 1e-4));
    let sat = (mx - min(min(game.r, game.g), game.b)) / mx;
    let amount = clamp(p.pad1, 0.0, 1.0) * smoothstep(0.08, 0.3, sat);
    col = max(hue_turn(col, (game_h - green_h) * amount), vec3<f32>(0.0));
    return srgb_lin(col);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
    let t = p.time * p.speed * 0.25;
    var col: vec3<f32>;
    let style = i32(p.style + 0.5);
    if (style == 0) {
        col = aurora(uv, t);
    } else if (style == 1) {
        col = waves(uv, t);
    } else if (style == 2) {
        col = nebula(uv, t);
    } else if (style == 7) {
        // Driven by the clock (seconds of the day), like the real menu.
        col = ps2(uv, p.pad0);
    } else if (style == 8) {
        col = xbox(uv, p.time * p.speed * 0.6);
    } else if (style == 6) {
        col = galactic(uv, p.time * p.speed * 0.25);
    } else if (style == 5) {
        col = synthwave(uv, p.time * p.speed);
    } else if (style == 3) {
        col = gradient(uv, t);
    } else {
        col = p.color_a.rgb;
    }
    col = col * p.intensity;
    // vignette
    let v = length((in.uv - vec2<f32>(0.5)) * vec2<f32>(1.0, 0.8));
    col *= 1.0 - smoothstep(0.35, 0.95, v) * 0.55;
    // dither to avoid banding in dark gradients
    col += (hash21(in.uv * 1000.0 + p.time) - 0.5) / 255.0;
    return vec4<f32>(col, 1.0);
}
