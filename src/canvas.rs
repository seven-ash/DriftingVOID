//! GPU display of the document through egui, with texture caching.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::{
    Color32, ColorImage, Context, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2, pos2, vec2,
};

use crate::doc::{Document, ImageLayer, Layer, LayerId, LayerKind, ShapeKind, ShapeLayer, TextLayer};
use crate::raster::TILE;
use crate::vector::VStroke;

/// World <-> screen mapping for one frame.
#[derive(Clone, Copy)]
pub struct Xf {
    pub min: Pos2,
    pub pan: Vec2,
    pub zoom: f32,
}

impl Xf {
    pub fn w2s(&self, p: Pos2) -> Pos2 {
        self.min + self.pan + p.to_vec2() * self.zoom
    }
    pub fn s2w(&self, s: Pos2) -> Pos2 {
        ((s - self.min - self.pan) / self.zoom).to_pos2()
    }
    pub fn rect_w2s(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.w2s(r.min), self.w2s(r.max))
    }
}

pub const TEX_OPTS: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Linear,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(egui::TextureFilter::Linear),
};

const MAX_TEX: u32 = 8192;

#[derive(Default)]
pub struct TexCache {
    frame: u64,
    images: HashMap<(u64, u64), (TextureHandle, u64)>,
    tiles: HashMap<(LayerId, (i32, i32)), (u64, TextureHandle, u64)>,
    texts: HashMap<LayerId, (u64, TextureHandle, u64, std::sync::Arc<Vec<u8>>, [u32; 2])>,
    /// Flattened layers placed in 3D spaces: (content key, texture, last frame,
    /// content rect, texture size).
    placed: HashMap<LayerId, (u64, TextureHandle, u64, Rect, Vec2)>,
    /// Video layers: (frame source id + adjust key, texture, last frame), by slot.
    videos: HashMap<u64, ((u64, u64), TextureHandle, u64)>,
    /// Masked layers rendered on the CPU: (key, texture, last frame, region).
    masked: HashMap<LayerId, (u64, TextureHandle, u64, Rect)>,
    /// Lit extrusion sides: (content key, flattened pixmap, edge normals, light key,
    /// texture, last frame).
    sides: HashMap<LayerId, SideTex>,
}

struct SideTex {
    key: u64,
    pm: tiny_skia::Pixmap,
    normals: Vec<[f32; 2]>,
    lkey: u64,
    tex: Option<TextureHandle>,
    frame: u64,
}

impl TexCache {
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Drops textures unused for a while (deleted layers, undo history, ...).
    pub fn gc(&mut self) {
        let f = self.frame;
        self.images.retain(|_, v| f - v.1 < 120);
        self.tiles.retain(|_, v| f - v.2 < 120);
        self.texts.retain(|_, v| f - v.2 < 120);
        self.placed.retain(|_, v| f - v.2 < 120);
        self.videos.retain(|_, v| f - v.2 < 120);
        self.masked.retain(|_, v| f - v.2 < 120);
        self.sides.retain(|_, v| f - v.frame < 120);
    }

    /// The sides texture of a lit extrusion (same layout as `placed`).
    fn sides(&mut self, ctx: &Context, layer: &Layer, res: f32, u: crate::persp::V3, v: crate::persp::V3, light: &crate::persp::Light) -> Option<egui::TextureId> {
        let key = content_key(layer) ^ (res.to_bits() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let lkey = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for x in u.iter().chain(v.iter()).chain(light.rot.iter()) {
                ((x * 1000.0).round() as i64).hash(&mut h);
            }
            light.ambient.to_bits().hash(&mut h);
            h.finish()
        };
        let frame = self.frame;
        let fresh = self.sides.get(&layer.id).is_none_or(|e| e.key != key);
        if fresh {
            let (pm, _, _) = crate::export::flatten_layer(layer, res)?;
            let normals = crate::export::edge_normals(&pm);
            self.sides.insert(layer.id, SideTex { key, pm, normals, lkey: lkey ^ 1, tex: None, frame });
        }
        let e = self.sides.get_mut(&layer.id)?;
        e.frame = frame;
        if e.lkey != lkey || e.tex.is_none() {
            let lit = crate::export::shade_sides(&e.pm, &e.normals, u, v, light);
            let ci = ColorImage::from_rgba_premultiplied([lit.width() as usize, lit.height() as usize], lit.data());
            let opts = TextureOptions { mipmap_mode: None, ..TEX_OPTS };
            match &mut e.tex {
                Some(t) => t.set(ci, opts),
                None => e.tex = Some(ctx.load_texture(format!("sides{}", layer.id), ci, opts)),
            }
            e.lkey = lkey;
        }
        e.tex.as_ref().map(|t| t.id())
    }

    /// A masked layer as one texture over the visible part of it (CPU path, unused).
    #[allow(dead_code)]
    fn masked(&mut self, ctx: &Context, doc: &Document, layer: &Layer, m: &crate::raster::RasterLayer, view_w: Rect, zoom: f32) -> Option<(egui::TextureId, Rect)> {
        let cams = crate::persp::cameras(&doc.spaces);
        let b = layer.shown_bounds(&cams)?;
        // power-of-two scale and a coarse grid, so small pans and zooms reuse it
        let mut scale = 2f32.powf((zoom * ctx.pixels_per_point()).max(1.0 / 16.0).log2().ceil()).min(4.0);
        let grid = 128.0 / scale;
        let r = b.intersect(view_w.expand2(view_w.size() * 0.25));
        if !r.is_positive() {
            return None;
        }
        let r = Rect::from_min_max(
            pos2((r.min.x / grid).floor() * grid, (r.min.y / grid).floor() * grid),
            pos2((r.max.x / grid).ceil() * grid, (r.max.y / grid).ceil() * grid),
        );
        while r.width() * scale > 4096.0 || r.height() * scale > 4096.0 {
            scale *= 0.5;
        }
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        content_key(layer).hash(&mut h);
        for (k, t) in &m.tiles {
            (k, t.rev).hash(&mut h);
        }
        [m.offset.x, m.offset.y, r.min.x, r.min.y, r.max.x, r.max.y, scale, layer.opacity].map(f32::to_bits).hash(&mut h);
        doc.timeline.time.to_bits().hash(&mut h);
        if let Some(pl) = &layer.place {
            serde_json::to_string(pl).unwrap_or_default().hash(&mut h);
            serde_json::to_string(&doc.spaces.iter().find(|s| s.id == pl.space).map(|s| (s.frame, &s.lines, s.origin))).unwrap_or_default().hash(&mut h);
        }
        let key = h.finish();
        let frame = self.frame;
        if let Some(e) = self.masked.get_mut(&layer.id) {
            if e.0 == key {
                e.2 = frame;
                return Some((e.1.id(), e.3));
            }
        }
        let pm = crate::export::render_masked(doc, layer, m, r, scale).ok()?;
        let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
        let opts = TextureOptions { mipmap_mode: None, ..TEX_OPTS };
        match self.masked.get_mut(&layer.id) {
            Some(e) => {
                e.1.set(img, opts);
                *e = (key, e.1.clone(), frame, r);
            }
            None => {
                let t = ctx.load_texture(format!("masked{}", layer.id), img, opts);
                self.masked.insert(layer.id, (key, t, frame, r));
            }
        }
        self.masked.get(&layer.id).map(|e| (e.1.id(), e.3))
    }

    /// Texture of a layer's 2D content at `res` texels per canvas pixel.
    pub fn placed(&mut self, ctx: &Context, layer: &Layer, res: f32) -> Option<(egui::TextureId, Rect, Vec2)> {
        let key = content_key(layer) ^ (res.to_bits() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let frame = self.frame;
        if let Some(e) = self.placed.get_mut(&layer.id) {
            if e.0 == key {
                e.2 = frame;
                return Some((e.1.id(), e.3, e.4));
            }
        }
        let (pm, rect, res) = crate::export::flatten_layer(layer, res)?;
        let size = vec2(pm.width() as f32, pm.height() as f32) / res;
        let ci = ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
        // no mipmaps: at grazing angles and from afar they averaged thin strokes away
        let opts = TextureOptions { mipmap_mode: None, ..TEX_OPTS };
        let tex = ctx.load_texture(format!("placed{}", layer.id), ci, opts);
        let id = tex.id();
        self.placed.insert(layer.id, (key, tex, frame, rect, size));
        Some((id, rect, size))
    }

    pub fn image(&mut self, ctx: &Context, im: &ImageLayer) -> egui::TextureId {
        let key = (im.src.id, im.adjust.key());
        let frame = self.frame;
        if let Some(v) = &im.video {
            // frames change all the time: keep one texture per video and refill it
            if let Some(e) = self.videos.get_mut(&v.slot) {
                e.2 = frame;
                if e.0 != key {
                    let mut img = (*im.src.rgba).clone();
                    im.adjust.apply(&mut img);
                    e.1.set(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()), TEX_OPTS);
                    e.0 = key;
                }
                return e.1.id();
            }
            let mut img = (*im.src.rgba).clone();
            im.adjust.apply(&mut img);
            let tex = ctx.load_texture(format!("video{}", v.slot), ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()), TEX_OPTS);
            let id = tex.id();
            self.videos.insert(v.slot, (key, tex, frame));
            return id;
        }
        if let Some(e) = self.images.get_mut(&key) {
            e.1 = frame;
            return e.0.id();
        }
        let src = &*im.src.rgba;
        let (w, h) = src.dimensions();
        let mut img = if w.max(h) > MAX_TEX {
            let s = MAX_TEX as f32 / w.max(h) as f32;
            image::imageops::resize(
                src,
                ((w as f32 * s) as u32).max(1),
                ((h as f32 * s) as u32).max(1),
                image::imageops::FilterType::Triangle,
            )
        } else {
            src.clone()
        };
        im.adjust.apply(&mut img);
        let ci = ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let tex = ctx.load_texture(format!("img{}", im.src.id), ci, TEX_OPTS);
        let id = tex.id();
        self.images.insert(key, (tex, frame));
        id
    }

    /// Keeps an image texture alive without drawing it.
    pub fn touch_image(&mut self, im: &ImageLayer) {
        if let Some(v) = &im.video {
            if let Some(e) = self.videos.get_mut(&v.slot) {
                e.2 = self.frame;
            }
        }
        if let Some(e) = self.images.get_mut(&(im.src.id, im.adjust.key())) {
            e.1 = self.frame;
        }
    }

    pub fn tile(&mut self, ctx: &Context, layer: LayerId, key: (i32, i32), t: &crate::raster::Tile) -> egui::TextureId {
        let frame = self.frame;
        let k = (layer, key);
        if let Some(e) = self.tiles.get_mut(&k) {
            e.2 = frame;
            if e.0 != t.rev {
                e.1.set(ColorImage::from_rgba_premultiplied([TILE as usize, TILE as usize], &t.px), TEX_OPTS);
                e.0 = t.rev;
            }
            return e.1.id();
        }
        let ci = ColorImage::from_rgba_premultiplied([TILE as usize, TILE as usize], &t.px);
        let tex = ctx.load_texture(format!("tile{layer}_{}_{}", key.0, key.1), ci, TEX_OPTS);
        let id = tex.id();
        self.tiles.insert(k, (t.rev, tex, frame));
        id
    }

    fn text(&mut self, ctx: &Context, id: LayerId, t: &TextLayer, scale: f32) -> Option<(egui::TextureId, Vec2, std::sync::Arc<Vec<u8>>, [u32; 2], u64)> {
        let lay = crate::text::layout(t);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        t.text.hash(&mut h);
        t.size.to_bits().hash(&mut h);
        t.color.hash(&mut h);
        t.family.hash(&mut h);
        t.style.hash(&mut h);
        crate::fontdb::families().is_some().hash(&mut h);
        t.bg.hash(&mut h);
        t.wrap.to_bits().hash(&mut h);
        scale.to_bits().hash(&mut h);
        let key = h.finish();
        let frame = self.frame;
        if let Some(e) = self.texts.get_mut(&id) {
            if e.0 == key {
                e.2 = frame;
                return Some((e.1.id(), lay.size, e.3.clone(), e.4, key));
            }
        }
        let pm = crate::text::render(t, &lay, scale, Vec2::ZERO)?;
        let ci = ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
        let tex = ctx.load_texture(format!("text{id}"), ci, TEX_OPTS);
        let tid = tex.id();
        let px = std::sync::Arc::new(pm.data().to_vec());
        let dims = [pm.width(), pm.height()];
        self.texts.insert(id, (key, tex, frame, px.clone(), dims));
        Some((tid, lay.size, px, dims, key))
    }
}

/// Hash of everything that changes how a layer's 2D content looks.
pub fn content_key(l: &Layer) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let f = |h: &mut std::collections::hash_map::DefaultHasher, v: f32| v.to_bits().hash(h);
    match &l.kind {
        LayerKind::Image(im) => {
            (0u8, im.src.id, im.adjust.key()).hash(&mut h);
            for v in [im.crop.min.x, im.crop.min.y, im.crop.max.x, im.crop.max.y, im.scale, im.rotation, im.center.x, im.center.y] {
                f(&mut h, v);
            }
        }
        LayerKind::Raster(r) => {
            1u8.hash(&mut h);
            f(&mut h, r.offset.x);
            f(&mut h, r.offset.y);
            let mut acc = 0u64;
            for (k, t) in &r.tiles {
                let mut th = std::collections::hash_map::DefaultHasher::new();
                (k, t.rev).hash(&mut th);
                acc = acc.wrapping_add(th.finish());
            }
            (acc, r.tiles.len()).hash(&mut h);
        }
        LayerKind::Vector(v) => {
            (2u8, v.strokes.len()).hash(&mut h);
            f(&mut h, v.offset.x);
            f(&mut h, v.offset.y);
            for s in &v.strokes {
                s.color.hash(&mut h);
                for x in [s.bounds.min.x, s.bounds.min.y, s.bounds.max.x, s.bounds.max.y] {
                    f(&mut h, x);
                }
            }
        }
        LayerKind::Text(t) => {
            (3u8, &t.text, &t.family, &t.style, t.color, t.bg).hash(&mut h);
            for v in [t.size, t.wrap, t.pos.x, t.pos.y, t.rotation] {
                f(&mut h, v);
            }
        }
        LayerKind::Shape(s) => {
            (4u8, format!("{:?}", s.kind), s.color, s.fill).hash(&mut h);
            for v in [s.a.x, s.a.y, s.b.x, s.b.y, s.width, s.rotation] {
                f(&mut h, v);
            }
        }
    }
    h.finish()
}

/// Side colour of an extrusion: the content darkened.
pub const SIDE_SHADE: f32 = 0.55;

/// Draws a layer placed in a space: its flattened 2D content on a subdivided mesh
/// through the space camera's homography; with thickness, as stacked slices along
/// the plane normal (darker in between, far to near).
#[allow(clippy::too_many_arguments)]
fn draw_placed(painter: &Painter, ctx: &Context, layer: &Layer, pl: &crate::persp::Placement, cam: &crate::persp::Camera, xf: &Xf, cache: &mut TexCache, view_w: Rect, light: Option<crate::persp::Light>) {
    use crate::persp::apply_h;
    let h = pl.homography(cam);
    let Some(b) = layer.bounds() else { return };
    let corners: Option<Vec<Pos2>> = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()].iter().map(|p| apply_h(&h, *p)).collect();
    let Some(corners) = corners else { return };
    if pl.thick <= 0.0 && !Rect::from_points(&corners).intersects(view_w) {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let mag = crate::export::magnification(&h, b) * xf.zoom * ppp;
    let res = 2f32.powf(mag.max(1e-3).log2().ceil()).clamp(1.0 / 16.0, 2.0);
    let Some((tex, rect, size)) = cache.placed(ctx, layer, res) else { return };
    let op = layer.opacity.clamp(0.0, 1.0);
    let cap = tint(op);
    let side = {
        let v = (255.0 * op * SIDE_SHADE) as u8;
        Color32::from_rgba_premultiplied(v, v, v, (255.0 * op) as u8)
    };
    let slices = pl.slices(|w| cam.project(w).map(|p| xf.w2s(p)), |w| cam.to_cam(w)[2]);
    // lit: caps by their facing, sides from a texture shaded by edge direction
    let light = light.filter(|_| pl.thick > 0.0);
    let lit_sides = light.and_then(|l| {
        let (u, v) = pl.axes();
        cache.sides(ctx, layer, res, u, v, &l)
    });
    let n0 = pl.normal();
    const N: usize = 12;
    for (off, is_cap) in slices {
        let h = if off == 0.0 { h } else { pl.homography_at(cam, off) };
        let (color, slice_tex) = match (light, lit_sides) {
            (Some(l), Some(st)) => {
                if is_cap {
                    let n = if off == 0.0 { crate::persp::scale(n0, -1.0) } else { n0 };
                    let s = l.shade(n);
                    let v = (255.0 * op * s) as u8;
                    (Color32::from_rgba_premultiplied(v, v, v, (255.0 * op) as u8), tex)
                } else {
                    (cap, st)
                }
            }
            _ => (if is_cap { cap } else { side }, tex),
        };
        let mut m = Mesh::with_texture(slice_tex);
        let mut ok = true;
        for j in 0..=N {
            for i in 0..=N {
                let t = vec2(i as f32 / N as f32, j as f32 / N as f32);
                let p = rect.min + rect.size() * t;
                let Some(s) = apply_h(&h, p) else {
                    ok = false;
                    break;
                };
                let uv = ((p - rect.min) / size).to_pos2();
                m.vertices.push(egui::epaint::Vertex { pos: xf.w2s(s), uv, color });
            }
        }
        if !ok {
            continue;
        }
        for j in 0..N as u32 {
            for i in 0..N as u32 {
                let a = j * (N as u32 + 1) + i;
                let c = a + N as u32 + 1;
                m.indices.extend_from_slice(&[a, a + 1, c + 1, a, c + 1, c]);
            }
        }
        painter.add(Shape::mesh(m));
    }
}

fn tint(op: f32) -> Color32 {
    Color32::WHITE.gamma_multiply(op.clamp(0.0, 1.0))
}

pub fn image_mesh(tex: egui::TextureId, corners: [Pos2; 4], uv: Rect, color: Color32) -> Mesh {
    let mut m = Mesh::with_texture(tex);
    let uvs = [uv.left_top(), uv.right_top(), uv.right_bottom(), uv.left_bottom()];
    for i in 0..4 {
        m.vertices.push(egui::epaint::Vertex { pos: corners[i], uv: uvs[i], color });
    }
    m.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    m
}

pub fn stroke_mesh(m: &mut Mesh, s: &VStroke, xf: &Xf, offset: Vec2, opacity: f32) {
    let color = s.color.gamma_multiply(opacity);
    let base = m.vertices.len() as u32;
    m.vertices.reserve(s.tris.len());
    for p in &s.tris {
        m.vertices.push(egui::epaint::Vertex { pos: xf.w2s(*p + offset), uv: egui::epaint::WHITE_UV, color });
    }
    m.indices.extend((0..s.tris.len() as u32).map(|i| base + i));
}

pub fn draw_shape(painter: &Painter, s: &ShapeLayer, xf: &Xf, opacity: f32) {
    let col = s.color.gamma_multiply(opacity);
    let w = (s.width * xf.zoom).max(0.5);
    let stroke = Stroke::new(w, col);
    match s.kind {
        ShapeKind::Line => {
            painter.line_segment([xf.w2s(s.a), xf.w2s(s.b)], stroke);
            painter.circle_filled(xf.w2s(s.a), w * 0.5, col);
            painter.circle_filled(xf.w2s(s.b), w * 0.5, col);
        }
        ShapeKind::Arrow => {
            let (end, head) = s.arrow_head();
            painter.line_segment([xf.w2s(s.a), xf.w2s(end)], stroke);
            painter.circle_filled(xf.w2s(s.a), w * 0.5, col);
            painter.add(Shape::convex_polygon(head.iter().map(|p| xf.w2s(*p)).collect(), col, Stroke::NONE));
        }
        ShapeKind::Rect => {
            let r = xf.rect_w2s(Rect::from_two_pos(s.a, s.b));
            if s.rotation == 0.0 {
                if let Some(f) = s.fill {
                    painter.rect_filled(r, 0.0, f.gamma_multiply(opacity));
                }
                if s.width > 0.0 {
                    painter.rect_stroke(r, 0.0, stroke, StrokeKind::Middle);
                }
            } else {
                let c = r.center();
                let pts: Vec<Pos2> = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
                    .iter()
                    .map(|p| crate::doc::rot_about(*p, c, s.rotation))
                    .collect();
                if let Some(f) = s.fill {
                    painter.add(Shape::convex_polygon(pts.clone(), f.gamma_multiply(opacity), Stroke::NONE));
                }
                if s.width > 0.0 {
                    painter.add(Shape::closed_line(pts, stroke));
                }
            }
        }
        ShapeKind::Ellipse => {
            let n = ((Rect::from_two_pos(s.a, s.b).size().max_elem() * xf.zoom) as usize / 3).clamp(24, 256);
            let c = Rect::from_two_pos(s.a, s.b).center();
            let pts: Vec<Pos2> = s.ellipse_points(n).iter().map(|p| xf.w2s(crate::doc::rot_about(*p, c, s.rotation))).collect();
            if let Some(f) = s.fill {
                painter.add(Shape::convex_polygon(pts.clone(), f.gamma_multiply(opacity), Stroke::NONE));
            }
            if s.width > 0.0 {
                painter.add(Shape::closed_line(pts, stroke));
            }
        }
    }
}

/// Draws all visible layers. `skip_text` hides the text layer being edited.
pub fn draw_doc(
    painter: &Painter,
    ctx: &Context,
    doc: &Document,
    xf: &Xf,
    cache: &mut TexCache,
    skip_text: Option<LayerId>,
) {
    let view = painter.clip_rect();
    let view_w = Rect::from_min_max(xf.s2w(view.min), xf.s2w(view.max));
    let ppp = ctx.pixels_per_point();
    let cams = crate::persp::cameras(&doc.spaces);
    for layer in doc.draw_order(&cams) {
        if let LayerKind::Image(im) = &layer.kind {
            cache.touch_image(im);
        }
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        if layer.mask_of.is_some() {
            continue;
        }
        let timed = crate::anim::layer_at_time(doc, layer);
        let layer = timed.as_ref().unwrap_or(layer);
        let mask = doc.layers.iter().find(|m| m.mask_of == Some(layer.id) && m.visible);
        let masking = mask.is_some() && crate::glass::gl_ok();
        if masking {
            painter.add(crate::glass::mask_begin(painter.clip_rect()));
        }
        draw_one(painter, ctx, doc, layer, xf, cache, skip_text, view_w, ppp, &cams);
        if let (true, Some(m)) = (masking, mask) {
            if let LayerKind::Raster(r) = &m.kind {
                let mut tiles = Vec::new();
                for (&key, t) in &r.tiles {
                    let tr = Rect::from_min_size(pos2((key.0 * TILE) as f32, (key.1 * TILE) as f32), vec2(TILE as f32, TILE as f32)).translate(r.offset);
                    if !tr.intersects(view_w) {
                        continue;
                    }
                    let sr = xf.rect_w2s(tr);
                    tiles.push(([sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom()], cache.tile(ctx, m.id, key, t)));
                }
                if !tiles.is_empty() {
                    painter.add(crate::glass::mask_restore(painter.clip_rect(), tiles));
                }
            }
        }
    }
}

/// One layer as it shows on the canvas.
#[allow(clippy::too_many_arguments)]
fn draw_one(painter: &Painter, ctx: &Context, doc: &Document, layer: &Layer, xf: &Xf, cache: &mut TexCache, skip_text: Option<LayerId>, view_w: Rect, ppp: f32, cams: &HashMap<u64, crate::persp::Camera>) {
    {
        if let Some((pl, cam)) = layer.place.as_ref().and_then(|pl| cams.get(&pl.space).map(|c| (pl, c))) {
            if skip_text != Some(layer.id) {
                let clip = match doc.spaces.iter().find(|s| s.id == pl.space) {
                    Some(sp) => painter.clip_rect().intersect(xf.rect_w2s(sp.clip())),
                    None => painter.clip_rect(),
                };
                let light = doc.spaces.iter().find(|s| s.id == pl.space).and_then(|s| s.light);
                draw_placed(&painter.with_clip_rect(clip), ctx, layer, pl, cam, xf, cache, view_w, light);
            }
            return;
        }
        let op = layer.opacity;
        match &layer.kind {
            LayerKind::Image(im) => {
                if !Rect::from_points(&im.corners()).intersects(view_w) {
                    return;
                }
                let tex = cache.image(ctx, im);
                let size = im.src.size();
                let uv = Rect::from_min_max((im.crop.min.to_vec2() / size).to_pos2(), (im.crop.max.to_vec2() / size).to_pos2());
                let c = im.corners().map(|p| xf.w2s(p));
                painter.add(Shape::mesh(image_mesh(tex, c, uv, tint(op))));
            }
            LayerKind::Raster(r) => {
                let local = view_w.translate(-r.offset);
                for (&key, t) in &r.tiles {
                    let tr = Rect::from_min_size(
                        pos2((key.0 * TILE) as f32, (key.1 * TILE) as f32),
                        vec2(TILE as f32, TILE as f32),
                    );
                    if !tr.intersects(local) {
                        return;
                    }
                    let tex = cache.tile(ctx, layer.id, key, t);
                    let sr = xf.rect_w2s(tr.translate(r.offset));
                    painter.image(tex, sr, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), tint(op));
                }
            }
            LayerKind::Vector(v) => {
                let local = view_w.translate(-v.offset);
                let mut m = Mesh::default();
                for s in &v.strokes {
                    if s.bounds.intersects(local) {
                        stroke_mesh(&mut m, s, xf, v.offset, op);
                    }
                }
                if !m.is_empty() {
                    painter.add(Shape::mesh(m));
                }
            }
            LayerKind::Text(t) => {
                if skip_text == Some(layer.id) {
                    return;
                }
                let want = (xf.zoom * ppp).max(1.0 / 16.0);
                let mut scale = 2f32.powf(want.log2().ceil()).clamp(1.0 / 16.0, 16.0);
                let lay_size = crate::text::layout(t).size;
                while lay_size.max_elem() * scale > MAX_TEX as f32 && scale > 1.0 / 16.0 {
                    scale *= 0.5;
                }
                let r = Rect::from_min_size(t.pos, lay_size);
                if !r.intersects(view_w) {
                    return;
                }
                if let Some((tex, size, px, dims, key)) = cache.text(ctx, layer.id, t, scale) {
                    let pm_size = vec2(
                        (size.x * scale).ceil() + 1.0,
                        (size.y * scale).ceil() + 1.0,
                    );
                    let sr = Rect::from_min_size(xf.w2s(t.pos), pm_size / scale * xf.zoom);
                    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                    if t.blend != crate::doc::Blend::Normal && crate::glass::gl_ok() {
                        let c = xf.w2s(Rect::from_min_size(t.pos, size).center());
                        let corners = [sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom()]
                            .map(|p| crate::doc::rot_about(p, c, t.rotation));
                        painter.add(crate::glass::blend_shape(corners, px, dims, key, t.blend.code(), op));
                    } else if t.rotation == 0.0 {
                        painter.image(tex, sr, uv, tint(op));
                    } else {
                        // rotate about the text box centre (not the padded texture's)
                        let c = xf.w2s(Rect::from_min_size(t.pos, size).center());
                        let corners = [sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom()]
                            .map(|p| crate::doc::rot_about(p, c, t.rotation));
                        painter.add(Shape::mesh(image_mesh(tex, corners, uv, tint(op))));
                    }
                }
            }
            LayerKind::Shape(s) => draw_shape(painter, s, xf, op),
        }
    }
}
