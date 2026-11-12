// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (AGPL-3.0 section 7): see NOTICE.

//! Brush strokes as dabs (notes/updates/2026-11-12-vector-brush.md). A
//! stroke is its input (positions and pressure); the dabs that paint it are
//! made for the scale it is drawn at, so their number follows the stroke's
//! length on screen, not in canvas units. The CPU engine (`RasterStroke`)
//! and the GPU (`strokes.wgsl`, `fs_dab`) take the same dabs and the same
//! coverage, so a stroke looks the same whichever draws it.

use egui::{Pos2, Rect, Vec2, pos2};

use crate::raster::BrushParams;

/// One dab in target pixels: centre, radius and alpha (pressure → opacity).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    pub c: Pos2,
    pub r: f32,
    pub a: f32,
}

/// Lays dabs along a stroke as its points come, for a brush drawn at `s`
/// target pixels per canvas unit. Points are given in target pixels.
pub struct Stepper {
    p: BrushParams,
    s: f32,
    last: Option<(Pos2, f32)>,
    carry: f32,
    /// Only dabs that reach into this rect (target pixels) are given; the
    /// spacing is kept along the whole stroke, so the ones given are where
    /// they would be without it.
    clip: Option<Rect>,
}

impl Stepper {
    pub fn new(p: BrushParams, s: f32) -> Self {
        Self { p, s, last: None, carry: 0.0, clip: None }
    }

    /// As `new`, giving only the dabs that reach into `clip`.
    pub fn clipped(p: BrushParams, s: f32, clip: Rect) -> Self {
        Self { p, s, last: None, carry: 0.0, clip: Some(clip) }
    }

    fn shows(&self, c: Pos2, r: f32) -> bool {
        self.clip.is_none_or(|k| k.expand(r + 1.0).contains(c))
    }

    /// Dab radius in target pixels at this pressure (at least half a pixel).
    pub fn radius_at(&self, pressure: f32) -> f32 {
        let r = if self.p.pressure_size { self.p.radius * (0.12 + 0.88 * pressure) } else { self.p.radius };
        (r * self.s).max(0.5)
    }

    pub fn alpha_at(&self, pressure: f32) -> f32 {
        if self.p.pressure_opacity { pressure.clamp(0.0, 1.0) } else { 1.0 }
    }

    /// The dabs from the last point to `p` (the first point is one dab).
    pub fn push(&mut self, p: Pos2, pressure: f32, out: &mut Vec<Dab>) {
        let pressure = pressure.clamp(0.0, 1.0);
        match self.last {
            None => {
                let r = self.radius_at(pressure);
                if self.shows(p, r) {
                    out.push(Dab { c: p, r, a: self.alpha_at(pressure) });
                }
                self.last = Some((p, pressure));
                self.carry = 0.0;
            }
            Some((q, qp)) => {
                let d = p - q;
                let dist = d.length();
                if dist < 1e-4 {
                    return;
                }
                // `t` is the position of the previous dab relative to q along the segment.
                let mut t = -self.carry;
                loop {
                    let pr = qp + (pressure - qp) * (t.max(0.0) / dist).min(1.0);
                    let step = (self.radius_at(pr) * 0.12).max(0.35);
                    if t + step > dist {
                        self.carry = dist - t;
                        break;
                    }
                    t += step;
                    let c = q + d * (t / dist);
                    let pr = qp + (pressure - qp) * (t / dist);
                    let r = self.radius_at(pr);
                    if self.shows(c, r) {
                        out.push(Dab { c, r, a: self.alpha_at(pr) });
                    }
                }
                self.last = Some((p, pressure));
            }
        }
    }
}

/// Coverage of a dab at a pixel centre `d` target pixels from its centre:
/// full inside `r · hardness`, a smoothstep to nothing at `r`, the edge one
/// pixel wide; times the dab's alpha. `fs_dab` computes the same.
#[inline]
pub fn dab_alpha(d: f32, r: f32, hardness: f32, alpha: f32) -> f32 {
    if d > r + 0.5 {
        return 0.0;
    }
    let inner = r * hardness.clamp(0.0, 1.0);
    let soft = (r - inner).max(1e-3);
    let a = if d <= inner {
        1.0
    } else {
        let t = ((d - inner) / soft).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    };
    a.min((r + 0.5 - d).clamp(0.0, 1.0)) * alpha
}

/// A brush stroke kept as its input: points (x, y, pressure) in the layer's
/// space and the brush it was made with.
#[derive(Clone, Debug)]
pub struct BStroke {
    pub pts: Vec<[f32; 3]>,
    pub p: BrushParams,
    /// The points' box grown by the brush radius (layer space).
    pub bounds: Rect,
    /// Hash of the points and the brush, for the picture caches
    /// (`canvas::content_key`).
    pub key: u64,
}

impl BStroke {
    pub fn new(pts: Vec<[f32; 3]>, p: BrushParams) -> Self {
        let mut b = Rect::NOTHING;
        for q in &pts {
            b = b.union(Rect::from_min_max(pos2(q[0], q[1]), pos2(q[0], q[1])));
        }
        let bounds = if pts.is_empty() { Rect::NOTHING } else { b.expand(p.radius.max(0.5) + 1.0) };
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for v in [p.radius, p.opacity, p.hardness] {
                v.to_bits().hash(&mut h);
            }
            (p.color, p.pressure_size, p.pressure_opacity, p.erase).hash(&mut h);
            for q in &pts {
                for v in q {
                    v.to_bits().hash(&mut h);
                }
            }
            h.finish()
        };
        Self { pts, p, bounds, key }
    }

    /// The dabs that paint this stroke at `s` target pixels per layer unit,
    /// with the layer point `origin` at the target's top left.
    pub fn dabs(&self, origin: Pos2, s: f32) -> Vec<Dab> {
        self.dabs_mapped(|q| pos2((q.x - origin.x) * s, (q.y - origin.y) * s), s)
    }

    /// As `dabs`, only those reaching into `clip` (target pixels): a stroke
    /// drawn zoomed out and seen zoomed in is laid where it shows, not along
    /// all of its length.
    pub fn dabs_in(&self, origin: Pos2, s: f32, clip: Rect) -> Vec<Dab> {
        let mut st = Stepper::clipped(self.p, s, clip);
        let mut out = Vec::new();
        for q in &self.pts {
            st.push(pos2((q[0] - origin.x) * s, (q[1] - origin.y) * s), q[2], &mut out);
        }
        out
    }

    /// The dabs for a drawing that maps layer points to target pixels by
    /// `map`: a uniform scale `s` (with any turn and shift).
    pub fn dabs_mapped(&self, map: impl Fn(Pos2) -> Pos2, s: f32) -> Vec<Dab> {
        self.dabs_mapped_in(map, s, None)
    }

    /// As `dabs_mapped`, only those reaching into `clip` when given.
    pub fn dabs_mapped_in(&self, map: impl Fn(Pos2) -> Pos2, s: f32, clip: Option<Rect>) -> Vec<Dab> {
        let mut st = match clip {
            Some(k) => Stepper::clipped(self.p, s, k),
            None => Stepper::new(self.p, s),
        };
        let mut out = Vec::new();
        for q in &self.pts {
            st.push(map(pos2(q[0], q[1])), q[2], &mut out);
        }
        out
    }

    /// The same stroke moved by `map` (a uniform scale `s` with any turn and
    /// shift): its points follow, its brush grows by `s`.
    pub fn mapped(&self, map: impl Fn(Pos2) -> Pos2, s: f32) -> Self {
        let pts = self.pts.iter().map(|q| {
            let m = map(pos2(q[0], q[1]));
            [m.x, m.y, q[2]]
        }).collect();
        Self::new(pts, BrushParams { radius: self.p.radius * s, ..self.p })
    }

    /// Does the stroke cover layer point `q` (an eraser stroke never does)?
    pub fn hit(&self, q: Pos2) -> bool {
        if self.p.erase || !self.bounds.contains(q) {
            return false;
        }
        let st = Stepper::new(self.p, 1.0);
        let near = |a: &[f32; 3], b: &[f32; 3]| {
            let (pa, pb) = (pos2(a[0], a[1]), pos2(b[0], b[1]));
            let ab = pb - pa;
            let t = ((q - pa).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
            let r = st.radius_at(a[2] + (b[2] - a[2]) * t);
            (pa + ab * t).distance(q) <= r
        };
        match self.pts.len() {
            0 => false,
            1 => near(&self.pts[0], &self.pts[0]),
            _ => self.pts.windows(2).any(|w| near(&w[0], &w[1])),
        }
    }
}

/// A brush stroke being made: into the layer's pixels (masks, recording:
/// the CPU engine) or as a stroke object that grows (`BStroke`, the last of
/// the layer's strokes while it is drawn).
pub enum Painting {
    Pixels(crate::raster::RasterStroke),
    Stroke { p: BrushParams, pts: Vec<[f32; 3]> },
}

impl Painting {
    pub fn params(&self) -> BrushParams {
        match self {
            Painting::Pixels(st) => st.p,
            Painting::Stroke { p, .. } => *p,
        }
    }

    /// Starts on `layer` at layer point `lp`.
    pub fn start(layer: &mut crate::raster::RasterLayer, p: BrushParams, as_stroke: bool, lp: Pos2, pressure: f32) -> Self {
        if as_stroke {
            let pts = vec![[lp.x, lp.y, pressure]];
            layer.strokes.push(std::sync::Arc::new(BStroke::new(pts.clone(), p)));
            Painting::Stroke { p, pts }
        } else {
            let mut st = crate::raster::RasterStroke::new(p);
            st.add_point(layer, lp, pressure);
            Painting::Pixels(st)
        }
    }

    /// More points (layer space); a stroke object is made again once per call.
    pub fn add(&mut self, layer: &mut crate::raster::RasterLayer, more: &[[f32; 3]]) {
        match self {
            Painting::Pixels(st) => {
                for q in more {
                    st.add_point(layer, pos2(q[0], q[1]), q[2]);
                }
            }
            Painting::Stroke { p, pts } => {
                if more.is_empty() {
                    return;
                }
                pts.extend_from_slice(more);
                let s = std::sync::Arc::new(BStroke::new(pts.clone(), *p));
                match layer.strokes.last_mut() {
                    Some(last) => *last = s,
                    None => layer.strokes.push(s),
                }
            }
        }
    }

    pub fn finish(self, layer: &mut crate::raster::RasterLayer) {
        if let Painting::Pixels(st) = self {
            st.finish(layer);
        }
    }

    /// Takes the stroke back out (Esc).
    pub fn cancel(self, layer: &mut crate::raster::RasterLayer) {
        match self {
            Painting::Pixels(st) => st.cancel(layer),
            Painting::Stroke { .. } => {
                layer.strokes.pop();
            }
        }
    }
}

/// Paints one stroke's dabs into a premultiplied RGBA buffer (w × h), as
/// `RasterStroke` does into tiles: the dabs' largest coverage per pixel,
/// then the colour over what is there (an eraser: what is there taken
/// away), with the brush opacity.
pub fn paint(px: &mut [u8], w: u32, h: u32, dabs: &[Dab], p: &BrushParams) {
    let b = dabs_box(dabs).intersect(Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32)));
    if !b.is_positive() {
        return;
    }
    let (x0, y0) = (b.min.x.floor().max(0.0) as i32, b.min.y.floor().max(0.0) as i32);
    let (x1, y1) = ((b.max.x.ceil() as i32).min(w as i32), (b.max.y.ceil() as i32).min(h as i32));
    let (bw, bh) = ((x1 - x0).max(0) as usize, (y1 - y0).max(0) as usize);
    let mut mask = vec![0f32; bw * bh];
    for d in dabs {
        let (dx0, dy0) = (((d.c.x - d.r - 1.0).floor() as i32).max(x0), ((d.c.y - d.r - 1.0).floor() as i32).max(y0));
        let (dx1, dy1) = (((d.c.x + d.r + 1.0).ceil() as i32).min(x1 - 1), ((d.c.y + d.r + 1.0).ceil() as i32).min(y1 - 1));
        for y in dy0..=dy1 {
            let fy = y as f32 + 0.5 - d.c.y;
            for x in dx0..=dx1 {
                let fx = x as f32 + 0.5 - d.c.x;
                let a = dab_alpha((fx * fx + fy * fy).sqrt(), d.r, p.hardness, d.a);
                let m = &mut mask[(y - y0) as usize * bw + (x - x0) as usize];
                if a > *m {
                    *m = a;
                }
            }
        }
    }
    let op = p.opacity.clamp(0.0, 1.0);
    let col = p.color.to_srgba_unmultiplied().map(|x| x as f32 / 255.0);
    for y in 0..bh {
        for x in 0..bw {
            let a = mask[y * bw + x];
            if a <= 0.0 {
                continue;
            }
            let sa = a * op;
            let i = ((y0 as usize + y) * w as usize + x0 as usize + x) * 4;
            if p.erase {
                for k in 0..4 {
                    px[i + k] = (px[i + k] as f32 * (1.0 - sa)).round() as u8;
                }
            } else {
                let src = [col[0] * sa, col[1] * sa, col[2] * sa, sa];
                for k in 0..4 {
                    px[i + k] = (src[k] * 255.0 + px[i + k] as f32 * (1.0 - sa)).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// A stroke's dabs at one scale, in layer units × scale (layer point 0 at
/// 0), kept so that drawing a part of the view lays nothing again; in runs
/// of `RUN` with each run's box, to find the ones in a part quickly.
pub struct DabSet {
    pub dabs: Vec<Dab>,
    runs: Vec<(Rect, usize, usize)>,
}

const RUN: usize = 64;

impl DabSet {
    pub fn new(st: &BStroke, s: f32) -> Self {
        let dabs = st.dabs(Pos2::ZERO, s);
        let runs = dabs.chunks(RUN).enumerate().map(|(i, c)| (dabs_box(c), i * RUN, i * RUN + c.len())).collect();
        Self { dabs, runs }
    }

    /// The dabs that reach into `clip`, moved by `shift` (both in target
    /// pixels: the set's place plus `shift`).
    pub fn visible(&self, clip: Rect, shift: Vec2, out: &mut Vec<Dab>) {
        let k = clip.translate(-shift);
        for (b, i0, i1) in &self.runs {
            if !b.intersects(k) {
                continue;
            }
            for d in &self.dabs[*i0..*i1] {
                if k.expand(d.r + 1.0).contains(d.c) {
                    out.push(Dab { c: d.c + shift, ..*d });
                }
            }
        }
    }
}

/// The target pixels a set of dabs can touch: x0, y0, x1, y1.
pub fn dabs_box(dabs: &[Dab]) -> Rect {
    dabs.iter().fold(Rect::NOTHING, |b, d| b.union(Rect::from_center_size(d.c, Vec2::splat((d.r + 1.0) * 2.0))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    fn params(radius: f32) -> BrushParams {
        BrushParams { color: Color32::RED, radius, opacity: 1.0, hardness: 0.8, pressure_size: false, pressure_opacity: false, erase: false }
    }

    /// A stroke being drawn grows as the layer's last stroke; Esc takes it
    /// out; baking paints it into pixels like the old engine; hitting and
    /// bounds see it.
    #[test]
    fn a_painting_stroke_lives_in_the_layer() {
        use crate::raster::{RasterLayer, RasterStroke};
        let mut l = RasterLayer::default();
        let mut p = Painting::start(&mut l, params(5.0), true, pos2(10.0, 10.0), 1.0);
        p.add(&mut l, &[[40.0, 10.0, 1.0], [80.0, 30.0, 1.0]]);
        assert_eq!((l.strokes.len(), l.strokes[0].pts.len(), l.tiles.len()), (1, 3, 0));
        assert!(l.alpha_at_world(pos2(40.0, 12.0)) > 0 && l.alpha_at_world(pos2(40.0, 40.0)) == 0);
        assert!(l.bounds().unwrap().contains(pos2(80.0, 30.0)));
        p.finish(&mut l);
        // baked: the same pixels as the old engine painting those points
        let mut old = RasterLayer::default();
        let mut st = RasterStroke::new(params(5.0));
        for q in [[10.0, 10.0], [40.0, 10.0], [80.0, 30.0]] {
            st.add_point(&mut old, pos2(q[0], q[1]), 1.0);
        }
        st.finish(&mut old);
        let mut baked = l.clone();
        baked.bake();
        assert!(baked.strokes.is_empty());
        assert_eq!(baked.read_region(0, 0, 100, 50), old.read_region(0, 0, 100, 50));
        // a second stroke, cancelled: gone
        let p2 = Painting::start(&mut l, params(5.0), true, pos2(0.0, 0.0), 1.0);
        assert_eq!(l.strokes.len(), 2);
        p2.cancel(&mut l);
        assert_eq!(l.strokes.len(), 1);
        // moved exactly by a scale and a turn: points follow, the brush grows
        let t = crate::raster::transformed(&l, pos2(0.0, 0.0), 2.0, std::f32::consts::FRAC_PI_2).unwrap();
        let q = t.strokes[0].pts[1];
        let w = pos2(q[0], q[1]) + t.offset;
        assert!((w - pos2(-20.0, 80.0)).length() < 1e-3, "{w:?}");
        assert_eq!(t.strokes[0].p.radius, 10.0);
    }

    /// Dabs laid only where they show are the very dabs of the whole stroke
    /// that reach there (same places, sizes and alphas), and far fewer of
    /// them for a long stroke seen up close.
    #[test]
    fn clipped_dabs_are_the_same_dabs() {
        let mut p = params(6.0);
        p.pressure_size = true;
        let pts: Vec<[f32; 3]> = (0..=150).map(|i| [i as f32 * 400.0, (i as f32 * 0.04).sin() * 5000.0, if i % 30 < 15 { 1.0 } else { 0.4 + (i as f32 * 0.1).sin().abs() * 0.6 }]).collect();
        let st = BStroke::new(pts, p);
        let origin = pos2(20000.0, -3000.0);
        let clip = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        let all = st.dabs(origin, 1.0);
        let shown: Vec<Dab> = all.iter().copied().filter(|d| clip.expand(d.r + 1.0).contains(d.c)).collect();
        let fast = st.dabs_in(origin, 1.0, clip);
        assert_eq!(fast.len(), shown.len(), "{} of {}", fast.len(), all.len());
        for (a, b) in fast.iter().zip(&shown) {
            assert!((a.c - b.c).length() < 0.05 && (a.r - b.r).abs() < 1e-3 && (a.a - b.a).abs() < 1e-4, "{a:?} {b:?}");
        }
        assert!(fast.len() * 20 < all.len(), "{} of {}", fast.len(), all.len());
    }

    /// The dabs follow the stroke's length on screen: the same stroke shown
    /// at 1 % takes no more dabs than it does at 100 % (the old engine laid
    /// them in canvas units: about 100 times as many).
    #[test]
    fn dabs_follow_the_length_on_screen() {
        let pts: Vec<[f32; 3]> = (0..=150).map(|i| [i as f32 * 400.0, (i as f32 * 0.4).sin() * 5000.0, 1.0]).collect();
        let st = BStroke::new(pts, params(6.0));
        let far = st.dabs(Pos2::ZERO, 0.01).len();
        let near_screen: Vec<[f32; 3]> = (0..=150).map(|i| [i as f32 * 4.0, (i as f32 * 0.4).sin() * 50.0, 1.0]).collect();
        let near = BStroke::new(near_screen, params(6.0)).dabs(Pos2::ZERO, 1.0).len();
        // one pixel per canvas unit, as the old engine painted
        let old = st.dabs(Pos2::ZERO, 1.0).len();
        // (at 1 % the brush is under half a pixel: the 0.35 px spacing applies)
        assert!(far <= near * 3, "1 %: {far}, the same length at 100 %: {near}");
        assert!(far * 20 < old, "1 %: {far}, in canvas units: {old}");
    }
}
