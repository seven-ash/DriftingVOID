// Pen (vector) strokes filled on the GPU (src/gpu.rs, raster_strokes). A
// stroke is a chain of tapered capsules; as the CPU fills their union, each
// pixel takes the largest coverage of the stroke's capsules (vs_seg/fs_seg,
// max blend into a coverage target cleared over the stroke's box by
// fs_zero), then the stroke's colour goes over the layer once (fs_put), so
// overlapping capsules of one stroke never thicken its edges.

struct S {
    rect: vec4f,  // the stroke's box: x0, y0, x1, y1 (target pixels)
    size: vec4f,  // target w, h
    col: vec4f,   // premultiplied colour
}

@group(0) @binding(0) var<uniform> s: S;
@group(0) @binding(1) var cov: texture_2d<f32>;

fn ndc(p: vec2f) -> vec4f {
    return vec4f(p.x / s.size.x * 2.0 - 1.0, 1.0 - p.y / s.size.y * 2.0, 0.0, 1.0);
}

fn corner(vi: u32) -> vec2f {
    var c = array<vec2f, 6>(vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0), vec2f(0.0, 1.0));
    return c[vi % 6u];
}

struct SegIn {
    @location(0) a: vec4f,  // x, y, radius (target pixels)
    @location(1) b: vec4f,
}

struct SegOut {
    @builtin(position) pos: vec4f,
    @location(0) a: vec3f,
    @location(1) b: vec3f,
}

@vertex
fn vs_seg(i: SegIn, @builtin(vertex_index) vi: u32) -> SegOut {
    var o: SegOut;
    let r = max(i.a.z, i.b.z) + 1.0;
    let lo = max(min(i.a.xy, i.b.xy) - r, s.rect.xy);
    let hi = max(min(max(i.a.xy, i.b.xy) + r, s.rect.zw), lo);
    o.pos = ndc(mix(lo, hi, corner(vi)));
    o.a = i.a.xyz;
    o.b = i.b.xyz;
    return o;
}

// Distance to a capsule whose ends have different radii (Inigo Quilez's
// uneven capsule); one circle inside the other: the bigger circle.
fn capsule(p0: vec2f, a: vec2f, b0: vec2f, ra: f32, rb: f32) -> f32 {
    let p = p0 - a;
    let b = b0 - a;
    let h = dot(b, b);
    let dr = ra - rb;
    if h <= dr * dr + 1e-6 {
        return min(length(p) - ra, length(p - b) - rb);
    }
    var q = vec2f(dot(p, vec2f(b.y, -b.x)), dot(p, b)) / h;
    q.x = abs(q.x);
    let c = vec2f(sqrt(h - dr * dr), dr);
    let k = c.x * q.y - c.y * q.x;
    let m = dot(c, q);
    let n = dot(q, q);
    if k < 0.0 {
        return sqrt(h * n) - ra;
    }
    if k > c.x {
        return sqrt(h * (n + 1.0 - 2.0 * q.y)) - rb;
    }
    return m - ra;
}

@fragment
fn fs_seg(i: SegOut) -> @location(0) vec4f {
    let d = capsule(i.pos.xy, i.a.xy, i.b.xy, i.a.z, i.b.z);
    // a stroke thinner than a pixel covers about its width
    let c = clamp(0.5 - d, 0.0, min(1.0, i.a.z + i.b.z));
    return vec4f(c);
}

@vertex
fn vs_rect(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
    return ndc(mix(s.rect.xy, s.rect.zw, corner(vi)));
}

@fragment
fn fs_zero() -> @location(0) vec4f {
    return vec4f(0.0);
}

@fragment
fn fs_put(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let c = textureLoad(cov, vec2i(floor(pos.xy)), 0).r;
    if c <= 0.0 {
        discard;
    }
    return s.col * c;
}
