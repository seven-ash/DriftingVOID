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
    /// Real pixel size when `rgba` is a smaller preview (video frames); geometry and
    /// crop are in these units.
    pub logical: Option<[u32; 2]>,
}

impl ImageSource {
    pub fn size(&self) -> Vec2 {
        match self.logical {
            Some([w, h]) => vec2(w as f32, h as f32),
            None => vec2(self.rgba.width() as f32, self.rgba.height() as f32),
        }
    }
}

fn one() -> f64 {
    1.0
}

/// A video shown by an image layer: the layer's picture is the frame at `time`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoRef {
    pub path: String,
    pub info: crate::video::VideoInfo,
    pub time: f64,
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default)]
    pub looping: bool,
    /// Texture slot that stays the same while frames change (runtime only).
    #[serde(skip, default = "next_rev")]
    pub slot: u64,
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
    pub video: Option<VideoRef>,
}

impl ImageLayer {
    pub fn new(src: ImageSource, center: Pos2) -> Self {
        let crop = Rect::from_min_size(Pos2::ZERO, src.size());
        Self { src, center, scale: 1.0, rotation: 0.0, crop, adjust: Adjust::default(), video: None }
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
    pub blend: Blend,
}

impl TextLayer {
    /// After the box changed size (typing, font, size...), keeps the rotated
    /// top-left corner — where the first character sits — where it was, so the
    /// text grows away from it along its own axes instead of around the centre.
    pub fn keep_corner(&mut self, old_size: Vec2) {
        if self.rotation == 0.0 {
            return;
        }
        let new_size = crate::text::layout(self).size;
        let d = (old_size - new_size) * 0.5;
        // corner = pos + (I - R)·size/2, so shift pos by (I - R)·(old - new)/2
        let (s, c) = self.rotation.sin_cos();
        let rd = vec2(d.x * c - d.y * s, d.x * s + d.y * c);
        self.pos += d - rd;
    }
}

/// How a text layer mixes with what is under it.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash, Debug, Serialize, Deserialize)]
pub enum Blend {
    #[default]
    Normal,
    Add,
    Subtract,
    Multiply,
    Divide,
    Difference,
    /// Punches the shape out of everything below.
    Erase,
}

impl Blend {
    pub const ALL: [Blend; 7] = [Blend::Normal, Blend::Add, Blend::Subtract, Blend::Multiply, Blend::Divide, Blend::Difference, Blend::Erase];

    pub fn label(self) -> &'static str {
        match self {
            Blend::Normal => "보통",
            Blend::Add => "더하기",
            Blend::Subtract => "빼기",
            Blend::Multiply => "곱하기",
            Blend::Divide => "나누기",
            Blend::Difference => "차이",
            Blend::Erase => "지우기",
        }
    }

    /// Shader / compositor mode number.
    pub fn code(self) -> i32 {
        self as i32
    }

    /// Separable blend on straight colour channels (0..1).
    pub fn mix(self, d: f32, s: f32) -> f32 {
        match self {
            Blend::Normal | Blend::Erase => s,
            Blend::Add => (d + s).min(1.0),
            Blend::Subtract => (d - s).max(0.0),
            Blend::Multiply => d * s,
            Blend::Divide => (d / s.max(1.0 / 255.0)).min(1.0),
            Blend::Difference => (d - s).abs(),
        }
    }
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
    /// Set when the layer lives in a 3D space (drawn through that space's camera).
    pub place: Option<crate::persp::Placement>,
    /// Keyframes and time span on the timeline.
    pub anim: Option<crate::anim::Anim>,
}

impl Layer {
    /// Where the layer shows on the canvas: its 2D bounds, or for a placed layer
    /// the bounding box of its projected quad (None when it is behind the camera).
    pub fn shown_bounds(&self, cams: &HashMap<u64, crate::persp::Camera>) -> Option<Rect> {
        let b = self.bounds()?;
        match &self.place {
            None => Some(b),
            Some(pl) => {
                // an unsolvable space shows its layers flat
                let Some(cam) = cams.get(&pl.space) else { return Some(b) };
                let h = pl.homography(cam);
                let pts: Option<Vec<Pos2>> = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()]
                    .iter()
                    .map(|p| crate::persp::apply_h(&h, *p))
                    .collect();
                pts.map(|p| Rect::from_points(&p))
            }
        }
    }

    /// Outline on the canvas (world points, closed): bounds corners, projected
    /// when the layer is placed in a space.
    pub fn shown_quad(&self, cams: &HashMap<u64, crate::persp::Camera>) -> Option<[Pos2; 4]> {
        let b = self.bounds()?;
        let c = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()];
        match &self.place {
            None => Some(c),
            Some(pl) => {
                let Some(cam) = cams.get(&pl.space) else { return Some(c) };
                let h = pl.homography(cam);
                Some([
                    crate::persp::apply_h(&h, c[0])?,
                    crate::persp::apply_h(&h, c[1])?,
                    crate::persp::apply_h(&h, c[2])?,
                    crate::persp::apply_h(&h, c[3])?,
                ])
            }
        }
    }

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
    /// Perspective-matched 3D spaces (docs/SPACES.md).
    pub spaces: Vec<crate::persp::Space>,
    pub timeline: crate::anim::Timeline,
    /// Canvas cameras (regions over time).
    pub cameras: Vec<crate::anim::CanvasCam>,
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
        let layer = Layer { id, name, visible: true, locked: false, opacity: 1.0, kind, place: None, anim: None };
        let idx = above.and_then(|a| self.index_of(a)).map(|i| i + 1).unwrap_or(self.layers.len());
        self.layers.insert(idx, layer);
        id
    }

    /// Shown at the playhead (layers with a span only exist inside it while the
    /// timeline is on).
    pub fn alive(&self, l: &Layer) -> bool {
        !self.timeline.on || l.anim.as_ref().is_none_or(|a| a.alive(self.timeline.time))
    }

    /// Layers in drawing order. Layers placed in a space are drawn together, far to
    /// near by camera depth, where the space's lowest member sits in the stack.
    pub fn draw_order<'a>(&'a self, cams: &HashMap<u64, crate::persp::Camera>) -> Vec<&'a Layer> {
        let mut out = Vec::with_capacity(self.layers.len());
        let mut done: Vec<u64> = Vec::new();
        for l in self.layers.iter().filter(|l| self.alive(l)) {
            match l.place.as_ref().filter(|pl| cams.contains_key(&pl.space)) {
                None => out.push(l),
                Some(pl) if done.contains(&pl.space) => {}
                Some(pl) => {
                    let cam = &cams[&pl.space];
                    let mut members: Vec<(&Layer, f64)> = self
                        .layers
                        .iter()
                        .filter(|m| self.alive(m))
                        .filter_map(|m| m.place.as_ref().filter(|p| p.space == pl.space).map(|p| (m, p.depth(cam))))
                        .collect();
                    members.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    out.extend(members.into_iter().map(|(m, _)| m));
                    done.push(pl.space);
                }
            }
        }
        out
    }

    pub fn content_bounds(&self) -> Option<Rect> {
        let cams = crate::persp::cameras(&self.spaces);
        let mut acc: Option<Rect> = None;
        for l in self.layers.iter().filter(|l| l.visible) {
            // layers in a space only show inside its mask
            let clip = l.place.as_ref().and_then(|pl| self.spaces.iter().find(|s| s.id == pl.space)).map(|s| s.clip());
            if let Some(b) = l.shown_bounds(&cams).map(|b| clip.map_or(b, |c| b.intersect(c))).filter(|b| b.is_positive()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_text_grows_from_its_first_corner() {
        let mut t = TextLayer {
            pos: pos2(100.0, 50.0),
            text: "ab".into(),
            size: 40.0,
            color: Color32::WHITE,
            family: String::new(),
            style: "Regular".into(),
            bg: None,
            wrap: 0.0,
            rotation: 0.6,
            blend: Blend::Normal,
        };
        let corner = |t: &TextLayer| {
            let s = crate::text::layout(t).size;
            rot_about(t.pos, t.pos + s * 0.5, t.rotation)
        };
        let before = corner(&t);
        let s0 = crate::text::layout(&t).size;
        t.text = "ab cd ef\nsecond line".into();
        t.keep_corner(s0);
        let after = corner(&t);
        assert!((before - after).length() < 1e-3, "{before:?} vs {after:?}");
    }
}
