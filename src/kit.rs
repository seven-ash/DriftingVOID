//! The controls kit (UI overhaul phase 1, `docs/updates/ui-overhaul.md`): one
//! control per job on the shared row grid (`theme::ROW_H`). Sections move onto
//! it in phase 4; until then `DV_KIT=1` shows every piece in every state.
//!
//! State colours (Ash, 10-22): off in the theme's ink, on in the accent, and
//! unavailable faded but still visible in both themes.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::theme::{self, ACCENT, ACCENT_INK, FILL_1, FILL_2, FILL_3, INK, INK_2, INK_3, ROW_H, SPACE_SM, SPACE_XS};

/// Ink of a state glyph: unavailable, on, or off.
pub fn state_ink(on: bool, enabled: bool) -> Color32 {
    if !enabled {
        INK().gamma_multiply(0.38)
    } else if on {
        ACCENT
    } else {
        INK()
    }
}

/// The accent ring of a control that has keyboard focus.
fn focus_ring(ui: &Ui, resp: &Response, r: Rect, radius: f32) {
    if resp.has_focus() {
        ui.painter().rect_stroke(r.expand(2.0), CornerRadius::same((radius + 2.0) as u8), Stroke::new(1.5, ACCENT), StrokeKind::Outside);
    }
}

/// An on/off switch (replaces ticks and round dots, Ash 10-27): a small
/// capsule, knob left and ink when off, right and accent when on, then the
/// label. True in `changed()` when clicked.
pub fn switch(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    switch_mixed(ui, on, text, false)
}

/// `switch` for a multi-selection: `mixed` when the selected layers differ,
/// shown with the knob in the middle (as "—" in number boxes, 0.76.0).
pub fn switch_mixed(ui: &mut Ui, on: &mut bool, text: &str, mixed: bool) -> Response {
    let track = vec2(26.0, 14.0);
    let font = egui::TextStyle::Body.resolve(ui.style());
    // a long label is cut with "…", never widening its panel
    let room = (ui.available_width() - track.x - SPACE_SM).max(20.0);
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, INK());
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let galley = ui.painter().layout_job(job);
    let w = track.x + if text.is_empty() { 0.0 } else { SPACE_SM + galley.size().x };
    let (r, mut resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let enabled = ui.is_enabled();
    let t = if mixed { 0.5 } else { ui.ctx().animate_bool_responsive(resp.id, *on) };
    let on_look = *on && !mixed;
    let tr = Rect::from_min_size(pos2(r.left(), r.center().y - track.y * 0.5), track);
    let rad = track.y * 0.5;
    let p = ui.painter();
    let ink = state_ink(on_look, enabled);
    if on_look {
        p.rect_filled(tr, CornerRadius::same(rad as u8), if enabled { ACCENT } else { ACCENT.gamma_multiply(0.38) });
    } else {
        if mixed {
            // mixed: a grey track, so it never reads as off (Ash, 10-28)
            p.rect_filled(tr, CornerRadius::same(rad as u8), INK_3().gamma_multiply(if enabled { 0.75 } else { 0.35 }));
        } else if resp.hovered() && enabled {
            p.rect_filled(tr, CornerRadius::same(rad as u8), FILL_2());
        }
        p.rect_stroke(tr, CornerRadius::same(rad as u8), Stroke::new(1.3, ink), StrokeKind::Inside);
    }
    let knob_r = rad - 3.0;
    let kx = egui::lerp((tr.left() + rad)..=(tr.right() - rad), t);
    let knob = if on_look { if enabled { ACCENT_INK } else { ACCENT_INK.gamma_multiply(0.5) } } else { ink };
    p.circle_filled(pos2(kx, tr.center().y), knob_r, knob);
    if !text.is_empty() {
        let col = if enabled { INK() } else { INK_2() };
        p.galley_with_override_text_color(pos2(tr.right() + SPACE_SM, r.center().y - galley.size().y * 0.5), galley, col);
    }
    focus_ring(ui, &resp, tr, rad);
    resp
}

/// One of 2–5 short options in a capsule (replaces radio dots and chip rows,
/// Ash 10-27). The chosen segment is filled with the accent; its corners are
/// concentric with the capsule's (Ash, 10-27).
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, opts: &[(T, &str)]) -> Response {
    let opts: Vec<(T, &str, bool)> = opts.iter().map(|(v, s)| (*v, *s, true)).collect();
    segmented_en(ui, value, &opts)
}

/// `segmented` where an option can be unavailable (faded, not clickable),
/// e.g. 내보낼 영역 while there is no export frame.
pub fn segmented_en<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, opts: &[(T, &str, bool)]) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let pad = 10.0;
    let inset = 2.0;
    let galleys: Vec<_> = opts.iter().map(|(_, s, _)| ui.painter().layout_no_wrap((*s).to_owned(), font.clone(), INK())).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + pad * 2.0).collect();
    let total = widths.iter().sum::<f32>() + inset * 2.0;
    let (r, mut resp) = ui.allocate_exact_size(vec2(total, ROW_H), Sense::hover());
    let enabled = ui.is_enabled();
    let outer = ROW_H * 0.5;
    let inner = outer - inset;
    ui.painter().rect_filled(r, CornerRadius::same(outer as u8), FILL_1());
    let mut x = r.left() + inset;
    for (i, ((v, _, on), g)) in opts.iter().zip(galleys).enumerate() {
        let sr = Rect::from_min_size(pos2(x, r.top() + inset), vec2(widths[i], ROW_H - inset * 2.0));
        x += widths[i];
        // one option unavailable (내보낼 영역 without a frame): sunk in a dark
        // fill with faint text, so it reads as off limits at once (Ash, 10-28)
        let blocked = enabled && !*on;
        let enabled = enabled && *on;
        let sresp = ui.interact(sr, resp.id.with(i), if enabled { Sense::click() } else { Sense::hover() });
        let chosen = *v == *value;
        if sresp.clicked() && !chosen {
            *value = *v;
            resp.mark_changed();
        }
        let p = ui.painter();
        let text_col = if chosen {
            if enabled { ACCENT_INK } else { ACCENT_INK.gamma_multiply(0.6) }
        } else if blocked {
            INK().gamma_multiply(0.3)
        } else if enabled {
            INK()
        } else {
            INK_2()
        };
        if blocked {
            p.rect_filled(sr, CornerRadius::same(inner as u8), Color32::from_black_alpha(if theme::is_light() { 30 } else { 80 }));
        }
        if chosen {
            p.rect_filled(sr, CornerRadius::same(inner as u8), if enabled { ACCENT } else { ACCENT.gamma_multiply(0.38) });
        } else if sresp.is_pointer_button_down_on() && enabled {
            p.rect_filled(sr, CornerRadius::same(inner as u8), FILL_3());
        } else if sresp.hovered() && enabled {
            p.rect_filled(sr, CornerRadius::same(inner as u8), FILL_2());
        }
        p.galley_with_override_text_color(sr.center() - g.size() * 0.5, g, text_col);
        focus_ring(ui, &sresp, sr, inner);
        resp = resp.union(sresp);
    }
    resp
}

/// A dialog's title (also for dialogs closed by their own 취소).
pub fn dialog_title(title: &str) -> egui::RichText {
    egui::RichText::new(title).font(FontId::new(15.0, theme::ui_bold())).color(INK())
}

/// A dialog's header row (Ash, 10-27, Q5: one look for every dialog): the
/// title left, 닫기 right. True when 닫기 is clicked.
pub fn dialog_header(ui: &mut Ui, title: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(dialog_title(title));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = ui.small_button("닫기").clicked();
        });
    });
    ui.add_space(SPACE_SM);
    close
}

/// The one primary action of a dialog (내보내기…, 저장): filled with the
/// accent (Ash, 10-27).
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let enabled = ui.is_enabled();
    let fill = if enabled { ACCENT } else { ACCENT.gamma_multiply(0.38) };
    let b = egui::Button::new(egui::RichText::new(text).color(if enabled { ACCENT_INK } else { ACCENT_INK.gamma_multiply(0.6) })).fill(fill).min_size(vec2(0.0, ROW_H));
    ui.add(b)
}

/// Glyphs for icon buttons.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    Up,
    Down,
    Duplicate,
    Delete,
    /// A new FX layer: a four-point spark.
    Fx,
}

fn draw_icon(p: &egui::Painter, icon: Icon, c: Pos2, col: Color32) {
    let s = Stroke::new(1.5, col);
    match icon {
        Icon::Up | Icon::Down => {
            let d = if icon == Icon::Up { -1.0 } else { 1.0 };
            p.line_segment([c + vec2(0.0, -6.0 * d), c + vec2(0.0, 6.0 * d)], s);
            p.add(Shape::line(vec![c + vec2(-4.5, 1.5 * d), c + vec2(0.0, 6.0 * d), c + vec2(4.5, 1.5 * d)], s));
        }
        Icon::Duplicate => {
            p.rect_stroke(Rect::from_min_size(c + vec2(-6.0, -6.0), vec2(9.0, 9.0)), 2.0, s, StrokeKind::Middle);
            p.rect_stroke(Rect::from_min_size(c + vec2(-3.0, -3.0), vec2(9.0, 9.0)), 2.0, s, StrokeKind::Middle);
        }
        Icon::Delete => {
            p.line_segment([c + vec2(-6.0, -4.0), c + vec2(6.0, -4.0)], s);
            p.line_segment([c + vec2(-2.0, -6.5), c + vec2(2.0, -6.5)], s);
            p.add(Shape::closed_line(vec![c + vec2(-4.5, -4.0), c + vec2(-3.5, 6.5), c + vec2(3.5, 6.5), c + vec2(4.5, -4.0)], s));
        }
        Icon::Fx => fx_glyph(p, c, 7.0, s),
    }
}

// ---------------------------------------------------------------- drop-downs
//
// Every drop-down (Ash, 10-28): the wheel over a closed one steps to the
// option before or after and applies it at once; pressing it and dragging
// down past its bottom edge opens the list (Ash, 10-29: no waiting), and
// releasing on an option picks it (elsewhere: nothing; on the box: the list
// stays open); a plain click opens the list as before.

/// Wheel travel (points) per option step; a mouse notch is 50.
const WHEEL_STEP: f32 = 40.0;

fn dd_ids(ui: &Ui, salt: impl egui::AsIdSalt) -> (egui::Id, egui::Id) {
    let button = ui.make_persistent_id(egui::IdSalt::new(salt));
    (button.with("popup"), button.with("press_drag"))
}

/// The press-drag state of the drop-down `salt` (read inside its list).
pub fn dd_dragging(ui: &Ui, salt: impl egui::AsIdSalt) -> egui::Id {
    dd_ids(ui, salt).1
}

/// A row of an open list: true when a press-drag from the box is released
/// on it (`drag` from `dd_dragging`, taken before the ComboBox). Lit while
/// the pointer is over it (other widgets get no hover during a drag).
pub fn dd_row_picked(ui: &Ui, drag: egui::Id, row: &Response) -> bool {
    if !ui.data(|d| d.get_temp::<bool>(drag)).unwrap_or(false) {
        return false;
    }
    let (pos, released) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_released()));
    let over = pos.is_some_and(|p| row.rect.contains(p));
    if over {
        ui.painter().rect_filled(row.rect, CornerRadius::same(theme::inner(theme::radius_card(), SPACE_SM)), FILL_2());
    }
    over && released
}

/// After a ComboBox (its button `resp`): the wheel step (−1 before, +1
/// after) when it turns over the closed box, and the press-and-hold list.
pub fn dd_button(ui: &Ui, salt: impl egui::AsIdSalt + Clone, resp: &Response) -> i32 {
    let (popup, drag) = dd_ids(ui, salt);
    let ctx = ui.ctx();
    let open = egui::Popup::is_id_open(ctx, popup);
    let dragging = ui.data(|d| d.get_temp::<bool>(drag)).unwrap_or(false);
    // pressed on the box and dragged down out of it: the list opens under it
    // pressed on the box and dragged down out of it: the list opens under
    // it. The press is remembered here: egui lets go of a click-only widget
    // as soon as a press on it moves.
    let press = drag.with("press");
    let (pressed_now, down, pos) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.latest_pos()));
    if pressed_now && resp.contains_pointer() {
        ui.data_mut(|d| d.insert_temp(press, true));
    }
    let pressed = down && ui.data(|d| d.get_temp::<bool>(press)).unwrap_or(false);
    if !down {
        ui.data_mut(|d| d.remove::<bool>(press));
    }
    if !open && !dragging && pressed && pos.is_some_and(|p| p.y > resp.rect.bottom()) {
        egui::Popup::open_id(ctx, popup);
        ui.data_mut(|d| {
            d.insert_temp(drag, true);
            d.remove::<bool>(press);
        });
    }
    // the press that opened it ends: on the box the list stays, elsewhere it
    // closes (a row under the pointer was picked in the list)
    if dragging && ui.input(|i| i.pointer.primary_released()) {
        ui.data_mut(|d| d.remove::<bool>(drag));
        let on_box = ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| resp.rect.contains(p));
        if on_box {
            egui::Popup::open_id(ctx, popup);
        } else {
            egui::Popup::close_id(ctx, popup);
        }
    }
    // the wheel over the closed box steps the choice
    if !open && resp.hovered() {
        // this frame's wheel events (egui smooths what it passes on)
        let dy: f32 = ui.input(|i| {
            i.events
                .iter()
                .map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => {
                        let k = match unit {
                            egui::MouseWheelUnit::Point => 1.0,
                            egui::MouseWheelUnit::Line => 50.0,
                            egui::MouseWheelUnit::Page => 500.0,
                        };
                        (delta.y + delta.x) * k
                    }
                    _ => 0.0,
                })
                .sum()
        });
        // the panel under it does not scroll too (nor later, from the smoothing)
        ctx.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        if dy != 0.0 {
            let acc_id = drag.with("wheel");
            let mut acc = ui.data(|d| d.get_temp::<f32>(acc_id)).unwrap_or(0.0) + dy;
            let mut step = 0;
            // a mouse notch is one step; a touchpad's small deltas add up
            if acc.abs() >= WHEEL_STEP || (acc.abs() > 0.0 && dy.abs() >= WHEEL_STEP * 0.5) {
                step = if acc < 0.0 { 1 } else { -1 };
                acc = 0.0;
            }
            ui.data_mut(|d| d.insert_temp(acc_id, acc));
            return step;
        }
    }
    0
}

/// A drop-down of plain options (`items`: value, label, enabled), the
/// current one `current` (None: mixed or none), shown as `shown`. Returns
/// the option picked by a click, the wheel or a press-drag-release, and the
/// box's response (for a tip).
pub fn dropdown<T: PartialEq + Clone>(ui: &mut Ui, salt: impl egui::AsIdSalt + Clone, shown: impl Into<egui::WidgetText>, width: f32, items: &[(T, String, bool)], current: Option<&T>) -> (Option<T>, Response) {
    let drag = dd_dragging(ui, salt.clone());
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(salt.clone())
        .truncate()
        .selected_text(shown)
        .width(width)
        .popup_style(theme::glass_popup_style(false))
        .show_ui(ui, |ui| {
            theme::glass_popup(ui, |ui| {
                for (v, label, on) in items {
                    let row = ui.add_enabled(*on, egui::Button::selectable(current == Some(v), label.as_str()));
                    if row.clicked() || (*on && dd_row_picked(ui, drag, &row)) {
                        picked = Some(v.clone());
                        ui.close();
                    }
                }
            })
        });
    let step = dd_button(ui, salt, &r.response);
    if step != 0 && picked.is_none() {
        // the next enabled option that way, from the current one
        if let Some(i) = current.and_then(|c| items.iter().position(|x| &x.0 == c)) {
            let mut j = i as i32 + step;
            while (0..items.len() as i32).contains(&j) && !items[j as usize].2 {
                j += step;
            }
            if (0..items.len() as i32).contains(&j) {
                picked = Some(items[j as usize].0.clone());
            }
        }
    }
    (picked, r.response)
}

/// DV_OPEN_COMBO=<id salt>: that drop-down stays open (captures; drop-downs
/// cannot be opened from outside). Call in the ui that makes the ComboBox.
pub fn dbg_open_combo(ui: &Ui, salt: &str) {
    if std::env::var("DV_OPEN_COMBO").is_ok_and(|v| v == salt) {
        let id = ui.make_persistent_id(egui::IdSalt::new(salt)).with("popup");
        if !egui::Popup::is_id_open(ui.ctx(), id) {
            egui::Popup::open_id(ui.ctx(), id);
        }
        // the popup's first pass only measures: draw again
        ui.ctx().request_repaint();
    }
}

/// The FX spark: four points, pinched sides (the layer button and the FX
/// layer's row).
pub fn fx_glyph(p: &egui::Painter, c: Pos2, r: f32, s: Stroke) {
    let k = r * 0.28;
    p.add(Shape::closed_line(
        vec![c + vec2(0.0, -r), c + vec2(k, -k), c + vec2(r, 0.0), c + vec2(k, k), c + vec2(0.0, r), c + vec2(-k, k), c + vec2(-r, 0.0), c + vec2(-k, -k)],
        s,
    ));
}

/// A square icon button on the row grid with a glass tip (list actions such
/// as the layer buttons, Ash 10-27). `on` shows it in the accent.
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, on: bool) -> Response {
    use crate::theme::GlassTip;
    let (r, resp) = ui.allocate_exact_size(vec2(ROW_H, ROW_H), Sense::click());
    let enabled = ui.is_enabled();
    let rad = theme::radius_ctrl();
    if enabled && resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(r, CornerRadius::same(rad), FILL_3());
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(rad), FILL_2());
    }
    draw_icon(ui.painter(), icon, r.center(), state_ink(on, enabled));
    focus_ring(ui, &resp, r, rad as f32);
    resp.glass_tip(tip)
}

/// A menu command: label left, its key right in a quieter ink, the whole row
/// highlighted on hover (no button fill; Ash 10-27, N4).
pub fn menu_row(ui: &mut Ui, text: &str, key: &str) -> Response {
    // as wide as label and key need (the menu grows to it), else the menu's width
    let font = egui::TextStyle::Body.resolve(ui.style());
    let need = ui.painter().layout_no_wrap(text.to_owned(), font, INK()).size().x
        + ui.painter().layout_no_wrap(key.to_owned(), FontId::proportional(theme::TEXT_SMALL), INK()).size().x
        + SPACE_SM * 2.0
        + 24.0;
    // the menu's width (rows line up), at least what label and key need; in
    // a place without a set width, only what they need
    let room = ui.available_width();
    let w = if room > 480.0 { need } else { room.max(need) };
    let (r, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
    let enabled = ui.is_enabled();
    let p = ui.painter();
    if enabled && resp.is_pointer_button_down_on() {
        p.rect_filled(r, CornerRadius::same(theme::inner(theme::radius_card(), SPACE_SM)), FILL_3());
    } else if enabled && resp.hovered() {
        p.rect_filled(r, CornerRadius::same(theme::inner(theme::radius_card(), SPACE_SM)), FILL_2());
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let ink = if enabled { INK() } else { INK_2() };
    p.text(pos2(r.left() + SPACE_SM, r.center().y), Align2::LEFT_CENTER, text, font, ink);
    if !key.is_empty() {
        p.text(pos2(r.right() - SPACE_SM, r.center().y), Align2::RIGHT_CENTER, key, FontId::proportional(theme::TEXT_SMALL), theme::LABEL());
    }
    focus_ring(ui, &resp, r, theme::inner(theme::radius_card(), SPACE_SM) as f32);
    resp
}

/// A row on the grid: the label cell (with the always-reserved key slot) and
/// whatever `add` puts after it, `SPACE_XS` apart.
pub fn row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_XS;
        let mut no = false;
        crate::dial::label_cell(ui, label, None, &mut no);
        add(ui)
    })
    .inner
}

/// `DV_KIT=1`: a glass card with every kit piece in each state, and dial rows
/// with and without keys, for captures (the state lives in egui's memory).
pub fn demo(ctx: &egui::Context) {
    #[derive(Clone)]
    struct S {
        sw: [bool; 2],
        seg3: u8,
        seg2: u8,
        v: [f32; 3],
        xy: [f32; 2],
    }
    let id = egui::Id::new("kit_demo");
    let mut s = ctx.data(|d| d.get_temp::<S>(id)).unwrap_or(S { sw: [false, true], seg3: 1, seg2: 0, v: [0.6, 40.0, 0.4], xy: [200.0, 0.0] });
    egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos2(96.0, 72.0)).show(ctx, |ui| {
        theme::glass(ui, egui::Margin::same(theme::SPACE_MD as i8), theme::radius_card(), |ui| {
            ui.set_width(theme::COL_W);
            let head = |ui: &mut Ui, t: &str| {
                ui.add_space(SPACE_SM);
                ui.label(theme::section_title(t));
            };
            head(ui, "스위치");
            switch(ui, &mut s.sw[0], "꺼짐");
            switch(ui, &mut s.sw[1], "켜짐");
            ui.add_enabled_ui(false, |ui| {
                let (mut a, mut b) = (false, true);
                switch(ui, &mut a, "사용 불가 · 꺼짐");
                switch(ui, &mut b, "사용 불가 · 켜짐");
            });
            head(ui, "세그먼트");
            segmented(ui, &mut s.seg3, &[(0, "연필"), (1, "펜촉"), (2, "마커")]);
            segmented(ui, &mut s.seg2, &[(0, "가로 기준"), (1, "세로 기준")]);
            ui.add_enabled_ui(false, |ui| segmented(ui, &mut s.seg2, &[(0, "선명"), (1, "사진 흐림")]));
            head(ui, "아이콘 버튼");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_XS;
                icon_button(ui, Icon::Up, "위로", false);
                icon_button(ui, Icon::Down, "아래로", false);
                icon_button(ui, Icon::Duplicate, "복제 · Ctrl+D", true);
                ui.add_enabled_ui(false, |ui| icon_button(ui, Icon::Delete, "삭제 · Backspace", false));
            });
            head(ui, "메뉴 줄");
            menu_row(ui, "전체 보기", "Ctrl+0");
            menu_row(ui, "100%", "Ctrl+1");
            ui.add_enabled_ui(false, |ui| menu_row(ui, "UI 숨기기", "Ctrl+H"));
            head(ui, "줄 격자");
            let (mut h, mut g) = (false, 0i8);
            ui.add(crate::dial::Dial::new(&mut s.v[0], 0.0..=1.0).text("불투명도").default(1.0));
            ui.add(crate::dial::Dial::new(&mut s.v[1], 0.0..=100.0).text("손떨림 보정").key(Some(crate::dial::KeyMark::None), &mut h));
            let (mut h2, mut g2) = (false, 0i8);
            ui.add(crate::dial::Dial::new(&mut s.v[2], 0.0..=1.0).text("크기 %").key(Some(crate::dial::KeyMark::Here), &mut h2).nav(crate::dial::KeyNav { prev: true, next: true }, &mut g2));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_XS;
                crate::dial::label_cell_nav(ui, "위치", Some(crate::dial::KeyMark::Keyed), &mut h, Some((crate::dial::KeyNav { prev: true, next: false }, &mut g)));
                ui.label("X");
                ui.add_sized(vec2(56.0, ROW_H), egui::DragValue::new(&mut s.xy[0]));
                ui.label("Y");
                ui.add_sized(vec2(56.0, ROW_H), egui::DragValue::new(&mut s.xy[1]));
            });
            row(ui, "정렬", |ui| segmented(ui, &mut s.seg3, &[(0, "왼쪽"), (1, "가운데"), (2, "오른쪽")]));
        });
    });
    ctx.data_mut(|d| d.insert_temp(id, s));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};

    /// A headless egui with one drop-down of three options (current B):
    /// each `frame` feeds the events at time `t` and returns what it picked,
    /// the box's rect and the open list's rect.
    struct Harness {
        ctx: egui::Context,
        t: f64,
        cur: u8,
    }

    impl Harness {
        fn new() -> Self {
            Harness { ctx: egui::Context::default(), t: 0.0, cur: 1 }
        }

        fn frame(&mut self, dt: f64, events: Vec<Event>) -> (Option<u8>, Rect, Option<Rect>) {
            self.t += dt;
            let input = RawInput { time: Some(self.t), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
            let mut out = (None, Rect::NOTHING, None);
            let cur = self.cur;
            let ctx = self.ctx.clone();
            let out_ref = &mut out;
            let full = self.ctx.run_ui(input, |ui| {
                {
                    let out = &mut *out_ref;
                    let items: Vec<(u8, String, bool)> = (0..3).map(|i| (i, ["A", "B", "C"][i as usize].to_string(), true)).collect();
                    let (picked, _) = dropdown(ui, "dd", ["A", "B", "C"][cur as usize], 120.0, &items, Some(&cur));
                    let popup = ui.make_persistent_id(egui::IdSalt::new("dd")).with("popup");
                    let open = egui::Popup::is_id_open(&ctx, popup).then(|| ctx.memory(|m| m.area_rect(popup))).flatten();
                    let button = ui.make_persistent_id(egui::IdSalt::new("dd"));
                    let rect = ctx.read_response(button).map_or(Rect::NOTHING, |r| r.rect);
                    *out = (picked, rect, open);
                }
            });
            full.drop_without_applying_deltas();
            if let Some(p) = out.0 {
                self.cur = p;
            }
            out
        }

        fn settle(&mut self) -> (Rect, Option<Rect>) {
            let mut r = self.frame(0.016, vec![]);
            for _ in 0..3 {
                r = self.frame(0.016, vec![]);
            }
            (r.1, r.2)
        }
    }

    fn mv(p: Pos2) -> Event {
        Event::PointerMoved(p)
    }

    fn btn(p: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    fn wheel(dy: f32) -> Event {
        Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, dy), phase: egui::TouchPhase::Move, modifiers: Default::default() }
    }

    #[test]
    fn wheel_steps_the_choice() {
        let mut h = Harness::new();
        let (b, _) = h.settle();
        h.frame(0.016, vec![mv(b.center())]);
        let (p, _, open) = h.frame(0.016, vec![wheel(-1.0)]);
        assert_eq!((p, open.is_some()), (Some(2), false), "wheel down: the next option, list closed");
        h.frame(0.016, vec![]);
        let (p, _, _) = h.frame(0.016, vec![wheel(1.0)]);
        assert_eq!(p, Some(1), "wheel up: back");
        let (p, _, _) = h.frame(0.016, vec![wheel(1.0)]);
        assert_eq!(p, Some(0));
        let (p, _, _) = h.frame(0.016, vec![wheel(1.0)]);
        assert_eq!(p, None, "past the first: stays");
    }

    #[test]
    fn plain_click_only_opens() {
        let mut h = Harness::new();
        let (b, _) = h.settle();
        h.frame(0.016, vec![mv(b.center())]);
        h.frame(0.016, vec![btn(b.center(), true)]);
        let (p, _, _) = h.frame(0.05, vec![btn(b.center(), false)]);
        let (_, open) = h.settle();
        assert_eq!((p, open.is_some()), (None, true), "a click opens the list and picks nothing");
    }

    /// Press on the box and drag down out of it: the list opens; returns
    /// the box and the list.
    fn hold_open(h: &mut Harness) -> (Rect, Rect) {
        let (b, _) = h.settle();
        h.frame(0.016, vec![mv(b.center())]);
        h.frame(0.016, vec![btn(b.center(), true)]);
        // still inside the box: nothing yet, however long
        h.frame(0.016, vec![mv(b.center() + vec2(0.0, b.height() * 0.4))]);
        for _ in 0..30 {
            h.frame(0.05, vec![]);
        }
        let (_, open) = h.settle();
        assert!(open.is_none(), "held inside the box: the list stays shut");
        h.frame(0.016, vec![mv(pos2(b.center().x, b.bottom() + 4.0))]);
        let (_, open) = h.settle();
        (b, open.expect("dragged below the box: the list is open"))
    }

    #[test]
    fn press_drag_release_picks() {
        let mut h = Harness::new();
        let (_, list) = hold_open(&mut h);
        // the third row: rows are spread evenly down the list's inside
        let p = pos2(list.center().x, list.top() + list.height() * (2.5 / 3.0));
        h.frame(0.016, vec![mv(p)]);
        let (picked, _, _) = h.frame(0.016, vec![btn(p, false)]);
        let (_, open) = h.settle();
        assert_eq!((picked, open.is_some()), (Some(2), false), "released on C: picked, list closed");
    }

    #[test]
    fn release_outside_or_on_box() {
        let mut h = Harness::new();
        let (_, list) = hold_open(&mut h);
        let away = pos2(list.right() + 200.0, list.bottom() + 100.0);
        h.frame(0.016, vec![mv(away)]);
        let (picked, _, _) = h.frame(0.016, vec![btn(away, false)]);
        let (_, open) = h.settle();
        assert_eq!((picked, open.is_some()), (None, false), "released away: nothing, closed");

        let mut h = Harness::new();
        let (b, _) = hold_open(&mut h);
        let (picked, _, _) = h.frame(0.016, vec![btn(b.center(), false)]);
        let (_, open) = h.settle();
        assert_eq!((picked, open.is_some()), (None, true), "released on the box: the list stays");
    }
}
