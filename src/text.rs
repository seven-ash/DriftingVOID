//! Text layout and rasterization shared by the canvas and the exporter, so what
//! you see is exactly what gets saved.

use std::sync::OnceLock;

use ab_glyph::{Font, FontArc, GlyphId, PxScale, ScaleFont, point};
use egui::{Pos2, Rect, Vec2, vec2};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::doc::TextLayer;

pub struct FontSet {
    pub regular: FontArc,
    pub bold: FontArc,
    pub regular_bytes: Vec<u8>,
    pub bold_bytes: Vec<u8>,
}

static FONTS: OnceLock<FontSet> = OnceLock::new();

pub fn fonts() -> &'static FontSet {
    FONTS.get_or_init(load_fonts)
}

fn load_fonts() -> FontSet {
    let dir = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let pairs = [("malgun.ttf", "malgunbd.ttf"), ("segoeui.ttf", "segoeuib.ttf"), ("arial.ttf", "arialbd.ttf")];
    for (r, b) in pairs {
        if let Ok(rb) = std::fs::read(format!("{dir}\\{r}")) {
            let bb = std::fs::read(format!("{dir}\\{b}")).unwrap_or_else(|_| rb.clone());
            if let (Ok(rf), Ok(bf)) = (FontArc::try_from_vec(rb.clone()), FontArc::try_from_vec(bb.clone())) {
                return FontSet { regular: rf, bold: bf, regular_bytes: rb, bold_bytes: bb };
            }
        }
    }
    let defs = egui::FontDefinitions::default();
    let data = defs.font_data.values().next().expect("egui ships default fonts").font.to_vec();
    let f = FontArc::try_from_vec(data.clone()).expect("default font parses");
    FontSet { regular: f.clone(), bold: f, regular_bytes: data.clone(), bold_bytes: data }
}

pub struct TextLayout {
    /// Glyph id, x, baseline y (relative to the content origin, before padding).
    pub glyphs: Vec<(GlyphId, f32, f32)>,
    pub size: Vec2,
    pub pad: f32,
    /// Caret spot before each character (and after the last), as the top of its
    /// line (x, y relative to the content origin, before padding).
    pub carets: Vec<(f32, f32)>,
    pub line_h: f32,
}

impl TextLayout {
    pub fn rect(&self, pos: Pos2) -> Rect {
        Rect::from_min_size(pos, self.size)
    }

    /// The character index whose caret spot is nearest to `p` (content
    /// coordinates, like `carets`): the line under `p` (clamped to the first and
    /// last), then the nearest spot on it.
    pub fn caret_at(&self, p: Pos2) -> usize {
        let Some(first) = self.carets.first() else { return 0 };
        let last = self.carets.last().map_or(first.1, |c| c.1);
        let row = (((p.y - first.1) / self.line_h.max(1e-3)).floor() * self.line_h + first.1).clamp(first.1, last);
        self.carets
            .iter()
            .enumerate()
            .filter(|(_, c)| (c.1 - row).abs() < self.line_h * 0.5)
            .min_by(|a, b| (a.1 .0 - p.x).abs().total_cmp(&(b.1 .0 - p.x).abs()))
            .map_or(0, |(i, _)| i)
    }
}

pub fn font_for(t: &TextLayer) -> FontArc {
    if !t.family.is_empty() {
        if let Some(l) = crate::fontdb::load(&t.family, &t.style) {
            return l.font;
        }
    }
    if t.style.to_lowercase().contains("bold") { fonts().bold.clone() } else { fonts().regular.clone() }
}

/// Scale whose em square is `size` pixels (egui's convention). ab_glyph's
/// pt_to_px_scale would add a 96/72 factor, making text 1.33x the set size.
pub fn em_scale(font: &FontArc, size: f32) -> PxScale {
    match font.units_per_em() {
        Some(upem) if upem > 0.0 => PxScale::from(size * font.height_unscaled() / upem),
        _ => PxScale::from(size),
    }
}

pub fn pad_for(t: &TextLayer) -> f32 {
    if t.bg.is_some() { (t.size * 0.35).max(2.0) } else { 0.0 }
}

pub fn layout(t: &TextLayer) -> TextLayout {
    let font = &font_for(t);
    let px = em_scale(font, t.size.max(1.0));
    let sf = font.as_scaled(px);
    let line_h = sf.height() + sf.line_gap();
    let ascent = sf.ascent();
    let pad = pad_for(t);
    let maxw = if t.wrap > 0.0 { (t.wrap - pad * 2.0).max(t.size) } else { 0.0 };

    let width_of = |chars: &[char]| -> f32 {
        let mut x = 0.0;
        let mut prev: Option<GlyphId> = None;
        for &c in chars {
            let g = font.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, g);
            }
            x += sf.h_advance(g);
            prev = Some(g);
        }
        x
    };

    // lines of (char, its index in the text), each with the index it starts at
    let mut lines: Vec<(usize, Vec<(char, usize)>)> = Vec::new();
    let mut idx = 0;
    for para in t.text.split('\n') {
        let mut start = idx;
        let mut cur: Vec<(char, usize)> = Vec::new();
        for ch in para.chars() {
            cur.push((ch, idx));
            idx += 1;
            let chars: Vec<char> = cur.iter().map(|x| x.0).collect();
            if maxw > 0.0 && cur.len() > 1 && width_of(&chars) > maxw {
                let space = chars[..cur.len() - 1].iter().rposition(|c| *c == ' ').filter(|&i| i > 0);
                if let Some(sp) = space {
                    let rest = cur.split_off(sp + 1);
                    cur.pop();
                    let next = rest.first().map_or(idx, |x| x.1);
                    lines.push((start, std::mem::replace(&mut cur, rest)));
                    start = next;
                } else {
                    let last = cur.pop().unwrap();
                    lines.push((start, std::mem::replace(&mut cur, vec![last])));
                    start = last.1;
                }
            }
        }
        lines.push((start, cur));
        idx += 1; // the line break
    }
    let n_chars = t.text.chars().count();
    let mut carets: Vec<Option<(f32, usize)>> = vec![None; n_chars + 1];

    let mut glyphs = Vec::new();
    let mut w: f32 = 0.0;
    // (first glyph, line width) per line, for alignment
    let mut spans: Vec<(usize, f32)> = Vec::new();
    for (i, (start, line)) in lines.iter().enumerate() {
        let first = glyphs.len();
        let y = ascent + i as f32 * line_h;
        let mut x = 0.0;
        let mut prev: Option<GlyphId> = None;
        for &(c, ci) in line {
            let g = font.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, g);
            }
            if let Some(slot) = carets.get_mut(ci) {
                *slot = Some((x, i));
            }
            if !c.is_whitespace() {
                glyphs.push((g, x, y));
            }
            x += sf.h_advance(g);
            prev = Some(g);
        }
        // after the last character (or at the start of an empty line)
        let end = line.last().map_or(*start, |l| l.1 + 1);
        if let Some(slot) = carets.get_mut(end).filter(|s| s.is_none()) {
            *slot = Some((x, i));
        }
        // trailing spaces don't count towards a line's width
        let trail: f32 = line.iter().rev().take_while(|c| c.0.is_whitespace()).map(|c| sf.h_advance(font.glyph_id(c.0))).sum();
        spans.push((first, x - trail));
        w = w.max(x);
    }
    let content_w = if maxw > 0.0 { maxw } else { w.max(t.size * 0.5) };
    let f = t.align.factor();
    let line_dx: Vec<f32> = spans.iter().map(|&(_, lw)| (content_w - lw) * f).collect();
    if f > 0.0 {
        for (i, &(first, _)) in spans.iter().enumerate() {
            let end = spans.get(i + 1).map_or(glyphs.len(), |s| s.0);
            for g in &mut glyphs[first..end] {
                g.1 += line_dx[i];
            }
        }
    }
    let mut h = lines.len().max(1) as f32 * line_h;
    // some fonts draw past their own ascent/descent (a "g" below the line):
    // grow the box to the ink so nothing is cut off
    let sy = sf.v_scale_factor();
    let mut ink: std::collections::HashMap<GlyphId, Option<ab_glyph::Rect>> = Default::default();
    let (mut top, mut bottom) = (0.0f32, h);
    for &(g, _, y) in &glyphs {
        if let Some(b) = *ink.entry(g).or_insert_with(|| font.outline(g).map(|o| o.bounds)) {
            // font units, y up (ab_glyph may hand min/max over swapped)
            let (lo, hi) = (b.min.y.min(b.max.y), b.min.y.max(b.max.y));
            top = top.min(y - hi * sy);
            bottom = bottom.max(y - lo * sy);
        }
    }
    let shift = (-top).max(0.0);
    if top < 0.0 {
        for g in &mut glyphs {
            g.2 -= top;
        }
        h -= top;
    }
    let carets = carets
        .into_iter()
        .map(|c| c.map_or((0.0, shift), |(x, li)| (x + line_dx.get(li).copied().unwrap_or(0.0), shift + li as f32 * line_h)))
        .collect();
    h += bottom - (lines.len().max(1) as f32 * line_h);
    TextLayout { glyphs, size: vec2(content_w + pad * 2.0, h + pad * 2.0), pad, carets, line_h }
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Renders the text box at `scale` into a new pixmap. `frac` is a sub-pixel
/// offset of the box origin inside the pixmap.
pub fn render(t: &TextLayer, lay: &TextLayout, scale: f32, frac: Vec2) -> Option<Pixmap> {
    let w = ((lay.size.x * scale + frac.x).ceil() as u32 + 1).min(16384);
    let h = ((lay.size.y * scale + frac.y).ceil() as u32 + 1).min(16384);
    render_into(t, lay, scale, frac, w, h)
}

/// Renders only `win` (box-local units, from the box's top-left) of the text
/// box at `scale`: long boxes stay sharp without one huge texture.
pub fn render_window(t: &TextLayer, lay: &TextLayout, scale: f32, win: Rect) -> Option<Pixmap> {
    let w = (win.width() * scale).ceil() as u32 + 1;
    let h = (win.height() * scale).ceil() as u32 + 1;
    render_into(t, lay, scale, -win.min.to_vec2() * scale, w, h)
}

fn render_into(t: &TextLayer, lay: &TextLayout, scale: f32, frac: Vec2, w: u32, h: u32) -> Option<Pixmap> {
    let mut pm = Pixmap::new(w, h)?;
    if let Some(bg) = t.bg {
        if let Some(path) = rounded_rect(frac.x, frac.y, lay.size.x * scale, lay.size.y * scale, t.size * 0.3 * scale) {
            let mut paint = Paint::default();
            let [r, g, b, a] = bg.to_srgba_unmultiplied();
            paint.set_color_rgba8(r, g, b, a);
            paint.anti_alias = true;
            pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
    let font = &font_for(t);
    let px = em_scale(font, (t.size * scale).max(0.5));
    let [cr, cg, cb, ca] = t.color.to_srgba_unmultiplied();
    let ca = ca as f32 / 255.0;
    let col = [cr as f32, cg as f32, cb as f32];
    let (pw, ph) = (w as i32, h as i32);
    let data = pm.data_mut();
    // glyphs well outside the pixmap are skipped (windows of long boxes)
    let reach = t.size * 2.0 * scale;
    for &(gid, x, y) in &lay.glyphs {
        let pos = point(frac.x + (lay.pad + x) * scale, frac.y + (lay.pad + y) * scale);
        if pos.x < -reach || pos.x > w as f32 + reach || pos.y < -reach || pos.y > h as f32 + reach {
            continue;
        }
        let Some(og) = font.outline_glyph(gid.with_scale_and_position(px, pos)) else { continue };
        let b = og.px_bounds();
        og.draw(|gx, gy, cov| {
            let xx = b.min.x as i32 + gx as i32;
            let yy = b.min.y as i32 + gy as i32;
            if xx < 0 || yy < 0 || xx >= pw || yy >= ph {
                return;
            }
            let a = cov.clamp(0.0, 1.0) * ca;
            if a <= 0.0 {
                return;
            }
            let i = ((yy * pw + xx) * 4) as usize;
            for k in 0..3 {
                data[i + k] = (col[k] * a + data[i + k] as f32 * (1.0 - a)).round().min(255.0) as u8;
            }
            data[i + 3] = (a * 255.0 + data[i + 3] as f32 * (1.0 - a)).round().min(255.0) as u8;
        });
    }
    Some(pm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_holds_every_glyph() {
        // whatever the font's ascent/descent say, the rendered ink stays inside
        let t = TextLayer { pos: Pos2::ZERO, text: "gjpqy ÅÉ Drifting".into(), size: 60.0, color: egui::Color32::WHITE, family: String::new(), style: String::new(), bg: None, wrap: 0.0, rotation: 0.0, blend: Default::default(), align: Default::default() };
        let lay = layout(&t);
        let pm = render(&t, &lay, 1.0, Vec2::ZERO).unwrap();
        let (w, h) = (pm.width() as usize, pm.height() as usize);
        let row = |y: usize| (0..w).any(|x| pm.data()[(y * w + x) * 4 + 3] > 0);
        // the box edges (last row is the +1 margin) carry no ink
        assert!(!row(0) && !row(h - 1), "ink touches the box edge");
    }
}

#[cfg(test)]
mod caret_tests {
    use super::*;

    #[test]
    fn carets_follow_lines_and_alignment() {
        let mut t = TextLayer { pos: Pos2::ZERO, text: "ab\nc".into(), size: 40.0, color: egui::Color32::WHITE, family: String::new(), style: String::new(), bg: None, wrap: 0.0, rotation: 0.0, blend: Default::default(), align: Default::default() };
        let lay = layout(&t);
        assert_eq!(lay.carets.len(), 5);
        assert_eq!(lay.carets[0].0, 0.0);
        assert!(lay.carets[2].0 > lay.carets[1].0, "after b");
        assert_eq!(lay.carets[3], (0.0, lay.carets[0].1 + lay.line_h), "c starts line 2");
        // centred: the short second line moves right by half the difference
        t.align = crate::doc::TextAlign::Center;
        let c = layout(&t);
        assert!(c.carets[3].0 > 0.0);
        assert!((c.carets[0].0).abs() < 1e-3, "the longest line stays put");
    }

    #[test]
    fn clicks_find_the_caret_on_the_line() {
        let t = TextLayer { pos: Pos2::ZERO, text: "ab
cd".into(), size: 40.0, color: egui::Color32::WHITE, family: String::new(), style: String::new(), bg: None, wrap: 0.0, rotation: 0.0, blend: Default::default(), align: Default::default() };
        let lay = layout(&t);
        let (x1, y0) = lay.carets[1];
        assert_eq!(lay.caret_at(Pos2::new(x1 + 1.0, y0 + lay.line_h * 0.5)), 1);
        // the second line, past its end: after d
        assert_eq!(lay.caret_at(Pos2::new(1000.0, y0 + lay.line_h * 1.5)), 5);
        // above and below the text clamp to the first and last line
        assert_eq!(lay.caret_at(Pos2::new(-50.0, -100.0)), 0);
        assert_eq!(lay.caret_at(Pos2::new(-50.0, 1000.0)), 3);
    }
}
