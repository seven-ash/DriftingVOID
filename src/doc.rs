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
    /// A video frame as the export's decoder delivered it (NV12); `rgba` is then
    /// empty and the GPU converts it (the CPU path converts with `rgba_px`).
    pub nv12: Option<Arc<Nv12>>,
}

/// A decoded video frame: NV12 (luma, then interleaved chroma at half size),
/// BT.709 limited range (the decoder converts any source to that).
pub struct Nv12 {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl Nv12 {
    pub fn to_rgba(&self) -> image::RgbaImage {
        let (w, h) = (self.w as usize, self.h as usize);
        let (ys, uvs) = self.data.split_at(w * h);
        let mut out = vec![255u8; w * h * 4];
        for (y, row) in out.chunks_mut(w * 4).enumerate() {
            for x in 0..w {
                let yy = (ys[y * w + x] as f32 - 16.0) / 219.0;
                let i = (y / 2) * w + (x / 2) * 2;
                let cb = (uvs[i] as f32 - 128.0) / 224.0;
                let cr = (uvs[i + 1] as f32 - 128.0) / 224.0;
                let r = yy + 1.5748 * cr;
                let g = yy - 0.187324 * cb - 0.468124 * cr;
                let b = yy + 1.8556 * cb;
                for (k, v) in [r, g, b].into_iter().enumerate() {
                    row[x * 4 + k] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        image::RgbaImage::from_raw(self.w, self.h, out).unwrap()
    }
}

impl ImageSource {
    pub fn size(&self) -> Vec2 {
        match self.logical {
            Some([w, h]) => vec2(w as f32, h as f32),
            None => {
                let (w, h) = self.px_size();
                vec2(w as f32, h as f32)
            }
        }
    }

    /// The pixels' own size (of `rgba`, or of the NV12 frame).
    pub fn px_size(&self) -> (u32, u32) {
        match &self.nv12 {
            Some(n) => (n.w, n.h),
            None => self.rgba.dimensions(),
        }
    }

    /// The pixels as RGBA (an NV12 frame converted).
    pub fn rgba_px(&self) -> Arc<image::RgbaImage> {
        match &self.nv12 {
            Some(n) => Arc::new(n.to_rgba()),
            None => self.rgba.clone(),
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
    /// Leave its soundtrack out.
    #[serde(default)]
    pub mute: bool,
    /// Soundtrack volume (keyable).
    #[serde(default = "unit_f32")]
    pub volume: f32,
    /// Texture slot that stays the same while frames change (runtime only).
    #[serde(skip, default = "next_rev")]
    pub slot: u64,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Adjust {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    /// Chroma key: pixels near this colour turn transparent.
    #[serde(default)]
    pub chroma: Option<[u8; 3]>,
    /// How far (chroma distance, 0..1) still counts as the key colour.
    #[serde(default = "key_tol")]
    pub key_tol: f32,
    /// Width of the soft edge beyond `key_tol`.
    #[serde(default = "key_soft")]
    pub key_soft: f32,
}

fn key_tol() -> f32 {
    0.12
}

fn key_soft() -> f32 {
    0.08
}

impl Default for Adjust {
    fn default() -> Self {
        Self { brightness: 0.0, contrast: 0.0, saturation: 0.0, chroma: None, key_tol: key_tol(), key_soft: key_soft() }
    }
}

/// Chroma (Cb, Cr) of an sRGB colour, each about -0.5..0.5.
fn cbcr(r: f32, g: f32, b: f32) -> (f32, f32) {
    (-0.168_736 * r - 0.331_264 * g + 0.5 * b, 0.5 * r - 0.418_688 * g - 0.081_312 * b)
}

impl Adjust {
    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0 && self.contrast == 0.0 && self.saturation == 0.0 && self.chroma.is_none()
    }

    /// Only the tone values (brightness, contrast, saturation) back to neutral.
    pub fn reset_tone(&mut self) {
        self.brightness = 0.0;
        self.contrast = 0.0;
        self.saturation = 0.0;
    }

    pub fn key(&self) -> u64 {
        let b = (self.brightness * 1000.0) as i64 as u64;
        let c = (self.contrast * 1000.0) as i64 as u64;
        let s = (self.saturation * 1000.0) as i64 as u64;
        let k = self.chroma.map_or(0, |[r, g, b]| 1 + ((r as u64) << 16 | (g as u64) << 8 | b as u64))
            ^ ((self.key_tol * 1000.0) as u64).wrapping_mul(31_337)
            ^ ((self.key_soft * 1000.0) as u64).wrapping_mul(65_537);
        b.wrapping_mul(1_000_003) ^ c.wrapping_mul(7_919) ^ s.wrapping_mul(104_729) ^ k.wrapping_mul(2_654_435_761)
    }

    /// Chroma key: alpha falls to 0 where a pixel's chroma is within `key_tol`
    /// of the key colour, rising over `key_soft`; what is left near the edge
    /// loses the key colour's cast (spill).
    fn apply_key(&self, rgba: &mut [u8]) {
        let Some([kr, kg, kb]) = self.chroma else { return };
        let (kcb, kcr) = cbcr(kr as f32 / 255.0, kg as f32 / 255.0, kb as f32 / 255.0);
        let kl = (kcb * kcb + kcr * kcr).sqrt().max(1e-4);
        let (lo, hi) = (self.key_tol, self.key_tol + self.key_soft.max(1e-4));
        for px in rgba.chunks_exact_mut(4) {
            if px[3] == 0 {
                continue;
            }
            let (r, g, b) = (px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0);
            let (cb, cr) = cbcr(r, g, b);
            let d = ((cb - kcb).powi(2) + (cr - kcr).powi(2)).sqrt();
            if d >= hi {
                continue;
            }
            let x = ((d - lo) / (hi - lo)).clamp(0.0, 1.0);
            let a = x * x * (3.0 - 2.0 * x);
            px[3] = (px[3] as f32 * a).round() as u8;
            // spill: remove the part of the chroma that points at the key
            let along = (cb * kcb + cr * kcr) / kl;
            if along > 0.0 {
                let k = along * (1.0 - a) / kl;
                let (ncb, ncr) = (cb - kcb * k, cr - kcr * k);
                let y = 0.299 * r + 0.587 * g + 0.114 * b;
                let rgb = [y + 1.402 * ncr, y - 0.344_136 * ncb - 0.714_136 * ncr, y + 1.772 * ncb];
                for i in 0..3 {
                    px[i] = (rgb[i].clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
    }

    /// Applies the adjustment in place to straight-alpha RGBA bytes.
    pub fn apply(&self, rgba: &mut [u8]) {
        if self.is_identity() {
            return;
        }
        self.apply_key(rgba);
        if self.brightness == 0.0 && self.contrast == 0.0 && self.saturation == 0.0 {
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
    pub align: TextAlign,
}

/// Where the lines of a text box sit across it.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash, Debug, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    pub const ALL: [TextAlign; 3] = [TextAlign::Left, TextAlign::Center, TextAlign::Right];

    /// 0 left, 0.5 centre, 1 right.
    pub fn factor(self) -> f32 {
        match self {
            TextAlign::Left => 0.0,
            TextAlign::Center => 0.5,
            TextAlign::Right => 1.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TextAlign::Left => "왼쪽 정렬",
            TextAlign::Center => "가운데 정렬",
            TextAlign::Right => "오른쪽 정렬",
        }
    }
}

impl TextLayer {
    /// After the box changed size (typing, font, size...), keeps its anchor —
    /// the top of the side the lines align to (left, middle or right) — where
    /// it was, turned with the box, so the text grows away from it.
    pub fn keep_corner(&mut self, old_size: Vec2) {
        let new_size = crate::text::layout(self).size;
        let f = self.align.factor();
        // anchor = pos + size/2 + R·(L - size/2) with L = (f·w, 0)
        let (s, c) = self.rotation.sin_cos();
        let r = |v: Vec2| vec2(v.x * c - v.y * s, v.x * s + v.y * c);
        let at = |pos: Pos2, size: Vec2| pos + size * 0.5 + r(vec2(f * size.x, 0.0) - size * 0.5);
        let want = at(self.pos, old_size);
        self.pos += want - at(self.pos, new_size);
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

/// A sound on the canvas: transparent, shown in the editor as a gizmo at `pos`
/// (never exported). Its time on the timeline is the layer's span, and the
/// source time at the span's start is `anim.video_start`, as for a video.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct AudioLayer {
    pub path: String,
    pub pos: Pos2,
    /// Length of the source in seconds; 0 until it is decoded the first time.
    pub len: f64,
    #[serde(default = "unit_f32")]
    pub volume: f32,
    #[serde(default)]
    pub mute: bool,
}

impl AudioLayer {
    /// The gizmo's box (world units; it scales with the canvas like any layer).
    pub const SIZE: f32 = 64.0;

    pub fn rect(&self) -> Rect {
        Rect::from_center_size(self.pos, Vec2::splat(Self::SIZE))
    }
}

/// A filter of an FX layer (`docs/updates/fx-layers.md`).
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash, Debug, Serialize, Deserialize)]
pub enum Filter {
    #[default]
    Blur,
    BrightContrast,
    Hsl,
    Levels,
    Curves,
    Desaturate,
    Invert,
    Posterize,
    Threshold,
    GradientMap,
}

/// One value of a filter: label, range, default, dial step, suffix.
pub struct FxParam {
    pub name: &'static str,
    pub lo: f32,
    pub hi: f32,
    pub default: f32,
    pub step: f32,
    pub suffix: &'static str,
}

impl Filter {
    pub const ALL: [Filter; 10] = [
        Filter::Blur,
        Filter::BrightContrast,
        Filter::Hsl,
        Filter::Levels,
        Filter::Curves,
        Filter::Desaturate,
        Filter::Invert,
        Filter::Posterize,
        Filter::Threshold,
        Filter::GradientMap,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Filter::Blur => "가우시안 흐림",
            Filter::BrightContrast => "밝기/대비",
            Filter::Hsl => "색조/채도/명도",
            Filter::Levels => "레벨",
            Filter::Curves => "커브",
            Filter::Desaturate => "흑백",
            Filter::Invert => "반전",
            Filter::Posterize => "포스터화",
            Filter::Threshold => "임계값",
            Filter::GradientMap => "그라디언트 맵",
        }
    }

    /// The point filters' code in `fs_fx` (pass 2); None for the others.
    pub fn point_code(self) -> Option<f32> {
        Some(match self {
            Filter::Blur => return None,
            Filter::BrightContrast => 0.0,
            Filter::Hsl => 1.0,
            Filter::Levels => 2.0,
            Filter::Curves => 3.0,
            Filter::Desaturate => 4.0,
            Filter::Invert => 5.0,
            Filter::Posterize => 6.0,
            Filter::Threshold => 7.0,
            Filter::GradientMap => 8.0,
        })
    }

    /// Its values, in the order of `FxLayer::v` (at most four; sizes in
    /// canvas pixels so every zoom and export scale agree).
    pub fn params(self) -> &'static [FxParam] {
        match self {
            Filter::Blur => &[FxParam { name: "반경", lo: 0.0, hi: 200.0, default: 8.0, step: 0.5, suffix: " px" }],
            Filter::BrightContrast => &[
                FxParam { name: "밝기", lo: -1.0, hi: 1.0, default: 0.0, step: 0.01, suffix: "" },
                FxParam { name: "대비", lo: -1.0, hi: 1.0, default: 0.0, step: 0.01, suffix: "" },
            ],
            Filter::Hsl => &[
                FxParam { name: "색조", lo: -180.0, hi: 180.0, default: 0.0, step: 1.0, suffix: "°" },
                FxParam { name: "채도", lo: -1.0, hi: 1.0, default: 0.0, step: 0.01, suffix: "" },
                FxParam { name: "명도", lo: -1.0, hi: 1.0, default: 0.0, step: 0.01, suffix: "" },
            ],
            Filter::Levels => &[
                FxParam { name: "검정점", lo: 0.0, hi: 1.0, default: 0.0, step: 0.01, suffix: "" },
                FxParam { name: "감마", lo: 0.1, hi: 4.0, default: 1.0, step: 0.01, suffix: "" },
                FxParam { name: "흰점", lo: 0.0, hi: 1.0, default: 1.0, step: 0.01, suffix: "" },
                FxParam { name: "출력 검정", lo: 0.0, hi: 1.0, default: 0.0, step: 0.01, suffix: "" },
            ],
            // three points of a monotone curve through (0, 0) and (1, 1)
            Filter::Curves => &[
                FxParam { name: "어두운 곳", lo: 0.0, hi: 1.0, default: 0.25, step: 0.01, suffix: "" },
                FxParam { name: "중간", lo: 0.0, hi: 1.0, default: 0.5, step: 0.01, suffix: "" },
                FxParam { name: "밝은 곳", lo: 0.0, hi: 1.0, default: 0.75, step: 0.01, suffix: "" },
            ],
            Filter::Desaturate => &[FxParam { name: "양", lo: 0.0, hi: 1.0, default: 1.0, step: 0.01, suffix: "" }],
            Filter::Invert | Filter::GradientMap => &[],
            Filter::Posterize => &[FxParam { name: "단계", lo: 2.0, hi: 32.0, default: 4.0, step: 1.0, suffix: "" }],
            Filter::Threshold => &[
                FxParam { name: "기준", lo: 0.0, hi: 1.0, default: 0.5, step: 0.01, suffix: "" },
                FxParam { name: "부드러움", lo: 0.0, hi: 0.5, default: 0.0, step: 0.01, suffix: "" },
            ],
        }
    }

    pub fn defaults(self) -> [f32; 4] {
        let mut v = [0.0; 4];
        for (i, p) in self.params().iter().enumerate() {
            v[i] = p.default;
        }
        v
    }
}

/// A filter instead of pixels: what is below it in the list, filtered (Ash,
/// 10-28). `blend` None is 복사 (the filtered picture replaces what is below,
/// alpha too); a mode lays it over the original with that mode. Opacity
/// mixes the original and the result; a mask limits where.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FxLayer {
    pub filter: Filter,
    pub v: [f32; 4],
    #[serde(default)]
    pub blend: Option<Blend>,
    /// 그라디언트 맵: the dark and the light end (straight sRGBA).
    #[serde(default = "fx_colors")]
    pub colors: [[u8; 4]; 2],
}

fn fx_colors() -> [[u8; 4]; 2] {
    [[0, 0, 0, 255], [255, 255, 255, 255]]
}

impl FxLayer {
    pub fn new(filter: Filter) -> Self {
        FxLayer { filter, v: filter.defaults(), blend: None, colors: fx_colors() }
    }

    /// How far the filter reads around a pixel, in canvas units (0: the pixel
    /// alone). A blur's radius is 2 sigma; it reads 3 sigma.
    pub fn reach(&self) -> f32 {
        match self.filter {
            Filter::Blur => self.v[0].max(0.0) * 1.5,
            _ => 0.0,
        }
    }

    /// Does nothing at these values (skipped when drawing).
    pub fn is_noop(&self) -> bool {
        match self.filter {
            Filter::Blur => self.v[0] <= 0.0,
            Filter::BrightContrast => self.v[0] == 0.0 && self.v[1] == 0.0,
            Filter::Hsl => self.v[..3] == [0.0; 3],
            Filter::Levels => self.v == [0.0, 1.0, 1.0, 0.0],
            Filter::Curves => self.v[..3] == [0.25, 0.5, 0.75],
            Filter::Desaturate => self.v[0] <= 0.0,
            Filter::Invert | Filter::Posterize | Filter::Threshold | Filter::GradientMap => false,
        }
    }
}

#[derive(Clone)]
pub enum LayerKind {
    Image(ImageLayer),
    Text(TextLayer),
    Shape(ShapeLayer),
    Vector(VectorLayer),
    Raster(RasterLayer),
    Audio(AudioLayer),
    Fx(FxLayer),
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
    /// This (raster) layer is the mask of that layer: its alpha is the coverage,
    /// in 2D canvas space. It is not drawn itself.
    pub mask_of: Option<LayerId>,
    /// A mask moves with this layer: (layer, its centre when the mask was last moved).
    pub mask_anchor: Option<(LayerId, [f32; 2])>,
    /// The anchor's rotation and extent when the mask was last turned/scaled.
    pub mask_pose: Option<[f32; 2]>,
    /// A mask that shows only what is painted (hides the rest).
    pub mask_invert: bool,
    /// A mask's pixels per world unit at its pose (1: canvas pixels). Other
    /// than 1 for a mask made on a photo: its pixels are that photo's own
    /// (the anchor is the photo), so they follow it through any move, turn
    /// or resize without being resampled.
    pub mask_res: f32,
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
                let c = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()];
                let h = pl.homography(cam);
                let mut pts = crate::persp::project_poly(&h, &c)?;
                // a thick layer reaches to its back face (masks and groups are
                // put down over these bounds; the sides must not be cut). An
                // engraving shows only through its own shape on the plane.
                if pl.thick > 0.0 && !pl.engrave {
                    let hb = pl.homography_at(cam, pl.thick);
                    pts.extend(crate::persp::project_poly(&hb, &c).unwrap_or_default());
                }
                Some(Rect::from_points(&pts))
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
            LayerKind::Audio(_) => "소리",
            LayerKind::Fx(_) => "FX",
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
            LayerKind::Audio(a) => Some(a.rect()),
            // the whole picture; nothing to pick on the canvas
            LayerKind::Fx(_) => None,
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
            // a photo mask stays on its photo's pixels (it moves with the photo)
            LayerKind::Raster(_) if self.mask_res != 1.0 => {}
            LayerKind::Raster(r) => r.offset = (r.offset + d).round(),
            LayerKind::Audio(a) => a.pos += d,
            LayerKind::Fx(_) => {}
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
    /// Every shared pixel/point buffer of the document as (address, bytes).
    /// Snapshots share these, so history memory is the bytes of distinct addresses.
    pub fn blobs(&self, f: &mut impl FnMut(usize, usize)) {
        for l in &self.layers {
            match &l.kind {
                LayerKind::Image(im) => {
                    f(Arc::as_ptr(&im.src.rgba) as usize, im.src.rgba.as_raw().len());
                    f(Arc::as_ptr(&im.src.encoded) as usize, im.src.encoded.len());
                }
                LayerKind::Vector(v) => {
                    for s in &v.strokes {
                        f(Arc::as_ptr(s) as usize, s.pts.len() * 12 + s.tris.len() * 8 + 64);
                    }
                }
                LayerKind::Raster(r) => {
                    for t in r.tiles.values() {
                        f(Arc::as_ptr(t) as usize, t.px.len() + 32);
                    }
                    for t in r.rec_base.iter().flat_map(|m| m.values()) {
                        f(Arc::as_ptr(t) as usize, t.px.len() + 32);
                    }
                    for rec in &r.recs {
                        f(Arc::as_ptr(rec) as usize, rec.pts.len() * 12 + rec.times.len() * 4 + 64);
                    }
                }
                _ => {}
            }
        }
    }

    /// Bytes of `self`'s shared buffers that `other` does not hold.
    pub fn bytes_not_in(&self, other: &Document) -> usize {
        let mut seen = std::collections::HashSet::new();
        other.blobs(&mut |p, _| {
            seen.insert(p);
        });
        let mut n = 0;
        let mut mine = std::collections::HashSet::new();
        self.blobs(&mut |p, b| {
            if !seen.contains(&p) && mine.insert(p) {
                n += b;
            }
        });
        n + 4096
    }

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
        let layer = Layer { id, name, visible: true, locked: false, opacity: 1.0, kind, place: None, anim: None, mask_of: None, mask_anchor: None, mask_pose: None, mask_invert: false, mask_res: 1.0 };
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
        // sounds draw nothing in a picture
        for l in self.layers.iter().filter(|l| l.visible && l.mask_of.is_none() && !matches!(l.kind, LayerKind::Audio(_))) {
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
    fn history_counts_only_new_buffers() {
        let mut r = crate::raster::RasterLayer::default();
        r.fill_rect(Rect::from_min_size(Pos2::ZERO, vec2(10.0, 10.0)), true);
        let mut a = Document::default();
        let id = a.insert("b".into(), LayerKind::Raster(r), None);
        let b = a.clone();
        // a clone shares every tile
        assert_eq!(a.bytes_not_in(&b), 4096);
        let mut c = a.clone();
        if let Some(Layer { kind: LayerKind::Raster(r), .. }) = c.get_mut(id) {
            r.fill_rect(Rect::from_min_size(pos2(300.0, 0.0), vec2(10.0, 10.0)), true);
        }
        // one new tile in c
        assert!(c.bytes_not_in(&a) > 256 * 256 * 4);
        assert_eq!(a.bytes_not_in(&c), 4096);
    }

    #[test]
    fn centred_text_grows_both_ways() {
        let mut t = TextLayer { pos: pos2(100.0, 50.0), text: "ab".into(), size: 40.0, color: Color32::WHITE, family: String::new(), style: "Regular".into(), bg: None, wrap: 0.0, rotation: 0.0, blend: Blend::Normal, align: TextAlign::Center };
        let mid = |t: &TextLayer| t.pos.x + crate::text::layout(t).size.x * 0.5;
        let before = mid(&t);
        let s0 = crate::text::layout(&t).size;
        t.text = "abcdef".into();
        t.keep_corner(s0);
        assert!((mid(&t) - before).abs() < 1e-3);
        // right: the right edge stays
        t.align = TextAlign::Right;
        let right = |t: &TextLayer| t.pos.x + crate::text::layout(t).size.x;
        let r0 = right(&t);
        let s0 = crate::text::layout(&t).size;
        t.text = "a".into();
        t.keep_corner(s0);
        assert!((right(&t) - r0).abs() < 1e-3);
    }

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
            align: TextAlign::Left,
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

fn unit_f32() -> f32 {
    1.0
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn chroma_key_clears_the_key_colour_only() {
        let a = Adjust { chroma: Some([20, 200, 40]), ..Default::default() };
        // green screen, a near green, skin, white
        let mut px = vec![20, 200, 40, 255, 40, 190, 60, 255, 220, 170, 140, 255, 255, 255, 255, 255];
        a.apply(&mut px);
        assert_eq!(px[3], 0, "key colour gone");
        assert!(px[7] < 40, "near colour mostly gone: {}", px[7]);
        assert_eq!(px[11], 255, "skin stays");
        assert_eq!(px[15], 255, "white stays");
        assert_eq!(&px[8..11], &[220, 170, 140], "far colours keep their RGB");
    }
}
