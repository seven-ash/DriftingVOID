// SPDX-License-Identifier: AGPL-3.0-or-later
//
// LSD - Line Segment Detector. This file is a Rust port of parts of the
// reference implementation published with the IPOL article (lsd.c, LSD 1.6):
//
//   Copyright (c) 2007-2011 rafael grompone von gioi <grompone@gmail.com>
//
//   This program is free software: you can redistribute it and/or modify
//   it under the terms of the GNU Affero General Public License as
//   published by the Free Software Foundation, either version 3 of the
//   License, or (at your option) any later version.
//
//   This program is distributed in the hope that it will be useful,
//   but WITHOUT ANY WARRANTY; without even the implied warranty of
//   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
//   GNU Affero General Public License for more details.
//
// Port and changes: Copyright (C) 2026 Ash (seven-ash), under the same
// licence (GNU AGPL version 3 or later; the text is in LICENSE), which is
// the licence of the whole of Drifting VOID. Additional terms (AGPL-3.0
// section 7): see NOTICE.
//
// Changed from the reference code: written in Rust for this program's image
// and segment types; only the parts needed here (the scale step, level-line
// angles, region growing, the rectangle, the NFA test and one refinement).

//! LSD: R. Grompone von Gioi, J. Jakubowicz, J.-M. Morel, G. Randall, "LSD: a
//! Fast Line Segment Detector with a False Detection Control", IEEE PAMI
//! 32(4), 2010; "LSD: a Line Segment Detector", Image Processing On Line, 2
//! (2012), https://doi.org/10.5201/ipol.2012.gjmr-lsd. Level-line angles
//! from a 2×2 gradient, regions grown from aligned neighbours in gradient
//! order, a rectangle per region, kept when its number of false alarms is
//! ≤ 1. No per-photo thresholds. Used by `lines.rs` for line suggestions.

use crate::lines::Seg;
use egui::pos2;

// ---------------------------------------------------------------- LSD

/// Angle tolerance (degrees) and gradient quantisation, as in the reference code.
const ANG_TH: f64 = 22.5;
const QUANT: f64 = 2.0;
/// Minimum share of aligned pixels in a region's rectangle.
const DENSITY: f64 = 0.7;
const BINS: usize = 1024;
const NOTDEF: f64 = -1024.0;

/// IPOL `isaligned`: level-line angle `a` within `prec` of `theta`.
fn aligned(a: f64, theta: f64, prec: f64) -> bool {
    if a == NOTDEF {
        return false;
    }
    let mut d = (theta - a).abs();
    if d > 1.5 * std::f64::consts::PI {
        d = (d - 2.0 * std::f64::consts::PI).abs();
    }
    d <= prec
}

fn angle_diff(a: f64, b: f64) -> f64 {
    let mut d = a - b;
    while d <= -std::f64::consts::PI {
        d += 2.0 * std::f64::consts::PI;
    }
    while d > std::f64::consts::PI {
        d -= 2.0 * std::f64::consts::PI;
    }
    d.abs()
}

/// ln Γ(x) for x > 0 (Lanczos, g = 7).
fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 9] = [0.999_999_999_999_809_9, 676.520_368_121_885_1, -1_259.139_216_722_402_8, 771.323_428_777_653_1, -176.615_029_162_140_6, 12.507_343_278_686_905, -0.138_571_095_265_720_12, 9.984_369_578_019_572e-6, 1.505_632_735_149_311_6e-7];
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + 7.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// −log₁₀ NFA of a rectangle with `k` aligned pixels out of `n` (IPOL `nfa`).
fn neg_log_nfa(n: usize, k: usize, p: f64, log_nt: f64) -> f64 {
    if n == 0 || k == 0 {
        return -log_nt;
    }
    if n == k {
        return -log_nt - n as f64 * p.log10();
    }
    let (nf, kf) = (n as f64, k as f64);
    let p_term = p / (1.0 - p);
    let log1 = ln_gamma(nf + 1.0) - ln_gamma(kf + 1.0) - ln_gamma(nf - kf + 1.0) + kf * p.ln() + (nf - kf) * (1.0 - p).ln();
    let mut term = log1.exp();
    if term == 0.0 {
        return if kf > nf * p { -log1 / std::f64::consts::LN_10 - log_nt } else { -log_nt };
    }
    let mut tail = term;
    for i in k + 1..=n {
        let bin = (nf - i as f64 + 1.0) / i as f64;
        let mult = bin * p_term;
        term *= mult;
        tail += term;
        if bin < 1.0 {
            let err = term * ((1.0 - mult.powf(nf - i as f64 + 1.0)) / (1.0 - mult) - 1.0);
            if err < 0.1 * (-tail.log10() - log_nt).abs() * tail {
                break;
            }
        }
    }
    -tail.log10() - log_nt
}

struct Rect {
    /// Centre line ends.
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    width: f64,
    theta: f64,
}

struct Field<'a> {
    ang: &'a [f64],
    mag: &'a [f32],
    w: usize,
    h: usize,
}

impl Field<'_> {
    fn rect(&self, reg: &[usize], reg_angle: f64, prec: f64) -> Rect {
        let w = self.w;
        let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
        for &i in reg {
            let g = self.mag[i] as f64;
            sx += (i % w) as f64 * g;
            sy += (i / w) as f64 * g;
            sw += g;
        }
        let (cx, cy) = (sx / sw, sy / sw);
        let (mut ixx, mut iyy, mut ixy) = (0.0, 0.0, 0.0);
        for &i in reg {
            let g = self.mag[i] as f64;
            let (dx, dy) = ((i % w) as f64 - cx, (i / w) as f64 - cy);
            ixx += dy * dy * g;
            iyy += dx * dx * g;
            ixy -= dx * dy * g;
        }
        let lambda = 0.5 * (ixx + iyy - ((ixx - iyy) * (ixx - iyy) + 4.0 * ixy * ixy).sqrt());
        let mut theta = if ixx.abs() > iyy.abs() { (lambda - ixx).atan2(ixy) } else { ixy.atan2(lambda - iyy) };
        if angle_diff(theta, reg_angle) > prec {
            theta += std::f64::consts::PI;
        }
        let (dx, dy) = (theta.cos(), theta.sin());
        let (mut l0, mut l1, mut w0, mut w1) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for &i in reg {
            let (px, py) = ((i % w) as f64 - cx, (i / w) as f64 - cy);
            let l = px * dx + py * dy;
            let ww = -px * dy + py * dx;
            l0 = l0.min(l);
            l1 = l1.max(l);
            w0 = w0.min(ww);
            w1 = w1.max(ww);
        }
        // the centre line runs midway across the width
        let wm = 0.5 * (w0 + w1);
        let (ox, oy) = (cx - wm * dy, cy + wm * dx);
        Rect { x1: ox + l0 * dx, y1: oy + l0 * dy, x2: ox + l1 * dx, y2: oy + l1 * dy, width: (w1 - w0).max(1.0), theta }
    }

    /// Pixels inside the rectangle, and how many of them are aligned with it.
    fn count(&self, r: &Rect, prec: f64) -> (usize, usize) {
        let (dx, dy) = (r.theta.cos(), r.theta.sin());
        let len = ((r.x2 - r.x1).powi(2) + (r.y2 - r.y1).powi(2)).sqrt();
        let hw = 0.5 * r.width;
        let (xs, ys) = ([r.x1, r.x2], [r.y1, r.y2]);
        let x0 = (xs[0].min(xs[1]) - hw).floor().max(0.0) as usize;
        let x1 = ((xs[0].max(xs[1]) + hw).ceil() as usize).min(self.w - 1);
        let y0 = (ys[0].min(ys[1]) - hw).floor().max(0.0) as usize;
        let y1 = ((ys[0].max(ys[1]) + hw).ceil() as usize).min(self.h - 1);
        let (mut n, mut k) = (0, 0);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f64 - r.x1, y as f64 - r.y1);
                let l = px * dx + py * dy;
                let ww = -px * dy + py * dx;
                if l < -0.5 || l > len + 0.5 || ww.abs() > hw {
                    continue;
                }
                n += 1;
                if aligned(self.ang[y * self.w + x], r.theta, prec) {
                    k += 1;
                }
            }
        }
        (n, k)
    }

    /// Grows a region of aligned pixels from `seed` (8-neighbourhood), marking them used.
    fn grow(&self, seed: usize, prec: f64, used: &mut [bool]) -> (Vec<usize>, f64) {
        let w = self.w;
        let mut reg = vec![seed];
        used[seed] = true;
        let mut angle = self.ang[seed];
        let (mut sdx, mut sdy) = (angle.cos(), angle.sin());
        let mut at = 0;
        while at < reg.len() {
            let i = reg[at];
            at += 1;
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            for yy in y - 1..=y + 1 {
                for xx in x - 1..=x + 1 {
                    if xx < 0 || yy < 0 || xx >= w as isize || yy >= self.h as isize {
                        continue;
                    }
                    let j = yy as usize * w + xx as usize;
                    if used[j] || !aligned(self.ang[j], angle, prec) {
                        continue;
                    }
                    used[j] = true;
                    reg.push(j);
                    sdx += self.ang[j].cos();
                    sdy += self.ang[j].sin();
                    angle = sdy.atan2(sdx);
                }
            }
        }
        (reg, angle)
    }
}

fn density(reg: &[usize], r: &Rect) -> f64 {
    let len = ((r.x2 - r.x1).powi(2) + (r.y2 - r.y1).powi(2)).sqrt();
    reg.len() as f64 / (len * r.width).max(1.0)
}

/// LSD's scale step: the image resampled by `SCALE` with a Gaussian of
/// σ = 0.6 / `SCALE` (IPOL `gaussian_sampler`). It removes the staircase of
/// nearly axis-aligned edges, which would otherwise break into short, exactly
/// horizontal or vertical pieces.
const SCALE: f64 = 0.8;

fn resample(src: &[f32], w: usize, h: usize) -> (Vec<f32>, usize, usize) {
    let sigma = 0.6 / SCALE;
    let half = (sigma * (2.0 * 3.0 * std::f64::consts::LN_10).sqrt()).ceil() as isize;
    let pass = |src: &[f32], n_in: usize, n_out: usize, lines: usize, at: &dyn Fn(usize, usize) -> usize, out_at: &dyn Fn(usize, usize) -> usize, out: &mut [f32]| {
        let dbl = 2 * n_in as isize;
        for x in 0..n_out {
            let xx = x as f64 / SCALE;
            let xc = (xx + 0.5).floor() as isize;
            let mean = half as f64 + xx - xc as f64;
            let ker: Vec<f64> = (0..=2 * half).map(|i| (-0.5 * ((i as f64 - mean) / sigma).powi(2)).exp()).collect();
            let sum: f64 = ker.iter().sum();
            for y in 0..lines {
                let mut acc = 0.0;
                for (i, k) in ker.iter().enumerate() {
                    let mut j = xc - half + i as isize;
                    while j < 0 {
                        j += dbl;
                    }
                    while j >= dbl {
                        j -= dbl;
                    }
                    if j >= n_in as isize {
                        j = dbl - 1 - j;
                    }
                    acc += k * src[at(j as usize, y)] as f64;
                }
                out[out_at(x, y)] = (acc / sum) as f32;
            }
        }
    };
    let (w2, h2) = ((w as f64 * SCALE).ceil() as usize, (h as f64 * SCALE).ceil() as usize);
    let mut aux = vec![0.0f32; w2 * h];
    pass(src, w, w2, h, &|j, y| y * w + j, &|x, y| y * w2 + x, &mut aux);
    let mut out = vec![0.0f32; w2 * h2];
    pass(&aux, h, h2, w2, &|j, x| j * w2 + x, &|y, x| y * w2 + x, &mut out);
    (out, w2, h2)
}

/// Line segments of a luminance image (0..255, row-major), in its pixels.
pub fn detect(lum: &[f32], w: usize, h: usize) -> Vec<Seg> {
    if w < 10 || h < 10 {
        return Vec::new();
    }
    let (img, w2, h2) = resample(lum, w, h);
    // scaled pixel centres back to source corner coordinates
    let back = |x: f64, y: f64| pos2((x / SCALE + 0.5) as f32, (y / SCALE + 0.5) as f32);
    detect_scaled(&img, w2, h2).into_iter().map(|(a, b)| Seg { a: back(a.0, a.1), b: back(b.0, b.1) }).collect()
}

/// LSD proper on the resampled image; ends in pixel-centre coordinates.
#[allow(clippy::type_complexity)]
fn detect_scaled(img: &[f32], w: usize, h: usize) -> Vec<((f64, f64), (f64, f64))> {
    let prec = ANG_TH.to_radians();
    let p = ANG_TH / 180.0;
    let rho = (QUANT / prec.sin()) as f32;
    let mut ang = vec![NOTDEF; w * h];
    let mut mag = vec![0.0f32; w * h];
    let mut top = 0.0f32;
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let i = y * w + x;
            let c1 = img[i + w + 1] - img[i];
            let c2 = img[i + 1] - img[i + w];
            let (gx, gy) = (c1 + c2, c1 - c2);
            let g = ((gx * gx + gy * gy) / 4.0).sqrt();
            mag[i] = g;
            if g > rho {
                ang[i] = (gx as f64).atan2(-gy as f64);
                top = top.max(g);
            }
        }
    }
    // pseudo-order: strongest gradients first
    let mut bins: Vec<Vec<usize>> = vec![Vec::new(); BINS];
    for (i, &a) in ang.iter().enumerate() {
        if a != NOTDEF {
            bins[((mag[i] / top * (BINS - 1) as f32) as usize).min(BINS - 1)].push(i);
        }
    }
    let field = Field { ang: &ang, mag: &mag, w, h };
    let log_nt = 2.5 * ((w as f64).log10() + (h as f64).log10()) + 11f64.log10();
    let min_reg = (-log_nt / p.log10()).ceil() as usize;
    let mut used = vec![false; w * h];
    let mut out = Vec::new();
    for &s in bins.iter().rev().flatten() {
        if used[s] {
            continue;
        }
        let (mut reg, mut ra) = field.grow(s, prec, &mut used);
        if reg.len() < min_reg {
            continue;
        }
        let mut r = field.rect(&reg, ra, prec);
        if density(&reg, &r) < DENSITY {
            // regrow tighter (IPOL `refine`): twice the angle spread near the seed
            let (sx, sy) = ((s % w) as f64, (s / w) as f64);
            let near: Vec<f64> = reg.iter().filter(|&&i| ((i % w) as f64 - sx).hypot((i / w) as f64 - sy) < r.width).map(|&i| angle_diff(ang[i], ra)).collect();
            let n = near.len().max(1) as f64;
            let mean = near.iter().sum::<f64>() / n;
            let tau = 2.0 * (near.iter().map(|d| d * d).sum::<f64>() / n - mean * mean).max(0.0).sqrt();
            for &i in &reg {
                used[i] = false;
            }
            (reg, ra) = field.grow(s, tau.max(1e-3), &mut used);
            if reg.len() < 2 {
                continue;
            }
            r = field.rect(&reg, ra, prec);
            // then shrink around the seed until dense enough
            let mut rad = reg.iter().map(|&i| ((i % w) as f64 - sx).hypot((i / w) as f64 - sy)).fold(0.0, f64::max);
            while density(&reg, &r) < DENSITY {
                rad *= 0.75;
                reg.retain(|&i| {
                    let keep = ((i % w) as f64 - sx).hypot((i / w) as f64 - sy) <= rad;
                    if !keep {
                        used[i] = false;
                    }
                    keep
                });
                if reg.len() < 2 {
                    break;
                }
                r = field.rect(&reg, ra, prec);
            }
            if reg.len() < 2 {
                continue;
            }
        }
        let (n, k) = field.count(&r, prec);
        if neg_log_nfa(n, k, p, log_nt) <= 0.0 {
            continue;
        }
        // the 2×2 gradient of pixel (x, y) sits between centres, at (x+½, y+½)
        out.push(((r.x1 + 0.5, r.y1 + 0.5), (r.x2 + 0.5, r.y2 + 0.5)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfa_matches_reference_values() {
        // a fully aligned run is very meaningful; a chance-level one is not
        assert!(neg_log_nfa(50, 50, 0.125, 7.0) > 30.0);
        assert!(neg_log_nfa(100, 12, 0.125, 7.0) < 0.0);
        assert!((ln_gamma(5.0) - 24f64.ln()).abs() < 1e-10);
    }

    #[test]
    fn detects_a_drawn_square() {
        let (w, h) = (200, 160);
        let mut lum = vec![30.0f32; w * h];
        for y in 40..120 {
            for x in 50..150 {
                lum[y * w + x] = 220.0;
            }
        }
        let segs = detect(&lum, w, h);
        // four long sides along the square's edges (x = 50, 150; y = 40, 120)
        let long: Vec<&Seg> = segs.iter().filter(|s| s.len() > 60.0).collect();
        assert!(long.len() >= 4, "{segs:?}");
        for (x, y) in [(50.0, f32::NAN), (150.0, f32::NAN), (f32::NAN, 40.0), (f32::NAN, 120.0)] {
            assert!(long.iter().any(|s| if x.is_nan() { (s.a.y - y).abs() < 1.0 && (s.b.y - y).abs() < 1.0 } else { (s.a.x - x).abs() < 1.0 && (s.b.x - x).abs() < 1.0 }), "side {x} {y}: {long:?}");
        }
    }
}
