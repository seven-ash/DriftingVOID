// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! Pressure-sensitive vector strokes. A stroke is a polyline whose points carry a
//! radius; on screen it is a triangle strip, on export the exact union of
//! tapered capsules.

use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

/// A stroke recorded over time: drawing starts at timeline time `start`, and point
/// i appears `times[i]` seconds later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rec {
    pub start: f64,
    pub times: Vec<f32>,
}

impl Rec {
    pub fn end(&self) -> f64 {
        self.start + self.times.last().copied().unwrap_or(0.0) as f64
    }
}

pub struct VStroke {
    /// (x, y, radius) in layer-local space.
    pub pts: Vec<[f32; 3]>,
    pub color: Color32,
    /// Display triangles in layer-local space (3 points per triangle).
    pub tris: Vec<Pos2>,
    pub bounds: Rect,
    /// Timing when the stroke was recorded on the timeline.
    pub rec: Option<Rec>,
    /// Hash of the points and the colour, for the picture caches
    /// (`canvas::content_key`).
    pub key: u64,
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

fn close(a: &[f32; 3], b: &[f32; 3]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

impl VStroke {
    /// A recorded stroke (point times relative to `start`).
    pub fn timed(pts: Vec<[f32; 3]>, times: Vec<f32>, color: Color32, start: f64) -> Self {
        let (mut pp, mut tt): (Vec<[f32; 3]>, Vec<f32>) = (Vec::new(), Vec::new());
        for (p, t) in pts.into_iter().zip(times) {
            if pp.last().is_some_and(|l| close(&p, l)) {
                continue;
            }
            pp.push(p);
            tt.push(t);
        }
        let mut s = Self::new(pp, color);
        tt.truncate(s.pts.len());
        s.rec = Some(Rec { start, times: tt });
        s
    }

    /// Keeps a recording through a rebuild (move, scale): times follow the points.
    pub fn with_rec(mut self, rec: Option<Rec>) -> Self {
        if let Some(mut r) = rec {
            r.times.resize(self.pts.len(), r.times.last().copied().unwrap_or(0.0));
            self.rec = Some(r);
        }
        self
    }

    /// What shows at timeline time `t`: None (not started), Some(None) (all of it)
    /// or Some(Some(part)) while it is being drawn.
    pub fn at_time(&self, t: f64) -> Option<Option<VStroke>> {
        let Some(r) = &self.rec else { return Some(None) };
        let local = (t - r.start) as f32;
        if local < 0.0 {
            return None;
        }
        let n = r.times.partition_point(|x| *x <= local);
        if n >= self.pts.len() {
            return Some(None);
        }
        if n == 0 {
            return None;
        }
        Some(Some(VStroke::new(self.pts[..n].to_vec(), self.color)))
    }

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
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            color.hash(&mut h);
            for q in &pts {
                for v in q {
                    v.to_bits().hash(&mut h);
                }
            }
            h.finish()
        };
        Self { pts, color, tris, bounds, rec: None, key }
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

/// The brush's path from the pen's samples, as Photoshop's smoothing behaves
/// (Ash, 10-22):
///
/// 1. The pen's path: a quadratic B-spline through the samples (each piece
///    runs from one midpoint between samples to the next, bent by the sample
///    between). It stays within its samples' hull: no overshoot, no bend
///    against itself.
/// 2. The brush follows a point running along that path with exponential
///    smoothing over the distance travelled (`length`: 1/e of the way per
///    that much path). It only ever moves part of the way towards where it is
///    pulled, so it never overshoots: a zigzag comes out as a wave, a jitter
///    as a calm line, and it trails the pen by about `length`.
/// 3. Between samples and while the pen rests (`follow`), the brush keeps
///    closing on its path at the pace it had; after a moment's rest the path
///    is completed up to the pen and the brush settles on it. When the pen
///    moves on, the path carries on from there, so the brush never turns back.
///
/// Points come out about `step` apart. `finish` ends the stroke where the
/// brush has got to (after the path's last stretch to the pen).
pub struct Curve {
    /// Smoothing length (0: the brush is on the path).
    length: f32,
    /// The brush (x, y, pressure) and the last point given out.
    brush: [f32; 3],
    out_last: [f32; 3],
    /// The path's pending piece: from `start`, bent by `ctrl` (the latest
    /// sample), not drawn until the next sample sets where it ends.
    start: [f32; 3],
    ctrl: [f32; 3],
    /// Where the path has been fed up to.
    fed: [f32; 3],
    /// How fast the path has been growing (units a second, smoothed), the
    /// path fed since `last_t`, and that time: a resting pen is followed at
    /// the pace the brush had.
    speed: f32,
    grown: f32,
    last_t: f64,
}

impl Curve {
    /// Starting at `p` (x, y, pressure), already drawn by the caller;
    /// `length`: the smoothing length (0: none), in the points' units.
    pub fn new(p: [f32; 3], length: f32) -> Self {
        Self { length: length.max(0.0), brush: p, out_last: p, start: p, ctrl: p, fed: p, speed: 0.0, grown: 0.0, last_t: f64::NAN }
    }

    /// A new sample at time `t` (seconds): the points the brush has drawn on
    /// the way.
    pub fn push(&mut self, p: [f32; 3], step: f32, t: f64) -> Vec<[f32; 3]> {
        let c = self.ctrl;
        if (p[0] - c[0]).abs() < 1e-4 && (p[1] - c[1]).abs() < 1e-4 {
            return Vec::new();
        }
        let end = mid(c, p);
        let mut out = Vec::new();
        let before = self.fed;
        self.feed_quad(self.start, c, end, step, &mut out);
        self.grown += dist(before, self.fed);
        // samples of one frame share its time: the pace is taken per frame
        if self.last_t.is_nan() {
            self.last_t = t;
            self.grown = 0.0;
        } else if t > self.last_t + 1e-4 {
            let v = self.grown / (t - self.last_t) as f32;
            self.speed = if self.speed > 0.0 { self.speed * 0.6 + v * 0.4 } else { v };
            self.grown = 0.0;
            self.last_t = t;
        }
        self.start = end;
        self.ctrl = p;
        out
    }

    /// A frame without a pen sample (`dt` seconds): the brush keeps closing
    /// on the end of its path at the pace it had, as it would have over
    /// that much more path, so it neither stalls nor rushes and brakes. Once
    /// the pen has `rested`, the path is completed up to the pen first, and
    /// the brush settles on it. False once there is nothing left to close.
    pub fn follow(&mut self, dt: f32, rested: bool, step: f32) -> (Vec<[f32; 3]>, bool) {
        let mut out = Vec::new();
        if rested {
            self.complete(step, &mut out);
        }
        let target = self.fed;
        let gap = dist(self.brush, target);
        if gap < step * 0.3 {
            if rested {
                // there: settled on the pen
                self.brush = target;
                if dist(self.out_last, target) > 1e-3 {
                    self.out_last = target;
                    out.push(target);
                }
            }
            return (out, false);
        }
        // over a smoothing length the pace closes 1/e of the gap; never slower
        // than a length per 0.6 s (a pen that crept, then stopped)
        let f = if self.length <= 1e-4 {
            1.0
        } else {
            let v = self.speed.max(self.length / 0.6);
            1.0 - (-v * dt.max(0.0) / self.length).exp()
        };
        let b = self.brush;
        let n = ((gap * f / (step * 0.5).max(1e-3)).ceil() as usize).clamp(1, 256);
        for i in 1..=n {
            self.brush = lerp3(b, target, f * i as f32 / n as f32);
            self.emit(step, &mut out);
        }
        (out, true)
    }

    /// The end: the path's last stretch to the pen, followed as usual; the
    /// stroke ends where the brush then is.
    pub fn finish(&mut self, step: f32) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        self.complete(step, &mut out);
        if dist(self.out_last, self.brush) > 1e-3 {
            self.out_last = self.brush;
            out.push(self.brush);
        }
        out
    }

    /// The pending piece fed straight on to the latest sample, and the next
    /// piece starting there (so the path never goes back to a midpoint the
    /// brush has passed).
    fn complete(&mut self, step: f32, out: &mut Vec<[f32; 3]>) {
        if dist(self.start, self.ctrl) > 1e-4 {
            let (a, c) = (self.start, self.ctrl);
            self.feed_quad(a, mid(a, c), c, step, out);
        }
        self.start = self.ctrl;
    }

    /// Feeds the quadratic piece a–b–c to the brush in small steps.
    fn feed_quad(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], step: f32, out: &mut Vec<[f32; 3]>) {
        // (twice the control polygon over half a step: the parameter's speed
        // varies along a quadratic, up to double at one end)
        let len = dist(a, b) + dist(b, c);
        let n = ((2.0 * len / (step * 0.5).max(1e-3)).ceil() as usize).clamp(1, 8192);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let q = [0, 1, 2].map(|k| a[k] * u * u + b[k] * 2.0 * u * t + c[k] * t * t);
            let ds = dist(self.fed, q);
            self.fed = q;
            let w = if self.length <= 1e-4 { 1.0 } else { 1.0 - (-ds / self.length).exp() };
            self.brush = lerp3(self.brush, q, w);
            self.emit(step, out);
        }
    }

    fn emit(&mut self, step: f32, out: &mut Vec<[f32; 3]>) {
        if dist(self.out_last, self.brush) >= step {
            self.out_last = self.brush;
            out.push(self.brush);
        }
    }
}

fn mid(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    lerp3(a, b, 0.5)
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    vec2(a[0] - b[0], a[1] - b[1]).length()
}

/// Point radius for a given pressure.
pub fn radius_for(base: f32, pressure: f32, use_pressure: bool) -> f32 {
    if use_pressure { base * (0.1 + 0.9 * pressure.clamp(0.0, 1.0)) } else { base }.max(0.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_circle_stays_round() {
        // samples every 30° on a circle of radius 100: joined straight they
        // miss it by 3.4 at each chord's middle
        let at = |k: usize| {
            let a = k as f32 * std::f32::consts::PI / 6.0;
            [100.0 * a.cos(), 100.0 * a.sin(), k as f32 / 12.0]
        };
        let mut c = Curve::new(at(0), 0.0);
        let mut t = 0.0;
        let mut pts = vec![at(0)];
        for k in 1..=12 {
            t += 1.0 / 120.0;
            pts.extend(c.push(at(k), 1.5, t));
        }
        pts.extend(c.finish(1.5));
        let last = pts.last().unwrap();
        assert!((last[0] - 100.0).abs() < 1e-3 && last[1].abs() < 1e-2, "ends on the last sample");
        // where both neighbouring samples are known (30°–330°): the spline
        // passes through the chords' midpoints (3.4 inside, as joined
        // straight) and between them bends smoothly back towards the samples,
        // with no corners
        let inner = |p: &&[f32; 3]| {
            let a = p[1].atan2(p[0]).rem_euclid(std::f32::consts::TAU).to_degrees();
            (31.0..329.0).contains(&a)
        };
        let worst = pts.iter().filter(inner).map(|p| ((p[0] * p[0] + p[1] * p[1]).sqrt() - 100.0).abs()).fold(0.0, f32::max);
        assert!(worst < 3.5, "off the circle by {worst}");
        let head: Vec<f32> = pts.windows(2).map(|w| (w[1][1] - w[0][1]).atan2(w[1][0] - w[0][0])).collect();
        let wrap = |d: f32| (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let corner = head.windows(2).map(|h| wrap(h[1] - h[0]).abs().to_degrees()).fold(0.0f32, f32::max);
        assert!(corner < 5.0, "a corner of {corner}° (joined straight: 30°)");
        // dense: no gap much wider than the step (the curve is a little longer than its chord)
        let gaps: Vec<(usize, f32)> = pts.windows(2).enumerate().map(|(i, w)| (i, (vec2(w[1][0], w[1][1]) - vec2(w[0][0], w[0][1])).length())).filter(|g| g.1 > 2.5).collect();
        assert!(gaps.is_empty(), "gaps {gaps:?} of {}", pts.len());
        // pressure rises monotonically with the samples
        assert!(pts.windows(2).all(|w| w[1][2] >= w[0][2] - 1e-6));
        // and the curve always turns the same way (no bends against itself)
        let turns: Vec<f32> = pts.windows(3).map(|w| (w[1][0] - w[0][0]) * (w[2][1] - w[1][1]) - (w[1][1] - w[0][1]) * (w[2][0] - w[1][0])).collect();
        assert!(turns.iter().all(|&t| t >= -0.05), "turns back: {:?}", turns.iter().cloned().fold(0.0f32, f32::min));
    }

    /// The pen stops after a quick line with strong smoothing: the brush,
    /// left behind by its smoothing length, carries on to the pen frame by
    /// frame and settles under it (Ash, 10-22: it waited at a distance), at
    /// the pace it had, slowing smoothly, not rushing and braking.
    #[test]
    fn a_resting_pen_is_followed_at_its_pace() {
        let mut c = Curve::new([0.0, 0.0, 0.5], 54.0);
        let mut pts = Vec::new();
        let mut t = 0.0;
        // 15 px every 1/120 s: 1800 px/s
        let mut last = [0.0f32; 2];
        let mut moved = 0.0;
        for k in 1..=20 {
            t += 1.0 / 120.0;
            pts.extend(c.push([k as f32 * 15.0, 0.0, 0.5], 1.5, t));
            let b = c.brush;
            if k > 10 {
                moved = (b[0] - last[0]) * 120.0;
            }
            last = [b[0], b[1]];
        }
        let behind = 300.0 - c.brush[0];
        assert!(behind > 20.0, "smoothing left it {behind} behind");
        // frames at 165 Hz without samples (a moment, then a rest)
        let mut frames = 0;
        let mut first = None;
        loop {
            let x0 = c.brush[0];
            let (more, going) = c.follow(1.0 / 165.0, frames >= 16, 1.5);
            pts.extend(more);
            if first.is_none() {
                first = Some((c.brush[0] - x0) * 165.0);
            }
            frames += 1;
            if (!going && frames >= 16) || frames > 400 {
                break;
            }
        }
        let gap = 300.0 - pts.last().unwrap()[0];
        assert!(gap.abs() < 0.5 && frames < 200, "{gap} short after {frames} frames");
        // its first pace once the samples stop matches the pace it had
        let v0 = first.unwrap();
        assert!(v0 > moved * 0.5 && v0 < moved * 1.5, "paced {v0} px/s after {moved}");
        // straight on, never back
        assert!(pts.windows(2).all(|w| w[1][0] >= w[0][0] - 1e-3 && w[1][1].abs() < 1e-3));
    }

    /// A zigzag dragged in bits, the pen resting at each corner (Ash, 10-22:
    /// burrs stuck out at the corners). The brush never turns back on itself,
    /// and without the rests the zigzag comes out as a wave.
    #[test]
    fn a_zigzag_with_rests_has_no_burrs() {
        for rest in [true, false] {
            let mut c = Curve::new([0.0, 0.0, 0.5], 24.0);
            let mut t = 0.0;
            let mut pts = vec![[0.0f32, 0.0, 0.5]];
            let mut p = [0.0f32, 0.0];
            for k in 0..8 {
                // 60 px to the right-down, or down, in samples 12 px apart
                let d = if k % 2 == 0 { [1.0, 0.3] } else { [0.3, 1.0] };
                for _ in 0..5 {
                    p = [p[0] + d[0] * 12.0, p[1] + d[1] * 12.0];
                    t += 1.0 / 120.0;
                    pts.extend(c.push([p[0], p[1], 0.5], 1.5, t));
                }
                if rest {
                    for f in 0..80 {
                        t += 1.0 / 165.0;
                        pts.extend(c.follow(1.0 / 165.0, f >= 16, 1.5).0);
                    }
                }
            }
            pts.extend(c.finish(1.5));
            // no step turns back against the one before (a burr turns ~180°)
            let worst = pts
                .windows(3)
                .map(|w| {
                    let (u, v) = (vec2(w[1][0] - w[0][0], w[1][1] - w[0][1]), vec2(w[2][0] - w[1][0], w[2][1] - w[1][1]));
                    (u.dot(v) / (u.length() * v.length()).max(1e-9)).clamp(-1.0, 1.0).acos().to_degrees()
                })
                .fold(0.0f32, f32::max);
            assert!(worst < 70.0, "rest {rest}: a step turns {worst}°");
            // both coordinates only ever grow (the zigzag goes right and down)
            assert!(pts.windows(2).all(|w| w[1][0] >= w[0][0] - 1e-3 && w[1][1] >= w[0][1] - 1e-3), "rest {rest}: goes back");
        }
    }

    /// DV_CURVE_PNG=path: the zigzag (with and without rests) and a jittery
    /// spiral at three smoothings, drawn for a look.
    #[test]
    #[ignore]
    fn draw_curves() {
        let Some(path) = std::env::var_os("DV_CURVE_PNG") else { return };
        let mut pm = tiny_skia::Pixmap::new(900, 420).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(20, 20, 26, 255));
        let mut seed = 5u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for (col, length) in [0.0f32, 24.0, 54.0].into_iter().enumerate() {
            let ox = 20.0 + col as f32 * 300.0;
            for (row, rest) in [false, true].into_iter().enumerate() {
                let start = [ox, 20.0 + row as f32 * 200.0, 0.5];
                let mut c = Curve::new(start, length);
                let mut t = 0.0;
                let mut pts = vec![start];
                let mut p = [start[0], start[1]];
                for k in 0..8 {
                    let d = if k % 2 == 0 { [1.0, 0.25] } else { [0.25, 1.0] };
                    for _ in 0..3 {
                        p = [p[0] + d[0] * 9.0 + rnd() * 2.0, p[1] + d[1] * 9.0 + rnd() * 2.0];
                        t += 1.0 / 120.0;
                        pts.extend(c.push([p[0], p[1], 0.5], 1.5, t));
                    }
                    if rest {
                        for f in 0..60 {
                            t += 1.0 / 165.0;
                            pts.extend(c.follow(1.0 / 165.0, f >= 16, 1.5).0);
                        }
                    }
                }
                pts.extend(c.finish(1.5));
                let mut pb = tiny_skia::PathBuilder::new();
                pb.move_to(pts[0][0], pts[0][1]);
                for q in &pts[1..] {
                    pb.line_to(q[0], q[1]);
                }
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(40, 90, 230, 255);
                paint.anti_alias = true;
                let st = tiny_skia::Stroke { width: 5.0, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                pm.stroke_path(&pb.finish().unwrap(), &paint, &st, tiny_skia::Transform::identity(), None);
            }
        }
        pm.save_png(std::path::Path::new(&path)).unwrap();
    }

    /// Quick handwriting as a tablet reports it: samples 6–14 apart (uneven),
    /// each up to 1.5 off the true path. Smoothed, a straight line stays
    /// straight and an arc keeps bending one way.
    #[test]
    fn jittery_samples_give_a_calm_curve() {
        let mut seed = 11u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        // a line along x
        let mut x = 0.0;
        let mut c = Curve::new([0.0, 0.0, 0.5], 40.0);
        let mut t = 0.0;
        let (mut pts, mut raw) = (Vec::new(), vec![[0.0f32, 0.0]]);
        while x < 600.0 {
            x += 10.0 + rnd() * 8.0;
            let y = rnd() * 3.0;
            raw.push([x, y]);
            t += 1.0 / 120.0;
            pts.extend(c.push([x, y, 0.5], 1.5, t));
        }
        pts.extend(c.finish(1.5));
        let mid: Vec<&[f32; 3]> = pts.iter().filter(|p| p[0] > 60.0 && p[0] < 540.0).collect();
        let off = mid.iter().map(|p| p[1].abs()).fold(0.0f32, f32::max);
        assert!(off < 1.2, "a jittery line wanders {off}");
        // no kinks: away from the ends, the heading turns little from one
        // point to the next
        let head: Vec<f32> = mid.windows(2).map(|w| (w[1][1] - w[0][1]).atan2(w[1][0] - w[0][0])).collect();
        let kink = head.windows(2).map(|h| (h[1] - h[0]).abs().to_degrees()).fold(0.0f32, f32::max);
        // joined straight, the samples turn this much at a corner
        let raw_head: Vec<f32> = raw.windows(2).map(|w| (w[1][1] - w[0][1]).atan2(w[1][0] - w[0][0])).collect();
        let raw_kink = raw_head.windows(2).map(|h| (h[1] - h[0]).abs().to_degrees()).fold(0.0f32, f32::max);
        assert!(kink < raw_kink / 3.0, "kink of {kink}° (straight: {raw_kink}°)");
        assert!(pts.windows(2).all(|w| w[1][0] >= w[0][0] - 1e-3), "goes back along x");
        // a letter-sized arc (radius 80) sampled every 10–16 px, each up to 0.5
        // off it
        let mut c = Curve::new([80.0, 0.0, 0.5], 40.0);
        let mut t = 0.0;
        let mut pts = vec![[80.0f32, 0.0, 0.5]];
        let mut a = 0.0f32;
        while a < 4.0 {
            a += 0.16 + rnd() * 0.08;
            let r = 80.0 + rnd() * 1.0;
            t += 1.0 / 120.0;
            pts.extend(c.push([r * a.cos(), r * a.sin(), 0.5], 1.5, t));
        }
        pts.extend(c.finish(1.5));
        // over every 15 px (ten steps) the heading turns the arc's way
        let head: Vec<f32> = pts.windows(2).map(|w| (w[1][1] - w[0][1]).atan2(w[1][0] - w[0][0])).collect();
        let wrap = |d: f32| (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        // (and the end too)
        let body = &head[..head.len() - 30];
        let back = body.windows(11).filter(|h| wrap(h[10] - h[0]) < 0.0).count();
        let tail_worst = head[head.len() - 41..].windows(11).map(|h| wrap(h[10] - h[0]).to_degrees()).fold(f32::MAX, f32::min);
        assert!(tail_worst > -5.0, "the tail bends back {tail_worst}°");
        assert!(back == 0, "{back} of {} stretches bend against the arc", head.len());
    }
}
