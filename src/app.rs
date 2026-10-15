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
    Picker,
}

const TOOLS: &[(Tool, &str, &str, Key)] = &[
    (Tool::Select, "선택", "V", Key::V),
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

pub struct Settings {
    pub color: Color32,
    pub brush_size: f32,
    pub brush_opacity: f32,
    pub brush_hardness: f32,
    pub brush_pressure_size: bool,
    pub brush_pressure_opacity: bool,
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
    pub text_bg: bool,
    pub text_bg_color: Color32,
    pub touch_draw: bool,
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
            text_bg: false,
            text_bg_color: Color32::from_rgba_unmultiplied(255, 255, 255, 220),
            touch_draw: false,
        }
    }
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
    Marquee { start: Pos2, cur: Pos2 },
    MoveMany { start: Pos2, origs: Vec<Layer> },
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

struct Palette {
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
    /// that contains the reference point.
    fn sample(self, r: (f32, f32, f32), win: Win, w: usize, h: usize) -> (Vec<Color32>, Vec<bool>) {
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
            }
            if i + 1 < w {
                stack.push(k + 1);
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
        let (mut dx, mut dy) = (0.0f32, 0.0f32);
        for (k, _) in keep.iter().enumerate().filter(|(_, k)| **k) {
            let x = full.x0 + ((k % w) as f32 + 0.5) / w as f32;
            let y = ((k / w) as f32 + 0.5) / h as f32;
            dx = dx.max((x - rx).abs());
            dy = dy.max((y - ry).abs());
        }
        let hx = (dx * 1.25 + 0.01).clamp(0.04, 0.5);
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
        let ghost_bg = theme::PAPER_2;
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

#[derive(Clone, Copy, PartialEq)]
enum ExportRegion {
    Frame,
    Content,
    View,
}

#[derive(Clone, Copy, PartialEq)]
enum ExportBg {
    Transparent,
    White,
    Custom,
}

struct ExportDlg {
    region: ExportRegion,
    padding: f32,
    scale: f32,
    bg: ExportBg,
    bg_color: Color32,
    jpeg_quality: u8,
}

pub struct App {
    doc: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    committed: Document,
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
    pending: Option<Pending>,
    status: String,
    recent: Vec<Color32>,
    view_init: bool,
    title: String,
    cursor_world: Option<Pos2>,
    font_filter: String,
    egui_fonts: std::collections::HashSet<String>,
    palette: Option<Palette>,
    restore_tool: Option<Tool>,
    ui_hidden: bool,
    fonts_applied: bool,
    frames: u64,
    rail_active: Option<Rect>,
    extra_sel: Vec<LayerId>,
    pan_latch: bool,
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
        let mut app = Self {
            doc: Document::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            committed: Document::default(),
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
            pending: None,
            status: String::new(),
            recent: Vec::new(),
            view_init: false,
            title: String::new(),
            cursor_world: None,
            font_filter: String::new(),
            egui_fonts: Default::default(),
            palette: None,
            restore_tool: None,
            ui_hidden: false,
            fonts_applied: false,
            frames: 0,
            rail_active: None,
            extra_sel: Vec::new(),
            pan_latch: false,
            middle_moved: false,
            input_log: Default::default(),
            show_log: false,
        };
        crate::penhook::install(cc);
        if let Some(p) = open {
            app.open_path(p);
        }
        if let Ok(fam) = std::env::var("DV_TEST_TEXT_FONT") {
            // reproduces "edit a text box that uses an installed font" for testing
            app.s.text_family = fam;
            app.s.text_style = "Regular".into();
            app.tool = Tool::Text;
            let id = app.new_text_layer(pos2(0.0, 0.0), "테스트 Test".into());
            app.sel = Some(id);
            app.editing = Some(id);
            app.edit_focus = true;
        }
        if std::env::var_os("DV_SHOW_EXPORT").is_some() {
            app.open_export();
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
        if self.changed && self.drag.is_none() && !typing {
            // drawing layers are created on demand, so empty ones just go away
            let before = self.doc.layers.len();
            self.doc.layers.retain(|l| match &l.kind {
                LayerKind::Raster(r) => !r.tiles.is_empty(),
                LayerKind::Vector(v) => !v.strokes.is_empty(),
                _ => true,
            });
            if self.doc.layers.len() != before {
                self.fix_sel();
            }
            let prev = std::mem::replace(&mut self.committed, self.doc.clone());
            self.undo.push(prev);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.changed = false;
        }
    }

    fn undo(&mut self) {
        self.cancel_drag();
        self.finish_text_edit();
        self.commit_if_idle(false);
        if let Some(d) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.doc, d));
            self.committed = self.doc.clone();
            self.modified = true;
            self.fix_sel();
        }
    }

    fn redo(&mut self) {
        self.cancel_drag();
        self.finish_text_edit();
        if let Some(d) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.doc, d));
            self.committed = self.doc.clone();
            self.modified = true;
            self.fix_sel();
        }
    }

    fn fix_sel(&mut self) {
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
        self.doc.layers.insert(idx, Layer { id, name, visible: true, locked: false, opacity: 1.0, kind });
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
            format!("사진 {n}장 불러옴")
        } else {
            format!("불러오기 실패: {}", errs.join(", "))
        };
    }

    fn import_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new().add_filter("이미지", crate::io::IMAGE_EXTS).pick_files() {
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
        if ids.is_empty() {
            return;
        }
        self.doc.layers.retain(|l| !ids.contains(&l.id));
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
                && match l.kind {
                    LayerKind::Vector(_) => vector,
                    LayerKind::Raster(_) => !vector,
                    _ => false,
                }
        };
        if let Some(l) = self.sel_layer() {
            if usable(l) {
                return l.id;
            }
        }
        if let Some(id) = self.doc.layers.iter().rev().find(|l| usable(l)).map(|l| l.id) {
            self.sel = Some(id);
            return id;
        }
        self.new_paint_layer(vector)
    }

    fn hit_layer(&self, w: Pos2, tol: f32) -> Option<LayerId> {
        for l in self.doc.layers.iter().rev() {
            if !l.visible || l.locked {
                continue;
            }
            let hit = match &l.kind {
                LayerKind::Image(im) => im.contains(w),
                LayerKind::Text(t) => crate::text::layout(t).rect(t.pos).expand(tol).contains(w),
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

    fn finish_text_edit(&mut self) {
        if let Some(id) = self.editing.take() {
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

    /// Handles of the selection in screen space: (id, position).
    fn handles(&self) -> Vec<(u8, Pos2)> {
        let xf = self.xf();
        let mut out = Vec::new();
        let Some(l) = self.sel_layer() else { return out };
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
                out.push((20, xf.w2s(r.right_center())));
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
        match self.tool {
            Tool::Select => {
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
                    // Shift: drag a selection box (a plain Shift+click toggles one layer)
                    self.drag = Some(Drag::Marquee { start: w, cur: w });
                    return;
                }
                match self.hit_layer(w, tol) {
                    Some(id) if self.is_selected(id) && !self.extra_sel.is_empty() => {
                        let origs: Vec<Layer> = self
                            .selected_ids()
                            .iter()
                            .filter_map(|i| self.doc.get(*i))
                            .filter(|l| !l.locked)
                            .cloned()
                            .collect();
                        self.drag = Some(Drag::MoveMany { start: w, origs });
                    }
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
                let layer = self.doc.get_mut(id).unwrap();
                match &mut layer.kind {
                    LayerKind::Raster(r) => {
                        let (size, ps) = if erase {
                            (self.s.eraser_size, self.s.eraser_pressure)
                        } else {
                            (self.s.brush_size, self.s.brush_pressure_size)
                        };
                        let p = BrushParams {
                            color: self.s.color,
                            radius: size * 0.5,
                            opacity: if erase { 1.0 } else { self.s.brush_opacity },
                            hardness: if erase { 0.9 } else { self.s.brush_hardness },
                            pressure_size: ps,
                            pressure_opacity: !erase && self.s.brush_pressure_opacity,
                            erase,
                        };
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
                    l.visible && !l.locked && matches!(&l.kind, LayerKind::Text(t) if crate::text::layout(t).rect(t.pos).expand(tol).contains(w))
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
            Tool::Hand | Tool::Picker => {}
        }
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
            Some(Drag::Marquee { start, .. }) => {
                self.drag = Some(Drag::Marquee { start, cur: w });
                return;
            }
            _ => {}
        }
        let Some(drag) = self.drag.as_mut() else { return };
        match drag {
            Drag::Zoom { .. } | Drag::Size { .. } | Drag::Marquee { .. } => {}
            Drag::MoveMany { start, origs } => {
                let mut d = w - *start;
                if mods.shift {
                    if d.x.abs() > d.y.abs() { d.y = 0.0 } else { d.x = 0.0 }
                }
                for o in origs.iter() {
                    let mut l = o.clone();
                    l.translate(d);
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
                l.translate(d);
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
                if mods.shift {
                    let step = 15f32.to_radians();
                    a = (a / step).round() * step;
                }
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
            Drag::Marquee { start, cur } => {
                let r = Rect::from_two_pos(start, cur);
                if r.width().max(r.height()) * self.zoom < 4.0 {
                    if let Some(id) = self.hit_layer(start, 6.0 / self.zoom) {
                        self.toggle_selected(id);
                    }
                } else {
                    // layers fully inside the box, topmost first
                    let ids: Vec<LayerId> = self
                        .doc
                        .layers
                        .iter()
                        .rev()
                        .filter(|l| l.visible && !l.locked && l.bounds().is_some_and(|b| r.contains_rect(b)))
                        .map(|l| l.id)
                        .collect();
                    self.sel = ids.first().copied();
                    self.extra_sel = ids.iter().skip(1).copied().collect();
                    self.status = if ids.is_empty() { "영역 안에 레이어 없음".into() } else { format!("{}개 선택", ids.len()) };
                }
            }
            Drag::Raster { id, stroke: Some(st), .. } => {
                if let Some(LayerKind::Raster(r)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    st.finish(r);
                }
            }
            Drag::Vector { id, mut pts, color, smooth, base_r, pressure, .. } => {
                let wsp = self.xf().s2w(sp);
                if let Some(LayerKind::Vector(v)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                    let l = wsp - v.offset;
                    let last = pts.last().copied().unwrap();
                    if (pos2(last[0], last[1]) - l).length() > 0.5 {
                        pts.push([l.x, l.y, radius_for(base_r, smooth.1, pressure)]);
                    }
                    v.strokes.push(Arc::new(VStroke::new(pts, color)));
                    self.changed = true;
                    self.modified = true;
                }
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
        let (events, space, mods) = ctx.input(|i| (i.events.clone(), i.key_down(Key::Space), i.modifiers));
        let typing = ctx.egui_wants_keyboard_input();
        let space = space && !typing;
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
        let typing = ctx.egui_wants_keyboard_input();
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
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
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
        if k(cmd, Key::E) {
            self.open_export();
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
        if k(Modifiers::NONE, Key::Delete) || k(Modifiers::NONE, Key::Backspace) {
            self.delete_selected();
        }
        if k(Modifiers::NONE, Key::F12) {
            self.show_log = !self.show_log;
        }
        if k(Modifiers::NONE, Key::Tab) {
            self.ui_hidden = !self.ui_hidden;
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
            for id in self.selected_ids() {
                if let Some(l) = self.doc.get_mut(id) {
                    if !l.locked {
                        l.translate(nudge);
                        self.changed = true;
                        self.modified = true;
                    }
                }
            }
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
            self.export = Some(ExportDlg {
                region: if self.doc.export_frame.is_some() { ExportRegion::Frame } else { ExportRegion::Content },
                padding: 20.0,
                scale: 1.0,
                bg: ExportBg::White,
                bg_color: Color32::from_rgb(250, 247, 240),
                jpeg_quality: 92,
            });
        }
    }

    fn export_region(&self, d: &ExportDlg) -> Option<Rect> {
        match d.region {
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
        match crate::export::render(&self.doc, r, scale, bg) {
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
        egui::Window::new("내보내기")
            .title_bar(false)
            .frame(egui::Frame::NONE)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| theme::glass(ui, egui::Margin::same(SPACE_LG as i8), theme::RADIUS_CARD, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("내보내기").font(FontId::new(15.0, theme::ui_bold())).color(INK));
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
            if d.region == ExportRegion::Frame && self.doc.export_frame.is_none() {
                d.region = ExportRegion::Content;
            }
            if d.region == ExportRegion::Content {
                ui.add(egui::Slider::new(&mut d.padding, 0.0..=400.0).text("여백 px"));
            }
            ui.horizontal(|ui| {
                ui.label("배율");
                for s in [0.25f32, 0.5, 1.0, 2.0] {
                    if ui.selectable_label((d.scale - s).abs() < 1e-4, format!("{}×", s)).clicked() {
                        d.scale = s;
                    }
                }
                ui.add(egui::DragValue::new(&mut d.scale).range(0.05..=8.0).speed(0.01).suffix("×"));
            });
            ui.horizontal(|ui| {
                ui.label("배경");
                ui.radio_value(&mut d.bg, ExportBg::Transparent, "투명");
                ui.radio_value(&mut d.bg, ExportBg::White, "흰색");
                ui.radio_value(&mut d.bg, ExportBg::Custom, "색");
                if d.bg == ExportBg::Custom {
                    ui.color_edit_button_srgba(&mut d.bg_color);
                }
            });
            ui.horizontal(|ui| {
                ui.label("JPG 품질");
                ui.add(egui::Slider::new(&mut d.jpeg_quality, 50..=100));
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
                    self.status = match crate::export::render(&self.doc, r, d.scale, bg)
                        .and_then(|pm| crate::export::save(&pm, &path, d.jpeg_quality).map(|_| (pm.width(), pm.height())))
                    {
                        Ok((w, h)) => format!("내보냄: {} ({w}×{h})", path.file_name().and_then(|n| n.to_str()).unwrap_or("")),
                        Err(e) => format!("내보내기 실패: {e}"),
                    };
                }
            } else {
                self.copy_region(r, d.scale, bg);
            }
        }
        if open {
            self.export = Some(d);
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
            p.rect_filled(r, 9.0, FILL_2);
        }
        // with glass the amber indicator is painted on the canvas layer below the rail
        // (see `draw_rail_indicator`), so it shows through blurred and refracted
        let col = if active {
            if under_glass { Color32::WHITE } else { ACCENT_INK }
        } else if resp.hovered() {
            INK
        } else {
            INK_2
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
                        &[Tool::Select, Tool::Hand],
                        &[Tool::Brush, Tool::Pen, Tool::Eraser],
                        &[Tool::Text, Tool::Line, Tool::Arrow, Tool::Rect, Tool::Ellipse],
                        &[Tool::Crop, Tool::Frame, Tool::Picker],
                    ];
                    for (gi, g) in groups.iter().enumerate() {
                        if gi > 0 {
                            let (r, _) = ui.allocate_exact_size(vec2(36.0, 9.0), Sense::hover());
                            ui.painter().line_segment([r.left_center() + vec2(10.0, 0.0), r.right_center() - vec2(10.0, 0.0)], Stroke::new(1.0, FILL_2));
                        }
                        for &t in g.iter() {
                            self.tool_button(ui, t);
                        }
                    }
                    ui.add_space(8.0);
                    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                    let p = ui.painter();
                    p.circle_filled(r.center(), 14.0, FILL_2);
                    p.circle_filled(r.center(), 11.0, self.s.color);
                    if resp.hovered() {
                        p.circle_stroke(r.center(), 14.0, Stroke::new(1.0, INK_2));
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
        let s = &mut self.s;
        match self.tool {
            Tool::Brush => {
                ui.strong("브러시");
                ui.add(egui::Slider::new(&mut s.brush_size, 1.0..=400.0).logarithmic(true).text("크기"));
                ui.add(egui::Slider::new(&mut s.brush_opacity, 0.05..=1.0).text("불투명도"));
                ui.add(egui::Slider::new(&mut s.brush_hardness, 0.0..=1.0).text("경도"));
                ui.checkbox(&mut s.brush_pressure_size, "필압 → 크기");
                ui.checkbox(&mut s.brush_pressure_opacity, "필압 → 농도");
                ui.add(egui::Slider::new(&mut s.smoothing, 0.0..=0.9).text("손떨림 보정"));
                ui.small("Ctrl+드래그: 크기 · Alt+클릭: 팔레트 · [ ]: 크기");
            }
            Tool::Pen => {
                ui.strong("펜 (벡터)");
                ui.add(egui::Slider::new(&mut s.pen_size, 0.5..=100.0).logarithmic(true).text("굵기"));
                ui.checkbox(&mut s.pen_pressure, "필압 → 굵기");
                ui.add(egui::Slider::new(&mut s.smoothing, 0.0..=0.9).text("손떨림 보정"));
                ui.small("획 단위로 저장 · 지우개는 한 획씩 지움 · Ctrl+드래그: 굵기");
            }
            Tool::Eraser => {
                ui.strong("지우개");
                ui.add(egui::Slider::new(&mut s.eraser_size, 2.0..=400.0).logarithmic(true).text("크기"));
                ui.checkbox(&mut s.eraser_pressure, "필압 → 크기");
                ui.small("브러시 레이어: 픽셀 지움 · 펜 레이어: 닿은 획 지움 · Ctrl+드래그: 크기");
            }
            Tool::Text => {
                ui.strong("글자");
                if let Some(LayerKind::Text(t)) = sel_text.map(|i| &mut self.doc.layers[i].kind) {
                    layer_changed |= ui.add(egui::Slider::new(&mut t.size, 6.0..=400.0).logarithmic(true).text("크기")).changed();
                    layer_changed |= font_picker(ui, "tool_font", &mut t.family, &mut t.style, &mut self.font_filter);
                    ui.horizontal(|ui| {
                        let mut has = t.bg.is_some();
                        if ui.checkbox(&mut has, "배경 상자").changed() {
                            t.bg = has.then_some(s.text_bg_color);
                            layer_changed = true;
                        }
                        if let Some(bg) = &mut t.bg {
                            layer_changed |= ui.color_edit_button_srgba(bg).changed();
                        }
                    });
                    // the next new text starts from the same style
                    s.text_size = t.size;
                    s.text_family = t.family.clone();
                    s.text_style = t.style.clone();
                    s.text_bg = t.bg.is_some();
                    if let Some(c) = t.bg {
                        s.text_bg_color = c;
                    }
                    ui.small("선택한 글자에 적용 · 새 글자도 이 설정으로 시작");
                } else {
                    ui.add(egui::Slider::new(&mut s.text_size, 6.0..=400.0).logarithmic(true).text("크기"));
                    font_picker(ui, "tool_font", &mut s.text_family, &mut s.text_style, &mut self.font_filter);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut s.text_bg, "배경 상자");
                        ui.color_edit_button_srgba(&mut s.text_bg_color);
                    });
                }
                ui.small("빈 곳 클릭: 새 글 · 글 클릭: 수정 · Esc: 끝내기");
            }
            Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse => {
                ui.strong("도형");
                if let Some(LayerKind::Shape(sh)) = sel_shape.map(|i| &mut self.doc.layers[i].kind) {
                    layer_changed |= ui.add(egui::Slider::new(&mut sh.width, 0.0..=60.0).text("선 굵기")).changed();
                    if matches!(sh.kind, ShapeKind::Rect | ShapeKind::Ellipse) {
                        ui.horizontal(|ui| {
                            let mut has = sh.fill.is_some();
                            if ui.checkbox(&mut has, "채우기").changed() {
                                sh.fill = has.then_some(s.fill_color);
                                layer_changed = true;
                            }
                            if let Some(f) = &mut sh.fill {
                                layer_changed |= ui.color_edit_button_srgba(f).changed();
                            }
                        });
                    }
                    s.shape_width = sh.width;
                    if let Some(f) = sh.fill {
                        s.fill_color = f;
                    }
                    ui.small("선택한 도형에 적용 · 새 도형도 이 설정으로 시작");
                } else {
                    ui.add(egui::Slider::new(&mut s.shape_width, 0.0..=60.0).text("선 굵기"));
                    if matches!(self.tool, Tool::Rect | Tool::Ellipse) {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut s.shape_fill, "채우기");
                            ui.color_edit_button_srgba(&mut s.fill_color);
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
                if let Some(r) = self.doc.export_frame {
                    ui.label(format!("{:.0} × {:.0} px", r.width(), r.height()));
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
            }
            Tool::Select => {
                ui.strong("선택");
                ui.small("클릭: 선택 · 끌기: 이동 · Shift+끌기: 영역 선택 · Shift+클릭: 추가/빼기 · 모서리: 크기 · 위 동그라미: 회전");
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
        ui.separator();
        ui.checkbox(&mut self.s.touch_draw, "손가락 터치로도 그리기")
            .on_hover_text("끄면 손가락 한 개는 화면 이동, 두 개는 확대/축소. 펜은 항상 그려.");
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.sel else {
            ui.weak("선택된 레이어 없음");
            return;
        };
        if !self.extra_sel.is_empty() {
            ui.label(egui::RichText::new(format!("{}개 선택 · 아래는 첫 번째 항목", self.extra_sel.len() + 1)).color(INK_3));
        }
        let Some(idx) = self.doc.index_of(id) else { return };
        let tool = self.tool;
        let shape_tool = matches!(tool, Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse);
        let mut changed = false;
        let mut start_crop = false;
        {
            let l = &mut self.doc.layers[idx];
            ui.horizontal(|ui| {
                ui.label(l.kind_label());
                if !matches!(l.kind, LayerKind::Text(_)) {
                    changed |= ui.text_edit_singleline(&mut l.name).changed();
                }
            });
            changed |= ui.add(egui::Slider::new(&mut l.opacity, 0.0..=1.0).text("불투명도")).changed();
            match &mut l.kind {
                LayerKind::Image(im) => {
                    ui.label(format!("원본 {}×{} px · 잘린 크기 {:.0}×{:.0}", im.src.rgba.width(), im.src.rgba.height(), im.crop.width(), im.crop.height()));
                    let mut pct = im.scale * 100.0;
                    if ui.add(egui::Slider::new(&mut pct, 1.0..=400.0).logarithmic(true).text("크기 %")).changed() {
                        im.scale = pct / 100.0;
                        changed = true;
                    }
                    let mut deg = im.rotation.to_degrees();
                    if ui.add(egui::Slider::new(&mut deg, -180.0..=180.0).text("회전 °")).changed() {
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
                    changed |= ui.add(egui::Slider::new(&mut a.brightness, -1.0..=1.0).text("밝기")).changed();
                    changed |= ui.add(egui::Slider::new(&mut a.contrast, -1.0..=1.0).text("대비")).changed();
                    changed |= ui.add(egui::Slider::new(&mut a.saturation, -1.0..=1.0).text("채도")).changed();
                    if ui.button("보정 초기화").clicked() {
                        *a = Adjust::default();
                        changed = true;
                    }
                }
                LayerKind::Text(t) => {
                    let r = ui.add(egui::TextEdit::multiline(&mut t.text).desired_rows(3).desired_width(f32::INFINITY));
                    changed |= r.changed();
                    if tool != Tool::Text {
                        changed |= ui.add(egui::Slider::new(&mut t.size, 6.0..=400.0).logarithmic(true).text("크기")).changed();
                        changed |= font_picker(ui, "prop_font", &mut t.family, &mut t.style, &mut self.font_filter);
                        ui.horizontal(|ui| {
                            let mut has = t.bg.is_some();
                            if ui.checkbox(&mut has, "배경 상자").changed() {
                                t.bg = has.then_some(self.s.text_bg_color);
                                changed = true;
                            }
                            if let Some(bg) = &mut t.bg {
                                changed |= ui.color_edit_button_srgba(bg).changed();
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("색");
                        changed |= ui.color_edit_button_srgba(&mut t.color).changed();
                    });
                    ui.horizontal(|ui| {
                        let mut wrap = t.wrap > 0.0;
                        if ui.checkbox(&mut wrap, "줄바꿈 폭").changed() {
                            t.wrap = if wrap { (crate::text::layout(t).size.x).max(t.size * 4.0) } else { 0.0 };
                            changed = true;
                        }
                        if t.wrap > 0.0 {
                            changed |= ui.add(egui::DragValue::new(&mut t.wrap).range(10.0..=20000.0).suffix(" px")).changed();
                        }
                    });
                }
                LayerKind::Shape(s) => {
                    ui.horizontal(|ui| {
                        ui.label("색");
                        changed |= ui.color_edit_button_srgba(&mut s.color).changed();
                        if !shape_tool {
                            changed |= ui.add(egui::Slider::new(&mut s.width, 0.0..=60.0).text("굵기")).changed();
                        }
                    });
                    if !shape_tool && matches!(s.kind, ShapeKind::Rect | ShapeKind::Ellipse) {
                        ui.horizontal(|ui| {
                            let mut has = s.fill.is_some();
                            if ui.checkbox(&mut has, "채우기").changed() {
                                s.fill = has.then_some(self.s.fill_color);
                                changed = true;
                            }
                            if let Some(f) = &mut s.fill {
                                changed |= ui.color_edit_button_srgba(f).changed();
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
                }
            }
        }
        if changed {
            self.mark();
        }
        if start_crop {
            self.set_tool(Tool::Crop);
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
            ui.label(egui::RichText::new("비어 있음").color(INK_3));
            return;
        }
        let mut changed = false;
        let mut select = None;
        ui.spacing_mut().item_spacing.y = 2.0;
        let width = ui.available_width();
        for l in self.doc.layers.iter_mut().rev() {
            let selected = self.sel == Some(l.id) || self.extra_sel.contains(&l.id);
            let (r, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
            let eye_r = Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(26.0, 26.0));
            let lock_r = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(26.0, 26.0));
            let eye = ui.interact(eye_r, ui.id().with(("eye", l.id)), Sense::click());
            let lock = ui.interact(lock_r, ui.id().with(("lock", l.id)), Sense::click());
            let p = ui.painter();
            if selected {
                p.rect_filled(r, 8.0, FILL_2);
                p.rect_filled(Rect::from_min_size(r.min + vec2(0.0, 7.0), vec2(3.0, 16.0)), 1.5, ACCENT);
            } else if resp.hovered() {
                p.rect_filled(r, 8.0, FILL_1);
            }
            let dim = if l.visible { 1.0 } else { 0.45 };
            theme::eye_icon(p, eye_r.center(), l.visible, if eye.hovered() { INK } else { INK_3 });
            if l.locked {
                theme::lock_icon(p, lock_r.center(), INK_2);
            } else if lock.hovered() || resp.hovered() {
                theme::lock_icon(p, lock_r.center(), FILL_3);
            }
            let clip = p.with_clip_rect(Rect::from_min_max(pos2(r.left() + 32.0, r.top()), pos2(r.right() - 32.0, r.bottom())));
            let kind = clip.text(
                pos2(r.left() + 34.0, r.center().y),
                Align2::LEFT_CENTER,
                l.kind_label(),
                FontId::proportional(theme::TEXT_SMALL),
                INK_3.gamma_multiply(dim),
            );
            clip.text(
                pos2(kind.right() + 8.0, r.center().y),
                Align2::LEFT_CENTER,
                l.display_name(),
                FontId::proportional(theme::TEXT_BODY),
                (if selected { INK } else { INK_2 }).gamma_multiply(dim),
            );
            if eye.clicked() {
                l.visible = !l.visible;
                changed = true;
            } else if lock.clicked() {
                l.locked = !l.locked;
                changed = true;
            } else if resp.clicked() {
                select = Some(l.id);
            }
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
                        ui.label(egui::RichText::new("Drifting VOID").font(FontId::new(14.0, theme::ui_bold())).color(INK));
                        ui.add_space(SPACE_SM);
                        self.menus(ui);
                    });
                });
            });
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("파일", |ui| {
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
            if ui.button("영역을 클립보드로 복사  Ctrl+Shift+C").clicked() {
                self.quick_copy();
            }
        });
        ui.menu_button("편집", |ui| {
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
        });
        ui.menu_button("보기", |ui| {
            if ui.button("전체 보기  Ctrl+0").clicked() {
                self.fit_view();
            }
            if ui.button("100%  Ctrl+1").clicked() {
                self.set_zoom_center(1.0);
            }
            if ui.button("UI 숨기기  Tab").clicked() {
                self.ui_hidden = true;
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
            ui.add_enabled_ui(on, |ui| {
                let mut k = crate::glass::strength();
                let r = ui.add(egui::Slider::new(&mut k, 0.0..=4.0).text("유리 가장자리")).on_hover_text("굴절 + 색 번짐 강도 · 0: 평평한 반투명");
                if r.changed() {
                    crate::glass::set_strength(k);
                }
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    crate::glass::save_settings();
                }
                let mut bw = crate::glass::band();
                let r = ui.add(egui::Slider::new(&mut bw, 1.0..=24.0).suffix(" pt").text("굴절 테두리 폭")).on_hover_text("가장자리에서 굴절·색 번짐이 일어나는 폭");
                if r.changed() {
                    crate::glass::set_band(bw);
                }
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    crate::glass::save_settings();
                }
            });
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
                            .add(egui::Label::new(egui::RichText::new(format!("{:.0}%", self.zoom * 100.0)).monospace().color(INK)).sense(Sense::click()))
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
                            ui.label(egui::RichText::new(format!("{:.0}, {:.0}", w.x, w.y)).monospace().color(INK_3));
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
                            ui.label(egui::RichText::new(&self.status).color(INK_2));
                        }
                    });
                });
            });
    }

    fn section(ui: &mut egui::Ui, title: &str) {
        ui.add_space(SPACE_SM);
        ui.label(egui::RichText::new(title).font(FontId::new(theme::TEXT_LABEL, theme::ui_bold())).color(INK_3));
        ui.add_space(2.0);
    }

    fn inspector(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("inspector"))
            .order(egui::Order::Middle)
            .anchor(Align2::RIGHT_TOP, vec2(-SPACE_LG, SPACE_LG))
            .show(ctx, |ui| {
                theme::glass(ui, egui::Margin::same(theme::SPACE_MD as i8), theme::RADIUS_CARD, |ui| {
                    ui.set_width(268.0);
                    Self::section(ui, "도구");
                    self.tool_options(ui);
                    ui.add_space(SPACE_MD);
                    Self::section(ui, "선택한 항목");
                    self.properties(ui);
                    ui.add_space(SPACE_MD);
                    Self::section(ui, "레이어");
                    let remaining = (screen.bottom() - ui.next_widget_position().y - SPACE_LG * 2.0 - 70.0).max(160.0);
                    // floating areas size from the previous frame, so ask for the height explicitly
                    let needed = self.doc.layers.len().max(1) as f32 * 32.0 + 48.0;
                    egui::ScrollArea::vertical()
                        .id_salt("layers")
                        .max_height(remaining)
                        .min_scrolled_height(needed.min(remaining))
                        .show(ui, |ui| {
                            self.layers_panel(ui);
                        });
                });
            });
    }

    // ------------------------------------------------------------ palette

    fn open_palette(&mut self, at: Pos2) {
        let hint = self.palette.as_ref().map(|p| p.h).unwrap_or(0.08);
        let (h, s, v) = crate::okhsv::from_rgb(self.s.color, hint);
        self.palette = Some(Palette {
            pos: at + vec2(14.0, 14.0),
            h,
            s,
            v,
            orig: self.s.color,
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
                pa.rect_stroke(r.expand(1.5), 6.0, Stroke::new(1.5, INK), StrokeKind::Outside);
            } else if resp.hovered() {
                pa.rect_stroke(r, 5.0, Stroke::new(1.0, INK_2), StrokeKind::Outside);
            }
            resp
        };
        let recent = if self.recent.is_empty() { PALETTE.to_vec() } else { self.recent.clone() };
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
                        ui.label(egui::RichText::new(format!("#{r:02X}{g:02X}{b:02X}")).monospace().color(INK_2));
                    });
                    let mut near = show_near;
                    if ui.checkbox(&mut near, egui::RichText::new("비슷한 색 지도").size(theme::TEXT_SMALL).color(INK_2)).changed() {
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
                        ui.label(egui::RichText::new(title).size(theme::TEXT_SMALL).color(INK_3)).on_hover_text(tip);
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
            self.s.color = c;
            self.recolor_selected();
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
        self.handle_canvas_input(&ctx, rect, ui.layer_id());

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
        let editing = self.editing;
        canvas::draw_doc(&painter, &ctx, &self.doc, &xf, &mut self.tex, editing);

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

        self.draw_overlays(&painter, &xf);
        self.draw_rail_indicator(&ctx, &painter);
        self.text_editor(&ctx, &xf);

        let size_anchor = match &self.drag {
            Some(Drag::Size { anchor, .. }) => Some(*anchor),
            _ => None,
        };
        if let Some(a) = size_anchor {
            if let Some(v) = self.tool_size() {
                let rr = if matches!(self.tool, Tool::Text) { 0.0 } else { v * 0.5 * self.zoom };
                painter.circle_stroke(a, rr.max(1.5) + 0.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
                painter.circle_stroke(a, rr.max(1.5), Stroke::new(1.5, ACCENT));
                painter.text(a + vec2(rr + 10.0, 0.0), Align2::LEFT_CENTER, format!("{v:.1} px"), FontId::proportional(13.0), INK);
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
                        painter.rect_filled(b, 6.0, theme::PAPER_2.gamma_multiply(0.92));
                        painter.rect_stroke(b, 6.0, Stroke::new(1.0, PAPER_4), StrokeKind::Inside);
                        theme::tool_icon(&painter, self.tool, b.shrink(4.0), INK);
                    }
                }
            }
        }

        if self.doc.layers.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "사진을 끌어다 놓거나 Ctrl+V로 붙여넣기",
                FontId::new(18.0, theme::ui_bold()),
                INK_3,
            );
        }
        let _ = resp;
    }

    /// Amber "selected tool" block under the tool rail's glass, gliding between tools.
    fn draw_rail_indicator(&self, ctx: &egui::Context, painter: &egui::Painter) {
        let Some(r) = self.rail_active.filter(|_| !self.ui_hidden && crate::glass::available()) else { return };
        let y = ctx.animate_value_with_time(egui::Id::new("rail_indicator_y"), r.center().y, 0.16);
        let c = pos2(r.center().x, y);
        painter.rect_filled(Rect::from_center_size(c, r.size() + vec2(6.0, 6.0)), 11.0, ACCENT);
    }

    fn draw_overlays(&self, painter: &egui::Painter, xf: &Xf) {
        let accent = ACCENT;
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
                    handle_box(painter, xf.w2s(p), INK);
                }
            }
            let pts = vec![sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom(), sr.left_top()];
            painter.extend(Shape::dashed_line(&pts, Stroke::new(1.5, INK), 8.0, 5.0));
            painter.text(
                sr.left_top() - vec2(0.0, 4.0),
                Align2::LEFT_BOTTOM,
                format!("내보낼 영역 {:.0}×{:.0}", fr.width(), fr.height()),
                FontId::proportional(12.0),
                INK,
            );
        }
        if let Some(Drag::Marquee { start, cur }) = &self.drag {
            let r = xf.rect_w2s(Rect::from_two_pos(*start, *cur));
            painter.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.08));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
        }
        for id in &self.extra_sel {
            if let Some(b) = self.doc.get(*id).and_then(|l| l.bounds()) {
                let sr = xf.rect_w2s(b).expand(2.0);
                let pts = vec![sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom(), sr.left_top()];
                painter.extend(Shape::dashed_line(&pts, Stroke::new(1.0, ACCENT), 5.0, 4.0));
            }
        }
        let Some(l) = self.sel_layer() else { return };
        if self.editing == Some(l.id) {
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
                    painter.add(Shape::line(pts, Stroke::new(1.5, INK)));
                    for (_, p) in self.handles() {
                        handle_box(painter, p, INK);
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
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("text_edit_bg")));
        if let Some(bg) = t.bg {
            painter.rect_filled(r, t.size * 0.3 * xf.zoom, bg);
        }
        painter.rect_stroke(r.expand(3.0), 2.0, Stroke::new(1.0, ACCENT), StrokeKind::Outside);
        let fam = self.egui_family(ctx, &t);
        let font = FontId::new((t.size * xf.zoom).max(2.0), fam);
        let pad = lay.pad * xf.zoom;
        let width = if t.wrap > 0.0 { (t.wrap - lay.pad * 2.0) * xf.zoom } else { (lay.size.x - lay.pad * 2.0) * xf.zoom + t.size * xf.zoom * 2.0 };
        let mut text = t.text.clone();
        let mut lost = false;
        egui::Area::new(egui::Id::new("text_edit_area"))
            .fixed_pos(r.min + vec2(pad, pad))
            // below the floating panels (they are Middle), and never pushed back on-screen
            .order(egui::Order::Background)
            .constrain(false)
            .show(ctx, |ui| {
                let te = egui::TextEdit::multiline(&mut text)
                    .font(font)
                    .text_color(t.color)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::ZERO)
                    .desired_width(width.max(10.0))
                    .desired_rows(1)
                    .hint_text("글자 입력");
                let resp = ui.add(te);
                if self.edit_focus {
                    resp.request_focus();
                    self.edit_focus = false;
                }
                lost = resp.lost_focus();
            });
        if text != t.text {
            if let Some(LayerKind::Text(tt)) = self.doc.get_mut(id).map(|l| &mut l.kind) {
                tt.text = text;
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
        egui::Modal::new(egui::Id::new("unsaved")).frame(egui::Frame::NONE).show(ctx, |ui| theme::glass(ui, egui::Margin::same(SPACE_LG as i8), theme::RADIUS_CARD, |ui| {
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
fn font_picker(ui: &mut egui::Ui, id: &str, family: &mut String, style: &mut String, filter: &mut String) -> bool {
    let mut changed = false;
    let fams = crate::fontdb::families();
    let shown = if family.is_empty() { DEFAULT_FONT.to_string() } else { family.clone() };
    ui.horizontal(|ui| {
        ui.label("글꼴");
        egui::ComboBox::from_id_salt(id).selected_text(shown).width(170.0).height(360.0).show_ui(ui, |ui| {
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
            for fam in fams.iter().filter(|x| f.is_empty() || x.name.to_lowercase().contains(&f)) {
                if ui.selectable_label(*family == fam.name, &fam.name).clicked() {
                    *family = fam.name.clone();
                    *style = fam.regular().style.clone();
                    changed = true;
                }
            }
        });
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
            egui::ComboBox::from_id_salt((id, "style")).selected_text(style.clone()).width(170.0).show_ui(ui, |ui| {
                for s in styles {
                    if ui.selectable_label(*style == s, &s).clicked() {
                        *style = s;
                        changed = true;
                    }
                }
            });
        });
    }
    changed
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

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

        if ctx.input(|i| i.viewport().close_requested()) && self.modified && self.pending.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }

        self.handle_shortcuts(&ctx);

        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
        if !self.ui_hidden {
            self.top_bar(&ctx);
            self.tool_rail(&ctx);
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
                        ui.label(egui::RichText::new("입력 기록 · F12로 닫기").font(FontId::new(theme::TEXT_LABEL, theme::ui_bold())).color(INK_3));
                        let pen = crate::penhook::state();
                        ui.label(
                            egui::RichText::new(format!(
                                "펜 범위 {} · 버튼 {} · 지우개 {} · flags {:X}/{:X}",
                                pen.in_range, pen.barrel, pen.eraser, pen.raw_pen, pen.raw_pointer
                            ))
                            .monospace()
                            .color(INK_2),
                        );
                        for line in &self.input_log {
                            ui.label(egui::RichText::new(line).monospace().color(INK_2));
                        }
                    });
                });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.export_ui(&ctx);
        self.unsaved_modal(&ctx);

        let typing = ctx.egui_wants_keyboard_input();
        self.commit_if_idle(typing);
        self.tex.gc();

        let name = self.path.as_ref().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or("제목 없음");
        let title = format!("{}{} — Drifting VOID", if self.modified { "● " } else { "" }, name);
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        self.frames += 1;
        if self.drag.is_some() || crate::penhook::state().barrel || self.frames < 6 {
            ctx.request_repaint();
        }
    }
}
