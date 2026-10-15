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
}

impl TextLayout {
    pub fn rect(&self, pos: Pos2) -> Rect {
        Rect::from_min_size(pos, self.size)
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

pub fn pad_for(t: &TextLayer) -> f32 {
    if t.bg.is_some() { (t.size * 0.35).max(2.0) } else { 0.0 }
}

pub fn layout(t: &TextLayer) -> TextLayout {
    let font = &font_for(t);
    let px = font.pt_to_px_scale(t.size.max(1.0)).unwrap_or(PxScale::from(t.size.max(1.0)));
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

    let mut lines: Vec<Vec<char>> = Vec::new();
    for para in t.text.split('\n') {
        let mut cur: Vec<char> = Vec::new();
        for ch in para.chars() {
            cur.push(ch);
            if maxw > 0.0 && cur.len() > 1 && width_of(&cur) > maxw {
                let space = cur[..cur.len() - 1].iter().rposition(|c| *c == ' ').filter(|&i| i > 0);
                if let Some(sp) = space {
                    let rest = cur.split_off(sp + 1);
                    cur.pop();
                    lines.push(std::mem::replace(&mut cur, rest));
                } else {
                    let last = cur.pop().unwrap();
                    lines.push(std::mem::replace(&mut cur, vec![last]));
                }
            }
        }
        lines.push(cur);
    }

    let mut glyphs = Vec::new();
    let mut w: f32 = 0.0;
    for (i, line) in lines.iter().enumerate() {
        let y = ascent + i as f32 * line_h;
        let mut x = 0.0;
        let mut prev: Option<GlyphId> = None;
        for &c in line {
            let g = font.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, g);
            }
            if !c.is_whitespace() {
                glyphs.push((g, x, y));
            }
            x += sf.h_advance(g);
            prev = Some(g);
        }
        w = w.max(x);
    }
    let content_w = if maxw > 0.0 { maxw } else { w.max(t.size * 0.5) };
    let h = lines.len().max(1) as f32 * line_h;
    TextLayout { glyphs, size: vec2(content_w + pad * 2.0, h + pad * 2.0), pad }
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
    let px = font.pt_to_px_scale((t.size * scale).max(0.5)).unwrap_or(PxScale::from(t.size * scale));
    let [cr, cg, cb, ca] = t.color.to_srgba_unmultiplied();
    let ca = ca as f32 / 255.0;
    let col = [cr as f32, cg as f32, cb as f32];
    let (pw, ph) = (w as i32, h as i32);
    let data = pm.data_mut();
    for &(gid, x, y) in &lay.glyphs {
        let pos = point(frac.x + (lay.pad + x) * scale, frac.y + (lay.pad + y) * scale);
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
