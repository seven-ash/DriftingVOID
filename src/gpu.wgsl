// GPU export passes (src/gpu.rs). Every pass draws one rectangle of the target
// (`rect`, target pixels) and works on whole pixels with textureLoad, so it
// matches the CPU export's pixel-centre arithmetic. Colours are premultiplied
// 8-bit sRGB values, as in tiny-skia; only the downsample works in linear light.

struct U {
    rect: vec4f,   // x0, y0, x1, y1 (target pixels)
    tsize: vec4f,  // target w, h, opacity, shade
    h0: vec4f,     // warp: inverse homography rows (target pixel -> canvas)
    h1: vec4f,
    h2: vec4f,
    p0: vec4f,     // tex0 offset x, y (target pixels); blend mode; mask: keep outside tex1
    p1: vec4f,     // warp: source rect min x, y, texels per canvas unit; down: k
    bg: vec4f,     // premultiplied background (0..1); blend: p1.x is 1 when there is one
    rot: vec4f,    // down: cos, sin, half big w, half big h
    rot2: vec4f,   // down: offset x, y (big pixels)
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var tex0: texture_2d<f32>;
@group(0) @binding(2) var tex1: texture_2d<f32>;
// FX passes only (their own layout): the mask coverage in the write-back
@group(0) @binding(3) var tex2: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    var c = array<vec2f, 6>(vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0), vec2f(0.0, 1.0));
    let p = mix(u.rect.xy, u.rect.zw, c[i]);
    let ndc = vec2f(p.x / u.tsize.x * 2.0 - 1.0, 1.0 - p.y / u.tsize.y * 2.0);
    return vec4f(ndc, 0.0, 1.0);
}

fn load(t: texture_2d<f32>, p: vec2i) -> vec4f {
    let d = vec2i(textureDimensions(t));
    if p.x < 0 || p.y < 0 || p.x >= d.x || p.y >= d.y {
        return vec4f(0.0);
    }
    return textureLoad(t, p, 0);
}

// A picture placed pixel for pixel at `p0.xy`, put over the target at `opacity`.
@fragment
fn fs_over(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let p = vec2i(floor(pos.xy)) - vec2i(u.p0.xy);
    return load(tex0, p) * u.tsize.z;
}

// A flattened layer through a homography, sampled bilinearly (export::warp_onto).
@fragment
fn fs_warp(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let f = pos.xy;
    let w = dot(u.h2.xyz, vec3f(f, 1.0));
    if w <= 1e-12 {
        discard;
    }
    let cx = dot(u.h0.xyz, vec3f(f, 1.0)) / w;
    let cy = dot(u.h1.xyz, vec3f(f, 1.0)) / w;
    let sx = (cx - u.p1.x) * u.p1.z - 0.5;
    let sy = (cy - u.p1.y) * u.p1.z - 0.5;
    let d = vec2f(textureDimensions(tex0));
    if sx < -1.0 || sy < -1.0 || sx > d.x || sy > d.y {
        discard;
    }
    let i = vec2i(i32(floor(sx)), i32(floor(sy)));
    let t = vec2f(sx - floor(sx), sy - floor(sy));
    let a = load(tex0, i);
    let b = load(tex0, i + vec2i(1, 0));
    let c = load(tex0, i + vec2i(0, 1));
    let e = load(tex0, i + vec2i(1, 1));
    var s = mix(mix(a, b, t.x), mix(c, e, t.x), t.y) * u.tsize.z;
    s = vec4f(s.rgb * u.tsize.w, s.a);
    if s.a <= 0.0 {
        discard;
    }
    return s;
}

fn mixmode(m: i32, d: f32, s: f32) -> f32 {
    switch m {
        case 1: { return min(d + s, 1.0); }
        case 2: { return max(d - s, 0.0); }
        case 3: { return d * s; }
        case 4: { return min(d / max(s, 1.0 / 255.0), 1.0); }
        case 5: { return abs(d - s); }
        default: { return s; }
    }
}

// `tex0` (at p0.xy) blended onto the backdrop copy `tex1` (export::blend_onto);
// written in place of the target (no fixed-function blending).
@fragment
fn fs_blend(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let q = vec2i(floor(pos.xy));
    let s = load(tex0, q - vec2i(u.p0.xy));
    let d = load(tex1, q);
    if s.a <= 0.0 {
        return d;
    }
    let m = i32(u.p0.z);
    let sa = s.a * u.tsize.z;
    let da = d.a;
    if m == 6 {
        // erase: back to the background (or clear)
        let under = select(vec4f(0.0), u.bg, u.p1.x > 0.5);
        return d * (1.0 - sa) + under * sa;
    }
    var o = vec3f(0.0);
    for (var k = 0; k < 3; k++) {
        let cs = min(s[k] / s.a, 1.0);
        var cd = 0.0;
        if d.a > 0.0 {
            cd = d[k] / d.a;
        }
        let b = mixmode(m, cd, cs);
        o[k] = sa * (1.0 - da) * cs + sa * da * b + (1.0 - sa) * da * cd;
    }
    return vec4f(o, sa + da * (1.0 - sa));
}

// A group texture (`tex0`, same size as the target) cut by a mask's keep values
// (`tex1` at p0.xy; outside it p0.w), put over the target.
@fragment
fn fs_mask(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let q = vec2i(floor(pos.xy));
    let s = load(tex0, q);
    let kq = q - vec2i(u.p0.xy);
    let kd = vec2i(textureDimensions(tex1));
    var keep = u.p0.w;
    if kq.x >= 0 && kq.y >= 0 && kq.x < kd.x && kq.y < kd.y {
        keep = textureLoad(tex1, kq, 0).r;
    }
    return s * keep;
}

// An engraving's inside (tex0) seen through its opening: the opening's
// alpha (tex1, the layer drawn on its plane) cuts it.
@fragment
fn fs_cut(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let q = vec2i(floor(pos.xy));
    return load(tex0, q) * load(tex1, q).a;
}

fn to_lin(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn sample_big(p: vec2f) -> vec4f {
    // bilinear, transparent outside (the rotated case; unrotated hits centres)
    let s = p - 0.5;
    let i = vec2i(floor(s));
    let t = s - floor(s);
    let a = load(tex0, i);
    let b = load(tex0, i + vec2i(1, 0));
    let c = load(tex0, i + vec2i(0, 1));
    let e = load(tex0, i + vec2i(1, 1));
    return mix(mix(a, b, t.x), mix(c, e, t.x), t.y);
}

// Supersampled picture -> output: k×k samples averaged in linear light
// (premultiplied), as export::render_ss; rotated exports turn while sampling.
@fragment
fn fs_down(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let k = i32(u.p1.w);
    let o = vec2i(floor(pos.xy));
    var acc = vec4f(0.0);
    for (var dy = 0; dy < k; dy++) {
        for (var dx = 0; dx < k; dx++) {
            let d = vec2f(f32(o.x * k + dx) + 0.5, f32(o.y * k + dy) + 0.5);
            var c: vec4f;
            if u.rot.x == 1.0 && u.rot.y == 0.0 {
                c = load(tex0, vec2i(floor(d)));
            } else {
                let r = d - u.rot.zw;
                let src = vec2f(u.rot.x * r.x - u.rot.y * r.y, u.rot.y * r.x + u.rot.x * r.y) - u.rot2.xy;
                c = sample_big(src);
            }
            if c.a <= 0.0 {
                continue;
            }
            let un = min(c.rgb / c.a, vec3f(1.0));
            acc += vec4f(to_lin(un.r) * c.a, to_lin(un.g) * c.a, to_lin(un.b) * c.a, c.a);
        }
    }
    let n = f32(k * k);
    let a = acc.a / n;
    if a <= 0.0 {
        return vec4f(0.0);
    }
    let lin = clamp(acc.rgb / n / a, vec3f(0.0), vec3f(1.0));
    return vec4f(to_srgb(lin.r) * a, to_srgb(lin.g) * a, to_srgb(lin.b) * a, a);
}

fn mitchell(x: f32) -> f32 {
    // B = C = 1/3, as tiny-skia's bicubic filter
    let a = abs(x);
    let b = 1.0 / 3.0;
    let c = 1.0 / 3.0;
    if a < 1.0 {
        return ((12.0 - 9.0 * b - 6.0 * c) * a * a * a + (-18.0 + 12.0 * b + 6.0 * c) * a * a + (6.0 - 2.0 * b)) / 6.0;
    }
    if a < 2.0 {
        return ((-b - 6.0 * c) * a * a * a + (6.0 * b + 30.0 * c) * a * a + (-12.0 * b - 48.0 * c) * a + (8.0 * b + 24.0 * c)) / 6.0;
    }
    return 0.0;
}

fn clamped(p: vec2i, l: i32) -> vec4f {
    let d = vec2i(textureDimensions(tex0, l));
    return textureLoad(tex0, clamp(p, vec2i(0), d - 1), l);
}

fn bicubic(sp: vec2f, d: vec2f, l: i32) -> vec4f {
    let s = sp * vec2f(textureDimensions(tex0, l)) / d - 0.5;
    let i = vec2i(floor(s));
    let f = s - floor(s);
    var c = vec4f(0.0);
    for (var y = -1; y <= 2; y++) {
        let wy = mitchell(f32(y) - f.y);
        for (var x = -1; x <= 2; x++) {
            c += clamped(i + vec2i(x, y), l) * (mitchell(f32(x) - f.x) * wy);
        }
    }
    return c;
}

// A photo or video frame: target pixel -> source pixel by an affine (h0, h1),
// sampled nearest (p1.y = 1, pixel for pixel) or bicubic between the mip
// levels about p1.z (fractional), edges padded; the rectangle's edge is
// anti-aliased over p1.x target pixels per source pixel.
@fragment
fn fs_image(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let q = vec3f(pos.xy, 1.0);
    let sp = vec2f(dot(u.h0.xyz, q), dot(u.h1.xyz, q));
    let d = vec2f(textureDimensions(tex0));
    let edge = min(min(sp.x, d.x - sp.x), min(sp.y, d.y - sp.y)) * u.p1.x;
    let cov = clamp(edge + 0.5, 0.0, 1.0);
    if cov <= 0.0 {
        discard;
    }
    var c: vec4f;
    if u.p1.y > 0.5 {
        c = clamped(vec2i(floor(sp)), 0);
    } else {
        let l = i32(floor(u.p1.z));
        let t = u.p1.z - floor(u.p1.z);
        c = bicubic(sp, d, l);
        if t > 0.0 {
            c = mix(c, bicubic(sp, d, l + 1), t);
        }
        let a = clamp(c.a, 0.0, 1.0);
        c = vec4f(clamp(c.rgb, vec3f(0.0), vec3f(a)), a);
    }
    return c * (cov * u.tsize.z);
}

// A photo or video frame in a 3D space: target pixel -> source pixel through
// the inverse homography (h0..h2), bicubic between the mip levels its
// footprint asks for, per pixel (as fs_image: about 1.5 texels per target
// pixel; the larger axis, so steep views don't alias); edges anti-aliased.
// Opacity and shade in tsize.z/w, the top mip level in p1.w.
@fragment
fn fs_warp_image(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let q = vec3f(pos.xy, 1.0);
    let w = dot(u.h2.xyz, q);
    if w <= 1e-12 {
        discard;
    }
    let sp = vec2f(dot(u.h0.xyz, q), dot(u.h1.xyz, q)) / w;
    // source pixels per target pixel, along x and along y
    let jx = (vec2f(u.h0.x, u.h1.x) - sp * u.h2.x) / w;
    let jy = (vec2f(u.h0.y, u.h1.y) - sp * u.h2.y) / w;
    let rho = max(max(length(jx), length(jy)), 1e-6);
    let d = vec2f(textureDimensions(tex0));
    let edge = min(min(sp.x, d.x - sp.x), min(sp.y, d.y - sp.y)) / rho;
    let cov = clamp(edge + 0.5, 0.0, 1.0);
    if cov <= 0.0 {
        discard;
    }
    var lv = 0.0;
    if rho > 2.0 {
        lv = clamp(log2(rho / 1.5), 0.0, u.p1.w);
    }
    let l = i32(floor(lv));
    let t = lv - floor(lv);
    var c = bicubic(sp, d, l);
    if t > 0.0 {
        c = mix(c, bicubic(sp, d, l + 1), t);
    }
    let a = clamp(c.a, 0.0, 1.0);
    c = vec4f(clamp(c.rgb, vec3f(0.0), vec3f(a)), a) * (cov * u.tsize.z);
    return vec4f(c.rgb * u.tsize.w, c.a);
}

// Video output as NV12 (BT.709, limited range), from the finished picture in
// tex0 (premultiplied, opaque for these codecs): fs_y writes the luma plane,
// fs_uv the half-size interleaved chroma plane (2x2 average).
fn luma(c: vec3f) -> f32 {
    return (16.0 + 219.0 * dot(c, vec3f(0.2126, 0.7152, 0.0722))) / 255.0;
}

@fragment
fn fs_y(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let c = textureLoad(tex0, vec2i(floor(pos.xy)), 0);
    return vec4f(luma(c.rgb), 0.0, 0.0, 1.0);
}

@fragment
fn fs_uv(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let p = vec2i(floor(pos.xy)) * 2;
    let c = (textureLoad(tex0, p, 0) + textureLoad(tex0, p + vec2i(1, 0), 0) + textureLoad(tex0, p + vec2i(0, 1), 0) + textureLoad(tex0, p + vec2i(1, 1), 0)).rgb * 0.25;
    let cb = (128.0 + 224.0 * dot(c, vec3f(-0.114572, -0.385428, 0.5))) / 255.0;
    let cr = (128.0 + 224.0 * dot(c, vec3f(0.5, -0.454153, -0.045847))) / 255.0;
    return vec4f(cb, cr, 0.0, 1.0);
}

// A decoded NV12 frame (tex0: R8, luma rows then chroma rows from p1.x) to
// opaque RGB, BT.709 limited range, as doc::Nv12::to_rgba.
@fragment
fn fs_from_nv12(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let p = vec2i(floor(pos.xy));
    let h = i32(u.p1.x);
    let yy = (textureLoad(tex0, p, 0).r * 255.0 - 16.0) / 219.0;
    let c = vec2i((p.x / 2) * 2, h + p.y / 2);
    let cb = (textureLoad(tex0, c, 0).r * 255.0 - 128.0) / 224.0;
    let cr = (textureLoad(tex0, c + vec2i(1, 0), 0).r * 255.0 - 128.0) / 224.0;
    let rgb = vec3f(yy + 1.5748 * cr, yy - 0.187324 * cb - 0.468124 * cr, yy + 1.8556 * cb);
    return vec4f(clamp(rgb, vec3f(0.0), vec3f(1.0)), 1.0);
}

// The canvas ground: void colour (h0), two blooms (h1/p0 warm, h2/p1 cool: colour;
// centre x, y and radius), and the dot grid (bg: colour; rot: step, offset x, y,
// radius), as theme::draw_ground paints it.
@fragment
fn fs_ground(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let p = pos.xy;
    var c = u.h0;
    for (var i = 0; i < 2; i++) {
        var col = u.h1;
        var at = u.p0;
        if i == 1 {
            col = u.h2;
            at = u.p1;
        }
        let a = max(0.0, 1.0 - distance(p, at.xy) / max(at.z, 1.0));
        let s = col * a;
        c = s + c * (1.0 - s.a);
    }
    let step = u.rot.x;
    let q = (p + u.rot.yz) - floor((p + u.rot.yz) / step + 0.5) * step;
    let cov = clamp(u.rot.w + 0.5 - length(q), 0.0, 1.0);
    let d = u.bg * cov;
    c = d + c * (1.0 - d.a);
    return vec4f(c.rgb, 1.0);
}

// A group texture (tex0) cut by a mask layer's tiles (tex1, its own pixels):
// h0/h1 map a target pixel centre to a mask pixel; the mask's alpha there (0
// outside) hides it, inverted when p1.x, times the strength p1.y.
@fragment
fn fs_mask_atlas(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let s = load(tex0, vec2i(floor(pos.xy)));
    let q = vec3f(pos.xy, 1.0);
    let m = vec2f(dot(u.h0.xyz, q), dot(u.h1.xyz, q));
    let a = load(tex1, vec2i(floor(m))).a;
    var hide = a;
    if u.p1.x > 0.5 {
        hide = 1.0 - a;
    }
    return s * (1.0 - hide * u.p1.y);
}

fn cbcr(c: vec3f) -> vec2f {
    return vec2f(dot(c, vec3f(-0.168736, -0.331264, 0.5)), dot(c, vec3f(0.5, -0.418688, -0.081312)));
}

// A photo or video frame made ready to sample, as export::image_pixmap: the
// crop (tex0 from p0.xy), the chroma key (h0: key colour, on when h0.w; h1:
// tolerance, soft edge), brightness, contrast, saturation (p1, on when p1.w),
// premultiplied. tex0 is straight alpha; values are rounded to 8 bits between
// steps as the CPU's bytes are.
@fragment
fn fs_prep(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    var c = textureLoad(tex0, vec2i(floor(pos.xy)) + vec2i(u.p0.xy), 0);
    if u.h0.w > 0.5 && c.a > 0.0 {
        let kc = cbcr(u.h0.rgb);
        let kl = max(length(kc), 1e-4);
        let lo = u.h1.x;
        let hi = u.h1.x + max(u.h1.y, 1e-4);
        let pc = cbcr(c.rgb);
        let d = distance(pc, kc);
        if d < hi {
            let x = clamp((d - lo) / (hi - lo), 0.0, 1.0);
            let a = x * x * (3.0 - 2.0 * x);
            c.a = round(c.a * 255.0 * a) / 255.0;
            // spill: the part of the chroma that points at the key goes
            let along = dot(pc, kc) / kl;
            if along > 0.0 {
                let n = pc - kc * (along * (1.0 - a) / kl);
                let y = dot(c.rgb, vec3f(0.299, 0.587, 0.114));
                let rgb = vec3f(y + 1.402 * n.y, y - 0.344136 * n.x - 0.714136 * n.y, y + 1.772 * n.x);
                c = vec4f(round(clamp(rgb, vec3f(0.0), vec3f(1.0)) * 255.0) / 255.0, c.a);
            }
        }
    }
    if u.p1.w > 0.5 {
        let v = (c.rgb - 0.5) * (1.0 + u.p1.y) + 0.5 + u.p1.x * 0.5;
        let l = dot(v, vec3f(0.2126, 0.7152, 0.0722));
        c = vec4f(floor(clamp(l + (v - l) * (1.0 + u.p1.z), vec3f(0.0), vec3f(1.0)) * 255.0 + 0.5) / 255.0, c.a);
    }
    return vec4f(c.rgb * c.a, c.a);
}

fn bilin(p: vec2f) -> vec4f {
    let s = p - 0.5;
    let i = vec2i(floor(s));
    let t = s - floor(s);
    let a = clamped(i, 0);
    let b = clamped(i + vec2i(1, 0), 0);
    let c = clamped(i + vec2i(0, 1), 0);
    let e = clamped(i + vec2i(1, 1), 0);
    return mix(mix(a, b, t.x), mix(c, e, t.x), t.y);
}

// One mip level from the one above (tex0, a single level): the 2x2 box (four
// bilinear taps, so odd sizes stay in place), premultiplied.
@fragment
fn fs_mip(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let r = vec2f(textureDimensions(tex0)) / u.tsize.xy;
    let c = pos.xy * r;
    let o = r * 0.25;
    return (bilin(c + vec2f(-o.x, -o.y)) + bilin(c + vec2f(o.x, -o.y)) + bilin(c + vec2f(-o.x, o.y)) + bilin(c + o)) * 0.25;
}

// ---------------------------------------------------------------- FX layers
// (docs/updates/fx-layers.md) p0.x is the pass:
//  1 Gaussian blur along h0.xy (one axis), sigma h0.z, taps h0.w apart, p0.y each side
//  2 brightness h0.x / contrast h0.y (GIMP's formulas)
// 10 write-back: tex0 filtered, tex1 the original, tex2 the mask coverage
//    (p1.y on); p0.z the blend (-1: 복사), opacity tsize.z

fn load_edge(t: texture_2d<f32>, p: vec2i) -> vec4f {
    let d = vec2i(textureDimensions(t)) - vec2i(1);
    return textureLoad(t, clamp(p, vec2i(0), d), 0);
}

fn straight(c: vec4f) -> vec4f {
    if c.a <= 0.0 {
        return vec4f(0.0);
    }
    return vec4f(min(c.rgb / c.a, vec3f(1.0)), c.a);
}

@fragment
fn fs_fx(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let q = vec2i(floor(pos.xy));
    let op = i32(u.p0.x);
    if op == 1 {
        let dir = vec2f(u.h0.xy);
        let sg = u.h0.z;
        let n = i32(u.p0.y);
        var acc = vec4f(0.0);
        var ws = 0.0;
        for (var k = -n; k <= n; k++) {
            let x = f32(k) * u.h0.w;
            let w = exp(-x * x / (2.0 * sg * sg));
            // past the picture's edge, its edge (the view does not hold more)
            acc += w * load_edge(tex0, q + vec2i(round(dir * x)));
            ws += w;
        }
        return acc / ws;
    }
    if op == 2 {
        let c = straight(textureLoad(tex0, q, 0));
        let b = u.h0.x;
        var v = c.rgb;
        if b < 0.0 {
            v = v * (1.0 + b);
        } else {
            v = v + (1.0 - v) * b;
        }
        let k = tan((clamp(u.h0.y, -0.99, 0.99) + 1.0) * 0.7853982);
        v = clamp((v - 0.5) * k + 0.5, vec3f(0.0), vec3f(1.0));
        return vec4f(v * c.a, c.a);
    }
    // write-back
    let f = textureLoad(tex0, q, 0);
    let d = textureLoad(tex1, q, 0);
    var cov = 1.0;
    if u.p1.y > 0.5 {
        cov = textureLoad(tex2, q, 0).a;
    }
    let m = i32(u.p0.z);
    var mixed = f;
    if m >= 0 && f.a > 0.0 {
        // the filtered picture over the original with that mode (as fs_blend)
        let sa = f.a;
        let da = d.a;
        var o = vec3f(0.0);
        for (var k = 0; k < 3; k++) {
            let cs = min(f[k] / sa, 1.0);
            var cd = 0.0;
            if da > 0.0 {
                cd = d[k] / da;
            }
            o[k] = sa * (1.0 - da) * cs + sa * da * mixmode(m, cd, cs) + (1.0 - sa) * da * cd;
        }
        mixed = vec4f(o, sa + da * (1.0 - sa));
    } else if m >= 0 {
        mixed = d;
    }
    return mix(d, mixed, clamp(u.tsize.z * cov, 0.0, 1.0));
}
