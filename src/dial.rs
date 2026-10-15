//! "Focus ring" value control: a strip of ticks that rolls like a lens ring under
//! a fixed amber index line, with an editable number (maths allowed) and, for
//! values with a clear default, a small reset pin that appears once changed.

use std::ops::RangeInclusive;

use egui::{Align2, Color32, FontId, Pos2, Response, Sense, Shape, Stroke, Ui, Widget, pos2, vec2};

use crate::theme::{ACCENT, FILL_1, INK, INK_2, INK_3};

pub struct Dial<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    log: bool,
    text: String,
    suffix: String,
    default: Option<f32>,
}

impl<'a> Dial<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>) -> Self {
        Self { value, range, log: false, text: String::new(), suffix: String::new(), default: None }
    }

    pub fn logarithmic(mut self, on: bool) -> Self {
        self.log = on;
        self
    }

    pub fn text(mut self, t: impl Into<String>) -> Self {
        self.text = t.into();
        self
    }

    pub fn suffix(mut self, s: impl Into<String>) -> Self {
        self.suffix = s.into();
        self
    }

    /// Shows the reset pin when the value differs from `d`.
    pub fn default(mut self, d: f32) -> Self {
        self.default = Some(d);
        self
    }
}

const TICK_PX: f32 = 7.0;
const TICKS_PER_RANGE: f32 = 60.0;

/// Small "reset" glyph: an open circular arrow.
fn reset_icon(p: &egui::Painter, c: Pos2, col: Color32) {
    let r = 5.0;
    let pts: Vec<Pos2> = (0..=18)
        .map(|i| {
            let a = -0.6 + i as f32 / 18.0 * 4.9;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    let tip = pts[0];
    p.add(Shape::line(pts, Stroke::new(1.4, col)));
    // arrow head at the start of the arc, pointing along it
    let dir = vec2((-0.6f32).sin(), -(-0.6f32).cos());
    let n = vec2(-dir.y, dir.x);
    p.add(Shape::convex_polygon(vec![tip + dir * 3.2, tip - n * 2.6, tip + n * 2.6], col, Stroke::NONE));
}

impl Widget for Dial<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Dial { value, range, log, text, suffix, default } = self;
        let (lo, hi) = (*range.start(), *range.end());
        let to_u = |v: f32| if log { v.max(1e-6).ln() } else { v };
        let from_u = |u: f32| if log { u.exp() } else { u };
        let (ulo, uhi) = (to_u(lo), to_u(hi));
        let span = (uhi - ulo).max(1e-6);
        let dt = span / TICKS_PER_RANGE;
        let before = *value;

        let decimals = if log || span > 50.0 { 0 } else if span > 5.0 { 1 } else { 2 };
        let width = ui.available_width();
        let row_h = 24.0;
        let mut out = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let label_w = if text.is_empty() { 0.0 } else { 58.0 };
                if label_w > 0.0 {
                    let (lr, _) = ui.allocate_exact_size(vec2(label_w, row_h), Sense::hover());
                    ui.painter().with_clip_rect(lr).text(lr.left_center(), Align2::LEFT_CENTER, &text, FontId::proportional(crate::theme::TEXT_SMALL + 0.5), INK_2);
                }
                let value_w = 52.0 + if suffix.is_empty() { 0.0 } else { 14.0 };
                let pin_w = if default.is_some() { 16.0 } else { 0.0 };
                let ring_w = (width - label_w - value_w - pin_w - 18.0).max(60.0);

                // ---- the ring
                let (r, resp) = ui.allocate_exact_size(vec2(ring_w, row_h - 4.0), Sense::click_and_drag());
                let mut u = to_u(*value).clamp(ulo, uhi);
                if resp.dragged() {
                    let fine = if ui.input(|i| i.modifiers.shift) { 0.2 } else { 1.0 };
                    u = (u - resp.drag_delta().x / TICK_PX * dt * fine).clamp(ulo, uhi);
                }
                if resp.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
                    if scroll != 0.0 {
                        u = (u + scroll.signum() * dt).clamp(ulo, uhi);
                    }
                }
                let nv = from_u(u).clamp(lo, hi);
                if (nv - *value).abs() > f32::EPSILON && (resp.dragged() || resp.hovered()) {
                    *value = nv;
                }
                let p = ui.painter().with_clip_rect(r);
                // ticks on a cylinder: angle per tick so neighbours are TICK_PX apart at
                // the centre, x = R·sin(angle), fading towards the sides
                let radius = r.width() * 0.5;
                let ang_per_u = TICK_PX / radius / dt;
                let first = ((u - 1.6 / ang_per_u - ulo) / dt).floor() as i64;
                let last = ((u + 1.6 / ang_per_u - ulo) / dt).ceil() as i64;
                for k in first.max(0)..=last.min(TICKS_PER_RANGE.round() as i64) {
                    let tu = ulo + k as f32 * dt;
                    let a = (tu - u) * ang_per_u;
                    if a.abs() > 1.5 {
                        continue;
                    }
                    let x = r.center().x + radius * a.sin();
                    let fade = a.cos().powf(1.6);
                    let major = k % 5 == 0;
                    let end = k == 0 || k == TICKS_PER_RANGE.round() as i64;
                    let h = if end { r.height() * 0.62 } else if major { r.height() * 0.46 } else { r.height() * 0.26 };
                    let col = (if end { INK_2 } else if major { INK_3.gamma_multiply(1.6) } else { INK_3 }).gamma_multiply(fade);
                    let y = r.center().y;
                    p.line_segment([pos2(x, y - h * 0.5), pos2(x, y + h * 0.5)], Stroke::new(if major || end { 1.3 } else { 1.0 }, col));
                }
                // fixed index line
                let cx = r.center().x;
                p.line_segment([pos2(cx, r.top() + 2.0), pos2(cx, r.bottom() - 2.0)], Stroke::new(2.0, ACCENT));
                p.add(Shape::convex_polygon(vec![pos2(cx - 3.5, r.top()), pos2(cx + 3.5, r.top()), pos2(cx, r.top() + 4.0)], ACCENT, Stroke::NONE));
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }

                // ---- number (typing and maths)
                let mut dv = egui::DragValue::new(value)
                    .range(lo..=hi)
                    .speed(dt as f64 * if log { 1.0 } else { 0.5 })
                    .fixed_decimals(decimals)
                    .custom_parser(crate::calc::parse);
                if !suffix.is_empty() {
                    dv = dv.suffix(suffix.clone());
                }
                let vr = ui.add_sized(vec2(value_w, row_h - 4.0), dv);

                // ---- reset pin
                if let Some(d) = default {
                    let (pr, presp) = ui.allocate_exact_size(vec2(pin_w, row_h), Sense::click());
                    let off = (*value - d).abs() > (hi - lo).abs() * 1e-4;
                    if off {
                        let col = if presp.hovered() { INK } else { INK_3 };
                        if presp.hovered() {
                            ui.painter().circle_filled(pr.center(), 8.5, FILL_1);
                        }
                        reset_icon(ui.painter(), pr.center(), col);
                        if presp.on_hover_text(format!("기본값으로 ({d})")).clicked() {
                            *value = d;
                        }
                    }
                }
                resp.union(vr)
            })
            .inner;
        if (*value - before).abs() > f32::EPSILON {
            out.mark_changed();
        }
        out
    }
}

/// Integer convenience for u8 settings (e.g. JPEG quality).
pub fn dial_u8(ui: &mut Ui, v: &mut u8, range: RangeInclusive<u8>, text: &str) -> Response {
    let mut f = *v as f32;
    let r = ui.add(Dial::new(&mut f, *range.start() as f32..=*range.end() as f32).text(text));
    *v = f.round().clamp(*range.start() as f32, *range.end() as f32) as u8;
    r
}
