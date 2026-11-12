// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (AGPL-3.0 section 7): see NOTICE.

//! Line suggestions for a space's vanishing lines (notes/updates/line-suggest.md).
//!
//! - Segments: LSD (`lsd.rs`, a port of its reference code; this file is
//!   the program's own).
//! - Groups: vanishing points found one after the other by RANSAC on segment
//!   pairs. A segment supports a point when the line from its midpoint to the
//!   point runs within a small angle of it (Rother 2002, Tardif 2009).
//! - Auto-pick: orthogonal directions have vanishing points with
//!   `(v₁−c)·(v₂−c) = −f²` (Caprile & Torre 1990), so the groups taken for
//!   X/Y/Z must give a positive, plausible and (for three) consistent f².
//!
//! Coordinates are the analysed image's pixels (corner convention: pixel i
//! spans i..i+1); grouping works in coordinates centred on `c` and divided by
//! the long side, so its thresholds don't depend on the picture size.

use crate::persp::{V3, cross, dot, normalize};
use egui::Pos2;
#[cfg(test)]
use egui::pos2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub a: Pos2,
    pub b: Pos2,
}

impl Seg {
    pub fn len(&self) -> f32 {
        self.a.distance(self.b)
    }
}

pub use crate::lsd::detect;

// ---------------------------------------------------------------- grouping

/// Segments that share a vanishing point.
#[derive(Clone, Debug)]
pub struct Group {
    /// Homogeneous vanishing point in normalised coordinates (unit, z ≥ 0).
    pub vp: V3,
    /// Indices into the segments, longest first.
    pub members: Vec<usize>,
    /// Total member length (pixels).
    pub weight: f64,
}

/// Maps image pixels to the normalised frame (centre `c`, long side `l`).
#[derive(Clone, Copy, Debug)]
pub struct Norm {
    pub c: Pos2,
    pub l: f32,
}

impl Norm {
    pub fn h(&self, p: Pos2) -> V3 {
        [((p.x - self.c.x) / self.l) as f64, ((p.y - self.c.y) / self.l) as f64, 1.0]
    }

    pub fn line(&self, s: &Seg) -> V3 {
        normalize(cross(self.h(s.a), self.h(s.b)))
    }
}

/// Does segment `s` point at the vanishing point `v`?
fn supports(n: &Norm, s: &Seg, v: V3) -> bool {
    let (a, b) = (n.h(s.a), n.h(s.b));
    let m = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, 1.0];
    let lm = cross(m, v);
    let ls = cross(a, b);
    let (n1, n2) = ((lm[0] * lm[0] + lm[1] * lm[1]).sqrt(), (ls[0] * ls[0] + ls[1] * ls[1]).sqrt());
    if n1 < 1e-12 || n2 < 1e-12 {
        return false;
    }
    let sin = ((lm[0] * ls[1] - lm[1] * ls[0]) / (n1 * n2)).abs();
    // 1.5°, or a pixel at the segment's far end for short ones
    let tol = (1.5f64.to_radians()).max((1.0 / s.len().max(1.0) as f64).atan()).sin();
    if sin > tol {
        return false;
    }
    // a point on the segment itself is a crossing, not a vanishing point
    if v[2].abs() > 1e-9 {
        let vp = [v[0] / v[2], v[1] / v[2]];
        let d = [b[0] - a[0], b[1] - a[1]];
        let t = ((vp[0] - a[0]) * d[0] + (vp[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]);
        if (-0.05..=1.05).contains(&t) {
            return false;
        }
    }
    true
}

/// Least-squares vanishing point of segments (length-weighted), z ≥ 0.
fn fit_vp(n: &Norm, segs: &[Seg], idx: &[usize]) -> V3 {
    let mut m = [[0.0; 3]; 3];
    for &i in idx {
        let l = n.line(&segs[i]);
        let w = segs[i].len() as f64;
        for r in 0..3 {
            for c in 0..3 {
                m[r][c] += w * l[r] * l[c];
            }
        }
    }
    let v = crate::persp::smallest_eigvec(m);
    if v[2] < 0.0 { [-v[0], -v[1], -v[2]] } else { v }
}

/// Groups segments by vanishing point, strongest group first (at most `max`).
/// Deterministic: the sampling uses a fixed seed.
pub fn group(segs: &[Seg], n: &Norm, max: usize) -> Vec<Group> {
    let total: f64 = segs.iter().map(|s| s.len() as f64).sum();
    let mut left: Vec<usize> = (0..segs.len()).collect();
    let mut out = Vec::new();
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    while out.len() < max && left.len() >= 3 {
        // sample hypotheses from pairs, each segment by its length
        let mut cum = Vec::with_capacity(left.len());
        let mut acc = 0.0;
        for &i in &left {
            acc += segs[i].len() as f64;
            cum.push(acc);
        }
        let pick = |r: f64| cum.partition_point(|&c| c < r * acc).min(left.len() - 1);
        let mut best: Option<(f64, V3)> = None;
        for _ in 0..1500 {
            let (i, j) = (left[pick(rand())], left[pick(rand())]);
            if i == j {
                continue;
            }
            let v = cross(n.line(&segs[i]), n.line(&segs[j]));
            if dot(v, v) < 1e-8 {
                continue;
            }
            let v = normalize(v);
            let score: f64 = left.iter().filter(|&&k| supports(n, &segs[k], v)).map(|&k| segs[k].len() as f64).sum();
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, v));
            }
        }
        let Some((_, mut v)) = best else { break };
        let mut members: Vec<usize> = Vec::new();
        for _ in 0..3 {
            let m: Vec<usize> = left.iter().copied().filter(|&k| supports(n, &segs[k], v)).collect();
            if m.len() < 2 {
                break;
            }
            members = m;
            v = fit_vp(n, segs, &members);
        }
        let weight: f64 = members.iter().map(|&k| segs[k].len() as f64).sum();
        // a few edges of an object can carry an axis beside a tiled floor:
        // a group needs a fifth of the long side in all, or 2 % of all edges
        if members.len() < 3 || weight < (0.2 * n.l as f64).min(0.02 * total) {
            break;
        }
        left.retain(|k| !members.contains(k));
        members.sort_by(|&a, &b| segs[b].len().total_cmp(&segs[a].len()));
        out.push(Group { vp: v, members, weight });
    }
    out
}

// ---------------------------------------------------------------- auto-pick

/// How vertical a group's segments run on the picture (0..1, length-weighted).
fn verticality(g: &Group, segs: &[Seg]) -> f64 {
    let (mut s, mut w) = (0.0, 0.0);
    for &i in &g.members {
        let d = segs[i].b - segs[i].a;
        let l = d.length() as f64;
        s += (d.y.abs() as f64 / l.max(1e-9)) * l;
        w += l;
    }
    s / w.max(1e-9)
}

/// Distance of a vanishing point from the centre (normalised; infinite: ∞).
fn reach(v: V3) -> f64 {
    if v[2].abs() < 1e-9 { f64::INFINITY } else { (v[0] * v[0] + v[1] * v[1]).sqrt() / v[2].abs() }
}

/// Vanishing points farther than this (long sides) say little about f: the
/// f² of a pair with one of them is mostly noise.
const FAR: f64 = 8.0;
/// Plausible lenses: 10–300 mm equivalent on the long side (normalised f²).
const F2_LO: f64 = (10.0 / 36.0) * (10.0 / 36.0);
const F2_HI: f64 = (300.0 / 36.0) * (300.0 / 36.0);

/// f² (normalised) implied by two orthogonal vanishing points; None when the
/// pair can't be orthogonal for a plausible lens. `Some(None)`: plausible,
/// but one point is too far to tell f.
fn pair_f2(a: V3, b: V3) -> Option<Option<f64>> {
    if reach(a) > FAR || reach(b) > FAR {
        return Some(None);
    }
    let f2 = -(a[0] * b[0] + a[1] * b[1]) / (a[2] * b[2]);
    (F2_LO..=F2_HI).contains(&f2).then_some(Some(f2))
}

/// Are three vanishing points mutually orthogonal directions (within 8°) for
/// one focal length? f comes from the first pair that tells it (the
/// strongest groups come first), else a 50 mm lens. A weak group's point is
/// rough, so the tolerance is loose; the solver does the fine fit.
fn triple_ok(v: [V3; 3], f2s: &[f64]) -> bool {
    let f = f2s.first().map_or(50.0 / 36.0, |x| x.sqrt());
    let d = v.map(|p| normalize([p[0], p[1], f * p[2]]));
    [(0, 1), (0, 2), (1, 2)].iter().all(|&(i, j)| dot(d[i], d[j]).abs() < 8f64.to_radians().sin())
}

/// The groups for X, Y and Z (any may be missing) chosen from orthogonality
/// alone, for a space that doesn't solve yet: the strongest consistent triple,
/// else the strongest plausible pair. Z is the most vertical group; of the
/// others X has the farther vanishing point.
pub fn auto_axes(groups: &[Group], segs: &[Seg]) -> [Option<usize>; 3] {
    let g = &groups[..groups.len().min(5)];
    let mut best: Option<(f64, Vec<usize>)> = None;
    for i in 0..g.len() {
        for j in i + 1..g.len() {
            let Some(fij) = pair_f2(g[i].vp, g[j].vp) else { continue };
            let pair_w = g[i].weight + g[j].weight;
            if best.as_ref().is_none_or(|(b, _)| pair_w > *b) {
                best = Some((pair_w, vec![i, j]));
            }
            for k in j + 1..g.len() {
                let (Some(fik), Some(fjk)) = (pair_f2(g[i].vp, g[k].vp), pair_f2(g[j].vp, g[k].vp)) else { continue };
                let f2s: Vec<f64> = [fij, fik, fjk].into_iter().flatten().collect();
                if !triple_ok([g[i].vp, g[j].vp, g[k].vp], &f2s) {
                    continue;
                }
                // a consistent triple beats any pair
                let w = 1e9 + pair_w + g[k].weight;
                if best.as_ref().is_none_or(|(b, _)| w > *b) {
                    best = Some((w, vec![i, j, k]));
                }
            }
        }
    }
    let mut out = [None; 3];
    let Some((_, mut pick)) = best else { return out };
    pick.sort_by(|&a, &b| verticality(&g[b], segs).total_cmp(&verticality(&g[a], segs)));
    let rest = if pick.len() == 3 || verticality(&g[pick[0]], segs) > 0.7 {
        out[2] = Some(pick[0]);
        pick[1..].to_vec()
    } else {
        pick
    };
    let mut rest = rest;
    rest.sort_by(|&a, &b| reach(g[b].vp).total_cmp(&reach(g[a].vp)));
    for (k, &gi) in rest.iter().enumerate() {
        out[k] = Some(gi);
    }
    out
}

/// The axis of each group from a solved camera, by the solver's own measure:
/// each member's interpretation plane (camera centre and segment) should hold
/// the axis direction, `|n·d| = sin` of the angle off. A group takes the axis
/// with the smallest length-weighted median of that, when it is under 1.5°
/// (None: no axis). A group's vanishing point alone is too rough for this
/// when its segments are few and short. Segments are in canvas coordinates
/// here, as the camera's.
pub fn axes_from_camera(groups: &[Group], segs: &[Seg], cam: &crate::persp::Camera) -> Vec<Option<u8>> {
    let ray = |p: Pos2| [(p.x as f64 - cam.c[0]) / cam.f, (p.y as f64 - cam.c[1]) / cam.f, 1.0];
    groups
        .iter()
        .map(|g| {
            let off = |ax: usize| {
                let d = cam.axis_dir(ax);
                let mut r: Vec<(f64, f64)> = g.members.iter().map(|&i| (dot(normalize(cross(ray(segs[i].a), ray(segs[i].b))), d).abs(), segs[i].len() as f64)).collect();
                r.sort_by(|a, b| a.0.total_cmp(&b.0));
                let half = r.iter().map(|x| x.1).sum::<f64>() * 0.5;
                let mut acc = 0.0;
                r.iter().find(|x| {
                    acc += x.1;
                    acc >= half
                }).map_or(1.0, |x| x.0)
            };
            let (ax, e) = (0..3).map(|a| (a as u8, off(a))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            (e < 1.5f64.to_radians().sin()).then_some(ax)
        })
        .collect()
}

/// Up to `count` members of a group: long ones spread over the picture (a
/// spread fixes the vanishing point better than neighbours), skipping any on
/// the same line as one already taken or as one of `taken` (the space's own).
/// The longest goes first; each next one is the long member (at least 40 % of
/// the longest) farthest from those taken.
pub fn pick_lines(g: &Group, segs: &[Seg], n: &Norm, taken: &[Seg], count: usize) -> Vec<usize> {
    let same = |s: &Seg, t: &Seg| {
        let l = n.line(t);
        let d = |p: Pos2| {
            let h = n.h(p);
            dot(l, h).abs() / (l[0] * l[0] + l[1] * l[1]).sqrt().max(1e-12)
        };
        d(s.a) < 0.015 && d(s.b) < 0.015
    };
    let mid = |s: &Seg| s.a.lerp(s.b, 0.5);
    let top = g.members.first().map_or(0.0, |&i| segs[i].len());
    let mut pool: Vec<usize> = g.members.iter().copied().filter(|&i| segs[i].len() >= 0.4 * top && !taken.iter().any(|t| same(&segs[i], t))).collect();
    let mut out: Vec<usize> = Vec::new();
    while out.len() < count {
        let next = if out.is_empty() && taken.is_empty() {
            pool.first().copied()
        } else {
            // farthest from what is there already (the space's lines count too)
            let gap = |i: usize| {
                let m = mid(&segs[i]);
                out.iter().map(|&o| mid(&segs[o])).chain(taken.iter().map(mid)).map(|p| p.distance(m)).fold(f32::MAX, f32::min)
            };
            pool.iter().copied().max_by(|&a, &b| (gap(a) * segs[a].len().sqrt()).total_cmp(&(gap(b) * segs[b].len().sqrt())))
        };
        let Some(i) = next else { break };
        out.push(i);
        pool.retain(|&k| k != i && !same(&segs[k], &segs[i]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> (Vec<f32>, usize, usize) {
        let im = image::open(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/room.png")).expect("room.png").to_rgba8();
        let (w, h) = (im.width() as usize, im.height() as usize);
        (crate::edges::luminance(im.as_raw()), w, h)
    }

    /// The synthetic room: auto-picked lines solve to the camera of the
    /// hand-drawn reference lines (`golden::ROOM_LINES`).
    #[test]
    fn room_auto_pick_matches_reference_camera() {
        let (lum, w, h) = room();
        let t = std::time::Instant::now();
        let segs: Vec<Seg> = detect(&lum, w, h).into_iter().filter(|s| s.len() >= 0.015 * w.max(h) as f32).collect();
        let n = Norm { c: pos2(w as f32 * 0.5, h as f32 * 0.5), l: w.max(h) as f32 };
        let groups = group(&segs, &n, 5);
        let took = t.elapsed();
        let axes = auto_axes(&groups, &segs);
        assert!(axes.iter().all(|a| a.is_some()), "{axes:?} {groups:?}");
        let frame = egui::Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(w as f32, h as f32));
        let mut sp = crate::persp::Space::new(1, "s".into(), frame);
        for (ax, gi) in axes.iter().enumerate() {
            for i in pick_lines(&groups[gi.unwrap()], &segs, &n, &[], 4) {
                sp.lines.push(crate::persp::VLine { a: segs[i].a, b: segs[i].b, axis: ax as u8 });
            }
        }
        let got = crate::persp::solve(&sp).unwrap();
        let mut rf = crate::persp::Space::new(1, "s".into(), frame);
        for part in crate::golden::ROOM_LINES.split(';') {
            let v: Vec<f32> = part.split(',').map(|x| x.trim().parse().unwrap()).collect();
            rf.lines.push(crate::persp::VLine { a: pos2(v[0], v[1]), b: pos2(v[2], v[3]), axis: v[4] as u8 });
        }
        let want = crate::persp::solve(&rf).unwrap();
        // the reference camera names the same groups
        let named = axes_from_camera(&groups, &segs, &want);
        for (ax, gi) in axes.iter().enumerate() {
            assert_eq!(named[gi.unwrap()], Some(ax as u8), "{named:?} {axes:?}");
        }
        let ang = (0..3).map(|a| dot(got.axis_dir(a), want.axis_dir(a)).clamp(-1.0, 1.0).acos().to_degrees()).fold(0.0, f64::max);
        eprintln!("room: {} segments, {} groups, {:?}; f {:.0} mm vs {:.0} mm, axes within {ang:.2}°, rms {:.2}°", segs.len(), groups.len(), took, got.focal_35mm(), want.focal_35mm(), got.rms_deg);
        assert!((got.focal_35mm() / want.focal_35mm() - 1.0).abs() < 0.05);
        assert!(ang < 1.0);
    }
    /// A level camera (vertical vanishing point at infinity), turned 20°:
    /// segments projected from world lines along X, Y and Z, with clutter.
    #[test]
    fn level_camera_axes() {
        let (w, h, f) = (1200.0f64, 800.0f64, 1000.0f64);
        let a = 20f64.to_radians();
        let dirs: [V3; 3] = [[a.cos(), 0.0, a.sin()], [-a.sin(), 0.0, a.cos()], [0.0, -1.0, 0.0]];
        let mut rng = 12345u64;
        let mut rand = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 11) as f64 / (1u64 << 53) as f64
        };
        let proj = |p: V3| pos2((f * p[0] / p[2] + w / 2.0) as f32, (f * p[1] / p[2] + h / 2.0) as f32);
        let mut segs = Vec::new();
        for (k, d) in dirs.iter().enumerate() {
            for _ in 0..[12, 12, 8][k] {
                let p = [rand() * 600.0 - 300.0, rand() * 300.0 - 150.0, 400.0 + rand() * 500.0];
                let q = [p[0] + d[0] * 80.0, p[1] + d[1] * 80.0, p[2] + d[2] * 80.0];
                let jit = |x: Pos2, r: &mut dyn FnMut() -> f64| pos2(x.x + (r() as f32 - 0.5) * 0.6, x.y + (r() as f32 - 0.5) * 0.6);
                segs.push(Seg { a: jit(proj(p), &mut rand), b: jit(proj(q), &mut rand) });
            }
        }
        for _ in 0..15 {
            let (x, y, t) = (rand() * w, rand() * h, rand() * 6.3);
            segs.push(Seg { a: pos2(x as f32, y as f32), b: pos2((x + 40.0 * t.cos()) as f32, (y + 40.0 * t.sin()) as f32) });
        }
        let n = Norm { c: pos2(w as f32 / 2.0, h as f32 / 2.0), l: w as f32 };
        let groups = group(&segs, &n, 5);
        let axes = auto_axes(&groups, &segs);
        // each axis got the group of its own world lines
        for (k, gi) in axes.iter().enumerate() {
            let g = &groups[gi.expect("axis found")];
            let lo = [0, 12, 24][k];
            assert!(g.members.iter().filter(|&&i| (lo..lo + [12, 12, 8][k]).contains(&i)).count() >= g.members.len() - 1, "axis {k}: {:?}", g.members);
        }
    }
}
