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
}
