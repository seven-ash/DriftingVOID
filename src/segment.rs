//! GrabCut (Rother, Kolmogorov, Blake 2004): two colour GMMs and an iterated
//! graph cut (Dinic max-flow on an 8-connected grid).

pub const BG: u8 = 0;
pub const FG: u8 = 1;
pub const PR_BG: u8 = 2;
pub const PR_FG: u8 = 3;

const K: usize = 5;
const GAMMA: f64 = 50.0;

#[derive(Clone, Copy, Default)]
struct Comp {
    w: f64,
    mu: [f64; 3],
    inv: [[f64; 3]; 3],
    det: f64,
}

struct Gmm {
    c: [Comp; K],
}

impl Gmm {
    fn comp_p(&self, k: usize, z: [f64; 3]) -> f64 {
        let c = &self.c[k];
        if c.w <= 0.0 {
            return 0.0;
        }
        let d = [z[0] - c.mu[0], z[1] - c.mu[1], z[2] - c.mu[2]];
        let mut m = 0.0;
        for i in 0..3 {
            for j in 0..3 {
                m += d[i] * c.inv[i][j] * d[j];
            }
        }
        c.w / c.det.sqrt() * (-0.5 * m).exp()
    }

    fn p(&self, z: [f64; 3]) -> f64 {
        (0..K).map(|k| self.comp_p(k, z)).sum()
    }

    fn best(&self, z: [f64; 3]) -> usize {
        (0..K).max_by(|a, b| self.comp_p(*a, z).partial_cmp(&self.comp_p(*b, z)).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0)
    }

    fn learn(s: &[[f64; 3]], assign: &[usize]) -> Gmm {
        let mut n = [0f64; K];
        let mut sum = [[0f64; 3]; K];
        let mut prod = [[[0f64; 3]; 3]; K];
        for (z, &k) in s.iter().zip(assign) {
            n[k] += 1.0;
            for i in 0..3 {
                sum[k][i] += z[i];
                for j in 0..3 {
                    prod[k][i][j] += z[i] * z[j];
                }
            }
        }
        let total = s.len().max(1) as f64;
        let mut c = [Comp::default(); K];
        for k in 0..K {
            if n[k] == 0.0 {
                continue;
            }
            let mu = [sum[k][0] / n[k], sum[k][1] / n[k], sum[k][2] / n[k]];
            let mut cov = [[0f64; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    cov[i][j] = prod[k][i][j] / n[k] - mu[i] * mu[j] + if i == j { 0.01 } else { 0.0 };
                }
            }
            let det = cov[0][0] * (cov[1][1] * cov[2][2] - cov[1][2] * cov[2][1]) - cov[0][1] * (cov[1][0] * cov[2][2] - cov[1][2] * cov[2][0])
                + cov[0][2] * (cov[1][0] * cov[2][1] - cov[1][1] * cov[2][0]);
            if det <= 1e-12 {
                continue;
            }
            let inv = [
                [
                    (cov[1][1] * cov[2][2] - cov[1][2] * cov[2][1]) / det,
                    (cov[0][2] * cov[2][1] - cov[0][1] * cov[2][2]) / det,
                    (cov[0][1] * cov[1][2] - cov[0][2] * cov[1][1]) / det,
                ],
                [
                    (cov[1][2] * cov[2][0] - cov[1][0] * cov[2][2]) / det,
                    (cov[0][0] * cov[2][2] - cov[0][2] * cov[2][0]) / det,
                    (cov[0][2] * cov[1][0] - cov[0][0] * cov[1][2]) / det,
                ],
                [
                    (cov[1][0] * cov[2][1] - cov[1][1] * cov[2][0]) / det,
                    (cov[0][1] * cov[2][0] - cov[0][0] * cov[2][1]) / det,
                    (cov[0][0] * cov[1][1] - cov[0][1] * cov[1][0]) / det,
                ],
            ];
            c[k] = Comp { w: n[k] / total, mu, inv, det };
        }
        Gmm { c }
    }
}

fn kmeans(s: &[[f64; 3]]) -> Vec<usize> {
    let mut centers: Vec<[f64; 3]> = (0..K).map(|k| s[k * s.len() / K]).collect();
    let mut assign = vec![0usize; s.len()];
    for _ in 0..10 {
        for (z, a) in s.iter().zip(assign.iter_mut()) {
            *a = (0..K)
                .min_by(|x, y| {
                    let dx: f64 = (0..3).map(|i| (z[i] - centers[*x][i]).powi(2)).sum();
                    let dy: f64 = (0..3).map(|i| (z[i] - centers[*y][i]).powi(2)).sum();
                    dx.partial_cmp(&dy).unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(0);
        }
        let mut acc = vec![([0f64; 3], 0f64); K];
        for (z, &a) in s.iter().zip(&assign) {
            for i in 0..3 {
                acc[a].0[i] += z[i];
            }
            acc[a].1 += 1.0;
        }
        for k in 0..K {
            if acc[k].1 > 0.0 {
                centers[k] = [acc[k].0[0] / acc[k].1, acc[k].0[1] / acc[k].1, acc[k].0[2] / acc[k].1];
            }
        }
    }
    assign
}

const NIL: u32 = u32::MAX;

struct Flow {
    head: Vec<u32>,
    to: Vec<u32>,
    next: Vec<u32>,
    cap: Vec<f64>,
    level: Vec<i32>,
    it: Vec<u32>,
}

impl Flow {
    fn new(n: usize, arcs: usize) -> Self {
        Self { head: vec![NIL; n], to: Vec::with_capacity(arcs), next: Vec::with_capacity(arcs), cap: Vec::with_capacity(arcs), level: vec![0; n], it: vec![NIL; n] }
    }

    fn add(&mut self, u: usize, v: usize, c: f64, rc: f64) {
        for (a, b, cc) in [(u, v, c), (v, u, rc)] {
            self.to.push(b as u32);
            self.cap.push(cc);
            self.next.push(self.head[a]);
            self.head[a] = (self.to.len() - 1) as u32;
        }
    }

    fn bfs(&mut self, s: usize) {
        self.level.iter_mut().for_each(|l| *l = -1);
        self.level[s] = 0;
        let mut q = std::collections::VecDeque::from([s]);
        while let Some(u) = q.pop_front() {
            let mut e = self.head[u];
            while e != NIL {
                let v = self.to[e as usize] as usize;
                if self.cap[e as usize] > 1e-9 && self.level[v] < 0 {
                    self.level[v] = self.level[u] + 1;
                    q.push_back(v);
                }
                e = self.next[e as usize];
            }
        }
    }

    fn augment(&mut self, s: usize, t: usize) -> f64 {
        let mut path: Vec<u32> = Vec::new();
        let mut u = s;
        loop {
            if u == t {
                let f = path.iter().map(|&e| self.cap[e as usize]).fold(f64::INFINITY, f64::min);
                for &e in &path {
                    self.cap[e as usize] -= f;
                    self.cap[(e ^ 1) as usize] += f;
                }
                return f;
            }
            let mut moved = false;
            while self.it[u] != NIL {
                let e = self.it[u] as usize;
                let v = self.to[e] as usize;
                if self.cap[e] > 1e-9 && self.level[v] == self.level[u] + 1 {
                    path.push(e as u32);
                    u = v;
                    moved = true;
                    break;
                }
                self.it[u] = self.next[e];
            }
            if !moved {
                if u == s {
                    return 0.0;
                }
                self.level[u] = -1;
                let e = path.pop().unwrap() as usize;
                u = self.to[e ^ 1] as usize;
                self.it[u] = self.next[self.it[u] as usize];
            }
        }
    }

    /// Max-flow; afterwards `level[v] >= 0` marks the source side.
    fn run(&mut self, s: usize, t: usize) {
        loop {
            self.bfs(s);
            if self.level[t] < 0 {
                break;
            }
            self.it.clone_from(&self.head);
            while self.augment(s, t) > 0.0 {}
        }
        self.bfs(s);
    }
}

/// Runs GrabCut on an RGB image (0..255), updating the probable labels in place.
pub fn grabcut(img: &[[f64; 3]], w: usize, h: usize, labels: &mut [u8], iters: usize) {
    let n = w * h;
    let idx = |x: usize, y: usize| y * w + x;
    // neighbour pairs (right, down, down-right, down-left) and beta
    let mut pairs: Vec<(usize, usize, f64)> = Vec::with_capacity(n * 4);
    for y in 0..h {
        for x in 0..w {
            let a = idx(x, y);
            if x + 1 < w {
                pairs.push((a, idx(x + 1, y), 1.0));
            }
            if y + 1 < h {
                pairs.push((a, idx(x, y + 1), 1.0));
                if x + 1 < w {
                    pairs.push((a, idx(x + 1, y + 1), std::f64::consts::SQRT_2));
                }
                if x > 0 {
                    pairs.push((a, idx(x - 1, y + 1), std::f64::consts::SQRT_2));
                }
            }
        }
    }
    let d2 = |a: usize, b: usize| (0..3).map(|i| (img[a][i] - img[b][i]).powi(2)).sum::<f64>();
    let mean = pairs.iter().map(|&(a, b, _)| d2(a, b)).sum::<f64>() / pairs.len().max(1) as f64;
    let beta = if mean > 0.0 { 1.0 / (2.0 * mean) } else { 0.0 };
    let wts: Vec<f64> = pairs.iter().map(|&(a, b, dist)| GAMMA * (-beta * d2(a, b)).exp() / dist).collect();
    let lambda = 9.0 * GAMMA;
    let is_fg = |l: u8| l == FG || l == PR_FG;
    let mut comp = vec![0usize; n];
    for round in 0..iters {
        let fg: Vec<usize> = (0..n).filter(|&i| is_fg(labels[i])).collect();
        let bg: Vec<usize> = (0..n).filter(|&i| !is_fg(labels[i])).collect();
        if fg.len() < K || bg.len() < K {
            return;
        }
        let fs: Vec<[f64; 3]> = fg.iter().map(|&i| img[i]).collect();
        let bs: Vec<[f64; 3]> = bg.iter().map(|&i| img[i]).collect();
        let (fa, ba) = if round == 0 {
            (kmeans(&fs), kmeans(&bs))
        } else {
            let fgm = Gmm::learn(&fs, &fg.iter().map(|&i| comp[i]).collect::<Vec<_>>());
            let bgm = Gmm::learn(&bs, &bg.iter().map(|&i| comp[i]).collect::<Vec<_>>());
            (fs.iter().map(|z| fgm.best(*z)).collect(), bs.iter().map(|z| bgm.best(*z)).collect())
        };
        for (k, &i) in fg.iter().enumerate() {
            comp[i] = fa[k];
        }
        for (k, &i) in bg.iter().enumerate() {
            comp[i] = ba[k];
        }
        let fgm = Gmm::learn(&fs, &fa);
        let bgm = Gmm::learn(&bs, &ba);
        let (s, t) = (n, n + 1);
        let mut f = Flow::new(n + 2, n * 4 + pairs.len() * 2);
        for i in 0..n {
            let (fs_, ts) = match labels[i] {
                FG => (lambda, 0.0),
                BG => (0.0, lambda),
                _ => (-(bgm.p(img[i]).max(1e-300)).ln(), -(fgm.p(img[i]).max(1e-300)).ln()),
            };
            f.add(s, i, fs_, 0.0);
            f.add(i, t, ts, 0.0);
        }
        for (k, &(a, b, _)) in pairs.iter().enumerate() {
            f.add(a, b, wts[k], wts[k]);
        }
        f.run(s, t);
        for i in 0..n {
            if labels[i] == PR_FG || labels[i] == PR_BG {
                labels[i] = if f.level[i] >= 0 { PR_FG } else { PR_BG };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grabcut_finds_a_square() {
        // bright square on a dark noisy background, box a bit larger than it
        let (w, h) = (40, 40);
        let mut img = vec![[0f64; 3]; w * h];
        let mut labels = vec![BG; w * h];
        for y in 0..h {
            for x in 0..w {
                let inside = (12..28).contains(&x) && (12..28).contains(&y);
                let n = ((x * 7 + y * 13) % 11) as f64;
                img[y * w + x] = if inside { [220.0 + n, 200.0, 40.0] } else { [30.0 + n, 40.0, 90.0] };
                if (6..34).contains(&x) && (6..34).contains(&y) {
                    labels[y * w + x] = PR_FG;
                }
            }
        }
        grabcut(&img, w, h, &mut labels, 4);
        let fg = |x: usize, y: usize| labels[y * w + x] == PR_FG;
        assert!(fg(20, 20) && fg(13, 13) && fg(26, 26));
        assert!(!fg(8, 8) && !fg(31, 20) && !fg(20, 32));
    }

    /// A two-colour picture of `w`×`h` cells: `inside(x, y)` red, else
    /// blue (about the same brightness, so only the colour tells them apart).
    fn picture(w: usize, h: usize, inside: impl Fn(usize, usize) -> bool) -> Vec<u8> {
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let c = if inside(x, y) { [200, 60, 60, 255] } else { [60, 92, 185, 255] };
                px[(y * w + x) * 4..][..4].copy_from_slice(&c);
            }
        }
        px
    }

    /// Φ, the standard normal distribution (Abramowitz–Stegun erf).
    fn phi(x: f32) -> f32 {
        let z = x / std::f32::consts::SQRT_2;
        let t = 1.0 / (1.0 + 0.3275911 * z.abs());
        let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * (-z * z).exp();
        0.5 * (1.0 + if z >= 0.0 { y } else { -y })
    }

    /// A red/blue edge along x = `x0` blurred by `sigma` (each pixel
    /// integrated over its width), 48×40 cells.
    fn blurred_edge(x0: f32, sigma: f32) -> Vec<u8> {
        let (w, h) = (48, 40);
        let s = (sigma * sigma + 1.0 / 12.0).sqrt();
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let a = phi((x as f32 + 0.5 - x0) / s);
                let (c0, c1) = ([200.0, 60.0, 60.0], [60.0, 92.0, 185.0]);
                for c in 0..3 {
                    px[(y * w + x) * 4 + c] = (c0[c] * (1.0 - a) + c1[c] * a).round() as u8;
                }
                px[(y * w + x) * 4 + 3] = 255;
            }
        }
        px
    }

    /// The edge's sub-pixel position and blur come back from the picture.
    #[test]
    fn edge_blur_is_measured() {
        for (x0, sigma) in [(20.3f32, 0.0f32), (20.75, 0.0), (20.3, 1.0), (21.1, 2.0), (20.5, 3.0)] {
            let map = EdgeMap::new(&blurred_edge(x0, sigma), 48, 40, egui::Pos2::ZERO, 1.0);
            let path: Vec<egui::Pos2> = (8..32).map(|y| egui::pos2(x0.floor() + 0.5, y as f32 + 0.5)).collect();
            let r = map.refine(&path);
            let mx = r.iter().map(|p| p.0.x).sum::<f32>() / r.len() as f32;
            let ms = r.iter().map(|p| p.1).sum::<f32>() / r.len() as f32;
            eprintln!("edge at {x0} blur {sigma}: found {mx:.3}, blur {ms:.3}");
            assert!((mx - x0).abs() < 0.12, "edge at {x0} (blur {sigma}) found at {mx}");
            assert!((ms - sigma).abs() < 0.35, "blur {sigma} measured {ms}");
        }
    }

    #[test]
    fn live_wire_follows_an_edge() {
        // left part red, right part blue: the edge is between cells 19 and 20
        // (world x 120)
        let (w, h) = (40, 40);
        let map = std::sync::Arc::new(EdgeMap::new(&picture(w, h, |x, _| x < 20), w, h, egui::pos2(100.0, 200.0), 1.0));
        let mut lw = LiveWire::new(map.clone(), egui::pos2(120.5, 202.5)).unwrap();
        lw.grow_all();
        let path = lw.path_to(egui::pos2(120.5, 237.5)).unwrap();
        assert_eq!(path.first(), Some(&egui::pos2(120.5, 202.5)));
        assert_eq!(path.last(), Some(&egui::pos2(120.5, 237.5)));
        assert!(path.iter().all(|p| (119.0..121.0).contains(&p.x)), "{path:?}");
        // a click a few cells off the edge snaps onto it
        let s = map.snap(egui::pos2(124.5, 220.5), 6.0);
        assert!((119.0..121.0).contains(&s.x), "{s:?}");
    }

    /// Times a full scissors map (1536², a noisy picture with a disc):
    /// building the edge map, a near target, the whole map
    /// (`cargo test --release scissor_speed -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn scissor_speed() {
        let n = 1536;
        let mut px = picture(n, n, |x, y| (x as f32 - 768.0).hypot(y as f32 - 768.0) < 500.0);
        let mut r = 12345u32;
        for v in px.iter_mut() {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            *v = v.saturating_add((r % 24) as u8);
        }
        let t = std::time::Instant::now();
        let map = std::sync::Arc::new(EdgeMap::new(&px, n, n, egui::Pos2::ZERO, 1.0));
        eprintln!("edge map {n}x{n}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let t = std::time::Instant::now();
        let mut lw = LiveWire::new(map, egui::pos2(1268.5, 768.5)).unwrap();
        eprintln!("wire alloc: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let t = std::time::Instant::now();
        lw.grow(Some(egui::pos2(1121.5, 1121.5)), std::time::Duration::MAX);
        eprintln!("to a point 1/8 round the rim: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let t = std::time::Instant::now();
        lw.grow_all();
        eprintln!("whole map: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    }

    #[test]
    fn live_wire_grows_on_demand() {
        let (w, h) = (200, 200);
        let map = std::sync::Arc::new(EdgeMap::new(&picture(w, h, |x, y| (x as f32 + 0.5 - 100.0).hypot(y as f32 + 0.5 - 100.0) < 60.0), w, h, egui::Pos2::ZERO, 1.0));
        let mut lw = LiveWire::new(map, egui::pos2(160.5, 100.5)).unwrap();
        // a near target is reached long before the map is done
        assert!(lw.grow(Some(egui::pos2(150.5, 130.5)), std::time::Duration::from_secs(5)));
        assert!(lw.path_to(egui::pos2(150.5, 130.5)).is_some());
        assert!(lw.path_to(egui::pos2(5.5, 5.5)).is_none(), "far cells are not reached yet");
        // half way round the circle: the path keeps to the rim
        lw.grow_all();
        let path = lw.path_to(egui::pos2(40.5, 100.5)).unwrap();
        for p in &path {
            let r = (p.x - 100.0).hypot(p.y - 100.0);
            assert!((r - 60.0).abs() < 2.0, "off the rim at {p:?} (r {r})");
        }
    }
}

/// Edge features of a picture for the scissors (Mortensen & Barrett 1995):
/// Laplacian zero-crossings, gradient magnitude and direction, on a grid of
/// `cell` world units from `origin` (the world point of cell (0,0)'s corner).
/// Gradients are taken per colour channel and the strongest kept (as GIMP),
/// after a light blur, so colour edges of equal brightness count too.
pub struct EdgeMap {
    pub origin: egui::Pos2,
    pub cell: f32,
    pub w: usize,
    pub h: usize,
    /// 0 on a zero-crossing of a real edge, 1 elsewhere.
    fz: Vec<u8>,
    /// 1 − G / max G.
    fg: Vec<f32>,
    /// Unit vector along the edge (the gradient turned a quarter), zero where
    /// there is no gradient.
    dir: Vec<[f32; 2]>,
    grad: Vec<f32>,
    /// The picture (unblurred) and, per cell, the channel with the strongest
    /// gradient: for the edge's blur.
    rgb: Vec<[u8; 3]>,
    chan: Vec<u8>,
}

/// `f(i)` for every i < n, on all cores (in order).
fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, |t| t.get()).min(16);
    let chunk = n.div_ceil(threads).max(4096);
    let mut parts: Vec<Vec<T>> = Vec::new();
    std::thread::scope(|sc| {
        let hs: Vec<_> = (0..n).step_by(chunk).map(|s| {
            let f = &f;
            sc.spawn(move || (s..(s + chunk).min(n)).map(f).collect::<Vec<T>>())
        }).collect();
        parts = hs.into_iter().map(|h| h.join().unwrap()).collect();
    });
    parts.into_iter().flatten().collect()
}

/// Cost weights: zero-crossing, gradient direction, gradient magnitude (the
/// paper's, also OpenCV's defaults).
const W_Z: f32 = 0.43;
const W_D: f32 = 0.43;
const W_G: f32 = 0.14;
/// Zero-crossings where the gradient is weaker than this share of the
/// strongest are noise, not edges.
const Z_MIN: f32 = 0.08;
/// What sampling a perfectly sharp edge adds to the measured spread (cells²):
/// the pixel's own width and the bilinear profile (measured on synthetic
/// edges, `edge_blur_is_measured`).
const SAMPLE_VAR: f32 = 0.375;

impl EdgeMap {
    /// `rgba`: the picture over `w`×`h` cells (premultiplied over black is fine).
    pub fn new(rgba: &[u8], w: usize, h: usize, origin: egui::Pos2, cell: f32) -> Self {
        let n = w * h;
        let at = |x: isize, y: isize| (y.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize;
        let xy = |i: usize| ((i % w) as isize, (i / w) as isize);
        // light blur (GIMP's 3×3: centre 24, the rest 1, over 32)
        let b: Vec<[f32; 3]> = par_map(n, |i| {
            let (x, y) = xy(i);
            let mut acc = [0f32; 3];
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let k = if dx == 0 && dy == 0 { 24.0 } else { 1.0 };
                    let j = at(x + dx, y + dy) * 4;
                    for c in 0..3 {
                        acc[c] += rgba[j + c] as f32 * k;
                    }
                }
            }
            acc.map(|v| v / (32.0 * 255.0))
        });
        // Sobel per channel, the strongest kept: (gx, gy, magnitude, channel)
        let sob: Vec<([f32; 2], f32, u8)> = par_map(n, |i| {
            let (x, y) = xy(i);
            let v = |dx: isize, dy: isize| b[at(x + dx, y + dy)];
            let (mut best, mut bg, mut bc) = (-1.0f32, [0f32; 2], 0u8);
            for c in 0..3 {
                let gx = v(1, -1)[c] + 2.0 * v(1, 0)[c] + v(1, 1)[c] - v(-1, -1)[c] - 2.0 * v(-1, 0)[c] - v(-1, 1)[c];
                let gy = v(-1, 1)[c] + 2.0 * v(0, 1)[c] + v(1, 1)[c] - v(-1, -1)[c] - 2.0 * v(0, -1)[c] - v(1, -1)[c];
                let m = gx * gx + gy * gy;
                if m > best {
                    best = m;
                    bg = [gx, gy];
                    bc = c as u8;
                }
            }
            (bg, best.sqrt(), bc)
        });
        let gmax = sob.iter().fold(1e-6f32, |m, s| m.max(s.1));
        // Laplacian per channel; at each cell the channel with the strongest
        // gradient decides (colour edges of equal brightness cross too), and
        // a crossing goes to the side nearer zero
        let lap: Vec<[f32; 3]> = par_map(n, |i| {
            let (x, y) = xy(i);
            let (l, r, u, d) = (b[at(x - 1, y)], b[at(x + 1, y)], b[at(x, y - 1)], b[at(x, y + 1)]);
            [0, 1, 2].map(|c| l[c] + r[c] + u[c] + d[c] - 4.0 * b[i][c])
        });
        let fz: Vec<u8> = par_map(n, |i| {
            if sob[i].1 < Z_MIN * gmax {
                return 1;
            }
            let c = sob[i].2 as usize;
            let (x, y) = xy(i);
            let cross = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| {
                let j = at(x + dx, y + dy);
                lap[i][c] * lap[j][c] < 0.0 && lap[i][c].abs() <= lap[j][c].abs()
            });
            if cross { 0 } else { 1 }
        });
        let grad: Vec<f32> = sob.iter().map(|s| s.1).collect();
        let fg = grad.iter().map(|g| 1.0 - g / gmax).collect();
        let dir = sob
            .iter()
            .map(|&([gx, gy], m, _)| if m > 1e-4 { [gy / m, -gx / m] } else { [0.0; 2] })
            .collect();
        let rgb = par_map(n, |i| [rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]]);
        let chan = sob.iter().map(|s| s.2).collect();
        Self { origin, cell, w, h, fz, fg, dir, grad, rgb, chan }
    }

    /// The cell under world point `p`.
    pub fn cell_of(&self, p: egui::Pos2) -> Option<usize> {
        let q = (p - self.origin) / self.cell;
        let (x, y) = (q.x.floor() as isize, q.y.floor() as isize);
        (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h).then(|| y as usize * self.w + x as usize)
    }

    /// The world point at the middle of cell `i`.
    pub fn centre(&self, i: usize) -> egui::Pos2 {
        self.origin + egui::vec2((i % self.w) as f32 + 0.5, (i / self.w) as f32 + 0.5) * self.cell
    }

    /// `p` lies at least `margin` cells inside the map.
    pub fn holds(&self, p: egui::Pos2, margin: f32) -> bool {
        let q = (p - self.origin) / self.cell;
        q.x >= margin && q.y >= margin && q.x <= self.w as f32 - margin && q.y <= self.h as f32 - margin
    }

    /// The strongest edge within `radius` cells of `p` (the nearest of equal
    /// ones), or `p` when there is none there.
    pub fn snap(&self, p: egui::Pos2, radius: f32) -> egui::Pos2 {
        let Some(c) = self.cell_of(p) else { return p };
        let (cx, cy) = ((c % self.w) as isize, (c / self.w) as isize);
        let r = radius.max(0.0).ceil() as isize;
        let (mut best, mut at) = (0.0f32, None);
        for y in (cy - r).max(0)..=(cy + r).min(self.h as isize - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(self.w as isize - 1) {
                let d2 = ((x - cx).pow(2) + (y - cy).pow(2)) as f32;
                if d2 > radius * radius {
                    continue;
                }
                let i = y as usize * self.w + x as usize;
                // edges proper (zero-crossings) first, then the strength
                let v = self.grad[i] * if self.fz[i] == 0 { 2.0 } else { 1.0 } - d2.sqrt() * 1e-4;
                if v > best {
                    best = v;
                    at = Some(i);
                }
            }
        }
        at.map_or(p, |i| self.centre(i))
    }

    /// Field `f` (one value per cell, at its middle) at continuous cell
    /// position `q`, bilinear.
    fn sample(&self, q: egui::Vec2, f: impl Fn(usize) -> f32) -> f32 {
        let x = (q.x - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (q.y - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| f(y * self.w + x);
        (at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx) * (1.0 - fy) + (at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx) * fy
    }

    /// Each point of a path (world, cell middles) moved across the edge to
    /// where the gradient peaks, with the edge's blur there: (point, σ in
    /// cells). The peak is a parabola through three gradient samples along
    /// the gradient (Devernay's sub-pixel edges); the blur is the spread of
    /// the brightness change across the edge (the strongest channel), less
    /// what sampling adds. Offsets and blurs are smoothed along the path.
    pub fn refine(&self, path: &[egui::Pos2]) -> Vec<(egui::Pos2, f32)> {
        let n = path.len();
        let mut off = vec![0f32; n];
        let mut sig = vec![f32::NAN; n];
        let mut nrm = vec![egui::Vec2::ZERO; n];
        for (k, p) in path.iter().enumerate() {
            let Some(i) = self.cell_of(*p) else { continue };
            let d = self.dir[i];
            if d == [0.0, 0.0] {
                continue;
            }
            // the gradient's direction (across the edge)
            let nv = egui::vec2(-d[1], d[0]);
            let q = (*p - self.origin) / self.cell;
            let g = |t: f32| self.sample(q + nv * t, |j| self.grad[j]);
            // climb to the nearest peak (within 1.5 cells), then the parabola
            let mut t0 = 0.0f32;
            for _ in 0..3 {
                if t0 < 1.5 && g(t0 + 0.5) > g(t0) {
                    t0 += 0.5;
                } else if t0 > -1.5 && g(t0 - 0.5) > g(t0) {
                    t0 -= 0.5;
                } else {
                    break;
                }
            }
            let (a, b, c) = (g(t0 - 1.0), g(t0), g(t0 + 1.0));
            let den = a - 2.0 * b + c;
            let t = if den < -1e-6 { t0 + 0.5 * (a - c) / den } else { t0 };
            off[k] = t.clamp(-2.0, 2.0);
            nrm[k] = nv;
            // the brightness change across the edge: differences of the
            // profile, the run around the largest one, its spread
            let ch = self.chan[i] as usize;
            let v = |t: f32| self.sample(q + nv * t, |j| self.rgb[j][ch] as f32);
            const R: i32 = 8;
            let prof: Vec<f32> = (-R..=R).map(|s| v(off[k] + s as f32)).collect();
            let diff: Vec<f32> = prof.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
            let (top, &dmax) = diff.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
            if dmax < 4.0 {
                continue;
            }
            let (mut lo, mut hi) = (top, top);
            while lo > 0 && diff[lo - 1] <= diff[lo] && diff[lo - 1] > dmax * 0.02 {
                lo -= 1;
            }
            while hi + 1 < diff.len() && diff[hi + 1] <= diff[hi] && diff[hi + 1] > dmax * 0.02 {
                hi += 1;
            }
            let run = &diff[lo..=hi];
            let sum: f32 = run.iter().sum();
            let mid = |j: usize| (lo + j) as f32;
            let mean = run.iter().enumerate().map(|(j, d)| d * mid(j)).sum::<f32>() / sum;
            let var = run.iter().enumerate().map(|(j, d)| d * (mid(j) - mean).powi(2)).sum::<f32>() / sum;
            sig[k] = (var - SAMPLE_VAR).max(0.0).sqrt();
        }
        // smoothed along the path: offsets [1 2 1], blurs over 9 points
        let off_s: Vec<f32> = (0..n)
            .map(|k| {
                if n < 3 || k == 0 || k + 1 == n {
                    off[k]
                } else {
                    (off[k - 1] + 2.0 * off[k] + off[k + 1]) / 4.0
                }
            })
            .collect();
        (0..n)
            .map(|k| {
                let near: Vec<f32> = (k.saturating_sub(4)..(k + 5).min(n)).map(|j| sig[j]).filter(|s| s.is_finite()).collect();
                let s = if near.is_empty() { 0.0 } else { near.iter().sum::<f32>() / near.len() as f32 };
                (path[k] + nrm[k] * off_s[k] * self.cell, s)
            })
            .collect()
    }

    /// Cost of the link p -> q (neighbouring cells, `l` the unit step;
    /// `diag` when diagonal).
    fn link(&self, p: usize, q: usize, l: [f32; 2], diag: bool) -> f32 {
        let (dp, dq) = (self.dir[p], self.dir[q]);
        // the link taken along the edge direction at p
        let s = if dp[0] * l[0] + dp[1] * l[1] >= 0.0 { 1.0 } else { -1.0 };
        let l = [l[0] * s, l[1] * s];
        let a = (dp[0] * l[0] + dp[1] * l[1]).clamp(-1.0, 1.0);
        let b = (l[0] * dq[0] + l[1] * dq[1]).clamp(-1.0, 1.0);
        let fd = (a.acos() + b.acos()) * (2.0 / (3.0 * std::f32::consts::PI));
        // the gradient term is 1 for diagonal links, 1/√2 for straight ones
        let g = if diag { 1.0 } else { std::f32::consts::FRAC_1_SQRT_2 };
        W_Z * self.fz[q] as f32 + W_D * fd + W_G * self.fg[q] * g + 0.01
    }
}

/// Intelligent scissors (Mortensen & Barrett 1995): Dijkstra from the seed
/// over an edge map, grown on demand until the pointer's cell is reached (a
/// time budget per call), so the path to any reached cell is a lookup.
pub struct LiveWire {
    pub map: std::sync::Arc<EdgeMap>,
    dist: Vec<f32>,
    pred: Vec<u32>,
    done: Vec<bool>,
    heap: std::collections::BinaryHeap<std::cmp::Reverse<(u32, u32)>>,
}

impl LiveWire {
    /// Seeded at world point `at` (None outside the map).
    pub fn new(map: std::sync::Arc<EdgeMap>, at: egui::Pos2) -> Option<Self> {
        let seed = map.cell_of(at)?;
        let n = map.w * map.h;
        let mut dist = vec![f32::INFINITY; n];
        dist[seed] = 0.0;
        let mut heap = std::collections::BinaryHeap::new();
        heap.push(std::cmp::Reverse(((0.0f32).to_bits(), seed as u32)));
        Some(Self { map, dist, pred: vec![u32::MAX; n], done: vec![false; n], heap })
    }

    /// Settles cells, cheapest first, until `target`'s cell is settled, the
    /// map is done or `budget` runs out. True when the target is reached.
    pub fn grow(&mut self, target: Option<egui::Pos2>, budget: std::time::Duration) -> bool {
        use std::cmp::Reverse;
        let goal = target.and_then(|p| self.map.cell_of(p));
        if goal.is_some_and(|g| self.done[g]) {
            return true;
        }
        let t0 = std::time::Instant::now();
        let (w, h) = (self.map.w as isize, self.map.h as isize);
        let mut k = 0u32;
        while let Some(Reverse((db, u))) = self.heap.pop() {
            let u = u as usize;
            if self.done[u] {
                continue;
            }
            self.done[u] = true;
            let du = f32::from_bits(db);
            let (ux, uy) = ((u as isize) % w, (u as isize) / w);
            for (dx, dy) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)] {
                let (vx, vy) = (ux + dx, uy + dy);
                if vx < 0 || vy < 0 || vx >= w || vy >= h {
                    continue;
                }
                let v = (vy * w + vx) as usize;
                if self.done[v] {
                    continue;
                }
                let diag = dx != 0 && dy != 0;
                let len = if diag { std::f32::consts::SQRT_2 } else { 1.0 };
                let nd = du + self.map.link(u, v, [dx as f32 / len, dy as f32 / len], diag);
                if nd < self.dist[v] {
                    self.dist[v] = nd;
                    self.pred[v] = u as u32;
                    // non-negative f32 bits order like the values
                    self.heap.push(Reverse((nd.to_bits(), v as u32)));
                }
            }
            if Some(u) == goal {
                return true;
            }
            k += 1;
            if k % 1024 == 0 && t0.elapsed() >= budget {
                return false;
            }
        }
        goal.is_some_and(|g| self.done[g])
    }

    /// Grows over the whole map.
    #[cfg(test)]
    pub fn grow_all(&mut self) {
        self.grow(None, std::time::Duration::MAX);
    }

    /// World points (cell middles) from the seed to the cell under `p`, or
    /// None when that cell is outside the map or not reached yet.
    pub fn path_to(&self, p: egui::Pos2) -> Option<Vec<egui::Pos2>> {
        let mut v = self.map.cell_of(p)?;
        if !self.done[v] {
            return None;
        }
        let mut out = vec![];
        while v != u32::MAX as usize && out.len() <= self.dist.len() {
            out.push(self.map.centre(v));
            v = self.pred[v] as usize;
        }
        out.reverse();
        Some(out)
    }
}
