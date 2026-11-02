// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! Extrusion meshes: the outline of a layer's flattened picture (marching squares
//! on alpha, simplified with Douglas–Peucker) becomes side quads between the
//! front and back faces. Faces turned away from the eye are dropped, so a solid
//! needs one cap and the visible sides, not a stack of slices.

use egui::{Pos2, Rect, pos2, vec2};
use tiny_skia::Pixmap;

use crate::persp::{Light, Placement, V3, add, dot, mul_mv, normalize, scale, sub};

/// One outline segment in content coordinates; `n` points out of the solid and
/// `ca`/`cb` are the (unpremultiplied) colours just inside its ends. `na`/`nb` are
/// the smooth normals at its ends: blended with the neighbouring segment across a
/// gentle bend, its own `n` at a sharp corner.
#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub a: Pos2,
    pub b: Pos2,
    pub n: [f32; 2],
    pub na: [f32; 2],
    pub nb: [f32; 2],
    pub ca: [u8; 3],
    pub cb: [u8; 3],
}

/// Bends sharper than this (degrees) stay hard edges under smooth shading.
const SMOOTH_MAX_DEG: f32 = 60.0;
/// How far (texels) a blended vertex normal reaches along a segment.
const SMOOTH_REACH: f32 = 4.0;

/// Longest edge piece, in texels (side colours follow the picture this closely).
const PIECE: f32 = 3.0;

/// Outline of `pm` (texels of `rect` at `res` texels per unit) as edges.
pub fn trace(pm: &Pixmap, rect: Rect, res: f32) -> Vec<Edge> {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let d = pm.data();
    let alpha = |x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= w || y >= h {
            0.0
        } else {
            d[((y * w + x) * 4 + 3) as usize] as f32 / 255.0
        }
    };
    let rgb = |p: Pos2| -> Option<[u8; 3]> {
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        if x < 0 || y < 0 || x >= w || y >= h {
            return None;
        }
        let i = ((y * w + x) * 4) as usize;
        let a = d[i + 3] as f32;
        (a >= 24.0).then(|| [0, 1, 2].map(|k| ((d[i + k] as f32 * 255.0 / a).round().min(255.0)) as u8))
    };
    // texel (x, y) has its value at (x + 0.5, y + 0.5); cells span texel centres
    let loops = march(w, h, &alpha);
    let mut out = Vec::new();
    for lp in loops {
        let mut lp = simplify_closed(&lp, 0.3);
        lp.dedup_by(|a, b| a.distance(*b) < 1e-4);
        let n = lp.len();
        if n < 3 {
            continue;
        }
        // segment normals (out of the solid; see `march` for the orientation), and
        // vertex normals shared by gentle bends
        let seg_n: Vec<egui::Vec2> = (0..n)
            .map(|i| {
                let d = (lp[(i + 1) % n] - lp[i]).normalized();
                vec2(d.y, -d.x)
            })
            .collect();
        let cos_max = SMOOTH_MAX_DEG.to_radians().cos();
        // the normal at vertex i as seen from segment `own`
        let vert_n = |i: usize, own: usize| {
            let (p, q) = (seg_n[(i + n - 1) % n], seg_n[i]);
            if p.dot(q) >= cos_max { (p + q).normalized() } else { seg_n[own] }
        };
        for i in 0..n {
            let (a, b) = (lp[i], lp[(i + 1) % n]);
            let len = a.distance(b);
            if len < 1e-4 {
                continue;
            }
            let nn = seg_n[i];
            let (n0, n1) = (vert_n(i, i), vert_n((i + 1) % n, i));
            let pieces = (len / PIECE).ceil().max(1.0) as usize;
            let colour = |p: Pos2| -> [u8; 3] {
                for k in [1.2f32, 2.5, 4.0] {
                    if let Some(c) = rgb(p - nn * k) {
                        return c;
                    }
                }
                [128; 3]
            };
            let mut prev = a;
            let mut cprev = colour(a);
            // a vertex's blended normal fades back to this segment's own within
            // SMOOTH_REACH texels (or half the segment), so a long flat side next
            // to a gentle bend stays flat; short curve segments blend throughout
            let reach = (len * 0.5).min(SMOOTH_REACH);
            let lerp_n = |t: f32| {
                let (da, db) = (t * len, (1.0 - t) * len);
                let v = if da < reach {
                    n0 + (nn - n0) * (da / reach)
                } else if db < reach {
                    n1 + (nn - n1) * (db / reach)
                } else {
                    nn
                };
                let v = v.normalized();
                [v.x, v.y]
            };
            for j in 1..=pieces {
                let (t0, t1) = ((j - 1) as f32 / pieces as f32, j as f32 / pieces as f32);
                let p = a + (b - a) * t1;
                let cp = colour(p);
                let to = |q: Pos2| rect.min + (q.to_vec2() - vec2(0.5, 0.5)) / res + vec2(0.5, 0.5) / res;
                out.push(Edge { a: to(prev), b: to(p), n: [nn.x, nn.y], na: lerp_n(t0), nb: lerp_n(t1), ca: cprev, cb: cp });
                prev = p;
                cprev = cp;
            }
        }
    }
    out
}

/// Closed iso-0.5 loops of `alpha` (zero outside), in texel coordinates. Each
/// segment runs from where a clockwise walk round its cell leaves the solid to
/// where it re-enters, so (x, y) -> (y, -x) of its direction points outward.
fn march(w: i32, h: i32, alpha: &impl Fn(i32, i32) -> f32) -> Vec<Vec<Pos2>> {
    use std::collections::HashMap;
    const T: f32 = 0.5;
    // edge keys: (x, y, 0) horizontal from texel (x,y) to (x+1,y); (x, y, 1) vertical to (x,y+1)
    type Key = (i32, i32, u8);
    let cross = |k: Key| -> Pos2 {
        let (x, y, o) = k;
        let (x1, y1) = if o == 0 { (x + 1, y) } else { (x, y + 1) };
        let (a0, a1) = (alpha(x, y), alpha(x1, y1));
        let t = ((T - a0) / (a1 - a0)).clamp(0.0, 1.0);
        let c = |v: i32| v as f32 + 0.5;
        pos2(c(x) + (c(x1) - c(x)) * t, c(y) + (c(y1) - c(y)) * t)
    };
    let mut next: HashMap<Key, Key> = HashMap::new();
    // loops start at their first segment in scan order (a HashMap's order changes
    // from run to run, and the start point shapes the simplified outline)
    let mut order: Vec<Key> = Vec::new();
    for y in -1..h {
        for x in -1..w {
            let v = [alpha(x, y), alpha(x + 1, y), alpha(x + 1, y + 1), alpha(x, y + 1)];
            let ins = v.map(|a| a >= T);
            let case = ins.iter().enumerate().fold(0u8, |m, (i, &b)| m | ((b as u8) << i));
            if case == 0 || case == 15 {
                continue;
            }
            // cell sides in order: top, right, bottom, left (corner i -> i+1)
            let side = [(x, y, 0u8), (x + 1, y, 1u8), (x, y + 1, 0u8), (x, y, 1u8)];
            // walking corners clockwise (screen, y down): a side is crossed where
            // insideness flips; from an inside corner to an outside one we leave
            // the solid. A leave pairs with the enter before it, cutting off the
            // inside corners between them.
            let mut enters = Vec::new();
            let mut leaves = Vec::new();
            for i in 0..4 {
                let (p, q) = (ins[i], ins[(i + 1) % 4]);
                if p != q {
                    if p { leaves.push(i) } else { enters.push(i) }
                }
            }
            let pairs: Vec<(usize, usize)> = if enters.len() == 1 {
                vec![(leaves[0], enters[0])]
            } else {
                // saddle: centre value decides whether the inside corners connect
                let centre = v.iter().sum::<f32>() / 4.0 >= T;
                let (l0, l1) = (leaves[0], leaves[1]);
                let (e0, e1) = (enters[0], enters[1]);
                // centre outside: each leave with the enter before it (two inside
                // corners cut off); inside: with the enter after it (the inside
                // corners joined through the centre)
                let after = |l: usize, e: usize| (e + 4 - l) % 4;
                let (first, second) = if after(l0, e0) < after(l0, e1) { (e0, e1) } else { (e1, e0) };
                if centre { vec![(l0, first), (l1, second)] } else { vec![(l0, second), (l1, first)] }
            };
            for (l, e) in pairs {
                next.insert(side[l], side[e]);
                order.push(side[l]);
            }
        }
    }
    let mut loops = Vec::new();
    for start in order {
        if !next.contains_key(&start) {
            continue;
        }
        let mut lp = Vec::new();
        let mut k = start;
        loop {
            lp.push(cross(k));
            let Some(n) = next.remove(&k) else { break };
            k = n;
            if k == start {
                break;
            }
        }
        if lp.len() >= 3 {
            loops.push(lp);
        }
    }
    loops
}

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_sq();
    if l2 < 1e-12 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn rdp(pts: &[Pos2], eps: f32, out: &mut Vec<Pos2>) {
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (mut idx, mut dmax) = (0, 0.0);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = seg_dist(*p, a, b);
        if d > dmax {
            dmax = d;
            idx = i;
        }
    }
    if dmax > eps {
        rdp(&pts[..=idx], eps, out);
        rdp(&pts[idx..], eps, out);
    } else {
        out.push(a);
    }
}

/// Douglas–Peucker on a closed loop (split at the point farthest from the first).
pub fn simplify_closed(lp: &[Pos2], eps: f32) -> Vec<Pos2> {
    if lp.len() < 4 {
        return lp.to_vec();
    }
    let far = (1..lp.len()).max_by(|&i, &j| lp[0].distance(lp[i]).total_cmp(&lp[0].distance(lp[j]))).unwrap_or(1);
    let mut out = Vec::new();
    let mut a: Vec<Pos2> = lp[..=far].to_vec();
    rdp(&a, eps, &mut out);
    a = lp[far..].to_vec();
    a.push(lp[0]);
    rdp(&a, eps, &mut out);
    out
}

/// A visible side quad: screen corners (front a, front b, back b, back a), the
/// shaded colours at its two ends, and its depth for sorting.
pub struct Side {
    pub q: [Pos2; 4],
    pub ca: [f32; 3],
    pub cb: [f32; 3],
    pub depth: f64,
}

/// Content point to world, `off` along the plane normal.
pub fn to_world(pl: &Placement, p: Pos2, off: f64) -> V3 {
    let (m, t) = pl.world_affine();
    let q = [(p.x - pl.anchor.x) as f64, (p.y - pl.anchor.y) as f64, 0.0];
    add(add(t, mul_mv(&m, q)), scale(pl.normal(), off))
}

/// Which face of an extrusion shows: (offset of the cap, its outward normal).
/// `to_eye(p)` is the direction from world point `p` towards the viewer. An
/// engraving shows its floor, facing out of the cavity.
pub fn visible_cap(pl: &Placement, to_eye: &impl Fn(V3) -> V3) -> (f64, V3) {
    let n = pl.normal();
    if pl.engrave {
        return (pl.thick, scale(n, -1.0));
    }
    let (_, t) = pl.world_affine();
    // the front (offset 0) faces against the extrusion direction
    if dot(scale(n, -1.0), to_eye(t)) >= 0.0 { (0.0, scale(n, -1.0)) } else { (pl.thick, n) }
}

/// The sides facing the viewer, far to near, shaded by `light` (or a flat side
/// shade without one). Engraved: the inner walls of the cavity, the sides
/// turned away seen from inside (their normal reversed).
pub fn sides(
    edges: &[Edge],
    pl: &Placement,
    project: &impl Fn(V3) -> Option<Pos2>,
    to_eye: &impl Fn(V3) -> V3,
    depth: &impl Fn(V3) -> f64,
    light: Option<&Light>,
    flat_shade: f32,
) -> Vec<Side> {
    let (m, _) = pl.world_affine();
    let inward = if pl.engrave { -1.0 } else { 1.0 };
    let mut out = Vec::new();
    for e in edges {
        let wa = to_world(pl, e.a, 0.0);
        let wb = to_world(pl, e.b, 0.0);
        let (ba, bb) = (to_world(pl, e.a, pl.thick), to_world(pl, e.b, pl.thick));
        // outward normal in world: the in-plane normal made perpendicular to the edge
        let d = sub(wb, wa);
        let wn = mul_mv(&m, [e.n[0] as f64, e.n[1] as f64, 0.0]);
        let dd = dot(d, d);
        let sn = if dd > 1e-18 { normalize(sub(wn, scale(d, dot(wn, d) / dd))) } else { normalize(wn) };
        let sn = scale(sn, inward);
        let mid = scale(add(add(wa, wb), add(ba, bb)), 0.25);
        if dot(sn, to_eye(mid)) <= 0.0 {
            continue;
        }
        let (Some(p0), Some(p1), Some(p2), Some(p3)) = (project(wa), project(wb), project(bb), project(ba)) else { continue };
        // smooth: each end lit by its blended normal (in the plane, so it stays
        // square to the extrusion like the face itself)
        let shade_at = |n2: [f32; 2]| match light {
            Some(l) if pl.smooth => l.shade(scale(normalize(mul_mv(&m, [n2[0] as f64, n2[1] as f64, 0.0])), inward)),
            Some(l) => l.shade(sn),
            None => flat_shade,
        };
        let (sa, sb) = (shade_at(e.na), shade_at(e.nb));
        let f = |c: [u8; 3], s: f32| c.map(|v| v as f32 / 255.0 * s);
        out.push(Side { q: [p0, p1, p2, p3], ca: f(e.ca, sa), cb: f(e.cb, sb), depth: depth(mid) });
    }
    out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(r: f32) -> Pixmap {
        let n = (r * 2.0 + 8.0) as u32;
        let mut pm = Pixmap::new(n, n).unwrap();
        let c = n as f32 / 2.0;
        for y in 0..n {
            for x in 0..n {
                let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
                let a = (r - d + 0.5).clamp(0.0, 1.0);
                let i = ((y * n + x) * 4) as usize;
                let v = (a * 255.0) as u8;
                pm.data_mut()[i..i + 4].copy_from_slice(&[v, 0, 0, v]);
            }
        }
        pm
    }

    #[test]
    fn disc_outline_is_round_and_points_out() {
        let r = 20.0;
        let pm = disc(r);
        let c = pm.width() as f32 / 2.0;
        let edges = trace(&pm, Rect::from_min_size(Pos2::ZERO, vec2(pm.width() as f32, pm.height() as f32)), 1.0);
        assert!(edges.len() > 20, "{}", edges.len());
        for e in &edges {
            let m = pos2((e.a.x + e.b.x) / 2.0, (e.a.y + e.b.y) / 2.0);
            let rr = m.distance(pos2(c, c));
            assert!((rr - r).abs() < 0.8, "radius {rr}");
            // outward
            let out = vec2(e.n[0], e.n[1]).dot(m - pos2(c, c));
            assert!(out > 0.0);
            // red inside
            assert_eq!(e.ca, [255, 0, 0]);
        }
    }

    #[test]
    fn ring_hole_points_into_the_hole() {
        let mut pm = disc(20.0);
        let n = pm.width();
        let c = n as f32 / 2.0;
        for y in 0..n {
            for x in 0..n {
                let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
                if d < 8.0 {
                    let i = ((y * n + x) * 4) as usize;
                    pm.data_mut()[i..i + 4].copy_from_slice(&[0; 4]);
                }
            }
        }
        let edges = trace(&pm, Rect::from_min_size(Pos2::ZERO, vec2(n as f32, n as f32)), 1.0);
        let inner: Vec<_> = edges.iter().filter(|e| e.a.distance(pos2(c, c)) < 12.0).collect();
        assert!(!inner.is_empty());
        for e in inner {
            let m = pos2((e.a.x + e.b.x) / 2.0, (e.a.y + e.b.y) / 2.0);
            assert!(vec2(e.n[0], e.n[1]).dot(m - pos2(c, c)) < 0.0);
        }
    }

    #[test]
    fn smooth_normals_blend_on_curves_and_keep_corners() {
        // a disc: neighbouring pieces share their end normals
        let pm = disc(20.0);
        let n = pm.width() as f32;
        let edges = trace(&pm, Rect::from_min_size(Pos2::ZERO, vec2(n, n)), 1.0);
        for w in edges.windows(2) {
            if w[0].b.distance(w[1].a) < 1e-4 {
                let (p, q) = (vec2(w[0].nb[0], w[0].nb[1]), vec2(w[1].na[0], w[1].na[1]));
                assert!(p.dot(q) > 0.999, "{p:?} {q:?}");
            }
        }
        // a square: blending stays within the one-texel chamfer marching squares
        // cuts at each corner; the flat sides keep their face normal
        let mut sq = Pixmap::new(40, 40).unwrap();
        sq.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));
        let corners = [pos2(0.0, 0.0), pos2(40.0, 0.0), pos2(40.0, 40.0), pos2(0.0, 40.0)];
        let near = |p: Pos2| corners.iter().any(|c| c.distance(p) < 1.5 + SMOOTH_REACH);
        for e in trace(&sq, Rect::from_min_size(Pos2::ZERO, vec2(40.0, 40.0)), 1.0) {
            let (f, a, b) = (vec2(e.n[0], e.n[1]), vec2(e.na[0], e.na[1]), vec2(e.nb[0], e.nb[1]));
            if !near(e.a) {
                assert!(f.dot(a) > 0.999, "{:?}", e.a);
            }
            if !near(e.b) {
                assert!(f.dot(b) > 0.999, "{:?}", e.b);
            }
        }
    }

    #[test]
    fn square_simplifies_to_few_pieces() {
        let mut pm = Pixmap::new(64, 64).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));
        let edges = trace(&pm, Rect::from_min_size(Pos2::ZERO, vec2(64.0, 64.0)), 1.0);
        // four straight sides cut into 3-texel pieces
        assert!(edges.len() <= 4 * 23, "{}", edges.len());
        let total: f32 = edges.iter().map(|e| e.a.distance(e.b)).sum();
        assert!((total - 4.0 * 64.0).abs() < 4.0, "{total}");
    }
}
