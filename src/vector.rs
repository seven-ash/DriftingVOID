//! Pressure-sensitive vector strokes. A stroke is a polyline whose points carry a
//! radius; on screen it is a triangle strip, on export the exact union of
//! tapered capsules.

use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};

pub struct VStroke {
    /// (x, y, radius) in layer-local space.
    pub pts: Vec<[f32; 3]>,
    pub color: Color32,
    /// Display triangles in layer-local space (3 points per triangle).
    pub tris: Vec<Pos2>,
    pub bounds: Rect,
}

fn p(a: &[f32; 3]) -> Pos2 {
    pos2(a[0], a[1])
}

fn circle_fan(out: &mut Vec<Pos2>, c: Pos2, r: f32) {
    let n = ((r * 0.6) as usize + 10).clamp(10, 40);
    let mut prev = c + vec2(r, 0.0);
    for i in 1..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let q = c + vec2(a.cos(), a.sin()) * r;
        out.extend_from_slice(&[c, prev, q]);
        prev = q;
    }
}

impl VStroke {
    pub fn new(mut pts: Vec<[f32; 3]>, color: Color32) -> Self {
        pts.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3);
        let mut tris = Vec::new();
        let mut bounds = Rect::NOTHING;
        for q in &pts {
            bounds = bounds.union(Rect::from_center_size(p(q), Vec2::splat(q[2] * 2.0)));
        }
        let n = pts.len();
        if n == 1 {
            circle_fan(&mut tris, p(&pts[0]), pts[0][2]);
        } else if n > 1 {
            let dir = |i: usize, j: usize| (p(&pts[j]) - p(&pts[i])).normalized();
            let mut normals = Vec::with_capacity(n);
            for i in 0..n {
                let t = dir(i.saturating_sub(1), (i + 1).min(n - 1));
                normals.push(vec2(-t.y, t.x));
            }
            for i in 0..n - 1 {
                let (a, b) = (p(&pts[i]), p(&pts[i + 1]));
                let (la, ra) = (a + normals[i] * pts[i][2], a - normals[i] * pts[i][2]);
                let (lb, rb) = (b + normals[i + 1] * pts[i + 1][2], b - normals[i + 1] * pts[i + 1][2]);
                tris.extend_from_slice(&[la, ra, lb, ra, rb, lb]);
            }
            circle_fan(&mut tris, p(&pts[0]), pts[0][2]);
            circle_fan(&mut tris, p(&pts[n - 1]), pts[n - 1][2]);
            for i in 1..n - 1 {
                if dir(i - 1, i).dot(dir(i, i + 1)) < 0.82 {
                    circle_fan(&mut tris, p(&pts[i]), pts[i][2]);
                }
            }
        }
        Self { pts, color, tris, bounds }
    }

    pub fn hit(&self, q: Pos2, tol: f32) -> bool {
        if !self.bounds.expand(tol).contains(q) {
            return false;
        }
        if self.pts.len() == 1 {
            return p(&self.pts[0]).distance(q) <= self.pts[0][2] + tol;
        }
        self.pts.windows(2).any(|w| {
            let (a, b) = (p(&w[0]), p(&w[1]));
            let ab = b - a;
            let t = ((q - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
            let r = w[0][2] + (w[1][2] - w[0][2]) * t;
            (a + ab * t).distance(q) <= r + tol
        })
    }

    /// Exact outline as a set of convex polygons (all with the same winding),
    /// suitable for a non-zero fill.
    pub fn capsules(&self, seg_scale: f32) -> Vec<Vec<Pos2>> {
        let mut out = Vec::new();
        if self.pts.len() == 1 {
            out.push(arc(p(&self.pts[0]), self.pts[0][2], 0.0, std::f32::consts::TAU, seg_scale));
            return out;
        }
        for w in self.pts.windows(2) {
            out.push(capsule(p(&w[0]), w[0][2], p(&w[1]), w[1][2], seg_scale));
        }
        out
    }
}

fn arc(c: Pos2, r: f32, a0: f32, a1: f32, seg_scale: f32) -> Vec<Pos2> {
    let span = a1 - a0;
    let n = ((r * seg_scale * span).sqrt() * 2.0).clamp(3.0, 64.0) as usize;
    (0..=n)
        .map(|i| {
            let a = a0 + span * i as f32 / n as f32;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

fn capsule(p0: Pos2, r0: f32, p1: Pos2, r1: f32, seg_scale: f32) -> Vec<Pos2> {
    use std::f32::consts::{PI, TAU};
    let d = p1 - p0;
    let len = d.length();
    if len < 1e-4 || len <= (r0 - r1).abs() {
        let (c, r) = if r0 >= r1 { (p0, r0) } else { (p1, r1) };
        return arc(c, r, 0.0, TAU, seg_scale);
    }
    let phi = d.y.atan2(d.x);
    let alpha = PI / 2.0 - ((r0 - r1) / len).clamp(-1.0, 1.0).asin();
    let mut pts = arc(p1, r1, phi - alpha, phi + alpha, seg_scale);
    pts.extend(arc(p0, r0, phi + alpha, phi + TAU - alpha, seg_scale));
    pts
}

/// Point radius for a given pressure.
pub fn radius_for(base: f32, pressure: f32, use_pressure: bool) -> f32 {
    if use_pressure { base * (0.1 + 0.9 * pressure.clamp(0.0, 1.0)) } else { base }.max(0.3)
}
