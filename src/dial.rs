//! "Focus ring" value control: a strip of ticks that rolls like a lens ring under
//! a fixed amber index line, with an editable number (maths allowed) and, for
//! values with a clear default, a small reset pin that appears once changed.

use std::ops::RangeInclusive;

use egui::{Align2, Color32, FontId, Pos2, Response, Sense, Shape, Stroke, Ui, Widget, pos2, vec2};

use crate::theme::{ACCENT, FILL_1, INK, INK_2, INK_3};

pub struct Dial<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    step: Option<f32>,
    text: String,
    suffix: String,
    default: Option<f32>,
    integer: bool,
    vertical: bool,
    key: Option<(KeyMark, &'a mut bool)>,
}

/// State of a keyable value at the playhead.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum KeyMark {
    /// Not animated.
    None,
    /// Animated, no key here.
    Keyed,
    /// A key sits at the playhead.
    Here,
}

const KEY_W: f32 = 16.0;

/// Small diamond that records the current value at the playhead.
fn draw_key(p: &egui::Painter, c: Pos2, mark: KeyMark, hot: bool) {
    let d = 4.5;
    let pts = vec![c + vec2(0.0, -d), c + vec2(d, 0.0), c + vec2(0.0, d), c + vec2(-d, 0.0)];
    match mark {
        KeyMark::Here => p.add(Shape::convex_polygon(pts, ACCENT, Stroke::NONE)),
        KeyMark::Keyed => p.add(Shape::closed_line(pts, Stroke::new(1.3, ACCENT))),
        KeyMark::None => p.add(Shape::closed_line(pts, Stroke::new(1.2, if hot { INK() } else { INK_3() }))),
    };
}

fn label_width(text: &str, key: bool) -> f32 {
    match (text.is_empty(), key) {
        (true, true) => KEY_W,
        (true, false) => 0.0,
        (false, k) => 58.0 + if k { 12.0 } else { 0.0 },
    }
}

/// The label slot of a dial row (text, then the key button when keyable), for
/// rows built from other widgets that should line up with the dials.
pub fn label_cell(ui: &mut Ui, text: &str, mark: Option<KeyMark>, hit: &mut bool) {
    let w = label_width(text, mark.is_some());
    let (lr, lresp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    let text_r = if mark.is_some() { egui::Rect::from_min_max(lr.min, egui::pos2(lr.right() - KEY_W, lr.bottom())) } else { lr };
    ui.painter().with_clip_rect(text_r).text(lr.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(crate::theme::TEXT_SMALL + 0.5), INK_2());
    if let Some(mark) = mark {
        let kr = egui::Rect::from_min_max(egui::pos2(lr.right() - KEY_W, lr.top()), lr.max);
        let kresp = ui.interact(kr, lresp.id.with("key"), Sense::click());
        if kresp.hovered() {
            ui.painter().circle_filled(kr.center(), 8.0, FILL_1());
        }
        draw_key(ui.painter(), kr.center(), mark, kresp.hovered());
        if kresp.on_hover_text("키 기록").clicked() {
            *hit = true;
        }
    }
}

/// A stand-alone key button (rows that are not dials). True when clicked.
pub fn key_button(ui: &mut Ui, mark: KeyMark) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(KEY_W, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().circle_filled(r.center(), 8.0, FILL_1());
    }
    draw_key(ui.painter(), r.center(), mark, resp.hovered());
    resp.on_hover_text("키 기록").clicked()
}

impl<'a> Dial<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>) -> Self {
        Self { value, range, step: None, text: String::new(), suffix: String::new(), default: None, integer: false, vertical: false, key: None }
    }

    /// Value per tick. Every tick is worth the same amount so the ring reads like a
    /// ruler; defaults to a 1-2-5 step giving about 60 ticks over the range.
    pub fn step(mut self, st: f32) -> Self {
        self.step = Some(st);
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

    /// Writes whole numbers only; the ring still turns smoothly underneath.
    pub fn integer(mut self) -> Self {
        self.integer = true;
        self
    }

    /// A narrow upright dial: the ring rolls vertically, value and label below.
    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }

    /// A key button before the ring; `hit` turns true when it is clicked.
    pub fn key(mut self, mark: Option<KeyMark>, hit: &'a mut bool) -> Self {
        self.key = mark.map(|m| (m, hit));
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

/// Nearest 1-2-5 × 10^n step to `x`.
fn nice_step(x: f32) -> f32 {
    let e = 10f32.powf(x.max(1e-9).log10().floor());
    let m = x / e;
    let n = if m < 1.5 { 1.0 } else if m < 3.5 { 2.0 } else if m < 7.5 { 5.0 } else { 10.0 };
    n * e
}

/// The 1-2-5 step one below `x` (5 -> 2 -> 1 -> 0.5 ...).
fn nice_down(x: f32) -> f32 {
    let e = 10f32.powf(x.max(1e-9).log10().floor());
    let m = (x / e).round();
    if m >= 10.0 { 5.0 * e } else if m >= 5.0 { 2.0 * e } else if m >= 2.0 { e } else { 0.5 * e }
}

/// Is `v` a whole multiple of `m`?
fn on_grid(v: f32, m: f32) -> bool {
    let k = v / m;
    (k - k.round()).abs() < 1e-3
}

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
        if self.vertical {
            return vertical_ui(self, ui);
        }
        let Dial { value, range, step, text, suffix, default, integer, key, .. } = self;
        let (lo, hi) = (*range.start(), *range.end());
        let span = (hi - lo).max(1e-6);
        // value per tick; the ring moves TICK_PX per step so ticks track the pointer.
        // An automatic step is refined (1-2-5 down) until the default lands on a
        // major tick of the zero-based grid, e.g. 0..400 with default 20 -> 2/tick.
        let dt = step.unwrap_or_else(|| {
            let mut st = nice_step(span / TICKS_PER_RANGE);
            if let Some(d) = default {
                for _ in 0..4 {
                    if on_grid(d, st * 5.0) {
                        break;
                    }
                    st = nice_down(st);
                }
            }
            if integer { st.max(1.0) } else { st }
        });
        // ticks count from 0 so majors fall on round values; only when the default
        // is off that grid do they count from the default instead
        let anchor = match default {
            Some(d) if !on_grid(d, dt * 5.0) => d,
            _ => 0.0,
        };
        let before = *value;

        let decimals = if dt >= 0.999 { 0 } else if dt >= 0.0999 { 1 } else { 2 };
        let width = ui.available_width();
        // the row is as tall as a number box, so ring, label and box share one centre
        let row_h = ui.spacing().interact_size.y.max(24.0);
        let mut out = ui
            .allocate_ui_with_layout(vec2(width, row_h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                // a fixed-height row, so every item centres on the same line
                ui.spacing_mut().item_spacing.x = 6.0;
                let label_w = label_width(&text, key.is_some());
                if label_w > 0.0 {
                    let mut dummy = false;
                    let (mark, hit) = match key {
                        Some((m, h)) => (Some(m), h),
                        None => (None, &mut dummy),
                    };
                    label_cell(ui, &text, mark, hit);
                }
                let value_w = 52.0 + if suffix.is_empty() { 0.0 } else { 14.0 };
                let pin_w = if default.is_some() { 16.0 } else { 0.0 };
                let ring_w = (width - label_w - value_w - pin_w - 18.0).max(60.0);

                // ---- the ring
                let (r, resp) = ui.allocate_exact_size(vec2(ring_w, row_h - 4.0), Sense::click_and_drag());
                // while dragging, keep turning from the unrounded position as long as
                // nobody else changed the value (integer dials and callers that
                // round would otherwise eat sub-step motion and stutter)
                let mem = resp.id.with("raw");
                let mut u = value.clamp(lo, hi);
                if resp.dragged() {
                    if let Some((raw, wrote)) = ui.data(|d| d.get_temp::<(f32, f32)>(mem)) {
                        if (wrote - *value).abs() <= dt * 1e-3 {
                            u = raw;
                        }
                    }
                }
                let mut moved = false;
                if resp.dragged() && resp.drag_delta().x != 0.0 {
                    moved = true;
                    let fine = if ui.input(|i| i.modifiers.shift) { 0.2 } else { 1.0 };
                    u = (u - resp.drag_delta().x / TICK_PX * dt * fine).clamp(lo, hi);
                }
                if resp.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
                    if scroll != 0.0 {
                        moved = true;
                        // one tick per notch, snapped onto the tick grid
                        u = (anchor + (((u - anchor) / dt).round() + scroll.signum()) * dt).clamp(lo, hi);
                    }
                }
                let nv = if integer { u.round().clamp(lo, hi) } else { u };
                // only a real drag or scroll writes, so hovering never nudges the value
                if moved && (nv - *value).abs() > f32::EPSILON {
                    *value = nv;
                }
                if resp.dragged() {
                    ui.data_mut(|d| d.insert_temp(mem, (u, *value)));
                }
                let p = ui.painter().with_clip_rect(r);
                // ticks on a cylinder: angle per tick so neighbours are TICK_PX apart at
                // the centre, x = R·sin(angle), fading towards the sides
                let radius = r.width() * 0.5;
                // pixels per unit stay TICK_PX/dt so the ticks follow the pointer 1:1
                let ang_per_u = TICK_PX / radius / dt;
                let (vis_lo, vis_hi) = (u - 1.55 / ang_per_u, u + 1.55 / ang_per_u);
                // evenly spaced ticks worth `dt` each, major every 5 (....|....|), counted
                // from the anchor; the range start may fall anywhere between ticks
                let mut ticks: Vec<(f32, u8)> = Vec::new(); // (value, 0 minor / 2 major)
                let (k0, k1) = (((vis_lo.max(lo) - anchor) / dt).floor() as i64, ((vis_hi.min(hi) - anchor) / dt).ceil() as i64);
                for k in k0..=k1 {
                    let t = anchor + k as f32 * dt;
                    if t >= lo - dt * 1e-3 && t <= hi + dt * 1e-3 {
                        ticks.push((t, if k.rem_euclid(5) == 0 { 2 } else { 0 }));
                    }
                }
                let y = r.center().y;
                let draw = |t: f32, kind: u8, end: bool| {
                    let a = (t - u) * ang_per_u;
                    if a.abs() > 1.5 {
                        return;
                    }
                    let x = r.center().x + radius * a.sin();
                    let fade = a.cos().powf(1.6);
                    let h = r.height() * if end { 0.62 } else { [0.26, 0.36, 0.46][kind as usize] };
                    let col = (if end { INK_2() } else if kind == 2 { INK_3().gamma_multiply(1.6) } else { INK_3() }).gamma_multiply(fade);
                    p.line_segment([pos2(x, y - h * 0.5), pos2(x, y + h * 0.5)], Stroke::new(if kind > 0 || end { 1.3 } else { 1.0 }, col));
                };
                for (t, kind) in ticks {
                    draw(t, kind, false);
                }
                // the real limits get end stops
                draw(lo, 2, true);
                draw(hi, 2, true);
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
                    .speed(dt as f64 * 0.5)
                    .fixed_decimals(decimals)
                    .custom_parser(crate::calc::parse);
                if !suffix.is_empty() {
                    dv = dv.suffix(suffix.clone());
                }
                ui.spacing_mut().interact_size.x = value_w;
                let vr = ui.add(dv);

                // ---- reset pin
                if let Some(d) = default {
                    let (pr, presp) = ui.allocate_exact_size(vec2(pin_w, row_h), Sense::click());
                    let off = (*value - d).abs() > (hi - lo).abs() * 1e-4;
                    if off {
                        let col = if presp.hovered() { INK() } else { INK_3() };
                        if presp.hovered() {
                            ui.painter().circle_filled(pr.center(), 8.5, FILL_1());
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
    let r = ui.add(Dial::new(&mut f, *range.start() as f32..=*range.end() as f32).integer().text(text));
    *v = f.round().clamp(*range.start() as f32, *range.end() as f32) as u8;
    r
}

/// Upright variant: pin, ring (dragging up raises the value), number, label.
fn vertical_ui(d: Dial<'_>, ui: &mut Ui) -> Response {
    let Dial { value, range, step, text, suffix, default, integer, .. } = d;
    let (lo, hi) = (*range.start(), *range.end());
    let span = (hi - lo).max(1e-6);
    let dt = step.unwrap_or_else(|| {
        let mut st = nice_step(span / TICKS_PER_RANGE);
        if let Some(dv) = default {
            for _ in 0..4 {
                if on_grid(dv, st * 5.0) {
                    break;
                }
                st = nice_down(st);
            }
        }
        if integer { st.max(1.0) } else { st }
    });
    let anchor = match default {
        Some(dv) if !on_grid(dv, dt * 5.0) => dv,
        _ => 0.0,
    };
    let decimals = if dt >= 0.999 { 0 } else if dt >= 0.0999 { 1 } else { 2 };
    let before = *value;
    let w = 40.0;
    let ring_w = 24.0;
    let ring_h = 92.0;
    let mut out = ui
        .vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.spacing_mut().button_padding = vec2(2.0, 1.0);
            ui.style_mut().override_font_id = Some(FontId::proportional(crate::theme::TEXT_SMALL));
            ui.set_width(w);
            // reset pin (keeps its row so dials line up)
            let (pr, presp) = ui.allocate_exact_size(vec2(w, 14.0), Sense::click());
            if let Some(dv) = default {
                if (*value - dv).abs() > span * 1e-4 {
                    if presp.hovered() {
                        ui.painter().circle_filled(pr.center(), 7.0, FILL_1());
                    }
                    reset_icon(ui.painter(), pr.center(), if presp.hovered() { INK() } else { INK_3() });
                    if presp.on_hover_text(format!("기본값 ({dv})")).clicked() {
                        *value = dv;
                    }
                }
            }
            // ring
            let (full, resp) = ui.allocate_exact_size(vec2(w, ring_h), Sense::click_and_drag());
            let r = egui::Rect::from_center_size(full.center(), vec2(ring_w, ring_h));
            let mut u = value.clamp(lo, hi);
            let mut moved = false;
            if resp.dragged() && resp.drag_delta().y != 0.0 {
                moved = true;
                let fine = if ui.input(|i| i.modifiers.shift) { 0.2 } else { 1.0 };
                u = (u - resp.drag_delta().y / TICK_PX * dt * fine).clamp(lo, hi);
            }
            if resp.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
                if scroll != 0.0 {
                    moved = true;
                    u = (anchor + (((u - anchor) / dt).round() + scroll.signum()) * dt).clamp(lo, hi);
                }
            }
            let nv = if integer { u.round().clamp(lo, hi) } else { u };
            if moved && (nv - *value).abs() > f32::EPSILON {
                *value = nv;
            }
            let p = ui.painter().with_clip_rect(r.expand2(vec2(4.0, 0.0)));
            let radius = r.height() * 0.5;
            let ang_per_u = TICK_PX / radius / dt;
            let (vis_lo, vis_hi) = (u - 1.55 / ang_per_u, u + 1.55 / ang_per_u);
            let (k0, k1) = (((vis_lo.max(lo) - anchor) / dt).floor() as i64, ((vis_hi.min(hi) - anchor) / dt).ceil() as i64);
            let cx = r.center().x;
            let draw = |t: f32, major: bool, end: bool| {
                let a = (t - u) * ang_per_u;
                if a.abs() > 1.5 {
                    return;
                }
                // higher values sit below, so the ring moves with the pointer
                let y = r.center().y + radius * a.sin();
                let fade = a.cos().powf(1.6);
                let half = r.width() * if end { 0.48 } else if major { 0.36 } else { 0.2 };
                let col = (if end { INK_2() } else if major { INK_3().gamma_multiply(1.6) } else { INK_3() }).gamma_multiply(fade);
                p.line_segment([pos2(cx - half, y), pos2(cx + half, y)], Stroke::new(if major || end { 1.3 } else { 1.0 }, col));
            };
            for k in k0..=k1 {
                let t = anchor + k as f32 * dt;
                if t >= lo - dt * 1e-3 && t <= hi + dt * 1e-3 {
                    draw(t, k.rem_euclid(5) == 0, false);
                }
            }
            draw(lo, true, true);
            draw(hi, true, true);
            let cy = r.center().y;
            p.line_segment([pos2(r.left() + 2.0, cy), pos2(r.right() - 2.0, cy)], Stroke::new(1.6, ACCENT));
            p.add(Shape::convex_polygon(vec![pos2(r.left() - 3.0, cy - 2.5), pos2(r.left() - 3.0, cy + 2.5), pos2(r.left() + 1.0, cy)], ACCENT, Stroke::NONE));
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
            // number and label
            let mut dv = egui::DragValue::new(value).range(lo..=hi).speed(dt as f64 * 0.5).fixed_decimals(decimals).custom_parser(crate::calc::parse);
            if !suffix.is_empty() {
                dv = dv.suffix(suffix.clone());
            }
            let vr = ui.add_sized(vec2(w, 18.0), dv);
            let (lr, _) = ui.allocate_exact_size(vec2(w, 13.0), Sense::hover());
            ui.painter().with_clip_rect(lr.expand2(vec2(6.0, 0.0))).text(lr.center(), Align2::CENTER_CENTER, &text, FontId::proportional(crate::theme::TEXT_SMALL - 0.5), INK_2());
            resp.union(vr)
        })
        .inner;
    if (*value - before).abs() > f32::EPSILON {
        out.mark_changed();
    }
    out
}
