//! Visual system: tokens, egui visuals, UI fonts, hand-drawn tool icons.
//! Direction: a dark "void" canvas with two faint fixed blooms, one warm amber
//! accent used only for state (active tool, selection, focus), floating
//! elevated cards instead of hairline panels, 4-pt spacing.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Painter, Pos2, Rect, Shadow, Stroke, Vec2, pos2, vec2};

// ---------------------------------------------------------------- tokens

pub const VOID: Color32 = Color32::from_rgb(13, 12, 19); // canvas ground
pub const PAPER_2: Color32 = Color32::from_rgb(24, 23, 32); // cards
pub const PAPER_4: Color32 = Color32::from_rgb(46, 44, 58); // pressed, selected rows
pub const INK: Color32 = Color32::from_rgb(236, 232, 225);
pub const INK_2: Color32 = Color32::from_rgb(167, 162, 176);
pub const INK_3: Color32 = Color32::from_rgb(109, 104, 120);
/// Translucent control fills so buttons, rows and inputs sit *in* the glass.
pub const FILL_1: Color32 = Color32::from_rgba_premultiplied(14, 14, 14, 14);
pub const FILL_2: Color32 = Color32::from_rgba_premultiplied(24, 24, 24, 24);
pub const FILL_3: Color32 = Color32::from_rgba_premultiplied(36, 36, 36, 36);
pub const ACCENT: Color32 = Color32::from_rgb(242, 165, 65);
pub const ACCENT_INK: Color32 = Color32::from_rgb(26, 18, 6);
pub const BLOOM_WARM: Color32 = Color32::from_rgb(242, 140, 50);
pub const BLOOM_COOL: Color32 = Color32::from_rgb(110, 80, 220);

pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_LG: f32 = 16.0;
pub const RADIUS_CARD: u8 = 14;
pub const RADIUS_CTRL: u8 = 8;

pub const TEXT_BODY: f32 = 13.5;
pub const TEXT_SMALL: f32 = 11.5;
pub const TEXT_LABEL: f32 = 12.0;

pub fn ui_bold() -> FontFamily {
    FontFamily::Name("ui_bold".into())
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(PAPER_2)
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(Margin::same(SPACE_MD as i8))
        .shadow(if crate::glass::shadow() {
            Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(110) }
        } else {
            Shadow::NONE
        })
}

/// A floating card: frosted glass when the GL path is available, the flat
/// elevated card otherwise.
pub fn glass<R>(ui: &mut egui::Ui, margin: Margin, radius: u8, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    if !crate::glass::available() {
        return card_frame().inner_margin(margin).corner_radius(radius).show(ui, add);
    }
    let idx = ui.painter().add(egui::Shape::Noop);
    let r = egui::Frame::NONE.inner_margin(margin).corner_radius(radius).show(ui, add);
    ui.painter().set(idx, glass_shape(ui.ctx(), r.response.rect, radius));
    r
}

pub fn glass_shape(ctx: &egui::Context, rect: Rect, radius: u8) -> egui::Shape {
    let [tr, tg, tb, _] = PAPER_2.to_array();
    let look = crate::glass::Look {
        radius: radius as f32,
        refract: 7.0,
        tint: [tr as f32 / 255.0, tg as f32 / 255.0, tb as f32 / 255.0, 0.62],
        dispersion: 5.0,
        frame: ctx.cumulative_pass_nr(),
    };
    crate::glass::shape(rect, look)
}

/// Popup style for menus and dropdowns that host their own glass: egui's popup
/// frame is made invisible and `glass_popup` paints the glass behind the content.
pub fn glass_popup_style(menu: bool) -> egui::style::StyleModifier {
    (move |s: &mut egui::Style| {
        if menu {
            egui::containers::menu::menu_style(s);
        }
        if crate::glass::available() {
            s.visuals.window_fill = Color32::TRANSPARENT;
            s.visuals.window_stroke = Stroke::NONE;
            s.visuals.popup_shadow = Shadow::NONE;
        }
    })
    .into()
}

/// Wraps popup content (menu / dropdown) with the glass, sized to the popup frame.
pub fn glass_popup<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if !crate::glass::available() {
        return add(ui);
    }
    let idx = ui.painter().add(egui::Shape::Noop);
    let out = add(ui);
    let m = ui.spacing().menu_margin;
    // scrolled popups (font list) only show part of their content
    let r = ui.min_rect().intersect(ui.clip_rect());
    let r = Rect::from_min_max(r.min - vec2(m.left as f32, m.top as f32), r.max + vec2(m.right as f32, m.bottom as f32));
    ui.painter().with_clip_rect(egui::Rect::EVERYTHING).set(idx, glass_shape(ui.ctx(), r, 12));
    out
}

pub fn apply_visuals(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = Some(INK);
    v.panel_fill = VOID;
    // popups (menus, dropdowns, tooltips) can't host the GL glass, so they use the
    // same tint nearly opaque, same radius, and follow the shadow setting
    v.window_fill = Color32::from_rgba_unmultiplied(24, 23, 32, 245);
    v.window_stroke = Stroke::new(1.0, FILL_1);
    v.window_corner_radius = CornerRadius::same(RADIUS_CARD);
    let shadow = crate::glass::shadow();
    v.window_shadow = if shadow { Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(130) } } else { Shadow::NONE };
    v.popup_shadow = if shadow { Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(120) } } else { Shadow::NONE };
    v.menu_corner_radius = CornerRadius::same(12);
    v.extreme_bg_color = FILL_1;
    v.faint_bg_color = FILL_1;
    v.code_bg_color = FILL_1;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.5, ACCENT);
    v.hyperlink_color = ACCENT;
    v.slider_trailing_fill = true;
    let ctrl = CornerRadius::same(RADIUS_CTRL);
    for (w, bg, fg) in [
        (&mut v.widgets.noninteractive, Color32::TRANSPARENT, INK_2),
        (&mut v.widgets.inactive, FILL_1, INK),
        (&mut v.widgets.hovered, FILL_2, INK),
        (&mut v.widgets.active, FILL_3, INK),
        (&mut v.widgets.open, FILL_2, INK),
    ] {
        w.bg_fill = bg;
        w.weak_bg_fill = bg;
        w.bg_stroke = Stroke::NONE;
        w.corner_radius = ctrl;
        w.fg_stroke = Stroke::new(1.2, fg);
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, FILL_2); // separators
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.hovered.bg_stroke = Stroke::NONE;

    let s = &mut style.spacing;
    s.item_spacing = vec2(SPACE_SM, 6.0);
    s.button_padding = vec2(10.0, 5.0);
    s.interact_size = vec2(28.0, 26.0);
    s.slider_width = 150.0;
    s.menu_margin = Margin::same(SPACE_SM as i8);
    s.window_margin = Margin::same(SPACE_LG as i8);
    s.combo_width = 170.0;

    style.text_styles = [
        (egui::TextStyle::Heading, FontId::new(15.0, ui_bold())),
        (egui::TextStyle::Body, FontId::new(TEXT_BODY, FontFamily::Proportional)),
        (egui::TextStyle::Button, FontId::new(TEXT_BODY, FontFamily::Proportional)),
        (egui::TextStyle::Small, FontId::new(TEXT_SMALL, FontFamily::Proportional)),
        (egui::TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_global_style(style);
}

/// UI faces, best first. Falls back to the document default (Malgun Gothic).
const UI_FACES: &[(&str, &str, &str)] = &[
    ("Pretendard", "Regular", "SemiBold"),
    ("Pretendard Variable", "Regular", "SemiBold"),
    ("SUIT", "Regular", "SemiBold"),
    ("나눔스퀘어 네오 OTF", "Regular", "ExtraBold"),
    ("나눔스퀘어 네오", "Regular", "ExtraBold"),
];

/// Builds egui font definitions; called again once the system font scan lands.
pub fn font_defs() -> egui::FontDefinitions {
    let f = crate::text::fonts();
    let mut defs = egui::FontDefinitions::default();
    defs.font_data.insert("doc".into(), std::sync::Arc::new(egui::FontData::from_owned(f.regular_bytes.clone())));
    defs.font_data.insert("doc_bold".into(), std::sync::Arc::new(egui::FontData::from_owned(f.bold_bytes.clone())));
    let mut ui_regular = "doc".to_string();
    let mut ui_bold_name = "doc_bold".to_string();
    if crate::fontdb::families().is_some() {
        for (fam, reg, bold) in UI_FACES {
            if let (Some(r), Some(b)) = (crate::fontdb::load(fam, reg), crate::fontdb::load(fam, bold)) {
                let mut rd = egui::FontData::from_owned((*r.bytes).clone());
                rd.index = r.index;
                let mut bd = egui::FontData::from_owned((*b.bytes).clone());
                bd.index = b.index;
                defs.font_data.insert("ui".into(), std::sync::Arc::new(rd));
                defs.font_data.insert("ui_bold".into(), std::sync::Arc::new(bd));
                ui_regular = "ui".into();
                ui_bold_name = "ui_bold".into();
                break;
            }
        }
    }
    let prop = defs.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, ui_regular.clone());
    prop.push("doc".into());
    defs.families.entry(FontFamily::Monospace).or_default().push("doc".into());
    defs.families.insert(FontFamily::Name("doc".into()), vec!["doc".into()]);
    defs.families.insert(FontFamily::Name("doc_bold".into()), vec!["doc_bold".into(), "doc".into()]);
    defs.families.insert(ui_bold(), vec![ui_bold_name, "doc_bold".into()]);
    defs
}

// ---------------------------------------------------------------- canvas ground

fn radial(painter: &Painter, c: Pos2, r: f32, color: Color32) {
    let mut m = egui::Mesh::default();
    let n = 48;
    m.colored_vertex(c, color);
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        m.colored_vertex(c + vec2(a.cos(), a.sin()) * r, Color32::TRANSPARENT);
    }
    for i in 1..=n as u32 {
        m.add_triangle(0, i, i + 1);
    }
    painter.add(egui::Shape::mesh(m));
}

pub fn draw_ground(painter: &Painter, rect: Rect, xf: &crate::canvas::Xf) {
    painter.rect_filled(rect, 0.0, VOID);
    let d = rect.size().max_elem();
    radial(painter, rect.left_bottom() + vec2(rect.width() * 0.12, -rect.height() * 0.1), d * 0.55, BLOOM_WARM.gamma_multiply(0.07));
    radial(painter, rect.right_top() + vec2(-rect.width() * 0.15, rect.height() * 0.08), d * 0.5, BLOOM_COOL.gamma_multiply(0.08));
    // drifting dot grid in world space, spacing kept >= 28 screen points
    let mut step = 32.0;
    while step * xf.zoom < 28.0 {
        step *= 2.0;
    }
    while step * xf.zoom > 112.0 && step > 8.0 {
        step *= 0.5;
    }
    let w0 = xf.s2w(rect.min);
    let w1 = xf.s2w(rect.max);
    let dot = INK_3.gamma_multiply(0.35);
    let mut x = (w0.x / step).floor() * step;
    while x <= w1.x {
        let mut y = (w0.y / step).floor() * step;
        while y <= w1.y {
            painter.circle_filled(xf.w2s(pos2(x, y)), 1.0, dot);
            y += step;
        }
        x += step;
    }
}

// ---------------------------------------------------------------- icons

/// Draws the tool glyph for `tool` inside `r` (about 20x20).
pub fn tool_icon(p: &Painter, tool: crate::app::Tool, r: Rect, c: Color32) {
    use crate::app::Tool;
    let s = Stroke::new(1.6, c);
    let at = |x: f32, y: f32| pos2(r.left() + x * r.width(), r.top() + y * r.height());
    match tool {
        Tool::Select => {
            let pts = vec![at(0.25, 0.12), at(0.25, 0.82), at(0.43, 0.64), at(0.56, 0.9), at(0.66, 0.85), at(0.54, 0.6), at(0.78, 0.6)];
            p.add(egui::Shape::closed_line(pts, s));
        }
        Tool::Lasso => {
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = i as f32 / 20.0 * std::f32::consts::TAU;
                    at(0.52 + 0.34 * a.cos(), 0.40 + 0.24 * a.sin())
                })
                .collect();
            p.add(egui::Shape::line(pts, s));
            p.add(egui::Shape::line(vec![at(0.30, 0.56), at(0.24, 0.74), at(0.34, 0.86)], s));
        }
        Tool::Hand => {
            for (i, h) in [0.42f32, 0.28, 0.3, 0.42].iter().enumerate() {
                let x = 0.3 + i as f32 * 0.13;
                p.line_segment([at(x, *h), at(x, 0.6)], s);
            }
            p.add(egui::Shape::line(vec![at(0.3, 0.55), at(0.3, 0.72), at(0.45, 0.88), at(0.68, 0.88), at(0.73, 0.65), at(0.73, 0.5)], s));
            p.line_segment([at(0.3, 0.6), at(0.18, 0.5)], s);
        }
        Tool::Brush => {
            p.line_segment([at(0.82, 0.12), at(0.48, 0.5)], Stroke::new(2.2, c));
            p.add(egui::Shape::convex_polygon(vec![at(0.44, 0.48), at(0.54, 0.58), at(0.36, 0.84), at(0.16, 0.86), at(0.24, 0.66)], c, Stroke::NONE));
        }
        Tool::Pen => {
            let pts = vec![at(0.62, 0.14), at(0.86, 0.38), at(0.42, 0.8), at(0.16, 0.86), at(0.22, 0.6)];
            p.add(egui::Shape::closed_line(pts, s));
            p.line_segment([at(0.18, 0.84), at(0.4, 0.62)], s);
            p.circle_filled(at(0.43, 0.59), 1.6, c);
        }
        Tool::Eraser => {
            let pts = vec![at(0.56, 0.14), at(0.88, 0.46), at(0.5, 0.84), at(0.3, 0.84), at(0.14, 0.68)];
            p.add(egui::Shape::closed_line(pts, s));
            p.line_segment([at(0.35, 0.35), at(0.67, 0.67)], s);
            p.line_segment([at(0.3, 0.86), at(0.88, 0.86)], Stroke::new(1.2, c.gamma_multiply(0.6)));
        }
        Tool::Text => {
            p.line_segment([at(0.2, 0.2), at(0.8, 0.2)], Stroke::new(2.0, c));
            p.line_segment([at(0.5, 0.2), at(0.5, 0.84)], Stroke::new(2.0, c));
            p.line_segment([at(0.38, 0.84), at(0.62, 0.84)], s);
        }
        Tool::Line => {
            p.line_segment([at(0.18, 0.82), at(0.82, 0.18)], s);
            p.circle_filled(at(0.18, 0.82), 2.2, c);
            p.circle_filled(at(0.82, 0.18), 2.2, c);
        }
        Tool::Arrow => {
            p.line_segment([at(0.18, 0.82), at(0.76, 0.24)], s);
            p.add(egui::Shape::convex_polygon(vec![at(0.86, 0.14), at(0.8, 0.44), at(0.56, 0.2)], c, Stroke::NONE));
        }
        Tool::Rect => {
            p.rect_stroke(Rect::from_min_max(at(0.16, 0.24), at(0.84, 0.76)), 2.0, s, egui::StrokeKind::Middle);
        }
        Tool::Ellipse => {
            p.circle_stroke(r.center(), r.width() * 0.34, s);
        }
        Tool::Crop => {
            p.add(egui::Shape::line(vec![at(0.3, 0.1), at(0.3, 0.7), at(0.9, 0.7)], s));
            p.add(egui::Shape::line(vec![at(0.1, 0.3), at(0.7, 0.3), at(0.7, 0.9)], s));
        }
        Tool::Frame => {
            let k = 0.22;
            for (cx, cy, dx, dy) in [(0.14, 0.14, 1.0, 1.0), (0.86, 0.14, -1.0, 1.0), (0.86, 0.86, -1.0, -1.0), (0.14, 0.86, 1.0, -1.0)] {
                p.add(egui::Shape::line(vec![at(cx + dx * k, cy), at(cx, cy), at(cx, cy + dy * k)], s));
            }
            p.circle_filled(r.center(), 1.8, c);
        }
        Tool::Picker => {
            p.line_segment([at(0.16, 0.84), at(0.56, 0.44)], s);
            p.add(egui::Shape::convex_polygon(vec![at(0.5, 0.38), at(0.62, 0.5), at(0.86, 0.26), at(0.74, 0.14)], c, Stroke::NONE));
            p.circle_filled(at(0.15, 0.85), 1.8, c);
        }
    }
}

/// Small eye / lock glyphs for layer rows.
pub fn eye_icon(p: &Painter, c: Pos2, on: bool, col: Color32) {
    let s = Stroke::new(1.3, col);
    if on {
        let w = 7.0;
        let pts: Vec<Pos2> = (0..=16)
            .map(|i| {
                let t = i as f32 / 16.0 * std::f32::consts::TAU;
                c + vec2(t.cos() * w, t.sin() * w * 0.55)
            })
            .collect();
        p.add(egui::Shape::closed_line(pts, s));
        p.circle_filled(c, 2.4, col);
    } else {
        p.line_segment([c + vec2(-7.0, 0.0), c + vec2(7.0, 0.0)], s);
    }
}

pub fn lock_icon(p: &Painter, c: Pos2, col: Color32) {
    let s = Stroke::new(1.3, col);
    p.rect_filled(Rect::from_center_size(c + vec2(0.0, 2.0), vec2(10.0, 7.0)), 1.5, col);
    p.add(egui::Shape::line(
        (0..=10)
            .map(|i| {
                let t = std::f32::consts::PI + i as f32 / 10.0 * std::f32::consts::PI;
                c + vec2(0.0, -1.5) + vec2(t.cos() * 3.4, t.sin() * 3.6)
            })
            .collect(),
        s,
    ));
}

pub const ICON: Vec2 = vec2(20.0, 20.0);
