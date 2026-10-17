//! Perspective spaces: a camera matched to a photo from hand-drawn vanishing lines.
//! Maths and references: docs/SPACES.md.
//!
//! World is Blender's (Z up, right-handed); camera axes are x right, y down, z forward,
//! and `Xc = R (Xw - C)`. Image coordinates are canvas world pixels.

use egui::{Pos2, Rect, pos2};
use serde::{Deserialize, Serialize};

pub type V3 = [f64; 3];
/// Row-major 3×3.
pub type M3 = [[f64; 3]; 3];

pub const AXIS_NAMES: [&str; 3] = ["X", "Y", "Z"];
/// Blender-like axis colours.
pub const AXIS_COLORS: [egui::Color32; 3] =
    [egui::Color32::from_rgb(240, 72, 90), egui::Color32::from_rgb(132, 204, 40), egui::Color32::from_rgb(64, 144, 255)];

// ---------------------------------------------------------------- small linear algebra

pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

pub fn normalize(a: V3) -> V3 {
    let n = norm(a);
    if n > 0.0 { scale(a, 1.0 / n) } else { a }
}

pub fn mul_mv(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

pub fn transpose(m: &M3) -> M3 {
    [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]]
}

pub fn mul_mm(a: &M3, b: &M3) -> M3 {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    o
}

/// Rotation matrix exp([w]×).
pub fn rodrigues(w: V3) -> M3 {
    let th = norm(w);
    if th < 1e-12 {
        return [[1.0, -w[2], w[1]], [w[2], 1.0, -w[0]], [-w[1], w[0], 1.0]];
    }
    let k = scale(w, 1.0 / th);
    let (s, c) = th.sin_cos();
    let v = 1.0 - c;
    [
        [c + k[0] * k[0] * v, k[0] * k[1] * v - k[2] * s, k[0] * k[2] * v + k[1] * s],
        [k[1] * k[0] * v + k[2] * s, c + k[1] * k[1] * v, k[1] * k[2] * v - k[0] * s],
        [k[2] * k[0] * v - k[1] * s, k[2] * k[1] * v + k[0] * s, c + k[2] * k[2] * v],
    ]
}

/// Eigenvector of the smallest eigenvalue of a symmetric 3×3 (cyclic Jacobi).
fn smallest_eigvec(mut a: M3) -> V3 {
    let mut v: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off < 1e-15 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            // A' = Jᵀ A J
            for k in 0..3 {
                let akp = a[k][p];
                let akq = a[k][q];
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let apk = a[p][k];
                let aqk = a[q][k];
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for k in 0..3 {
                let vkp = v[k][p];
                let vkq = v[k][q];
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }
    let i = (0..3).min_by(|&x, &y| a[x][x].partial_cmp(&a[y][y]).unwrap()).unwrap();
    normalize([v[0][i], v[1][i], v[2][i]])
}

/// Solves `A x = b` (n ≤ 8) by Gaussian elimination with partial pivoting.
fn solve_lin(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| a[i][col].abs().partial_cmp(&a[j][col].abs()).unwrap())?;
        if a[piv][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in col + 1..n {
            let f = a[r][col] / a[col][col];
            for k in col..n {
                a[r][k] -= f * a[col][k];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| a[r][k] * x[k]).sum();
        x[r] = (b[r] - s) / a[r][r];
    }
    Some(x)
}

// ---------------------------------------------------------------- document data

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VLine {
    pub a: Pos2,
    pub b: Pos2,
    /// 0 = X, 1 = Y, 2 = Z (up).
    pub axis: u8,
}

fn yes() -> bool {
    true
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub id: u64,
    pub name: String,
    /// The matched image area; its centre is the principal point unless estimated.
    pub frame: Rect,
    pub lines: Vec<VLine>,
    /// Where world (0,0,0) appears.
    pub origin: Pos2,
    /// Estimate the principal point too (needs all three axes).
    #[serde(default)]
    pub free_pp: bool,
    #[serde(default = "yes")]
    pub grid: bool,
}

impl Space {
    pub fn new(id: u64, name: String, frame: Rect) -> Self {
        let origin = pos2(frame.center().x, frame.top() + frame.height() * 0.7);
        Self { id, name, frame, lines: Vec::new(), origin, free_pp: false, grid: true }
    }

    pub fn count(&self, axis: u8) -> usize {
        self.lines.iter().filter(|l| l.axis == axis && l.a.distance(l.b) >= 2.0).count()
    }
}

// ---------------------------------------------------------------- the camera

#[derive(Clone, Debug)]
#[allow(dead_code)] // `frame`, `project`, `ray` and `unproject_plane` serve placement (phase 2)
pub struct Camera {
    /// Focal length in canvas pixels.
    pub f: f64,
    /// Principal point (canvas pixels).
    pub c: [f64; 2],
    /// World → camera rotation.
    pub r: M3,
    /// Camera centre in world units (1 unit = 1 px at the origin).
    pub center: V3,
    /// Long side of the frame in pixels (for the 35 mm equivalent).
    pub long_side: f64,
    pub frame: Rect,
    /// Per-line angular error, degrees (same order as `Space::lines`; NaN when unused).
    pub line_err: Vec<f64>,
    pub rms_deg: f64,
    /// True when f came from the 50 mm prior (axes did not constrain it).
    pub f_assumed: bool,
}

#[allow(dead_code)]
impl Camera {
    pub fn to_cam(&self, w: V3) -> V3 {
        mul_mv(&self.r, sub(w, self.center))
    }

    pub fn project_cam(&self, x: V3) -> Option<Pos2> {
        if x[2] <= 1e-9 {
            return None;
        }
        Some(pos2((self.f * x[0] / x[2] + self.c[0]) as f32, (self.f * x[1] / x[2] + self.c[1]) as f32))
    }

    /// Camera depth of the world origin (1 world unit = 1 px there).
    pub fn origin_depth(&self) -> f64 {
        self.to_cam([0.0; 3])[2]
    }

    pub fn project(&self, w: V3) -> Option<Pos2> {
        self.project_cam(self.to_cam(w))
    }

    /// Camera-space direction of a world axis (column of R).
    pub fn axis_dir(&self, axis: usize) -> V3 {
        [self.r[0][axis], self.r[1][axis], self.r[2][axis]]
    }

    /// Vanishing point of a world axis; None when it is at infinity.
    pub fn vp(&self, axis: usize) -> Option<Pos2> {
        let d = self.axis_dir(axis);
        if d[2].abs() < 1e-9 {
            return None;
        }
        let d = if d[2] < 0.0 { scale(d, -1.0) } else { d };
        self.project_cam(d)
    }

    pub fn focal_35mm(&self) -> f64 {
        self.f / self.long_side * 36.0
    }

    /// Field of view across the long side, degrees.
    pub fn fov_deg(&self) -> f64 {
        2.0 * (self.long_side * 0.5 / self.f).atan().to_degrees()
    }

    /// Clips a world segment to the front of the camera and projects it.
    pub fn project_segment(&self, a: V3, b: V3) -> Option<(Pos2, Pos2)> {
        let near = 1e-3 * self.f;
        let (mut ca, mut cb) = (self.to_cam(a), self.to_cam(b));
        if ca[2] < near && cb[2] < near {
            return None;
        }
        if ca[2] < near {
            let t = (near - ca[2]) / (cb[2] - ca[2]);
            ca = add(ca, scale(sub(cb, ca), t));
        } else if cb[2] < near {
            let t = (near - cb[2]) / (ca[2] - cb[2]);
            cb = add(cb, scale(sub(ca, cb), t));
        }
        Some((self.project_cam(ca)?, self.project_cam(cb)?))
    }

    /// Ray through an image point, in world space (origin = camera centre).
    pub fn ray(&self, p: Pos2) -> V3 {
        let d = [(p.x as f64 - self.c[0]) / self.f, (p.y as f64 - self.c[1]) / self.f, 1.0];
        normalize(mul_mv(&transpose(&self.r), d))
    }

    /// Where an image point meets the world plane `axis = 0` (e.g. 2 → ground).
    pub fn unproject_plane(&self, p: Pos2, axis: usize) -> Option<V3> {
        let d = self.ray(p);
        if d[axis].abs() < 1e-12 {
            return None;
        }
        let t = -self.center[axis] / d[axis];
        (t > 0.0).then(|| add(self.center, scale(d, t)))
    }
}

/// Default focal length: 50 mm equivalent on the long side.
const DEFAULT_F_35: f64 = 50.0;
/// Huber threshold on the per-line residual (sine of the angle).
const HUBER: f64 = 0.0087; // ≈ 0.5°
/// Tukey cut-off: lines further off than this stop counting.
const TUKEY: f64 = 0.035; // ≈ 2°

pub fn solve(space: &Space) -> Result<Camera, String> {
    let fr = space.frame;
    if fr.width() < 4.0 || fr.height() < 4.0 {
        return Err("공간 영역이 너무 작아".into());
    }
    let counts = [space.count(0), space.count(1), space.count(2)];
    let usable: Vec<usize> = (0..3).filter(|&a| counts[a] >= 2).collect();
    if usable.len() < 2 {
        return Err(format!(
            "두 축 이상에 선을 2개씩 그려 줘 (최소 4개) · 지금 X {} · Y {} · Z {}",
            counts[0], counts[1], counts[2]
        ));
    }

    // normalised image coordinates: centre of the frame, half the long side = 1
    let s = (fr.width().max(fr.height()) as f64) * 0.5;
    let (ox, oy) = (fr.center().x as f64, fr.center().y as f64);
    let nrm = |p: Pos2| [(p.x as f64 - ox) / s, (p.y as f64 - oy) / s];

    struct L {
        idx: usize,
        a: [f64; 2],
        b: [f64; 2],
        axis: usize,
        w: f64,
    }
    let mut ls: Vec<L> = space
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.a.distance(l.b) >= 2.0 && l.axis < 3)
        .map(|(i, l)| L { idx: i, a: nrm(l.a), b: nrm(l.b), axis: l.axis as usize, w: l.a.distance(l.b) as f64 })
        .collect();
    let mean_w = ls.iter().map(|l| l.w).sum::<f64>() / ls.len() as f64;
    for l in &mut ls {
        l.w /= mean_w;
    }

    // 1. per-axis VPs (homogeneous, unit length): the pair intersection most lines
    // agree with picks the inliers, then weighted least squares over those
    let hline = |l: &L| normalize(cross([l.a[0], l.a[1], 1.0], [l.b[0], l.b[1], 1.0]));
    // sine of the angle between a segment and the direction from its midpoint to v
    let vp_err = |l: &L, v: V3| {
        let m = [(l.a[0] + l.b[0]) * 0.5, (l.a[1] + l.b[1]) * 0.5, 1.0];
        let lv = cross(m, v);
        let h = hline(l);
        let (n1, n2) = ([h[0], h[1]], [lv[0], lv[1]]);
        let d = (n1[0] * n1[0] + n1[1] * n1[1]).sqrt() * (n2[0] * n2[0] + n2[1] * n2[1]).sqrt();
        if d < 1e-15 { 0.0 } else { ((n1[0] * n2[1] - n1[1] * n2[0]) / d).abs() }
    };
    let mut vp: [Option<V3>; 3] = [None; 3];
    for &ax in &usable {
        let al: Vec<&L> = ls.iter().filter(|l| l.axis == ax).collect();
        let mut best = (f64::MIN, None);
        for i in 0..al.len() {
            for j in i + 1..al.len() {
                let v = normalize(cross(hline(al[i]), hline(al[j])));
                if norm(v) < 0.5 {
                    continue;
                }
                let score: f64 = al.iter().filter(|l| vp_err(l, v) < 0.035).map(|l| l.w).sum();
                if score > best.0 {
                    best = (score, Some(v));
                }
            }
        }
        let inl: Vec<&&L> = match best.1 {
            Some(v) => al.iter().filter(|l| vp_err(l, v) < 0.035).collect(),
            None => al.iter().collect(),
        };
        let inl = if inl.len() >= 2 { inl } else { al.iter().collect() };
        let mut m = [[0.0; 3]; 3];
        for l in inl {
            let h = hline(l);
            for i in 0..3 {
                for j in 0..3 {
                    m[i][j] += l.w * h[i] * h[j];
                }
            }
        }
        vp[ax] = Some(smallest_eigvec(m));
    }

    // 2. focal length from orthogonal VP pairs (principal point at the centre)
    let f_default = DEFAULT_F_35 / 18.0; // half the long side is 18 mm
    let mut logs = Vec::new();
    for i in 0..usable.len() {
        for j in i + 1..usable.len() {
            let (a, b) = (vp[usable[i]].unwrap(), vp[usable[j]].unwrap());
            if a[2].abs() < 1e-6 || b[2].abs() < 1e-6 {
                continue;
            }
            let f2 = -(a[0] * b[0] + a[1] * b[1]) / (a[2] * b[2]);
            if f2.is_finite() && f2 > 0.0 {
                let f = f2.sqrt();
                if (0.05..50.0).contains(&f) {
                    logs.push(f.ln());
                }
            }
        }
    }
    let f_assumed = logs.is_empty();
    logs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let f0 = if f_assumed { f_default } else { logs[logs.len() / 2].exp() };

    // initial rotation: axis directions from the VPs, signs fixed so Z is up and X right
    let dir = |v: V3, f: f64| normalize([v[0], v[1], v[2] * f]);
    let mut d: [Option<V3>; 3] = [None; 3];
    for &ax in &usable {
        d[ax] = Some(dir(vp[ax].unwrap(), f0));
    }
    let flip = |v: V3| scale(v, -1.0);
    if let Some(z) = d[2] {
        if z[1] > 0.0 {
            d[2] = Some(flip(z));
        }
    }
    if let Some(x) = d[0] {
        if x[0] < 0.0 {
            d[0] = Some(flip(x));
        }
    }
    match (d[0], d[1], d[2]) {
        (Some(x), Some(y), Some(z)) => {
            if dot(cross(x, y), z) < 0.0 {
                d[1] = Some(flip(y));
            }
        }
        (None, Some(y), Some(z)) => {
            if cross(y, z)[0] < 0.0 {
                d[1] = Some(flip(y));
            }
        }
        (Some(x), Some(y), None) => {
            if cross(x, y)[1] > 0.0 {
                d[1] = Some(flip(y));
            }
        }
        _ => {}
    }
    // orthonormal basis from the first two known axes
    let (p, q) = (usable[0], usable[1]);
    let c1 = d[p].unwrap();
    let c2 = normalize(sub(d[q].unwrap(), scale(c1, dot(d[q].unwrap(), c1))));
    let mut cols = [[0.0; 3]; 3];
    cols[p] = c1;
    cols[q] = c2;
    let third = 3 - p - q;
    cols[third] = match third {
        0 => cross(cols[1], cols[2]),
        1 => cross(cols[2], cols[0]),
        _ => cross(cols[0], cols[1]),
    };
    let r0: M3 = [
        [cols[0][0], cols[1][0], cols[2][0]],
        [cols[0][1], cols[1][1], cols[2][1]],
        [cols[0][2], cols[1][2], cols[2][2]],
    ];

    // 3. joint weighted fit (LM over log f, rotation increment, optional principal point)
    let free_pp = space.free_pp && usable.len() == 3;
    let np = if free_pp { 6 } else { 4 };
    let unpack = |x: &[f64]| -> (f64, [f64; 2], M3) {
        let f = x[0].exp();
        let c = if free_pp { [x[4], x[5]] } else { [0.0, 0.0] };
        (f, c, mul_mm(&rodrigues([x[1], x[2], x[3]]), &r0))
    };
    let line_res = |f: f64, c: [f64; 2], r: &M3, l: &L| -> f64 {
        let ray = |p: [f64; 2]| [(p[0] - c[0]) / f, (p[1] - c[1]) / f, 1.0];
        let n = normalize(cross(ray(l.a), ray(l.b)));
        dot(n, [r[0][l.axis], r[1][l.axis], r[2][l.axis]])
    };
    let mut huber = vec![1.0; ls.len()];
    let residuals = |x: &[f64], huber: &[f64]| -> Vec<f64> {
        let (f, c, r) = unpack(x);
        let mut out: Vec<f64> = ls.iter().zip(huber).map(|(l, h)| (l.w * h).sqrt() * line_res(f, c, &r, l)).collect();
        // weak priors: keep f and c defined where the lines don't fix them
        out.push(1e-3 * (x[0] - f_default.ln()));
        if free_pp {
            out.push(1e-2 * x[4]);
            out.push(1e-2 * x[5]);
        }
        out
    };
    let mut x = vec![0.0; np];
    x[0] = f0.ln();
    let in_bounds = |x: &[f64]| (0.05f64.ln()..50f64.ln()).contains(&x[0]) && (!free_pp || x[4].abs() < 1.0 && x[5].abs() < 1.0);
    // Huber first (convex, safe from a rough start), then Tukey's biweight, which
    // drops clear outliers entirely instead of only damping them
    let set_huber = |x: &[f64], huber: &mut Vec<f64>, tukey: bool| {
        let (f, c, rr) = unpack(x);
        for (h, l) in huber.iter_mut().zip(&ls) {
            let e = line_res(f, c, &rr, l).abs();
            *h = if tukey {
                let u = e / TUKEY;
                if u < 1.0 { (1.0 - u * u).powi(2) } else { 0.0 }
            } else if e <= HUBER {
                1.0
            } else {
                HUBER / e
            };
        }
        // never let a whole axis vanish
        if huber.iter().zip(&ls).filter(|(h, _)| **h > 0.0).count() < 4 {
            huber.iter_mut().for_each(|h| *h = h.max(0.05));
        }
    };
    set_huber(&x, &mut huber, false);
    for round in 0..5 {
        let mut lambda = 1e-3;
        let mut r = residuals(&x, &huber);
        let mut cost: f64 = r.iter().map(|v| v * v).sum();
        for _ in 0..60 {
            // numeric Jacobian
            let m = r.len();
            let mut jac = vec![vec![0.0; np]; m];
            for k in 0..np {
                let mut xp = x.clone();
                let eps = 1e-6;
                xp[k] += eps;
                let rp = residuals(&xp, &huber);
                for i in 0..m {
                    jac[i][k] = (rp[i] - r[i]) / eps;
                }
            }
            let mut jtj = vec![vec![0.0; np]; np];
            let mut jtr = vec![0.0; np];
            for i in 0..m {
                for a in 0..np {
                    jtr[a] += jac[i][a] * r[i];
                    for b in 0..np {
                        jtj[a][b] += jac[i][a] * jac[i][b];
                    }
                }
            }
            let mut improved = false;
            for _ in 0..8 {
                let mut a = jtj.clone();
                for k in 0..np {
                    a[k][k] += lambda * (jtj[k][k] + 1e-12);
                }
                let Some(dx) = solve_lin(a, jtr.iter().map(|v| -v).collect()) else { break };
                let xn: Vec<f64> = x.iter().zip(&dx).map(|(a, b)| a + b).collect();
                if !in_bounds(&xn) {
                    lambda *= 4.0;
                    continue;
                }
                let rn = residuals(&xn, &huber);
                let cn: f64 = rn.iter().map(|v| v * v).sum();
                if cn < cost {
                    let step: f64 = dx.iter().map(|v| v * v).sum::<f64>().sqrt();
                    x = xn;
                    r = rn;
                    let done = (cost - cn) < 1e-14 * cost.max(1e-30) || step < 1e-10;
                    cost = cn;
                    lambda = (lambda / 3.0).max(1e-9);
                    improved = !done;
                    break;
                }
                lambda *= 4.0;
            }
            if !improved {
                break;
            }
        }
        // IRLS: robust weights from the current residuals
        set_huber(&x, &mut huber, round >= 1);
    }

    let (f, c, r) = unpack(&x);
    if !f.is_finite() || f <= 0.0 {
        return Err("카메라를 풀 수 없어 · 선 배치를 확인해 줘".into());
    }
    let mut line_err = vec![f64::NAN; space.lines.len()];
    let (mut se, mut sw) = (0.0, 0.0);
    for l in &ls {
        let e = line_res(f, c, &r, l).clamp(-1.0, 1.0).asin();
        line_err[l.idx] = e.to_degrees().abs();
        se += l.w * e * e;
        sw += l.w;
    }
    let f_px = f * s;
    let c_px = [ox + c[0] * s, oy + c[1] * s];
    // 4. translation: one world unit = one pixel at the origin
    let o = space.origin;
    let xc0 = [o.x as f64 - c_px[0], o.y as f64 - c_px[1], f_px];
    let center = scale(mul_mv(&transpose(&r), xc0), -1.0);
    Ok(Camera {
        f: f_px,
        c: c_px,
        r,
        center,
        long_side: 2.0 * s,
        frame: fr,
        line_err,
        rms_deg: (se / sw.max(1e-12)).sqrt().to_degrees(),
        f_assumed,
    })
}

// ---------------------------------------------------------------- placement (phase 2)

/// Rotation from Blender-style XYZ Euler angles in degrees: R = Rz · Ry · Rx.
pub fn euler_xyz(deg: V3) -> M3 {
    let (a, b, c) = (deg[0].to_radians(), deg[1].to_radians(), deg[2].to_radians());
    let rx = [[1.0, 0.0, 0.0], [0.0, a.cos(), -a.sin()], [0.0, a.sin(), a.cos()]];
    let ry = [[b.cos(), 0.0, b.sin()], [0.0, 1.0, 0.0], [-b.sin(), 0.0, b.cos()]];
    let rz = [[c.cos(), -c.sin(), 0.0], [c.sin(), c.cos(), 0.0], [0.0, 0.0, 1.0]];
    mul_mm(&rz, &mul_mm(&ry, &rx))
}

/// Inverse of `euler_xyz`: XYZ Euler angles (degrees) of a rotation matrix.
pub fn euler_from(r: &M3) -> V3 {
    let b = (-r[2][0]).clamp(-1.0, 1.0).asin();
    let (a, c) = if b.cos().abs() > 1e-9 {
        (r[2][1].atan2(r[2][2]), r[1][0].atan2(r[0][0]))
    } else {
        (0.0, (-r[0][1]).atan2(r[1][1]))
    };
    [a.to_degrees(), b.to_degrees(), c.to_degrees()]
}

pub fn invert(m: &M3) -> Option<M3> {
    let c0 = cross(m[1], m[2]);
    let det = dot(m[0], c0);
    if det.abs() < 1e-300 {
        return None;
    }
    let c1 = cross(m[2], m[0]);
    let c2 = cross(m[0], m[1]);
    // inverse = adj / det, with adj columns = cofactor rows
    Some([
        [c0[0] / det, c1[0] / det, c2[0] / det],
        [c0[1] / det, c1[1] / det, c2[1] / det],
        [c0[2] / det, c1[2] / det, c2[2] / det],
    ])
}

/// Applies a homography; None when the point maps to or behind the camera.
pub fn apply_h(h: &M3, p: Pos2) -> Option<Pos2> {
    let v = mul_mv(h, [p.x as f64, p.y as f64, 1.0]);
    (v[2] > 1e-12).then(|| pos2((v[0] / v[2]) as f32, (v[1] / v[2]) as f32))
}

/// A layer placed in a space. Its 2D content is drawn on a plane; content point `p`
/// sits at plane coordinates `p − anchor` (pixels, x right, y down).
///
/// plane → world: `X = base_t + base_r · base_k · (pos + R(rot) · scale · [q, 0])`.
/// `base` is set when the layer is dropped in (camera-facing, exact 2D look) and
/// `pos/rot/scale` are what the user edits, so they start at zero.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Placement {
    pub space: u64,
    pub anchor: Pos2,
    pub base_r: M3,
    pub base_t: V3,
    pub base_k: f64,
    pub pos: V3,
    /// Degrees, XYZ order.
    pub rot: V3,
    pub scale: f64,
}

impl Placement {
    /// A plane parallel to the image plane at the world origin's depth, through
    /// `anchor`. Such a plane projects by a pure similarity, so the layer looks
    /// exactly as it did in 2D.
    pub fn facing(space: u64, cam: &Camera, anchor: Pos2) -> Self {
        let z0 = cam.origin_depth().max(1e-6);
        let xc = [(anchor.x as f64 - cam.c[0]) / cam.f * z0, (anchor.y as f64 - cam.c[1]) / cam.f * z0, z0];
        let rt = transpose(&cam.r);
        Self {
            space,
            anchor,
            base_r: rt,
            base_t: add(cam.center, mul_mv(&rt, xc)),
            base_k: z0 / cam.f,
            pos: [0.0; 3],
            rot: [0.0; 3],
            scale: 1.0,
        }
    }

    /// Plane → world as (linear part, translation).
    pub fn world_affine(&self) -> (M3, V3) {
        let lin = mul_mm(&self.base_r, &euler_xyz(self.rot));
        let k = self.base_k * self.scale;
        let lin = [
            [lin[0][0] * k, lin[0][1] * k, lin[0][2] * k],
            [lin[1][0] * k, lin[1][1] * k, lin[1][2] * k],
            [lin[2][0] * k, lin[2][1] * k, lin[2][2] * k],
        ];
        (lin, add(self.base_t, scale(mul_mv(&self.base_r, self.pos), self.base_k)))
    }

    /// Unit normal of the plane in world space.
    pub fn normal(&self) -> V3 {
        let (m, _) = self.world_affine();
        normalize(cross([m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]]))
    }

    /// Homography from 2D content coordinates to canvas pixels.
    pub fn homography(&self, cam: &Camera) -> M3 {
        let (m, t) = self.world_affine();
        let a = mul_mm(&cam.r, &m);
        let b = mul_mv(&cam.r, sub(t, cam.center));
        let k = [[cam.f, 0.0, cam.c[0]], [0.0, cam.f, cam.c[1]], [0.0, 0.0, 1.0]];
        let p = [[a[0][0], a[0][1], b[0]], [a[1][0], a[1][1], b[1]], [a[2][0], a[2][1], b[2]]];
        let (ax, ay) = (self.anchor.x as f64, self.anchor.y as f64);
        let sh = [[1.0, 0.0, -ax], [0.0, 1.0, -ay], [0.0, 0.0, 1.0]];
        mul_mm(&mul_mm(&k, &p), &sh)
    }

    /// Camera depth of the plane origin (for back-to-front sorting).
    pub fn depth(&self, cam: &Camera) -> f64 {
        let (_, t) = self.world_affine();
        cam.to_cam(t)[2]
    }

    /// Where the camera ray through canvas point `p` meets this layer's plane (world).
    pub fn ray_hit(&self, cam: &Camera, p: Pos2) -> Option<V3> {
        let (_, t) = self.world_affine();
        let n = self.normal();
        let d = cam.ray(p);
        let den = dot(n, d);
        if den.abs() < 1e-12 {
            return None;
        }
        let s = dot(n, sub(t, cam.center)) / den;
        (s > 0.0).then(|| add(cam.center, scale(d, s)))
    }

    /// Moves the plane by a world-space offset (expressed back in `pos`).
    pub fn shift_world(&mut self, d: V3) {
        let l = scale(mul_mv(&transpose(&self.base_r), d), 1.0 / self.base_k);
        self.pos = add(self.pos, l);
    }

    /// Snaps `base_r` to the nearest right-handed set of world axes, so the layer lies
    /// flat on a wall or floor of the space (it turns visibly).
    pub fn align_to_axes(&mut self) {
        let col = |i: usize| [self.base_r[0][i], self.base_r[1][i], self.base_r[2][i]];
        let snap = |v: V3, skip: Option<usize>| {
            let i = (0..3).filter(|&i| Some(i) != skip).max_by(|&a, &b| v[a].abs().partial_cmp(&v[b].abs()).unwrap()).unwrap();
            let mut e = [0.0; 3];
            e[i] = v[i].signum();
            (e, i)
        };
        let (c0, i0) = snap(col(0), None);
        let (c1, _) = snap(col(1), Some(i0));
        let c2 = cross(c0, c1);
        self.base_r = [[c0[0], c1[0], c2[0]], [c0[1], c1[1], c2[1]], [c0[2], c1[2], c2[2]]];
    }
}

/// Solved cameras of all spaces that have enough lines.
pub fn cameras(spaces: &[Space]) -> std::collections::HashMap<u64, Camera> {
    spaces.iter().filter_map(|s| solve(s).ok().map(|c| (s.id, c))).collect()
}

/// Depth grid (option C in docs/SPACES.md): slices across the world axis closest to
/// the view direction, at equal world steps, plus the four rails joining their
/// corners and a cell grid on the depth-0 slice. Returns world segments with a weight
/// (0 faint, 1 normal, 2 strong).
pub fn depth_grid(cam: &Camera, frame: Rect) -> Vec<(V3, V3, u8)> {
    let fwd = [cam.r[2][0], cam.r[2][1], cam.r[2][2]];
    let d = (0..3).max_by(|&a, &b| fwd[a].abs().partial_cmp(&fwd[b].abs()).unwrap()).unwrap();
    let sd = fwd[d].signum();
    let (mut a, mut b) = ((d + 1) % 3, (d + 2) % 3);
    // a: the axis that reads as horizontal on screen
    if cam.r[0][a].abs() < cam.r[0][b].abs() {
        std::mem::swap(&mut a, &mut b);
    }
    let long = frame.width().max(frame.height()) as f64;
    let g = {
        let x = long / 8.0;
        let e = 10f64.powf(x.log10().floor());
        let m = x / e;
        (if m < 1.5 { 1.0 } else if m < 3.5 { 2.0 } else if m < 7.5 { 5.0 } else { 10.0 }) * e
    };
    let ea = (frame.width() as f64 * 0.5 / g).ceil() * g;
    let eb = (frame.height() as f64 * 0.5 / g).ceil() * g;
    let pt = |td: f64, ua: f64, ub: f64| {
        let mut p = [0.0; 3];
        p[d] = td;
        p[a] = ua;
        p[b] = ub;
        p
    };
    // with Z (up) across the slices they stand on the floor instead of straddling it
    let (blo, bhi) = if b == 2 { (0.0, 2.0 * eb) } else { (-eb, eb) };
    let mut out = Vec::new();
    let (k0, k1) = (-3i32, 12i32);
    for k in k0..=k1 {
        let t = k as f64 * g * sd;
        let w = if k == 0 { 2 } else { 1 };
        let c = [pt(t, -ea, blo), pt(t, ea, blo), pt(t, ea, bhi), pt(t, -ea, bhi)];
        for i in 0..4 {
            out.push((c[i], c[(i + 1) % 4], w));
        }
    }
    for (ua, ub) in [(-ea, blo), (ea, blo), (ea, bhi), (-ea, bhi)] {
        out.push((pt(k0 as f64 * g * sd, ua, ub), pt(k1 as f64 * g * sd, ua, ub), 1));
    }
    let na = (ea / g).round() as i32;
    for i in -na + 1..na {
        out.push((pt(0.0, i as f64 * g, blo), pt(0.0, i as f64 * g, bhi), 0));
    }
    let mut v = blo + g;
    while v < bhi - g * 0.5 {
        out.push((pt(0.0, -ea, v), pt(0.0, ea, v), 0));
        v += g;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera at `pos` looking along `fwd` with Z up.
    fn look(fwd: V3) -> M3 {
        let fwd = normalize(fwd);
        let right = normalize(cross(fwd, [0.0, 0.0, 1.0]));
        let down = cross(fwd, right);
        [right, down, fwd]
    }

    fn truth() -> Camera {
        let fwd = [0.45, 0.85, -0.32];
        let r = look(fwd);
        Camera {
            f: 1100.0,
            c: [800.0, 500.0],
            r,
            center: add(scale(normalize(fwd), -1400.0), [60.0, 0.0, 0.0]),
            long_side: 1600.0,
            frame: Rect::from_min_max(pos2(0.0, 0.0), pos2(1600.0, 1000.0)),
            line_err: vec![],
            rms_deg: 0.0,
            f_assumed: false,
        }
    }

    fn lines_for(cam: &Camera, axes: &[usize], noise: f32) -> Vec<VLine> {
        let mut out = Vec::new();
        let mut k = 0u32;
        for &ax in axes {
            for (u, v) in [(-200.0, 0.0), (150.0, 0.0), (0.0, 180.0), (-120.0, 220.0)] {
                let mut a = [0.0; 3];
                let mut b = [0.0; 3];
                let (o1, o2) = ((ax + 1) % 3, (ax + 2) % 3);
                a[o1] = u;
                a[o2] = v;
                b[o1] = u;
                b[o2] = v;
                a[ax] = -250.0;
                b[ax] = 250.0;
                let (pa, pb) = (cam.project(a).unwrap(), cam.project(b).unwrap());
                // deterministic pseudo-noise
                k = k.wrapping_mul(1664525).wrapping_add(1013904223);
                let n1 = ((k >> 8) as f32 / 16777216.0 - 0.5) * 2.0 * noise;
                k = k.wrapping_mul(1664525).wrapping_add(1013904223);
                let n2 = ((k >> 8) as f32 / 16777216.0 - 0.5) * 2.0 * noise;
                out.push(VLine { a: pa + egui::vec2(n1, -n2), b: pb + egui::vec2(-n2, n1), axis: ax as u8 });
            }
        }
        out
    }

    fn angle_deg(a: V3, b: V3) -> f64 {
        dot(normalize(a), normalize(b)).clamp(-1.0, 1.0).acos().to_degrees()
    }

    fn check(axes: &[usize], noise: f32, free_pp: bool, f_tol: f64, ang_tol: f64) {
        let t = truth();
        let origin = t.project([0.0; 3]).unwrap();
        let mut sp = Space::new(1, "t".into(), t.frame);
        sp.lines = lines_for(&t, axes, noise);
        sp.origin = origin;
        sp.free_pp = free_pp;
        let cam = solve(&sp).unwrap_or_else(|e| panic!("{e}"));
        assert!((cam.f / t.f - 1.0).abs() < f_tol, "f {} vs {}", cam.f, t.f);
        for ax in 0..3 {
            let e = angle_deg(cam.axis_dir(ax), t.axis_dir(ax));
            assert!(e < ang_tol, "axis {ax} off by {e}°");
        }
        let o = cam.project([0.0; 3]).unwrap();
        assert!(o.distance(origin) < 0.01);
        // same picture up to world scale about the origin
        let s = cam.f / t.to_cam([0.0; 3])[2];
        for w in [[100.0, 50.0, 0.0], [-80.0, 30.0, 120.0]] {
            let (pa, pb) = (cam.project(scale(w, s)).unwrap(), t.project(w).unwrap());
            assert!(pa.distance(pb) < 3.0 * (noise.max(0.1)), "{pa:?} vs {pb:?}");
        }
    }

    #[test]
    fn exact_two_axes() {
        check(&[0, 1], 0.0, false, 1e-4, 1e-3);
        check(&[0, 2], 0.0, false, 1e-4, 1e-3);
        check(&[1, 2], 0.0, false, 1e-4, 1e-3);
    }

    #[test]
    fn noisy_three_axes() {
        check(&[0, 1, 2], 1.0, false, 0.03, 0.5);
    }

    #[test]
    fn free_principal_point() {
        check(&[0, 1, 2], 0.0, true, 1e-3, 1e-2);
    }

    #[test]
    fn outlier_is_downweighted() {
        let t = truth();
        let mut sp = Space::new(1, "t".into(), t.frame);
        sp.lines = lines_for(&t, &[0, 1, 2], 0.3);
        sp.origin = t.project([0.0; 3]).unwrap();
        let mut bad = sp.lines[0];
        bad.b = bad.b + egui::vec2(0.0, 120.0);
        sp.lines.push(bad);
        let cam = solve(&sp).unwrap();
        assert!((cam.f / t.f - 1.0).abs() < 0.03, "f {}", cam.f);
        assert!(cam.line_err[sp.lines.len() - 1] > 2.0);
    }

    #[test]
    fn too_few_lines() {
        let t = truth();
        let mut sp = Space::new(1, "t".into(), t.frame);
        sp.lines = lines_for(&t, &[0, 1], 0.0);
        sp.lines.truncate(5); // 4 X + 1 Y
        assert!(solve(&sp).is_err());
    }

    #[test]
    fn facing_keeps_the_2d_look() {
        let t = truth();
        for anchor in [pos2(800.0, 500.0), pos2(150.0, 900.0), pos2(1500.0, 80.0)] {
            let pl = Placement::facing(1, &t, anchor);
            let h = pl.homography(&t);
            for p in [anchor, pos2(0.0, 0.0), pos2(1600.0, 1000.0), anchor + egui::vec2(-333.0, 77.0)] {
                let q = apply_h(&h, p).unwrap();
                assert!(q.distance(p) < 1e-3, "{p:?} -> {q:?}");
            }
        }
    }

    #[test]
    fn local_edits_and_inverse() {
        let t = truth();
        let mut pl = Placement::facing(1, &t, pos2(700.0, 450.0));
        pl.rot = [20.0, -35.0, 10.0];
        pl.scale = 1.3;
        pl.pos = [40.0, -10.0, 200.0];
        let h = pl.homography(&t);
        let hi = invert(&h).unwrap();
        let p = pos2(760.0, 420.0);
        let back = apply_h(&hi, apply_h(&h, p).unwrap()).unwrap();
        assert!(back.distance(p) < 1e-3);
        // the ray through a projected point hits the plane at that point's world position
        let (m, tr) = pl.world_affine();
        let q = [(p.x - 700.0) as f64, (p.y - 450.0) as f64, 0.0];
        let w = add(tr, mul_mv(&m, q));
        let hit = pl.ray_hit(&t, apply_h(&h, p).unwrap()).unwrap();
        assert!(norm(sub(hit, w)) < 1e-6 * norm(w).max(1.0));
        // shifting by a world offset moves exactly that much
        let before = pl.world_affine().1;
        pl.shift_world([5.0, -7.0, 3.0]);
        let after = pl.world_affine().1;
        assert!(norm(sub(sub(after, before), [5.0, -7.0, 3.0])) < 1e-9);
    }

    #[test]
    fn align_gives_a_rotation() {
        let t = truth();
        let mut pl = Placement::facing(1, &t, pos2(700.0, 450.0));
        pl.align_to_axes();
        let r = pl.base_r;
        let det = dot(r[0], cross(r[1], r[2]));
        assert!((det - 1.0).abs() < 1e-12);
        for row in r {
            assert!((norm(row) - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn placement_round_trips_through_json() {
        let t = truth();
        let mut pl = Placement::facing(7, &t, pos2(700.0, 450.0));
        pl.rot = [12.5, -3.0, 44.0];
        let back: Placement = serde_json::from_str(&serde_json::to_string(&pl).unwrap()).unwrap();
        assert_eq!(back, pl);
    }

    #[test]
    fn euler_round_trip() {
        for e in [[10.0, 20.0, 30.0], [-170.0, 45.0, 5.0], [0.0, -89.0, 120.0], [33.0, 0.0, -10.0]] {
            let back = euler_from(&euler_xyz(e));
            let (m1, m2) = (euler_xyz(e), euler_xyz(back));
            for i in 0..3 {
                for j in 0..3 {
                    assert!((m1[i][j] - m2[i][j]).abs() < 1e-9, "{e:?} -> {back:?}");
                }
            }
        }
    }
}
