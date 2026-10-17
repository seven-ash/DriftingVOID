//! 3D workspace of a space (docs/SPACES.md, phase 3): a Blender-like orbit view with
//! view presets, the matched camera as a gizmo, picking, and the G/R/S modal
//! transforms. The world is the space's (Z up); one unit is one canvas pixel at the
//! origin.

use egui::{Align2, Color32, Context, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::canvas::{TexCache, Xf};
use crate::doc::{Document, Layer, LayerId};
use crate::persp::{self, Camera, M3, Placement, V3, add, cross, dot, mul_mm, mul_mv, normalize, scale, sub, transpose};
use crate::theme::{ACCENT, INK_2, INK_3};

/// Walls are drawn in this colour everywhere.
pub const WALL_COLOR: Color32 = Color32::from_rgb(80, 220, 230);

#[derive(Clone, Debug, PartialEq)]
pub struct View3d {
    pub space: u64,
    pub target: V3,
    pub dist: f64,
    /// Degrees around Z; 0 looks along +Y (Blender's front view).
    pub yaw: f64,
    /// Degrees; positive looks down.
    pub pitch: f64,
    pub ortho: bool,
    /// Vertical field of view, degrees.
    pub fov: f64,
    /// Looking through the matched camera (Numpad 0).
    pub camera: bool,
}

impl View3d {
    /// Starts where the matched camera stands, looking where it looks.
    pub fn from_camera(space: u64, cam: &Camera) -> Self {
        let f = [cam.r[2][0], cam.r[2][1], cam.r[2][2]];
        let d = cam.origin_depth().max(1.0);
        Self {
            space,
            target: add(cam.center, scale(f, d)),
            dist: d,
            yaw: f[0].atan2(f[1]).to_degrees(),
            pitch: (-f[2]).clamp(-1.0, 1.0).asin().to_degrees(),
            ortho: false,
            fov: 2.0 * (cam.frame.height() as f64 * 0.5 / cam.f).atan().to_degrees(),
            camera: false,
        }
    }

    /// Default user view: pulled back and orbited from the matched camera, so the
    /// camera gizmo and the members are both in sight.
    pub fn user(space: u64, cam: &Camera) -> Self {
        let mut v = Self::from_camera(space, cam);
        v.yaw += 30.0;
        v.pitch = (v.pitch + 15.0).clamp(-89.0, 89.0);
        v.dist *= 1.8;
        v
    }

    pub fn forward(&self) -> V3 {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        [y.sin() * p.cos(), y.cos() * p.cos(), -p.sin()]
    }

    pub fn right(&self) -> V3 {
        let y = self.yaw.to_radians();
        [y.cos(), -y.sin(), 0.0]
    }

    /// World → view rotation (rows: right, down, forward).
    pub fn rot(&self) -> M3 {
        let (r, f) = (self.right(), self.forward());
        [r, cross(f, r), f]
    }

    pub fn eye(&self) -> V3 {
        sub(self.target, scale(self.forward(), self.dist))
    }

    /// Blender's numpad views (Ctrl: the opposite side). They switch to orthographic.
    pub fn preset(&mut self, key: u8, opposite: bool) {
        let (y, p) = match (key, opposite) {
            (1, false) => (0.0, 0.0),
            (1, true) => (180.0, 0.0),
            (3, false) => (-90.0, 0.0),
            (3, true) => (90.0, 0.0),
            (7, false) => (0.0, 90.0),
            (7, true) => (0.0, -90.0),
            _ => return,
        };
        self.yaw = y;
        self.pitch = p;
        self.ortho = true;
        self.camera = false;
    }
}

/// Screen projection of one frame: perspective (`ppu` None) or orthographic.
#[derive(Clone)]
pub struct Proj {
    pub rect: Rect,
    pub r: M3,
    pub eye: V3,
    pub f: f64,
    pub c: [f64; 2],
    pub ppu: Option<f64>,
    pub near: f64,
    /// Camera view: canvas point p shows at `off + p·k`.
    pub canvas: Option<(f32, egui::Vec2)>,
}

impl Proj {
    pub fn new(view: &View3d, cam: &Camera, rect: Rect) -> Self {
        if view.camera {
            // fit the space frame into the viewport; the canvas maps by a similarity
            let fr = cam.frame;
            let k = (rect.width() / fr.width()).min(rect.height() / fr.height()) * 0.92;
            let off = rect.center().to_vec2() - fr.center().to_vec2() * k;
            return Self {
                rect,
                r: cam.r,
                eye: cam.center,
                f: cam.f * k as f64,
                c: [off.x as f64 + cam.c[0] * k as f64, off.y as f64 + cam.c[1] * k as f64],
                ppu: None,
                near: 1e-3 * cam.f,
                canvas: Some((k, off)),
            };
        }
        let half = (view.fov.to_radians() * 0.5).tan();
        let h2 = rect.height() as f64 * 0.5;
        Self {
            rect,
            r: view.rot(),
            eye: view.eye(),
            f: h2 / half,
            c: [rect.center().x as f64, rect.center().y as f64],
            ppu: view.ortho.then(|| h2 / (view.dist * half)),
            near: view.dist * 1e-3,
            canvas: None,
        }
    }

    pub fn to_view(&self, w: V3) -> V3 {
        mul_mv(&self.r, sub(w, self.eye))
    }

    pub fn view_to_screen(&self, x: V3) -> Option<Pos2> {
        match self.ppu {
            Some(k) => Some(pos2((self.c[0] + x[0] * k) as f32, (self.c[1] + x[1] * k) as f32)),
            None if x[2] > self.near * 0.5 => Some(pos2((self.c[0] + self.f * x[0] / x[2]) as f32, (self.c[1] + self.f * x[1] / x[2]) as f32)),
            None => None,
        }
    }

    pub fn project(&self, w: V3) -> Option<Pos2> {
        self.view_to_screen(self.to_view(w))
    }

    /// Segment clipped to the near plane (perspective only).
    pub fn segment(&self, a: V3, b: V3) -> Option<[Pos2; 2]> {
        let (mut va, mut vb) = (self.to_view(a), self.to_view(b));
        if self.ppu.is_none() {
            let n = self.near;
            if va[2] < n && vb[2] < n {
                return None;
            }
            if va[2] < n {
                va = add(va, scale(sub(vb, va), (n - va[2]) / (vb[2] - va[2])));
            } else if vb[2] < n {
                vb = add(vb, scale(sub(va, vb), (n - vb[2]) / (va[2] - vb[2])));
            }
        }
        Some([self.view_to_screen(va)?, self.view_to_screen(vb)?])
    }

    /// World units per screen point at a world point.
    pub fn units_per_px(&self, w: V3) -> f64 {
        match self.ppu {
            Some(k) => 1.0 / k,
            None => self.to_view(w)[2].max(self.near) / self.f,
        }
    }

    /// Ray through a screen point: (origin, unit direction).
    pub fn ray(&self, s: Pos2) -> (V3, V3) {
        let rt = transpose(&self.r);
        let (dx, dy) = (s.x as f64 - self.c[0], s.y as f64 - self.c[1]);
        match self.ppu {
            Some(k) => (add(self.eye, mul_mv(&rt, [dx / k, dy / k, 0.0])), [self.r[2][0], self.r[2][1], self.r[2][2]]),
            None => (self.eye, normalize(mul_mv(&rt, [dx / self.f, dy / self.f, 1.0]))),
        }
    }

    /// Canvas drawing transform in camera view.
    pub fn canvas_xf(&self) -> Option<Xf> {
        self.canvas.map(|(k, off)| Xf { min: Pos2::ZERO, pan: off, zoom: k })
    }
}

/// 1-2-5 step near `x`.
fn nice(x: f64) -> f64 {
    let e = 10f64.powf(x.max(1e-9).log10().floor());
    let m = x / e;
    (if m < 1.5 { 1.0 } else if m < 3.5 { 2.0 } else if m < 7.5 { 5.0 } else { 10.0 }) * e
}

/// Members of a space with their placements.
fn members<'a>(doc: &'a Document, space: u64) -> impl Iterator<Item = (&'a Layer, &'a Placement)> {
    doc.layers.iter().filter_map(move |l| l.place.as_ref().filter(|p| p.space == space).map(|p| (l, p)))
}

/// World point of a content point on a placed layer.
fn content_to_world(pl: &Placement, m: &M3, t: V3, p: Pos2) -> V3 {
    let q = [(p.x - pl.anchor.x) as f64, (p.y - pl.anchor.y) as f64, 0.0];
    add(t, mul_mv(m, q))
}

/// Draws the workspace. Camera view reuses the 2D renderer (it is the same picture).
#[allow(clippy::too_many_arguments)]
pub fn draw(painter: &Painter, ctx: &Context, doc: &Document, view: &View3d, proj: &Proj, cam: &Camera, cache: &mut TexCache, sel: Option<LayerId>) {
    let rect = proj.rect;
    if let Some(xf) = proj.canvas_xf() {
        let fr = xf.rect_w2s(cam.frame);
        let clip = painter.with_clip_rect(fr.intersect(rect));
        crate::canvas::draw_doc(&clip, ctx, doc, &xf, cache, None);
        painter.rect_stroke(fr, 0.0, Stroke::new(1.0, INK_3), egui::StrokeKind::Outside);
        draw_selection(painter, doc, proj, sel);
        return;
    }
    // floor grid (XY) with the X and Y axes
    let g = nice(cam.frame.width().max(cam.frame.height()) as f64 / 8.0);
    let n = 20i32;
    let e = g * n as f64;
    for i in -n..=n {
        let t = i as f64 * g;
        let c = if i == 0 { None } else { Some(Color32::from_white_alpha(if i % 5 == 0 { 46 } else { 22 })) };
        for (a, b, axis) in [([-e, t, 0.0], [e, t, 0.0], 0usize), ([t, -e, 0.0], [t, e, 0.0], 1)] {
            if let Some(s) = proj.segment(a, b) {
                let col = c.unwrap_or(persp::AXIS_COLORS[axis].gamma_multiply(0.85));
                painter.line_segment(s, Stroke::new(if c.is_none() { 1.4 } else { 1.0 }, col));
            }
        }
    }
    camera_gizmo(painter, proj, cam);
    if let Some(sp) = doc.spaces.iter().find(|s| s.id == view.space) {
        for w in &sp.walls {
            let Some(f) = persp::wall_frame(cam, w) else { continue };
            let q: Vec<V3> = w.corners.iter().filter_map(|p| f.hit(cam, *p)).collect();
            if q.len() == 4 {
                for i in 0..4 {
                    if let Some(s) = proj.segment(q[i], q[(i + 1) % 4]) {
                        painter.line_segment(s, Stroke::new(1.2, WALL_COLOR));
                    }
                }
            }
        }
    }

    // placed layers, far to near
    let mut list: Vec<(&Layer, &Placement, f64)> = members(doc, view.space)
        .filter(|(l, _)| l.visible && l.opacity > 0.0)
        .map(|(l, p)| (l, p, proj.to_view(p.world_affine().1)[2]))
        .collect();
    list.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    for (l, pl, _) in list {
        draw_plane(painter, ctx, l, pl, proj, cache);
    }
    draw_selection(painter, doc, proj, sel);
    // origin
    if let Some(o) = proj.project([0.0; 3]) {
        painter.circle_stroke(o, 3.5, Stroke::new(1.2, INK_2));
    }
}

fn draw_plane(painter: &Painter, ctx: &Context, l: &Layer, pl: &Placement, proj: &Proj, cache: &mut TexCache) {
    let Some(b) = l.bounds() else { return };
    let (m, t) = pl.world_affine();
    let corners: Vec<Option<Pos2>> = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()]
        .iter()
        .map(|p| proj.project(content_to_world(pl, &m, t, *p)))
        .collect();
    // texture resolution from the projected size
    let shown = corners.iter().flatten().copied().collect::<Vec<_>>();
    let res = if shown.len() >= 2 {
        let sz = Rect::from_points(&shown).size().max_elem() as f64;
        let k = sz / b.size().max_elem().max(1.0) as f64 * ctx.pixels_per_point() as f64;
        2f32.powf((k.max(1e-3) as f32).log2().ceil()).clamp(1.0 / 16.0, 2.0)
    } else {
        0.25
    };
    let Some((tex, rect, size)) = cache.placed(ctx, l, res) else { return };
    const N: usize = 12;
    let op = l.opacity.clamp(0.0, 1.0);
    let cap = Color32::WHITE.gamma_multiply(op);
    let side = {
        let v = (255.0 * op * crate::canvas::SIDE_SHADE) as u8;
        Color32::from_rgba_premultiplied(v, v, v, (255.0 * op) as u8)
    };
    let n = pl.normal();
    for (off, is_cap) in pl.slices(|w| proj.project(w), |w| proj.to_view(w)[2]) {
        let color = if is_cap { cap } else { side };
        let t = add(t, scale(n, off));
        let mut mesh = Mesh::with_texture(tex);
        let mut ok = Vec::with_capacity((N + 1) * (N + 1));
        for j in 0..=N {
            for i in 0..=N {
                let tt = vec2(i as f32 / N as f32, j as f32 / N as f32);
                let p = rect.min + rect.size() * tt;
                let s = proj.project(content_to_world(pl, &m, t, p));
                ok.push(s.is_some());
                mesh.vertices.push(egui::epaint::Vertex { pos: s.unwrap_or(Pos2::ZERO), uv: ((p - rect.min) / size).to_pos2(), color });
            }
        }
        for j in 0..N {
            for i in 0..N {
                let a = j * (N + 1) + i;
                let c = a + N + 1;
                if ok[a] && ok[a + 1] && ok[c] && ok[c + 1] {
                    let (a, c) = (a as u32, c as u32);
                    mesh.indices.extend_from_slice(&[a, a + 1, c + 1, a, c + 1, c]);
                }
            }
        }
        painter.add(Shape::mesh(mesh));
    }
}

fn draw_selection(painter: &Painter, doc: &Document, proj: &Proj, sel: Option<LayerId>) {
    let Some(l) = sel.and_then(|id| doc.get(id)) else { return };
    let (Some(pl), Some(b)) = (l.place.as_ref(), l.bounds()) else { return };
    let (m, t) = pl.world_affine();
    let c = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()].map(|p| content_to_world(pl, &m, t, p));
    for i in 0..4 {
        if let Some(s) = proj.segment(c[i], c[(i + 1) % 4]) {
            painter.line_segment(s, Stroke::new(1.6, ACCENT));
        }
    }
    if let Some(o) = proj.project(t) {
        painter.circle_filled(o, 3.0, ACCENT);
    }
}

/// The matched camera as a frustum with its frame and an "up" marker.
fn camera_gizmo(painter: &Painter, proj: &Proj, cam: &Camera) {
    let d = cam.origin_depth().max(1.0) * 0.18;
    // seen from (nearly) its own position the frustum would just frame the screen
    if proj.ppu.is_none() && crate::persp::norm(sub(proj.eye, cam.center)) < d * 0.5 {
        return;
    }
    let rt = transpose(&cam.r);
    let fr = cam.frame;
    let at = |p: Pos2| add(cam.center, mul_mv(&rt, [(p.x as f64 - cam.c[0]) / cam.f * d, (p.y as f64 - cam.c[1]) / cam.f * d, d]));
    let c = [at(fr.left_top()), at(fr.right_top()), at(fr.right_bottom()), at(fr.left_bottom())];
    let st = Stroke::new(1.2, INK_2);
    for i in 0..4 {
        if let Some(s) = proj.segment(c[i], c[(i + 1) % 4]) {
            painter.line_segment(s, st);
        }
        if let Some(s) = proj.segment(cam.center, c[i]) {
            painter.line_segment(s, Stroke::new(1.0, INK_3));
        }
    }
    let top = at(pos2(fr.center().x, fr.top() - fr.height() * 0.12));
    if let (Some(a), Some(b), Some(t)) = (proj.project(c[0]), proj.project(c[1]), proj.project(top)) {
        let (a, b) = (a.lerp(b, 0.3), a.lerp(b, 0.7));
        painter.add(Shape::convex_polygon(vec![a, b, t], INK_3.gamma_multiply(0.6), Stroke::NONE));
    }
}

/// Topmost member of the space under a screen point (free views; camera view uses
/// the 2D hit test on the canvas point).
pub fn pick(doc: &Document, proj: &Proj, space: u64, s: Pos2) -> Option<LayerId> {
    let (o, d) = proj.ray(s);
    let mut best: Option<(f64, LayerId)> = None;
    for (l, pl) in members(doc, space) {
        if !l.visible || l.locked {
            continue;
        }
        let Some(b) = l.bounds() else { continue };
        let (m, t) = pl.world_affine();
        let (u, v) = ([m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]]);
        let n = normalize(cross(u, v));
        let den = dot(n, d);
        if den.abs() < 1e-12 {
            continue;
        }
        let tt = dot(n, sub(t, o)) / den;
        if tt <= 0.0 && proj.ppu.is_none() {
            continue;
        }
        let x = sub(add(o, scale(d, tt)), t);
        let q = pos2((dot(x, u) / dot(u, u)) as f32 + pl.anchor.x, (dot(x, v) / dot(v, v)) as f32 + pl.anchor.y);
        if b.contains(q) && best.is_none_or(|(bt, _)| tt < bt) {
            best = Some((tt, l.id));
        }
    }
    best.map(|b| b.1)
}

// ---------------------------------------------------------------- navigation

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Nav {
    Orbit,
    Pan,
    Zoom,
}

pub fn navigate(view: &mut View3d, proj: &Proj, nav: Nav, d: egui::Vec2) {
    match nav {
        Nav::Orbit => {
            view.camera = false;
            view.yaw += d.x as f64 * 0.4;
            view.pitch = (view.pitch + d.y as f64 * 0.4).clamp(-90.0, 90.0);
        }
        Nav::Pan => {
            let k = proj.units_per_px(view.target);
            let (r, f) = (view.right(), view.forward());
            let down = cross(f, r);
            view.target = sub(view.target, add(scale(r, d.x as f64 * k), scale(down, d.y as f64 * k)));
            view.camera = false;
        }
        Nav::Zoom => zoom(view, (d.x - d.y) as f64 * 0.006),
    }
}

/// Dolly in (`amount` > 0) or out.
pub fn zoom(view: &mut View3d, amount: f64) {
    view.camera = false;
    view.dist = (view.dist * (-amount).exp()).clamp(1.0, 1e7);
}

// ---------------------------------------------------------------- G / R / S

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Op {
    Grab,
    Rotate,
    Scale,
}

#[derive(Clone, Debug)]
pub struct Modal {
    pub op: Op,
    pub id: LayerId,
    pub orig: Placement,
    pub start: Pos2,
    pub center: Pos2,
    pub axis: Option<usize>,
    pub local: bool,
    pub typed: String,
}

impl Modal {
    pub fn new(op: Op, id: LayerId, orig: Placement, start: Pos2, proj: &Proj) -> Self {
        let center = proj.project(orig.world_affine().1).unwrap_or(start);
        Self { op, id, orig, start, center, axis: None, local: false, typed: String::new() }
    }

    /// X/Y/Z: global axis → local axis → free (as in Blender).
    pub fn press_axis(&mut self, a: usize) {
        match (self.axis, self.local) {
            (Some(x), false) if x == a => self.local = true,
            (Some(x), true) if x == a => {
                self.axis = None;
                self.local = false;
            }
            _ => {
                self.axis = Some(a);
                self.local = false;
            }
        }
    }

    pub fn type_char(&mut self, c: char) {
        match c {
            '-' => {
                if self.typed.starts_with('-') {
                    self.typed.remove(0);
                } else {
                    self.typed.insert(0, '-');
                }
            }
            '0'..='9' | '.' => self.typed.push(c),
            _ => {}
        }
    }

    fn typed_value(&self) -> Option<f64> {
        let t = self.typed.trim_start_matches('-');
        if t.is_empty() {
            return None;
        }
        let v: f64 = t.parse().ok()?;
        Some(if self.typed.starts_with('-') { -v } else { v })
    }

    fn axis_dir(&self) -> Option<V3> {
        let a = self.axis?;
        if self.local {
            let w = mul_mm(&self.orig.base_r, &persp::euler_xyz(self.orig.rot));
            Some(normalize([w[0][a], w[1][a], w[2][a]]))
        } else {
            let mut e = [0.0; 3];
            e[a] = 1.0;
            Some(e)
        }
    }

    /// The placement for the current pointer position.
    pub fn apply(&self, proj: &Proj, cur: Pos2, ctrl: bool, shift: bool) -> Placement {
        let mut pl = self.orig.clone();
        let t = self.orig.world_affine().1;
        let md = cur - self.start;
        let fine = if shift { 0.1 } else { 1.0 };
        match self.op {
            Op::Grab => {
                let d = match (self.axis_dir(), self.typed_value()) {
                    (Some(u), Some(v)) => scale(u, v),
                    (None, Some(v)) => [v, 0.0, 0.0],
                    (Some(u), None) => {
                        let l = proj.units_per_px(t) * 50.0;
                        let su = match (proj.project(t), proj.project(add(t, scale(u, l)))) {
                            (Some(a), Some(b)) => (b - a) / l as f32,
                            _ => egui::Vec2::ZERO,
                        };
                        let mut amount = if su.length_sq() > 1e-12 { (md.dot(su) / su.length_sq()) as f64 * fine } else { 0.0 };
                        if ctrl {
                            amount = (amount / 10.0).round() * 10.0;
                        }
                        scale(u, amount)
                    }
                    (None, None) => {
                        let k = proj.units_per_px(t) * fine;
                        let (r, f) = (proj.r[0], proj.r[2]);
                        let down = cross(f, r);
                        let mut d = add(scale(r, md.x as f64 * k), scale(down, md.y as f64 * k));
                        if ctrl {
                            d = d.map(|x| (x / 10.0).round() * 10.0);
                        }
                        d
                    }
                };
                pl.shift_world(d);
            }
            Op::Rotate => {
                let fwd = proj.r[2];
                let (u, mut th) = match self.typed_value() {
                    Some(v) => (self.axis_dir().unwrap_or(fwd), v.to_radians()),
                    None => {
                        let a0 = (self.start - self.center).angle();
                        let a1 = (cur - self.center).angle();
                        let th = ((a1 - a0) as f64) * fine;
                        match self.axis_dir() {
                            Some(u) => (u, th * if dot(u, fwd) < 0.0 { -1.0 } else { 1.0 }),
                            None => (fwd, th),
                        }
                    }
                };
                if ctrl {
                    th = (th.to_degrees() / 5.0).round() * 5.0;
                    th = th.to_radians();
                }
                let w = mul_mm(&self.orig.base_r, &persp::euler_xyz(self.orig.rot));
                let w2 = mul_mm(&persp::rodrigues(scale(normalize(u), th)), &w);
                pl.rot = persp::euler_from(&mul_mm(&transpose(&self.orig.base_r), &w2));
            }
            Op::Scale => {
                let mut f = match self.typed_value() {
                    Some(v) => v,
                    None => {
                        let d0 = (self.start - self.center).length().max(1.0) as f64;
                        let d1 = (cur - self.center).length() as f64;
                        1.0 + (d1 / d0 - 1.0) * fine
                    }
                };
                if ctrl {
                    f = (f * 10.0).round() / 10.0;
                }
                pl.scale = (self.orig.scale * f).max(0.01);
            }
        }
        pl
    }

    pub fn label(&self) -> String {
        let op = match self.op {
            Op::Grab => "이동",
            Op::Rotate => "회전",
            Op::Scale => "크기",
        };
        let axis = match self.axis {
            Some(a) => format!(" · {} 축{}", persp::AXIS_NAMES[a], if self.local { " (로컬)" } else { "" }),
            None => String::new(),
        };
        let typed = if self.typed.is_empty() { String::new() } else { format!(" · {}", self.typed) };
        format!("{op}{axis}{typed}   X/Y/Z 축 · 숫자 입력 · Ctrl 스냅 · Shift 정밀 · Enter/클릭 확정 · Esc/우클릭 취소")
    }
}

/// Label at the top-left of the viewport.
pub fn hud(painter: &Painter, rect: Rect, text: &str) {
    painter.text(rect.left_top() + vec2(96.0, 92.0), Align2::LEFT_TOP, text, FontId::proportional(13.0), INK_2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        // the same synthetic room as the solver tests, solved from exact lines
        let fwd = normalize([0.45, 0.85, -0.32]);
        let right = normalize(cross(fwd, [0.0, 0.0, 1.0]));
        let down = cross(fwd, right);
        Camera {
            f: 1100.0,
            c: [800.0, 500.0],
            r: [right, down, fwd],
            center: add(scale(fwd, -1400.0), [60.0, 0.0, 0.0]),
            long_side: 1600.0,
            frame: Rect::from_min_max(pos2(0.0, 0.0), pos2(1600.0, 1000.0)),
            line_err: vec![],
            rms_deg: 0.0,
            f_assumed: false,
        }
    }

    fn setup() -> (Camera, View3d, Proj, Placement) {
        let c = cam();
        let v = View3d::user(1, &c);
        let p = Proj::new(&v, &c, Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
        let pl = Placement::facing(1, &c, pos2(700.0, 450.0));
        (c, v, p, pl)
    }

    #[test]
    fn camera_view_matches_the_canvas() {
        let c = cam();
        let mut v = View3d::user(1, &c);
        v.camera = true;
        let p = Proj::new(&v, &c, Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
        let xf = p.canvas_xf().unwrap();
        for w in [[0.0, 0.0, 0.0], [120.0, -40.0, 60.0]] {
            let a = p.project(w).unwrap();
            let b = xf.w2s(c.project(w).unwrap());
            assert!(a.distance(b) < 1e-2, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn typed_grab_and_rotate() {
        let (_, _, p, pl) = setup();
        let mut m = Modal::new(Op::Grab, 1, pl.clone(), pos2(600.0, 400.0), &p);
        m.press_axis(2);
        for ch in "-25".chars() {
            m.type_char(ch);
        }
        let out = m.apply(&p, pos2(600.0, 400.0), false, false);
        let d = sub(out.world_affine().1, pl.world_affine().1);
        assert!(norm_close(d, [0.0, 0.0, -25.0]));

        let mut m = Modal::new(Op::Rotate, 1, pl.clone(), pos2(600.0, 400.0), &p);
        m.press_axis(2);
        for ch in "90".chars() {
            m.type_char(ch);
        }
        let out = m.apply(&p, pos2(600.0, 400.0), false, false);
        let w0 = mul_mm(&pl.base_r, &persp::euler_xyz(pl.rot));
        let w1 = mul_mm(&out.base_r, &persp::euler_xyz(out.rot));
        let want = mul_mm(&persp::rodrigues([0.0, 0.0, std::f64::consts::FRAC_PI_2]), &w0);
        for i in 0..3 {
            for j in 0..3 {
                assert!((w1[i][j] - want[i][j]).abs() < 1e-9);
            }
        }
        // the plane origin stays put while rotating
        assert!(norm_close(sub(out.world_affine().1, pl.world_affine().1), [0.0; 3]));
    }

    #[test]
    fn scale_and_axis_cycle() {
        let (_, _, p, pl) = setup();
        let mut m = Modal::new(Op::Scale, 1, pl, pos2(600.0, 400.0), &p);
        m.type_char('2');
        assert!((m.apply(&p, pos2(0.0, 0.0), false, false).scale - 2.0).abs() < 1e-12);
        m.press_axis(0);
        assert_eq!((m.axis, m.local), (Some(0), false));
        m.press_axis(0);
        assert_eq!((m.axis, m.local), (Some(0), true));
        m.press_axis(0);
        assert_eq!((m.axis, m.local), (None, false));
    }

    #[test]
    fn picking_hits_the_plane() {
        let (_, _, p, pl) = setup();
        let mut doc = Document::default();
        let id = doc.insert(
            "t".into(),
            crate::doc::LayerKind::Shape(crate::doc::ShapeLayer {
                kind: crate::doc::ShapeKind::Rect,
                a: pos2(600.0, 400.0),
                b: pos2(800.0, 500.0),
                width: 4.0,
                color: Color32::WHITE,
                fill: Some(Color32::WHITE),
                rotation: 0.0,
            }),
            None,
        );
        doc.get_mut(id).unwrap().place = Some(pl.clone());
        let (m, t) = pl.world_affine();
        let inside = p.project(content_to_world(&pl, &m, t, pos2(700.0, 450.0))).unwrap();
        let outside = p.project(content_to_world(&pl, &m, t, pos2(1000.0, 450.0))).unwrap();
        assert_eq!(pick(&doc, &p, 1, inside), Some(id));
        assert_eq!(pick(&doc, &p, 1, outside), None);
    }

    fn norm_close(a: V3, b: V3) -> bool {
        crate::persp::norm(sub(a, b)) < 1e-6
    }
}
