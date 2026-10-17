//! Full-resolution CPU compositor (tiny-skia). Used for export, clipboard copy
//! and the eyedropper, so all three agree with each other.

use egui::{Color32, Pos2, Rect, vec2};
use tiny_skia::{
    FillRule, FilterQuality, LineCap, LineJoin, Paint, PathBuilder, Pattern, Pixmap, PixmapPaint, SpreadMode,
    Stroke, Transform,
};

use crate::doc::{Blend, Document, ImageLayer, Layer, LayerKind, ShapeKind};

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
    let cams = crate::persp::cameras(&doc.spaces);
    for layer in doc.draw_order(&cams).into_iter().filter(|l| l.visible && l.opacity > 0.0) {
        if !layer.shown_bounds(&cams).is_some_and(|b| b.intersects(region)) {
            continue;
        }
        if let Some((pl, cam)) = layer.place.as_ref().and_then(|pl| cams.get(&pl.space).map(|c| (pl, c))) {
            // content -> canvas -> output pixels
            let s = scale as f64;
            let out_m = [[s, 0.0, -region.min.x as f64 * s], [0.0, s, -region.min.y as f64 * s], [0.0, 0.0, 1.0]];
            let h = crate::persp::mul_mm(&out_m, &pl.homography(cam));
            let res = magnification(&h, layer.bounds().unwrap_or(region)).clamp(0.05, 4.0);
            if let Some((src, rect, res)) = flatten_layer(layer, res) {
                warp_onto(&mut out, &src, rect, res, &h, layer.opacity);
            }
            continue;
        }
        if let LayerKind::Text(t) = &layer.kind {
            if t.blend != Blend::Normal {
                let mut tmp = Pixmap::new(w, h).ok_or("임시 버퍼를 만들 수 없음")?;
                draw_layer(&mut tmp, layer, base, region, scale);
                blend_onto(&mut out, &tmp, t.blend, layer.opacity, bg);
                continue;
            }
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

/// Composites premultiplied `src` onto `dst` with a separable blend mode
/// (W3C compositing: Co = as(1-ad)cs + as·ad·B(cd,cs) + (1-as)ad·cd).
/// `bg`: the export background, which 지우기 (erase) reveals instead of punching
/// through to transparency.
pub fn blend_onto(dst: &mut Pixmap, src: &Pixmap, mode: Blend, opacity: f32, bg: Option<Color32>) {
    let bgp = bg.map(|c| {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        let af = a as f32 / 255.0;
        [r as f32 * af, g as f32 * af, b as f32 * af, a as f32]
    });
    for (d, s) in dst.data_mut().chunks_exact_mut(4).zip(src.data().chunks_exact(4)) {
        if s[3] == 0 {
            continue;
        }
        let sa = s[3] as f32 / 255.0 * opacity;
        let da = d[3] as f32 / 255.0;
        if mode == Blend::Erase {
            for k in 0..4 {
                let under = bgp.map_or(0.0, |b| b[k]);
                d[k] = (d[k] as f32 * (1.0 - sa) + under * sa).round().clamp(0.0, 255.0) as u8;
            }
            continue;
        }
        for k in 0..3 {
            let cs = (s[k] as f32 / s[3] as f32).min(1.0);
            let cd = if d[3] > 0 { d[k] as f32 / d[3] as f32 } else { 0.0 };
            let b = mode.mix(cd, cs);
            let co = sa * (1.0 - da) * cs + sa * da * b + (1.0 - sa) * da * cd;
            d[k] = (co * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        d[3] = ((sa + da * (1.0 - sa)) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
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
                if t.rotation == 0.0 {
                    pm.draw_pixmap(ix as i32, iy as i32, tp.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                } else {
                    let c = o + lay.size * scale * 0.5;
                    let ts = Transform::from_translate(ix, iy).post_concat(Transform::from_rotate_at(t.rotation.to_degrees(), c.x, c.y));
                    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..Default::default() };
                    pm.draw_pixmap(0, 0, tp.as_ref(), &paint, ts, None);
                }
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
                    let c = r.center();
                    let ts = base.pre_concat(Transform::from_rotate_at(s.rotation.to_degrees(), c.x, c.y));
                    if let Some(path) = path {
                        if let Some(f) = s.fill {
                            pm.fill_path(&path, &paint_of(f), FillRule::Winding, ts, None);
                        }
                        if s.width > 0.0 {
                            pm.stroke_path(&path, &sp, &stroke, ts, None);
                        }
                    }
                }
            }
        }
    }
}

/// Largest local magnification of a homography over a rectangle's corners.
pub fn magnification(h: &crate::persp::M3, r: Rect) -> f32 {
    use crate::persp::apply_h;
    let mut m = 0.0f32;
    for p in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.center()] {
        let (Some(a), Some(bx), Some(by)) = (apply_h(h, p), apply_h(h, p + vec2(1.0, 0.0)), apply_h(h, p + vec2(0.0, 1.0))) else {
            continue;
        };
        m = m.max(a.distance(bx)).max(a.distance(by));
    }
    if m > 0.0 { m } else { 1.0 }
}

/// One layer's 2D content (its bounds plus a pixel) rendered at `res` pixels per
/// canvas pixel; the resolution is lowered to keep each side within 8192.
pub fn flatten_layer(layer: &Layer, res: f32) -> Option<(Pixmap, Rect, f32)> {
    let b = layer.bounds()?.expand(1.0);
    let res = res.min(8192.0 / b.width().max(b.height()).max(1.0));
    let w = (b.width() * res).ceil().max(1.0) as u32;
    let h = (b.height() * res).ceil().max(1.0) as u32;
    let mut pm = Pixmap::new(w, h)?;
    let base = Transform::from_translate(-b.min.x, -b.min.y).post_scale(res, res);
    let mut flat = layer.clone();
    flat.place = None;
    draw_layer(&mut pm, &flat, base, b, res);
    Some((pm, b, res))
}

/// Draws a flattened layer through `h` (content -> `dst` pixels), sampling the source
/// bilinearly at each destination pixel (exact perspective, no mesh).
pub fn warp_onto(dst: &mut Pixmap, src: &Pixmap, rect: Rect, res: f32, h: &crate::persp::M3, opacity: f32) {
    use crate::persp::{apply_h, invert};
    let Some(hi) = invert(h) else { return };
    let Some(q) = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()]
        .iter()
        .map(|p| apply_h(h, *p))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let bb = Rect::from_points(&q);
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let x0 = (bb.min.x.floor() as i32 - 1).max(0);
    let y0 = (bb.min.y.floor() as i32 - 1).max(0);
    let x1 = (bb.max.x.ceil() as i32 + 1).min(dw);
    let y1 = (bb.max.y.ceil() as i32 + 1).min(dh);
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    let sd = src.data();
    let px = |x: i32, y: i32| -> [f32; 4] {
        if x < 0 || y < 0 || x >= sw || y >= sh {
            return [0.0; 4];
        }
        let i = ((y * sw + x) * 4) as usize;
        [sd[i] as f32, sd[i + 1] as f32, sd[i + 2] as f32, sd[i + 3] as f32]
    };
    let (rx, ry, res) = (rect.min.x as f64, rect.min.y as f64, res as f64);
    let data = dst.data_mut();
    for y in y0..y1 {
        for x in x0..x1 {
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let w = hi[2][0] * fx + hi[2][1] * fy + hi[2][2];
            if w <= 1e-12 {
                continue;
            }
            let cx = (hi[0][0] * fx + hi[0][1] * fy + hi[0][2]) / w;
            let cy = (hi[1][0] * fx + hi[1][1] * fy + hi[1][2]) / w;
            let sx = (cx - rx) * res - 0.5;
            let sy = (cy - ry) * res - 0.5;
            if sx < -1.0 || sy < -1.0 || sx > sw as f64 || sy > sh as f64 {
                continue;
            }
            let (ix, iy) = (sx.floor() as i32, sy.floor() as i32);
            let (tx, ty) = ((sx - ix as f64) as f32, (sy - iy as f64) as f32);
            let (a, b, c, d) = (px(ix, iy), px(ix + 1, iy), px(ix, iy + 1), px(ix + 1, iy + 1));
            let mut s = [0f32; 4];
            for k in 0..4 {
                let top = a[k] + (b[k] - a[k]) * tx;
                let bot = c[k] + (d[k] - c[k]) * tx;
                s[k] = (top + (bot - top) * ty) * opacity;
            }
            if s[3] <= 0.0 {
                continue;
            }
            let i = ((y * dw + x) * 4) as usize;
            let inv = 1.0 - s[3] / 255.0;
            for k in 0..4 {
                data[i + k] = (s[k] + data[i + k] as f32 * inv).round().clamp(0.0, 255.0) as u8;
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
