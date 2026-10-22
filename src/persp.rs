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

/// A wall (any plane) matched by fitting a quad to a rectangle in the photo: corners
/// top-left, top-right, bottom-right, bottom-left. Layers put on it move in its
/// local axes (u along the top edge, v down the side).
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Wall {
    pub id: u64,
    pub name: String,
    pub corners: [Pos2; 4],
    /// Moves the plane along its normal (world units).
    #[serde(default)]
    pub offset: f64,
    /// Snap the normal to the nearest space axis when it is within 15° (off by
    /// default: the quad itself is the truth).
    #[serde(default)]
    pub snap_axis: bool,
}

/// A wall in world space: `r` has columns u (along the top edge), v (down the side
/// edge) and n (unit normal, into the wall); `o` is the world point at corner 0.
/// u and v follow the quad's edges exactly, so they need not be perpendicular when
/// the quad or the camera is slightly off; `skew_deg` says by how much.
#[derive(Clone, Debug)]
pub struct WallFrame {
    pub r: M3,
    pub o: V3,
    pub skew_deg: f64,
}

impl WallFrame {
    pub fn col(&self, i: usize) -> V3 {
        [self.r[0][i], self.r[1][i], self.r[2][i]]
    }

    /// Where the camera ray through canvas point `p` meets the wall.
    pub fn hit(&self, cam: &Camera, p: Pos2) -> Option<V3> {
        let n = self.col(2);
        let d = cam.ray(p);
        let den = dot(n, d);
        if den.abs() < 1e-12 {
            return None;
        }
        let t = dot(n, sub(self.o, cam.center)) / den;
        (t > 0.0).then(|| add(cam.center, scale(d, t)))
    }

    /// Wall coordinates (u, v) of a world point.
    pub fn uv(&self, x: V3) -> [f64; 2] {
        let d = sub(x, self.o);
        match invert(&self.r) {
            Some(ri) => {
                let c = mul_mv(&ri, d);
                [c[0], c[1]]
            }
            None => [dot(d, self.col(0)), dot(d, self.col(1))],
        }
    }

    pub fn point(&self, uv: [f64; 2]) -> V3 {
        add(self.o, add(scale(self.col(0), uv[0]), scale(self.col(1), uv[1])))
    }
}

/// Plane of a wall quad. The orientation comes from the quad's two vanishing
/// points; the position from where its bottom edge meets the floor for
/// standing walls, the floor itself for floor-like quads, else the origin's depth.
pub fn wall_frame(cam: &Camera, w: &Wall) -> Option<WallFrame> {
    let [tl, tr, br, bl] = w.corners;
    let h = |p: Pos2| [p.x as f64, p.y as f64, 1.0];
    let rt = transpose(&cam.r);
    // camera-space direction of the lines a0-a1 and b0-b1, pointing from a0 to a1
    let dir = |a0: Pos2, a1: Pos2, b0: Pos2, b1: Pos2| -> Option<V3> {
        let vp = cross(cross(h(a0), h(a1)), cross(h(b0), h(b1)));
        let mut d = normalize([(vp[0] - cam.c[0] * vp[2]) / cam.f, (vp[1] - cam.c[1] * vp[2]) / cam.f, vp[2]]);
        if !d.iter().all(|x| x.is_finite()) || norm(d) < 0.5 {
            return None;
        }
        let (a, b) = ((a0.x as f64 - cam.c[0]) / cam.f, (a0.y as f64 - cam.c[1]) / cam.f);
        let motion = [d[0] - a * d[2], d[1] - b * d[2]];
        let want = [(a1.x - a0.x) as f64, (a1.y - a0.y) as f64];
        if motion[0] * want[0] + motion[1] * want[1] < 0.0 {
            d = scale(d, -1.0);
        }
        Some(mul_mv(&rt, d))
    };
    let d1 = normalize(dir(tl, tr, bl, br)?);
    let d2 = normalize(dir(tl, bl, tr, br)?);
    let mut n = normalize(cross(d1, d2));
    // u and v are the quad's own edge directions, so grid lines run along its edges
    let (mut u, mut v) = (d1, d2);
    if w.snap_axis {
        let i = (0..3).max_by(|&a, &b| n[a].abs().partial_cmp(&n[b].abs()).unwrap()).unwrap();
        if n[i].abs() > 15f64.to_radians().cos() {
            let mut e = [0.0; 3];
            e[i] = n[i].signum();
            n = e;
            u = normalize(sub(d1, scale(n, dot(d1, n))));
            v = normalize(sub(d2, scale(n, dot(d2, n))));
        }
    }
    let skew_deg = 90.0 - dot(u, v).clamp(-1.0, 1.0).acos().to_degrees();
    // a point on the plane
    let ground = |p: Pos2| cam.hit_floor(p);
    let mid = pos2((tl.x + tr.x + br.x + bl.x) * 0.25, (tl.y + tr.y + br.y + bl.y) * 0.25);
    let p = if n[2].abs() < 0.5 {
        ground(bl).or_else(|| ground(br))
    } else {
        ground(mid)
    };
    let p = p.unwrap_or_else(|| add(cam.center, scale(cam.ray(tl), cam.origin_depth().max(1.0))));
    let p = add(p, scale(n, w.offset));
    let r = [[u[0], v[0], n[0]], [u[1], v[1], n[1]], [u[2], v[2], n[2]]];
    // origin: corner 0 on the plane
    let d0 = cam.ray(tl);
    let den = dot(n, d0);
    let o = if den.abs() > 1e-12 { add(cam.center, scale(d0, dot(n, sub(p, cam.center)) / den)) } else { p };
    Some(WallFrame { r, o, skew_deg })
}

fn nice125(x: f64) -> f64 {
    let e = 10f64.powf(x.max(1e-9).log10().floor());
    let m = x / e;
    (if m < 1.5 { 1.0 } else if m < 3.5 { 2.0 } else if m < 7.5 { 5.0 } else { 10.0 }) * e
}

/// The picture quad (TL, TR, BR, BL) of a vertical wall standing on the floor
/// along the base line `a`–`b` (canvas points on the floor), `height` tall.
/// None when an end is not on the floor.
pub fn wall_on_base(cam: &Camera, a: Pos2, b: Pos2, height: f64) -> Option<[Pos2; 4]> {
    let (pa, pb) = (cam.hit_floor(a)?, cam.hit_floor(b)?);
    // keep the top in front of the camera (it may not reach the full height)
    let up = |p: V3| {
        let mut h = height;
        for _ in 0..20 {
            if let Some(q) = cam.project(add(p, [0.0, 0.0, h])) {
                return Some(q);
            }
            h *= 0.5;
        }
        None
    };
    Some([up(pa)?, up(pb)?, cam.project(pb)?, cam.project(pa)?])
}

/// Grid on a wall's plane around its quad (weight 2 every fifth line).
pub fn wall_grid(cam: &Camera, w: &Wall) -> Vec<(V3, V3, u8)> {
    let Some(f) = wall_frame(cam, w) else { return Vec::new() };
    let uvs: Vec<[f64; 2]> = w.corners.iter().filter_map(|p| f.hit(cam, *p)).map(|x| f.uv(x)).collect();
    if uvs.len() < 2 {
        return Vec::new();
    }
    let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for [u, v] in &uvs {
        u0 = u0.min(*u);
        u1 = u1.max(*u);
        v0 = v0.min(*v);
        v1 = v1.max(*v);
    }
    let g = nice125((u1 - u0).max(v1 - v0).max(1.0) / 6.0);
    let (u0, u1) = (((u0 / g).floor() - 2.0) * g, ((u1 / g).ceil() + 2.0) * g);
    let (v0, v1) = (((v0 / g).floor() - 2.0) * g, ((v1 / g).ceil() + 2.0) * g);
    let mut out = Vec::new();
    let weight = |x: f64| if ((x / g).round() as i64).rem_euclid(5) == 0 { 2 } else { 1 };
    let mut u = u0;
    while u <= u1 + g * 1e-6 {
        out.push((f.point([u, v0]), f.point([u, v1]), weight(u)));
        u += g;
    }
    let mut v = v0;
    while v <= v1 + g * 1e-6 {
        out.push((f.point([u0, v]), f.point([u1, v]), weight(v)));
        v += g;
    }
    out
}

/// Re-derives the base of every layer that sits on a wall from the wall's current
/// frame, so editing a wall (or the camera) carries its layers along.
pub fn sync_walls(doc: &mut crate::doc::Document) {
    let cams = cameras(&doc.spaces);
    let mut frames: std::collections::HashMap<u64, WallFrame> = std::collections::HashMap::new();
    for sp in &doc.spaces {
        if let Some(cam) = cams.get(&sp.id) {
            for w in &sp.walls {
                if let Some(f) = wall_frame(cam, w) {
                    frames.insert(w.id, f);
                }
            }
        }
    }
    for l in &mut doc.layers {
        if let Some(pl) = l.place.as_mut() {
            if let Some(f) = pl.wall.and_then(|w| frames.get(&w)) {
                pl.base_r = f.r;
                pl.base_t = f.point(pl.wall_uv);
            }
        }
    }
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
    /// Where the space shows on the canvas: its layers and grid are clipped to this
    /// (None: the whole frame). Separate from `frame`, which defines the camera.
    #[serde(default)]
    pub mask: Option<Rect>,
    #[serde(default)]
    pub walls: Vec<Wall>,
    /// Moves, scales and turns with this photo (its transform when last applied).
    #[serde(default)]
    pub follow: Option<SpaceFollow>,
    /// How far the image has been turned since the axes were drawn (radians): the
    /// axis directions are told apart relative to it, so turning keeps them.
    #[serde(default)]
    pub roll: f32,
    /// Directional light for layers with thickness (shading only, no shadows).
    #[serde(default)]
    pub light: Option<Light>,
    /// A length of known size in the picture: the metre scale.
    #[serde(default)]
    pub ref_line: Option<RefLine>,
    /// How far the floor is below the camera (world units, cm). The camera sits
    /// at the world origin, so the eye-level plane z = 0 is the horizon.
    #[serde(default = "default_floor")]
    pub floor: f64,
    /// Eye-level world (camera at the origin). Older files were floor-origin
    /// worlds and are converted on load (`to_eye_level`).
    #[serde(default)]
    pub eye: bool,
    /// A tape measure drawn on the picture (its `metres` is unused).
    #[serde(default)]
    pub tape: Option<RefLine>,
}

/// Floor depth below the camera when nothing measured it (cm): eye height.
pub const DEFAULT_FLOOR: f64 = 160.0;

fn default_floor() -> f64 {
    DEFAULT_FLOOR
}

/// A reference length drawn on the picture (canvas points). It starts on the
/// floor at `a`; along X or Y it runs on the floor to `b`, along Z it rises
/// straight up to the height of `b`. `metres` is its real length.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct RefLine {
    pub a: Pos2,
    pub b: Pos2,
    pub axis: u8,
    pub metres: f64,
}

impl RefLine {
    /// The line's length for a camera 1 unit above the horizontal plane the
    /// line stands on (single-view metrology). It depends only on the
    /// camera's direction and focal length, not on where the origin (floor)
    /// is, so a wrong floor cannot skew it. None when a point is at or above
    /// the horizon.
    pub fn rel_len(&self, cam: &Camera) -> Option<f64> {
        // `a` on the plane one unit below the camera
        let on_plane = |p: Pos2| {
            let d = cam.ray(p);
            (d[2] < -1e-9).then(|| add(cam.center, scale(d, -1.0 / d[2])))
        };
        let a = on_plane(self.a)?;
        // the drawn direction decides (older lines may hold the wrong axis)
        let axis = self.infer_axis(cam).unwrap_or(self.axis);
        if axis < 2 {
            let b = on_plane(self.b)?;
            return Some(norm(sub(b, a)));
        }
        // straight up from `a` to where the ray through `b` passes closest
        let d = cam.ray(self.b);
        let e = [0.0, 0.0, 1.0];
        let w = sub(a, cam.center);
        let (ed, dd) = (dot(e, d), dot(d, d));
        let den = dd - ed * ed;
        if den.abs() < 1e-12 {
            return None;
        }
        let t = (ed * dot(d, w) - dd * dot(e, w)) / den;
        Some(t.abs())
    }

    /// The world axis the drawn line follows best: the one whose direction at
    /// its start point (towards that axis' vanishing point) is closest to it.
    pub fn infer_axis(&self, cam: &Camera) -> Option<u8> {
        let v = self.b - self.a;
        if v.length() < 1e-3 {
            return None;
        }
        let d = cam.ray(self.a);
        if d[2] >= -1e-9 {
            return None;
        }
        let p = add(cam.center, scale(d, -1.0 / d[2]));
        let pa = cam.project(p)?;
        (0..3u8)
            .filter_map(|k| {
                let mut e = [0.0; 3];
                e[k as usize] = 0.01;
                let q = cam.project(add(p, e))?;
                let dir = q - pa;
                (dir.length() > 1e-9).then(|| (k, (dir.normalized().dot(v.normalized())).abs()))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| x.0)
    }

    /// How high the camera is (metres) above the plane the line stands on.
    pub fn camera_height_m(&self, cam: &Camera) -> Option<f64> {
        self.rel_len(cam).filter(|l| *l > 1e-9).map(|l| self.metres / l)
    }
}

/// A directional light: turned by `rot` (degrees, X then Y then Z) from shining
/// along +Z; `ambient` is the brightness of faces turned away.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Light {
    pub rot: V3,
    pub ambient: f32,
}

impl Default for Light {
    fn default() -> Self {
        Self { rot: [-45.0, 30.0, 0.0], ambient: 0.35 }
    }
}

impl Light {
    /// Light from over the camera's upper left shoulder (fronts lit, tops brighter).
    pub fn facing(cam: &Camera) -> Self {
        let row = |i: usize| [cam.r[i][0], cam.r[i][1], cam.r[i][2]];
        let (right, down, fwd) = (row(0), row(1), row(2));
        let d = normalize(add(add(fwd, scale(down, 0.8)), scale(right, 0.6)));
        Self::from_dir(d, 0.35)
    }

    /// A light travelling along `d` (angles fitted to whole degrees... refined).
    pub fn from_dir(d: V3, ambient: f32) -> Self {
        let d = normalize(d);
        // the X/Y angles whose rotation sends +Z closest to d (whole degrees)
        let mut best = ([0.0, 0.0, 0.0], f64::MAX);
        for a in -180..180 {
            for b in -90..=90 {
                let r = [a as f64, b as f64, 0.0];
                let e = sub(mul_mv(&euler_xyz(r), [0.0, 0.0, 1.0]), d);
                let e = dot(e, e);
                if e < best.1 {
                    best = (r, e);
                }
            }
        }
        // refine to a tenth of a degree around the best whole-degree fit
        let (mut r0, mut e0) = best;
        for _ in 0..2 {
            let base = r0;
            for i in -10..=10 {
                for j in -10..=10 {
                    let r = [base[0] + i as f64 * 0.1, base[1] + j as f64 * 0.1, 0.0];
                    let e = sub(mul_mv(&euler_xyz(r), [0.0, 0.0, 1.0]), d);
                    let e = dot(e, e);
                    if e < e0 {
                        r0 = r;
                        e0 = e;
                    }
                }
            }
        }
        Self { rot: r0, ambient }
    }

    /// Direction the light travels, in world space.
    pub fn dir(&self) -> V3 {
        normalize(mul_mv(&euler_xyz(self.rot), [0.0, 0.0, 1.0]))
    }

    /// Brightness (ambient..1) of a face with outward normal `n`.
    pub fn shade(&self, n: V3) -> f32 {
        let l = (-dot(normalize(n), self.dir())).max(0.0) as f32;
        self.ambient + (1.0 - self.ambient) * l
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct SpaceFollow {
    pub layer: u64,
    /// centre x, centre y, scale, rotation of the photo last time.
    pub last: [f32; 4],
}

/// Spaces that follow a photo take its move, scale and turn as a whole: lines,
/// origin, frame, mask and walls move on the canvas, and the world (with the layers
/// placed in it) scales with the photo, so nothing has to be matched again.
pub fn follow_photos(doc: &mut crate::doc::Document) {
    use crate::doc::LayerKind;
    for i in 0..doc.spaces.len() {
        let Some(f) = doc.spaces[i].follow else { continue };
        let Some(LayerKind::Image(im)) = doc.get(f.layer).map(|l| &l.kind) else { continue };
        let cur = [im.center.x, im.center.y, im.scale, im.rotation];
        if cur == f.last {
            continue;
        }
        let [lx, ly, ls, lr] = f.last;
        let k = cur[2] / ls.max(1e-6);
        let th = cur[3] - lr;
        let (c0, c1) = (pos2(lx, ly), pos2(cur[0], cur[1]));
        let map = |p: Pos2| -> Pos2 {
            let d = (p - c0) * k;
            let (s, c) = th.sin_cos();
            c1 + egui::vec2(d.x * c - d.y * s, d.x * s + d.y * c)
        };
        let map_rect = |r: Rect| Rect::from_center_size(map(r.center()), r.size() * k);
        let sp = &mut doc.spaces[i];
        for l in &mut sp.lines {
            l.a = map(l.a);
            l.b = map(l.b);
        }
        sp.origin = map(sp.origin);
        sp.frame = map_rect(sp.frame);
        sp.mask = sp.mask.map(map_rect);
        for w in &mut sp.walls {
            w.corners = w.corners.map(map);
        }
        sp.follow = Some(SpaceFollow { layer: f.layer, last: cur });
        sp.roll += th;
        let id = sp.id;
        // eye level: only the picture (focal length) scales, the world stays
        let kk = if sp.eye { 1.0 } else { k as f64 };
        for l in &mut doc.layers {
            if let Some(pl) = l.place.as_mut().filter(|p| p.space == id) {
                pl.base_t = scale(pl.base_t, kk);
                pl.base_k *= kk;
                pl.thick *= kk;
                pl.wall_uv = [pl.wall_uv[0] * kk, pl.wall_uv[1] * kk];
            }
        }
    }
}

impl Space {
    /// The tape measure's length in metres: measured on the same kind of plane
    /// as the reference (it starts on the reference's plane), so it needs a
    /// reference length. None without one or when either is above the horizon.
    pub fn tape_metres(&self, cam: &Camera) -> Option<f64> {
        let h = self.ref_line?.camera_height_m(cam)?;
        Some(self.tape?.rel_len(cam)? * h)
    }

    pub fn new(id: u64, name: String, frame: Rect) -> Self {
        let origin = pos2(frame.center().x, frame.top() + frame.height() * 0.7);
        Self { id, name, frame, lines: Vec::new(), origin, free_pp: false, grid: true, mask: None, walls: Vec::new(), follow: None, roll: 0.0, light: None, ref_line: None, floor: DEFAULT_FLOOR, eye: true, tape: None }
    }

    /// The canvas area the space shows in.
    pub fn clip(&self) -> Rect {
        self.mask.unwrap_or(self.frame)
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
    /// Camera centre in world units: the origin in an eye-level world.
    pub center: V3,
    /// World height of the floor plane (negative: below the camera).
    pub floor_z: f64,
    /// A point on the floor ahead of the camera (under the frame's bottom
    /// middle), the default depth for placing and viewing.
    pub ref_pt: V3,
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

    /// Camera depth of the reference floor point (the scene's working depth).
    pub fn origin_depth(&self) -> f64 {
        self.to_cam(self.ref_pt)[2]
    }

    /// Where the ray through image point `p` meets the floor (None at or above
    /// the horizon).
    pub fn hit_floor(&self, p: Pos2) -> Option<V3> {
        let d = self.ray(p);
        if d[2] > -1e-9 {
            return None;
        }
        let t = (self.floor_z - self.center[2]) / d[2];
        (t > 0.0).then(|| add(self.center, scale(d, t)))
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
        return Err("공간 영역이 너무 작음".into());
    }
    let counts = [space.count(0), space.count(1), space.count(2)];
    let usable: Vec<usize> = (0..3).filter(|&a| counts[a] >= 2).collect();
    if usable.len() < 2 {
        return Err(format!(
            "두 축에 선 2개씩 필요 · X {} · Y {} · Z {}",
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
    // "right" and "up" of the image, turned with the photo it follows
    let (rs, rc) = (space.roll as f64).sin_cos();
    let right = |v: V3| v[0] * rc + v[1] * rs;
    let up = |v: V3| v[0] * rs - v[1] * rc;
    if let Some(z) = d[2] {
        if up(z) < 0.0 {
            d[2] = Some(flip(z));
        }
    }
    if let Some(x) = d[0] {
        if right(x) < 0.0 {
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
            if right(cross(y, z)) < 0.0 {
                d[1] = Some(flip(y));
            }
        }
        (Some(x), Some(y), None) => {
            if up(cross(x, y)) < 0.0 {
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
        return Err("카메라 계산 불가 · 선 배치 확인".into());
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
    // 4. translation. Eye level: the camera is the origin and the floor lies
    // `floor` below it. Older floor-origin worlds: one unit = one pixel at the
    // origin point (kept only to convert them).
    let (center, floor_z) = if space.eye {
        ([0.0; 3], -space.floor.abs().max(1e-6))
    } else {
        let o = space.origin;
        let xc0 = [o.x as f64 - c_px[0], o.y as f64 - c_px[1], f_px];
        (scale(mul_mv(&transpose(&r), xc0), -1.0), 0.0)
    };
    let mut cam = Camera {
        f: f_px,
        c: c_px,
        r,
        center,
        floor_z,
        ref_pt: [0.0; 3],
        long_side: 2.0 * s,
        frame: fr,
        line_err,
        rms_deg: (se / sw.max(1e-12)).sqrt().to_degrees(),
        f_assumed,
    };
    // working depth: the floor under the frame's bottom middle, else three
    // floor-depths ahead along the view
    let h = (center[2] - floor_z).abs().max(1e-6);
    cam.ref_pt = cam.hit_floor(fr.center_bottom()).filter(|p| norm(sub(*p, center)) < h * 40.0).unwrap_or_else(|| {
        let fwd = [r[2][0], r[2][1], r[2][2]];
        let mut p = add(center, scale(fwd, h * 3.0));
        p[2] = floor_z;
        p
    });
    Ok(cam)
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

/// A planar polygon through `h`. When part of it lies behind the camera (the
/// 3D workspace's free views), it is cut where its depth falls under a
/// thousandth of its farthest corner's; None when none of it is in front.
pub fn project_poly(h: &M3, pts: &[Pos2]) -> Option<Vec<Pos2>> {
    let w: Vec<f64> = pts.iter().map(|p| h[2][0] * p.x as f64 + h[2][1] * p.y as f64 + h[2][2]).collect();
    if w.iter().all(|&x| x > 1e-12) {
        return pts.iter().map(|p| apply_h(h, *p)).collect();
    }
    let far = w.iter().copied().fold(f64::MIN, f64::max);
    if far <= 1e-12 {
        return None;
    }
    let eps = far * 1e-3;
    let mut out = Vec::new();
    for i in 0..pts.len() {
        let j = (i + 1) % pts.len();
        if w[i] >= eps {
            out.push(apply_h(h, pts[i])?);
        }
        if (w[i] >= eps) != (w[j] >= eps) {
            let t = ((eps - w[i]) / (w[j] - w[i])) as f32;
            out.push(apply_h(h, pts[i] + (pts[j] - pts[i]) * t)?);
        }
    }
    (!out.is_empty()).then_some(out)
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
    /// Thickness: the content is extruded this far behind its plane (world units,
    /// along `normal`); 0 = flat.
    #[serde(default)]
    pub thick: f64,
    /// Wall the layer sits on: `base` then follows that wall (see `sync_walls`).
    #[serde(default)]
    pub wall: Option<u64>,
    /// Where the plane origin sits on the wall (wall units).
    #[serde(default)]
    pub wall_uv: [f64; 2],
    /// Rests on its underside: the plane sits `thick` above where it is placed
    /// (against its normal), so a thickness grows up from the floor (lying layers).
    #[serde(default)]
    pub rest: bool,
    /// Outline detail of the extrusion: the outline is traced at this many times
    /// the texture's resolution (1–4).
    #[serde(default = "one_seg")]
    pub segs: u8,
    /// Smooth shading of the sides: normals blend across gentle bends.
    #[serde(default)]
    pub smooth: bool,
}

fn one_seg() -> u8 {
    1
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
            thick: 0.0,
            wall: None,
            wall_uv: [0.0; 2],
            rest: false,
            segs: 1,
            smooth: false,
        }
    }

    /// Standing on the floor at `at` (a floor point), upright and turned to face
    /// the camera across the floor; content point `anchor` (its bottom middle)
    /// touches the floor there. One content pixel is as big as an image pixel
    /// at that distance, so the layer starts at the size it showed in 2D.
    pub fn standing(space: u64, cam: &Camera, at: V3, anchor: Pos2) -> Self {
        let mut n = sub(cam.center, at);
        n[2] = 0.0;
        let n = if norm(n) > 1e-9 { normalize(n) } else { [0.0, -1.0, 0.0] };
        // content x runs to the right on screen, y straight down
        let mut u = normalize(cross([0.0, 0.0, 1.0], n));
        let right = |v: V3| {
            let a = cam.project(at);
            let b = cam.project(add(at, v));
            match (a, b) {
                (Some(a), Some(b)) => (b.x - a.x) as f64,
                _ => 1.0,
            }
        };
        if right(u) < 0.0 {
            u = scale(u, -1.0);
        }
        let v = [0.0, 0.0, -1.0];
        let w = cross(u, v);
        Self {
            space,
            anchor,
            base_r: [[u[0], v[0], w[0]], [u[1], v[1], w[1]], [u[2], v[2], w[2]]],
            base_t: at,
            base_k: cam.to_cam(at)[2].max(1e-6) / cam.f,
            pos: [0.0; 3],
            rot: [0.0; 3],
            scale: 1.0,
            thick: 0.0,
            wall: None,
            wall_uv: [0.0; 2],
            rest: false,
            segs: 1,
            smooth: false,
        }
    }

    /// Lying flat on the floor at `at` (a floor point), its centre `anchor` there,
    /// read from the camera: content x runs to the right on screen and content y
    /// towards the camera (down the picture). Its normal points down, so its front
    /// faces up; it rests on its underside (`rest`), so a thickness lifts the plane
    /// and the layer stays on the floor. One content pixel is as big as an image
    /// pixel at that distance.
    pub fn lying(space: u64, cam: &Camera, at: V3, anchor: Pos2) -> Self {
        let mut st = Self::standing(space, cam, at, anchor);
        // standing: (right, down, away); lying: (right, towards the camera, down)
        let u = [st.base_r[0][0], st.base_r[1][0], st.base_r[2][0]];
        let away = [st.base_r[0][2], st.base_r[1][2], st.base_r[2][2]];
        let v = scale(away, -1.0);
        let w = cross(u, v);
        st.base_r = [[u[0], v[0], w[0]], [u[1], v[1], w[1]], [u[2], v[2], w[2]]];
        st.rest = true;
        st
    }

    /// On a wall: the plane through the wall point under `anchor`, lying flat on the
    /// wall, with a scale that keeps the layer about its 2D size there.
    pub fn on_wall(space: u64, cam: &Camera, wall: &Wall, anchor: Pos2) -> Option<Self> {
        let fr = wall_frame(cam, wall)?;
        let x = fr.hit(cam, anchor)?;
        let d = sub(x, fr.o);
        let (u, v) = (fr.col(0), fr.col(1));
        Some(Self {
            space,
            anchor,
            base_r: fr.r,
            base_t: x,
            base_k: cam.to_cam(x)[2].max(1e-6) / cam.f,
            pos: [0.0; 3],
            rot: [0.0; 3],
            scale: 1.0,
            thick: 0.0,
            wall: Some(wall.id),
            wall_uv: [dot(d, u), dot(d, v)],
            rest: false,
            segs: 1,
            smooth: false,
        })
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
        let mut t = add(self.base_t, scale(mul_mv(&self.base_r, self.pos), self.base_k));
        if self.rest && self.thick > 0.0 {
            // resting on its underside: the plane rises by the thickness
            let n = normalize(cross([lin[0][0], lin[1][0], lin[2][0]], [lin[0][1], lin[1][1], lin[2][1]]));
            t = sub(t, scale(n, self.thick));
        }
        (lin, t)
    }

    /// Unit normal of the plane in world space.
    pub fn normal(&self) -> V3 {
        let (m, _) = self.world_affine();
        normalize(cross([m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]]))
    }

    /// Homography from 2D content coordinates to canvas pixels.
    pub fn homography(&self, cam: &Camera) -> M3 {
        self.homography_at(cam, 0.0)
    }

    /// Homography of the plane moved `off` world units along its normal (extrusion).
    pub fn homography_at(&self, cam: &Camera, off: f64) -> M3 {
        let (m, t) = self.world_affine();
        let t = if off != 0.0 { add(t, scale(self.normal(), off)) } else { t };
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
    /// The lowest world point of content box `b`, both caps included.
    pub fn lowest(&self, b: Rect) -> V3 {
        let (m, t) = self.world_affine();
        let n = scale(self.normal(), self.thick);
        let mut low = [0.0, 0.0, f64::MAX];
        for p in [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()] {
            let w = add(t, mul_mv(&m, [(p.x - self.anchor.x) as f64, (p.y - self.anchor.y) as f64, 0.0]));
            for q in [w, add(w, n)] {
                if q[2] < low[2] {
                    low = q;
                }
            }
        }
        low
    }

    pub fn shift_world(&mut self, d: V3) {
        // base_r may be sheared (walls follow their quad), so invert it properly
        let inv = invert(&self.base_r).unwrap_or_else(|| transpose(&self.base_r));
        let l = scale(mul_mv(&inv, d), 1.0 / self.base_k);
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

/// Converts floor-origin spaces (older files) to eye-level worlds: the camera
/// moves to the origin, the scale makes the old floor sit `DEFAULT_FLOOR` below
/// it, and everything placed in the space (layers, walls, keyed thickness)
/// moves with it, so the picture stays exactly the same.
pub fn to_eye_level(doc: &mut crate::doc::Document) {
    for i in 0..doc.spaces.len() {
        if doc.spaces[i].eye {
            continue;
        }
        let Ok(cam) = solve(&doc.spaces[i]) else {
            // not solvable yet: nothing placed can depend on it
            doc.spaces[i].eye = true;
            doc.spaces[i].floor = DEFAULT_FLOOR;
            continue;
        };
        let c = cam.center;
        let k = if c[2].abs() > 1e-6 { DEFAULT_FLOOR / c[2].abs() } else { 1.0 };
        let sp = &mut doc.spaces[i];
        sp.eye = true;
        sp.floor = DEFAULT_FLOOR;
        for w in &mut sp.walls {
            w.offset *= k;
        }
        let id = sp.id;
        for l in &mut doc.layers {
            let Some(pl) = l.place.as_mut().filter(|p| p.space == id) else { continue };
            pl.base_t = scale(sub(pl.base_t, c), k);
            pl.base_k *= k;
            pl.thick *= k;
            pl.wall_uv = [pl.wall_uv[0] * k, pl.wall_uv[1] * k];
            if let Some(a) = l.anim.as_mut() {
                for key in &mut a.keys {
                    if let Some(pk) = key.place.as_mut() {
                        pk.thick *= k;
                    }
                }
            }
        }
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
    // world width the frame covers at the working depth
    let long = frame.width().max(frame.height()) as f64 * cam.origin_depth().max(1e-6) / cam.f;
    let base = cam.ref_pt;
    let g = {
        let x = long / 8.0;
        let e = 10f64.powf(x.log10().floor());
        let m = x / e;
        (if m < 1.5 { 1.0 } else if m < 3.5 { 2.0 } else if m < 7.5 { 5.0 } else { 10.0 }) * e
    };
    // wide enough to reach the whole frame even when the view is strongly tilted
    let k = cam.origin_depth().max(1e-6) / cam.f;
    let ea = (frame.width() as f64 * k * 1.5 / g).ceil() * g;
    let eb = (frame.height() as f64 * k * 1.5 / g).ceil() * g;
    let pt = |td: f64, ua: f64, ub: f64| {
        let mut p = [0.0; 3];
        p[d] = td;
        p[a] = ua;
        p[b] = ub;
        add(p, base)
    };
    // with Z (up) across the slices they stand on the floor instead of straddling it
    let (blo, bhi) = if b == 2 { (0.0, 2.0 * eb) } else { (-eb, eb) };
    let mut out = Vec::new();
    let (k0, k1) = (-8i32, 60i32);
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
            floor_z: 0.0,
            ref_pt: [0.0; 3],
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
        sp.eye = false;
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
    fn reference_lengths_give_the_camera_height() {
        let t = truth();
        let hc = t.center[2];
        // 1 unit = 1 cm: a 120 cm floor edge and a 90 cm post
        let x = RefLine { a: t.project([0.0, 0.0, 0.0]).unwrap(), b: t.project([120.0, 0.0, 0.0]).unwrap(), axis: 0, metres: 1.2 };
        assert!((x.camera_height_m(&t).unwrap() - hc / 100.0).abs() < 1e-4, "{:?} vs {}", x.camera_height_m(&t), hc / 100.0);
        let z = RefLine { a: t.project([50.0, 40.0, 0.0]).unwrap(), b: t.project([50.0, 40.0, 90.0]).unwrap(), axis: 2, metres: 0.9 };
        assert!((z.camera_height_m(&t).unwrap() - hc / 100.0).abs() < 1e-3);
        // a post standing on a table (plane z = 70): height above the table
        let tb = RefLine { a: t.project([-60.0, 80.0, 70.0]).unwrap(), b: t.project([-60.0, 80.0, 100.0]).unwrap(), axis: 2, metres: 0.3 };
        // the axis comes from the drawing itself
        assert_eq!(z.infer_axis(&t), Some(2));
        assert_eq!(x.infer_axis(&t), Some(0));
        let y = RefLine { a: t.project([0.0, 0.0, 0.0]).unwrap(), b: t.project([0.0, 150.0, 0.0]).unwrap(), axis: 0, metres: 1.0 };
        assert_eq!(y.infer_axis(&t), Some(1));
        assert!((tb.camera_height_m(&t).unwrap() - (hc - 70.0) / 100.0).abs() < 1e-3);
    }

    #[test]
    fn eye_level_world_and_old_files_convert_unchanged() {
        let t = truth();
        // an older floor-origin space with a layer placed in it
        let mut sp = Space::new(1, "t".into(), t.frame);
        sp.eye = false;
        sp.lines = lines_for(&t, &[0, 1, 2], 0.0);
        sp.origin = t.project([0.0; 3]).unwrap();
        let old = solve(&sp).unwrap();
        let mut pl = Placement::facing(1, &old, pos2(700.0, 450.0));
        pl.thick = 40.0;
        pl.rot = [10.0, 20.0, 0.0];
        let h_old = pl.homography_at(&old, 25.0);
        let mut doc = crate::doc::Document::default();
        doc.spaces.push(sp);
        let id = doc.insert("x".into(), crate::doc::LayerKind::Shape(crate::doc::ShapeLayer { kind: crate::doc::ShapeKind::Rect, a: pos2(600.0, 400.0), b: pos2(800.0, 500.0), color: egui::Color32::RED, width: 0.0, fill: None, rotation: 0.0 }), None);
        doc.get_mut(id).unwrap().place = Some(pl);
        to_eye_level(&mut doc);
        let sp = &doc.spaces[0];
        assert!(sp.eye && (sp.floor - DEFAULT_FLOOR).abs() < 1e-9);
        let cam = solve(sp).unwrap();
        // the camera is the origin, the floor 160 below
        assert!(norm(cam.center) < 1e-9 && (cam.floor_z + DEFAULT_FLOOR).abs() < 1e-9);
        // same picture: the layer (and its extrusion, scaled with the world) maps alike
        let pl = doc.get(id).unwrap().place.clone().unwrap();
        let k = pl.thick / 40.0;
        let h_new = pl.homography_at(&cam, 25.0 * k);
        for p in [pos2(600.0, 400.0), pos2(800.0, 500.0), pos2(650.0, 480.0)] {
            let (a, b) = (apply_h(&h_old, p).unwrap(), apply_h(&h_new, p).unwrap());
            assert!(a.distance(b) < 1e-2, "{a:?} vs {b:?}");
        }
        // floor points still land on the floor
        let fp = cam.hit_floor(pos2(800.0, 900.0)).unwrap();
        assert!((fp[2] + DEFAULT_FLOOR).abs() < 1e-6);
    }

    #[test]
    fn standing_layers_touch_the_floor_upright() {
        let t = truth();
        let at = [120.0, 300.0, 0.0];
        let anchor = pos2(50.0, 80.0);
        let pl = Placement::standing(1, &t, at, anchor);
        let (m, o) = pl.world_affine();
        // the anchor is on the floor point, content up is world up
        let world = |p: Pos2| add(o, mul_mv(&m, [(p.x - anchor.x) as f64, (p.y - anchor.y) as f64, 0.0]));
        assert!(norm(sub(world(anchor), at)) < 1e-9);
        let top = world(pos2(50.0, 0.0));
        assert!(top[2] > 0.0 && (top[0] - at[0]).abs() < 1e-9 && (top[1] - at[1]).abs() < 1e-9);
        // content right shows to the right
        let r = world(pos2(90.0, 80.0));
        assert!(t.project(r).unwrap().x > t.project(at).unwrap().x);
    }

    #[test]
    fn walls_stand_on_their_base_line() {
        let t = truth();
        // a floor-origin camera: floor at z = 0
        let (a3, b3) = ([100.0, 400.0, 0.0], [400.0, 380.0, 0.0]);
        let (a, b) = (t.project(a3).unwrap(), t.project(b3).unwrap());
        let q = wall_on_base(&t, a, b, 200.0).unwrap();
        let w = Wall { id: 1, name: "w".into(), corners: q, offset: 0.0, snap_axis: false };
        let f = wall_frame(&t, &w).unwrap();
        // vertical, through both base points
        assert!(f.col(2)[2].abs() < 1e-3, "normal {:?}", f.col(2));
        for p3 in [a3, b3] {
            let hit = f.hit(&t, t.project(p3).unwrap()).unwrap();
            assert!(norm(sub(hit, p3)) < 0.5, "{hit:?} vs {p3:?}");
        }
    }

    #[test]
    fn tape_measures_against_the_reference() {
        let t = truth();
        let mut sp = Space::new(1, "t".into(), t.frame);
        sp.eye = false;
        // 1 unit = 1 cm: a 90 cm post as the reference, a tape 250 cm along Y
        sp.ref_line = Some(RefLine { a: t.project([50.0, 40.0, 0.0]).unwrap(), b: t.project([50.0, 40.0, 90.0]).unwrap(), axis: 2, metres: 0.9 });
        sp.tape = Some(RefLine { a: t.project([-30.0, 100.0, 0.0]).unwrap(), b: t.project([-30.0, 350.0, 0.0]).unwrap(), axis: 1, metres: 0.0 });
        let m = sp.tape_metres(&t).unwrap();
        assert!((m - 2.5).abs() < 1e-3, "{m}");
        sp.ref_line = None;
        assert!(sp.tape_metres(&t).is_none());
    }

    #[test]
    fn lying_layers_face_up_and_rest_on_the_floor() {
        let t = truth();
        let at = [120.0, 300.0, 0.0];
        let anchor = pos2(50.0, 20.0);
        let mut pl = Placement::lying(1, &t, at, anchor);
        assert!(pl.rest && pl.normal()[2] < -0.99, "{:?}", pl.normal());
        let world = |pl: &Placement, p: Pos2| {
            let (m, o) = pl.world_affine();
            add(o, mul_mv(&m, [(p.x - anchor.x) as f64, (p.y - anchor.y) as f64, 0.0]))
        };
        assert!(norm(sub(world(&pl, anchor), at)) < 1e-9);
        // flat on the floor: every content point at the floor's height
        assert!(world(&pl, pos2(0.0, 0.0))[2].abs() < 1e-9 && world(&pl, pos2(100.0, 40.0))[2].abs() < 1e-9);
        // content right shows right, content down shows lower in the picture
        let (pa, pr, pd) = (t.project(at).unwrap(), t.project(world(&pl, pos2(90.0, 20.0))).unwrap(), t.project(world(&pl, pos2(50.0, 60.0))).unwrap());
        assert!(pr.x > pa.x && pd.y > pa.y);
        // a thickness lifts the plane; its underside stays on the floor
        pl.thick = 30.0;
        let top = world(&pl, anchor);
        let under = add(top, scale(pl.normal(), pl.thick));
        assert!((top[2] - 30.0).abs() < 1e-6, "{top:?}");
        assert!(under[2].abs() < 1e-6, "{under:?}");
        // standing layers keep their plane
        let mut st = Placement::standing(1, &t, at, anchor);
        st.thick = 30.0;
        assert!(norm(sub(st.world_affine().1, at)) < 1e-9);
    }

    #[test]
    fn lowest_point_of_a_standing_layer_is_on_the_floor() {
        let t = truth();
        let at = [120.0, 300.0, 0.0];
        let b = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 80.0));
        let mut pl = Placement::standing(1, &t, at, pos2(50.0, 80.0));
        assert!(pl.lowest(b)[2].abs() < 1e-9);
        pl.shift_world([0.0, 0.0, 25.0]);
        assert!((pl.lowest(b)[2] - 25.0).abs() < 1e-9);
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
        sp.eye = false;
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
        sp.eye = false;
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

    #[test]
    fn wall_from_a_quad() {
        let t = truth();
        // a real wall: the plane x = 100, from y = -100..200, z = 0..150
        let q = [[100.0, -100.0, 150.0], [100.0, 200.0, 150.0], [100.0, 200.0, 0.0], [100.0, -100.0, 0.0]];
        let corners = q.map(|w| t.project(w).unwrap());
        let wall = Wall { id: 9, name: "w".into(), corners, offset: 0.0, snap_axis: false };
        let f = wall_frame(&t, &wall).unwrap();
        // normal along X, plane through x = 100, origin at corner 0
        assert!((f.col(2)[0].abs() - 1.0).abs() < 1e-9, "{:?}", f.col(2));
        assert!(norm(sub(f.o, q[0])) < 1e-2, "{:?}", f.o);
        // u runs along the top edge, v down the side
        assert!(dot(f.col(0), normalize(sub(q[1], q[0]))) > 0.999);
        assert!(dot(f.col(1), normalize(sub(q[3], q[0]))) > 0.999);
        // a layer put on it lands exactly on the wall point under its anchor
        let anchor = t.project([100.0, 50.0, 60.0]).unwrap();
        let pl = Placement::on_wall(1, &t, &wall, anchor).unwrap();
        assert!(norm(sub(pl.base_t, [100.0, 50.0, 60.0])) < 1e-2);
        assert!(apply_h(&pl.homography(&t), anchor).unwrap().distance(anchor) < 1e-3);
    }

    #[test]
    fn extrusion_shows_one_cap_and_its_near_sides() {
        let t = truth();
        let mut pl = Placement::facing(1, &t, pos2(700.0, 450.0));
        pl.thick = 50.0;
        // a 100-unit square outline, outward normals
        let sq = [pos2(650.0, 400.0), pos2(750.0, 400.0), pos2(750.0, 500.0), pos2(650.0, 500.0)];
        let ns = [[0.0, -1.0], [1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]];
        let edges: Vec<crate::extrude::Edge> = (0..4).map(|i| crate::extrude::Edge { a: sq[i], b: sq[(i + 1) % 4], n: ns[i], na: ns[i], nb: ns[i], ca: [255; 3], cb: [255; 3] }).collect();
        let to_eye = |w: V3| sub(t.center, w);
        // facing the camera: the front cap shows
        assert_eq!(crate::extrude::visible_cap(&pl, &to_eye).0, 0.0);
        let sides = crate::extrude::sides(&edges, &pl, &|w| t.project(w), &to_eye, &|w| t.to_cam(w)[2], None, 0.5);
        // off-centre view: some sides show, never all four
        assert!(sides.len() < 4, "{}", sides.len());
        assert!(sides.windows(2).all(|w| w[0].depth >= w[1].depth));
    }

    #[test]
    fn wall_grid_runs_along_a_skewed_quad() {
        let t = truth();
        // a sloppy quad: one corner off the true rectangle
        let q = [[100.0, -100.0, 150.0], [100.0, 200.0, 150.0], [100.0, 200.0, 0.0], [100.0, -100.0, 0.0]];
        let mut corners = q.map(|w| t.project(w).unwrap());
        corners[2] += egui::vec2(25.0, -12.0);
        let wall = Wall { id: 9, name: "w".into(), corners, offset: 0.0, snap_axis: false };
        let f = wall_frame(&t, &wall).unwrap();
        let uv: Vec<[f64; 2]> = corners.iter().map(|p| f.uv(f.hit(&t, *p).unwrap())).collect();
        let tol = 1e-3 * (uv[2][0].abs() + uv[2][1].abs());
        assert!(uv[0][0].abs() < tol && uv[0][1].abs() < tol, "{uv:?}");
        assert!(uv[1][1].abs() < tol, "top edge is a u line: {uv:?}");
        assert!(uv[3][0].abs() < tol, "left edge is a v line: {uv:?}");
        assert!((uv[2][0] - uv[1][0]).abs() < tol && (uv[2][1] - uv[3][1]).abs() < tol, "{uv:?}");
        assert!(f.skew_deg.abs() > 0.1);
    }

    /// Moving, scaling and turning the photo carries the whole space along: a layer
    /// placed in it lands where the same similarity sends its old screen position.
    #[test]
    fn space_follows_its_photo() {
        use crate::doc::{Document, ImageLayer, ImageSource, LayerKind, ShapeKind, ShapeLayer, next_rev};
        use std::sync::Arc;
        let t = truth();
        let mut doc = Document::default();
        let src = ImageSource { id: next_rev(), rgba: Arc::new(image::RgbaImage::new(1600, 1000)), encoded: Arc::new(Vec::new()), ext: "png".into(), logical: None, nv12: None };
        let pid = doc.insert("p".into(), LayerKind::Image(ImageLayer::new(src, pos2(800.0, 500.0))), None);
        let mut sp = Space::new(1, "s".into(), t.frame);
        sp.eye = false;
        sp.lines = lines_for(&t, &[0, 1, 2], 0.0);
        sp.origin = t.project([0.0; 3]).unwrap();
        sp.follow = Some(SpaceFollow { layer: pid, last: [800.0, 500.0, 1.0, 0.0] });
        doc.spaces.push(sp);
        let cam = solve(&doc.spaces[0]).unwrap();
        let sid = doc.insert("s".into(), LayerKind::Shape(ShapeLayer { kind: ShapeKind::Rect, a: pos2(900.0, 600.0), b: pos2(960.0, 640.0), color: egui::Color32::WHITE, width: 1.0, fill: None, rotation: 0.0 }), None);
        let mut pl = Placement::facing(1, &cam, pos2(930.0, 620.0));
        pl.rot = [20.0, 10.0, 0.0];
        doc.get_mut(sid).unwrap().place = Some(pl);
        let before = {
            let pl = doc.get(sid).unwrap().place.clone().unwrap();
            let h = pl.homography(&cam);
            [pos2(900.0, 600.0), pos2(960.0, 640.0)].map(|p| apply_h(&h, p).unwrap())
        };
        // the photo moves, grows 1.5× and turns 2.6 rad (past where the axes used to flip),
        // in steps as a drag would
        for i in 1..=10 {
            let u = i as f32 / 10.0;
            if let Some(LayerKind::Image(im)) = doc.get_mut(pid).map(|l| &mut l.kind) {
                im.center = pos2(800.0 + 300.0 * u, 500.0 - 200.0 * u);
                im.scale = 1.0 + 0.5 * u;
                im.rotation = 2.6 * u;
            }
            follow_photos(&mut doc);
        }
        let cam2 = solve(&doc.spaces[0]).unwrap();
        let pl = doc.get(sid).unwrap().place.clone().unwrap();
        let h2 = pl.homography(&cam2);
        let map = |p: Pos2| {
            let d = (p - pos2(800.0, 500.0)) * 1.5;
            let (s, c) = 2.6f32.sin_cos();
            pos2(1100.0 + d.x * c - d.y * s, 300.0 + d.x * s + d.y * c)
        };
        for (i, p) in [pos2(900.0, 600.0), pos2(960.0, 640.0)].iter().enumerate() {
            let got = apply_h(&h2, *p).unwrap();
            let want = map(before[i]);
            assert!(got.distance(want) < 0.5, "{got:?} vs {want:?}");
        }
    }

    #[test]
    fn polygons_behind_the_camera_are_cut() {
        // depth w = 1 + x: the left part of the square (x < -1) is behind
        let h = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0]];
        let sq = [pos2(-3.0, -1.0), pos2(1.0, -1.0), pos2(1.0, 1.0), pos2(-3.0, 1.0)];
        assert!(sq.iter().any(|p| apply_h(&h, *p).is_none()));
        let q = project_poly(&h, &sq).unwrap();
        assert_eq!(q.len(), 4);
        // the cut edge lies where w is a thousandth of the far corners' (2)
        assert!(q.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
        assert!(q.iter().any(|p| p.x < -400.0));
        // all in front: the plain corners
        let front = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0)];
        assert_eq!(project_poly(&h, &front).unwrap(), front.iter().map(|p| apply_h(&h, *p).unwrap()).collect::<Vec<_>>());
        // all behind: nothing
        assert!(project_poly(&h, &[pos2(-2.0, 0.0), pos2(-3.0, 0.0), pos2(-3.0, 1.0)]).is_none());
    }
}
