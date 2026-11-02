// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

// Glass under wgpu (src/glass_gpu.rs): each card drawn from the canvas and,
// when it overlaps earlier UI, a grab of the lower cards over it, blurred at
// quarter size, with the GL version's look: tint, refraction and red/blue
// dispersion at the rim, a hairline and an optional shadow. egui's overlay on
// the canvas is laid over the glass, lightly blurred, after the tint, where no
// lower card covers it (Ash, 10-15).

struct U {
    panel: vec4f, // card centre x, y, half w, h (framebuffer pixels)
    tint: vec4f,  // straight RGBA
    p: vec4f,     // radius, rim band, refraction, dispersion (pixels)
    q: vec4f,     // shadow reach, shadow on, pixels per point, sRGB target
    map: vec4f,   // backdrop texture origin x, y (framebuffer px), its size w, h
    dir: vec4f,   // blur: direction (texels); card: framebuffer size, corner smoothing
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var ta: texture_2d<f32>;
@group(0) @binding(2) var tb: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;
@group(0) @binding(4) var tc: texture_2d<f32>; // the overlay (premultiplied), half size
@group(0) @binding(5) var td: texture_2d<f32>; // the grab: lower cards (premultiplied, clear elsewhere)
@group(0) @binding(6) var te: texture_2d<f32>; // the grab blurred

struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VOut {
    var c = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    var o: VOut;
    o.pos = vec4f(c[i], 0.0, 1.0);
    o.uv = vec2f(c[i].x * 0.5 + 0.5, 0.5 - c[i].y * 0.5);
    return o;
}

// 2:1 downsample (a linear tap between four texels averages them)
@fragment
fn fs_copy(in: VOut) -> @location(0) vec4f {
    return textureSampleLevel(ta, smp, in.uv, 0.0);
}

@fragment
fn fs_blur(in: VOut) -> @location(0) vec4f {
    var w = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    var acc = textureSampleLevel(ta, smp, in.uv, 0.0) * w[0];
    for (var i = 1; i < 5; i++) {
        let d = u.dir.xy * f32(i);
        acc += (textureSampleLevel(ta, smp, in.uv + d, 0.0) + textureSampleLevel(ta, smp, in.uv - d, 0.0)) * w[i];
    }
    return acc;
}

fn sd(p: vec2f) -> f32 {
    let b = u.panel.zw;
    // a radius past half the short side is a capsule
    let r = min(u.p.x, min(b.x, b.y));
    // corner smoothing (Figma's, roughly): the corner starts further along
    // the side, as a superellipse that eases into the straight edge; room
    // past half the short side is not there, so a capsule stays round
    let re = min(r * (1.0 + u.dir.z), min(b.x, b.y));
    let s = select(0.0, re / r - 1.0, r > 0.0);
    let k = 2.0 + 2.5 * s;
    let q = abs(p) - b + re;
    let m = max(q, vec2f(1e-6)) / max(re, 1e-3);
    let c = select(length(m), pow(pow(m.x, k) + pow(m.y, k), 1.0 / k), s > 0.001) * max(re, 1e-3);
    return c + min(max(q.x, q.y), 0.0) - re;
}

// A quarter-size blur sampled with a cubic B-spline (four bilinear taps)
// instead of plain bilinear, which shows the low resolution as blocks and
// diamonds when stretched 4x (Ash, 10-28: "too pixelly").
fn bspline(t: texture_2d<f32>, uv: vec2f) -> vec4f {
    let size = vec2f(textureDimensions(t));
    let st = uv * size - 0.5;
    let i = floor(st);
    let f = st - i;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let h0 = (i - 0.5 + w1 / g0) / size;
    let h1 = (i + 1.5 + w3 / g1) / size;
    let a = textureSampleLevel(t, smp, vec2f(h0.x, h0.y), 0.0);
    let b = textureSampleLevel(t, smp, vec2f(h1.x, h0.y), 0.0);
    let c = textureSampleLevel(t, smp, vec2f(h0.x, h1.y), 0.0);
    let d = textureSampleLevel(t, smp, vec2f(h1.x, h1.y), 0.0);
    return g0.y * (g0.x * a + g1.x * b) + g1.y * (g0.x * c + g1.x * d);
}

fn suv(p: vec2f) -> vec2f {
    return (p - u.map.xy) / u.map.zw;
}

fn to_lin(c: vec3f) -> vec3f {
    return select(pow((c + 0.055) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}

@fragment
fn fs_card(in: VOut) -> @location(0) vec4f {
    let frag = in.pos.xy;
    let ppp = u.q.z;
    let p = frag - u.panel.xy;
    let g = vec2f(sd(p + vec2f(1.0, 0.0)) - sd(p - vec2f(1.0, 0.0)), sd(p + vec2f(0.0, 1.0)) - sd(p - vec2f(0.0, 1.0)));
    // in pixels across the edge: the smoothed corner is no exact distance
    let d = sd(p) / max(length(g) * 0.5, 0.25);
    // the shadow falls a little below the card, under its antialiased edge
    var sh = 0.0;
    if u.q.y > 0.5 {
        let ds = sd(p - vec2f(0.0, 6.0 * ppp));
        let t = clamp(1.0 - ds / u.q.x, 0.0, 1.0);
        sh = 0.38 * t * t * clamp(d + 0.5, 0.0, 1.0);
    }
    if d >= 0.5 {
        return vec4f(0.0, 0.0, 0.0, sh);
    }
    let n = g / max(length(g), 1e-4);
    let edge = clamp(1.0 + d / u.p.y, 0.0, 1.0);
    let base = frag - n * edge * edge * u.p.z;
    let cap = n * edge * edge * u.p.w;
    // the canvas, with the grab's lower cards over it (premultiplied)
    let fb = u.dir.xy;
    let gr = textureSampleLevel(td, smp, (base + cap) / fb, 0.0);
    let gg = textureSampleLevel(td, smp, base / fb, 0.0);
    let gb = textureSampleLevel(td, smp, (base - cap) / fb, 0.0);
    let sharp = vec3f(
        gr.r + textureSampleLevel(ta, smp, suv(base + cap), 0.0).r * (1.0 - gr.a),
        gg.g + textureSampleLevel(ta, smp, suv(base), 0.0).g * (1.0 - gg.a),
        gb.b + textureSampleLevel(ta, smp, suv(base - cap), 0.0).b * (1.0 - gb.a),
    );
    let er = bspline(te, (base + cap) / fb);
    let eg = bspline(te, base / fb);
    let eb = bspline(te, (base - cap) / fb);
    let blur = vec3f(
        er.r + bspline(tb, suv(base + cap)).r * (1.0 - er.a),
        eg.g + bspline(tb, suv(base)).g * (1.0 - eg.a),
        eb.b + bspline(tb, suv(base - cap)).b * (1.0 - eb.a),
    );
    var col = mix(blur, sharp, edge * 0.45);
    col = mix(col, u.tint.rgb, u.tint.a * (1.0 - edge * 0.35));
    // neutral hairline so cards separate from what is behind them
    col = mix(col, vec3f(1.0), 0.07 * smoothstep(-1.6 * ppp, -0.4 * ppp, d));
    // the overlay, lightly blurred: not tinted, refracted or split, and only
    // where no lower card covers it (a lower card shows it in its own glass)
    let cover = textureSampleLevel(td, smp, frag / fb, 0.0).a;
    let ov = bspline(tc, frag / fb) * (1.0 - cover);
    col = col * (1.0 - ov.a) + ov.rgb;
    let a = clamp(0.5 - d, 0.0, 1.0);
    if u.q.w > 0.5 {
        col = to_lin(col);
    }
    return vec4f(col * a, a + sh * (1.0 - a));
}
