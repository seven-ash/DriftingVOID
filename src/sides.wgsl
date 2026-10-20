// Extrusion sides built on the GPU (src/gpu.rs, Node::Sides): one instance per
// outline edge (extrude::Edge, content coordinates), six vertices making its
// quad between the front face and the back one, as extrude::sides and
// export::fill_sides make and draw them on the CPU. A turning layer changes
// only this uniform. Faces turned away are dropped; overlaps resolve per pixel
// in a depth buffer.

struct S {
    // (content x, y, offset along the normal, 1) -> homogeneous target pixel
    // (x, y, camera depth); rows
    p0: vec4f,
    p1: vec4f,
    p2: vec4f,
    // the placement's world affine: rows of the linear part, translation in w
    m0: vec4f,
    m1: vec4f,
    m2: vec4f,
    eye: vec4f,    // camera centre (world), thickness in w
    nrm: vec4f,    // plane normal (world), smooth shading when w > 0.5
    light: vec4f,  // direction the light travels, ambient in w
    box_: vec4f,   // box min x, y (target pixels), box w, h
    k: vec4f,      // opacity, flat shade, lit when z > 0.5, near depth w
    anchor: vec4f, // the placement's anchor (content), x, y
}

@group(0) @binding(0) var<uniform> s: S;

struct In {
    @location(0) a: vec2f,
    @location(1) b: vec2f,
    @location(2) n: vec2f,
    @location(3) na: vec2f,
    @location(4) nb: vec2f,
    @location(5) ca: vec4f,
    @location(6) cb: vec4f,
}

struct Out {
    @builtin(position) pos: vec4f,
    @location(0) col: vec4f,
}

fn lin(v: vec2f) -> vec3f {
    let q = vec3f(v, 0.0);
    return vec3f(dot(s.m0.xyz, q), dot(s.m1.xyz, q), dot(s.m2.xyz, q));
}

fn world(p: vec2f, off: f32) -> vec3f {
    return lin(p - s.anchor.xy) + vec3f(s.m0.w, s.m1.w, s.m2.w) + s.nrm.xyz * off;
}

fn proj(p: vec2f, off: f32) -> vec3f {
    let q = vec4f(p, off, 1.0);
    return vec3f(dot(s.p0, q), dot(s.p1, q), dot(s.p2, q));
}

fn shade(n: vec3f) -> f32 {
    let l = max(-dot(normalize(n), s.light.xyz), 0.0);
    return s.light.w + (1.0 - s.light.w) * l;
}

fn grow(p: vec2f, q: vec2f) -> vec2f {
    let v = q - p;
    let l = length(v);
    if l > 1e-3 {
        return p - v / l * 0.35;
    }
    return p;
}

@vertex
fn vs_side(i: In, @builtin(vertex_index) vi: u32) -> Out {
    var o: Out;
    o.pos = vec4f(0.0, 0.0, 0.0, 1.0);
    o.col = vec4f(0.0);
    let th = s.eye.w;
    let wa = world(i.a, 0.0);
    let wb = world(i.b, 0.0);
    let ba = world(i.a, th);
    let bb = world(i.b, th);
    // outward normal in world: the in-plane normal made square to the edge
    let d = wb - wa;
    let wn = lin(i.n);
    let dd = dot(d, d);
    var sn = normalize(wn);
    if dd > 1e-18 {
        sn = normalize(wn - d * (dot(wn, d) / dd));
    }
    let mid = (wa + wb + ba + bb) * 0.25;
    if dot(sn, s.eye.xyz - mid) <= 0.0 {
        return o;
    }
    // corners: front a, front b, back b, back a
    var h = array<vec3f, 4>(proj(i.a, 0.0), proj(i.b, 0.0), proj(i.b, th), proj(i.a, th));
    var q: array<vec2f, 4>;
    for (var k = 0; k < 4; k++) {
        if h[k].z <= 1e-9 {
            return o;
        }
        q[k] = h[k].xy / h[k].z - s.box_.xy;
    }
    // each quad stretched a little along its edge (no seams between pieces)
    var g = array<vec2f, 4>(grow(q[0], q[1]), grow(q[1], q[0]), grow(q[2], q[3]), grow(q[3], q[2]));
    var tri = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let c = tri[vi % 6u];
    let at_b = c == 1u || c == 2u;
    var sh = s.k.y;
    if s.k.z > 0.5 {
        // smooth: each end lit by its blended normal
        if s.nrm.w > 0.5 {
            sh = shade(lin(select(i.na, i.nb, at_b)));
        } else {
            sh = shade(sn);
        }
    }
    let base = select(i.ca, i.cb, at_b).rgb;
    let c8 = round(clamp(base * sh, vec3f(0.0), vec3f(1.0)) * 255.0) / 255.0;
    o.col = vec4f(c8 * s.k.x, s.k.x);
    let p = g[c];
    let z = h[c].z;
    let ndc = vec2f(p.x / s.box_.z * 2.0 - 1.0, 1.0 - p.y / s.box_.w * 2.0);
    // depth 1 - near / z: nearer is smaller, perspective-correct
    o.pos = vec4f(ndc * z, z - s.k.w, z);
    return o;
}

@fragment
fn fs_side(i: Out) -> @location(0) vec4f {
    return i.col;
}
