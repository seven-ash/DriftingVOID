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
        Self { pts, color, tris, bounds, rec: None }
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

/// The pointer's path, smoothed and joined by curves as Krita's freehand
/// helper does (`kis_tool_freehand_helper.cpp`, Ash 10-22):
///
/// 1. Weighted smoothing: each sample becomes the Gaussian-weighted mean of
///    the (already smoothed) samples before it, weighted by arc length back
///    along the stroke (σ = a third of the smoothing distance), pressure
///    too. Jitter is averaged away before anything is curved.
/// 2. Each piece between two smoothed points is a cubic Bézier whose
///    tangents are the central differences of its neighbours; the control
///    points stay within the chord (where the tangent rays meet, or half the
///    chord), and the slower end is pulled less (Krita's velocity
///    similarity). So a curve never overshoots or bends against itself.
///
/// A piece is made once the sample after it has arrived, so the stroke
/// trails the pen by one sample; `finish` draws on to where the pen left.
pub struct Curve {
    /// Raw samples and the arc length from each to the next one.
    raw: Vec<([f32; 3], f32)>,
    sigma: f32,
    /// The last three smoothed points (older, previous, current).
    q: Vec<[f32; 3]>,
    /// Tangent at the oldest of `q` (None until the first piece).
    t_older: Option<Vec2>,
    last_raw: [f32; 3],
    /// The pen's own samples (unsmoothed), for the tail.
    trail: Vec<[f32; 3]>,
}

impl Curve {
    /// Starting at `p` (x, y, pressure), already drawn by the caller;
    /// `distance`: the smoothing distance (0: none), in the points' units.
    pub fn new(p: [f32; 3], distance: f32) -> Self {
        Self { raw: vec![(p, 0.0)], sigma: distance / 3.0, q: vec![p], t_older: None, last_raw: p, trail: vec![p] }
    }

    fn smoothed(&self) -> [f32; 3] {
        let n = self.raw.len();
        if self.sigma <= 1e-3 || n < 3 {
            return self.raw[n - 1].0;
        }
        let (mut acc, mut sum, mut d) = ([0.0f32; 3], 0.0f32, 0.0f32);
        for i in (0..n).rev() {
            let w = (-(d * d) / (2.0 * self.sigma * self.sigma)).exp();
            if w < 0.01 {
                break;
            }
            for k in 0..3 {
                acc[k] += self.raw[i].0[k] * w;
            }
            sum += w;
            if i > 0 {
                d += self.raw[i - 1].1;
            }
        }
        acc.map(|v| v / sum)
    }

    /// A new sample: the points of the curve up to the smoothed point before
    /// it, about `step` apart.
    pub fn push(&mut self, p: [f32; 3], step: f32) -> Vec<[f32; 3]> {
        let l = self.last_raw;
        let dist = vec2(p[0] - l[0], p[1] - l[1]).length();
        if dist < 1e-4 {
            return Vec::new();
        }
        self.last_raw = p;
        self.trail.push(p);
        if let Some(last) = self.raw.last_mut() {
            last.1 = dist;
        }
        self.raw.push((p, 0.0));
        let s = self.smoothed();
        // as Krita: the smoothed point stands in for the sample in the history,
        // so later means build on it (stronger, steadier smoothing)
        if let Some(last) = self.raw.last_mut() {
            last.0 = s;
        }
        self.advance(s, step)
    }

    /// Moves the curve on to `s`, drawing the piece before it.
    fn advance(&mut self, s: [f32; 3], step: f32) -> Vec<[f32; 3]> {
        let prev = *self.q.last().unwrap();
        if vec2(s[0] - prev[0], s[1] - prev[1]).length() < 1e-4 {
            return Vec::new();
        }
        self.q.push(s);
        if self.q.len() > 3 {
            self.q.remove(0);
        }
        if self.q.len() < 3 {
            return Vec::new();
        }
        let xy = |p: [f32; 3]| vec2(p[0], p[1]);
        let (a, b, c) = (self.q[0], self.q[1], self.q[2]);
        let t_a = self.t_older.unwrap_or(xy(b) - xy(a));
        let t_b = (xy(c) - xy(a)) * 0.5;
        self.t_older = Some(t_b);
        bezier_piece(a, b, t_a, t_b, step)
    }

    /// The pen rests (no sample this frame): the smoothed point moves `frac`
    /// of the way on to it, as Photoshop's stroke catch-up does, so the brush
    /// keeps following and settles under the pen instead of waiting at its
    /// smoothing distance. False once it is there.
    pub fn catch_up(&mut self, frac: f32, step: f32) -> (Vec<[f32; 3]>, bool) {
        let s = self.raw.last().map_or(self.last_raw, |r| r.0);
        let p = self.last_raw;
        let gap = vec2(p[0] - s[0], p[1] - s[1]).length();
        if gap < step * 0.3 {
            return (Vec::new(), false);
        }
        let f = frac.clamp(0.0, 1.0);
        let n = [s[0] + (p[0] - s[0]) * f, s[1] + (p[1] - s[1]) * f, s[2] + (p[2] - s[2]) * f];
        if let Some(last) = self.raw.last_mut() {
            last.1 = gap * f;
        }
        self.raw.push((n, 0.0));
        (self.advance(n, step), true)
    }

    /// The rest: from the smoothed point (which trails the pen) on along the
    /// pen's own path to where it left, the trailing offset fading out on
    /// the way, so the tail bends in with the stroke instead of cutting
    /// across to the end; then the last piece.
    pub fn finish(&mut self, step: f32) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        let s = *self.q.last().unwrap();
        let tr = &self.trail;
        if tr.len() >= 2 {
            // the nearest point to `s` on the pen's recent path
            let xy = |p: [f32; 3]| vec2(p[0], p[1]);
            let (mut best, mut at, mut near) = (f32::MAX, tr.len() - 1, s);
            for i in tr.len().saturating_sub(40)..tr.len() - 1 {
                let (a, b) = (xy(tr[i]), xy(tr[i + 1]));
                let d = b - a;
                let t = ((xy(s) - a).dot(d) / d.length_sq().max(1e-9)).clamp(0.0, 1.0);
                let c = a + d * t;
                let dist = (c - xy(s)).length();
                if dist < best {
                    best = dist;
                    at = i + 1;
                    near = [c.x, c.y, tr[i][2] + (tr[i + 1][2] - tr[i][2]) * t];
                }
            }
            let mut path = vec![near];
            path.extend_from_slice(&tr[at..]);
            let lens: Vec<f32> = std::iter::once(0.0)
                .chain(path.windows(2).scan(0.0, |acc, w| {
                    *acc += (xy(w[1]) - xy(w[0])).length();
                    Some(*acc)
                }))
                .collect();
            let total = lens.last().copied().unwrap_or(0.0).max(1e-6);
            let off = [s[0] - near[0], s[1] - near[1], s[2] - near[2]];
            let pts: Vec<[f32; 3]> = path.iter().zip(&lens).skip(1).map(|(p, l)| {
                let k = 1.0 - l / total;
                [p[0] + off[0] * k, p[1] + off[1] * k, p[2] + off[2] * k]
            }).collect();
            for p in pts {
                out.extend(self.advance(p, step));
            }
        }
        let xy = |p: [f32; 3]| vec2(p[0], p[1]);
        let n = self.q.len();
        if n >= 2 {
            let (a, b) = (self.q[n - 2], self.q[n - 1]);
            let t_a = self.t_older.filter(|_| n == 3).unwrap_or(xy(b) - xy(a));
            out.extend(bezier_piece(a, b, t_a, xy(b) - xy(a), step));
        }
        self.q.truncate(1);
        out
    }
}

/// Points from `a` (left out) to `b` (included) on a cubic Bézier leaving `a`
/// along `ta` and reaching `b` along `tb` (velocities). As Krita: the control
/// points head for where the tangent rays meet when that lies between the
/// ends, else half the chord along each tangent; the faster end's control is
/// pulled 0.8 of the way, the slower one less by their speed ratio (at least
/// half of it).
fn bezier_piece(a: [f32; 3], b: [f32; 3], ta: Vec2, tb: Vec2, step: f32) -> Vec<[f32; 3]> {
    let (pa, pb) = (vec2(a[0], a[1]), vec2(b[0], b[1]));
    let chord = (pb - pa).length();
    let n = ((chord * 1.25 / step.max(1e-3)).ceil() as usize).clamp(1, 512);
    let (va, vb) = (ta.length(), tb.length());
    let line = |i: usize| {
        let f = i as f32 / n as f32;
        let p = pa + (pb - pa) * f;
        [p.x, p.y, a[2] + (b[2] - a[2]) * f]
    };
    if va < 1e-6 || vb < 1e-6 || chord < 1e-6 {
        return (1..=n).map(line).collect();
    }
    let (da, db) = (ta / va, tb / vb);
    // where the rays a + s·da and b − u·db meet (s, u ≥ 0, within the chord)
    let cross = |u: Vec2, v: Vec2| u.x * v.y - u.y * v.x;
    let den = cross(da, -db);
    let meet = (den.abs() > 1e-6)
        .then(|| {
            let d = pb - pa;
            (cross(d, -db) / den, cross(da, d) / den)
        })
        .filter(|&(s, u)| s > 0.0 && u > 0.0 && s < chord && u < chord);
    let (ca, cb) = match meet {
        Some((s, u)) => (pa + da * s, pb - db * u),
        None => (pa + da * chord * 0.5, pb - db * chord * 0.5),
    };
    let sim = (va / vb).min(vb / va).max(0.5);
    let (ka, kb) = if va >= vb { (0.8, 0.8 * sim) } else { (0.8 * sim, 0.8) };
    let c1 = pa + (ca - pa) * ka;
    let c2 = pb + (cb - pb) * kb;
    (1..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let p = if i == n { pb } else { pa * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + pb * (t * t * t) };
            [p.x, p.y, a[2] + (b[2] - a[2]) * t]
        })
        .collect()
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
        let mut pts = vec![at(0)];
        for k in 1..=12 {
            pts.extend(c.push(at(k), 1.5));
        }
        pts.extend(c.finish(1.5));
        let last = pts.last().unwrap();
        assert!((last[0] - 100.0).abs() < 1e-3 && last[1].abs() < 1e-2, "ends on the last sample");
        // where both neighbouring samples are known (30°–330°), within a unit
        // of the circle (joined straight: 3.4)
        let inner = |p: &&[f32; 3]| {
            let a = p[1].atan2(p[0]).rem_euclid(std::f32::consts::TAU).to_degrees();
            (31.0..329.0).contains(&a)
        };
        let worst = pts.iter().filter(inner).map(|p| ((p[0] * p[0] + p[1] * p[1]).sqrt() - 100.0).abs()).fold(0.0, f32::max);
        assert!(worst < 1.0, "off the circle by {worst}");
        // dense: no gap much wider than the step (the curve is a little longer than its chord)
        assert!(pts.windows(2).all(|w| (vec2(w[1][0], w[1][1]) - vec2(w[0][0], w[0][1])).length() <= 2.5));
        // pressure rises monotonically with the samples
        assert!(pts.windows(2).all(|w| w[1][2] >= w[0][2] - 1e-6));
        // and the curve always turns the same way (no bends against itself)
        let turns: Vec<f32> = pts.windows(3).map(|w| (w[1][0] - w[0][0]) * (w[2][1] - w[1][1]) - (w[1][1] - w[0][1]) * (w[2][0] - w[1][0])).collect();
        assert!(turns.iter().all(|&t| t >= -0.05), "turns back: {:?}", turns.iter().cloned().fold(0.0f32, f32::min));
    }

    /// The pen stops after a quick line with strong smoothing: the brush,
    /// left behind by its smoothing distance, follows on to the pen frame by
    /// frame and settles under it (Ash, 10-22: it waited at a distance).
    #[test]
    fn a_resting_pen_is_caught_up() {
        let mut c = Curve::new([0.0, 0.0, 0.5], 90.0);
        let mut pts = Vec::new();
        for k in 1..=20 {
            pts.extend(c.push([k as f32 * 15.0, 0.0, 0.5], 1.5));
        }
        let behind = 300.0 - pts.last().unwrap()[0];
        assert!(behind > 20.0, "smoothing left it {behind} behind");
        let mut frames = 0;
        loop {
            let (more, going) = c.catch_up(0.1, 1.5);
            pts.extend(more);
            frames += 1;
            if !going || frames > 200 {
                break;
            }
        }
        let gap = 300.0 - pts.last().unwrap()[0];
        assert!(gap < 1.0 && frames < 80, "{gap} short after {frames} frames");
        // straight on, never back
        assert!(pts.windows(2).all(|w| w[1][0] >= w[0][0] - 1e-3 && w[1][1].abs() < 1e-3));
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
        let (mut pts, mut raw) = (Vec::new(), vec![[0.0f32, 0.0]]);
        while x < 600.0 {
            x += 10.0 + rnd() * 8.0;
            let y = rnd() * 3.0;
            raw.push([x, y]);
            pts.extend(c.push([x, y, 0.5], 1.5));
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
        let mut pts = vec![[80.0f32, 0.0, 0.5]];
        let mut a = 0.0f32;
        while a < 4.0 {
            a += 0.16 + rnd() * 0.08;
            let r = 80.0 + rnd() * 1.0;
            pts.extend(c.push([r * a.cos(), r * a.sin(), 0.5], 1.5));
        }
        pts.extend(c.finish(1.5));
        // over every 15 px (ten steps) the heading turns the arc's way
        let head: Vec<f32> = pts.windows(2).map(|w| (w[1][1] - w[0][1]).atan2(w[1][0] - w[0][0])).collect();
        let wrap = |d: f32| (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        // (the last 45 px follow the pen's own samples to where it left)
        let body = &head[..head.len() - 30];
        let back = body.windows(11).filter(|h| wrap(h[10] - h[0]) < 0.0).count();
        let tail_worst = head[head.len() - 41..].windows(11).map(|h| wrap(h[10] - h[0]).to_degrees()).fold(f32::MAX, f32::min);
        assert!(tail_worst > -5.0, "the tail bends back {tail_worst}°");
        assert!(back == 0, "{back} of {} stretches bend against the arc", head.len());
    }
}
