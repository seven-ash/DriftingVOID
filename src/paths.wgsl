// 2D layers drawn on the GPU (src/gpu_paths.rs): text glyphs and shape outlines
// as line edges, filled with the exact area coverage a CPU rasterizer
// accumulates (ab_glyph_rasterizer, font-rs): for pixel (X, Y), every edge adds
// the part of the pixel's row to its right, signed by its direction; the
// coverage is the absolute sum, at most 1 (non-zero fill).
//
// One instance per path (a glyph, a shape's fill or stroke outline): its edges
// in its own units, `m`/`t` to target pixels. Axis-aligned paths look up only
// the edges in the horizontal bands their pixel row crosses; turned ones walk
// all their edges.

struct U {
    size: vec4f, // target w, h
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> edges: array<vec4f>;
// per band: first index into `idx`, count
@group(0) @binding(2) var<storage, read> bands: array<vec2u>;
// edge index; the top bit marks the first band the edge is listed in
@group(0) @binding(3) var<storage, read> idx: array<u32>;

struct In {
    @location(0) bbox: vec4f,  // the path's bounds, its units: x0, y0, x1, y1
    @location(1) m: vec4f,     // its units -> target pixels: x' = m.x x + m.z y + t.x, y' = m.y x + m.w y + t.y
    @location(2) t: vec4f,     // t.xy
    @location(3) col: vec4f,   // premultiplied colour
    @location(4) rec: vec4u,   // band offset, band count, edge offset, edge count
}

struct V {
    @builtin(position) pos: vec4f,
    @location(0) @interpolate(flat) bbox: vec4f,
    @location(1) @interpolate(flat) m: vec4f,
    @location(2) @interpolate(flat) t: vec4f,
    @location(3) @interpolate(flat) col: vec4f,
    @location(4) @interpolate(flat) rec: vec4u,
}

fn to_px(i: V, p: vec2f) -> vec2f {
    return vec2f(i.m.x * p.x + i.m.z * p.y, i.m.y * p.x + i.m.w * p.y) + i.t.xy;
}

@vertex
fn vs_path(i: In, @builtin(vertex_index) vi: u32) -> V {
    var o: V;
    o.bbox = i.bbox;
    o.m = i.m;
    o.t = i.t;
    o.col = i.col;
    o.rec = i.rec;
    // the pixels the bounds land on, a pixel of margin
    let c = array<vec2f, 4>(i.bbox.xy, i.bbox.zy, i.bbox.zw, i.bbox.xw);
    var lo = vec2f(1e30);
    var hi = vec2f(-1e30);
    for (var k = 0; k < 4; k++) {
        let p = to_px(o, c[k]);
        lo = min(lo, p);
        hi = max(hi, p);
    }
    lo = floor(lo) - 1.0;
    hi = ceil(hi) + 1.0;
    var q = array<vec2f, 6>(vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0), vec2f(0.0, 1.0));
    let p = mix(lo, hi, q[vi % 6u]);
    o.pos = vec4f(p.x / u.size.x * 2.0 - 1.0, 1.0 - p.y / u.size.y * 2.0, 0.0, 1.0);
    return o;
}

fn ramp(t: f32) -> f32 {
    // the integral of clamp(t, 0, 1)
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return t - 0.5;
    }
    return 0.5 * t * t;
}

// The signed share of pixel (x, y) lying right of edge p -> q.
fn area(p: vec2f, q: vec2f, x: f32, y: f32) -> f32 {
    if p.y == q.y {
        return 0.0;
    }
    var a = p;
    var b = q;
    var dir = 1.0;
    if p.y > q.y {
        a = q;
        b = p;
        dir = -1.0;
    }
    let y0 = max(a.y, y);
    let y1 = min(b.y, y + 1.0);
    if y1 <= y0 {
        return 0.0;
    }
    let k = (b.x - a.x) / (b.y - a.y);
    let fa = x + 1.0 - (a.x + (y0 - a.y) * k);
    let fb = x + 1.0 - (a.x + (y1 - a.y) * k);
    var g: f32;
    if fa <= 0.0 && fb <= 0.0 {
        g = 0.0;
    } else if fa >= 1.0 && fb >= 1.0 {
        g = 1.0;
    } else if fa >= 0.0 && fa <= 1.0 && fb >= 0.0 && fb <= 1.0 {
        g = 0.5 * (fa + fb);
    } else {
        g = (ramp(fb) - ramp(fa)) / (fb - fa);
    }
    return dir * (y1 - y0) * g;
}

@fragment
fn fs_path(i: V) -> @location(0) vec4f {
    let x = floor(i.pos.x);
    let y = floor(i.pos.y);
    var acc = 0.0;
    let nb = i.rec.y;
    if i.m.y == 0.0 && i.m.z == 0.0 && nb > 0u {
        // axis-aligned: the bands the row [y, y + 1] crosses (its units)
        let ya = (y - i.t.y) / i.m.w;
        let yb = (y + 1.0 - i.t.y) / i.m.w;
        let bh = (i.bbox.w - i.bbox.y) / f32(nb);
        let b0 = u32(clamp(floor((min(ya, yb) - i.bbox.y) / bh), 0.0, f32(nb - 1u)));
        let b1 = u32(clamp(floor((max(ya, yb) - i.bbox.y) / bh), 0.0, f32(nb - 1u)));
        for (var b = b0; b <= b1; b++) {
            let r = bands[i.rec.x + b];
            for (var k = 0u; k < r.y; k++) {
                let e = idx[r.x + k];
                // an edge listed in several bands counts once: in the first
                // band of this row, or its own first band
                if b != b0 && (e & 0x80000000u) == 0u {
                    continue;
                }
                let s = edges[e & 0x7fffffffu];
                acc += area(to_px(i, s.xy), to_px(i, s.zw), x, y);
            }
        }
    } else {
        for (var k = 0u; k < i.rec.w; k++) {
            let s = edges[i.rec.z + k];
            acc += area(to_px(i, s.xy), to_px(i, s.zw), x, y);
        }
    }
    let cov = min(abs(acc), 1.0);
    if cov <= 0.0 {
        discard;
    }
    return i.col * cov;
}
