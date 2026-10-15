//! GPU display of the document through egui, with texture caching.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::{
    Color32, ColorImage, Context, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2, pos2, vec2,
};

use crate::doc::{Document, ImageLayer, LayerId, LayerKind, ShapeKind, ShapeLayer, TextLayer};
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
    texts: HashMap<LayerId, (u64, TextureHandle, u64)>,
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
    }

    pub fn image(&mut self, ctx: &Context, im: &ImageLayer) -> egui::TextureId {
        let key = (im.src.id, im.adjust.key());
        let frame = self.frame;
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
        if let Some(e) = self.images.get_mut(&(im.src.id, im.adjust.key())) {
            e.1 = self.frame;
        }
    }

    fn tile(&mut self, ctx: &Context, layer: LayerId, key: (i32, i32), t: &crate::raster::Tile) -> egui::TextureId {
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

    fn text(&mut self, ctx: &Context, id: LayerId, t: &TextLayer, scale: f32) -> Option<(egui::TextureId, Vec2)> {
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
                return Some((e.1.id(), lay.size));
            }
        }
        let pm = crate::text::render(t, &lay, scale, Vec2::ZERO)?;
        let ci = ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
        let tex = ctx.load_texture(format!("text{id}"), ci, TEX_OPTS);
        let tid = tex.id();
        self.texts.insert(id, (key, tex, frame));
        Some((tid, lay.size))
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
    for layer in &doc.layers {
        if let LayerKind::Image(im) = &layer.kind {
            cache.touch_image(im);
        }
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        let op = layer.opacity;
        match &layer.kind {
            LayerKind::Image(im) => {
                if !Rect::from_points(&im.corners()).intersects(view_w) {
                    continue;
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
                        continue;
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
                    continue;
                }
                let want = (xf.zoom * ppp).max(1.0 / 16.0);
                let mut scale = 2f32.powf(want.log2().ceil()).clamp(1.0 / 16.0, 16.0);
                let lay_size = crate::text::layout(t).size;
                while lay_size.max_elem() * scale > MAX_TEX as f32 && scale > 1.0 / 16.0 {
                    scale *= 0.5;
                }
                let r = Rect::from_min_size(t.pos, lay_size);
                if !r.intersects(view_w) {
                    continue;
                }
                if let Some((tex, size)) = cache.text(ctx, layer.id, t, scale) {
                    let pm_size = vec2(
                        (size.x * scale).ceil() + 1.0,
                        (size.y * scale).ceil() + 1.0,
                    );
                    let sr = Rect::from_min_size(xf.w2s(t.pos), pm_size / scale * xf.zoom);
                    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                    if t.rotation == 0.0 {
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
