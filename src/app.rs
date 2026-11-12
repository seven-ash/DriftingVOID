// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{
    Align2, Color32, FontFamily, FontId, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
    TouchId, TouchPhase, Vec2, pos2, vec2,
};

use crate::canvas::{self, TexCache, Xf};
use crate::theme::{self, GlassTip, ACCENT, ACCENT_INK, FILL_1, FILL_2, FILL_3, INK, INK_2, INK_3, PAPER_4, SPACE_2XS, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS};
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
    (Tool::Select, crate::i18n::k!("선택"), "V", Key::V),
    (Tool::Lasso, crate::i18n::k!("올가미 선택"), "Q", Key::Q),
    (Tool::Hand, crate::i18n::k!("화면 이동"), "H", Key::H),
    (Tool::Brush, crate::i18n::k!("브러시"), "B", Key::B),
    (Tool::Pen, crate::i18n::k!("펜 (벡터)"), "P", Key::P),
    (Tool::Eraser, crate::i18n::k!("지우개"), "E", Key::E),
    (Tool::Text, crate::i18n::k!("글자"), "T", Key::T),
    (Tool::Line, crate::i18n::k!("선"), "L", Key::L),
    (Tool::Arrow, crate::i18n::k!("화살표"), "A", Key::A),
    (Tool::Rect, crate::i18n::k!("사각형"), "R", Key::R),
    (Tool::Ellipse, crate::i18n::k!("원"), "O", Key::O),
    (Tool::Crop, crate::i18n::k!("사진 자르기"), "C", Key::C),
    (Tool::Frame, crate::i18n::k!("내보낼 영역"), "F", Key::F),
    (Tool::Space, crate::i18n::k!("3D 공간"), "K", Key::K),
    (Tool::Picker, crate::i18n::k!("스포이드"), "I", Key::I),
];

/// A rail button's name, key and how to use it.
type RailHint = (&'static str, &'static str, &'static str);

/// How to use a tool, shown in the box beside the rail one action per line
/// (Ash, 10-27: the hints left the tool card, which starts with the settings).
fn tool_hint(t: Tool) -> &'static str {
    match t {
        Tool::Brush => crate::i18n::tr("Ctrl+끌기: 크기 · Alt+클릭: 팔레트 · [ ]: 크기"),
        Tool::Pen => crate::i18n::tr("획 단위로 저장 · 지우개는 한 획씩 지움 · Ctrl+끌기: 굵기"),
        Tool::Eraser => crate::i18n::tr("브러시 레이어: 픽셀 지움 · 펜 레이어: 닿은 획 지움 · Ctrl+끌기: 크기"),
        Tool::Text => crate::i18n::tr("빈 곳 클릭: 새 글 · 글 클릭: 수정 · Esc: 끝내기"),
        Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse => crate::i18n::tr("Shift: 수평·수직·45° / 정사각·정원 · Ctrl+끌기: 굵기 (선택한 도형에도 적용)"),
        Tool::Crop => crate::i18n::tr("사진 선택 → 테두리 손잡이로 자르기 · 안쪽 끌기: 위치 이동 · Enter/Esc: 끝"),
        Tool::Frame => crate::i18n::tr("끌어서 영역 지정 · 손잡이로 조절 · Ctrl+E: 내보내기"),
        Tool::Select => crate::i18n::tr("클릭: 선택 · 끌기: 이동 · Shift+끌기: 영역 선택(중심 기준, Ctrl 함께: 완전히 포함) · Shift+클릭: 추가/빼기 · 펜은 획, 브러시는 영역 안 픽셀만 · 모서리: 크기 · 위 동그라미: 회전"),
        Tool::Lasso => crate::i18n::tr("자유롭게 둘러 선택 · 펜은 획, 브러시는 둘러싼 픽셀만, 나머지는 중심 기준 · 선택 안쪽 끌기: 이동"),
        Tool::Hand => crate::i18n::tr("휠: 확대/축소 · 가운데 버튼(펜 버튼)/Space+끌기: 이동 · Shift+가운데 끌기: 확대/축소 · F12: 입력 기록"),
        Tool::Picker => crate::i18n::tr("캔버스에서 색 집기"),
        Tool::Space => crate::i18n::tr("끌기: 공간 영역 (사진 클릭: 사진에 맞춤) · 선 긋기: 소실선 · 1/2/3 또는 X/Y/Z: 축 · 끝점 끌기 · Delete: 선 지우기"),
    }
}

/// The UI scale's range (Ash, 10-27: 90–150 %).
const UI_SCALE: (f32, f32) = (0.9, 1.5);

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
    /// The text tool's own colour: the last one a text was given (not the
    /// main colour; Ash 11-12).
    pub text_color: Color32,
    pub text_bg: bool,
    pub text_bg_color: Color32,
    pub text_look: crate::doc::TextLook,
    /// Last border / outline colours, for when they are switched on again.
    pub text_border_color: Color32,
    pub text_stroke_color: Color32,
    /// 1 once the text tool's font has moved to `fontdb::DEFAULT_FAMILY`
    /// (settings from before had no family named: the old default).
    pub font_v: u8,
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
    /// The scissors' edge: the photo's own measured softness (true) or a
    /// crisp anti-aliased cut on the sub-pixel line.
    pub scissor_soft: bool,
    /// Size of the whole UI, 0.9–1.5 (Ash, 10-27; 보기 · Ctrl+Alt +/−/0).
    pub ui_scale: f32,
    /// Glass corners: card radius (pt) and smoothing 0..1 (Ash, 10-28; 보기).
    pub corner_radius: f32,
    pub corner_smooth: f32,
    /// UI language ("ko", "en", "ja"); None: the system's on first start (Ash, 10-29).
    #[serde(default)]
    pub lang: Option<String>,
    /// Ask GitHub for a newer release at start (`update.rs`; Ash, 11-02).
    #[serde(default = "yes")]
    pub update_check: bool,
}

fn yes() -> bool {
    true
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
            text_family: crate::fontdb::DEFAULT_FAMILY.into(),
            text_style: "Regular".into(),
            text_align: Default::default(),
            text_color: Color32::from_rgb(230, 57, 70),
            text_bg: false,
            text_bg_color: Color32::from_rgba_unmultiplied(255, 255, 255, 220),
            text_look: Default::default(),
            text_border_color: Color32::from_rgb(30, 30, 30),
            text_stroke_color: Color32::from_rgb(20, 20, 20),
            font_v: 0,
            touch_draw: false,
            theme: 1,
            peek_delay: 0.2,
            folded: [false; 3],
            wand_tol: 24.0,
            mask_refine: 2.0,
            mask_grow: 1.0,
            scissor_soft: false,
            ui_scale: 1.0,
            corner_radius: theme::RADIUS_DEFAULT,
            corner_smooth: theme::SMOOTH_DEFAULT,
            lang: None,
            update_check: true,
        }
    }
}

/// Intelligent scissors in progress: the outline so far, its anchors and the
/// live wire from the last one.
struct Scissor {
    mid: LayerId,
    on: bool,
    /// The outline (sub-pixel, on the edges) and the edge's blur at each of
    /// its points (σ, in the edge map's cells).
    pts: Vec<Pos2>,
    sig: Vec<f32>,
    anchors: Vec<Pos2>,
    wire: Option<crate::segment::LiveWire>,
    /// While tracing with a left drag: the live path last time and how many
    /// updates each of its points has stayed (path cooling).
    last: Vec<Pos2>,
    age: Vec<u32>,
}

/// Scissors: the edge map around an anchor (cells), how near its border a new
/// anchor may come before a new map is made, the click snap radius and the
/// path cooling rules (screen px, updates), the growth budget per frame.
const SCISSOR_MAP: f32 = 1536.0;
const SCISSOR_MARGIN: f32 = 192.0;
const SCISSOR_SNAP_PX: f32 = 10.0;
const COOL_AGE: u32 = 6;
const COOL_MIN_PX: f32 = 24.0;
const COOL_BACK_PX: f32 = 16.0;
const SCISSOR_BUDGET: std::time::Duration = std::time::Duration::from_millis(6);

enum Drag {
    Pan,
    /// Tracing with the scissors (left drag): anchors come by path cooling;
    /// `fresh` when the press started the outline.
    Scissor { start: Pos2, moved: bool, fresh: bool, refine: bool },
    Move { id: LayerId, start: Pos2, orig: Layer },
    ScaleImage { id: LayerId, sign: Vec2, orig: ImageLayer },
    Rotate { id: LayerId, orig: ImageLayer },
    ShapePt { id: LayerId, which: u8 },
    TextWrap { id: LayerId },
    NewShape { id: LayerId, start: Pos2 },
    Crop { id: LayerId, handle: u8, start_src: Pos2, orig: ImageLayer },
    Frame { handle: u8, start: Pos2, orig: Rect },
    Raster { id: LayerId, stroke: Option<RasterStroke>, smooth: (Pos2, f32), curve: crate::vector::Curve },
    Vector { id: LayerId, pts: Vec<[f32; 3]>, smooth: (Pos2, f32), base_r: f32, pressure: bool, color: Color32, curve: crate::vector::Curve },
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
            Op::Grab => crate::i18n::tr("이동"),
            Op::Rotate => crate::i18n::tr("회전"),
            Op::Scale => crate::i18n::tr("크기"),
        };
        let axis = match self.axis {
            Some(0) => crate::i18n::tr(" · X 축"),
            Some(_) => crate::i18n::tr(" · Y 축"),
            None => "",
        };
        let typed = if self.typed.is_empty() { String::new() } else { format!(" · {}", self.typed) };
        crate::i18n::trf("{}{}{}   X/Y 축 · 숫자 입력 · Ctrl 스냅 · Shift 정밀 · Enter/클릭 확정 · Esc/우클릭 취소", &[format!("{op}"), format!("{axis}"), format!("{typed}")])
    }
}

/// A video layer that is playing: its decoder and the clock it follows.
struct Playback {
    /// The decoder; None while the frames come from the video cache (`vcache`).
    player: Option<crate::video::Player>,
    /// Video time and wall time when it started, and the speed.
    t0: f64,
    wall0: f64,
    speed: f64,
    /// A decoded frame that is not due yet.
    pending: Option<(f64, image::RgbaImage)>,
    /// From the cache: the frame shown last.
    shown: Option<u64>,
}

/// A row of the timeline: a layer's or a canvas camera's head row (channel
/// 0: name, span or clip), or one of its key rows (one value's channel,
/// `anim::ch`): each family of keys on a row of its own (Ash, 10-29).
#[derive(Clone, Copy, PartialEq, Debug)]
enum TlRow {
    Layer(LayerId, u16),
    Cam(u64, u16),
}

impl TlRow {
    /// The channel whose keys the row holds (0: the head row, no keys).
    fn ch(self) -> u16 {
        match self {
            TlRow::Layer(_, c) | TlRow::Cam(_, c) => c,
        }
    }

    /// The head row of the same layer or camera.
    fn head(self) -> TlRow {
        match self {
            TlRow::Layer(id, _) => TlRow::Layer(id, 0),
            TlRow::Cam(id, _) => TlRow::Cam(id, 0),
        }
    }
}

/// A key row's label: the value it keys (an FX layer's own names for its values).
fn ch_label(l: Option<&Layer>, c: u16) -> String {
    use crate::anim::ch;
    if let Some(i) = ch::FX.iter().position(|&f| f == c) {
        let name = l.and_then(|l| match &l.kind {
            LayerKind::Fx(fx) => fx.filter.params().get(i).map(|p| crate::i18n::tr(p.name)),
            _ => None,
        });
        return name.unwrap_or(crate::i18n::tr("필터 값")).to_string();
    }
    match c {
        ch::POS => crate::i18n::tr("위치"),
        ch::SIZE => crate::i18n::tr("크기"),
        ch::ROT => crate::i18n::tr("회전"),
        ch::OPA => crate::i18n::tr("불투명도"),
        ch::P_POS => crate::i18n::tr("3D 위치"),
        ch::P_ROT => crate::i18n::tr("3D 회전"),
        ch::P_SCALE => crate::i18n::tr("3D 배율"),
        ch::P_THICK => crate::i18n::tr("두께"),
        ch::FOLLOW => crate::i18n::tr("따라가기"),
        ch::VOL => crate::i18n::tr("볼륨"),
        _ => crate::i18n::tr("값"),
    }
    .to_string()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum TlDrag {
    Scrub,
    Key { row: TlRow, from: f64 },
    SpanIn(TlRow),
    SpanOut(TlRow),
    /// Sound or video clip: move (grabbed at `grab` seconds into it) or trim
    /// either end (`AudioIn`, `AudioOut`).
    AudioMove { id: LayerId, grab: f64 },
    /// Video clip, Alt-drag: slide the video inside the clip (the clip stays;
    /// `from` is the video start when the drag began, `at` the pointer time).
    VideoSlip { id: LayerId, from: f64, at: f64 },
    /// Shift-drag box over the tracks.
    Marquee { start: Pos2 },
    /// Several selected keys moving together (last snapped time under the pointer).
    Keys { last: f64 },
    AudioIn(LayerId),
    AudioOut(LayerId),
}

/// Audio clip colour on the timeline.
const AUDIO_COLOR: Color32 = Color32::from_rgb(90, 200, 170);

/// Video clip colour on the timeline.
const VIDEO_COLOR: Color32 = Color32::from_rgb(130, 150, 255);

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
    TextBorder(LayerId),
    TextStroke(LayerId),
    ShapeColor(LayerId),
    ShapeFill(LayerId),
    /// An FX layer's gradient map colour (0 dark, 1 light).
    FxColor(LayerId, usize),
    DefaultTextColor,
    DefaultTextBg,
    DefaultTextBorder,
    DefaultTextStroke,
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

/// Undo and redo bring back the document, not where one was looking (Ash,
/// 11-12): the playhead, the timeline's own switch and the auto-key switch
/// stay as `now` has them. Where that moves the restored document's time,
/// its keyed layers show the playhead's moment (at the same time they keep
/// what they had, a change without a key included). A video layer's own
/// preview position stays too (the same clip in both).
fn keep_navigation(doc: &mut Document, now: &Document) {
    for l in &mut doc.layers {
        let LayerKind::Image(im) = &mut l.kind else { continue };
        let Some(v) = im.video.as_mut() else { continue };
        if let Some(LayerKind::Image(cur)) = now.get(l.id).map(|c| &c.kind) {
            if let Some(cv) = cur.video.as_ref().filter(|cv| cv.path == v.path) {
                v.time = cv.time;
            }
        }
    }
    let now = &now.timeline;
    let tl = &mut doc.timeline;
    tl.on = now.on;
    tl.autokey = now.autokey;
    let t = now.time.clamp(0.0, tl.duration.max(0.0));
    if (t - tl.time).abs() > 1e-9 {
        tl.time = t;
        crate::anim::evaluate(doc);
    }
}

/// The hue at the top (and bottom) of the palette's hue bar: pure red. The
/// colour model's own zero is a pinkish red, so the bar is turned to start here.
fn hue_top() -> f32 {
    static H: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *H.get_or_init(|| crate::okhsv::from_rgb(Color32::from_rgb(255, 0, 0), 0.0).0)
}

/// Swatch (opens the palette for this field) + editable hex + copy / paste.
fn color_row(ui: &mut egui::Ui, c: &mut Color32, target: PalTarget, req: &mut PalReq) -> bool {
    color_row_mx(ui, c, target, req, false)
}

/// As `color_row`; `mixed`: the selected layers' colours differ (a blank
/// swatch and "—"; a colour set here goes to all of them).
fn color_row_mx(ui: &mut egui::Ui, c: &mut Color32, target: PalTarget, req: &mut PalReq, mixed: bool) -> bool {
    let mut changed = false;
    // in a wrapping row, the whole colour row goes down when it won't fit
    // beside its label (not its last buttons alone); its width as last drawn
    let wid = ui.id().with(("color_row_w", format!("{target:?}")));
    let need = ui.data(|d| d.get_temp::<f32>(wid)).unwrap_or(240.0);
    let room = ui.max_rect().right() - ui.cursor().min.x;
    if room < need && room < ui.max_rect().width() - 1.0 {
        ui.end_row();
    }
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    if mixed {
        ui.painter().rect_filled(r, 5.0, FILL_1());
        ui.painter().text(r.center(), Align2::CENTER_CENTER, crate::dial::MIXED, FontId::proportional(theme::TEXT_BODY), INK());
    } else {
        ui.painter().rect_filled(r, 5.0, *c);
    }
    ui.painter().rect_stroke(r, 5.0, Stroke::new(1.0, if resp.hovered() { INK_2() } else { FILL_3() }), StrokeKind::Inside);
    if resp.glass_tip(crate::i18n::tr("팔레트 열기")).clicked() {
        *req = Some((target, *c, r.right_top()));
    }
    let id = ui.id().with(("hex", format!("{target:?}")));
    let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| hex_of(*c));
    let te = ui.add(egui::TextEdit::singleline(&mut buf).hint_text(crate::dial::MIXED).desired_width(76.0).font(egui::TextStyle::Monospace)); // "#RRGGBBAA" fits
    if te.changed() {
        if let Some(nc) = parse_hex(&buf) {
            *c = nc;
            changed = true;
        }
    }
    if !te.has_focus() {
        buf = if mixed { String::new() } else { hex_of(*c) };
    }
    ui.data_mut(|d| d.insert_temp(id, buf));
    if ui.add_enabled(!mixed, egui::Button::new(crate::i18n::tr("복사")).small()).glass_tip(crate::i18n::tr("헥스 코드 복사")).clicked() {
        ui.ctx().copy_text(hex_of(*c));
    }
    let paste = ui.small_button(crate::i18n::tr("붙여넣기"));
    if (paste.rect.center().y - r.center().y).abs() < 4.0 {
        ui.data_mut(|d| d.insert_temp(wid, paste.rect.right() - r.left()));
    }
    if paste.glass_tip(crate::i18n::tr("클립보드의 헥스 코드 적용")).clicked() {
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
    /// The size was picked by hand; until then it follows the region's own
    /// pixels (one canvas pixel per output pixel).
    size_set: bool,
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
    /// Newer releases (`update.rs`); `update_go`: install once this quits.
    upd: crate::update::Updater,
    update_go: Option<PathBuf>,
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
    /// The rail button under the pointer this frame (name, key, hint) and its rect.
    rail_hover: Option<(RailHint, Rect)>,
    /// The hint box beside the rail: what it shows, beside which button, since
    /// when hovered, and whether it is open (Ash, 10-27: a box beside the rail
    /// instead of a tooltip at the pointer).
    rail_hint: Option<(RailHint, Rect, f64, bool)>,
    /// The hint box's height last frame (to keep it inside the rail's span).
    rail_hint_h: f32,
    space_held: bool,
    layer_drag: Option<LayerId>,
    renaming: Option<LayerId>,
    rename_buf: String,
    rename_focus: bool,
    last_export: Option<ExportDlg>,
    prefs_saved: String,
    /// This run's recovery copy (`recovery.rs`; None: off, as in debug runs).
    rec: Option<crate::recovery::Session>,
    /// The document revision the copy holds, and when it was last written.
    rec_rev: u64,
    rec_at: f64,
    /// Copies left by sessions that ended without saving, offered at start.
    rec_found: Vec<crate::recovery::Found>,
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
    /// 선 찾기: line candidates of a space (notes/updates/line-suggest.md).
    line_cands: Option<LineCands>,
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
    /// When the current pen or brush stroke last got a pen sample (it catches
    /// up once the pen has rested a little).
    stroke_fed: f64,
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
    /// The next click on the floor places this layer there: standing, or lying (true).
    space_place: Option<(LayerId, bool)>,
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
    /// The canvas on the GPU renderer, drawn into a texture egui shows; None
    /// until it starts (the canvas then paints egui meshes).
    cgpu: Option<CanvasGpu>,
    /// Frames of the canvas rendered ahead for playback (phase 4).
    pcache: crate::playcache::PlayCache,
    /// Bumped on every commit, undo/redo and load (the cache follows it).
    doc_rev: u64,
    /// When the pointer or keyboard last did something (the cache fills after a
    /// short rest).
    last_input: f64,
    /// DV_TIMING while playing: (since, cached frames, live frames, ms drawing
    /// cached, ms drawing live), printed every 2 s.
    cache_stats: (f64, u32, u32, f64, f64),
    /// DV_TIMING: GPU canvas frames and time since `.0`.
    canvas_stats: (f64, u32, f64, f64),
    /// Pre-roll: wall time the play button was pressed, while the video decoders
    /// fill up; the clock waits at its start until every one has a frame.
    tl_arm: Option<f64>,
    /// Decoders started ahead at the paused playhead (video time, player), so play
    /// starts at once; and when the playhead last moved.
    warm: HashMap<LayerId, (f64, crate::video::Player)>,
    warm_moved: f64,
    /// DV_TL_PLAY=s: start timeline playback s seconds after launch; DV_TIMING
    /// then prints the pre-roll and the frames shown / skipped in the first 2 s.
    dv_play_at: Option<f64>,
    play_stats: Option<(f64, u32, u32)>,
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
    /// The timeline's rows scrolled by this many points.
    tl_scroll: f32,
    cam_sel: Option<u64>,
    /// Sound output (opened on first use), decoded audio by file path (None while
    /// decoding), and the selected audio clip.
    audio_out: Option<crate::audio::Output>,
    audio_failed: bool,
    audio_cache: std::collections::HashMap<String, Option<Arc<crate::audio::AudioData>>>,
    /// Sounds still being decoded (in `audio_cache` as None, like ones that failed).
    audio_busy: std::collections::HashSet<String>,
    audio_tx: std::sync::mpsc::Sender<(String, Result<crate::audio::AudioData, String>)>,
    audio_rx: std::sync::mpsc::Receiver<(String, Result<crate::audio::AudioData, String>)>,
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
    /// The edge loupe's picture: (texture, target x, y, zoom, edge map on).
    edge_tex: Option<(egui::TextureHandle, [f32; 4])>,
    middle_moved: bool,
    input_log: std::collections::VecDeque<String>,
    show_log: bool,
}

/// Darkens `area` outside the convex quad `q` (screen corners in order).
fn canvas_dim_outside(painter: &egui::Painter, area: Rect, q: [Pos2; 4], col: Color32) {
    // a ring between a rect around both and the quad, clipped to the area
    let outer = Rect::from_points(&q).union(area).expand(8.0);
    let o = [outer.left_top(), outer.right_top(), outer.right_bottom(), outer.left_bottom()];
    // the quad's corners in the same turning order as the outer rect's
    let area2 = |p: &[Pos2; 4]| (0..4).map(|i| p[i].x * p[(i + 1) % 4].y - p[(i + 1) % 4].x * p[i].y).sum::<f32>();
    let q = if area2(&q) * area2(&o) < 0.0 { [q[0], q[3], q[2], q[1]] } else { q };
    // pair each outer corner with the nearest quad corner
    let start = (0..4).min_by(|a, b| q[*a].distance_sq(o[0]).total_cmp(&q[*b].distance_sq(o[0]))).unwrap();
    let q = [q[start], q[(start + 1) % 4], q[(start + 2) % 4], q[(start + 3) % 4]];
    let mut mesh = egui::Mesh::default();
    for p in o.iter().chain(q.iter()) {
        mesh.colored_vertex(*p, col);
    }
    for k in 0..4u32 {
        let n = (k + 1) % 4;
        mesh.add_triangle(k, n, 4 + n);
        mesh.add_triangle(k, 4 + n, 4 + k);
    }
    painter.with_clip_rect(area).add(Shape::mesh(mesh));
}

/// The canvas renderer and its target textures (registered with egui): the
/// canvas, the 3D workspace and the hold-Tab peek card (`SLOT_*`).
struct CanvasGpu {
    gpu: crate::gpu::Gpu,
    tex: HashMap<u8, (eframe::wgpu::Texture, egui::TextureId)>,
    rs: eframe::egui_wgpu::RenderState,
    /// The last zoom and when it changed.
    zoom: (f32, f64),
}

/// Straight-edge candidates found in a space's frame (canvas coordinates),
/// grouped by vanishing point. Valid while the frame stays as it was.
struct LineCands {
    sid: u64,
    frame: Rect,
    segs: Vec<crate::lines::Seg>,
    groups: Vec<crate::lines::Group>,
    /// Each segment's group.
    of: Vec<Option<usize>>,
    norm: crate::lines::Norm,
}

/// Candidate tints, one per group: neutral hues away from the axis colours
/// (a candidate isn't on an axis until it is taken).
const CAND_COLORS: [Color32; 5] = [
    Color32::from_rgb(255, 170, 60),
    Color32::from_rgb(200, 130, 255),
    Color32::from_rgb(60, 220, 200),
    Color32::from_rgb(255, 120, 200),
    Color32::from_rgb(225, 225, 120),
];

impl LineCands {
    fn of_space(&self, s: &crate::persp::Space) -> bool {
        self.sid == s.id && self.frame == s.frame
    }

    /// Candidates not yet taken as one of the space's lines.
    fn open<'a>(&'a self, s: &'a crate::persp::Space) -> impl Iterator<Item = (usize, &'a crate::lines::Seg)> + 'a {
        self.segs.iter().enumerate().filter(move |(_, c)| !s.lines.iter().any(|l| l.a.distance(c.a) < 0.01 && l.b.distance(c.b) < 0.01))
    }
}

/// `export::render` on the canvas's GPU renderer when there is one (its
/// photos, masks and 3D layers are already there), else on the CPU.
fn render_region(cg: &mut Option<CanvasGpu>, doc: &Document, region: Rect, scale: f32, bg: Option<Color32>) -> Result<tiny_skia::Pixmap, String> {
    render_region_for(cg, doc, region, scale, bg, false)
}

/// As `render_region`; `output`: the picture leaves the app (the clipboard),
/// so one with FX layers fails rather than come from the CPU without them
/// (the tools that only look at the picture take it either way).
fn render_region_for(cg: &mut Option<CanvasGpu>, doc: &Document, region: Rect, scale: f32, bg: Option<Color32>, output: bool) -> Result<tiny_skia::Pixmap, String> {
    let mut why = crate::i18n::tr("GPU 사용 불가").to_string();
    if let Some(cg) = cg.as_mut() {
        if std::env::var_os("DV_CPU_EXPORT").is_none() {
            let g = &mut cg.gpu;
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.render_aside(doc, region, scale, bg)));
            match r {
                Ok(Ok(pm)) => return Ok(pm),
                Ok(Err(e)) => why = e,
                Err(_) => {}
            }
        }
    }
    if output && doc.has_fx() {
        return Err(crate::gpu::fx_needs_gpu(&why));
    }
    crate::export::render(doc, region, scale, bg)
}

/// The colour at a canvas point (as `export::sample`), on the GPU when there is one.
fn sample_region(cg: &mut Option<CanvasGpu>, doc: &Document, p: Pos2) -> Option<Color32> {
    let pm = render_region(cg, doc, Rect::from_min_size(p.floor(), vec2(1.0, 1.0)), 1.0, None).ok()?;
    let c = pm.pixel(0, 0)?.demultiply();
    (c.alpha() > 0).then(|| Color32::from_rgb(c.red(), c.green(), c.blue()))
}

/// Seconds without a pen sample before a stroke counts as resting and the
/// brush catches up with the pen (a hesitation at a zigzag's corner stays a
/// curve; a real pause brings the brush to the pen).
const STROKE_REST: f64 = 0.1;

/// Screen points between the points of a drawn curve (`vector::Curve`).
const CURVE_STEP: f32 = 1.5;

/// Screen points of smoothing length at 손떨림 보정 1 (the default 0.4: the
/// brush trails the pen by about 24 px).
const SMOOTH_PX: f32 = 60.0;

const SLOT_CANVAS: u8 = 0;
const SLOT_3D: u8 = 1;
const SLOT_PEEK: u8 = 2;

/// Layers drawn on the canvas's GPU renderer into the painter's clip
/// rectangle, through the canvas transform or a 3D view's camera (over the
/// ground when `ground`, else over what is already painted). False when there
/// is no GPU renderer.
fn gpu_layers(cg: &mut Option<CanvasGpu>, slot: u8, painter: &egui::Painter, doc: &Document, what: crate::view3d::Layers, ground: bool) -> bool {
    use crate::view3d::Layers;
    // DV_MESH_3D: the old egui meshes in the 3D workspace (comparisons)
    if std::env::var_os("DV_MESH_3D").is_some() {
        return false;
    }
    let Some(cg) = cg.as_mut() else { return false };
    let rect = painter.clip_rect();
    if !rect.is_positive() {
        return true;
    }
    let ppp = painter.ctx().pixels_per_point();
    let (w, h) = ((rect.width() * ppp).ceil() as u32 + 1, (rect.height() * ppp).ceil() as u32 + 1);
    let (tex, id) = cg.target(slot, w, h);
    let (frac, size) = match what {
        Layers::Canvas(xf) => {
            let gp = ground.then(|| theme::ground_params(w as f32, h as f32, ppp, xf.zoom));
            (cg.gpu.render_canvas(doc, xf.s2w(rect.min), xf.zoom * ppp, &tex, gp.as_ref(), false, None), vec2(w as f32, h as f32) / ppp)
        }
        Layers::Free { cam, space, moving } => {
            let vc = crate::scene::ViewCam { space, cam: cam.clone() };
            (cg.gpu.render_canvas(doc, rect.min, ppp, &tex, None, moving, Some(&vc)), vec2(w as f32, h as f32) / ppp)
        }
    };
    painter.image(id, Rect::from_min_size(rect.min - frac / ppp, size), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    true
}

impl CanvasGpu {
    /// The target texture of `slot` at `w`×`h` pixels (made, or remade at a
    /// new size).
    fn target(&mut self, slot: u8, w: u32, h: u32) -> (eframe::wgpu::Texture, egui::TextureId) {
        use eframe::wgpu;
        if let Some((t, id)) = self.tex.get(&slot) {
            if t.width() == w && t.height() == h {
                return (t.clone(), *id);
            }
        }
        let t = self.rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = t.create_view(&Default::default());
        let mut r = self.rs.renderer.write();
        let id = match self.tex.get(&slot) {
            Some((_, id)) => {
                r.update_egui_texture_from_wgpu_texture(&self.rs.device, &view, wgpu::FilterMode::Nearest, *id);
                *id
            }
            None => r.register_native_texture(&self.rs.device, &view, wgpu::FilterMode::Nearest),
        };
        self.tex.insert(slot, (t.clone(), id));
        (t, id)
    }
}

fn is_paint_layer(k: &LayerKind) -> bool {
    matches!(k, LayerKind::Raster(_) | LayerKind::Vector(_))
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        crate::glass::load_settings();
        cc.egui_ctx.set_fonts(theme::font_defs());
        theme::apply_visuals(&cc.egui_ctx);
        // the UI scale is the app's own setting (saved, Ctrl+Alt keys); egui's
        // Ctrl +/−/0 would clash with Ctrl+0 (전체 보기) and not be kept
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
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
            upd: crate::update::Updater::new(),
            update_go: None,
            rail_active: None,
            rail_hover: None,
            rail_hint: None,
            rail_hint_h: 0.0,
            space_held: false,
            layer_drag: None,
            renaming: None,
            rename_buf: String::new(),
            rename_focus: false,
            last_export: None,
            prefs_saved: String::new(),
            rec: None,
            rec_rev: 0,
            rec_at: f64::MIN,
            rec_found: Vec::new(),
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
            line_cands: None,
            ws3d: false,
            view3d: None,
            modal3d: None,
            light_drag: None,
            img_job: None,
            nav3d: None,
            stroke_fed: 0.0,
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
            pcache: crate::playcache::PlayCache::new(),
            cgpu: cc.wgpu_render_state.as_ref().and_then(|rs| {
                crate::gpu::install(rs);
                crate::glass_gpu::install(rs, 4);
                crate::gpu::Gpu::on_window_device().map(|gpu| CanvasGpu { gpu, tex: HashMap::new(), rs: rs.clone(), zoom: (0.0, 0.0) })
            }),
            doc_rev: 0,
            last_input: 0.0,
            cache_stats: (0.0, 0, 0, 0.0, 0.0),
            canvas_stats: (0.0, 0, 0.0, 0.0),
            tl_arm: None,
            warm: HashMap::new(),
            warm_moved: 0.0,
            dv_play_at: std::env::var("DV_TL_PLAY").ok().and_then(|v| v.parse().ok()),
            play_stats: None,
            tl_key: None,
            tl_sel: Vec::new(),
            tl_scroll: 0.0,
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
            audio_busy: Default::default(),
            audio_tx: audio_ch.0,
            audio_rx: audio_ch.1,
            voice_sig: 0,
            rec_on: false,
            rec_clock: None,
            rec_times: Vec::new(),
            rec_pts: Vec::new(),
            loupe_tex: None,
            edge_tex: None,
            middle_moved: false,
            input_log: Default::default(),
            show_log: false,
        };
        crate::penhook::install(cc);
        crate::oledrop::install(cc, &cc.egui_ctx);
        if let Some(text) = prefs_path().and_then(|p| std::fs::read_to_string(p).ok()) {
            // each part on its own: one that cannot be read (another version's
            // export dialog, a damaged value) does not take the settings with it
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                let pr = PrefsIn {
                    settings: v.get("settings").cloned().and_then(|x| serde_json::from_value(x).ok()),
                    recent: v.get("recent").cloned().and_then(|x| serde_json::from_value(x).ok()).unwrap_or_default(),
                    export: v.get("export").cloned().and_then(|x| serde_json::from_value(x).ok()),
                };
                if let Some(st) = pr.settings {
                    app.s = st;
                }
                // settings from before the shipped default: the text tool
                // moves to it once (texts already made keep their font)
                if app.s.font_v < 1 {
                    if app.s.text_family.is_empty() {
                        app.s.text_family = crate::fontdb::DEFAULT_FAMILY.into();
                        app.s.text_style = "Regular".into();
                    }
                    app.s.font_v = 1;
                }
                app.recent = pr.recent;
                app.last_export = pr.export;
                app.prefs_saved = text;
            }
        }
        // recovery copies: off in debug runs (DV_* flags), unless they name a
        // folder for them (DV_RECOVERY_DIR)
        let debug_run = std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("DV_"));
        if !debug_run || std::env::var_os("DV_RECOVERY_DIR").is_some() {
            if let Some(dir) = crate::recovery::dir() {
                app.rec_found = crate::recovery::found_in(&dir);
                app.rec = crate::recovery::Session::start(&dir);
                // started by an update's installer: the project that was open
                // (a file given to open comes first)
                if let Some(p) = crate::recovery::take_reopen(&dir).filter(|_| open.is_none()) {
                    app.open_path(p);
                }
            }
        }
        // the UI language: as chosen, else the system's (DV_LANG=en|ja|ko for captures)
        let lang = std::env::var("DV_LANG").ok().and_then(|c| crate::i18n::Lang::from_code(&c)).or_else(|| app.s.lang.as_deref().and_then(crate::i18n::Lang::from_code)).unwrap_or_else(crate::i18n::system_lang);
        crate::i18n::set_lang(lang);
        cc.egui_ctx.set_fonts(theme::font_defs());
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
            // (test content, not UI text)
            let body = if al.is_some() { "가운데 정렬 Test
짧은 줄
조금 더 긴 줄입니다" } else { "테스트 Test" };
            if let Some(a) = al {
                app.s.text_align = a;
                app.s.color = Color32::from_rgb(255, 225, 90);
            }
            if std::env::var_os("DV_TEST_TEXT_LOOK").is_some() {
                // box padding, corners, border and letter outline all set
                app.s.color = Color32::from_rgb(255, 225, 90);
                app.s.text_size = 72.0;
                app.s.text_bg = true;
                app.s.text_bg_color = Color32::from_rgb(40, 60, 90);
                app.s.text_look = crate::doc::TextLook { pad_x: 0.6, pad_y: 0.25, radius: 0.5, border: Some(Color32::from_rgb(230, 70, 70)), border_w: 0.06, stroke: Some(Color32::BLACK), stroke_w: 0.1 };
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
                let id = app.new_text_layer(pos2(-300.0, -150.0 + i as f32 * 120.0), crate::i18n::trf("글자 {}", &[format!("{}", i + 1)]));
                if let Some(LayerKind::Text(t)) = app.doc.get_mut(id).map(|l| &mut l.kind) {
                    t.rotation = f32::to_radians(deg);
                    // "style": the second also differs in every style value (mixed "—")
                    if i == 1 && std::env::var("DV_MULTI_DEMO").is_ok_and(|v| v == "style") {
                        t.color = Color32::from_rgb(255, 120, 90);
                        t.family = "Arial".into();
                        t.style = "Bold".into();
                        t.blend = Blend::Add;
                        t.align = if t.align == crate::doc::TextAlign::Right { crate::doc::TextAlign::Left } else { crate::doc::TextAlign::Right };
                        t.bg = Some(Color32::from_rgb(40, 40, 60));
                    }
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
            let txt = crate::i18n::tr("가나다라마바사아자차카타파하 ABCDEFGHIJ abcdefghij gjpqy 0123456789 ").repeat(reps);
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
                let mut sp = crate::persp::Space::new(id, crate::i18n::tr("공간 1").into(), b);
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
                if std::env::var_os("DV_LINE_FIND").is_some() {
                    // 선 찾기 at start; DV_LINE_AUTO=1 then presses 자동 선택
                    app.find_lines();
                    if std::env::var_os("DV_LINE_AUTO").is_some() {
                        app.auto_pick_lines();
                    }
                }
                if let Ok(mode) = std::env::var("DV_SPACE_DEMO") {
                    // two text layers dropped in: one kept camera-facing, one snapped to
                    // the space axes; "select" leaves the first selected in the select tool
                    let o = app.doc.spaces.last().unwrap().origin;
                    let t1 = app.new_text_layer(o - vec2(160.0, 120.0), "Drifting VOID".into());
                    let t2_text = std::env::var("DV_SPACE_TEXT").unwrap_or_else(|_| crate::i18n::tr("벽에 붙인 글자").into());
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
                                app.doc.spaces.last_mut().unwrap().walls.push(crate::persp::Wall { id: wid, name: crate::i18n::tr("벽 1").into(), corners: q, offset: 0.0, snap_axis: false });
                                app.wall_sel = Some(wid);
                                app.put_in_wall(t2, id, wid);
                            }
                        }
                    }
                    // DV_STAND / DV_LIE="x,y" (photo pixels): the first text stands / lies on the floor there
                    for (var, lie) in [("DV_STAND", false), ("DV_LIE", true)] {
                        let Ok(v) = std::env::var(var) else { continue };
                        let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        if n.len() == 2 {
                            if let Some(im) = app.doc.layers.iter().find_map(|l| match &l.kind {
                                LayerKind::Image(im) => Some(im.clone()),
                                _ => None,
                            }) {
                                app.stand_on_floor(t1, id, im.src_to_world(pos2(n[0], n[1])), lie);
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
                            app.doc.spaces.last_mut().unwrap().walls.push(crate::persp::Wall { id: wid, name: crate::i18n::tr("벽 1").into(), corners: [pts[0], pts[1], pts[2], pts[3]], offset: 0.0, snap_axis: false });
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
                                // DV_SEGS=n, DV_SMOOTH=1: outline detail and smooth sides
                                if let Some(n) = std::env::var("DV_SEGS").ok().and_then(|v| v.parse::<u8>().ok()) {
                                    pl.segs = n.clamp(1, 4);
                                }
                                pl.smooth = std::env::var_os("DV_SMOOTH").is_some();
                                // DV_ENGRAVE=1: carved in (음각)
                                pl.engrave = std::env::var_os("DV_ENGRAVE").is_some();
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
            // DV_SOUND_KEYS=1: volume keys at 0.5 s (1.0) and 1.5 s (0.2) on the first sound
            if std::env::var_os("DV_SOUND_KEYS").is_some() {
                if let Some(l) = app.doc.layers.iter_mut().find(|l| matches!(l.kind, LayerKind::Audio(_))) {
                    let mut an = l.anim.clone().unwrap_or_default();
                    for (t, v) in [(0.5, 1.0f32), (1.5, 0.2)] {
                        let mut k = crate::anim::key_of(l, t);
                        k.ch = crate::anim::ch::VOL;
                        k.vol = v;
                        an.set_key(k);
                    }
                    l.anim = Some(an);
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
            let mut cam = crate::anim::CanvasCam { id: cid, name: crate::i18n::tr("카메라 1").into(), rect: Rect::from_center_size(pos2(0.0, 0.0), vec2(640.0, 360.0)), anim: Default::default(), rot: 0.0, follow: None };
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
        if let Ok(v) = std::env::var("DV_FX") {
            // DV_FX="filter,v0,v1,blend": an FX layer on top, selected (captures);
            // filter: Filter::ALL index; blend: Blend::ALL index, -1 or none 복사
            let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            app.sel = None;
            app.add_fx();
            if let Some(LayerKind::Fx(fx)) = app.sel.and_then(|id| app.doc.get_mut(id)).map(|l| &mut l.kind) {
                let f = crate::doc::Filter::ALL.get(n.first().copied().unwrap_or(0.0) as usize).copied().unwrap_or_default();
                *fx = crate::doc::FxLayer::new(f);
                for (i, x) in n.iter().skip(1).take(2).enumerate() {
                    fx.v[i] = *x;
                }
                if f == crate::doc::Filter::Curves {
                    // an S on RGB and red lifted (captures of the editor)
                    fx.v[0] = 1.0;
                    fx.curves[0] = crate::doc::curve_pts(&[[0.0, 0.0], [0.25, 0.15], [0.75, 0.88], [1.0, 1.0]]);
                    fx.curves[1] = crate::doc::curve_pts(&[[0.0, 0.08], [0.5, 0.62], [1.0, 1.0]]);
                    // the S's upper point: a Free right side and a Constant
                    // left one (captures of the handles and the menu)
                    fx.curves[0][2].l = crate::doc::Tan::Constant;
                    fx.curves[0][2].r = crate::doc::Tan::Free;
                    fx.curves[0][2].rs = 1.2;
                }
                if f == crate::doc::Filter::ChannelAlpha {
                    // red made the alpha, lifted (captures)
                    fx.source = 0;
                    fx.curves[0] = crate::doc::curve_pts(&[[0.0, 0.0], [0.4, 0.7], [1.0, 1.0]]);
                }
                fx.blend = n.get(3).filter(|b| **b >= 0.0).and_then(|b| Blend::ALL.get(*b as usize).copied());
            }
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
                        let mut clicks: Vec<Pos2> = [vec2(-200.0, -200.0), vec2(200.0, -200.0), vec2(200.0, 200.0), vec2(-200.0, 200.0)].iter().map(|d| o + *d).collect();
                        // DV_PHOTO_SCALE=k: the first photo shown at k first (photo masks keep its pixels)
                        if let Some(k) = std::env::var("DV_PHOTO_SCALE").ok().and_then(|v| v.parse::<f32>().ok()) {
                            if let Some(LayerKind::Image(im)) = app.doc.layers.iter_mut().map(|l| &mut l.kind).find(|k| matches!(k, LayerKind::Image(_))) {
                                im.scale = k;
                            }
                        }
                        // DV_SCISSOR_PTS="x,y;x,y;…": the anchors clicked, in pixels of the first photo
                        if let (Ok(v), Some(im)) = (std::env::var("DV_SCISSOR_PTS"), app.doc.layers.iter().find_map(|l| match &l.kind {
                            LayerKind::Image(im) => Some(im.clone()),
                            _ => None,
                        })) {
                            clicks = v.split(';').filter_map(|q| q.split_once(',').and_then(|(x, y)| Some(im.src_to_world(pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))))).collect();
                        }
                        app.zoom = app.zoom.max(1e-3);
                        // DV_SCISSOR_SOFT=1: the edge softness measured in the photo
                        app.s.scissor_soft = std::env::var_os("DV_SCISSOR_SOFT").is_some();
                        for p in clicks {
                            app.scissor_click(mid, p, true, false);
                        }
                        if let Some(sc) = &app.scissor {
                            eprintln!("scissor anchors (snapped): {:?}", sc.anchors);
                            let fin: Vec<f32> = sc.sig.iter().copied().filter(|v| v.is_finite()).collect();
                            eprintln!("outline: {} pts, {} sig, sig min {:.2} max {:.2} mean {:.2}", sc.pts.len(), sc.sig.len(), fin.iter().fold(f32::MAX, |a, b| a.min(*b)), fin.iter().fold(0.0f32, |a, b| a.max(*b)), fin.iter().sum::<f32>() / fin.len().max(1) as f32);
                        }
                        app.scissor_close(std::env::var_os("DV_SCISSOR_CUT").is_some());
                        eprintln!("scissor: {} in {:?}", app.status, t0.elapsed());
                        // DV_SCISSOR_ROW="y,x0,x1" (photo pixels): the mask's coverage along that row
                        if let (Ok(v), Some(im)) = (std::env::var("DV_SCISSOR_ROW"), app.doc.layers.iter().find_map(|l| match &l.kind {
                            LayerKind::Image(im) => Some(im.clone()),
                            _ => None,
                        })) {
                            let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                            if let (3, Some(m)) = (n.len(), app.doc.get(mid)) {
                                let map = crate::export::mask_map(&app.doc, m);
                                let row: Vec<String> = (n[1] as i32..=n[2] as i32).map(|x| format!("{:.2}", crate::export::mask_hide(m, map, im.src_to_world(pos2(x as f32 + 0.5, n[0] + 0.5))))).collect();
                                eprintln!("mask row y={}: {}", n[0], row.join(" "));
                            }
                        }
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
        // DV_EXPORT_FRAME="x,y,w,h": an export frame (export dialog captures)
        if let Ok(v) = std::env::var("DV_EXPORT_FRAME") {
            let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if n.len() == 4 {
                app.doc.export_frame = Some(Rect::from_min_size(pos2(n[0], n[1]), vec2(n[2], n[3])));
            }
        }
        // DV_VIDEO_CLIP="from,to,video_start": the first video layer's clip
        // (seconds; a negative video start or a span past the end shows stills)
        if let Ok(v) = std::env::var("DV_VIDEO_CLIP") {
            let n: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            let vid = app.doc.layers.iter().find(|l| matches!(&l.kind, LayerKind::Image(im) if im.video.is_some())).map(|l| l.id);
            if let (3, Some(id)) = (n.len(), vid) {
                let an = app.doc.get_mut(id).unwrap().anim.get_or_insert_with(Default::default);
                an.span = Some([n[0], n[1]]);
                an.video_start = n[2];
                app.doc.timeline.on = true;
                app.doc.timeline.duration = app.doc.timeline.duration.max(n[1] + 1.0);
                app.select_only(Some(id));
                // as moving the playhead does: paused videos show their frame
                let t = app.doc.timeline.time;
                app.set_time(t);
            }
        }
        // DV_SHOW_VEXPORT=1 (=mbps: with the bitrate chosen)
        if let Ok(v) = std::env::var("DV_SHOW_VEXPORT") {
            app.open_video_export();
            if let Some(d) = app.vexport.as_mut().filter(|_| v == "mbps") {
                d.use_mbps = true;
            }
        }
        // DV_SELECT_ALL=1: every unlocked layer selected (group box captures)
        if std::env::var_os("DV_SELECT_ALL").is_some() {
            let ids: Vec<LayerId> = app.doc.layers.iter().filter(|l| !l.locked && l.mask_of.is_none()).map(|l| l.id).collect();
            app.sel = ids.first().copied();
            app.extra_sel = ids.iter().skip(1).copied().collect();
        }
        // DV_LOCK_SEL=1: the selected layer starts locked (captures of the locked panel)
        if std::env::var_os("DV_LOCK_SEL").is_some() {
            if let Some(l) = app.sel.and_then(|id| app.doc.get_mut(id)) {
                l.locked = true;
            }
        }
        // DV_TOOL=n: start with the n-th tool of the rail (panel captures)
        if let Some(t) = std::env::var("DV_TOOL").ok().and_then(|v| v.parse::<usize>().ok()).and_then(|i| TOOLS.get(i)) {
            app.tool = t.0;
        }
        // DV_FOLD_TOOL=1: the tool section starts folded (captures of the selection);
        // =cams: the selection folded too and the cameras open
        if let Ok(v) = std::env::var("DV_FOLD_TOOL") {
            app.s.folded[0] = true;
            if v == "cams" {
                app.s.folded[1] = true;
                app.s.folded[2] = false;
            }
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

    /// What the playback cache films: the region last picked for a video
    /// export, else the selected camera, the first camera, the export frame;
    /// the view when there is none (Ash, 10-21).
    fn cache_target(&self, view: crate::playcache::View) -> crate::playcache::Target {
        use crate::playcache::Target;
        let cam_ok = |id: u64| self.doc.cameras.iter().any(|c| c.id == id);
        let picked = self.vexport.as_ref().or(self.vexport_last.as_ref()).map(|d| d.region);
        match picked {
            Some(ExportRegion::Camera(id)) if cam_ok(id) => return Target::Camera(id),
            Some(ExportRegion::Frame) if self.doc.export_frame.is_some() => return Target::Frame(self.doc.export_frame.unwrap()),
            _ => {}
        }
        if let Some(id) = self.cam_sel.filter(|id| cam_ok(*id)) {
            return Target::Camera(id);
        }
        if let Some(c) = self.doc.cameras.first() {
            return Target::Camera(c.id);
        }
        if let Some(r) = self.doc.export_frame {
            return Target::Frame(r);
        }
        Target::View(view)
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
                    let before = self.committed.cameras.iter().find(|c| c.id == id).cloned();
                    if let Some(c) = self.doc.cameras.iter_mut().find(|c| c.id == id) {
                        let mut k = c.key(t);
                        k.ch = before.map_or(k.ch, |b| crate::anim::autokey_channels_cam(&b, c, t));
                        if k.ch != 0 {
                            c.anim.set_key(k);
                        }
                    }
                }
            }
            for id in self.selected_ids() {
                let before = self.committed.get(id).cloned();
                if let Some(l) = self.doc.get_mut(id) {
                    let mut key = crate::anim::key_of(l, t);
                    key.ch = before.map_or(key.ch, |b| crate::anim::autokey_channels(&b, l, t));
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
            self.doc_rev += 1;
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
        keep_navigation(&mut self.doc, &doc);
        if self.tl_play.is_none() {
            self.sync_videos_to_timeline();
        }
        let cur = Snap {
            doc,
            sel: std::mem::replace(&mut self.sel, u.sel),
            extra: std::mem::replace(&mut self.extra_sel, u.extra),
            sub: std::mem::replace(&mut self.sub, u.sub),
            cost,
        };
        self.committed = self.doc.clone();
        self.doc_rev += 1;
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
        // nothing that points into the document being left stays: layer ids
        // start again from 1 in the next one and would name other layers
        if self.tl_play.is_some() {
            self.tl_toggle_play();
        }
        if let Some(o) = &self.audio_out {
            o.stop();
        }
        self.playing.clear();
        self.warm.clear();
        self.scrub_want.clear();
        self.scrub_gen.clear();
        self.scrub_shown.clear();
        self.tl_arm = None;
        self.tl_drag = None;
        self.tl_key = None;
        self.tl_sel.clear();
        self.grs2d = None;
        self.modal3d = None;
        self.nav3d = None;
        self.light_drag = None;
        self.layer_drag = None;
        self.renaming = None;
        self.palette = None;
        self.wand = None;
        self.scissor = None;
        self.mask_rect = None;
        self.mask_cut = false;
        self.line_cands = None;
        self.space_sel = None;
        self.space_line = None;
        self.space_place = None;
        self.wall_sel = None;
        self.cam_sel = None;
        self.view3d = None;
        self.ws3d = false;
        self.extra_sel.clear();
        self.sub = SubSel::default();
        self.doc = doc;
        // a file from before 0.104.3 may hold layers of a deleted space
        let spaces: Vec<u64> = self.doc.spaces.iter().map(|s| s.id).collect();
        for l in &mut self.doc.layers {
            if l.place.as_ref().is_some_and(|p| !spaces.contains(&p.space)) {
                l.place = None;
            }
        }
        self.committed = self.doc.clone();
        self.doc_rev += 1;
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
        self.doc.layers.insert(idx, Layer { id, name, visible: true, locked: false, opacity: 1.0, kind, place: None, anim: None, mask_of: None, mask_anchor: None, mask_pose: None, mask_invert: false, mask_res: 1.0 });
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
                self.add_audio(&p, pos);
                pos += vec2(crate::doc::AudioLayer::SIZE * 1.5, 0.0);
                n += 1;
                continue;
            }
            // an animated GIF is a video (a clip on the timeline, looping); a
            // still one is a photo
            let animated_gif = crate::video::is_gif(&p) && crate::video::probe(&p).is_ok_and(|i| i.duration > i.frame_len() * 1.5);
            if crate::video::VIDEO_EXTS.contains(&ext.as_str()) || animated_gif {
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
                    let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or(crate::i18n::tr("사진")).to_string();
                    self.add_image(src, name, pos);
                    pos += vec2(w * 0.5 + 40.0, 0.0);
                    n += 1;
                }
                Err(e) => errs.push(e),
            }
        }
        self.status = if errs.is_empty() {
            crate::i18n::trf("{}개 불러옴", &[format!("{n}")])
        } else {
            crate::i18n::trf("불러오기 실패: {}", &[format!("{}", errs.join(", "))])
        };
    }

    /// Something dropped through `oledrop`. Bytes without a file (a browser's
    /// image) become a photo straight from memory: the .void keeps a photo's
    /// own bytes, so no file is written. Only what a .void refers to by path
    /// (video, sound, an animated GIF, a .void) is written, to
    /// `%APPDATA%/DriftingVOID/drops`, and must stay there.
    fn take_drop(&mut self, d: crate::oledrop::Dropped, at: Pos2) {
        use crate::oledrop::Dropped;
        match d {
            Dropped::Paths(p) => self.import_paths(p, at),
            Dropped::Bytes { name, bytes, url } => {
                let path = std::path::Path::new(&name);
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                let stem = path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or(crate::i18n::tr("사진")).to_string();
                let needs_file = ext == "void" || ext == "gif" || crate::video::VIDEO_EXTS.contains(&ext.as_str()) || crate::audio::AUDIO_EXTS.contains(&ext.as_str());
                if !needs_file {
                    match crate::io::decode_image(bytes, Some(&ext)) {
                        Ok(src) => {
                            self.add_image(src, stem, at);
                            self.status = crate::i18n::trf("{}개 불러옴", &[format!("1")]);
                        }
                        // a link to something that isn't a picture: the link as text
                        Err(_) if url.is_some() => self.take_drop(Dropped::Text(url.unwrap()), at),
                        Err(e) => self.status = crate::i18n::trf("불러오기 실패: {}", &[format!("{name}: {e}")]),
                    }
                    return;
                }
                let dir = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("DriftingVOID").join("drops"));
                let saved = dir.and_then(|d| {
                    std::fs::create_dir_all(&d).ok()?;
                    let safe: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
                    let p = d.join(format!("{}-{safe}", next_rev()));
                    std::fs::write(&p, &bytes).ok().map(|_| p)
                });
                match saved {
                    Some(p) => self.import_paths(vec![p], at),
                    None => self.status = crate::i18n::trf("불러오기 실패: {}", &[name]),
                }
            }
            Dropped::Text(t) => {
                let t = t.replace("\r\n", "\n");
                let id = self.new_text_layer(at, t);
                self.sel = Some(id);
                self.mark();
                self.status = crate::i18n::tr("글자 붙여넣음").into();
            }
            Dropped::NoFetch { url, why } => {
                self.take_drop(Dropped::Text(url), at);
                self.status = crate::i18n::trf("링크를 받을 수 없음 ({}) · 글자로 넣음", &[why]);
            }
            Dropped::Failed(e) => self.status = crate::i18n::trf("불러오기 실패: {}", &[e]),
        }
    }

    /// A video comes in as an image layer showing its first frame.
    fn add_video(&mut self, p: &std::path::Path, at: Pos2) -> Result<f32, String> {
        let info = crate::video::probe(p)?;
        let size = info.preview_size(crate::video::PREVIEW_SIDE);
        let img = crate::video::frame_at(p, 0.0, size)?;
        let src = ImageSource { id: next_rev(), rgba: Arc::new(img), encoded: Arc::new(Vec::new()), ext: "png".into(), logical: Some([info.width, info.height]), nv12: None };
        let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or(crate::i18n::tr("영상")).to_string();
        let id = self.add_image(src, name, at);
        if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
            // GIFs loop, as everywhere they are shown
            let looping = crate::video::is_gif(p);
            im.video = Some(VideoRef { path: p.to_string_lossy().into(), info: info.clone(), time: 0.0, speed: 1.0, looping, mute: false, volume: 1.0, slot: next_rev() });
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
                im.src = ImageSource { id: next_rev(), rgba: Arc::new(img), encoded: Arc::new(Vec::new()), ext: "png".into(), logical: Some([v.info.width, v.info.height]), nv12: None };
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
        // held in the video cache: at once, no seek
        let hit = crate::vcache::get(&v.path, t);
        crate::vcache::SCRUBS[hit.is_none() as usize].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(f) = hit {
            let g = self.scrub_gen.get(&id).copied().unwrap_or(0) + 1;
            self.scrub_gen.insert(id, g);
            self.scrub_shown.insert(id, g);
            self.scrub_want.remove(&id);
            self.set_video_frame(id, t, f.to_rgba());
            return;
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
                let voice = crate::audio::Voice { data, t0: 0.0, src0: t, speed: v.speed, span: (0.0, f64::MAX), gain: v.volume, env: None, wrap: None };
                if let Some(o) = self.audio_output() {
                    o.play(vec![voice], 0.0);
                }
            }
        }
        // the next second held in the video cache: played from memory (no
        // decoder start, no pre-roll wait)
        if crate::vcache::holds(&v.path, t, 1.0) {
            self.playing.insert(id, Playback { player: None, t0: t, wall0: self.now, speed: v.speed, pending: None, shown: None });
            return;
        }
        let size = v.info.preview_size(crate::video::PREVIEW_SIDE);
        match crate::video::Player::start(std::path::Path::new(&v.path), t, v.info.fps, size) {
            Ok(player) => {
                self.playing.insert(id, Playback { player: Some(player), t0: t, wall0: self.now, speed: v.speed, pending: None, shown: None });
            }
            Err(e) => self.status = crate::i18n::trf("재생 실패: {}", &[format!("{e}")]),
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
                self.status = crate::i18n::tr("프레임 → 사진 레이어").into();
                self.mark();
            }
            Err(e) => self.status = crate::i18n::trf("프레임 읽기 실패: {}", &[format!("{e}")]),
        }
    }

    /// The video cache's list: every video layer, the frames its clip shows
    /// (the whole video with the timeline off).
    fn video_cache_wants(&self) -> Vec<crate::vcache::Want> {
        let mut out: Vec<crate::vcache::Want> = Vec::new();
        for l in &self.doc.layers {
            let LayerKind::Image(im) = &l.kind else { continue };
            let Some(v) = &im.video else { continue };
            let fps = v.info.fps.max(1.0);
            let last = ((v.info.duration - v.info.frame_len()).max(0.0) * fps).floor() as u64;
            let mut range = (0, last);
            // (a looping clip comes round to every frame)
            if self.doc.timeline.on && !v.looping {
                if let Some((span, media)) = self.video_clip(l.id) {
                    let speed = v.speed.max(1e-3);
                    let a = ((span[0] - media[0]) * speed).clamp(0.0, v.info.duration);
                    let b = ((span[1] - media[0]) * speed).clamp(0.0, v.info.duration);
                    range = (((a * fps).floor() as u64).min(last), ((b * fps).ceil() as u64).min(last));
                }
            }
            let w = crate::vcache::Want { path: v.path.clone(), fps: v.info.fps, size: v.info.preview_size(crate::video::PREVIEW_SIDE), range, alpha: v.info.alpha };
            if !out.iter().any(|o| o.path == w.path) {
                out.push(w);
            }
        }
        out
    }

    fn tick_videos(&mut self, ctx: &egui::Context) {
        crate::vcache::want(self.video_cache_wants());
        if crate::vcache::busy() && self.doc.timeline.on {
            // the clip bars show the cache filling
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
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
        if self.tl_arm.is_some() {
            // pre-roll: the decoders fill up; their clocks start with the timeline's
            return;
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
            if pb.player.is_none() {
                // from the video cache; where it runs out, a decoder takes over
                let fps = v.info.fps.max(1.0);
                let idx = (clock.min(v.info.duration - v.info.frame_len()).max(0.0) * fps).floor() as u64;
                let end = clock >= v.info.duration - v.info.frame_len() * 0.5;
                if !end && pb.shown != Some(idx) {
                    match crate::vcache::get(&v.path, idx as f64 / fps) {
                        Some(f) => {
                            pb.shown = Some(idx);
                            if let Some(st) = self.play_stats.as_mut() {
                                st.1 += 1;
                            }
                            self.set_video_frame(id, idx as f64 / fps, f.to_rgba());
                        }
                        None => {
                            let size = v.info.preview_size(crate::video::PREVIEW_SIDE);
                            if let Ok(player) = crate::video::Player::start(std::path::Path::new(&v.path), clock, v.info.fps, size) {
                                let pb = self.playing.get_mut(&id).unwrap();
                                pb.player = Some(player);
                                pb.t0 = clock;
                                pb.wall0 = now;
                            }
                        }
                    }
                }
                if end {
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
                continue;
            }
            let player = pb.player.as_ref().unwrap();
            let mut show = None;
            let mut drained = false;
            loop {
                if pb.pending.is_none() {
                    match player.rx.try_recv() {
                        Ok(f) => pb.pending = Some(f),
                        Err(_) => {
                            drained = true;
                            break;
                        }
                    }
                }
                if pb.pending.as_ref().is_some_and(|(t, _)| *t <= clock + 1e-6) {
                    if show.is_some() {
                        if let Some(st) = self.play_stats.as_mut() {
                            st.2 += 1; // a due frame skipped for a later one
                        }
                    }
                    show = pb.pending.take();
                } else {
                    break;
                }
            }
            let done = drained && player.ended.load(std::sync::atomic::Ordering::Relaxed);
            if let Some((t, img)) = show {
                if let Some(st) = self.play_stats.as_mut() {
                    st.1 += 1;
                }
                self.set_video_frame(id, t, img);
            }
            if done || clock > v.info.duration + 1.0 {
                self.playing.remove(&id);
                if self.tl_play.is_none() && self.playing.is_empty() {
                    if let Some(o) = &self.audio_out {
                        o.stop();
                    }
                }
                if v.looping && std::path::Path::new(&v.path).is_file() {
                    if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                        if let Some(vv) = im.video.as_mut() {
                            vv.time = 0.0;
                        }
                    }
                    self.video_play(id, true);
                }
            }
        }
        if let Some((t0, shown, skipped)) = self.play_stats {
            if self.now - t0 >= 2.0 {
                eprintln!("first 2 s: {shown} frames shown, {skipped} skipped");
                self.play_stats = None;
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
                        im.src = ImageSource { id: next_rev(), rgba: Arc::new(img), encoded: Arc::new(Vec::new()), ext: "png".into(), logical: None, nv12: None };
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
        self.warm_moved = self.now;
        let tl = &mut self.doc.timeline;
        tl.time = t.clamp(0.0, tl.duration.max(0.0));
        crate::anim::evaluate(&mut self.doc);
        if self.tl_play.is_none() {
            self.sync_videos_to_timeline();
        }
    }

    /// A video layer's clip on the timeline: (shown span, where the video's
    /// first and last frames fall). Outside the video the layer holds its first
    /// or last frame as a still (Ash, 10-21).
    fn video_clip(&self, id: LayerId) -> Option<([f64; 2], [f64; 2])> {
        let l = self.doc.get(id)?;
        let LayerKind::Image(im) = &l.kind else { return None };
        let v = im.video.as_ref()?;
        let speed = v.speed.max(1e-3);
        let (start, vs) = l.anim.as_ref().map_or((0.0, 0.0), |a| (a.start(), a.video_start));
        let m0 = start - vs / speed;
        let media = [m0, m0 + v.info.duration / speed];
        let span = l.anim.as_ref().and_then(|a| a.span).unwrap_or(media);
        Some((span, media))
    }

    fn timeline_videos(&self) -> Vec<(LayerId, f64)> {
        let t = self.doc.timeline.time;
        self.doc
            .layers
            .iter()
            .filter(|l| self.doc.alive(l))
            .filter_map(|l| match &l.kind {
                LayerKind::Image(im) => im.video.as_ref().map(|v| (l.id, crate::anim::clip_time(l.anim.as_ref(), v, t))),
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
            self.tl_arm = None;
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
        // Pre-roll: the button reads as playing at once, but the clock (and the
        // sound) start only when every video decoder has its first frame, so the
        // picture doesn't stutter while ffmpeg starts and seeks (~0.3–0.5 s).
        // Decoders already warmed at this playhead are taken over as they are.
        self.tl_play = Some((self.doc.timeline.time, self.now));
        // DV_NO_PREROLL: start at once, as before 0.58 (for comparisons)
        self.tl_arm = Some(if std::env::var_os("DV_NO_PREROLL").is_some() { self.now - 10.0 } else { self.now });
        for (id, vt) in self.timeline_videos() {
            if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                if let Some(v) = im.video.as_mut() {
                    v.time = vt.clamp(0.0, v.info.duration);
                }
            }
            let fl = self.video_of(id).map_or(0.03, |v| v.info.frame_len());
            let dur = self.video_of(id).map_or(0.0, |v| v.info.duration);
            if vt < 0.0 || vt >= dur - fl * 2.0 {
                // before the first frame or after the last: a still until it starts
                self.video_scrub(id, vt);
                continue;
            }
            match self.warm.remove(&id) {
                Some((wt, player)) if (wt - vt).abs() <= fl * 0.5 => {
                    let speed = self.video_of(id).map_or(1.0, |v| v.speed);
                    self.playing.insert(id, Playback { player: Some(player), t0: wt, wall0: self.now, speed, pending: None, shown: None });
                }
                _ => self.video_play(id, true),
            }
        }
    }

    /// While paused: a decoder per video layer, started at the playhead once it has
    /// rested for a moment, waiting with its first frames (the channel is full, so
    /// ffmpeg idles). Play takes them over.
    fn warm_decoders(&mut self) {
        if self.tl_play.is_some() || !self.doc.timeline.on || std::env::var_os("DV_NO_PREROLL").is_some() {
            return;
        }
        let want = self.timeline_videos();
        self.warm.retain(|id, _| want.iter().any(|(w, _)| w == id));
        if self.now - self.warm_moved < 0.25 {
            return;
        }
        for (id, vt) in want {
            let Some(v) = self.video_of(id) else { continue };
            let fl = v.info.frame_len();
            if self.warm.get(&id).is_some_and(|(wt, _)| (wt - vt).abs() <= fl * 0.5) {
                continue;
            }
            let (path, fps, size) = (v.path.clone(), v.info.fps, v.info.preview_size(crate::video::PREVIEW_SIDE));
            let vt = vt.clamp(0.0, (v.info.duration - fl).max(0.0));
            if let Ok(player) = crate::video::Player::start(std::path::Path::new(&path), vt, fps, size) {
                self.warm.insert(id, (vt, player));
            }
        }
    }

    fn tick_timeline(&mut self, ctx: &egui::Context) {
        if let Some(at) = self.dv_play_at {
            // the timeline is on from the start, as when Ash plays it
            self.doc.timeline.on = true;
            ctx.request_repaint();
            if self.now >= at {
                self.dv_play_at = None;
                self.tl_toggle_play();
            }
        }
        self.warm_decoders();
        let Some((t0, w0)) = self.tl_play else { return };
        if let Some(armed) = self.tl_arm {
            // every playing video needs a frame in hand (at most 2 s of waiting)
            let mut ready = true;
            for pb in self.playing.values_mut() {
                // from the video cache: ready at once
                let Some(player) = &pb.player else { continue };
                if pb.pending.is_none() {
                    pb.pending = player.rx.try_recv().ok();
                }
                ready &= pb.pending.is_some() || player.ended.load(std::sync::atomic::Ordering::Relaxed);
            }
            if !ready && self.now - armed < 2.0 {
                self.tl_play = Some((t0, self.now));
                ctx.request_repaint();
                return;
            }
            // go: clock, videos and sound from the same instant
            if std::env::var_os("DV_TIMING").is_some() {
                eprintln!("pre-roll {:.0} ms ({} videos)", (self.now - armed) * 1e3, self.playing.len());
                self.play_stats = Some((self.now, 0, 0));
            }
            self.tl_arm = None;
            self.tl_play = Some((t0, self.now));
            let now = self.now;
            for pb in self.playing.values_mut() {
                pb.wall0 = now;
            }
            let voices = self.timeline_voices();
            if !voices.is_empty() {
                if let Some(o) = self.audio_output() {
                    o.play(voices, t0);
                }
            }
            ctx.request_repaint();
            return;
        }
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
            // video layers leaving their span stop
            let now_alive: Vec<(LayerId, bool, bool)> = self.doc.layers.iter().zip(before).map(|(l, b)| (l.id, b, self.doc.alive(l))).collect();
            for (id, was, is) in now_alive {
                if was && !is {
                    self.playing.remove(&id);
                }
            }
            // and start when their first frame comes round (in their span, past
            // the still before it); before that and after the end they hold
            for (id, vt) in self.timeline_videos() {
                if self.playing.contains_key(&id) {
                    continue;
                }
                let Some(v) = self.video_of(id) else { continue };
                if vt >= 0.0 && vt < v.info.duration - v.info.frame_len() * 2.0 {
                    if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                        if let Some(v) = im.video.as_mut() {
                            v.time = vt;
                        }
                    }
                    self.video_play(id, true);
                } else if (v.time - vt.clamp(0.0, v.info.duration)).abs() > v.info.frame_len() && !self.scrub_want.contains_key(&id) {
                    self.video_scrub(id, vt);
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
                self.status = crate::i18n::trf("{} 키 · {}", &[format!("{}", c.name), format!("{}", crate::video::timecode(t))]);
                self.mark();
                return;
            }
        }
        let ids = self.selected_ids();
        if ids.is_empty() {
            self.status = crate::i18n::tr("키 넣을 레이어 없음").into();
            return;
        }
        for id in &ids {
            if let Some(l) = self.doc.get_mut(*id) {
                let key = crate::anim::key_of(l, t);
                l.anim.get_or_insert_with(Default::default).set_key(key);
            }
        }
        self.status = crate::i18n::trf("키 {}개 · {}", &[format!("{}", ids.len()), format!("{}", crate::video::timecode(t))]);
        self.mark();
    }

    /// Automatic selection: renders what lies under the masked layer at full canvas
    /// resolution (one pixel per mask pixel) around `w`, takes the connected region
    /// of similar colour, softens its edge with a guided filter (the picture as the
    /// guide) and hides it (or shows it again).
    fn wand_fill(&mut self, mid: LayerId, w: Pos2, on: bool) {
        self.adopt_photo(mid, w);
        if self.doc.get(mid).and_then(|l| l.mask_of).is_none() {
            return;
        }
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
        let Some(d) = self.trace_doc(mid) else { return Default::default() };
        let Ok(pm) = render_region(&mut self.cgpu, &d, reg, 1.0, Some(Color32::BLACK)) else { return };
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
        self.status = crate::i18n::trf("자동 선택 · {} px {}", &[format!("{n}"), format!("{}", if on { crate::i18n::tr("가림") } else { crate::i18n::tr("되살림") })]);
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
        let vals: Vec<f32> = q.iter().map(|v| if sharpen { ((v - 0.5) * 2.5 + 0.5).clamp(0.0, 1.0) } else { *v }).collect();
        let space = self.mask_space(mid);
        let Some(LayerKind::Raster(m)) = self.doc.get_mut(mid).map(|l| &mut l.kind) else { return 0 };
        if let Some(map) = space {
            return write_grid_mapped(m, map, origin, bw, bh, &vals, on);
        }
        let mut n = 0usize;
        for y in 0..bh {
            for x in 0..bw {
                let v = vals[y * bw + x];
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

    /// The edge map of what lies under the masked layer around `at`: the one
    /// the outline has when `at` is well inside it, else a new one.
    fn scissor_map(&mut self, mid: LayerId, at: Pos2) -> Option<std::sync::Arc<crate::segment::EdgeMap>> {
        let keep = self.scissor.as_ref().filter(|sc| sc.mid == mid).and_then(|sc| sc.wire.as_ref()).map(|wr| wr.map.clone());
        if let Some(m) = keep.filter(|m| m.holds(at, SCISSOR_MARGIN)) {
            return Some(m);
        }
        // a photo mask is searched on its photo's own pixels
        let cell = self.mask_space(mid).map_or(1.0, |m| m.s);
        let reg = Rect::from_center_size(at, vec2(SCISSOR_MAP, SCISSOR_MAP) * cell);
        let reg = Rect::from_min_max(reg.min.floor(), reg.max.ceil());
        let Some(d) = self.trace_doc(mid) else { return Default::default() };
        let pm = render_region(&mut self.cgpu, &d, reg, 1.0 / cell, Some(Color32::BLACK)).ok()?;
        let (pw, ph) = (pm.width() as usize, pm.height() as usize);
        Some(std::sync::Arc::new(crate::segment::EdgeMap::new(pm.data(), pw, ph, reg.min, cell)))
    }

    /// Where an anchor clicked at `w` goes (the strongest edge within a few
    /// screen pixels), and the edge map it lies in.
    fn scissor_anchor_at(&mut self, mid: LayerId, w: Pos2) -> (Pos2, Option<std::sync::Arc<crate::segment::EdgeMap>>) {
        let map = self.scissor_map(mid, w);
        let p = map.as_ref().map_or(w, |m| m.snap(w, SCISSOR_SNAP_PX / self.zoom / m.cell));
        (p, map)
    }

    /// The outline from the last anchor to `w`: along the live wire (as far
    /// as it has grown), moved onto the edges' sub-pixel line, with the edge's
    /// blur at each point; or straight.
    fn scissor_path(sc: &Scissor, w: Pos2) -> Vec<(Pos2, f32)> {
        match sc.wire.as_ref().and_then(|wr| wr.path_to(w).map(|p| wr.map.refine(&p))) {
            Some(p) => p,
            None => vec![(w, 0.0)],
        }
    }

    /// An anchor at `w` (snapped to the edge): the outline follows the live
    /// wire to it; near the first anchor it closes. Starts an outline when
    /// none is in progress.
    fn scissor_click(&mut self, mid: LayerId, w: Pos2, on: bool, refine: bool) {
        let near_first = self.scissor.as_ref().is_some_and(|sc| sc.mid == mid && sc.pts.len() > 2 && (sc.pts[0] - w).length() * self.zoom < 10.0);
        if near_first {
            self.scissor_close(refine);
            return;
        }
        if !self.scissor.as_ref().is_some_and(|sc| sc.mid == mid) {
            self.adopt_photo(mid, w);
        }
        let (p, map) = self.scissor_anchor_at(mid, w);
        let wire = map.and_then(|m| crate::segment::LiveWire::new(m, p));
        match self.scissor.take() {
            Some(mut sc) if sc.mid == mid => {
                if let Some(wr) = sc.wire.as_mut() {
                    wr.grow(Some(p), std::time::Duration::from_millis(300));
                }
                let path = Self::scissor_path(&sc, p);
                for (q, s) in path.into_iter().skip(1) {
                    sc.pts.push(q);
                    sc.sig.push(s);
                }
                sc.anchors.push(*sc.pts.last().unwrap_or(&p));
                sc.wire = wire;
                sc.last.clear();
                sc.age.clear();
                self.scissor = Some(sc);
            }
            _ => {
                self.scissor = Some(Scissor { mid, on, pts: vec![p], sig: vec![0.0], anchors: vec![p], wire, last: Vec::new(), age: Vec::new() });
                self.status = crate::i18n::tr("가위 · 끌기: 경계 따라 자동 · 우클릭: 앵커 · 첫 점/Enter: 닫기 · Esc: 취소").into();
            }
        }
    }

    /// Tracing with a left drag at `w`: the live wire grows to the pointer,
    /// and a stretch of it that has stayed put while the pointer moved on is
    /// fixed with an anchor at its end (path cooling).
    fn scissor_trace(&mut self, w: Pos2) {
        let zoom = self.zoom;
        let cut = {
            let Some(sc) = self.scissor.as_mut() else { return };
            let Some(wr) = sc.wire.as_mut() else { return };
            if !wr.grow(Some(w), std::time::Duration::from_millis(8)) {
                return;
            }
            let Some(path) = wr.path_to(w) else { return };
            // the start shared with the last path has stayed one update more
            let common = sc.last.iter().zip(&path).take_while(|(a, b)| a == b).count();
            sc.age.truncate(common);
            for a in &mut sc.age {
                *a += 1;
            }
            sc.age.resize(path.len(), 0);
            let total: f32 = path.windows(2).map(|s| s[0].distance(s[1])).sum();
            let mut along = 0.0;
            let mut cut = None;
            for i in 1..path.len() {
                along += path[i - 1].distance(path[i]);
                if sc.age[i] >= COOL_AGE && along * zoom >= COOL_MIN_PX && (total - along) * zoom >= COOL_BACK_PX {
                    cut = Some(i);
                }
            }
            sc.last = path.clone();
            let Some(i) = cut else { return };
            for (q, s) in wr.map.refine(&path[..=i]).into_iter().skip(1) {
                sc.pts.push(q);
                sc.sig.push(s);
            }
            sc.anchors.push(*sc.pts.last().unwrap_or(&path[i]));
            sc.last.clear();
            sc.age.clear();
            (sc.mid, path[i])
        };
        let (mid, p) = cut;
        let map = self.scissor_map(mid, p);
        if let Some(sc) = self.scissor.as_mut() {
            sc.wire = map.and_then(|m| crate::segment::LiveWire::new(m, p));
        }
    }

    /// World -> pixel space of mask `mid` when it is a photo mask (None:
    /// canvas pixels).
    fn mask_space(&self, mid: LayerId) -> Option<crate::export::MaskMap> {
        let l = self.doc.get(mid)?;
        if l.mask_res == 1.0 {
            return None;
        }
        crate::export::mask_map(&self.doc, l)
    }

    /// The picture the selection tools trace for mask `mid`: everything but
    /// masks and the masked layer, or with it when that is a photo (cutting a
    /// photo out traces the photo).
    fn trace_doc(&self, mid: LayerId) -> Option<Document> {
        let owner = self.doc.get(mid).and_then(|l| l.mask_of)?;
        let keep_owner = self.doc.get(owner).is_some_and(|l| matches!(l.kind, LayerKind::Image(_)) && l.place.is_none());
        let mut d = self.doc.clone();
        d.layers.retain(|l| (l.id != owner || keep_owner) && l.mask_of.is_none());
        Some(d)
    }

    /// The flat photo the selection tools see at `w` (topmost, visible, not in
    /// a 3D space).
    fn photo_at(&self, mid: LayerId, w: Pos2) -> Option<LayerId> {
        let owner = self.doc.get(mid).and_then(|l| l.mask_of)?;
        let keep_owner = self.doc.get(owner).is_some_and(|l| matches!(l.kind, LayerKind::Image(_)));
        self.doc
            .layers
            .iter()
            .rev()
            .filter(|l| l.visible && l.place.is_none() && l.mask_of.is_none() && (l.id != owner || keep_owner))
            .find(|l| matches!(&l.kind, LayerKind::Image(im) if im.contains(w)))
            .map(|l| l.id)
    }

    /// An empty mask about to be traced at `w` on a photo becomes a photo
    /// mask: its pixels are that photo's own and it follows the photo
    /// (Ash, 10-23: masks at the photo's resolution, kept through resizes).
    fn adopt_photo(&mut self, mid: LayerId, w: Pos2) {
        let Some(pid) = self.photo_at(mid, w) else { return };
        let Some(l) = self.doc.get(mid) else { return };
        let empty = matches!(&l.kind, LayerKind::Raster(r) if r.tiles.is_empty());
        if l.mask_res != 1.0 || !empty {
            return;
        }
        let Some(LayerKind::Image(im)) = self.doc.get(pid).map(|p| &p.kind) else { return };
        let (cc, sc) = (im.crop.center(), im.scale);
        if let Some(l) = self.doc.get_mut(mid) {
            l.mask_anchor = Some((pid, [cc.x, cc.y]));
            l.mask_pose = Some([0.0, sc]);
            l.mask_res = 1.0 / sc.max(1e-6);
            if (l.mask_res - 1.0).abs() < 1e-6 {
                // a photo at 100 %: still a photo mask (follows it, never resampled)
                l.mask_res = 1.0 + 1e-6;
            }
        }
    }

    /// A photo mask turned back into an ordinary one: its pixels resampled
    /// (bilinear) into canvas pixels where it shows now.
    fn bake_photo_mask(&mut self, id: LayerId) {
        let Some(map) = self.mask_space(id) else { return };
        let Some(LayerKind::Raster(r)) = self.doc.get(id).map(|l| &l.kind) else { return };
        let mut out = crate::raster::RasterLayer { blend: r.blend, ..Default::default() };
        if let Some(b) = r.bounds() {
            let q = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()].map(|p| map.fwd(p));
            let wb = Rect::from_points(&q);
            let a = |p: Pos2| r.alpha_at_world(p) as f32;
            for y in wb.min.y.floor() as i32..wb.max.y.ceil() as i32 {
                for x in wb.min.x.floor() as i32..wb.max.x.ceil() as i32 {
                    let m = map.inv(pos2(x as f32 + 0.5, y as f32 + 0.5)) - vec2(0.5, 0.5);
                    let (fx, fy) = (m.x - m.x.floor(), m.y - m.y.floor());
                    let o = pos2(m.x.floor() + 0.5, m.y.floor() + 0.5);
                    let v = (a(o) * (1.0 - fx) + a(o + vec2(1.0, 0.0)) * fx) * (1.0 - fy) + (a(o + vec2(0.0, 1.0)) * (1.0 - fx) + a(o + vec2(1.0, 1.0)) * fx) * fy;
                    if v >= 1.0 {
                        out.set_alpha_world(x, y, v.round() as u8);
                    }
                }
            }
        }
        if let Some(l) = self.doc.get_mut(id) {
            l.kind = LayerKind::Raster(out);
            l.mask_res = 1.0;
        }
    }

    /// The mask the scissors draw into, when they are the tool in use.
    fn scissor_target(&self) -> Option<LayerId> {
        self.wand.filter(|_| self.wand_mode == 2 && self.selecting()).filter(|mid| self.doc.get(*mid).is_some_and(|l| l.mask_of.is_some()))
    }

    /// The mask's selection modes (자동 선택, 그랩컷, 가위) act only with the
    /// select tools (Ash, 10-23): brush and eraser keep painting the mask.
    fn selecting(&self) -> bool {
        matches!(self.tool, Tool::Select | Tool::Lasso)
    }

    /// Closes the outline and fills it into the mask (anti-aliased).
    /// `refine`: the outline seeds GrabCut (deep inside: certain, a band along the
    /// outline: undecided, outside: background) instead of filling as drawn.
    fn scissor_close(&mut self, refine: bool) {
        let Some(mut sc) = self.scissor.take() else { return };
        // the last stretch back to the first anchor along the edge
        let first = sc.pts[0];
        if let Some(wr) = sc.wire.as_mut() {
            wr.grow(Some(first), std::time::Duration::from_millis(300));
        }
        let mut back = Self::scissor_path(&sc, first);
        if back.len() > 1 {
            // it ends on the first anchor, moved onto the edge with its blur:
            // that becomes the first point (it was placed unrefined)
            if let Some((q, s)) = back.pop() {
                sc.pts[0] = q;
                sc.sig[0] = s;
            }
            for (q, s) in back.into_iter().skip(1) {
                sc.pts.push(q);
                sc.sig.push(s);
            }
        } else if sc.sig.len() > 1 {
            sc.sig[0] = sc.sig[1];
        }
        if sc.pts.len() < 3 {
            return;
        }
        if refine {
            let Some(pm) = poly_pixmap(&sc.pts) else { return };
            let b = Rect::from_points(&sc.pts);
            let b = Rect::from_min_max(b.min.floor() - vec2(1.0, 1.0), b.max.ceil() + vec2(1.0, 1.0));
            self.scissor_refine(sc.mid, b, &pm.0, sc.on);
            return;
        }
        // filled in the mask's own pixels (a photo mask's are its photo's)
        let pts: Vec<Pos2> = match self.mask_space(sc.mid) {
            Some(map) => sc.pts.iter().map(|p| map.inv(*p)).collect(),
            None => sc.pts.clone(),
        };
        let soft = self.s.scissor_soft;
        let Some(LayerKind::Raster(m)) = self.doc.get_mut(sc.mid).map(|l| &mut l.kind) else { return };
        // the edge map's cells are the mask's pixels (a photo's, or the canvas's)
        let n = if soft { soft_mask_poly(m, &pts, &sc.sig, sc.on) } else { fill_mask_poly(m, &pts, sc.on) };
        self.status = crate::i18n::trf("가위 · {} px {}", &[format!("{n}"), format!("{}", if sc.on { crate::i18n::tr("가림") } else { crate::i18n::tr("되살림") })]);
        self.mark();
    }

    fn scissor_refine(&mut self, mid: LayerId, b: Rect, poly: &tiny_skia::Pixmap, on: bool) {
        use crate::segment::{BG, FG, PR_BG, PR_FG};
        if self.doc.get(mid).and_then(|l| l.mask_of).is_none() {
            return;
        }
        const PAD: i32 = 16;
        const BAND: usize = 6;
        let reg = Rect::from_min_max(b.min - vec2(PAD as f32, PAD as f32), b.max + vec2(PAD as f32, PAD as f32));
        let Some(d) = self.trace_doc(mid) else { return Default::default() };
        let Ok(pm) = render_region(&mut self.cgpu, &d, reg, 1.0, Some(Color32::BLACK)) else { return };
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
        self.status = crate::i18n::trf("가위+그랩컷 · {} px {}", &[format!("{n}"), format!("{}", if on { crate::i18n::tr("가림") } else { crate::i18n::tr("되살림") })]);
        self.mark();
    }

    /// GrabCut over a dragged box: what is inside the box and stands apart from its
    /// surroundings (colour models + edges) is hidden (or shown again). Coverage
    /// already painted in the box counts as certain.
    fn grabcut_fill(&mut self, mid: LayerId, rect: Rect, on: bool) {
        self.adopt_photo(mid, rect.center());
        use crate::segment::{BG, FG, PR_FG};
        if self.doc.get(mid).and_then(|l| l.mask_of).is_none() {
            return;
        }
        let rect = Rect::from_min_max(rect.min.floor(), rect.max.ceil());
        if rect.width() < 4.0 || rect.height() < 4.0 {
            return;
        }
        let margin = (rect.size().max_elem() * 0.1).max(12.0).round();
        let reg = rect.expand(margin);
        if reg.width() > 6000.0 || reg.height() > 6000.0 {
            self.status = crate::i18n::tr("영역이 너무 큼").into();
            return;
        }
        let Some(d) = self.trace_doc(mid) else { return Default::default() };
        let Ok(pm) = render_region(&mut self.cgpu, &d, reg, 1.0, Some(Color32::BLACK)) else { return };
        let (pw, ph) = (pm.width() as usize, pm.height() as usize);
        let px = pm.data();
        // the cut runs on a copy of at most ~90k pixels
        let step = (((pw * ph) as f64 / 90_000.0).sqrt().ceil() as usize).max(1);
        let (gw, gh) = (pw.div_ceil(step), ph.div_ceil(step));
        let hint = match self.doc.get(mid).map(|l| &l.kind) {
            Some(LayerKind::Raster(m)) => Some(m.clone()),
            _ => None,
        };
        let hint_space = self.mask_space(mid);
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
                    let painted = on && hint.as_ref().is_some_and(|m| m.alpha_at_world(hint_space.map_or(wp, |s| s.inv(wp))) > 127);
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
        self.status = crate::i18n::trf("그랩컷 · {} px {}", &[format!("{n}"), format!("{}", if on { crate::i18n::tr("가림") } else { crate::i18n::tr("되살림") })]);
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
        let name = crate::i18n::trf("{} 마스크", &[format!("{}", self.doc.layers[i].display_name())]);
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
            mask_res: 1.0,
        };
        // just under its layer, so the list shows it beneath
        self.doc.layers.insert(i, layer);
        self.select_only(Some(id));
        if !matches!(self.tool, Tool::Brush | Tool::Eraser | Tool::Pen | Tool::Rect) {
            self.set_tool(Tool::Brush);
        }
        self.status = crate::i18n::tr("마스크 · 칠한 곳 가림 · 지우개: 되살리기 · 사각형: 가리기 (Alt: 되살리기)").into();
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
            self.status = crate::i18n::trf("키 · {}", &[format!("{}", crate::video::timecode(kt))]);
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
            self.status = crate::i18n::trf("{} 키 · {}", &[format!("{}", c.name), format!("{}", crate::video::timecode(kt))]);
            self.mark();
        }
    }

    fn delete_selected_keys(&mut self) {
        let sel = std::mem::take(&mut self.tl_sel);
        let n = sel.len();
        for (row, t) in sel {
            let c = row.ch();
            if let Some(a) = self.row_anim_mut(row) {
                a.keys.retain(|k| k.ch & c == 0 || (k.t - t).abs() >= 1e-6);
            }
        }
        self.tl_key = None;
        if n > 0 {
            self.set_time(self.doc.timeline.time);
            self.mark();
        }
        self.status = crate::i18n::trf("키 {}개 지움", &[format!("{n}")]);
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
        self.status = crate::i18n::trf("키 {}개 지움", &[format!("{n}")]);
    }

    fn row_anim(&self, row: TlRow) -> Option<&crate::anim::Anim> {
        match row {
            TlRow::Layer(id, _) => self.doc.get(id).and_then(|l| l.anim.as_ref()),
            TlRow::Cam(id, _) => self.doc.cameras.iter().find(|c| c.id == id).map(|c| &c.anim),
        }
    }

    fn row_anim_mut(&mut self, row: TlRow) -> Option<&mut crate::anim::Anim> {
        match row {
            TlRow::Layer(id, _) => self.doc.get_mut(id).map(|l| l.anim.get_or_insert_with(Default::default)),
            TlRow::Cam(id, _) => self.doc.cameras.iter_mut().find(|c| c.id == id).map(|c| &mut c.anim),
        }
    }

    fn cameras_panel(&mut self, ui: &mut egui::Ui) {
        if !Self::fold_section(ui, crate::i18n::tr("카메라"), &mut self.s.folded[2]) {
            return;
        }
        ui.small(crate::i18n::tr("영역 → 카메라 · 손잡이: 크기 · I: 키 · 구간 밖: 꺼짐"));
        let mut pick = None;
        let mut add = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
            for c in &self.doc.cameras {
                if crate::kit::chip(ui, self.cam_sel == Some(c.id), egui::RichText::new(&c.name).color(CAM_COLOR)).clicked() {
                    pick = Some(c.id);
                }
            }
            add = ui.button(crate::i18n::tr("+ 카메라")).glass_tip(crate::i18n::tr("내보낼 영역(없으면 화면)으로 새 카메라")).clicked();
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
            self.doc.cameras.push(crate::anim::CanvasCam { id, name: crate::i18n::trf("카메라 {}", &[format!("{n}")]), rect: Rect::from_min_max(rect.min.round(), rect.max.round()), anim: Default::default(), rot: 0.0, follow: None });
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
                ui.label(crate::i18n::tr("이름"));
                ch |= ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(ui.available_width())).changed();
            });
            ui.label(crate::i18n::trf("키 {}개", &[format!("{}", c.anim.keys.len())]));
            ch |= rect_fields(ui, &mut c.rect, "cam", cm(ch::POS | ch::SIZE), &mut ck[0]);
            let deg0 = wrap_deg(c.rot.to_degrees());
            let mut deg = deg0;
            if ui.add(crate::dial::Dial::new(&mut deg, -180.0..=180.0).step(1.0).text(crate::i18n::tr("회전 °")).default(0.0).key(cm(ch::ROT), &mut ck[1])).changed() {
                c.rot += (deg - deg0).to_radians();
                ch = true;
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
                from_frame = ui.add_enabled(self.doc.export_frame.is_some(), egui::Button::new(crate::i18n::tr("영역 → 카메라"))).clicked();
                to_frame = ui.button(crate::i18n::tr("카메라 → 영역")).clicked();
                del = ui.button(crate::i18n::tr("카메라 삭제")).clicked();
            });
        }
        // centre on a layer, or follow one (constraint)
        let targets: Vec<(LayerId, String)> = self.doc.layers.iter().rev().map(|l| (l.id, l.display_name())).collect();
        let mut snap_to = None;
        let follow = self.doc.cameras[i].follow;
        let mut new_follow = follow;
        let picked = self.sel;
        crate::kit::row(ui, crate::i18n::tr("중심"), |ui| {
            // the button at the right edge, the drop-down fills up to it
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(picked.is_some(), egui::Button::new(crate::i18n::tr("선택"))).glass_tip(crate::i18n::tr("선택한 객체 중심으로")).clicked() {
                    snap_to = picked;
                }
                let items: Vec<(LayerId, String, bool)> = targets.iter().map(|(id, n)| (*id, n.clone(), true)).collect();
                if let (Some(id), _) = crate::kit::dropdown(ui, ("cam_snap", self.doc.cameras[i].id), crate::i18n::tr("맞추기…"), crate::kit::FILL, &items, None) {
                    snap_to = Some(id);
                }
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_XS;
            let mut hit = false;
            crate::dial::label_cell(ui, crate::i18n::tr("따라가기"), cm(ch::FOLLOW), &mut hit);
            ck[2] |= hit;
            let shown = follow.and_then(|f| targets.iter().find(|t| t.0 == f.layer)).map_or(crate::i18n::tr("없음").to_string(), |t| t.1.clone());
            let items: Vec<(Option<LayerId>, String, bool)> = std::iter::once((None, crate::i18n::tr("없음").to_string(), true)).chain(targets.iter().map(|(id, n)| (Some(*id), n.clone(), true))).collect();
            let cur = follow.map(|f| f.layer);
            if let (Some(pick), _) = crate::kit::dropdown(ui, ("cam_follow", self.doc.cameras[i].id), shown, crate::kit::FILL, &items, Some(&cur)) {
                new_follow = pick.map(|layer| crate::anim::Follow { layer, pos: true, rot: false, offset: [0.0, 0.0], rot_offset: 0.0 });
            }
        });
        let bind = picked.filter(|p| follow.is_none_or(|f| f.layer != *p));
        if ui.add_enabled(bind.is_some(), egui::Button::new(crate::i18n::tr("선택한 객체 따라가기"))).glass_tip(crate::i18n::tr("위치 따라가기로 묶기")).clicked() {
            if let Some(layer) = bind {
                new_follow = Some(crate::anim::Follow { layer, pos: true, rot: false, offset: [0.0, 0.0], rot_offset: 0.0 });
            }
        }
        if let Some(mut f) = new_follow {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
                let mut c2 = false;
                c2 |= crate::kit::switch(ui, &mut f.pos, crate::i18n::tr("위치")).changed();
                c2 |= crate::kit::switch(ui, &mut f.rot, crate::i18n::tr("회전")).changed();
                if ui.button(crate::i18n::tr("지금 배치 유지")).glass_tip(crate::i18n::tr("지금 간격·각도 차이 유지")).clicked() {
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
                theme::glass(ui, egui::Margin::same(10), theme::radius_card(), |ui| {
                    ui.set_width(width);
                    self.timeline_body(ui, width);
                });
            });
        self.tl_h = resp.response.rect.height();
    }

    fn timeline_body(&mut self, ui: &mut egui::Ui, width: f32) {
        // one key per value, whatever made them (old code paths, debug flags):
        // a key row then never moves another value's key
        for a in self.doc.layers.iter_mut().filter_map(|l| l.anim.as_mut()).chain(self.doc.cameras.iter_mut().map(|c| &mut c.anim)) {
            a.split_channels();
        }
        let playing = self.tl_play.is_some();
        let mut act_play = false;
        let mut new_time: Option<f64> = None;
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_SM;
            if ui.button(if playing { crate::i18n::tr("멈춤") } else { crate::i18n::tr("재생") }).glass_tip("Alt+A").clicked() {
                act_play = true;
            }
            if ui.button(crate::i18n::tr("처음")).glass_tip("Shift+,").clicked() {
                new_time = Some(0.0);
            }
            let tl = &self.doc.timeline;
            ui.label(egui::RichText::new(format!("{} / {}", crate::video::timecode(tl.time), crate::video::timecode(tl.duration))).monospace());
            ui.label(egui::RichText::new(format!("{}f", (tl.time * tl.fps).round() as i64)).monospace().color(theme::LABEL()));
            ui.separator();
            let tl = &mut self.doc.timeline;
            ui.label("fps");
            changed |= ui.add(crate::calc::drag(&mut tl.fps).range(1.0..=240.0).speed(0.1).max_decimals(3)).changed();
            ui.label(crate::i18n::tr("길이"));
            changed |= ui.add(crate::calc::drag(&mut tl.duration).range(0.1..=36000.0).speed(0.1).suffix(" s").max_decimals(2)).changed();
            changed |= crate::kit::switch(ui, &mut tl.autokey, crate::i18n::tr("자동 키")).glass_tip(crate::i18n::tr("선택 레이어 변경 시 자동으로 키")).changed();
            let rec_label = egui::RichText::new(crate::i18n::tr("● 녹화")).color(if self.rec_on { Color32::from_rgb(255, 90, 90) } else { theme::LABEL() });
            if crate::kit::chip(ui, self.rec_on, rec_label)
                .glass_tip(crate::i18n::tr("재생 헤드부터 그리는 과정 기록 · 끝나면 재생 헤드가 획 끝으로"))
                .clicked()
            {
                self.rec_on = !self.rec_on;
            }
            ui.separator();
            if ui.button(crate::i18n::tr("키 넣기")).glass_tip("I").clicked() {
                self.insert_keys();
            }
            if ui.button(crate::i18n::tr("키 지우기")).glass_tip(crate::i18n::tr("선택한 키 (Delete) · 없으면 재생 헤드의 키 (Alt+I)")).clicked() {
                if self.tl_sel.is_empty() {
                    self.delete_keys_here();
                } else {
                    self.delete_selected_keys();
                }
            }
            ui.separator();
            if ui.button(crate::i18n::tr("영상 내보내기")).glass_tip("Ctrl+Shift+E").clicked() {
                self.open_video_export();
            }
        });
        // selected key: easing
        if let Some((row, kt)) = self.tl_key {
            let ease = self.row_anim(row).and_then(|a| a.keys.iter().find(|k| k.ch & row.ch() != 0 && (k.t - kt).abs() < 1e-6)).map(|k| k.ease);
            match ease {
                Some(e) => {
                    let mut pick = None;
                    ui.horizontal(|ui| {
                        let n = self.tl_sel.len().max(1);
                        let what = if n > 1 { crate::i18n::trf("선택한 키 {}개", &[format!("{n}")]) } else { crate::i18n::trf("선택한 키 {}", &[format!("{}", crate::video::timecode(kt))]) };
                        ui.label(egui::RichText::new(what).color(theme::LABEL()));
                        ui.label(crate::i18n::tr("다음 키까지"));
                        for o in crate::anim::Ease::ALL {
                            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                            key_icon(ui.painter(), ir.center(), o, if e == o { ACCENT } else { INK() }, Stroke::new(1.0, Color32::from_black_alpha(170)));
                            if crate::kit::chip(ui, e == o, o.label()).clicked() {
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
                            let c = rw.ch();
                            if let Some(k) = self.row_anim_mut(rw).and_then(|a| a.keys.iter_mut().find(|k| k.ch & c != 0 && (k.t - t).abs() < 1e-6)) {
                                k.ease = o;
                                changed = true;
                            }
                        }
                    }
                }
                None => self.tl_key = None,
            }
        }
        // rows: cameras, then layers with keys or a span, then the selection;
        // each a box of its head row and one row per value it has keys of
        let sel = self.selected_ids();
        // the values it has keys of, among those it has at all (older files
        // keyed everything: keys of values a layer lacks stay, unshown)
        let key_rows = |anim: Option<&crate::anim::Anim>, has: u16| -> Vec<u16> {
            let mut m = 0u16;
            for k in anim.map_or(&[][..], |a| &a.keys[..]) {
                m |= k.ch;
            }
            (0..crate::anim::ch::COUNT).map(|b| 1u16 << b).filter(|c| m & has & c != 0).collect()
        };
        let mut rows: Vec<(TlRow, String, bool)> = Vec::new();
        for c in &self.doc.cameras {
            rows.push((TlRow::Cam(c.id, 0), c.name.clone(), true));
            for b in key_rows(Some(&c.anim), crate::anim::ch::CAMERA) {
                rows.push((TlRow::Cam(c.id, b), ch_label(None, b), true));
            }
        }
        for l in self.doc.layers.iter().rev() {
            let recorded = match &l.kind {
                LayerKind::Vector(v) => v.strokes.iter().any(|s| s.rec.is_some()),
                LayerKind::Raster(r) => !r.recs.is_empty(),
                _ => false,
            };
            // sound layers always have a row (their clip and volume keys)
            let sound = matches!(l.kind, LayerKind::Audio(_));
            if sound || recorded || l.anim.as_ref().is_some_and(|a| !a.keys.is_empty() || a.span.is_some()) || sel.contains(&l.id) {
                rows.push((TlRow::Layer(l.id, 0), l.display_name(), false));
                for b in key_rows(l.anim.as_ref(), crate::anim::channels_of(l)) {
                    rows.push((TlRow::Layer(l.id, b), ch_label(Some(l), b), false));
                }
            }
        }
        let row_h = 22.0;
        let key_h = 18.0;
        let ruler_h = 20.0;
        let label_w = 140.0;
        let h_of = |r: &TlRow| if r.ch() == 0 { row_h } else { key_h };
        let total: f32 = rows.iter().map(|r| h_of(&r.0)).sum();
        // at most eight head rows tall; more scrolls (the wheel over the rows)
        let view_h = total.clamp(row_h, row_h * 8.0);
        let h = ruler_h + view_h + 4.0;
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
        // cached frames (playback cache): a line along the ruler's foot
        let cfps = self.pcache.fps().max(1.0);
        for (a, b) in self.pcache.ranges() {
            // frames past a shortened end are kept (for when it grows back)
            if a as f64 / cfps >= dur {
                continue;
            }
            let (x0, x1) = (x_of(a as f64 / cfps), x_of(((b + 1) as f64 / cfps).min(dur)));
            p.rect_filled(Rect::from_min_max(pos2(x0, rect.top() + ruler_h - 2.0), pos2(x1.max(x0 + 1.0), rect.top() + ruler_h)), 0.0, ACCENT.gamma_multiply(0.8));
        }
        let mut t = 0.0;
        while t <= dur + 1e-9 {
            let x = x_of(t);
            p.line_segment([pos2(x, rect.top() + 12.0), pos2(x, rect.top() + ruler_h)], Stroke::new(1.0, INK_3()));
            let label = if step < 1.0 { format!("{t:.1}") } else { format!("{}", t.round() as i64) };
            p.text(pos2(x + 3.0, rect.top() + 1.0), Align2::LEFT_TOP, label, FontId::proportional(10.0), theme::LABEL());
            t += step;
        }
        // rows, scrolled by the wheel when they do not fit
        let rows_r = Rect::from_min_max(pos2(rect.left(), rect.top() + ruler_h), pos2(rect.right(), rect.top() + ruler_h + view_h));
        if total > view_h && resp.hovered() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                self.tl_scroll -= dy;
                ui.ctx().input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            }
        }
        self.tl_scroll = self.tl_scroll.clamp(0.0, (total - view_h).max(0.0));
        let p = p.with_clip_rect(rows_r);
        let mut hit_rows: Vec<(TlRow, Rect)> = Vec::new();
        // each layer's or camera's box: its head row and its key rows
        let mut y = rows_r.top() - self.tl_scroll;
        let mut boxes: Vec<(TlRow, Rect)> = Vec::new();
        for (row, _, _) in &rows {
            let rr = Rect::from_min_max(pos2(rect.left(), y), pos2(rect.right(), y + h_of(row)));
            match boxes.last_mut() {
                Some((b, r)) if *b == row.head() => *r = r.union(rr),
                _ => boxes.push((row.head(), rr)),
            }
            y += h_of(row);
        }
        for (b, r) in &boxes {
            let selected = match b {
                TlRow::Layer(id, _) => sel.contains(id),
                TlRow::Cam(id, _) => self.cam_sel == Some(*id),
            };
            let r = r.shrink2(vec2(0.0, 1.0));
            if selected {
                p.rect_filled(r, 6.0, FILL_1());
            }
            p.rect_stroke(r, 6.0, Stroke::new(1.0, FILL_2()), StrokeKind::Inside);
        }
        let mut y = rows_r.top() - self.tl_scroll;
        for (row, name, cam) in rows.iter() {
            let y0 = y;
            y += h_of(row);
            let rr = Rect::from_min_max(pos2(rect.left(), y0), pos2(rect.right(), y0 + h_of(row)));
            if rr.bottom() < rows_r.top() || rr.top() > rows_r.bottom() {
                continue;
            }
            // a press is matched only where the row shows
            hit_rows.push((*row, rr.intersect(rows_r)));
            if row.ch() != 0 {
                // a key row: its value's name, a faint line, its keys
                let col = if *cam { CAM_COLOR } else { theme::LABEL() };
                let g = p.layout_job(elided(name.to_string(), FontId::proportional(theme::TEXT_SMALL - 0.5), col.gamma_multiply(0.9), label_w - 26.0));
                p.galley(pos2(rr.left() + 18.0, rr.center().y - g.size().y * 0.5), g, col);
                let cy = rr.center().y;
                p.line_segment([pos2(tr.left(), cy), pos2(tr.right(), cy)], Stroke::new(1.0, col.gamma_multiply(0.18)));
                if let Some(a) = self.row_anim(*row) {
                    let c = row.ch();
                    let ks: Vec<&crate::anim::Key> = a.keys.iter().filter(|k| k.ch & c != 0).collect();
                    // the curve between keys shows how it eases: straight, S-shaped or a step
                    for w2 in ks.windows(2) {
                        let (x0, x1) = (x_of(w2[0].t), x_of(w2[1].t));
                        if x1 - x0 < 6.0 {
                            continue;
                        }
                        let (y0, y1) = (cy + 3.5, cy - 3.5);
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
                    for k in ks {
                        let kc = pos2(x_of(k.t), cy);
                        let on = self.tl_sel.iter().any(|(r, t)| r == row && (t - k.t).abs() < 1e-6) || self.tl_key.is_some_and(|(r, t)| r == *row && (t - k.t).abs() < 1e-6);
                        let fill = if on { ACCENT } else { INK() };
                        key_icon(&p, kc, k.ease, fill, Stroke::new(1.0, Color32::from_black_alpha(170)));
                    }
                }
                continue;
            }
            let sound = match row {
                TlRow::Layer(id, _) => self.doc.get(*id).and_then(|l| match &l.kind {
                    LayerKind::Audio(a) => Some((a.clone(), l.anim.as_ref().and_then(|an| an.span).unwrap_or([0.0, 0.0]), l.anim.as_ref().map_or(0.0, |an| an.video_start))),
                    _ => None,
                }),
                _ => None,
            };
            let selected = match row {
                TlRow::Layer(id, _) => sel.contains(id),
                TlRow::Cam(id, _) => self.cam_sel == Some(*id),
            };
            let video = match row {
                TlRow::Layer(id, _) => self.video_clip(*id),
                _ => None,
            };
            let col = if *cam {
                CAM_COLOR
            } else if sound.is_some() {
                AUDIO_COLOR
            } else if video.is_some() {
                VIDEO_COLOR
            } else {
                theme::LABEL()
            };
            let g = p.layout_job(elided(name.to_string(), FontId::proportional(theme::TEXT_SMALL + 0.5), col, label_w - 14.0));
            p.galley(pos2(rr.left() + 6.0, rr.center().y - g.size().y * 0.5), g, col);
            let cy = rr.center().y;
            if let Some((a, [s0, e0], src0)) = &sound {
                let bar = Rect::from_min_max(pos2(x_of(*s0), rr.top() + 3.0), pos2(x_of(*e0).max(x_of(*s0) + 2.0), rr.bottom() - 3.0));
                p.rect_filled(bar, 3.0, AUDIO_COLOR.gamma_multiply(if a.mute { 0.08 } else { 0.22 }));
                // waveform
                if let Some(Some(d)) = self.audio_cache.get(&a.path) {
                    let x0 = bar.left().max(tr.left());
                    let x1 = bar.right().min(tr.right());
                    let mut x = x0;
                    while x < x1 {
                        let src = src0 + t_of(x) - s0;
                        let i = (src / crate::audio::PEAK_SEC) as usize;
                        let pk = d.peaks.get(i).copied().unwrap_or(0.0) * a.volume.min(1.5);
                        let hh = (bar.height() * 0.5 - 1.0) * pk.min(1.0);
                        p.line_segment([pos2(x, cy - hh), pos2(x, cy + hh)], Stroke::new(1.0, AUDIO_COLOR.gamma_multiply(if a.mute { 0.3 } else { 0.9 })));
                        x += 2.0;
                    }
                } else if !a.path.is_empty() {
                    p.text(bar.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, crate::i18n::tr("불러오는 중…"), FontId::proportional(10.0), theme::LABEL());
                }
                if selected {
                    p.rect_stroke(bar, 3.0, Stroke::new(1.2, AUDIO_COLOR), StrokeKind::Inside);
                }
            }
            if let Some(([s0, e0], [m0, m1])) = video {
                // the clip; where it holds a still (before the first frame, after
                // the last) it is paler and striped
                let bar = Rect::from_min_max(pos2(x_of(s0), rr.top() + 3.0), pos2(x_of(e0).max(x_of(s0) + 2.0), rr.bottom() - 3.0));
                p.rect_filled(bar, 3.0, VIDEO_COLOR.gamma_multiply(0.12));
                let live = Rect::from_min_max(pos2(x_of(m0.max(s0)), bar.top()), pos2(x_of(m1.min(e0)), bar.bottom()));
                if live.width() > 0.0 {
                    // a graph of the video frame cache: dark where frames are
                    // still to be decoded, the clip's colour where they are held
                    p.rect_filled(live, 3.0, VIDEO_COLOR.gamma_multiply(0.09));
                    if let TlRow::Layer(id, _) = row {
                        if let Some(v) = self.video_of(*id) {
                            let (held, fps) = crate::vcache::ranges(&v.path);
                            let speed = v.speed.max(1e-3);
                            let pc = p.with_clip_rect(live.intersect(rect));
                            for (a, b) in held {
                                let (ta, tb) = (m0 + a as f64 / fps / speed, m0 + (b + 1) as f64 / fps / speed);
                                let seg = Rect::from_min_max(pos2(x_of(ta).max(live.left()), live.top()), pos2(x_of(tb).min(live.right()), live.bottom()));
                                if seg.width() > 0.0 {
                                    pc.rect_filled(seg, 3.0, VIDEO_COLOR.gamma_multiply(0.3));
                                }
                            }
                        }
                    }
                }
                // (after the end a looping clip starts over: no still there)
                let looping = matches!(row, TlRow::Layer(id, _) if self.video_of(*id).is_some_and(|v| v.looping));
                for (a, b) in [(s0, m0.min(e0)), (m1.max(s0), e0)] {
                    if b <= a || (looping && a >= m1.max(s0)) {
                        continue;
                    }
                    let (xa, xb) = (x_of(a), x_of(b));
                    let pc = p.with_clip_rect(Rect::from_min_max(pos2(xa, bar.top()), pos2(xb, bar.bottom())).intersect(rect));
                    let mut x = xa - bar.height();
                    while x < xb {
                        pc.line_segment([pos2(x, bar.bottom()), pos2(x + bar.height(), bar.top())], Stroke::new(1.0, VIDEO_COLOR.gamma_multiply(0.45)));
                        x += 6.0;
                    }
                }
                if selected {
                    p.rect_stroke(bar, 3.0, Stroke::new(1.2, VIDEO_COLOR), StrokeKind::Inside);
                }
            }
            if let TlRow::Layer(lid, _) = row {
                // recorded strokes: thin red marks along the bottom of the row
                for (a, b) in self.rec_spans(*lid) {
                    let x0 = x_of(a);
                    let x1 = x_of(b).max(x0 + 2.0);
                    p.rect_filled(Rect::from_min_max(pos2(x0, rr.bottom() - 5.0), pos2(x1, rr.bottom() - 2.0)), 1.0, Color32::from_rgb(255, 90, 90).gamma_multiply(0.8));
                }
            }
            let anim = self.row_anim(*row);
            // a sound's or video's span is its clip bar (drawn above)
            let clip = sound.is_some() || video.is_some();
            match anim.and_then(|a| a.span).filter(|_| !clip) {
                Some([a, b]) => {
                    let bar = Rect::from_min_max(pos2(x_of(a), cy - 4.0), pos2(x_of(b), cy + 4.0));
                    p.rect_filled(bar, 3.0, col.gamma_multiply(0.35));
                    for x in [bar.left(), bar.right()] {
                        p.rect_filled(Rect::from_center_size(pos2(x, cy), vec2(4.0, 12.0)), 1.5, col);
                    }
                }
                None if !clip => {
                    p.line_segment([pos2(tr.left(), cy), pos2(tr.right(), cy)], Stroke::new(1.0, col.gamma_multiply(0.25)));
                }
                None => {}
            }
        }
        if total > view_h {
            // where the view is in the rows: a thin bar at the right
            let k = view_h / total;
            let bar = Rect::from_min_size(pos2(rect.right() - 3.0, rows_r.top() + self.tl_scroll * k), vec2(2.0, view_h * k));
            p.rect_filled(bar, 1.0, INK_3());
        }
        // playhead
        let px = x_of(tl.time);
        p.line_segment([pos2(px, rect.top()), pos2(px, rect.bottom())], Stroke::new(1.5, ACCENT));
        p.add(Shape::convex_polygon(vec![pos2(px - 5.0, rect.top()), pos2(px + 5.0, rect.top()), pos2(px, rect.top() + 7.0)], ACCENT, Stroke::NONE));

        // interaction
        let pointer = resp.interact_pointer_pos();
        if resp.drag_started() || resp.clicked() {
            // what was pressed is judged where the press began: a drag starts only
            // after the pointer has moved a few pixels, enough to miss a key or a
            // clip's 5 px trim handle
            let origin = ui.input(|i| i.pointer.press_origin());
            if let Some(pp) = origin.or(pointer) {
                let mut drag = TlDrag::Scrub;
                let shift = ui.input(|i| i.modifiers.shift);
                // Shift on the playhead drags it (snapping to keys); elsewhere it boxes keys
                let on_head = (x_of(self.doc.timeline.time) - pp.x).abs() <= 5.0;
                if shift && on_head {
                    drag = TlDrag::Scrub;
                } else if shift && pp.y > rect.top() + ruler_h && pp.x >= tr.left() {
                    drag = TlDrag::Marquee { start: pp };
                } else if pp.y > rect.top() + ruler_h {
                    if let Some((row, _)) = hit_rows.iter().find(|(_, rr)| rr.contains(pp)).copied() {
                        let anim = self.row_anim(row);
                        let key = anim.and_then(|a| a.keys.iter().find(|k| k.ch & row.ch() != 0 && (x_of(k.t) - pp.x).abs() <= 6.0)).map(|k| k.t);
                        let span = anim.and_then(|a| a.span);
                        let sound_id = match row {
                            TlRow::Layer(id, _) if self.doc.get(id).is_some_and(|l| matches!(l.kind, LayerKind::Audio(_))) => Some(id),
                            TlRow::Layer(id, _) if self.video_clip(id).is_some() => Some(id),
                            _ => None,
                        };
                        // a video clip without a span yet: its span is the video's length
                        let span = match (sound_id, span) {
                            (Some(id), None) => self.video_clip(id).map(|(s, _)| s),
                            _ => span,
                        };
                        let alt = ui.input(|i| i.modifiers.alt);
                        let t_at = t_of(pp.x);
                        if let Some(kt) = key {
                            let picked = self.tl_sel.iter().any(|(r, t)| *r == row && (t - kt).abs() < 1e-6);
                            self.tl_key = Some((row, kt));
                            if picked && self.tl_sel.len() > 1 {
                                drag = TlDrag::Keys { last: kt };
                            } else {
                                self.tl_sel = vec![(row, kt)];
                                drag = TlDrag::Key { row, from: kt };
                            }
                        } else if let (Some(id), Some([a, b])) = (sound_id, span.filter(|_| pp.x >= tr.left())) {
                            // a sound's or video's clip: trim either end, or move it
                            if self.video_clip(id).is_some() {
                                let l = self.doc.get_mut(id).unwrap();
                                let an = l.anim.get_or_insert_with(Default::default);
                                if an.span.is_none() {
                                    an.span = Some([a, b]);
                                }
                            }
                            if alt && self.video_clip(id).is_some() && t_at > a && t_at < b {
                                let vs = self.doc.get(id).and_then(|l| l.anim.as_ref()).map_or(0.0, |an| an.video_start);
                                drag = TlDrag::VideoSlip { id, from: vs, at: t_at };
                            } else if (x_of(a) - pp.x).abs() <= 5.0 {
                                drag = TlDrag::AudioIn(id);
                            } else if (x_of(b) - pp.x).abs() <= 5.0 {
                                drag = TlDrag::AudioOut(id);
                            } else if t_at >= a && t_at <= b {
                                drag = TlDrag::AudioMove { id, grab: t_at - a };
                            }
                            if drag != TlDrag::Scrub {
                                self.select_only(Some(id));
                            }
                        } else if span.is_some_and(|[a, _]| (x_of(a) - pp.x).abs() <= 5.0) {
                            drag = TlDrag::SpanIn(row);
                        } else if span.is_some_and(|[_, b]| (x_of(b) - pp.x).abs() <= 5.0) {
                            drag = TlDrag::SpanOut(row);
                        } else if pp.x < tr.left() {
                            // the label selects the layer or camera
                            match row {
                                TlRow::Layer(id, _) => self.select_only(Some(id)),
                                TlRow::Cam(id, _) => {
                                    self.cam_sel = Some(id);
                                    self.tool = Tool::Frame;
                                }
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
                    let sel = self.tl_sel.clone();
                    // the keys move as one: the earliest stops at 0 and the rest
                    // keep their distances from it
                    let first = sel.iter().map(|x| x.1).fold(f64::MAX, f64::min);
                    let dlt = (t - last).max(-first);
                    if dlt.abs() > 1e-9 && !sel.is_empty() {
                        // every key is found before any moves (one moved onto
                        // another's time would be taken for it), and a key shown
                        // on two rows moves once
                        let mut hits: Vec<(TlRow, usize, f64)> = Vec::new();
                        for (row, kt) in &sel {
                            let c = row.ch();
                            if let Some(i) = self.row_anim(*row).and_then(|a| a.keys.iter().position(|k| k.ch & c != 0 && (k.t - kt).abs() < 1e-6)) {
                                hits.push((*row, i, *kt));
                            }
                        }
                        let mut done: Vec<(TlRow, usize)> = Vec::new();
                        let mut moved = Vec::new();
                        for (row, i, kt) in &hits {
                            if !done.contains(&(row.head(), *i)) {
                                done.push((row.head(), *i));
                                if let Some(k) = self.row_anim_mut(*row).and_then(|a| a.keys.get_mut(*i)) {
                                    k.t = kt + dlt;
                                }
                            }
                            moved.push((*row, kt + dlt));
                        }
                        for (head, _) in &done {
                            if let Some(a) = self.row_anim_mut(*head) {
                                a.sort();
                            }
                        }
                        self.tl_sel = moved;
                        self.tl_key = self.tl_sel.first().copied();
                        self.tl_drag = Some(TlDrag::Keys { last: last + dlt });
                        changed = true;
                    }
                }
                TlDrag::Scrub => {
                    let mut t = t;
                    // Shift: the nearest key on the shown rows within reach. The reach
                    // is about 0.1 s in whole frames of the project rate (2–6 frames:
                    // 24 fps → 2, 30 → 3, 60 → 6), and at least 6 px on screen.
                    if ui.input(|i| i.modifiers.shift) {
                        let fps = tl.fps.max(1.0);
                        let reach = ((fps * 0.1).round().clamp(2.0, 6.0) / fps).max(6.0 / px_per_s.max(1e-9));
                        let raw = t_of(pp.x);
                        let near = hit_rows
                            .iter()
                            .filter_map(|(row, _)| self.row_anim(*row).map(|a| (row.ch(), a)))
                            .flat_map(|(c, a)| a.keys.iter().filter(move |k| k.ch & c != 0).map(|k| k.t))
                            .filter(|kt| (kt - raw).abs() <= reach)
                            .min_by(|a, b| (a - raw).abs().total_cmp(&(b - raw).abs()));
                        if let Some(kt) = near {
                            t = kt;
                        }
                    }
                    if (t - self.doc.timeline.time).abs() > 1e-9 {
                        new_time = Some(t);
                    }
                }
                TlDrag::Key { row, from } => {
                    if (t - from).abs() > 1e-9 {
                        let c = row.ch();
                        if let Some(a) = self.row_anim_mut(row) {
                            // only its own value's keys are in the way
                            if !a.keys.iter().any(|k| k.ch & c != 0 && (k.t - t).abs() < 1e-6) {
                                if let Some(k) = a.keys.iter_mut().find(|k| k.ch & c != 0 && (k.t - from).abs() < 1e-6) {
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
                    if let Some(an) = self.doc.get_mut(id).and_then(|l| l.anim.as_mut()) {
                        if let Some([s0, e0]) = an.span {
                            let ns = tl.snap(t_of(pp.x) - grab).max(0.0);
                            let dlt = ns - s0;
                            if dlt.abs() > fl * 0.5 {
                                an.span = Some([ns, e0 + dlt]);
                                for k in &mut an.keys {
                                    k.t += dlt;
                                }
                                changed = true;
                            }
                        }
                    }
                }
                TlDrag::AudioIn(id) | TlDrag::AudioOut(id) => {
                    let fl = tl.frame_len();
                    if let Some(l) = self.doc.get_mut(id) {
                        // a sound ends where its samples do; a video goes on as a
                        // still of its first or last frame
                        let (src_len, video_speed) = match &l.kind {
                            LayerKind::Audio(a) if a.len > 0.0 => (Some(a.len), None),
                            LayerKind::Image(im) => (None, im.video.as_ref().map(|v| v.speed.max(1e-3))),
                            _ => (None, None),
                        };
                        if let Some(an) = l.anim.as_mut() {
                            if let Some([s0, e0]) = an.span {
                                if matches!(d, TlDrag::AudioIn(_)) {
                                    // trim the head: the picture/sound stays where it is in time
                                    // a clip shorter than a frame (a sound not decoded
                                    // yet is [t, t]) has no room: the start stays
                                    let lo = if video_speed.is_some() { 0.0 } else { s0 - an.video_start };
                                    let hi = e0 - fl;
                                    let ns = if hi < lo { s0 } else { t.clamp(lo, hi) };
                                    an.video_start += (ns - s0) * video_speed.unwrap_or(1.0);
                                    an.span = Some([ns, e0]);
                                } else {
                                    let max_end = src_len.map_or(f64::MAX, |len| s0 + (len - an.video_start));
                                    an.span = Some([s0, t.min(max_end).max(s0 + fl)]);
                                }
                                changed = true;
                            }
                        }
                    }
                }
                TlDrag::VideoSlip { id, from, at } => {
                    // the video slides under the clip: later in the clip ⇒ earlier video
                    let speed = self.video_of(id).map_or(1.0, |v| v.speed);
                    if let Some(an) = self.doc.get_mut(id).and_then(|l| l.anim.as_mut()) {
                        let dt = tl.snap(t_of(pp.x)) - tl.snap(at);
                        an.video_start = from - dt * speed;
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
                        for k in a.keys.iter().filter(|k| k.ch & row.ch() != 0) {
                            if mr.contains(pos2(x_of(k.t), rr.center().y)) {
                                picked.push((*row, k.t));
                            }
                        }
                    }
                }
                self.tl_key = picked.first().copied();
                self.tl_sel = picked;
            }
            if matches!(self.tl_drag, Some(TlDrag::Key { .. } | TlDrag::Keys { .. } | TlDrag::SpanIn(_) | TlDrag::SpanOut(_) | TlDrag::AudioMove { .. } | TlDrag::AudioIn(_) | TlDrag::AudioOut(_) | TlDrag::VideoSlip { .. })) {
                self.mark();
                self.set_time(self.doc.timeline.time);
            }
            self.tl_drag = None;
        }
        // right-click on a row's empty track: show the layer from here / until here
        if resp.secondary_clicked() {
            if let Some(pp) = pointer.or(resp.hover_pos()) {
                if let Some((row, _)) = hit_rows.iter().find(|(_, rr)| rr.contains(pp)).copied() {
                    let sound = matches!(row, TlRow::Layer(id, _) if self.doc.get(id).is_some_and(|l| matches!(l.kind, LayerKind::Audio(_))));
                    if pp.x >= tr.left() && !sound {
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
        // a device that went away (unplugged, the default changed): opened anew
        if self.audio_out.as_ref().is_some_and(|o| o.broken()) {
            self.audio_out = None;
            self.audio_failed = false;
        }
        if self.audio_out.is_none() && !self.audio_failed {
            match crate::audio::Output::open() {
                Ok(o) => self.audio_out = Some(o),
                Err(e) => {
                    self.audio_failed = true;
                    self.status = crate::i18n::trf("소리 출력 불가: {}", &[format!("{e}")]);
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
        self.audio_busy.insert(path.to_string());
        let rate = self.audio_rate();
        let tx = self.audio_tx.clone();
        let p = path.to_string();
        std::thread::spawn(move || {
            let r = crate::audio::decode(std::path::Path::new(&p), rate);
            let _ = tx.send((p, r));
        });
    }

    /// A sound file becomes a sound layer: a gizmo at `at` (where it was dropped,
    /// or the middle of the view), its span starting at the playhead (the timeline
    /// turns on). The span's end is set once the file is decoded.
    /// A new FX layer above the selected layer (above a mask's owner), else
    /// at the top: it filters what is below it (fx-layers.md).
    fn add_fx(&mut self) {
        let above = self.sel.and_then(|id| self.doc.get(id)).map(|l| l.mask_of.unwrap_or(l.id));
        let kind = LayerKind::Fx(crate::doc::FxLayer::new(crate::doc::Filter::Blur));
        let id = self.doc.insert("FX".into(), kind, above.or_else(|| self.doc.layers.last().map(|l| l.id)));
        self.select_only(Some(id));
        self.status = crate::i18n::tr("FX 레이어 · 아래 레이어에 필터").into();
        self.mark();
    }

    fn add_audio(&mut self, p: &std::path::Path, at: Pos2) {
        let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or(crate::i18n::tr("소리")).to_string();
        let path = p.to_string_lossy().to_string();
        let kind = LayerKind::Audio(crate::doc::AudioLayer { path: path.clone(), pos: at, len: 0.0, volume: 1.0, mute: false });
        let id = self.doc.insert(name, kind, None);
        let t0 = self.doc.timeline.time;
        if let Some(l) = self.doc.get_mut(id) {
            l.anim = Some(crate::anim::Anim { span: Some([t0, t0]), ..Default::default() });
        }
        self.doc.timeline.on = true;
        self.select_only(Some(id));
        self.ensure_audio(&path);
        self.mark();
    }

    /// Fingerprint of everything that can sound (to refresh a running mix).
    fn voices_sig(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for l in &self.doc.layers {
            if let (LayerKind::Audio(a), Some(an)) = (&l.kind, l.anim.as_ref()) {
                // keyed volume: the keys, not the sampled value that moves while playing
                let keyed = an.keys.iter().any(|k| k.ch & crate::anim::ch::VOL != 0);
                let span = an.span.map(|s| (s[0].to_bits(), s[1].to_bits()));
                (l.id, l.visible, span, an.video_start.to_bits(), a.len.to_bits(), if keyed { 0 } else { a.volume.to_bits() }, a.mute).hash(&mut h);
                vol_keys_hash(an, &mut h);
            }
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
            self.audio_busy.remove(&path);
            match res {
                Ok(d) => {
                    let dur = d.duration();
                    self.audio_cache.insert(path.clone(), Some(Arc::new(d)));
                    // first decode sets the source length (and a new layer's span end);
                    // a long sound stretches the timeline
                    let mut grow = 0.0f64;
                    for l in &mut self.doc.layers {
                        let LayerKind::Audio(a) = &mut l.kind else { continue };
                        if a.path != path || a.len > 0.0 {
                            continue;
                        }
                        a.len = dur;
                        let an = l.anim.get_or_insert_with(Default::default);
                        let [s0, e0] = an.span.unwrap_or([0.0, 0.0]);
                        let e = if e0 <= s0 { s0 + (dur - an.video_start).max(0.0) } else { e0 };
                        an.span = Some([s0, e]);
                        grow = grow.max(e);
                    }
                    if grow > self.doc.timeline.duration {
                        self.doc.timeline.duration = grow.ceil();
                    }
                }
                Err(e) => {
                    self.audio_cache.remove(&path);
                    if self.doc.layers.iter().any(|l| matches!(&l.kind, LayerKind::Audio(a) if a.path == path)) {
                        self.status = crate::i18n::trf("소리 읽기 실패: {}", &[format!("{e}")]);
                    }
                    // keep video files without sound from retrying
                    self.audio_cache.insert(path, None);
                }
            }
        }
        // decode what the document needs (clips, videos with sound)
        let mut need: Vec<String> = Vec::new();
        for l in &self.doc.layers {
            if let LayerKind::Audio(a) = &l.kind {
                need.push(a.path.clone());
            }
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
        for l in self.doc.layers.iter().filter(|l| l.visible) {
            let (LayerKind::Audio(a), Some(an)) = (&l.kind, l.anim.as_ref()) else { continue };
            let Some([s0, e0]) = an.span.filter(|s| s[1] > s[0]) else { continue };
            if a.mute || a.len <= 0.0 {
                continue;
            }
            if let Some(Some(d)) = self.audio_cache.get(&a.path) {
                let env = crate::anim::vol_envelope(an, s0, e0).map(Arc::new);
                out.push(crate::audio::Voice { data: d.clone(), t0: s0, src0: an.video_start, speed: 1.0, span: (s0, e0), gain: a.volume, env, wrap: None });
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
                out.push(crate::audio::Voice { data: d.clone(), t0, src0, speed: v.speed, span, gain: v.volume, env, wrap: v.looping.then_some(v.info.duration) });
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
            // NVENC when the export GPU is NVIDIA's
            gpu: crate::gpu::with_gpu(|g| g.vendor == 0x10de).unwrap_or(false),
            bg: ExportBg::White,
            bg_color: Color32::BLACK,
            audio: true,
            ss: 1,
            size_set: false,
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
                    Err(e) => crate::i18n::trf("영상 내보내기: {}", &[format!("{e}")]),
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
        if let (false, Some((_, r))) = (d.size_set, &region) {
            // the region's own size, until a size is picked
            d.by_width = true;
            d.n = (r.width().round() as u32).clamp(16, 8192);
        }
        let voices = self.timeline_voices().len();
        egui::Window::new(crate::i18n::tr("영상 내보내기"))
            // a fixed id: the title changes with the language
            .id(egui::Id::new("video_export_dialog"))
            // dialogs stay above the fixed panels, whichever is clicked (Ash, 10-28)
            .order(egui::Order::Foreground)
            .title_bar(false)
            .frame(egui::Frame::NONE)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::same(SPACE_LG as i8), theme::radius_card(), |ui| {
                    // as wide as a dial row, so every row ends at one right edge (Ash, 10-29)
                    ui.set_width(crate::dial::ROW_W);
                    if crate::kit::dialog_header(ui, crate::i18n::tr("영상 내보내기")) {
                        open = false;
                    }
                    if let Some(p) = &running {
                        let done = p.done.load(std::sync::atomic::Ordering::Relaxed);
                        let total = p.total.load(std::sync::atomic::Ordering::Relaxed).max(1);
                        ui.add_space(SPACE_SM);
                        ui.add(egui::ProgressBar::new(done as f32 / total as f32).text(crate::i18n::trf("{} / {} 프레임", &[format!("{done}"), format!("{total}")])));
                        if ui.button(crate::i18n::tr("취소")).clicked() {
                            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        return;
                    }
                    let frame_on = self.doc.export_frame.is_some();
                    crate::kit::row(ui, crate::i18n::tr("영역"), |ui| {
                        crate::kit::segmented_en(ui, &mut d.region, &[(ExportRegion::Frame, crate::i18n::tr("내보낼 영역"), frame_on), (ExportRegion::Content, crate::i18n::tr("내용 전체"), true), (ExportRegion::View, crate::i18n::tr("지금 화면"), true)])
                            .glass_tip(crate::i18n::tr("내보낼 영역: F 도구로 그린 사각형"));
                    });
                    if !self.doc.cameras.is_empty() {
                        crate::kit::row(ui, crate::i18n::tr("카메라"), |ui| camera_region_picker(ui, &mut d.region, &self.doc.cameras));
                    }
                    if d.region == ExportRegion::Content {
                        ui.add(crate::dial::Dial::new(&mut d.padding, 0.0..=400.0).text(crate::i18n::tr("여백")).suffix(" px").default(20.0));
                    }
                    crate::kit::row(ui, crate::i18n::tr("크기"), |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(crate::calc::drag(&mut d.n).range(16..=8192).suffix(" px")).changed() {
                                d.size_set = true;
                            }
                            if crate::kit::segmented(ui, &mut d.by_width, &[(true, crate::i18n::tr("가로 기준")), (false, crate::i18n::tr("세로 기준"))]).changed() {
                                d.size_set = true;
                            }
                        });
                    });
                    crate::kit::row(ui, "", |ui| {
                        let presets: &[(u32, &str)] = if d.by_width {
                            &[(3840, "3840"), (2560, "2560"), (1920, "1920"), (1280, "1280"), (1080, "1080"), (854, "854")]
                        } else {
                            &[(2160, "2160"), (1440, "1440"), (1080, "1080"), (720, "720"), (480, "480")]
                        };
                        if crate::kit::segmented(ui, &mut d.n, presets).changed() {
                            d.size_set = true;
                        }
                    });
                    let (w, h) = match &region {
                        Some((_, r)) => crate::vexport::size_for(r.width() / r.height().max(1.0), d.by_width, d.n, d.codec.even()),
                        None => (0, 0),
                    };
                    ui.label(egui::RichText::new(if w > 0 { crate::i18n::trf("→ {} × {} px (영역 비율)", &[format!("{w}"), format!("{h}")]) } else { crate::i18n::tr("영역 없음").into() }).color(theme::LABEL()));
                    crate::kit::row(ui, "fps", |ui| {
                        ui.add(crate::calc::drag(&mut d.fps).range(1.0..=240.0).speed(0.1).max_decimals(3));
                        crate::kit::switch(ui, &mut d.all, crate::i18n::tr("타임라인 전체"));
                    });
                    if !d.all {
                        crate::kit::row(ui, crate::i18n::tr("구간"), |ui| {
                            ui.add(crate::calc::drag(&mut d.t0).range(0.0..=36000.0).speed(0.05).suffix(" s").max_decimals(2));
                            ui.label("~");
                            ui.add(crate::calc::drag(&mut d.t1).range(0.0..=36000.0).speed(0.05).suffix(" s").max_decimals(2));
                        });
                    }
                    ui.add_space(SPACE_SM);
                    crate::kit::row(ui, crate::i18n::tr("형식"), |ui| {
                        let items: Vec<(Codec, String, bool)> = Codec::ALL.iter().map(|c| (*c, c.label().to_string(), true)).collect();
                        if let (Some(c), _) = crate::kit::dropdown(ui, "vcodec", d.codec.label(), crate::kit::FILL, &items, Some(&d.codec)) {
                            d.codec = c;
                        }
                    });
                    if d.codec.has_quality() {
                        crate::kit::row(ui, crate::i18n::tr("화질 기준"), |ui| {
                            crate::kit::segmented(ui, &mut d.use_mbps, &[(false, crate::i18n::tr("품질")), (true, crate::i18n::tr("비트레이트"))]);
                        });
                        // the bitrate is a dial row like the quality, under the
                        // choice (Ash, 10-30: not squeezed in beside it)
                        if d.use_mbps {
                            // the dial's label: a short word where the choice's does not fit the label cell
                            let mut m = d.mbps as f32;
                            if ui.add(crate::dial::Dial::new(&mut m, 0.2..=400.0).step(0.5).suffix(" Mbps").text(crate::i18n::tr_ctx("다이얼", "비트레이트"))).changed() {
                                d.mbps = m as _;
                            }
                        } else {
                            let mut q = d.quality as f32;
                            if ui.add(crate::dial::Dial::new(&mut q, 0.0..=51.0).integer().text(crate::i18n::tr("품질"))).glass_tip(crate::i18n::tr("낮을수록 고화질")).changed() {
                                d.quality = q.round() as _;
                            }
                        }
                    }
                    if d.codec.has_gpu() {
                        crate::kit::switch(ui, &mut d.gpu, crate::i18n::tr("GPU 인코딩 (NVIDIA NVENC)")).glass_tip(crate::i18n::tr("NVIDIA 그래픽카드 필요"));
                    }
                    crate::kit::row(ui, crate::i18n::tr("슈퍼샘플링"), |ui| {
                        crate::kit::segmented(ui, &mut d.ss, &[(1u8, "1×"), (2, "2×"), (3, "3×"), (4, "4×")])
                            .glass_tip(crate::i18n::tr("크게 그린 뒤 줄여서 저장 · 경계와 두께 줄무늬가 부드러워짐"));
                    });
                    if !d.codec.alpha() && d.bg == ExportBg::Transparent {
                        d.bg = ExportBg::White;
                    }
                    crate::kit::row(ui, crate::i18n::tr("배경"), |ui| {
                        let alpha = d.codec.alpha();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if d.bg == ExportBg::Custom {
                                let mut c = d.bg_color;
                                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
                                    d.bg_color = c;
                                }
                            }
                            crate::kit::segmented_en(ui, &mut d.bg, &[(ExportBg::Transparent, crate::i18n::tr("투명"), alpha), (ExportBg::White, crate::i18n::tr("흰색"), true), (ExportBg::Custom, crate::i18n::tr("색"), true)]);
                        });
                    });
                    if d.codec.audio() {
                        ui.add_enabled_ui(voices > 0, |ui| crate::kit::switch(ui, &mut d.audio, &crate::i18n::trf("소리 포함 ({}개)", &[format!("{voices}")])));
                    }
                    ui.add_space(SPACE_SM);
                    ui.add_space(SPACE_SM);
                    ui.horizontal(|ui| {
                        if ui.add_enabled_ui(w > 0, |ui| crate::kit::primary_button(ui, crate::i18n::tr("내보내기…"))).inner.clicked() {
                            go = true;
                        }
                        ui.small(crate::i18n::trf("{} s · {} 프레임", &[format!("{:.2}", (if d.all { self.doc.timeline.duration } else { d.t1 - d.t0 }).max(0.0)), format!("{}", ((if d.all { self.doc.timeline.duration } else { d.t1 - d.t0 }).max(0.0) * d.fps).ceil())]));
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
            self.status = crate::i18n::tr("빈 구간").into();
            return;
        }
        // a sound still being decoded would be left out of the video
        if d.audio && d.codec.audio() {
            let waiting = self.doc.layers.iter().filter(|l| l.visible).any(|l| match &l.kind {
                LayerKind::Audio(a) => !a.mute && self.audio_busy.contains(&a.path),
                LayerKind::Image(im) => im.video.as_ref().is_some_and(|v| v.info.audio && !v.mute && self.audio_busy.contains(&v.path)),
                _ => false,
            });
            if waiting {
                self.status = crate::i18n::tr("소리 읽는 중 · 끝난 뒤 다시 내보내기").into();
                return;
            }
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
        self.status = crate::i18n::tr("영상 내보내는 중…").into();
    }


    fn import_dialog(&mut self) {
        let all: Vec<&str> = crate::io::IMAGE_EXTS.iter().chain(crate::video::VIDEO_EXTS).chain(crate::audio::AUDIO_EXTS).copied().collect();
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter(crate::i18n::tr("사진·영상"), &all)
            .add_filter(crate::i18n::tr("사진"), crate::io::IMAGE_EXTS)
            .add_filter(crate::i18n::tr("영상"), crate::video::VIDEO_EXTS)
            .add_filter(crate::i18n::tr("소리"), crate::audio::AUDIO_EXTS)
            .pick_files()
        {
            let at = self.view_center_world();
            self.import_paths(paths, at);
        }
    }

    fn paste(&mut self) {
        let at = self.view_center_world();
        let Ok(mut cb) = arboard::Clipboard::new() else {
            self.status = crate::i18n::tr("클립보드를 열 수 없음").into();
            return;
        };
        if let Ok(img) = cb.get_image() {
            if let Some(rgba) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) {
                let src = crate::io::image_from_rgba(rgba);
                self.add_image(src, crate::i18n::tr("붙여넣은 사진").into(), at);
                self.status = crate::i18n::tr("클립보드 사진 붙여넣음").into();
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
            // a link (http, https, data:image): fetched and placed as a photo
            // when it is one, else the link as text (`take_drop`)
            if crate::oledrop::is_url(&text) {
                crate::oledrop::fetch(text.trim().to_string(), None);
                self.status = crate::i18n::tr("링크 받는 중…").into();
                return;
            }
            let text = text.replace("\r\n", "\n");
            if !text.trim().is_empty() {
                let t = self.new_text_layer(at, text);
                self.sel = Some(t);
                self.mark();
                self.status = crate::i18n::tr("글자 붙여넣음").into();
            }
        }
    }

    fn new_text_layer(&mut self, at: Pos2, text: String) -> LayerId {
        let t = TextLayer {
            pos: at,
            text,
            size: self.s.text_size,
            color: self.s.text_color,
            family: self.s.text_family.clone(),
            style: self.s.text_style.clone(),
            bg: self.s.text_bg.then_some(self.s.text_bg_color),
            wrap: 0.0,
            rotation: 0.0,
            blend: Blend::Normal,
            align: self.s.text_align,
            look: self.s.text_look,
        };
        self.doc.insert(crate::i18n::tr("글").into(), LayerKind::Text(t), self.sel)
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
                self.status = crate::i18n::tr("새 작업").into();
            }
            Pending::Open(path) => {
                let path = path.or_else(|| rfd::FileDialog::new().add_filter("Drifting VOID", &["void"]).pick_file());
                if let Some(p) = path {
                    self.open_path(p);
                }
            }
            Pending::Quit => {
                self.save_prefs();
                self.pending = None;
                // (after an update that would not start, the next change is copied again)
                let end_recovery = |me: &mut Self| {
                    if let Some(r) = me.rec.as_mut() {
                        r.end();
                    }
                };
                // an update waiting: its installer runs once this has quit. If
                // it cannot start, the app stays (the notice says why).
                if let Some(p) = self.update_go.take() {
                    // the app the installer starts opens this project again
                    let note = self.path.clone().zip(crate::recovery::dir()).filter(|_| self.rec.is_some());
                    if let Some((project, dir)) = &note {
                        crate::recovery::leave_reopen(dir, project);
                    }
                    if !self.upd.install(&p) {
                        if let Some((_, dir)) = &note {
                            crate::recovery::take_reopen(dir);
                        }
                        self.status = crate::i18n::tr("업데이트 설치 실패").into();
                        return;
                    }
                }
                self.modified = false;
                // closing in order: the changes were saved or given up
                end_recovery(self);
                std::process::exit(0);
            }
        }
    }

    fn open_path(&mut self, p: PathBuf) {
        match crate::io::load_project(&p) {
            Ok(d) => {
                self.reset_doc(d, Some(p.clone()));
                self.status = crate::i18n::trf("열림: {}", &[format!("{}", p.file_name().and_then(|n| n.to_str()).unwrap_or(""))]);
            }
            Err(e) => self.status = crate::i18n::trf("열기 실패: {}", &[format!("{e}")]),
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
                self.status = crate::i18n::trf("저장됨: {}", &[format!("{}", p.file_name().and_then(|n| n.to_str()).unwrap_or(""))]);
                self.path = Some(p);
                self.modified = false;
                true
            }
            Err(e) => {
                self.status = crate::i18n::trf("저장 실패: {}", &[format!("{e}")]);
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

    /// Empties the selected brush and pen layers and masks (their pixels,
    /// recordings, strokes), keeping the layers (Delete).
    fn clear_selected(&mut self) {
        let mut n = 0;
        for id in self.selected_ids() {
            if self.clear_layer(id) {
                n += 1;
            }
        }
        if n > 0 {
            self.mark();
            self.status = crate::i18n::trf("레이어 {}개 비움 · 삭제: Backspace", &[format!("{n}")]);
        } else if self.sel.is_some() {
            self.status = crate::i18n::tr("비울 수 없는 레이어 · 삭제: Backspace").into();
        }
    }

    /// Empties one brush or pen layer or mask; false for other kinds (or a
    /// locked layer). A photo mask goes back to an ordinary one (the next
    /// trace makes it a photo mask again, on whatever photo it is then).
    fn clear_layer(&mut self, id: LayerId) -> bool {
        let Some(l) = self.doc.get_mut(id).filter(|l| !l.locked) else { return false };
        match &mut l.kind {
            LayerKind::Raster(r) => {
                r.tiles.clear();
                r.recs.clear();
                r.rec_base = None;
            }
            LayerKind::Vector(v) => v.strokes.clear(),
            _ => return false,
        }
        if l.mask_res != 1.0 {
            l.mask_res = 1.0;
            l.mask_pose = None;
            l.mask_anchor = l.mask_of.map(|o| (o, [0.0, 0.0]));
        }
        let owner = l.mask_of;
        // an ordinary mask's anchor point starts where its layer is now
        if let Some(o) = owner {
            let c = self.doc.get(o).map(|ol| crate::anim::key_of(ol, 0.0).center);
            if let (Some(c), Some(l)) = (c, self.doc.get_mut(id)) {
                if l.mask_anchor.is_some_and(|a| a.0 == o) {
                    l.mask_anchor = Some((o, c));
                }
            }
        }
        true
    }

    fn duplicate_selected(&mut self) {
        let ids = self.selected_ids();
        let mut new_ids = Vec::new();
        for id in ids {
            let Some(idx) = self.doc.index_of(id) else { continue };
            // a mask is copied with its layer, not by itself (two masks on one
            // layer: only the first would count)
            if self.doc.layers[idx].mask_of.is_some() {
                continue;
            }
            let d = vec2(30.0, 30.0) / self.zoom.max(0.1);
            let mut copy = self.doc.layers[idx].clone();
            copy.id = self.doc.new_id();
            copy.translate(d);
            // its position keys move with it (else it plays back where the original is)
            if let Some(an) = copy.anim.as_mut() {
                for k in an.keys.iter_mut().filter(|k| k.ch & crate::anim::ch::POS != 0) {
                    k.center[0] += d.x;
                    k.center[1] += d.y;
                }
            }
            let mut masks: Vec<Layer> = self.doc.layers.iter().filter(|m| m.mask_of == Some(id)).cloned().collect();
            for m in &mut masks {
                m.id = self.doc.new_id();
                m.mask_of = Some(copy.id);
                m.translate(d);
                if let Some((a, last)) = m.mask_anchor {
                    m.mask_anchor = Some(if a == id { (copy.id, [last[0] + d.x, last[1] + d.y]) } else { (a, last) });
                }
            }
            new_ids.push(copy.id);
            // masks sit just under their layer
            let n = masks.len();
            for (k, m) in masks.into_iter().enumerate() {
                self.doc.layers.insert(idx + 1 + k, m);
            }
            self.doc.layers.insert(idx + 1 + n, copy);
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
        // sounds and FX layers have no picture to place
        if self.doc.get(id).is_some_and(|l| matches!(l.kind, LayerKind::Audio(_) | LayerKind::Fx(_))) {
            return;
        }
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
        self.status = crate::i18n::trf("{}에 넣음", &[format!("{name}")]);
        self.mark();
    }

    /// Stands a layer upright on the floor where canvas point `w` shows, facing the
    /// camera, its bottom middle on the floor (AR-style placing).
    fn stand_on_floor(&mut self, id: LayerId, sid: u64, w: Pos2, lie: bool) {
        if self.doc.get(id).is_some_and(|l| matches!(l.kind, LayerKind::Audio(_) | LayerKind::Fx(_))) {
            return;
        }
        let Some(sp) = self.doc.spaces.iter().find(|s| s.id == sid) else { return };
        let Ok(cam) = crate::persp::solve(sp) else {
            self.status = crate::i18n::tr("공간 카메라 계산 불가").into();
            return;
        };
        let Some(at) = cam.hit_floor(w) else {
            self.status = crate::i18n::tr("바닥 아님 (지평선 위)").into();
            return;
        };
        let Some(i) = self.doc.index_of(id) else { return };
        let Some(b) = self.doc.layers[i].bounds() else { return };
        // standing: its bottom middle on the floor; lying: its centre there
        let mut pl = if lie { crate::persp::Placement::lying(sid, &cam, at, b.center()) } else { crate::persp::Placement::standing(sid, &cam, at, b.center_bottom()) };
        if let Some(old) = &self.doc.layers[i].place {
            pl.thick = old.thick;
            pl.segs = old.segs;
            pl.smooth = old.smooth;
        }
        self.doc.layers[i].place = Some(pl);
        if let Some(top) = self.doc.layers.iter().rposition(|l| l.id != id && l.place.as_ref().is_some_and(|p| p.space == sid)) {
            let l = self.doc.layers.remove(i);
            let to = if top > i { top } else { top + 1 };
            self.doc.layers.insert(to.min(self.doc.layers.len()), l);
        }
        self.status = if lie { crate::i18n::tr("바닥에 눕힘") } else { crate::i18n::tr("바닥에 세움") }.into();
        self.mark();
    }

    /// Puts a layer on a wall: its plane lies on the wall where its centre shows, and it
    /// moves along the wall from then on.
    fn put_in_wall(&mut self, id: LayerId, sid: u64, wid: u64) {
        if self.doc.get(id).is_some_and(|l| matches!(l.kind, LayerKind::Audio(_) | LayerKind::Fx(_))) {
            return;
        }
        let Some(sp) = self.doc.spaces.iter().find(|s| s.id == sid) else { return };
        let Ok(cam) = crate::persp::solve(sp) else {
            self.status = crate::i18n::tr("공간 카메라 계산 불가").into();
            return;
        };
        let Some(wall) = sp.walls.iter().find(|w| w.id == wid).cloned() else { return };
        let Some(i) = self.doc.index_of(id) else { return };
        let Some(b) = self.doc.layers[i].bounds() else { return };
        let Some(mut pl) = crate::persp::Placement::on_wall(sid, &cam, &wall, b.center()) else {
            self.status = crate::i18n::tr("벽에 닿지 않음 (카메라 뒤쪽)").into();
            return;
        };
        if let Some(old) = &self.doc.layers[i].place {
            pl.thick = old.thick;
            pl.segs = old.segs;
            pl.smooth = old.smooth;
        }
        self.doc.layers[i].place = Some(pl);
        let same = |l: &Layer| l.id != id && l.place.as_ref().is_some_and(|p| p.wall == Some(wid));
        let near = self.doc.layers.iter().rposition(same).or_else(|| self.doc.layers.iter().rposition(|l| l.id != id && l.place.as_ref().is_some_and(|p| p.space == sid)));
        if let Some(top) = near {
            let l = self.doc.layers.remove(i);
            let to = if top > i { top } else { top + 1 };
            self.doc.layers.insert(to.min(self.doc.layers.len()), l);
        }
        self.status = crate::i18n::trf("{}에 붙임", &[format!("{}", wall.name)]);
        self.mark();
    }

    fn take_out_of_space(&mut self, id: LayerId) {
        if let Some(l) = self.doc.get_mut(id) {
            if l.place.take().is_some() {
                self.status = crate::i18n::tr("공간에서 뺌 (원래 2D 모습으로)").into();
                self.mark();
            }
        }
    }

    fn new_paint_layer(&mut self, vector: bool) -> LayerId {
        let (name, kind) = if vector {
            let n = self.doc.count_kind(|k| matches!(k, LayerKind::Vector(_))) + 1;
            (crate::i18n::trf("펜 {}", &[format!("{n}")]), LayerKind::Vector(VectorLayer::default()))
        } else {
            let n = self.doc.count_kind(|k| matches!(k, LayerKind::Raster(_))) + 1;
            (crate::i18n::trf("브러시 {}", &[format!("{n}")]), LayerKind::Raster(RasterLayer::default()))
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
                LayerKind::Audio(a) => a.rect().expand(tol).contains(w),
                // picked in the layer list only
                LayerKind::Fx(_) => false,
            };
            if hit {
                return Some(l.id);
            }
        }
        None
    }

    fn save_prefs(&mut self) {
        // debug runs (DV_* flags) set up scenes and panels; they never touch Ash's prefs
        if std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("DV_")) {
            return;
        }
        let out = PrefsOut { settings: &self.s, recent: &self.recent, export: self.last_export.as_ref() };
        let Ok(text) = serde_json::to_string_pretty(&out) else { return };
        if text != self.prefs_saved {
            if let Some(p) = prefs_path() {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                // beside it, then renamed: a write cut short never leaves half a file
                let tmp = p.with_extension("json.tmp");
                if std::fs::write(&tmp, &text).is_ok() && std::fs::rename(&tmp, &p).is_ok() {
                    self.prefs_saved = text;
                } else {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
        }
    }

    /// Align / distribute buttons for a multi-selection.
    fn align_bar(&mut self, ui: &mut egui::Ui) {
        let mut act = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_2XS, SPACE_2XS);
            for (i, (label, tip)) in [
                (crate::i18n::tr("왼쪽"), crate::i18n::tr("왼쪽 맞춤")),
                (crate::i18n::tr("가로 중앙"), crate::i18n::tr("가로 가운데 맞춤")),
                (crate::i18n::tr("오른쪽"), crate::i18n::tr("오른쪽 맞춤")),
                (crate::i18n::tr("위"), crate::i18n::tr("위쪽 맞춤")),
                (crate::i18n::tr("세로 중앙"), crate::i18n::tr("세로 가운데 맞춤")),
                (crate::i18n::tr("아래"), crate::i18n::tr("아래쪽 맞춤")),
                (crate::i18n::tr("가로 간격"), crate::i18n::tr("가로 간격 같게 (양 끝 고정)")),
                (crate::i18n::tr("세로 간격"), crate::i18n::tr("세로 간격 같게 (양 끝 고정)")),
            ]
            .into_iter()
            .enumerate()
            {
                if ui.add(egui::Button::new(label).small()).glass_tip(tip).clicked() {
                    act = Some(i);
                }
            }
        });
        ui.add_space(SPACE_XS);
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
    /// Layers in a 3D space are left out: the box scales and turns in 2D, and they
    /// turn and scale in their space (G/R/S, the panel); their own outlines show
    /// where they are.
    fn group_box(&self) -> Option<Rect> {
        if self.extra_sel.is_empty() && self.sub.is_empty() {
            return self.single_handles().and_then(|_| self.sel_layer()?.bounds());
        }
        let whole = self.selected_ids().iter().filter_map(|id| self.doc.get(*id)).filter(|l| !l.locked && l.place.is_none()).filter_map(|l| l.bounds()).reduce(|a, b| a.union(b));
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
            Some(Drag::ScaleImage { id, orig, .. } | Drag::Rotate { id, orig } | Drag::Crop { id, orig, .. }) => {
                if let Some(LayerKind::Image(im)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    *im = orig;
                }
            }
            Some(Drag::MoveMany { origs, .. } | Drag::Group { origs, .. }) => {
                for (o, _) in origs {
                    if let Some(i) = self.doc.index_of(o.id) {
                        self.doc.layers[i] = o;
                    }
                }
            }
            Some(Drag::Cam { id, orig, .. }) => {
                if let Some(c) = self.doc.cameras.iter_mut().find(|c| c.id == id) {
                    c.rect = orig;
                }
            }
            // (a zero-size original: it was being drawn new)
            Some(Drag::Frame { orig, .. }) => self.doc.export_frame = (orig.width() > 0.0 || orig.height() > 0.0).then_some(orig),
            Some(Drag::SpaceFrame { id, orig, .. }) => {
                if orig.width() > 0.0 || orig.height() > 0.0 {
                    if let Some(s) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                        s.frame = orig;
                    }
                } else {
                    self.doc.spaces.retain(|s| s.id != id);
                    self.space_sel = self.doc.spaces.last().map(|s| s.id);
                }
            }
            Some(Drag::SpaceLine { id, idx, orig, .. }) => {
                if let Some(s) = self.doc.spaces.iter_mut().find(|s| s.id == id) {
                    if orig.a == orig.b {
                        if idx < s.lines.len() {
                            s.lines.remove(idx);
                        }
                        self.space_line = None;
                    } else if let Some(l) = s.lines.get_mut(idx) {
                        *l = orig;
                    }
                }
            }
            // a shape being drawn goes; one drawn for a mask (its rectangle, a
            // GrabCut box) leaves the mask selected and nothing waiting to be
            // written into it
            Some(Drag::NewShape { id, .. }) => {
                if let Some(i) = self.doc.index_of(id) {
                    self.doc.layers.remove(i);
                }
                let mask = self.mask_rect.take().map(|m| m.0);
                self.mask_cut = false;
                if self.sel == Some(id) || mask.is_some() {
                    self.sel = mask.filter(|m| self.doc.get(*m).is_some());
                }
                self.mark();
            }
            _ => {}
        }
        // whatever was cancelled, no "write into this mask on release" survives it
        self.mask_rect = None;
        self.mask_cut = false;
    }

    fn pointer_down(&mut self, sp: Pos2, pressure: f32, mods: Modifiers, space: bool) {
        self.stroke_fed = self.now;
        let xf = self.xf();
        let w = xf.s2w(sp);
        if let Some(mid) = self.wand.filter(|_| !space && self.selecting()) {
            if self.doc.get(mid).is_some_and(|l| l.mask_of.is_some()) {
                if self.wand_mode == 2 {
                    // a press starts the outline (or carries it on); dragging
                    // traces, letting go (or a click) puts an anchor there
                    let fresh = !self.scissor.as_ref().is_some_and(|sc| sc.mid == mid);
                    if fresh {
                        self.scissor_click(mid, w, !mods.alt, mods.shift);
                    }
                    self.drag = Some(Drag::Scissor { start: sp, moved: false, fresh, refine: mods.shift });
                } else if self.wand_mode == 1 {
                    // GrabCut: drag a box (a temporary outline, run on release)
                    let s = ShapeLayer { kind: ShapeKind::Rect, a: w, b: w, color: ACCENT, width: 1.5 / self.zoom, fill: None, rotation: 0.0 };
                    let id = self.doc.insert(crate::i18n::tr("영역").into(), LayerKind::Shape(s), None);
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
        let mask_rect_tool = self.tool == Tool::Rect && self.sel_layer().is_some_and(|l| l.mask_of.is_some());
        if mods.alt && !mask_rect_tool {
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
            if let Some(c) = sample_region(&mut self.cgpu, &self.doc, w) {
                self.s.color = c;
                self.remember_color(c);
                self.status = crate::i18n::trf("색 선택: #{}{}{}", &[format!("{:02X}", c.r()), format!("{:02X}", c.g()), format!("{:02X}", c.b())]);
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
                        // 2D layers only (see `group_box`)
                        let origs: Vec<_> = self.sel_parts().into_iter().filter(|(l, _)| l.place.is_none()).collect();
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
                        // in a 3D space the layer shows projected: the pointer's
                        // canvas point is not where its pixels are (as the brush
                        // and the pen, which never take a placed layer)
                        Some(l) if is_paint_layer(&l.kind) && l.place.is_some() => {
                            self.status = crate::i18n::tr("3D 공간 안 레이어는 지우개 불가 · 공간에서 뺀 뒤 지우기").into();
                            return;
                        }
                        Some(l) if is_paint_layer(&l.kind) && !l.locked && l.visible => l.id,
                        _ => {
                            self.status = crate::i18n::tr("지울 그림 레이어를 먼저 선택").into();
                            return;
                        }
                    }
                } else {
                    self.paint_target(false)
                };
                let (tl_on, tl_time) = (self.doc.timeline.on, self.doc.timeline.time);
                // a photo mask is painted in its photo's pixels
                let space = self.mask_space(id);
                let to_px = |p: Pos2| space.map_or(p, |m| m.inv(p));
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
                            radius: size * 0.5 / space.map_or(1.0, |m| m.s),
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
                            let lp = to_px(w) - r.offset;
                            self.rec_clock = Some((std::time::Instant::now(), tl_time, self.rec_on));
                            self.rec_pts = vec![[lp.x, lp.y, pressure]];
                            self.rec_times = vec![0.0];
                        }
                        let mut st = RasterStroke::new(p);
                        st.add_point(r, to_px(w) - r.offset, pressure);
                        self.drag = Some(Drag::Raster { id, stroke: Some(st), smooth: (w, pressure), curve: crate::vector::Curve::new([w.x, w.y, pressure], self.smooth_distance()) });
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
                    curve: crate::vector::Curve::new([w.x, w.y, pressure], self.smooth_distance()),
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
                    ShapeKind::Line => crate::i18n::tr("선"),
                    ShapeKind::Arrow => crate::i18n::tr("화살표"),
                    ShapeKind::Rect => crate::i18n::tr("사각형"),
                    ShapeKind::Ellipse => crate::i18n::tr("원"),
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
                if let Some(LayerKind::Image(im)) = self.sel_layer().filter(|l| !l.locked && l.place.is_none()).map(|l| &l.kind) {
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
            self.status = crate::i18n::tr("2D 작업 공간").into();
        } else if let Some((sid, cam)) = self.ws_space() {
            if self.view3d.as_ref().is_none_or(|v| v.space != sid) {
                self.view3d = Some(crate::view3d::View3d::user(sid, &cam));
            }
            self.finish_text_edit();
            self.drag = None;
            self.ws3d = true;
            self.status = crate::i18n::tr("3D 작업 공간 · Tab: 2D로 · 꾹 누르면 미리보기").into();
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
        let moving = self.nav3d.is_some() || self.modal3d.is_some();
        let (cg, doc) = (&mut self.cgpu, &self.doc);
        let mut gpu = |p: &egui::Painter, what: crate::view3d::Layers| gpu_layers(cg, SLOT_3D, p, doc, what, false);
        crate::view3d::draw(&painter, ctx, &self.doc, &view, &proj, cam, &mut self.tex, self.sel, moving, &mut gpu);
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
                let mode = if view.camera { crate::i18n::tr("카메라") } else if view.ortho { crate::i18n::tr("직교") } else { crate::i18n::tr("원근") };
                crate::view3d::hud(&painter, rect, &crate::i18n::trf("{} · {} · 가운데 버튼: 회전 (Shift 이동, Ctrl 확대) · 휠: 확대 · G/R/S · Tab: 2D", &[format!("{name}"), format!("{mode}")]));
            }
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
                            self.status = crate::i18n::tr("빛 회전 · Esc: 취소").into();
                        }
                        (_, None, _) => self.status = crate::i18n::tr("조명 꺼짐 · 공간 패널에서 켜기").into(),
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
                    .filter(|l| !l.locked)
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
                            _ => self.status = crate::i18n::tr("이 공간의 레이어 선택 필요").into(),
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
                theme::glass(ui, egui::Margin::same(6), theme::radius_card(), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_XS;
                        let Some(v) = self.view3d.as_mut() else { return };
                        if crate::kit::chip(ui, v.camera, crate::i18n::tr("카메라 0")).clicked() {
                            v.camera = true;
                        }
                        if ui.button(crate::i18n::tr("앞 1")).clicked() {
                            v.preset(1, false);
                        }
                        if ui.button(crate::i18n::tr("오른쪽 3")).clicked() {
                            v.preset(3, false);
                        }
                        if ui.button(crate::i18n::tr("위 7")).clicked() {
                            v.preset(7, false);
                        }
                        if crate::kit::chip(ui, v.ortho && !v.camera, crate::i18n::tr("직교 5")).clicked() {
                            v.ortho = !v.ortho;
                            v.camera = false;
                        }
                        ui.separator();
                        if ui.button(crate::i18n::tr("2D로 · Tab")).clicked() {
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
            if !gpu_layers(&mut self.cgpu, SLOT_PEEK, &clip, &self.doc, crate::view3d::Layers::Canvas(&xf), true) {
                theme::draw_ground(&clip, pr, &xf);
                canvas::draw_doc(&clip, ctx, &self.doc, &xf, &mut self.tex, None);
            }
        } else if let Some((sid, cam)) = self.ws_space() {
            let view = match &self.view3d {
                Some(v) if v.space == sid => v.clone(),
                _ => crate::view3d::View3d::user(sid, &cam),
            };
            theme::draw_void(&clip, pr);
            let proj = crate::view3d::Proj::new(&view, &cam, pr);
            let (cg, doc) = (&mut self.cgpu, &self.doc);
            let mut gpu = |p: &egui::Painter, what: crate::view3d::Layers| gpu_layers(cg, SLOT_PEEK, p, doc, what, false);
            crate::view3d::draw(&clip, ctx, &self.doc, &view, &proj, &cam, &mut self.tex, self.sel, false, &mut gpu);
        }
        p.rect_stroke(pr, 12.0, Stroke::new(1.5, INK_3()), StrokeKind::Outside);
        p.text(pr.left_top() + vec2(16.0, 14.0), Align2::LEFT_TOP, if self.ws3d { crate::i18n::tr("2D 미리보기") } else { crate::i18n::tr("3D 미리보기") }, FontId::proportional(13.0), theme::LABEL());
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
        // the open canvas: right of the tool rail, left of the panels
        let room = Rect::from_min_max(pos2(rect.left() + 72.0, rect.top()), pos2((rect.right() - self.right_reserve).max(rect.left() + 300.0), rect.bottom()));
        let mut c = ts - vec2(r + gap, r + gap);
        if c.x - r < room.left() + 8.0 {
            c.x = ts.x + r + gap;
        }
        if c.y - r < room.top() + 8.0 {
            c.y = ts.y + r + gap;
        }
        // the pixels around the target, rendered on the CPU and drawn as a round disc
        let zoom = (self.zoom * 6.0).max(4.0);
        let ppp = ctx.pixels_per_point();
        // Shift while dragging: the edge view instead (a third of the magnification,
        // so nine times the area; levels stretched; Canny edges on a dimmed
        // picture). DV_LOUPE=edge holds it for captures.
        let edge_view = ctx.input(|i| i.modifiers.shift) || std::env::var("DV_LOUPE").is_ok_and(|v| v == "edge");
        if edge_view {
            let zoom2 = zoom / 3.0;
            let key2 = [target.x, target.y, zoom2, 1.0];
            if self.edge_tex.as_ref().is_none_or(|(_, k)| *k != key2) {
                let region = Rect::from_center_size(target, vec2(2.0 * r / zoom2, 2.0 * r / zoom2));
                let m = zoom2 * ppp;
                if let Ok(pm) = render_region(&mut self.cgpu, &self.doc, region, m, None) {
                    let (w, h) = (pm.width() as usize, pm.height() as usize);
                    let mut px = pm.data().to_vec();
                    let e = crate::edges::canny(&crate::edges::luminance(&px), w, h, (m * 0.8).clamp(1.0, 4.0), 0.85);
                    crate::edges::levels(&mut px);
                    {
                        for (k, p) in px.chunks_exact_mut(4).enumerate() {
                            if e[k] {
                                p.copy_from_slice(&[255, 226, 120, 255]);
                            } else {
                                for c in &mut p[..3] {
                                    *c = (*c as f32 * 0.45) as u8;
                                }
                            }
                        }
                    }
                    let ci = egui::ColorImage::from_rgba_premultiplied([w, h], &px);
                    let opts = egui::TextureOptions::NEAREST;
                    match &mut self.edge_tex {
                        Some((t, k)) => {
                            t.set(ci, opts);
                            *k = key2;
                        }
                        None => self.edge_tex = Some((ctx.load_texture("edge_loupe", ci, opts), key2)),
                    }
                }
            }
            if let Some((t, _)) = &self.edge_tex {
                loupe_disc(painter, t.id(), c, r, zoom2, target, line);
            }
            return;
        }
        let key = [target.x, target.y, zoom];
        if self.loupe_tex.as_ref().is_none_or(|(_, k)| *k != key) {
            let region = Rect::from_center_size(target, vec2(2.0 * r / zoom, 2.0 * r / zoom));
            if let Ok(pm) = render_region(&mut self.cgpu, &self.doc, region, zoom * ppp, None) {
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
        if let Some((t, _)) = &self.loupe_tex {
            loupe_disc(painter, t.id(), c, r, zoom, target, line);
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

    /// 선 찾기: renders the active space's frame and finds straight-edge
    /// candidates in it (`lines.rs`), shown until toggled off.
    fn find_lines(&mut self) {
        let Some(i) = self.space_idx() else { return };
        let (sid, fr) = (self.doc.spaces[i].id, self.doc.spaces[i].frame);
        let long = fr.width().max(fr.height());
        if long < 8.0 {
            return;
        }
        // about 1600 px on the long side: enough for edges, fast enough to wait for
        let m = (1600.0 / long).min(4.0);
        let pm = match render_region(&mut self.cgpu, &self.doc, fr, m, None) {
            Ok(pm) => pm,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let t = std::time::Instant::now();
        let (w, h) = (pm.width() as usize, pm.height() as usize);
        let segs: Vec<crate::lines::Seg> = crate::lines::detect(&crate::edges::luminance(pm.data()), w, h)
            .into_iter()
            .map(|s| crate::lines::Seg { a: fr.min + s.a.to_vec2() / m, b: fr.min + s.b.to_vec2() / m })
            .filter(|s| s.len() >= 0.015 * long)
            .collect();
        let norm = crate::lines::Norm { c: fr.center(), l: long };
        let groups = crate::lines::group(&segs, &norm, CAND_COLORS.len());
        let mut of = vec![None; segs.len()];
        for (g, gr) in groups.iter().enumerate() {
            for &k in &gr.members {
                of[k] = Some(g);
            }
        }
        if std::env::var_os("DV_LAYOUT_LOG").is_some() {
            eprintln!("lines: {} segments, {} groups, {:?} ({w}x{h})", segs.len(), groups.len(), t.elapsed());
        }
        self.status = crate::i18n::trf("직선 후보 {}개", &[format!("{}", segs.len())]);
        self.line_cands = Some(LineCands { sid, frame: fr, segs, groups, of, norm });
    }

    /// 자동 선택: adds up to four candidate lines per axis to the active space,
    /// in one undo step. Axes come from the space's camera when its own lines
    /// already fix one (two axes of two lines), else from the candidates'
    /// orthogonality (`lines::auto_axes`).
    fn auto_pick_lines(&mut self) {
        use crate::lines::Seg;
        let Some(i) = self.space_idx() else { return };
        let sp = &self.doc.spaces[i];
        let Some(lc) = self.line_cands.as_ref().filter(|c| c.of_space(sp)) else { return };
        let fixed = (0..3).filter(|&a| sp.count(a) >= 2).count() >= 2;
        let axes: [Option<usize>; 3] = match crate::persp::solve(sp).ok().filter(|_| fixed) {
            Some(cam) => {
                let named = crate::lines::axes_from_camera(&lc.groups, &lc.segs, &cam);
                // groups come strongest first: each axis takes its first
                std::array::from_fn(|a| named.iter().position(|n| *n == Some(a as u8)))
            }
            None => crate::lines::auto_axes(&lc.groups, &lc.segs),
        };
        let taken: Vec<Seg> = sp.lines.iter().map(|l| Seg { a: l.a, b: l.b }).collect();
        let mut add = Vec::new();
        for (ax, g) in axes.iter().enumerate() {
            let Some(g) = g else { continue };
            let need = 4usize.saturating_sub(sp.count(ax as u8));
            for k in crate::lines::pick_lines(&lc.groups[*g], &lc.segs, &lc.norm, &taken, need) {
                add.push(crate::persp::VLine { a: lc.segs[k].a, b: lc.segs[k].b, axis: ax as u8 });
            }
        }
        if add.is_empty() {
            self.status = crate::i18n::tr("고를 축 후보 없음").into();
            return;
        }
        self.status = crate::i18n::trf("선 {}개 추가", &[format!("{}", add.len())]);
        self.doc.spaces[i].lines.extend(add);
        self.space_line = None;
        self.mark();
    }

    fn space_down(&mut self, sp: Pos2, w: Pos2) {
        let xf = self.xf();
        if self.wall_base_new {
            if let Some(i) = self.space_idx() {
                let sid = self.doc.spaces[i].id;
                let wid = self.doc.new_id();
                let n = self.doc.spaces[i].walls.len() + 1;
                self.doc.spaces[i].walls.push(crate::persp::Wall { id: wid, name: crate::i18n::trf("벽 {}", &[format!("{n}")]), corners: [w; 4], offset: 0.0, snap_axis: false });
                self.wall_sel = Some(wid);
                self.wall_base_new = false;
                self.drag = Some(Drag::WallBase { sid, wid, start: w });
                self.mark();
                return;
            }
        }
        if let Some((lid, lie)) = self.space_place.take() {
            if let Some(i) = self.space_idx() {
                self.stand_on_floor(lid, self.doc.spaces[i].id, w, lie);
                return;
            }
        }
        if self.wall_new {
            if let Some(i) = self.space_idx() {
                let sid = self.doc.spaces[i].id;
                let wid = self.doc.new_id();
                let n = self.doc.spaces[i].walls.len() + 1;
                self.doc.spaces[i].walls.push(crate::persp::Wall { id: wid, name: crate::i18n::trf("벽 {}", &[format!("{n}")]), corners: [w; 4], offset: 0.0, snap_axis: false });
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
            // a candidate line under the pointer: taken on the current axis
            let cand = if hit.is_some() {
                None
            } else {
                self.line_cands.as_ref().filter(|c| c.of_space(s)).and_then(|c| {
                    c.open(s).map(|(_, g)| (seg_dist(sp, xf.w2s(g.a), xf.w2s(g.b)), *g)).filter(|(d, _)| *d <= 5.0).min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1)
                })
            };
            let drag = match hit {
                Some(d) => d,
                None if cand.is_some() => {
                    let g = cand.unwrap();
                    let l = crate::persp::VLine { a: g.a, b: g.b, axis: self.space_axis };
                    self.doc.spaces[i].lines.push(l);
                    let idx = self.doc.spaces[i].lines.len() - 1;
                    self.space_line = Some(idx);
                    Drag::SpaceLine { id, idx, end: 2, start: w, orig: l }
                }
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
        self.doc.spaces.push(crate::persp::Space::new(id, crate::i18n::trf("공간 {}", &[format!("{n}")]), Rect::from_min_max(p, p)));
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
        let zoom = self.zoom;
        // Space held mid-drag: the view pans with the pointer while the dragged thing
        // stays under it; letting go of Space carries on from the same spot
        if self.space_held && self.drag.as_ref().is_some_and(|d| !matches!(d, Drag::Pan | Drag::Zoom { .. } | Drag::Size { .. })) {
            self.pan += delta_s;
            return;
        }
        if let Some(Drag::Scissor { start, moved, fresh, refine }) = self.drag {
            let moved = moved || sp.distance(start) > 3.0;
            self.drag = Some(Drag::Scissor { start, moved, fresh, refine });
            if moved {
                self.scissor_trace(w);
            }
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
        // a brush stroke on a photo mask goes into its photo's pixels
        let raster_space = match &self.drag {
            Some(Drag::Raster { id, .. }) => self.mask_space(*id),
            _ => None,
        };
        let Some(drag) = self.drag.as_mut() else { return };
        match drag {
            Drag::Zoom { .. } | Drag::Size { .. } | Drag::Marquee { .. } | Drag::Lasso { .. } | Drag::Scissor { .. } => {}
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
                        // Slides along its own plane under the pointer. Alt: floor
                        // snapping (opt-in; walls are the usual way to place): a
                        // layer touching the floor when the drag starts slides along
                        // the floor, any other lands on the floor once its lowest
                        // point comes within 10 px of it.
                        let snap = mods.alt;
                        let bx = orig.bounds().unwrap_or(Rect::ZERO);
                        let fz = cam.floor_z;
                        let tol = (cam.origin_depth() * 1e-3).max(0.05);
                        let on_floor = snap && (o.lowest(bx)[2] - fz).abs() < tol;
                        let mut pl = o.clone();
                        if on_floor {
                            if let (Some(a), Some(b)) = (cam.hit_floor(*start), cam.hit_floor(*start + d)) {
                                pl.shift_world(crate::persp::sub(b, a));
                            }
                        } else if let (Some(a), Some(b)) = (o.ray_hit(&cam, *start), o.ray_hit(&cam, *start + d)) {
                            pl.shift_world(crate::persp::sub(b, a));
                            if snap {
                                let low = pl.lowest(bx);
                                let on = [low[0], low[1], fz];
                                if let (Some(p), Some(q)) = (cam.project(low), cam.project(on)) {
                                    if p.distance(q) * self.zoom < 10.0 {
                                        pl.shift_world([0.0, 0.0, fz - low[2]]);
                                    }
                                }
                            }
                        }
                        l.place = Some(pl);
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
                    // along the box's own direction; its corner stays where it is
                    let s0 = t.frame();
                    let r = crate::text::layout(t).rect(t.pos);
                    let wl = crate::doc::rot_about(w, r.center(), -t.rotation);
                    t.wrap = (wl.x - r.left()).max(t.size * 1.5);
                    t.keep_corner(s0);
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
            Drag::Raster { id, stroke, smooth, curve } => {
                // smoothing happens in the curve (weighted by the path's length)
                *smooth = (w, pressure);
                self.stroke_fed = self.now;
                let id = *id;
                let more = curve.push([w.x, w.y, pressure], CURVE_STEP / zoom, self.now);
                if let (Some(st), Some(LayerKind::Raster(r))) = (stroke.as_mut(), self.doc.get_mut(id).map(|l| &mut l.kind)) {
                    Self::raster_points(st, r, raster_space, &more, self.rec_clock, &mut self.rec_pts, &mut self.rec_times);
                }
                self.changed = true;
                self.modified = true;
            }
            Drag::Vector { id, pts, smooth, base_r, pressure: use_p, curve, .. } => {
                *smooth = (w, pressure);
                self.stroke_fed = self.now;
                let off = match self.doc.get(*id).map(|l| &l.kind) {
                    Some(LayerKind::Vector(v)) => v.offset,
                    _ => Vec2::ZERO,
                };
                let more = curve.push([w.x, w.y, pressure], CURVE_STEP / zoom, self.now);
                Self::vector_points(pts, &more, off, *base_r, *use_p, zoom, self.rec_clock, &mut self.rec_times);
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

    /// A frame of a pen or brush stroke without a pen sample (`dt` seconds):
    /// the brush keeps following at its pace (`vector::Curve::follow`), and
    /// settles on the pen once it has `rested`. True while it still moves.
    fn stroke_follow(&mut self, dt: f32, rested: bool) -> bool {
        let step = CURVE_STEP / self.zoom;
        let zoom = self.zoom;
        let space = match &self.drag {
            Some(Drag::Raster { id, .. }) => self.mask_space(*id),
            _ => None,
        };
        match &mut self.drag {
            Some(Drag::Raster { id, stroke: Some(st), curve, .. }) => {
                let (more, going) = curve.follow(dt, rested, step);
                let id = *id;
                if let Some(LayerKind::Raster(r)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    Self::raster_points(st, r, space, &more, self.rec_clock, &mut self.rec_pts, &mut self.rec_times);
                }
                if !more.is_empty() {
                    self.changed = true;
                    self.modified = true;
                }
                going
            }
            Some(Drag::Vector { id, pts, base_r, pressure, curve, .. }) => {
                let (more, going) = curve.follow(dt, rested, step);
                let off = match self.doc.get(*id).map(|l| &l.kind) {
                    Some(LayerKind::Vector(v)) => v.offset,
                    _ => Vec2::ZERO,
                };
                Self::vector_points(pts, &more, off, *base_r, *pressure, zoom, self.rec_clock, &mut self.rec_times);
                going
            }
            _ => false,
        }
    }

    /// The smoothing length for a new stroke, in canvas units.
    fn smooth_distance(&self) -> f32 {
        self.s.smoothing.clamp(0.0, 0.95) * SMOOTH_PX / self.zoom
    }

    /// Curve points into a brush stroke (and its recording); `space`: a
    /// photo mask's pixels.
    #[allow(clippy::too_many_arguments)]
    fn raster_points(st: &mut RasterStroke, r: &mut crate::raster::RasterLayer, space: Option<crate::export::MaskMap>, more: &[[f32; 3]], rec: Option<(std::time::Instant, f64, bool)>, rec_pts: &mut Vec<[f32; 3]>, rec_times: &mut Vec<f32>) {
        for p in more {
            let w = pos2(p[0], p[1]);
            let lp = space.map_or(w, |m| m.inv(w)) - r.offset;
            st.add_point(r, lp, p[2]);
            if let Some((c, _, live)) = rec {
                rec_pts.push([lp.x, lp.y, p[2]]);
                rec_times.push(if live { c.elapsed().as_secs_f32() } else { 0.0 });
            }
        }
    }

    /// Curve points into a pen stroke's points (and its recording times).
    #[allow(clippy::too_many_arguments)]
    fn vector_points(pts: &mut Vec<[f32; 3]>, more: &[[f32; 3]], off: Vec2, base_r: f32, use_p: bool, zoom: f32, rec: Option<(std::time::Instant, f64, bool)>, rec_times: &mut Vec<f32>) {
        for p in more {
            let l = pos2(p[0], p[1]) - off;
            let last = pts.last().copied().unwrap();
            if (pos2(last[0], last[1]) - l).length() >= 0.75 / zoom {
                pts.push([l.x, l.y, radius_for(base_r, p[2], use_p)]);
                if let Some((c, _, _)) = rec {
                    rec_times.push(c.elapsed().as_secs_f32());
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
            Drag::Scissor { moved, fresh, refine, .. } => {
                // the press already put the first anchor
                if fresh && !moved {
                    return;
                }
                if let Some((mid, on)) = self.scissor.as_ref().map(|sc| (sc.mid, sc.on)) {
                    let w = self.xf().s2w(sp);
                    self.scissor_click(mid, w, on, refine);
                }
            }
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
                    let cams = crate::persp::cameras(&self.doc.spaces);
                    for l in self.doc.layers.iter().rev().filter(|l| l.visible && !l.locked && l.mask_of.is_none()) {
                        // layers in a space: whole, judged where they show (not
                        // where they sat on the canvas before going in)
                        if l.place.is_some() {
                            if l.shown_bounds(&cams).is_some_and(inside) {
                                ids.push(l.id);
                            }
                            continue;
                        }
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
                        crate::i18n::tr("영역 안에 선택할 것 없음").into()
                    } else {
                        let mut parts = Vec::new();
                        if !ids.is_empty() {
                            parts.push(crate::i18n::trf("레이어 {}", &[format!("{}", ids.len())]));
                        }
                        if ns > 0 {
                            parts.push(crate::i18n::trf("펜 획 {}", &[format!("{ns}")]));
                        }
                        if np > 0 {
                            parts.push(crate::i18n::trf("브러시 영역 {}", &[format!("{np}")]));
                        }
                        crate::i18n::trf("선택: {}", &[format!("{}", parts.join(" · "))])
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
                let cams = crate::persp::cameras(&self.doc.spaces);
                for l in self.doc.layers.iter().rev().filter(|l| l.visible && !l.locked && l.mask_of.is_none()) {
                    if l.place.is_some() {
                        if l.shown_bounds(&cams).is_some_and(|lb| in_poly(lb.center(), &pts)) {
                            ids.push(l.id);
                        }
                        continue;
                    }
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
                self.status = crate::i18n::trf("선택: 레이어 {} · 펜 획 {} · 브러시 영역 {}", &[format!("{}", ids.len()), format!("{ns}"), format!("{np}")]);
            }
            Drag::Group { center, s, th, .. } => {
                self.remap_pixel_sel(center, s, th, Vec2::ZERO);
            }
            Drag::Raster { id, stroke: Some(mut st), mut curve, .. } => {
                let params = st.p;
                let more = curve.finish(CURVE_STEP / self.zoom);
                let space = self.mask_space(id);
                if let Some(LayerKind::Raster(r)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    Self::raster_points(&mut st, r, space, &more, self.rec_clock, &mut self.rec_pts, &mut self.rec_times);
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
            Drag::Vector { id, mut pts, color, base_r, pressure, mut curve, .. } => {
                let more = curve.finish(CURVE_STEP / self.zoom);
                let zoom = self.zoom;
                if let Some(LayerKind::Vector(v)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    // (it ends where the brush is, not at the pen: see `Curve::finish`)
                    Self::vector_points(&mut pts, &more, v.offset, base_r, pressure, zoom, self.rec_clock, &mut self.rec_times);
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
                } else if let Some(rect) = rect {
                    match self.mask_space(mid) {
                        // a photo mask: the rectangle in its pixels (turned with the photo)
                        Some(map) => {
                            let pts: Vec<Pos2> = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].iter().map(|p| map.inv(*p)).collect();
                            if let Some(LayerKind::Raster(m)) = self.doc.get_mut(mid).map(|l| &mut l.kind) {
                                fill_mask_poly(m, &pts, on);
                            }
                        }
                        None => {
                            if let Some(LayerKind::Raster(m)) = self.doc.get_mut(mid).map(|l| &mut l.kind) {
                                m.fill_rect(rect, on);
                            }
                        }
                    }
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
                // scissors: right-click puts an anchor (starts an outline, or
                // closes it on the first anchor); also while tracing with a drag
                egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed: true, modifiers }
                    if over(*pos) && self.drag.as_ref().is_none_or(|d| matches!(d, Drag::Scissor { .. })) && self.scissor_target().is_some() =>
                {
                    if let Some(mid) = self.scissor_target() {
                        let w = self.xf().s2w(*pos);
                        let on = self.scissor.as_ref().filter(|sc| sc.mid == mid).map_or(!modifiers.alt, |sc| sc.on);
                        self.scissor_click(mid, w, on, modifiers.shift);
                        if let Some(Drag::Scissor { start, refine, .. }) = self.drag {
                            // the drag carries on from this anchor
                            self.drag = Some(Drag::Scissor { start, moved: true, fresh: false, refine });
                        }
                    }
                }
                egui::Event::PointerButton { pos, button: PointerButton::Middle, pressed, modifiers } => {
                    self.log_input(format!("mouse middle {}", if *pressed { "down" } else { "up" }));
                    if *pressed {
                        if self.pan_latch {
                            self.pan_latch = false;
                            if self.source == Source::Middle {
                                self.drag = None;
                            }
                            self.status = crate::i18n::tr("펜 버튼 이동 끔").into();
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
                            self.status = crate::i18n::tr("펜 버튼 이동 켬 · 다시 누르면 끔").into();
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

        // a pen or brush stroke: in frames without a pen sample the brush keeps
        // following at its pace, and once the pen has rested a moment the path
        // is completed up to it and the brush settles there (not sooner: that
        // pulled the brush straight at the pen's newest point and left burrs)
        if matches!(self.drag, Some(Drag::Raster { .. } | Drag::Vector { .. })) {
            let rested = self.now - self.stroke_fed > STROKE_REST;
            let fed_now = self.stroke_fed == self.now;
            let dt = ctx.input(|i| i.unstable_dt).clamp(0.0, 0.1);
            let going = !fed_now && self.stroke_follow(dt, rested);
            if going {
                ctx.request_repaint();
            } else if !rested {
                // frames keep coming while the pen holds still
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(STROKE_REST));
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
        // G/R/S in progress: it has the keys (no paste, no workspace switch)
        if self.modal3d.is_some() || self.grs2d.is_some() {
            self.tab_raw.clear();
            self.tab_t0 = None;
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
                    self.status = crate::i18n::tr("3D 공간 없음").into();
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
            self.status = if self.doc.timeline.on { crate::i18n::tr("타임라인 켬 · Alt+A 재생 · I 키 넣기 · , . 프레임").into() } else { crate::i18n::tr("타임라인 끔").into() };
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
            let placed = self.sel_layer().filter(|l| self.extra_sel.is_empty() && !l.locked).and_then(|l| l.place.clone().map(|p| (l.id, p)));
            if placed.is_none() {
                use crate::view3d::Op;
                for (key, op) in [(Key::G, Op::Grab), (Key::R, Op::Rotate), (Key::S, Op::Scale)] {
                    // turning and scaling are 2D: layers in a space keep theirs (see `group_box`)
                    let origs: Vec<_> = self.sel_parts().into_iter().filter(|(l, _)| op == Op::Grab || l.place.is_none()).collect();
                    if origs.is_empty() {
                        continue;
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
        // UI scale: before Ctrl+0, which also matches while Alt is held
        let ca = Modifiers::COMMAND | Modifiers::ALT;
        if k(ca, Key::Plus) || k(ca, Key::Equals) {
            self.set_ui_scale(self.s.ui_scale + 0.1);
        }
        if k(ca, Key::Minus) {
            self.set_ui_scale(self.s.ui_scale - 0.1);
        }
        if k(ca, Key::Num0) {
            self.set_ui_scale(1.0);
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
        // Backspace deletes layers, Delete empties them (Ash, 10-23); a part of
        // a layer (strokes, pixels) selected goes with either
        let has_part = !self.sub.strokes.is_empty() || !self.sub.pixels.is_empty();
        if k(Modifiers::NONE, Key::Backspace) {
            self.delete_selected();
        }
        if k(Modifiers::NONE, Key::Delete) {
            if has_part {
                self.delete_selected();
            } else {
                self.clear_selected();
            }
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
            None => self.status = crate::i18n::tr("복사할 내용 없음").into(),
        }
    }

    fn copy_region(&mut self, r: Rect, scale: f32, bg: Option<Color32>) {
        self.copy_shot(r, 0.0, scale, bg, 1, crate::doc::Film::Fixed(r));
    }

    /// A region to the clipboard, as an export of it to a file would look
    /// (`rot`: a camera's turn; `ss`: supersampling; `film`: what vignette
    /// and colour fringes centre on).
    fn copy_shot(&mut self, r: Rect, rot: f32, scale: f32, bg: Option<Color32>, ss: u8, film: crate::doc::Film) {
        let mut doc = self.export_doc();
        doc.film = Some(film);
        let shot = if rot == 0.0 && ss <= 1 { render_region_for(&mut self.cgpu, &doc, r, scale, bg, true) } else { crate::gpu::render_ss(&doc, r.center(), r.size(), rot, scale, bg, ss, false) };
        match shot {
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
                    Ok(()) => crate::i18n::trf("클립보드에 복사됨 ({}×{})", &[format!("{}", pm.width()), format!("{}", pm.height())]),
                    Err(e) => crate::i18n::trf("복사 실패: {}", &[format!("{e}")]),
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
        egui::Window::new(crate::i18n::tr("내보내기"))
            .id(egui::Id::new("export_dialog"))
            // dialogs stay above the fixed panels, whichever is clicked (Ash, 10-28)
            .order(egui::Order::Foreground)
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
            .show(ctx, |ui| theme::glass(ui, egui::Margin::same(SPACE_LG as i8), theme::radius_card(), |ui| {
            // as wide as a dial row, so every row ends at one right edge (Ash, 10-29)
            ui.set_width(crate::dial::ROW_W);
            if crate::kit::dialog_header(ui, crate::i18n::tr("내보내기")) {
                open = false;
            }
            let frame_on = self.doc.export_frame.is_some();
            crate::kit::row(ui, crate::i18n::tr("영역"), |ui| {
                crate::kit::segmented_en(ui, &mut d.region, &[(ExportRegion::Frame, crate::i18n::tr("내보낼 영역"), frame_on), (ExportRegion::Content, crate::i18n::tr("내용 전체"), true), (ExportRegion::View, crate::i18n::tr("지금 화면"), true)])
                    .glass_tip(crate::i18n::tr("내보낼 영역: F 도구로 그린 사각형"));
            });
            if !self.doc.cameras.is_empty() {
                crate::kit::row(ui, crate::i18n::tr("카메라"), |ui| camera_region_picker(ui, &mut d.region, &self.doc.cameras));
            }
            if let ExportRegion::Camera(id) = d.region {
                if !self.doc.cameras.iter().any(|c| c.id == id) {
                    d.region = ExportRegion::Content;
                }
            }
            if d.region == ExportRegion::Frame && !frame_on {
                d.region = ExportRegion::Content;
            }
            if d.region == ExportRegion::Content {
                ui.add(crate::dial::Dial::new(&mut d.padding, 0.0..=400.0).text(crate::i18n::tr("여백")).suffix(" px").default(20.0));
            }
            crate::kit::row(ui, crate::i18n::tr("배율"), |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(crate::calc::drag(&mut d.scale).custom_parser(crate::calc::parse).range(0.05..=8.0).speed(0.01).suffix("×"));
                    crate::kit::segmented(ui, &mut d.scale, &[(0.25f32, "0.25×"), (0.5, "0.5×"), (1.0, "1×"), (2.0, "2×")]);
                });
            });
            crate::kit::row(ui, crate::i18n::tr("배경"), |ui| {
                crate::kit::segmented(ui, &mut d.bg, &[(ExportBg::Transparent, crate::i18n::tr("투명")), (ExportBg::White, crate::i18n::tr("흰색")), (ExportBg::Custom, crate::i18n::tr("색"))]);
            });
            if d.bg == ExportBg::Custom {
                crate::kit::row(ui, "", |ui| color_row(ui, &mut d.bg_color, PalTarget::ExportBg, &mut pal_req));
            }
            crate::kit::row(ui, crate::i18n::tr("슈퍼샘플링"), |ui| {
                crate::kit::segmented(ui, &mut d.ss, &[(1u8, "1×"), (2, "2×"), (3, "3×"), (4, "4×")])
                    .glass_tip(crate::i18n::tr("크게 그린 뒤 줄여서 저장 · 경계와 두께 줄무늬가 부드러워짐"));
            });
            crate::dial::dial_u8(ui, &mut d.jpeg_quality, 50..=100, crate::i18n::tr("JPG 품질"));
            let region = self.export_region(&d);
            match region {
                Some(r) => {
                    let (w, h) = crate::export::output_size(r, d.scale);
                    ui.label(crate::i18n::trf("결과 크기: {} × {} px", &[format!("{w}"), format!("{h}")]));
                }
                None => {
                    ui.label(crate::i18n::tr("내보낼 내용 없음"));
                }
            }
            ui.separator();
            // buttons flow from the left and wrap (Japanese is longer: Ash, 10-29)
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(region.is_some(), |ui| {
                    if crate::kit::primary_button(ui, crate::i18n::tr("파일로 저장…")).clicked() {
                        action = Some(1);
                    }
                    if ui.button(crate::i18n::tr("클립보드로 복사")).clicked() {
                        action = Some(2);
                    }
                });
            });
            ui.small(crate::i18n::tr("PNG: 투명 배경 유지 · JPG: 배경색으로 채움"));
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
                    let mut doc = self.export_doc();
                    doc.film = Some(match d.region {
                        ExportRegion::Camera(id) => crate::doc::Film::Camera(id),
                        _ => crate::doc::Film::Fixed(r),
                    });
                    let (scale, ss, q) = (d.scale, d.ss, d.jpeg_quality);
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let res = crate::gpu::render_ss(&doc, r.center(), r.size(), rot, scale, bg, ss, false)
                            .and_then(|pm| crate::export::save(&pm, &path, q).map(|_| (pm.width(), pm.height())))
                            .map(|(w, h)| crate::i18n::trf("내보냄: {} ({}×{})", &[format!("{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("")), format!("{w}"), format!("{h}")]))
                            .unwrap_or_else(|e| crate::i18n::trf("내보내기 실패: {}", &[format!("{e}")]));
                        let _ = tx.send(res);
                    });
                    self.img_job = Some(rx);
                    self.status = crate::i18n::tr("이미지 내보내는 중…").into();
                }
            } else {
                let (rot, film) = match d.region {
                    ExportRegion::Camera(id) => (self.doc.cameras.iter().find(|c| c.id == id).map_or(0.0, |c| c.rot), crate::doc::Film::Camera(id)),
                    _ => (0.0, crate::doc::Film::Fixed(r)),
                };
                self.copy_shot(r, rot, d.scale, bg, d.ss, film);
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
        if active {
            self.rail_active = Some(r);
        }
        let p = ui.painter();
        // the active tool's amber fill glides: under the rail's glass on the
        // canvas overlay (the glass spreads it), or in the rail without glass
        if resp.hovered() && !active {
            p.rect_filled(r, theme::inner(theme::radius_card(), 6.0) as f32, FILL_2());
        }
        // off in the theme's ink, not grey (Ash, 10-22)
        let col = if active {
            if crate::glass::available() { Color32::WHITE } else { ACCENT_INK }
        } else {
            INK()
        };
        theme::tool_icon(p, t, Rect::from_center_size(r.center(), theme::ICON), col);
        let (name, key) = TOOLS.iter().find(|x| x.0 == t).map(|x| (crate::i18n::tr(x.1), x.2)).unwrap_or(("", ""));
        if resp.hovered() {
            self.rail_hover = Some(((name, key, tool_hint(t)), r));
        }
        if resp.clicked() {
            self.set_tool(t);
        }
    }

    fn tool_rail(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        self.rail_hover = None;
        let rail = egui::Area::new(egui::Id::new("rail"))
            .order(egui::Order::Middle)
            .fixed_pos(pos2(screen.left() + SPACE_LG, screen.top() + 76.0))
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::same(6), theme::radius_card(), |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, SPACE_2XS);
                    // the active tool's fill goes under the icons, above the glass
                    let fill = ui.painter().add(Shape::Noop);
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
                    if !crate::glass::available() {
                        if let Some(ir) = self.rail_indicator(ui.ctx()) {
                            ui.painter().set(fill, Shape::rect_filled(ir, theme::inner(theme::radius_card(), 6.0) as f32, ACCENT));
                        }
                    }
                    ui.add_space(SPACE_SM);
                    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                    let p = ui.painter();
                    p.circle_filled(r.center(), 14.0, FILL_2());
                    p.circle_filled(r.center(), 11.0, self.s.color);
                    if resp.hovered() {
                        p.circle_stroke(r.center(), 14.0, Stroke::new(1.0, INK_2()));
                    }
                    if resp.hovered() {
                        self.rail_hover = Some(((crate::i18n::tr("색"), "", crate::i18n::tr("클릭: 팔레트 · 캔버스에서는 Alt+클릭")), r));
                    }
                    if resp.clicked() {
                        self.open_palette(r.right_top() + vec2(10.0, -150.0));
                    }
                })
                .response
                .rect
            });
        self.rail_hint_box(ctx, rail.inner);
    }

    /// The hint box beside the rail (Ash, 10-27): sized to its text, level
    /// with the hovered button, and moving only within the rail's top and
    /// bottom. It opens after a short hover and then follows the pointer from
    /// button to button at once. `DV_RAIL_TIP=n` keeps the n-th tool's open.
    fn rail_hint_box(&mut self, ctx: &egui::Context, rail: Rect) {
        const DELAY: f64 = 0.25;
        let forced = std::env::var("DV_RAIL_TIP").ok().and_then(|v| v.parse::<usize>().ok()).and_then(|i| TOOLS.get(i)).map(|t| {
            // the button's place: 36 px rows 2 apart from the rail's top, groups split by 9
            let i = TOOLS.iter().position(|x| x.0 == t.0).unwrap_or(0);
            let groups = [3usize, 6, 11];
            let gaps = groups.iter().filter(|&&g| i >= g).count() as f32;
            let y = rail.top() + 6.0 + i as f32 * 38.0 + gaps * 11.0;
            ((crate::i18n::tr(t.1), t.2, tool_hint(t.0)), Rect::from_min_size(pos2(rail.left() + 6.0, y), vec2(36.0, 36.0)))
        });
        let hover = forced.or(self.rail_hover);
        self.rail_hint = match (hover, self.rail_hint) {
            (None, _) => None,
            (Some((h, r)), Some((_, _, since, open))) => Some((h, r, since, open || self.now - since >= DELAY || forced.is_some())),
            (Some((h, r)), None) => Some((h, r, self.now, forced.is_some())),
        };
        let Some(((name, key, hint), btn, _, open)) = self.rail_hint else { return };
        if !open {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(DELAY));
            return;
        }
        // centred on the button, kept between the rail's top and bottom
        let h = self.rail_hint_h;
        let top = (btn.center().y - h * 0.5).min(rail.bottom() - h).max(rail.top());
        let pad = theme::SPACE_MD;
        let shown = egui::Area::new(egui::Id::new("rail_hint"))
            .order(egui::Order::Middle)
            .interactable(false)
            .fixed_pos(pos2(rail.right() + SPACE_SM, top))
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::same(pad as i8), theme::radius_card(), |ui| {
                    ui.set_width(232.0);
                    ui.spacing_mut().item_spacing.y = SPACE_SM;
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(name).font(FontId::new(theme::TEXT_BODY + 1.0, theme::ui_bold())).color(INK()));
                        if !key.is_empty() {
                            ui.label(egui::RichText::new(key).monospace().color(INK()).background_color(FILL_2()));
                        }
                    });
                    for item in hint.split(" · ") {
                        // "input: what it does": the input bold, the result after it,
                        // in one text so every line is spaced alike
                        let mut job = egui::text::LayoutJob::default();
                        let plain = egui::TextFormat { font_id: FontId::proportional(theme::TEXT_SMALL + 0.5), color: theme::LABEL(), ..Default::default() };
                        match item.split_once(": ") {
                            Some((input, what)) => {
                                job.append(input, 0.0, egui::TextFormat { font_id: FontId::new(theme::TEXT_SMALL + 0.5, theme::ui_bold()), color: INK(), ..Default::default() });
                                job.append(what, SPACE_XS, plain);
                            }
                            None => job.append(item, 0.0, plain),
                        }
                        job.wrap.max_width = ui.available_width();
                        ui.label(job);
                    }
                });
            });
        if (shown.response.rect.height() - self.rail_hint_h).abs() > 0.5 {
            self.rail_hint_h = shown.response.rect.height();
            ctx.request_repaint();
        }
    }

    /// Applies the picked main colour to a selected shape layer. (A text has
    /// the text tool's own colour and is left alone.)
    fn recolor_selected(&mut self) {
        let c = self.s.color;
        if let Some(id) = self.sel {
            if let Some(l) = self.doc.get_mut(id).filter(|l| !l.locked) {
                match &mut l.kind {
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
        // (a locked layer is not edited from here: the tool shows its defaults)
        let sel_idx = self.sel.and_then(|id| self.doc.index_of(id)).filter(|&i| self.extra_sel.is_empty() && !self.doc.layers[i].locked);
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
                ui.strong(crate::i18n::tr("브러시"));
                // presets: (name, size, opacity, hardness, pressure->size, pressure->opacity, smoothing)
                const PRESETS: [(&str, f32, f32, f32, bool, bool, f32); 5] = [
                    (crate::i18n::k!("연필"), 3.0, 0.9, 0.95, true, false, 0.2),
                    (crate::i18n::k!("펜촉"), 6.0, 1.0, 1.0, true, false, 0.45),
                    (crate::i18n::k!("마커"), 18.0, 0.55, 0.7, false, false, 0.3),
                    (crate::i18n::k!("에어브러시"), 60.0, 0.25, 0.0, false, true, 0.2),
                    (crate::i18n::k!("수채"), 34.0, 0.4, 0.2, true, true, 0.35),
                ];
                // one of five presets (Ash, 10-27: segmented, no chips)
                let mut pick = PRESETS.iter().position(|p| p.0 == s.brush_preset).unwrap_or(usize::MAX);
                // names stay Korean in the preferences; shown in the UI's language
                let opts: Vec<(usize, &str)> = PRESETS.iter().enumerate().map(|(i, p)| (i, crate::i18n::tr(p.0))).collect();
                if crate::kit::segmented(ui, &mut pick, &opts).changed() {
                    let (name, size, op, hard, ps, po, sm) = PRESETS[pick];
                    s.brush_preset = name.to_string();
                    s.brush_size = size;
                    s.brush_opacity = op;
                    s.brush_hardness = hard;
                    s.brush_pressure_size = ps;
                    s.brush_pressure_opacity = po;
                    s.smoothing = sm;
                }
                // reset targets: the picked preset's values, else the app defaults
                let base = Settings::default();
                let (d_size, d_op, d_hard, d_sm) = PRESETS
                    .iter()
                    .find(|p| p.0 == s.brush_preset)
                    .map(|p| (p.1, p.2, p.3, p.6))
                    .unwrap_or((base.brush_size, base.brush_opacity, base.brush_hardness, base.smoothing));
                ui.add(crate::dial::Dial::new(&mut s.brush_size, 1.0..=400.0).step(2.0).text(crate::i18n::tr("크기")).default(d_size));
                ui.add(crate::dial::Dial::new(&mut s.brush_opacity, 0.05..=1.0).text(crate::i18n::tr("불투명도")).default(d_op));
                ui.add(crate::dial::Dial::new(&mut s.brush_hardness, 0.0..=1.0).text(crate::i18n::tr("경도")).default(d_hard));
                crate::kit::switch(ui, &mut s.brush_pressure_size, crate::i18n::tr("필압 → 크기"));
                crate::kit::switch(ui, &mut s.brush_pressure_opacity, crate::i18n::tr("필압 → 농도"));
                ui.add(crate::dial::Dial::new(&mut s.smoothing, 0.0..=0.9).text(crate::i18n::tr("손떨림 보정")).default(d_sm));
            }
            Tool::Pen => {
                ui.strong(crate::i18n::tr("펜 (벡터)"));
                ui.add(crate::dial::Dial::new(&mut s.pen_size, 0.5..=100.0).step(0.5).text(crate::i18n::tr("굵기")));
                crate::kit::switch(ui, &mut s.pen_pressure, crate::i18n::tr("필압 → 굵기"));
                ui.add(crate::dial::Dial::new(&mut s.smoothing, 0.0..=0.9).text(crate::i18n::tr("손떨림 보정")));
            }
            Tool::Eraser => {
                ui.strong(crate::i18n::tr("지우개"));
                ui.add(crate::dial::Dial::new(&mut s.eraser_size, 2.0..=400.0).step(2.0).text(crate::i18n::tr("크기")));
                crate::kit::switch(ui, &mut s.eraser_pressure, crate::i18n::tr("필압 → 크기"));
            }
            Tool::Text => {
                ui.strong(crate::i18n::tr("글자"));
                if let Some(LayerKind::Text(t)) = sel_text.map(|i| &mut self.doc.layers[i].kind) {
                    let s0 = t.frame();
                    layer_changed |= ui.add(crate::dial::Dial::new(&mut t.size, 6.0..=400.0).step(2.0).text(crate::i18n::tr("크기"))).changed();
                    layer_changed |= font_picker(ui, "tool_font", &mut t.family, &mut t.style, &mut self.font_ui);
                    layer_changed |= align_row(ui, &mut t.align, false);
                    ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                        ui.label(crate::i18n::tr("색"));
                        layer_changed |= color_row(ui, &mut t.color, PalTarget::TextColor(lid), &mut pal_req);
                    });
                    ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                        let mut has = t.bg.is_some();
                        if crate::kit::switch(ui, &mut has, crate::i18n::tr("배경 상자")).changed() {
                            t.bg = has.then_some(s.text_bg_color);
                            layer_changed = true;
                        }
                        if let Some(bg) = &mut t.bg {
                            layer_changed |= color_row(ui, bg, PalTarget::TextBg(lid), &mut pal_req);
                        }
                    });
                    layer_changed |= text_look_ui(ui, &mut t.look, t.bg.is_some(), (&mut s.text_border_color, &mut s.text_stroke_color), (PalTarget::TextBorder(lid), PalTarget::TextStroke(lid)), &mut pal_req);
                    if layer_changed {
                        t.keep_corner(s0);
                    }
                    // the next new text starts from the same style
                    s.text_size = t.size;
                    s.text_family = t.family.clone();
                    s.text_style = t.style.clone();
                    s.text_align = t.align;
                    s.text_color = t.color;
                    s.text_bg = t.bg.is_some();
                    s.text_look = t.look;
                    if let Some(c) = t.bg {
                        s.text_bg_color = c;
                    }
                    ui.small(crate::i18n::tr("선택한 글자에 적용 · 새 글자도 이 설정으로 시작"));
                } else {
                    ui.add(crate::dial::Dial::new(&mut s.text_size, 6.0..=400.0).step(2.0).text(crate::i18n::tr("크기")));
                    font_picker(ui, "tool_font", &mut s.text_family, &mut s.text_style, &mut self.font_ui);
                    align_row(ui, &mut s.text_align, false);
                    ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                        ui.label(crate::i18n::tr("색"));
                        color_row(ui, &mut s.text_color, PalTarget::DefaultTextColor, &mut pal_req);
                    });
                    ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                        crate::kit::switch(ui, &mut s.text_bg, crate::i18n::tr("배경 상자"));
                        color_row(ui, &mut s.text_bg_color, PalTarget::DefaultTextBg, &mut pal_req);
                    });
                    text_look_ui(ui, &mut s.text_look, s.text_bg, (&mut s.text_border_color, &mut s.text_stroke_color), (PalTarget::DefaultTextBorder, PalTarget::DefaultTextStroke), &mut pal_req);
                }
            }
            Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse => {
                ui.strong(crate::i18n::tr("도형"));
                if let Some(LayerKind::Shape(sh)) = sel_shape.map(|i| &mut self.doc.layers[i].kind) {
                    layer_changed |= ui.add(crate::dial::Dial::new(&mut sh.width, 0.0..=60.0).text(crate::i18n::tr("선 굵기"))).changed();
                    if matches!(sh.kind, ShapeKind::Rect | ShapeKind::Ellipse) {
                        ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                            let mut has = sh.fill.is_some();
                            if crate::kit::switch(ui, &mut has, crate::i18n::tr("채우기")).changed() {
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
                    ui.small(crate::i18n::tr("선택한 도형에 적용 · 새 도형도 이 설정으로 시작"));
                } else {
                    ui.add(crate::dial::Dial::new(&mut s.shape_width, 0.0..=60.0).text(crate::i18n::tr("선 굵기")));
                    if matches!(self.tool, Tool::Rect | Tool::Ellipse) {
                        ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                            crate::kit::switch(ui, &mut s.shape_fill, crate::i18n::tr("채우기"));
                            color_row(ui, &mut s.fill_color, PalTarget::DefaultFill, &mut pal_req);
                        });
                    }
                }
            }
            Tool::Crop => {
                ui.strong(crate::i18n::tr("사진 자르기"));
            }
            Tool::Frame => {
                ui.strong(crate::i18n::tr("내보낼 영역"));
                if let Some(mut r) = self.doc.export_frame {
                    if rect_fields(ui, &mut r, "frame", None, &mut false) {
                        self.doc.export_frame = Some(r);
                        self.mark();
                    }
                    if ui.button(crate::i18n::tr("영역 지우기")).clicked() {
                        self.doc.export_frame = None;
                        self.mark();
                    }
                }
                if ui.button(crate::i18n::tr("내용 전체에 맞추기")).clicked() {
                    if let Some(b) = self.doc.content_bounds() {
                        let b = b.expand(20.0);
                        self.doc.export_frame = Some(Rect::from_min_max(b.min.floor(), b.max.ceil()));
                        self.mark();
                    }
                }
                // exactly the selection's bounds (padding is set when saving)
                if let Some(b) = self.selection_bounds() {
                    if ui.button(crate::i18n::tr("선택에 맞추기")).clicked() {
                        self.doc.export_frame = Some(Rect::from_min_max(b.min.floor(), b.max.ceil()));
                        self.mark();
                    }
                }
            }
            Tool::Space => self.space_panel(ui),
            Tool::Select => {
                ui.strong(crate::i18n::tr("선택"));
            }
            Tool::Lasso => {
                ui.strong(crate::i18n::tr("올가미 선택"));
            }
            Tool::Hand => {
                ui.strong(crate::i18n::tr("화면 이동"));
            }
            Tool::Picker => {
                ui.strong(crate::i18n::tr("스포이드"));
            }
        }
        if layer_changed {
            self.mark();
        }
        if let Some((t, c, at)) = pal_req {
            self.open_palette_for(t, c, at);
        }
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        if self.sel_parts().len() >= 2 {
            self.align_bar(ui);
        }
        let Some(id) = self.sel else {
            if self.sub.is_empty() {
                ui.weak(crate::i18n::tr("선택된 레이어 없음"));
            } else {
                let ns: usize = self.sub.strokes.iter().map(|x| x.1.len()).sum();
                let np = self.sub.pixels.len();
                ui.label(crate::i18n::trf("펜 획 {}개 · 브러시 영역 {}개", &[format!("{ns}"), format!("{np}")]));
                ui.small(crate::i18n::tr("끌어서 이동 · 모서리/위 동그라미: 크기·회전 · Del: 지우기"));
            }
            return;
        };
        if !self.extra_sel.is_empty() {
            ui.label(egui::RichText::new(crate::i18n::trf("{}개 선택 · 다른 값: — · 모두에 적용", &[format!("{}", self.extra_sel.len() + 1)])).color(theme::LABEL()));
        }
        if let Some(owner) = self.doc.get(id).and_then(|l| l.mask_of) {
            self.mask_props(ui, id, owner);
            return;
        }
        let Some(idx) = self.doc.index_of(id) else { return };
        // the primary as it was, to pass the panel's edits on to the rest
        let before = self.doc.layers[idx].clone();
        // values that differ across the selection show as "—"
        let (mx, mx_adj, mx_width, mxs) = {
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
            (mx, mx_adj, mx_width, mixed_style(&before, &others))
        };
        let mxc = |c: u16| mx & c != 0;
        // set on a mixed field: goes to every selected layer of the kind, even
        // when it is what the first one already has (no change there to spread)
        let mut force: Vec<Force> = Vec::new();
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
        // keys of a value before / after the playhead (the < > beside its key mark)
        let kn = |c: u16| {
            let ks = an.as_ref().map_or(&[][..], |a| &a.keys[..]);
            crate::dial::KeyNav { prev: ks.iter().any(|k| k.ch & c != 0 && k.t < tt - tol), next: ks.iter().any(|k| k.ch & c != 0 && k.t > tt + tol) }
        };
        use crate::anim::ch;
        // key buttons pressed, by value bit; key jumps asked for (-1/1), by value bit
        let mut kh = [false; ch::COUNT as usize];
        let mut kv = [0i8; ch::COUNT as usize];
        {
            let l = &mut self.doc.layers[idx];
            ui.horizontal(|ui| {
                ui.label(l.kind_label());
                // the button keeps its place; the name field takes what is left
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !matches!(l.kind, LayerKind::Audio(_)) {
                        want_mask = ui.button(crate::i18n::tr("마스크")).glass_tip(crate::i18n::tr("마스크 추가·선택")).clicked();
                    }
                    if !matches!(l.kind, LayerKind::Text(_)) {
                        let w = ui.available_width() - 8.0; // the field's own margins
                        changed |= ui.add(egui::TextEdit::singleline(&mut l.name).desired_width(w.max(24.0))).changed();
                    }
                });
            });
            if !matches!(l.kind, LayerKind::Audio(_)) {
                changed |= ui.add(crate::dial::Dial::new(&mut l.opacity, 0.0..=1.0).text(crate::i18n::tr("불투명도")).default(1.0).key(km(ch::OPA), &mut kh[3]).nav(kn(ch::OPA), &mut kv[3]).mixed(mxc(ch::OPA))).changed();
            }
            if l.place.is_none() && !matches!(l.kind, LayerKind::Fx(_)) {
                // 2D position (the centre); placed layers use the 3D fields below
                let c0 = crate::anim::key_of(l, 0.0).center;
                let (mut x, mut y) = (c0[0], c0[1]);
                let mut mv = false;
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_XS;
                    crate::dial::label_cell_nav(ui, crate::i18n::tr("위치"), km(ch::POS), &mut kh[0], Some((kn(ch::POS), &mut kv[0])));
                    let m = mxc(ch::POS);
                    let (a, b) = pair_boxes(ui, "X", pos_box(&mut x, m), "Y", pos_box(&mut y, m));
                    mv |= a.changed() || b.changed();
                });
                if mv {
                    l.translate(vec2(x - c0[0], y - c0[1]));
                    changed = true;
                }
            }
            let is_mask = l.mask_of.is_some();
            let span = l.anim.as_ref().and_then(|a| a.span);
            match &mut l.kind {
                LayerKind::Fx(fx) => {
                    changed |= fx_props(ui, fx);
                    if fx.filter == crate::doc::Filter::GradientMap {
                        gradient_bar(ui, fx.colors, id, &mut pal_req);
                    }
                    if matches!(fx.filter, crate::doc::Filter::Curves | crate::doc::Filter::ChannelAlpha) {
                        changed |= curve_editor(ui, fx, id);
                    }
                    for (i, p) in fx.filter.params().iter().enumerate() {
                        let c = ch::FX[i];
                        let b = 10 + i;
                        changed |= ui
                            .add(crate::dial::Dial::new(&mut fx.v[i], p.lo..=p.hi).step(p.step).suffix(p.suffix).text(crate::i18n::tr(p.name)).default(p.default).key(km(c), &mut kh[b]).nav(kn(c), &mut kv[b]).mixed(mxc(c)))
                            .changed();
                    }
                    if fx.filter == crate::doc::Filter::Grain {
                        changed |= crate::kit::switch(ui, &mut fx.moving, crate::i18n::tr("프레임마다 변화")).changed();
                    }
                }
                LayerKind::Audio(a) => {
                    changed |= ui.add(crate::dial::Dial::new(&mut a.volume, 0.0..=4.0).step(0.05).text(crate::i18n::tr("볼륨")).default(1.0).key(km(ch::VOL), &mut kh[9]).nav(kn(ch::VOL), &mut kv[9]).mixed(mxc(ch::VOL))).changed();
                    changed |= crate::kit::switch(ui, &mut a.mute, crate::i18n::tr("소리 끔")).changed();
                    if let Some([s0, e0]) = span {
                        ui.small(egui::RichText::new(crate::i18n::trf("{} ~ {} · 원본 {}", &[format!("{}", crate::video::timecode(s0)), format!("{}", crate::video::timecode(e0)), format!("{}", crate::video::timecode(a.len))])).monospace().color(theme::LABEL()));
                    }
                    let file = std::path::Path::new(&a.path).file_name().map_or(a.path.clone(), |f| f.to_string_lossy().to_string());
                    let w = ui.available_width();
                    ui.label(elide(ui, file, w)).glass_tip(&a.path);
                    ui.small(egui::RichText::new(crate::i18n::tr("타임라인: 막대 끌기로 이동 · 양 끝으로 자르기")).color(theme::LABEL()));
                }
                LayerKind::Image(im) => {
                    if let Some(v) = &mut im.video {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = SPACE_XS;
                            if ui.button(if is_playing { crate::i18n::tr("멈춤") } else { crate::i18n::tr("재생") }).clicked() {
                                vid_act = Some(VidAct::Play(!is_playing));
                            }
                            if ui.button(crate::i18n::tr("이전")).glass_tip(crate::i18n::tr("이전 프레임 ( , )")).clicked() {
                                vid_act = Some(VidAct::Step(-1));
                            }
                            if ui.button(crate::i18n::tr("다음")).glass_tip(crate::i18n::tr("다음 프레임 ( . )")).clicked() {
                                vid_act = Some(VidAct::Step(1));
                            }
                        });
                        // on its own line: beside the buttons it was wider than the card,
                        // and egui widened every row below with it
                        ui.label(egui::RichText::new(format!("{} / {}", crate::video::timecode(v.time), crate::video::timecode(v.info.duration))).monospace());
                        let mut t = v.time as f32;
                        let sr = ui.add(crate::dial::Dial::new(&mut t, 0.0..=v.info.duration.max(0.01) as f32).text(crate::i18n::tr("재생 위치")).suffix(" s"));
                        let t = t as f64;
                        if sr.changed() {
                            vid_act = Some(VidAct::Seek(t));
                        }
                        if sr.drag_stopped() {
                            changed = true;
                        }
                        crate::kit::row(ui, crate::i18n::tr("속도"), |ui| {
                            if crate::kit::segmented(ui, &mut v.speed, &[(0.25, "0.25×"), (0.5, "0.5×"), (1.0, "1×"), (2.0, "2×"), (4.0, "4×")]).changed() {
                                vid_act = Some(VidAct::Restart);
                                changed = true;
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            changed |= crate::kit::switch(ui, &mut v.looping, crate::i18n::tr("반복")).changed();
                            if v.info.audio {
                                changed |= crate::kit::switch(ui, &mut v.mute, crate::i18n::tr("소리 끔")).changed();
                            }
                        });
                        if v.info.audio {
                            changed |= ui.add(crate::dial::Dial::new(&mut v.volume, 0.0..=4.0).step(0.05).text(crate::i18n::tr("볼륨")).default(1.0).key(km(ch::VOL), &mut kh[9]).nav(kn(ch::VOL), &mut kv[9]).mixed(mxc(ch::VOL))).changed();
                        }
                        ui.small(format!(
                            "{}×{} · {:.2} fps · {}{}",
                            v.info.width,
                            v.info.height,
                            v.info.fps,
                            v.info.codec,
                            if v.info.audio { crate::i18n::tr(" · 소리 있음") } else { "" }
                        ));
                        if ui.button(crate::i18n::tr("이 프레임을 사진으로")).glass_tip(crate::i18n::tr("지금 프레임을 원본 해상도 사진 레이어로 복사")).clicked() {
                            vid_act = Some(VidAct::Still);
                        }
                        ui.separator();
                    }
                    ui.label(theme::info(crate::i18n::trf("원본 {}×{} px · 잘린 크기 {}×{}", &[format!("{}", im.src.size().x), format!("{}", im.src.size().y), format!("{:.0}", im.crop.width()), format!("{:.0}", im.crop.height())])));
                    let mut pct = im.scale * 100.0;
                    if ui.add(crate::dial::Dial::new(&mut pct, 1.0..=400.0).step(2.0).text(crate::i18n::tr("크기 %")).default(100.0).key(km(ch::SIZE), &mut kh[1]).nav(kn(ch::SIZE), &mut kv[1]).mixed(mxc(ch::SIZE))).changed() {
                        im.scale = pct / 100.0;
                        changed = true;
                    }
                    let deg0 = wrap_deg(im.rotation.to_degrees());
                    let mut deg = deg0;
                    if ui.add(crate::dial::Dial::new(&mut deg, -180.0..=180.0).text(crate::i18n::tr("회전 °")).default(0.0).key(km(ch::ROT), &mut kh[2]).nav(kn(ch::ROT), &mut kv[2]).mixed(mxc(ch::ROT))).changed() {
                        im.rotation += (deg - deg0).to_radians();
                        changed = true;
                    }
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::tr("원본 크기")).clicked() {
                            im.scale = 1.0;
                            changed = true;
                        }
                        if ui.button(crate::i18n::tr("회전 0°")).clicked() {
                            im.rotation = 0.0;
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::tr("자르기…")).clicked() {
                            start_crop = true;
                        }
                        if ui.button(crate::i18n::tr("자르기 해제")).clicked() {
                            let full = Rect::from_min_size(Pos2::ZERO, im.src.size());
                            let c = im.src_to_world(full.center());
                            im.crop = full;
                            im.center = c;
                            changed = true;
                        }
                    });
                    ui.label(theme::group_label(crate::i18n::tr("보정")));
                    let a = &mut im.adjust;
                    changed |= ui.add(crate::dial::Dial::new(&mut a.brightness, -1.0..=1.0).text(crate::i18n::tr("밝기")).default(0.0).mixed(mx_adj[0])).changed();
                    changed |= ui.add(crate::dial::Dial::new(&mut a.contrast, -1.0..=1.0).text(crate::i18n::tr("대비")).default(0.0).mixed(mx_adj[1])).changed();
                    changed |= ui.add(crate::dial::Dial::new(&mut a.saturation, -1.0..=1.0).text(crate::i18n::tr("채도")).default(0.0).mixed(mx_adj[2])).changed();
                    if ui.button(crate::i18n::tr("보정 초기화")).clicked() {
                        a.reset_tone();
                        changed = true;
                    }
                    // chroma key: drag the slot onto the canvas to take a colour
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_SM;
                        let mut no = false;
                        crate::dial::label_cell(ui, crate::i18n::tr("크로마 키"), None, &mut no);
                        let (sr, sresp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click_and_drag());
                        let p = ui.painter();
                        match a.chroma {
                            Some([r, g, b]) => {
                                p.rect_filled(sr, 4.0, Color32::from_rgb(r, g, b));
                            }
                            None => {
                                p.line_segment([sr.left_bottom() + vec2(4.0, -4.0), sr.right_top() + vec2(-4.0, 4.0)], Stroke::new(1.2, INK()));
                            }
                        }
                        p.rect_stroke(sr, 4.0, Stroke::new(1.0, if sresp.hovered() || sresp.dragged() { ACCENT } else { INK() }), StrokeKind::Inside);
                        if sresp.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                            chroma_drag = ui.ctx().pointer_latest_pos();
                        }
                        if sresp.drag_stopped() {
                            chroma_drop = ui.ctx().pointer_latest_pos();
                        }
                        sresp.glass_tip(crate::i18n::tr("캔버스로 끌어 색 선택"));
                        if a.chroma.is_some() && ui.small_button(crate::i18n::tr("해제")).clicked() {
                            a.chroma = None;
                            changed = true;
                        }
                    });
                    if a.chroma.is_some() {
                        changed |= ui.add(crate::dial::Dial::new(&mut a.key_tol, 0.0..=0.6).step(0.01).text(crate::i18n::tr("허용 범위")).default(0.12)).changed();
                        changed |= ui.add(crate::dial::Dial::new(&mut a.key_soft, 0.0..=0.4).step(0.01).text(crate::i18n::tr("부드럽게")).default(0.08)).changed();
                    }
                }
                LayerKind::Text(t) => {
                    let s0 = t.frame();
                    let r = ui.add(egui::TextEdit::multiline(&mut t.text).desired_rows(3).desired_width(f32::INFINITY));
                    changed |= r.changed();
                    if tool != Tool::Text {
                        changed |= ui.add(crate::dial::Dial::new(&mut t.size, 6.0..=400.0).step(2.0).text(crate::i18n::tr("크기")).key(km(ch::SIZE), &mut kh[1]).nav(kn(ch::SIZE), &mut kv[1]).mixed(mxc(ch::SIZE))).changed();
                        if font_picker_mx(ui, "prop_font", &mut t.family, &mut t.style, &mut self.font_ui, mxs.family, mxs.style) {
                            changed = true;
                            if mxs.family || mxs.style {
                                force.push(Force::Font(t.family.clone(), t.style.clone()));
                            }
                        }
                        if align_row(ui, &mut t.align, mxs.align) {
                            changed = true;
                            if mxs.align {
                                force.push(Force::Align(t.align));
                            }
                        }
                        ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                            let mut has = t.bg.is_some();
                            if crate::kit::switch_mixed(ui, &mut has, crate::i18n::tr("배경 상자"), mxs.bg_has).changed() {
                                // switched off: this layer's colour is the one that comes back
                                if let Some(old) = t.bg {
                                    self.s.text_bg_color = old;
                                }
                                // from mixed, a click turns it on for all
                                t.bg = (has || mxs.bg_has).then(|| t.bg.unwrap_or(self.s.text_bg_color));
                                changed = true;
                                if mxs.bg_has {
                                    force.push(Force::TextBg(t.bg));
                                }
                            }
                            if let Some(bg) = &mut t.bg {
                                if color_row_mx(ui, bg, PalTarget::TextBg(id), &mut pal_req, mxs.bg) {
                                    changed = true;
                                    if mxs.bg {
                                        force.push(Force::TextBg(Some(*bg)));
                                    }
                                }
                            }
                        });
                        changed |= text_look_ui(ui, &mut t.look, t.bg.is_some(), (&mut self.s.text_border_color, &mut self.s.text_stroke_color), (PalTarget::TextBorder(id), PalTarget::TextStroke(id)), &mut pal_req);
                    }
                    ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                        ui.label(crate::i18n::tr("색"));
                        if color_row_mx(ui, &mut t.color, PalTarget::TextColor(id), &mut pal_req, mxs.color) {
                            changed = true;
                            self.s.text_color = t.color;
                            if mxs.color {
                                force.push(Force::TextColor(t.color));
                            }
                        }
                    });
                    if blend_row(ui, "text_blend", &mut t.blend, mxs.blend) {
                        changed = true;
                        if mxs.blend {
                            force.push(Force::TextBlend(t.blend));
                        }
                    }
                    let deg0 = wrap_deg(t.rotation.to_degrees());
                    let mut deg = deg0;
                    if ui.add(crate::dial::Dial::new(&mut deg, -180.0..=180.0).text(crate::i18n::tr("회전 °")).default(0.0).key(km(ch::ROT), &mut kh[2]).nav(kn(ch::ROT), &mut kv[2]).mixed(mxc(ch::ROT))).changed() {
                        t.rotation += (deg - deg0).to_radians();
                        changed = true;
                    }
                    ui.horizontal(|ui| {
                        let mut wrap = t.wrap > 0.0;
                        if crate::kit::switch(ui, &mut wrap, crate::i18n::tr("줄바꿈 폭")).changed() {
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
                    ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                        ui.label(crate::i18n::tr("색"));
                        if color_row_mx(ui, &mut s.color, PalTarget::ShapeColor(id), &mut pal_req, mxs.color) {
                            changed = true;
                            if mxs.color {
                                force.push(Force::ShapeColor(s.color));
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if !shape_tool {
                            changed |= ui.add(crate::dial::Dial::new(&mut s.width, 0.0..=60.0).text(crate::i18n::tr("굵기")).mixed(mx_width)).changed();
                        }
                    });
                    if matches!(s.kind, ShapeKind::Rect | ShapeKind::Ellipse) {
                        let deg0 = wrap_deg(s.rotation.to_degrees());
                        let mut deg = deg0;
                        if ui.add(crate::dial::Dial::new(&mut deg, -180.0..=180.0).text(crate::i18n::tr("회전 °")).default(0.0).key(km(ch::ROT), &mut kh[2]).nav(kn(ch::ROT), &mut kv[2]).mixed(mxc(ch::ROT))).changed() {
                            s.rotation += (deg - deg0).to_radians();
                            changed = true;
                        }
                    }
                    if !shape_tool && matches!(s.kind, ShapeKind::Rect | ShapeKind::Ellipse) {
                        ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
                            let mut has = s.fill.is_some();
                            if crate::kit::switch_mixed(ui, &mut has, crate::i18n::tr("채우기"), mxs.fill_has).changed() {
                                // from mixed, a click turns it on for all
                                s.fill = (has || mxs.fill_has).then(|| s.fill.unwrap_or(self.s.fill_color));
                                changed = true;
                                if mxs.fill_has {
                                    force.push(Force::ShapeFill(s.fill));
                                }
                            }
                            if let Some(f) = &mut s.fill {
                                if color_row_mx(ui, f, PalTarget::ShapeFill(id), &mut pal_req, mxs.fill) {
                                    changed = true;
                                    if mxs.fill {
                                        force.push(Force::ShapeFill(Some(*f)));
                                    }
                                }
                            }
                        });
                    }
                }
                LayerKind::Vector(v) => {
                    ui.label(crate::i18n::trf("획 {}개", &[format!("{}", v.strokes.len())]));
                }
                LayerKind::Raster(r) => {
                    ui.label(crate::i18n::trf("타일 {}개", &[format!("{}", r.tiles.len())]));
                    if ui.button(crate::i18n::tr("모두 지우기")).clicked() {
                        r.tiles.clear();
                        changed = true;
                    }
                    if !is_mask && blend_row(ui, "raster_blend", &mut r.blend, mxs.blend) {
                        changed = true;
                        if mxs.blend {
                            force.push(Force::RasterBlend(r.blend));
                        }
                    }
                }
            }
            if !spaces.is_empty() && !matches!(l.kind, LayerKind::Audio(_) | LayerKind::Fx(_)) {
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
                    None => crate::i18n::tr("없음").to_string(),
                };
                crate::kit::row(ui, crate::i18n::tr("3D 공간"), |ui| {
                    // a space, or a wall in it (spaces without a camera unavailable)
                    let mut items: Vec<(Option<(u64, Option<u64>)>, String, bool)> = vec![(None, crate::i18n::tr("없음").to_string(), true)];
                    for (sid, name, ok, walls) in &spaces {
                        items.push((Some((*sid, None)), name.clone(), *ok));
                        for (wid, wn) in walls {
                            items.push((Some((*sid, Some(*wid))), format!("   {name} › {wn}"), *ok));
                        }
                    }
                    if let (Some(here), _) = crate::kit::dropdown(ui, ("space_of", id), shown, crate::kit::FILL, &items, Some(&cur)) {
                        if here != cur {
                            space_req = Some(here);
                        }
                    }
                });
                if let Some(pl) = &mut l.place {
                    for (i, ax) in ["X", "Y", "Z"].iter().enumerate() {
                        let mut v = pl.pos[i] as f32;
                        if ui.add(crate::dial::Dial::new(&mut v, -100000.0..=100000.0).step(10.0).text(crate::i18n::trf("위치 {}", &[format!("{ax}")])).default(0.0).key(km(ch::P_POS), &mut kh[4]).nav(kn(ch::P_POS), &mut kv[4]).mixed(mxc(ch::P_POS))).changed() {
                            pl.pos[i] = v as f64;
                            changed = true;
                        }
                    }
                    for (i, ax) in ["X", "Y", "Z"].iter().enumerate() {
                        let mut v = pl.rot[i] as f32;
                        if ui.add(crate::dial::Dial::new(&mut v, -180.0..=180.0).step(1.0).text(crate::i18n::trf("회전 {} °", &[format!("{ax}")])).default(0.0).key(km(ch::P_ROT), &mut kh[5]).nav(kn(ch::P_ROT), &mut kv[5]).mixed(mxc(ch::P_ROT))).changed() {
                            pl.rot[i] = v as f64;
                            changed = true;
                        }
                    }
                    let mut pct = (pl.scale * 100.0) as f32;
                    if ui.add(crate::dial::Dial::new(&mut pct, 1.0..=1000.0).step(2.0).text(crate::i18n::tr("크기 %")).default(100.0).key(km(ch::P_SCALE), &mut kh[6]).nav(kn(ch::P_SCALE), &mut kv[6]).mixed(mxc(ch::P_SCALE))).changed() {
                        pl.scale = pct as f64 / 100.0;
                        changed = true;
                    }
                    let mut th = pl.thick as f32;
                    if ui
                        .add(crate::dial::Dial::new(&mut th, 0.0..=5000.0).step(2.0).text(crate::i18n::tr("두께")).default(0.0).key(km(ch::P_THICK), &mut kh[7]).nav(kn(ch::P_THICK), &mut kv[7]).mixed(mxc(ch::P_THICK)))
                        .glass_tip(if pl.engrave { crate::i18n::tr("표면 안으로 파낸 깊이") } else { crate::i18n::tr("평면 뒤로 밀어낸 두께") })
                        .changed()
                    {
                        pl.thick = th as f64;
                        changed = true;
                    }
                    // with thickness: outline detail and smooth sides
                    if pl.thick > 0.0 {
                        let mut sg = pl.segs.clamp(1, 4) as f32;
                        if ui.add(crate::dial::Dial::new(&mut sg, 1.0..=4.0).step(1.0).integer().text(crate::i18n::tr("세그먼트")).default(1.0)).glass_tip(crate::i18n::tr("외곽선 정밀도 (배율)")).changed() {
                            pl.segs = sg.round().clamp(1.0, 4.0) as u8;
                            changed = true;
                        }
                        changed |= crate::kit::switch(ui, &mut pl.smooth, crate::i18n::tr("스무스 셰이딩")).glass_tip(crate::i18n::tr("완만한 굽이의 옆면 명암을 부드럽게 · 날카로운 모서리는 유지")).changed();
                        if crate::kit::switch_mixed(ui, &mut pl.engrave, crate::i18n::tr("음각"), mxs.engrave).glass_tip(crate::i18n::tr("두께만큼 표면 안으로 파냄")).changed() {
                            changed = true;
                            if mxs.engrave {
                                // from mixed, a click engraves them all
                                pl.engrave = true;
                                force.push(Force::Engrave(true));
                            }
                        }
                    }
                    let on_wall = pl.wall.is_some();
                    if !on_wall {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(crate::i18n::tr("공간 축에 맞추기")).glass_tip(crate::i18n::tr("가장 가까운 벽·바닥 방향으로")).clicked() {
                            pl.align_to_axes();
                            changed = true;
                        }
                        if let Some(cam) = cams.get(&pl.space) {
                            if ui.button(crate::i18n::tr("2D 모습 유지")).glass_tip(crate::i18n::tr("지금 카메라 기준으로 다시 맞춤")).clicked() {
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
                        crate::i18n::tr("벽 기준 축 · X/Y: 벽을 따라 · Z: 벽에서 떨어짐")
                    } else {
                        crate::i18n::tr("축은 넣을 때의 카메라 정면 기준 · 1단위 = 원점에서 1px")
                    });
                }
            }
        }
        // another filter: its values are other things, so keys of the old
        // filter's values go (they would drive the new one's dials)
        if let (LayerKind::Fx(b), Some(l)) = (&before.kind, self.doc.get_mut(id)) {
            if matches!(&l.kind, LayerKind::Fx(a) if a.filter != b.filter) {
                if let Some(an) = l.anim.as_mut() {
                    let fx_bits = ch::FX.iter().fold(0u16, |m, c| m | c);
                    for k in &mut an.keys {
                        k.ch &= !fx_bits;
                    }
                    an.keys.retain(|k| k.ch != 0);
                }
            }
        }
        if changed {
            self.spread_edit(&before);
            for lid in self.extra_sel.clone() {
                if let Some(l) = self.doc.get_mut(lid).filter(|l| !l.locked) {
                    for f in &force {
                        f.apply(l);
                    }
                }
            }
            self.mark();
        }
        let kc = kh.iter().enumerate().filter(|x| *x.1).fold(0u16, |a, (i, _)| a | (1 << i));
        if kc != 0 {
            for sid in self.selected_ids() {
                self.key_layer(sid, kc);
            }
        }
        // < >: the playhead jumps to that value's key before / after it
        if let Some((bit, dir)) = kv.iter().enumerate().find(|x| *x.1 != 0).map(|(i, d)| (1u16 << i, *d)) {
            let (t, tol) = (self.doc.timeline.time, self.doc.timeline.frame_len() * 0.5);
            let ks = self.doc.get(id).and_then(|l| l.anim.as_ref()).map(|a| a.keys.clone()).unwrap_or_default();
            let times = ks.iter().filter(|k| k.ch & bit != 0).map(|k| k.t);
            let to = if dir < 0 { times.filter(|&kt| kt < t - tol).reduce(f64::max) } else { times.filter(|&kt| kt > t + tol).reduce(f64::min) };
            if let Some(to) = to {
                self.set_time(to);
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
        ui.label(theme::group_label(crate::i18n::trf("마스크 · {}", &[format!("{owner_name}")])));
        let inv = self.doc.get(id).is_some_and(|l| l.mask_invert);
        ui.small(if inv { crate::i18n::tr("칠한 곳만 보임 · 지우개: 가리기") } else { crate::i18n::tr("칠한 곳 가림 · 지우개: 되살리기 · 사각형: 가리기 (Alt: 되살리기)") });
        // strength (the mask layer's opacity, keyable) and invert
        let (tl_on, tt, tol) = (self.doc.timeline.on, self.doc.timeline.time, self.doc.timeline.frame_len() * 0.5);
        let mut key_hit = false;
        let mut mask_changed = false;
        if let Some(l) = self.doc.get_mut(id) {
            let mark = tl_on.then(|| key_mark(l.anim.as_ref(), tt, tol, crate::anim::ch::OPA));
            mask_changed |= ui.add(crate::dial::Dial::new(&mut l.opacity, 0.0..=1.0).text(crate::i18n::tr("강도")).default(1.0).key(mark, &mut key_hit)).changed();
            mask_changed |= crate::kit::switch(ui, &mut l.mask_invert, crate::i18n::tr("반전")).changed();
        }
        if mask_changed {
            self.mark();
        }
        if key_hit {
            self.key_layer(id, crate::anim::ch::OPA);
        }
        let anchor = self.doc.get(id).and_then(|l| l.mask_anchor).map(|a| a.0);
        let mut pick: Option<Option<LayerId>> = None;
        crate::kit::row(ui, crate::i18n::tr("따라가기"), |ui| {
            let shown = anchor.and_then(|a| names.iter().find(|n| n.0 == a)).map_or(crate::i18n::tr("없음").to_string(), |n| n.1.clone());
            let items: Vec<(Option<LayerId>, String, bool)> = std::iter::once((None, crate::i18n::tr("없음").to_string(), true)).chain(names.iter().map(|(lid, n)| (Some(*lid), n.clone(), true))).collect();
            if let (Some(p), _) = crate::kit::dropdown(ui, ("mask_anchor", id), shown, crate::kit::FILL, &items, Some(&anchor)) {
                pick = Some(p);
            }
        });
        let mut clear = false;
        let armed = self.wand == Some(id);
        let mut toggle = false;
        let mut pick_mode: Option<u8> = None;
        // buttons flow from the left and wrap (English is longer: Ash, 10-29)
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_XS;
            for (m, label, tip) in [(0u8, crate::i18n::tr("자동 선택"), crate::i18n::tr("선택(V)·올가미(Q) 도구에서 · 클릭한 면을 가림 · Alt: 되살리기")), (1, crate::i18n::tr("그랩컷"), crate::i18n::tr("선택(V)·올가미(Q) 도구에서 · 상자로 끌기 · 안의 물체를 가림 · Alt: 되살리기")), (2, crate::i18n::tr("가위"), crate::i18n::tr("선택(V)·올가미(Q) 도구에서 · 끌기: 경계 따라 자동 · 클릭/우클릭: 앵커 · 첫 점/Enter: 닫기 · Shift: 그랩컷으로 다듬어 닫기 · Esc: 취소 · Alt: 되살리기"))] {
                if crate::kit::chip(ui, armed && self.wand_mode == m, label).glass_tip(tip).clicked() {
                    pick_mode = Some(m);
                }
            }
            clear = ui.button(crate::i18n::tr("비우기")).clicked();
        });
        if let Some(m) = pick_mode {
            if armed && self.wand_mode == m {
                toggle = true;
            } else {
                self.wand_mode = m;
                self.wand = Some(id);
                // they act with the select tools only (other tools keep their own)
                if !matches!(self.tool, Tool::Select | Tool::Lasso) {
                    self.set_tool(Tool::Select);
                }
            }
        }
        ui.add(crate::dial::Dial::new(&mut self.s.wand_tol, 0.0..=100.0).step(1.0).integer().text(crate::i18n::tr("허용 오차")).default(24.0));
        ui.add(crate::dial::Dial::new(&mut self.s.mask_refine, 0.0..=8.0).step(1.0).integer().suffix(" px").text(crate::i18n::tr("다듬기")).default(2.0))
            .glass_tip(crate::i18n::tr("경계를 그림에 맞춰 부드럽게 · 0: 딱 떨어지는 경계"));
        ui.add(crate::dial::Dial::new(&mut self.s.mask_grow, -4.0..=4.0).step(1.0).integer().suffix(" px").text(crate::i18n::tr("경계 이동")).default(1.0))
            .glass_tip(crate::i18n::tr("+: 가리는 영역 넓힘 (틈 메우기) · -: 좁힘"));
        if self.wand_mode == 2 {
            crate::kit::row(ui, crate::i18n::tr("가위 경계"), |ui| {
                crate::kit::segmented(ui, &mut self.s.scissor_soft, &[(false, crate::i18n::tr("선명")), (true, crate::i18n::tr("사진 흐림"))])
                    .glass_tip(crate::i18n::tr("선명: 경계선에서 깔끔하게 자름 (서브픽셀) · 사진 흐림: 사진 속 경계가 흐린 만큼 부드럽게"));
            });
        }
        if toggle {
            self.wand = if armed { None } else { Some(id) };
        }
        if let Some(a) = pick {
            // a photo mask given another anchor goes back to canvas pixels
            self.bake_photo_mask(id);
            let c = a.and_then(|a| self.doc.get(a)).map(|l| crate::anim::key_of(l, 0.0).center);
            if let Some(l) = self.doc.get_mut(id) {
                l.mask_anchor = a.zip(c);
                l.mask_pose = None;
            }
            self.mark();
        }
        if clear {
            self.clear_layer(id);
            self.mark();
        }
    }

    fn layers_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // icon buttons with glass tips (Ash, 10-27, Q2)
            ui.spacing_mut().item_spacing.x = SPACE_XS;
            let sel = self.sel.is_some();
            ui.add_enabled_ui(sel, |ui| {
                use crate::kit::{icon_button, Icon};
                if icon_button(ui, Icon::Up, crate::i18n::tr("위로"), false).clicked() {
                    self.move_selected(1);
                }
                if icon_button(ui, Icon::Down, crate::i18n::tr("아래로"), false).clicked() {
                    self.move_selected(-1);
                }
                if icon_button(ui, Icon::Duplicate, crate::i18n::tr("복제 · Ctrl+D"), false).clicked() {
                    self.duplicate_selected();
                }
                if icon_button(ui, Icon::Delete, crate::i18n::tr("삭제 · Backspace"), false).clicked() {
                    self.delete_selected();
                }
            });
            if crate::kit::icon_button(ui, crate::kit::Icon::Fx, crate::i18n::tr("FX 레이어 추가 · 아래 레이어에 필터"), false).clicked() {
                self.add_fx();
            }
        });
        ui.add_space(SPACE_SM);
        if self.doc.layers.is_empty() {
            ui.label(egui::RichText::new(crate::i18n::tr("비어 있음")).color(theme::LABEL()));
            return;
        }
        let mut changed = false;
        let mut select = None;
        ui.spacing_mut().item_spacing.y = SPACE_2XS;
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
            let is_sound = matches!(l.kind, LayerKind::Audio(_));
            let mask_b = (l.mask_of.is_none() && !is_sound).then(|| ui.interact(mask_r, ui.id().with(("mask", l.id)), Sense::click()));
            let p = ui.painter().clone();
            let dragging_this = self.layer_drag == Some(l.id);
            if selected || dragging_this {
                p.rect_filled(r, 8.0, FILL_2());
                p.rect_filled(Rect::from_min_size(r.min + vec2(0.0, 7.0), vec2(3.0, 16.0)), 1.5, ACCENT);
            } else if resp.hovered() {
                p.rect_filled(r, 8.0, FILL_1());
            }
            let dim = if l.visible { 1.0 } else { 0.45 };
            // state icons in ink, not grey (Ash, 10-22); hover shows as a fill
            if eye.hovered() {
                p.circle_filled(eye_r.center(), 10.0, FILL_2());
            }
            theme::eye_icon(&p, eye_r.center(), l.visible, INK());
            if l.locked {
                // locked is "on": the accent (Ash, 10-28)
                theme::lock_icon(&p, lock_r.center(), ACCENT);
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
                None if is_sound => {
                    let mute = matches!(&l.kind, LayerKind::Audio(a) if a.mute);
                    crate::canvas::sound_gizmo(&p, thumb_r.shrink(2.0), mute);
                }
                None if matches!(l.kind, LayerKind::Fx(_)) => {
                    crate::kit::fx_glyph(&p, thumb_r.center(), thumb_r.height() * 0.32, Stroke::new(1.3, INK().gamma_multiply(dim)));
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
                    theme::tool_icon(&p, glyph, thumb_r.shrink(5.0), INK().gamma_multiply(dim));
                }
            }
            if let Some(mb) = &mask_b {
                {
                    // on (has a mask) in the accent, off in ink
                    let col = if has_mask { ACCENT } else { INK() };
                    if mb.hovered() {
                        p.circle_filled(mask_r.center(), 10.0, FILL_2());
                    }
                    let c = mask_r.center();
                    p.rect_stroke(Rect::from_center_size(c, vec2(14.0, 14.0)), 3.0, Stroke::new(1.3, col), StrokeKind::Inside);
                    p.circle_filled(c, 4.0, col);
                }
                if mb.clicked() {
                    mask_click = Some(l.id);
                }
                mb.clone().glass_tip(if has_mask { crate::i18n::tr("마스크 선택") } else { crate::i18n::tr("마스크 추가") });
            }
            if l.mask_of.is_some() {
                let c = thumb_r.center();
                p.rect_filled(thumb_r, 5.0, FILL_2());
                p.rect_stroke(Rect::from_center_size(c, vec2(14.0, 14.0)), 3.0, Stroke::new(1.3, INK()), StrokeKind::Inside);
                p.circle_filled(c, 4.0, INK());
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
                let col = (if selected { INK() } else { theme::LABEL() }).gamma_multiply(dim);
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
                theme::glass(ui, egui::Margin::symmetric(14, 8), theme::radius_card(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Drifting VOID").font(FontId::new(14.0, theme::ui_bold())).color(INK()));
                        ui.add_space(SPACE_SM);
                        self.menus(ui);
                        // a newer release: a quiet line, no pop-up (Ash, 11-02)
                        if let (Some(t), tip) = self.update_text() {
                            ui.add_space(SPACE_XS);
                            let r = ui.add(egui::Label::new(egui::RichText::new(t).size(theme::TEXT_SMALL).color(ACCENT)).sense(Sense::click()));
                            let r = if tip.is_empty() { r } else { r.glass_tip(&tip) };
                            if r.clicked() {
                                self.update_click(ui.ctx());
                            }
                        }
                    });
                });
            });
    }

    /// The 파일 menu: a set width, so the rows fill it and their keys line up.
    fn file_menu_body(&mut self, ui: &mut egui::Ui) {
        ui.set_width(250.0);
        if crate::kit::menu_row(ui, crate::i18n::tr("새로 만들기"), "Ctrl+N").clicked() {
            self.request(Pending::New);
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("열기…"), "Ctrl+O").clicked() {
            self.request(Pending::Open(None));
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("저장"), "Ctrl+S").clicked() {
            self.save(false);
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("다른 이름으로 저장…"), "Ctrl+Shift+S").clicked() {
            self.save(true);
        }
        ui.separator();
        if crate::kit::menu_row(ui, crate::i18n::tr("사진 불러오기…"), "Ctrl+I").clicked() {
            self.import_dialog();
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("내보내기…"), "Ctrl+E").clicked() {
            self.open_export();
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("영상 내보내기…"), "Ctrl+Shift+E").clicked() {
            self.open_video_export();
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("영역을 클립보드로 복사"), "Ctrl+Shift+C").clicked() {
            self.quick_copy();
        }
    }

    /// The 편집 menu.
    fn edit_menu_body(&mut self, ui: &mut egui::Ui) {
        ui.set_width(180.0);
        if crate::kit::menu_row(ui, crate::i18n::tr("실행 취소"), "Ctrl+Z").clicked() {
            self.undo();
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("다시 실행"), "Ctrl+Y").clicked() {
            self.redo();
        }
        ui.separator();
        if crate::kit::menu_row(ui, crate::i18n::tr("붙여넣기"), "Ctrl+V").clicked() {
            self.paste();
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("복제"), "Ctrl+D").clicked() {
            self.duplicate_selected();
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("삭제"), "Backspace").clicked() {
            self.delete_selected();
        }
        if crate::kit::menu_row(ui, crate::i18n::tr("비우기"), "Del").clicked() {
            self.clear_selected();
        }
        ui.separator();
        if crate::kit::menu_row(ui, crate::i18n::tr("FX 레이어 추가"), "").clicked() {
            self.add_fx();
        }
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        egui::containers::menu::MenuButton::new(crate::i18n::tr("파일")).config(egui::containers::menu::MenuConfig::new().style(theme::glass_popup_style(true))).ui(ui, |ui| theme::glass_popup(ui, |ui| self.file_menu_body(ui)));
        egui::containers::menu::MenuButton::new(crate::i18n::tr("편집")).config(egui::containers::menu::MenuConfig::new().style(theme::glass_popup_style(true))).ui(ui, |ui| theme::glass_popup(ui, |ui| self.edit_menu_body(ui)));
        egui::containers::menu::MenuButton::new(crate::i18n::tr("설정")).config(egui::containers::menu::MenuConfig::new().style(theme::glass_popup_style(true)).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)).ui(ui, |ui| theme::glass_popup(ui, |ui| {
            self.view_menu_body(ui);
        }));
    }

    /// Sets the UI scale (clamped, in 5 % steps) and keeps it.
    fn set_ui_scale(&mut self, v: f32) {
        let v = ((v * 20.0).round() / 20.0).clamp(UI_SCALE.0, UI_SCALE.1);
        if (v - self.s.ui_scale).abs() > 1e-3 {
            self.s.ui_scale = v;
            self.save_prefs();
        }
        self.status = crate::i18n::trf("UI 크기 {}% · Ctrl+Alt+0: 100%", &[format!("{:.0}", v * 100.0)]);
    }

    fn view_menu_body(&mut self, ui: &mut egui::Ui) {
            ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_SM;
            let col_h = ui.vertical(|ui| {
                // a fixed column: menu rows and the separator fill it, keys at its right
                ui.set_width(168.0);
                ui.spacing_mut().item_spacing.y = SPACE_XS;
                if crate::kit::menu_row(ui, crate::i18n::tr("전체 보기"), "Ctrl+0").clicked() {
                    self.fit_view();
                    ui.close();
                }
                if crate::kit::menu_row(ui, "100%", "Ctrl+1").clicked() {
                    self.set_zoom_center(1.0);
                    ui.close();
                }
                if crate::kit::menu_row(ui, crate::i18n::tr("UI 숨기기"), "Ctrl+H").clicked() {
                    self.ui_hidden = true;
                    ui.close();
                }
                ui.separator();
                let mut on = crate::glass::enabled();
                if crate::kit::switch(ui, &mut on, crate::i18n::tr("유리 효과")).glass_tip(crate::i18n::tr("끄면 불투명한 카드")).changed() {
                    crate::glass::set_enabled(on);
                    crate::glass::save_settings();
                }
                let mut sh = crate::glass::shadow();
                if crate::kit::switch(ui, &mut sh, crate::i18n::tr("패널 그림자")).changed() {
                    crate::glass::set_shadow(sh);
                    crate::glass::save_settings();
                    theme::apply_visuals(ui.ctx());
                }
                // moved here from the tool card (Ash, 10-28)
                // each language in its own words; a submenu, as a drop-down
                // would close this menu (Ash, 10-29)
                let items: Vec<(crate::i18n::Lang, String)> = crate::i18n::Lang::ALL.iter().map(|l| (*l, l.name().to_string())).collect();
                let cur = crate::i18n::lang();
                if let (Some(l), _) = crate::kit::submenu_choice(ui, crate::i18n::tr("언어"), cur.name(), &items, &cur) {
                    crate::i18n::set_lang(l);
                    self.s.lang = Some(l.code().to_string());
                    ui.ctx().set_fonts(theme::font_defs());
                    self.egui_fonts.clear();
                    self.save_prefs();
                }
                if crate::kit::switch(ui, &mut self.s.touch_draw, crate::i18n::tr("손가락으로 그리기"))
                    .glass_tip(crate::i18n::tr("끄면: 손가락 하나 화면 이동 · 둘 확대/축소 · 펜은 항상 그리기"))
                    .changed()
                {
                    self.save_prefs();
                }
                ui.separator();
                // updates: the switch, then the row that acts (Ash, 11-02)
                let inst = crate::update::installed();
                if ui.add_enabled_ui(inst, |ui| crate::kit::switch(ui, &mut self.s.update_check, crate::i18n::tr("업데이트 확인"))).inner.glass_tip(crate::i18n::tr("시작할 때 GitHub에서 새 릴리즈 확인")).changed() {
                    self.save_prefs();
                    if self.s.update_check {
                        self.upd.check(ui.ctx());
                    }
                }
                use crate::update::State;
                let cur = crate::update::current();
                let (key, can) = match self.upd.state() {
                    _ if cur.is_empty() => (crate::i18n::tr("개발 빌드").to_string(), false),
                    _ if !inst => (crate::i18n::tr("설치본 전용").to_string(), false),
                    State::Checking => (crate::i18n::tr("확인 중…").to_string(), false),
                    State::Latest => (crate::i18n::trf("최신 {}", &[cur.clone()]), true),
                    State::Available(r) => (format!("→ {}", r.version), true),
                    State::Downloading(_, f) => (format!("{:.0}%", f * 100.0), false),
                    State::Ready(r, _) => (crate::i18n::trf("{} 설치", &[r.version]), true),
                    State::Failed(_) => (crate::i18n::tr("다시 시도").to_string(), true),
                    State::Idle => (crate::i18n::tr("지금 확인").to_string(), true),
                };
                let tip = self.update_text().1;
                let r = ui.add_enabled_ui(can, |ui| crate::kit::menu_row(ui, crate::i18n::tr("업데이트"), &key)).inner;
                let r = if tip.is_empty() { r } else { r.glass_tip(&tip) };
                if r.clicked() {
                    self.update_click(ui.ctx());
                }
            }).response.rect.height();
            ui.separator();
            if theme_switch(ui, &mut self.s.theme) {
                self.save_prefs();
            }
            ui.separator();
            ui.add_enabled_ui(crate::glass::enabled(), |ui| {
                let mut k = crate::glass::strength();
                let r = ui.add(crate::dial::Dial::new(&mut k, 0.0..=4.0).vertical().height(col_h).text(crate::i18n::tr("가장자리")).default(2.75)).glass_tip(crate::i18n::tr("유리 가장자리: 굴절 + 색 번짐 강도 · 0: 평평한 반투명"));
                if r.changed() {
                    crate::glass::set_strength(k);
                }
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    crate::glass::save_settings();
                }
                let mut bw = crate::glass::band();
                let r = ui.add(crate::dial::Dial::new(&mut bw, 1.0..=24.0).vertical().height(col_h).suffix(" pt").text(crate::i18n::tr("테두리")).default(7.0)).glass_tip(crate::i18n::tr("굴절 테두리 폭"));
                if r.changed() {
                    crate::glass::set_band(bw);
                }
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    crate::glass::save_settings();
                }
            });
            ui.add(crate::dial::Dial::new(&mut self.s.peek_delay, 0.05..=1.0).vertical().height(col_h).step(0.05).suffix(" s").text(crate::i18n::tr("미리보기")).default(0.2))
                .glass_tip(crate::i18n::tr("반대쪽 작업 공간 미리보기까지 Tab 누르는 시간"));
            let r = ui.add(crate::dial::Dial::new(&mut self.s.corner_radius, theme::RADIUS_RANGE.0..=theme::RADIUS_RANGE.1).vertical().height(col_h).step(1.0).integer().suffix(" pt").text(crate::i18n::tr("모서리")).default(theme::RADIUS_DEFAULT))
                .glass_tip(crate::i18n::tr("패널 모서리 반지름"));
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                self.save_prefs();
            }
            let r = ui.add(crate::dial::Dial::new(&mut self.s.corner_smooth, 0.0..=1.0).vertical().height(col_h).step(0.05).text(crate::i18n::tr("스무딩")).default(theme::SMOOTH_DEFAULT))
                .glass_tip(crate::i18n::tr("모서리가 직선으로 이어지는 부드러움 · 0: 원호"));
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                self.save_prefs();
            }
            // applied when the drag ends: scaling while dragging would move the
            // menu under the pointer
            let id = egui::Id::new("ui_scale_drag");
            let mut pct = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(self.s.ui_scale * 100.0);
            let r = ui.add(crate::dial::Dial::new(&mut pct, UI_SCALE.0 * 100.0..=UI_SCALE.1 * 100.0).vertical().height(col_h).step(5.0).integer().suffix("%").text(crate::i18n::tr("UI 크기")).default(100.0))
                .glass_tip(crate::i18n::tr("화면 글자와 패널 크기 · Ctrl+Alt +/−/0"));
            if r.dragged() {
                ui.data_mut(|d| d.insert_temp(id, pct));
            } else {
                ui.data_mut(|d| d.remove::<f32>(id));
                if r.changed() || r.drag_stopped() {
                    self.set_ui_scale(pct / 100.0);
                }
            }
        });
    }

    fn status_pill(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("status"))
            .order(egui::Order::Middle)
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -SPACE_LG))
            .constrain_to(screen)
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::symmetric(12, 6), theme::radius_card(), |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("−").clicked() {
                            let z = self.zoom / 1.25;
                            self.set_zoom_center(z);
                        }
                        if ui
                            .add(egui::Label::new(egui::RichText::new(format!("{:.0}%", self.zoom * 100.0)).monospace().color(INK())).sense(Sense::click()))
                            .glass_tip(crate::i18n::tr("클릭: 100% · Ctrl+0: 전체 보기"))
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
                            ui.label(egui::RichText::new(format!("{:.0}, {:.0}", w.x, w.y)).monospace().color(theme::LABEL()));
                        }
                        let pen = crate::penhook::state();
                        if pen.in_range && (pen.barrel || pen.eraser) {
                            ui.add_space(SPACE_SM);
                            let t = if pen.eraser { crate::i18n::tr("펜 지우개") } else { crate::i18n::tr("펜 버튼") };
                            ui.label(egui::RichText::new(t).color(ACCENT))
                                .glass_tip(format!("pen 0x{:X} · pointer 0x{:X}", pen.raw_pen, pen.raw_pointer));
                        }
                        if !self.status.is_empty() {
                            ui.add_space(SPACE_SM);
                            ui.label(egui::RichText::new(&self.status).color(theme::LABEL()));
                        }
                    });
                });
            });
    }

    fn section(ui: &mut egui::Ui, title: &str) {
        ui.add_space(SPACE_SM);
        ui.label(theme::section_title(title));
        ui.add_space(SPACE_XS);
    }

    /// A section title that folds its section; true while open.
    fn fold_section(ui: &mut egui::Ui, title: &str, folded: &mut bool) -> bool {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
        let col = theme::FOLD();
        let c = pos2(r.left() + 4.0, r.center().y);
        let tri = if *folded {
            vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
        } else {
            vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
        };
        ui.painter().add(Shape::convex_polygon(tri, if resp.hovered() { ACCENT } else { col }, Stroke::NONE));
        ui.painter().text(pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, title, FontId::new(theme::TEXT_LABEL, theme::ui_bold()), col);
        if resp.clicked() {
            *folded = !*folded;
        }
        if !*folded {
            ui.add_space(SPACE_XS);
        }
        !*folded
    }

    /// Right side, from the edge inwards: layers, the tool's main properties (with
    /// the selection; the region tool's cameras fold in under it).
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
                theme::glass(ui, egui::Margin::same(pad as i8), theme::radius_card(), |ui| {
                    ui.set_width(layers_w);
                    Self::section(ui, crate::i18n::tr("레이어"));
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
        // as wide as a dial row (its parts sit at fixed places)
        let main_w = crate::dial::ROW_W;
        egui::Area::new(egui::Id::new("inspector"))
            .order(egui::Order::Middle)
            .anchor(Align2::RIGHT_TOP, vec2(-x, SPACE_LG))
            .show(ctx, |ui| {
                let cams = self.tool == Tool::Frame;
                let all_folded = self.s.folded[0] && self.s.folded[1] && (!cams || self.s.folded[2]);
                // the top stays at the folded spacing, so the title never moves
                let m = egui::Margin { top: (pad * 0.5) as i8, bottom: if all_folded { (pad * 0.5) as i8 } else { pad as i8 }, ..egui::Margin::same(pad as i8) };
                theme::glass(ui, m, theme::radius_card(), |ui| {
                    ui.set_width(main_w);
                    // floating areas size from the previous frame: ask for last frame's content height
                    let want = self.col_h[0].min(max_h);
                    let out = egui::ScrollArea::vertical().id_salt("props").max_height(max_h).min_scrolled_height(want).show(ui, |ui| {
                        fixed_width(ui, |ui| {
                            if Self::fold_section(ui, crate::i18n::tr("도구"), &mut self.s.folded[0]) {
                                self.tool_options(ui);
                                ui.add_space(SPACE_MD);
                            }
                            if Self::fold_section(ui, crate::i18n::tr("선택한 항목"), &mut self.s.folded[1]) {
                                // a locked layer: its values show as they are and
                                // nothing here takes input (Ash, 11-12)
                                if self.sel.and_then(|id| self.doc.get(id)).is_some_and(|l| l.locked) {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                                        theme::lock_icon(ui.painter(), r.center(), ACCENT);
                                        ui.label(egui::RichText::new(crate::i18n::tr("잠김")).color(ACCENT));
                                    });
                                    ui.scope(|ui| {
                                        ui.visuals_mut().disabled_alpha = 1.0;
                                        ui.disable();
                                        self.properties(ui);
                                    });
                                } else {
                                    self.properties(ui);
                                }
                            }
                            // the region tool's cameras fold in here, so the
                            // number of columns never changes (Ash, 10-27)
                            if cams {
                                ui.add_space(SPACE_MD);
                                self.cameras_panel(ui);
                            }
                        });
                    });
                    self.col_h[0] = out.content_size.y;
                });
            });
        x += card(main_w) + gap;
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
        // a field of a selected layer goes to every selected layer of its kind
        // (as the panel's own edits do; locked ones keep theirs)
        let ids = |app: &Self, id: LayerId| -> Vec<LayerId> {
            if !app.is_selected(id) {
                return vec![id];
            }
            app.selected_ids().into_iter().filter(|&i| i == id || app.doc.get(i).is_some_and(|l| !l.locked)).collect()
        };
        let force = match target {
            PalTarget::Main => {
                self.s.color = c;
                self.recolor_selected();
                return;
            }
            PalTarget::TextColor(id) => {
                self.s.text_color = c;
                Some((id, Force::TextColor(c)))
            }
            PalTarget::TextBg(id) => {
                self.s.text_bg_color = c;
                Some((id, Force::TextBg(Some(c))))
            }
            PalTarget::TextBorder(id) => {
                self.s.text_border_color = c;
                Some((id, Force::TextBorder(c)))
            }
            PalTarget::TextStroke(id) => {
                self.s.text_stroke_color = c;
                Some((id, Force::TextStroke(c)))
            }
            PalTarget::ShapeColor(id) => Some((id, Force::ShapeColor(c))),
            PalTarget::ShapeFill(id) => {
                self.s.fill_color = c;
                Some((id, Force::ShapeFill(Some(c))))
            }
            PalTarget::FxColor(id, i) => {
                if let Some(LayerKind::Fx(fx)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    fx.colors[i.min(1)] = c.to_srgba_unmultiplied();
                    self.mark();
                }
                return;
            }
            PalTarget::DefaultTextColor => {
                self.s.text_color = c;
                None
            }
            PalTarget::DefaultTextBg => {
                self.s.text_bg_color = c;
                None
            }
            PalTarget::DefaultTextBorder => {
                self.s.text_border_color = c;
                if self.s.text_look.border.is_some() {
                    self.s.text_look.border = Some(c);
                }
                None
            }
            PalTarget::DefaultTextStroke => {
                self.s.text_stroke_color = c;
                if self.s.text_look.stroke.is_some() {
                    self.s.text_look.stroke = Some(c);
                }
                None
            }
            PalTarget::DefaultFill => {
                self.s.fill_color = c;
                None
            }
            PalTarget::ExportBg => {
                if let Some(d) = self.export.as_mut() {
                    d.bg_color = c;
                }
                None
            }
        };
        if let Some((id, f)) = force {
            for lid in ids(self, id) {
                if let Some(l) = self.doc.get_mut(lid) {
                    f.apply(l);
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
            let px: Vec<Color32> = (0..256).map(|i| crate::okhsv::to_rgb((i as f32 / 255.0 + hue_top()).rem_euclid(1.0), 0.9, 0.95)).collect();
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
                theme::glass(ui, egui::Margin::same(theme::SPACE_MD as i8), theme::radius_card(), |ui| ui.vertical(|ui| {
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
                                picked = Some(((((pp.y - hr.top()) / SQ).clamp(0.0, 0.999) + hue_top()).rem_euclid(1.0), p.s, p.v));
                            }
                        }
                        let hy = hr.top() + (p.h - hue_top()).rem_euclid(1.0) * SQ;
                        ui.painter().rect_stroke(Rect::from_center_size(pos2(hr.center().x, hy), vec2(20.0, 6.0)), 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Middle);
                    });
                    ui.add_space(SPACE_SM);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_XS;
                        if swatch(ui, p.orig, 22.0, false).glass_tip(crate::i18n::tr("원래 색으로")).clicked() {
                            restore = true;
                        }
                        swatch(ui, cur, 22.0, true);
                        let [r, g, b, _] = cur.to_array();
                        ui.add_space(SPACE_SM);
                        ui.label(egui::RichText::new(format!("#{r:02X}{g:02X}{b:02X}")).monospace().color(theme::LABEL()));
                    });
                    let mut near = show_near;
                    if crate::kit::switch(ui, &mut near, crate::i18n::tr("비슷한 색 지도")).changed() {
                        crate::glass::set_palette_near(near);
                        crate::glass::save_settings();
                    }
                    if !recent.is_empty() {
                        ui.add_space(SPACE_XS);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = SPACE_XS;
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
                    (crate::i18n::tr("같은 명도 · 채색"), p.value_tex.as_ref().unwrap(), GamutMap::Value, crate::i18n::tr("밝기 고정 · 가로: 색상 · 세로: 채도")),
                    (crate::i18n::tr("같은 채도 · 명암"), p.strip_tex.as_ref().unwrap(), GamutMap::Chroma, crate::i18n::tr("채도 고정 · 가로: 색상 · 세로: 밝기")),
                ];
                if show_near {
                    maps.push((crate::i18n::tr("비슷한 색"), p.patch_tex.as_ref().unwrap(), GamutMap::Near, crate::i18n::tr("색 거리가 가까운 색 · 가로: 색상 · 세로: 채도")));
                }
                for (title, view, which, tip) in maps {
                    theme::glass(ui, egui::Margin::same(10), theme::radius_card(), |ui| ui.vertical(|ui| {
                        ui.label(egui::RichText::new(title).size(theme::TEXT_SMALL).color(theme::LABEL())).glass_tip(tip);
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
            // (the palette picks the colour; the alpha typed in the hex box stays)
            let [r, g, b, _] = crate::okhsv::to_rgb(h, s, v).to_array();
            new_color = Some(Color32::from_rgba_unmultiplied(r, g, b, p.orig.to_srgba_unmultiplied()[3]));
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

        // DV_ZOOM_SWEEP / DV_PAN_SWEEP: the view zooms in and out, or pans, by
        // itself every frame (measuring the canvas without input)
        // DV_SCRUB_SWEEP: after 4 s the playhead jumps about every frame (as a
        // fast scrub), to measure the video cache
        if std::env::var_os("DV_SCRUB_SWEEP").is_some() {
            ctx.request_repaint();
            if self.now > 4.0 && self.tl_play.is_none() {
                let k = (self.now * 60.0) as u64;
                let t = ((k * 7919) % 1000) as f64 / 1000.0 * self.doc.timeline.duration;
                self.set_time(t);
            }
        }
        if std::env::var_os("DV_ZOOM_SWEEP").is_some() || std::env::var_os("DV_PAN_SWEEP").is_some() {
            ctx.request_repaint();
        }
        if std::env::var_os("DV_ZOOM_SWEEP").is_some() && self.now > 3.0 {
            let k = if ((self.now - 3.0) / 2.0) as i64 % 2 == 0 { 1.02 } else { 1.0 / 1.02 };
            self.zoom_at(rect.center(), k);
            ctx.request_repaint();
        }
        if let Some(z) = std::env::var("DV_PAN_SWEEP").ok().and_then(|v| v.parse::<f32>().ok()) {
            // DV_PAN_SWEEP=zoom: pans at that zoom
            if self.now <= 3.0 && (self.zoom - z).abs() > 1e-3 {
                self.zoom_at(rect.center(), z / self.zoom);
            }
        }
        if std::env::var_os("DV_PAN_SWEEP").is_some() && self.now > 3.0 {
            let d = if ((self.now - 3.0) / 2.0) as i64 % 2 == 0 { 6.0 } else { -6.0 };
            self.pan.x += d;
            ctx.request_repaint();
        }
        let painter = ui.painter_at(rect);
        let xf = self.xf();
        if self.cgpu.is_none() {
            theme::draw_ground(&painter, rect, &xf);
        }

        self.tex.begin_frame();
        // Crop mode: ghost of the full photo underneath.
        if self.tool == Tool::Crop && self.cgpu.is_none() {
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
        // playback cache: the canvas as framed now, rendered ahead
        let ppp = ctx.pixels_per_point();
        let view = crate::playcache::View { center: xf.s2w(rect.center()), size: rect.size() / self.zoom, scale: self.zoom * ppp };
        let target = self.cache_target(view);
        // vignette and colour fringes centre on the camera the cache films
        // (else the export frame or the content, as an export does)
        self.doc.film = match target {
            crate::playcache::Target::Camera(id) => Some(crate::doc::Film::Camera(id)),
            _ => None,
        };
        let editing = self.changed || self.drag.is_some();
        if self.doc.timeline.on && !editing {
            self.pcache.sync(self.doc_rev, &self.doc, target);
        }
        let playing = self.tl_play.is_some() && self.tl_arm.is_none();
        // around the playhead after 1.5 s without input, or at once while a
        // camera or export frame has stayed set up for 2 s (it won't move)
        let idle = !playing && !editing && self.doc.timeline.on && (self.now - self.last_input > 1.5 || self.pcache.region_settled(2.0));
        if self.doc.timeline.on {
            self.pcache.pump(&ctx, playing, idle, self.doc.timeline.time);
        }
        let t_draw = std::time::Instant::now();
        let cached = (playing && !editing).then(|| self.pcache.texture(&ctx, self.doc.timeline.time)).flatten();
        // a region (camera or export frame) while playing: where its frame sits now
        let region_quad = match target {
            crate::playcache::Target::View(_) => None,
            _ if !playing => None,
            t => crate::playcache::placement(&self.doc, t, self.pcache.size()).map(|(c, s, rot)| {
                let r = Rect::from_center_size(c, s);
                [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| xf.w2s(crate::doc::rot_about(p, c, rot)))
            }),
        };
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        match (cached, region_quad) {
            (Some(tex), Some(q)) => {
                painter.add(Shape::mesh(canvas::image_mesh(tex, q, uv, Color32::WHITE)));
            }
            (Some(tex), None) => {
                painter.image(tex, rect, uv, Color32::WHITE);
            }
            _ => match &mut self.cgpu {
                Some(cg) => {
                    // the document on the GPU renderer, over its ground
                    let t_gpu = std::time::Instant::now();
                    let (w, h) = ((rect.width() * ppp).ceil() as u32 + 1, (rect.height() * ppp).ceil() as u32 + 1);
                    let (tex, id) = cg.target(SLOT_CANVAS, w, h);
                    let ground = theme::ground_params(w as f32, h as f32, ppp, self.zoom);
                    // cropping: the whole photo faintly under the cropped one
                    let ghost = (self.tool == Tool::Crop).then(|| self.sel).flatten().and_then(|id| {
                        let i = self.doc.index_of(id)?;
                        let l = self.doc.get(id)?;
                        let LayerKind::Image(im) = &l.kind else { return None };
                        let mut g = l.clone();
                        g.id = u64::MAX - 7;
                        g.opacity = 0.3 * l.opacity;
                        g.anim = None;
                        if let LayerKind::Image(gi) = &mut g.kind {
                            gi.crop = Rect::from_min_size(Pos2::ZERO, im.src.size());
                            gi.center = im.src_to_world(gi.crop.center());
                        }
                        let mut d = self.doc.clone();
                        d.layers.insert(i, g);
                        Some(d)
                    });
                    // zooming: 3D-space layers keep the picture they have until the
                    // zoom rests, then are made sharp at the new scale
                    if (cg.zoom.0 - self.zoom).abs() > 1e-6 {
                        cg.zoom = (self.zoom, self.now);
                    }
                    let zooming = self.now - cg.zoom.1 < 0.25;
                    if zooming {
                        ctx.request_repaint_after(std::time::Duration::from_millis(260));
                    }
                    let frac = cg.gpu.render_canvas(ghost.as_ref().unwrap_or(&self.doc), xf.s2w(rect.min), self.zoom * ppp, &tex, Some(&ground), zooming, None);
                    let at = Rect::from_min_size(rect.min - frac / ppp, vec2(w as f32, h as f32) / ppp);
                    painter.image(id, at, uv, Color32::WHITE);
                    // the glass cards blur this picture
                    crate::glass_gpu::update(&cg.rs, &tex, id, [at.min.x * ppp, at.min.y * ppp]);
                    crate::glass::scene_drawn(ctx.cumulative_pass_nr());
                    // what only the editor shows
                    for l in self.doc.layers.iter().filter(|l| l.visible && self.doc.alive(l)) {
                        if let LayerKind::Audio(a) = &l.kind {
                            canvas::sound_gizmo(&painter, xf.rect_w2s(a.rect()), a.mute);
                        }
                    }
                    if std::env::var_os("DV_TIMING").is_some() {
                        let st = &mut self.canvas_stats;
                        st.1 += 1;
                        let ms = t_gpu.elapsed().as_secs_f64() * 1e3;
                        st.2 += ms;
                        st.3 = st.3.max(ms);
                        if self.now - st.0 >= 2.0 {
                            let g = crate::gpu::take_stats();
                            let n = st.1.max(1) as f64;
                            eprintln!(
                                "GPU canvas: {} frames in {:.1} s, {:.2} ms each, worst {:.1} (CPU side): prepare {:.2}, rasterize {:.2}, encode {:.2}; cached {}",
                                st.1,
                                self.now - st.0,
                                st.2 / n,
                                st.3,
                                g[0] / n,
                                g[1] / n,
                                g[2] / n,
                                self.pcache.ranges().iter().map(|(a, b)| b - a + 1).sum::<u64>()
                            );
                            let sc = &crate::vcache::SCRUBS;
                            eprintln!(
                                "video cache: {} frames, {} MB; scrubs from memory {}, by seek {}; playback cache frames from it {}",
                                crate::vcache::held(),
                                crate::vcache::bytes() >> 20,
                                sc[0].load(std::sync::atomic::Ordering::Relaxed),
                                sc[1].load(std::sync::atomic::Ordering::Relaxed),
                                sc[2].load(std::sync::atomic::Ordering::Relaxed)
                            );
                            *st = (self.now, 0, 0.0, 0.0);
                        }
                    }
                }
                None => canvas::draw_doc(&painter, &ctx, &self.doc, &xf, &mut self.tex, None),
            },
        }
        if let Some(q) = region_quad {
            // a preview monitor: what lies outside the region is dimmed while playing
            canvas_dim_outside(&painter, rect, q, Color32::from_black_alpha(190));
        }
        if playing && std::env::var_os("DV_TIMING").is_some() {
            let ms = t_draw.elapsed().as_secs_f64() * 1e3;
            let st = &mut self.cache_stats;
            if st.0 == 0.0 {
                st.0 = self.now;
            }
            if cached.is_some() {
                st.1 += 1;
                st.3 += ms;
            } else {
                st.2 += 1;
                st.4 += ms;
            }
            if self.now - st.0 >= 2.0 {
                eprintln!(
                    "canvas while playing, {:.1} s: {} cached frames ({:.2} ms each), {} live ({:.2} ms each); cache {} frames, {} MB",
                    self.now - st.0,
                    st.1,
                    st.3 / st.1.max(1) as f64,
                    st.2,
                    st.4 / st.2.max(1) as f64,
                    self.pcache.ranges().iter().map(|(a, b)| b - a + 1).sum::<u64>(),
                    self.pcache.bytes() >> 20
                );
                *st = (self.now, 0, 0, 0.0, 0.0);
            }
        }
        // scissors outline in progress
        if let Some(sc) = &mut self.scissor {
            let hover = ctx.input(|i| i.pointer.hover_pos()).map(|p| xf.s2w(p));
            // the live wire grows towards the pointer a little every frame
            if let (Some(hw), Some(wr)) = (hover, sc.wire.as_mut()) {
                wr.grow(Some(hw), SCISSOR_BUDGET);
            }
            let mut pts = sc.pts.clone();
            if let Some(hw) = hover {
                pts.extend(Self::scissor_path(sc, hw).into_iter().map(|q| q.0));
            }
            let sp: Vec<Pos2> = pts.iter().map(|p| xf.w2s(*p)).collect();
            painter.add(Shape::line(sp, Stroke::new(1.6, ACCENT)));
            painter.circle_stroke(xf.w2s(sc.pts[0]), 5.0, Stroke::new(1.5, ACCENT));
            for a in &sc.anchors[1..] {
                painter.circle_filled(xf.w2s(*a), 2.5, ACCENT);
            }
            // where a click would land (snapped to the edge)
            if let (Some(hw), Some(wr)) = (hover, sc.wire.as_ref()) {
                let s = wr.map.snap(hw, SCISSOR_SNAP_PX / xf.zoom / wr.map.cell);
                painter.circle_stroke(xf.w2s(s), 3.5, Stroke::new(1.2, INK()));
            }
            ctx.request_repaint();
        }
        // the mask being edited shows as a red veil
        if let Some(l) = self.sel.and_then(|id| self.doc.get(id)).filter(|l| l.mask_of.is_some() && l.visible) {
            if let LayerKind::Raster(r) = &l.kind {
                let view_w = Rect::from_min_max(xf.s2w(painter.clip_rect().min), xf.s2w(painter.clip_rect().max));
                // a photo mask's tiles are in its photo's pixels
                let map = if l.mask_res != 1.0 { crate::export::mask_map(&self.doc, l) } else { None };
                let veil = Color32::from_rgba_unmultiplied(255, 60, 60, 110);
                for (&key, t) in &r.tiles {
                    let tr = Rect::from_min_size(pos2((key.0 * crate::raster::TILE) as f32, (key.1 * crate::raster::TILE) as f32), vec2(crate::raster::TILE as f32, crate::raster::TILE as f32)).translate(r.offset);
                    match map {
                        Some(m) => {
                            let q = [tr.left_top(), tr.right_top(), tr.right_bottom(), tr.left_bottom()].map(|p| m.fwd(p));
                            if !Rect::from_points(&q).intersects(view_w) {
                                continue;
                            }
                            let tex = self.tex.tile(&ctx, l.id, key, t);
                            painter.add(Shape::mesh(canvas::image_mesh(tex, q.map(|p| xf.w2s(p)), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), veil)));
                        }
                        None => {
                            if !tr.intersects(view_w) {
                                continue;
                            }
                            let tex = self.tex.tile(&ctx, l.id, key, t);
                            painter.image(tex, xf.rect_w2s(tr), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), veil);
                        }
                    }
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

        // brush cursor (DV_RING_AT="x,y" pins it there, for glass captures)
        let pinned = std::env::var("DV_RING_AT").ok().and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some(pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
        });
        if let Some(hp) = pinned.or_else(|| ctx.input(|i| i.pointer.hover_pos())) {
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


        // the active tool's fill under the rail's glass: half-strength amber the
        // glass spreads (Ash, 10-15)
        if crate::glass::available() && !self.ui_hidden {
            if let Some(ir) = self.rail_indicator(&ctx) {
                painter.with_clip_rect(egui::Rect::EVERYTHING).rect_filled(ir.expand(3.0), theme::inner(theme::radius_card(), 3.0) as f32, ACCENT.gamma_multiply(0.5));
            }
        }

        // (not under the recovery question, which sits there)
        if self.doc.layers.is_empty() && self.rec_found.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                crate::i18n::tr("사진을 끌어다 놓거나 Ctrl+V로 붙여넣기"),
                FontId::new(18.0, theme::ui_bold()),
                INK_2(),
            );
        }
        let _ = resp;
    }

    /// Amber "selected tool" block in the overlay layer under the tool rail's glass.
    /// Where the active tool's fill is this frame: on its button, or gliding
    /// there (ease-out only, Ash 10-15). With glass it is drawn under the rail
    /// on the canvas overlay, which the glass lays over itself lightly blurred
    /// (`glass_gpu`); without glass, in the rail.
    fn rail_indicator(&mut self, ctx: &egui::Context) -> Option<Rect> {
        let r = self.rail_active?;
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
        Some(Rect::from_center_size(pos2(r.center().x, y), r.size()))
    }

    fn space_panel(&mut self, ui: &mut egui::Ui) {
        use crate::persp::{AXIS_COLORS, AXIS_NAMES};
        ui.strong(crate::i18n::tr("3D 공간"));
        let mut pick = None;
        let mut new = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
            for sp in &self.doc.spaces {
                let name = elide(ui, sp.name.clone(), 150.0);
                if crate::kit::chip(ui, self.space_sel == Some(sp.id) && !self.space_new, name).clicked() {
                    pick = Some(sp.id);
                }
            }
            if crate::kit::chip(ui, self.space_new, crate::i18n::tr("+ 새 공간")).clicked() {
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
            ui.small(crate::i18n::tr("캔버스에서 끌어 공간 영역 지정"));
            return;
        }
        let Some(i) = self.space_idx() else { return };
        let mut axis_pick = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
            ui.label(crate::i18n::tr("축"));
            for ax in 0..3u8 {
                let n = self.doc.spaces[i].count(ax);
                let txt = egui::RichText::new(format!("{} · {n}", AXIS_NAMES[ax as usize])).color(AXIS_COLORS[ax as usize]);
                if crate::kit::chip(ui, self.space_axis == ax, txt).clicked() {
                    axis_pick = Some(ax);
                }
            }
        });
        if let Some(ax) = axis_pick {
            self.set_space_axis(ax);
        }
        // line suggestions: candidates only; 자동 선택 is a separate, explicit step
        let shown = self.line_cands.as_ref().is_some_and(|c| c.of_space(&self.doc.spaces[i]));
        let (mut find, mut auto) = (false, false);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_XS;
            find = crate::kit::chip(ui, shown, crate::i18n::tr("선 찾기")).glass_tip(crate::i18n::tr("사진의 직선 후보 · 클릭: 현재 축에 추가")).clicked();
            if shown {
                auto = ui.button(crate::i18n::tr_ctx("선", "자동 선택")).glass_tip(crate::i18n::tr("후보에서 X·Y·Z 선 추가")).clicked();
            }
        });
        if find {
            if shown {
                self.line_cands = None;
            } else {
                self.find_lines();
            }
        }
        if auto {
            self.auto_pick_lines();
        }
        let mut floor_req: Option<f64> = None;
        let mut place_toggle: Option<(LayerId, bool)> = None;
        let (mut tape_toggle, mut tape_clear) = (false, false);
        let (mut ref_toggle, mut ref_clear) = (false, false);
        let mut ref_metres: Option<f64> = None;
        let sp = &self.doc.spaces[i];
        match crate::persp::solve(sp) {
            Ok(cam) => {
                ui.label(crate::i18n::trf("초점거리 {} mm{} · 시야각 {}°", &[format!("{:.0}", cam.focal_35mm()), format!("{}", if cam.f_assumed { crate::i18n::tr(" (가정)") } else { "" }), format!("{:.1}", cam.fov_deg())]));
                ui.small(crate::i18n::trf("35 mm 환산 · 선 오차 {}° (RMS)", &[format!("{:.2}", cam.rms_deg)]));
                // the camera is the origin (eye level = horizon); the floor is a
                // plane this far below it (world units = cm)
                let f0 = self.doc.spaces[i].floor as f32;
                let mut fl = f0;
                // a fixed range and step: a range that follows the value runs away
                let r = ui.add(crate::dial::Dial::new(&mut fl, 1.0..=100_000.0).step(1.0).text(crate::i18n::tr("바닥 깊이")));
                if r.changed() && (fl - f0).abs() > 1e-3 {
                    floor_req = Some(fl as f64);
                }
                r.glass_tip(crate::i18n::tr("카메라(눈높이)에서 바닥면까지 · cm"));
                // metres only with a reference length (a photo alone has no scale)
                let rl = self.doc.spaces[i].ref_line;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_XS;
                    if crate::kit::chip(ui, self.space_ref_draw, crate::i18n::tr("기준 길이")).glass_tip(crate::i18n::tr("수평면(바닥·탁자·창틀 등) 위 점에서 시작 · 수직: 높이, 수평: 그 면 위 길이 · 축 자동")).clicked() {
                        ref_toggle = true;
                    }
                    if let Some(mut r) = rl {
                        let before = r.metres;
                        ui.spacing_mut().interact_size.x = 56.0;
                        ui.add(crate::calc::drag(&mut r.metres).range(0.001..=100000.0).speed(0.01).max_decimals(3).suffix(" m"));
                        if r.metres != before {
                            ref_metres = Some(r.metres);
                        }
                        if ui.small_button(crate::i18n::tr("지우기")).clicked() {
                            ref_clear = true;
                        }
                    }
                });
                match rl.map(|r| r.camera_height_m(&cam)) {
                    Some(Some(m)) => {
                        ui.horizontal(|ui| {
                            ui.small(crate::i18n::trf("카메라 높이 ≈ {} m (기준면에서)", &[format!("{m:.2}")]));
                        });
                        // the floor plane onto the plane the reference stands on (cm)
                        let on_floor = (self.doc.spaces[i].floor - m * 100.0).abs() < 0.5;
                        if !on_floor && ui.button(crate::i18n::tr("기준면을 공간 바닥으로")).glass_tip(crate::i18n::tr("바닥 깊이 = 기준선이 놓인 수평면까지 (실측 cm)")).clicked() {
                            floor_req = Some(m * 100.0);
                        }
                    }
                    Some(None) => {
                        ui.small(egui::RichText::new(crate::i18n::tr("기준선이 지평선 위: 측정 불가")).color(theme::LABEL()));
                    }
                    None => {}
                }
                // AR-style placing: the selected layer stands where the floor is clicked
                // tape measure: metres against the reference length
                let tape = self.doc.spaces[i].tape;
                let tape_m = self.doc.spaces[i].tape_metres(&cam);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_XS;
                    if crate::kit::chip(ui, self.space_tape_draw, crate::i18n::tr("줄자")).glass_tip(crate::i18n::tr("기준면 위 점에서 끌기 · 수평: 거리, 수직: 높이")).clicked() {
                        tape_toggle = true;
                    }
                    if tape.is_some() {
                        match tape_m {
                            Some(m) => ui.label(format!("{m:.2} m")),
                            None => ui.label(egui::RichText::new(crate::i18n::tr("기준 길이 필요")).color(theme::LABEL())),
                        };
                        if ui.small_button(crate::i18n::tr("지우기")).clicked() {
                            tape_clear = true;
                        }
                    }
                });
                let placeable = self.sel.and_then(|id| self.doc.get(id)).filter(|l| l.mask_of.is_none() && !l.locked && !matches!(l.kind, LayerKind::Audio(_) | LayerKind::Fx(_))).map(|l| l.id);
                if let Some(lid) = placeable {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_XS;
                        for (lie, label) in [(false, crate::i18n::tr("바닥에 세우기")), (true, crate::i18n::tr("바닥에 눕히기"))] {
                            let armed = self.space_place == Some((lid, lie));
                            if crate::kit::chip(ui, armed, label).glass_tip(crate::i18n::tr("선택한 레이어 · 사진 속 바닥 클릭")).clicked() {
                                place_toggle = Some((lid, lie));
                            }
                        }
                    });
                }
            }
            Err(e) => {
                ui.small(egui::RichText::new(e).color(theme::LABEL()));
            }
        }
        let all3 = (0..3).all(|a| sp.count(a) >= 2);
        let (mut grid, mut fpp) = (sp.grid, sp.free_pp);
        let mut ch = crate::kit::switch(ui, &mut grid, crate::i18n::tr("격자")).changed();
        let mut light = sp.light;
        let mut lon = light.is_some();
        let mut lch = crate::kit::switch(ui, &mut lon, crate::i18n::tr("조명")).glass_tip(crate::i18n::tr("두께 있는 객체에 방향광 음영")).changed();
        if lon {
            let l = light.get_or_insert_with(Default::default);
            for (k, ax) in ["X", "Y", "Z"].iter().enumerate() {
                let mut d = l.rot[k] as f32;
                if ui.add(crate::dial::Dial::new(&mut d, -180.0..=180.0).step(1.0).text(crate::i18n::trf("빛 {} °", &[format!("{ax}")])).default(crate::persp::Light::default().rot[k] as f32)).changed() {
                    l.rot[k] = d as f64;
                    lch = true;
                }
            }
            lch |= ui.add(crate::dial::Dial::new(&mut l.ambient, 0.0..=1.0).text(crate::i18n::tr("어두운 면")).default(0.35)).changed();
        } else {
            light = None;
        }
        ch |= ui.add_enabled_ui(all3, |ui| crate::kit::switch(ui, &mut fpp, crate::i18n::tr("주점도 추정 (세 축 모두 2개 이상)"))).inner.changed();
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
                let label = crate::i18n::trf("사진 따라가기 · {}", &[format!("{n}")]);
                if crate::kit::switch(ui, &mut on, &label).glass_tip(crate::i18n::trf("{}\n사진 이동·크기·회전에 공간 전체가 따라감", &[format!("{n}")])).changed() {
                    unfollow = true;
                }
            });
        }
        let has_mask = sp.mask.is_some();
        let (mut mask_draw, mut mask_clear) = (false, false);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
            ui.label(if has_mask { crate::i18n::tr("마스크: 지정됨") } else { crate::i18n::tr("마스크: 공간 영역 전체") });
            mask_draw = crate::kit::chip(ui, self.space_mask_draw, crate::i18n::tr("마스크 그리기"))
                .glass_tip(crate::i18n::tr("공간이 보일 영역 (카메라 계산과 별개)"))
                .clicked();
            mask_clear = ui.add_enabled(has_mask, egui::Button::new(crate::i18n::tr("마스크 해제"))).clicked();
        });
        let walls: Vec<(u64, String)> = sp.walls.iter().map(|w| (w.id, w.name.clone())).collect();
        let (mut wall_pick, mut wall_add, mut wall_base) = (None, false, false);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
            ui.label(crate::i18n::tr("벽"));
            for (wid, name) in &walls {
                let short = elide(ui, name.clone(), 150.0);
                if crate::kit::chip(ui, self.wall_sel == Some(*wid), short).clicked() {
                    wall_pick = Some(*wid);
                }
            }
            wall_add = crate::kit::chip(ui, self.wall_new, crate::i18n::tr("+ 벽"))
                .glass_tip(crate::i18n::tr("끌어서 사각형 → 네 모서리를 사진 속 평면에 맞춤 · 넣은 레이어는 벽 좌표로 이동"))
                .clicked();
            wall_base = crate::kit::chip(ui, self.wall_base_new, crate::i18n::tr("+ 밑선 벽")).glass_tip(crate::i18n::tr("바닥 위 벽 밑선을 끌기 → 그 선에 선 수직 벽")).clicked();
        });
        let mut wall_edit: Option<(f64, bool, bool)> = None;
        if let Some(w) = self.wall_sel.and_then(|id| sp.walls.iter().find(|w| w.id == id)) {
            let (mut off, mut snap, mut wdel, mut ch2) = (w.offset as f32, w.snap_axis, false, false);
            ch2 |= ui
                .add(crate::dial::Dial::new(&mut off, -5000.0..=5000.0).step(5.0).text(crate::i18n::tr("벽 거리")).default(0.0))
                .glass_tip(crate::i18n::tr("벽 앞뒤 이동 · 바닥 경계로 잡은 위치가 어긋날 때"))
                .changed();
            ui.horizontal(|ui| {
                ch2 |= crate::kit::switch(ui, &mut snap, crate::i18n::tr("공간 축에 스냅")).glass_tip(crate::i18n::tr("공간 축과 15° 이내면 그 축으로")).changed();
                wdel = ui.button(crate::i18n::tr("벽 삭제")).clicked();
            });
            if ch2 || wdel {
                wall_edit = Some((off as f64, snap, wdel));
            }
            if let Some(f) = crate::persp::solve(sp).ok().and_then(|c| crate::persp::wall_frame(&c, w)) {
                ui.small(crate::i18n::trf("모서리 직각 오차 {}°", &[format!("{:.1}", f.skew_deg.abs())]));
            }
        }
        let (mut clear, mut fit, mut del) = (false, false, false);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
            clear = ui.button(crate::i18n::tr("선 모두 지우기")).clicked();
            fit = ui.add_enabled(img_b.is_some(), egui::Button::new(crate::i18n::tr("선택한 사진에 맞추기"))).clicked();
            del = ui.button(crate::i18n::tr("공간 삭제")).clicked();
        });
        if mask_draw {
            self.space_mask_draw = !self.space_mask_draw;
            if self.space_mask_draw {
                self.status = crate::i18n::tr("캔버스에서 끌어 마스크 지정").into();
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
                self.status = crate::i18n::tr("기준면 위 점에서 끌어 재기").into();
            }
        }
        if let Some(pick) = place_toggle {
            self.space_place = if self.space_place == Some(pick) { None } else { Some(pick) };
            if self.space_place.is_some() {
                self.space_ref_draw = false;
                self.space_mask_draw = false;
                self.status = if pick.1 { crate::i18n::tr("바닥을 클릭해 눕히기") } else { crate::i18n::tr("바닥을 클릭해 세우기") }.into();
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
                self.status = crate::i18n::tr("수평면 위 점에서 끌어 기준 길이").into();
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
                self.status = crate::i18n::tr("바닥 위 벽 밑선 끌기").into();
            }
        }
        if wall_add {
            self.wall_new = !self.wall_new;
            if self.wall_new {
                self.status = crate::i18n::tr("캔버스에서 끌어 벽 사각형 지정").into();
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
            let gone = self.doc.spaces.remove(i).id;
            // its layers come out (flat again, as 공간에서 빼기): a placement
            // in a space that is not there cannot be edited or taken out
            for l in &mut self.doc.layers {
                if l.place.as_ref().is_some_and(|p| p.space == gone) {
                    l.place = None;
                }
            }
            self.wall_sel = None;
            self.space_place = None;
            self.line_cands = None;
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
                    painter.text(b + vec2(-4.0, -4.0), Align2::RIGHT_BOTTOM, crate::i18n::tr("지평선"), FontId::proportional(11.0), hc);
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
                painter.text(mr.left_bottom() + vec2(0.0, 4.0), Align2::LEFT_TOP, crate::i18n::tr("마스크"), FontId::proportional(11.0), ACCENT);
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
            if let Some(lc) = self.line_cands.as_ref().filter(|c| c.of_space(s)) {
                // the candidate under the pointer stands out (a click takes it)
                let hp = painter.ctx().pointer_hover_pos();
                let hover = hp.and_then(|p| lc.open(s).map(|(k, g)| (k, seg_dist(p, xf.w2s(g.a), xf.w2s(g.b)))).filter(|x| x.1 <= 5.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0));
                for (k, g) in lc.open(s) {
                    let col = lc.of[k].map_or(Color32::from_white_alpha(110), |gi| CAND_COLORS[gi % CAND_COLORS.len()]);
                    let (a, b) = (xf.w2s(g.a), xf.w2s(g.b));
                    if hover == Some(k) {
                        painter.line_segment([a, b], Stroke::new(6.0, Color32::from_white_alpha(70)));
                        painter.line_segment([a, b], Stroke::new(2.5, col));
                    } else {
                        painter.line_segment([a, b], Stroke::new(1.4, col.gamma_multiply(0.8)));
                    }
                }
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
                crate::i18n::trf("내보낼 영역 {}×{}", &[format!("{:.0}", fr.width()), format!("{:.0}", fr.height())]),
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
        let content_px = (lay.size.x - lay.pad.x * 2.0) * xf.zoom;
        let width = if t.wrap > 0.0 {
            (t.wrap - lay.pad.x * 2.0) * xf.zoom
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
            .fixed_pos(r.min + pad)
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
                    .hint_text(crate::i18n::tr("글자 입력"));
                // hide egui's own caret/selection: they would be upright
                ui.visuals_mut().text_cursor.stroke.color = Color32::TRANSPARENT;
                ui.visuals_mut().selection.bg_fill = Color32::TRANSPARENT;
                let mut out = te.show(ui);
                let resp = out.response.response.clone();
                // Clicks and drags place the caret from the renderer's layout (egui's
                // own rows are rounded and never rotated): the pointer is turned back
                // into the text's frame and matched to the nearest caret spot.
                let (ptr, pressed, down) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_down()));
                let spot = ptr.map(|p| crate::doc::rot_about(p, c, -rot)).filter(|p| r.expand(4.0).contains(*p)).map(|p| lay.caret_at(pos2((p.x - r.min.x) / xf.zoom - lay.pad.x, (p.y - r.min.y) / xf.zoom - lay.pad.y)));
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
                        let ours: Vec<f32> = (0..out.galley.rows.len()).map(|i| r.min.y + (lay.pad.y + i as f32 * lay.line_h) * xf.zoom).collect();
                        let theirs: Vec<f32> = out.galley.rows.iter().map(|row| out.galley_pos.y + row.pos.y).collect();
                        eprintln!("rows ours {ours:?} editor {theirs:?} line_h {}", lay.line_h * xf.zoom);
                    }
                    // every caret spot as a tick (alignment checks without focus)
                    for &(cx, cy) in &lay.carets {
                        let p0 = r.min + vec2(lay.pad.x + cx, lay.pad.y + cy) * xf.zoom;
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
                            Rect::from_min_size(r.min + vec2(lay.pad.x + cx, lay.pad.y + cy) * xf.zoom, vec2(0.0, lay.line_h * xf.zoom))
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
                let s0 = tt.frame();
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

    /// Keeps the recovery copy in step: written soon after the first unsaved
    /// change and then at most every `RECOVERY_EVERY` seconds while there are
    /// newer ones (never mid-drag), removed once nothing is unsaved.
    fn tick_recovery(&mut self, ctx: &egui::Context) {
        let Some(r) = self.rec.as_mut() else { return };
        if !self.modified {
            r.clear();
            self.rec_rev = self.doc_rev;
            return;
        }
        if self.doc_rev == self.rec_rev || self.drag.is_some() || self.changed {
            return;
        }
        let every = std::env::var("DV_AUTOSAVE").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(RECOVERY_EVERY);
        let wait = every - (self.now - self.rec_at);
        if wait > 0.0 || r.busy() {
            // a frame when it is due, also with no input coming
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.clamp(0.25, every.max(0.25))));
            return;
        }
        if r.save(self.doc.clone(), self.path.clone()) {
            self.rec_rev = self.doc_rev;
            self.rec_at = self.now;
        }
    }

    /// At start: a copy left by a session that ended without saving. Recover
    /// it (it opens as unsaved changes of its project), keep it for later,
    /// or give it up.
    fn recovery_modal(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let Some(f) = self.rec_found.first().cloned() else { return };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("recovery")).frame(egui::Frame::NONE).backdrop_color(Color32::from_black_alpha(45)).show(ctx, |ui| theme::glass(ui, egui::Margin::same(SPACE_LG as i8), theme::radius_card(), |ui| {
            ui.set_width(300.0);
            ui.label(crate::kit::dialog_title(crate::i18n::tr("저장 안 된 작업")));
            ui.add_space(SPACE_SM);
            let name = f.name().unwrap_or_else(|| crate::i18n::tr("새 작업").to_string());
            ui.label(elide(ui, format!("{name} · {}", crate::recovery::ago(f.age)), 300.0));
            ui.add_space(SPACE_SM);
            ui.horizontal(|ui| {
                if crate::kit::primary_button(ui, crate::i18n::tr("복구")).clicked() {
                    choice = Some(0);
                }
                if ui.button(crate::i18n::tr("나중에")).clicked() {
                    choice = Some(1);
                }
                if ui.button(crate::i18n::tr("버리기")).clicked() {
                    choice = Some(2);
                }
            });
        }));
        // DV_RECOVER=1: 복구 chosen at once (checks without input)
        if std::env::var_os("DV_RECOVER").is_some() {
            choice = Some(0);
        }
        match choice {
            Some(0) => {
                self.rec_found.remove(0);
                match crate::io::load_project(&f.file) {
                    Ok(d) => {
                        self.reset_doc(d, f.origin.clone());
                        // unsaved changes of that project; the copy is this session's now
                        self.modified = true;
                        if let Some(r) = self.rec.as_mut() {
                            r.adopt(&f);
                        }
                        self.rec_rev = self.doc_rev;
                        self.rec_at = self.now;
                        self.status = crate::i18n::tr("복구됨 · 저장 필요").into();
                    }
                    // the copy stays where it is
                    Err(e) => self.status = crate::i18n::trf("복구 실패: {}", &[format!("{e}")]),
                }
            }
            // kept: offered again at the next start
            Some(1) => self.rec_found.clear(),
            Some(2) => {
                self.rec_found.remove(0);
                f.remove();
            }
            _ => {}
        }
    }

    fn unsaved_modal(&mut self, ctx: &egui::Context) {
        let Some(p) = self.pending.clone() else { return };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("unsaved")).frame(egui::Frame::NONE).backdrop_color(Color32::from_black_alpha(45)).show(ctx, |ui| theme::glass(ui, egui::Margin::same(SPACE_LG as i8), theme::radius_card(), |ui| {
            ui.set_width(300.0);
            ui.label(crate::kit::dialog_title(crate::i18n::tr("저장하지 않은 변경 사항")));
            ui.add_space(SPACE_SM);
            ui.label(crate::i18n::tr("변경 사항을 저장할까요?"));
            ui.add_space(SPACE_SM);
            ui.horizontal(|ui| {
                if crate::kit::primary_button(ui, crate::i18n::tr("저장")).clicked() {
                    choice = Some(0);
                }
                if ui.button(crate::i18n::tr("저장 안 함")).clicked() {
                    choice = Some(1);
                }
                if ui.button(crate::i18n::tr("취소")).clicked() {
                    choice = Some(2);
                }
            });
        }));
        match choice {
            Some(0) => {
                self.pending = None;
                if self.save(false) {
                    self.run_pending(p);
                } else {
                    // not saved (the file dialog cancelled, an error): nothing
                    // follows, and an update waits for the next click
                    self.update_go = None;
                }
            }
            Some(1) => {
                // the changes count as unsaved until what was asked for has
                // happened (a cancelled Open, an update that would not start)
                self.pending = None;
                self.run_pending(p);
            }
            Some(2) => {
                // cancelled: an update waits for the next click
                self.pending = None;
                self.update_go = None;
            }
            _ => {}
        }
    }

    /// The update notice, its click: download an available release, install
    /// a downloaded one (quitting through the unsaved-changes question), try
    /// again after a failure.
    fn update_click(&mut self, ctx: &egui::Context) {
        match self.upd.state() {
            crate::update::State::Available(_) => self.upd.download(ctx),
            crate::update::State::Ready(_, p) => {
                self.update_go = Some(p);
                if self.modified {
                    self.pending = Some(Pending::Quit);
                } else {
                    self.run_pending(Pending::Quit);
                }
            }
            crate::update::State::Failed(_) | crate::update::State::Latest | crate::update::State::Idle => self.upd.check(ctx),
            _ => {}
        }
    }

    /// The notice's words for this state (None: nothing to show beside the
    /// menus) and its tip.
    fn update_text(&self) -> (Option<String>, String) {
        use crate::update::State;
        let cur = crate::update::current();
        match self.upd.state() {
            State::Available(r) => (Some(crate::i18n::trf("업데이트 {} → {}", &[cur, r.version])), crate::i18n::tr("클릭: 내려받아 설치").into()),
            State::Downloading(_, f) => (Some(crate::i18n::trf("업데이트 받는 중 {}%", &[format!("{:.0}", f * 100.0)])), String::new()),
            State::Ready(r, _) => (Some(crate::i18n::trf("{} 설치", &[r.version])), crate::i18n::tr("클릭: 앱을 닫고 설치 · 끝나면 다시 열림").into()),
            State::Failed(e) => (None, e),
            _ => (None, String::new()),
        }
    }
}

/// The picker's first entry: no family named, which is what texts made
/// before the shipped default have.
const DEFAULT_FONT: &str = crate::i18n::k!("맑은 고딕 (이전 기본)");

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
    let t = TextLayer { pos: Pos2::ZERO, text: name.to_string(), size: 16.0, color: INK(), family: name.to_string(), style: style.to_string(), bg: None, wrap: 0.0, rotation: 0.0, blend: Blend::Normal, align: Default::default(), look: Default::default() };
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
    font_picker_mx(ui, id, family, style, fui, false, false)
}

/// As `font_picker`; `mixed_family` / `mixed_style`: the selected layers'
/// fonts / styles differ (shown as "—", nothing marked in the lists).
fn font_picker_mx(ui: &mut egui::Ui, id: &str, family: &mut String, style: &mut String, fui: &mut FontUi, mixed_family: bool, mixed_style: bool) -> bool {
    let filter = &mut fui.filter;
    let previews = &mut fui.previews;
    let mut changed = false;
    let fams = crate::fontdb::families();
    let shown = if family.is_empty() { crate::i18n::tr(DEFAULT_FONT).to_string() } else { family.clone() };
    crate::kit::row(ui, crate::i18n::tr("글꼴"), |ui| {
        // the closed box shows the chosen family in its own face
        let shown: egui::WidgetText = match installed_font_family(ui.ctx(), family, style) {
            _ if mixed_family => crate::dial::MIXED.into(),
            Some(fam) => egui::RichText::new(shown).family(fam).size(theme::TEXT_BODY + 1.0).into(),
            None => shown.into(),
        };
        let drag = crate::kit::dd_dragging(ui, id);
        let combo = egui::ComboBox::from_id_salt(id).truncate().selected_text(shown).width(ui.available_width()).height(360.0).popup_style(theme::glass_popup_style(false)).show_ui(ui, |ui| theme::glass_popup(ui, |ui| {
            let Some(fams) = &fams else {
                ui.label(crate::i18n::tr("설치된 글꼴 읽는 중…"));
                return;
            };
            ui.add(egui::TextEdit::singleline(filter).hint_text(crate::i18n::tr("검색")).desired_width(160.0)).request_focus();
            if ui.selectable_label(!mixed_family && family.is_empty(), crate::i18n::tr(DEFAULT_FONT)).clicked() {
                family.clear();
                *style = "Regular".into();
                changed = true;
            }
            let f = filter.to_lowercase();
            // each name is drawn in its own face; previews are built lazily, only for
            // rows on screen and a few per frame, so opening the list stays quick
            let mut budget = 4;
            let width = ui.available_width().max(220.0);
            ui.spacing_mut().item_spacing.y = SPACE_2XS;
            for fam in fams.iter().filter(|x| f.is_empty() || x.name.to_lowercase().contains(&f)) {
                let (r, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
                if !ui.is_rect_visible(r) {
                    continue;
                }
                let selected = !mixed_family && *family == fam.name;
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
                if resp.clicked() || crate::kit::dd_row_picked(ui, drag, &resp) {
                    *family = fam.name.clone();
                    *style = fam.regular().style.clone();
                    changed = true;
                    ui.close();
                }
            }
        }));
        // the wheel over the closed box: the family before / after (the
        // default first, then the installed ones in order)
        let step = crate::kit::dd_button(ui, id, &combo.response);
        if let (true, Some(fams)) = (step != 0 && !mixed_family, &fams) {
            let at = if family.is_empty() { Some(0) } else { fams.iter().position(|f| f.name == *family).map(|i| i + 1) };
            if let Some(i) = at {
                let j = i as i32 + step;
                if j == 0 {
                    family.clear();
                    *style = "Regular".into();
                    changed = true;
                } else if let Some(f) = fams.get(j as usize - 1).filter(|_| j > 0) {
                    *family = f.name.clone();
                    *style = f.regular().style.clone();
                    changed = true;
                }
            }
        }
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
        crate::kit::row(ui, crate::i18n::tr("스타일"), |ui| {
            let mx = mixed_family || mixed_style;
            let shown = if mx { crate::dial::MIXED.to_string() } else { style.clone() };
            let items: Vec<(String, String, bool)> = styles.iter().map(|s| (s.clone(), s.clone(), true)).collect();
            let now = (!mx).then(|| style.clone());
            if let (Some(s), _) = crate::kit::dropdown(ui, (id, "style"), shown, crate::kit::FILL, &items, now.as_ref()) {
                *style = s;
                changed = true;
            }
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
        LayerKind::Audio(a) => a.pos = map(a.pos),
        LayerKind::Fx(_) => {}
        // a photo mask stays on its photo's pixels (it turns and scales with the photo)
        LayerKind::Raster(_) if o.mask_res != 1.0 => {}
        LayerKind::Raster(r) => {
            if let Some(n) = crate::raster::transformed(r, c, s, th) {
                *r = n;
            }
        }
    }
    l
}

/// Seconds between recovery copies while there are unsaved changes.
const RECOVERY_EVERY: f64 = 30.0;

/// Degrees folded into -180..=180 for a rotation dial (a handle or a turn of
/// several layers leaves any angle; the dial shows the same turn, and its
/// edits go to the stored angle as a difference, so whole turns are kept).
fn wrap_deg(d: f32) -> f32 {
    let w = (d + 180.0).rem_euclid(360.0) - 180.0;
    // a half turn reads 180 from either side
    if w == -180.0 && d > 0.0 { 180.0 } else { w }
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
        theme::tool_icon(p, Tool::Space, ir, INK());
    }
    p.text(pos2(r.left() + 32.0, cy), Align2::LEFT_CENTER, name, FontId::proportional(theme::TEXT_BODY), theme::LABEL());
    let info = if solved { format!("{count}") } else { crate::i18n::tr("선 부족").to_string() };
    p.text(pos2(r.right() - 10.0, cy), Align2::RIGHT_CENTER, info, FontId::proportional(theme::TEXT_SMALL), theme::LABEL());
    resp.glass_tip(if wall { crate::i18n::tr("벽 폴더 · 레이어를 놓으면 벽에 붙음") } else { crate::i18n::tr("공간 폴더 · 레이어를 놓으면 공간에 넣음") })
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
    let names = [crate::i18n::tr("라이트"), crate::i18n::tr("다크"), crate::i18n::tr("시스템")];
    for i in 0..3u8 {
        let c = pos2(r.center().x, r.top() + slot * (i as f32 + 0.5));
        let on = *mode == i;
        if on {
            p.circle_filled(c, 10.5, FILL_3());
            p.circle_stroke(c, 10.5, Stroke::new(1.2, ACCENT));
        }
        let col = if on { ACCENT } else { INK() };
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
        resp.glass_tip(names[h]);
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
/// Polygon `pts` filled anti-aliased into a pixmap over its bounds (a pixel
/// of margin); with the bounds' integer top left.
fn poly_pixmap(pts: &[Pos2]) -> Option<(tiny_skia::Pixmap, (i32, i32))> {
    poly_pixmap_margin(pts, 1.0)
}

/// As `poly_pixmap`, with `margin` pixels round the bounds.
fn poly_pixmap_margin(pts: &[Pos2], margin: f32) -> Option<(tiny_skia::Pixmap, (i32, i32))> {
    if pts.len() < 3 {
        return None;
    }
    let b = Rect::from_points(pts);
    let b = Rect::from_min_max(b.min.floor() - vec2(margin, margin), b.max.ceil() + vec2(margin, margin));
    let (w, h) = (b.width() as u32, b.height() as u32);
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    let mut pm = tiny_skia::Pixmap::new(w, h)?;
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(pts[0].x - b.min.x, pts[0].y - b.min.y);
    for p in &pts[1..] {
        pb.line_to(p.x - b.min.x, p.y - b.min.y);
    }
    pb.close();
    let path = pb.finish()?;
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;
    pm.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
    Some((pm, (b.min.x as i32, b.min.y as i32)))
}

/// Polygon `pts` (the mask's own pixels) hidden (`on`) or shown again in
/// mask `m`, anti-aliased; the pixels touched.
fn fill_mask_poly(m: &mut crate::raster::RasterLayer, pts: &[Pos2], on: bool) -> usize {
    let Some((pm, (x0, y0))) = poly_pixmap(pts) else { return 0 };
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data();
    let mut n = 0;
    for y in 0..h {
        for x in 0..w {
            let a = data[((y * w + x) * 4 + 3) as usize] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let (wx, wy) = (x0 + x, y0 + y);
            let old = m.alpha_at_world(pos2(wx as f32 + 0.5, wy as f32 + 0.5)) as f32 / 255.0;
            let nv = if on { old.max(a) } else { old * (1.0 - a) };
            m.set_alpha_world(wx, wy, (nv * 255.0).round() as u8);
            n += 1;
        }
    }
    n
}

/// Φ, the standard normal distribution (Abramowitz–Stegun erf, |error| < 2e-7).
fn phi(x: f32) -> f32 {
    let z = x / std::f32::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * z.abs());
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * (-z * z).exp();
    0.5 * (1.0 + if z >= 0.0 { y } else { -y })
}

/// Polygon `pts` (the mask's own pixels) with the edge softness measured in
/// the photo: near the outline a pixel takes Φ(−d/σ) of its signed distance
/// d to it (σ: the edge's blur there, `sig`, with a pixel's own width
/// added), elsewhere it is in or out. Hides (`on`) or shows again; the
/// pixels touched.
fn soft_mask_poly(m: &mut crate::raster::RasterLayer, pts: &[Pos2], sig: &[f32], on: bool) -> usize {
    const BUCKET: f32 = 16.0;
    let n = pts.len();
    if n < 3 || sig.len() != n {
        return fill_mask_poly(m, pts, on);
    }
    // σ of the alpha ramp: the edge's own blur plus a pixel's width (1/12)
    let sa: Vec<f32> = sig.iter().map(|s| (s * s + 1.0 / 12.0).sqrt().min(8.0)).collect();
    let reach = sa.iter().fold(0.0f32, |a, s| a.max(*s)) * 3.0 + 1.0;
    let Some((cov, (x0, y0))) = poly_pixmap_margin(pts, reach.ceil()) else { return 0 };
    let (w, h) = (cov.width() as i32, cov.height() as i32);
    // segments by bucket (each where its ramp reaches)
    let (bw, bh) = ((w as f32 / BUCKET).ceil() as usize, (h as f32 / BUCKET).ceil() as usize);
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); bw * bh];
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let r = 3.0 * sa[i].max(sa[(i + 1) % n]) + 1.0;
        let bx0 = ((a.x.min(b.x) - r - x0 as f32) / BUCKET).floor().max(0.0) as usize;
        let by0 = ((a.y.min(b.y) - r - y0 as f32) / BUCKET).floor().max(0.0) as usize;
        let bx1 = (((a.x.max(b.x) + r - x0 as f32) / BUCKET).floor() as usize).min(bw - 1);
        let by1 = (((a.y.max(b.y) + r - y0 as f32) / BUCKET).floor() as usize).min(bh - 1);
        for by in by0..=by1 {
            for bx in bx0..=bx1 {
                buckets[by * bw + bx].push(i);
            }
        }
    }
    let data = cov.data();
    let mut count = 0;
    for y in 0..h {
        for x in 0..w {
            let c = data[((y * w + x) * 4 + 3) as usize] as f32 / 255.0;
            let p = pos2((x0 + x) as f32 + 0.5, (y0 + y) as f32 + 0.5);
            let bk = &buckets[(y as usize / BUCKET as usize) * bw + x as usize / BUCKET as usize];
            // the nearest outline point and the ramp there
            let mut best = (f32::MAX, 1.0f32);
            for &i in bk {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                let ab = b - a;
                let t = if ab.length_sq() > 1e-12 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
                let d = p.distance(a + ab * t);
                if d < best.0 {
                    best = (d, sa[i] + (sa[(i + 1) % n] - sa[i]) * t);
                }
            }
            let a = if best.0 < 3.0 * best.1 + 1.0 {
                let s = if c >= 0.5 { -best.0 } else { best.0 };
                phi(-s / best.1)
            } else {
                c
            };
            if a <= 1.0 / 510.0 {
                continue;
            }
            let (px, py) = (x0 + x, y0 + y);
            let old = m.alpha_at_world(p) as f32 / 255.0;
            let nv = if on { old.max(a) } else { old * (1.0 - a) };
            m.set_alpha_world(px, py, (nv * 255.0).round() as u8);
            count += 1;
        }
    }
    count
}

/// Values on a grid of canvas pixels (`origin`, `bw`×`bh`) written into a
/// photo mask through its map: each of its pixels takes the grid's value
/// where it lies (bilinear).
fn write_grid_mapped(m: &mut crate::raster::RasterLayer, map: crate::export::MaskMap, origin: (i32, i32), bw: usize, bh: usize, vals: &[f32], on: bool) -> usize {
    let (ox, oy) = (origin.0 as f32, origin.1 as f32);
    let corners = [pos2(ox, oy), pos2(ox + bw as f32, oy), pos2(ox + bw as f32, oy + bh as f32), pos2(ox, oy + bh as f32)].map(|p| map.inv(p));
    let b = Rect::from_points(&corners);
    let at = |x: i32, y: i32| vals[(y.clamp(0, bh as i32 - 1) as usize) * bw + x.clamp(0, bw as i32 - 1) as usize];
    let mut n = 0;
    for my in b.min.y.floor() as i32..b.max.y.ceil() as i32 {
        for mx in b.min.x.floor() as i32..b.max.x.ceil() as i32 {
            let w = map.fwd(pos2(mx as f32 + 0.5, my as f32 + 0.5));
            let (gx, gy) = (w.x - ox - 0.5, w.y - oy - 0.5);
            if gx < -0.5 || gy < -0.5 || gx > bw as f32 - 0.5 || gy > bh as f32 - 0.5 {
                continue;
            }
            let (x0, y0) = (gx.floor() as i32, gy.floor() as i32);
            let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
            let v = (at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx) * (1.0 - fy) + (at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx) * fy;
            if v < 0.02 {
                continue;
            }
            let old = m.alpha_at_world(pos2(mx as f32 + 0.5, my as f32 + 0.5)) as f32 / 255.0;
            let nv = if on { old.max(v) } else { old * (1.0 - v) };
            m.set_alpha_world(mx, my, (nv * 255.0).round() as u8);
            n += 1;
        }
    }
    n
}

pub(crate) fn sync_masks(doc: &mut Document, dragging: bool) {
    let poses: Vec<(LayerId, Pos2, f32, f32)> = doc
        .layers
        .iter()
        .filter(|l| l.mask_of.is_none())
        .map(|l| (l.id, crate::anim::key_of(l, 0.0)))
        .map(|(id, k)| (id, pos2(k.center[0], k.center[1]), k.rot, k.ext[0]))
        .collect();
    for l in &mut doc.layers {
        let Some((aid, last)) = l.mask_anchor else { continue };
        // a photo mask follows its photo through the mask map (never resampled)
        if l.mask_res != 1.0 {
            continue;
        }
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
/// Picks a camera as the export region: a segmented row for up to four
/// cameras, a drop-down beyond (Ash, 10-27: no radio dots).
fn camera_region_picker(ui: &mut egui::Ui, region: &mut ExportRegion, cams: &[crate::anim::CanvasCam]) {
    if cams.len() <= 4 {
        let opts: Vec<(ExportRegion, &str)> = cams.iter().map(|c| (ExportRegion::Camera(c.id), c.name.as_str())).collect();
        crate::kit::segmented(ui, region, &opts);
    } else {
        let cur = cams.iter().find(|c| ExportRegion::Camera(c.id) == *region).map(|c| c.name.as_str()).unwrap_or("—");
        let items: Vec<(ExportRegion, String, bool)> = cams.iter().map(|c| (ExportRegion::Camera(c.id), c.name.clone(), true)).collect();
        let now = *region;
        if let (Some(r), _) = crate::kit::dropdown(ui, "cam_region", cur, crate::kit::FILL, &items, Some(&now)) {
            *region = r;
        }
    }
}

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
    // DV_LAYOUT_LOG=1: content wider than the card (a row that does not fit
    // widens every row after it: Ash, 10-29, the Mask button pushed out)
    if std::env::var_os("DV_LAYOUT_LOG").is_some() && child.min_rect().right() > rect.right() + 0.5 {
        eprintln!("overflow: content to x {:.1}, card to {:.1} (+{:.1})", child.min_rect().right(), rect.right(), child.min_rect().right() - rect.right());
    }
    let h = child.min_rect().height();
    ui.advance_cursor_after_rect(Rect::from_min_size(rect.min, vec2(w, h)));
    out
}

/// A round loupe at `c` (radius `r`) showing texture `tex` (the window around
/// `target`, magnified `zoom` times), the line being placed, and a thin X reticle.
fn loupe_disc(painter: &egui::Painter, tex: egui::TextureId, c: Pos2, r: f32, zoom: f32, target: Pos2, line: Option<(Pos2, Pos2, Color32)>) {
    painter.circle_filled(c, r, theme::VOID());
    let mut m = egui::Mesh::with_texture(tex);
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
        ui.spacing_mut().item_spacing.x = SPACE_XS;
        if let Some(m) = key {
            *key_hit |= crate::dial::key_button(ui, m);
        }
        let (a, b) = pair_boxes(ui, "X", crate::calc::drag(&mut x).speed(1.0).max_decimals(0), "Y", crate::calc::drag(&mut y).speed(1.0).max_decimals(0));
        ch |= a.changed() || b.changed();
    });
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = SPACE_XS;
        if let Some(m) = key {
            *key_hit |= crate::dial::key_button(ui, m);
        }
        let (a, b) = pair_boxes(ui, "W", crate::calc::drag(&mut w).range(2.0..=32000.0).speed(1.0).max_decimals(0), "H", crate::calc::drag(&mut h).range(2.0..=32000.0).speed(1.0).max_decimals(0));
        ch |= a.changed() || b.changed();
    });
    // a line of its own, flowing from the left like the other buttons (the
    // English label is long; Ash, 10-29)
    {
        if ui.button(crate::i18n::tr("가로 ↔ 세로")).glass_tip(crate::i18n::tr("가운데 유지, 가로·세로 교체")).clicked() {
            let c = pos2(x + w * 0.5, y + h * 0.5);
            std::mem::swap(&mut w, &mut h);
            x = c.x - w * 0.5;
            y = c.y - h * 0.5;
            ch = true;
        }
    }
    // six sizes in one drop-down, not a grid of chips (Ash, 10-27)
    let portrait = h > w;
    let sizes: Vec<(f32, f32)> = [(1920.0f32, 1080.0f32), (1280.0, 720.0), (2560.0, 1440.0), (3840.0, 2160.0), (1080.0, 1080.0), (1080.0, 1350.0)]
        .iter()
        .map(|&(pw, ph)| if portrait && pw != ph { (ph.min(pw), ph.max(pw)) } else { (pw, ph) })
        .collect();
    let cur = sizes.iter().find(|(pw, ph)| (w - pw).abs() < 0.5 && (h - ph).abs() < 0.5).map(|(pw, ph)| format!("{pw:.0}×{ph:.0}")).unwrap_or_else(|| crate::i18n::tr("직접").into());
    let mut pick = None;
    crate::kit::row(ui, crate::i18n::tr("크기 프리셋"), |ui| {
        let items: Vec<(usize, String, bool)> = sizes.iter().enumerate().map(|(i, (pw, ph))| (i, format!("{pw:.0}×{ph:.0}"), true)).collect();
        let now = sizes.iter().position(|(pw, ph)| (w - pw).abs() < 0.5 && (h - ph).abs() < 0.5);
        let (p, resp) = crate::kit::dropdown(ui, (salt, "size_preset"), cur, crate::kit::FILL, &items, now.as_ref());
        resp.glass_tip(crate::i18n::tr("가운데 유지"));
        pick = p.map(|i| sizes[i]);
    });
    if let Some((pw, ph)) = pick {
        let c = pos2(x + w * 0.5, y + h * 0.5);
        w = pw;
        h = ph;
        x = c.x - w * 0.5;
        y = c.y - h * 0.5;
        ch = true;
    }
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

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.now = ctx.input(|i| i.time);
        crate::glass::begin_pass(ctx.cumulative_pass_nr());
        if ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving() || i.pointer.any_down()) {
            self.last_input = self.now;
        }
        self.drop_stale_ime(&ctx);
        // DV_FULLSCREEN=1: full screen from the first frames (manual screenshots;
        // the start-up option alone is not honoured on Windows)
        if std::env::var("DV_FULLSCREEN").is_ok_and(|v| v == "1") {
            if ctx.cumulative_pass_nr() < 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
            // the document's view was made for the smaller window: fit it once grown
            if (2.0..3.0).contains(&self.now) {
                self.fit_view();
            }
            if self.now < 3.0 {
                ctx.request_repaint();
            }
        }
        // DV_UI_SCALE=1.25: start at that UI scale (captures; not saved)
        if ctx.cumulative_pass_nr() == 0 {
            if let Some(v) = std::env::var("DV_UI_SCALE").ok().and_then(|v| v.parse::<f32>().ok()) {
                self.s.ui_scale = v;
            }
        }
        let z = self.s.ui_scale.clamp(UI_SCALE.0, UI_SCALE.1);
        if (ctx.zoom_factor() - z).abs() > 1e-3 {
            ctx.set_zoom_factor(z);
        }
        if std::env::var("DV_KIT").is_ok_and(|v| v == "1") {
            crate::kit::demo(&ctx);
        }

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
        // drops through our OLE target (oledrop): files, browser images, URLs, text
        if std::env::var_os("DV_DROP_CLIPBOARD").is_some() && self.frames == 30 {
            crate::oledrop::from_clipboard(frame);
        }
        // DV_PASTE=1: Ctrl+V once (URL paste captures)
        if std::env::var_os("DV_PASTE").is_some() && self.frames == 30 {
            self.paste();
        }
        for (d, at) in crate::oledrop::take() {
            let ppp = ctx.pixels_per_point();
            let at = at.map(|p| pos2(p[0] / ppp, p[1] / ppp)).filter(|p| self.canvas_rect.contains(*p)).map(|p| self.xf().s2w(p)).unwrap_or_else(|| self.view_center_world());
            self.take_drop(d, at);
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_prefs();
        }
        // a video being exported would be cut off: it is cancelled first
        if ctx.input(|i| i.viewport().close_requested()) && self.vjob.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = crate::i18n::tr("영상 내보내는 중 · 취소 후 닫기").into();
            if self.vexport.is_none() {
                self.open_video_export();
            }
        }
        // every close request while there are unsaved changes is held back and
        // asks (a second one too: the question is still up, or one for New / Open)
        if ctx.input(|i| i.viewport().close_requested()) && self.modified && self.vjob.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }

        let light = match self.s.theme {
            0 => true,
            1 => false,
            _ => ctx.system_theme() == Some(egui::Theme::Light),
        };
        let corners = theme::set_corners(self.s.corner_radius, self.s.corner_smooth);
        if light != theme::is_light() || corners {
            theme::set_light(light);
            theme::apply_visuals(&ctx);
        }
        self.handle_shortcuts(&ctx);
        crate::persp::follow_photos(&mut self.doc);
        crate::persp::sync_walls(&mut self.doc);
        sync_masks(&mut self.doc, self.drag.is_some() || self.tl_play.is_some());
        // a mask's 자동 선택 / 그랩컷 / 가위 end when the mask is no longer the
        // selected layer (another layer picked, the mask deleted): the select
        // tools then work on layers again (Ash, 10-23)
        if self.wand.is_some_and(|mid| self.sel != Some(mid) || self.doc.get(mid).is_none()) {
            self.wand = None;
        }
        if self.scissor.as_ref().is_some_and(|sc| self.wand != Some(sc.mid)) {
            self.scissor = None;
        }
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
                    theme::glass(ui, egui::Margin::same(theme::SPACE_MD as i8), theme::radius_card(), |ui| {
                        ui.set_width(330.0);
                        ui.label(theme::section_title(crate::i18n::tr("입력 기록 · F12로 닫기")));
                        let pen = crate::penhook::state();
                        ui.label(
                            egui::RichText::new(crate::i18n::trf(
                                "펜 범위 {} · 버튼 {} · 지우개 {} · flags {}/{}",
                                &[format!("{}", pen.in_range), format!("{}", pen.barrel), format!("{}", pen.eraser), format!("{:X}", pen.raw_pen), format!("{:X}", pen.raw_pointer)],
                            ))
                            .monospace()
                            .color(theme::LABEL()),
                        );
                        for line in &self.input_log {
                            ui.label(egui::RichText::new(line).monospace().color(theme::LABEL()));
                        }
                    });
                });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.doc.timeline.on && !self.ui_hidden {
            self.timeline_ui(&ctx);
        }
        // DV_SHOW_MENU=file|edit: that menu's contents in a card (as DV_SHOW_VIEW)
        if let Ok(m) = std::env::var("DV_SHOW_MENU") {
            egui::Area::new(egui::Id::new("dbg_menu")).order(egui::Order::Foreground).fixed_pos(pos2(40.0, 110.0)).show(&ctx, |ui| {
                theme::glass(ui, egui::Margin::same(8), theme::radius_card(), |ui| if m == "edit" { self.edit_menu_body(ui) } else { self.file_menu_body(ui) });
            });
        }
        if std::env::var_os("DV_SHOW_VIEW").is_some() {
            // the View menu's contents in a card (screenshots; menus cannot be opened from code)
            egui::Area::new(egui::Id::new("dbg_view")).order(egui::Order::Foreground).fixed_pos(pos2(40.0, 110.0)).show(&ctx, |ui| {
                theme::glass(ui, egui::Margin::same(8), theme::radius_card(), |ui| self.view_menu_body(ui));
            });
        }
        self.export_ui(&ctx);
        self.video_export_ui(&ctx);
        self.unsaved_modal(&ctx);
        self.recovery_modal(&ctx);
        self.tick_recovery(&ctx);
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

        let name = self.path.as_ref().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or(crate::i18n::tr("제목 없음"));
        let title = format!("{}{} — Drifting VOID", if self.modified { "● " } else { "" }, name);
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        self.frames += 1;
        // a release build asks for a newer release once, after the first frames
        if self.frames == 30 && self.s.update_check {
            self.upd.check(&ctx);
        }
        // DV_UPDATE_AUTO=1: an available release downloads by itself (the
        // download and checksum tested without input; with DV_UPDATE_NOLAUNCH)
        if std::env::var_os("DV_UPDATE_AUTO").is_some() && matches!(self.upd.state(), crate::update::State::Available(_)) {
            self.upd.download(&ctx);
        }
        if self.frames % 90 == 0 {
            self.save_prefs();
        }
        if self.drag.is_some() || crate::penhook::state().barrel || self.frames < 6 {
            ctx.request_repaint();
        }
        // last: what the glass cards show through them (all layers are laid out)
        if let Some(rs) = frame.wgpu_render_state() {
            crate::glass_gpu::backdrop(&ctx, rs);
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
/// The 정렬 buttons; `mixed`: the selected texts' alignments differ (none lit).
/// A text box's shape and border (with the box on) and the letters' outline,
/// for a text layer or the new-text defaults. `mem`: the last border and
/// outline colours, taken when one is switched on again.
fn text_look_ui(ui: &mut egui::Ui, look: &mut crate::doc::TextLook, has_box: bool, mem: (&mut Color32, &mut Color32), targets: (PalTarget, PalTarget), req: &mut PalReq) -> bool {
    let d = crate::doc::TextLook::default();
    // fractions of the em, shown in per cent
    let pct = |ui: &mut egui::Ui, v: &mut f32, max: f32, text: &str, def: f32| {
        let mut p = *v * 100.0;
        let ch = ui.add(crate::dial::Dial::new(&mut p, 0.0..=max).step(1.0).suffix("%").text(text).default(def * 100.0)).changed();
        if ch {
            *v = p / 100.0;
        }
        ch
    };
    let on_color = |ui: &mut egui::Ui, c: &mut Option<Color32>, mem: &mut Color32, text: &str, target: PalTarget, req: &mut PalReq| {
        let mut ch = false;
        ui.horizontal_wrapped(|ui| { // colour row: wraps instead of overflowing the card
            let mut on = c.is_some();
            if crate::kit::switch(ui, &mut on, text).changed() {
                // switched off: this layer's colour is the one that comes back
                if let Some(old) = *c {
                    *mem = old;
                }
                *c = on.then_some(*mem);
                ch = true;
            }
            if let Some(col) = c {
                if color_row(ui, col, target, req) {
                    *mem = *col;
                    ch = true;
                }
            }
        });
        ch
    };
    let mut changed = false;
    if has_box {
        changed |= pct(ui, &mut look.pad_x, 200.0, crate::i18n::tr("가로 여백"), d.pad_x);
        changed |= pct(ui, &mut look.pad_y, 200.0, crate::i18n::tr("세로 여백"), d.pad_y);
        changed |= pct(ui, &mut look.radius, 100.0, crate::i18n::tr("모서리"), d.radius);
        changed |= on_color(ui, &mut look.border, mem.0, crate::i18n::tr("상자 테두리"), targets.0, req);
        if look.border.is_some() {
            changed |= pct(ui, &mut look.border_w, 50.0, crate::i18n::tr("테두리 굵기"), d.border_w);
        }
    }
    changed |= on_color(ui, &mut look.stroke, mem.1, crate::i18n::tr("글자 외곽선"), targets.1, req);
    if look.stroke.is_some() {
        changed |= pct(ui, &mut look.stroke_w, 50.0, crate::i18n::tr("외곽선 굵기"), d.stroke_w);
    }
    changed
}

fn align_row(ui: &mut egui::Ui, a: &mut crate::doc::TextAlign, mixed: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2XS;
        let mut no = false;
        crate::dial::label_cell(ui, crate::i18n::tr("정렬"), None, &mut no);
        for al in crate::doc::TextAlign::ALL {
            let on = !mixed && *a == al;
            let (r, resp) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
            let p = ui.painter();
            if on || resp.hovered() {
                p.rect_filled(r, 5.0, if on { ACCENT.gamma_multiply(0.25) } else { FILL_1() });
            }
            let col = if on { ACCENT } else { INK() };
            // four lines, long and short, flush to the side
            let (x0, x1) = (r.left() + 6.0, r.right() - 6.0);
            for (i, long) in [true, false, true, false].into_iter().enumerate() {
                let w = if long { x1 - x0 } else { (x1 - x0) * 0.6 };
                let y = r.top() + 6.0 + i as f32 * 3.4;
                let left = x0 + (x1 - x0 - w) * al.factor();
                p.line_segment([pos2(left, y), pos2(left + w, y)], Stroke::new(1.4, col));
            }
            if resp.glass_tip(al.label()).clicked() && !on {
                *a = al;
                changed = true;
            }
        }
    });
    changed
}

/// A curve FX layer's curves as points on a graph (11-02). 커브: the channel
/// picked above it (RGB, red, green, blue); 채널을 알파로: one curve, the
/// row picks the channel made the alpha (R, G, B, L). A click adds a point,
/// a drag moves one (between its neighbours), dragging one off the graph or
/// double-clicking it removes it (two always stay). A right click on a
/// point sets how its left, right or both sides meet their segments
/// (Constant, Linear, Free or Auto; Ash 11-02); a Free side has a handle on the
/// picked point.
fn curve_editor(ui: &mut egui::Ui, fx: &mut crate::doc::FxLayer, id: LayerId) -> bool {
    use crate::doc::{CurvePt, Tan, curve_identity, curve_slope};
    let mut changed = false;
    let alpha = fx.filter == crate::doc::Filter::ChannelAlpha;
    let ch_id = egui::Id::new(("fx_curve_ch", id));
    let drag_id = egui::Id::new(("fx_curve_drag", id));
    let sel_id = egui::Id::new(("fx_curve_sel", id));
    let menu_id = egui::Id::new(("fx_curve_menu", id));
    let mut ch: usize = if alpha { 0 } else { ui.data(|d| d.get_temp(ch_id)).unwrap_or(0) };
    crate::kit::row(ui, crate::i18n::tr("채널"), |ui| {
        // the channels fill the row up to the reset button (segmented takes
        // all the room it is given)
        let label = crate::i18n::tr("초기화");
        let bw = ui.painter().layout_no_wrap(label.to_owned(), egui::TextStyle::Button.resolve(ui.style()), INK()).size().x + ui.spacing().button_padding.x * 2.0;
        let seg_w = (ui.available_width() - bw - SPACE_XS).max(80.0);
        ui.allocate_ui_with_layout(vec2(seg_w, theme::ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if alpha {
                let was = fx.source;
                crate::kit::segmented(ui, &mut fx.source, &[(0, "R"), (1, "G"), (2, "B"), (3, "L")]);
                changed |= fx.source != was;
            } else {
                crate::kit::segmented(ui, &mut ch, &[(0, "RGB"), (1, "R"), (2, "G"), (3, "B")]);
            }
        });
        if ui.add_enabled(fx.curves[ch] != curve_identity(), egui::Button::new(crate::i18n::tr("초기화"))).clicked() {
            fx.curves[ch] = curve_identity();
            changed = true;
        }
    });
    if !alpha {
        ui.data_mut(|d| d.insert_temp(ch_id, ch));
    }
    let tint = |c: usize| match c {
        1 => Color32::from_rgb(235, 85, 80),
        2 => Color32::from_rgb(90, 200, 110),
        3 => Color32::from_rgb(90, 140, 245),
        _ => INK(),
    };
    // the alpha's curve in the colour of its channel (L: ink)
    let ink = if alpha { tint((fx.source as usize + 1) % 4) } else { tint(ch) };
    let w = ui.available_width();
    let (g, resp) = ui.allocate_exact_size(vec2(w, (w * 0.75).round()), Sense::click_and_drag());
    let g = g.shrink(4.0);
    let to_s = |p: &CurvePt| pos2(g.left() + p.x * g.width(), g.bottom() - p.y * g.height());
    let to_c = |s: Pos2| [((s.x - g.left()) / g.width()).clamp(0.0, 1.0), ((g.bottom() - s.y) / g.height()).clamp(0.0, 1.0)];
    let near = |pts: &[CurvePt], s: Pos2| pts.iter().enumerate().map(|(i, p)| (i, to_s(p).distance(s))).filter(|(_, d)| *d <= 9.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i);
    // a Free side's handle: along its slope, a set length on screen
    let handle = |pts: &[CurvePt], i: usize, right: bool| {
        let d = vec2(g.width(), -curve_slope(pts, i, right) * g.height()).normalized() * 30.0;
        to_s(&pts[i]) + if right { d } else { -d }
    };
    let pts = &mut fx.curves[ch];
    // the picked point (its Free handles show), per channel
    let mut sel: Option<usize> = ui.data(|d| d.get_temp::<Option<(usize, usize)>>(sel_id)).flatten().filter(|(c, i)| *c == ch && *i < pts.len()).map(|(_, i)| i);
    let handles = |pts: &[CurvePt], sel: Option<usize>| -> Vec<(usize, bool, Pos2)> {
        let Some(i) = sel else { return Vec::new() };
        let p = pts[i];
        [(false, p.l), (true, p.r)].into_iter().filter(|(_, t)| *t == Tan::Free).map(|(r, _)| (i, r, handle(pts, i, r))).collect()
    };
    // DV_CURVE_SEL=i: that point picked; with DV_CURVE_MENU its menu open (captures)
    let dbg = std::env::var("DV_CURVE_SEL").ok().and_then(|v| v.parse::<usize>().ok()).filter(|i| *i < pts.len());
    if let Some(i) = dbg {
        sel = sel.or(Some(i));
        if std::env::var_os("DV_CURVE_MENU").is_some() {
            ui.data_mut(|d| d.insert_temp(menu_id.with("pt"), Some((ch, i))));
        }
    }
    let on_handle = |pts: &[CurvePt], sel: Option<usize>, s: Pos2| handles(pts, sel).into_iter().find(|h| h.2.distance(s) <= 7.0).map(|h| (h.0, h.1));
    // what is dragged: a point (i, None) or a handle (i, Some(right))
    let mut drag: Option<(usize, Option<bool>)> = ui.data(|d| d.get_temp(drag_id)).flatten();
    if resp.drag_started_by(egui::PointerButton::Primary) {
        let at = ui.input(|i| i.pointer.press_origin()).unwrap_or(g.center());
        drag = on_handle(pts, sel, at).map(|(i, r)| (i, Some(r))).or_else(|| {
            near(pts, at).or_else(|| {
                // a new point where the press was
                let p = to_c(at);
                let i = pts.iter().position(|q| q.x > p[0]).unwrap_or(pts.len());
                let lo = if i > 0 { pts[i - 1].x + 0.01 } else { 0.0 };
                let hi = if i < pts.len() { pts[i].x - 0.01 } else { 1.0 };
                (lo <= hi).then(|| {
                    pts.insert(i, CurvePt::from([p[0].clamp(lo, hi), p[1]]));
                    changed = true;
                    i
                })
            })
            .map(|i| (i, None))
        });
        if let Some((i, _)) = drag {
            sel = Some(i);
        }
    }
    if let (true, Some((i, h)), Some(s)) = (resp.dragged_by(egui::PointerButton::Primary), drag, resp.interact_pointer_pos()) {
        if i < pts.len() {
            if let Some(right) = h {
                // the handle sets its side's slope; it stays on its side
                let dv = s - to_s(&pts[i]);
                let (dx, dy) = (dv.x / g.width(), -dv.y / g.height());
                let dx = if right { dx.max(1e-3) } else { dx.min(-1e-3) };
                let m = (dy / dx).clamp(-50.0, 50.0);
                if right {
                    pts[i].rs = m;
                } else {
                    pts[i].ls = m;
                }
            } else if !g.expand(24.0).contains(s) && pts.len() > 2 {
                // dragged off the graph: gone
                pts.remove(i);
                drag = None;
                sel = None;
            } else {
                let p = to_c(s);
                let lo = if i > 0 { pts[i - 1].x + 0.01 } else { 0.0 };
                let hi = if i + 1 < pts.len() { pts[i + 1].x - 0.01 } else { 1.0 };
                pts[i].x = p[0].clamp(lo, hi.max(lo));
                pts[i].y = p[1];
            }
            changed = true;
        }
    }
    if resp.drag_stopped() {
        drag = None;
    }
    if resp.double_clicked() && pts.len() > 2 {
        if let Some(i) = resp.interact_pointer_pos().and_then(|s| near(pts, s)) {
            pts.remove(i);
            sel = None;
            changed = true;
        }
    } else if resp.clicked() {
        // a click on a point picks it; away from the points (and handles)
        // it puts one there
        let s = resp.interact_pointer_pos();
        if let Some(i) = s.and_then(|s| near(pts, s)) {
            sel = Some(i);
        } else if let Some(s) = s.filter(|s| on_handle(pts, sel, *s).is_none()) {
            let p = to_c(s);
            let i = pts.iter().position(|q| q.x > p[0]).unwrap_or(pts.len());
            let lo = if i > 0 { pts[i - 1].x + 0.01 } else { 0.0 };
            let hi = if i < pts.len() { pts[i].x - 0.01 } else { 1.0 };
            if lo <= hi {
                pts.insert(i, CurvePt::from([p[0].clamp(lo, hi), p[1]]));
                sel = Some(i);
                changed = true;
            }
        }
    }
    // a right click on a point: its tangent menu (elsewhere none)
    if resp.secondary_clicked() {
        let t = resp.interact_pointer_pos().and_then(|s| near(pts, s));
        ui.data_mut(|d| d.insert_temp(menu_id.with("pt"), t.map(|i| (ch, i))));
        ui.data_mut(|d| d.remove::<Option<usize>>(menu_id.with("side")));
        if t.is_some() {
            sel = t;
        }
    }
    let target = ui.data(|d| d.get_temp::<Option<(usize, usize)>>(menu_id.with("pt"))).flatten().filter(|(c, i)| *c == ch && *i < pts.len()).map(|(_, i)| i);
    let open = if (resp.secondary_clicked() || dbg.is_some() && std::env::var_os("DV_CURVE_MENU").is_some()) && target.is_some() {
        Some(egui::containers::SetOpenCommand::Bool(true))
    } else if resp.clicked() || resp.drag_started() || target.is_none() {
        Some(egui::containers::SetOpenCommand::Bool(false))
    } else {
        None
    };
    // beside the pin, not the pointer (Ash, 11-02): right of it, else left,
    // else above or below, whichever fits on the screen
    let pin = target.map_or(g.center(), |i| to_s(&pts[i]));
    const SIDES: [egui::RectAlign; 5] = [egui::RectAlign::LEFT_START, egui::RectAlign::RIGHT_END, egui::RectAlign::LEFT_END, egui::RectAlign::BOTTOM_START, egui::RectAlign::TOP_START];
    egui::Popup::menu(&resp)
        .id(menu_id)
        .open_memory(open)
        .anchor(Rect::from_center_size(pin, Vec2::splat(10.0)))
        .align(egui::RectAlign::RIGHT_START)
        .align_alternatives(&SIDES)
        .gap(SPACE_XS)
        .style(theme::glass_popup_style(true))
        .show(|ui| {
            theme::glass_popup(ui, |ui| {
                let Some(i) = target else { return };
                // the side whose modes show beside the rows, in the same
                // panel, which widens to them (Ash, 11-02: no submenus); the
                // row last hovered. DV_CURVE_MENU=1..3 opens one (captures)
                let side_id = menu_id.with("side");
                let mut side: Option<usize> = ui.data(|d| d.get_temp(side_id)).unwrap_or_else(|| std::env::var("DV_CURVE_MENU").ok().and_then(|v| v.parse::<usize>().ok()).filter(|n| (1..=3).contains(n)).map(|n| n - 1));
                let p = pts[i];
                // the slopes as drawn now: a side made Free starts from it
                let (ls, rs) = (curve_slope(pts, i, false), curve_slope(pts, i, true));
                let cur = |k: usize| match k {
                    0 => Some(p.l),
                    1 => Some(p.r),
                    _ => (p.l == p.r).then_some(p.l),
                };
                let round = egui::CornerRadius::same(theme::inner(theme::radius_card(), SPACE_SM));
                let mut pick = None;
                let mut delete = false;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_XS;
                    let rows = ui.vertical(|ui| {
                        ui.set_width(150.0);
                        ui.spacing_mut().item_spacing.y = SPACE_XS;
                        for (k, label) in [(0, crate::i18n::tr("왼쪽")), (1, crate::i18n::tr("오른쪽")), (2, crate::i18n::tr("양쪽"))] {
                            let bg = ui.painter().add(Shape::Noop);
                            let r = crate::kit::menu_row(ui, label, cur(k).map_or("—", |t| t.label()));
                            if r.hovered() || r.clicked() {
                                side = Some(k);
                            }
                            if side == Some(k) && !r.hovered() {
                                ui.painter().set(bg, Shape::rect_filled(r.rect, round, FILL_2()));
                            }
                        }
                        ui.separator();
                        ui.add_enabled_ui(pts.len() > 2, |ui| {
                            let r = crate::kit::menu_row(ui, crate::i18n::tr("점 삭제"), "");
                            if r.hovered() {
                                side = None;
                            }
                            delete = r.clicked();
                        });
                    });
                    // the modes, from the panel's top whichever row opened them
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let full = Tan::ALL.iter().map(|t| ui.painter().layout_no_wrap(t.label().to_owned(), font.clone(), INK()).size().x).fold(0.0, f32::max) + SPACE_SM * 2.0 + 24.0;
                    let t = ui.ctx().animate_bool_with_time(side_id.with("open"), side.is_some(), 0.12);
                    if t > 0.0 {
                        let h = rows.response.rect.height();
                        let (col, _) = ui.allocate_exact_size(vec2((full + SPACE_XS) * t, h), Sense::hover());
                        let body = Rect::from_min_size(pos2(col.left() + SPACE_XS, col.top()), vec2(full, h));
                        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
                        cu.set_clip_rect(col.intersect(ui.clip_rect()));
                        cu.spacing_mut().item_spacing.y = SPACE_XS;
                        cu.painter().vline(col.left(), col.y_range(), Stroke::new(1.0, FILL_3()));
                        if let Some(k) = side {
                            for m in Tan::ALL {
                                let bg = cu.painter().add(Shape::Noop);
                                let r = crate::kit::menu_row(&mut cu, m.label(), "");
                                if cur(k) == Some(m) && !r.hovered() {
                                    cu.painter().set(bg, Shape::rect_filled(r.rect, round, cu.visuals().selection.bg_fill));
                                }
                                if r.clicked() {
                                    pick = Some((k, m));
                                }
                            }
                        }
                    }
                });
                ui.data_mut(|d| d.insert_temp(side_id, side));
                if let Some((k, t)) = pick {
                    let q = &mut pts[i];
                    if k != 1 {
                        q.ls = if t != Tan::Free { 0.0 } else if q.l == Tan::Free { q.ls } else { ls };
                        q.l = t;
                    }
                    if k != 0 {
                        q.rs = if t != Tan::Free { 0.0 } else if q.r == Tan::Free { q.rs } else { rs };
                        q.r = t;
                    }
                    changed = true;
                    ui.close();
                }
                if delete {
                    pts.remove(i);
                    sel = None;
                    changed = true;
                    ui.close();
                }
            })
        });
    ui.data_mut(|d| {
        d.insert_temp(drag_id, drag);
        d.insert_temp(sel_id, sel.map(|i| (ch, i)));
    });
    let hover = drag.map(|d| d.0).or_else(|| resp.hover_pos().and_then(|s| near(pts, s)));
    // the graph: quarters, the straight line, the other channels' curves
    // faintly, then this channel's curve, its points and the picked one's
    // Free handles
    let p = ui.painter();
    p.rect_filled(g.expand(4.0), 6.0, FILL_1());
    for k in 1..4 {
        let t = k as f32 / 4.0;
        p.line_segment([pos2(g.left() + t * g.width(), g.top()), pos2(g.left() + t * g.width(), g.bottom())], Stroke::new(1.0, FILL_2()));
        p.line_segment([pos2(g.left(), g.top() + t * g.height()), pos2(g.right(), g.top() + t * g.height())], Stroke::new(1.0, FILL_2()));
    }
    p.line_segment([g.left_bottom(), g.right_top()], Stroke::new(1.0, FILL_3()));
    // enough samples that a Constant step stands straight
    let line = |c: &[CurvePt]| (0..=256).map(|k| k as f32 / 256.0).map(|x| to_s(&CurvePt::from([x, crate::doc::curve_at(c, x)]))).collect::<Vec<_>>();
    if !alpha {
        for c in 0..4 {
            if c != ch && fx.curves[c] != curve_identity() {
                p.add(Shape::line(line(&fx.curves[c]), Stroke::new(1.0, tint(c).gamma_multiply(0.45))));
            }
        }
    }
    let pts = &fx.curves[ch];
    p.add(Shape::line(line(pts), Stroke::new(1.5, ink)));
    for (i, r, h) in handles(pts, sel) {
        p.line_segment([to_s(&pts[i]), h], Stroke::new(1.0, theme::LABEL()));
        let on = drag == Some((i, Some(r)));
        p.circle(h, 3.5, if on { ACCENT } else { FILL_1() }, Stroke::new(1.2, if on { ACCENT } else { theme::LABEL() }));
    }
    for (i, q) in pts.iter().enumerate() {
        let on = hover == Some(i);
        let fill = if sel == Some(i) { ink } else { FILL_1() };
        p.circle(to_s(q), if on { 5.0 } else { 4.0 }, fill, Stroke::new(1.5, if on { ACCENT } else { ink }));
    }
    resp.glass_tip(crate::i18n::tr("클릭: 점 추가 · 밖으로 끌기: 삭제 · 우클릭: 탄젠트"));
    changed
}

/// The gradient map's colours as the gradient itself (Ash, 10-28): a bar on
/// the label grid with a swatch at each end; a swatch opens the palette
/// (hex, copy and paste live there).
fn gradient_bar(ui: &mut egui::Ui, colors: [[u8; 4]; 2], id: LayerId, req: &mut PalReq) {
    let col = |c: [u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    let (a, b) = (col(colors[0]), col(colors[1]));
    // laid out as a dial row: the whole row, the label cell at its left,
    // the bar from where the dials' rings start
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), theme::ROW_H), Sense::hover());
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(row.min, vec2(crate::dial::LABEL_CELL, row.height()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    crate::dial::label_cell(&mut lu, crate::i18n::tr("색"), None, &mut false);
    let row = Rect::from_min_max(pos2(row.left() + crate::dial::RING_X, row.top()), row.max);
    let sw = 22.0;
    let r0 = Rect::from_center_size(pos2(row.left() + sw * 0.5, row.center().y), vec2(sw, sw));
    let r1 = Rect::from_center_size(pos2(row.right() - sw * 0.5, row.center().y), vec2(sw, sw));
    // the ramp between the swatches, under them at both ends
    let bar = Rect::from_min_max(pos2(r0.center().x, row.center().y - 6.0), pos2(r1.center().x, row.center().y + 6.0));
    let mut m = egui::Mesh::default();
    m.colored_vertex(bar.left_top(), a);
    m.colored_vertex(bar.right_top(), b);
    m.colored_vertex(bar.right_bottom(), b);
    m.colored_vertex(bar.left_bottom(), a);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    ui.painter().add(Shape::mesh(m));
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, FILL_3()), StrokeKind::Outside);
    for (i, (r, c, tip)) in [(r0, a, crate::i18n::tr("어두운 쪽 색 · 팔레트")), (r1, b, crate::i18n::tr("밝은 쪽 색 · 팔레트"))].into_iter().enumerate() {
        let resp = ui.interact(r, ui.id().with(("fx_grad", i)), Sense::click());
        ui.painter().rect_filled(r, 5.0, c);
        ui.painter().rect_stroke(r, 5.0, Stroke::new(1.0, if resp.hovered() { INK_2() } else { FILL_3() }), StrokeKind::Inside);
        if resp.glass_tip(tip).clicked() {
            *req = Some((PalTarget::FxColor(id, i), c, r.right_top()));
        }
    }
}

/// An FX layer's filter and blend (복사, or a mode over the original). A
/// filter comes with the settings it had on this layer before, else its
/// defaults.
fn fx_props(ui: &mut egui::Ui, fx: &mut crate::doc::FxLayer) -> bool {
    let mut changed = false;
    crate::kit::row(ui, crate::i18n::tr("필터"), |ui| {
        let items: Vec<(crate::doc::Filter, String, bool)> = crate::doc::Filter::ALL.iter().map(|f| (*f, f.label().to_string(), true)).collect();
        if let (Some(f), _) = crate::kit::dropdown(ui, "fx_filter", fx.filter.label(), crate::kit::FILL, &items, Some(&fx.filter)) {
            if f != fx.filter {
                fx.set_filter(f);
                changed = true;
            }
        }
    });
    crate::kit::row(ui, crate::i18n::tr("혼합"), |ui| {
        crate::kit::dbg_open_combo(ui, "fx_blend");
        // 복사 replaces what is below with the filtered picture; a mode lays it over
        let items: Vec<(Option<Blend>, String, bool)> = std::iter::once((None, crate::i18n::tr_ctx("혼합", "복사").to_string(), true)).chain(Blend::ALL.into_iter().filter(|b| *b != Blend::Erase).map(|b| (Some(b), b.label().to_string(), true))).collect();
        let (pick, resp) = crate::kit::dropdown(ui, "fx_blend", fx.blend.map_or(crate::i18n::tr_ctx("혼합", "복사"), |b| b.label()), crate::kit::FILL, &items, Some(&fx.blend));
        resp.glass_tip(crate::i18n::tr("복사: 아래 그림을 필터 결과로 바꿈"));
        if let Some(b) = pick {
            changed |= fx.blend != b;
            fx.blend = b;
        }
    });
    changed
}

/// The 혼합 dropdown; `mixed`: the selected layers' blends differ ("—").
fn blend_row(ui: &mut egui::Ui, id: &str, b: &mut Blend, mixed: bool) -> bool {
    let mut changed = false;
    crate::kit::row(ui, crate::i18n::tr("혼합"), |ui| {
        let items: Vec<(Blend, String, bool)> = Blend::ALL.iter().map(|x| (*x, x.label().to_string(), true)).collect();
        let now = (!mixed).then_some(*b);
        if let (Some(x), _) = crate::kit::dropdown(ui, id, if mixed { crate::dial::MIXED } else { b.label() }, crate::kit::FILL, &items, now.as_ref()) {
            *b = x;
            changed = true;
        }
    });
    changed
}

/// Style values that differ across the selection (shown as "—"): the first
/// layer against the other selected layers of its kind.
#[derive(Default, Clone, Copy)]
struct MixedStyle {
    color: bool,
    bg_has: bool,
    bg: bool,
    fill_has: bool,
    fill: bool,
    family: bool,
    style: bool,
    blend: bool,
    align: bool,
    engrave: bool,
}

fn mixed_style(first: &Layer, others: &[&Layer]) -> MixedStyle {
    let mut m = MixedStyle::default();
    for o in others {
        if let (Some(a), Some(b)) = (&first.place, &o.place) {
            m.engrave |= a.engrave != b.engrave;
        }
        match (&first.kind, &o.kind) {
            (LayerKind::Text(a), LayerKind::Text(b)) => {
                m.color |= a.color != b.color;
                m.bg_has |= a.bg.is_some() != b.bg.is_some();
                m.bg |= a.bg.is_some() && b.bg.is_some() && a.bg != b.bg;
                m.family |= a.family != b.family;
                m.style |= a.style != b.style;
                m.blend |= a.blend != b.blend;
                m.align |= a.align != b.align;
            }
            (LayerKind::Shape(a), LayerKind::Shape(b)) => {
                m.color |= a.color != b.color;
                m.fill_has |= a.fill.is_some() != b.fill.is_some();
                m.fill |= a.fill.is_some() && b.fill.is_some() && a.fill != b.fill;
            }
            (LayerKind::Raster(a), LayerKind::Raster(b)) => m.blend |= a.blend != b.blend,
            _ => {}
        }
    }
    m
}

/// A style value set on a mixed field, for every selected layer of its kind.
enum Force {
    TextColor(Color32),
    TextBg(Option<Color32>),
    /// A text box border's / letter outline's colour (switched on).
    TextBorder(Color32),
    TextStroke(Color32),
    Font(String, String),
    Align(crate::doc::TextAlign),
    TextBlend(Blend),
    ShapeColor(Color32),
    ShapeFill(Option<Color32>),
    RasterBlend(Blend),
    /// 음각, for every selected layer in a space.
    Engrave(bool),
}

impl Force {
    fn apply(&self, l: &mut Layer) {
        if let Force::Engrave(on) = self {
            if let Some(p) = l.place.as_mut() {
                p.engrave = *on;
            }
            return;
        }
        let s0 = if let LayerKind::Text(t) = &l.kind { Some(t.frame()) } else { None };
        match (self, &mut l.kind) {
            (Force::TextColor(c), LayerKind::Text(t)) => t.color = *c,
            (Force::TextBg(c), LayerKind::Text(t)) => t.bg = *c,
            (Force::TextBorder(c), LayerKind::Text(t)) => t.look.border = Some(*c),
            (Force::TextStroke(c), LayerKind::Text(t)) => t.look.stroke = Some(*c),
            (Force::Font(f, s), LayerKind::Text(t)) => {
                t.family = f.clone();
                t.style = s.clone();
            }
            (Force::Align(a), LayerKind::Text(t)) => t.align = *a,
            (Force::TextBlend(b), LayerKind::Text(t)) => t.blend = *b,
            (Force::ShapeColor(c), LayerKind::Shape(s)) => s.color = *c,
            (Force::ShapeFill(c), LayerKind::Shape(s)) => s.fill = *c,
            (Force::RasterBlend(b), LayerKind::Raster(r)) => r.blend = *b,
            _ => {}
        }
        if let (Some(s0), LayerKind::Text(t)) = (s0, &mut l.kind) {
            t.keep_corner(s0);
        }
    }
}

/// A number without trailing zeros (up to 3 decimals).
fn trim_num(v: f64) -> String {
    let t = format!("{v:.3}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A position number box ("—" when the selection's positions differ).
/// Two labelled number boxes (X/Y, W/H) sharing the rest of the row, so the
/// second ends at the card's right edge like every right-anchored part
/// (Ash, 10-29).
fn pair_boxes(ui: &mut egui::Ui, la: &str, a: impl egui::Widget, lb: &str, b: impl egui::Widget) -> (egui::Response, egui::Response) {
    let gap = ui.spacing().item_spacing.x;
    let (lw, h) = (12.0, ui.spacing().interact_size.y);
    let w = ((ui.available_width() - 2.0 * (lw + gap) - gap) / 2.0).floor().max(32.0);
    let lab = |ui: &mut egui::Ui, t: &str| ui.add_sized([lw, h], egui::Label::new(egui::RichText::new(t).color(theme::LABEL())));
    lab(ui, la);
    let ra = ui.add_sized([w, h], a);
    lab(ui, lb);
    let rb = ui.add_sized([w, h], b);
    (ra, rb)
}

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
    // not keyed: outline detail and smooth shading spread as plain settings
    if let (Some(b), Some(a), Some(p)) = (before.place.as_ref(), after.place.as_ref(), l.place.as_mut()) {
        if a.segs != b.segs {
            p.segs = a.segs;
        }
        if a.smooth != b.smooth {
            p.smooth = a.smooth;
        }
        if a.engrave != b.engrave {
            p.engrave = a.engrave;
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
            let s0 = l.frame();
            pass!(b.color, a.color, l.color);
            pass!(b.family, a.family, l.family);
            pass!(b.style, a.style, l.style);
            pass!(b.bg, a.bg, l.bg);
            pass!(b.wrap, a.wrap, l.wrap);
            pass!(b.blend, a.blend, l.blend);
            pass!(b.align, a.align, l.align);
            pass!(b.look.pad_x, a.look.pad_x, l.look.pad_x);
            pass!(b.look.pad_y, a.look.pad_y, l.look.pad_y);
            pass!(b.look.radius, a.look.radius, l.look.radius);
            pass!(b.look.border, a.look.border, l.look.border);
            pass!(b.look.border_w, a.look.border_w, l.look.border_w);
            pass!(b.look.stroke, a.look.stroke, l.look.stroke);
            pass!(b.look.stroke_w, a.look.stroke_w, l.look.stroke_w);
            // the letters stay put while the box changes round them
            l.keep_corner(s0);
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
        let t = TextLayer { pos: pos2(x, 0.0), text: "ab".into(), size, color: Color32::WHITE, family: String::new(), style: String::new(), bg: None, wrap: 0.0, rotation: 0.0, blend: Blend::Normal, align: Default::default(), look: Default::default() };
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
    fn soft_scissors_edge_follows_the_measured_blur() {
        let pts = vec![pos2(10.0, 10.0), pos2(90.0, 10.0), pos2(90.0, 90.0), pos2(10.0, 90.0)];
        // an edge blurred by 2: alpha Φ(d/σ) across it (σ with a pixel's width)
        let mut m = crate::raster::RasterLayer::default();
        soft_mask_poly(&mut m, &pts, &[2.0; 4], true);
        let s = (4.0f32 + 1.0 / 12.0).sqrt();
        for x in [5.5f32, 7.5, 9.5, 10.5, 12.5, 14.5] {
            let a = m.alpha_at_world(pos2(x, 50.5)) as f32 / 255.0;
            assert!((a - phi((x - 10.0) / s)).abs() < 0.01, "x {x}: {a} vs {}", phi((x - 10.0) / s));
        }
        // a sharp edge: crisp
        let mut c = crate::raster::RasterLayer::default();
        soft_mask_poly(&mut c, &pts, &[0.0; 4], true);
        assert!(c.alpha_at_world(pos2(8.5, 50.5)) < 3 && c.alpha_at_world(pos2(11.5, 50.5)) > 252);
        assert_eq!(c.alpha_at_world(pos2(50.5, 50.5)), 255);
    }

    #[test]
    fn mixed_styles_show_and_a_set_value_reaches_all() {
        let a = text(0.0, 40.0);
        let mut b = text(100.0, 60.0);
        if let LayerKind::Text(t) = &mut b.kind {
            t.blend = Blend::Add;
            t.bg = Some(Color32::BLACK);
        }
        let m = mixed_style(&a, &[&b]);
        assert!(m.blend && m.bg_has, "blend and the background box differ");
        assert!(!m.color && !m.family && !m.style && !m.align && !m.bg, "the rest is the same");
        // undo keeps the playhead and the timeline's switch
        let mut old = Document::default();
        old.timeline.duration = 4.0;
        let mut now = Document::default();
        now.timeline = crate::anim::Timeline { on: true, time: 7.0, autokey: true, ..Default::default() };
        keep_navigation(&mut old, &now);
        assert_eq!((old.timeline.on, old.timeline.autokey, old.timeline.time, old.timeline.duration), (true, true, 4.0, 4.0));
        // a filter's settings come back when the layer returns to it
        use crate::doc::{Filter, FxLayer};
        let mut fx = FxLayer::new(Filter::Blur);
        fx.v[0] = 33.0;
        fx.set_filter(Filter::Curves);
        assert_eq!(fx.v, Filter::Curves.defaults());
        fx.curves[0][0].y = 0.4;
        let curve = fx.curves.clone();
        fx.set_filter(Filter::Blur);
        assert_eq!((fx.v[0], fx.kept.len()), (33.0, 1));
        fx.set_filter(Filter::Curves);
        assert_eq!((&fx.curves, fx.kept.len()), (&curve, 1));
        let back: FxLayer = serde_json::from_str(&serde_json::to_string(&fx).unwrap()).unwrap();
        assert_eq!(back, fx);
        // the hue bar starts at pure red
        assert_eq!(crate::okhsv::to_rgb(hue_top(), 1.0, 1.0), Color32::from_rgb(255, 0, 0));
        // 보통 picked on the mixed 혼합: the first already has it, so nothing
        // changed there to spread; the forced value still reaches the other
        Force::TextBlend(Blend::Normal).apply(&mut b);
        Force::TextBg(None).apply(&mut b);
        let LayerKind::Text(t) = &b.kind else { unreachable!() };
        assert_eq!((t.blend, t.bg), (Blend::Normal, None));
        // a shape is left alone by a text's value
        let mut s = a.clone();
        s.kind = LayerKind::Shape(crate::doc::ShapeLayer { kind: ShapeKind::Rect, a: Pos2::ZERO, b: pos2(1.0, 1.0), color: Color32::RED, width: 2.0, fill: None, rotation: 0.0 });
        Force::TextColor(Color32::BLUE).apply(&mut s);
        let LayerKind::Shape(sh) = &s.kind else { unreachable!() };
        assert_eq!(sh.color, Color32::RED);
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
