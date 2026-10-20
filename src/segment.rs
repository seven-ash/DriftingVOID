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
/// A box drawn across an edge (Ash, 10-21: the larger side is hidden). An edge
/// running through the box cuts its border into two arcs of different colour:
/// the two cut points are the pair that best separates the border's colours
/// (between-arc share of the border's colour variance). When the two arcs hold
/// different colours (little histogram overlap, not one pattern all round)
/// and each holds a fifth of the border or more, and the inside holds no
/// colours the border lacks (an enclosed object), the arcs seed a
/// graph cut of the inside (one certain object, one certain background, the
/// colour models learn the rest) and the larger part is returned (true:
/// hidden). Else None: an object inside the box, plain GrabCut. `hint`: cells
/// painted already, kept with the first arc's side.
pub fn split_box(img: &[[f64; 3]], w: usize, h: usize, hint: &[bool]) -> Option<Vec<bool>> {
    if w < 4 || h < 4 {
        return None;
    }
    let n = w * h;
    // the border, clockwise from the top left
    let mut ring: Vec<usize> = (0..w).collect();
    ring.extend((1..h).map(|y| y * w + w - 1));
    ring.extend((0..w - 1).rev().map(|x| (h - 1) * w + x));
    ring.extend((1..h - 1).rev().map(|y| y * w));
    let p = ring.len();
    let mut pre = vec![[0f64; 3]; p + 1];
    for (k, &i) in ring.iter().enumerate() {
        for c in 0..3 {
            pre[k + 1][c] = pre[k][c] + img[i][c];
        }
    }
    let total = pre[p];
    let mean = total.map(|v| v / p as f64);
    let ss: f64 = ring.iter().map(|&i| (0..3).map(|c| (img[i][c] - mean[c]).powi(2)).sum::<f64>()).sum();
    if ss <= 0.0 {
        return None;
    }
    let min_arc = p / 5;
    let mut best = (0.0, 0, 0);
    for a in 0..p {
        for b in a + min_arc..=(a + p - min_arc).min(p) {
            let (na, nb) = ((b - a) as f64, (p - (b - a)) as f64);
            let sa: [f64; 3] = std::array::from_fn(|c| pre[b][c] - pre[a][c]);
            let d: f64 = (0..3).map(|c| (sa[c] / na - (total[c] - sa[c]) / nb).powi(2)).sum();
            let between = na * nb / p as f64 * d;
            if between > best.0 {
                best = (between, a, b);
            }
        }
    }
    let (between, a, b) = best;
    if between / ss < 0.2 {
        return None;
    }
    // the two arcs must hold different colours, not the same mix (a pattern
    // all round an object): overlap of their colour histograms (8 levels a
    // channel)
    let bin = |i: usize| img[i].iter().fold(0usize, |m, v| m * 8 + (v.clamp(0.0, 255.0) as usize >> 5));
    let (mut ha, mut hb) = (vec![0f64; 512], vec![0f64; 512]);
    for (k, &i) in ring.iter().enumerate() {
        if (a..b).contains(&k) { ha[bin(i)] += 1.0 } else { hb[bin(i)] += 1.0 }
    }
    let (na, nb) = ((b - a) as f64, (p - (b - a)) as f64);
    let overlap: f64 = ha.iter().zip(&hb).map(|(x, y)| (x / na).min(y / nb)).sum();
    if overlap > 0.25 {
        return None;
    }
    // colours inside that the border doesn't hold (a bin away) are an object
    // the box encloses: GrabCut's case, whatever runs behind it
    let mut seen = vec![false; 512];
    for (q, (x, y)) in ha.iter().zip(&hb).enumerate() {
        if x + y >= 2.0 {
            let (r, g, bb) = ((q / 64) as i32, (q / 8 % 8) as i32, (q % 8) as i32);
            for dr in -1..=1 {
                for dg in -1..=1 {
                    for db in -1..=1 {
                        let (r2, g2, b2) = (r + dr, g + dg, bb + db);
                        if (0..8).contains(&r2) && (0..8).contains(&g2) && (0..8).contains(&b2) {
                            seen[(r2 * 64 + g2 * 8 + b2) as usize] = true;
                        }
                    }
                }
            }
        }
    }
    if (0..n).filter(|&i| !seen[bin(i)]).count() * 20 > n {
        return None;
    }
    let mut labels = vec![PR_BG; n];
    let arc_a: Vec<usize> = ring[a..b].to_vec();
    let ma = arc_a.iter().fold([0f64; 3], |m, &i| std::array::from_fn(|c| m[c] + img[i][c] / arc_a.len() as f64));
    let mb: [f64; 3] = std::array::from_fn(|c| (total[c] - ma[c] * arc_a.len() as f64) / (p - arc_a.len()) as f64);
    let d2 = |x: &[f64; 3], m: &[f64; 3]| (0..3).map(|c| (x[c] - m[c]).powi(2)).sum::<f64>();
    for i in 0..n {
        labels[i] = if hint[i] { FG } else if d2(&img[i], &ma) < d2(&img[i], &mb) { PR_FG } else { PR_BG };
    }
    for (k, &i) in ring.iter().enumerate() {
        labels[i] = if (a..b).contains(&k) { FG } else { BG };
    }
    grabcut(img, w, h, &mut labels, 4);
    let part: Vec<bool> = labels.iter().map(|&l| l == FG || l == PR_FG).collect();
    let inside = part.iter().filter(|&&q| q).count();
    Some(if inside * 2 >= n { part } else { part.iter().map(|&q| !q).collect() })
}

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
    fn box_across_an_edge_takes_the_larger_side() {
        // a box over a diagonal edge: the dark side holds two thirds of it
        let (w, h) = (30, 30);
        let img: Vec<[f64; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let n = ((x * 7 + y * 13) % 9) as f64;
                if x + y < 40 { [40.0 + n, 50.0, 70.0] } else { [200.0 + n, 190.0, 170.0] }
            })
            .collect();
        let m = split_box(&img, w, h, &vec![false; w * h]).expect("crossing");
        assert!(m[0] && m[5 * w + 5] && !m[29 * w + 29], "the dark (larger) side");
        // an object inside the box is not a crossing
        let img2: Vec<[f64; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if (10..20).contains(&x) && (10..20).contains(&y) { [220.0, 200.0, 40.0] } else { [30.0, 40.0, 90.0] }
            })
            .collect();
        assert!(split_box(&img2, w, h, &vec![false; w * h]).is_none());
        // an object on a checkerboard, the box a little inside its top: not a
        // crossing by colour alone (the border alternates), and an object
        // crossing a checkered floor's edge keeps the floor's two colours together
        let img3: Vec<[f64; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if x < 12 { [200.0, 110.0, 80.0] } else if (x / 4 + y / 4) % 2 == 0 { [60.0, 60.0, 70.0] } else { [160.0, 160.0, 170.0] }
            })
            .collect();
        // all checker (an object well inside): no crossing
        let img4: Vec<[f64; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if (10..20).contains(&x) && (10..20).contains(&y) { [200.0, 110.0, 80.0] } else if (x / 4 + y / 4) % 2 == 0 { [60.0, 60.0, 70.0] } else { [160.0, 160.0, 170.0] }
            })
            .collect();
        assert!(split_box(&img4, w, h, &vec![false; w * h]).is_none(), "checker");
        let m = split_box(&img3, w, h, &vec![false; w * h]).expect("crossing");
        // the floor (18 of 30 columns) is the larger side, light and dark tiles alike
        assert!(m[15 * w + 20] && m[15 * w + 25] && m[2 * w + 14] && !m[15 * w + 5]);
    }

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

    #[test]
    fn live_wire_follows_an_edge() {
        // dark left half, bright right half: the edge is between x=19 and x=20
        let (w, h) = (40, 40);
        let lum: Vec<f32> = (0..w * h).map(|i| if i % w < 20 { 0.1 } else { 0.9 }).collect();
        let lw = LiveWire::new(&lum, w, h, (100, 200), (20, 2));
        let path = lw.path_to(120, 237).unwrap();
        assert_eq!(path.first(), Some(&(120, 202)));
        assert_eq!(path.last(), Some(&(120, 237)));
        // a straight line would also be x=20; check it never wanders off the edge
        assert!(path.iter().all(|&(x, _)| (119..=120).contains(&x)), "{path:?}");
        let diag = lw.path_to(119, 30 + 200).unwrap();
        assert!(diag.iter().all(|&(x, _)| (119..=120).contains(&x)));
    }
}

/// Intelligent scissors (Mortensen & Barrett 1995), gradient cost only: one
/// Dijkstra from the seed over a window, so the path to any point is a lookup.
pub struct LiveWire {
    /// World pixel of the window's (0,0).
    pub origin: (i32, i32),
    w: usize,
    h: usize,
    pred: Vec<u32>,
}

impl LiveWire {
    /// `lum`: luminance 0..1 of the window; `seed`: window pixel.
    pub fn new(lum: &[f32], w: usize, h: usize, origin: (i32, i32), seed: (usize, usize)) -> Self {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let n = w * h;
        // Sobel gradient magnitude -> cost (low on strong edges)
        let at = |x: isize, y: isize| lum[(y.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize];
        let mut g = vec![0f32; n];
        let mut gmax = 1e-6f32;
        for y in 0..h as isize {
            for x in 0..w as isize {
                let gx = at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x - 1, y) - at(x - 1, y + 1);
                let gy = at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1);
                let m = (gx * gx + gy * gy).sqrt();
                g[y as usize * w + x as usize] = m;
                gmax = gmax.max(m);
            }
        }
        let cost: Vec<f32> = g.iter().map(|m| 1.0 - m / gmax + 0.02).collect();
        let mut dist = vec![f32::INFINITY; n];
        let mut pred = vec![u32::MAX; n];
        let s = seed.1.min(h - 1) * w + seed.0.min(w - 1);
        dist[s] = 0.0;
        let mut heap = BinaryHeap::new();
        heap.push(Reverse(((0.0f32).to_bits(), s as u32)));
        while let Some(Reverse((db, u))) = heap.pop() {
            let u = u as usize;
            let du = f32::from_bits(db);
            if du > dist[u] {
                continue;
            }
            let (ux, uy) = ((u % w) as isize, (u / w) as isize);
            for (dx, dy, k) in [(1, 0, 1.0), (-1, 0, 1.0), (0, 1, 1.0), (0, -1, 1.0), (1, 1, 1.414), (-1, 1, 1.414), (1, -1, 1.414), (-1, -1, 1.414)] {
                let (vx, vy) = (ux + dx, uy + dy);
                if vx < 0 || vy < 0 || vx >= w as isize || vy >= h as isize {
                    continue;
                }
                let v = vy as usize * w + vx as usize;
                let nd = du + cost[v] * k as f32;
                if nd < dist[v] {
                    dist[v] = nd;
                    pred[v] = u as u32;
                    // non-negative f32 bits order like the values
                    heap.push(Reverse((nd.to_bits(), v as u32)));
                }
            }
        }
        Self { origin, w, h, pred }
    }

    /// World pixels from the seed to (x, y), or None outside the window.
    pub fn path_to(&self, x: i32, y: i32) -> Option<Vec<(i32, i32)>> {
        let (lx, ly) = (x - self.origin.0, y - self.origin.1);
        if lx < 0 || ly < 0 || lx >= self.w as i32 || ly >= self.h as i32 {
            return None;
        }
        let mut v = ly as usize * self.w + lx as usize;
        let mut out = vec![];
        while v != u32::MAX as usize && out.len() <= self.w * self.h {
            out.push(((v % self.w) as i32 + self.origin.0, (v / self.w) as i32 + self.origin.1));
            v = self.pred[v] as usize;
        }
        out.reverse();
        Some(out)
    }
}
