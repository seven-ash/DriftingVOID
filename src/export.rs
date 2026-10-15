//! Full-resolution CPU compositor (tiny-skia). Used for export, clipboard copy
//! and the eyedropper, so all three agree with each other.

use egui::{Color32, Pos2, Rect, vec2};
use tiny_skia::{
    FillRule, FilterQuality, LineCap, LineJoin, Paint, PathBuilder, Pattern, Pixmap, PixmapPaint, SpreadMode,
    Stroke, Transform,
};

use crate::doc::{Document, ImageLayer, Layer, LayerKind, ShapeKind};

pub const MAX_SIDE: u32 = 32000;
pub const MAX_PIXELS: u64 = 400_000_000;

fn paint_of(c: Color32) -> Paint<'static> {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let mut p = Paint::default();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

pub fn output_size(region: Rect, scale: f32) -> (u32, u32) {
    (
        (region.width() * scale).round().max(1.0) as u32,
        (region.height() * scale).round().max(1.0) as u32,
    )
}

pub fn render(doc: &Document, region: Rect, scale: f32, bg: Option<Color32>) -> Result<Pixmap, String> {
    let (w, h) = output_size(region, scale);
    if w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS {
        return Err(format!("너무 큼: {w}×{h}px"));
    }
    let mut out = Pixmap::new(w, h).ok_or("이미지 버퍼를 만들 수 없음")?;
    if let Some(bg) = bg {
        let [r, g, b, a] = bg.to_srgba_unmultiplied();
        out.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
    }
    let base = Transform::from_translate(-region.min.x, -region.min.y).post_scale(scale, scale);
    for layer in doc.layers.iter().filter(|l| l.visible && l.opacity > 0.0) {
        if !layer.bounds().is_some_and(|b| b.intersects(region)) {
            continue;
        }
        if layer.opacity < 0.999 {
            let mut tmp = Pixmap::new(w, h).ok_or("임시 버퍼를 만들 수 없음")?;
            draw_layer(&mut tmp, layer, base, region, scale);
            let paint = PixmapPaint { opacity: layer.opacity, ..Default::default() };
            out.draw_pixmap(0, 0, tmp.as_ref(), &paint, Transform::identity(), None);
        } else {
            draw_layer(&mut out, layer, base, region, scale);
        }
    }
    Ok(out)
}

fn draw_layer(pm: &mut Pixmap, layer: &Layer, base: Transform, region: Rect, scale: f32) {
    match &layer.kind {
        LayerKind::Image(im) => draw_image(pm, im, base, scale),
        LayerKind::Raster(r) => {
            let local = region.translate(-r.offset);
            let Some(b) = r.bounds().map(|b| b.translate(-r.offset)) else { return };
            let x0 = local.min.x.floor().max(b.min.x) as i32;
            let y0 = local.min.y.floor().max(b.min.y) as i32;
            let x1 = local.max.x.ceil().min(b.max.x) as i32;
            let y1 = local.max.y.ceil().min(b.max.y) as i32;
            if x1 <= x0 || y1 <= y0 {
                return;
            }
            let (rw, rh) = (x1 - x0, y1 - y0);
            let data = r.read_region(x0, y0, rw, rh);
            let Some(src) = Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(rw as u32, rh as u32).unwrap())
            else {
                return;
            };
            let ts = Transform::from_translate(x0 as f32 + r.offset.x, y0 as f32 + r.offset.y).post_concat(base);
            let quality = if (scale - 1.0).abs() < 1e-4 { FilterQuality::Nearest } else { FilterQuality::Bicubic };
            let paint = PixmapPaint { quality, ..Default::default() };
            pm.draw_pixmap(0, 0, src.as_ref(), &paint, ts, None);
        }
        LayerKind::Vector(v) => {
            let ts = Transform::from_translate(v.offset.x, v.offset.y).post_concat(base);
            let local_region = region.translate(-v.offset);
            for s in &v.strokes {
                if !s.bounds.intersects(local_region) {
                    continue;
                }
                let mut pb = PathBuilder::new();
                for poly in s.capsules(scale) {
                    if poly.len() < 3 {
                        continue;
                    }
                    pb.move_to(poly[0].x, poly[0].y);
                    for q in &poly[1..] {
                        pb.line_to(q.x, q.y);
                    }
                    pb.close();
                }
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &paint_of(s.color), FillRule::Winding, ts, None);
                }
            }
        }
        LayerKind::Text(t) => {
            let lay = crate::text::layout(t);
            let o = (t.pos - region.min) * scale;
            let (ix, iy) = (o.x.floor(), o.y.floor());
            if let Some(tp) = crate::text::render(t, &lay, scale, vec2(o.x - ix, o.y - iy)) {
                pm.draw_pixmap(ix as i32, iy as i32, tp.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
        }
        LayerKind::Shape(s) => {
            let stroke = Stroke { width: s.width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
            let sp = paint_of(s.color);
            match s.kind {
                ShapeKind::Line | ShapeKind::Arrow => {
                    let end = if s.kind == ShapeKind::Arrow { s.arrow_head().0 } else { s.b };
                    let mut pb = PathBuilder::new();
                    pb.move_to(s.a.x, s.a.y);
                    pb.line_to(end.x, end.y);
                    if let Some(path) = pb.finish() {
                        pm.stroke_path(&path, &sp, &stroke, base, None);
                    }
                    if s.kind == ShapeKind::Arrow {
                        let (_, h) = s.arrow_head();
                        let mut pb = PathBuilder::new();
                        pb.move_to(h[0].x, h[0].y);
                        pb.line_to(h[1].x, h[1].y);
                        pb.line_to(h[2].x, h[2].y);
                        pb.close();
                        if let Some(path) = pb.finish() {
                            pm.fill_path(&path, &sp, FillRule::Winding, base, None);
                        }
                    }
                }
                ShapeKind::Rect | ShapeKind::Ellipse => {
                    let r = Rect::from_two_pos(s.a, s.b);
                    let Some(tr) = tiny_skia::Rect::from_ltrb(r.min.x, r.min.y, r.max.x.max(r.min.x + 0.01), r.max.y.max(r.min.y + 0.01))
                    else {
                        return;
                    };
                    let path = if s.kind == ShapeKind::Rect {
                        Some(PathBuilder::from_rect(tr))
                    } else {
                        PathBuilder::from_oval(tr)
                    };
                    if let Some(path) = path {
                        if let Some(f) = s.fill {
                            pm.fill_path(&path, &paint_of(f), FillRule::Winding, base, None);
                        }
                        if s.width > 0.0 {
                            pm.stroke_path(&path, &sp, &stroke, base, None);
                        }
                    }
                }
            }
        }
    }
}

/// Cropped, adjusted, premultiplied copy of an image, optionally downsampled.
/// Returns the pixmap and the source-pixels-per-pixmap-pixel factor.
pub fn image_pixmap(im: &ImageLayer, target_scale: f32) -> Option<(Pixmap, f32)> {
    let (iw, ih) = (im.src.rgba.width() as i32, im.src.rgba.height() as i32);
    let x0 = (im.crop.min.x.round() as i32).clamp(0, iw - 1);
    let y0 = (im.crop.min.y.round() as i32).clamp(0, ih - 1);
    let x1 = (im.crop.max.x.round() as i32).clamp(x0 + 1, iw);
    let y1 = (im.crop.max.y.round() as i32).clamp(y0 + 1, ih);
    let mut sub = image::imageops::crop_imm(&*im.src.rgba, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32).to_image();
    let mut factor = 1.0;
    if target_scale < 0.5 {
        let nw = ((sub.width() as f32 * target_scale * 1.5).round() as u32).max(1);
        let nh = ((sub.height() as f32 * target_scale * 1.5).round() as u32).max(1);
        factor = sub.width() as f32 / nw as f32;
        sub = image::imageops::resize(&sub, nw, nh, image::imageops::FilterType::Triangle);
    }
    im.adjust.apply(&mut sub);
    let (w, h) = sub.dimensions();
    let mut data = sub.into_raw();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
    Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h)?).map(|p| (p, factor))
}

fn draw_image(pm: &mut Pixmap, im: &ImageLayer, base: Transform, scale: f32) {
    let Some((src, factor)) = image_pixmap(im, im.scale * scale) else { return };
    let (cw, ch) = (src.width() as f32 * factor, src.height() as f32 * factor);
    let (sin, cos) = im.rotation.sin_cos();
    let s = im.scale;
    // local (0..cw, 0..ch) -> world
    let place = Transform::from_row(cos * s, sin * s, -sin * s, cos * s, im.center.x, im.center.y)
        .pre_translate(-cw / 2.0, -ch / 2.0)
        .pre_scale(factor, factor);
    let ts = place.post_concat(base);
    let quality = if (im.scale * scale - 1.0).abs() < 1e-4 && im.rotation == 0.0 {
        FilterQuality::Nearest
    } else {
        FilterQuality::Bicubic
    };
    let mut paint = Paint::default();
    paint.shader = Pattern::new(src.as_ref(), SpreadMode::Pad, quality, 1.0, Transform::identity());
    paint.anti_alias = true;
    if let Some(rect) = tiny_skia::Rect::from_xywh(0.0, 0.0, src.width() as f32, src.height() as f32) {
        pm.fill_rect(rect, &paint, ts, None);
    }
}

/// Straight-alpha RGBA image from a premultiplied pixmap.
pub fn to_rgba(pm: &Pixmap) -> image::RgbaImage {
    let mut data = Vec::with_capacity(pm.data().len());
    for p in pm.pixels() {
        let c = p.demultiply();
        data.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    image::RgbaImage::from_raw(pm.width(), pm.height(), data).unwrap()
}

pub fn save(pm: &Pixmap, path: &std::path::Path, jpeg_quality: u8) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let rgba = to_rgba(pm);
    if ext == "jpg" || ext == "jpeg" {
        let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
        let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), jpeg_quality);
        enc.encode_image(&rgb).map_err(|e| e.to_string())
    } else {
        rgba.save_with_format(path, image::ImageFormat::Png).map_err(|e| e.to_string())
    }
}

/// Composited colour under a world point (None if fully transparent).
pub fn sample(doc: &Document, p: Pos2) -> Option<Color32> {
    let r = Rect::from_min_size(p.floor(), vec2(1.0, 1.0));
    let pm = render(doc, r, 1.0, None).ok()?;
    let c = pm.pixel(0, 0)?.demultiply();
    (c.alpha() > 0).then(|| Color32::from_rgb(c.red(), c.green(), c.blue()))
}
