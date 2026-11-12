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
}

impl Stepper {
    pub fn new(p: BrushParams, s: f32) -> Self {
        Self { p, s, last: None, carry: 0.0 }
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
                out.push(Dab { c: p, r: self.radius_at(pressure), a: self.alpha_at(pressure) });
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
                    out.push(Dab { c, r: self.radius_at(pr), a: self.alpha_at(pr) });
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
}

impl BStroke {
    pub fn new(pts: Vec<[f32; 3]>, p: BrushParams) -> Self {
        let mut b = Rect::NOTHING;
        for q in &pts {
            b = b.union(Rect::from_min_max(pos2(q[0], q[1]), pos2(q[0], q[1])));
        }
        let bounds = if pts.is_empty() { Rect::NOTHING } else { b.expand(p.radius.max(0.5) + 1.0) };
        Self { pts, p, bounds }
    }

    /// The dabs that paint this stroke at `s` target pixels per layer unit,
    /// with the layer point `origin` at the target's top left.
    pub fn dabs(&self, origin: Pos2, s: f32) -> Vec<Dab> {
        let mut st = Stepper::new(self.p, s);
        let mut out = Vec::new();
        for q in &self.pts {
            st.push(pos2((q[0] - origin.x) * s, (q[1] - origin.y) * s), q[2], &mut out);
        }
        out
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
