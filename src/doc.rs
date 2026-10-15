//! Document model. World units are pixels: an image placed at scale 1.0
//! exports at its original resolution when the export scale is 1.0.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::raster::RasterLayer;
use crate::vector::VStroke;

static NEXT_REV: AtomicU64 = AtomicU64::new(1);

/// Process-wide unique revision counter (texture cache keys, image ids).
pub fn next_rev() -> u64 {
    NEXT_REV.fetch_add(1, Ordering::Relaxed)
}

pub type LayerId = u64;

#[derive(Clone)]
pub struct ImageSource {
    pub id: u64,
    pub rgba: Arc<image::RgbaImage>,
    /// Original file bytes, kept so projects store photos losslessly.
    pub encoded: Arc<Vec<u8>>,
    pub ext: String,
}

impl ImageSource {
    pub fn size(&self) -> Vec2 {
        vec2(self.rgba.width() as f32, self.rgba.height() as f32)
    }
}

#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Adjust {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
}

impl Adjust {
    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0 && self.contrast == 0.0 && self.saturation == 0.0
    }

    pub fn key(&self) -> u64 {
        let b = (self.brightness * 1000.0) as i64 as u64;
        let c = (self.contrast * 1000.0) as i64 as u64;
        let s = (self.saturation * 1000.0) as i64 as u64;
        b.wrapping_mul(1_000_003) ^ c.wrapping_mul(7_919) ^ s.wrapping_mul(104_729)
    }

    /// Applies the adjustment in place to straight-alpha RGBA bytes.
    pub fn apply(&self, rgba: &mut [u8]) {
        if self.is_identity() {
            return;
        }
        let mut lut = [[0f32; 256]; 1];
        for (i, v) in lut[0].iter_mut().enumerate() {
            *v = i as f32 / 255.0;
        }
        let contrast = 1.0 + self.contrast;
        let bright = self.brightness * 0.5;
        let sat = 1.0 + self.saturation;
        for px in rgba.chunks_exact_mut(4) {
            let mut c = [lut[0][px[0] as usize], lut[0][px[1] as usize], lut[0][px[2] as usize]];
            for v in &mut c {
                *v = (*v - 0.5) * contrast + 0.5 + bright;
            }
            let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            for (i, v) in c.iter().enumerate() {
                let o = l + (v - l) * sat;
                px[i] = (o.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
}

#[derive(Clone)]
pub struct ImageLayer {
    pub src: ImageSource,
    /// World position of the crop rectangle's centre.
    pub center: Pos2,
    pub scale: f32,
    /// Radians, clockwise on screen.
    pub rotation: f32,
    /// Crop rectangle in source pixels (integer-valued).
    pub crop: Rect,
    pub adjust: Adjust,
}

impl ImageLayer {
    pub fn new(src: ImageSource, center: Pos2) -> Self {
        let crop = Rect::from_min_size(Pos2::ZERO, src.size());
        Self { src, center, scale: 1.0, rotation: 0.0, crop, adjust: Adjust::default() }
    }

    pub fn axes(&self) -> (Vec2, Vec2) {
        let (s, c) = self.rotation.sin_cos();
        (vec2(c, s), vec2(-s, c))
    }

    pub fn half(&self) -> Vec2 {
        self.crop.size() * self.scale * 0.5
    }

    /// Corners TL, TR, BR, BL in world space.
    pub fn corners(&self) -> [Pos2; 4] {
        let (u, v) = self.axes();
        let h = self.half();
        let c = self.center;
        [c - u * h.x - v * h.y, c + u * h.x - v * h.y, c + u * h.x + v * h.y, c - u * h.x + v * h.y]
    }

    /// World point -> source pixel coordinates.
    pub fn world_to_src(&self, p: Pos2) -> Pos2 {
        let (u, v) = self.axes();
        let d = p - self.center;
        let local = vec2(d.dot(u), d.dot(v)) / self.scale;
        self.crop.center() + local
    }

    pub fn src_to_world(&self, s: Pos2) -> Pos2 {
        let (u, v) = self.axes();
        let d = (s - self.crop.center()) * self.scale;
        self.center + u * d.x + v * d.y
    }

    pub fn contains(&self, p: Pos2) -> bool {
        self.crop.contains(self.world_to_src(p))
    }
}

#[derive(Clone)]
pub struct TextLayer {
    /// Top-left corner of the text box.
    pub pos: Pos2,
    pub text: String,
    /// Em size in world pixels.
    pub size: f32,
    pub color: Color32,
    /// Installed font family name; empty = built-in default.
    pub family: String,
    pub style: String,
    pub bg: Option<Color32>,
    /// Wrap width of the box (0 = no wrapping).
    pub wrap: f32,
    /// Radians, about the box centre.
    pub rotation: f32,
}

/// Rotates `p` by `th` (clockwise on screen) about `c`.
pub fn rot_about(p: Pos2, c: Pos2, th: f32) -> Pos2 {
    if th == 0.0 {
        return p;
    }
    let (s, co) = th.sin_cos();
    let d = p - c;
    c + vec2(d.x * co - d.y * s, d.x * s + d.y * co)
}

fn rotated_bounds(r: Rect, th: f32) -> Rect {
    if th == 0.0 {
        return r;
    }
    let c = r.center();
    Rect::from_points(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| rot_about(p, c, th)))
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum ShapeKind {
    Line,
    Arrow,
    Rect,
    Ellipse,
}

#[derive(Clone)]
pub struct ShapeLayer {
    pub kind: ShapeKind,
    pub a: Pos2,
    pub b: Pos2,
    pub color: Color32,
    pub width: f32,
    pub fill: Option<Color32>,
    /// Radians about the centre (rectangles and ellipses; lines use their end points).
    pub rotation: f32,
}

impl ShapeLayer {
    /// Arrow geometry: shaft end and the three head points.
    pub fn arrow_head(&self) -> (Pos2, [Pos2; 3]) {
        let d = self.b - self.a;
        let len = d.length().max(1e-3);
        let dir = d / len;
        let n = vec2(-dir.y, dir.x);
        let head = (self.width * 3.5 + 6.0).min(len * 0.6);
        let base = self.b - dir * head;
        let hw = head * 0.55;
        (base + dir * (head * 0.15), [self.b, base + n * hw, base - n * hw])
    }

    pub fn ellipse_points(&self, n: usize) -> Vec<Pos2> {
        let r = Rect::from_two_pos(self.a, self.b);
        let c = r.center();
        let h = r.size() * 0.5;
        (0..n)
            .map(|i| {
                let t = i as f32 / n as f32 * std::f32::consts::TAU;
                pos2(c.x + h.x * t.cos(), c.y + h.y * t.sin())
            })
            .collect()
    }
}

#[derive(Clone, Default)]
pub struct VectorLayer {
    pub offset: Vec2,
    pub strokes: Vec<Arc<VStroke>>,
}

#[derive(Clone)]
pub enum LayerKind {
    Image(ImageLayer),
    Text(TextLayer),
    Shape(ShapeLayer),
    Vector(VectorLayer),
    Raster(RasterLayer),
}

#[derive(Clone)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub kind: LayerKind,
}

impl Layer {
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            LayerKind::Image(_) => "사진",
            LayerKind::Text(_) => "글",
            LayerKind::Shape(_) => "도형",
            LayerKind::Vector(_) => "펜",
            LayerKind::Raster(_) => "브러시",
        }
    }

    pub fn display_name(&self) -> String {
        if let LayerKind::Text(t) = &self.kind {
            let line = t.text.lines().next().unwrap_or("").trim();
            if !line.is_empty() {
                let s: String = line.chars().take(24).collect();
                return s;
            }
        }
        self.name.clone()
    }

    /// Axis-aligned world bounds of visible content.
    pub fn bounds(&self) -> Option<Rect> {
        match &self.kind {
            LayerKind::Image(im) => Some(Rect::from_points(&im.corners())),
            LayerKind::Text(t) => Some(rotated_bounds(crate::text::layout(t).rect(t.pos), t.rotation)),
            LayerKind::Shape(s) => {
                let mut r = Rect::from_two_pos(s.a, s.b);
                if matches!(s.kind, ShapeKind::Rect | ShapeKind::Ellipse) {
                    r = rotated_bounds(r, s.rotation);
                }
                if s.kind == ShapeKind::Arrow {
                    let (_, h) = s.arrow_head();
                    for p in h {
                        r.extend_with(p);
                    }
                }
                Some(r.expand(s.width * 0.5 + 1.0))
            }
            LayerKind::Vector(v) => {
                let mut acc: Option<Rect> = None;
                for s in &v.strokes {
                    let b = s.bounds.translate(v.offset);
                    acc = Some(acc.map_or(b, |a| a.union(b)));
                }
                acc
            }
            LayerKind::Raster(r) => r.bounds(),
        }
    }

    pub fn translate(&mut self, d: Vec2) {
        match &mut self.kind {
            LayerKind::Image(im) => im.center += d,
            LayerKind::Text(t) => t.pos += d,
            LayerKind::Shape(s) => {
                s.a += d;
                s.b += d;
            }
            LayerKind::Vector(v) => v.offset += d,
            LayerKind::Raster(r) => r.offset = (r.offset + d).round(),
        }
    }
}

#[derive(Clone, Default)]
pub struct Document {
    pub layers: Vec<Layer>,
    pub export_frame: Option<Rect>,
    pub next_id: LayerId,
}

impl Document {
    pub fn new_id(&mut self) -> LayerId {
        self.next_id += 1;
        self.next_id
    }

    pub fn index_of(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    pub fn get(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn get_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// Inserts above `above` (or at the top) and returns the new id.
    pub fn insert(&mut self, name: String, kind: LayerKind, above: Option<LayerId>) -> LayerId {
        let id = self.new_id();
        let layer = Layer { id, name, visible: true, locked: false, opacity: 1.0, kind };
        let idx = above.and_then(|a| self.index_of(a)).map(|i| i + 1).unwrap_or(self.layers.len());
        self.layers.insert(idx, layer);
        id
    }

    pub fn content_bounds(&self) -> Option<Rect> {
        let mut acc: Option<Rect> = None;
        for l in self.layers.iter().filter(|l| l.visible) {
            if let Some(b) = l.bounds() {
                acc = Some(acc.map_or(b, |a| a.union(b)));
            }
        }
        acc
    }

    pub fn count_kind(&self, f: impl Fn(&LayerKind) -> bool) -> usize {
        self.layers.iter().filter(|l| f(&l.kind)).count()
    }
}

pub type TileMap = HashMap<(i32, i32), Arc<crate::raster::Tile>>;
