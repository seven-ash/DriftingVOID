// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.

//! Edge loupe processing: an automatic levels stretch and a Canny edge map of a
//! small window of the picture, for placing vanishing lines and wall corners.
//!
//! - Levels: the luminance's 1st and 99th percentiles are stretched to 0..255
//!   (a percentile contrast stretch, so a few outliers don't hold the range).
//! - Edges: J. Canny, "A Computational Approach to Edge Detection", IEEE PAMI
//!   8(6), 1986: Gaussian smoothing, Sobel gradient, non-maximum suppression
//!   along the gradient, then hysteresis between two thresholds. The thresholds
//!   come from the window's own gradients (the high one at a percentile of the
//!   magnitudes, the low one 0.4 of it), as MATLAB's `edge(…, 'canny')` does.

/// Luminance (0..255) of premultiplied RGBA pixels; transparent ones read 0.
pub fn luminance(rgba: &[u8]) -> Vec<f32> {
    rgba.chunks_exact(4).map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).collect()
}

/// Stretches premultiplied RGBA so the luminance's 1st..99th percentiles span
/// 0..255 (alpha kept). A window with almost no contrast is left as it is.
pub fn levels(rgba: &mut [u8]) {
    let mut hist = [0u32; 256];
    let mut n = 0u32;
    for p in rgba.chunks_exact(4) {
        if p[3] >= 8 {
            let l = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) as usize;
            hist[l.min(255)] += 1;
            n += 1;
        }
    }
    if n == 0 {
        return;
    }
    let at = |q: f32| {
        let goal = (n as f32 * q) as u32;
        let mut acc = 0;
        for (i, &c) in hist.iter().enumerate() {
            acc += c;
            if acc > goal {
                return i as f32;
            }
        }
        255.0
    };
    let (lo, hi) = (at(0.01), at(0.99));
    if hi - lo < 8.0 {
        return;
    }
    let k = 255.0 / (hi - lo);
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as f32;
        // stretch within the pixel's own alpha (premultiplied)
        for c in &mut p[..3] {
            *c = ((*c as f32 - lo * a / 255.0) * k).clamp(0.0, a) as u8;
        }
    }
}

pub fn blur(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let r = (sigma * 3.0).ceil().max(1.0) as isize;
    let ker: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = ker.iter().sum();
    let ker: Vec<f32> = ker.iter().map(|v| v / sum).collect();
    let pass = |src: &[f32], horiz: bool| -> Vec<f32> {
        let mut out = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for (j, k) in ker.iter().enumerate() {
                    let o = j as isize - r;
                    let (sx, sy) = if horiz { ((x as isize + o).clamp(0, w as isize - 1) as usize, y) } else { (x, (y as isize + o).clamp(0, h as isize - 1) as usize) };
                    acc += src[sy * w + sx] * k;
                }
                out[y * w + x] = acc;
            }
        }
        out
    };
    pass(&pass(src, true), false)
}

/// Canny edges of a `w`×`h` luminance image (true on an edge pixel). `sigma` is
/// the smoothing in pixels; `pct` the share of pixels whose gradient lies below
/// the high threshold (0.85: the strongest 15 % can start an edge).
pub fn canny(lum: &[f32], w: usize, h: usize, sigma: f32, pct: f32) -> Vec<bool> {
    let mut edge = vec![false; w * h];
    if w < 3 || h < 3 {
        return edge;
    }
    let s = blur(lum, w, h, sigma.max(0.5));
    let at = |x: usize, y: usize| s[y * w + x];
    let mut mag = vec![0.0f32; w * h];
    let mut dir = vec![0u8; w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x - 1, y) - at(x - 1, y + 1);
            let gy = at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1);
            let i = y * w + x;
            mag[i] = (gx * gx + gy * gy).sqrt();
            // gradient direction in four bins: 0°, 45°, 90°, 135°
            let a = gy.atan2(gx).to_degrees().rem_euclid(180.0);
            dir[i] = if !(22.5..157.5).contains(&a) { 0 } else if a < 67.5 { 1 } else if a < 112.5 { 2 } else { 3 };
        }
    }
    // thin: keep a pixel only where it is a maximum across the edge
    let mut thin = vec![0.0f32; w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let m = mag[i];
            if m <= 0.0 {
                continue;
            }
            let (a, b) = match dir[i] {
                0 => (mag[i - 1], mag[i + 1]),
                1 => (mag[i - w - 1], mag[i + w + 1]),
                2 => (mag[i - w], mag[i + w]),
                _ => (mag[i - w + 1], mag[i + w - 1]),
            };
            if m >= a && m >= b {
                thin[i] = m;
            }
        }
    }
    // thresholds from this window's gradients
    let mut sorted: Vec<f32> = mag.iter().copied().filter(|m| *m > 0.0).collect();
    if sorted.is_empty() {
        return edge;
    }
    sorted.sort_by(|a, b| a.total_cmp(b));
    let high = sorted[((sorted.len() as f32 * pct.clamp(0.0, 0.999)) as usize).min(sorted.len() - 1)].max(4.0);
    let low = high * 0.4;
    // hysteresis: strong pixels, then weak ones connected to them (8-neighbours)
    let mut stack: Vec<usize> = (0..w * h).filter(|&i| thin[i] >= high).collect();
    for &i in &stack {
        edge[i] = true;
    }
    while let Some(i) = stack.pop() {
        let (x, y) = ((i % w) as isize, (i / w) as isize);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if !edge[j] && thin[j] >= low {
                    edge[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    edge
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_has_edges_only_on_its_outline() {
        let (w, h) = (64, 64);
        let lum: Vec<f32> = (0..w * h).map(|i| if (16..48).contains(&(i % w)) && (16..48).contains(&(i / w)) { 200.0 } else { 40.0 }).collect();
        let e = canny(&lum, w, h, 1.0, 0.85);
        let on = |x: usize, y: usize| e[y * w + x];
        // the left and top sides are found (within a pixel), the inside and outside are clear
        assert!((14..=17).any(|x| on(x, 32)), "left side");
        assert!((14..=17).any(|y| on(32, y)), "top side");
        assert!(!on(32, 32) && !on(4, 4) && !on(60, 60));
        // thin: one pixel across
        let across = (8..24).filter(|&x| on(x, 32)).count();
        assert!(across <= 2, "{across}");
    }

    #[test]
    fn levels_stretch_a_dull_window() {
        let mut px: Vec<u8> = (0..100).flat_map(|i| {
            let v = 100 + (i % 40) as u8;
            [v, v, v, 255]
        }).collect();
        levels(&mut px);
        let l = luminance(&px);
        let (lo, hi) = l.iter().fold((255.0f32, 0.0f32), |a, v| (a.0.min(*v), a.1.max(*v)));
        assert!(lo < 10.0 && hi > 245.0, "{lo} {hi}");
        // flat windows stay as they are
        let mut flat = vec![120u8, 120, 120, 255].repeat(50);
        levels(&mut flat);
        assert_eq!(flat[0], 120);
    }
}
