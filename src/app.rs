use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{
    Align2, Color32, FontFamily, FontId, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
    TouchId, TouchPhase, Vec2, pos2, vec2,
};

use crate::canvas::{self, TexCache, Xf};
use crate::theme::{self, ACCENT, ACCENT_INK, FILL_1, FILL_2, FILL_3, INK, INK_2, INK_3, PAPER_4, SPACE_LG, SPACE_MD, SPACE_SM};
use crate::doc::*;
use crate::raster::{BrushParams, RasterLayer, RasterStroke};
use crate::vector::{VStroke, radius_for};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Lasso,
    Hand,
    Brush,
    Pen,
    Eraser,
    Text,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Crop,
    Frame,
    Space,
    Picker,
}

const TOOLS: &[(Tool, &str, &str, Key)] = &[
    (Tool::Select, "선택", "V", Key::V),
    (Tool::Lasso, "올가미 선택", "Q", Key::Q),
    (Tool::Hand, "화면 이동", "H", Key::H),
    (Tool::Brush, "브러시", "B", Key::B),
    (Tool::Pen, "펜 (벡터)", "P", Key::P),
    (Tool::Eraser, "지우개", "E", Key::E),
    (Tool::Text, "글자", "T", Key::T),
    (Tool::Line, "선", "L", Key::L),
    (Tool::Arrow, "화살표", "A", Key::A),
    (Tool::Rect, "사각형", "R", Key::R),
    (Tool::Ellipse, "원", "O", Key::O),
    (Tool::Crop, "사진 자르기", "C", Key::C),
    (Tool::Frame, "내보낼 영역", "F", Key::F),
    (Tool::Space, "3D 공간", "K", Key::K),
    (Tool::Picker, "스포이드", "I", Key::I),
];

const PALETTE: &[Color32] = &[
    Color32::from_rgb(20, 20, 20),
    Color32::from_rgb(255, 255, 255),
    Color32::from_rgb(230, 57, 70),
    Color32::from_rgb(244, 140, 6),
    Color32::from_rgb(255, 209, 59),
    Color32::from_rgb(82, 183, 136),
    Color32::from_rgb(58, 134, 255),
    Color32::from_rgb(131, 56, 236),
    Color32::from_rgb(255, 0, 110),
    Color32::from_rgb(120, 120, 120),
];

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub color: Color32,
    pub brush_size: f32,
    pub brush_opacity: f32,
    pub brush_hardness: f32,
    pub brush_pressure_size: bool,
    pub brush_pressure_opacity: bool,
    /// Last picked brush preset (its values are the reset defaults); empty = none.
    pub brush_preset: String,
    pub pen_size: f32,
    pub pen_pressure: bool,
    pub eraser_size: f32,
    pub eraser_pressure: bool,
    pub smoothing: f32,
    pub shape_width: f32,
    pub shape_fill: bool,
    pub fill_color: Color32,
    pub text_size: f32,
    pub text_family: String,
    pub text_style: String,
    #[serde(default)]
    pub text_align: crate::doc::TextAlign,
    pub text_bg: bool,
    pub text_bg_color: Color32,
    pub touch_draw: bool,
    /// 0 light, 1 dark, 2 follow the system.
    pub theme: u8,
    /// Seconds Tab must be held before the other workspace peeks in.
    pub peek_delay: f32,
    /// Folded property sections: tool, selection, cameras.
    pub folded: [bool; 3],
    /// Colour tolerance of the mask's automatic selection (0..100).
    pub wand_tol: f32,
    /// Mask edge: guided-filter radius (0 = hard edge) and grow (+) / shrink (-) in px.
    pub mask_refine: f32,
    pub mask_grow: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            color: Color32::from_rgb(230, 57, 70),
            brush_size: 12.0,
            brush_opacity: 1.0,
            brush_hardness: 0.8,
            brush_pressure_size: true,
            brush_pressure_opacity: false,
            brush_preset: String::new(),
            pen_size: 4.0,
            pen_pressure: true,
            eraser_size: 30.0,
            eraser_pressure: false,
            smoothing: 0.4,
            shape_width: 4.0,
            shape_fill: false,
            fill_color: Color32::from_rgba_unmultiplied(255, 209, 59, 160),
            text_size: 40.0,
            text_family: String::new(),
            text_style: "Regular".into(),
            text_align: Default::default(),
            text_bg: false,
            text_bg_color: Color32::from_rgba_unmultiplied(255, 255, 255, 220),
            touch_draw: false,
            theme: 1,
            peek_delay: 0.2,
            folded: [false; 3],
            wand_tol: 24.0,
            mask_refine: 2.0,
            mask_grow: 1.0,
        }
    }
}

/// Intelligent scissors in progress: the outline so far and the live wire from
/// its last anchor.
struct Scissor {
    mid: LayerId,
    on: bool,
    pts: Vec<Pos2>,
    wire: Option<crate::segment::LiveWire>,
}

enum Drag {
    Pan,
    Move { id: LayerId, start: Pos2, orig: Layer },
    ScaleImage { id: LayerId, sign: Vec2, orig: ImageLayer },
    Rotate { id: LayerId, orig: ImageLayer },
    ShapePt { id: LayerId, which: u8 },
    TextWrap { id: LayerId },
    NewShape { id: LayerId, start: Pos2 },
    Crop { id: LayerId, handle: u8, start_src: Pos2, orig: ImageLayer },
    Frame { handle: u8, start: Pos2, orig: Rect },
    Raster { id: LayerId, stroke: Option<RasterStroke>, smooth: (Pos2, f32) },
    Vector { id: LayerId, pts: Vec<[f32; 3]>, smooth: (Pos2, f32), base_r: f32, pressure: bool, color: Color32 },
    VErase { id: LayerId, r: f32 },
    Zoom { anchor: Pos2 },
    Size { anchor: Pos2, start: f32, x0: f32 },
    /// `full`: Ctrl held — only items entirely inside count (else: centre inside).
    Marquee { start: Pos2, cur: Pos2, full: bool },
    /// Free-form selection outline (world points).
    Lasso { pts: Vec<Pos2> },
    MoveMany { start: Pos2, origs: Vec<(Layer, Part)>, d: Vec2 },
    /// Group scale/rotate about the selection's common centre.
    Group { rotate: bool, center: Pos2, start: Pos2, box_: Rect, origs: Vec<(Layer, Part)>, s: f32, th: f32 },
    /// 3D space frame (handle 0..7 as `rect_handles`; a zero-size `orig` = new space).
    SpaceFrame { id: u64, handle: u8, start: Pos2, orig: Rect },
    /// Vanishing line: `end` 0 = a, 1 = b, 2 = whole line.
    SpaceLine { id: u64, idx: usize, end: u8, start: Pos2, orig: crate::persp::VLine },
    SpaceMask { id: u64, start: Pos2 },
    SpaceRef { id: u64 },
    SpaceTape { id: u64 },
    WallRect { sid: u64, wid: u64, start: Pos2 },
    /// A wall standing on a base line drawn on the floor.
    WallBase { sid: u64, wid: u64, start: Pos2 },
    /// Canvas camera rectangle (handles as `rect_handles`, 8 = move).
    Cam { id: u64, handle: u8, start: Pos2, orig: Rect },
    WallCorner { sid: u64, wid: u64, idx: usize },
}

/// Which part of a layer a selection covers.
#[derive(Clone)]
pub(crate) enum Part {
    Whole,
    /// Indices of selected pen strokes.
    Strokes(Vec<usize>),
    /// World polygons of selected brush pixels (boxes are 4-point polygons).
    Pixels(Vec<Vec<Pos2>>),
}

/// G/R/S on a flat selection (layers, pen strokes, brush pixels): move, rotate and
/// scale about the selection's centre, like the group handles but from the keyboard.
struct Grs2d {
    op: crate::view3d::Op,
    origs: Vec<(Layer, Part)>,
    /// Pointer (screen) when it started.
    start: Pos2,
    /// Pivot (world).
    center: Pos2,
    /// 0 = X, 1 = Y.
    axis: Option<usize>,
    typed: String,
    s: f32,
    th: f32,
    d: Vec2,
}

impl Grs2d {
    fn typed_value(&self) -> Option<f32> {
        let t = self.typed.trim_start_matches('-');
        let v: f32 = t.parse().ok()?;
        Some(if self.typed.starts_with('-') { -v } else { v })
    }

    fn label(&self) -> String {
        use crate::view3d::Op;
        let op = match self.op {
            Op::Grab => "이동",
            Op::Rotate => "회전",
            Op::Scale => "크기",
        };
        let axis = match self.axis {
            Some(0) => " · X 축",
            Some(_) => " · Y 축",
            None => "",
        };
        let typed = if self.typed.is_empty() { String::new() } else { format!(" · {}", self.typed) };
        format!("{op}{axis}{typed}   X/Y 축 · 숫자 입력 · Ctrl 스냅 · Shift 정밀 · Enter/클릭 확정 · Esc/우클릭 취소")
    }
}

/// A video layer that is playing: its decoder and the clock it follows.
struct Playback {
    player: crate::video::Player,
    /// Video time and wall time when it started, and the speed.
    t0: f64,
    wall0: f64,
    speed: f64,
    /// A decoded frame that is not due yet.
    pending: Option<(f64, image::RgbaImage)>,
}

/// A row of the timeline: a layer or a canvas camera.
#[derive(Clone, Copy, PartialEq, Debug)]
enum TlRow {
    Layer(LayerId),
    Cam(u64),
    Audio(u64),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum TlDrag {
    Scrub,
    Key { row: TlRow, from: f64 },
    SpanIn(TlRow),
    SpanOut(TlRow),
    /// Audio clip: move (grabbed at `grab` seconds into it) or trim either end.
    AudioMove { id: u64, grab: f64 },
    /// Shift-drag box over the tracks.
    Marquee { start: Pos2 },
    /// Several selected keys moving together (last snapped time under the pointer).
    Keys { last: f64 },
    AudioIn(u64),
    AudioOut(u64),
}

/// Audio clip colour on the timeline.
const AUDIO_COLOR: Color32 = Color32::from_rgb(90, 200, 170);

/// Canvas camera colour.
const CAM_COLOR: Color32 = Color32::from_rgb(176, 140, 255);

enum VidAct {
    Play(bool),
    Step(i32),
    Seek(f64),
    Restart,
    Still,
}

/// One undo/redo state: the document, the selection it had, and the bytes it
/// alone holds (shared buffers not in the state after it).
struct Snap {
    doc: Document,
    sel: Option<LayerId>,
    extra: Vec<LayerId>,
    sub: SubSel,
    cost: usize,
}

/// History limits: memory first (distinct buffers), then a step count.
const UNDO_BYTES: usize = 1 << 30;
const UNDO_STEPS: usize = 500;

/// Selections inside drawing layers (pen strokes, brush pixel boxes).
#[derive(Clone, Default)]
struct SubSel {
    strokes: Vec<(LayerId, Vec<usize>)>,
    pixels: Vec<(LayerId, Vec<Vec<Pos2>>)>,
}

impl SubSel {
    fn is_empty(&self) -> bool {
        self.strokes.is_empty() && self.pixels.is_empty()
    }
}

/// Applies p -> c + R(th)·s·(p - c) + d to the selected part of a layer,
/// always starting from the untouched original.
pub(crate) fn apply_part(o: &Layer, part: &Part, c: Pos2, s: f32, th: f32, d: Vec2) -> Layer {
    let plain_move = s == 1.0 && th == 0.0;
    match part {
        Part::Whole => {
            let mut l = if plain_move { o.clone() } else { transform_layer(o, c, s, th) };
            if d != Vec2::ZERO {
                l.translate(d);
            }
            l
        }
        Part::Strokes(idx) => {
            let mut l = o.clone();
            if let LayerKind::Vector(v) = &mut l.kind {
                let (sin, cos) = th.sin_cos();
                let off = v.offset;
                let map = |p: Pos2| {
                    let q = (p - c) * s;
                    c + vec2(q.x * cos - q.y * sin, q.x * sin + q.y * cos) + d
                };
                for &i in idx {
                    if let Some(st) = v.strokes.get(i) {
                        let pts = st
                            .pts
                            .iter()
                            .map(|q| {
                                let p = map(pos2(q[0], q[1]) + off) - off;
                                [p.x, p.y, q[2] * s]
                            })
                            .collect();
                        v.strokes[i] = Arc::new(VStroke::new(pts, st.color).with_rec(st.rec.clone()));
                    }
                }
            }
            l
        }
        Part::Pixels(polys) => {
            let mut l = o.clone();
            if let LayerKind::Raster(r) = &mut l.kind {
                let (mut patch, mut rest) = r.lift(polys);
                if plain_move {
                    patch.offset += d.round();
                } else if let Some(t) = crate::raster::transformed(&patch, c, s, th) {
                    patch = t;
                }
                rest.composite_over(&patch);
                *r = rest;
            }
            l
        }
    }
}

/// Where a selected pixel outline ends up after the same transform.
fn map_poly(poly: &[Pos2], c: Pos2, s: f32, th: f32, d: Vec2) -> Vec<Pos2> {
    if s == 1.0 && th == 0.0 {
        return poly.iter().map(|p| *p + d.round()).collect();
    }
    let (sin, cos) = th.sin_cos();
    poly.iter()
        .map(|p| {
            let q = (*p - c) * s;
            c + vec2(q.x * cos - q.y * sin, q.x * sin + q.y * cos) + d
        })
        .collect()
}

fn rect_poly(r: Rect) -> Vec<Pos2> {
    vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
}

/// Even-odd point-in-polygon test.
fn in_poly(p: Pos2, poly: &[Pos2]) -> bool {
    let mut inside = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a.y <= p.y) != (b.y <= p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Mouse,
    Touch(TouchId),
    /// Pen hovering with its side button held.
    PenHover,
    /// Middle mouse button (also what tablet drivers send for a pen button).
    Middle,
}

/// What the palette is editing.
#[derive(Clone, Copy, PartialEq, Debug)]
enum PalTarget {
    Main,
    TextColor(LayerId),
    TextBg(LayerId),
    ShapeColor(LayerId),
    ShapeFill(LayerId),
    DefaultTextBg,
    DefaultFill,
    ExportBg,
}

type PalReq = Option<(PalTarget, Color32, Pos2)>;

fn hex_of(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") }
}

fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        3 => {
            let d = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|x| x * 17);
            Some(Color32::from_rgb(d(0)?, d(1)?, d(2)?))
        }
        6 => Some(Color32::from_rgb(v(0)?, v(2)?, v(4)?)),
        8 => Some(Color32::from_rgba_unmultiplied(v(0)?, v(2)?, v(4)?, v(6)?)),
        _ => None,
    }
}

/// Swatch (opens the palette for this field) + editable hex + copy / paste.
fn color_row(ui: &mut egui::Ui, c: &mut Color32, target: PalTarget, req: &mut PalReq) -> bool {
    let mut changed = false;
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    ui.painter().rect_filled(r, 5.0, *c);
    ui.painter().rect_stroke(r, 5.0, Stroke::new(1.0, if resp.hovered() { INK_2() } else { FILL_3() }), StrokeKind::Inside);
    if resp.on_hover_text("팔레트 열기").clicked() {
        *req = Some((target, *c, r.right_top()));
    }
    let id = ui.id().with(("hex", format!("{target:?}")));
    let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| hex_of(*c));
    let te = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(84.0).font(egui::TextStyle::Monospace));
    if te.changed() {
        if let Some(nc) = parse_hex(&buf) {
            *c = nc;
            changed = true;
        }
    }
    if !te.has_focus() {
        buf = hex_of(*c);
    }
    ui.data_mut(|d| d.insert_temp(id, buf));
    if ui.small_button("복사").on_hover_text("헥스 코드 복사").clicked() {
        ui.ctx().copy_text(hex_of(*c));
    }
    if ui.small_button("붙여넣기").on_hover_text("클립보드의 헥스 코드 적용").clicked() {
        if let Some(nc) = arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok()).and_then(|t| parse_hex(&t)) {
            *c = nc;
            changed = true;
        }
    }
    changed
}

struct Palette {
    target: PalTarget,
    pos: Pos2,
    h: f32,
    s: f32,
    v: f32,
    orig: Color32,
    sv_tex: Option<(f32, egui::TextureHandle)>,
    hue_tex: Option<egui::TextureHandle>,
    rect: Rect,
    born: f64,
    sub_center: (f32, f32, f32),
    sub_down: bool,
    strip_tex: Option<MapView>,
    patch_tex: Option<MapView>,
    value_tex: Option<MapView>,
}

/// A colour plane with only the colours "similar" to a reference kept at full
/// strength; the rest is a dim ghost of the same plane. Only the region that is
/// connected to the reference survives (stray islands are dropped), and the view
/// window re-centres and zooms on that region.
#[derive(Clone, Copy, PartialEq)]
enum GamutMap {
    /// OKLCH slice at the reference lightness: x = hue, y = chroma (for colouring)
    Value,
    /// OKLCH slice at the reference chroma: x = hue, y = lightness (for shading)
    Chroma,
    /// OKHSV hue x saturation at the reference value; keeps small OKLab distance
    Near,
}

/// Chroma shown at the top of the "same value" map.
const MAX_C: f32 = 0.33;

/// View window in plane coordinates: x = hue (unwrapped, may leave 0..1),
/// y = 0 at the top .. 1 at the bottom.
#[derive(Clone, Copy)]
struct Win {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

struct MapView {
    key: (f32, f32, f32),
    tex: egui::TextureHandle,
    win: Win,
    mask: Vec<bool>,
}

impl GamutMap {
    const W: usize = 132;
    const H: usize = 132;

    /// Colour at a plane point and whether it belongs to the matching set.
    /// Out-of-gamut points return a clamped ghost colour and `false`.
    fn eval(self, r: (f32, f32, f32), lch: (f32, f32, f32), x: f32, y: f32) -> (Color32, bool) {
        let h = x.rem_euclid(1.0);
        let t = (1.0 - y).clamp(0.0, 1.0);
        match self {
            GamutMap::Value => crate::okhsv::oklch(lch.0, t * MAX_C, h),
            GamutMap::Chroma => crate::okhsv::oklch(t, lch.1, h),
            GamutMap::Near => {
                let c = crate::okhsv::to_rgb(h, t, r.2);
                let a = crate::okhsv::to_oklab(r.0, r.1, r.2);
                let b = crate::okhsv::to_oklab(h, t, r.2);
                let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                (c, (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() < 0.12)
            }
        }
    }

    /// Plane position of an OKHSV colour, hue unwrapped next to `xc`.
    fn plane(self, xc: f32, hsv: (f32, f32, f32)) -> (f32, f32) {
        let (l, c, lh) = crate::okhsv::lch_of(crate::okhsv::to_rgb(hsv.0, hsv.1, hsv.2), hsv.0);
        let hue = if self == GamutMap::Near { hsv.0 } else { lh };
        let x = xc + ((hue - xc + 0.5).rem_euclid(1.0) - 0.5);
        match self {
            GamutMap::Value => (x, 1.0 - c / MAX_C),
            GamutMap::Chroma => (x, 1.0 - l),
            GamutMap::Near => (x, 1.0 - hsv.1),
        }
    }

    /// Samples `win` on a w x h grid and keeps only the 4-connected component
    /// that contains the reference point. A window spanning every hue wraps
    /// around, so a region crossing its side edges stays one region.
    fn sample(self, r: (f32, f32, f32), win: Win, w: usize, h: usize) -> (Vec<Color32>, Vec<bool>) {
        let wrap = win.x1 - win.x0 >= 0.999;
        let lch = crate::okhsv::lch_of(crate::okhsv::to_rgb(r.0, r.1, r.2), r.0);
        let mut cols = Vec::with_capacity(w * h);
        let mut raw = Vec::with_capacity(w * h);
        for j in 0..h {
            for i in 0..w {
                let x = win.x0 + (i as f32 + 0.5) / w as f32 * (win.x1 - win.x0);
                let y = win.y0 + (j as f32 + 0.5) / h as f32 * (win.y1 - win.y0);
                let (c, k) = self.eval(r, lch, x, y);
                cols.push(c);
                raw.push(k);
            }
        }
        let (rx, ry) = self.plane((win.x0 + win.x1) * 0.5, r);
        let ci = (((rx - win.x0) / (win.x1 - win.x0) * w as f32) as isize).clamp(0, w as isize - 1) as usize;
        let cj = (((ry - win.y0) / (win.y1 - win.y0) * h as f32) as isize).clamp(0, h as isize - 1) as usize;
        let mut keep = vec![false; w * h];
        let seed = (0..w * h).filter(|&k| raw[k]).min_by_key(|&k| {
            let (i, j) = ((k % w) as isize - ci as isize, (k / w) as isize - cj as isize);
            i * i + j * j
        });
        let mut stack: Vec<usize> = seed.into_iter().collect();
        while let Some(k) = stack.pop() {
            if keep[k] || !raw[k] {
                continue;
            }
            keep[k] = true;
            let (i, j) = (k % w, k / w);
            if i > 0 {
                stack.push(k - 1);
            } else if wrap {
                stack.push(k + w - 1);
            }
            if i + 1 < w {
                stack.push(k + 1);
            } else if wrap {
                stack.push(k + 1 - w);
            }
            if j > 0 {
                stack.push(k - w);
            }
            if j + 1 < h {
                stack.push(k + w);
            }
        }
        (cols, keep)
    }

    /// Window centred on the reference that frames its connected region.
    fn fit(self, r: (f32, f32, f32)) -> Win {
        let (rx, ry) = self.plane(r.0, r);
        let full = Win { x0: rx - 0.5, x1: rx + 0.5, y0: 0.0, y1: 1.0 };
        let (w, h) = (180, 90);
        let (_, keep) = self.sample(r, full, w, h);
        let mut dy = 0.0f32;
        let mut cols = vec![false; w];
        for (k, _) in keep.iter().enumerate().filter(|(_, k)| **k) {
            cols[k % w] = true;
            dy = dy.max((((k / w) as f32 + 0.5) / h as f32 - ry).abs());
        }
        // The region's hue span, read around the hue circle: it is what the
        // longest run of empty columns leaves, so a region crossing the seam
        // behind the reference is centred whole instead of split at the sides.
        let (mut gap, mut gap_at, mut run) = (0usize, 0usize, 0usize);
        for i in 0..2 * w {
            if cols[i % w] {
                run = 0;
            } else {
                run += 1;
                if run > gap && run <= w {
                    gap = run;
                    gap_at = i + 1 - run;
                }
            }
        }
        let (cx, half) = if gap == 0 || gap >= w {
            (rx, 0.5)
        } else {
            let span = (w - gap) as f32;
            (full.x0 + ((gap_at + gap) as f32 + span * 0.5) / w as f32, span * 0.5 / w as f32)
        };
        let rx = cx;
        let hx = (half * 1.25 + 0.01).clamp(0.04, 0.5);
        let hy = (dy * 1.25 + 0.01).clamp(0.08, 0.5);
        let (mut y0, mut y1) = (ry - hy, ry + hy);
        if y0 < 0.0 {
            y1 -= y0;
            y0 = 0.0;
        }
        if y1 > 1.0 {
            y0 -= y1 - 1.0;
            y1 = 1.0;
        }
        Win { x0: rx - hx, x1: rx + hx, y0: y0.max(0.0), y1: y1.min(1.0) }
    }

    fn render(self, r: (f32, f32, f32)) -> (egui::ColorImage, Win, Vec<bool>) {
        let win = self.fit(r);
        let (w, h) = (Self::W, Self::H);
        let (cols, keep) = self.sample(r, win, w, h);
        let ghost_bg = theme::PAPER_2();
        let px = (0..w * h)
            .map(|i| {
                let c = cols[i];
                if keep[i] {
                    let (x, y) = (i % w, i / w);
                    let edge = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
                        .iter()
                        .any(|n| n.is_some_and(|n| !keep[n]));
                    if edge { c.lerp_to_gamma(Color32::WHITE, 0.45) } else { c }
                } else {
                    ghost_bg.lerp_to_gamma(c, 0.22)
                }
            })
            .collect();
        (egui::ColorImage::new([w, h], px), win, keep)
    }

    /// Colour picked at a plane point, as OKHSV.
    fn pick(self, r: (f32, f32, f32), x: f32, y: f32) -> (f32, f32, f32) {
        let lch = crate::okhsv::lch_of(crate::okhsv::to_rgb(r.0, r.1, r.2), r.0);
        let (c, _) = self.eval(r, lch, x, y);
        crate::okhsv::from_rgb(c, x.rem_euclid(1.0))
    }

    fn view(self, ctx: &egui::Context, slot: &mut Option<MapView>, key: (f32, f32, f32), name: &str) {
        if slot.as_ref().is_some_and(|m| m.key == key) {
            return;
        }
        let (img, win, mask) = self.render(key);
        match slot {
            Some(m) => {
                m.tex.set(img, egui::TextureOptions::LINEAR);
                m.key = key;
                m.win = win;
                m.mask = mask;
            }
            None => *slot = Some(MapView { key, tex: ctx.load_texture(name, img, egui::TextureOptions::LINEAR), win, mask }),
        }
    }
}

#[derive(Clone)]
enum Pending {
    New,
    Open(Option<PathBuf>),
    Quit,
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum ExportRegion {
    Camera(u64),
    Frame,
    Content,
    View,
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum ExportBg {
    Transparent,
    White,
    Custom,
}

/// Remembered between runs: %APPDATA%/DriftingVOID/prefs.json
#[derive(serde::Serialize)]
struct PrefsOut<'a> {
    settings: &'a Settings,
    recent: &'a [Color32],
    export: Option<&'a ExportDlg>,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct PrefsIn {
    settings: Option<Settings>,
    recent: Vec<Color32>,
    export: Option<ExportDlg>,
}

fn prefs_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("DriftingVOID").join("prefs.json"))
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ExportDlg {
    region: ExportRegion,
    padding: f32,
    scale: f32,
    bg: ExportBg,
    bg_color: Color32,
    jpeg_quality: u8,
    /// Supersampling: render this many times larger, then scale down.
    #[serde(default = "one_u8")]
    ss: u8,
}

fn one_u8() -> u8 {
    1
}

/// Video export settings (one window for every region source, like image export).
#[derive(Clone)]
struct VideoExportDlg {
    region: ExportRegion,
    padding: f32,
    /// Size given by the width (else the height).
    by_width: bool,
    n: u32,
    fps: f64,
    /// Whole timeline, or [t0, t1).
    all: bool,
    t0: f64,
    t1: f64,
    codec: crate::vexport::Codec,
    quality: u8,
    use_mbps: bool,
    mbps: f64,
    gpu: bool,
    bg: ExportBg,
    bg_color: Color32,
    audio: bool,
    ss: u8,
}

pub struct App {
    doc: Document,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    committed: Document,
    /// The selection when `committed` was taken.
    committed_sel: (Option<LayerId>, Vec<LayerId>, SubSel),
    /// Last scroll input (s): scroll bursts on dials become one undo step.
    scroll_at: f64,
    changed: bool,
    modified: bool,
    path: Option<PathBuf>,

    pan: Vec2,
    zoom: f32,
    canvas_rect: Rect,
    tool: Tool,
    sel: Option<LayerId>,
    s: Settings,

    drag: Option<Drag>,
    source: Source,
    drag_t0: f64,
    last_screen: Pos2,
    touches: std::collections::BTreeMap<TouchId, Pos2>,

    editing: Option<LayerId>,
    edit_focus: bool,

    tex: TexCache,
    export: Option<ExportDlg>,
    vexport: Option<VideoExportDlg>,
    vexport_last: Option<VideoExportDlg>,
    vjob: Option<Arc<crate::vexport::Progress>>,
    pending: Option<Pending>,
    status: String,
    recent: Vec<Color32>,
    view_init: bool,
    title: String,
    cursor_world: Option<Pos2>,
    font_ui: FontUi,
    egui_fonts: std::collections::HashSet<String>,
    palette: Option<Palette>,
    restore_tool: Option<Tool>,
    ui_hidden: bool,
    fonts_applied: bool,
    frames: u64,
    rail_active: Option<Rect>,
    space_held: bool,
    layer_drag: Option<LayerId>,
    renaming: Option<LayerId>,
    rename_buf: String,
    rename_focus: bool,
    last_export: Option<ExportDlg>,
    prefs_saved: String,
    now: f64,
    ime_guard_until: f64,
    /// Tool indicator glide: (from y, to y, start time).
    ind_anim: (f32, f32, f64),
    extra_sel: Vec<LayerId>,
    sub: SubSel,
    pan_latch: bool,
    /// Active 3D space, axis for new vanishing lines, selected line, and whether the
    /// next drag makes a new space.
    space_sel: Option<u64>,
    space_axis: u8,
    space_line: Option<usize>,
    space_new: bool,
    /// 3D workspace (phase 3): on/off, its view, a running G/R/S, a navigation drag,
    /// when Tab went down, and the hold-Tab peek at the other workspace.
    ws3d: bool,
    view3d: Option<crate::view3d::View3d>,
    modal3d: Option<crate::view3d::Modal>,
    /// The 3D view's light gizmo is selected.
    /// R on the light: (space, light before, pointer at start).
    /// Turning the light on the compass: space, light before, part, last pointer.
    light_drag: Option<(u64, crate::persp::Light, crate::view3d::CompassPart, Pos2)>,
    /// An image export running on a worker thread (its status line when done).
    img_job: Option<std::sync::mpsc::Receiver<String>>,
    nav3d: Option<crate::view3d::Nav>,
    tab_t0: Option<f64>,
    peek: bool,
    /// Tab presses taken out of the raw input (pressed, shift) before egui could use
    /// them for focus traversal.
    tab_raw: Vec<(bool, bool)>,
    /// The next drag in the space tool draws the active space's mask.
    space_mask_draw: bool,
    /// The next drag in the space tool draws the reference length.
    space_ref_draw: bool,
    /// The next click on the floor stands this layer there.
    space_place: Option<LayerId>,
    /// The next drag on the floor draws a wall's base line.
    wall_base_new: bool,
    /// The next drag in the space tool draws the tape measure.
    space_tape_draw: bool,
    /// Caret index where a press inside the edited text began (drag selection).
    edit_press: Option<usize>,
    /// Selected wall of the active space, and whether the next drag draws a new one.
    wall_sel: Option<u64>,
    wall_new: bool,
    grs2d: Option<Grs2d>,
    /// Video playback per layer; single-frame decodes (scrubbing) come back on
    /// `scrub_rx`, only the newest request per layer counts.
    playing: std::collections::HashMap<LayerId, Playback>,
    scrub_tx: std::sync::mpsc::Sender<(LayerId, u64, f64, image::RgbaImage)>,
    scrub_rx: std::sync::mpsc::Receiver<(LayerId, u64, f64, image::RgbaImage)>,
    scrub_gen: std::collections::HashMap<LayerId, u64>,
    scrub_want: std::collections::HashMap<LayerId, f64>,
    scrub_busy: usize,
    scrub_shown: std::collections::HashMap<LayerId, u64>,
    /// Width the right-hand columns take (the timeline stops short of them).
    right_reserve: f32,
    /// Content heights of the property columns last frame.
    col_h: [f32; 3],
    /// Timeline card height last frame.
    tl_h: f32,
    /// Timeline playback (timeline time and wall time at the start), the selected key,
    /// a drag on the timeline, and the selected canvas camera.
    tl_play: Option<(f64, f64)>,
    tl_key: Option<(TlRow, f64)>,
    /// Keys picked on the timeline (click, Shift-drag box).
    tl_sel: Vec<(TlRow, f64)>,
    /// A rect tool drag that fills (or with Alt clears) this mask.
    mask_rect: Option<(LayerId, bool)>,
    /// Automatic selection armed for this mask: a canvas click picks a region.
    wand: Option<LayerId>,
    /// 0: automatic selection (click), 1: GrabCut (drag a box).
    wand_mode: u8,
    /// The rect drag on a mask runs GrabCut instead of a plain fill.
    mask_cut: bool,
    /// Scissors outline in progress.
    scissor: Option<Scissor>,
    tl_drag: Option<TlDrag>,
    cam_sel: Option<u64>,
    /// Sound output (opened on first use), decoded audio by file path (None while
    /// decoding), and the selected audio clip.
    audio_out: Option<crate::audio::Output>,
    audio_failed: bool,
    audio_cache: std::collections::HashMap<String, Option<Arc<crate::audio::AudioData>>>,
    audio_tx: std::sync::mpsc::Sender<(String, Result<crate::audio::AudioData, String>)>,
    audio_rx: std::sync::mpsc::Receiver<(String, Result<crate::audio::AudioData, String>)>,
    aud_sel: Option<u64>,
    voice_sig: u64,
    /// Stroke recording: the record switch, and for the stroke being drawn its clock
    /// (wall start, timeline start, live timing or instant) with the points' times
    /// (and, for brushes, the input points).
    rec_on: bool,
    rec_clock: Option<(std::time::Instant, f64, bool)>,
    rec_times: Vec<f32>,
    rec_pts: Vec<[f32; 3]>,
    /// Magnifier texture and the (target, scale) it was rendered for.
    loupe_tex: Option<(egui::TextureHandle, [f32; 3])>,
    middle_moved: bool,
    input_log: std::collections::VecDeque<String>,
    show_log: bool,
}

fn is_paint_layer(k: &LayerKind) -> bool {
    matches!(k, LayerKind::Raster(_) | LayerKind::Vector(_))
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        crate::glass::load_settings();
        cc.egui_ctx.set_fonts(theme::font_defs());
        theme::apply_visuals(&cc.egui_ctx);
        let scrub_ch = std::sync::mpsc::channel();
        let audio_ch = std::sync::mpsc::channel();
        let mut app = Self {
            doc: Document::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            committed: Document::default(),
            committed_sel: Default::default(),
            scroll_at: 0.0,
            changed: false,
            modified: false,
            path: None,
            pan: Vec2::ZERO,
            zoom: 1.0,
            canvas_rect: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)),
            tool: Tool::Select,
            sel: None,
            s: Settings::default(),
            drag: None,
            source: Source::Mouse,
            drag_t0: 0.0,
            last_screen: Pos2::ZERO,
            touches: Default::default(),
            editing: None,
            edit_focus: false,
            tex: TexCache::default(),
            export: None,
            vexport: None,
            vexport_last: None,
            vjob: None,
            pending: None,
            status: String::new(),
            recent: Vec::new(),
            view_init: false,
            title: String::new(),
            cursor_world: None,
            font_ui: FontUi::default(),
            egui_fonts: Default::default(),
            palette: None,
            restore_tool: None,
            ui_hidden: false,
            fonts_applied: false,
            frames: 0,
            rail_active: None,
            space_held: false,
            layer_drag: None,
            renaming: None,
            rename_buf: String::new(),
            rename_focus: false,
            last_export: None,
            prefs_saved: String::new(),
            now: 0.0,
            ime_guard_until: 0.0,
            ind_anim: (f32::NAN, f32::NAN, 0.0),
            extra_sel: Vec::new(),
            sub: SubSel::default(),
            pan_latch: false,
            space_sel: None,
            space_axis: 0,
            space_line: None,
            space_new: false,
            ws3d: false,
            view3d: None,
            modal3d: None,
            light_drag: None,
            img_job: None,
            nav3d: None,
            tab_t0: None,
            peek: false,
            tab_raw: Vec::new(),
            space_mask_draw: false,
            space_ref_draw: false,
            space_place: None,
            wall_base_new: false,
            space_tape_draw: false,
            edit_press: None,
            wall_sel: None,
            wall_new: false,
            grs2d: None,
            playing: Default::default(),
            scrub_tx: scrub_ch.0,
            scrub_rx: scrub_ch.1,
            scrub_gen: Default::default(),
            scrub_want: Default::default(),
            scrub_busy: 0,
            scrub_shown: Default::default(),
            right_reserve: 600.0,
            col_h: [200.0; 3],
            tl_h: 120.0,
            tl_play: None,
            tl_key: None,
            tl_sel: Vec::new(),
            mask_rect: None,
            wand: None,
            wand_mode: 0,
            mask_cut: false,
            scissor: None,
            tl_drag: None,
            cam_sel: None,
            audio_out: None,
            audio_failed: false,
            audio_cache: Default::default(),
            audio_tx: audio_ch.0,
            audio_rx: audio_ch.1,
            aud_sel: None,
            voice_sig: 0,
            rec_on: false,
            rec_clock: None,
            rec_times: Vec::new(),
            rec_pts: Vec::new(),
            loupe_tex: None,
            middle_moved: false,
            input_log: Default::default(),
            show_log: false,
        };
        crate::penhook::install(cc);
        if let Some(text) = prefs_path().and_then(|p| std::fs::read_to_string(p).ok()) {
            if let Ok(pr) = serde_json::from_str::<PrefsIn>(&text) {
                if let Some(st) = pr.settings {
                    app.s = st;
                }
                app.recent = pr.recent;
                app.last_export = pr.export;
                app.prefs_saved = text;
            }
        }
        if let Some(p) = open {
            app.open_path(p);
        }
        if let Ok(fam) = std::env::var("DV_TEST_TEXT_FONT") {
            // reproduces "edit a text box that uses an installed font" for testing
            app.s.text_family = fam;
            app.s.text_style = "Regular".into();
            app.tool = Tool::Text;
            // DV_TEST_TEXT_ALIGN=0/1/2: three lines of different length, aligned
            let al = std::env::var("DV_TEST_TEXT_ALIGN").ok().and_then(|v| v.parse::<usize>().ok()).and_then(|i| crate::doc::TextAlign::ALL.get(i).copied());
            let body = if al.is_some() { "가운데 정렬 Test
짧은 줄
조금 더 긴 줄입니다" } else { "테스트 Test" };
            if let Some(a) = al {
                app.s.text_align = a;
                app.s.color = Color32::from_rgb(255, 225, 90);
            }
            let id = app.new_text_layer(pos2(0.0, 0.0), body.into());
            if let Some(deg) = std::env::var("DV_TEST_TEXT_ROT").ok().and_then(|v| v.parse::<f32>().ok()) {
                if let Some(LayerKind::Text(t)) = app.doc.get_mut(id).map(|l| &mut l.kind) {
                    t.rotation = deg.to_radians();
                }
            }
            app.sel = Some(id);
            app.editing = Some(id);
            app.edit_focus = true;
        }
        if std::env::var_os("DV_MULTI_DEMO").is_some() {
            // three texts that differ in size and turn, all selected (panel checks)
            app.s.color = Color32::from_rgb(120, 230, 255);
            let mut ids = Vec::new();
            for (i, (sz, deg)) in [(40.0, 0.0), (60.0, 15.0), (80.0, 0.0)].into_iter().enumerate() {
                app.s.text_size = sz;
                let id = app.new_text_layer(pos2(-300.0, -150.0 + i as f32 * 120.0), format!("글자 {}", i + 1));
                if let Some(LayerKind::Text(t)) = app.doc.get_mut(id).map(|l| &mut l.kind) {
                    t.rotation = f32::to_radians(deg);
                }
                ids.push(id);
            }
            // "single": the turned middle one alone (handle checks)
            if std::env::var("DV_MULTI_DEMO").is_ok_and(|v| v == "single") {
                app.sel = Some(ids[1]);
            } else {
                app.sel = ids.first().copied();
                app.extra_sel = ids[1..].to_vec();
            }
        }
        if std::env::var_os("DV_TEST_TEXT_LONG").is_some() {
            // a long one-line text box (layout/texture checks)
            app.tool = Tool::Text;
            // "size,repeats"
            let v = std::env::var("DV_TEST_TEXT_LONG").unwrap_or_default();
            let mut it = v.split(',').map(|x| x.parse::<f32>().ok());
            let size = it.next().flatten().unwrap_or(40.0);
            let reps = it.next().flatten().unwrap_or(8.0) as usize;
            app.s.text_size = size;
            // a colour that stands out on the void
            app.s.color = Color32::from_rgb(120, 230, 255);
            let txt = "가나다라마바사아자차카타파하 ABCDEFGHIJ abcdefghij gjpqy 0123456789 ".repeat(reps);
            let id = app.new_text_layer(pos2(-600.0, 0.0), txt);
            app.sel = Some(id);
            // third value: a zoom on the box's middle
            if let Some(z) = it.next().flatten() {
                app.zoom = z;
                app.view_init = true;
                if let Some(b) = app.doc.get(id).and_then(|l| l.bounds()) {
                    app.pan = vec2(700.0, 450.0) - b.center().to_vec2() * z;
                }
            }
            if v.ends_with(",edit") {
                app.editing = Some(id);
                app.edit_focus = true;
            } else if v.ends_with(",plain") {
                // nothing selected: no outline over the glyphs
                app.sel = None;
                app.editing = None;
                app.tool = Tool::Select;
            } else {
                app.editing = None;
                app.tool = Tool::Select;
            }
        }
        if std::env::var_os("DV_SELECT_PHOTO").is_some() {
            // selects the first photo (for screenshots of its properties)
            app.sel = app.doc.layers.iter().find(|l| matches!(l.kind, LayerKind::Image(_))).map(|l| l.id);
        }
        if let Ok(v) = std::env::var("DV_SPACE_LINES") {
            // "x1,y1,x2,y2,axis;..." in the first photo's source pixels; opens the space
            // tool on a space fitted to that photo (screenshots of the solver)
            if let Some(im) = app.doc.layers.iter().find_map(|l| match &l.kind {
                LayerKind::Image(im) => Some(im.clone()),
                _ => None,
            }) {
                let b = Rect::from_points(&im.corners());
                let id = app.doc.new_id();
                let mut sp = crate::persp::Space::new(id, "공간 1".into(), b);
                for part in v.split(';') {
                    let n: Vec<f32> = part.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if n.len() == 5 {
                        let a = im.src_to_world(pos2(n[0], n[1]));
                        let bb = im.src_to_world(pos2(n[2], n[3]));
                        sp.lines.push(crate::persp::VLine { a, b: bb, axis: n[4] as u8 });
                    }
                }
                if let Ok(o) = std::env::var("DV_SPACE_ORIGIN") {
                    let n: Vec<f32> = o.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if n.len() == 2 {
                        sp.origin = im.src_to_world(pos2(n[0], n[1]));
                    }
                }
                // DV_SPACE_TAPE="x1,y1,x2,y2" (photo pixels): a tape measure
                if let Ok(v) = std::env::var("DV_SPACE_TAPE") {
                    let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if n.len() == 4 {
                        let (a, b) = (im.src_to_world(pos2(n[0], n[1])), im.src_to_world(pos2(n[2], n[3])));
                        let mut r = crate::persp::RefLine { a, b, axis: 0, metres: 0.0 };
                        if let Some(ax) = crate::persp::solve(&sp).ok().and_then(|c| r.infer_axis(&c)) {
                            r.axis = ax;
                        }
                        sp.tape = Some(r);
                    }
                }
                // DV_SPACE_FOLLOW=1: the space follows the photo (its name shows in the panel)
                if std::env::var_os("DV_SPACE_FOLLOW").is_some() {
                    if let Some(l) = app.doc.layers.iter().find(|l| matches!(l.kind, LayerKind::Image(_))) {
                        sp.follow = Some(crate::persp::SpaceFollow { layer: l.id, last: [im.center.x, im.center.y, im.scale, im.rotation] });
                    }
                }
                // DV_SPACE_REF="x1,y1,x2,y2,axis,metres" (photo pixels): a reference length
                if let Ok(v) = std::env::var("DV_SPACE_REF") {
                    let n: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if n.len() == 6 {
                        let a = im.src_to_world(pos2(n[0] as f32, n[1] as f32));
                        let b = im.src_to_world(pos2(n[2] as f32, n[3] as f32));
                        sp.ref_line = Some(crate::persp::RefLine { a, b, axis: n[4] as u8, metres: n[5] });
                    }
                }
                app.doc.spaces.push(sp);
                app.space_sel = Some(id);
                app.tool = Tool::Space;
                if let Ok(m) = std::env::var("DV_SPACE_MASK") {
                    let n: Vec<f32> = m.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if n.len() == 4 {
                        let r = Rect::from_two_pos(im.src_to_world(pos2(n[0], n[1])), im.src_to_world(pos2(n[2], n[3])));
                        app.doc.spaces.last_mut().unwrap().mask = Some(r);
                    }
                }
                if std::env::var_os("DV_LOUPE").is_some() {
                    // a line end held mid-drag, to screenshot the magnifier
                    if let Some(l) = app.doc.spaces.last().and_then(|s| s.lines.first()).copied() {
                        app.space_line = Some(0);
                        app.drag = Some(Drag::SpaceLine { id, idx: 0, end: 1, start: l.b, orig: l });
                    }
                }
                if let Ok(mode) = std::env::var("DV_SPACE_DEMO") {
                    // two text layers dropped in: one kept camera-facing, one snapped to
                    // the space axes; "select" leaves the first selected in the select tool
                    let o = app.doc.spaces.last().unwrap().origin;
                    let t1 = app.new_text_layer(o - vec2(160.0, 120.0), "Drifting VOID".into());
                    let t2_text = std::env::var("DV_SPACE_TEXT").unwrap_or_else(|_| "벽에 붙인 글자".into());
                    let t2 = app.new_text_layer(o + vec2(-140.0, -40.0), t2_text);
                    app.put_in_space(t1, id);
                    app.put_in_space(t2, id);
                    // DV_WALL_BASE="x1,y1,x2,y2" (photo pixels): a wall on that floor line, the second text on it
                    if let Ok(v) = std::env::var("DV_WALL_BASE") {
                        let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        let im = app.doc.layers.iter().find_map(|l| match &l.kind {
                            LayerKind::Image(im) => Some(im.clone()),
                            _ => None,
                        });
                        if let (4, Some(im)) = (n.len(), im) {
                            let sp = app.doc.spaces.last().unwrap();
                            let quad = crate::persp::solve(sp).ok().and_then(|c| crate::persp::wall_on_base(&c, im.src_to_world(pos2(n[0], n[1])), im.src_to_world(pos2(n[2], n[3])), sp.floor * 1.6));
                            if let Some(q) = quad {
                                let wid = app.doc.new_id();
                                app.doc.spaces.last_mut().unwrap().walls.push(crate::persp::Wall { id: wid, name: "벽 1".into(), corners: q, offset: 0.0, snap_axis: false });
                                app.wall_sel = Some(wid);
                                app.put_in_wall(t2, id, wid);
                            }
                        }
                    }
                    // DV_STAND="x,y" (photo pixels): the first text stands on the floor there
                    if let Ok(v) = std::env::var("DV_STAND") {
                        let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        if n.len() == 2 {
                            if let Some(im) = app.doc.layers.iter().find_map(|l| match &l.kind {
                                LayerKind::Image(im) => Some(im.clone()),
                                _ => None,
                            }) {
                                app.stand_on_floor(t1, id, im.src_to_world(pos2(n[0], n[1])));
                            }
                        }
                    }
                    if let Some(pl) = app.doc.get_mut(t2).and_then(|l| l.place.as_mut()) {
                        pl.align_to_axes();
                        pl.pos = [0.0, 0.0, 0.0];
                    }
                    // DV_WALL="x,y;x,y;x,y;x,y" (photo pixels): a wall, with the second text on it
                    if let (Ok(v), Some(im)) = (std::env::var("DV_WALL"), app.doc.layers.iter().find_map(|l| match &l.kind {
                        LayerKind::Image(im) => Some(im.clone()),
                        _ => None,
                    })) {
                        let pts: Vec<Pos2> = v
                            .split(';')
                            .filter_map(|q| q.split_once(',').and_then(|(x, y)| Some(im.src_to_world(pos2(x.trim().parse().ok()?, y.trim().parse().ok()?)))))
                            .collect();
                        if pts.len() == 4 {
                            let wid = app.doc.new_id();
                            app.doc.spaces.last_mut().unwrap().walls.push(crate::persp::Wall { id: wid, name: "벽 1".into(), corners: [pts[0], pts[1], pts[2], pts[3]], offset: 0.0, snap_axis: false });
                            app.wall_sel = Some(wid);
                            app.put_in_wall(t2, id, wid);
                        }
                    }
                    // DV_OPACITY / DV_BLEND (Blend::ALL index) on both texts
                    for t in [t1, t2] {
                        if let Some(l) = app.doc.get_mut(t) {
                            if let Some(op) = std::env::var("DV_OPACITY").ok().and_then(|v| v.parse::<f32>().ok()) {
                                l.opacity = op;
                            }
                            if let (Some(b), LayerKind::Text(tx)) = (std::env::var("DV_BLEND").ok().and_then(|v| v.parse::<usize>().ok()).and_then(|i| Blend::ALL.get(i).copied()), &mut l.kind) {
                                tx.blend = b;
                            }
                        }
                    }
                    if let Some(th) = std::env::var("DV_THICK").ok().and_then(|v| v.parse::<f64>().ok()) {
                        for t in [t1, t2] {
                            if let Some(pl) = app.doc.get_mut(t).and_then(|l| l.place.as_mut()) {
                                pl.thick = th;
                            }
                        }
                    }
                    if std::env::var_os("DV_LIGHT").is_some() {
                        let cams = crate::persp::cameras(&app.doc.spaces);
                        let l = cams.get(&id).map(crate::persp::Light::facing).unwrap_or_default();
                        app.doc.spaces.last_mut().unwrap().light = Some(l);
                    }
                    crate::persp::sync_walls(&mut app.doc);
                    if mode == "select" {
                        app.tool = Tool::Select;
                        app.sel = Some(t1);
                    }
                    // DV_WS3D=persp|camera|top|front|peek: open the 3D workspace (or its peek)
                    if let Ok(v) = std::env::var("DV_WS3D") {
                        app.sel = Some(t2);
                        if v == "peek" {
                            app.peek = true;
                        } else {
                            app.toggle_ws();
                            if let Some(view) = app.view3d.as_mut() {
                                match v.as_str() {
                                    "camera" => view.camera = true,
                                    "top" => view.preset(7, false),
                                    "front" => view.preset(1, false),
                                    _ => {
                                        view.yaw += 35.0;
                                        view.pitch += 12.0;
                                        view.dist *= 1.6;
                                    }
                                }
                            }
                        }
                    }
                    if let Ok(out) = std::env::var("DV_SPACE_EXPORT") {
                        if let Some(b) = app.doc.content_bounds() {
                            if let Ok(pm) = crate::export::render(&app.doc, b, 1.0, Some(Color32::WHITE)) {
                                let _ = crate::export::save(&pm, std::path::Path::new(&out), 90);
                            }
                        }
                    }
                }
            }
        }
        if let Ok(v) = std::env::var("DV_IMPORT") {
            // files to bring in at start (screenshots); DV_PLAY=1 also plays a video
            let paths: Vec<PathBuf> = v.split(';').map(PathBuf::from).collect();
            app.import_paths(paths, Pos2::ZERO);
            if std::env::var_os("DV_PLAY").is_some() {
                if let Some(id) = app.sel {
                    app.video_play(id, true);
                }
            }
        }
        if let Ok(v) = std::env::var("DV_CHROMA") {
            // "r,g,b": a chroma key on the first photo, which gets selected
            let n: Vec<u8> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let (3, Some(l)) = (n.len(), app.doc.layers.iter_mut().find(|l| matches!(l.kind, LayerKind::Image(_)))) {
                if let LayerKind::Image(im) = &mut l.kind {
                    im.adjust.chroma = Some([n[0], n[1], n[2]]);
                }
                app.sel = Some(l.id);
            }
        }
        if std::env::var_os("DV_TL_DEMO").is_some() {
            // timeline on, the selected layer keyed to move and grow over 2 s, a camera
            // that pans, playhead at 1 s (screenshots)
            app.doc.timeline.on = true;
            if let Some(id) = app.sel {
                if let Some(l) = app.doc.get_mut(id) {
                    let k0 = crate::anim::key_of(l, 0.0);
                    let mut k1 = k0;
                    k1.t = 2.0;
                    k1.center[0] += 400.0;
                    k1.ext[0] *= 0.5;
                    k1.rot = 0.4;
                    k1.ease = crate::anim::Ease::Linear;
                    let mut k2 = k1;
                    k2.t = 3.5;
                    k2.ease = crate::anim::Ease::Hold;
                    let mut k3 = k2;
                    k3.t = 5.0;
                    k3.center[1] += 200.0;
                    l.anim = Some(crate::anim::Anim { keys: vec![k0, k1, k2, k3], span: Some([0.0, 6.0]), video_start: 0.0 });
                }
            }
            let cid = app.doc.new_id();
            let mut cam = crate::anim::CanvasCam { id: cid, name: "카메라 1".into(), rect: Rect::from_center_size(pos2(0.0, 0.0), vec2(640.0, 360.0)), anim: Default::default(), rot: 0.0, follow: None };
            let c0 = cam.key(0.0);
            let mut c1 = c0;
            c1.t = 3.0;
            c1.center[0] += 300.0;
            cam.anim.keys = vec![c0, c1];
            if std::env::var_os("DV_CAM_FOLLOW").is_some() {
                cam.follow = app.sel.map(|id| crate::anim::Follow { layer: id, pos: true, rot: true, offset: [0.0, 0.0], rot_offset: 0.0 });
                app.tool = Tool::Frame;
            }
            app.doc.cameras.push(cam);
            app.cam_sel = Some(cid);
            app.set_time(1.0);
        }
        if std::env::var_os("DV_MASK_DEMO").is_some() {
            // the selected layer masked to its left half, the mask selected (screenshots)
            if let Some(id) = app.sel {
                let c = app.doc.get(id).map(|l| crate::anim::key_of(l, 0.0).center).unwrap_or([0.0, 0.0]);
                app.mask_button(id);
                // DV_MASK_INV=strength: inverted, at that strength
                if let (Some(k), Some(m)) = (std::env::var("DV_MASK_INV").ok().and_then(|v| v.parse::<f32>().ok()), app.sel.and_then(|m| app.doc.get_mut(m))) {
                    m.mask_invert = true;
                    m.opacity = k;
                }
                if let Some(LayerKind::Raster(r)) = app.sel.and_then(|m| app.doc.get_mut(m)).map(|l| &mut l.kind) {
                    r.fill_rect(Rect::from_min_max(pos2(c[0], c[1] - 4000.0), pos2(c[0] + 4000.0, c[1] + 4000.0)), true);
                }
                if std::env::var_os("DV_WAND").is_some() {
                    // automatic selection at the layer's centre instead of the half fill
                    if let Some(mid) = app.sel {
                        if let Some(LayerKind::Raster(r)) = app.doc.get_mut(mid).map(|l| &mut l.kind) {
                            r.tiles.clear();
                        }
                        let xf = app.xf();
                        app.canvas_rect = Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0));
                        let _ = xf;
                        app.wand_fill(mid, pos2(c[0], c[1]), true);
                        eprintln!("wand: {}", app.status);
                    }
                }
                if std::env::var_os("DV_GRABCUT").is_some() {
                    if let Some(mid) = app.sel {
                        if let Some(LayerKind::Raster(r)) = app.doc.get_mut(mid).map(|l| &mut l.kind) {
                            r.tiles.clear();
                        }
                        let t0 = std::time::Instant::now();
                        app.grabcut_fill(mid, Rect::from_center_size(pos2(c[0] - 640.0, c[1]), vec2(500.0, 500.0)), true);
                        eprintln!("grabcut: {} in {:?}", app.status, t0.elapsed());
                    }
                }
                if std::env::var_os("DV_SCISSOR").is_some() {
                    if let Some(mid) = app.sel {
                        if let Some(LayerKind::Raster(r)) = app.doc.get_mut(mid).map(|l| &mut l.kind) {
                            r.tiles.clear();
                        }
                        let t0 = std::time::Instant::now();
                        let o = pos2(c[0] - 640.0, c[1]);
                        for d in [vec2(-200.0, -200.0), vec2(200.0, -200.0), vec2(200.0, 200.0), vec2(-200.0, 200.0)] {
                            app.scissor_click(mid, o + d, true, false);
                        }
                        app.scissor_close(std::env::var_os("DV_SCISSOR_CUT").is_some());
                        eprintln!("scissor: {} in {:?}", app.status, t0.elapsed());
                    }
                }
                if std::env::var_os("DV_MASK_OWNER").is_some() {
                    app.select_only(Some(id));
                }
            }
        }
        if std::env::var_os("DV_SHOW_UNSAVED").is_some() {
            app.pending = Some(Pending::New);
        }
        if std::env::var_os("DV_SHOW_EXPORT").is_some() {
            app.open_export();
        }
        if let Ok(t) = std::env::var("DV_THEME") {
            app.s.theme = t.parse().unwrap_or(1);
        }
        if std::env::var_os("DV_SHOW_VEXPORT").is_some() {
            app.open_video_export();
        }
        if let Ok(v) = std::env::var("DV_SHOW_PALETTE") {
            if let Ok(n) = u32::from_str_radix(v.trim_start_matches('#'), 16) {
                if v.trim_start_matches('#').len() == 6 {
                    app.s.color = Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8);
                }
            }
            app.open_palette(pos2(420.0, 220.0));
        }
        app
    }

    // ------------------------------------------------------------ helpers

    fn xf(&self) -> Xf {
        Xf { min: self.canvas_rect.min, pan: self.pan, zoom: self.zoom }
    }

    fn mark(&mut self) {
        self.changed = true;
        self.modified = true;
    }

    fn commit_if_idle(&mut self, typing: bool) {
        if self.changed && self.drag.is_none() && !typing && self.doc.timeline.on && self.doc.timeline.autokey && self.tl_play.is_none() {
            let t = self.doc.timeline.time;
            // only the values that changed since the last commit get keyed
            if self.tool == Tool::Frame {
                if let Some(id) = self.cam_sel {
                    let prev = self.committed.cameras.iter().find(|c| c.id == id).map(|c| c.key(t));
                    if let Some(c) = self.doc.cameras.iter_mut().find(|c| c.id == id) {
                        let mut k = c.key(t);
                        k.ch = prev.map_or(crate::anim::ch::ALL, |p| crate::anim::diff_ch(&p, &k));
                        if k.ch != 0 {
                            c.anim.set_key(k);
                        }
                    }
                }
            }
            for id in self.selected_ids() {
                let prev = self.committed.get(id).map(|l| crate::anim::key_of(l, t));
                if let Some(l) = self.doc.get_mut(id) {
                    let mut key = crate::anim::key_of(l, t);
                    key.ch = prev.map_or(crate::anim::ch::ALL, |p| crate::anim::diff_ch(&p, &key));
                    if key.ch != 0 {
                        l.anim.get_or_insert_with(Default::default).set_key(key);
                    }
                }
            }
        }
        if self.changed && self.drag.is_none() && !typing {
            // drawing layers are created on demand, so empty ones just go away
            let before = self.doc.layers.len();
            // (masks stay: an empty mask is a valid "hide nothing" mask)
            self.doc.layers.retain(|l| match &l.kind {
                LayerKind::Raster(_) if l.mask_of.is_some() => true,
                LayerKind::Raster(r) => !r.tiles.is_empty(),
                LayerKind::Vector(v) => !v.strokes.is_empty(),
                _ => true,
            });
            if self.doc.layers.len() != before {
                self.fix_sel();
            }
            let prev = std::mem::replace(&mut self.committed, self.doc.clone());
            let (sel, extra, sub) = std::mem::replace(&mut self.committed_sel, (self.sel, self.extra_sel.clone(), self.sub.clone()));
            let cost = prev.bytes_not_in(&self.doc);
            self.undo.push(Snap { doc: prev, sel, extra, sub, cost });
            let mut total: usize = self.undo.iter().map(|u| u.cost).sum();
            while self.undo.len() > 1 && (total > UNDO_BYTES || self.undo.len() > UNDO_STEPS) {
                total -= self.undo.remove(0).cost;
            }
            self.redo.clear();
            self.changed = false;
        }
    }

    fn undo(&mut self) {
        self.cancel_drag();
        self.finish_text_edit();
        self.commit_if_idle(false);
        if let Some(u) = self.undo.pop() {
            let cost = self.doc.bytes_not_in(&u.doc);
            let cur = self.take_state(u, cost);
            self.redo.push(cur);
        }
    }

    /// Swaps in a history state (document and its selection) and returns the
    /// current one, costed at `cost`.
    fn take_state(&mut self, u: Snap, cost: usize) -> Snap {
        let doc = std::mem::replace(&mut self.doc, u.doc);
        let cur = Snap {
            doc,
            sel: std::mem::replace(&mut self.sel, u.sel),
            extra: std::mem::replace(&mut self.extra_sel, u.extra),
            sub: std::mem::replace(&mut self.sub, u.sub),
            cost,
        };
        self.committed = self.doc.clone();
        self.committed_sel = (self.sel, self.extra_sel.clone(), self.sub.clone());
        self.modified = true;
        self.fix_sel();
        cur
    }

    fn redo(&mut self) {
        self.cancel_drag();
        self.finish_text_edit();
        if let Some(u) = self.redo.pop() {
            let cost = self.doc.bytes_not_in(&u.doc);
            let cur = self.take_state(u, cost);
            self.undo.push(cur);
        }
    }

    fn fix_sel(&mut self) {
        // drawing selections only where their layer (and strokes) still exist
        let doc = &self.doc;
        self.sub.strokes.retain_mut(|(id, idx)| match doc.get(*id).map(|l| &l.kind) {
            Some(LayerKind::Vector(v)) => {
                idx.retain(|&i| i < v.strokes.len());
                !idx.is_empty()
            }
            _ => false,
        });
        self.sub.pixels.retain(|(id, _)| matches!(doc.get(*id).map(|l| &l.kind), Some(LayerKind::Raster(_))));
        if let Some(id) = self.sel {
            if self.doc.get(id).is_none() {
                self.sel = None;
            }
        }
        let doc = &self.doc;
        self.extra_sel.retain(|id| doc.get(*id).is_some());
        if self.sel.is_none() {
            self.sel = self.extra_sel.pop();
        }
    }

    /// Primary selection first, then the rest.
    fn selected_ids(&self) -> Vec<LayerId> {
        self.sel.into_iter().chain(self.extra_sel.iter().copied()).collect()
    }

    fn is_selected(&self, id: LayerId) -> bool {
        self.sel == Some(id) || self.extra_sel.contains(&id)
    }

    fn select_only(&mut self, id: Option<LayerId>) {
        self.sel = id;
        self.extra_sel.clear();
        self.sub = SubSel::default();
    }

    /// Everything selected, as (original layer, part) pairs; locked layers skipped.
    fn sel_parts(&self) -> Vec<(Layer, Part)> {
        let mut out = Vec::new();
        for id in self.selected_ids() {
            if let Some(l) = self.doc.get(id).filter(|l| !l.locked) {
                out.push((l.clone(), Part::Whole));
            }
        }
        for (id, idx) in &self.sub.strokes {
            if let Some(l) = self.doc.get(*id).filter(|l| !l.locked) {
                out.push((l.clone(), Part::Strokes(idx.clone())));
            }
        }
        for (id, polys) in &self.sub.pixels {
            if let Some(l) = self.doc.get(*id).filter(|l| !l.locked) {
                out.push((l.clone(), Part::Pixels(polys.clone())));
            }
        }
        out
    }

    fn sub_bounds(&self) -> Option<Rect> {
        let mut acc: Option<Rect> = None;
        let mut add = |b: Rect| acc = Some(acc.map_or(b, |a| a.union(b)));
        for (id, idx) in &self.sub.strokes {
            if let Some(LayerKind::Vector(v)) = self.doc.get(*id).map(|l| &l.kind) {
                for &i in idx {
                    if let Some(st) = v.strokes.get(i) {
                        add(st.bounds.translate(v.offset));
                    }
                }
            }
        }
        for (_, polys) in &self.sub.pixels {
            for p in polys {
                add(Rect::from_points(p));
            }
        }
        acc
    }

    /// Where the whole selection shows on the canvas (layers and drawing parts).
    fn selection_bounds(&self) -> Option<Rect> {
        let cams = crate::persp::cameras(&self.doc.spaces);
        let layers = self.selected_ids().into_iter().filter_map(|id| self.doc.get(id)?.shown_bounds(&cams));
        layers.chain(self.sub_bounds()).reduce(|a, b| a.union(b))
    }

    /// Does a world point land on something selected (to start a group move)?
    fn hits_selection(&self, w: Pos2, tol: f32) -> bool {
        if self.hit_layer(w, tol).is_some_and(|id| self.is_selected(id)) {
            return true;
        }
        for (id, idx) in &self.sub.strokes {
            if let Some(LayerKind::Vector(v)) = self.doc.get(*id).map(|l| &l.kind) {
                if idx.iter().filter_map(|&i| v.strokes.get(i)).any(|st| st.hit(w - v.offset, tol)) {
                    return true;
                }
            }
        }
        self.sub.pixels.iter().any(|(_, ps)| ps.iter().any(|p| in_poly(w, p)))
    }

    /// Applies a finished move/transform to the stored pixel boxes.
    fn remap_pixel_sel(&mut self, c: Pos2, s: f32, th: f32, d: Vec2) {
        for (_, polys) in &mut self.sub.pixels {
            for p in polys.iter_mut() {
                *p = map_poly(p, c, s, th, d);
            }
        }
    }

    fn toggle_selected(&mut self, id: LayerId) {
        if self.sel == Some(id) {
            self.sel = self.extra_sel.pop();
        } else if let Some(i) = self.extra_sel.iter().position(|x| *x == id) {
            self.extra_sel.remove(i);
        } else {
            if let Some(s) = self.sel {
                self.extra_sel.push(s);
            }
            self.sel = Some(id);
        }
    }

    fn log_input(&mut self, s: String) {
        self.input_log.push_front(s);
        self.input_log.truncate(14);
    }

    fn reset_doc(&mut self, doc: Document, path: Option<PathBuf>) {
        self.cancel_drag();
        self.editing = None;
        self.doc = doc;
        self.committed = self.doc.clone();
        self.undo.clear();
        self.redo.clear();
        self.committed_sel = Default::default();
        self.changed = false;
        self.modified = false;
        self.path = path;
        self.sel = None;
        self.view_init = false;
    }

    fn sel_layer(&self) -> Option<&Layer> {
        self.sel.and_then(|id| self.doc.get(id))
    }

    fn view_center_world(&self) -> Pos2 {
        self.xf().s2w(self.canvas_rect.center())
    }

    fn fit_view(&mut self) {
        let r = self.canvas_rect;
        match self.doc.content_bounds() {
            Some(b) if b.width() > 0.0 && b.height() > 0.0 => {
                let z = ((r.width() - 60.0) / b.width()).min((r.height() - 60.0) / b.height()).clamp(0.01, 8.0);
                self.zoom = z;
                self.pan = r.size() * 0.5 - b.center().to_vec2() * z;
            }
            _ => {
                self.zoom = 1.0;
                self.pan = r.size() * 0.5;
            }
        }
    }

    fn zoom_at(&mut self, screen: Pos2, factor: f32) {
        let w = self.xf().s2w(screen);
        self.zoom = (self.zoom * factor).clamp(0.01, 64.0);
        self.pan = (screen - self.canvas_rect.min) - w.to_vec2() * self.zoom;
    }

    fn set_zoom_center(&mut self, z: f32) {
        let c = self.canvas_rect.center();
        self.zoom_at(c, z / self.zoom);
    }

    // ------------------------------------------------------------ import

    fn add_image(&mut self, src: ImageSource, name: String, at: Pos2) -> LayerId {
        // photos go under the drawing layers so ink stays on top
        let idx = self.doc.layers.iter().position(|l| is_paint_layer(&l.kind)).unwrap_or(self.doc.layers.len());
        let id = self.doc.new_id();
        let kind = LayerKind::Image(ImageLayer::new(src, at));
        self.doc.layers.insert(idx, Layer { id, name, visible: true, locked: false, opacity: 1.0, kind, place: None, anim: None, mask_of: None, mask_anchor: None, mask_pose: None, mask_invert: false });
        self.sel = Some(id);
        self.mark();
        id
    }

    fn import_paths(&mut self, paths: Vec<PathBuf>, at: Pos2) {
        let mut pos = at;
        let mut errs = Vec::new();
        let mut n = 0;
        for p in paths {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if ext == "void" {
                self.request(Pending::Open(Some(p)));
                return;
            }
            if crate::audio::AUDIO_EXTS.contains(&ext.as_str()) {
                self.add_audio(&p);
                n += 1;
                continue;
            }
            if crate::video::VIDEO_EXTS.contains(&ext.as_str()) {
                match self.add_video(&p, pos) {
                    Ok(w) => {
                        pos += vec2(w * 0.5 + 40.0, 0.0);
                        n += 1;
                    }
                    Err(e) => errs.push(format!("{}: {e}", p.display())),
                }
                continue;
            }
            match crate::io::load_image_file(&p) {
                Ok(src) => {
                    let w = src.size().x;
                    let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("사진").to_string();
                    self.add_image(src, name, pos);
                    pos += vec2(w * 0.5 + 40.0, 0.0);
                    n += 1;
                }
                Err(e) => errs.push(e),
            }
        }
        self.status = if errs.is_empty() {
            format!("{n}개 불러옴")
        } else {
            format!("불러오기 실패: {}", errs.join(", "))
        };
    }

    /// A video comes in as an image layer showing its first frame.
    fn add_video(&mut self, p: &std::path::Path, at: Pos2) -> Result<f32, String> {
        let info = crate::video::probe(p)?;
        let size = info.preview_size(crate::video::PREVIEW_SIDE);
        let img = crate::video::frame_at(p, 0.0, size)?;
        let src = ImageSource { id: next_rev(), rgba: Arc::new(img), encoded: Arc::new(Vec::new()), ext: "png".into(), logical: Some([info.width, info.height]) };
        let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("영상").to_string();
        let id = self.add_image(src, name, at);
        if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
            im.video = Some(VideoRef { path: p.to_string_lossy().into(), info: info.clone(), time: 0.0, speed: 1.0, looping: false, mute: false, volume: 1.0, slot: next_rev() });
        }
        Ok(info.width as f32)
    }

    fn video_of(&self, id: LayerId) -> Option<VideoRef> {
        match self.doc.get(id).map(|l| &l.kind) {
            Some(LayerKind::Image(im)) => im.video.clone(),
            _ => None,
        }
    }

    fn set_video_frame(&mut self, id: LayerId, t: f64, img: image::RgbaImage) {
        if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
            if let Some(v) = im.video.as_mut() {
                v.time = t;
                im.src = ImageSource { id: next_rev(), rgba: Arc::new(img), encoded: Arc::new(Vec::new()), ext: "png".into(), logical: Some([v.info.width, v.info.height]) };
            }
        }
    }

    /// Shows the frame at `t` (decoded on a worker; the newest request wins).
    fn video_scrub(&mut self, id: LayerId, t: f64) {
        let Some(v) = self.video_of(id) else { return };
        let t = t.clamp(0.0, (v.info.duration - v.info.frame_len() * 0.5).max(0.0));
        if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
            if let Some(vv) = im.video.as_mut() {
                vv.time = t;
            }
        }
        self.scrub_want.insert(id, t);
        self.dispatch_scrubs();
    }

    fn dispatch_scrubs(&mut self) {
        while self.scrub_busy < 3 {
            let Some((&id, &t)) = self.scrub_want.iter().next() else { return };
            self.scrub_want.remove(&id);
            let Some(v) = self.video_of(id) else { continue };
            let g = self.scrub_gen.get(&id).copied().unwrap_or(0) + 1;
            self.scrub_gen.insert(id, g);
            let tx = self.scrub_tx.clone();
            let path = PathBuf::from(&v.path);
            let size = v.info.preview_size(1280);
            self.scrub_busy += 1;
            std::thread::spawn(move || {
                let img = crate::video::frame_at(&path, t, size).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
                let _ = tx.send((id, g, t, img));
            });
        }
    }

    fn video_play(&mut self, id: LayerId, on: bool) {
        self.playing.remove(&id);
        let solo = self.tl_play.is_none();
        if !on {
            if solo && self.playing.is_empty() {
                if let Some(o) = &self.audio_out {
                    o.stop();
                }
            }
            return;
        }
        let Some(v) = self.video_of(id) else { return };
        let t = if v.time >= v.info.duration - v.info.frame_len() * 1.5 { 0.0 } else { v.time };
        if solo && v.info.audio && !v.mute {
            if let Some(Some(data)) = self.audio_cache.get(&v.path).cloned() {
                let voice = crate::audio::Voice { data, t0: 0.0, src0: t, speed: v.speed, span: (0.0, f64::MAX), gain: v.volume, env: None };
                if let Some(o) = self.audio_output() {
                    o.play(vec![voice], 0.0);
                }
            }
        }
        let size = v.info.preview_size(crate::video::PREVIEW_SIDE);
        match crate::video::Player::start(std::path::Path::new(&v.path), t, v.info.fps, size) {
            Ok(player) => {
                self.playing.insert(id, Playback { player, t0: t, wall0: self.now, speed: v.speed, pending: None });
            }
            Err(e) => self.status = format!("재생 실패: {e}"),
        }
    }

    fn video_step(&mut self, id: LayerId, d: i32) {
        self.video_play(id, false);
        if let Some(v) = self.video_of(id) {
            self.video_scrub(id, v.time + d as f64 * v.info.frame_len());
            self.mark();
        }
    }

    /// The current frame at full resolution as a new photo layer above the video.
    fn video_still(&mut self, id: LayerId) {
        let Some(l) = self.doc.get(id).cloned() else { return };
        let LayerKind::Image(im) = &l.kind else { return };
        let Some(v) = &im.video else { return };
        match crate::video::frame_at(std::path::Path::new(&v.path), v.time, (v.info.width, v.info.height)) {
            Ok(img) => {
                let mut nl = l.clone();
                nl.id = self.doc.new_id();
                nl.name = format!("{} {}", l.name, crate::video::timecode(v.time));
                if let LayerKind::Image(nim) = &mut nl.kind {
                    nim.src = crate::io::image_from_rgba(img);
                    nim.video = None;
                }
                let idx = self.doc.index_of(id).map_or(self.doc.layers.len(), |i| i + 1);
                let nid = nl.id;
                self.doc.layers.insert(idx, nl);
                self.select_only(Some(nid));
                self.status = "프레임 → 사진 레이어".into();
                self.mark();
            }
            Err(e) => self.status = format!("프레임 읽기 실패: {e}"),
        }
    }

    fn tick_videos(&mut self, ctx: &egui::Context) {
        while let Ok((id, g, t, img)) = self.scrub_rx.try_recv() {
            self.scrub_busy = self.scrub_busy.saturating_sub(1);
            // any answer newer than what is shown goes up at once (dragging shows
            // frames along the way, the last request still wins)
            if self.scrub_shown.get(&id).is_none_or(|s| g > *s) && img.width() > 1 {
                self.scrub_shown.insert(id, g);
                self.set_video_frame(id, t, img);
            }
        }
        self.dispatch_scrubs();
        if self.scrub_busy > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let ids: Vec<LayerId> = self.playing.keys().copied().collect();
        for id in ids {
            let Some(v) = self.video_of(id) else {
                self.playing.remove(&id);
                if self.tl_play.is_none() && self.playing.is_empty() {
                    if let Some(o) = &self.audio_out {
                        o.stop();
                    }
                }
                continue;
            };
            let now = self.now;
            let pb = self.playing.get_mut(&id).unwrap();
            let clock = pb.t0 + (now - pb.wall0) * pb.speed;
            let mut show = None;
            let mut drained = false;
            loop {
                if pb.pending.is_none() {
                    match pb.player.rx.try_recv() {
                        Ok(f) => pb.pending = Some(f),
                        Err(_) => {
                            drained = true;
                            break;
                        }
                    }
                }
                if pb.pending.as_ref().is_some_and(|(t, _)| *t <= clock + 1e-6) {
                    show = pb.pending.take();
                } else {
                    break;
                }
            }
            let done = drained && pb.player.ended.load(std::sync::atomic::Ordering::Relaxed);
            if let Some((t, img)) = show {
                self.set_video_frame(id, t, img);
            }
            if done || clock > v.info.duration + 1.0 {
                self.playing.remove(&id);
                if self.tl_play.is_none() && self.playing.is_empty() {
                    if let Some(o) = &self.audio_out {
                        o.stop();
                    }
                }
                if v.looping {
                    if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                        if let Some(vv) = im.video.as_mut() {
                            vv.time = 0.0;
                        }
                    }
                    self.video_play(id, true);
                }
            }
        }
        if !self.playing.is_empty() {
            ctx.request_repaint();
        }
    }

    /// The document with every video layer showing its frame at full resolution
    /// (export, clipboard).
    fn export_doc(&self) -> crate::doc::Document {
        let mut d = self.doc.clone();
        for l in &mut d.layers {
            if let LayerKind::Image(im) = &mut l.kind {
                if let Some(v) = &im.video {
                    if let Ok(img) = crate::video::frame_at(std::path::Path::new(&v.path), v.time, (v.info.width, v.info.height)) {
                        im.src = ImageSource { id: next_rev(), rgba: Arc::new(img), encoded: Arc::new(Vec::new()), ext: "png".into(), logical: None };
                    }
                }
            }
        }
        d
    }

    // ------------------------------------------------------------ timeline

    /// Moves the playhead: keyed layers and cameras take their sampled state, and
    /// (when not playing) video layers show their frame for that time.
    fn set_time(&mut self, t: f64) {
        let tl = &mut self.doc.timeline;
        tl.time = t.clamp(0.0, tl.duration.max(0.0));
        crate::anim::evaluate(&mut self.doc);
        if self.tl_play.is_none() {
            self.sync_videos_to_timeline();
        }
    }

    fn timeline_videos(&self) -> Vec<(LayerId, f64)> {
        let t = self.doc.timeline.time;
        self.doc
            .layers
            .iter()
            .filter(|l| self.doc.alive(l))
            .filter_map(|l| match &l.kind {
                LayerKind::Image(im) => im.video.as_ref().map(|v| (l.id, crate::anim::video_time(l.anim.as_ref(), v.speed, t))),
                _ => None,
            })
            .collect()
    }

    fn sync_videos_to_timeline(&mut self) {
        for (id, vt) in self.timeline_videos() {
            let cur = self.video_of(id).map(|v| v.time);
            if cur.is_none_or(|c| (c - vt).abs() > 1e-4) {
                self.video_scrub(id, vt);
            }
        }
    }

    fn tl_toggle_play(&mut self) {
        if self.tl_play.take().is_some() {
            if let Some(o) = &self.audio_out {
                o.stop();
            }
            self.playing.clear();
            let t = self.doc.timeline.snap(self.doc.timeline.time);
            self.set_time(t);
            return;
        }
        if self.doc.timeline.time >= self.doc.timeline.duration - self.doc.timeline.frame_len() {
            self.set_time(0.0);
        }
        self.tl_play = Some((self.doc.timeline.time, self.now));
        let voices = self.timeline_voices();
        if !voices.is_empty() {
            let t = self.doc.timeline.time;
            if let Some(o) = self.audio_output() {
                o.play(voices, t);
            }
        }
        for (id, vt) in self.timeline_videos() {
            if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                if let Some(v) = im.video.as_mut() {
                    v.time = vt.clamp(0.0, v.info.duration);
                }
            }
            self.video_play(id, true);
        }
    }

    fn tick_timeline(&mut self, ctx: &egui::Context) {
        let Some((t0, w0)) = self.tl_play else { return };
        // the sound card is the clock while sound plays
        let t = self.audio_out.as_ref().and_then(|o| o.pos()).unwrap_or(t0 + (self.now - w0));
        if t >= self.doc.timeline.duration {
            // loop
            self.tl_play = None;
            self.playing.clear();
            self.set_time(0.0);
            self.tl_toggle_play();
        } else {
            let before: Vec<bool> = self.doc.layers.iter().map(|l| self.doc.alive(l)).collect();
            self.doc.timeline.time = t;
            crate::anim::evaluate(&mut self.doc);
            // video layers that just came into their span start playing
            let now_alive: Vec<(LayerId, bool, bool)> = self.doc.layers.iter().zip(before).map(|(l, b)| (l.id, b, self.doc.alive(l))).collect();
            for (id, was, is) in now_alive {
                if !was && is && self.video_of(id).is_some() {
                    let vt = self.doc.get(id).map(|l| crate::anim::video_time(l.anim.as_ref(), self.video_of(id).map_or(1.0, |v| v.speed), t)).unwrap_or(0.0);
                    if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                        if let Some(v) = im.video.as_mut() {
                            v.time = vt.max(0.0);
                        }
                    }
                    self.video_play(id, true);
                } else if was && !is {
                    self.playing.remove(&id);
                }
            }
        }
        ctx.request_repaint();
    }

    /// After a live recorded stroke: the playhead moves to its end (so the next
    /// stroke follows on) and the timeline grows to fit.
    fn after_recorded(&mut self, end: f64) {
        if end > self.doc.timeline.duration {
            self.doc.timeline.duration = end.ceil();
        }
        if self.tl_play.is_none() {
            self.doc.timeline.time = end.min(self.doc.timeline.duration);
            crate::anim::evaluate(&mut self.doc);
        }
    }

    /// Recorded stroke intervals of a layer (for the timeline).
    fn rec_spans(&self, id: LayerId) -> Vec<(f64, f64)> {
        match self.doc.get(id).map(|l| &l.kind) {
            Some(LayerKind::Vector(v)) => v.strokes.iter().filter_map(|s| s.rec.as_ref().map(|r| (r.start, r.end()))).collect(),
            Some(LayerKind::Raster(r)) => r.recs.iter().map(|x| (x.start, x.end())).collect(),
            _ => Vec::new(),
        }
    }

    /// I: keys the selection (or, with the region tool, the selected camera) at the
    /// playhead.
    fn insert_keys(&mut self) {
        let t = self.doc.timeline.time;
        if self.tool == Tool::Frame {
            if let Some(c) = self.cam_sel.and_then(|id| self.doc.cameras.iter_mut().find(|c| c.id == id)) {
                let k = c.key(t);
                c.anim.set_key(k);
                self.status = format!("{} 키 · {}", c.name, crate::video::timecode(t));
                self.mark();
                return;
            }
        }
        let ids = self.selected_ids();
        if ids.is_empty() {
            self.status = "키 넣을 레이어 없음".into();
            return;
        }
        for id in &ids {
            if let Some(l) = self.doc.get_mut(*id) {
                let key = crate::anim::key_of(l, t);
                l.anim.get_or_insert_with(Default::default).set_key(key);
            }
        }
        self.status = format!("키 {}개 · {}", ids.len(), crate::video::timecode(t));
        self.mark();
    }

    /// Automatic selection: renders what lies under the masked layer at full canvas
    /// resolution (one pixel per mask pixel) around `w`, takes the connected region
    /// of similar colour, softens its edge with a guided filter (the picture as the
    /// guide) and hides it (or shows it again).
    fn wand_fill(&mut self, mid: LayerId, w: Pos2, on: bool) {
        let Some(owner) = self.doc.get(mid).and_then(|l| l.mask_of) else { return };
        let xf = self.xf();
        let view = Rect::from_min_max(xf.s2w(self.canvas_rect.min), xf.s2w(self.canvas_rect.max));
        if !view.contains(w) {
            return;
        }
        // whole canvas pixels, at most 4096 around the click
        const SIDE: f32 = 4096.0;
        let lim = Rect::from_center_size(w, vec2(SIDE, SIDE));
        let reg = view.expand(64.0).intersect(lim);
        let reg = Rect::from_min_max(reg.min.floor(), reg.max.ceil());
        let mut d = self.doc.clone();
        d.layers.retain(|l| l.id != owner && l.mask_of.is_none());
        let Ok(pm) = crate::export::render(&d, reg, 1.0, Some(Color32::BLACK)) else { return };
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let px = pm.data();
        let rgb = |x: i32, y: i32| {
            let i = ((y * pw + x) * 4) as usize;
            [px[i] as f32, px[i + 1] as f32, px[i + 2] as f32]
        };
        let sx = ((w.x - reg.min.x) as i32).clamp(0, pw - 1);
        let sy = ((w.y - reg.min.y) as i32).clamp(0, ph - 1);
        let seed = rgb(sx, sy);
        let tol = self.s.wand_tol / 100.0 * 255.0 * 1.732;
        let mut hit = vec![false; (pw * ph) as usize];
        let mut stack = vec![(sx, sy)];
        hit[(sy * pw + sx) as usize] = true;
        let (mut x0, mut y0, mut x1, mut y1) = (sx, sy, sx, sy);
        while let Some((x, y)) = stack.pop() {
            for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if nx < 0 || ny < 0 || nx >= pw || ny >= ph {
                    continue;
                }
                let j = (ny * pw + nx) as usize;
                if hit[j] {
                    continue;
                }
                let c = rgb(nx, ny);
                let dd = ((c[0] - seed[0]).powi(2) + (c[1] - seed[1]).powi(2) + (c[2] - seed[2]).powi(2)).sqrt();
                if dd <= tol {
                    hit[j] = true;
                    stack.push((nx, ny));
                    x0 = x0.min(nx);
                    y0 = y0.min(ny);
                    x1 = x1.max(nx);
                    y1 = y1.max(ny);
                }
            }
        }
        // guided filter over the region's box (plus a margin), guide = luminance
        let r = (self.s.mask_refine.round() as i32 + self.s.mask_grow.abs().round() as i32).max(1);
        let (bx0, by0) = ((x0 - 2 * r).max(0), (y0 - 2 * r).max(0));
        let (bx1, by1) = ((x1 + 2 * r + 1).min(pw), (y1 + 2 * r + 1).min(ph));
        let (bw, bh) = ((bx1 - bx0) as usize, (by1 - by0) as usize);
        let mut gi = vec![0f32; bw * bh];
        let mut gp = vec![0f32; bw * bh];
        for y in 0..bh {
            for x in 0..bw {
                let (ix, iy) = (bx0 + x as i32, by0 + y as i32);
                let c = rgb(ix, iy);
                gi[y * bw + x] = (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) / 255.0;
                gp[y * bw + x] = if hit[(iy * pw + ix) as usize] { 1.0 } else { 0.0 };
            }
        }
        let n = self.mask_soft_write(mid, (reg.min.x as i32 + bx0, reg.min.y as i32 + by0), bw, bh, &gi, &gp, on);
        self.status = format!("자동 선택 · {n} px {}", if on { "가림" } else { "되살림" });
        self.mark();
    }

    /// Writes a hard 0/1 selection into a mask with a soft edge: a guided filter
    /// (guide = luminance) snaps it to the picture, then the edge band is narrowed
    /// so it does not spill over the boundary. `origin` is the world pixel of (0,0).
    #[allow(clippy::too_many_arguments)]
    fn mask_soft_write(&mut self, mid: LayerId, origin: (i32, i32), bw: usize, bh: usize, gi: &[f32], gp: &[f32], on: bool) -> usize {
        // grow or shrink the hard selection first (closes hairline gaps along edges)
        let g = self.s.mask_grow.round() as i32;
        let mut hard = gp.to_vec();
        if g != 0 {
            let grow = g > 0;
            for _ in 0..g.abs() {
                let src = hard.clone();
                for y in 0..bh {
                    for x in 0..bw {
                        let mut v = src[y * bw + x];
                        for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                            if nx < 0 || ny < 0 || nx >= bw as i32 || ny >= bh as i32 {
                                continue;
                            }
                            let n = src[ny as usize * bw + nx as usize];
                            v = if grow { v.max(n) } else { v.min(n) };
                        }
                        hard[y * bw + x] = v;
                    }
                }
            }
        }
        let rad = self.s.mask_refine.round() as usize;
        let q = if rad == 0 { hard } else { guided_filter(gi, &hard, bw, bh, rad, 1e-3) };
        let sharpen = rad > 0;
        let Some(LayerKind::Raster(m)) = self.doc.get_mut(mid).map(|l| &mut l.kind) else { return 0 };
        let mut n = 0usize;
        for y in 0..bh {
            for x in 0..bw {
                let v = if sharpen { ((q[y * bw + x] - 0.5) * 2.5 + 0.5).clamp(0.0, 1.0) } else { q[y * bw + x] };
                if v < 0.02 {
                    continue;
                }
                let (wx, wy) = (origin.0 + x as i32, origin.1 + y as i32);
                let old = m.alpha_at_world(pos2(wx as f32 + 0.5, wy as f32 + 0.5)) as f32 / 255.0;
                let nv = if on { old.max(v) } else { old * (1.0 - v) };
                m.set_alpha_world(wx, wy, (nv * 255.0).round() as u8);
                n += 1;
            }
        }
        n
    }

    /// A live wire seeded at `at` (a 768 px window of what lies under the masked layer).
    fn scissor_wire(&self, mid: LayerId, at: Pos2) -> Option<crate::segment::LiveWire> {
        let owner = self.doc.get(mid).and_then(|l| l.mask_of)?;
        let reg = Rect::from_center_size(at, vec2(768.0, 768.0));
        let reg = Rect::from_min_max(reg.min.floor(), reg.max.ceil());
        let mut d = self.doc.clone();
        d.layers.retain(|l| l.id != owner && l.mask_of.is_none());
        let pm = crate::export::render(&d, reg, 1.0, Some(Color32::BLACK)).ok()?;
        let (pw, ph) = (pm.width() as usize, pm.height() as usize);
        let px = pm.data();
        let lum: Vec<f32> = (0..pw * ph).map(|i| (0.299 * px[i * 4] as f32 + 0.587 * px[i * 4 + 1] as f32 + 0.114 * px[i * 4 + 2] as f32) / 255.0).collect();
        let seed = ((at.x - reg.min.x) as usize, (at.y - reg.min.y) as usize);
        Some(crate::segment::LiveWire::new(&lum, pw, ph, (reg.min.x as i32, reg.min.y as i32), seed))
    }

    /// The outline from the last anchor to `w`: along the live wire, or straight.
    fn scissor_path(sc: &Scissor, w: Pos2) -> Vec<Pos2> {
        match sc.wire.as_ref().and_then(|wr| wr.path_to(w.x.floor() as i32, w.y.floor() as i32)) {
            Some(p) => p.into_iter().map(|(x, y)| pos2(x as f32 + 0.5, y as f32 + 0.5)).collect(),
            None => vec![w],
        }
    }

    fn scissor_click(&mut self, mid: LayerId, w: Pos2, on: bool, refine: bool) {
        let near_first = self.scissor.as_ref().is_some_and(|sc| sc.pts.len() > 2 && (sc.pts[0] - w).length() * self.zoom < 10.0);
        if near_first {
            self.scissor_close(refine);
            return;
        }
        match self.scissor.take() {
            Some(mut sc) if sc.mid == mid => {
                let path = Self::scissor_path(&sc, w);
                sc.pts.extend(path);
                sc.wire = self.scissor_wire(mid, w);
                self.scissor = Some(sc);
            }
            _ => {
                let wire = self.scissor_wire(mid, w);
                self.scissor = Some(Scissor { mid, on, pts: vec![w], wire });
                self.status = "가위 · 경계를 따라 클릭 · 첫 점/Enter: 닫기 · Esc: 취소".into();
            }
        }
    }

    /// Closes the outline and fills it into the mask (anti-aliased).
    /// `refine`: the outline seeds GrabCut (deep inside: certain, a band along the
    /// outline: undecided, outside: background) instead of filling as drawn.
    fn scissor_close(&mut self, refine: bool) {
        let Some(sc) = self.scissor.take() else { return };
        if sc.pts.len() < 3 {
            return;
        }
        let b = Rect::from_points(&sc.pts);
        let b = Rect::from_min_max(b.min.floor() - vec2(1.0, 1.0), b.max.ceil() + vec2(1.0, 1.0));
        let (w, h) = (b.width() as u32, b.height() as u32);
        if w == 0 || h == 0 || w > 8192 || h > 8192 {
            return;
        }
        let Some(mut pm) = tiny_skia::Pixmap::new(w, h) else { return };
        let mut pb = tiny_skia::PathBuilder::new();
        pb.move_to(sc.pts[0].x - b.min.x, sc.pts[0].y - b.min.y);
        for p in &sc.pts[1..] {
            pb.line_to(p.x - b.min.x, p.y - b.min.y);
        }
        pb.close();
        let Some(path) = pb.finish() else { return };
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(255, 255, 255, 255);
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
        if refine {
            self.scissor_refine(sc.mid, b, &pm, sc.on);
            return;
        }
        let Some(LayerKind::Raster(m)) = self.doc.get_mut(sc.mid).map(|l| &mut l.kind) else { return };
        let data = pm.data();
        let mut n = 0;
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let a = data[((y * w as i32 + x) * 4 + 3) as usize] as f32 / 255.0;
                if a <= 0.0 {
                    continue;
                }
                let (wx, wy) = (b.min.x as i32 + x, b.min.y as i32 + y);
                let old = m.alpha_at_world(pos2(wx as f32 + 0.5, wy as f32 + 0.5)) as f32 / 255.0;
                let nv = if sc.on { old.max(a) } else { old * (1.0 - a) };
                m.set_alpha_world(wx, wy, (nv * 255.0).round() as u8);
                n += 1;
            }
        }
        self.status = format!("가위 · {n} px {}", if sc.on { "가림" } else { "되살림" });
        self.mark();
    }

    fn scissor_refine(&mut self, mid: LayerId, b: Rect, poly: &tiny_skia::Pixmap, on: bool) {
        use crate::segment::{BG, FG, PR_BG, PR_FG};
        let Some(owner) = self.doc.get(mid).and_then(|l| l.mask_of) else { return };
        const PAD: i32 = 16;
        const BAND: usize = 6;
        let reg = Rect::from_min_max(b.min - vec2(PAD as f32, PAD as f32), b.max + vec2(PAD as f32, PAD as f32));
        let mut d = self.doc.clone();
        d.layers.retain(|l| l.id != owner && l.mask_of.is_none());
        let Ok(pm) = crate::export::render(&d, reg, 1.0, Some(Color32::BLACK)) else { return };
        let (pw, ph) = (pm.width() as usize, pm.height() as usize);
        let px = pm.data();
        let (qw, qh) = (poly.width() as i32, poly.height() as i32);
        let pd = poly.data();
        let mut inside = vec![false; pw * ph];
        for y in 0..ph {
            for x in 0..pw {
                let (qx, qy) = (x as i32 - PAD, y as i32 - PAD);
                if qx >= 0 && qy >= 0 && qx < qw && qy < qh {
                    inside[y * pw + x] = pd[((qy * qw + qx) * 4 + 3) as usize] > 127;
                }
            }
        }
        // erode / dilate by BAND for the certain core and the outer limit
        let morph = |src: &Vec<bool>, grow: bool| {
            let mut cur = src.clone();
            for _ in 0..BAND {
                let prev = cur.clone();
                for y in 0..ph {
                    for x in 0..pw {
                        let mut v = prev[y * pw + x];
                        for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                            let n = nx >= 0 && ny >= 0 && (nx as usize) < pw && (ny as usize) < ph && prev[ny as usize * pw + nx as usize];
                            v = if grow { v || n } else { v && n };
                        }
                        cur[y * pw + x] = v;
                    }
                }
            }
            cur
        };
        let core = morph(&inside, false);
        let outer = morph(&inside, true);
        let step = (((pw * ph) as f64 / 90_000.0).sqrt().ceil() as usize).max(1);
        let (gw, gh) = (pw.div_ceil(step), ph.div_ceil(step));
        let mut img = vec![[0f64; 3]; gw * gh];
        let mut labels = vec![BG; gw * gh];
        for gy in 0..gh {
            for gx in 0..gw {
                let (cx, cy) = ((gx * step + step / 2).min(pw - 1), (gy * step + step / 2).min(ph - 1));
                let i = (cy * pw + cx) * 4;
                img[gy * gw + gx] = [px[i] as f64, px[i + 1] as f64, px[i + 2] as f64];
                let j = cy * pw + cx;
                labels[gy * gw + gx] = if core[j] { FG } else if inside[j] { PR_FG } else if outer[j] { PR_BG } else { BG };
            }
        }
        crate::segment::grabcut(&img, gw, gh, &mut labels, 4);
        let mut gi = vec![0f32; pw * ph];
        let mut gp = vec![0f32; pw * ph];
        for y in 0..ph {
            for x in 0..pw {
                let i = (y * pw + x) * 4;
                gi[y * pw + x] = (0.299 * px[i] as f32 + 0.587 * px[i + 1] as f32 + 0.114 * px[i + 2] as f32) / 255.0;
                gp[y * pw + x] = upsample_fg(&labels, gw, gh, step, x, y);
            }
        }
        let n = self.mask_soft_write(mid, (reg.min.x as i32, reg.min.y as i32), pw, ph, &gi, &gp, on);
        self.status = format!("가위+그랩컷 · {n} px {}", if on { "가림" } else { "되살림" });
        self.mark();
    }

    /// GrabCut over a dragged box: what is inside the box and stands apart from its
    /// surroundings (colour models + edges) is hidden (or shown again). Coverage
    /// already painted in the box counts as certain.
    fn grabcut_fill(&mut self, mid: LayerId, rect: Rect, on: bool) {
        use crate::segment::{BG, FG, PR_FG};
        let Some(owner) = self.doc.get(mid).and_then(|l| l.mask_of) else { return };
        let rect = Rect::from_min_max(rect.min.floor(), rect.max.ceil());
        if rect.width() < 4.0 || rect.height() < 4.0 {
            return;
        }
        let margin = (rect.size().max_elem() * 0.1).max(12.0).round();
        let reg = rect.expand(margin);
        if reg.width() > 6000.0 || reg.height() > 6000.0 {
            self.status = "영역이 너무 큼".into();
            return;
        }
        let mut d = self.doc.clone();
        d.layers.retain(|l| l.id != owner && l.mask_of.is_none());
        let Ok(pm) = crate::export::render(&d, reg, 1.0, Some(Color32::BLACK)) else { return };
        let (pw, ph) = (pm.width() as usize, pm.height() as usize);
        let px = pm.data();
        // the cut runs on a copy of at most ~90k pixels
        let step = (((pw * ph) as f64 / 90_000.0).sqrt().ceil() as usize).max(1);
        let (gw, gh) = (pw.div_ceil(step), ph.div_ceil(step));
        let hint = match self.doc.get(mid).map(|l| &l.kind) {
            Some(LayerKind::Raster(m)) => Some(m.clone()),
            _ => None,
        };
        let mut img = vec![[0f64; 3]; gw * gh];
        let mut labels = vec![BG; gw * gh];
        for gy in 0..gh {
            for gx in 0..gw {
                let (mut acc, mut cnt) = ([0f64; 3], 0.0);
                for y in gy * step..((gy + 1) * step).min(ph) {
                    for x in gx * step..((gx + 1) * step).min(pw) {
                        let i = (y * pw + x) * 4;
                        acc[0] += px[i] as f64;
                        acc[1] += px[i + 1] as f64;
                        acc[2] += px[i + 2] as f64;
                        cnt += 1.0;
                    }
                }
                img[gy * gw + gx] = [acc[0] / cnt, acc[1] / cnt, acc[2] / cnt];
                let wp = pos2(reg.min.x + ((gx * step) as f32 + step as f32 * 0.5), reg.min.y + ((gy * step) as f32 + step as f32 * 0.5));
                if rect.contains(wp) {
                    let painted = on && hint.as_ref().is_some_and(|m| m.alpha_at_world(wp) > 127);
                    labels[gy * gw + gx] = if painted { FG } else { PR_FG };
                }
            }
        }
        crate::segment::grabcut(&img, gw, gh, &mut labels, 4);
        let mut gi = vec![0f32; pw * ph];
        let mut gp = vec![0f32; pw * ph];
        for y in 0..ph {
            for x in 0..pw {
                let i = (y * pw + x) * 4;
                gi[y * pw + x] = (0.299 * px[i] as f32 + 0.587 * px[i + 1] as f32 + 0.114 * px[i + 2] as f32) / 255.0;
                gp[y * pw + x] = upsample_fg(&labels, gw, gh, step, x, y);
            }
        }
        let n = self.mask_soft_write(mid, (reg.min.x as i32, reg.min.y as i32), pw, ph, &gi, &gp, on);
        self.status = format!("그랩컷 · {n} px {}", if on { "가림" } else { "되살림" });
        self.mark();
    }

    /// The mask button of a layer row: selects its mask, or makes one (anchored to
    /// the layer) and picks the brush.
    fn mask_button(&mut self, owner: LayerId) {
        if let Some(mid) = self.doc.layers.iter().find(|m| m.mask_of == Some(owner)).map(|m| m.id) {
            self.select_only(Some(mid));
            return;
        }
        let Some(i) = self.doc.index_of(owner) else { return };
        let name = format!("{} 마스크", self.doc.layers[i].display_name());
        let c = crate::anim::key_of(&self.doc.layers[i], 0.0).center;
        let id = self.doc.new_id();
        let layer = Layer {
            id,
            name,
            visible: true,
            locked: false,
            opacity: 1.0,
            kind: LayerKind::Raster(Default::default()),
            place: None,
            anim: None,
            mask_of: Some(owner),
            mask_anchor: Some((owner, c)),
            mask_pose: None,
            mask_invert: false,
        };
        // just under its layer, so the list shows it beneath
        self.doc.layers.insert(i, layer);
        self.select_only(Some(id));
        if !matches!(self.tool, Tool::Brush | Tool::Eraser | Tool::Pen | Tool::Rect) {
            self.set_tool(Tool::Brush);
        }
        self.status = "마스크 · 칠한 곳 가림 · 지우개: 되살리기 · 사각형: 가리기 (Alt: 되살리기)".into();
        self.mark();
    }

    /// Records a layer's current state as a key at the playhead (onto a key
    /// already within half a frame).
    fn key_layer(&mut self, id: LayerId, kc: u16) {
        let t = self.doc.timeline.time;
        let tol = self.doc.timeline.frame_len() * 0.5;
        if let Some(l) = self.doc.get_mut(id) {
            let kt = l.anim.as_ref().and_then(|a| a.keys.iter().map(|k| k.t).find(|kt| (kt - t).abs() <= tol)).unwrap_or(t);
            let mut key = crate::anim::key_of(l, kt);
            key.ch = kc;
            l.anim.get_or_insert_with(Default::default).set_key(key);
            self.status = format!("키 · {}", crate::video::timecode(kt));
            self.mark();
        }
    }

    fn key_camera(&mut self, i: usize, kc: u16) {
        let t = self.doc.timeline.time;
        let tol = self.doc.timeline.frame_len() * 0.5;
        if let Some(c) = self.doc.cameras.get_mut(i) {
            let kt = c.anim.keys.iter().map(|k| k.t).find(|kt| (kt - t).abs() <= tol).unwrap_or(t);
            let mut k = c.key(kt);
            k.ch = kc;
            c.anim.set_key(k);
            self.status = format!("{} 키 · {}", c.name, crate::video::timecode(kt));
            self.mark();
        }
    }

    fn delete_selected_keys(&mut self) {
        let sel = std::mem::take(&mut self.tl_sel);
        let n = sel.len();
        for (row, t) in sel {
            if let Some(a) = self.row_anim_mut(row) {
                a.keys.retain(|k| (k.t - t).abs() >= 1e-6);
            }
        }
        self.tl_key = None;
        if n > 0 {
            self.set_time(self.doc.timeline.time);
            self.mark();
        }
        self.status = format!("키 {n}개 지움");
    }

    /// Alt+I: removes the selection's keys at the playhead.
    fn delete_keys_here(&mut self) {
        let t = self.doc.timeline.time;
        let tol = self.doc.timeline.frame_len() * 0.5;
        let mut n = 0;
        if self.tool == Tool::Frame {
            if let Some(c) = self.cam_sel.and_then(|id| self.doc.cameras.iter_mut().find(|c| c.id == id)) {
                n += c.anim.remove_near(t, tol) as usize;
            }
        }
        for id in self.selected_ids() {
            if let Some(a) = self.doc.get_mut(id).and_then(|l| l.anim.as_mut()) {
                n += a.remove_near(t, tol) as usize;
            }
        }
        if n > 0 {
            self.mark();
        }
        self.status = format!("키 {n}개 지움");
    }

    fn row_anim(&self, row: TlRow) -> Option<&crate::anim::Anim> {
        match row {
            TlRow::Audio(id) => self.doc.audio.iter().find(|a| a.id == id).map(|a| &a.anim),
            TlRow::Layer(id) => self.doc.get(id).and_then(|l| l.anim.as_ref()),
            TlRow::Cam(id) => self.doc.cameras.iter().find(|c| c.id == id).map(|c| &c.anim),
        }
    }

    fn row_anim_mut(&mut self, row: TlRow) -> Option<&mut crate::anim::Anim> {
        match row {
            TlRow::Audio(id) => self.doc.audio.iter_mut().find(|a| a.id == id).map(|a| &mut a.anim),
            TlRow::Layer(id) => self.doc.get_mut(id).map(|l| l.anim.get_or_insert_with(Default::default)),
            TlRow::Cam(id) => self.doc.cameras.iter_mut().find(|c| c.id == id).map(|c| &mut c.anim),
        }
    }

    fn cameras_panel(&mut self, ui: &mut egui::Ui) {
        if !Self::fold_section(ui, "카메라", &mut self.s.folded[2]) {
            return;
        }
        ui.small("영역 → 카메라 · 손잡이: 크기 · I: 키 · 구간 밖: 꺼짐");
        let mut pick = None;
        let mut add = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for c in &self.doc.cameras {
                if ui.selectable_label(self.cam_sel == Some(c.id), egui::RichText::new(&c.name).color(CAM_COLOR)).clicked() {
                    pick = Some(c.id);
                }
            }
            add = ui.button("+ 카메라").on_hover_text("내보낼 영역(없으면 화면)으로 새 카메라").clicked();
        });
        if let Some(id) = pick {
            self.cam_sel = if self.cam_sel == Some(id) { None } else { Some(id) };
        }
        if add {
            let rect = self.doc.export_frame.unwrap_or_else(|| {
                let xf = self.xf();
                Rect::from_min_max(xf.s2w(self.canvas_rect.min), xf.s2w(self.canvas_rect.max)).shrink(40.0 / self.zoom)
            });
            let id = self.doc.new_id();
            let n = self.doc.cameras.len() + 1;
            self.doc.cameras.push(crate::anim::CanvasCam { id, name: format!("카메라 {n}"), rect: Rect::from_min_max(rect.min.round(), rect.max.round()), anim: Default::default(), rot: 0.0, follow: None });
            self.cam_sel = Some(id);
            self.mark();
        }
        let Some(i) = self.cam_sel.and_then(|id| self.doc.cameras.iter().position(|c| c.id == id)) else { return };
        let mut ch = false;
        let (mut del, mut to_frame, mut from_frame) = (false, false, false);
        let tl = &self.doc.timeline;
        let (tl_on, tt, tol) = (tl.on, tl.time, tl.frame_len() * 0.5);
        let can = self.doc.cameras[i].anim.clone();
        use crate::anim::ch;
        let cm = |c: u16| tl_on.then(|| key_mark(Some(&can), tt, tol, c));
        // rect, rotation, follow
        let mut ck = [false; 3];
        {
            let c = &mut self.doc.cameras[i];
            ui.horizontal(|ui| {
                ui.label("이름");
                ch |= ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(ui.available_width())).changed();
            });
            ui.label(format!("키 {}개", c.anim.keys.len()));
            ch |= rect_fields(ui, &mut c.rect, "cam", cm(ch::POS | ch::SIZE), &mut ck[0]);
            let mut deg = c.rot.to_degrees();
            if ui.add(crate::dial::Dial::new(&mut deg, -180.0..=180.0).step(1.0).text("회전 °").default(0.0).key(cm(ch::ROT), &mut ck[1])).changed() {
                c.rot = deg.to_radians();
                ch = true;
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                from_frame = ui.add_enabled(self.doc.export_frame.is_some(), egui::Button::new("영역 → 카메라")).clicked();
                to_frame = ui.button("카메라 → 영역").clicked();
                del = ui.button("카메라 삭제").clicked();
            });
        }
        // centre on a layer, or follow one (constraint)
        let targets: Vec<(LayerId, String)> = self.doc.layers.iter().rev().map(|l| (l.id, l.display_name())).collect();
        let mut snap_to = None;
        let follow = self.doc.cameras[i].follow;
        let mut new_follow = follow;
        let picked = self.sel;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label("객체 중심에");
            egui::ComboBox::from_id_salt(("cam_snap", self.doc.cameras[i].id)).truncate()
                .selected_text("맞추기…")
                .width(84.0)
                .popup_style(theme::glass_popup_style(false))
                .show_ui(ui, |ui| {
                    theme::glass_popup(ui, |ui| {
                        for (id, name) in &targets {
                            if ui.selectable_label(false, name.as_str()).clicked() {
                                snap_to = Some(*id);
                            }
                        }
                    })
                });
            if ui.add_enabled(picked.is_some(), egui::Button::new("선택")).on_hover_text("선택한 객체 중심으로").clicked() {
                snap_to = picked;
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if let Some(m) = cm(ch::FOLLOW) {
                ck[2] |= crate::dial::key_button(ui, m);
            }
            ui.label("따라가기");
            let shown = follow.and_then(|f| targets.iter().find(|t| t.0 == f.layer)).map_or("없음".to_string(), |t| t.1.clone());
            egui::ComboBox::from_id_salt(("cam_follow", self.doc.cameras[i].id)).truncate()
                .selected_text(shown)
                .width(ui.available_width() - 24.0)
                .popup_style(theme::glass_popup_style(false))
                .show_ui(ui, |ui| {
                    theme::glass_popup(ui, |ui| {
                        if ui.selectable_label(follow.is_none(), "없음").clicked() {
                            new_follow = None;
                        }
                        for (id, name) in &targets {
                            if ui.selectable_label(follow.is_some_and(|f| f.layer == *id), name.as_str()).clicked() {
                                new_follow = Some(crate::anim::Follow { layer: *id, pos: true, rot: false, offset: [0.0, 0.0], rot_offset: 0.0 });
                            }
                        }
                    })
                });
        });
        let bind = picked.filter(|p| follow.is_none_or(|f| f.layer != *p));
        if ui.add_enabled(bind.is_some(), egui::Button::new("선택한 객체 따라가기")).on_hover_text("위치 따라가기로 묶기").clicked() {
            if let Some(layer) = bind {
                new_follow = Some(crate::anim::Follow { layer, pos: true, rot: false, offset: [0.0, 0.0], rot_offset: 0.0 });
            }
        }
        if let Some(mut f) = new_follow {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                let mut c2 = false;
                c2 |= ui.checkbox(&mut f.pos, "위치").changed();
                c2 |= ui.checkbox(&mut f.rot, "회전").changed();
                if ui.button("지금 배치 유지").on_hover_text("지금 간격·각도 차이 유지").clicked() {
                    if let Some((p, a)) = self.doc.get(f.layer).and_then(|l| crate::anim::target_pose(&self.doc, l)) {
                        let c = &self.doc.cameras[i];
                        let d = c.rect.center() - p;
                        f.offset = [d.x, d.y];
                        f.rot_offset = c.rot - a;
                        c2 = true;
                    }
                }
                if c2 {
                    new_follow = Some(f);
                }
            });
        }
        if new_follow != follow {
            self.doc.cameras[i].follow = new_follow;
            ch = true;
        }
        if let Some(id) = snap_to {
            if let Some((p, _)) = self.doc.get(id).and_then(|l| crate::anim::target_pose(&self.doc, l)) {
                let c = &mut self.doc.cameras[i];
                c.rect = Rect::from_center_size(p, c.rect.size());
                ch = true;
            }
        }
        if from_frame {
            if let Some(f) = self.doc.export_frame {
                self.doc.cameras[i].rect = f;
                ch = true;
            }
        }
        if to_frame {
            self.doc.export_frame = Some(self.doc.cameras[i].rect);
            ch = true;
        }
        let kc = [ch::POS | ch::SIZE, ch::ROT, ch::FOLLOW].iter().zip(ck).filter(|x| x.1).fold(0u16, |a, (c, _)| a | c);
        if kc != 0 && !del {
            self.key_camera(i, kc);
        }
        if del {
            self.doc.cameras.remove(i);
            self.cam_sel = None;
            ch = true;
        }
        if ch {
            self.mark();
        }
    }

    fn timeline_ui(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        // full width along the bottom; the property columns stop above it
        // the same margin on both sides (the left one clears the tool bar)
        let width = (screen.width() - 70.0 * 2.0 - 20.0).max(360.0);
        let resp = egui::Area::new(egui::Id::new("timeline"))
            .order(egui::Order::Middle)
            .anchor(Align2::LEFT_BOTTOM, vec2(70.0, -64.0))
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::same(10), theme::RADIUS_CARD, |ui| {
                    ui.set_width(width);
                    self.timeline_body(ui, width);
                });
            });
        self.tl_h = resp.response.rect.height();
    }

    fn timeline_body(&mut self, ui: &mut egui::Ui, width: f32) {
        let playing = self.tl_play.is_some();
        let mut act_play = false;
        let mut new_time: Option<f64> = None;
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui.button(if playing { "멈춤" } else { "재생" }).on_hover_text("Alt+A").clicked() {
                act_play = true;
            }
            if ui.button("처음").on_hover_text("Shift+,").clicked() {
                new_time = Some(0.0);
            }
            let tl = &self.doc.timeline;
            ui.label(egui::RichText::new(format!("{} / {}", crate::video::timecode(tl.time), crate::video::timecode(tl.duration))).monospace());
            ui.label(egui::RichText::new(format!("{}f", (tl.time * tl.fps).round() as i64)).monospace().color(INK_3()));
            ui.separator();
            let tl = &mut self.doc.timeline;
            ui.label("fps");
            changed |= ui.add(crate::calc::drag(&mut tl.fps).range(1.0..=240.0).speed(0.1).max_decimals(3)).changed();
            ui.label("길이");
            changed |= ui.add(crate::calc::drag(&mut tl.duration).range(0.1..=36000.0).speed(0.1).suffix(" s").max_decimals(2)).changed();
            changed |= ui.checkbox(&mut tl.autokey, "자동 키").on_hover_text("선택 레이어 변경 시 자동으로 키").changed();
            let rec_label = egui::RichText::new("● 녹화").color(if self.rec_on { Color32::from_rgb(255, 90, 90) } else { INK_3() });
            if ui
                .selectable_label(self.rec_on, rec_label)
                .on_hover_text("재생 헤드부터 그리는 과정 기록 · 끝나면 재생 헤드가 획 끝으로")
                .clicked()
            {
                self.rec_on = !self.rec_on;
            }
            ui.separator();
            if ui.button("키 넣기").on_hover_text("I").clicked() {
                self.insert_keys();
            }
            if ui.button("키 지우기").on_hover_text("선택한 키 (Delete) · 없으면 재생 헤드의 키 (Alt+I)").clicked() {
                if self.tl_sel.is_empty() {
                    self.delete_keys_here();
                } else {
                    self.delete_selected_keys();
                }
            }
            ui.separator();
            if ui.button("영상 내보내기").on_hover_text("Ctrl+Shift+E").clicked() {
                self.open_video_export();
            }
        });
        // selected key: easing
        if let Some((row, kt)) = self.tl_key {
            let ease = self.row_anim(row).and_then(|a| a.keys.iter().find(|k| (k.t - kt).abs() < 1e-6)).map(|k| k.ease);
            match ease {
                Some(e) => {
                    let mut pick = None;
                    ui.horizontal(|ui| {
                        let n = self.tl_sel.len().max(1);
                        let what = if n > 1 { format!("선택한 키 {n}개") } else { format!("선택한 키 {}", crate::video::timecode(kt)) };
                        ui.label(egui::RichText::new(what).color(INK_2()));
                        ui.label("다음 키까지");
                        for o in crate::anim::Ease::ALL {
                            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                            key_icon(ui.painter(), ir.center(), o, if e == o { ACCENT } else { INK_2() }, Stroke::new(1.0, Color32::from_black_alpha(170)));
                            if ui.selectable_label(e == o, o.label()).clicked() {
                                pick = Some(o);
                            }
                        }
                    });
                    if let Some(o) = pick {
                        let mut targets = self.tl_sel.clone();
                        if targets.is_empty() {
                            targets.push((row, kt));
                        }
                        for (rw, t) in targets {
                            if let Some(k) = self.row_anim_mut(rw).and_then(|a| a.keys.iter_mut().find(|k| (k.t - t).abs() < 1e-6)) {
                                k.ease = o;
                                changed = true;
                            }
                        }
                    }
                }
                None => self.tl_key = None,
            }
        }
        if let Some(i) = self.aud_sel.and_then(|id| self.doc.audio.iter().position(|a| a.id == id)) {
            let mut del = false;
            ui.horizontal(|ui| {
                let a = &mut self.doc.audio[i];
                ui.label(egui::RichText::new(format!("소리 · {}", a.name)).color(AUDIO_COLOR));
                let tt = self.doc.timeline.time;
                let mark = if a.anim.keys.iter().any(|k| (k.t - tt).abs() < 1e-3 && k.ch & crate::anim::ch::VOL != 0) {
                    crate::dial::KeyMark::Here
                } else if a.anim.keys.is_empty() {
                    crate::dial::KeyMark::None
                } else {
                    crate::dial::KeyMark::Keyed
                };
                let mut hit = false;
                crate::dial::label_cell(ui, "볼륨", Some(mark), &mut hit);
                let vr = ui.add(crate::calc::drag(&mut a.gain).range(0.0..=4.0).speed(0.01).max_decimals(2));
                changed |= vr.changed();
                // auto-key: a changed volume keys itself
                if hit || (vr.changed() && self.doc.timeline.autokey) {
                    let k = a.vol_key(tt);
                    a.anim.set_key(k);
                    changed = true;
                }
                changed |= ui.checkbox(&mut a.mute, "음소거").changed();
                ui.label(egui::RichText::new(format!("{} ~ {}", crate::video::timecode(a.start), crate::video::timecode(a.end()))).monospace().color(INK_3()));
                del = ui.button("삭제").clicked();
            });
            if del {
                self.doc.audio.remove(i);
                self.aud_sel = None;
                changed = true;
            }
        }
        // rows: cameras, then layers with keys or a span, then the selection
        let sel = self.selected_ids();
        let mut rows: Vec<(TlRow, String, bool)> = self.doc.cameras.iter().map(|c| (TlRow::Cam(c.id), c.name.clone(), true)).collect();
        rows.extend(self.doc.audio.iter().map(|a| (TlRow::Audio(a.id), a.name.clone(), false)));
        for l in self.doc.layers.iter().rev() {
            let recorded = match &l.kind {
                LayerKind::Vector(v) => v.strokes.iter().any(|s| s.rec.is_some()),
                LayerKind::Raster(r) => !r.recs.is_empty(),
                _ => false,
            };
            if recorded || l.anim.as_ref().is_some_and(|a| !a.keys.is_empty() || a.span.is_some()) || sel.contains(&l.id) {
                rows.push((TlRow::Layer(l.id), l.display_name(), false));
            }
        }
        let shown = rows.len().min(8);
        let row_h = 22.0;
        let ruler_h = 20.0;
        let label_w = 140.0;
        let h = ruler_h + row_h * shown.max(1) as f32 + 4.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click_and_drag());
        let p = ui.painter().with_clip_rect(rect);
        let tl = self.doc.timeline.clone();
        let tr = Rect::from_min_max(pos2(rect.left() + label_w, rect.top()), pos2(rect.right() - 8.0, rect.bottom()));
        let dur = tl.duration.max(1e-3);
        let x_of = |t: f64| tr.left() + (t / dur) as f32 * tr.width();
        let t_of = |x: f32| ((x - tr.left()) / tr.width()) as f64 * dur;
        // ruler
        let px_per_s = tr.width() as f64 / dur;
        let step = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0].into_iter().find(|s| s * px_per_s >= 50.0).unwrap_or(600.0);
        // frame ticks at the real fps; when they would crowd closer than 3 physical
        // pixels, every 2nd, then every 4th, … frame
        let fps = tl.fps.max(1.0);
        let ppp = ui.ctx().pixels_per_point() as f64;
        let mut every = 1u64;
        while px_per_s / fps * every as f64 * ppp < 3.0 && every < (1 << 24) {
            every *= 2;
        }
        let frames = (dur * fps + 1e-6).floor() as u64;
        let tick = Stroke::new(1.0 / ui.ctx().pixels_per_point(), INK_3().gamma_multiply(0.55));
        let mut f = 0u64;
        while f <= frames {
            let x = x_of(f as f64 / fps);
            p.line_segment([pos2(x, rect.top() + ruler_h - 5.0), pos2(x, rect.top() + ruler_h)], tick);
            f += every;
        }
        let mut t = 0.0;
        while t <= dur + 1e-9 {
            let x = x_of(t);
            p.line_segment([pos2(x, rect.top() + 12.0), pos2(x, rect.top() + ruler_h)], Stroke::new(1.0, INK_3()));
            let label = if step < 1.0 { format!("{t:.1}") } else { format!("{}", t.round() as i64) };
            p.text(pos2(x + 3.0, rect.top() + 1.0), Align2::LEFT_TOP, label, FontId::proportional(10.0), INK_3());
            t += step;
        }
        // rows
        let mut hit_rows: Vec<(TlRow, Rect)> = Vec::new();
        for (i, (row, name, cam)) in rows.iter().take(shown).enumerate() {
            let y0 = rect.top() + ruler_h + i as f32 * row_h;
            let rr = Rect::from_min_max(pos2(rect.left(), y0), pos2(rect.right(), y0 + row_h));
            hit_rows.push((*row, rr));
            let selected = match row {
                TlRow::Layer(id) => sel.contains(id),
                TlRow::Cam(id) => self.cam_sel == Some(*id),
                TlRow::Audio(id) => self.aud_sel == Some(*id),
            };
            if selected {
                p.rect_filled(rr, 6.0, FILL_1());
            }
            let col = if *cam {
                CAM_COLOR
            } else if matches!(row, TlRow::Audio(_)) {
                AUDIO_COLOR
            } else {
                INK_2()
            };
            let g = p.layout_job(elided(name.to_string(), FontId::proportional(theme::TEXT_SMALL + 0.5), col, label_w - 14.0));
            p.galley(pos2(rr.left() + 6.0, rr.center().y - g.size().y * 0.5), g, col);
            let cy = rr.center().y;
            if let TlRow::Audio(aid) = row {
                if let Some(a) = self.doc.audio.iter().find(|a| a.id == *aid) {
                    let bar = Rect::from_min_max(pos2(x_of(a.start), rr.top() + 3.0), pos2(x_of(a.end()).max(x_of(a.start) + 2.0), rr.bottom() - 3.0));
                    p.rect_filled(bar, 3.0, AUDIO_COLOR.gamma_multiply(if a.mute { 0.08 } else { 0.22 }));
                    // waveform
                    if let Some(Some(d)) = self.audio_cache.get(&a.path) {
                        let x0 = bar.left().max(tr.left());
                        let x1 = bar.right().min(tr.right());
                        let mut x = x0;
                        while x < x1 {
                            let src = a.offset + t_of(x) - a.start;
                            let i = (src / crate::audio::PEAK_SEC) as usize;
                            let pk = d.peaks.get(i).copied().unwrap_or(0.0) * a.gain.min(1.5);
                            let hh = (bar.height() * 0.5 - 1.0) * pk.min(1.0);
                            p.line_segment([pos2(x, cy - hh), pos2(x, cy + hh)], Stroke::new(1.0, AUDIO_COLOR.gamma_multiply(if a.mute { 0.3 } else { 0.9 })));
                            x += 2.0;
                        }
                    } else if !a.path.is_empty() {
                        p.text(bar.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, "불러오는 중…", FontId::proportional(10.0), INK_3());
                    }
                    if self.aud_sel == Some(*aid) {
                        p.rect_stroke(bar, 3.0, Stroke::new(1.2, AUDIO_COLOR), StrokeKind::Inside);
                    }
                }
                continue;
            }
            if let TlRow::Layer(lid) = row {
                // recorded strokes: thin red marks along the bottom of the row
                for (a, b) in self.rec_spans(*lid) {
                    let x0 = x_of(a);
                    let x1 = x_of(b).max(x0 + 2.0);
                    p.rect_filled(Rect::from_min_max(pos2(x0, rr.bottom() - 5.0), pos2(x1, rr.bottom() - 2.0)), 1.0, Color32::from_rgb(255, 90, 90).gamma_multiply(0.8));
                }
            }
            let anim = self.row_anim(*row);
            match anim.and_then(|a| a.span) {
                Some([a, b]) => {
                    let bar = Rect::from_min_max(pos2(x_of(a), cy - 4.0), pos2(x_of(b), cy + 4.0));
                    p.rect_filled(bar, 3.0, col.gamma_multiply(0.35));
                    for x in [bar.left(), bar.right()] {
                        p.rect_filled(Rect::from_center_size(pos2(x, cy), vec2(4.0, 12.0)), 1.5, col);
                    }
                }
                None => {
                    p.line_segment([pos2(tr.left(), cy), pos2(tr.right(), cy)], Stroke::new(1.0, col.gamma_multiply(0.25)));
                }
            }
            if let Some(a) = anim {
                // the curve between keys shows how it eases: straight, S-shaped or a step
                for w2 in a.keys.windows(2) {
                    let (x0, x1) = (x_of(w2[0].t), x_of(w2[1].t));
                    if x1 - x0 < 6.0 {
                        continue;
                    }
                    let (y0, y1) = (cy + 4.0, cy - 4.0);
                    let pts: Vec<Pos2> = match w2[0].ease {
                        crate::anim::Ease::Linear => vec![pos2(x0, y0), pos2(x1, y1)],
                        crate::anim::Ease::Hold => vec![pos2(x0, y0), pos2(x1, y0), pos2(x1, y1)],
                        crate::anim::Ease::Smooth => (0..=16)
                            .map(|i| {
                                let u = i as f32 / 16.0;
                                pos2(x0 + (x1 - x0) * u, y0 + (y1 - y0) * u * u * (3.0 - 2.0 * u))
                            })
                            .collect(),
                    };
                    p.add(Shape::line(pts, Stroke::new(1.0, col.gamma_multiply(0.45))));
                }
                for k in &a.keys {
                    let c = pos2(x_of(k.t), cy);
                    let on = self.tl_sel.iter().any(|(r, t)| r == row && (t - k.t).abs() < 1e-6) || self.tl_key.is_some_and(|(r, t)| r == *row && (t - k.t).abs() < 1e-6);
                    let fill = if on { ACCENT } else { INK() };
                    let edge = Stroke::new(1.0, Color32::from_black_alpha(170));
                    key_icon(&p, c, k.ease, fill, edge);
                }
            }
        }
        if rows.len() > shown {
            p.text(pos2(rect.left() + 6.0, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, format!("+{}개 더", rows.len() - shown), FontId::proportional(10.0), INK_3());
        }
        // playhead
        let px = x_of(tl.time);
        p.line_segment([pos2(px, rect.top()), pos2(px, rect.bottom())], Stroke::new(1.5, ACCENT));
        p.add(Shape::convex_polygon(vec![pos2(px - 5.0, rect.top()), pos2(px + 5.0, rect.top()), pos2(px, rect.top() + 7.0)], ACCENT, Stroke::NONE));

        // interaction
        let pointer = resp.interact_pointer_pos();
        if resp.drag_started() || resp.clicked() {
            if let Some(pp) = pointer {
                let mut drag = TlDrag::Scrub;
                let shift = ui.input(|i| i.modifiers.shift);
                if shift && pp.y > rect.top() + ruler_h && pp.x >= tr.left() {
                    drag = TlDrag::Marquee { start: pp };
                } else if pp.y > rect.top() + ruler_h {
                    if let Some((TlRow::Audio(aid), _)) = hit_rows.iter().find(|(_, rr)| rr.contains(pp)).copied() {
                        self.aud_sel = Some(aid);
                        if let Some(a) = self.doc.audio.iter().find(|a| a.id == aid) {
                            let t = t_of(pp.x);
                            if (x_of(a.start) - pp.x).abs() <= 5.0 {
                                drag = TlDrag::AudioIn(aid);
                            } else if (x_of(a.end()) - pp.x).abs() <= 5.0 {
                                drag = TlDrag::AudioOut(aid);
                            } else if t >= a.start && t <= a.end() {
                                drag = TlDrag::AudioMove { id: aid, grab: t - a.start };
                            }
                        }
                    } else if let Some((row, _)) = hit_rows.iter().find(|(_, rr)| rr.contains(pp)).copied() {
                        let anim = self.row_anim(row);
                        let key = anim.and_then(|a| a.keys.iter().find(|k| (x_of(k.t) - pp.x).abs() <= 6.0)).map(|k| k.t);
                        let span = anim.and_then(|a| a.span);
                        if let Some(kt) = key {
                            let picked = self.tl_sel.iter().any(|(r, t)| *r == row && (t - kt).abs() < 1e-6);
                            self.tl_key = Some((row, kt));
                            if picked && self.tl_sel.len() > 1 {
                                drag = TlDrag::Keys { last: kt };
                            } else {
                                self.tl_sel = vec![(row, kt)];
                                drag = TlDrag::Key { row, from: kt };
                            }
                        } else if span.is_some_and(|[a, _]| (x_of(a) - pp.x).abs() <= 5.0) {
                            drag = TlDrag::SpanIn(row);
                        } else if span.is_some_and(|[_, b]| (x_of(b) - pp.x).abs() <= 5.0) {
                            drag = TlDrag::SpanOut(row);
                        } else if pp.x < tr.left() {
                            // the label selects the layer or camera
                            match row {
                                TlRow::Layer(id) => self.select_only(Some(id)),
                                TlRow::Cam(id) => {
                                    self.cam_sel = Some(id);
                                    self.tool = Tool::Frame;
                                }
                                TlRow::Audio(id) => self.aud_sel = Some(id),
                            }
                            self.tl_drag = None;
                            return;
                        }
                    }
                }
                if drag == TlDrag::Scrub {
                    self.tl_sel.clear();
                }
                self.tl_drag = Some(drag);
            }
        }
        if let (Some(d), Some(pp)) = (self.tl_drag, pointer) {
            let t = tl.snap(t_of(pp.x));
            match d {
                TlDrag::Marquee { start } => {
                    let mr = Rect::from_two_pos(start, pp);
                    ui.painter().rect(mr, 2.0, ACCENT.gamma_multiply(0.12), Stroke::new(1.0, ACCENT), StrokeKind::Inside);
                }
                TlDrag::Keys { last } => {
                    let dlt = t - last;
                    if dlt.abs() > 1e-9 {
                        let sel = self.tl_sel.clone();
                        let mut moved = Vec::new();
                        for (row, kt) in sel {
                            if let Some(a) = self.row_anim_mut(row) {
                                if let Some(k) = a.keys.iter_mut().find(|k| (k.t - kt).abs() < 1e-6) {
                                    k.t = (kt + dlt).max(0.0);
                                    moved.push((row, k.t));
                                }
                                a.sort();
                            }
                        }
                        self.tl_sel = moved;
                        self.tl_key = self.tl_sel.first().copied();
                        self.tl_drag = Some(TlDrag::Keys { last: t });
                        changed = true;
                    }
                }
                TlDrag::Scrub => {
                    if (t - self.doc.timeline.time).abs() > 1e-9 {
                        new_time = Some(t);
                    }
                }
                TlDrag::Key { row, from } => {
                    if (t - from).abs() > 1e-9 {
                        if let Some(a) = self.row_anim_mut(row) {
                            if !a.keys.iter().any(|k| (k.t - t).abs() < 1e-6) {
                                if let Some(k) = a.keys.iter_mut().find(|k| (k.t - from).abs() < 1e-6) {
                                    k.t = t;
                                }
                                a.sort();
                                self.tl_drag = Some(TlDrag::Key { row, from: t });
                                self.tl_key = Some((row, t));
                                self.tl_sel = vec![(row, t)];
                                changed = true;
                            }
                        }
                    }
                }
                TlDrag::AudioMove { id, grab } => {
                    let fl = tl.frame_len();
                    if let Some(a) = self.doc.audio.iter_mut().find(|a| a.id == id) {
                        let ns = tl.snap(t_of(pp.x) - grab).max(0.0);
                        if (ns - a.start).abs() > fl * 0.5 {
                            a.start = ns;
                            changed = true;
                        }
                    }
                }
                TlDrag::AudioIn(id) | TlDrag::AudioOut(id) => {
                    let fl = tl.frame_len();
                    let src_len = self.doc.audio.iter().find(|a| a.id == id).and_then(|a| self.audio_cache.get(&a.path)).and_then(|d| d.as_ref()).map(|d| d.duration());
                    if let Some(a) = self.doc.audio.iter_mut().find(|a| a.id == id) {
                        if matches!(d, TlDrag::AudioIn(_)) {
                            // trim the head: the sound stays where it is in time
                            let ns = t.clamp(a.start - a.offset, a.end() - fl);
                            let dlt = ns - a.start;
                            a.start = ns;
                            a.offset += dlt;
                            a.len -= dlt;
                        } else {
                            let max_end = src_len.map_or(f64::MAX, |l| a.start + (l - a.offset));
                            a.len = (t.min(max_end) - a.start).max(fl);
                        }
                        changed = true;
                    }
                }
                TlDrag::SpanIn(row) | TlDrag::SpanOut(row) => {
                    let fl = tl.frame_len();
                    let is_in = matches!(d, TlDrag::SpanIn(_));
                    if let Some(a) = self.row_anim_mut(row) {
                        let [mut x, mut y] = a.span.unwrap_or([0.0, dur]);
                        if is_in {
                            x = t.min(y - fl);
                        } else {
                            y = t.max(x + fl);
                        }
                        a.span = Some([x, y]);
                        changed = true;
                    }
                }
            }
        }
        if resp.drag_stopped() || (resp.clicked() && !resp.dragged()) {
            if let (Some(TlDrag::Marquee { start }), Some(pp)) = (self.tl_drag, pointer.or(resp.hover_pos())) {
                let mr = Rect::from_two_pos(start, pp);
                let mut picked = Vec::new();
                for (row, rr) in &hit_rows {
                    if rr.max.y < mr.min.y || rr.min.y > mr.max.y {
                        continue;
                    }
                    if let Some(a) = self.row_anim(*row) {
                        for k in &a.keys {
                            if mr.contains(pos2(x_of(k.t), rr.center().y)) {
                                picked.push((*row, k.t));
                            }
                        }
                    }
                }
                self.tl_key = picked.first().copied();
                self.tl_sel = picked;
            }
            if matches!(self.tl_drag, Some(TlDrag::Key { .. } | TlDrag::Keys { .. } | TlDrag::SpanIn(_) | TlDrag::SpanOut(_) | TlDrag::AudioMove { .. } | TlDrag::AudioIn(_) | TlDrag::AudioOut(_))) {
                self.mark();
                self.set_time(self.doc.timeline.time);
            }
            self.tl_drag = None;
        }
        // right-click on a row's empty track: show the layer from here / until here
        if resp.secondary_clicked() {
            if let Some(pp) = pointer.or(resp.hover_pos()) {
                if let Some((row, _)) = hit_rows.iter().find(|(_, rr)| rr.contains(pp)).copied() {
                    if pp.x >= tr.left() {
                        let t = tl.snap(t_of(pp.x));
                        if let Some(a) = self.row_anim_mut(row) {
                            let [x, y] = a.span.unwrap_or([0.0, dur]);
                            a.span = Some(if t - x < y - t { [t, y] } else { [x, t.max(t + 1e-3)] });
                            changed = true;
                        }
                    }
                }
            }
        }
        if act_play {
            self.tl_toggle_play();
        }
        if let Some(t) = new_time {
            if self.tl_play.is_some() {
                self.playing.clear();
                self.tl_play = None;
                self.set_time(t);
                self.tl_toggle_play();
            } else {
                self.set_time(t);
            }
        }
        if changed {
            self.mark();
        }
    }


    // ------------------------------------------------------------ audio

    /// The sound card, opened on first use (a failure is reported once).
    fn audio_output(&mut self) -> Option<&crate::audio::Output> {
        if self.audio_out.is_none() && !self.audio_failed {
            match crate::audio::Output::open() {
                Ok(o) => self.audio_out = Some(o),
                Err(e) => {
                    self.audio_failed = true;
                    self.status = format!("소리 출력 불가: {e}");
                }
            }
        }
        self.audio_out.as_ref()
    }

    fn audio_rate(&mut self) -> u32 {
        self.audio_output().map_or(48_000, |o| o.rate)
    }

    /// Starts decoding a file's sound if it is not cached yet.
    fn ensure_audio(&mut self, path: &str) {
        if path.is_empty() || self.audio_cache.contains_key(path) {
            return;
        }
        self.audio_cache.insert(path.to_string(), None);
        let rate = self.audio_rate();
        let tx = self.audio_tx.clone();
        let p = path.to_string();
        std::thread::spawn(move || {
            let r = crate::audio::decode(std::path::Path::new(&p), rate);
            let _ = tx.send((p, r));
        });
    }

    /// A sound file goes on the timeline at the playhead (the timeline turns on).
    fn add_audio(&mut self, p: &std::path::Path) {
        let id = self.doc.new_id();
        let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("소리").to_string();
        let path = p.to_string_lossy().to_string();
        self.doc.audio.push(crate::anim::AudioClip { id, name, path: path.clone(), start: self.doc.timeline.time, offset: 0.0, len: 0.0, gain: 1.0, mute: false, anim: Default::default() });
        self.doc.timeline.on = true;
        self.aud_sel = Some(id);
        self.ensure_audio(&path);
        self.mark();
    }

    /// Fingerprint of everything that can sound (to refresh a running mix).
    fn voices_sig(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for a in &self.doc.audio {
            // keyed volume: the keys, not the sampled value that moves while playing
            let keyed = a.anim.keys.iter().any(|k| k.ch & crate::anim::ch::VOL != 0);
            (a.id, a.start.to_bits(), a.offset.to_bits(), a.len.to_bits(), if keyed { 0 } else { a.gain.to_bits() }, a.mute).hash(&mut h);
            vol_keys_hash(&a.anim, &mut h);
        }
        for l in &self.doc.layers {
            if let LayerKind::Image(im) = &l.kind {
                if let Some(v) = &im.video {
                    (l.id, l.visible, v.mute, v.speed.to_bits(), l.anim.as_ref().and_then(|a| a.span).map(|s| (s[0].to_bits(), s[1].to_bits()))).hash(&mut h);
                    let keyed = l.anim.as_ref().is_some_and(|a| a.keys.iter().any(|k| k.ch & crate::anim::ch::VOL != 0));
                    if keyed {
                        vol_keys_hash(l.anim.as_ref().unwrap(), &mut h);
                    } else {
                        v.volume.to_bits().hash(&mut h);
                    }
                }
            }
        }
        h.finish()
    }

    fn tick_audio(&mut self) {
        // sources changed while the timeline plays (a layer deleted, a clip muted …)
        let sig = self.voices_sig();
        if sig != self.voice_sig {
            self.voice_sig = sig;
            if self.tl_play.is_some() {
                let v = self.timeline_voices();
                let t = self.doc.timeline.time;
                if let Some(o) = &self.audio_out {
                    if v.is_empty() {
                        o.stop();
                    } else {
                        o.play(v, t);
                    }
                }
            }
        }
        while let Ok((path, res)) = self.audio_rx.try_recv() {
            match res {
                Ok(d) => {
                    let dur = d.duration();
                    self.audio_cache.insert(path.clone(), Some(Arc::new(d)));
                    // first decode sets the clip length; a long clip stretches the timeline
                    let mut grow = 0.0f64;
                    for a in self.doc.audio.iter_mut().filter(|a| a.path == path && a.len <= 0.0) {
                        a.len = (dur - a.offset).max(0.0);
                        grow = grow.max(a.end());
                    }
                    if grow > self.doc.timeline.duration {
                        self.doc.timeline.duration = grow.ceil();
                    }
                }
                Err(e) => {
                    self.audio_cache.remove(&path);
                    if self.doc.audio.iter().any(|a| a.path == path) {
                        self.status = format!("소리 읽기 실패: {e}");
                    }
                    // keep video files without sound from retrying
                    self.audio_cache.insert(path, None);
                }
            }
        }
        // decode what the document needs (clips, videos with sound)
        let mut need: Vec<String> = self.doc.audio.iter().map(|a| a.path.clone()).collect();
        for l in &self.doc.layers {
            if let LayerKind::Image(im) = &l.kind {
                if let Some(v) = im.video.as_ref().filter(|v| v.info.audio) {
                    need.push(v.path.clone());
                }
            }
        }
        for p in need {
            self.ensure_audio(&p);
        }
    }

    /// Everything that sounds on the timeline right now.
    fn timeline_voices(&self) -> Vec<crate::audio::Voice> {
        let mut out = Vec::new();
        for a in self.doc.audio.iter().filter(|a| !a.mute && a.len > 0.0) {
            if let Some(Some(d)) = self.audio_cache.get(&a.path) {
                let env = crate::anim::vol_envelope(&a.anim, a.start, a.end()).map(Arc::new);
                out.push(crate::audio::Voice { data: d.clone(), t0: a.start, src0: a.offset, speed: 1.0, span: (a.start, a.end()), gain: a.gain, env });
            }
        }
        for l in &self.doc.layers {
            let LayerKind::Image(im) = &l.kind else { continue };
            let Some(v) = im.video.as_ref().filter(|v| v.info.audio && !v.mute) else { continue };
            let Some(Some(d)) = self.audio_cache.get(&v.path) else { continue };
            let (t0, src0) = l.anim.as_ref().map_or((0.0, 0.0), |a| (a.start(), a.video_start));
            let span = l.anim.as_ref().and_then(|a| a.span).map_or((0.0, f64::MAX), |s| (s[0], s[1]));
            if l.visible {
                let end = span.1.min(self.doc.timeline.duration.max(t0) + 1.0);
                let env = l.anim.as_ref().and_then(|a| crate::anim::vol_envelope(a, span.0.max(0.0), end)).map(Arc::new);
                out.push(crate::audio::Voice { data: d.clone(), t0, src0, speed: v.speed, span, gain: v.volume, env });
            }
        }
        out
    }


    // ------------------------------------------------------------ video export

    fn open_video_export(&mut self) {
        self.finish_text_edit();
        if self.vexport.is_some() {
            return;
        }
        let tl = &self.doc.timeline;
        let mut d = self.vexport_last.clone().unwrap_or(VideoExportDlg {
            region: ExportRegion::Content,
            padding: 20.0,
            by_width: true,
            n: 1920,
            fps: tl.fps,
            all: true,
            t0: 0.0,
            t1: tl.duration,
            codec: crate::vexport::Codec::H264,
            quality: 20,
            use_mbps: false,
            mbps: 12.0,
            gpu: false,
            bg: ExportBg::White,
            bg_color: Color32::BLACK,
            audio: true,
            ss: 1,
        });
        // a camera if there is one, else the export frame
        let region_ok = match d.region {
            ExportRegion::Camera(id) => self.doc.cameras.iter().any(|c| c.id == id),
            ExportRegion::Frame => self.doc.export_frame.is_some(),
            _ => true,
        };
        if !region_ok || self.vexport_last.is_none() {
            d.region = match (self.doc.cameras.first(), self.doc.export_frame) {
                (Some(c), _) => ExportRegion::Camera(c.id),
                (None, Some(_)) => ExportRegion::Frame,
                _ => ExportRegion::Content,
            };
        }
        if self.vexport_last.is_none() {
            d.fps = tl.fps;
            d.t1 = tl.duration;
        }
        self.vexport = Some(d);
    }

    /// The region a video export films, and its rect now (for the aspect ratio).
    fn video_region(&self, d: &VideoExportDlg) -> Option<(crate::vexport::Region, Rect)> {
        use crate::vexport::Region;
        match d.region {
            ExportRegion::Camera(id) => self.doc.cameras.iter().find(|c| c.id == id).map(|c| (Region::Camera(id), c.rect)),
            ExportRegion::Frame => self.doc.export_frame.map(|r| (Region::Fixed(r), r)),
            ExportRegion::Content => self.doc.content_bounds().map(|b| {
                let r = b.expand(d.padding);
                (Region::Fixed(r), r)
            }),
            ExportRegion::View => {
                let xf = self.xf();
                let r = Rect::from_min_max(xf.s2w(self.canvas_rect.min), xf.s2w(self.canvas_rect.max));
                Some((Region::Fixed(r), r))
            }
        }
    }

    fn video_export_ui(&mut self, ctx: &egui::Context) {
        // a running export: progress and cancel
        if let Some(p) = self.vjob.clone() {
            if let Some(res) = p.result.lock().ok().and_then(|mut g| g.take()) {
                self.status = match res {
                    Ok(m) => m,
                    Err(e) => format!("영상 내보내기: {e}"),
                };
                self.vjob = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        let Some(mut d) = self.vexport.take() else { return };
        use crate::vexport::Codec;
        let mut open = true;
        let mut go = false;
        let running = self.vjob.clone();
        let region = self.video_region(&d);
        let voices = self.timeline_voices().len();
        egui::Window::new("영상 내보내기")
            .title_bar(false)
            .frame(egui::Frame::NONE)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::same(SPACE_LG as i8), theme::RADIUS_CARD, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("영상 내보내기").font(FontId::new(15.0, theme::ui_bold())).color(INK()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("닫기").clicked() {
                                open = false;
                            }
                        });
                    });
                    if let Some(p) = &running {
                        let done = p.done.load(std::sync::atomic::Ordering::Relaxed);
                        let total = p.total.load(std::sync::atomic::Ordering::Relaxed).max(1);
                        ui.add_space(SPACE_SM);
                        ui.add(egui::ProgressBar::new(done as f32 / total as f32).text(format!("{done} / {total} 프레임")));
                        if ui.button("취소").clicked() {
                            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        return;
                    }
                    ui.add_space(SPACE_SM);
                    ui.label("영역");
                    ui.horizontal_wrapped(|ui| {
                        for c in &self.doc.cameras {
                            ui.radio_value(&mut d.region, ExportRegion::Camera(c.id), egui::RichText::new(&c.name).color(CAM_COLOR));
                        }
                        ui.add_enabled_ui(self.doc.export_frame.is_some(), |ui| {
                            ui.radio_value(&mut d.region, ExportRegion::Frame, "내보낼 영역");
                        });
                        ui.radio_value(&mut d.region, ExportRegion::Content, "내용 전체");
                        ui.radio_value(&mut d.region, ExportRegion::View, "지금 화면");
                    });
                    if d.region == ExportRegion::Content {
                        ui.add(crate::dial::Dial::new(&mut d.padding, 0.0..=400.0).text("여백").suffix(" px").default(20.0));
                    }
                    ui.add_space(SPACE_SM);
                    ui.horizontal(|ui| {
                        ui.label("크기");
                        ui.selectable_value(&mut d.by_width, true, "가로 기준");
                        ui.selectable_value(&mut d.by_width, false, "세로 기준");
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        let presets: &[(u32, &str)] = if d.by_width {
                            &[(3840, "3840"), (2560, "2560"), (1920, "1920"), (1280, "1280"), (1080, "1080"), (854, "854")]
                        } else {
                            &[(2160, "2160"), (1440, "1440"), (1080, "1080"), (720, "720"), (480, "480")]
                        };
                        for (v, l) in presets {
                            if ui.selectable_label(d.n == *v, *l).clicked() {
                                d.n = *v;
                            }
                        }
                        ui.add(crate::calc::drag(&mut d.n).range(16..=8192).suffix(" px"));
                    });
                    let (w, h) = match &region {
                        Some((_, r)) => crate::vexport::size_for(r.width() / r.height().max(1.0), d.by_width, d.n, d.codec.even()),
                        None => (0, 0),
                    };
                    ui.label(egui::RichText::new(if w > 0 { format!("→ {w} × {h} px (영역 비율)") } else { "영역 없음".into() }).color(INK_2()));
                    ui.horizontal(|ui| {
                        ui.label("fps");
                        ui.add(crate::calc::drag(&mut d.fps).range(1.0..=240.0).speed(0.1).max_decimals(3));
                        ui.checkbox(&mut d.all, "타임라인 전체");
                    });
                    if !d.all {
                        ui.horizontal(|ui| {
                            ui.label("구간");
                            ui.add(crate::calc::drag(&mut d.t0).range(0.0..=36000.0).speed(0.05).suffix(" s").max_decimals(2));
                            ui.label("~");
                            ui.add(crate::calc::drag(&mut d.t1).range(0.0..=36000.0).speed(0.05).suffix(" s").max_decimals(2));
                        });
                    }
                    ui.add_space(SPACE_SM);
                    ui.horizontal(|ui| {
                        ui.label("형식");
                        egui::ComboBox::from_id_salt("vcodec").truncate()
                            .selected_text(d.codec.label())
                            .width(200.0)
                            .popup_style(theme::glass_popup_style(false))
                            .show_ui(ui, |ui| {
                                theme::glass_popup(ui, |ui| {
                                    for c in Codec::ALL {
                                        ui.selectable_value(&mut d.codec, c, c.label());
                                    }
                                })
                            });
                    });
                    if d.codec.has_quality() {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut d.use_mbps, false, "품질");
                            ui.selectable_value(&mut d.use_mbps, true, "비트레이트");
                            if d.use_mbps {
                                ui.add(crate::calc::drag(&mut d.mbps).range(0.2..=400.0).speed(0.1).suffix(" Mbps").max_decimals(1));
                            } else {
                                ui.add(egui::Slider::new(&mut d.quality, 0..=51).text("낮을수록 고화질"));
                            }
                        });
                    }
                    if d.codec.has_gpu() {
                        ui.checkbox(&mut d.gpu, "GPU 인코딩 (NVIDIA NVENC)").on_hover_text("NVIDIA 그래픽카드 필요");
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label("슈퍼샘플링").on_hover_text("크게 그린 뒤 줄여서 저장 · 경계와 두께 줄무늬가 부드러워짐");
                        for k in 1..=4u8 {
                            if ui.selectable_label(d.ss == k, format!("{k}×")).clicked() {
                                d.ss = k;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("배경");
                        if d.codec.alpha() {
                            ui.radio_value(&mut d.bg, ExportBg::Transparent, "투명");
                        } else if d.bg == ExportBg::Transparent {
                            d.bg = ExportBg::White;
                        }
                        ui.radio_value(&mut d.bg, ExportBg::White, "흰색");
                        ui.radio_value(&mut d.bg, ExportBg::Custom, "색");
                        if d.bg == ExportBg::Custom {
                            let mut c = d.bg_color;
                            if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
                                d.bg_color = c;
                            }
                        }
                    });
                    if d.codec.audio() {
                        ui.add_enabled(voices > 0, egui::Checkbox::new(&mut d.audio, format!("소리 포함 ({voices}개)")));
                    }
                    ui.add_space(SPACE_SM);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(w > 0, egui::Button::new(egui::RichText::new("내보내기…").strong())).clicked() {
                            go = true;
                        }
                        ui.small(format!("{:.2} s · {} 프레임", (if d.all { self.doc.timeline.duration } else { d.t1 - d.t0 }).max(0.0), ((if d.all { self.doc.timeline.duration } else { d.t1 - d.t0 }).max(0.0) * d.fps).ceil()));
                    });
                });
            });
        if go {
            self.start_video_export(&d);
        }
        if open {
            self.vexport = Some(d);
        } else {
            self.vexport_last = Some(d);
        }
    }

    fn start_video_export(&mut self, d: &VideoExportDlg) {
        let Some((region, rect)) = self.video_region(d) else { return };
        let (w, h) = crate::vexport::size_for(rect.width() / rect.height().max(1.0), d.by_width, d.n, d.codec.even());
        let (t0, t1) = if d.all { (0.0, self.doc.timeline.duration) } else { (d.t0.min(d.t1), d.t0.max(d.t1)) };
        if t1 - t0 <= 0.0 {
            self.status = "빈 구간".into();
            return;
        }
        let base = self.path.as_ref().and_then(|p| p.file_stem()).and_then(|s| s.to_str()).unwrap_or("video").to_string();
        let Some(mut out) = rfd::FileDialog::new()
            .add_filter(d.codec.label(), &[d.codec.ext()])
            .set_file_name(format!("{base}.{}", d.codec.ext()))
            .save_file()
        else {
            return;
        };
        if out.extension().is_none() {
            out.set_extension(d.codec.ext());
        }
        let bg = match d.bg {
            ExportBg::Transparent => None,
            ExportBg::White => Some(Color32::WHITE),
            ExportBg::Custom => Some(d.bg_color),
        };
        let audio = if d.audio && d.codec.audio() { self.timeline_voices() } else { Vec::new() };
        let job = crate::vexport::Job {
            doc: self.doc.clone(),
            region,
            size: (w, h),
            fps: d.fps,
            t0,
            t1,
            codec: d.codec,
            quality: d.quality,
            mbps: d.use_mbps.then_some(d.mbps),
            gpu: d.gpu && d.codec.has_gpu(),
            bg,
            audio,
            ss: d.ss,
            out,
        };
        self.vjob = Some(crate::vexport::start(job));
        self.status = "영상 내보내는 중…".into();
    }


    fn import_dialog(&mut self) {
        let all: Vec<&str> = crate::io::IMAGE_EXTS.iter().chain(crate::video::VIDEO_EXTS).chain(crate::audio::AUDIO_EXTS).copied().collect();
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("사진·영상", &all)
            .add_filter("사진", crate::io::IMAGE_EXTS)
            .add_filter("영상", crate::video::VIDEO_EXTS)
            .add_filter("소리", crate::audio::AUDIO_EXTS)
            .pick_files()
        {
            let at = self.view_center_world();
            self.import_paths(paths, at);
        }
    }

    fn paste(&mut self) {
        let at = self.view_center_world();
        let Ok(mut cb) = arboard::Clipboard::new() else {
            self.status = "클립보드를 열 수 없음".into();
            return;
        };
        if let Ok(img) = cb.get_image() {
            if let Some(rgba) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) {
                let src = crate::io::image_from_rgba(rgba);
                self.add_image(src, "붙여넣은 사진".into(), at);
                self.status = "클립보드 사진 붙여넣음".into();
                return;
            }
        }
        if let Ok(files) = cb.get().file_list() {
            if !files.is_empty() {
                self.import_paths(files, at);
                return;
            }
        }
        if let Ok(text) = cb.get_text() {
            let text = text.replace("\r\n", "\n");
            if !text.trim().is_empty() {
                let t = self.new_text_layer(at, text);
                self.sel = Some(t);
                self.mark();
                self.status = "글자 붙여넣음".into();
            }
        }
    }

    fn new_text_layer(&mut self, at: Pos2, text: String) -> LayerId {
        let t = TextLayer {
            pos: at,
            text,
            size: self.s.text_size,
            color: self.s.color,
            family: self.s.text_family.clone(),
            style: self.s.text_style.clone(),
            bg: self.s.text_bg.then_some(self.s.text_bg_color),
            wrap: 0.0,
            rotation: 0.0,
            blend: Blend::Normal,
            align: self.s.text_align,
        };
        self.doc.insert("글".into(), LayerKind::Text(t), self.sel)
    }

    // ------------------------------------------------------------ file

    fn request(&mut self, p: Pending) {
        if self.modified {
            self.pending = Some(p);
        } else {
            self.run_pending(p);
        }
    }

    fn run_pending(&mut self, p: Pending) {
        match p {
            Pending::New => {
                self.reset_doc(Document::default(), None);
                self.status = "새 작업".into();
            }
            Pending::Open(path) => {
                let path = path.or_else(|| rfd::FileDialog::new().add_filter("Drifting VOID", &["void"]).pick_file());
                if let Some(p) = path {
                    self.open_path(p);
                }
            }
            Pending::Quit => {
                self.save_prefs();
                self.modified = false;
                self.pending = None;
                std::process::exit(0);
            }
        }
    }

    fn open_path(&mut self, p: PathBuf) {
        match crate::io::load_project(&p) {
            Ok(d) => {
                self.reset_doc(d, Some(p.clone()));
                self.status = format!("열림: {}", p.file_name().and_then(|n| n.to_str()).unwrap_or(""));
            }
            Err(e) => self.status = format!("열기 실패: {e}"),
        }
    }

    fn save(&mut self, as_new: bool) -> bool {
        self.finish_text_edit();
        let path = match (&self.path, as_new) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new().add_filter("Drifting VOID", &["void"]).set_file_name("collage.void").save_file(),
        };
        let Some(mut p) = path else { return false };
        if p.extension().is_none() {
            p.set_extension("void");
        }
        match crate::io::save_project(&self.doc, &p) {
            Ok(()) => {
                self.status = format!("저장됨: {}", p.file_name().and_then(|n| n.to_str()).unwrap_or(""));
                self.path = Some(p);
                self.modified = false;
                true
            }
            Err(e) => {
                self.status = format!("저장 실패: {e}");
                false
            }
        }
    }

    // ------------------------------------------------------------ layer ops

    fn delete_selected(&mut self) {
        let ids = self.selected_ids();
        let sub = std::mem::take(&mut self.sub);
        for (id, mut idx) in sub.strokes {
            if let Some(LayerKind::Vector(v)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                idx.sort_unstable();
                for i in idx.into_iter().rev() {
                    if i < v.strokes.len() {
                        v.strokes.remove(i);
                    }
                }
                self.changed = true;
                self.modified = true;
            }
        }
        for (id, polys) in sub.pixels {
            if let Some(LayerKind::Raster(r)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                *r = r.lift(&polys).1;
                self.changed = true;
                self.modified = true;
            }
        }
        if ids.is_empty() {
            return;
        }
        self.doc.layers.retain(|l| !ids.contains(&l.id) && !l.mask_of.is_some_and(|o| ids.contains(&o)));
        if self.editing.is_some_and(|e| ids.contains(&e)) {
            self.editing = None;
        }
        self.select_only(None);
        self.mark();
    }

    fn duplicate_selected(&mut self) {
        let ids = self.selected_ids();
        let mut new_ids = Vec::new();
        for id in ids {
            let Some(idx) = self.doc.index_of(id) else { continue };
            let mut copy = self.doc.layers[idx].clone();
            copy.id = self.doc.new_id();
            copy.translate(vec2(30.0, 30.0) / self.zoom.max(0.1));
            new_ids.push(copy.id);
            self.doc.layers.insert(idx + 1, copy);
        }
        if let Some((first, rest)) = new_ids.split_first() {
            self.sel = Some(*first);
            self.extra_sel = rest.to_vec();
            self.mark();
        }
    }

    fn move_selected(&mut self, delta: i32) {
        let Some(id) = self.sel else { return };
        let Some(i) = self.doc.index_of(id) else { return };
        let j = i as i32 + delta;
        if j >= 0 && (j as usize) < self.doc.layers.len() {
            self.doc.layers.swap(i, j as usize);
            self.mark();
        }
    }

    /// Puts a layer into a space on a camera-facing plane, so it looks exactly as
    /// before; the layer moves next to the space's other members in the stack.
    fn put_in_space(&mut self, id: LayerId, sid: u64) {
        let Some(sp) = self.doc.spaces.iter().find(|s| s.id == sid) else { return };
        let name = sp.name.clone();
        let cam = match crate::persp::solve(sp) {
            Ok(c) => c,
            Err(e) => {
                self.status = format!("{name}: {e}");
                return;
            }
        };
        let Some(i) = self.doc.index_of(id) else { return };
        let Some(b) = self.doc.layers[i].bounds() else { return };
        self.doc.layers[i].place = Some(crate::persp::Placement::facing(sid, &cam, b.center()));
        if let Some(top) = self.doc.layers.iter().rposition(|l| l.id != id && l.place.as_ref().is_some_and(|p| p.space == sid)) {
            let l = self.doc.layers.remove(i);
            let to = if top > i { top } else { top + 1 };
            self.doc.layers.insert(to.min(self.doc.layers.len()), l);
        }
        self.status = format!("{name}에 넣음");
        self.mark();
    }

    /// Stands a layer upright on the floor where canvas point `w` shows, facing the
    /// camera, its bottom middle on the floor (AR-style placing).
    fn stand_on_floor(&mut self, id: LayerId, sid: u64, w: Pos2) {
        let Some(sp) = self.doc.spaces.iter().find(|s| s.id == sid) else { return };
        let Ok(cam) = crate::persp::solve(sp) else {
            self.status = "공간 카메라 계산 불가".into();
            return;
        };
        let Some(at) = cam.hit_floor(w) else {
            self.status = "바닥 아님 (지평선 위)".into();
            return;
        };
        let Some(i) = self.doc.index_of(id) else { return };
        let Some(b) = self.doc.layers[i].bounds() else { return };
        let mut pl = crate::persp::Placement::standing(sid, &cam, at, b.center_bottom());
        if let Some(old) = &self.doc.layers[i].place {
            pl.thick = old.thick;
        }
        self.doc.layers[i].place = Some(pl);
        if let Some(top) = self.doc.layers.iter().rposition(|l| l.id != id && l.place.as_ref().is_some_and(|p| p.space == sid)) {
            let l = self.doc.layers.remove(i);
            let to = if top > i { top } else { top + 1 };
            self.doc.layers.insert(to.min(self.doc.layers.len()), l);
        }
        self.status = "바닥에 세움".into();
        self.mark();
    }

    /// Puts a layer on a wall: its plane lies on the wall where its centre shows, and it
    /// moves along the wall from then on.
    fn put_in_wall(&mut self, id: LayerId, sid: u64, wid: u64) {
        let Some(sp) = self.doc.spaces.iter().find(|s| s.id == sid) else { return };
        let Ok(cam) = crate::persp::solve(sp) else {
            self.status = "공간 카메라 계산 불가".into();
            return;
        };
        let Some(wall) = sp.walls.iter().find(|w| w.id == wid).cloned() else { return };
        let Some(i) = self.doc.index_of(id) else { return };
        let Some(b) = self.doc.layers[i].bounds() else { return };
        let Some(mut pl) = crate::persp::Placement::on_wall(sid, &cam, &wall, b.center()) else {
            self.status = "벽에 닿지 않음 (카메라 뒤쪽)".into();
            return;
        };
        if let Some(old) = &self.doc.layers[i].place {
            pl.thick = old.thick;
        }
        self.doc.layers[i].place = Some(pl);
        let same = |l: &Layer| l.id != id && l.place.as_ref().is_some_and(|p| p.wall == Some(wid));
        let near = self.doc.layers.iter().rposition(same).or_else(|| self.doc.layers.iter().rposition(|l| l.id != id && l.place.as_ref().is_some_and(|p| p.space == sid)));
        if let Some(top) = near {
            let l = self.doc.layers.remove(i);
            let to = if top > i { top } else { top + 1 };
            self.doc.layers.insert(to.min(self.doc.layers.len()), l);
        }
        self.status = format!("{}에 붙임", wall.name);
        self.mark();
    }

    fn take_out_of_space(&mut self, id: LayerId) {
        if let Some(l) = self.doc.get_mut(id) {
            if l.place.take().is_some() {
                self.status = "공간에서 뺌 (원래 2D 모습으로)".into();
                self.mark();
            }
        }
    }

    fn new_paint_layer(&mut self, vector: bool) -> LayerId {
        let (name, kind) = if vector {
            let n = self.doc.count_kind(|k| matches!(k, LayerKind::Vector(_))) + 1;
            (format!("펜 {n}"), LayerKind::Vector(VectorLayer::default()))
        } else {
            let n = self.doc.count_kind(|k| matches!(k, LayerKind::Raster(_))) + 1;
            (format!("브러시 {n}"), LayerKind::Raster(RasterLayer::default()))
        };
        let id = self.doc.insert(name, kind, None);
        self.sel = Some(id);
        self.mark();
        id
    }

    /// Selected layer if it is a usable paint layer of the right kind, otherwise
    /// the topmost existing one, otherwise a new one.
    fn paint_target(&mut self, vector: bool) -> LayerId {
        let usable = |l: &Layer| {
            l.visible
                && !l.locked
                && l.mask_of.is_none()
                && l.place.is_none()
                && match l.kind {
                    LayerKind::Vector(_) => vector,
                    LayerKind::Raster(_) => !vector,
                    _ => false,
                }
        };
        if let Some(l) = self.sel_layer() {
            if usable(l) || (l.mask_of.is_some() && !vector && !l.locked) {
                return l.id;
            }
        }
        if let Some(id) = self.doc.layers.iter().rev().find(|l| usable(l)).map(|l| l.id) {
            self.sel = Some(id);
            return id;
        }
        self.new_paint_layer(vector)
    }

    fn hit_layer(&self, w0: Pos2, tol0: f32) -> Option<LayerId> {
        let cams = crate::persp::cameras(&self.doc.spaces);
        for l in self.doc.draw_order(&cams).into_iter().rev() {
            if !l.visible || l.locked || l.mask_of.is_some() {
                continue;
            }
            // placed layers: test in their 2D content coordinates
            if let Some(pl) = &l.place {
                if self.doc.spaces.iter().find(|s| s.id == pl.space).is_some_and(|s| !s.clip().contains(w0)) {
                    continue;
                }
            }
            let (w, tol) = match l.place.as_ref().and_then(|pl| cams.get(&pl.space).map(|c| pl.homography(c))) {
                Some(h) => {
                    let Some(q) = crate::persp::invert(&h).and_then(|hi| crate::persp::apply_h(&hi, w0)) else { continue };
                    let m = l.bounds().map_or(1.0, |b| crate::export::magnification(&h, b));
                    (q, tol0 / m.max(1e-3))
                }
                None => (w0, tol0),
            };
            let hit = match &l.kind {
                LayerKind::Image(im) => im.contains(w),
                LayerKind::Text(t) => {
                    let r = crate::text::layout(t).rect(t.pos);
                    r.expand(tol).contains(crate::doc::rot_about(w, r.center(), -t.rotation))
                }
                LayerKind::Shape(s) => shape_hit(s, w, tol),
                LayerKind::Vector(v) => v.strokes.iter().any(|s| s.hit(w - v.offset, tol)),
                LayerKind::Raster(r) => r.alpha_at_world(w) > 8,
            };
            if hit {
                return Some(l.id);
            }
        }
        None
    }

    fn save_prefs(&mut self) {
        let out = PrefsOut { settings: &self.s, recent: &self.recent, export: self.last_export.as_ref() };
        let Ok(text) = serde_json::to_string_pretty(&out) else { return };
        if text != self.prefs_saved {
            if let Some(p) = prefs_path() {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if std::fs::write(p, &text).is_ok() {
                    self.prefs_saved = text;
                }
            }
        }
    }

    /// Align / distribute buttons for a multi-selection.
    fn align_bar(&mut self, ui: &mut egui::Ui) {
        let mut act = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            for (i, (label, tip)) in [
                ("왼쪽", "왼쪽 맞춤"),
                ("가로 중앙", "가로 가운데 맞춤"),
                ("오른쪽", "오른쪽 맞춤"),
                ("위", "위쪽 맞춤"),
                ("세로 중앙", "세로 가운데 맞춤"),
                ("아래", "아래쪽 맞춤"),
                ("가로 간격", "가로 간격 같게 (양 끝 고정)"),
                ("세로 간격", "세로 간격 같게 (양 끝 고정)"),
            ]
            .into_iter()
            .enumerate()
            {
                if ui.add(egui::Button::new(label).small()).on_hover_text(tip).clicked() {
                    act = Some(i);
                }
            }
        });
        ui.add_space(4.0);
        if let Some(a) = act {
            self.align(a);
        }
    }

    /// 0..5: align left / h-centre / right / top / v-centre / bottom,
    /// 6: distribute horizontally, 7: vertically (by centres, ends fixed).
    fn align(&mut self, mode: usize) {
        let parts = self.sel_parts();
        let bounds = |(l, part): &(Layer, Part)| -> Option<Rect> {
            match part {
                Part::Whole => l.bounds(),
                Part::Strokes(idx) => match &l.kind {
                    LayerKind::Vector(v) => idx.iter().filter_map(|&i| v.strokes.get(i)).map(|s| s.bounds.translate(v.offset)).reduce(|a, b| a.union(b)),
                    _ => None,
                },
                Part::Pixels(ps) => ps.iter().map(|p| Rect::from_points(p)).reduce(|a, b| a.union(b)),
            }
        };
        let items: Vec<(usize, Rect)> = parts.iter().enumerate().filter_map(|(i, p)| bounds(p).map(|b| (i, b))).collect();
        if items.len() < 2 {
            return;
        }
        let all = items.iter().map(|x| x.1).reduce(|a, b| a.union(b)).unwrap();
        let mut moves: Vec<(usize, Vec2)> = Vec::new();
        match mode {
            0..=5 => {
                for &(i, b) in &items {
                    let d = match mode {
                        0 => vec2(all.min.x - b.min.x, 0.0),
                        1 => vec2(all.center().x - b.center().x, 0.0),
                        2 => vec2(all.max.x - b.max.x, 0.0),
                        3 => vec2(0.0, all.min.y - b.min.y),
                        4 => vec2(0.0, all.center().y - b.center().y),
                        _ => vec2(0.0, all.max.y - b.max.y),
                    };
                    moves.push((i, d));
                }
            }
            _ => {
                let horiz = mode == 6;
                let mut sorted = items.clone();
                sorted.sort_by(|a, b| {
                    let (ka, kb) = if horiz { (a.1.center().x, b.1.center().x) } else { (a.1.center().y, b.1.center().y) };
                    ka.partial_cmp(&kb).unwrap()
                });
                let (first, last) = (sorted[0].1.center(), sorted[sorted.len() - 1].1.center());
                let n = (sorted.len() - 1) as f32;
                for (k, &(i, b)) in sorted.iter().enumerate() {
                    let t = k as f32 / n;
                    let target = first + (last - first) * t;
                    let d = if horiz { vec2(target.x - b.center().x, 0.0) } else { vec2(0.0, target.y - b.center().y) };
                    moves.push((i, d));
                }
            }
        }
        for (i, d) in moves {
            let (o, part) = &parts[i];
            // whole layers of one id may appear once; strokes/pixels each once
            let cur = self.doc.get(o.id).cloned().unwrap_or_else(|| o.clone());
            let l = apply_part(&cur, part, Pos2::ZERO, 1.0, 0.0, d);
            if let Some(ix) = self.doc.index_of(l.id) {
                self.doc.layers[ix] = l;
            }
            if let Part::Pixels(_) = part {
                if let Some(e) = self.sub.pixels.iter_mut().find(|x| x.0 == o.id) {
                    for p in e.1.iter_mut() {
                        *p = map_poly(p, Pos2::ZERO, 1.0, 0.0, d);
                    }
                }
            }
        }
        self.mark();
    }

    fn drop_stale_ime(&self, ctx: &egui::Context) {
        if self.now < self.ime_guard_until {
            ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Ime(_))));
        }
    }

    fn finish_text_edit(&mut self) {
        if let Some(id) = self.editing.take() {
            // The Korean IME commits the syllable it was still composing *after* focus
            // moves; egui already put that syllable into this box as preedit, so drop
            // IME events briefly instead of letting them land in the next text box.
            self.ime_guard_until = self.now + 0.25;
            let empty = matches!(self.doc.get(id).map(|l| &l.kind), Some(LayerKind::Text(t)) if t.text.trim().is_empty());
            if empty {
                if let Some(i) = self.doc.index_of(id) {
                    self.doc.layers.remove(i);
                    if self.sel == Some(id) {
                        self.sel = None;
                    }
                    self.mark();
                }
            }
        }
    }

    fn set_tool(&mut self, t: Tool) {
        if self.tool != t {
            self.finish_text_edit();
            self.tool = t;
        }
    }

    fn remember_color(&mut self, c: Color32) {
        self.recent.retain(|x| *x != c);
        self.recent.insert(0, c);
        self.recent.truncate(8);
    }

    // ------------------------------------------------------------ handles

    /// A lone text box or shape that gets the group's scale/rotate handles
    /// (true: text, corners too; false: shape, rotate only — its ends resize it).
    fn single_handles(&self) -> Option<bool> {
        if !self.extra_sel.is_empty() || !self.sub.is_empty() || self.editing.is_some() {
            return None;
        }
        let l = self.sel_layer().filter(|l| !l.locked && l.place.is_none() && l.mask_of.is_none())?;
        match &l.kind {
            LayerKind::Text(_) => Some(true),
            LayerKind::Shape(_) => Some(false),
            _ => None,
        }
    }

    /// Handles of the selection in screen space: (id, position).
    /// Union of the selected layers' bounds (several layers, or one text/shape).
    fn group_box(&self) -> Option<Rect> {
        if self.extra_sel.is_empty() && self.sub.is_empty() {
            return self.single_handles().and_then(|_| self.sel_layer()?.bounds());
        }
        let whole = self.selected_ids().iter().filter_map(|id| self.doc.get(*id)).filter(|l| !l.locked).filter_map(|l| l.bounds()).reduce(|a, b| a.union(b));
        match (whole, self.sub_bounds()) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (a, b) => a.or(b),
        }
    }

    /// Group handles in screen space: 30..=33 corners, 34 rotate.
    fn group_handles(&self) -> Vec<(u8, Pos2)> {
        let Some(b) = self.group_box().filter(|_| self.tool == Tool::Select) else { return Vec::new() };
        let xf = self.xf();
        let r = xf.rect_w2s(b);
        let rot = (34, r.center_top() - vec2(0.0, 28.0));
        match (self.single_handles(), self.sel_layer().map(|l| &l.kind)) {
            (Some(false), _) => vec![rot],
            // a lone text box: on its own (turned) outline
            (Some(true), Some(LayerKind::Text(t))) => {
                let tr = crate::text::layout(t).rect(t.pos);
                let c = tr.center();
                let q = [tr.left_top(), tr.right_top(), tr.right_bottom(), tr.left_bottom()].map(|p| xf.w2s(crate::doc::rot_about(p, c, t.rotation)));
                let up = vec2(t.rotation.sin(), -t.rotation.cos());
                let top = q[0] + (q[1] - q[0]) * 0.5;
                vec![(30, q[0]), (31, q[1]), (32, q[2]), (33, q[3]), (34, top + up * 28.0)]
            }
            _ => vec![(30, r.left_top()), (31, r.right_top()), (32, r.right_bottom()), (33, r.left_bottom()), rot],
        }
    }

    fn handles(&self) -> Vec<(u8, Pos2)> {
        let xf = self.xf();
        let mut out = Vec::new();
        let Some(l) = self.sel_layer() else { return out };
        if l.place.is_some() {
            return out;
        }
        if l.locked || !self.extra_sel.is_empty() {
            return out;
        }
        match &l.kind {
            LayerKind::Image(im) if self.tool == Tool::Crop => {
                for (i, p) in rect_handles(im.crop).iter().enumerate() {
                    out.push((i as u8, xf.w2s(im.src_to_world(*p))));
                }
            }
            LayerKind::Image(im) if self.tool == Tool::Select => {
                let c = im.corners();
                for (i, p) in c.iter().enumerate() {
                    out.push((i as u8, xf.w2s(*p)));
                }
                out.push((10, self.rot_handle(im)));
            }
            LayerKind::Shape(s) if self.tool == Tool::Select => {
                out.push((0, xf.w2s(s.a)));
                out.push((1, xf.w2s(s.b)));
            }
            LayerKind::Text(t) if self.tool == Tool::Select => {
                let r = crate::text::layout(t).rect(t.pos);
                out.push((20, xf.w2s(crate::doc::rot_about(r.right_center(), r.center(), t.rotation))));
            }
            _ => {}
        }
        if self.tool == Tool::Frame {
            out.clear();
        }
        out
    }

    fn rot_handle(&self, im: &ImageLayer) -> Pos2 {
        let xf = self.xf();
        let (_, v) = im.axes();
        let top = im.center - v * im.half().y;
        xf.w2s(top) - v * 28.0
    }

    // ------------------------------------------------------------ pointer

    fn cancel_drag(&mut self) {
        if let Some(t) = self.restore_tool.take() {
            self.tool = t;
        }
        match self.drag.take() {
            Some(Drag::Raster { id, stroke: Some(st), .. }) => {
                if let Some(LayerKind::Raster(r)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    st.cancel(r);
                }
            }
            Some(Drag::Move { id, orig, .. }) => {
                if let Some(i) = self.doc.index_of(id) {
                    self.doc.layers[i] = orig;
                }
            }
            _ => {}
        }
    }

    fn pointer_down(&mut self, sp: Pos2, pressure: f32, mods: Modifiers, space: bool) {
        let xf = self.xf();
        let w = xf.s2w(sp);
        if let Some(mid) = self.wand.filter(|_| !space) {
            if self.doc.get(mid).is_some_and(|l| l.mask_of.is_some()) {
                if self.wand_mode == 2 {
                    self.scissor_click(mid, w, !mods.alt, mods.shift);
                } else if self.wand_mode == 1 {
                    // GrabCut: drag a box (a temporary outline, run on release)
                    let s = ShapeLayer { kind: ShapeKind::Rect, a: w, b: w, color: ACCENT, width: 1.5 / self.zoom, fill: None, rotation: 0.0 };
                    let id = self.doc.insert("영역".into(), LayerKind::Shape(s), None);
                    self.drag = Some(Drag::NewShape { id, start: w });
                    self.mask_rect = Some((mid, !mods.alt));
                    self.mask_cut = true;
                } else {
                    self.wand_fill(mid, w, !mods.alt);
                }
                return;
            }
            self.wand = None;
        }
        let tol = 6.0 / self.zoom;
        self.last_screen = sp;
        if self.editing.is_some() && self.tool != Tool::Text {
            self.finish_text_edit();
        }
        if space || self.tool == Tool::Hand {
            self.drag = Some(Drag::Pan);
            return;
        }
        if mods.alt {
            self.open_palette(sp);
            return;
        }
        let paint_tool = matches!(self.tool, Tool::Brush | Tool::Pen | Tool::Eraser);
        let shape_tool = matches!(self.tool, Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse);
        if (paint_tool || shape_tool) && mods.ctrl {
            if let Some(v) = self.tool_size() {
                self.drag = Some(Drag::Size { anchor: sp, start: v, x0: sp.x });
            }
            return;
        }
        if self.tool == Tool::Picker {
            if let Some(c) = crate::export::sample(&self.doc, w) {
                self.s.color = c;
                self.remember_color(c);
                self.status = format!("색 선택: #{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
            }
            return;
        }
        // on a mask the pen paints coverage like the brush
        let on_mask = self.sel_layer().is_some_and(|l| l.mask_of.is_some());
        let tool_now = if on_mask && self.tool == Tool::Pen { Tool::Brush } else { self.tool };
        match tool_now {
            Tool::Select => {
                for (h, hp) in self.group_handles() {
                    if hp.distance(sp) <= 9.0 {
                        let box_ = self.group_box().unwrap();
                        let origs = self.sel_parts();
                        self.drag = Some(Drag::Group { rotate: h == 34, center: box_.center(), start: w, box_, origs, s: 1.0, th: 0.0 });
                        return;
                    }
                }
                for (h, hp) in self.handles() {
                    if hp.distance(sp) <= 9.0 {
                        let id = self.sel.unwrap();
                        let l = self.doc.get(id).unwrap();
                        self.drag = Some(match (&l.kind, h) {
                            (LayerKind::Image(im), 10) => Drag::Rotate { id, orig: im.clone() },
                            (LayerKind::Image(im), c) => {
                                let sign = [vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0)][c as usize];
                                Drag::ScaleImage { id, sign, orig: im.clone() }
                            }
                            (LayerKind::Shape(_), c) => Drag::ShapePt { id, which: c },
                            (LayerKind::Text(_), _) => Drag::TextWrap { id },
                            _ => return,
                        });
                        return;
                    }
                }
                if mods.shift {
                    // Shift: drag a selection box (a plain Shift+click toggles one layer);
                    // Shift+Ctrl: only things entirely inside the box
                    self.drag = Some(Drag::Marquee { start: w, cur: w, full: mods.ctrl });
                    return;
                }
                if (!self.extra_sel.is_empty() || !self.sub.is_empty()) && self.hits_selection(w, tol) {
                    let origs = self.sel_parts();
                    self.drag = Some(Drag::MoveMany { start: w, origs, d: Vec2::ZERO });
                    return;
                }
                match self.hit_layer(w, tol) {
                    Some(id) => {
                        self.select_only(Some(id));
                        let orig = self.doc.get(id).unwrap().clone();
                        self.drag = Some(Drag::Move { id, start: w, orig });
                    }
                    None => {
                        self.select_only(None);
                        self.drag = Some(Drag::Pan);
                    }
                }
            }
            Tool::Lasso => {
                // drag inside the current selection moves it; anywhere else draws a new outline
                if !self.sub.is_empty() || self.sel.is_some() {
                    if self.hits_selection(w, tol) {
                        let origs = self.sel_parts();
                        self.drag = Some(Drag::MoveMany { start: w, origs, d: Vec2::ZERO });
                        return;
                    }
                }
                self.drag = Some(Drag::Lasso { pts: vec![w] });
            }
            Tool::Brush | Tool::Eraser => {
                let erase = self.tool == Tool::Eraser;
                let id = if erase {
                    match self.sel_layer() {
                        Some(l) if is_paint_layer(&l.kind) && !l.locked && l.visible => l.id,
                        _ => {
                            self.status = "지울 그림 레이어를 먼저 선택".into();
                            return;
                        }
                    }
                } else {
                    self.paint_target(false)
                };
                let (tl_on, tl_time) = (self.doc.timeline.on, self.doc.timeline.time);
                let layer = self.doc.get_mut(id).unwrap();
                match &mut layer.kind {
                    LayerKind::Raster(r) => {
                        let (size, ps) = if erase {
                            (self.s.eraser_size, self.s.eraser_pressure)
                        } else {
                            (self.s.brush_size, self.s.brush_pressure_size)
                        };
                        let is_mask = layer.mask_of.is_some();
                        let p = BrushParams {
                            color: if is_mask { Color32::WHITE } else { self.s.color },
                            radius: size * 0.5,
                            opacity: if erase { 1.0 } else { self.s.brush_opacity },
                            hardness: if erase { 0.9 } else { self.s.brush_hardness },
                            pressure_size: ps,
                            pressure_opacity: !erase && self.s.brush_pressure_opacity,
                            erase,
                        };
                        // recording: a live stroke when recording, or an instant one on a
                        // layer that already has recorded strokes (so replay stays whole)
                        let record = tl_on && (self.rec_on || !r.recs.is_empty());
                        self.rec_clock = None;
                        if record {
                            if r.rec_base.is_none() {
                                r.rec_base = Some(r.tiles.clone());
                            }
                            let lp = w - r.offset;
                            self.rec_clock = Some((std::time::Instant::now(), tl_time, self.rec_on));
                            self.rec_pts = vec![[lp.x, lp.y, pressure]];
                            self.rec_times = vec![0.0];
                        }
                        let mut st = RasterStroke::new(p);
                        st.add_point(r, w - r.offset, pressure);
                        self.drag = Some(Drag::Raster { id, stroke: Some(st), smooth: (w, pressure) });
                        if !erase {
                            self.remember_color(self.s.color);
                        }
                    }
                    LayerKind::Vector(v) => {
                        let r = radius_for(self.s.eraser_size * 0.5, pressure, self.s.eraser_pressure);
                        let before = v.strokes.len();
                        let off = v.offset;
                        v.strokes.retain(|s| !s.hit(w - off, r));
                        if v.strokes.len() != before {
                            self.changed = true;
                            self.modified = true;
                        }
                        self.drag = Some(Drag::VErase { id, r: self.s.eraser_size * 0.5 });
                    }
                    _ => {}
                }
                self.mark_if_changed_raster();
            }
            Tool::Pen => {
                let id = self.paint_target(true);
                let off = match &self.doc.get(id).unwrap().kind {
                    LayerKind::Vector(v) => v.offset,
                    _ => Vec2::ZERO,
                };
                let base_r = self.s.pen_size * 0.5;
                let l = w - off;
                self.rec_clock = (self.doc.timeline.on && self.rec_on).then(|| (std::time::Instant::now(), self.doc.timeline.time, true));
                self.rec_times = vec![0.0];
                self.drag = Some(Drag::Vector {
                    id,
                    pts: vec![[l.x, l.y, radius_for(base_r, pressure, self.s.pen_pressure)]],
                    smooth: (w, pressure),
                    base_r,
                    pressure: self.s.pen_pressure,
                    color: self.s.color,
                });
                self.remember_color(self.s.color);
            }
            Tool::Text => {
                let hit = self.doc.layers.iter().rev().find(|l| {
                    l.visible && !l.locked && l.place.is_none() && matches!(&l.kind, LayerKind::Text(t) if {
                        let r = crate::text::layout(t).rect(t.pos);
                        r.expand(tol).contains(crate::doc::rot_about(w, r.center(), -t.rotation))
                    })
                });
                if let Some(l) = hit {
                    let id = l.id;
                    if self.editing != Some(id) {
                        self.finish_text_edit();
                        self.editing = Some(id);
                        self.edit_focus = true;
                    }
                    self.sel = Some(id);
                } else {
                    self.finish_text_edit();
                    let at = w - vec2(0.0, self.s.text_size * 0.6);
                    let id = self.new_text_layer(at, String::new());
                    self.sel = Some(id);
                    self.editing = Some(id);
                    self.edit_focus = true;
                    self.mark();
                }
            }
            Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse => {
                if self.tool == Tool::Rect {
                    if let Some(mid) = self.sel_layer().filter(|l| l.mask_of.is_some()).map(|l| l.id) {
                        let clear = mods.alt;
                        self.mask_rect = Some((mid, !clear));
                    }
                }
                let kind = match self.tool {
                    Tool::Line => ShapeKind::Line,
                    Tool::Arrow => ShapeKind::Arrow,
                    Tool::Rect => ShapeKind::Rect,
                    _ => ShapeKind::Ellipse,
                };
                let s = ShapeLayer {
                    kind,
                    a: w,
                    b: w,
                    color: self.s.color,
                    width: self.s.shape_width,
                    fill: (self.s.shape_fill && matches!(kind, ShapeKind::Rect | ShapeKind::Ellipse)).then_some(self.s.fill_color),
                    rotation: 0.0,
                };
                let name = match kind {
                    ShapeKind::Line => "선",
                    ShapeKind::Arrow => "화살표",
                    ShapeKind::Rect => "사각형",
                    ShapeKind::Ellipse => "원",
                };
                let id = self.doc.insert(name.into(), LayerKind::Shape(s), self.sel);
                self.sel = Some(id);
                self.drag = Some(Drag::NewShape { id, start: w });
                self.remember_color(self.s.color);
                self.mark();
            }
            Tool::Crop => {
                for (h, hp) in self.handles() {
                    if hp.distance(sp) <= 9.0 {
                        if let Some(LayerKind::Image(im)) = self.sel_layer().map(|l| &l.kind) {
                            let start_src = im.world_to_src(w);
                            self.drag = Some(Drag::Crop { id: self.sel.unwrap(), handle: h, start_src, orig: im.clone() });
                        }
                        return;
                    }
                }
                if let Some(LayerKind::Image(im)) = self.sel_layer().map(|l| &l.kind) {
                    let full = Rect::from_min_size(Pos2::ZERO, im.src.size());
                    if im.crop.contains(im.world_to_src(w)) || full.contains(im.world_to_src(w)) {
                        let start_src = im.world_to_src(w);
                        self.drag = Some(Drag::Crop { id: self.sel.unwrap(), handle: 8, start_src, orig: im.clone() });
                        return;
                    }
                }
                let img = self.doc.layers.iter().rev().find(|l| {
                    l.visible && !l.locked && matches!(&l.kind, LayerKind::Image(im) if im.contains(w))
                });
                match img {
                    Some(l) => self.sel = Some(l.id),
                    None => self.drag = Some(Drag::Pan),
                }
            }
            Tool::Frame => {
                // the selected camera comes first
                if let Some(c) = self.cam_sel.and_then(|id| self.doc.cameras.iter().find(|c| c.id == id)) {
                    let (id, cr, upright, inside) = (c.id, c.rect, c.rot.abs() < 1e-6, c.contains(w));
                    for (i, p) in rect_handles(cr).iter().enumerate() {
                        if upright && xf.w2s(*p).distance(sp) <= 9.0 {
                            self.drag = Some(Drag::Cam { id, handle: i as u8, start: w, orig: cr });
                            self.mark();
                            return;
                        }
                    }
                    if inside {
                        self.drag = Some(Drag::Cam { id, handle: 8, start: w, orig: cr });
                        self.mark();
                        return;
                    }
                }
                if let Some(fr) = self.doc.export_frame {
                    for (i, p) in rect_handles(fr).iter().enumerate() {
                        if xf.w2s(*p).distance(sp) <= 9.0 {
                            self.drag = Some(Drag::Frame { handle: i as u8, start: w, orig: fr });
                            return;
                        }
                    }
                    if fr.contains(w) {
                        self.drag = Some(Drag::Frame { handle: 8, start: w, orig: fr });
                        return;
                    }
                }
                let p = w.round();
                self.doc.export_frame = Some(Rect::from_min_max(p, p));
                self.drag = Some(Drag::Frame { handle: 4, start: p, orig: Rect::from_min_max(p, p) });
                self.mark();
            }
            Tool::Space => self.space_down(sp, w),
            Tool::Hand | Tool::Picker => {}
        }
    }

    /// Space the 3D workspace shows: the selected layer's, the one already in view,
    /// the active one in the space tool, else the first that can be solved.
    fn ws_space(&self) -> Option<(u64, crate::persp::Camera)> {
        let cams = crate::persp::cameras(&self.doc.spaces);
        let ok = |s: &u64| cams.contains_key(s);
        let pick = self
            .sel_layer()
            .and_then(|l| l.place.as_ref())
            .map(|p| p.space)
            .filter(ok)
            .or(self.view3d.as_ref().map(|v| v.space).filter(ok))
            .or(self.space_sel.filter(ok))
            .or_else(|| self.doc.spaces.iter().map(|s| s.id).find(ok))?;
        cams.get(&pick).map(|c| (pick, c.clone()))
    }

    fn toggle_ws(&mut self) {
        if self.ws3d {
            self.cancel_modal3d();
            self.nav3d = None;
            self.ws3d = false;
            self.status = "2D 작업 공간".into();
        } else if let Some((sid, cam)) = self.ws_space() {
            if self.view3d.as_ref().is_none_or(|v| v.space != sid) {
                self.view3d = Some(crate::view3d::View3d::user(sid, &cam));
            }
            self.finish_text_edit();
            self.drag = None;
            self.ws3d = true;
            self.status = "3D 작업 공간 · Tab: 2D로 · 꾹 누르면 미리보기".into();
        }
    }

    fn grs_flat(&mut self, ctx: &egui::Context) {
        use crate::view3d::Op;
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let (mut confirm, mut cancel) = (false, false);
        for ev in &events {
            match ev {
                egui::Event::Text(t) => {
                    if let Some(g) = self.grs2d.as_mut() {
                        for c in t.chars() {
                            match c {
                                '-' => {
                                    if g.typed.starts_with('-') {
                                        g.typed.remove(0);
                                    } else {
                                        g.typed.insert(0, '-');
                                    }
                                }
                                '0'..='9' | '.' => g.typed.push(c),
                                _ => {}
                            }
                        }
                    }
                }
                egui::Event::PointerButton { button: PointerButton::Primary, pressed: true, .. } => confirm = true,
                egui::Event::PointerButton { button: PointerButton::Secondary, pressed: true, .. } => cancel = true,
                egui::Event::Touch { phase: TouchPhase::Start, force: Some(_), .. } => confirm = true,
                _ => {}
            }
        }
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        for (key, a) in [(Key::X, 0usize), (Key::Y, 1)] {
            if k(Modifiers::NONE, key) || k(Modifiers::SHIFT, key) {
                if let Some(g) = self.grs2d.as_mut() {
                    g.axis = if g.axis == Some(a) { None } else { Some(a) };
                }
            }
        }
        if k(Modifiers::NONE, Key::Backspace) {
            if let Some(g) = self.grs2d.as_mut() {
                g.typed.pop();
            }
        }
        if k(Modifiers::NONE, Key::Enter) {
            confirm = true;
        }
        if k(Modifiers::NONE, Key::Escape) {
            cancel = true;
        }
        let xf = self.xf();
        let zoom = self.zoom;
        let Some(g) = self.grs2d.as_mut() else { return };
        let cur = hover.unwrap_or(g.start);
        let cs = xf.w2s(g.center);
        let fine = if mods.shift { 0.1 } else { 1.0 };
        let (mut s, mut th, mut d) = (1.0f32, 0.0f32, Vec2::ZERO);
        match g.op {
            Op::Grab => {
                d = match (g.typed_value(), g.axis) {
                    (Some(v), Some(1)) => vec2(0.0, v),
                    (Some(v), _) => vec2(v, 0.0),
                    (None, _) => (cur - g.start) / zoom * fine,
                };
                match g.axis {
                    Some(0) => d.y = 0.0,
                    Some(_) => d.x = 0.0,
                    None => {}
                }
                if mods.ctrl {
                    d = (d / 10.0).round() * 10.0;
                }
            }
            Op::Rotate => {
                th = match g.typed_value() {
                    Some(v) => v.to_radians(),
                    None => ((cur - cs).angle() - (g.start - cs).angle()) * fine,
                };
                if mods.ctrl {
                    th = (th.to_degrees() / 5.0).round().to_radians() * 5.0;
                }
            }
            Op::Scale => {
                s = match g.typed_value() {
                    Some(v) => v,
                    None => 1.0 + ((cur - cs).length() / (g.start - cs).length().max(1.0) - 1.0) * fine,
                };
                if mods.ctrl {
                    s = (s * 10.0).round() / 10.0;
                }
                s = s.max(0.02);
            }
        }
        g.s = s;
        g.th = th;
        g.d = d;
        let c = g.center;
        let news: Vec<Layer> = if cancel {
            g.origs.iter().map(|(o, _)| o.clone()).collect()
        } else {
            g.origs.iter().map(|(o, part)| apply_part(o, part, c, s, th, d)).collect()
        };
        for l in news {
            if let Some(i) = self.doc.index_of(l.id) {
                self.doc.layers[i] = l;
            }
        }
        if cancel {
            self.grs2d = None;
        } else if confirm {
            self.grs2d = None;
            self.remap_pixel_sel(c, s, th, d);
            self.mark();
        }
    }

    /// The space camera as a projection of the 2D canvas.
    fn canvas_proj(&self, space: u64) -> Option<crate::view3d::Proj> {
        let cam = self.doc.spaces.iter().find(|s| s.id == space).and_then(|s| crate::persp::solve(s).ok())?;
        Some(crate::view3d::Proj::for_canvas(&cam, &self.xf(), self.canvas_rect))
    }

    /// G/R/S running on the 2D canvas: same keys as in the 3D workspace.
    fn modal_2d(&mut self, ctx: &egui::Context) {
        let Some(space) = self.modal3d.as_ref().and_then(|m| self.doc.get(m.id)).and_then(|l| l.place.as_ref()).map(|p| p.space) else {
            self.modal3d = None;
            return;
        };
        let Some(proj) = self.canvas_proj(space) else {
            self.cancel_modal3d();
            return;
        };
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let (mut confirm, mut cancel) = (false, false);
        for ev in &events {
            match ev {
                egui::Event::Text(t) => {
                    if let Some(m) = self.modal3d.as_mut() {
                        for c in t.chars() {
                            m.type_char(c);
                        }
                    }
                }
                egui::Event::PointerButton { button: PointerButton::Primary, pressed: true, .. } => confirm = true,
                egui::Event::PointerButton { button: PointerButton::Secondary, pressed: true, .. } => cancel = true,
                egui::Event::Touch { phase: TouchPhase::Start, force: Some(_), .. } => confirm = true,
                _ => {}
            }
        }
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        for (key, a) in [(Key::X, 0usize), (Key::Y, 1), (Key::Z, 2)] {
            if k(Modifiers::NONE, key) || k(Modifiers::SHIFT, key) {
                if let Some(m) = self.modal3d.as_mut() {
                    m.press_axis(a);
                }
            }
        }
        if k(Modifiers::NONE, Key::Backspace) {
            if let Some(m) = self.modal3d.as_mut() {
                m.typed.pop();
            }
        }
        if k(Modifiers::NONE, Key::Enter) {
            confirm = true;
        }
        if k(Modifiers::NONE, Key::Escape) {
            cancel = true;
        }
        if let Some(m) = &self.modal3d {
            let pl = m.apply(&proj, hover.unwrap_or(m.start), mods.ctrl, mods.shift);
            if let Some(l) = self.doc.get_mut(m.id) {
                l.place = Some(pl);
            }
        }
        if cancel {
            self.cancel_modal3d();
        } else if confirm {
            self.modal3d = None;
            self.mark();
        }
    }

    fn cancel_modal3d(&mut self) {
        if let Some(m) = self.modal3d.take() {
            if let Some(l) = self.doc.get_mut(m.id) {
                l.place = Some(m.orig);
            }
        }
    }

    fn canvas_3d(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, rect: Rect, sid: u64, cam: &crate::persp::Camera) {
        use crate::view3d::{Proj, View3d};
        if self.view3d.as_ref().is_none_or(|v| v.space != sid) {
            self.view3d = Some(View3d::user(sid, cam));
        }
        let painter = ui.painter_at(rect);
        theme::draw_void(&painter, rect);
        self.tex.begin_frame();
        let proj = Proj::new(self.view3d.as_ref().unwrap(), cam, rect);
        self.handle_3d_input(ctx, rect, ui.layer_id(), &proj);
        let view = self.view3d.clone().unwrap();
        let proj = Proj::new(&view, cam, rect);
        crate::view3d::draw(&painter, ctx, &self.doc, &view, &proj, cam, &mut self.tex, self.sel);
        // the light compass, turned with this view
        let light = self.doc.spaces.iter().find(|s| s.id == sid).and_then(|s| s.light);
        let compass = crate::view3d::Compass::new(rect);
        let hov = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| compass.contains(p));
        let ring = match self.light_drag {
            Some((_, _, crate::view3d::CompassPart::Ring(a, _), _)) => Some(a),
            _ => None,
        };
        // DV_COMPASS=hover shows the rings, =0/1/2 one ring as if dragged (captures)
        let dbg = std::env::var("DV_COMPASS").ok();
        let ring = ring.or(dbg.as_deref().and_then(|v| v.parse().ok()));
        let hov = hov || dbg.is_some();
        compass.draw(&painter, &proj, light.as_ref(), hov || self.light_drag.is_some(), ring);
        if crate::glass::available() {
            painter.add(crate::glass::scene_marker(ctx.content_rect(), ctx.cumulative_pass_nr()));
        }
        match &self.modal3d {
            Some(m) => {
                crate::view3d::hud(&painter, rect, &m.label());
                if m.op != crate::view3d::Op::Grab {
                    if let Some(hp) = ctx.input(|i| i.pointer.hover_pos()) {
                        painter.extend(Shape::dashed_line(&[m.center, hp], Stroke::new(1.0, INK_2()), 4.0, 4.0));
                    }
                }
            }
            None => {
                let name = self.doc.spaces.iter().find(|s| s.id == sid).map(|s| s.name.clone()).unwrap_or_default();
                let mode = if view.camera { "카메라" } else if view.ortho { "직교" } else { "원근" };
                crate::view3d::hud(&painter, rect, &format!("{name} · {mode} · 가운데 버튼: 회전 (Shift 이동, Ctrl 확대) · 휠: 확대 · G/R/S · Tab: 2D"));
            }
        }
        if crate::glass::available() {
            painter.add(crate::glass::ui_marker(ctx.content_rect(), ctx.cumulative_pass_nr()));
        }
        if self.modal3d.is_some() || self.nav3d.is_some() {
            ctx.request_repaint();
        }
    }

    fn handle_3d_input(&mut self, ctx: &egui::Context, rect: Rect, layer: egui::LayerId, proj: &crate::view3d::Proj) {
        use crate::view3d::{Modal, Nav, Op};
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let typing = ctx.text_edit_focused();
        let over = |p: Pos2| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == layer);
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let pen = crate::penhook::state();
        let sid = self.view3d.as_ref().map(|v| v.space);
        let nav_of = |m: Modifiers| if m.shift { Nav::Pan } else if m.ctrl { Nav::Zoom } else { Nav::Orbit };
        // pen side button: orbit (Shift: pan, Ctrl: zoom) while hovering
        if pen.barrel && pen.in_range && self.nav3d.is_none() && self.modal3d.is_none() {
            if let Some(hp) = hover.filter(|p| over(*p)) {
                self.nav3d = Some(nav_of(mods));
                self.source = Source::PenHover;
                self.last_screen = hp;
            }
        }
        if (!pen.barrel || !pen.in_range) && self.source == Source::PenHover && self.nav3d.is_some() {
            self.nav3d = None;
        }
        let mut click: Option<Pos2> = None;
        let mut cancel = false;
        let compass = crate::view3d::Compass::new(rect);
        for ev in &events {
            match ev {
                egui::Event::PointerButton { pos, button: PointerButton::Middle, pressed: true, modifiers } if over(*pos) && self.modal3d.is_none() => {
                    self.nav3d = Some(nav_of(*modifiers));
                    self.source = Source::Middle;
                    self.last_screen = *pos;
                }
                egui::Event::PointerButton { button: PointerButton::Middle, pressed: false, .. } if self.source == Source::Middle => {
                    self.nav3d = None;
                }
                egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } if over(*pos) && self.modal3d.is_none() && compass.contains(*pos) => {
                    let sp = sid.and_then(|s| self.doc.spaces.iter().find(|x| x.id == s));
                    let light = sp.and_then(|x| x.light);
                    match (sp.map(|x| x.id), light, compass.hit(proj, light.as_ref(), *pos)) {
                        (Some(s), Some(l), Some(part)) if part != crate::view3d::CompassPart::Body => {
                            self.light_drag = Some((s, l, part, *pos));
                            self.status = "빛 회전 · Esc: 취소".into();
                        }
                        (_, None, _) => self.status = "조명 꺼짐 · 공간 패널에서 켜기".into(),
                        _ => {}
                    }
                }
                egui::Event::PointerButton { button: PointerButton::Primary, pressed: false, .. } if self.light_drag.is_some() => {
                    self.light_drag = None;
                    self.status.clear();
                    self.mark();
                }
                egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } if over(*pos) => click = Some(*pos),
                egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed: true, .. } if over(*pos) => cancel = true,
                egui::Event::Touch { phase: TouchPhase::Start, pos, force: Some(_), .. } if over(*pos) && !pen.barrel => click = Some(*pos),
                egui::Event::Touch { phase: TouchPhase::Move, pos, .. } if self.nav3d.is_some() && self.source == Source::PenHover => {
                    let d = *pos - self.last_screen;
                    self.last_screen = *pos;
                    if let (Some(v), Some(n)) = (self.view3d.as_mut(), self.nav3d) {
                        crate::view3d::navigate(v, proj, n, d);
                    }
                }
                egui::Event::PointerMoved(p) if self.nav3d.is_some() => {
                    let d = *p - self.last_screen;
                    self.last_screen = *p;
                    if let (Some(v), Some(n)) = (self.view3d.as_mut(), self.nav3d) {
                        crate::view3d::navigate(v, proj, n, d);
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } if hover.is_some_and(|p| over(p)) => {
                    let dy = match unit {
                        egui::MouseWheelUnit::Point => delta.y,
                        egui::MouseWheelUnit::Line => delta.y * 40.0,
                        egui::MouseWheelUnit::Page => delta.y * 400.0,
                    };
                    if let Some(v) = self.view3d.as_mut() {
                        crate::view3d::zoom(v, dy as f64 * 0.0025);
                    }
                }
                egui::Event::Text(t) if !typing => {
                    if let Some(m) = self.modal3d.as_mut() {
                        for c in t.chars() {
                            m.type_char(c);
                        }
                    }
                }
                _ => {}
            }
        }
        if !typing {
            let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
            if self.modal3d.is_some() {
                for (key, a) in [(Key::X, 0usize), (Key::Y, 1), (Key::Z, 2)] {
                    if k(Modifiers::NONE, key) || k(Modifiers::SHIFT, key) {
                        if let Some(m) = self.modal3d.as_mut() {
                            m.press_axis(a);
                        }
                    }
                }
                if k(Modifiers::NONE, Key::Backspace) {
                    if let Some(m) = self.modal3d.as_mut() {
                        m.typed.pop();
                    }
                }
                if k(Modifiers::NONE, Key::Enter) {
                    click = Some(hover.unwrap_or(rect.center()));
                }
                if k(Modifiers::NONE, Key::Escape) {
                    cancel = true;
                }
            } else if self.light_drag.is_some() {
                if k(Modifiers::NONE, Key::Escape) {
                    cancel = true;
                }
            } else {
                let target = self
                    .sel_layer()
                    .and_then(|l| l.place.clone().filter(|p| Some(p.space) == sid).map(|p| (l.id, p)));
                for (key, op) in [(Key::G, Op::Grab), (Key::R, Op::Rotate), (Key::S, Op::Scale)] {
                    if k(Modifiers::ALT, key) {
                        if let Some((id, _)) = &target {
                            if let Some(pl) = self.doc.get_mut(*id).and_then(|l| l.place.as_mut()) {
                                match op {
                                    Op::Grab => pl.pos = [0.0; 3],
                                    Op::Rotate => pl.rot = [0.0; 3],
                                    Op::Scale => pl.scale = 1.0,
                                }
                            }
                            self.mark();
                        }
                    } else if k(Modifiers::NONE, key) {
                        match (&target, hover) {
                            (Some((id, pl)), Some(hp)) => self.modal3d = Some(Modal::new(op, *id, pl.clone(), hp, proj)),
                            _ => self.status = "이 공간의 레이어 선택 필요".into(),
                        }
                    }
                }
                if let Some(v) = self.view3d.as_mut() {
                    for (key, n) in [(Key::Num1, 1u8), (Key::Num3, 3), (Key::Num7, 7)] {
                        if k(Modifiers::NONE, key) {
                            v.preset(n, false);
                        } else if k(Modifiers::CTRL, key) {
                            v.preset(n, true);
                        }
                    }
                    if k(Modifiers::NONE, Key::Num5) {
                        v.ortho = !v.ortho;
                        v.camera = false;
                    }
                    if k(Modifiers::NONE, Key::Num0) {
                        v.camera = true;
                    }
                    if k(Modifiers::NONE, Key::Period) {
                        if let Some((_, pl)) = &target {
                            v.target = pl.world_affine().1;
                            v.camera = false;
                        }
                    }
                }
                if k(Modifiers::NONE, Key::Home) {
                    if let Some((sid, cam)) = self.ws_space() {
                        self.view3d = Some(crate::view3d::View3d::user(sid, &cam));
                    }
                }
                if k(Modifiers::NONE, Key::Escape) {
                    self.select_only(None);
                }
            }
        }
        // live transform
        if let Some(m) = &self.modal3d {
            let cur = hover.unwrap_or(m.start);
            let pl = m.apply(proj, cur, mods.ctrl, mods.shift);
            if let Some(l) = self.doc.get_mut(m.id) {
                l.place = Some(pl);
            }
        }
        // live light turn on the compass: a ring turns it about that world axis by
        // the pointer's motion along the ring; the sun follows the pointer
        if let Some((s, l0, part, last)) = self.light_drag {
            use crate::view3d::CompassPart;
            let cur = hover.unwrap_or(last);
            let (dir, part) = match part {
                CompassPart::Ring(a, t) => {
                    let dt = compass.ring_step(proj, a, t, cur - last);
                    let mut axis = [0.0; 3];
                    axis[a] = 1.0;
                    let cur_l = self.doc.spaces.iter().find(|x| x.id == s).and_then(|x| x.light).unwrap_or(l0);
                    (rotate_about(cur_l.dir(), axis, dt), CompassPart::Ring(a, t + dt))
                }
                CompassPart::Sun { front } => (compass.sun_dir(proj, cur, front), part),
                CompassPart::Body => (l0.dir(), part),
            };
            if cur != last {
                let ambient = l0.ambient;
                if let Some(sp) = self.doc.spaces.iter_mut().find(|x| x.id == s) {
                    sp.light = Some(crate::persp::Light::from_dir(dir, ambient));
                }
            }
            self.light_drag = Some((s, l0, part, cur));
            ctx.request_repaint();
        }
        if cancel {
            if let Some((s, l0, _, _)) = self.light_drag.take() {
                if let Some(sp) = self.doc.spaces.iter_mut().find(|x| x.id == s) {
                    sp.light = Some(l0);
                }
            }
            self.cancel_modal3d();
        } else if let Some(p) = click {
            if self.modal3d.take().is_some() {
                self.mark();
            } else if !compass.contains(p) {
                let id = match proj.canvas_xf() {
                    Some(xf) => self.hit_layer(xf.s2w(p), 6.0 / xf.zoom),
                    None => sid.and_then(|s| crate::view3d::pick(&self.doc, proj, s, p)),
                };
                self.select_only(id);
            }
        }
    }

    fn header3d(&mut self, ctx: &egui::Context) {
        let mut back = false;
        egui::Area::new(egui::Id::new("header3d"))
            .order(egui::Order::Middle)
            .anchor(Align2::CENTER_TOP, vec2(0.0, SPACE_LG))
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::same(6), theme::RADIUS_CARD, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let Some(v) = self.view3d.as_mut() else { return };
                        if ui.selectable_label(v.camera, "카메라 0").clicked() {
                            v.camera = true;
                        }
                        if ui.button("앞 1").clicked() {
                            v.preset(1, false);
                        }
                        if ui.button("오른쪽 3").clicked() {
                            v.preset(3, false);
                        }
                        if ui.button("위 7").clicked() {
                            v.preset(7, false);
                        }
                        if ui.selectable_label(v.ortho && !v.camera, "직교 5").clicked() {
                            v.ortho = !v.ortho;
                            v.camera = false;
                        }
                        ui.separator();
                        if ui.button("2D로 · Tab").clicked() {
                            back = true;
                        }
                    });
                });
            });
        if back {
            self.toggle_ws();
        }
    }

    /// Hold Tab: the other workspace in a card that nearly fills the canvas.
    fn peek_ui(&mut self, ctx: &egui::Context) {
        if !self.peek {
            return;
        }
        let cr = self.canvas_rect;
        let pr = cr.shrink2(cr.size() * 0.04);
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("peek")));
        p.rect_filled(cr, 0.0, Color32::from_black_alpha(110));
        let clip = p.with_clip_rect(pr);
        if self.ws3d {
            // the 2D canvas as it is, scaled into the card
            let k = pr.width() / cr.width();
            let xf = Xf { min: pr.min, pan: self.pan * k, zoom: self.zoom * k };
            theme::draw_ground(&clip, pr, &xf);
            canvas::draw_doc(&clip, ctx, &self.doc, &xf, &mut self.tex, None);
        } else if let Some((sid, cam)) = self.ws_space() {
            let view = match &self.view3d {
                Some(v) if v.space == sid => v.clone(),
                _ => crate::view3d::View3d::user(sid, &cam),
            };
            theme::draw_void(&clip, pr);
            let proj = crate::view3d::Proj::new(&view, &cam, pr);
            crate::view3d::draw(&clip, ctx, &self.doc, &view, &proj, &cam, &mut self.tex, self.sel);
        }
        p.rect_stroke(pr, 12.0, Stroke::new(1.5, INK_3()), StrokeKind::Outside);
        p.text(pr.left_top() + vec2(16.0, 14.0), Align2::LEFT_TOP, if self.ws3d { "2D 미리보기" } else { "3D 미리보기" }, FontId::proportional(13.0), INK_2());
        ctx.request_repaint();
    }


    /// While a vanishing-line end or the origin pin is dragged: a round magnifier of
    /// the pixels around it with a thin X reticle, set off from the pen tip.
    fn draw_loupe(&mut self, ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
        let (target, line) = match &self.drag {
            Some(Drag::SpaceLine { id, idx, end, .. }) if *end < 2 => {
                let Some(l) = self.doc.spaces.iter().find(|s| s.id == *id).and_then(|s| s.lines.get(*idx)) else { return };
                (if *end == 0 { l.a } else { l.b }, Some((l.a, l.b, crate::persp::AXIS_COLORS[(l.axis as usize).min(2)])))
            }
            Some(Drag::WallCorner { sid, wid, idx }) => {
                match self.doc.spaces.iter().find(|s| s.id == *sid).and_then(|s| s.walls.iter().find(|x| x.id == *wid)) {
                    Some(wl) => (wl.corners[*idx], Some((wl.corners[(*idx + 3) % 4], wl.corners[*idx], crate::view3d::WALL_COLOR))),
                    None => return,
                }
            }
            _ => return,
        };
        let xf = self.xf();
        let ts = xf.w2s(target);
        let (r, gap) = (72.0f32, 36.0f32);
        let mut c = ts - vec2(r + gap, r + gap);
        if c.x - r < rect.left() + 8.0 {
            c.x = ts.x + r + gap;
        }
        if c.y - r < rect.top() + 8.0 {
            c.y = ts.y + r + gap;
        }
        // the pixels around the target, rendered on the CPU and drawn as a round disc
        let zoom = (self.zoom * 6.0).max(4.0);
        let ppp = ctx.pixels_per_point();
        let key = [target.x, target.y, zoom];
        if self.loupe_tex.as_ref().is_none_or(|(_, k)| *k != key) {
            let region = Rect::from_center_size(target, vec2(2.0 * r / zoom, 2.0 * r / zoom));
            if let Ok(pm) = crate::export::render(&self.doc, region, zoom * ppp, None) {
                let ci = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
                let opts = egui::TextureOptions::NEAREST;
                match &mut self.loupe_tex {
                    Some((t, k)) => {
                        t.set(ci, opts);
                        *k = key;
                    }
                    None => self.loupe_tex = Some((ctx.load_texture("loupe", ci, opts), key)),
                }
            }
        }
        let Some((tex, _)) = &self.loupe_tex else { return };
        painter.circle_filled(c, r, theme::VOID());
        let mut m = egui::Mesh::with_texture(tex.id());
        m.vertices.push(egui::epaint::Vertex { pos: c, uv: pos2(0.5, 0.5), color: Color32::WHITE });
        let n = 72u32;
        for i in 0..=n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let d = vec2(a.cos(), a.sin());
            m.vertices.push(egui::epaint::Vertex { pos: c + d * r, uv: pos2(0.5 + 0.5 * d.x, 0.5 + 0.5 * d.y), color: Color32::WHITE });
        }
        for i in 1..=n {
            m.add_triangle(0, i, i + 1);
        }
        painter.add(Shape::mesh(m));
        let to_l = |p: Pos2| c + (p - target) * zoom;
        if let Some((a, b, col)) = line {
            let lp = painter.with_clip_rect(Rect::from_center_size(c, vec2(2.0 * r, 2.0 * r)).intersect(painter.clip_rect()));
            // clip the line to the disc
            let (pa, pb) = (to_l(a), to_l(b));
            let d = pb - pa;
            let f = pa - c;
            let (qa, qb, qc) = (d.dot(d), 2.0 * f.dot(d), f.dot(f) - r * r);
            let disc = qb * qb - 4.0 * qa * qc;
            if qa > 0.0 && disc > 0.0 {
                let sq = disc.sqrt();
                let t0 = ((-qb - sq) / (2.0 * qa)).max(0.0);
                let t1 = ((-qb + sq) / (2.0 * qa)).min(1.0);
                if t1 > t0 {
                    lp.line_segment([pa + d * t0, pa + d * t1], Stroke::new(1.0, col.gamma_multiply(0.9)));
                }
            }
        }
        painter.circle_stroke(c, r, Stroke::new(2.5, Color32::from_black_alpha(160)));
        painter.circle_stroke(c, r, Stroke::new(1.2, INK_2()));
        for d in [vec2(1.0, 1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(-1.0, -1.0)] {
            let d = d.normalized();
            let (p0, p1) = (c + d * 5.0, c + d * (r - 6.0));
            painter.line_segment([p0, p1], Stroke::new(2.0, Color32::from_black_alpha(150)));
            painter.line_segment([p0, p1], Stroke::new(0.8, Color32::WHITE));
        }
    }

    fn space_idx(&self) -> Option<usize> {
        self.space_sel.and_then(|id| self.doc.spaces.iter().position(|s| s.id == id))
    }

    fn set_space_axis(&mut self, ax: u8) {
        self.space_axis = ax;
        if let (Some(i), Some(li)) = (self.space_idx(), self.space_line) {
            if let Some(l) = self.doc.spaces[i].lines.get_mut(li) {
                if l.axis != ax {
                    l.axis = ax;
                    self.mark();
                }
            }
        }
    }

    fn space_down(&mut self, sp: Pos2, w: Pos2) {
        let xf = self.xf();
        if self.wall_base_new {
            if let Some(i) = self.space_idx() {
                let sid = self.doc.spaces[i].id;
                let wid = self.doc.new_id();
                let n = self.doc.spaces[i].walls.len() + 1;
                self.doc.spaces[i].walls.push(crate::persp::Wall { id: wid, name: format!("벽 {n}"), corners: [w; 4], offset: 0.0, snap_axis: false });
                self.wall_sel = Some(wid);
                self.wall_base_new = false;
                self.drag = Some(Drag::WallBase { sid, wid, start: w });
                self.mark();
                return;
            }
        }
        if let Some(lid) = self.space_place.take() {
            if let Some(i) = self.space_idx() {
                self.stand_on_floor(lid, self.doc.spaces[i].id, w);
                return;
            }
        }
        if self.wall_new {
            if let Some(i) = self.space_idx() {
                let sid = self.doc.spaces[i].id;
                let wid = self.doc.new_id();
                let n = self.doc.spaces[i].walls.len() + 1;
                self.doc.spaces[i].walls.push(crate::persp::Wall { id: wid, name: format!("벽 {n}"), corners: [w; 4], offset: 0.0, snap_axis: false });
                self.wall_sel = Some(wid);
                self.wall_new = false;
                self.drag = Some(Drag::WallRect { sid, wid, start: w });
                self.mark();
                return;
            }
        }
        if self.space_tape_draw {
            if let Some(i) = self.space_idx() {
                let id = self.doc.spaces[i].id;
                self.doc.spaces[i].tape = Some(crate::persp::RefLine { a: w, b: w, axis: 0, metres: 0.0 });
                self.drag = Some(Drag::SpaceTape { id });
                self.mark();
                return;
            }
        }
        if self.space_ref_draw {
            if let Some(i) = self.space_idx() {
                let id = self.doc.spaces[i].id;
                let axis = self.space_axis;
                let metres = self.doc.spaces[i].ref_line.map_or(1.0, |r| r.metres);
                self.doc.spaces[i].ref_line = Some(crate::persp::RefLine { a: w, b: w, axis, metres });
                self.drag = Some(Drag::SpaceRef { id });
                self.mark();
                return;
            }
        }
        if self.space_mask_draw {
            if let Some(i) = self.space_idx() {
                let id = self.doc.spaces[i].id;
                self.doc.spaces[i].mask = Some(Rect::from_min_max(w, w));
                self.drag = Some(Drag::SpaceMask { id, start: w });
                self.mark();
                return;
            }
        }
        if let (Some(i), false) = (self.space_idx(), self.space_new) {
            let s = &self.doc.spaces[i];
            let id = s.id;
            // wall corners (the selected wall first)
            let mut walls: Vec<&crate::persp::Wall> = s.walls.iter().collect();
            walls.sort_by_key(|w| Some(w.id) != self.wall_sel);
            for wl in walls {
                for (ci, c) in wl.corners.iter().enumerate() {
                    if xf.w2s(*c).distance(sp) <= 9.0 {
                        let wid = wl.id;
                        self.wall_sel = Some(wid);
                        self.drag = Some(Drag::WallCorner { sid: id, wid, idx: ci });
                        self.mark();
                        return;
                    }
                }
            }
            let mut hit: Option<Drag> = None;
            // endpoints first (the selected line wins ties)
            let order: Vec<usize> = self.space_line.into_iter().chain(0..s.lines.len()).collect();
            'ends: for &li in &order {
                let Some(l) = s.lines.get(li) else { continue };
                for (end, p) in [(0u8, l.a), (1, l.b)] {
                    if xf.w2s(p).distance(sp) <= 8.0 {
                        self.space_line = Some(li);
                        hit = Some(Drag::SpaceLine { id, idx: li, end, start: w, orig: *l });
                        break 'ends;
                    }
                }
            }
            if hit.is_none() {
                for (h, p) in rect_handles(s.frame).iter().enumerate() {
                    if xf.w2s(*p).distance(sp) <= 8.0 {
                        hit = Some(Drag::SpaceFrame { id, handle: h as u8, start: w, orig: s.frame });
                        break;
                    }
                }
            }
            if hit.is_none() {
                for (li, l) in s.lines.iter().enumerate() {
                    if seg_dist(sp, xf.w2s(l.a), xf.w2s(l.b)) <= 5.0 {
                        self.space_line = Some(li);
                        hit = Some(Drag::SpaceLine { id, idx: li, end: 2, start: w, orig: *l });
                        break;
                    }
                }
            }
            let drag = match hit {
                Some(d) => d,
                None => {
                    // a new vanishing line on the current axis
                    let l = crate::persp::VLine { a: w, b: w, axis: self.space_axis };
                    self.doc.spaces[i].lines.push(l);
                    let idx = self.doc.spaces[i].lines.len() - 1;
                    self.space_line = Some(idx);
                    Drag::SpaceLine { id, idx, end: 1, start: w, orig: l }
                }
            };
            self.drag = Some(drag);
            self.mark();
            return;
        }
        // a new space: drag its frame (a click fits it to the photo underneath)
        let id = self.doc.new_id();
        let n = self.doc.spaces.len() + 1;
        let p = w.round();
        self.doc.spaces.push(crate::persp::Space::new(id, format!("공간 {n}"), Rect::from_min_max(p, p)));
        self.space_sel = Some(id);
        self.space_new = false;
        self.space_line = None;
        self.drag = Some(Drag::SpaceFrame { id, handle: 4, start: p, orig: Rect::from_min_max(p, p) });
        self.mark();
    }

    fn mark_if_changed_raster(&mut self) {
        if matches!(self.drag, Some(Drag::Raster { .. })) {
            self.mark();
        }
    }

    fn pointer_move(&mut self, sp: Pos2, pressure: f32, mods: Modifiers) {
        let xf = self.xf();
        let w = xf.s2w(sp);
        let delta_s = sp - self.last_screen;
        self.last_screen = sp;
        let smoothing = self.s.smoothing.clamp(0.0, 0.95);
        let zoom = self.zoom;
        // Space held mid-drag: the view pans with the pointer while the dragged thing
        // stays under it; letting go of Space carries on from the same spot
        if self.space_held && self.drag.as_ref().is_some_and(|d| !matches!(d, Drag::Pan | Drag::Zoom { .. } | Drag::Size { .. })) {
            self.pan += delta_s;
            return;
        }
        match self.drag {
            Some(Drag::Zoom { anchor }) => {
                self.zoom_at(anchor, ((delta_s.x - delta_s.y) * 0.006).exp());
                return;
            }
            Some(Drag::Size { start, x0, .. }) => {
                let lo = if matches!(self.tool, Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse) { 0.0 } else { 0.5 };
                let v = (start * ((sp.x - x0) * 0.012).exp()).clamp(lo, 1000.0);
                if let Some(slot) = self.tool_size_mut() {
                    *slot = v;
                }
                // a selected line/shape follows along
                if matches!(self.tool, Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse) {
                    if let Some(id) = self.sel {
                        if let Some(LayerKind::Shape(s)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                            s.width = v;
                            self.changed = true;
                            self.modified = true;
                        }
                    }
                }
                return;
            }
            Some(Drag::Marquee { start, full, .. }) => {
                self.drag = Some(Drag::Marquee { start, cur: w, full });
                return;
            }
            Some(Drag::Lasso { ref mut pts }) => {
                if pts.last().is_none_or(|q| (*q - w).length() * zoom > 2.0) {
                    pts.push(w);
                }
                return;
            }
            _ => {}
        }
        let Some(drag) = self.drag.as_mut() else { return };
        match drag {
            Drag::Zoom { .. } | Drag::Size { .. } | Drag::Marquee { .. } | Drag::Lasso { .. } => {}
            Drag::Group { rotate, center, start, origs, s: s_out, th: th_out, .. } => {
                let c = *center;
                let (mut s, mut th) = (1.0f32, 0.0f32);
                if *rotate {
                    let a0 = (*start - c).y.atan2((*start - c).x);
                    let a1 = (w - c).y.atan2((w - c).x);
                    th = a1 - a0;
                    // absolute snapping is measured on the first photo's angle, if any
                    let base = origs
                        .iter()
                        .find_map(|(l, _)| match &l.kind {
                            LayerKind::Image(im) => Some(im.rotation),
                            _ => None,
                        })
                        .unwrap_or(0.0);
                    th = snap_angle(base + th, base, mods) - base;
                } else {
                    s = ((w - c).length() / (*start - c).length().max(1e-3)).max(0.02);
                    if mods.shift {
                        s = ((s * 20.0).round() / 20.0).max(0.05);
                    }
                }
                *s_out = s;
                *th_out = th;
                for (o, part) in origs.iter() {
                    let l = apply_part(o, part, c, s, th, Vec2::ZERO);
                    if let Some(i) = self.doc.index_of(l.id) {
                        self.doc.layers[i] = l;
                    }
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::MoveMany { start, origs, d: d_out } => {
                let mut d = w - *start;
                if mods.shift {
                    if d.x.abs() > d.y.abs() { d.y = 0.0 } else { d.x = 0.0 }
                }
                *d_out = d;
                for (o, part) in origs.iter() {
                    let l = apply_part(o, part, Pos2::ZERO, 1.0, 0.0, d);
                    if let Some(i) = self.doc.index_of(l.id) {
                        self.doc.layers[i] = l;
                    }
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::Pan => self.pan += delta_s,
            Drag::Move { id, start, orig } => {
                let mut d = w - *start;
                if mods.shift {
                    if d.x.abs() > d.y.abs() { d.y = 0.0 } else { d.x = 0.0 }
                }
                let mut l = orig.clone();
                let cam = orig.place.as_ref().and_then(|pl| self.doc.spaces.iter().find(|s| s.id == pl.space)).and_then(|s| crate::persp::solve(s).ok());
                match (orig.place.as_ref(), cam) {
                    (Some(o), Some(cam)) => {
                        // slide along the layer's own plane under the pointer
                        if let (Some(a), Some(b)) = (o.ray_hit(&cam, *start), o.ray_hit(&cam, *start + d)) {
                            let mut pl = o.clone();
                            pl.shift_world(crate::persp::sub(b, a));
                            l.place = Some(pl);
                        }
                    }
                    _ => l.translate(d),
                }
                if let Some(i) = self.doc.index_of(*id) {
                    self.doc.layers[i] = l;
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::ScaleImage { id, sign, orig } => {
                let (u, v) = orig.axes();
                let h = orig.half();
                let opp = orig.center - u * (sign.x * h.x) - v * (sign.y * h.y);
                let d = w - opp;
                let du = d.dot(u) * sign.x;
                let dv = d.dot(v) * sign.y;
                let full = h * 2.0;
                let t = ((du * full.x + dv * full.y) / full.length_sq()).max(0.01);
                let mut s = orig.scale * t;
                if mods.shift {
                    // snap to 5% steps
                    s = (s * 20.0).round() / 20.0;
                }
                let s = s.max(0.005);
                let nh = orig.crop.size() * s * 0.5;
                let id = *id;
                if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    im.scale = s;
                    im.center = opp + u * (sign.x * nh.x) + v * (sign.y * nh.y);
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::Rotate { id, orig } => {
                let d = w - orig.center;
                let mut a = d.y.atan2(d.x) + std::f32::consts::FRAC_PI_2;
                a = snap_angle(a, orig.rotation, mods);
                let id = *id;
                if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    im.rotation = a;
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::ShapePt { id, which } => {
                let (id, which) = (*id, *which);
                if let Some(LayerKind::Shape(s)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    let other = if which == 0 { s.b } else { s.a };
                    let p = constrain_shape(s.kind, other, w, mods.shift);
                    if which == 0 { s.a = p } else { s.b = p }
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::TextWrap { id } => {
                let id = *id;
                if let Some(LayerKind::Text(t)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    t.wrap = (w.x - t.pos.x).max(t.size * 1.5);
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::NewShape { id, start } => {
                let (id, start) = (*id, *start);
                if let Some(LayerKind::Shape(s)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    s.b = constrain_shape(s.kind, start, w, mods.shift);
                }
                self.changed = true;
            }
            Drag::Crop { id, handle, start_src, orig } => {
                let src = orig.world_to_src(w);
                let full = Rect::from_min_size(Pos2::ZERO, orig.src.size());
                let mut c = orig.crop;
                if *handle == 8 {
                    let d = (src - *start_src).round();
                    let d = vec2(
                        d.x.clamp(full.min.x - c.min.x, full.max.x - c.max.x),
                        d.y.clamp(full.min.y - c.min.y, full.max.y - c.max.y),
                    );
                    c = c.translate(d);
                } else {
                    let p = src.round().clamp(full.min, full.max);
                    let (hx, hy) = handle_axes(*handle);
                    if hx < 0 { c.min.x = p.x.min(c.max.x - 1.0) }
                    if hx > 0 { c.max.x = p.x.max(c.min.x + 1.0) }
                    if hy < 0 { c.min.y = p.y.min(c.max.y - 1.0) }
                    if hy > 0 { c.max.y = p.y.max(c.min.y + 1.0) }
                }
                let mut n = orig.clone();
                n.crop = c;
                n.center = orig.src_to_world(c.center());
                let id = *id;
                if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    *im = n;
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::SpaceFrame { id, handle, start, orig } => {
                let p = w.round();
                let mut r = *orig;
                if orig.width() == 0.0 && orig.height() == 0.0 {
                    r = Rect::from_two_pos(*start, p);
                } else {
                    let (hx, hy) = handle_axes(*handle);
                    if hx < 0 { r.min.x = p.x }
                    if hx > 0 { r.max.x = p.x }
                    if hy < 0 { r.min.y = p.y }
                    if hy > 0 { r.max.y = p.y }
                    r = Rect::from_two_pos(r.min, r.max);
                }
                let id = *id;
                if let Some(s) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                    s.frame = r;
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::SpaceLine { id, idx, end, start, orig } => {
                let (id, idx, end, start, orig) = (*id, *idx, *end, *start, *orig);
                if let Some(l) = self.doc.spaces.iter_mut().find(|s| s.id == id).and_then(|s| s.lines.get_mut(idx)) {
                    match end {
                        0 => l.a = w,
                        1 => l.b = w,
                        _ => {
                            let d = w - start;
                            l.a = orig.a + d;
                            l.b = orig.b + d;
                        }
                    }
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::WallRect { sid, wid, start } => {
                let (sid, wid, start) = (*sid, *wid, *start);
                let rr = Rect::from_two_pos(start, w);
                if let Some(wl) = self.doc.spaces.iter_mut().find(|s| s.id == sid).and_then(|s| s.walls.iter_mut().find(|x| x.id == wid)) {
                    wl.corners = [rr.left_top(), rr.right_top(), rr.right_bottom(), rr.left_bottom()];
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::WallBase { sid, wid, start } => {
                let (sid, wid, start) = (*sid, *wid, *start);
                if let Some(sp) = self.doc.spaces.iter_mut().find(|s| s.id == sid) {
                    let quad = crate::persp::solve(sp).ok().and_then(|cam| crate::persp::wall_on_base(&cam, start, w, sp.floor * 1.6));
                    if let (Some(q), Some(wl)) = (quad, sp.walls.iter_mut().find(|x| x.id == wid)) {
                        wl.corners = q;
                    }
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::WallCorner { sid, wid, idx } => {
                let (sid, wid, idx) = (*sid, *wid, *idx);
                if let Some(wl) = self.doc.spaces.iter_mut().find(|s| s.id == sid).and_then(|s| s.walls.iter_mut().find(|x| x.id == wid)) {
                    wl.corners[idx] = w;
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::SpaceTape { id } => {
                let id = *id;
                if let Some(sp) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                    let cam = crate::persp::solve(sp).ok();
                    if let Some(r) = sp.tape.as_mut() {
                        r.b = w;
                        if let Some(ax) = cam.as_ref().and_then(|c| r.infer_axis(c)) {
                            r.axis = ax;
                        }
                    }
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::SpaceRef { id } => {
                let id = *id;
                if let Some(sp) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                    let cam = crate::persp::solve(sp).ok();
                    if let Some(r) = sp.ref_line.as_mut() {
                        r.b = w;
                        // the axis follows the drawing (X/Y along the floor, Z up)
                        if let Some(ax) = cam.as_ref().and_then(|c| r.infer_axis(c)) {
                            r.axis = ax;
                        }
                    }
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::SpaceMask { id, start } => {
                let (id, start) = (*id, *start);
                if let Some(s) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                    s.mask = Some(Rect::from_two_pos(start, w));
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::Cam { id, handle, start, orig } => {
                let p = w.round();
                let mut rr = *orig;
                if *handle == 8 {
                    rr = rr.translate((w - *start).round());
                } else {
                    let (hx, hy) = handle_axes(*handle);
                    if hx < 0 { rr.min.x = p.x }
                    if hx > 0 { rr.max.x = p.x }
                    if hy < 0 { rr.min.y = p.y }
                    if hy > 0 { rr.max.y = p.y }
                    rr = Rect::from_two_pos(rr.min, rr.max);
                }
                let id = *id;
                if let Some(c) = self.doc.cameras.iter_mut().find(|c| c.id == id) {
                    c.rect = rr;
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::Frame { handle, start, orig } => {
                let p = w.round();
                let mut r = *orig;
                if *handle == 8 {
                    r = r.translate((w - *start).round());
                } else if orig.width() == 0.0 && orig.height() == 0.0 {
                    r = Rect::from_two_pos(*start, p);
                } else {
                    let (hx, hy) = handle_axes(*handle);
                    if hx < 0 { r.min.x = p.x }
                    if hx > 0 { r.max.x = p.x }
                    if hy < 0 { r.min.y = p.y }
                    if hy > 0 { r.max.y = p.y }
                    r = Rect::from_two_pos(r.min, r.max);
                }
                self.doc.export_frame = Some(r);
                self.changed = true;
                self.modified = true;
            }
            Drag::Raster { id, stroke, smooth } => {
                let np = smooth.0 + (w - smooth.0) * (1.0 - smoothing);
                let npr = smooth.1 + (pressure - smooth.1) * (1.0 - smoothing * 0.5);
                *smooth = (np, npr);
                let id = *id;
                if let (Some(st), Some(LayerKind::Raster(r))) = (stroke.as_mut(), self.doc.get_mut(id).map(|l| &mut l.kind)) {
                    st.add_point(r, np - r.offset, npr);
                    if let Some((c, _, live)) = self.rec_clock {
                        let lp = np - r.offset;
                        self.rec_pts.push([lp.x, lp.y, npr]);
                        self.rec_times.push(if live { c.elapsed().as_secs_f32() } else { 0.0 });
                    }
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::Vector { id, pts, smooth, base_r, pressure: use_p, .. } => {
                let np = smooth.0 + (w - smooth.0) * (1.0 - smoothing);
                let npr = smooth.1 + (pressure - smooth.1) * (1.0 - smoothing * 0.5);
                *smooth = (np, npr);
                let off = match self.doc.get(*id).map(|l| &l.kind) {
                    Some(LayerKind::Vector(v)) => v.offset,
                    _ => Vec2::ZERO,
                };
                let l = np - off;
                let last = pts.last().copied().unwrap();
                if (pos2(last[0], last[1]) - l).length() >= 0.75 / zoom {
                    pts.push([l.x, l.y, radius_for(*base_r, npr, *use_p)]);
                    if let Some((c, _, _)) = self.rec_clock {
                        self.rec_times.push(c.elapsed().as_secs_f32());
                    }
                }
            }
            Drag::VErase { id, r } => {
                let id = *id;
                let rr = radius_for(*r, pressure, self.s.eraser_pressure);
                if let Some(LayerKind::Vector(v)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    let before = v.strokes.len();
                    let off = v.offset;
                    v.strokes.retain(|s| !s.hit(w - off, rr));
                    if v.strokes.len() != before {
                        self.changed = true;
                        self.modified = true;
                    }
                }
            }
        }
    }

    fn pointer_up(&mut self, sp: Pos2, mods: Modifiers) {
        self.pointer_move(sp, self.last_pressure(), mods);
        let d = self.drag.take();
        self.finish_drag(d, sp);
        if let Some(t) = self.restore_tool.take() {
            self.tool = t;
        }
    }

    fn finish_drag(&mut self, drag: Option<Drag>, sp: Pos2) {
        let Some(drag) = drag else { return };
        match drag {
            Drag::Marquee { start, cur, full } => {
                let r = Rect::from_two_pos(start, cur);
                if r.width().max(r.height()) * self.zoom < 4.0 {
                    if let Some(id) = self.hit_layer(start, 6.0 / self.zoom) {
                        self.toggle_selected(id);
                    }
                } else {
                    // photos, text, shapes: centre inside (Ctrl: entirely inside);
                    // pen layers: individual strokes by the same rule;
                    // brush layers: the pixels inside the box
                    let inside = |b: Rect| if full { r.contains_rect(b) } else { r.contains(b.center()) };
                    let mut ids = Vec::new();
                    let mut sub = SubSel::default();
                    for l in self.doc.layers.iter().rev().filter(|l| l.visible && !l.locked && l.mask_of.is_none()) {
                        match &l.kind {
                            LayerKind::Vector(v) => {
                                let idx: Vec<usize> = (0..v.strokes.len()).filter(|&i| inside(v.strokes[i].bounds.translate(v.offset))).collect();
                                if !idx.is_empty() {
                                    sub.strokes.push((l.id, idx));
                                }
                            }
                            LayerKind::Raster(rl) => {
                                if let Some(b) = rl.bounds() {
                                    let ir = r.intersect(b);
                                    if ir.width() >= 1.0 && ir.height() >= 1.0 {
                                        sub.pixels.push((l.id, vec![rect_poly(Rect::from_min_max(ir.min.floor(), ir.max.ceil()))]));
                                    }
                                }
                            }
                            _ => {
                                if l.bounds().is_some_and(inside) {
                                    ids.push(l.id);
                                }
                            }
                        }
                    }
                    let (ns, np) = (sub.strokes.iter().map(|x| x.1.len()).sum::<usize>(), sub.pixels.len());
                    self.sel = ids.first().copied();
                    self.extra_sel = ids.iter().skip(1).copied().collect();
                    self.sub = sub;
                    self.status = if ids.is_empty() && ns == 0 && np == 0 {
                        "영역 안에 선택할 것 없음".into()
                    } else {
                        let mut parts = Vec::new();
                        if !ids.is_empty() {
                            parts.push(format!("레이어 {}", ids.len()));
                        }
                        if ns > 0 {
                            parts.push(format!("펜 획 {ns}"));
                        }
                        if np > 0 {
                            parts.push(format!("브러시 영역 {np}"));
                        }
                        format!("선택: {}", parts.join(" · "))
                    };
                }
            }
            Drag::MoveMany { d, .. } => {
                self.remap_pixel_sel(Pos2::ZERO, 1.0, 0.0, d);
            }
            Drag::Lasso { pts } => {
                let b = Rect::from_points(&pts);
                if pts.len() < 3 || b.width().max(b.height()) * self.zoom < 4.0 {
                    self.select_only(None);
                    return;
                }
                // same rules as the box: centres inside; pen strokes one by one;
                // brush pixels inside the outline
                let mut ids = Vec::new();
                let mut sub = SubSel::default();
                for l in self.doc.layers.iter().rev().filter(|l| l.visible && !l.locked && l.mask_of.is_none()) {
                    match &l.kind {
                        LayerKind::Vector(v) => {
                            let idx: Vec<usize> = (0..v.strokes.len()).filter(|&i| in_poly(v.strokes[i].bounds.translate(v.offset).center(), &pts)).collect();
                            if !idx.is_empty() {
                                sub.strokes.push((l.id, idx));
                            }
                        }
                        LayerKind::Raster(rl) => {
                            if rl.bounds().is_some_and(|rb| rb.intersects(b)) {
                                sub.pixels.push((l.id, vec![pts.clone()]));
                            }
                        }
                        _ => {
                            if l.bounds().is_some_and(|lb| in_poly(lb.center(), &pts)) {
                                ids.push(l.id);
                            }
                        }
                    }
                }
                let (ns, np) = (sub.strokes.iter().map(|x| x.1.len()).sum::<usize>(), sub.pixels.len());
                self.sel = ids.first().copied();
                self.extra_sel = ids.iter().skip(1).copied().collect();
                self.sub = sub;
                self.status = format!("선택: 레이어 {} · 펜 획 {ns} · 브러시 영역 {np}", ids.len());
            }
            Drag::Group { center, s, th, .. } => {
                self.remap_pixel_sel(center, s, th, Vec2::ZERO);
            }
            Drag::Raster { id, stroke: Some(st), .. } => {
                let params = st.p;
                if let Some(LayerKind::Raster(r)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    st.finish(r);
                    if let Some((_, start, live)) = self.rec_clock.take() {
                        let rec = crate::raster::BrushRec { start, params, pts: std::mem::take(&mut self.rec_pts), times: std::mem::take(&mut self.rec_times) };
                        let end = rec.end();
                        r.recs.push(Arc::new(rec));
                        if live {
                            self.after_recorded(end);
                        }
                    }
                }
            }
            Drag::Vector { id, mut pts, color, smooth, base_r, pressure, .. } => {
                let wsp = self.xf().s2w(sp);
                if let Some(LayerKind::Vector(v)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    let l = wsp - v.offset;
                    let last = pts.last().copied().unwrap();
                    if (pos2(last[0], last[1]) - l).length() > 0.5 {
                        pts.push([l.x, l.y, radius_for(base_r, smooth.1, pressure)]);
                        if let Some((c, _, _)) = self.rec_clock {
                            self.rec_times.push(c.elapsed().as_secs_f32());
                        }
                    }
                    let mut recorded_end = None;
                    match self.rec_clock.take() {
                        Some((_, start, _)) => {
                            let st = VStroke::timed(pts, std::mem::take(&mut self.rec_times), color, start);
                            recorded_end = st.rec.as_ref().map(|r| r.end());
                            v.strokes.push(Arc::new(st));
                        }
                        None => v.strokes.push(Arc::new(VStroke::new(pts, color))),
                    }
                    if let Some(end) = recorded_end {
                        self.after_recorded(end);
                    }
                    self.changed = true;
                    self.modified = true;
                }
            }
            Drag::NewShape { id, .. } if self.mask_rect.is_some() => {
                let (mid, on) = self.mask_rect.take().unwrap();
                let rect = match self.doc.get(id).map(|l| &l.kind) {
                    Some(LayerKind::Shape(s)) => Some(Rect::from_two_pos(s.a, s.b)),
                    _ => None,
                };
                if let Some(i) = self.doc.index_of(id) {
                    self.doc.layers.remove(i);
                }
                if std::mem::take(&mut self.mask_cut) {
                    if let Some(rect) = rect {
                        self.grabcut_fill(mid, rect, on);
                    }
                } else if let (Some(rect), Some(LayerKind::Raster(m))) = (rect, self.doc.get_mut(mid).map(|l| &mut l.kind)) {
                    m.fill_rect(rect, on);
                }
                self.sel = Some(mid);
                self.changed = true;
                self.modified = true;
            }
            Drag::NewShape { id, .. } => {
                let tiny = matches!(self.doc.get(id).map(|l| &l.kind), Some(LayerKind::Shape(s)) if (s.b - s.a).length() * self.zoom < 3.0);
                if tiny {
                    if let Some(i) = self.doc.index_of(id) {
                        self.doc.layers.remove(i);
                        self.sel = None;
                    }
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::WallBase { sid, wid, start } => {
                let zoom = self.zoom;
                if let Some(sp) = self.doc.spaces.iter_mut().find(|s| s.id == sid) {
                    // a click or a base off the floor makes no wall
                    let tiny = sp.walls.iter().find(|x| x.id == wid).is_none_or(|wl| wl.corners[3].distance(wl.corners[2]) * zoom < 8.0 || wl.corners[3].distance(start) > 0.5);
                    if tiny {
                        sp.walls.retain(|x| x.id != wid);
                        self.wall_sel = None;
                    }
                }
            }
            Drag::WallRect { sid, wid, .. } => {
                let zoom = self.zoom;
                if let Some(sp) = self.doc.spaces.iter_mut().find(|s| s.id == sid) {
                    let tiny = sp.walls.iter().find(|x| x.id == wid).is_some_and(|wl| {
                        let rr = Rect::from_points(&wl.corners);
                        rr.width() * zoom < 8.0 || rr.height() * zoom < 8.0
                    });
                    if tiny {
                        sp.walls.retain(|x| x.id != wid);
                        self.wall_sel = None;
                    }
                }
            }
            Drag::SpaceTape { id } => {
                self.space_tape_draw = false;
                let zoom = self.zoom;
                if let Some(s) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                    if s.tape.is_some_and(|r| r.a.distance(r.b) * zoom < 6.0) {
                        s.tape = None;
                    }
                }
            }
            Drag::SpaceRef { id } => {
                self.space_ref_draw = false;
                let zoom = self.zoom;
                if let Some(s) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                    if s.ref_line.is_some_and(|r| r.a.distance(r.b) * zoom < 6.0) {
                        s.ref_line = None;
                    }
                }
            }
            Drag::SpaceMask { id, .. } => {
                self.space_mask_draw = false;
                let zoom = self.zoom;
                if let Some(s) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                    if s.mask.is_some_and(|m| m.width() * zoom < 6.0 || m.height() * zoom < 6.0) {
                        s.mask = None;
                    }
                }
            }
            Drag::SpaceLine { id, idx, .. } => {
                let zoom = self.zoom;
                if let Some(s) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                    if s.lines.get(idx).is_some_and(|l| l.a.distance(l.b) * zoom < 4.0) {
                        s.lines.remove(idx);
                        self.space_line = None;
                    }
                }
            }
            Drag::SpaceFrame { id, orig, .. } => {
                let fresh = orig.width() == 0.0 && orig.height() == 0.0;
                if let Some(i) = self.doc.spaces.iter().position(|s| s.id == id) {
                    let r = self.doc.spaces[i].frame;
                    if r.width() * self.zoom < 6.0 || r.height() * self.zoom < 6.0 {
                        if fresh {
                            // a click: fit the space to the photo under the pointer
                            let w = self.xf().s2w(sp);
                            let img = self.doc.layers.iter().rev().find_map(|l| match &l.kind {
                                LayerKind::Image(im) if l.visible && im.contains(w) => l.bounds().map(|b| (b, l.id, [im.center.x, im.center.y, im.scale, im.rotation])),
                                _ => None,
                            });
                            match img {
                                Some((b, lid, last)) => {
                                    self.doc.spaces[i].frame = b;
                                    self.doc.spaces[i].follow = Some(crate::persp::SpaceFollow { layer: lid, last });
                                }
                                None => {
                                    self.doc.spaces.remove(i);
                                    self.space_sel = self.doc.spaces.last().map(|s| s.id);
                                    return;
                                }
                            }
                        } else {
                            self.doc.spaces[i].frame = orig;
                        }
                    }
                    if fresh {
                        let f = self.doc.spaces[i].frame;
                        self.doc.spaces[i].origin = pos2(f.center().x, f.top() + f.height() * 0.7);
                    }
                }
            }
            Drag::Frame { .. } => {
                if let Some(r) = self.doc.export_frame {
                    if r.width() < 2.0 || r.height() < 2.0 {
                        self.doc.export_frame = None;
                    }
                }
            }
            _ => {}
        }
    }

    fn last_pressure(&self) -> f32 {
        match &self.drag {
            Some(Drag::Raster { smooth, .. }) | Some(Drag::Vector { smooth, .. }) => smooth.1,
            _ => 1.0,
        }
    }

    // ------------------------------------------------------------ input

    fn handle_canvas_input(&mut self, ctx: &egui::Context, rect: Rect, canvas_layer: egui::LayerId) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let typing = ctx.text_edit_focused();
        // Track Space ourselves: the Korean IME can swallow the key-up of a Space typed
        // into a text box, which left egui thinking Space was still held (every click
        // then panned until Space was pressed again).
        if typing {
            self.space_held = false;
        }
        for e in &events {
            match e {
                egui::Event::Key { key: Key::Space, pressed, repeat: false, .. } if !typing => self.space_held = *pressed,
                egui::Event::WindowFocused(false) => self.space_held = false,
                _ => {}
            }
        }
        let space = self.space_held;
        let over = |p: Pos2| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == canvas_layer);
        let touch_frame = events.iter().any(|e| matches!(e, egui::Event::Touch { .. }));
        let now = ctx.input(|i| i.time);
        let pen = crate::penhook::state();
        if self.show_log {
            let tag = format!("pen flags {:X}/{:X}", pen.raw_pen, pen.raw_pointer);
            if pen.in_range && self.input_log.front().is_none_or(|f| *f != tag) && (pen.barrel || pen.eraser) {
                self.log_input(tag);
            }
        }

        // Pen side button: pan (Shift: zoom) while hovering or touching.
        if pen.barrel && pen.in_range && self.drag.is_none() {
            if let Some(hp) = ctx.input(|i| i.pointer.hover_pos()).filter(|p| over(*p)) {
                self.source = Source::PenHover;
                self.last_screen = hp;
                self.drag = Some(if mods.shift { Drag::Zoom { anchor: hp } } else { Drag::Pan });
            }
        }
        if (!pen.barrel || !pen.in_range) && self.source == Source::PenHover && self.drag.is_some() {
            self.drag = None;
        }

        for ev in &events {
            match ev {
                egui::Event::Touch { id, phase, pos, force, .. } => match phase {
                    TouchPhase::Start => {
                        self.log_input(format!("touch start {:?} force {:?} pen {:X}/{:X}", id.0, force, pen.raw_pen, pen.raw_pointer));
                        self.touches.insert(*id, *pos);
                        if self.touches.len() >= 2 {
                            if self.drag.is_some() && matches!(self.source, Source::Touch(_)) {
                                if now - self.drag_t0 < 0.4 {
                                    self.cancel_drag();
                                } else {
                                    self.pointer_up(self.last_screen, mods);
                                }
                            }
                            continue;
                        }
                        if self.drag.is_none() && over(*pos) {
                            self.source = Source::Touch(*id);
                            self.drag_t0 = now;
                            if force.is_some() && pen.eraser && self.tool != Tool::Eraser {
                                self.restore_tool = Some(self.tool);
                                self.tool = Tool::Eraser;
                                self.pointer_down(*pos, force.unwrap_or(1.0), mods, space);
                                if self.drag.is_none() {
                                    if let Some(t) = self.restore_tool.take() {
                                        self.tool = t;
                                    }
                                }
                            } else if force.is_some() || self.s.touch_draw {
                                self.pointer_down(*pos, force.unwrap_or(1.0), mods, space);
                            } else {
                                self.last_screen = *pos;
                                self.drag = Some(Drag::Pan);
                            }
                        }
                    }
                    TouchPhase::Move => {
                        if matches!(self.source, Source::PenHover | Source::Middle) && self.drag.is_some() {
                            if pos.distance(self.last_screen) > 0.5 {
                                self.middle_moved = true;
                            }
                            self.pointer_move(*pos, 1.0, mods);
                        } else if self.touches.contains_key(id) {
                            self.touches.insert(*id, *pos);
                            if self.source == Source::Touch(*id) && self.drag.is_some() && self.touches.len() == 1 {
                                self.pointer_move(*pos, force.unwrap_or(self.last_pressure()), mods);
                            }
                        }
                    }
                    TouchPhase::End | TouchPhase::Cancel => {
                        self.log_input(format!("touch end {:?}", id.0));
                        self.touches.remove(id);
                        if self.source == Source::Touch(*id) && self.drag.is_some() {
                            self.pointer_up(*pos, mods);
                        }
                    }
                },
                egui::Event::PointerButton { pos, button: PointerButton::Middle, pressed, modifiers } => {
                    self.log_input(format!("mouse middle {}", if *pressed { "down" } else { "up" }));
                    if *pressed {
                        if self.pan_latch {
                            self.pan_latch = false;
                            if self.source == Source::Middle {
                                self.drag = None;
                            }
                            self.status = "펜 버튼 이동 끔".into();
                        } else if self.drag.is_none() && over(*pos) {
                            self.source = Source::Middle;
                            self.drag_t0 = now;
                            self.middle_moved = false;
                            self.last_screen = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(*pos);
                            let a = self.last_screen;
                            self.drag = Some(if modifiers.shift { Drag::Zoom { anchor: a } } else { Drag::Pan });
                        }
                    } else if self.source == Source::Middle && self.drag.is_some() && !self.pan_latch {
                        // Some tablet drivers send the pen button as an instant click; then a
                        // click toggles "pan while hovering" instead of needing a held drag.
                        let quick = now - self.drag_t0 < 0.25 && !self.middle_moved;
                        if quick && (touch_frame || pen.in_range || !self.touches.is_empty()) {
                            self.pan_latch = true;
                            self.status = "펜 버튼 이동 켬 · 다시 누르면 끔".into();
                        } else {
                            self.drag = None;
                        }
                    }
                }
                egui::Event::PointerMoved(p) if self.source == Source::Middle && self.drag.is_some() => {
                    if p.distance(self.last_screen) > 0.5 {
                        self.middle_moved = true;
                    }
                    self.pointer_move(*p, 1.0, mods);
                }
                egui::Event::PointerMoved(p) if !touch_frame && self.touches.is_empty() => {
                    if self.source == Source::Mouse && self.drag.is_some() {
                        self.pointer_move(*p, 1.0, mods);
                    }
                }
                egui::Event::PointerButton { button, pressed, .. } if touch_frame || !self.touches.is_empty() => {
                    self.log_input(format!("mouse {button:?} {} (with pen)", if *pressed { "down" } else { "up" }));
                }
                egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                    self.log_input(format!("mouse {button:?} {}", if *pressed { "down" } else { "up" }));
                    if *pressed && self.drag.is_none() && over(*pos) {
                        self.source = Source::Mouse;
                        self.drag_t0 = now;
                        match button {
                            PointerButton::Middle => {
                                self.last_screen = *pos;
                                self.drag = Some(if modifiers.shift { Drag::Zoom { anchor: *pos } } else { Drag::Pan });
                            }
                            PointerButton::Primary => self.pointer_down(*pos, 1.0, *modifiers, space),
                            _ => {}
                        }
                    } else if !*pressed && self.source == Source::Mouse && self.drag.is_some() {
                        self.pointer_up(*pos, *modifiers);
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    if let Some(hp) = ctx.input(|i| i.pointer.hover_pos()) {
                        if over(hp) {
                            let dy = match unit {
                                egui::MouseWheelUnit::Point => delta.y,
                                egui::MouseWheelUnit::Line => delta.y * 40.0,
                                egui::MouseWheelUnit::Page => delta.y * 400.0,
                            };
                            self.zoom_at(hp, (dy * 0.0025).exp());
                        }
                    }
                }
                egui::Event::Zoom(z) => {
                    if let Some(hp) = ctx.input(|i| i.pointer.hover_pos()) {
                        if over(hp) {
                            self.zoom_at(hp, *z);
                        }
                    }
                }
                _ => {}
            }
        }

        if self.touches.len() >= 2 {
            if let Some(mt) = ctx.input(|i| i.multi_touch()) {
                self.zoom_at(mt.center_pos, mt.zoom_delta);
                self.pan += mt.translation_delta;
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.text_edit_focused();
        let cmd = Modifiers::COMMAND;
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        let mut paste = false;
        ctx.input(|i| {
            for e in &i.events {
                if let egui::Event::Key { key: Key::V, pressed: false, modifiers, .. } = e {
                    if modifiers.command {
                        paste = true;
                    }
                }
            }
        });
        if typing {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.finish_text_edit();
            }
            return;
        }
        if paste {
            self.paste();
        }
        let tabs = std::mem::take(&mut self.tab_raw);
        let can3d = self.ws3d || self.ws_space().is_some();
        // Tab only switches workspaces now; hiding the UI moved to Ctrl+H
        for (pressed, shift) in tabs {
            if shift {
                continue;
            }
            if !can3d {
                if pressed {
                    self.status = "3D 공간 없음".into();
                }
                continue;
            }
            if pressed {
                self.tab_t0 = Some(self.now);
            } else {
                if self.peek {
                    self.peek = false;
                } else if self.tab_t0.is_some() {
                    self.toggle_ws();
                }
                self.tab_t0 = None;
            }
        }
        if let Some(t0) = self.tab_t0 {
            if self.now - t0 > self.s.peek_delay as f64 {
                self.peek = true;
            }
            ctx.request_repaint();
        }
        if self.modal3d.is_some() || self.grs2d.is_some() {
            return;
        }
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        if k(Modifiers::COMMAND, Key::H) {
            self.ui_hidden = !self.ui_hidden;
        }
        if k(cs, Key::Z) || k(cmd, Key::Y) {
            self.redo();
        }
        if k(cmd, Key::Z) {
            self.undo();
        }
        if k(cs, Key::S) {
            self.save(true);
        }
        if k(cmd, Key::S) {
            self.save(false);
        }
        if k(cmd, Key::O) {
            self.request(Pending::Open(None));
        }
        if k(cmd, Key::N) {
            self.request(Pending::New);
        }
        if k(cmd, Key::I) {
            self.import_dialog();
        }
        if k(cs, Key::E) {
            self.open_video_export();
        }
        if k(cmd, Key::E) {
            self.open_export();
        }
        // timeline (also in the 3D workspace)
        if k(cmd, Key::T) {
            self.doc.timeline.on = !self.doc.timeline.on;
            if !self.doc.timeline.on && self.tl_play.take().is_some() {
                self.playing.clear();
            }
            self.status = if self.doc.timeline.on { "타임라인 켬 · Alt+A 재생 · I 키 넣기 · , . 프레임".into() } else { "타임라인 끔".into() };
        }
        if self.doc.timeline.on {
            if k(Modifiers::ALT, Key::A) {
                self.tl_toggle_play();
            }
            if k(Modifiers::NONE, Key::I) {
                self.insert_keys();
            }
            if k(Modifiers::ALT, Key::I) {
                self.delete_keys_here();
            }
            let fl = self.doc.timeline.frame_len();
            let t = self.doc.timeline.time;
            if k(Modifiers::NONE, Key::Comma) {
                self.set_time(t - fl);
            }
            if k(Modifiers::NONE, Key::Period) {
                self.set_time(t + fl);
            }
            if k(Modifiers::SHIFT, Key::Comma) {
                self.set_time(0.0);
            }
            if k(Modifiers::SHIFT, Key::Period) {
                self.set_time(self.doc.timeline.duration);
            }
        }
        if self.scissor.is_some() && k(Modifiers::NONE, Key::Escape) {
            self.scissor = None;
        } else if self.wand.is_some() && k(Modifiers::NONE, Key::Escape) {
            self.wand = None;
        }
        if self.scissor.is_some() && (k(Modifiers::NONE, Key::Enter) || k(Modifiers::SHIFT, Key::Enter)) {
            let refine = k(Modifiers::SHIFT, Key::Enter);
            self.scissor_close(refine);
        }
        if self.doc.timeline.on && !self.tl_sel.is_empty() && (k(Modifiers::NONE, Key::Delete) || k(Modifiers::NONE, Key::Backspace)) {
            self.delete_selected_keys();
        }
        if self.ws3d {
            // the 3D workspace handles its own keys (G/R/S, views, Esc)
            return;
        }
        if !self.doc.timeline.on {
            if let Some(id) = self.sel.filter(|id| self.video_of(*id).is_some()) {
                if k(Modifiers::NONE, Key::Comma) {
                    self.video_step(id, -1);
                }
                if k(Modifiers::NONE, Key::Period) {
                    self.video_step(id, 1);
                }
            }
        }
        // G/R/S on a layer in a space, right on the canvas (before R picks the
        // rectangle tool)
        if self.tool == Tool::Select && self.drag.is_none() && self.grs2d.is_none() {
            let placed = self.sel_layer().filter(|_| self.extra_sel.is_empty()).and_then(|l| l.place.clone().map(|p| (l.id, p)));
            if placed.is_none() {
                use crate::view3d::Op;
                for (key, op) in [(Key::G, Op::Grab), (Key::R, Op::Rotate), (Key::S, Op::Scale)] {
                    let origs = self.sel_parts();
                    if origs.is_empty() {
                        break;
                    }
                    if k(Modifiers::NONE, key) {
                        let center = self.group_box().or_else(|| self.sel_layer().and_then(|l| l.bounds())).map(|b| b.center());
                        if let (Some(center), Some(hp)) = (center, ctx.input(|i| i.pointer.hover_pos())) {
                            self.grs2d = Some(Grs2d { op, origs, start: hp, center, axis: None, typed: String::new(), s: 1.0, th: 0.0, d: Vec2::ZERO });
                        }
                        break;
                    }
                }
            }
            if let Some((id, pl)) = placed {
                use crate::view3d::Op;
                for (key, op) in [(Key::G, Op::Grab), (Key::R, Op::Rotate), (Key::S, Op::Scale)] {
                    if k(Modifiers::ALT, key) {
                        if let Some(p) = self.doc.get_mut(id).and_then(|l| l.place.as_mut()) {
                            match op {
                                Op::Grab => p.pos = [0.0; 3],
                                Op::Rotate => p.rot = [0.0; 3],
                                Op::Scale => p.scale = 1.0,
                            }
                        }
                        self.mark();
                    } else if k(Modifiers::NONE, key) {
                        if let (Some(proj), Some(hp)) = (self.canvas_proj(pl.space), ctx.input(|i| i.pointer.hover_pos())) {
                            self.modal3d = Some(crate::view3d::Modal::new(op, id, pl.clone(), hp, &proj));
                        }
                    }
                }
            }
        }
        if k(cs, Key::C) {
            self.quick_copy();
        }
        if k(cmd, Key::D) {
            self.duplicate_selected();
        }
        if k(cmd, Key::Num0) {
            self.fit_view();
        }
        if k(cmd, Key::Num1) {
            self.set_zoom_center(1.0);
        }
        if self.tool == Tool::Space {
            for (key, ax) in [(Key::Num1, 0u8), (Key::Num2, 1), (Key::Num3, 2), (Key::X, 0), (Key::Y, 1), (Key::Z, 2)] {
                if k(Modifiers::NONE, key) {
                    self.set_space_axis(ax);
                }
            }
            if let (Some(i), Some(li)) = (self.space_idx(), self.space_line) {
                if k(Modifiers::NONE, Key::Delete) || k(Modifiers::NONE, Key::Backspace) {
                    if li < self.doc.spaces[i].lines.len() {
                        self.doc.spaces[i].lines.remove(li);
                        self.mark();
                    }
                    self.space_line = None;
                }
            }
        }
        if k(Modifiers::NONE, Key::Delete) || k(Modifiers::NONE, Key::Backspace) {
            self.delete_selected();
        }
        if k(Modifiers::NONE, Key::F12) {
            self.show_log = !self.show_log;
        }
        if k(Modifiers::NONE, Key::Escape) {
            if self.pan_latch {
                self.pan_latch = false;
                if self.source == Source::Middle {
                    self.drag = None;
                }
            } else if self.palette.is_some() {
                self.palette = None;
            } else if self.drag.is_some() {
                self.cancel_drag();
            } else if self.tool == Tool::Crop {
                self.tool = Tool::Select;
            } else if self.tool == Tool::Space && (self.space_line.is_some() || self.space_new) {
                self.space_line = None;
                self.space_new = false;
            } else {
                self.select_only(None);
            }
        }
        if k(Modifiers::NONE, Key::Enter) && self.tool == Tool::Crop {
            self.tool = Tool::Select;
        }
        for &(t, _, _, key) in TOOLS {
            if k(Modifiers::NONE, key) {
                self.set_tool(t);
            }
        }
        if k(Modifiers::NONE, Key::OpenBracket) {
            self.scale_tool_size(1.0 / 1.2);
        }
        if k(Modifiers::NONE, Key::CloseBracket) {
            self.scale_tool_size(1.2);
        }
        let mut nudge = Vec2::ZERO;
        for (key, d) in [(Key::ArrowLeft, vec2(-1.0, 0.0)), (Key::ArrowRight, vec2(1.0, 0.0)), (Key::ArrowUp, vec2(0.0, -1.0)), (Key::ArrowDown, vec2(0.0, 1.0))] {
            if k(Modifiers::NONE, key) {
                nudge += d;
            }
            if k(Modifiers::SHIFT, key) {
                nudge += d * 10.0;
            }
        }
        if nudge != Vec2::ZERO {
            for (o, part) in self.sel_parts() {
                let l = apply_part(&o, &part, Pos2::ZERO, 1.0, 0.0, nudge);
                if let Some(i) = self.doc.index_of(l.id) {
                    self.doc.layers[i] = l;
                    self.changed = true;
                    self.modified = true;
                }
            }
            self.remap_pixel_sel(Pos2::ZERO, 1.0, 0.0, nudge);
        }
    }

    fn tool_size_mut(&mut self) -> Option<&mut f32> {
        Some(match self.tool {
            Tool::Brush => &mut self.s.brush_size,
            Tool::Pen => &mut self.s.pen_size,
            Tool::Eraser => &mut self.s.eraser_size,
            Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse => &mut self.s.shape_width,
            Tool::Text => &mut self.s.text_size,
            _ => return None,
        })
    }

    fn tool_size(&mut self) -> Option<f32> {
        self.tool_size_mut().map(|v| *v)
    }

    fn scale_tool_size(&mut self, f: f32) {
        if let Some(v) = self.tool_size_mut() {
            *v = (*v * f).clamp(0.5, 1000.0);
        }
    }

    // ------------------------------------------------------------ export

    fn open_export(&mut self) {
        self.finish_text_edit();
        if self.export.is_none() {
            let mut d = self.last_export.clone().unwrap_or(ExportDlg {
                region: ExportRegion::Content,
                padding: 20.0,
                scale: 1.0,
                bg: ExportBg::White,
                bg_color: Color32::from_rgb(250, 247, 240),
                jpeg_quality: 92,
                ss: 1,
            });
            if self.doc.export_frame.is_some() && self.last_export.is_none() {
                d.region = ExportRegion::Frame;
            }
            if self.doc.export_frame.is_none() && d.region == ExportRegion::Frame {
                d.region = ExportRegion::Content;
            }
            self.export = Some(d);
        }
    }

    fn export_region(&self, d: &ExportDlg) -> Option<Rect> {
        match d.region {
            ExportRegion::Camera(id) => self.doc.cameras.iter().find(|c| c.id == id).map(|c| c.rect),
            ExportRegion::Frame => self.doc.export_frame,
            ExportRegion::Content => self.doc.content_bounds().map(|b| b.expand(d.padding)),
            ExportRegion::View => {
                let xf = self.xf();
                Some(Rect::from_min_max(xf.s2w(self.canvas_rect.min), xf.s2w(self.canvas_rect.max)))
            }
        }
        .map(|r| Rect::from_min_max(r.min.floor(), r.max.ceil()))
    }

    fn quick_copy(&mut self) {
        let region = self.doc.export_frame.or_else(|| self.doc.content_bounds().map(|b| b.expand(20.0)));
        match region {
            Some(r) => self.copy_region(r, 1.0, Some(Color32::WHITE)),
            None => self.status = "복사할 내용 없음".into(),
        }
    }

    fn copy_region(&mut self, r: Rect, scale: f32, bg: Option<Color32>) {
        match crate::export::render(&self.export_doc(), r, scale, bg) {
            Ok(pm) => {
                let img = crate::export::to_rgba(&pm);
                let res = arboard::Clipboard::new().and_then(|mut cb| {
                    cb.set_image(arboard::ImageData {
                        width: img.width() as usize,
                        height: img.height() as usize,
                        bytes: std::borrow::Cow::Owned(img.into_raw()),
                    })
                });
                self.status = match res {
                    Ok(()) => format!("클립보드에 복사됨 ({}×{})", pm.width(), pm.height()),
                    Err(e) => format!("복사 실패: {e}"),
                };
            }
            Err(e) => self.status = e,
        }
    }

    fn export_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.export.take() else { return };
        let mut open = true;
        let mut action: Option<u8> = None;
        let mut pal_req: PalReq = None;
        egui::Window::new("내보내기")
            .title_bar(false)
            .frame(egui::Frame::NONE)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(
                // DV_EXPORT_AT="x,y" places it for screenshots (e.g. over the inspector)
                std::env::var("DV_EXPORT_AT")
                    .ok()
                    .and_then(|v| v.split_once(',').and_then(|(x, y)| Some(pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))))
                    .unwrap_or(ctx.content_rect().center()),
            )
            .show(ctx, |ui| theme::glass(ui, egui::Margin::same(SPACE_LG as i8), theme::RADIUS_CARD, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("내보내기").font(FontId::new(15.0, theme::ui_bold())).color(INK()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("닫기").clicked() {
                        open = false;
                    }
                });
            });
            ui.add_space(SPACE_SM);
            ui.label("영역");
            ui.horizontal(|ui| {
                ui.add_enabled_ui(self.doc.export_frame.is_some(), |ui| {
                    ui.radio_value(&mut d.region, ExportRegion::Frame, "내보낼 영역 (F)");
                });
                ui.radio_value(&mut d.region, ExportRegion::Content, "내용 전체");
                ui.radio_value(&mut d.region, ExportRegion::View, "지금 화면");
            });
            if !self.doc.cameras.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("카메라");
                    for c in &self.doc.cameras {
                        ui.radio_value(&mut d.region, ExportRegion::Camera(c.id), c.name.as_str());
                    }
                });
            }
            if let ExportRegion::Camera(id) = d.region {
                if !self.doc.cameras.iter().any(|c| c.id == id) {
                    d.region = ExportRegion::Content;
                }
            }
            if d.region == ExportRegion::Frame && self.doc.export_frame.is_none() {
                d.region = ExportRegion::Content;
            }
            if d.region == ExportRegion::Content {
                ui.add(crate::dial::Dial::new(&mut d.padding, 0.0..=400.0).text("여백").suffix(" px").default(20.0));
            }
            ui.horizontal(|ui| {
                ui.label("배율");
                for s in [0.25f32, 0.5, 1.0, 2.0] {
                    if ui.selectable_label((d.scale - s).abs() < 1e-4, format!("{}×", s)).clicked() {
                        d.scale = s;
                    }
                }
                ui.add(crate::calc::drag(&mut d.scale).custom_parser(crate::calc::parse).range(0.05..=8.0).speed(0.01).suffix("×"));
            });
            ui.horizontal(|ui| {
                ui.label("배경");
                ui.radio_value(&mut d.bg, ExportBg::Transparent, "투명");
                ui.radio_value(&mut d.bg, ExportBg::White, "흰색");
                ui.radio_value(&mut d.bg, ExportBg::Custom, "색");
                if d.bg == ExportBg::Custom {
                    color_row(ui, &mut d.bg_color, PalTarget::ExportBg, &mut pal_req);
                }
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label("슈퍼샘플링").on_hover_text("크게 그린 뒤 줄여서 저장 · 경계와 두께 줄무늬가 부드러워짐");
                for k in 1..=4u8 {
                    if ui.selectable_label(d.ss == k, format!("{k}×")).clicked() {
                        d.ss = k;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("JPG 품질");
                crate::dial::dial_u8(ui, &mut d.jpeg_quality, 50..=100, "");
            });
            let region = self.export_region(&d);
            match region {
                Some(r) => {
                    let (w, h) = crate::export::output_size(r, d.scale);
                    ui.label(format!("결과 크기: {w} × {h} px"));
                }
                None => {
                    ui.label("내보낼 내용 없음");
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add_enabled(region.is_some(), egui::Button::new("파일로 저장…")).clicked() {
                    action = Some(1);
                }
                if ui.add_enabled(region.is_some(), egui::Button::new("클립보드로 복사")).clicked() {
                    action = Some(2);
                }
            });
            ui.small("PNG: 투명 배경 유지 · JPG: 배경색으로 채움");
        }));
        let bg = match d.bg {
            ExportBg::Transparent => None,
            ExportBg::White => Some(Color32::WHITE),
            ExportBg::Custom => Some(d.bg_color),
        };
        if let (Some(a), Some(r)) = (action, self.export_region(&d)) {
            if a == 1 {
                let base = self.path.as_ref().and_then(|p| p.file_stem()).and_then(|s| s.to_str()).unwrap_or("collage").to_string();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PNG", &["png"])
                    .add_filter("JPEG", &["jpg", "jpeg"])
                    .set_file_name(format!("{base}.png"))
                    .save_file()
                {
                    let is_jpg = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
                    let bg = if is_jpg { bg.or(Some(Color32::WHITE)) } else { bg };
                    let rot = match d.region {
                        ExportRegion::Camera(id) => self.doc.cameras.iter().find(|c| c.id == id).map_or(0.0, |c| c.rot),
                        _ => 0.0,
                    };
                    // on a worker thread: big supersampled renders take a while
                    let doc = self.export_doc();
                    let (scale, ss, q) = (d.scale, d.ss, d.jpeg_quality);
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let res = crate::export::render_ss(&doc, r.center(), r.size(), rot, scale, bg, ss)
                            .and_then(|pm| crate::export::save(&pm, &path, q).map(|_| (pm.width(), pm.height())))
                            .map(|(w, h)| format!("내보냄: {} ({w}×{h})", path.file_name().and_then(|n| n.to_str()).unwrap_or("")))
                            .unwrap_or_else(|e| format!("내보내기 실패: {e}"));
                        let _ = tx.send(res);
                    });
                    self.img_job = Some(rx);
                    self.status = "이미지 내보내는 중…".into();
                }
            } else {
                self.copy_region(r, d.scale, bg);
            }
        }
        self.last_export = Some(d.clone());
        if open {
            self.export = Some(d);
        }
        if let Some((t, c, at)) = pal_req {
            self.open_palette_for(t, c, at);
        }
    }

    // ------------------------------------------------------------ UI panels

    fn tool_button(&mut self, ui: &mut egui::Ui, t: Tool) {
        let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
        let active = self.tool == t;
        let under_glass = crate::glass::available();
        if active {
            self.rail_active = Some(r);
        }
        let p = ui.painter();
        if active && !under_glass {
            p.rect_filled(r, 9.0, ACCENT);
        } else if resp.hovered() && !active {
            p.rect_filled(r, 9.0, FILL_2());
        }
        // with glass the amber indicator is drawn in the canvas overlay layer under the
        // rail (see `draw_rail_indicator`): blurred by the glass, but not tinted or split
        let col = if active {
            if under_glass { Color32::WHITE } else { ACCENT_INK }
        } else if resp.hovered() {
            INK()
        } else {
            INK_2()
        };
        theme::tool_icon(p, t, Rect::from_center_size(r.center(), theme::ICON), col);
        let (name, key) = TOOLS.iter().find(|x| x.0 == t).map(|x| (x.1, x.2)).unwrap_or(("", ""));
        if resp.on_hover_text(format!("{name}  ·  {key}")).clicked() {
            self.set_tool(t);
        }
    }

    fn tool_rail(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("rail"))
            .order(egui::Order::Middle)
            .fixed_pos(pos2(screen.left() + SPACE_LG, screen.top() + 76.0))
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::same(6), theme::RADIUS_CARD, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                    let groups: [&[Tool]; 4] = [
                        &[Tool::Select, Tool::Lasso, Tool::Hand],
                        &[Tool::Brush, Tool::Pen, Tool::Eraser],
                        &[Tool::Text, Tool::Line, Tool::Arrow, Tool::Rect, Tool::Ellipse],
                        &[Tool::Crop, Tool::Frame, Tool::Space, Tool::Picker],
                    ];
                    for (gi, g) in groups.iter().enumerate() {
                        if gi > 0 {
                            let (r, _) = ui.allocate_exact_size(vec2(36.0, 9.0), Sense::hover());
                            ui.painter().line_segment([r.left_center() + vec2(10.0, 0.0), r.right_center() - vec2(10.0, 0.0)], Stroke::new(1.0, FILL_2()));
                        }
                        for &t in g.iter() {
                            self.tool_button(ui, t);
                        }
                    }
                    ui.add_space(8.0);
                    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                    let p = ui.painter();
                    p.circle_filled(r.center(), 14.0, FILL_2());
                    p.circle_filled(r.center(), 11.0, self.s.color);
                    if resp.hovered() {
                        p.circle_stroke(r.center(), 14.0, Stroke::new(1.0, INK_2()));
                    }
                    if resp.on_hover_text("색  ·  캔버스에서 Alt+클릭").clicked() {
                        self.open_palette(r.right_top() + vec2(10.0, -150.0));
                    }
                });
            });
    }

    /// Applies the picked colour to a selected text/shape layer, like most editors.
    fn recolor_selected(&mut self) {
        let c = self.s.color;
        if let Some(id) = self.sel {
            if let Some(l) = self.doc.get_mut(id) {
                match &mut l.kind {
                    LayerKind::Text(t) if t.color != c => {
                        t.color = c;
                        self.mark();
                    }
                    LayerKind::Shape(s) if s.color != c => {
                        s.color = c;
                        self.mark();
                    }
                    _ => {}
                }
            }
        }
    }

    fn tool_options(&mut self, ui: &mut egui::Ui) {
        // Text and shape tools show one set of style controls: they edit the selected
        // layer of that kind (and become the defaults for new ones), or just the defaults.
        let sel_idx = self.sel.and_then(|id| self.doc.index_of(id)).filter(|_| self.extra_sel.is_empty());
        let sel_text = sel_idx.filter(|&i| matches!(self.doc.layers[i].kind, LayerKind::Text(_)) && self.tool == Tool::Text);
        let sel_shape = sel_idx.filter(|&i| {
            matches!(self.doc.layers[i].kind, LayerKind::Shape(_))
                && matches!(self.tool, Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse)
        });
        let mut layer_changed = false;
        let mut pal_req: PalReq = None;
        let lid = sel_text.or(sel_shape).map(|i| self.doc.layers[i].id).unwrap_or(0);
        let s = &mut self.s;
        match self.tool {
            Tool::Brush => {
                ui.strong("브러시");
                // presets: (name, size, opacity, hardness, pressure->size, pressure->opacity, smoothing)
                const PRESETS: [(&str, f32, f32, f32, bool, bool, f32); 5] = [
                    ("연필", 3.0, 0.9, 0.95, true, false, 0.2),
                    ("펜촉", 6.0, 1.0, 1.0, true, false, 0.45),
                    ("마커", 18.0, 0.55, 0.7, false, false, 0.3),
                    ("에어브러시", 60.0, 0.25, 0.0, false, true, 0.2),
                    ("수채", 34.0, 0.4, 0.2, true, true, 0.35),
                ];
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for (name, size, op, hard, ps, po, sm) in PRESETS {
                        if ui.selectable_label(s.brush_preset == name, name).clicked() {
                            s.brush_preset = name.to_string();
                            s.brush_size = size;
                            s.brush_opacity = op;
                            s.brush_hardness = hard;
                            s.brush_pressure_size = ps;
                            s.brush_pressure_opacity = po;
                            s.smoothing = sm;
                        }
                    }
                });
                // reset targets: the picked preset's values, else the app defaults
                let base = Settings::default();
                let (d_size, d_op, d_hard, d_sm) = PRESETS
                    .iter()
                    .find(|p| p.0 == s.brush_preset)
                    .map(|p| (p.1, p.2, p.3, p.6))
                    .unwrap_or((base.brush_size, base.brush_opacity, base.brush_hardness, base.smoothing));
                ui.add(crate::dial::Dial::new(&mut s.brush_size, 1.0..=400.0).step(2.0).text("크기").default(d_size));
                ui.add(crate::dial::Dial::new(&mut s.brush_opacity, 0.05..=1.0).text("불투명도").default(d_op));
                ui.add(crate::dial::Dial::new(&mut s.brush_hardness, 0.0..=1.0).text("경도").default(d_hard));
                ui.checkbox(&mut s.brush_pressure_size, "필압 → 크기");
                ui.checkbox(&mut s.brush_pressure_opacity, "필압 → 농도");
                ui.add(crate::dial::Dial::new(&mut s.smoothing, 0.0..=0.9).text("손떨림 보정").default(d_sm));
                ui.small("Ctrl+드래그: 크기 · Alt+클릭: 팔레트 · [ ]: 크기");
            }
            Tool::Pen => {
                ui.strong("펜 (벡터)");
                ui.add(crate::dial::Dial::new(&mut s.pen_size, 0.5..=100.0).step(0.5).text("굵기"));
                ui.checkbox(&mut s.pen_pressure, "필압 → 굵기");
                ui.add(crate::dial::Dial::new(&mut s.smoothing, 0.0..=0.9).text("손떨림 보정"));
                ui.small("획 단위로 저장 · 지우개는 한 획씩 지움 · Ctrl+드래그: 굵기");
            }
            Tool::Eraser => {
                ui.strong("지우개");
                ui.add(crate::dial::Dial::new(&mut s.eraser_size, 2.0..=400.0).step(2.0).text("크기"));
                ui.checkbox(&mut s.eraser_pressure, "필압 → 크기");
                ui.small("브러시 레이어: 픽셀 지움 · 펜 레이어: 닿은 획 지움 · Ctrl+드래그: 크기");
            }
            Tool::Text => {
                ui.strong("글자");
                if let Some(LayerKind::Text(t)) = sel_text.map(|i| &mut self.doc.layers[i].kind) {
                    let s0 = crate::text::layout(t).size;
                    layer_changed |= ui.add(crate::dial::Dial::new(&mut t.size, 6.0..=400.0).step(2.0).text("크기")).changed();
                    layer_changed |= font_picker(ui, "tool_font", &mut t.family, &mut t.style, &mut self.font_ui);
                    layer_changed |= align_row(ui, &mut t.align);
                    ui.horizontal(|ui| {
                        let mut has = t.bg.is_some();
                        if ui.checkbox(&mut has, "배경 상자").changed() {
                            t.bg = has.then_some(s.text_bg_color);
                            layer_changed = true;
                        }
                        if let Some(bg) = &mut t.bg {
                            layer_changed |= color_row(ui, bg, PalTarget::TextBg(lid), &mut pal_req);
                        }
                    });
                    if layer_changed {
                        t.keep_corner(s0);
                    }
                    // the next new text starts from the same style
                    s.text_size = t.size;
                    s.text_family = t.family.clone();
                    s.text_style = t.style.clone();
                    s.text_align = t.align;
                    s.text_bg = t.bg.is_some();
                    if let Some(c) = t.bg {
                        s.text_bg_color = c;
                    }
                    ui.small("선택한 글자에 적용 · 새 글자도 이 설정으로 시작");
                } else {
                    ui.add(crate::dial::Dial::new(&mut s.text_size, 6.0..=400.0).step(2.0).text("크기"));
                    font_picker(ui, "tool_font", &mut s.text_family, &mut s.text_style, &mut self.font_ui);
                    align_row(ui, &mut s.text_align);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut s.text_bg, "배경 상자");
                        color_row(ui, &mut s.text_bg_color, PalTarget::DefaultTextBg, &mut pal_req);
                    });
                }
                ui.small("빈 곳 클릭: 새 글 · 글 클릭: 수정 · Esc: 끝내기");
            }
            Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse => {
                ui.strong("도형");
                if let Some(LayerKind::Shape(sh)) = sel_shape.map(|i| &mut self.doc.layers[i].kind) {
                    layer_changed |= ui.add(crate::dial::Dial::new(&mut sh.width, 0.0..=60.0).text("선 굵기")).changed();
                    if matches!(sh.kind, ShapeKind::Rect | ShapeKind::Ellipse) {
                        ui.horizontal(|ui| {
                            let mut has = sh.fill.is_some();
                            if ui.checkbox(&mut has, "채우기").changed() {
                                sh.fill = has.then_some(s.fill_color);
                                layer_changed = true;
                            }
                            if let Some(f) = &mut sh.fill {
                                layer_changed |= color_row(ui, f, PalTarget::ShapeFill(lid), &mut pal_req);
                            }
                        });
                    }
                    s.shape_width = sh.width;
                    if let Some(f) = sh.fill {
                        s.fill_color = f;
                    }
                    ui.small("선택한 도형에 적용 · 새 도형도 이 설정으로 시작");
                } else {
                    ui.add(crate::dial::Dial::new(&mut s.shape_width, 0.0..=60.0).text("선 굵기"));
                    if matches!(self.tool, Tool::Rect | Tool::Ellipse) {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut s.shape_fill, "채우기");
                            color_row(ui, &mut s.fill_color, PalTarget::DefaultFill, &mut pal_req);
                        });
                    }
                }
                ui.small("Shift: 수평·수직·45° / 정사각·정원 · Ctrl+드래그: 굵기 (선택한 도형에도 적용)");
            }
            Tool::Crop => {
                ui.strong("사진 자르기");
                ui.small("사진 선택 → 테두리 손잡이로 자르기 · 안쪽 끌기: 위치 이동 · Enter/Esc: 끝");
            }
            Tool::Frame => {
                ui.strong("내보낼 영역");
                ui.small("끌어서 영역 지정 · 손잡이로 조절 · Ctrl+E: 내보내기");
                if let Some(mut r) = self.doc.export_frame {
                    if rect_fields(ui, &mut r, "frame", None, &mut false) {
                        self.doc.export_frame = Some(r);
                        self.mark();
                    }
                    if ui.button("영역 지우기").clicked() {
                        self.doc.export_frame = None;
                        self.mark();
                    }
                }
                if ui.button("내용 전체에 맞추기").clicked() {
                    if let Some(b) = self.doc.content_bounds() {
                        let b = b.expand(20.0);
                        self.doc.export_frame = Some(Rect::from_min_max(b.min.floor(), b.max.ceil()));
                        self.mark();
                    }
                }
                // exactly the selection's bounds (padding is set when saving)
                if let Some(b) = self.selection_bounds() {
                    if ui.button("선택에 맞추기").clicked() {
                        self.doc.export_frame = Some(Rect::from_min_max(b.min.floor(), b.max.ceil()));
                        self.mark();
                    }
                }
            }
            Tool::Space => self.space_panel(ui),
            Tool::Select => {
                ui.strong("선택");
                ui.small("클릭: 선택 · 끌기: 이동 · Shift+끌기: 영역 선택(중심 기준, Ctrl 함께: 완전히 포함) · Shift+클릭: 추가/빼기 · 펜은 획, 브러시는 영역 안 픽셀만 · 모서리: 크기 · 위 동그라미: 회전");
            }
            Tool::Lasso => {
                ui.strong("올가미 선택");
                ui.small("자유롭게 둘러 선택 · 펜은 획, 브러시는 둘러싼 픽셀만, 나머지는 중심 기준 · 선택 안쪽을 끌면 이동");
            }
            Tool::Hand => {
                ui.strong("화면 이동");
                ui.small("휠: 확대/축소 · 가운데 버튼(펜 버튼)/Space+끌기: 이동 · Shift+가운데 끌기: 확대/축소 · F12: 입력 기록");
            }
            Tool::Picker => {
                ui.strong("스포이드");
                ui.small("캔버스에서 색 집기");
            }
        }
        if layer_changed {
            self.mark();
        }
        if let Some((t, c, at)) = pal_req {
            self.open_palette_for(t, c, at);
        }
        ui.separator();
        ui.checkbox(&mut self.s.touch_draw, "손가락 터치로도 그리기")
            .on_hover_text("끄면 손가락 한 개는 화면 이동, 두 개는 확대/축소. 펜은 항상 그려.");
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        if self.sel_parts().len() >= 2 {
            self.align_bar(ui);
        }
        let Some(id) = self.sel else {
            if self.sub.is_empty() {
                ui.weak("선택된 레이어 없음");
            } else {
                let ns: usize = self.sub.strokes.iter().map(|x| x.1.len()).sum();
                let np = self.sub.pixels.len();
                ui.label(format!("펜 획 {ns}개 · 브러시 영역 {np}개"));
                ui.small("끌어서 이동 · 모서리/위 동그라미: 크기·회전 · Del: 지우기");
            }
            return;
        };
        if !self.extra_sel.is_empty() {
            ui.label(egui::RichText::new(format!("{}개 선택 · 첫 항목 값 표시 · 모두에 적용", self.extra_sel.len() + 1)).color(INK_3()));
        }
        if let Some(owner) = self.doc.get(id).and_then(|l| l.mask_of) {
            self.mask_props(ui, id, owner);
            return;
        }
        let Some(idx) = self.doc.index_of(id) else { return };
        // the primary as it was, to pass the panel's edits on to the rest
        let before = self.doc.layers[idx].clone();
        // values that differ across the selection show as "—"
        let (mx, mx_adj, mx_width) = {
            let k0 = crate::anim::key_of(&before, 0.0);
            let others: Vec<&Layer> = self.extra_sel.iter().filter_map(|i| self.doc.get(*i)).collect();
            let mx = others.iter().fold(0u16, |m, o| m | crate::anim::diff_ch(&k0, &crate::anim::key_of(o, 0.0)));
            let adj = |l: &Layer| match &l.kind {
                LayerKind::Image(im) => Some([im.adjust.brightness, im.adjust.contrast, im.adjust.saturation]),
                _ => None,
            };
            let a0 = adj(&before);
            let mut mx_adj = [false; 3];
            for o in &others {
                if let (Some(a), Some(b)) = (a0, adj(o)) {
                    for i in 0..3 {
                        mx_adj[i] |= a[i] != b[i];
                    }
                }
            }
            let width = |l: &Layer| match &l.kind {
                LayerKind::Shape(s) => Some(s.width),
                _ => None,
            };
            let mx_width = others.iter().any(|o| width(o).is_some_and(|w| Some(w) != width(&before)));
            (mx, mx_adj, mx_width)
        };
        let mxc = |c: u16| mx & c != 0;
        let tool = self.tool;
        let shape_tool = matches!(tool, Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse);
        let mut changed = false;
        let mut pal_req: PalReq = None;
        let mut start_crop = false;
        let mut want_mask = false;
        let cams = crate::persp::cameras(&self.doc.spaces);
        let spaces: Vec<(u64, String, bool, Vec<(u64, String)>)> = self
            .doc
            .spaces
            .iter()
            .map(|s| (s.id, s.name.clone(), cams.contains_key(&s.id), s.walls.iter().map(|w| (w.id, w.name.clone())).collect()))
            .collect();
        let mut space_req: Option<Option<(u64, Option<u64>)>> = None;
        // chroma key slot dragged (pointer now) / dropped (pointer at release)
        let mut chroma_drag: Option<Pos2> = None;
        let mut chroma_drop: Option<Pos2> = None;
        let mut vid_act: Option<VidAct> = None;
        let is_playing = self.playing.contains_key(&id);
        let tl = &self.doc.timeline;
        let (tl_on, tt, tol) = (tl.on, tl.time, tl.frame_len() * 0.5);
        let an = self.doc.layers[idx].anim.clone();
        let km = |c: u16| tl_on.then(|| key_mark(an.as_ref(), tt, tol, c));
        use crate::anim::ch;
        // key buttons pressed, by value bit
        let mut kh = [false; 10];
        {
            let l = &mut self.doc.layers[idx];
            ui.horizontal(|ui| {
                ui.label(l.kind_label());
                // the button keeps its place; the name field takes what is left
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    want_mask = ui.button("마스크").on_hover_text("마스크 추가·선택").clicked();
                    if !matches!(l.kind, LayerKind::Text(_)) {
                        let w = ui.available_width() - 8.0; // the field's own margins
                        changed |= ui.add(egui::TextEdit::singleline(&mut l.name).desired_width(w.max(24.0))).changed();
                    }
                });
            });
            changed |= ui.add(crate::dial::Dial::new(&mut l.opacity, 0.0..=1.0).text("불투명도").default(1.0).key(km(ch::OPA), &mut kh[3]).mixed(mxc(ch::OPA))).changed();
            if l.place.is_none() {
                // 2D position (the centre); placed layers use the 3D fields below
                let c0 = crate::anim::key_of(l, 0.0).center;
                let (mut x, mut y) = (c0[0], c0[1]);
                let mut mv = false;
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    crate::dial::label_cell(ui, "위치", km(ch::POS), &mut kh[0]);
                    ui.add_sized([10.0, 26.0], egui::Label::new(egui::RichText::new("X").color(INK_3())));
                    ui.spacing_mut().interact_size.x = 56.0;
                    let m = mxc(ch::POS);
                    mv |= ui.add(pos_box(&mut x, m)).changed();
                    ui.add_sized([10.0, 26.0], egui::Label::new(egui::RichText::new("Y").color(INK_3())));
                    mv |= ui.add(pos_box(&mut y, m)).changed();
                });
                if mv {
                    l.translate(vec2(x - c0[0], y - c0[1]));
                    changed = true;
                }
            }
            let is_mask = l.mask_of.is_some();
            match &mut l.kind {
                LayerKind::Image(im) => {
                    if let Some(v) = &mut im.video {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if ui.button(if is_playing { "멈춤" } else { "재생" }).clicked() {
                                vid_act = Some(VidAct::Play(!is_playing));
                            }
                            if ui.button("이전").on_hover_text("이전 프레임 ( , )").clicked() {
                                vid_act = Some(VidAct::Step(-1));
                            }
                            if ui.button("다음").on_hover_text("다음 프레임 ( . )").clicked() {
                                vid_act = Some(VidAct::Step(1));
                            }
                            ui.label(egui::RichText::new(format!("{} / {}", crate::video::timecode(v.time), crate::video::timecode(v.info.duration))).monospace());
                        });
                        let mut t = v.time;
                        ui.spacing_mut().slider_width = ui.available_width() - 8.0;
                        let sr = ui.add(egui::Slider::new(&mut t, 0.0..=v.info.duration.max(0.01)).show_value(false));
                        if sr.changed() {
                            vid_act = Some(VidAct::Seek(t));
                        }
                        if sr.drag_stopped() {
                            changed = true;
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                            ui.label("속도");
                            for sp in [0.25, 0.5, 1.0, 2.0, 4.0] {
                                if ui.selectable_label((v.speed - sp).abs() < 1e-6, format!("{sp}×")).clicked() {
                                    v.speed = sp;
                                    vid_act = Some(VidAct::Restart);
                                    changed = true;
                                }
                            }
                            changed |= ui.checkbox(&mut v.looping, "반복").changed();
                            if v.info.audio {
                                changed |= ui.checkbox(&mut v.mute, "소리 끔").changed();
                            }
                        });
                        if v.info.audio {
                            changed |= ui.add(crate::dial::Dial::new(&mut v.volume, 0.0..=4.0).step(0.05).text("볼륨").default(1.0).key(km(ch::VOL), &mut kh[9]).mixed(mxc(ch::VOL))).changed();
                        }
                        ui.small(format!(
                            "{}×{} · {:.2} fps · {}{}",
                            v.info.width,
                            v.info.height,
                            v.info.fps,
                            v.info.codec,
                            if v.info.audio { " · 소리 있음" } else { "" }
                        ));
                        if ui.button("이 프레임을 사진으로").on_hover_text("지금 프레임을 원본 해상도 사진 레이어로 복사").clicked() {
                            vid_act = Some(VidAct::Still);
                        }
                        ui.separator();
                    }
                    ui.label(format!("원본 {}×{} px · 잘린 크기 {:.0}×{:.0}", im.src.size().x, im.src.size().y, im.crop.width(), im.crop.height()));
                    let mut pct = im.scale * 100.0;
                    if ui.add(crate::dial::Dial::new(&mut pct, 1.0..=400.0).step(2.0).text("크기 %").default(100.0).key(km(ch::SIZE), &mut kh[1]).mixed(mxc(ch::SIZE))).changed() {
                        im.scale = pct / 100.0;
                        changed = true;
                    }
                    let mut deg = im.rotation.to_degrees();
                    if ui.add(crate::dial::Dial::new(&mut deg, -180.0..=180.0).text("회전 °").default(0.0).key(km(ch::ROT), &mut kh[2]).mixed(mxc(ch::ROT))).changed() {
                        im.rotation = deg.to_radians();
                        changed = true;
                    }
                    ui.horizontal(|ui| {
                        if ui.button("원본 크기").clicked() {
                            im.scale = 1.0;
                            changed = true;
                        }
                        if ui.button("회전 0°").clicked() {
                            im.rotation = 0.0;
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("자르기…").clicked() {
                            start_crop = true;
                        }
                        if ui.button("자르기 해제").clicked() {
                            let full = Rect::from_min_size(Pos2::ZERO, im.src.size());
                            let c = im.src_to_world(full.center());
                            im.crop = full;
                            im.center = c;
                            changed = true;
                        }
                    });
                    ui.label("보정");
                    let a = &mut im.adjust;
                    changed |= ui.add(crate::dial::Dial::new(&mut a.brightness, -1.0..=1.0).text("밝기").default(0.0).mixed(mx_adj[0])).changed();
                    changed |= ui.add(crate::dial::Dial::new(&mut a.contrast, -1.0..=1.0).text("대비").default(0.0).mixed(mx_adj[1])).changed();
                    changed |= ui.add(crate::dial::Dial::new(&mut a.saturation, -1.0..=1.0).text("채도").default(0.0).mixed(mx_adj[2])).changed();
                    if ui.button("보정 초기화").clicked() {
                        a.reset_tone();
                        changed = true;
                    }
                    // chroma key: drag the slot onto the canvas to take a colour
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let mut no = false;
                        crate::dial::label_cell(ui, "크로마 키", None, &mut no);
                        let (sr, sresp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click_and_drag());
                        let p = ui.painter();
                        match a.chroma {
                            Some([r, g, b]) => {
                                p.rect_filled(sr, 4.0, Color32::from_rgb(r, g, b));
                            }
                            None => {
                                p.line_segment([sr.left_bottom() + vec2(4.0, -4.0), sr.right_top() + vec2(-4.0, 4.0)], Stroke::new(1.2, INK_3()));
                            }
                        }
                        p.rect_stroke(sr, 4.0, Stroke::new(1.0, if sresp.hovered() || sresp.dragged() { ACCENT } else { INK_3() }), StrokeKind::Inside);
                        if sresp.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                            chroma_drag = ui.ctx().pointer_latest_pos();
                        }
                        if sresp.drag_stopped() {
                            chroma_drop = ui.ctx().pointer_latest_pos();
                        }
                        sresp.on_hover_text("캔버스로 끌어 색 선택");
                        if a.chroma.is_some() && ui.small_button("해제").clicked() {
                            a.chroma = None;
                            changed = true;
                        }
                    });
                    if a.chroma.is_some() {
                        changed |= ui.add(crate::dial::Dial::new(&mut a.key_tol, 0.0..=0.6).step(0.01).text("허용 범위").default(0.12)).changed();
                        changed |= ui.add(crate::dial::Dial::new(&mut a.key_soft, 0.0..=0.4).step(0.01).text("부드럽게").default(0.08)).changed();
                    }
                }
                LayerKind::Text(t) => {
                    let s0 = crate::text::layout(t).size;
                    let r = ui.add(egui::TextEdit::multiline(&mut t.text).desired_rows(3).desired_width(f32::INFINITY));
                    changed |= r.changed();
                    if tool != Tool::Text {
                        changed |= ui.add(crate::dial::Dial::new(&mut t.size, 6.0..=400.0).step(2.0).text("크기").key(km(ch::SIZE), &mut kh[1]).mixed(mxc(ch::SIZE))).changed();
                        changed |= font_picker(ui, "prop_font", &mut t.family, &mut t.style, &mut self.font_ui);
                        changed |= align_row(ui, &mut t.align);
                        ui.horizontal(|ui| {
                            let mut has = t.bg.is_some();
                            if ui.checkbox(&mut has, "배경 상자").changed() {
                                t.bg = has.then_some(self.s.text_bg_color);
                                changed = true;
                            }
                            if let Some(bg) = &mut t.bg {
                                changed |= color_row(ui, bg, PalTarget::TextBg(id), &mut pal_req);
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("색");
                        changed |= color_row(ui, &mut t.color, PalTarget::TextColor(id), &mut pal_req);
                    });
                    ui.horizontal(|ui| {
                        ui.label("혼합");
                        egui::ComboBox::from_id_salt("text_blend").truncate()
                            .selected_text(t.blend.label())
                            .width(110.0)
                            .popup_style(theme::glass_popup_style(false))
                            .show_ui(ui, |ui| {
                                theme::glass_popup(ui, |ui| {
                                    for b in Blend::ALL {
                                        if ui.selectable_label(t.blend == b, b.label()).clicked() {
                                            t.blend = b;
                                            changed = true;
                                        }
                                    }
                                })
                            });
                    });
                    let mut deg = t.rotation.to_degrees();
                    if ui.add(crate::dial::Dial::new(&mut deg, -180.0..=180.0).text("회전 °").default(0.0).key(km(ch::ROT), &mut kh[2]).mixed(mxc(ch::ROT))).changed() {
                        t.rotation = deg.to_radians();
                        changed = true;
                    }
                    ui.horizontal(|ui| {
                        let mut wrap = t.wrap > 0.0;
                        if ui.checkbox(&mut wrap, "줄바꿈 폭").changed() {
                            t.wrap = if wrap { (crate::text::layout(t).size.x).max(t.size * 4.0) } else { 0.0 };
                            changed = true;
                        }
                        if t.wrap > 0.0 {
                            changed |= ui.add(crate::calc::drag(&mut t.wrap).custom_parser(crate::calc::parse).range(10.0..=20000.0).suffix(" px")).changed();
                        }
                    });
                    if changed {
                        t.keep_corner(s0);
                    }
                }
                LayerKind::Shape(s) => {
                    ui.horizontal(|ui| {
                        ui.label("색");
                        changed |= color_row(ui, &mut s.color, PalTarget::ShapeColor(id), &mut pal_req);
                    });
                    ui.horizontal(|ui| {
                        if !shape_tool {
                            changed |= ui.add(crate::dial::Dial::new(&mut s.width, 0.0..=60.0).text("굵기").mixed(mx_width)).changed();
                        }
                    });
                    if matches!(s.kind, ShapeKind::Rect | ShapeKind::Ellipse) {
                        let mut deg = s.rotation.to_degrees();
                        if ui.add(crate::dial::Dial::new(&mut deg, -180.0..=180.0).text("회전 °").default(0.0).key(km(ch::ROT), &mut kh[2]).mixed(mxc(ch::ROT))).changed() {
                            s.rotation = deg.to_radians();
                            changed = true;
                        }
                    }
                    if !shape_tool && matches!(s.kind, ShapeKind::Rect | ShapeKind::Ellipse) {
                        ui.horizontal(|ui| {
                            let mut has = s.fill.is_some();
                            if ui.checkbox(&mut has, "채우기").changed() {
                                s.fill = has.then_some(self.s.fill_color);
                                changed = true;
                            }
                            if let Some(f) = &mut s.fill {
                                changed |= color_row(ui, f, PalTarget::ShapeFill(id), &mut pal_req);
                            }
                        });
                    }
                }
                LayerKind::Vector(v) => {
                    ui.label(format!("획 {}개", v.strokes.len()));
                }
                LayerKind::Raster(r) => {
                    ui.label(format!("타일 {}개", r.tiles.len()));
                    if ui.button("모두 지우기").clicked() {
                        r.tiles.clear();
                        changed = true;
                    }
                    if !is_mask {
                        ui.horizontal(|ui| {
                            ui.label("혼합");
                            egui::ComboBox::from_id_salt("raster_blend").truncate()
                                .selected_text(r.blend.label())
                                .width(110.0)
                                .popup_style(theme::glass_popup_style(false))
                                .show_ui(ui, |ui| {
                                    theme::glass_popup(ui, |ui| {
                                        for b in Blend::ALL {
                                            if ui.selectable_label(r.blend == b, b.label()).clicked() {
                                                r.blend = b;
                                                changed = true;
                                            }
                                        }
                                    })
                                });
                        });
                    }
                }
            }
            if !spaces.is_empty() {
                ui.separator();
                let cur = l.place.as_ref().map(|p| (p.space, p.wall));
                let shown = match cur {
                    Some((sid, wid)) => {
                        let sp = spaces.iter().find(|s| s.0 == sid);
                        let sn = sp.map_or(String::new(), |s| s.1.clone());
                        match wid.and_then(|w| sp.and_then(|s| s.3.iter().find(|x| x.0 == w))) {
                            Some(w) => format!("{sn} › {}", w.1),
                            None => sn,
                        }
                    }
                    None => "없음".to_string(),
                };
                ui.horizontal(|ui| {
                    ui.label("3D 공간");
                    egui::ComboBox::from_id_salt(("space_of", id)).truncate()
                        .selected_text(shown)
                        .width(120.0)
                        .popup_style(theme::glass_popup_style(false))
                        .show_ui(ui, |ui| {
                            theme::glass_popup(ui, |ui| {
                                if ui.selectable_label(cur.is_none(), "없음").clicked() && cur.is_some() {
                                    space_req = Some(None);
                                }
                                for (sid, name, ok, walls) in &spaces {
                                    ui.add_enabled_ui(*ok, |ui| {
                                        let here = Some((*sid, None));
                                        if ui.selectable_label(cur == here, name.as_str()).clicked() && cur != here {
                                            space_req = Some(here);
                                        }
                                        for (wid, wn) in walls {
                                            let here = Some((*sid, Some(*wid)));
                                            if ui.selectable_label(cur == here, format!("   {name} › {wn}")).clicked() && cur != here {
                                                space_req = Some(here);
                                            }
                                        }
                                    });
                                }
                            })
                        });
                });
                if let Some(pl) = &mut l.place {
                    for (i, ax) in ["X", "Y", "Z"].iter().enumerate() {
                        let mut v = pl.pos[i] as f32;
                        if ui.add(crate::dial::Dial::new(&mut v, -100000.0..=100000.0).step(10.0).text(format!("위치 {ax}")).default(0.0).key(km(ch::P_POS), &mut kh[4]).mixed(mxc(ch::P_POS))).changed() {
                            pl.pos[i] = v as f64;
                            changed = true;
                        }
                    }
                    for (i, ax) in ["X", "Y", "Z"].iter().enumerate() {
                        let mut v = pl.rot[i] as f32;
                        if ui.add(crate::dial::Dial::new(&mut v, -180.0..=180.0).step(1.0).text(format!("회전 {ax} °")).default(0.0).key(km(ch::P_ROT), &mut kh[5]).mixed(mxc(ch::P_ROT))).changed() {
                            pl.rot[i] = v as f64;
                            changed = true;
                        }
                    }
                    let mut pct = (pl.scale * 100.0) as f32;
                    if ui.add(crate::dial::Dial::new(&mut pct, 1.0..=1000.0).step(2.0).text("크기 %").default(100.0).key(km(ch::P_SCALE), &mut kh[6]).mixed(mxc(ch::P_SCALE))).changed() {
                        pl.scale = pct as f64 / 100.0;
                        changed = true;
                    }
                    let mut th = pl.thick as f32;
                    if ui
                        .add(crate::dial::Dial::new(&mut th, 0.0..=5000.0).step(2.0).text("두께").default(0.0).key(km(ch::P_THICK), &mut kh[7]).mixed(mxc(ch::P_THICK)))
                        .on_hover_text("평면 뒤로 밀어낸 두께")
                        .changed()
                    {
                        pl.thick = th as f64;
                        changed = true;
                    }
                    let on_wall = pl.wall.is_some();
                    if !on_wall {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("공간 축에 맞추기").on_hover_text("가장 가까운 벽·바닥 방향으로").clicked() {
                            pl.align_to_axes();
                            changed = true;
                        }
                        if let Some(cam) = cams.get(&pl.space) {
                            if ui.button("2D 모습 유지").on_hover_text("지금 카메라 기준으로 다시 맞춤").clicked() {
                                let f = crate::persp::Placement::facing(pl.space, cam, pl.anchor);
                                pl.base_r = f.base_r;
                                pl.base_t = f.base_t;
                                pl.base_k = f.base_k;
                                changed = true;
                            }
                        }
                    });
                    }
                    ui.small(if on_wall {
                        "벽 기준 축 · X/Y: 벽을 따라 · Z: 벽에서 떨어짐"
                    } else {
                        "축은 넣을 때의 카메라 정면 기준 · 1단위 = 원점에서 1px"
                    });
                }
            }
        }
        if changed {
            self.spread_edit(&before);
            self.mark();
        }
        let kc = kh.iter().enumerate().filter(|x| *x.1).fold(0u16, |a, (i, _)| a | (1 << i));
        if kc != 0 {
            for sid in self.selected_ids() {
                self.key_layer(sid, kc);
            }
        }
        if let Some((t, c, at)) = pal_req {
            self.open_palette_for(t, c, at);
        }
        if let Some(sp) = chroma_drag {
            // preview of the colour under the pointer
            if let Some(c) = self.chroma_sample(id, sp) {
                let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("chroma_pick")));
                p.circle_filled(sp + vec2(18.0, -18.0), 10.0, c);
                p.circle_stroke(sp + vec2(18.0, -18.0), 10.0, Stroke::new(1.5, Color32::WHITE));
            }
        }
        if let Some(sp) = chroma_drop {
            if let Some(c) = self.chroma_sample(id, sp) {
                if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    im.adjust.chroma = Some([c.r(), c.g(), c.b()]);
                }
                self.mark();
            }
        }
        match vid_act {
            Some(VidAct::Play(on)) => self.video_play(id, on),
            Some(VidAct::Step(d)) => self.video_step(id, d),
            Some(VidAct::Seek(t)) => {
                if is_playing {
                    if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                        if let Some(v) = im.video.as_mut() {
                            v.time = t;
                        }
                    }
                    self.video_play(id, true);
                } else {
                    self.video_scrub(id, t);
                }
            }
            Some(VidAct::Restart) => {
                if is_playing {
                    self.video_play(id, true);
                }
            }
            Some(VidAct::Still) => self.video_still(id),
            None => {}
        }
        for lid in self.selected_ids() {
            match space_req {
                Some(Some((sid, None))) => self.put_in_space(lid, sid),
                Some(Some((sid, Some(wid)))) => self.put_in_wall(lid, sid, wid),
                Some(None) => self.take_out_of_space(lid),
                None => {}
            }
        }
        if want_mask {
            self.mask_button(id);
            return;
        }
        if start_crop {
            self.set_tool(Tool::Crop);
        }
    }

    /// The colour under screen point `sp` for a chroma key on photo `id`: its
    /// own source pixel there (before any adjustment), else the picture.
    fn chroma_sample(&self, id: LayerId, sp: Pos2) -> Option<Color32> {
        if !self.canvas_rect.contains(sp) {
            return None;
        }
        let w = self.xf().s2w(sp);
        if let Some((LayerKind::Image(im), true)) = self.doc.get(id).map(|l| (&l.kind, l.place.is_none())) {
            if im.contains(w) {
                let s = im.world_to_src(w);
                let k = im.src.rgba.width() as f32 / im.src.size().x.max(1.0);
                let (x, y) = ((s.x * k) as u32, (s.y * k) as u32);
                if let Some(p) = im.src.rgba.get_pixel_checked(x, y) {
                    return Some(Color32::from_rgb(p[0], p[1], p[2]));
                }
            }
        }
        crate::export::sample(&self.doc, w)
    }

    /// Passes an edit of the primary selection (`before` -> its state now) on to
    /// the other selected layers: moves, turns and resizes as deltas, set
    /// values (opacity, thickness, colours, fonts, blend...) as they are.
    fn spread_edit(&mut self, before: &Layer) {
        let Some(after) = self.sel_layer().cloned() else { return };
        for id in self.extra_sel.clone() {
            if let Some(l) = self.doc.get_mut(id).filter(|l| !l.locked) {
                spread_to(before, &after, l);
            }
        }
    }

    fn mask_props(&mut self, ui: &mut egui::Ui, id: LayerId, owner: LayerId) {
        let names: Vec<(LayerId, String)> = self.doc.layers.iter().rev().filter(|l| l.mask_of.is_none()).map(|l| (l.id, l.display_name())).collect();
        let owner_name = names.iter().find(|n| n.0 == owner).map_or(String::new(), |n| n.1.clone());
        ui.label(egui::RichText::new(format!("마스크 · {owner_name}")).color(INK_2()));
        let inv = self.doc.get(id).is_some_and(|l| l.mask_invert);
        ui.small(if inv { "칠한 곳만 보임 · 지우개: 가리기" } else { "칠한 곳 가림 · 지우개: 되살리기 · 사각형: 가리기 (Alt: 되살리기)" });
        // strength (the mask layer's opacity, keyable) and invert
        let (tl_on, tt, tol) = (self.doc.timeline.on, self.doc.timeline.time, self.doc.timeline.frame_len() * 0.5);
        let mut key_hit = false;
        let mut mask_changed = false;
        if let Some(l) = self.doc.get_mut(id) {
            let mark = tl_on.then(|| key_mark(l.anim.as_ref(), tt, tol, crate::anim::ch::OPA));
            mask_changed |= ui.add(crate::dial::Dial::new(&mut l.opacity, 0.0..=1.0).text("강도").default(1.0).key(mark, &mut key_hit)).changed();
            mask_changed |= ui.checkbox(&mut l.mask_invert, "반전").changed();
        }
        if mask_changed {
            self.mark();
        }
        if key_hit {
            self.key_layer(id, crate::anim::ch::OPA);
        }
        let anchor = self.doc.get(id).and_then(|l| l.mask_anchor).map(|a| a.0);
        let mut pick: Option<Option<LayerId>> = None;
        ui.horizontal(|ui| {
            ui.label("따라가기");
            let shown = anchor.and_then(|a| names.iter().find(|n| n.0 == a)).map_or("없음".to_string(), |n| n.1.clone());
            egui::ComboBox::from_id_salt(("mask_anchor", id)).truncate().selected_text(shown).width(ui.available_width() - 24.0).popup_style(theme::glass_popup_style(false)).show_ui(ui, |ui| {
                theme::glass_popup(ui, |ui| {
                    if ui.selectable_label(anchor.is_none(), "없음").clicked() {
                        pick = Some(None);
                    }
                    for (lid, n) in &names {
                        if ui.selectable_label(anchor == Some(*lid), n.as_str()).clicked() {
                            pick = Some(Some(*lid));
                        }
                    }
                })
            });
        });
        let mut clear = false;
        let armed = self.wand == Some(id);
        let mut toggle = false;
        let mut pick_mode: Option<u8> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (m, label, tip) in [(0u8, "자동 선택", "클릭한 면을 가림 · Alt: 되살리기"), (1, "그랩컷", "상자로 끌기 · 안의 물체를 가림 · Alt: 되살리기"), (2, "가위", "경계를 따라 클릭 · 첫 점 클릭/Enter: 닫기 · Shift: 그랩컷으로 다듬어 닫기 · Esc: 취소 · Alt: 되살리기")] {
                if ui.selectable_label(armed && self.wand_mode == m, label).on_hover_text(tip).clicked() {
                    pick_mode = Some(m);
                }
            }
            clear = ui.button("비우기").clicked();
        });
        if let Some(m) = pick_mode {
            if armed && self.wand_mode == m {
                toggle = true;
            } else {
                self.wand_mode = m;
                self.wand = Some(id);
            }
        }
        ui.add(crate::dial::Dial::new(&mut self.s.wand_tol, 0.0..=100.0).step(1.0).integer().text("허용 오차").default(24.0));
        ui.add(crate::dial::Dial::new(&mut self.s.mask_refine, 0.0..=8.0).step(1.0).integer().suffix(" px").text("다듬기").default(2.0))
            .on_hover_text("경계를 그림에 맞춰 부드럽게 · 0: 딱 떨어지는 경계");
        ui.add(crate::dial::Dial::new(&mut self.s.mask_grow, -4.0..=4.0).step(1.0).integer().suffix(" px").text("경계 이동").default(1.0))
            .on_hover_text("+: 가리는 영역 넓힘 (틈 메우기) · -: 좁힘");
        if toggle {
            self.wand = if armed { None } else { Some(id) };
        }
        if let Some(a) = pick {
            let c = a.and_then(|a| self.doc.get(a)).map(|l| crate::anim::key_of(l, 0.0).center);
            if let Some(l) = self.doc.get_mut(id) {
                l.mask_anchor = a.zip(c);
                l.mask_pose = None;
            }
            self.mark();
        }
        if clear {
            if let Some(LayerKind::Raster(r)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                r.tiles.clear();
            }
            self.mark();
        }
    }

    fn layers_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let sel = self.sel.is_some();
            if ui.add_enabled(sel, egui::Button::new("위로").small()).clicked() {
                self.move_selected(1);
            }
            if ui.add_enabled(sel, egui::Button::new("아래로").small()).clicked() {
                self.move_selected(-1);
            }
            if ui.add_enabled(sel, egui::Button::new("복제").small()).clicked() {
                self.duplicate_selected();
            }
            if ui.add_enabled(sel, egui::Button::new("삭제").small()).clicked() {
                self.delete_selected();
            }
        });
        ui.add_space(SPACE_SM);
        if self.doc.layers.is_empty() {
            ui.label(egui::RichText::new("비어 있음").color(INK_3()));
            return;
        }
        let mut changed = false;
        let mut select = None;
        ui.spacing_mut().item_spacing.y = 2.0;
        let width = ui.available_width();
        let ctx = ui.ctx().clone();
        // thumbnails: photos show their own (cropped) texture
        let thumbs: Vec<Option<(egui::TextureId, Rect, Vec2)>> = {
            let tex = &mut self.tex;
            self.doc
                .layers
                .iter()
                .map(|l| match &l.kind {
                    LayerKind::Image(im) => {
                        let id = tex.image(&ctx, im);
                        let size = im.src.size();
                        let uv = Rect::from_min_max((im.crop.min.to_vec2() / size).to_pos2(), (im.crop.max.to_vec2() / size).to_pos2());
                        Some((id, uv, im.crop.size()))
                    }
                    _ => None,
                })
                .collect()
        };
        let n = self.doc.layers.len();
        let cams = crate::persp::cameras(&self.doc.spaces);
        let mut header_done: Vec<u64> = Vec::new();
        let mut wall_done: Vec<u64> = Vec::new();
        let mut headers: Vec<(u64, Option<u64>, Rect)> = Vec::new();
        let mut space_click: Option<(u64, Option<u64>)> = None;
        let mut drop_space: Option<(LayerId, u64, Option<u64>)> = None;
        let mut move_out: Option<LayerId> = None;
        let wall_count = |layers: &[Layer], wid: u64| layers.iter().filter(|m| m.place.as_ref().is_some_and(|p| p.wall == Some(wid))).count();
        // empty spaces first (with their walls)
        for sp in &self.doc.spaces {
            if !self.doc.layers.iter().any(|m| m.place.as_ref().is_some_and(|p| p.space == sp.id)) {
                let resp = space_header(ui, &sp.name, 0, cams.contains_key(&sp.id), width, self.space_sel == Some(sp.id), 0.0, false);
                headers.push((sp.id, None, resp.rect));
                if resp.clicked() {
                    space_click = Some((sp.id, None));
                }
                header_done.push(sp.id);
                for w in &sp.walls {
                    let resp = space_header(ui, &w.name, 0, true, width, self.wall_sel == Some(w.id), 14.0, true);
                    headers.push((sp.id, Some(w.id), resp.rect));
                    if resp.clicked() {
                        space_click = Some((sp.id, Some(w.id)));
                    }
                    wall_done.push(w.id);
                }
            }
        }
        let mut rows: Vec<(usize, Rect)> = Vec::with_capacity(n);
        let masked_ids: Vec<LayerId> = self.doc.layers.iter().filter_map(|m| m.mask_of).collect();
        let mut mask_click: Option<LayerId> = None;
        let renaming = self.renaming;
        let mut rename_done = false;
        for di in 0..n {
            let li = n - 1 - di; // list shows the top layer first
            let member_of = self.doc.layers[li].place.as_ref().map(|p| (p.space, p.wall));
            if let Some((sid, _)) = member_of.filter(|(s, _)| !header_done.contains(s)) {
                header_done.push(sid);
                if let Some(sp) = self.doc.spaces.iter().find(|s| s.id == sid) {
                    let cnt = self.doc.layers.iter().filter(|m| m.place.as_ref().is_some_and(|p| p.space == sid)).count();
                    let resp = space_header(ui, &sp.name, cnt, cams.contains_key(&sid), width, self.space_sel == Some(sid), 0.0, false);
                    headers.push((sid, None, resp.rect));
                    if resp.clicked() {
                        space_click = Some((sid, None));
                    }
                    // walls with nothing on them yet
                    for w in &sp.walls {
                        if wall_count(&self.doc.layers, w.id) == 0 {
                            let resp = space_header(ui, &w.name, 0, true, width, self.wall_sel == Some(w.id), 14.0, true);
                            headers.push((sid, Some(w.id), resp.rect));
                            if resp.clicked() {
                                space_click = Some((sid, Some(w.id)));
                            }
                            wall_done.push(w.id);
                        }
                    }
                }
            }
            if let Some((sid, Some(wid))) = member_of.filter(|(_, w)| w.is_some_and(|w| !wall_done.contains(&w))) {
                wall_done.push(wid);
                if let Some(w) = self.doc.spaces.iter().find(|s| s.id == sid).and_then(|s| s.walls.iter().find(|w| w.id == wid)) {
                    let resp = space_header(ui, &w.name, wall_count(&self.doc.layers, wid), true, width, self.wall_sel == Some(wid), 14.0, true);
                    headers.push((sid, Some(wid), resp.rect));
                    if resp.clicked() {
                        space_click = Some((sid, Some(wid)));
                    }
                }
            }
            let l = &mut self.doc.layers[li];
            let selected = self.sel == Some(l.id) || self.extra_sel.contains(&l.id);
            let (r, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click_and_drag());
            rows.push((li, r));
            // members of a space are indented under its folder row
            let indent = match member_of {
                Some((_, Some(_))) => 28.0,
                Some(_) => 14.0,
                None => 0.0,
            } + if l.mask_of.is_some() { 18.0 } else { 0.0 };
            let r = Rect::from_min_max(r.min + vec2(indent, 0.0), r.max);
            let eye_r = Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(26.0, 26.0));
            let lock_r = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(26.0, 26.0));
            let thumb_r = Rect::from_center_size(pos2(r.left() + 44.0, r.center().y), vec2(24.0, 24.0));
            let eye = ui.interact(eye_r, ui.id().with(("eye", l.id)), Sense::click());
            let lock = ui.interact(lock_r, ui.id().with(("lock", l.id)), Sense::click());
            // mask button (not on masks themselves)
            let mask_r = Rect::from_center_size(pos2(r.right() - 42.0, r.center().y), vec2(24.0, 26.0));
            let has_mask = l.mask_of.is_none() && masked_ids.contains(&l.id);
            let mask_b = (l.mask_of.is_none()).then(|| ui.interact(mask_r, ui.id().with(("mask", l.id)), Sense::click()));
            let p = ui.painter().clone();
            let dragging_this = self.layer_drag == Some(l.id);
            if selected || dragging_this {
                p.rect_filled(r, 8.0, FILL_2());
                p.rect_filled(Rect::from_min_size(r.min + vec2(0.0, 7.0), vec2(3.0, 16.0)), 1.5, ACCENT);
            } else if resp.hovered() {
                p.rect_filled(r, 8.0, FILL_1());
            }
            let dim = if l.visible { 1.0 } else { 0.45 };
            theme::eye_icon(&p, eye_r.center(), l.visible, if eye.hovered() { INK() } else { INK_3() });
            if l.locked {
                theme::lock_icon(&p, lock_r.center(), INK_2());
            } else if lock.hovered() || resp.hovered() {
                theme::lock_icon(&p, lock_r.center(), FILL_3());
            }
            // thumbnail / kind chip
            p.rect_filled(thumb_r, 5.0, FILL_1());
            match thumbs[li] {
                Some((tid, uv, size)) => {
                    let k = (thumb_r.width() / size.x).min(thumb_r.height() / size.y);
                    let ir = Rect::from_center_size(thumb_r.center(), size * k);
                    p.image(tid, ir, uv, Color32::WHITE.gamma_multiply(dim));
                }
                None => {
                    let glyph = match l.kind {
                        LayerKind::Text(_) => Tool::Text,
                        LayerKind::Shape(ref s) => match s.kind {
                            ShapeKind::Line => Tool::Line,
                            ShapeKind::Arrow => Tool::Arrow,
                            ShapeKind::Rect => Tool::Rect,
                            ShapeKind::Ellipse => Tool::Ellipse,
                        },
                        LayerKind::Vector(_) => Tool::Pen,
                        _ => Tool::Brush,
                    };
                    theme::tool_icon(&p, glyph, thumb_r.shrink(5.0), INK_3().gamma_multiply(dim));
                }
            }
            if let Some(mb) = &mask_b {
                {
                    let col = if mb.hovered() { INK() } else if has_mask { ACCENT } else { INK_3() };
                    let c = mask_r.center();
                    p.rect_stroke(Rect::from_center_size(c, vec2(14.0, 14.0)), 3.0, Stroke::new(1.3, col), StrokeKind::Inside);
                    p.circle_filled(c, 4.0, col);
                }
                if mb.clicked() {
                    mask_click = Some(l.id);
                }
                mb.clone().on_hover_text(if has_mask { "마스크 선택" } else { "마스크 추가" });
            }
            if l.mask_of.is_some() {
                let c = thumb_r.center();
                p.rect_filled(thumb_r, 5.0, FILL_2());
                p.rect_stroke(Rect::from_center_size(c, vec2(14.0, 14.0)), 3.0, Stroke::new(1.3, INK_2()), StrokeKind::Inside);
                p.circle_filled(c, 4.0, INK_2());
            }
            let name_r = Rect::from_min_max(pos2(r.left() + 62.0, r.top() + 3.0), pos2(r.right() - 58.0, r.bottom() - 3.0));
            if renaming == Some(l.id) {
                let te = ui.put(name_r, egui::TextEdit::singleline(&mut self.rename_buf).desired_width(name_r.width()));
                if self.rename_focus {
                    te.request_focus();
                    self.rename_focus = false;
                }
                if te.lost_focus() {
                    let enter_or_click = !ui.input(|i| i.key_pressed(Key::Escape));
                    if enter_or_click && !self.rename_buf.trim().is_empty() {
                        l.name = self.rename_buf.trim().to_string();
                        if let LayerKind::Text(_) = l.kind {
                            l.name = self.rename_buf.trim().to_string();
                        }
                        changed = true;
                    }
                    rename_done = true;
                }
            } else {
                let clip = p.with_clip_rect(name_r.intersect(ui.clip_rect()));
                let col = (if selected { INK() } else { INK_2() }).gamma_multiply(dim);
                let g = clip.layout_job(elided(l.display_name(), FontId::proportional(theme::TEXT_BODY), col, name_r.width()));
                clip.galley(pos2(name_r.left(), r.center().y - g.size().y * 0.5), g, col);
            }
            if eye.clicked() {
                l.visible = !l.visible;
                changed = true;
            } else if lock.clicked() {
                l.locked = !l.locked;
                changed = true;
            } else if resp.double_clicked() && !matches!(l.kind, LayerKind::Text(_)) {
                self.renaming = Some(l.id);
                self.rename_buf = l.name.clone();
                self.rename_focus = true;
            } else if resp.clicked() {
                select = Some(l.id);
            }
            if resp.drag_started() && renaming != Some(l.id) {
                self.layer_drag = Some(l.id);
            }
        }
        if rename_done {
            self.renaming = None;
        }
        if let Some(owner) = mask_click {
            self.mask_button(owner);
            return;
        }

        // drag to reorder: insertion line between rows, drop on release
        if let Some(did) = self.layer_drag {
            let pos = ui.input(|i| i.pointer.interact_pos());
            let released = ui.input(|i| i.pointer.any_released());
            if let (Some(pp), false) = (pos, rows.is_empty()) {
                // slot k = gap above display row k (k == rows.len(): below the last)
                let mut slot = rows.len();
                for (k, (_, r)) in rows.iter().enumerate() {
                    if pp.y < r.center().y {
                        slot = k;
                        break;
                    }
                }
                let y = if slot < rows.len() { rows[slot].1.top() - 1.0 } else { rows[rows.len() - 1].1.bottom() + 1.0 };
                let x0 = rows[0].1.left();
                let over = headers.iter().find(|(_, _, hr)| hr.contains(pp)).copied();
                match over {
                    Some((_, _, hr)) => {
                        ui.painter().rect_stroke(hr, 8.0, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
                    }
                    None => {
                        ui.painter().line_segment([pos2(x0 + 6.0, y), pos2(x0 + width - 6.0, y)], Stroke::new(2.0, ACCENT));
                    }
                }
                // the folder a slot falls in: from just under its header to its last member
                let space_at = |k: usize| rows.get(k).filter(|(li, _)| self.doc.layers[*li].id != did).and_then(|(li, _)| self.doc.layers[*li].place.as_ref().map(|p| (p.space, p.wall)));
                let mut inside: Option<(u64, Option<u64>)> = None;
                let spaces_here: Vec<u64> = rows.iter().filter_map(|(li, _)| self.doc.layers[*li].place.as_ref().map(|p| p.space)).collect();
                for sp in spaces_here {
                    let idx: Vec<usize> = (0..rows.len()).filter(|k| space_at(*k).is_some_and(|(s, _)| s == sp)).collect();
                    if let (Some(&fi), Some(&la)) = (idx.first(), idx.last()) {
                        if slot >= fi && slot <= la {
                            let w = match (slot.checked_sub(1).and_then(space_at), space_at(slot)) {
                                (Some((_, a)), Some((_, b))) if a == b => a,
                                (None, Some((_, b))) => b,
                                _ => None,
                            };
                            inside = Some((sp, w));
                        }
                    }
                }
                if released {
                    if let Some((sid, wid, _)) = over {
                        drop_space = Some((did, sid, wid));
                    } else if let Some(from) = self.doc.index_of(did) {
                        // display slot -> layer index (list is top-first)
                        let n = self.doc.layers.len();
                        let mut to = n - slot; // index the moved layer should occupy before removal adjust
                        let layer = self.doc.layers.remove(from);
                        if to > from {
                            to -= 1;
                        }
                        let to = to.min(self.doc.layers.len());
                        self.doc.layers.insert(to, layer);
                        changed = true;
                        match (self.doc.layers[to].place.as_ref().map(|p| p.space), inside) {
                            (Some(_), None) => move_out = Some(did),
                            (Some(cur), Some((sp, w))) if cur != sp => drop_space = Some((did, sp, w)),
                            (None, Some((sp, w))) => drop_space = Some((did, sp, w)),
                            _ => {}
                        }
                    }
                    self.layer_drag = None;
                }
            } else if released {
                self.layer_drag = None;
            }
            ui.ctx().request_repaint();
        }
        if let Some(id) = select {
            if self.editing.is_some_and(|e| e != id) {
                self.finish_text_edit();
            }
            if ui.input(|i| i.modifiers.shift) {
                self.toggle_selected(id);
            } else {
                self.select_only(Some(id));
            }
        }
        if let Some((sid, wid)) = space_click {
            self.space_sel = Some(sid);
            self.space_new = false;
            if wid.is_some() {
                self.wall_sel = wid;
            }
        }
        if let Some(did) = move_out {
            self.take_out_of_space(did);
        }
        match drop_space {
            Some((did, sid, None)) => self.put_in_space(did, sid),
            Some((did, sid, Some(wid))) => self.put_in_wall(did, sid, wid),
            None => {}
        }
        if changed {
            self.mark();
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("topbar"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min + vec2(SPACE_LG, SPACE_LG))
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::symmetric(14, 8), theme::RADIUS_CARD, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Drifting VOID").font(FontId::new(14.0, theme::ui_bold())).color(INK()));
                        ui.add_space(SPACE_SM);
                        self.menus(ui);
                    });
                });
            });
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        egui::containers::menu::MenuButton::new("파일").config(egui::containers::menu::MenuConfig::new().style(theme::glass_popup_style(true))).ui(ui, |ui| theme::glass_popup(ui, |ui| {
            if ui.button("새로 만들기  Ctrl+N").clicked() {
                self.request(Pending::New);
            }
            if ui.button("열기…  Ctrl+O").clicked() {
                self.request(Pending::Open(None));
            }
            if ui.button("저장  Ctrl+S").clicked() {
                self.save(false);
            }
            if ui.button("다른 이름으로 저장…  Ctrl+Shift+S").clicked() {
                self.save(true);
            }
            ui.separator();
            if ui.button("사진 불러오기…  Ctrl+I").clicked() {
                self.import_dialog();
            }
            if ui.button("내보내기…  Ctrl+E").clicked() {
                self.open_export();
            }
            if ui.button("영상 내보내기…  Ctrl+Shift+E").clicked() {
                self.open_video_export();
            }
            if ui.button("영역을 클립보드로 복사  Ctrl+Shift+C").clicked() {
                self.quick_copy();
            }
        }));
        egui::containers::menu::MenuButton::new("편집").config(egui::containers::menu::MenuConfig::new().style(theme::glass_popup_style(true))).ui(ui, |ui| theme::glass_popup(ui, |ui| {
            if ui.button("실행 취소  Ctrl+Z").clicked() {
                self.undo();
            }
            if ui.button("다시 실행  Ctrl+Y").clicked() {
                self.redo();
            }
            ui.separator();
            if ui.button("붙여넣기  Ctrl+V").clicked() {
                self.paste();
            }
            if ui.button("복제  Ctrl+D").clicked() {
                self.duplicate_selected();
            }
            if ui.button("삭제  Del").clicked() {
                self.delete_selected();
            }
        }));
        egui::containers::menu::MenuButton::new("보기").config(egui::containers::menu::MenuConfig::new().style(theme::glass_popup_style(true)).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)).ui(ui, |ui| theme::glass_popup(ui, |ui| {
            self.view_menu_body(ui);
        }));
    }

    fn view_menu_body(&mut self, ui: &mut egui::Ui) {
            ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.vertical(|ui| {
                ui.set_max_width(132.0);
                    ui.spacing_mut().item_spacing.y = 4.0;
                if ui.button("전체 보기  Ctrl+0").clicked() {
                    self.fit_view();
                    ui.close();
                }
                if ui.button("100%  Ctrl+1").clicked() {
                    self.set_zoom_center(1.0);
                    ui.close();
                }
                if ui.button("UI 숨기기  Ctrl+H").clicked() {
                    self.ui_hidden = true;
                    ui.close();
                }
                ui.separator();
                let mut on = crate::glass::enabled();
                if ui.checkbox(&mut on, "유리 효과").on_hover_text("끄면 불투명한 카드").changed() {
                    crate::glass::set_enabled(on);
                    crate::glass::save_settings();
                }
                let mut sh = crate::glass::shadow();
                if ui.checkbox(&mut sh, "패널 그림자").changed() {
                    crate::glass::set_shadow(sh);
                    crate::glass::save_settings();
                    theme::apply_visuals(ui.ctx());
                }
            });
            ui.separator();
            if theme_switch(ui, &mut self.s.theme) {
                self.save_prefs();
            }
            ui.separator();
            ui.add_enabled_ui(crate::glass::enabled(), |ui| {
                let mut k = crate::glass::strength();
                let r = ui.add(crate::dial::Dial::new(&mut k, 0.0..=4.0).vertical().text("가장자리").default(2.75)).on_hover_text("유리 가장자리: 굴절 + 색 번짐 강도 · 0: 평평한 반투명");
                if r.changed() {
                    crate::glass::set_strength(k);
                }
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    crate::glass::save_settings();
                }
                let mut bw = crate::glass::band();
                let r = ui.add(crate::dial::Dial::new(&mut bw, 1.0..=24.0).vertical().suffix(" pt").text("테두리").default(7.0)).on_hover_text("굴절 테두리 폭");
                if r.changed() {
                    crate::glass::set_band(bw);
                }
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    crate::glass::save_settings();
                }
            });
            ui.add(crate::dial::Dial::new(&mut self.s.peek_delay, 0.05..=1.0).vertical().step(0.05).suffix(" s").text("미리보기").default(0.2))
                .on_hover_text("Tab을 이만큼 누르고 있으면 반대쪽 작업 공간 미리보기");
        });
    }

    fn status_pill(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("status"))
            .order(egui::Order::Middle)
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -SPACE_LG))
            .constrain_to(screen)
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::symmetric(12, 6), 18, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("−").clicked() {
                            let z = self.zoom / 1.25;
                            self.set_zoom_center(z);
                        }
                        if ui
                            .add(egui::Label::new(egui::RichText::new(format!("{:.0}%", self.zoom * 100.0)).monospace().color(INK())).sense(Sense::click()))
                            .on_hover_text("클릭: 100% · Ctrl+0: 전체 보기")
                            .clicked()
                        {
                            self.set_zoom_center(1.0);
                        }
                        if ui.small_button("+").clicked() {
                            let z = self.zoom * 1.25;
                            self.set_zoom_center(z);
                        }
                        if let Some(w) = self.cursor_world {
                            ui.add_space(SPACE_SM);
                            ui.label(egui::RichText::new(format!("{:.0}, {:.0}", w.x, w.y)).monospace().color(INK_3()));
                        }
                        let pen = crate::penhook::state();
                        if pen.in_range && (pen.barrel || pen.eraser) {
                            ui.add_space(SPACE_SM);
                            let t = if pen.eraser { "펜 지우개" } else { "펜 버튼" };
                            ui.label(egui::RichText::new(t).color(ACCENT))
                                .on_hover_text(format!("pen 0x{:X} · pointer 0x{:X}", pen.raw_pen, pen.raw_pointer));
                        }
                        if !self.status.is_empty() {
                            ui.add_space(SPACE_SM);
                            ui.label(egui::RichText::new(&self.status).color(INK_2()));
                        }
                    });
                });
            });
    }

    fn section(ui: &mut egui::Ui, title: &str) {
        ui.add_space(SPACE_SM);
        ui.label(egui::RichText::new(title).font(FontId::new(theme::TEXT_LABEL, theme::ui_bold())).color(INK_3()));
        ui.add_space(2.0);
    }

    /// A section title that folds its section; true while open.
    fn fold_section(ui: &mut egui::Ui, title: &str, folded: &mut bool) -> bool {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
        let col = if resp.hovered() { INK_2() } else { INK_3() };
        let c = pos2(r.left() + 4.0, r.center().y);
        let tri = if *folded {
            vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
        } else {
            vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
        };
        ui.painter().add(Shape::convex_polygon(tri, col, Stroke::NONE));
        ui.painter().text(pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, title, FontId::new(theme::TEXT_LABEL, theme::ui_bold()), col);
        if resp.clicked() {
            *folded = !*folded;
        }
        if !*folded {
            ui.add_space(2.0);
        }
        !*folded
    }

    /// Right side, from the edge inwards: layers, the tool's main properties (with
    /// the selection), and a secondary column only where a tool needs one (cameras).
    fn inspector(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let gap = 10.0;
        let pad = theme::SPACE_MD;
        // with the timeline open the columns end above it
        let bottom = if self.doc.timeline.on && !self.ui_hidden { self.tl_h + 64.0 + 10.0 } else { 56.0 };
        let max_h = (screen.height() - SPACE_LG * 2.0 - 2.0 * pad - bottom).max(160.0);
        let card = |w: f32| w + pad * 2.0;
        let layers_w = 232.0;
        egui::Area::new(egui::Id::new("layers_col"))
            .order(egui::Order::Middle)
            .anchor(Align2::RIGHT_TOP, vec2(-SPACE_LG, SPACE_LG))
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::same(pad as i8), theme::RADIUS_CARD, |ui| {
                    ui.set_width(layers_w);
                    Self::section(ui, "레이어");
                    // as tall as the list (last frame's height); scrolls only past the window
                    let room = (max_h - 30.0).max(80.0);
                    let want = self.col_h[2].min(room);
                    let out = egui::ScrollArea::vertical().id_salt("layers").max_height(room).min_scrolled_height(want).show(ui, |ui| {
                        fixed_width(ui, |ui| self.layers_panel(ui));
                    });
                    self.col_h[2] = out.content_size.y;
                });
            });
        let mut x = SPACE_LG + card(layers_w) + gap;
        let main_w = 244.0;
        egui::Area::new(egui::Id::new("inspector"))
            .order(egui::Order::Middle)
            .anchor(Align2::RIGHT_TOP, vec2(-x, SPACE_LG))
            .show(ctx, |ui| {
                let all_folded = self.s.folded[0] && self.s.folded[1];
                // the top stays at the folded spacing, so the title never moves
                let m = egui::Margin { top: (pad * 0.5) as i8, bottom: if all_folded { (pad * 0.5) as i8 } else { pad as i8 }, ..egui::Margin::same(pad as i8) };
                theme::glass(ui, m, theme::RADIUS_CARD, |ui| {
                    ui.set_width(main_w);
                    // floating areas size from the previous frame: ask for last frame's content height
                    let want = self.col_h[0].min(max_h);
                    let out = egui::ScrollArea::vertical().id_salt("props").max_height(max_h).min_scrolled_height(want).show(ui, |ui| {
                        fixed_width(ui, |ui| {
                            if Self::fold_section(ui, "도구", &mut self.s.folded[0]) {
                                self.tool_options(ui);
                                ui.add_space(SPACE_MD);
                            }
                            if Self::fold_section(ui, "선택한 항목", &mut self.s.folded[1]) {
                                self.properties(ui);
                            }
                        });
                    });
                    self.col_h[0] = out.content_size.y;
                });
            });
        x += card(main_w) + gap;
        if self.tool == Tool::Frame {
            let side_w = 244.0;
            egui::Area::new(egui::Id::new("inspector2"))
                .order(egui::Order::Middle)
                .anchor(Align2::RIGHT_TOP, vec2(-x, SPACE_LG))
                .show(ctx, |ui| {
                    let m = egui::Margin { top: (pad * 0.5) as i8, bottom: if self.s.folded[2] { (pad * 0.5) as i8 } else { pad as i8 }, ..egui::Margin::same(pad as i8) };
                    theme::glass(ui, m, theme::RADIUS_CARD, |ui| {
                        ui.set_width(side_w);
                        let want = self.col_h[1].min(max_h);
                        let out = egui::ScrollArea::vertical().id_salt("props2").max_height(max_h).min_scrolled_height(want).show(ui, |ui| {
                            fixed_width(ui, |ui| self.cameras_panel(ui));
                        });
                        self.col_h[1] = out.content_size.y;
                    });
                });
            x += card(side_w) + gap;
        }
        self.right_reserve = x;
    }

    // ------------------------------------------------------------ palette

    fn open_palette(&mut self, at: Pos2) {
        self.open_palette_for(PalTarget::Main, self.s.color, at);
    }

    fn open_palette_for(&mut self, target: PalTarget, color: Color32, at: Pos2) {
        let hint = self.palette.as_ref().map(|p| p.h).unwrap_or(0.08);
        let (h, s, v) = crate::okhsv::from_rgb(color, hint);
        self.palette = Some(Palette {
            target,
            pos: at + vec2(14.0, 14.0),
            h,
            s,
            v,
            orig: color,
            sv_tex: None,
            hue_tex: None,
            rect: Rect::NOTHING,
            born: 0.0,
            sub_center: (h, s, v),
            sub_down: false,
            strip_tex: None,
            patch_tex: None,
            value_tex: None,
        });
    }

    fn apply_pal(&mut self, target: PalTarget, c: Color32) {
        fn layer(doc: &mut Document, id: LayerId) -> Option<&mut LayerKind> {
            doc.get_mut(id).map(|l| &mut l.kind)
        }
        match target {
            PalTarget::Main => {
                self.s.color = c;
                self.recolor_selected();
                return;
            }
            PalTarget::TextColor(id) => {
                if let Some(LayerKind::Text(t)) = layer(&mut self.doc, id) {
                    t.color = c;
                }
            }
            PalTarget::TextBg(id) => {
                if let Some(LayerKind::Text(t)) = layer(&mut self.doc, id) {
                    t.bg = Some(c);
                }
                self.s.text_bg_color = c;
            }
            PalTarget::ShapeColor(id) => {
                if let Some(LayerKind::Shape(s)) = layer(&mut self.doc, id) {
                    s.color = c;
                }
            }
            PalTarget::ShapeFill(id) => {
                if let Some(LayerKind::Shape(s)) = layer(&mut self.doc, id) {
                    s.fill = Some(c);
                }
                self.s.fill_color = c;
            }
            PalTarget::DefaultTextBg => self.s.text_bg_color = c,
            PalTarget::DefaultFill => self.s.fill_color = c,
            PalTarget::ExportBg => {
                if let Some(d) = self.export.as_mut() {
                    d.bg_color = c;
                }
            }
        }
        self.mark();
    }

    fn palette_ui(&mut self, ctx: &egui::Context) {
        let Some(mut p) = self.palette.take() else { return };
        let now = ctx.input(|i| i.time);
        if p.born == 0.0 {
            p.born = now;
        }
        const SQ: f32 = 196.0;
        const N: usize = 96;
        if p.sv_tex.as_ref().is_none_or(|(h, _)| (h - p.h).abs() > 1e-3) {
            let mut px = Vec::with_capacity(N * N);
            for y in 0..N {
                for x in 0..N {
                    px.push(crate::okhsv::to_rgb(p.h, x as f32 / (N - 1) as f32, 1.0 - y as f32 / (N - 1) as f32));
                }
            }
            let img = egui::ColorImage::new([N, N], px);
            match &mut p.sv_tex {
                Some((h, t)) => {
                    t.set(img, egui::TextureOptions::LINEAR);
                    *h = p.h;
                }
                None => p.sv_tex = Some((p.h, ctx.load_texture("palette_sv", img, egui::TextureOptions::LINEAR))),
            }
        }
        if p.hue_tex.is_none() {
            let px: Vec<Color32> = (0..256).map(|i| crate::okhsv::to_rgb(i as f32 / 255.0, 0.9, 0.95)).collect();
            p.hue_tex = Some(ctx.load_texture("palette_hue", egui::ColorImage::new([1, 256], px), egui::TextureOptions::LINEAR));
        }
        if !p.sub_down {
            p.sub_center = (p.h, p.s, p.v);
        }
        let c = p.sub_center;
        let show_near = crate::glass::palette_near();
        GamutMap::Value.view(ctx, &mut p.value_tex, c, "palette_value");
        GamutMap::Chroma.view(ctx, &mut p.strip_tex, c, "palette_chroma");
        if show_near {
            GamutMap::Near.view(ctx, &mut p.patch_tex, c, "palette_near");
        }
        let mut sub_down = false;
        let screen = ctx.content_rect();
        let est = vec2(820.0, 300.0);
        let pos = pos2(
            p.pos.x.min(screen.right() - est.x - 8.0).max(screen.left() + 8.0),
            p.pos.y.min(screen.bottom() - est.y - 8.0).max(screen.top() + 8.0),
        );
        let mut picked: Option<(f32, f32, f32)> = None;
        let mut restore = false;
        let swatch = |ui: &mut egui::Ui, c: Color32, size: f32, ring: bool| -> egui::Response {
            let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
            let pa = ui.painter();
            pa.rect_filled(r, 5.0, c);
            if ring {
                pa.rect_stroke(r.expand(1.5), 6.0, Stroke::new(1.5, INK()), StrokeKind::Outside);
            } else if resp.hovered() {
                pa.rect_stroke(r, 5.0, Stroke::new(1.0, INK_2()), StrokeKind::Outside);
            }
            resp
        };
        let mut recent: Vec<Color32> = self.recent.iter().copied().take(8).collect();
        for c in PALETTE {
            if recent.len() >= 8 {
                break;
            }
            if !recent.contains(c) {
                recent.push(*c);
            }
        }
        let cur = crate::okhsv::to_rgb(p.h, p.s, p.v);
        let area = egui::Area::new(egui::Id::new("palette")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(SPACE_SM, SPACE_SM);
                theme::glass(ui, egui::Margin::same(theme::SPACE_MD as i8), theme::RADIUS_CARD, |ui| ui.vertical(|ui| {
                    ui.horizontal_top(|ui| {
                        let (r, resp) = ui.allocate_exact_size(vec2(SQ, SQ), Sense::click_and_drag());
                        let (_, tex) = p.sv_tex.as_ref().unwrap();
                        ui.painter().image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                        if resp.is_pointer_button_down_on() {
                            if let Some(pp) = resp.interact_pointer_pos() {
                                let s = ((pp.x - r.left()) / SQ).clamp(0.0, 1.0);
                                let v = 1.0 - ((pp.y - r.top()) / SQ).clamp(0.0, 1.0);
                                picked = Some((p.h, s, v));
                            }
                        }
                        let m = pos2(r.left() + p.s * SQ, r.top() + (1.0 - p.v) * SQ);
                        ui.painter().circle_stroke(m, 6.0, Stroke::new(2.0, Color32::WHITE));
                        ui.painter().circle_stroke(m, 7.5, Stroke::new(1.0, Color32::from_black_alpha(160)));

                        let (hr, hresp) = ui.allocate_exact_size(vec2(16.0, SQ), Sense::click_and_drag());
                        ui.painter().image(p.hue_tex.as_ref().unwrap().id(), hr, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                        if hresp.is_pointer_button_down_on() {
                            if let Some(pp) = hresp.interact_pointer_pos() {
                                picked = Some((((pp.y - hr.top()) / SQ).clamp(0.0, 0.999), p.s, p.v));
                            }
                        }
                        let hy = hr.top() + p.h * SQ;
                        ui.painter().rect_stroke(Rect::from_center_size(pos2(hr.center().x, hy), vec2(20.0, 6.0)), 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Middle);
                    });
                    ui.add_space(SPACE_SM);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if swatch(ui, p.orig, 22.0, false).on_hover_text("원래 색으로").clicked() {
                            restore = true;
                        }
                        swatch(ui, cur, 22.0, true);
                        let [r, g, b, _] = cur.to_array();
                        ui.add_space(SPACE_SM);
                        ui.label(egui::RichText::new(format!("#{r:02X}{g:02X}{b:02X}")).monospace().color(INK_2()));
                    });
                    let mut near = show_near;
                    if ui.checkbox(&mut near, egui::RichText::new("비슷한 색 지도").size(theme::TEXT_SMALL).color(INK_2())).changed() {
                        crate::glass::set_palette_near(near);
                        crate::glass::save_settings();
                    }
                    if !recent.is_empty() {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for c in &recent {
                                if swatch(ui, *c, 18.0, false).clicked() {
                                    picked = Some(crate::okhsv::from_rgb(*c, p.h));
                                }
                            }
                        });
                    }
                }));

                // Similar-colour maps: dim gamut ghost + the connected matching region,
                // re-centred and zoomed on it. The reference freezes while dragging.
                let center = p.sub_center;
                let mut maps = vec![
                    ("같은 명도 · 채색", p.value_tex.as_ref().unwrap(), GamutMap::Value, "밝기 고정 · 가로: 색상 · 세로: 채도"),
                    ("같은 채도 · 명암", p.strip_tex.as_ref().unwrap(), GamutMap::Chroma, "채도 고정 · 가로: 색상 · 세로: 밝기"),
                ];
                if show_near {
                    maps.push(("비슷한 색", p.patch_tex.as_ref().unwrap(), GamutMap::Near, "색 거리가 가까운 색 · 가로: 색상 · 세로: 채도"));
                }
                for (title, view, which, tip) in maps {
                    theme::glass(ui, egui::Margin::same(10), theme::RADIUS_CARD, |ui| ui.vertical(|ui| {
                        ui.label(egui::RichText::new(title).size(theme::TEXT_SMALL).color(INK_3())).on_hover_text(tip);
                        let side = SQ + 30.0;
                        let (r, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
                        ui.painter().image(view.tex.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                        let win = view.win;
                        if resp.is_pointer_button_down_on() {
                            sub_down = true;
                            if let Some(pp) = resp.interact_pointer_pos() {
                                let u = ((pp.x - r.left()) / r.width()).clamp(0.0, 0.999);
                                let v = ((pp.y - r.top()) / r.height()).clamp(0.0, 0.999);
                                let k = (v * GamutMap::H as f32) as usize * GamutMap::W + (u * GamutMap::W as f32) as usize;
                                if view.mask.get(k).copied().unwrap_or(false) {
                                    let x = win.x0 + u * (win.x1 - win.x0);
                                    let y = win.y0 + v * (win.y1 - win.y0);
                                    picked = Some(which.pick(center, x, y));
                                }
                            }
                        }
                        let (px, py) = which.plane((win.x0 + win.x1) * 0.5, (p.h, p.s, p.v));
                        let m = pos2(
                            r.left() + (px - win.x0) / (win.x1 - win.x0) * r.width(),
                            r.top() + (py - win.y0) / (win.y1 - win.y0) * r.height(),
                        );
                        if r.expand(2.0).contains(m) {
                            ui.painter().circle_stroke(m, 6.0, Stroke::new(2.0, Color32::WHITE));
                            ui.painter().circle_stroke(m, 7.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
                        }
                    }));
                }
            });
        });
        p.rect = area.response.rect;
        p.sub_down = sub_down;
        let mut new_color = None;
        if let Some((h, s, v)) = picked {
            p.h = h;
            p.s = s;
            p.v = v;
            new_color = Some(crate::okhsv::to_rgb(h, s, v));
        }
        if restore {
            let (h, s, v) = crate::okhsv::from_rgb(p.orig, p.h);
            p.h = h;
            p.s = s;
            p.v = v;
            new_color = Some(p.orig);
        }
        if let Some(c) = new_color {
            self.apply_pal(p.target, c);
        }
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|q| !p.rect.contains(q)));
        if clicked_outside && now - p.born > 0.2 {
            self.remember_color(self.s.color);
        } else {
            self.palette = Some(p);
        }
    }

    // ------------------------------------------------------------ canvas

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        self.canvas_rect = rect;
        if !self.view_init {
            self.view_init = true;
            self.fit_view();
        }
        let resp = ui.allocate_rect(rect, Sense::hover());
        let ctx = ui.ctx().clone();
        if self.ws3d {
            if let Some((sid, cam)) = self.ws_space() {
                self.canvas_3d(&ctx, ui, rect, sid, &cam);
                return;
            }
            self.ws3d = false;
        }
        if self.modal3d.is_some() {
            self.modal_2d(&ctx);
        } else if self.grs2d.is_some() {
            self.grs_flat(&ctx);
        } else {
            self.handle_canvas_input(&ctx, rect, ui.layer_id());
        }

        let painter = ui.painter_at(rect);
        let xf = self.xf();
        theme::draw_ground(&painter, rect, &xf);

        self.tex.begin_frame();
        // Crop mode: ghost of the full photo underneath.
        if self.tool == Tool::Crop {
            if let Some(LayerKind::Image(im)) = self.sel_layer().map(|l| &l.kind).cloned() {
                let im = &im;
                let full = Rect::from_min_size(Pos2::ZERO, im.src.size());
                let corners = [full.left_top(), full.right_top(), full.right_bottom(), full.left_bottom()].map(|p| xf.w2s(im.src_to_world(p)));
                let tex = self.tex.image(&ctx, im);
                painter.add(Shape::mesh(canvas::image_mesh(tex, corners, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(0.3))));
            }
        }
        // while editing, the renderer still draws the text (so blend modes, fonts and the
        // background box look final); only rotated text is hidden for the upright editor
        // constraints again after this frame's input, so a following camera never trails
        crate::anim::apply_constraints(&mut self.doc);
        canvas::draw_doc(&painter, &ctx, &self.doc, &xf, &mut self.tex, None);
        // scissors outline in progress
        if let Some(sc) = &self.scissor {
            let hover = ctx.input(|i| i.pointer.hover_pos()).map(|p| xf.s2w(p));
            let mut pts = sc.pts.clone();
            if let Some(hw) = hover {
                pts.extend(Self::scissor_path(sc, hw));
            }
            let sp: Vec<Pos2> = pts.iter().map(|p| xf.w2s(*p)).collect();
            painter.add(Shape::line(sp, Stroke::new(1.6, ACCENT)));
            painter.circle_stroke(xf.w2s(sc.pts[0]), 5.0, Stroke::new(1.5, ACCENT));
            ctx.request_repaint();
        }
        // the mask being edited shows as a red veil
        if let Some(l) = self.sel.and_then(|id| self.doc.get(id)).filter(|l| l.mask_of.is_some() && l.visible) {
            if let LayerKind::Raster(r) = &l.kind {
                let view_w = Rect::from_min_max(xf.s2w(painter.clip_rect().min), xf.s2w(painter.clip_rect().max));
                for (&key, t) in &r.tiles {
                    let tr = Rect::from_min_size(pos2((key.0 * crate::raster::TILE) as f32, (key.1 * crate::raster::TILE) as f32), vec2(crate::raster::TILE as f32, crate::raster::TILE as f32)).translate(r.offset);
                    if !tr.intersects(view_w) {
                        continue;
                    }
                    let tex = self.tex.tile(&ctx, l.id, key, t);
                    painter.image(tex, xf.rect_w2s(tr), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::from_rgba_unmultiplied(255, 60, 60, 110));
                }
            }
        }

        // in-progress vector stroke
        if let Some(Drag::Vector { id, pts, color, .. }) = &self.drag {
            let off = match self.doc.get(*id).map(|l| &l.kind) {
                Some(LayerKind::Vector(v)) => v.offset,
                _ => Vec2::ZERO,
            };
            let st = VStroke::new(pts.clone(), *color);
            let mut m = egui::Mesh::default();
            canvas::stroke_mesh(&mut m, &st, &xf, off, 1.0);
            painter.add(Shape::mesh(m));
        }

        // ---- render layers: scene above, overlay layer below (cursor, selection, indicator)
        if crate::glass::available() {
            painter.add(crate::glass::scene_marker(ctx.content_rect(), ctx.cumulative_pass_nr()));
        }
        self.draw_overlays(&painter, &xf);
        self.text_editor(&ctx, &xf);
        self.draw_loupe(&ctx, &painter, rect);
        if let Some(g) = &self.grs2d {
            crate::view3d::hud(&painter, rect, &g.label());
            if g.op != crate::view3d::Op::Grab {
                if let Some(hp) = ctx.input(|i| i.pointer.hover_pos()) {
                    painter.extend(Shape::dashed_line(&[xf.w2s(g.center), hp], Stroke::new(1.0, INK_2()), 4.0, 4.0));
                }
            }
            ctx.request_repaint();
        }
        if let Some(m) = &self.modal3d {
            crate::view3d::hud(&painter, rect, &m.label());
            if m.op != crate::view3d::Op::Grab {
                if let Some(hp) = ctx.input(|i| i.pointer.hover_pos()) {
                    painter.extend(Shape::dashed_line(&[m.center, hp], Stroke::new(1.0, INK_2()), 4.0, 4.0));
                }
            }
            ctx.request_repaint();
        }

        let size_anchor = match &self.drag {
            Some(Drag::Size { anchor, .. }) => Some(*anchor),
            _ => None,
        };
        if let Some(a) = size_anchor {
            if let Some(v) = self.tool_size() {
                let rr = if matches!(self.tool, Tool::Text) { 0.0 } else { v * 0.5 * self.zoom };
                painter.circle_stroke(a, rr.max(1.5) + 0.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
                painter.circle_stroke(a, rr.max(1.5), Stroke::new(1.5, ACCENT));
                painter.text(a + vec2(rr + 10.0, 0.0), Align2::LEFT_CENTER, format!("{v:.1} px"), FontId::proportional(13.0), INK());
            }
        }

        // brush cursor
        if let Some(hp) = ctx.input(|i| i.pointer.hover_pos()) {
            if rect.contains(hp) {
                let r = match self.tool {
                    Tool::Brush => Some(self.s.brush_size * 0.5),
                    Tool::Pen => Some(self.s.pen_size * 0.5),
                    Tool::Eraser => Some(self.s.eraser_size * 0.5),
                    _ => None,
                };
                if let Some(r) = r {
                    let rr = (r * self.zoom).max(1.5);
                    painter.circle_stroke(hp, rr + 0.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
                    painter.circle_stroke(hp, rr, Stroke::new(1.0, Color32::from_white_alpha(200)));
                }
                let w = xf.s2w(hp);
                self.cursor_world = Some(w);
                if ctx.layer_id_at(hp).is_none_or(|l| l == ui.layer_id()) {
                    let panning = matches!(self.drag, Some(Drag::Pan));
                    let icon = match self.tool {
                        _ if panning => egui::CursorIcon::Grabbing,
                        Tool::Hand => egui::CursorIcon::Grab,
                        Tool::Brush | Tool::Pen | Tool::Eraser => egui::CursorIcon::None,
                        Tool::Text => egui::CursorIcon::Text,
                        Tool::Select => egui::CursorIcon::Default,
                        _ => egui::CursorIcon::Crosshair,
                    };
                    ctx.set_cursor_icon(icon);
                    if matches!(self.tool, Tool::Brush | Tool::Pen | Tool::Eraser) {
                        painter.circle_filled(hp, 1.5, Color32::WHITE);
                        painter.circle_stroke(hp, 2.5, Stroke::new(1.0, Color32::from_black_alpha(140)));
                    }
                    // small badge naming the active tool next to the pointer
                    if !matches!(self.tool, Tool::Select | Tool::Hand) && !panning {
                        let b = Rect::from_min_size(hp + vec2(14.0, 12.0), vec2(22.0, 22.0));
                        painter.rect_filled(b, 6.0, theme::PAPER_2().gamma_multiply(0.92));
                        painter.rect_stroke(b, 6.0, Stroke::new(1.0, PAPER_4()), StrokeKind::Inside);
                        theme::tool_icon(&painter, self.tool, b.shrink(4.0), INK());
                    }
                }
            }
        }

        self.draw_rail_indicator(&ctx, &painter);
        if crate::glass::available() {
            painter.add(crate::glass::ui_marker(ctx.content_rect(), ctx.cumulative_pass_nr()));
        }

        if self.doc.layers.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "사진을 끌어다 놓거나 Ctrl+V로 붙여넣기",
                FontId::new(18.0, theme::ui_bold()),
                INK_3(),
            );
        }
        let _ = resp;
    }

    /// Amber "selected tool" block in the overlay layer under the tool rail's glass.
    fn draw_rail_indicator(&mut self, ctx: &egui::Context, painter: &egui::Painter) {
        let Some(r) = self.rail_active.filter(|_| !self.ui_hidden && crate::glass::available()) else { return };
        // eased glide (ease-out only: starts at full speed) from wherever it currently is to the active tool
        const DUR: f64 = 0.08;
        let now = ctx.input(|i| i.time);
        let target = r.center().y;
        let (from, to, t0) = self.ind_anim;
        let cur = |t: f64| {
            let k = ((t - t0) / DUR).clamp(0.0, 1.0) as f32;
            from + (to - from) * egui::emath::easing::cubic_out(k)
        };
        if to.is_nan() || (to - target).abs() > 0.01 {
            let start = if to.is_nan() { target } else { cur(now) };
            self.ind_anim = (start, target, now);
        }
        let (from, to, t0) = self.ind_anim;
        let k = ((now - t0) / DUR).clamp(0.0, 1.0) as f32;
        let y = from + (to - from) * egui::emath::easing::cubic_out(k);
        if k < 1.0 {
            ctx.request_repaint();
        }
        let c = pos2(r.center().x, y);
        // softer than a solid chip: half-strength amber, the glass spreads it
        painter.rect_filled(Rect::from_center_size(c, r.size() + vec2(6.0, 6.0)), 11.0, ACCENT.gamma_multiply(0.5));
    }

    fn space_panel(&mut self, ui: &mut egui::Ui) {
        use crate::persp::{AXIS_COLORS, AXIS_NAMES};
        ui.strong("3D 공간");
        ui.small("끌기: 공간 영역 (사진 클릭: 사진에 맞춤) · 선 긋기: 소실선 · 1/2/3 또는 X/Y/Z: 축 · 끝점 끌기 · Delete: 선 지우기");
        let mut pick = None;
        let mut new = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for sp in &self.doc.spaces {
                let name = elide(ui, sp.name.clone(), 150.0);
                if ui.selectable_label(self.space_sel == Some(sp.id) && !self.space_new, name).clicked() {
                    pick = Some(sp.id);
                }
            }
            if ui.selectable_label(self.space_new, "+ 새 공간").clicked() {
                new = true;
            }
        });
        if let Some(id) = pick {
            self.space_sel = Some(id);
            self.space_new = false;
            self.space_line = None;
        }
        if new {
            self.space_new = true;
            self.space_line = None;
        }
        if self.space_new || self.doc.spaces.is_empty() {
            ui.small("캔버스에서 끌어 공간 영역 지정");
            return;
        }
        let Some(i) = self.space_idx() else { return };
        let mut axis_pick = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            ui.label("축");
            for ax in 0..3u8 {
                let n = self.doc.spaces[i].count(ax);
                let txt = egui::RichText::new(format!("{} · {n}", AXIS_NAMES[ax as usize])).color(AXIS_COLORS[ax as usize]);
                if ui.selectable_label(self.space_axis == ax, txt).clicked() {
                    axis_pick = Some(ax);
                }
            }
        });
        if let Some(ax) = axis_pick {
            self.set_space_axis(ax);
        }
        let mut floor_req: Option<f64> = None;
        let mut place_toggle: Option<LayerId> = None;
        let (mut tape_toggle, mut tape_clear) = (false, false);
        let (mut ref_toggle, mut ref_clear) = (false, false);
        let mut ref_metres: Option<f64> = None;
        let sp = &self.doc.spaces[i];
        match crate::persp::solve(sp) {
            Ok(cam) => {
                ui.label(format!(
                    "초점거리 {:.0} mm{} · 시야각 {:.1}°",
                    cam.focal_35mm(),
                    if cam.f_assumed { " (가정)" } else { "" },
                    cam.fov_deg()
                ));
                ui.small(format!("35 mm 환산 · 선 오차 {:.2}° (RMS)", cam.rms_deg));
                // the camera is the origin (eye level = horizon); the floor is a
                // plane this far below it (world units = cm)
                let f0 = self.doc.spaces[i].floor as f32;
                let mut fl = f0;
                // a fixed range and step: a range that follows the value runs away
                let r = ui.add(crate::dial::Dial::new(&mut fl, 1.0..=100_000.0).step(1.0).text("바닥 깊이"));
                if r.changed() && (fl - f0).abs() > 1e-3 {
                    floor_req = Some(fl as f64);
                }
                r.on_hover_text("카메라(눈높이)에서 바닥면까지 · cm");
                // metres only with a reference length (a photo alone has no scale)
                let rl = self.doc.spaces[i].ref_line;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if ui.selectable_label(self.space_ref_draw, "기준 길이").on_hover_text("수평면(바닥·탁자·창틀 등) 위 점에서 시작 · 수직: 높이, 수평: 그 면 위 길이 · 축 자동").clicked() {
                        ref_toggle = true;
                    }
                    if let Some(mut r) = rl {
                        let before = r.metres;
                        ui.spacing_mut().interact_size.x = 56.0;
                        ui.add(crate::calc::drag(&mut r.metres).range(0.001..=100000.0).speed(0.01).max_decimals(3).suffix(" m"));
                        if r.metres != before {
                            ref_metres = Some(r.metres);
                        }
                        if ui.small_button("지우기").clicked() {
                            ref_clear = true;
                        }
                    }
                });
                match rl.map(|r| r.camera_height_m(&cam)) {
                    Some(Some(m)) => {
                        ui.horizontal(|ui| {
                            ui.small(format!("카메라 높이 ≈ {m:.2} m (기준면에서)"));
                        });
                        // the floor plane onto the plane the reference stands on (cm)
                        let on_floor = (self.doc.spaces[i].floor - m * 100.0).abs() < 0.5;
                        if !on_floor && ui.button("기준면을 공간 바닥으로").on_hover_text("바닥 깊이 = 기준선이 놓인 수평면까지 (실측 cm)").clicked() {
                            floor_req = Some(m * 100.0);
                        }
                    }
                    Some(None) => {
                        ui.small(egui::RichText::new("기준선이 지평선 위: 측정 불가").color(INK_3()));
                    }
                    None => {}
                }
                // AR-style placing: the selected layer stands where the floor is clicked
                // tape measure: metres against the reference length
                let tape = self.doc.spaces[i].tape;
                let tape_m = self.doc.spaces[i].tape_metres(&cam);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if ui.selectable_label(self.space_tape_draw, "줄자").on_hover_text("기준면 위 점에서 끌기 · 수평: 거리, 수직: 높이").clicked() {
                        tape_toggle = true;
                    }
                    if tape.is_some() {
                        match tape_m {
                            Some(m) => ui.label(format!("{m:.2} m")),
                            None => ui.label(egui::RichText::new("기준 길이 필요").color(INK_3())),
                        };
                        if ui.small_button("지우기").clicked() {
                            tape_clear = true;
                        }
                    }
                });
                let placeable = self.sel.and_then(|id| self.doc.get(id)).filter(|l| l.mask_of.is_none() && !l.locked).map(|l| l.id);
                if let Some(lid) = placeable {
                    let armed = self.space_place == Some(lid);
                    if ui.selectable_label(armed, "바닥에 세우기").on_hover_text("선택한 레이어 · 사진 속 바닥 클릭").clicked() {
                        place_toggle = Some(lid);
                    }
                }
            }
            Err(e) => {
                ui.small(egui::RichText::new(e).color(INK_2()));
            }
        }
        let all3 = (0..3).all(|a| sp.count(a) >= 2);
        let (mut grid, mut fpp) = (sp.grid, sp.free_pp);
        let mut ch = ui.checkbox(&mut grid, "격자").changed();
        let mut light = sp.light;
        let mut lon = light.is_some();
        let mut lch = ui.checkbox(&mut lon, "조명").on_hover_text("두께 있는 객체에 방향광 음영").changed();
        if lon {
            let l = light.get_or_insert_with(Default::default);
            for (k, ax) in ["X", "Y", "Z"].iter().enumerate() {
                let mut d = l.rot[k] as f32;
                if ui.add(crate::dial::Dial::new(&mut d, -180.0..=180.0).step(1.0).text(format!("빛 {ax} °")).default(crate::persp::Light::default().rot[k] as f32)).changed() {
                    l.rot[k] = d as f64;
                    lch = true;
                }
            }
            lch |= ui.add(crate::dial::Dial::new(&mut l.ambient, 0.0..=1.0).text("어두운 면").default(0.35)).changed();
        } else {
            light = None;
        }
        ch |= ui.add_enabled(all3, egui::Checkbox::new(&mut fpp, "주점도 추정 (세 축 모두 2개 이상)")).changed();
        let img_b = self.sel.and_then(|id| self.doc.get(id)).filter(|l| matches!(l.kind, LayerKind::Image(_))).and_then(|l| l.bounds());
        let img_follow = self.sel.and_then(|id| self.doc.get(id)).and_then(|l| match &l.kind {
            LayerKind::Image(im) => Some(crate::persp::SpaceFollow { layer: l.id, last: [im.center.x, im.center.y, im.scale, im.rotation] }),
            _ => None,
        });
        let follow_name = sp.follow.and_then(|f| self.doc.get(f.layer)).map(|l| l.display_name());
        let mut unfollow = false;
        if let Some(n) = &follow_name {
            ui.horizontal(|ui| {
                let mut on = true;
                // a long photo name is cut, not allowed to widen the panel
                let label = elide(ui, format!("사진 따라가기 · {n}"), ui.available_width() - 28.0);
                if ui.checkbox(&mut on, label).on_hover_text(format!("{n}\n사진을 옮기거나 크기·회전을 바꾸면 공간 전체가 함께")).changed() {
                    unfollow = true;
                }
            });
        }
        let has_mask = sp.mask.is_some();
        let (mut mask_draw, mut mask_clear) = (false, false);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            ui.label(if has_mask { "마스크: 지정됨" } else { "마스크: 공간 영역 전체" });
            mask_draw = ui
                .selectable_label(self.space_mask_draw, "마스크 그리기")
                .on_hover_text("공간이 보일 영역 (카메라 계산과 별개)")
                .clicked();
            mask_clear = ui.add_enabled(has_mask, egui::Button::new("마스크 해제")).clicked();
        });
        let walls: Vec<(u64, String)> = sp.walls.iter().map(|w| (w.id, w.name.clone())).collect();
        let (mut wall_pick, mut wall_add, mut wall_base) = (None, false, false);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            ui.label("벽");
            for (wid, name) in &walls {
                let short = elide(ui, name.clone(), 150.0);
                if ui.selectable_label(self.wall_sel == Some(*wid), short).clicked() {
                    wall_pick = Some(*wid);
                }
            }
            wall_add = ui
                .selectable_label(self.wall_new, "+ 벽")
                .on_hover_text("끌어서 사각형 → 네 모서리를 사진 속 평면에 맞춤 · 넣은 레이어는 벽 좌표로 이동")
                .clicked();
            wall_base = ui.selectable_label(self.wall_base_new, "+ 밑선 벽").on_hover_text("바닥 위 벽 밑선을 끌기 → 그 선에 선 수직 벽").clicked();
        });
        let mut wall_edit: Option<(f64, bool, bool)> = None;
        if let Some(w) = self.wall_sel.and_then(|id| sp.walls.iter().find(|w| w.id == id)) {
            let (mut off, mut snap, mut wdel, mut ch2) = (w.offset as f32, w.snap_axis, false, false);
            ch2 |= ui
                .add(crate::dial::Dial::new(&mut off, -5000.0..=5000.0).step(5.0).text("벽 거리").default(0.0))
                .on_hover_text("벽을 앞뒤로 옮겨 (바닥 경계로 잡은 위치가 어긋날 때)")
                .changed();
            ui.horizontal(|ui| {
                ch2 |= ui.checkbox(&mut snap, "공간 축에 스냅").on_hover_text("공간 축과 15° 이내면 그 축으로").changed();
                wdel = ui.button("벽 삭제").clicked();
            });
            if ch2 || wdel {
                wall_edit = Some((off as f64, snap, wdel));
            }
            if let Some(f) = crate::persp::solve(sp).ok().and_then(|c| crate::persp::wall_frame(&c, w)) {
                ui.small(format!("모서리 직각 오차 {:.1}°", f.skew_deg.abs()));
            }
        }
        let (mut clear, mut fit, mut del) = (false, false, false);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            clear = ui.button("선 모두 지우기").clicked();
            fit = ui.add_enabled(img_b.is_some(), egui::Button::new("선택한 사진에 맞추기")).clicked();
            del = ui.button("공간 삭제").clicked();
        });
        if mask_draw {
            self.space_mask_draw = !self.space_mask_draw;
            if self.space_mask_draw {
                self.status = "캔버스에서 끌어 마스크 지정".into();
            }
        }
        let facing = crate::persp::cameras(&self.doc.spaces).get(&self.doc.spaces[i].id).map(crate::persp::Light::facing);
        let s = &mut self.doc.spaces[i];
        if let Some(f) = floor_req {
            s.floor = f.max(1.0);
            ch = true;
        }
        if tape_clear {
            s.tape = None;
            ch = true;
        }
        if tape_toggle {
            self.space_tape_draw = !self.space_tape_draw;
            if self.space_tape_draw {
                self.space_ref_draw = false;
                self.status = "기준면 위 점에서 끌어 재기".into();
            }
        }
        if let Some(lid) = place_toggle {
            self.space_place = if self.space_place == Some(lid) { None } else { Some(lid) };
            if self.space_place.is_some() {
                self.space_ref_draw = false;
                self.space_mask_draw = false;
                self.status = "바닥을 클릭해 세우기".into();
            }
        }
        if let (Some(m), Some(r)) = (ref_metres, s.ref_line.as_mut()) {
            r.metres = m;
            ch = true;
        }
        if ref_clear {
            s.ref_line = None;
            ch = true;
        }
        if ref_toggle {
            self.space_ref_draw = !self.space_ref_draw;
            if self.space_ref_draw {
                self.space_mask_draw = false;
                self.status = "수평면 위 점에서 끌어 기준 길이".into();
            }
        }
        if mask_clear {
            s.mask = None;
            ch = true;
        }
        if ch {
            s.grid = grid;
            s.free_pp = fpp;
        }
        if lch {
            let fresh = s.light.is_none() && light.is_some();
            s.light = light;
            if fresh {
                if let Some(l) = facing {
                    s.light = Some(l);
                }
            }
            ch = true;
        }
        if clear && !s.lines.is_empty() {
            s.lines.clear();
            self.space_line = None;
            ch = true;
        }
        if let (true, Some(b)) = (fit, img_b) {
            s.frame = b;
            s.follow = img_follow;
            ch = true;
        }
        if unfollow {
            s.follow = None;
            ch = true;
        }
        if let Some(id) = wall_pick {
            self.wall_sel = Some(id);
            self.wall_new = false;
        }
        if wall_base {
            self.wall_base_new = !self.wall_base_new;
            if self.wall_base_new {
                self.wall_new = false;
                self.status = "바닥 위 벽 밑선 끌기".into();
            }
        }
        if wall_add {
            self.wall_new = !self.wall_new;
            if self.wall_new {
                self.status = "캔버스에서 끌어 벽 사각형 지정".into();
            }
        }
        if let (Some((off, snap, wdel)), Some(wid)) = (wall_edit, self.wall_sel) {
            if wdel {
                self.doc.spaces[i].walls.retain(|w| w.id != wid);
                // its layers stay where they are, no longer tied to the wall
                for l in &mut self.doc.layers {
                    if let Some(pl) = l.place.as_mut().filter(|p| p.wall == Some(wid)) {
                        pl.wall = None;
                    }
                }
                self.wall_sel = None;
            } else if let Some(w) = self.doc.spaces[i].walls.iter_mut().find(|w| w.id == wid) {
                w.offset = off;
                w.snap_axis = snap;
            }
            ch = true;
        }
        if del {
            self.doc.spaces.remove(i);
            self.space_sel = self.doc.spaces.last().map(|s| s.id);
            self.space_line = None;
            ch = true;
        }
        if ch {
            self.mark();
        }
    }

    fn draw_spaces(&self, painter: &egui::Painter, xf: &Xf) {
        use crate::persp::{AXIS_COLORS, AXIS_NAMES, V3};
        let editing = self.tool == Tool::Space;
        // outside the space tool only the grid of the selected layer's space shows
        let focus = self.sel_layer().and_then(|l| l.place.as_ref()).map(|p| p.space);
        let focus_wall = self.sel_layer().and_then(|l| l.place.as_ref()).and_then(|p| p.wall);
        if !editing && focus.is_none() {
            return;
        }
        for s in &self.doc.spaces {
            if !editing && Some(s.id) != focus {
                continue;
            }
            let active = !editing || (self.space_sel == Some(s.id) && !self.space_new);
            let sr = xf.rect_w2s(s.frame);
            if editing {
                let col = if active { INK() } else { INK_3() };
                let pts = vec![sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom(), sr.left_top()];
                painter.extend(Shape::dashed_line(&pts, Stroke::new(1.2, col), 6.0, 4.0));
                painter.text(sr.left_top() - vec2(0.0, 4.0), Align2::LEFT_BOTTOM, &s.name, FontId::proportional(12.0), col);
            }
            if !active {
                continue;
            }
            let cam = crate::persp::solve(s).ok();
            let pc = painter.with_clip_rect(xf.rect_w2s(s.clip()).intersect(painter.clip_rect()));
            if let Some(cam) = &cam {
                let seg = |a: V3, b: V3| cam.project_segment(a, b).map(|(p, q)| [xf.w2s(p), xf.w2s(q)]);
                if s.grid {
                    // a layer on a wall shows that wall's grid; otherwise the depth grid
                    let wall = focus_wall.filter(|_| !editing).and_then(|wid| s.walls.iter().find(|w| w.id == wid));
                    let lines = match wall {
                        Some(w) => crate::persp::wall_grid(cam, w),
                        None => crate::persp::depth_grid(cam, s.frame),
                    };
                    for (a, b, wgt) in lines {
                        if let Some(sgm) = seg(a, b) {
                            let c = Color32::from_white_alpha([36, 72, 150][wgt as usize]);
                            pc.line_segment(sgm, Stroke::new(if wgt == 2 { 1.4 } else { 1.0 }, c));
                        }
                    }
                }
                if !editing {
                    continue;
                }
                // horizon: the image of the ground plane's line at infinity, l = K^-T n
                let n = cam.axis_dir(2);
                let l = [n[0] / cam.f, n[1] / cam.f, -(cam.c[0] * n[0] + cam.c[1] * n[1]) / cam.f + n[2]];
                if l[1].abs() > 1e-12 {
                    let y_at = |x: f64| -(l[0] * x + l[2]) / l[1];
                    let (x0, x1) = (s.frame.left() as f64, s.frame.right() as f64);
                    let a = xf.w2s(pos2(x0 as f32, y_at(x0) as f32));
                    let b = xf.w2s(pos2(x1 as f32, y_at(x1) as f32));
                    let hc = Color32::from_rgb(255, 196, 90);
                    painter.extend(Shape::dashed_line(&[a, b], Stroke::new(1.2, hc), 10.0, 6.0));
                    painter.text(b + vec2(-4.0, -4.0), Align2::RIGHT_BOTTOM, "지평선", FontId::proportional(11.0), hc);
                }
                for ax in 0..3 {
                    if s.count(ax as u8) == 0 {
                        continue;
                    }
                    if let Some(v) = cam.vp(ax) {
                        let v = xf.w2s(v);
                        painter.circle_stroke(v, 6.0, Stroke::new(1.5, AXIS_COLORS[ax]));
                        painter.text(v + vec2(8.0, -8.0), Align2::LEFT_BOTTOM, AXIS_NAMES[ax], FontId::proportional(12.0), AXIS_COLORS[ax]);
                    }
                }
            }
            if !editing {
                continue;
            }
            if let Some(m) = s.mask {
                let mr = xf.rect_w2s(m);
                let pts = vec![mr.left_top(), mr.right_top(), mr.right_bottom(), mr.left_bottom(), mr.left_top()];
                painter.extend(Shape::dashed_line(&pts, Stroke::new(1.4, ACCENT), 4.0, 4.0));
                painter.text(mr.left_bottom() + vec2(0.0, 4.0), Align2::LEFT_TOP, "마스크", FontId::proportional(11.0), ACCENT);
            }
            if let Some(r) = s.ref_line {
                let (a, b) = (xf.w2s(r.a), xf.w2s(r.b));
                let col = AXIS_COLORS[(r.axis as usize).min(2)].lerp_to_gamma(Color32::WHITE, 0.45);
                painter.line_segment([a, b], Stroke::new(4.0, Color32::from_black_alpha(140)));
                painter.line_segment([a, b], Stroke::new(2.0, col));
                // end ticks across the line
                let n = (b - a).normalized().rot90() * 6.0;
                for p in [a, b] {
                    painter.line_segment([p - n, p + n], Stroke::new(2.0, col));
                }
                painter.text(a.lerp(b, 0.5) + n * 1.6, Align2::CENTER_CENTER, format!("{} m", trim_num(r.metres)), FontId::proportional(12.0), col);
            }
            if let Some(r) = s.tape {
                let (a, b) = (xf.w2s(r.a), xf.w2s(r.b));
                let col = Color32::from_rgb(255, 214, 90);
                painter.extend(Shape::dashed_line(&[a, b], Stroke::new(2.0, col), 6.0, 4.0));
                let n = (b - a).normalized().rot90() * 5.0;
                for p in [a, b] {
                    painter.line_segment([p - n, p + n], Stroke::new(2.0, col));
                }
                let label = cam.as_ref().and_then(|c| s.tape_metres(c)).map_or("?".to_string(), |m| format!("{m:.2} m"));
                painter.text(a.lerp(b, 0.5) + n * 2.0, Align2::CENTER_CENTER, label, FontId::proportional(12.0), col);
            }
            for (i, l) in s.lines.iter().enumerate() {
                let c = AXIS_COLORS[(l.axis as usize).min(2)];
                let (a, b) = (xf.w2s(l.a), xf.w2s(l.b));
                if let Some(v) = cam.as_ref().and_then(|cam| cam.vp(l.axis as usize)) {
                    let v = xf.w2s(v);
                    let near = if a.distance(v) < b.distance(v) { a } else { b };
                    painter.line_segment([near, v], Stroke::new(1.0, c.gamma_multiply(0.3)));
                }
                if self.space_line == Some(i) {
                    painter.line_segment([a, b], Stroke::new(6.0, Color32::from_white_alpha(70)));
                }
                painter.line_segment([a, b], Stroke::new(2.0, c));
                for p in [a, b] {
                    painter.circle_filled(p, 4.0, c);
                    painter.circle_stroke(p, 4.0, Stroke::new(1.0, Color32::WHITE));
                }
                if let Some(e) = cam.as_ref().and_then(|cam| cam.line_err.get(i).copied()).filter(|e| e.is_finite() && *e > 1.0) {
                    painter.text(a.lerp(b, 0.5) + vec2(0.0, -6.0), Align2::CENTER_BOTTOM, format!("{e:.1}°"), FontId::proportional(11.0), Color32::from_rgb(255, 120, 120));
                }
            }
            for wl in &s.walls {
                let sel = self.wall_sel == Some(wl.id);
                let pts: Vec<Pos2> = wl.corners.iter().map(|p| xf.w2s(*p)).collect();
                let col = crate::view3d::WALL_COLOR;
                painter.add(Shape::convex_polygon(pts.clone(), col.gamma_multiply(if sel { 0.16 } else { 0.08 }), Stroke::NONE));
                if sel {
                    if let Some(cam) = &cam {
                        for (a, b, wgt) in crate::persp::wall_grid(cam, wl) {
                            if let Some((p, q)) = cam.project_segment(a, b) {
                                pc.line_segment([xf.w2s(p), xf.w2s(q)], Stroke::new(1.0, col.gamma_multiply([0.2, 0.35, 0.7][wgt as usize])));
                            }
                        }
                    }
                }
                painter.add(Shape::closed_line(pts.clone(), Stroke::new(if sel { 1.8 } else { 1.2 }, col)));
                for p in &pts {
                    painter.circle_filled(*p, 4.5, col);
                    painter.circle_stroke(*p, 4.5, Stroke::new(1.0, Color32::WHITE));
                }
                painter.text(pts[0] + vec2(6.0, -6.0), Align2::LEFT_BOTTOM, &wl.name, FontId::proportional(11.0), col);
            }
            for p in rect_handles(s.frame) {
                handle_box(painter, xf.w2s(p), INK());
            }
        }
    }


    fn draw_overlays(&self, painter: &egui::Painter, xf: &Xf) {
        self.draw_spaces(painter, xf);
        let accent = ACCENT;
        // canvas cameras
        let active = crate::anim::active_camera(&self.doc, self.doc.timeline.time).map(|c| c.id);
        for c in &self.doc.cameras {
            let sr = xf.rect_w2s(c.rect);
            let sel = self.cam_sel == Some(c.id) && self.tool == Tool::Frame;
            let corners = c.corners().map(|p| xf.w2s(p));
            let alive = !self.doc.timeline.on || c.anim.alive(self.doc.timeline.time);
            let col = if alive { CAM_COLOR } else { CAM_COLOR.gamma_multiply(0.35) };
            let pts = vec![corners[0], corners[1], corners[2], corners[3], corners[0]];
            painter.extend(Shape::dashed_line(&pts, Stroke::new(if sel { 2.0 } else { 1.4 }, col), 10.0, 4.0));
            let tag = if Some(c.id) == active && self.doc.timeline.on { format!("● {}", c.name) } else { c.name.clone() };
            let follow = if c.follow.is_some() { " ⛓" } else { "" };
            painter.text(corners[0] + vec2(0.0, -4.0), Align2::LEFT_BOTTOM, format!("{tag}{follow}  {:.0}×{:.0}", c.rect.width(), c.rect.height()), FontId::proportional(12.0), col);
            let _ = sr;
            if sel && c.rot.abs() < 1e-6 {
                for p in rect_handles(c.rect) {
                    handle_box(painter, xf.w2s(p), CAM_COLOR);
                }
            }
        }
        // export frame
        if let Some(fr) = self.doc.export_frame {
            let sr = xf.rect_w2s(fr);
            if self.tool == Tool::Frame {
                let dim = Color32::from_black_alpha(110);
                let c = painter.clip_rect();
                for r in [
                    Rect::from_min_max(c.min, pos2(c.max.x, sr.min.y)),
                    Rect::from_min_max(pos2(c.min.x, sr.max.y), c.max),
                    Rect::from_min_max(pos2(c.min.x, sr.min.y), pos2(sr.min.x, sr.max.y)),
                    Rect::from_min_max(pos2(sr.max.x, sr.min.y), pos2(c.max.x, sr.max.y)),
                ] {
                    painter.rect_filled(r, 0.0, dim);
                }
                for p in rect_handles(fr) {
                    handle_box(painter, xf.w2s(p), INK());
                }
            }
            let pts = vec![sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom(), sr.left_top()];
            painter.extend(Shape::dashed_line(&pts, Stroke::new(1.5, INK()), 8.0, 5.0));
            painter.text(
                sr.left_top() - vec2(0.0, 4.0),
                Align2::LEFT_BOTTOM,
                format!("내보낼 영역 {:.0}×{:.0}", fr.width(), fr.height()),
                FontId::proportional(12.0),
                INK(),
            );
        }
        // selected pen strokes and brush pixel boxes
        if !matches!(self.drag, Some(Drag::MoveMany { .. }) | Some(Drag::Group { .. })) {
            for (id, idx) in &self.sub.strokes {
                if let Some(LayerKind::Vector(v)) = self.doc.get(*id).map(|l| &l.kind) {
                    for &i in idx {
                        if let Some(st) = v.strokes.get(i) {
                            let sr = xf.rect_w2s(st.bounds.translate(v.offset)).expand(2.0);
                            painter.rect_stroke(sr, 2.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.8)), StrokeKind::Outside);
                        }
                    }
                }
            }
            for (_, polys) in &self.sub.pixels {
                for poly in polys {
                    let mut pts: Vec<Pos2> = poly.iter().map(|p| xf.w2s(*p)).collect();
                    pts.push(pts[0]);
                    painter.extend(Shape::dashed_line(&pts, Stroke::new(1.5, Color32::WHITE), 5.0, 5.0));
                    painter.extend(Shape::dashed_line_with_offset(&pts, Stroke::new(1.5, Color32::BLACK), &[5.0], &[5.0], 5.0));
                }
            }
        }
        if let Some(Drag::Lasso { pts }) = &self.drag {
            let mut sp: Vec<Pos2> = pts.iter().map(|p| xf.w2s(*p)).collect();
            if sp.len() > 1 {
                sp.push(sp[0]);
                painter.add(Shape::line(sp, Stroke::new(1.5, ACCENT)));
            }
        }
        if let Some(Drag::Marquee { start, cur, .. }) = &self.drag {
            let r = xf.rect_w2s(Rect::from_two_pos(*start, *cur));
            painter.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.08));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
        }
        if let Some(Drag::Group { box_, center, .. }) = &self.drag {
            // original box, carried along with the current transform of the group
            let pts: Vec<Pos2> = [box_.left_top(), box_.right_top(), box_.right_bottom(), box_.left_bottom(), box_.left_top()]
                .iter()
                .map(|p| xf.w2s(*p))
                .collect();
            painter.extend(Shape::dashed_line(&pts, Stroke::new(1.0, INK_3()), 4.0, 4.0));
            painter.circle_filled(xf.w2s(*center), 3.0, ACCENT);
        } else if let Some(b) = self.group_box().filter(|_| self.tool == Tool::Select) {
            let r = xf.rect_w2s(b);
            // a lone text/shape keeps its own outline; only the handles are added
            if self.single_handles().is_none() {
                painter.rect_stroke(r, 0.0, Stroke::new(1.5, ACCENT), StrokeKind::Middle);
            }
            let gh = self.group_handles();
            for (h, p) in &gh {
                if *h == 34 {
                    // stem from the outline's top middle (the turned one for text)
                    let foot = if gh.len() == 5 { gh[0].1 + (gh[1].1 - gh[0].1) * 0.5 } else { r.center_top() };
                    painter.line_segment([foot, *p], Stroke::new(1.0, ACCENT));
                    painter.circle_filled(*p, 5.0, Color32::WHITE);
                    painter.circle_stroke(*p, 5.0, Stroke::new(1.5, ACCENT));
                } else {
                    handle_box(painter, *p, ACCENT);
                }
            }
        }
        let cams = crate::persp::cameras(&self.doc.spaces);
        for id in &self.extra_sel {
            if let Some(q) = self.doc.get(*id).and_then(|l| l.shown_quad(&cams)) {
                let mut pts: Vec<Pos2> = q.iter().map(|p| xf.w2s(*p)).collect();
                pts.push(pts[0]);
                painter.extend(Shape::dashed_line(&pts, Stroke::new(1.0, ACCENT), 5.0, 4.0));
            }
        }
        let Some(l) = self.sel_layer() else { return };
        if self.editing == Some(l.id) {
            return;
        }
        if l.place.is_some() {
            if let Some(q) = l.shown_quad(&cams) {
                let mut pts: Vec<Pos2> = q.iter().map(|p| xf.w2s(*p)).collect();
                pts.push(pts[0]);
                painter.add(Shape::line(pts, Stroke::new(1.5, accent)));
            }
            return;
        }
        match &l.kind {
            LayerKind::Image(im) => {
                if self.tool == Tool::Crop {
                    let c = im.crop;
                    let pts: Vec<Pos2> = [c.left_top(), c.right_top(), c.right_bottom(), c.left_bottom(), c.left_top()]
                        .iter()
                        .map(|p| xf.w2s(im.src_to_world(*p)))
                        .collect();
                    painter.add(Shape::line(pts, Stroke::new(1.5, INK())));
                    for (_, p) in self.handles() {
                        handle_box(painter, p, INK());
                    }
                } else {
                    let mut pts: Vec<Pos2> = im.corners().iter().map(|p| xf.w2s(*p)).collect();
                    pts.push(pts[0]);
                    painter.add(Shape::line(pts, Stroke::new(1.5, accent)));
                    if self.tool == Tool::Select && !l.locked {
                        let (_, v) = im.axes();
                        let top = xf.w2s(im.center - v * im.half().y);
                        let rh = self.rot_handle(im);
                        painter.line_segment([top, rh], Stroke::new(1.0, accent));
                        painter.circle_filled(rh, 5.0, Color32::WHITE);
                        painter.circle_stroke(rh, 5.0, Stroke::new(1.5, accent));
                        for (h, p) in self.handles() {
                            if h < 4 {
                                handle_box(painter, p, accent);
                            }
                        }
                    }
                }
            }
            LayerKind::Text(t) if t.rotation != 0.0 => {
                // the turned box itself
                let tr = crate::text::layout(t).rect(t.pos);
                let c = tr.center();
                let mut pts: Vec<Pos2> = [tr.left_top(), tr.right_top(), tr.right_bottom(), tr.left_bottom()].iter().map(|p| xf.w2s(crate::doc::rot_about(*p, c, t.rotation))).collect();
                pts.push(pts[0]);
                painter.extend(Shape::dashed_line(&pts, Stroke::new(1.0, accent), 5.0, 4.0));
                for (_, p) in self.handles() {
                    handle_box(painter, p, accent);
                }
            }
            _ => {
                if let Some(b) = l.bounds() {
                    let sr = xf.rect_w2s(b).expand(2.0);
                    let pts = vec![sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom(), sr.left_top()];
                    painter.extend(Shape::dashed_line(&pts, Stroke::new(1.0, accent), 5.0, 4.0));
                }
                for (_, p) in self.handles() {
                    handle_box(painter, p, accent);
                }
            }
        }
    }

    fn text_editor(&mut self, ctx: &egui::Context, xf: &Xf) {
        let Some(id) = self.editing else { return };
        let Some(LayerKind::Text(t)) = self.doc.get(id).map(|l| &l.kind) else {
            self.editing = None;
            return;
        };
        let t = t.clone();
        let lay = crate::text::layout(&t);
        let r = xf.rect_w2s(lay.rect(t.pos));
        // The renderer draws the text underneath (as placed, rotation included); the
        // invisible editor only takes input. Caret and selection are drawn here,
        // rotated with the text.
        let rendered = true;
        let rot = t.rotation;
        let c = r.center();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("text_edit_bg")));
        let frame: Vec<Pos2> = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
            .iter()
            .map(|p| crate::doc::rot_about(c + (*p - c) * ((r.width() + 6.0) / r.width().max(1.0)), c, rot))
            .collect();
        painter.add(Shape::closed_line(frame, Stroke::new(1.0, ACCENT)));
        let fam = self.egui_family(ctx, &t);
        // renderer and editor both treat the size as the em in pixels
        let font = FontId::new((t.size * xf.zoom).max(2.0), fam);
        let pad = lay.pad * xf.zoom;
        // centred / right text: the editor lays its rows out across the same width
        let aligned = t.align != crate::doc::TextAlign::Left;
        let content_px = (lay.size.x - lay.pad * 2.0) * xf.zoom;
        let width = if t.wrap > 0.0 {
            (t.wrap - lay.pad * 2.0) * xf.zoom
        } else if aligned {
            content_px + 0.5
        } else {
            content_px + t.size * xf.zoom * 2.0
        };
        let halign = match t.align {
            crate::doc::TextAlign::Left => egui::Align::LEFT,
            crate::doc::TextAlign::Center => egui::Align::Center,
            crate::doc::TextAlign::Right => egui::Align::RIGHT,
        };
        let mut text = t.text.clone();
        let mut lost = false;
        egui::Area::new(egui::Id::new("text_edit_area"))
            .fixed_pos(r.min + vec2(pad, pad))
            // below the floating panels (they are Middle), and never pushed back on-screen
            .order(egui::Order::Background)
            .constrain(false)
            .show(ctx, |ui| {
                self.drop_stale_ime(ui.ctx());
                // Without a wrap width the box grows sideways while typing: lay the text
                // out unwrapped instead of wrapping at last frame's width.
                let wrap_px = if t.wrap > 0.0 || aligned { width.max(10.0) } else { f32::INFINITY };
                let lcol = if !rendered {
                    t.color
                } else if std::env::var_os("DV_EDIT_DEBUG").is_some() {
                    Color32::from_rgba_unmultiplied(0, 160, 255, 150) // shows editor glyphs for alignment checks
                } else {
                    Color32::TRANSPARENT
                };
                let lfont = font.clone();
        let line_px = lay.line_h * xf.zoom;
                let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _w: f32| {
                    let mut job = egui::text::LayoutJob::simple(buf.as_str().to_owned(), lfont.clone(), lcol, wrap_px);
                    job.halign = halign;
                    // the renderer's line spacing, so rows (and clicks on them) line up
                    for sec in &mut job.sections {
                        sec.format.line_height = Some(line_px);
                    }
                    ui.ctx().fonts_mut(|f| f.layout_job(job))
                };
                let te = egui::TextEdit::multiline(&mut text)
                    .font(font)
                    .text_color(lcol)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::ZERO)
                    .desired_width(width.max(10.0))
                    .desired_rows(1)
                    .clip_text(false)
                    .layouter(&mut layouter)
                    .hint_text("글자 입력");
                // hide egui's own caret/selection: they would be upright
                ui.visuals_mut().text_cursor.stroke.color = Color32::TRANSPARENT;
                ui.visuals_mut().selection.bg_fill = Color32::TRANSPARENT;
                let mut out = te.show(ui);
                let resp = out.response.response.clone();
                // Clicks and drags place the caret from the renderer's layout (egui's
                // own rows are rounded and never rotated): the pointer is turned back
                // into the text's frame and matched to the nearest caret spot.
                let (ptr, pressed, down) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_down()));
                let spot = ptr.map(|p| crate::doc::rot_about(p, c, -rot)).filter(|p| r.expand(4.0).contains(*p)).map(|p| lay.caret_at(pos2((p.x - r.min.x) / xf.zoom - lay.pad, (p.y - r.min.y) / xf.zoom - lay.pad)));
                if pressed {
                    // shift extends from the selection's fixed end
                    let shift = ui.input(|i| i.modifiers.shift);
                    self.edit_press = if shift { out.cursor_range.map(|r| r.secondary.index.0).or(spot) } else { spot };
                }
                if !down && !pressed {
                    self.edit_press = None;
                }
                // a click on rotated text can miss egui's upright rows: keep the focus
                if spot.is_some() && ui.input(|i| i.pointer.primary_released()) {
                    resp.request_focus();
                }
                if let (Some(a), Some(b), true) = (self.edit_press, spot, down || pressed) {
                    use egui::text::{CCursor, CCursorRange};
                    let range = CCursorRange::two(CCursor::new(a), CCursor::new(b));
                    out.state.cursor.set_char_range(Some(range));
                    out.state.clone().store(ui.ctx(), resp.id);
                    out.cursor_range = Some(range);
                    resp.request_focus();
                }
                if self.edit_focus {
                    resp.request_focus();
                    self.edit_focus = false;
                }
                lost = resp.lost_focus();
                if std::env::var_os("DV_EDIT_DEBUG").is_some() {
                    if std::env::var_os("DV_EDIT_ROWS").is_some() {
                        let ours: Vec<f32> = (0..out.galley.rows.len()).map(|i| r.min.y + (lay.pad + i as f32 * lay.line_h) * xf.zoom).collect();
                        let theirs: Vec<f32> = out.galley.rows.iter().map(|row| out.galley_pos.y + row.pos.y).collect();
                        eprintln!("rows ours {ours:?} editor {theirs:?} line_h {}", lay.line_h * xf.zoom);
                    }
                    // every caret spot as a tick (alignment checks without focus)
                    for &(cx, cy) in &lay.carets {
                        let p0 = r.min + vec2(lay.pad + cx, lay.pad + cy) * xf.zoom;
                        let p1 = p0 + vec2(0.0, lay.line_h * xf.zoom);
                        ui.painter().line_segment([crate::doc::rot_about(p0, c, rot), crate::doc::rot_about(p1, c, rot)], Stroke::new(1.0, Color32::from_rgb(255, 60, 200)));
                    }
                }
                if resp.has_focus() {
                    if let Some(cr) = out.cursor_range {
                        let map = |p: Pos2| crate::doc::rot_about(p, c, rot);
                        // caret spots from the renderer's own layout, so caret and selection
                        // sit exactly on the drawn text (egui rounds its rows to pixels)
                        let at = |i: usize| {
                            let (cx, cy) = lay.carets.get(i).or(lay.carets.last()).copied().unwrap_or((0.0, 0.0));
                            Rect::from_min_size(r.min + vec2(lay.pad + cx, lay.pad + cy) * xf.zoom, vec2(0.0, lay.line_h * xf.zoom))
                        };
                        // selection: one quad per character span on the same row
                        let (pi, si) = (cr.primary.index.0, cr.secondary.index.0);
                        let (a, b) = (pi.min(si), pi.max(si));
                        let sel = ACCENT.gamma_multiply(0.35);
                        for i in a..b {
                            let (p0, p1) = (at(i), at(i + 1));
                            if (p0.min.y - p1.min.y).abs() > 0.5 {
                                continue; // line break
                            }
                            let q = Rect::from_min_max(pos2(p0.min.x, p0.min.y), pos2(p1.min.x, p0.max.y));
                            let poly: Vec<Pos2> = [q.left_top(), q.right_top(), q.right_bottom(), q.left_bottom()].iter().map(|p| map(*p)).collect();
                            ui.painter().add(Shape::convex_polygon(poly, sel, Stroke::NONE));
                        }
                        // caret, blinking like egui's
                        let blink = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
                        if blink {
                            let k = at(pi);
                            ui.painter().line_segment([map(k.left_top()), map(k.left_bottom())], Stroke::new(2.0, INK()));
                        }
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                    }
                }
            });
        if text != t.text {
            if let Some(LayerKind::Text(tt)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                let s0 = crate::text::layout(tt).size;
                tt.text = text;
                tt.keep_corner(s0);
            }
            self.mark();
        }
        if lost {
            self.finish_text_edit();
        }
    }

    /// egui font family matching a text layer, registering installed fonts on demand.
    fn egui_family(&mut self, ctx: &egui::Context, t: &TextLayer) -> FontFamily {
        if !t.family.is_empty() {
            if let Some(l) = crate::fontdb::load(&t.family, &t.style) {
                let key = format!("sys:{}|{}", t.family, t.style);
                let fam = FontFamily::Name(key.clone().into());
                // egui applies add_font on the next frame; using the family before that panics
                if ctx.fonts(|f| f.definitions().families.contains_key(&fam)) {
                    return fam;
                }
                if self.egui_fonts.insert(key.clone()) {
                    let mut data = egui::FontData::from_owned((*l.bytes).clone());
                    data.index = l.index;
                    ctx.add_font(egui::epaint::text::FontInsert {
                        name: key.clone(),
                        data,
                        families: vec![
                            egui::epaint::text::InsertFontFamily {
                                family: FontFamily::Name(key.clone().into()),
                                priority: egui::epaint::text::FontPriority::Highest,
                            },
                        ],
                    });
                }
                self.egui_fonts.remove(&key); // re-check next frame
                ctx.request_repaint();
            }
        }
        FontFamily::Name(if t.style.to_lowercase().contains("bold") { "doc_bold".into() } else { "doc".into() })
    }

    fn unsaved_modal(&mut self, ctx: &egui::Context) {
        let Some(p) = self.pending.clone() else { return };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("unsaved")).frame(egui::Frame::NONE).backdrop_color(Color32::from_black_alpha(45)).show(ctx, |ui| theme::glass(ui, egui::Margin::same(SPACE_LG as i8), theme::RADIUS_CARD, |ui| {
            ui.set_width(300.0);
            ui.heading("저장하지 않은 변경 사항");
            ui.label("변경 사항을 저장할까요?");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("저장").clicked() {
                    choice = Some(0);
                }
                if ui.button("저장 안 함").clicked() {
                    choice = Some(1);
                }
                if ui.button("취소").clicked() {
                    choice = Some(2);
                }
            });
        }));
        match choice {
            Some(0) => {
                self.pending = None;
                if self.save(false) {
                    self.run_pending(p);
                }
            }
            Some(1) => {
                self.pending = None;
                self.modified = false;
                self.run_pending(p);
            }
            Some(2) => self.pending = None,
            _ => {}
        }
    }
}

const DEFAULT_FONT: &str = "기본 (맑은 고딕)";

/// Family + style chooser over the installed fonts. Returns true if changed.
/// Search text and rendered name previews for the font dropdowns.
#[derive(Default)]
pub struct FontUi {
    filter: String,
    previews: HashMap<String, Option<(egui::TextureHandle, Vec2)>>,
}

/// The family name rendered in its own face (None if it can't be drawn).
fn font_preview(ctx: &egui::Context, name: &str, style: &str) -> Option<(egui::TextureHandle, Vec2)> {
    use ab_glyph::Font;
    let face = crate::fontdb::load(name, style)?;
    if !name.chars().all(|c| c == ' ' || face.font.glyph_id(c).0 != 0) {
        return None;
    }
    let t = TextLayer { pos: Pos2::ZERO, text: name.to_string(), size: 16.0, color: INK(), family: name.to_string(), style: style.to_string(), bg: None, wrap: 0.0, rotation: 0.0, blend: Blend::Normal, align: Default::default() };
    let ppp = ctx.pixels_per_point();
    let lay = crate::text::layout(&t);
    let pm = crate::text::render(&t, &lay, ppp, Vec2::ZERO)?;
    let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
    let tex = ctx.load_texture(format!("fontpv:{name}"), img, egui::TextureOptions::LINEAR);
    Some((tex, vec2(pm.width() as f32, pm.height() as f32) / ppp))
}

/// egui family for an installed font, registering it on first use. Returns None
/// until egui has applied it (the next frame), so callers fall back for one frame.
fn installed_font_family(ctx: &egui::Context, family: &str, style: &str) -> Option<FontFamily> {
    if family.is_empty() {
        return None;
    }
    let key = format!("sys:{family}|{style}");
    let fam = FontFamily::Name(key.clone().into());
    if ctx.fonts(|f| f.definitions().families.contains_key(&fam)) {
        return Some(fam);
    }
    let l = crate::fontdb::load(family, style)?;
    let mut data = egui::FontData::from_owned((*l.bytes).clone());
    data.index = l.index;
    ctx.add_font(egui::epaint::text::FontInsert {
        name: key,
        data,
        families: vec![egui::epaint::text::InsertFontFamily { family: fam, priority: egui::epaint::text::FontPriority::Highest }],
    });
    ctx.request_repaint();
    None
}

fn font_picker(ui: &mut egui::Ui, id: &str, family: &mut String, style: &mut String, fui: &mut FontUi) -> bool {
    let filter = &mut fui.filter;
    let previews = &mut fui.previews;
    let mut changed = false;
    let fams = crate::fontdb::families();
    let shown = if family.is_empty() { DEFAULT_FONT.to_string() } else { family.clone() };
    ui.horizontal(|ui| {
        ui.label("글꼴");
        // the closed box shows the chosen family in its own face
        let shown: egui::WidgetText = match installed_font_family(ui.ctx(), family, style) {
            Some(fam) => egui::RichText::new(shown).family(fam).size(theme::TEXT_BODY + 1.0).into(),
            None => shown.into(),
        };
        egui::ComboBox::from_id_salt(id).truncate().selected_text(shown).width(170.0).height(360.0).popup_style(theme::glass_popup_style(false)).show_ui(ui, |ui| theme::glass_popup(ui, |ui| {
            let Some(fams) = &fams else {
                ui.label("설치된 글꼴 읽는 중…");
                return;
            };
            ui.add(egui::TextEdit::singleline(filter).hint_text("검색").desired_width(160.0)).request_focus();
            if ui.selectable_label(family.is_empty(), DEFAULT_FONT).clicked() {
                family.clear();
                *style = "Regular".into();
                changed = true;
            }
            let f = filter.to_lowercase();
            // each name is drawn in its own face; previews are built lazily, only for
            // rows on screen and a few per frame, so opening the list stays quick
            let mut budget = 4;
            let width = ui.available_width().max(220.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            for fam in fams.iter().filter(|x| f.is_empty() || x.name.to_lowercase().contains(&f)) {
                let (r, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
                if !ui.is_rect_visible(r) {
                    continue;
                }
                let selected = *family == fam.name;
                if selected {
                    ui.painter().rect_filled(r, 6.0, FILL_2());
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 6.0, FILL_1());
                }
                if !previews.contains_key(&fam.name) {
                    if budget > 0 {
                        budget -= 1;
                        let pv = font_preview(ui.ctx(), &fam.name, &fam.regular().style);
                        previews.insert(fam.name.clone(), pv);
                    } else {
                        ui.ctx().request_repaint();
                    }
                }
                match previews.get(&fam.name) {
                    Some(Some((tex, size))) => {
                        let img = Rect::from_min_size(pos2(r.left() + 8.0, r.center().y - size.y * 0.5), *size);
                        ui.painter().with_clip_rect(r.intersect(ui.clip_rect())).image(
                            tex.id(),
                            img,
                            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    _ => {
                        ui.painter().text(pos2(r.left() + 8.0, r.center().y), Align2::LEFT_CENTER, &fam.name, FontId::proportional(theme::TEXT_BODY), INK_2());
                    }
                }
                if resp.clicked() {
                    *family = fam.name.clone();
                    *style = fam.regular().style.clone();
                    changed = true;
                    ui.close();
                }
            }
        }));
    });
    let styles: Vec<String> = if family.is_empty() {
        vec!["Regular".into(), "Bold".into()]
    } else {
        fams.as_ref()
            .and_then(|f| f.iter().find(|x| x.name == *family))
            .map(|f| f.faces.iter().map(|x| x.style.clone()).collect())
            .unwrap_or_default()
    };
    if styles.len() > 1 {
        ui.horizontal(|ui| {
            ui.label("스타일");
            egui::ComboBox::from_id_salt((id, "style")).truncate().selected_text(style.clone()).width(170.0).popup_style(theme::glass_popup_style(false)).show_ui(ui, |ui| theme::glass_popup(ui, |ui| {
                for s in styles {
                    if ui.selectable_label(*style == s, &s).clicked() {
                        *style = s;
                        changed = true;
                    }
                }
            }));
        });
    }
    changed
}

/// Scales by `s` and rotates by `th` (radians, clockwise on screen) about `c`,
/// starting from the untouched original layer each time (no drift).
pub(crate) fn transform_layer(o: &Layer, c: Pos2, s: f32, th: f32) -> Layer {
    let (sin, cos) = th.sin_cos();
    let map = |p: Pos2| {
        let d = (p - c) * s;
        c + vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
    };
    let mut l = o.clone();
    match &mut l.kind {
        LayerKind::Image(im) => {
            im.center = map(im.center);
            im.scale *= s;
            im.rotation += th;
        }
        LayerKind::Text(t) => {
            // text can't rotate yet: its centre orbits and its size scales
            let r = crate::text::layout(t).rect(t.pos);
            let nc = map(r.center());
            t.size *= s;
            t.wrap *= s;
            t.rotation += th;
            let ns = crate::text::layout(t).size;
            t.pos = nc - ns * 0.5;
        }
        LayerKind::Shape(sh) => {
            sh.width *= s;
            match sh.kind {
                ShapeKind::Line | ShapeKind::Arrow => {
                    sh.a = map(sh.a);
                    sh.b = map(sh.b);
                }
                ShapeKind::Rect | ShapeKind::Ellipse => {
                    // centre orbits, extents scale, own rotation turns
                    let r = Rect::from_two_pos(sh.a, sh.b);
                    let nc = map(r.center());
                    let h = r.size() * 0.5 * s;
                    sh.a = nc - h;
                    sh.b = nc + h;
                    sh.rotation += th;
                }
            }
        }
        LayerKind::Vector(v) => {
            let off = v.offset;
            v.strokes = v
                .strokes
                .iter()
                .map(|st| {
                    let pts = st
                        .pts
                        .iter()
                        .map(|q| {
                            let p = map(pos2(q[0], q[1]) + off) - off;
                            [p.x, p.y, q[2] * s]
                        })
                        .collect();
                    Arc::new(VStroke::new(pts, st.color).with_rec(st.rec.clone()))
                })
                .collect();
        }
        LayerKind::Raster(r) => {
            if let Some(n) = crate::raster::transformed(r, c, s, th) {
                *r = n;
            }
        }
    }
    l
}

/// Rotation snapping in 15° steps: Shift/Ctrl relative to where the drag started
/// (`start`), Alt relative to 0°.
fn snap_angle(a: f32, start: f32, mods: Modifiers) -> f32 {
    let step = 15f32.to_radians();
    if mods.alt {
        (a / step).round() * step
    } else if mods.shift || mods.ctrl {
        start + ((a - start) / step).round() * step
    } else {
        a
    }
}

/// Folder row of a 3D space in the layer list.
#[allow(clippy::too_many_arguments)]
fn space_header(ui: &mut egui::Ui, name: &str, count: usize, solved: bool, width: f32, active: bool, indent: f32, wall: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    let r = Rect::from_min_max(r.min + vec2(indent, 0.0), r.max);
    let p = ui.painter();
    if active {
        p.rect_filled(r, 8.0, FILL_1());
    } else if resp.hovered() {
        p.rect_filled(r, 8.0, FILL_1().gamma_multiply(0.6));
    }
    let cy = r.center().y;
    let ir = Rect::from_center_size(pos2(r.left() + 16.0, cy), vec2(18.0, 18.0));
    if wall {
        let q = [pos2(ir.left() + 2.0, ir.top() + 3.0), pos2(ir.right() - 2.0, ir.top() + 6.0), pos2(ir.right() - 2.0, ir.bottom() - 6.0), pos2(ir.left() + 2.0, ir.bottom() - 3.0)];
        p.add(Shape::closed_line(q.to_vec(), Stroke::new(1.4, crate::view3d::WALL_COLOR)));
    } else {
        theme::tool_icon(p, Tool::Space, ir, INK_2());
    }
    p.text(pos2(r.left() + 32.0, cy), Align2::LEFT_CENTER, name, FontId::proportional(theme::TEXT_BODY), INK_2());
    let info = if solved { format!("{count}") } else { "선 부족".to_string() };
    p.text(pos2(r.right() - 10.0, cy), Align2::RIGHT_CENTER, info, FontId::proportional(theme::TEXT_SMALL), INK_3());
    resp.on_hover_text(if wall { "벽 폴더 · 레이어를 놓으면 벽에 붙음" } else { "공간 폴더 · 레이어를 놓으면 공간에 넣음" })
}

/// Vertical light / dark / system switch with icons only. True when it changed.
fn theme_switch(ui: &mut egui::Ui, mode: &mut u8) -> bool {
    let slot = 24.0;
    let (r, resp) = ui.allocate_exact_size(vec2(24.0, slot * 3.0), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(r, 12.0, FILL_1());
    let mut changed = false;
    if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.clicked() || resp.dragged()) {
        let m = (((pp.y - r.top()) / slot).floor() as i32).clamp(0, 2) as u8;
        if m != *mode {
            *mode = m;
            changed = true;
        }
    }
    let names = ["라이트", "다크", "시스템"];
    for i in 0..3u8 {
        let c = pos2(r.center().x, r.top() + slot * (i as f32 + 0.5));
        let on = *mode == i;
        if on {
            p.circle_filled(c, 10.5, FILL_3());
            p.circle_stroke(c, 10.5, Stroke::new(1.2, ACCENT));
        }
        let col = if on { INK() } else { INK_3() };
        let st = Stroke::new(1.2, col);
        match i {
            0 => {
                p.circle_stroke(c, 2.8, st);
                for k in 0..8 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4;
                    let d = vec2(a.cos(), a.sin());
                    p.line_segment([c + d * 4.6, c + d * 6.4], st);
                }
            }
            1 => {
                let outer: Vec<Pos2> = (0..=20).map(|k| {
                    let a = 0.55 + k as f32 / 20.0 * 5.2;
                    c + vec2(a.cos(), a.sin()) * 5.2
                }).collect();
                let ic = c + vec2(2.6, -1.8);
                let inner: Vec<Pos2> = (0..=20).rev().map(|k| {
                    let a = 1.15 + k as f32 / 20.0 * 3.75;
                    ic + vec2(a.cos(), a.sin()) * 4.0
                }).collect();
                let mut pts = outer;
                pts.extend(inner);
                p.add(Shape::closed_line(pts, st));
            }
            _ => {
                let scr = Rect::from_center_size(c + vec2(0.0, -1.2), vec2(11.0, 7.5));
                p.rect_stroke(scr, 1.2, st, StrokeKind::Middle);
                p.line_segment([c + vec2(0.0, 2.6), c + vec2(0.0, 4.6)], st);
                p.line_segment([c + vec2(-3.0, 5.0), c + vec2(3.0, 5.0)], st);
            }
        }
    }
    let hover = resp.hover_pos().map(|pp| (((pp.y - r.top()) / slot).floor() as i32).clamp(0, 2) as usize);
    if let Some(h) = hover {
        resp.on_hover_text(names[h]);
    }
    changed
}

/// Keyframe mark by easing: a diamond (linear), a bow tie (smooth in/out, like an
/// hourglass on its side) or a square (hold).
/// Masks move with their anchor layer (translation of its centre, whole pixels;
/// the remainder carries over).
/// Masks move with their anchor layer: its centre live (whole pixels, the
/// remainder carries over), its rotation and size once a drag ends (each turn or
/// scale resamples the mask, so not on every frame of a drag).
fn sync_masks(doc: &mut Document, dragging: bool) {
    let poses: Vec<(LayerId, Pos2, f32, f32)> = doc
        .layers
        .iter()
        .filter(|l| l.mask_of.is_none())
        .map(|l| (l.id, crate::anim::key_of(l, 0.0)))
        .map(|(id, k)| (id, pos2(k.center[0], k.center[1]), k.rot, k.ext[0]))
        .collect();
    for l in &mut doc.layers {
        let Some((aid, last)) = l.mask_anchor else { continue };
        let Some(&(_, c, rot, ext)) = poses.iter().find(|x| x.0 == aid) else { continue };
        let d = vec2((c.x - last[0]).round(), (c.y - last[1]).round());
        if d != Vec2::ZERO {
            if let LayerKind::Raster(r) = &mut l.kind {
                r.offset += d;
            }
            l.mask_anchor = Some((aid, [last[0] + d.x, last[1] + d.y]));
        }
        match l.mask_pose {
            None => l.mask_pose = Some([rot, ext]),
            Some([pr, pe]) if !dragging => {
                let s = if pe.abs() > 1e-6 && ext.abs() > 1e-6 { ext / pe } else { 1.0 };
                let th = rot - pr;
                if th.abs() > 1e-4 || (s - 1.0).abs() > 1e-4 {
                    if let LayerKind::Raster(r) = &mut l.kind {
                        if r.tiles.is_empty() {
                        } else if let Some(t) = crate::raster::transformed(r, c, s, th) {
                            *r = t;
                        }
                    }
                    l.mask_pose = Some([rot, ext]);
                }
            }
            _ => {}
        }
    }
}

/// A low-res GrabCut result read back at full resolution: bilinear between cell
/// centres (so the edge is a smooth line, not cell steps).
fn upsample_fg(labels: &[u8], gw: usize, gh: usize, step: usize, x: usize, y: usize) -> f32 {
    use crate::segment::{FG, PR_FG};
    let fg = |gx: usize, gy: usize| {
        let l = labels[gy.min(gh - 1) * gw + gx.min(gw - 1)];
        if l == FG || l == PR_FG { 1.0 } else { 0.0 }
    };
    if step == 1 {
        return fg(x, y);
    }
    let fx = ((x as f32 + 0.5) / step as f32 - 0.5).max(0.0);
    let fy = ((y as f32 + 0.5) / step as f32 - 0.5).max(0.0);
    let (x0, y0) = (fx as usize, fy as usize);
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let top = fg(x0, y0) * (1.0 - tx) + fg(x0 + 1, y0) * tx;
    let bot = fg(x0, y0 + 1) * (1.0 - tx) + fg(x0 + 1, y0 + 1) * tx;
    // back to a crisp 0/1 along the smooth iso-line; the guided filter softens it
    if top * (1.0 - ty) + bot * ty >= 0.5 { 1.0 } else { 0.0 }
}

/// Box mean with radius `r` (clamped at the borders), via an integral image.
fn box_mean(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut ii = vec![0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0f64;
        for x in 0..w {
            row += v[y * w + x] as f64;
            ii[(y + 1) * (w + 1) + x + 1] = ii[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        let (ya, yb) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (xa, xb) = (x.saturating_sub(r), (x + r + 1).min(w));
            let s = ii[yb * (w + 1) + xb] - ii[ya * (w + 1) + xb] - ii[yb * (w + 1) + xa] + ii[ya * (w + 1) + xa];
            out[y * w + x] = (s / ((yb - ya) * (xb - xa)) as f64) as f32;
        }
    }
    out
}

/// Guided filter (He et al. 2010): `p` filtered with edges taken from guide `i`.
fn guided_filter(i: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let mi = box_mean(i, w, h, r);
    let mp = box_mean(p, w, h, r);
    let ii: Vec<f32> = i.iter().map(|v| v * v).collect();
    let ip: Vec<f32> = i.iter().zip(p).map(|(a, b)| a * b).collect();
    let mii = box_mean(&ii, w, h, r);
    let mip = box_mean(&ip, w, h, r);
    let mut a = vec![0f32; w * h];
    let mut b = vec![0f32; w * h];
    for k in 0..w * h {
        let var = mii[k] - mi[k] * mi[k];
        let cov = mip[k] - mi[k] * mp[k];
        a[k] = cov / (var + eps);
        b[k] = mp[k] - a[k] * mi[k];
    }
    let ma = box_mean(&a, w, h, r);
    let mb = box_mean(&b, w, h, r);
    (0..w * h).map(|k| ma[k] * i[k] + mb[k]).collect()
}

/// `v` turned by `a` radians about the unit axis `k` (Rodrigues).
fn rotate_about(v: crate::persp::V3, k: crate::persp::V3, a: f64) -> crate::persp::V3 {
    use crate::persp::{add, cross, dot, scale};
    let (s, c) = a.sin_cos();
    add(add(scale(v, c), scale(cross(k, v), s)), scale(k, dot(k, v) * (1.0 - c)))
}

/// Key state of an animation at `t` (a key within `tol` counts as here).
fn key_mark(a: Option<&crate::anim::Anim>, t: f64, tol: f64, c: u16) -> crate::dial::KeyMark {
    use crate::dial::KeyMark;
    match a {
        Some(a) if a.keys.iter().any(|k| k.ch & c != 0 && (k.t - t).abs() <= tol) => KeyMark::Here,
        Some(a) if a.keys.iter().any(|k| k.ch & c != 0) => KeyMark::Keyed,
        _ => KeyMark::None,
    }
}

/// A name for a widget, cut with an ellipsis at `width`. Widgets overwrite a
/// `LayoutJob`'s wrapping with the Ui's own, so this hands them a finished galley;
/// its colour is the widget's own (placeholder).
fn elide(ui: &egui::Ui, text: String, width: f32) -> egui::WidgetText {
    let job = elided(text, egui::TextStyle::Body.resolve(ui.style()), Color32::PLACEHOLDER, width);
    egui::WidgetText::Galley(ui.fonts_mut(|f| f.layout_job(job)))
}

/// Lays out `add` at the Ui's current width and never wider: what does not fit
/// is clipped, so a long line cannot widen a floating panel.
fn fixed_width<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let w = ui.available_width();
    let r = ui.available_rect_before_wrap();
    let rect = Rect::from_min_size(r.min, vec2(w, r.height()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()));
    child.set_clip_rect(ui.clip_rect().intersect(Rect::from_x_y_ranges(rect.x_range(), egui::Rangef::EVERYTHING)));
    let out = add(&mut child);
    let h = child.min_rect().height();
    ui.advance_cursor_after_rect(Rect::from_min_size(rect.min, vec2(w, h)));
    out
}

/// One line of text cut with an ellipsis at `width`.
fn elided(text: String, font: FontId, color: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping { max_width: width.max(8.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    job
}

fn key_icon(p: &egui::Painter, c: Pos2, ease: crate::anim::Ease, fill: Color32, edge: Stroke) {
    use crate::anim::Ease;
    match ease {
        Ease::Linear => {
            let d = 5.5;
            p.add(Shape::convex_polygon(vec![c + vec2(0.0, -d), c + vec2(d, 0.0), c + vec2(0.0, d), c + vec2(-d, 0.0)], fill, edge));
        }
        Ease::Smooth => {
            for s in [-1.0f32, 1.0] {
                let pts = vec![c + vec2(s * 5.5, -5.5), c + vec2(s * 5.5, 5.5), c + vec2(s * 1.0, 1.6), c + vec2(s * 1.0, -1.6)];
                let pts = if s < 0.0 { pts } else { pts.into_iter().rev().collect() };
                p.add(Shape::convex_polygon(pts, fill, edge));
            }
        }
        Ease::Hold => {
            p.rect(Rect::from_center_size(c, vec2(9.5, 9.5)), 1.0, fill, edge, StrokeKind::Inside);
        }
    }
}

/// Pixel fields (x, y, width, height), size presets that keep the centre, and a
/// landscape/portrait swap for a region. True when it changed.
fn rect_fields(ui: &mut egui::Ui, r: &mut Rect, salt: &str, key: Option<crate::dial::KeyMark>, key_hit: &mut bool) -> bool {
    let mut ch = false;
    let (mut x, mut y, mut w, mut h) = (r.min.x, r.min.y, r.width(), r.height());
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if let Some(m) = key {
            *key_hit |= crate::dial::key_button(ui, m);
        }
        ui.label("X");
        ch |= ui.add(crate::calc::drag(&mut x).speed(1.0).max_decimals(0)).changed();
        ui.label("Y");
        ch |= ui.add(crate::calc::drag(&mut y).speed(1.0).max_decimals(0)).changed();
    });
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if let Some(m) = key {
            *key_hit |= crate::dial::key_button(ui, m);
        }
        ui.label("W");
        ch |= ui.add(crate::calc::drag(&mut w).range(2.0..=32000.0).speed(1.0).max_decimals(0)).changed();
        ui.label("H");
        ch |= ui.add(crate::calc::drag(&mut h).range(2.0..=32000.0).speed(1.0).max_decimals(0)).changed();
        if ui.button("가로 ↔ 세로").on_hover_text("가운데 유지, 가로·세로 교체").clicked() {
            let c = pos2(x + w * 0.5, y + h * 0.5);
            std::mem::swap(&mut w, &mut h);
            x = c.x - w * 0.5;
            y = c.y - h * 0.5;
            ch = true;
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        let portrait = h > w;
        for (pw, ph) in [(1920.0f32, 1080.0f32), (1280.0, 720.0), (2560.0, 1440.0), (3840.0, 2160.0), (1080.0, 1080.0), (1080.0, 1350.0)] {
            let (pw, ph) = if portrait && pw != ph { (ph.min(pw), ph.max(pw)) } else { (pw, ph) };
            let on = (w - pw).abs() < 0.5 && (h - ph).abs() < 0.5;
            if ui.selectable_label(on, format!("{pw:.0}×{ph:.0}")).on_hover_text("가운데 유지").clicked() {
                let c = pos2(x + w * 0.5, y + h * 0.5);
                w = pw;
                h = ph;
                x = c.x - w * 0.5;
                y = c.y - h * 0.5;
                ch = true;
            }
        }
    });
    let _ = salt;
    if ch {
        *r = Rect::from_min_size(pos2(x.round(), y.round()), vec2(w.round().max(2.0), h.round().max(2.0)));
    }
    ch
}

/// Distance from `p` to the segment `a`-`b`.
fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

fn handle_box(painter: &egui::Painter, p: Pos2, c: Color32) {
    let r = Rect::from_center_size(p, vec2(9.0, 9.0));
    painter.rect_filled(r, 1.0, Color32::WHITE);
    painter.rect_stroke(r, 1.0, Stroke::new(1.5, c), StrokeKind::Inside);
}

/// 8 handles: TL, T, TR, R, BR, B, BL, L.
fn rect_handles(r: Rect) -> [Pos2; 8] {
    [
        r.left_top(),
        r.center_top(),
        r.right_top(),
        r.right_center(),
        r.right_bottom(),
        r.center_bottom(),
        r.left_bottom(),
        r.left_center(),
    ]
}

fn handle_axes(h: u8) -> (i8, i8) {
    match h {
        0 => (-1, -1),
        1 => (0, -1),
        2 => (1, -1),
        3 => (1, 0),
        4 => (1, 1),
        5 => (0, 1),
        6 => (-1, 1),
        7 => (-1, 0),
        _ => (0, 0),
    }
}

fn constrain_shape(kind: ShapeKind, anchor: Pos2, p: Pos2, shift: bool) -> Pos2 {
    if !shift {
        return p;
    }
    let d = p - anchor;
    match kind {
        ShapeKind::Line | ShapeKind::Arrow => {
            let step = std::f32::consts::FRAC_PI_4;
            let a = (d.y.atan2(d.x) / step).round() * step;
            anchor + vec2(a.cos(), a.sin()) * d.length()
        }
        _ => {
            let m = d.x.abs().max(d.y.abs());
            anchor + vec2(m * d.x.signum(), m * d.y.signum())
        }
    }
}

fn shape_hit(s: &ShapeLayer, w: Pos2, tol: f32) -> bool {
    let t = s.width * 0.5 + tol;
    let w = crate::doc::rot_about(w, Rect::from_two_pos(s.a, s.b).center(), -s.rotation);
    let seg = |a: Pos2, b: Pos2| {
        let ab = b - a;
        let k = ((w - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
        (a + ab * k).distance(w) <= t
    };
    match s.kind {
        ShapeKind::Line | ShapeKind::Arrow => seg(s.a, s.b),
        ShapeKind::Rect => {
            let r = Rect::from_two_pos(s.a, s.b);
            if s.fill.is_some() && r.contains(w) {
                return true;
            }
            r.expand(t).contains(w) && !r.shrink(t).contains(w)
        }
        ShapeKind::Ellipse => {
            let r = Rect::from_two_pos(s.a, s.b);
            let c = r.center();
            let h = (r.size() * 0.5).max(Vec2::splat(1e-3));
            let d = (w - c) / h;
            let n = d.length();
            if s.fill.is_some() && n <= 1.0 {
                return true;
            }
            (n - 1.0).abs() * h.min_elem() <= t
        }
    }
}

impl eframe::App for App {
    /// Takes Tab out of the input before egui sees it: egui would otherwise move
    /// keyboard focus to the next button (File, View, ...) and Tab would never reach
    /// the workspace switch. Text fields still get Tab while one is being edited.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if ctx.text_edit_focused() {
            return;
        }
        let tabs = &mut self.tab_raw;
        let before = tabs.len();
        raw.events.retain(|e| match e {
            egui::Event::Key { key: Key::Tab, pressed, repeat, modifiers, .. } => {
                if !repeat {
                    tabs.push((*pressed, modifiers.shift));
                }
                false
            }
            _ => true,
        });
        // a button focused earlier (by an old Tab) would keep stealing keys
        if tabs.len() > before {
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.now = ctx.input(|i| i.time);
        self.drop_stale_ime(&ctx);

        // swap in the nicer UI face once the system font scan lands
        if !self.fonts_applied && crate::fontdb::families().is_some() {
            ctx.set_fonts(theme::font_defs());
            self.egui_fonts.clear();
            self.fonts_applied = true;
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect()
        });
        if !dropped.is_empty() {
            let at = ctx
                .input(|i| i.pointer.hover_pos())
                .filter(|p| self.canvas_rect.contains(*p))
                .map(|p| self.xf().s2w(p))
                .unwrap_or_else(|| self.view_center_world());
            self.import_paths(dropped, at);
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_prefs();
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.modified && self.pending.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }

        let light = match self.s.theme {
            0 => true,
            1 => false,
            _ => ctx.system_theme() == Some(egui::Theme::Light),
        };
        if light != theme::is_light() {
            theme::set_light(light);
            theme::apply_visuals(&ctx);
        }
        self.handle_shortcuts(&ctx);
        crate::persp::follow_photos(&mut self.doc);
        crate::persp::sync_walls(&mut self.doc);
        sync_masks(&mut self.doc, self.drag.is_some() || self.tl_play.is_some());
        crate::anim::apply_constraints(&mut self.doc);
        self.tick_audio();
        self.tick_timeline(&ctx);
        self.tick_videos(&ctx);
        if let Some(rx) = &self.img_job {
            match rx.try_recv() {
                Ok(msg) => {
                    self.status = msg;
                    self.img_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.img_job = None,
                Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }

        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
        if !self.ui_hidden {
            self.top_bar(&ctx);
            if self.ws3d {
                self.header3d(&ctx);
            } else {
                self.tool_rail(&ctx);
            }
            self.inspector(&ctx);
            self.status_pill(&ctx);
        }
        self.palette_ui(&ctx);
        if self.show_log {
            egui::Area::new(egui::Id::new("input_log"))
                .order(egui::Order::Foreground)
                .anchor(Align2::LEFT_BOTTOM, vec2(SPACE_LG, -SPACE_LG))
                .show(&ctx, |ui| {
                    theme::glass(ui, egui::Margin::same(theme::SPACE_MD as i8), theme::RADIUS_CARD, |ui| {
                        ui.set_width(330.0);
                        ui.label(egui::RichText::new("입력 기록 · F12로 닫기").font(FontId::new(theme::TEXT_LABEL, theme::ui_bold())).color(INK_3()));
                        let pen = crate::penhook::state();
                        ui.label(
                            egui::RichText::new(format!(
                                "펜 범위 {} · 버튼 {} · 지우개 {} · flags {:X}/{:X}",
                                pen.in_range, pen.barrel, pen.eraser, pen.raw_pen, pen.raw_pointer
                            ))
                            .monospace()
                            .color(INK_2()),
                        );
                        for line in &self.input_log {
                            ui.label(egui::RichText::new(line).monospace().color(INK_2()));
                        }
                    });
                });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.doc.timeline.on && !self.ui_hidden {
            self.timeline_ui(&ctx);
        }
        if std::env::var_os("DV_SHOW_VIEW").is_some() {
            // the View menu's contents in a card (screenshots; menus cannot be opened from code)
            egui::Area::new(egui::Id::new("dbg_view")).order(egui::Order::Foreground).fixed_pos(pos2(40.0, 110.0)).show(&ctx, |ui| {
                theme::glass(ui, egui::Margin::same(8), theme::RADIUS_CARD, |ui| self.view_menu_body(ui));
            });
        }
        self.export_ui(&ctx);
        self.video_export_ui(&ctx);
        self.unsaved_modal(&ctx);
        self.peek_ui(&ctx);

        // a dial drag or a scroll burst is one undo step: commit once the
        // pointer is up and scrolling has paused
        let (now, scrolled, down) = ctx.input(|i| (i.time, i.smooth_scroll_delta != Vec2::ZERO, i.pointer.any_down()));
        if scrolled {
            self.scroll_at = now;
        }
        let settling = now - self.scroll_at < 0.4;
        if self.changed && settling {
            ctx.request_repaint_after(std::time::Duration::from_millis(450));
        }
        let typing = ctx.text_edit_focused() || down || settling;
        self.commit_if_idle(typing);
        self.tex.gc();

        let name = self.path.as_ref().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or("제목 없음");
        let title = format!("{}{} — Drifting VOID", if self.modified { "● " } else { "" }, name);
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        self.frames += 1;
        if self.frames % 90 == 0 {
            self.save_prefs();
        }
        if self.drag.is_some() || crate::penhook::state().barrel || self.frames < 6 {
            ctx.request_repaint();
        }
    }
}

/// Hashes the volume keys of `a` (what the mixer's envelope is built from).
fn vol_keys_hash(a: &crate::anim::Anim, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    for k in a.keys.iter().filter(|k| k.ch & crate::anim::ch::VOL != 0) {
        (k.t.to_bits(), k.vol.to_bits(), k.ease as u8).hash(h);
    }
}

/// Left / centre / right buttons for text alignment, drawn as line icons.
fn align_row(ui: &mut egui::Ui, a: &mut crate::doc::TextAlign) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut no = false;
        crate::dial::label_cell(ui, "정렬", None, &mut no);
        for al in crate::doc::TextAlign::ALL {
            let on = *a == al;
            let (r, resp) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
            let p = ui.painter();
            if on || resp.hovered() {
                p.rect_filled(r, 5.0, if on { ACCENT.gamma_multiply(0.25) } else { FILL_1() });
            }
            let col = if on { ACCENT } else { INK_2() };
            // four lines, long and short, flush to the side
            let (x0, x1) = (r.left() + 6.0, r.right() - 6.0);
            for (i, long) in [true, false, true, false].into_iter().enumerate() {
                let w = if long { x1 - x0 } else { (x1 - x0) * 0.6 };
                let y = r.top() + 6.0 + i as f32 * 3.4;
                let left = x0 + (x1 - x0 - w) * al.factor();
                p.line_segment([pos2(left, y), pos2(left + w, y)], Stroke::new(1.4, col));
            }
            if resp.on_hover_text(al.label()).clicked() && !on {
                *a = al;
                changed = true;
            }
        }
    });
    changed
}

/// A number without trailing zeros (up to 3 decimals).
fn trim_num(v: f64) -> String {
    let t = format!("{v:.3}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A position number box ("—" when the selection's positions differ).
fn pos_box(v: &mut f32, mixed: bool) -> egui::DragValue<'_> {
    let d = crate::calc::drag(v).speed(1.0).max_decimals(1);
    if mixed { d.custom_formatter(|_, _| crate::dial::MIXED.into()) } else { d }
}

/// One edit (`before` -> `after`) applied to another layer `l`: moves, turns
/// and resizes as deltas; opacity, thickness and style values as they are.
fn spread_to(before: &Layer, after: &Layer, l: &mut Layer) {
    use crate::anim::{ch, diff_ch, key_of};
    let (kb, ka) = (key_of(before, 0.0), key_of(after, 0.0));
    let c = diff_ch(&kb, &ka);
    let ratio = |i: usize| if kb.ext[i].abs() > 1e-6 { ka.ext[i] / kb.ext[i] } else { 1.0 };
    let mut k = key_of(l, 0.0);
    if c & ch::POS != 0 {
        k.center[0] += ka.center[0] - kb.center[0];
        k.center[1] += ka.center[1] - kb.center[1];
    }
    if c & ch::SIZE != 0 {
        k.ext = [k.ext[0] * ratio(0), k.ext[1] * ratio(1)];
    }
    if c & ch::ROT != 0 {
        k.rot += ka.rot - kb.rot;
    }
    if c & ch::OPA != 0 {
        k.opacity = ka.opacity;
    }
    if let (Some(b), Some(a), Some(p)) = (kb.place, ka.place, k.place.as_mut()) {
        if c & ch::P_POS != 0 {
            for i in 0..3 {
                p.pos[i] += a.pos[i] - b.pos[i];
            }
        }
        if c & ch::P_ROT != 0 {
            for i in 0..3 {
                p.rot[i] += a.rot[i] - b.rot[i];
            }
        }
        if c & ch::P_SCALE != 0 && b.scale.abs() > 1e-9 {
            p.scale *= a.scale / b.scale;
        }
        if c & ch::P_THICK != 0 {
            p.thick = a.thick;
        }
    }
    crate::anim::apply_key(l, &k);
    copy_style(&before.kind, &after.kind, &mut l.kind);
}

/// Copies the style values that changed from `b` to `a` onto `l` (same kind
/// only; the text itself is never copied).
fn copy_style(b: &LayerKind, a: &LayerKind, l: &mut LayerKind) {
    macro_rules! pass {
        ($x:expr, $y:expr, $z:expr) => {
            if $x != $y {
                $z = $y.clone();
            }
        };
    }
    match (b, a, l) {
        (LayerKind::Text(b), LayerKind::Text(a), LayerKind::Text(l)) => {
            pass!(b.color, a.color, l.color);
            pass!(b.family, a.family, l.family);
            pass!(b.style, a.style, l.style);
            pass!(b.bg, a.bg, l.bg);
            pass!(b.wrap, a.wrap, l.wrap);
            pass!(b.blend, a.blend, l.blend);
            pass!(b.align, a.align, l.align);
        }
        (LayerKind::Shape(b), LayerKind::Shape(a), LayerKind::Shape(l)) => {
            pass!(b.color, a.color, l.color);
            pass!(b.fill, a.fill, l.fill);
            pass!(b.width, a.width, l.width);
        }
        (LayerKind::Image(b), LayerKind::Image(a), LayerKind::Image(l)) => {
            pass!(b.adjust.brightness, a.adjust.brightness, l.adjust.brightness);
            pass!(b.adjust.contrast, a.adjust.contrast, l.adjust.contrast);
            pass!(b.adjust.saturation, a.adjust.saturation, l.adjust.saturation);
            pass!(b.adjust.chroma, a.adjust.chroma, l.adjust.chroma);
            pass!(b.adjust.key_tol, a.adjust.key_tol, l.adjust.key_tol);
            pass!(b.adjust.key_soft, a.adjust.key_soft, l.adjust.key_soft);
        }
        (LayerKind::Raster(b), LayerKind::Raster(a), LayerKind::Raster(l)) => {
            pass!(b.blend, a.blend, l.blend);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(x: f32, size: f32) -> Layer {
        let mut d = Document::default();
        let t = TextLayer { pos: pos2(x, 0.0), text: "ab".into(), size, color: Color32::WHITE, family: String::new(), style: String::new(), bg: None, wrap: 0.0, rotation: 0.0, blend: Blend::Normal, align: Default::default() };
        let id = d.insert("t".into(), LayerKind::Text(t), None);
        d.get(id).unwrap().clone()
    }

    #[test]
    fn edits_spread_as_deltas_and_values() {
        let before = text(0.0, 40.0);
        let mut after = before.clone();
        // primary: moved by 30, turned 0.2, made 2x bigger, half opaque, red
        if let LayerKind::Text(t) = &mut after.kind {
            t.rotation = 0.2;
            t.color = Color32::RED;
        }
        let mut k = crate::anim::key_of(&after, 0.0);
        k.center[0] += 30.0;
        k.ext[0] *= 2.0;
        k.opacity = 0.5;
        crate::anim::apply_key(&mut after, &k);
        let mut other = text(500.0, 20.0);
        if let LayerKind::Text(t) = &mut other.kind {
            t.rotation = 1.0;
        }
        let c0 = crate::anim::key_of(&other, 0.0).center;
        spread_to(&before, &after, &mut other);
        let ko = crate::anim::key_of(&other, 0.0);
        let LayerKind::Text(t) = &other.kind else { unreachable!() };
        assert!((ko.center[0] - c0[0] - 30.0).abs() < 1e-3, "moved by the same amount");
        assert!((t.size - 40.0).abs() < 1e-3, "scaled by the same ratio");
        assert!((t.rotation - 1.2).abs() < 1e-5, "turned by the same angle");
        assert_eq!(other.opacity, 0.5);
        assert_eq!(t.color, Color32::RED);
        assert_eq!(t.text, "ab", "text itself is not copied");
    }

    #[test]
    fn similar_colour_regions_are_never_split_at_the_sides() {
        // a region touching both side edges must be a full band (every column)
        let mut split = Vec::new();
        for hi in 0..24 {
            for (s, v) in [(0.3, 0.6), (0.6, 0.8), (0.9, 0.5), (0.15, 0.9), (0.5, 0.3)] {
                let r = (hi as f32 / 24.0, s, v);
                for m in [GamutMap::Value, GamutMap::Chroma, GamutMap::Near] {
                    let win = m.fit(r);
                    let (w, h) = (96, 48);
                    let (_, keep) = m.sample(r, win, w, h);
                    // a full-hue window wraps: a kept pixel on a side edge whose
                    // neighbour across the seam matches too must have it kept
                    if win.x1 - win.x0 < 0.999 {
                        continue;
                    }
                    let lch = crate::okhsv::lch_of(crate::okhsv::to_rgb(r.0, r.1, r.2), r.0);
                    let cut = (0..h).any(|j| {
                        let y = win.y0 + (j as f32 + 0.5) / h as f32 * (win.y1 - win.y0);
                        [(0, w - 1), (w - 1, 0)].iter().any(|&(i, o)| {
                            let x = win.x0 + (o as f32 + 0.5) / w as f32 * (win.x1 - win.x0);
                            keep[j * w + i] && !keep[j * w + o] && m.eval(r, lch, x, y).1
                        })
                    });
                    if cut {
                        split.push((r, m as u8));
                    }
                }
            }
        }
        assert!(split.is_empty(), "{} split: {:?}", split.len(), &split[..split.len().min(6)]);
    }
}
